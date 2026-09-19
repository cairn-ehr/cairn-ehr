//! #615 / #608 — the ONE refusal all five event-log write doors share (db/007's two since #619).
//!
//! # What a substitution is, and why silence is the danger
//!
//! A substitution is a SECOND, DIFFERENT event filed under an `event_id` the log already holds.
//! Every write door inserts `ON CONFLICT (…) DO NOTHING`, because an idempotent re-write of the
//! SAME event must stay a silent no-op — that is set-union, and it is what makes sync safe
//! (principle 1). But the identical no-op is exactly what a substitution looks like from the
//! INSERT's point of view, so without a comparison the two are indistinguishable and the rival
//! is discarded without a word. Two nodes then hold different bytes under one `event_id`,
//! forever, with no alarm.
//!
//! # Why this is a shared helper and not a third inline copy
//!
//! Before #615 the guard was written twice, inline, in `db/005_submit.sql` and
//! `db/020_apply_remote_event.sql`, and not at all in `db/009_node_supersede_and_restore.sql`.
//! Both copies compared with `<>`, which yields NULL — and therefore does NOT fire — when the
//! sub-select feeding it returns no row (#608). Writing a third copy of that into the restore
//! door was the obvious way to fix #615, and would have put a known fail-open into the
//! safety-critical floor a third time, in the one door where the record at stake is the node's
//! own trust set.
//!
//! # What the helper is
//!
//! A PURE raiser. It reads no table; both content-addresses arrive as arguments. That is what
//! lets one function serve `event_log` (db/005, db/020) and `node_event` (db/007, db/009)
//! without knowing about either, and it is why each door keeps its own read: db/005 and db/020
//! are on the 100k-event clinical path and read only when their INSERT was a no-op, while db/007
//! and db/009 read unconditionally and are thereby robust to a later edit disarming a
//! `ROW_COUNT` they no longer set.
//!
//! The door-by-door behaviour lives with the doors — `restore_one_node_event_id_one_body.rs`
//! (db/009), `node_plane_one_event_id_one_body.rs` (db/007's two doors, #619) and
//! `restore_one_event_id_one_body.rs` case 2 (db/020). This file tests the predicate itself,
//! including the arm no door can currently reach.

use cairn_node::db;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic 32-byte content-address stand-in.
///
/// Derived at runtime, never written as a literal (house rule 6a), and the discriminator is
/// called `lineage` rather than `seed`/`salt`/`nonce` (house rule 6b): CodeQL picks its sink by
/// the NAME of the binding a value flows into, and nothing here is cryptographic — these are
/// opaque identifiers the guard only ever compares for equality.
fn address(lineage: u8) -> Vec<u8> {
    (0..32u8)
        .map(|i| i.wrapping_mul(7).wrapping_add(lineage))
        .collect()
}

/// Run the helper once and give back its refusal message, or `None` if it allowed the write.
///
/// `found` is `Option` because the whole point of the helper is that an ABSENT row is a distinct
/// third case from "same" and "different" — the one `<>` gets wrong.
///
/// The id goes over as TEXT and is cast in SQL (`$3::text::uuid`): this project's
/// `tokio-postgres` carries no uuid `ToSql`/`FromSql` binding, and the repo-wide idiom is the
/// cast rather than a feature flag (see `apply_proposal.rs`, which says so at its own call site).
async fn refuse(
    c: &tokio_postgres::Client,
    found: Option<Vec<u8>>,
    new: Vec<u8>,
) -> Option<String> {
    let id = uuid::Uuid::now_v7().to_string();
    c.execute(
        "SELECT cairn_refuse_substitution($1, $2, $3::text::uuid, 'test_door')",
        &[&found, &new, &id],
    )
    .await
    .err()
    .map(|e| {
        e.as_db_error()
            .map(|d| d.message().to_string())
            .unwrap_or_default()
    })
}

/// An idempotent re-write of the SAME event must stay a silent no-op.
///
/// This is the arm that must NOT raise, and it is load-bearing in its own right: every door
/// re-offers events routinely (a peer's full sweep, a resumed restore over the same medium), and
/// a guard that refused a repeat would turn ordinary set-union convergence into a hard failure.
#[tokio::test]
async fn the_same_content_address_is_not_a_substitution() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    assert_eq!(
        refuse(&c, Some(address(1)), address(1)).await,
        None,
        "an idempotent re-write of the SAME event must stay a silent no-op (set-union)"
    );
}

/// Two different bodies under one id are refused, and the refusal names the door that raised it.
///
/// The door name is interpolated rather than hard-coded so this one function can reproduce all
/// three doors' messages byte-for-byte — the reason no existing test's expected text had to move
/// when the inline copies were replaced.
#[tokio::test]
async fn a_different_content_address_is_refused_and_names_its_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let msg = refuse(&c, Some(address(1)), address(2))
        .await
        .expect("two different bodies under one event_id must be refused");
    assert!(
        msg.starts_with("test_door: event_id ")
            && msg.ends_with("already exists with different content (substitution refused)"),
        "the refusal must name the DOOR that raised it, so an operator reading a log knows which \
         write path refused and a caller matching on the text keeps working; got: {msg}"
    );
}

/// The arm `<>` gets wrong, and the whole reason this is a helper rather than a third copy.
///
/// A caller reaches the guard only when its INSERT was a no-op — i.e. when a row with that id
/// exists — so the read-back should always find one. Under READ COMMITTED the next statement's
/// snapshot sees the committed conflicting row, and under REPEATABLE READ or SERIALIZABLE the
/// `ON CONFLICT DO NOTHING` raises 40001 first. So this state should be unreachable today.
///
/// "Should be unreachable" is not a reason to pass. If the read finds nothing, the honest answer
/// is that this floor cannot establish what is stored under the id it is about to write, and on
/// the §9 safety-critical surface that is a refusal. `<>` would yield NULL here and let the write
/// through silently, which is #608.
#[tokio::test]
async fn an_absent_row_fails_closed_rather_than_passing_silently() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // ⚠️ `.is_some()` alone is NOT an assertion here, and it was the first thing this test did.
    // Any database error is `Some` — including `42883 function … does not exist`, which is what
    // a missing helper produces. The test therefore passed vacuously against a tree with no
    // helper at all, i.e. against the exact defect it exists to catch. Match the MESSAGE.
    let msg = refuse(&c, None, address(1))
        .await
        .expect("a NULL found-address must be refused, not allowed through");
    assert!(
        msg.contains("substitution refused"),
        "a NULL found-address means the guard could not establish what is stored: refuse. This \
         is the #608 fail-open, and it must not be reachable through the helper. The refusal \
         must be THE substitution refusal — any other error means this test is passing for a \
         reason that has nothing to do with the guard; got: {msg}"
    );
}
