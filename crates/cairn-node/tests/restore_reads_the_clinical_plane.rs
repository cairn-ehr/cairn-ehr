//! #554 slice 2d — the guarantee this whole disaster-recovery programme exists for.
//!
//! A solo clinic backs up nightly, `verify-backup` passes, the disk dies, and `cairn-node
//! restore` brings back a node that knows who it peered with and **zero patients**. Slice 2c
//! made the bytes exist off-machine; this is the half that gives them back.
//!
//! # Why the headline test reads a body in CLEAR
//!
//! Because it is **the only assertion that can distinguish a correct restore from the
//! double-wrap of design §2.1.** `apply_remote_event`'s `p_dek` is fed straight into
//! `cairn_wrap_dek(p_dek, v_pub)` — the door wraps what it is handed — and both carriers hold
//! keys that are already wrapped to this node. Pipe one through and every `event_dek` row is
//! present, well-formed, exactly the right length, and unwraps to noise. Counts agree.
//! `verify-backup` is green. The defect surfaces months later, when a clinician opens a chart
//! on a node that can no longer be re-restored. A test that counted rows would have shipped it.
//!
//! # What lives here and what does not
//!
//! Here: the end-to-end guarantee, the named double-wrap regression, and the custody rules
//! (design §6) — a sealed body with NO custody restores anyway, a record that CARRIES custody
//! with no key installed is penned WITH that key. The pen's own quota carve-out and the
//! registry door's fences live where they are enforced: `db/tests/052_restore_doors_test.sql`.

use tokio_postgres::Client;
use uuid::Uuid;

use cairn_event::seal::{seal_event_payload, seal_stub_twin, Secret32};
use cairn_event::{sign, EventBody, SigningKey};
use cairn_node::restore::clinical::{apply_clinical_plane, RESTORE_PEER_SENTINEL};
use cairn_node::{backup, db, identity};

mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A provisioned solo clinic, exactly as `dr_clinical_guarantee_gap.rs` builds one: node
/// identity, an enrolled actor, and a registered unwrap key derived from the DEVICE key.
async fn provisioned_clinic(c: &Client) -> (SigningKey, String) {
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(c).await;
    identity::provision(c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();
    (sk, kid)
}

/// Submit ONE real born-sealed clinical event through the STRICT door, and return its id,
/// its signed bytes, and the twin text the restore must be able to read back in clear.
///
/// A production-door body, never a hand-built row: the `event_dek` custody these tests read
/// has to be what the real writer produces, or the restore is being tested against a fixture
/// rather than against the system.
async fn author_sealed_clinical_event(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
) -> (String, Vec<u8>, String) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;

    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    // The sentinel the headline test looks for on the far side of a disaster. It is a real
    // twin string, so finding it proves the body was UNSEALED, not merely that a row exists.
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id: event_id.clone(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
    .expect("a sealed body with its DEK is admitted");

    // ANTI-VACUITY: the clear view really exists on the SOURCE node, so "it came back" below
    // is a statement about the restore rather than about a body that was never readable.
    let before: String = c
        .query_one(
            "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("the strict door writes a clear view for a sealed body")
        .get(0);
    assert_eq!(before, twin, "the source node can read its own chart");

    (event_id, signed.signed_bytes, twin)
}

/// Put the database in the state a disaster-recovery machine is in: no clinical tier, no
/// federation identity. The list mirrors the one `dr_clinical_guarantee_gap.rs` uses.
async fn wipe_to_a_fresh_dr_machine(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, event_dek, event_clear, erasure_shred_log, patient_chart CASCADE",
    )
    .await
    .expect("wiping the clinical tier, as a fresh DR machine would have it");
    c.batch_execute("DELETE FROM sync_quarantine")
        .await
        .unwrap();
    db::reset_node_federation_tables(c).await.unwrap();
}

/// This node's registered unwrap SECRET, as a restore inherits it.
///
/// The fixture registers a key derived from the device signing key (`medication_setup`), so
/// this reproduces the same derivation rather than reading a keystore file the test never
/// wrote. **Not a widening of `derive_unwrap_secret`'s allow-list** — that guard sweeps
/// PRODUCTION trees only, and this is a test reconstructing what the fixture already did.
fn fixture_unwrap_secret(sk: &SigningKey) -> Secret32 {
    cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()))
}

/// Capture a medium from the live database and hand back its clinical records, in the order
/// a restore consumes them.
async fn capture_clinical_records(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dir: &std::path::Path,
) -> Vec<cairn_medium::MediumRecord> {
    let medium_path = dir.join("cairn.medium");
    let health_path = dir.join("backup-status.json");
    backup::backup_to(c, &medium_path, &health_path, 0, Some((sk, kid)))
        .await
        .expect("the backup ceremony succeeds");
    let image = cairn_node::medium::parse_any(&std::fs::read(&medium_path).unwrap())
        .expect("the medium `backup_to` wrote must parse");
    let records = backup::clinical_plane_records(&image).expect("the clinical reader");
    assert!(
        !records.is_empty(),
        "anti-vacuity: the medium must actually carry clinical records, or every assertion \
         about restoring them is a statement about an empty set"
    );
    records
}

// ---------------------------------------------------------------------------

/// **THE GUARANTEE.** Seal a real clinical body, capture it, wipe the machine, restore, and
/// **read the payload back in clear**.
///
/// The clear-view assertion is the whole point: a double-wrapped `event_dek` row is present,
/// well-formed and the right length, so only decrypting through it can tell a correct restore
/// from one that silently destroyed every key in the clinic's record.
#[tokio::test]
async fn a_clinical_event_restores_from_a_medium_and_its_body_opens() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, clinical_bytes, twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let carried = records
        .iter()
        .find(|r| r.signed_bytes == clinical_bytes)
        .expect("slice 2c's guarantee: the medium carries the clinical event");
    assert!(
        carried.dek_wrapped.is_some(),
        "and its custody — without which this test could not tell a correct restore from a \
         restore that dropped every key"
    );

    let secret = fixture_unwrap_secret(&sk);
    // The dead node's local-state export: its custody key and its ACTOR REGISTRY. Read here,
    // while the node is still alive, exactly as `backup` writes it.
    let bundle = cairn_node::localstate_read::read_local_state(&c, Some(&secret))
        .await
        .expect("the export is readable on a live node");
    assert!(
        !bundle.actor_registry().is_empty(),
        "anti-vacuity: the export must carry the registry, or the ceremony below proves \
         nothing about restoring it"
    );

    // THE DISASTER. A fresh machine: no clinical tier, no federation identity, no registry.
    wipe_to_a_fresh_dr_machine(&c).await;
    clear_actor_registry(&c).await;

    // THE CEREMONY, in design §3's order and for its reasons. The custody key installs and
    // REGISTERS first, because `apply_remote_event` wraps every DEK to the registered public
    // half. The actor registry installs with it, because every apply door resolves its author
    // through `actor_current` — WITHOUT THIS STEP the door refuses this node's own history
    // with "signer … is not an enrolled, non-revoked actor", which is the "zero patients"
    // outcome wearing a different costume, and is what this test caught when the step was
    // missing. `finalize_identity` would run LAST, after the clinical apply below.
    let keydir = tempfile::tempdir().unwrap();
    let applied_state = cairn_node::localstate::apply_local_state(
        &c,
        &bundle,
        &cairn_node::localstate::CustodyKeyDestination::Plaintext {
            path: &keydir.path().join("restored.key.unwrap"),
        },
    )
    .await
    .expect("the export applies into a fresh database");
    assert!(
        applied_state.actor_registry_restored() > 0,
        "the registry must actually have landed"
    );

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("the clinical plane applies");
    assert_eq!(
        report.penned(),
        0,
        "nothing on this medium should be refused: {:?}",
        report.refusals
    );
    assert!(report.applied > 0, "anti-vacuity: records genuinely landed");
    // COMPLETENESS. Every record on the medium must be accounted for by exactly one of the
    // three outcomes. Without this, a loop that stopped after the first record — or a
    // `continue` on a path that should have applied — passes every other assertion here
    // while a chart comes back holding its first event and nothing else. "Restored N of M,
    // honestly" is the whole subject of #500/#554, and N was never compared to M.
    assert_eq!(
        report.applied + report.already_present + report.penned(),
        records.len(),
        "every record on the medium must land in exactly one outcome: {report:?}"
    );

    let rows: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert!(rows > 0, "the clinical log came back");

    let custody: i64 = c
        .query_one("SELECT count(*) FROM event_dek", &[])
        .await
        .unwrap()
        .get(0);
    assert!(custody > 0, "and its custody came back with it");

    // THE ASSERTION THAT MATTERS. Not "a row exists" — the body OPENS.
    let restored_twin: String = c
        .query_one(
            "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect(
            "a restored node must be able to READ the chart, not merely hold ciphertext — \
             this is the sentence #554 exists for",
        )
        .get(0);
    assert_eq!(
        restored_twin, twin,
        "the restored body must decrypt to what the dead node held"
    );

    // AND THE CLINICIAN CAN FIND IT. `event_clear` holding a readable body is not yet a chart:
    // db/020 raises `cairn.remote_apply` across the projection triggers so they clamp-and-flag
    // rather than veto, and a projection that silently no-opped under that marker would leave
    // `event_log` and `event_clear` exactly as asserted above with the chart list EMPTY. That
    // is the zero-patients outcome wearing its third costume, and it costs one query to close.
    let charted: i64 = c
        .query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .unwrap()
        .get(0);
    assert!(
        charted > 0,
        "a restored node must have PATIENTS, not merely rows: the projections must have run \
         under the remote-apply marker"
    );
}

/// **THE DOUBLE-WRAP REGRESSION, NAMED.** The DEK reaching `apply_remote_event` is the
/// PLAINTEXT one.
///
/// Stated as its own assertion so a future "simplification" that passes `dek_wrapped` straight
/// through reddens HERE, with a legible reason, rather than in the guarantee test above as an
/// opaque decryption failure. The proof is the round trip: unwrap the stored `event_dek` row
/// with this node's secret and check the result opens the body — which it cannot do if the
/// door was handed something already wrapped.
#[tokio::test]
async fn the_dek_handed_to_the_door_is_the_plaintext_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, _bytes, _twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let secret = fixture_unwrap_secret(&sk);

    wipe_clinical_tier_only(&c).await;
    apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("the clinical plane applies");

    let stored: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("custody landed")
        .get(0);
    let opened = cairn_event::seal::unwrap_dek(&stored, &secret).expect(
        "the restored event_dek row must unwrap in ONE step. A row that needs two is \
         double-wrapped: the caller handed the door a key that was already wrapped, and the \
         door wrapped it again. Present, well-formed, right length, and permanently useless.",
    );
    assert_eq!(
        opened.as_bytes().len(),
        32,
        "and what comes out is a DEK, not another wrapper"
    );
}

/// **A sealed body with NO custody restores anyway** (design §6, test 5).
///
/// A body crypto-shredded BEFORE its first capture arrives sealed with `dek_wrapped = None`:
/// its ciphertext travels, its key was destroyed on purpose (ADR-0005 — a shred destroys the
/// key, never the event). It must restore custody-less, exactly as it stands on the dead node.
///
/// This is the case a rule keyed on SEALEDNESS rather than on CUSTODY would pen forever, over
/// a key that does not exist and is not supposed to.
#[tokio::test]
async fn a_sealed_record_with_no_custody_restores_rather_than_pens() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, clinical_bytes, _twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let mut records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    // Model the shredded-before-capture record: the same sealed bytes, no key beside them.
    for r in &mut records {
        if r.signed_bytes == clinical_bytes {
            r.dek_wrapped = None;
        }
    }
    let secret = fixture_unwrap_secret(&sk);

    wipe_clinical_tier_only(&c).await;
    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("the clinical plane applies");
    assert_eq!(
        report.penned(),
        0,
        "a sealed body whose key was destroyed must RESTORE, not pen: {:?}",
        report.refusals
    );

    let present: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        present, 1,
        "the ciphertext is in the record, as it should be"
    );
    let custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        custody, 0,
        "and it has no custody, exactly as it stood on the dead node — a restore must not \
         invent a key an erasure destroyed"
    );
}

/// **The no-export path, keyed on CUSTODY** (design §6, test 6).
///
/// With no usable export — no passphrase (every unattended cron run), a corrupt `.lsk`, or an
/// export carrying rows but no key — a record that CARRIES a `dek_wrapped` is refused here
/// rather than admitted by db/020's lenient arm, and penned **with that custody intact**.
///
/// db/020's arm downgrades a missing unwrap key to a WARNING and admits the event without
/// custody. Correct for a puller, which sees the DEK again next cycle. For a restore it would
/// admit ciphertext into a node `finalize_identity` then fences, with no second delivery ever.
#[tokio::test]
async fn with_no_custody_key_a_record_carrying_one_is_penned_with_its_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (_event_id, clinical_bytes, _twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let with_custody = records.iter().filter(|r| r.dek_wrapped.is_some()).count();
    assert!(
        with_custody > 0,
        "anti-vacuity: something on this medium must carry custody"
    );

    wipe_clinical_tier_only(&c).await;
    c.batch_execute("DELETE FROM sync_quarantine")
        .await
        .unwrap();

    // `None` — no usable export was applied.
    let report = apply_clinical_plane(&c, &records, None)
        .await
        .expect("a keyless restore still completes; it does not abort");
    assert_eq!(
        report.penned(),
        with_custody,
        "EVERY record carrying custody is penned, and only those: {:?}",
        report.refusals
    );

    let digest = cairn_event::event_address(&clinical_bytes);
    let row = c
        .query_one(
            "SELECT peer, dek_wrapped, reason FROM sync_quarantine WHERE content_digest = $1",
            &[&digest],
        )
        .await
        .expect("the sealed event is in the pen");
    let peer: String = row.get(0);
    let penned_dek: Option<Vec<u8>> = row.get(1);
    let reason: String = row.get(2);
    assert_eq!(
        peer, RESTORE_PEER_SENTINEL,
        "a restore-penned row must be identifiable as one, never blend into an unnamed link"
    );
    assert!(
        penned_dek.is_some(),
        "THE CUSTODY MUST SURVIVE THE PEN. A restored solo node has no peer to re-serve this \
         key: dropping it here means a later requeue admits permanently-unopenable \
         ciphertext at exit 0 — #500's own shape, one layer down"
    );
    assert!(
        reason.starts_with("restore:"),
        "and the reason must name the restore as its origin: {reason}"
    );

    // NO FLOOR PINNED (design §2.3): no peer re-offers a medium, so a floor set here would
    // make the restored node's first real pull re-fetch from a position nobody will resolve.
    let floors: i64 = c
        .query_one(
            "SELECT count(*) FROM sync_state WHERE quarantine_floor_seq IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        floors, 0,
        "a restore must not pin the re-offer floor — it would wedge federation over an event \
         that has nothing to do with any peer"
    );
}

/// Wipe only the clinical tier, leaving the node enrolled and its unwrap key registered.
///
/// Distinct from [`wipe_to_a_fresh_dr_machine`] on purpose: these tests exercise the clinical
/// APPLY in isolation, and the surrounding ceremony (un-enrolled fence, custody install,
/// registry install, `finalize_identity` last) is `main.rs`'s and is tested against the CLI.
async fn wipe_clinical_tier_only(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, event_dek, event_clear, erasure_shred_log, patient_chart CASCADE",
    )
    .await
    .expect("wiping the clinical tier");
}

/// Clear `actor_event`, as a freshly-installed disaster-recovery machine has it.
///
/// `actor_event` is append-only (db/004 refuses DELETE by trigger), so this disables that
/// trigger for the duration. A test-fixture act, never something a node does — the door's own
/// fence 2 is what protects a real registry, and it is pinned in the SQL mirror.
async fn clear_actor_registry(c: &Client) {
    c.batch_execute(
        "ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update;
         DELETE FROM actor_event;
         ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;",
    )
    .await
    .unwrap();
}

/// **CUSTODY THAT DOES NOT LAND IS A REFUSAL, NOT A SUCCESS** (PR #566 review, critical 1).
///
/// `apply_remote_event` has two LENIENT arms that warn and admit: a presented DEK that fails
/// to open the sealed body (db/020's *"sidecar DEK failed to open sealed body"*), and an
/// unregistered node unwrap key. Both leave `v_inner` or `v_pub` NULL, which skips db/020's
/// step 9 entirely — no `event_dek`, no `event_clear`, no twin, no projection — and then
/// **return normally**. That is correct for a PULLER, which will see the DEK again next cycle.
///
/// For a RESTORE it is the zero-patients outcome with a clean summary on top. The door said
/// OK, so the record counted as `applied`; the only signal db/020 gives is a Postgres
/// `WARNING`, and nothing in this tree polls the connection's message stream. The clinic reads
/// *"N applied"* at exit 0 and a clinician opens an empty chart months later, by which time the
/// medium has been rotated. The module header reasons about db/020's lenient arm and pens the
/// two failures it can see IN RUST; this is the third, and only the door can see it.
///
/// The fixture presents a **validly wrapped but wrong** DEK: it unwraps cleanly on this side,
/// so every restore-side custody check passes and the record reaches the door — which is
/// precisely the state the lenient arm exists for.
#[tokio::test]
async fn a_record_whose_custody_does_not_land_is_penned_not_counted_applied() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, clinical_bytes, _twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let secret = fixture_unwrap_secret(&sk);

    // A DEK that is WRONG for this body but perfectly well-formed and wrapped to this node,
    // derived at runtime rather than written as a literal (house rule 6a). `lineage`, not
    // `salt`: it discriminates a fixture, it constructs nothing cryptographic (rule 6b).
    let lineage = 7u8;
    let wrong_dek = Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(lineage).wrapping_add(3)
    }));
    let wrong_wrapped =
        cairn_event::seal::wrap_dek_for(&wrong_dek, &cairn_event::seal::unwrap_public(&secret))
            .expect("a well-formed wrap to this node's own public half");

    let mutated: Vec<cairn_medium::MediumRecord> = records
        .iter()
        .cloned()
        .map(|mut r| {
            if r.signed_bytes == clinical_bytes {
                r.dek_wrapped = Some(wrong_wrapped.clone());
            }
            r
        })
        .collect();
    assert!(
        mutated
            .iter()
            .any(|r| r.signed_bytes == clinical_bytes
                && r.dek_wrapped.as_ref() == Some(&wrong_wrapped)),
        "anti-vacuity: the fixture must actually have swapped the DEK, or this test asserts \
         nothing about the lenient arm"
    );

    let bundle = cairn_node::localstate_read::read_local_state(&c, Some(&secret))
        .await
        .expect("the export is readable on a live node");

    wipe_to_a_fresh_dr_machine(&c).await;
    clear_actor_registry(&c).await;

    let keydir = tempfile::tempdir().unwrap();
    cairn_node::localstate::apply_local_state(
        &c,
        &bundle,
        &cairn_node::localstate::CustodyKeyDestination::Plaintext {
            path: &keydir.path().join("restored.key.unwrap"),
        },
    )
    .await
    .expect("the export applies into a fresh database");

    let report = apply_clinical_plane(&c, &mutated, Some(&secret))
        .await
        .expect("a wrong DEK is a refusal of one record, never a failure of the run");

    // THE ASSERTION. The door admitted the row and withheld custody; the restore must not
    // report that as a record it applied.
    let custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        custody, 0,
        "fixture precondition: db/020's lenient arm withholds custody for a wrong DEK"
    );
    assert!(
        report.penned() > 0,
        "a record whose custody did not land is a REFUSAL: it must be penned, with its \
         bytes and its key held for a later `cairn-sync requeue`, not counted as applied. \
         report = {report:?}"
    );

    let penned: i64 = c
        .query_one(
            "SELECT count(*) FROM sync_quarantine WHERE peer = $1",
            &[&RESTORE_PEER_SENTINEL],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        penned > 0,
        "and it is genuinely in the pen, not merely counted"
    );
}
/// **WITHOUT A REGISTRY, NOT ONE RECORD IS OFFERED** (PR #566 review, important 2).
///
/// Every apply door resolves its author through `actor_current`, so with no registry the door
/// refuses this node's own history as an unenrolled signer — every record, one at a time. The
/// old behaviour warned about that correctly and then applied anyway, which wrote the clinic's
/// entire clinical corpus a SECOND time into `sync_quarantine` to build a pen that cannot be
/// drained: `finalize_identity` runs at the end of the restore and closes the registry door
/// permanently, so nothing can ever release those rows.
///
/// It then closed by printing the pen's standard remedy, promising `cairn-sync requeue` would
/// complete the restore — the exact false promise the warning above it exists to prevent, made
/// to someone mid-disaster who reads the tail of a long run.
///
/// Refusing to start is the honest answer, and it leaves the database restorable from the same
/// medium once the export is recovered.
#[tokio::test]
async fn with_no_actor_registry_the_clinical_plane_is_not_offered_at_all() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (_event_id, _bytes, _twin) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let secret = fixture_unwrap_secret(&sk);

    // The disaster, WITHOUT the registry half of the export ever arriving.
    wipe_to_a_fresh_dr_machine(&c).await;
    clear_actor_registry(&c).await;

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("a missing registry is a precondition, never a crash");

    assert!(
        report.skipped_no_registry,
        "the run must SAY it declined, so the summary can print the fresh-database remedy \
         instead of the pen's requeue promise: {report:?}"
    );
    assert_eq!(
        report.penned(),
        0,
        "not one record may be penned: {report:?}"
    );
    assert_eq!(report.applied, 0, "and none applied: {report:?}");

    let penned: i64 = c
        .query_one("SELECT count(*) FROM sync_quarantine", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        penned, 0,
        "the pen must be EMPTY — filling it here doubles the clinic's corpus on disk to \
         build something no `requeue` can ever drain"
    );
}
/// **A RESUME IS NOT A RUN THAT DID NOTHING** (PR #566 review).
///
/// The door is idempotent, so re-running a restore over a medium it already applied is safe —
/// and that is precisely why the two outcomes must be told apart. `already_present` exists so
/// a resumed restore cannot be mistaken for a first run that silently applied nothing, and
/// until now nothing asserted it: the field was printed and never checked, so folding it back
/// into `applied` would have gone unnoticed.
#[tokio::test]
async fn a_second_pass_counts_already_present_rather_than_applying_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let _ = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let secret = fixture_unwrap_secret(&sk);
    let bundle = cairn_node::localstate_read::read_local_state(&c, Some(&secret))
        .await
        .expect("the export is readable on a live node");

    wipe_to_a_fresh_dr_machine(&c).await;
    clear_actor_registry(&c).await;
    let keydir = tempfile::tempdir().unwrap();
    cairn_node::localstate::apply_local_state(
        &c,
        &bundle,
        &cairn_node::localstate::CustodyKeyDestination::Plaintext {
            path: &keydir.path().join("restored.key.unwrap"),
        },
    )
    .await
    .expect("the export applies into a fresh database");

    let first = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("the first pass applies");
    assert!(
        first.applied > 0,
        "anti-vacuity: the first pass did the work"
    );
    assert_eq!(first.already_present, 0, "nothing was here before it");

    // THE RESUME: the same medium, into the same database, exactly as an operator re-running
    // an interrupted restore would.
    let second = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("re-applying the same medium is safe");
    assert_eq!(
        second.applied, 0,
        "a second pass applies nothing new: {second:?}"
    );
    assert_eq!(
        second.already_present,
        records.len(),
        "and it must SAY the records were already here — folded into `applied` this reads \
         as a working restore, folded into nothing it reads as one that did nothing: {second:?}"
    );
    assert_eq!(second.penned(), 0, "and refuses nothing: {second:?}");
}
/// **AN ACKED PEN ROW DOES NOT GET THE REQUEUE PROMISE** (PR #566 review).
///
/// `cairn_quarantine_event` returns TRUE when the bytes are already ACKED — db/052 documents
/// that return as load-bearing, and `cairn-sync` reads it. The restore threw it away, so on a
/// resumed restore an operator's recorded decision that a record will never enter the record
/// was counted as an ordinary refusal, and both the summary and the reason stored in
/// `sync_quarantine.reason` promised `cairn-sync requeue` would complete the restore — for a
/// row `do_requeue` deliberately skips.
#[tokio::test]
async fn a_pen_row_already_acked_is_counted_apart_from_an_ordinary_refusal() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let _ = author_sealed_clinical_event(&c, &sk, &kid).await;

    let tmp = tempfile::tempdir().unwrap();
    let records = capture_clinical_records(&c, &sk, &kid, tmp.path()).await;
    let secret = fixture_unwrap_secret(&sk);
    let bundle = cairn_node::localstate_read::read_local_state(&c, Some(&secret))
        .await
        .expect("the export is readable on a live node");

    wipe_to_a_fresh_dr_machine(&c).await;
    clear_actor_registry(&c).await;
    let keydir = tempfile::tempdir().unwrap();
    cairn_node::localstate::apply_local_state(
        &c,
        &bundle,
        &cairn_node::localstate::CustodyKeyDestination::Plaintext {
            path: &keydir.path().join("restored.key.unwrap"),
        },
    )
    .await
    .expect("the export applies into a fresh database");

    // Pass 1 with NO custody key: every record carrying one is penned with its key.
    let first = apply_clinical_plane(&c, &records, None)
        .await
        .expect("the no-export path pens rather than fails");
    assert!(first.penned() > 0, "anti-vacuity: the pen is not empty");
    assert_eq!(
        first.penned_but_acked, 0,
        "a freshly penned row carries no decision yet: {first:?}"
    );

    // The operator records a decision: these bytes will never enter the record.
    c.execute("UPDATE sync_quarantine SET acked = true", &[])
        .await
        .unwrap();

    // Pass 2, as a resumed restore re-offers the same medium.
    let second = apply_clinical_plane(&c, &records, None)
        .await
        .expect("re-penning an acked row is not a failure");
    assert!(
        second.penned_but_acked > 0,
        "the pen SAID these were already acked and the restore must not discard that — \
         otherwise the summary promises a `requeue` that deliberately skips them: {second:?}"
    );
}
