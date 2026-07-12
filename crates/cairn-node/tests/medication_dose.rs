//! §3.15 medication dose overlay (slice 2) — DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard. Patients need no pre-existence (offline-first).
//! Key material is runtime-derived (generate_key), never literal (house rule 6).
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_dose_change_body, change_dose, correct_dose,
    resolve_correction_target, AssertMedicationInput, ChangeDoseInput, CorrectDoseInput,
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

fn sample_assert() -> AssertMedicationInput<'static> {
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

#[tokio::test]
async fn floor_rejects_dose_change_without_info_source() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Hand-build a dose-change with a blank info_source; submit directly.
    let input = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "   ",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_dose_change_body(Uuid::now_v7(), med_id, patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("info_source"), "got: {err}");
}

#[tokio::test]
async fn floor_rejects_empty_dose_change_noop() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // info_source present but no dose / effective / reason → a pure no-op.
    let input = ChangeDoseInput {
        dose_amount: None,
        dose_unit: None,
        effective: None,
        effective_precision: None,
        info_source: "clinician-observed",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_dose_change_body(Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("must carry a dose"), "got: {err}");
}

#[tokio::test]
async fn floor_rejects_empty_dose_object_noop() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // A raw-SQL client could submit `{"dose":{}}` — present key, empty content.
    // The floor's no-op guard must reject on CONTENT, not mere key-presence.
    let input = ChangeDoseInput {
        dose_amount: None,
        dose_unit: None,
        effective: None,
        effective_precision: None,
        info_source: "clinician-observed",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let mut body: EventBody =
        build_dose_change_body(Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    body.payload
        .as_object_mut()
        .unwrap()
        .insert("dose".into(), serde_json::json!({})); // empty dose object — the raw-client bypass
                                                       // re-render the twin is unnecessary; submit as-is
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("must carry a dose"),
        "empty dose object must be rejected, got: {err}"
    );
}

#[tokio::test]
async fn floor_accepts_wellformed_change_and_correction_into_log() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();

    let ch = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    let change_evt = change_dose(&c, &sk, &kid, "test-node", patient, med_id, &ch)
        .await
        .unwrap();

    // A correction of the change we just made (target it explicitly).
    let corr = CorrectDoseInput {
        dose_amount: Some("60"),
        dose_unit: Some("mg"),
        info_source: None,
        reason: Some("mis-keyed"),
    };
    let target = resolve_correction_target(&c, med_id, Some(change_evt))
        .await
        .unwrap();
    correct_dose(&c, &sk, &kid, "test-node", patient, med_id, target, &corr)
        .await
        .unwrap();

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type LIKE 'clinical.medication-dose-%'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "both dose events landed in the log");
}

// helper: the current dose (amount, unit, dose_event_id, corrected) for a thread.
// dose_amount/dose_unit come from patient_medication_current (proving the reworked
// view shows the TIMELINE dose, not the frozen assert dose); dose_event_id/corrected
// come from the separate medication_current_dose view (patient_medication_current is
// deliberately NOT widened — see the CRITICAL note above). NOTE: this project's
// tokio-postgres has NO uuid FromSql — a uuid column must be SELECTed as ::text and
// parsed (see crates/cairn-node/tests/apply_proposal.rs:61).
async fn current_dose(c: &Client, med_id: Uuid) -> (Option<String>, Option<String>, Uuid, bool) {
    let r = c
        .query_one(
            "SELECT pmc.dose_amount, pmc.dose_unit, mcd.dose_event_id::text, mcd.corrected \
             FROM patient_medication_current pmc \
             JOIN medication_current_dose mcd USING (medication_id) \
             WHERE pmc.medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    (
        r.get::<_, Option<String>>(0),
        r.get::<_, Option<String>>(1),
        r.get::<_, String>(2).parse::<Uuid>().unwrap(),
        r.get::<_, bool>(3),
    )
}

async fn history_amounts(c: &Client, med_id: Uuid) -> Vec<Option<String>> {
    c.query(
        "SELECT amount FROM patient_medication_dose_history \
         WHERE medication_id = $1::text::uuid ORDER BY recorded_at, dose_event_id",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, Option<String>>(0))
    .collect()
}

#[tokio::test]
async fn assert_seeds_point0_and_it_is_current() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("40"));
    assert_eq!(unit.as_deref(), Some("mg"));
    assert!(!corrected);
    // history has exactly the initial point.
    assert_eq!(
        history_amounts(&c, med_id).await,
        vec![Some("40".to_string())]
    );
}

#[tokio::test]
async fn change_moves_current_and_keeps_history() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    change_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        },
    )
    .await
    .unwrap();

    let (amt, _u, _de, _corr) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("80"), "latest effective is current");
    // Both points present, chronological (40 @2024, then 80 @2025-06).
    assert_eq!(
        history_amounts(&c, med_id).await,
        vec![Some("40".to_string()), Some("80".to_string())]
    );
}

#[tokio::test]
async fn backdated_change_does_not_override_later_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // assert dose 40 @2024 (point 0), then a real increase to 80 @2025-06.
    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    change_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: None,
        },
    )
    .await
    .unwrap();
    // A later-RECORDED but EARLIER-effective backfill ("was 50 back in 2023").
    change_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: Some("50"),
            dose_unit: Some("mg"),
            effective: Some("2023"),
            effective_precision: Some("year"),
            info_source: "patient-reported",
            reason: Some("historical backfill"),
        },
    )
    .await
    .unwrap();

    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(
        amt.as_deref(),
        Some("80"),
        "latest EFFECTIVE (2025-06) stays current, not the last recorded"
    );
}

#[tokio::test]
async fn undated_change_becomes_current_over_older_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // 40 @2024
                   // "they upped it, don't know to what or when" — no effective, no amount.
    change_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            info_source: "patient-reported",
            reason: Some("patient says increased"),
        },
    )
    .await
    .unwrap();

    // The undated change's effective key derives from its (later) recording time, so it
    // wins over the 2024 point. Its amount is unknown (NULL) — honestly current.
    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(
        amt, None,
        "current dose is honestly unknown after an unquantified increase"
    );
}

#[tokio::test]
async fn correction_overlays_current_and_sets_flag() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // point 0 = 40 mg, current
                   // Correct the CURRENT dose (target defaults to point 0) to 20 mg.
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("mis-keyed"),
        },
    )
    .await
    .unwrap();

    let (amt, _u, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(
        amt.as_deref(),
        Some("20"),
        "current dose reflects the correction"
    );
    assert!(corrected, "corrected flag is set");
}

#[tokio::test]
async fn correct_to_unknown_shows_unknown_not_original() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // 40 mg
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    // "the 40 was a guess — strike it, unknown."
    correct_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            info_source: None,
            reason: Some("was a guess"),
        },
    )
    .await
    .unwrap();

    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(
        amt, None,
        "correct-to-unknown must NOT fall back to the original 40"
    );
    assert_eq!(unit, None);
    assert!(corrected);
}

#[tokio::test]
async fn orphan_correction_converges_when_target_arrives() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Pick a target dose_event_id that does not exist locally yet.
    let future_target = Uuid::now_v7();
    correct_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        future_target,
        &CorrectDoseInput {
            dose_amount: Some("15"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("early correction"),
        },
    )
    .await
    .unwrap();
    // The correction row exists but no dose point references it yet → no current row.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_current_dose WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "orphan correction renders nothing until its target arrives"
    );

    // Now inject the assert whose event_id == future_target (build+sign directly to
    // choose the event_id), seeding point 0 that the correction targets.
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = cairn_node::medication::build_assert_body(
        future_target,
        med_id,
        patient,
        &sample_assert(),
        &kid,
        hlc,
    );
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    let (amt, _u, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(
        amt.as_deref(),
        Some("15"),
        "the pre-arrived correction now overlays point 0"
    );
    assert!(corrected);
}

#[tokio::test]
async fn later_correction_of_same_point_wins() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: None,
        },
    )
    .await
    .unwrap();
    correct_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("25"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("re-corrected"),
        },
    )
    .await
    .unwrap();

    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(
        amt.as_deref(),
        Some("25"),
        "the later (higher-HLC) correction of the same point wins"
    );
}
