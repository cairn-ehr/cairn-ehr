//! ADR-0066 decision 6 — the node's custody key is PROVISIONED, and the write path checks.
//!
//! WHY THIS SUITE EXISTS (for a junior reader). Every clinical body Cairn writes is sealed
//! under a fresh per-event DEK, and that DEK is wrapped to this NODE's X25519 unwrap key so
//! the ADR-0005 erasure ladder stays reachable forever. Before ADR-0066 the write path
//! *registered* that key on every sealed write, deriving it from whichever signing key
//! happened to be in hand. Two things were wrong with that:
//!
//! 1. It made a node's custody key an implicit consequence of who signed first, when a
//!    custody key is a **provisioned fact about the node**.
//! 2. It tied custody to identity — and disaster recovery deliberately mints a FRESH
//!    signing seed, so a restored node's derived unwrap secret changed and every inherited
//!    wrapped-DEK row went permanently dark (#495/#500).
//!
//! So registration moved to `cairn-node init` / `cairn-node establish-unwrap-key`, and the
//! write path now VERIFIES. This suite pins both halves of that verification: it refuses
//! when nothing is registered (and names the remedy, because a refusal an operator cannot
//! act on is not a safety control), and it admits when a key IS registered — the positive
//! control, without which a permanently-refusing implementation would still look green.

mod common;

use cairn_node::db;
use uuid::Uuid;

/// Clear the custody-key singleton, putting the database in the exact state a freshly
/// created one is in before `init`/`establish-unwrap-key` has run.
///
/// `node_unwrap_key` has no foreign key to `event_log`, so the CASCADE the other suites
/// use never reaches it — a key left behind by whichever suite ran before us would make
/// this test pass for the wrong reason. Deleting explicitly is what makes the "no key
/// registered" precondition real rather than assumed.
async fn clear_registered_unwrap_key(c: &tokio_postgres::Client) {
    c.execute("DELETE FROM node_unwrap_key", &[])
        .await
        .expect("clearing the unwrap-key singleton");
}

#[tokio::test]
async fn a_sealed_write_fails_loudly_when_no_unwrap_key_is_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // The advisory-lock guard is taken BEFORE the schema load: every DB-gated suite
    // truncates shared tables on entry, so overlapping runs race.
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    clear_registered_unwrap_key(&c).await;

    let err = cairn_node::medication::sealed_submit::ensure_unwrap_key(&c)
        .await
        .expect_err("an unprovisioned node must refuse, not silently write without custody");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("establish-unwrap-key"),
        "the refusal must name the remedy the operator can actually run; got: {msg}"
    );
}

#[tokio::test]
async fn the_check_admits_a_node_whose_unwrap_key_is_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    clear_registered_unwrap_key(&c).await;
    // House rule 6: the key material is COMPUTED at runtime by the same generator
    // provisioning uses, never written as a literal.
    let secret = cairn_event::seal::generate_unwrap_secret().expect("mint an unwrap secret");
    let public = cairn_event::seal::unwrap_public(&secret);
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&public.as_bytes().as_slice()],
    )
    .await
    .expect("registering a freshly minted unwrap key");

    cairn_node::medication::sealed_submit::ensure_unwrap_key(&c)
        .await
        .expect("a provisioned node must be admitted — otherwise the refusal above is vacuous");
}

/// **The floor's own refusal — the half that actually binds (review finding I1).**
///
/// The two tests above pin `medication::sealed_submit::ensure_unwrap_key`, which runs in the
/// DAEMON. That check is a courtesy: it fails early and names the remedy, but any client
/// talking raw SQL goes straight past it. Under principle 12 the binding refusal is the one
/// in the database — `submit_event`'s `IF v_pub IS NULL THEN RAISE` (db/005) — and until now
/// nothing reached it. Every suite registers a key before writing, so softening that RAISE to
/// a `RAISE WARNING`, or dropping it in a refactor, would have left the whole suite green.
///
/// What that would cost: a sealed clinical body written with **no `event_dek` row at all** —
/// permanently unopenable AND permanently un-crypto-shreddable, so the ADR-0005 erasure ladder
/// is unreachable for that event forever. That is the exact outcome ADR-0052 exists to prevent.
///
/// So this test submits the way a raw-SQL client would: it builds a genuinely sealed, signed
/// medication assert with production code and calls `submit_event` itself, with
/// `node_unwrap_key` empty.
#[tokio::test]
async fn the_in_db_floor_refuses_a_sealed_write_with_no_registered_unwrap_key() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // The full clinical fixture (enrolled actors, a patient chart, a registered key) so the
    // door's OTHER floor checks all pass — then take the custody key away, so the unwrap-key
    // clause is the only thing left that can refuse. Without the fixture a refusal here would
    // prove nothing: it could be any one of a dozen unrelated floor rules.
    let (sk_d, kid_d, _sk_h, _kid_h) = common::medication_setup(&c).await;
    // #345: the first event on a chart must be its registration, so the assert below needs a
    // registered chart or the door would refuse it for that reason instead.
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk_d, &kid_d, patient, 0).await;
    clear_registered_unwrap_key(&c).await;

    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let hlc = db::next_hlc(&c, &kid_d).await.unwrap();
    let input = cairn_node::medication::AssertMedicationInput {
        term: "amoxicillin",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let body = cairn_node::medication::build_assert_body(
        event_id,
        medication_id,
        patient,
        &input,
        &kid_d,
        hlc,
        None,
    );
    let (signed_bytes, dek) =
        cairn_node::medication::sealed_submit::seal_and_sign(body, &sk_d).unwrap();

    // The raw door, exactly as `seal_sign_submit` calls it — but WITHOUT the Rust-side
    // `ensure_unwrap_key` in front of it. This is the shape a hostile or merely ignorant
    // direct-SQL client presents.
    let err = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed_bytes, &dek.as_bytes().as_slice()],
        )
        .await
        .expect_err("THE POINT: the in-DB floor must refuse a sealed write with no custody key");
    // `common::db_msg`, not `{err}`: tokio_postgres::Error renders as the bare string
    // "db error" — the RAISE text an operator would actually see lives in the DbError.
    let msg = common::db_msg(&err);
    assert!(
        msg.contains("establish-unwrap-key"),
        "the floor's refusal must name the remedy an operator can run, not just fail: {msg}"
    );

    // Anti-vacuity: the SAME write must SUCCEED once a key is registered, or the refusal
    // above could be any unrelated floor rule and this test would prove nothing.
    let secret = cairn_event::seal::generate_unwrap_secret().unwrap();
    let public = cairn_event::seal::unwrap_public(&secret);
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&public.as_bytes().as_slice()],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
    .expect("positive control: the identical write must be admitted once custody is provisioned");
}
