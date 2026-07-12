//! §3.15/§3.16 medication reconciliation resolution (slice 3) — DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Patients and
//! threads need no pre-existence (offline-first). Key material is runtime-derived.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_reconcile_body, cease_medication, change_dose, reconcile_medications,
    separate_medications, AssertMedicationInput, CeaseMedicationInput, ChangeDoseInput,
    ReconcileInput,
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

// ---------------------------------------------------------------------------
// Oversize group guard (review fix, Task 4): mirrors identity_linkage.rs's
// oversize_component_guard_rejects / component_at_exactly_cap_is_accepted for
// cairn_recompute_component, one level down over medication threads. A component
// larger than the cap is a matcher-pathology signature (mass false-merge); on LOCAL
// authoring cairn_recompute_medication_group must RAISE and refuse wholesale rather
// than silently truncate the group. The remote clamp-and-flag branch (apply-door,
// cairn.remote_apply='on') has no medication apply-door test harness yet in slice 3;
// see the filed follow-up issue for that branch.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oversize_group_over_cap_is_refused() {
    // With a tiny cap, the reconcile that would grow the connected component past it
    // is refused wholesale on LOCAL authoring (fail-loud, never a silent cap/truncate).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    c.batch_execute("SET cairn.max_medication_group_size = 3")
        .await
        .unwrap();
    let patient = Uuid::now_v7();
    let (a, b, cc, d) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap(); // {a,b} size 2 — ok
    reconcile_medications(&c, &sk, &kid, "test-node", patient, b, cc, &input)
        .await
        .unwrap(); // {a,b,c} size 3 == cap — ok
    let err = reconcile_medications(&c, &sk, &kid, "test-node", patient, cc, d, &input)
        .await
        .unwrap_err(); // {a,b,c,d} size 4 > cap — refused
    let msg = format!("{err:#}");
    assert!(
        msg.contains("exceeds max size") && msg.contains("matcher pathology"),
        "oversize medication group must be refused with a legible reason: {msg}"
    );
    // The refused edge must not have landed: c/d stay collapsed to themselves and
    // a/b/cc keep their pre-refusal group (proves the RAISE rolled back the whole txn,
    // not just the projection recompute).
    let expected_abc = std::cmp::min(a, std::cmp::min(b, cc));
    assert_eq!(group_of(&c, a).await, expected_abc);
    assert_eq!(group_of(&c, cc).await, expected_abc);
    assert_eq!(group_of(&c, d).await, d, "d never joined the group");
}

#[tokio::test]
async fn oversize_group_at_cap_is_accepted() {
    // The guard is strictly-greater (`> cap`), so a component of exactly `cap` members
    // is accepted. Pins the boundary against a future `>=` regression that would wrongly
    // reject a legitimate at-cap reconciliation.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    c.batch_execute("SET cairn.max_medication_group_size = 3")
        .await
        .unwrap();
    let patient = Uuid::now_v7();
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap(); // {a,b} size 2 — ok
    reconcile_medications(&c, &sk, &kid, "test-node", patient, b, cc, &input)
        .await
        .unwrap(); // {a,b,c} size 3 == cap — accepted
    let expected = std::cmp::min(a, std::cmp::min(b, cc));
    assert_eq!(group_of(&c, a).await, expected);
    assert_eq!(group_of(&c, b).await, expected);
    assert_eq!(
        group_of(&c, cc).await,
        expected,
        "a component of exactly cap members is accepted"
    );
}

/// Rows in patient_medication_current for a patient (medication_id, term, dose).
async fn current_rows(c: &Client, patient: Uuid) -> Vec<(Uuid, String, Option<String>)> {
    c.query(
        "SELECT medication_id::text, term, dose_amount \
         FROM patient_medication_current WHERE patient_id = $1::text::uuid \
         ORDER BY term, medication_id",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1),
            r.get::<_, Option<String>>(2),
        )
    })
    .collect()
}

async fn flag_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn reconcile_collapses_to_one_row_and_clears_flag() {
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
    // Two active same-term threads: flagged, two rows.
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    assert_eq!(flag_count(&c, patient).await, 1);
    // Reconcile: one row, flag clears.
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1, "collapsed to one row");
    assert_eq!(
        rows[0].0,
        std::cmp::min(a, b),
        "keyed by the min-UUID group"
    );
    assert_eq!(
        flag_count(&c, patient).await,
        0,
        "flag cleared without a cessation"
    );
    // Separate: re-splits, flag returns.
    separate_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    assert_eq!(flag_count(&c, patient).await, 1);
}

#[tokio::test]
async fn brand_generic_collapse_without_shared_key() {
    // No shared dup_key (never flagged) — human judgment still collapses them.
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
        &sample_assert("Lipitor"),
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
    assert_eq!(
        flag_count(&c, patient).await,
        0,
        "different terms are never flagged"
    );
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: Some("brand vs generic"),
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    assert_eq!(
        current_rows(&c, patient).await.len(),
        1,
        "collapsed by human judgment"
    );
}

#[tokio::test]
async fn group_current_dose_is_latest_effective_across_members() {
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
    // Thread B gets a later-effective dose change to 80.
    let ch = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    change_dose(&c, &sk, &kid, "test-node", patient, b, &ch)
        .await
        .unwrap();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].2.as_deref(),
        Some("80"),
        "group current dose = latest-effective across members"
    );
}

#[tokio::test]
async fn mixed_status_resolves_latest_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // A active (dose change effective 2025-06); B ceased effective 2024-01 (earlier).
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
    let ch = ChangeDoseInput {
        dose_amount: Some("1000"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: None,
    };
    change_dose(&c, &sk, &kid, "test-node", patient, a, &ch)
        .await
        .unwrap();
    let cease = CeaseMedicationInput {
        stopped: Some("2024-01"),
        stopped_precision: Some("month"),
        reason: None,
    };
    cease_medication(&c, &sk, &kid, "test-node", patient, b, &cease)
        .await
        .unwrap();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    // The later-effective standing statement (A's 2025-06 dose) wins → ACTIVE.
    let rows = current_rows(&c, patient).await;
    assert_eq!(
        rows.len(),
        1,
        "mixed group resolves ACTIVE (later dose beats earlier cessation)"
    );
    assert_eq!(rows[0].2.as_deref(), Some("1000"));

    // Now cease A effective 2026 (later than the dose change) → group flips CEASED.
    let cease_a = CeaseMedicationInput {
        stopped: Some("2026"),
        stopped_precision: Some("year"),
        reason: None,
    };
    cease_medication(&c, &sk, &kid, "test-node", patient, a, &cease_a)
        .await
        .unwrap();
    assert_eq!(
        current_rows(&c, patient).await.len(),
        0,
        "all members ceased → group ceased"
    );
    let past: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_past WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(past, 1, "the ceased group shows as one past row");
}

#[tokio::test]
async fn single_thread_semantics_unchanged() {
    // Regression: a lone active thread and a lone ceased thread render exactly as slices 1/2.
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
        &sample_assert("aspirin"),
    )
    .await
    .unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, a, "un-reconciled thread keys by its own id");
    assert_eq!(rows[0].2.as_deref(), Some("40"), "as-asserted dose shows");
    let cease = CeaseMedicationInput {
        stopped: Some("2025"),
        stopped_precision: Some("year"),
        reason: Some("done"),
    };
    cease_medication(&c, &sk, &kid, "test-node", patient, a, &cease)
        .await
        .unwrap();
    assert_eq!(
        current_rows(&c, patient).await.len(),
        0,
        "ceased → not current (slice-1 semantics)"
    );
    let past: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_past WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(past, 1);
}
