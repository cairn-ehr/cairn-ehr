//! §3.15/§3.16 medication reconciliation resolution (slice 3) — DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Patients and
//! threads need no pre-existence (offline-first). Key material is runtime-derived.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_reconcile_body, reconcile_medications, separate_medications,
    AssertMedicationInput, ReconcileInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log + every medication projection and enroll a fresh device actor.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN TRUNCATE medication_dose_correction; END IF; \
           IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN TRUNCATE medication_reconciliation; END IF; \
           IF to_regclass('public.medication_group_member') IS NOT NULL THEN TRUNCATE medication_group_member; END IF; \
           IF to_regclass('public.medication_projection_flag') IS NOT NULL THEN TRUNCATE medication_projection_flag; END IF; \
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

/// A minimal, valid medication assertion input for a given term — used to mint
/// real threads (rather than bare UUIDs) for the grouping tests below.
fn sample_assert(term: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
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

#[tokio::test]
async fn floor_accepts_valid_reconciliation() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: Some("brand vs generic"),
    };
    // Offline-first: neither thread need exist locally.
    let ev = reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the reconciliation event landed in the log");
}

#[tokio::test]
async fn floor_rejects_self_reconcile() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    // Hand-build a self-reconcile (bypass the Rust guard) and submit directly.
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = build_reconcile_body(Uuid::now_v7(), a, a, patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("self-reconcile") || err.contains("distinct"),
        "got: {err}"
    );
}

#[tokio::test]
async fn floor_rejects_missing_provenance() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let input = ReconcileInput {
        provenance: "   ",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = build_reconcile_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        patient,
        &input,
        &kid,
        hlc,
    );
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("provenance"), "got: {err}");
}

/// Helper: the group_id a thread maps to (or the thread itself when un-reconciled).
async fn group_of(c: &Client, med: Uuid) -> Uuid {
    let row = c
        .query_opt(
            "SELECT group_id::text FROM medication_group_member WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap();
    match row {
        Some(r) => r.get::<_, String>(0).parse().unwrap(),
        None => med, // no row = collapses to itself
    }
}

#[tokio::test]
async fn reconcile_maps_both_threads_to_min_uuid_group() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
    )
    .await
    .unwrap();
    let b = assert_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
    )
    .await
    .unwrap();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let expected = std::cmp::min(a, b);
    assert_eq!(group_of(&c, a).await, expected);
    assert_eq!(
        group_of(&c, b).await,
        expected,
        "both threads collapse to the min-UUID group"
    );
}

#[tokio::test]
async fn transitive_component_and_clean_split() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert("metformin"),
    )
    .await
    .unwrap();
    let b = assert_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert("metformin"),
    )
    .await
    .unwrap();
    let d = assert_medication(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert("metformin"),
    )
    .await
    .unwrap();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    reconcile_medications(&c, &sk, &kid, "test-node", patient, b, d, &input)
        .await
        .unwrap();
    let min = std::cmp::min(a, std::cmp::min(b, d));
    assert_eq!(group_of(&c, a).await, min);
    assert_eq!(
        group_of(&c, d).await,
        min,
        "A-B, B-C transitively one group"
    );
    // Separating B-D splits D back out; A-B stays together.
    separate_medications(&c, &sk, &kid, "test-node", patient, b, d, &input)
        .await
        .unwrap();
    assert_eq!(group_of(&c, a).await, std::cmp::min(a, b));
    assert_eq!(group_of(&c, b).await, std::cmp::min(a, b));
    assert_eq!(
        group_of(&c, d).await,
        d,
        "D is isolated again after separation"
    );
}

#[tokio::test]
async fn reconciliation_before_threads_converges() {
    // Offline-first: the reconciliation applies before either assert is local.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    // The edge stands and the group is computed even with no statements yet.
    assert_eq!(group_of(&c, a).await, std::cmp::min(a, b));
    assert_eq!(group_of(&c, b).await, std::cmp::min(a, b));
}
