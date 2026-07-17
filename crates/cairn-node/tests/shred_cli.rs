//! ADR-0052 rung-3 shred ceremony. DB-gated on $CAIRN_TEST_PG, serialized cluster-wide
//! via db::test_serial_guard (shared-DB + TRUNCATE pattern, like medication.rs /
//! seal_submit.rs). Key material is derived at runtime (generate_key), never a literal
//! (house rule 6).
//!
//! Drives `shred::shred_event` directly (not the CLI binary) — the same convention
//! every other verb test in this crate uses (see medication.rs).
use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::medication::{assert_medication, AssertMedicationInput};
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the log + medication projections + the ADR-0052 custody plane and enroll a
/// fresh device actor. node_unwrap_key/event_dek/event_clear/erasure_shred_log have NO
/// FK to event_log, so the CASCADE from event_log does not reach them — a stale
/// prior-test node key would otherwise collide with this test's fresh one at
/// cairn_register_unwrap_key (the singleton refuses a different key). Mirrors
/// tests/medication.rs's setup_node verbatim.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

fn sample_input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "atorvastatin",
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    }
}

/// Look up the event_id of the `clinical.medication.asserted` event that minted
/// `medication_id`. `assert_medication` returns the THREAD id, not the content event's
/// own id (it mints both, see medication/assert.rs), so shred's target — a specific
/// event_id — has to be resolved separately. Reads through `cairn_clear_payload`
/// (ADR-0052: sealed content carries ciphertext in `body`; the thread key lives in the
/// `event_clear` shadow) — mirrors the lookup in tests/medication_attestation.rs.
async fn assert_event_id(c: &Client, medication_id: Uuid) -> Uuid {
    let s: String = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE event_type = 'clinical.medication.asserted' \
             AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    s.parse().unwrap()
}

async fn count(c: &Client, sql: &str, id: Uuid) -> i64 {
    c.query_one(sql, &[&id.to_string()]).await.unwrap().get(0)
}

#[tokio::test]
async fn shred_event_appends_tombstone_and_scrubs() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // 1. Seal-submit a medication assert (device-additive, no attestation) via the
    //    real product orchestrator — exactly what a clinician's CLI call would do.
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_input(),
        None,
    )
    .await
    .expect("assert_medication succeeds");
    let target = assert_event_id(&c, med_id).await;

    // 2. Confirm the pre-shred custody + projection exist (otherwise the scrub
    //    assertions below would be vacuously true).
    let stmt_before = count(
        &c,
        "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
        patient,
    )
    .await;
    assert_eq!(stmt_before, 1, "the assert projected before the shred");
    let dek_before = count(
        &c,
        "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
        target,
    )
    .await;
    assert_eq!(dek_before, 1, "custody exists before the shred");
    let clear_before = count(
        &c,
        "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
        target,
    )
    .await;
    assert_eq!(clear_before, 1, "derived plaintext exists before the shred");

    // 3. Shred it — device-additive (attest = None), the required deliverable path.
    let shred_id = shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        target,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred_event succeeds on a locally-present target");

    // 4. erasure_shred_log carries the row, with the basis we gave.
    let (logged_shred_id, basis): (String, String) = {
        let row = c
            .query_one(
                "SELECT shred_event_id::text, basis FROM erasure_shred_log \
                 WHERE target_event_id = $1::text::uuid",
                &[&target.to_string()],
            )
            .await
            .expect("the shred log carries the target's row");
        (row.get(0), row.get(1))
    };
    assert_eq!(
        logged_shred_id,
        shred_id.to_string(),
        "the log names the shredding event"
    );
    assert_eq!(basis, "retention ceiling");

    // 5. Custody, derived plaintext, and the projection are all scrubbed.
    let dek_after = count(
        &c,
        "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
        target,
    )
    .await;
    assert_eq!(dek_after, 0, "the shred scrubbed custody");
    let clear_after = count(
        &c,
        "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
        target,
    )
    .await;
    assert_eq!(clear_after, 0, "the shred scrubbed the derived plaintext");
    let stmt_after = count(
        &c,
        "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
        patient,
    )
    .await;
    assert_eq!(stmt_after, 0, "the shred scrubbed the projection");

    // 6. The tombstone itself is legible: its plaintext_twin names BOTH the target and
    //    the basis, and it lands in the SAME chart as the event it describes (never
    //    an orphaned tombstone unfindable from the record it is about). Append-only:
    //    the event_log row for the tombstone stays, unlike the target's derived state.
    let (twin, tomb_patient): (String, String) = {
        let row = c
            .query_one(
                "SELECT plaintext_twin, patient_id::text FROM event_log WHERE event_id = $1::text::uuid",
                &[&shred_id.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(
        twin.contains(&target.to_string()),
        "the tombstone's twin names the target, got: {twin}"
    );
    assert!(
        twin.contains("retention ceiling"),
        "the tombstone's twin names the basis, got: {twin}"
    );
    assert_eq!(
        tomb_patient,
        patient.to_string(),
        "the tombstone lands in the same chart as its target"
    );
}

#[tokio::test]
async fn shred_refuses_an_unknown_target_legibly() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;

    let err = shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        Uuid::now_v7(), // an event id nothing authored — never present locally
        "retention ceiling",
        None,
    )
    .await
    .expect_err("an unknown target must be refused, not silently accepted");
    assert!(
        err.to_string().contains("nothing to shred"),
        "the refusal names the missing target, got: {err}"
    );
}
