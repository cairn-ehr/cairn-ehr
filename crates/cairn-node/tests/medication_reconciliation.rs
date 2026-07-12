//! §3.15/§3.16 medication reconciliation resolution (slice 3) — DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Patients and
//! threads need no pre-existence (offline-first). Key material is runtime-derived.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{build_reconcile_body, reconcile_medications, ReconcileInput};
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
