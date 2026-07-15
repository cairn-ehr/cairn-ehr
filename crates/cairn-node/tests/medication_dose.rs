//! §3.15 medication dose overlay (slice 2) — DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard. Patients need no pre-existence (offline-first).
//! Key material is runtime-derived (generate_key), never literal (house rule 6).
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_dose_change_body, build_dose_correction_body, change_dose,
    correct_dose, resolve_correction_target, AssertMedicationInput, ChangeDoseInput,
    CorrectDoseInput,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
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
    let change_evt = change_dose(&mut c, &sk, &kid, "test-node", patient, med_id, &ch, None)
        .await
        .unwrap();

    // A correction of the change we just made (target it explicitly).
    let corr = CorrectDoseInput {
        dose_amount: Some("60"),
        dose_unit: Some("mg"),
        effective: None,
        effective_precision: None,
        reason: None,
        strike: &[],
        note: Some("mis-keyed"),
        info_source: None,
    };
    let target = resolve_correction_target(&c, med_id, Some(change_evt))
        .await
        .unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &corr,
        None,
    )
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
/// The current-dose winner's effective_value for a thread (None if no timeline).
async fn current_effective(c: &Client, med_id: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT effective_value FROM medication_current_dose WHERE medication_id = $1::text::uuid",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .and_then(|r| r.get::<_, Option<String>>(0))
}

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

/// (amount, effective_value, reason) rows of a thread's dose history, effective-ASC.
async fn dose_history(
    c: &Client,
    med_id: Uuid,
) -> Vec<(Option<String>, Option<String>, Option<String>)> {
    c.query(
        "SELECT amount, effective_value, reason FROM patient_medication_dose_history \
         WHERE medication_id = $1::text::uuid \
         ORDER BY cairn_dose_effective_sort_key(effective_value, extract(epoch FROM recorded_at)::bigint*1000) COLLATE \"C\" ASC, dose_event_id",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

#[tokio::test]
async fn assert_seeds_point0_and_it_is_current() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    change_dose(
        &mut c,
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
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // assert dose 40 @2024 (point 0), then a real increase to 80 @2025-06.
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();
    // A later-RECORDED but EARLIER-effective backfill ("was 50 back in 2023").
    change_dose(
        &mut c,
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
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap(); // 40 @2024
               // "they upped it, don't know to what or when" — no effective, no amount.
    change_dose(
        &mut c,
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
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap(); // point 0 = 40 mg, current
               // Correct the CURRENT dose (target defaults to point 0) to 20 mg.
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("mis-keyed"),
            info_source: None,
        },
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap(); // 40 mg
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    // "the 40 was a guess — strike it, unknown."
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: Some("was a guess"),
            info_source: None,
        },
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Pick a target dose_event_id that does not exist locally yet.
    let future_target = Uuid::now_v7();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        future_target,
        &CorrectDoseInput {
            dose_amount: Some("15"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("early correction"),
            info_source: None,
        },
        None,
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
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("25"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("re-corrected"),
            info_source: None,
        },
        None,
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

/// Regression (review Finding 1): a correction that NAMES one thread but `corrects` a
/// dose point belonging to a DIFFERENT thread (a mistargeted --target, or a hostile
/// raw-SQL client) must NOT overlay the wrong thread's dose. `corrects` is a plain uuid
/// the floor cannot bind to a thread offline, so the projection join is thread-scoped
/// (corr.medication_id = de.medication_id) — such a correction is a no-op on every
/// thread's displayed dose while staying auditable in event_log. Without the join fix
/// thread Y below would read the bogus 999.
#[tokio::test]
async fn cross_thread_correction_does_not_overlay_wrong_thread() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Thread Y (the victim): point 0 = 40 mg.
    let med_y = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let y_point = resolve_correction_target(&c, med_y, None).await.unwrap();

    // Thread X (a second, unrelated thread — also 40 mg at point 0).
    let med_x = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();

    // A correction that NAMES thread X but TARGETS thread Y's point 0 (the mistarget).
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_x,   // names thread X ...
        y_point, // ... but `corrects` a point of thread Y
        &CorrectDoseInput {
            dose_amount: Some("999"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("mistargeted"),
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();

    // Y is UNCHANGED (still the asserted 40, not the bogus 999) and not flagged corrected.
    let (amt_y, _u, _de, corrected_y) = current_dose(&c, med_y).await;
    assert_eq!(
        amt_y.as_deref(),
        Some("40"),
        "cross-thread correction must NOT overlay thread Y (would be 999 without the fix)"
    );
    assert!(!corrected_y, "thread Y must not be flagged corrected");

    // X (which the correction NAMED) is likewise unaffected: `corrects` points at a
    // dose_event that is not in X's timeline, so nothing overlays X either.
    let (amt_x, _u, _de, corrected_x) = current_dose(&c, med_x).await;
    assert_eq!(amt_x.as_deref(), Some("40"));
    assert!(!corrected_x);
}

/// Coverage (review Finding 2): correcting a NON-current (older) dose point must leave
/// the current dose untouched — a correction is scoped to its target point, not to "the
/// thread's current value". Proves the older point IS corrected (not silently dropped)
/// while the current point stays put.
#[tokio::test]
async fn correcting_older_point_leaves_current_unchanged() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Point 0 = 40 mg @2024 (from sample_assert), then a change to 80 mg @2025-06 (current).
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();

    // The OLDER point (point 0, is_initial) — NOT the current 2025-06 change point.
    let old_point: Uuid = c
        .query_one(
            "SELECT dose_event_id::text FROM medication_dose_event \
             WHERE medication_id = $1::text::uuid AND is_initial",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get::<_, String>(0)
        .parse()
        .unwrap();

    // Correct the 2024 point 0 from 40 → 45. The current (2025-06) point is untouched.
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        old_point,
        &CorrectDoseInput {
            dose_amount: Some("45"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("point-0 mis-keyed"),
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();

    // Current dose is still the 2025-06 point's 80 mg, and NOT flagged corrected.
    let (amt, _u, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(
        amt.as_deref(),
        Some("80"),
        "correcting the older point must not disturb the current dose"
    );
    assert!(
        !corrected,
        "the current point carries no correction — the correction landed on point 0"
    );

    // The correction DID land on point 0 (not silently dropped): the history trail shows
    // the corrected 45 at the initial point, then 80 at the change.
    assert_eq!(
        history_amounts(&c, med_id).await,
        vec![Some("45".to_string()), Some("80".to_string())],
        "point 0 shows the corrected 45; the current point still shows 80"
    );
}

/// Headline: correcting a point's effective date FORWARD makes a previously-earlier
/// point win as the current dose (winner selection is by effective date, so the fix is
/// bitemporal repair, not a label). Assert (2020) → change to 80mg effective 2025-06 →
/// change to 60mg effective 2024-01. Current = the 2025-06/80mg point. Then correct the
/// 80mg point's effective back to 2023-01: now the 60mg/2024-01 point is the latest → wins.
#[tokio::test]
async fn corrected_effective_flips_current_dose_winner() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let late = change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();
    change_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: Some("60"),
            dose_unit: Some("mg"),
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: None,
        },
        None,
    )
    .await
    .unwrap();

    let (amt0, _u, _de, _c0) = current_dose(&c, med_id).await;
    assert_eq!(
        amt0.as_deref(),
        Some("80"),
        "before correction the 2025-06 point is current"
    );

    // Correct the 80mg point's effective back to 2023-01 (a date-only patch).
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        late,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: Some("2023-01"),
            effective_precision: Some("month"),
            reason: None,
            strike: &[],
            note: Some("mis-keyed the date"),
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();

    let (amt1, _u, _de, _c1) = current_dose(&c, med_id).await;
    assert_eq!(
        amt1.as_deref(),
        Some("60"),
        "after the date fix the 2024-01/60mg point is latest and wins"
    );
    // And the corrected effective is surfaced on the moved point via history.
    assert_eq!(
        current_effective(&c, med_id).await.as_deref(),
        Some("2024-01")
    );
}

/// The floor rejects a no-op correction (touches no group) — under patch semantics a
/// bare correction is meaningless (slice 2's implicit "omit = strike dose" is gone).
#[tokio::test]
async fn floor_rejects_no_op_correction() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();

    let err = correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("must set or strike at least one"),
        "expected no-op floor rejection, got: {err:#}"
    );
}

// Floor: an unknown strike token is rejected legibly (closed group set).
#[tokio::test]
async fn floor_rejects_unknown_strike_token() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    let err = correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["bogus"],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("strike may only contain"),
        "got: {err:#}"
    );
}

// Floor: a group set AND struck in the same correction is a contradiction.
#[tokio::test]
async fn floor_rejects_set_and_struck_same_group() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    let err = correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        target,
        &CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("both set and struck"),
        "got: {err:#}"
    );
}

// Projection: correcting a point's reason surfaces the corrected reason (closes the
// slice-2 dead-column gap) and leaves the dose + effective untouched (per-field keep).
#[tokio::test]
async fn corrected_reason_surfaces_and_other_groups_kept() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let pt = change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        pt,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: Some("dose reduction, not titration"),
            strike: &[],
            note: Some("wrong reason keyed"),
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();
    let hist = dose_history(&c, med_id).await;
    // The corrected point keeps 80mg + 2025-06 but shows the corrected reason.
    assert!(
        hist.iter().any(|(a, e, r)| a.as_deref() == Some("80")
            && e.as_deref() == Some("2025-06")
            && r.as_deref() == Some("dose reduction, not titration")),
        "corrected reason must surface with dose/effective kept, got: {hist:?}"
    );
}

// Projection: strike dose → unknown, while effective/reason on the same point are kept.
#[tokio::test]
async fn strike_dose_reads_unknown_others_kept() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let pt = change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        pt,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: Some("was a guess"),
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();
    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt, None, "struck dose reads unknown");
    assert_eq!(unit, None);
    assert!(corrected);
    assert_eq!(
        current_effective(&c, med_id).await.as_deref(),
        Some("2025-06"),
        "effective kept"
    );
}

// Convergence: a later (higher-HLC) correction of the SAME point supersedes the earlier
// one WHOLESALE (documented boundary — not field-merged).
#[tokio::test]
async fn later_correction_supersedes_earlier_wholesale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let pt = change_dose(
        &mut c,
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
        None,
    )
    .await
    .unwrap();
    // Correction A: fix the effective only.
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        pt,
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();
    // Correction B (later HLC): fix the dose only — supersedes A wholesale.
    correct_dose(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        pt,
        &CorrectDoseInput {
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        },
        None,
    )
    .await
    .unwrap();
    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("40"), "B's dose wins");
    assert_eq!(
        current_effective(&c, med_id).await.as_deref(),
        Some("2025-06"),
        "B did not touch effective → reverts to the original (wholesale supersede, documented boundary)"
    );
}

// Backfill: a pre-035-shaped row (flags NULL, reason = correction-why) is normalized to
// dose_corrected=TRUE / note=reason / reason=NULL, and the backfill is idempotent.
#[tokio::test]
async fn backfill_normalizes_legacy_row_idempotently() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN IF to_regclass('public.medication_dose_correction') IS NOT NULL \
           THEN TRUNCATE medication_dose_correction; END IF; END $$;",
    )
    .await
    .unwrap();
    // Simulate a legacy row: value columns set, all touched-flags NULL, reason=why.
    c.execute(
        "INSERT INTO medication_dose_correction \
           (corrected_dose_event_id, medication_id, patient_id, amount, unit, reason, \
            dose_corrected, effective_corrected, reason_corrected, \
            hlc_wall, hlc_counter, origin, content_address) \
         VALUES (gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), '20', 'mg', 'mis-keyed', \
            NULL, NULL, NULL, 1, 0, 'legacy', '\\x00')",
        &[],
    )
    .await
    .unwrap();
    let backfill = "UPDATE medication_dose_correction \
        SET dose_corrected = TRUE, effective_corrected = FALSE, reason_corrected = FALSE, \
            note = reason, reason = NULL \
        WHERE dose_corrected IS NULL";
    c.execute(backfill, &[]).await.unwrap();
    let row = c
        .query_one(
            "SELECT dose_corrected, effective_corrected, reason_corrected, note, reason \
         FROM medication_dose_correction",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, bool>(0), true);
    assert_eq!(row.get::<_, bool>(1), false);
    assert_eq!(row.get::<_, bool>(2), false);
    assert_eq!(
        row.get::<_, Option<String>>(3).as_deref(),
        Some("mis-keyed")
    );
    assert_eq!(row.get::<_, Option<String>>(4), None);
    // Idempotent: a second run touches nothing (all flags now non-NULL).
    let n = c.execute(backfill, &[]).await.unwrap();
    assert_eq!(n, 0, "backfill must be idempotent");
}

/// Floor-hardening (Task 3 review finding): the dose-CHANGE branch of
/// `cairn_check_medication_dose` requires `jsonb_typeof(reason) = 'string'`, but the
/// dose-CORRECTION branch's set-reason guard only checked non-emptiness after `->>` —
/// so a raw-SQL client (bypassing the Rust builder entirely, which only ever offers a
/// `&str`) could submit `"reason": {...}` on a correction. `->>` on a jsonb object
/// returns its stringified text (non-null, non-empty), so the old guard let it through
/// and the object's JSON text landed verbatim in the `reason` text column — a floor
/// gap, since principle 12 requires the in-DB floor to be the complete defense, not
/// just the Rust path. This builds a well-formed correction via the same builder the
/// orchestrator uses, then hand-mutates `payload.reason` to a JSON object before
/// signing — the exact raw-SQL-client bypass shape used by the sibling
/// `floor_rejects_empty_dose_object_noop` test above.
#[tokio::test]
async fn floor_rejects_non_string_correction_reason() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &sample_assert(),
        None,
    )
    .await
    .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();

    let input = CorrectDoseInput {
        dose_amount: None,
        dose_unit: None,
        effective: None,
        effective_precision: None,
        reason: Some("placeholder — overwritten below"),
        strike: &[],
        note: None,
        info_source: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let mut body: EventBody =
        build_dose_correction_body(Uuid::now_v7(), med_id, patient, target, &input, &kid, hlc);
    // The raw-client bypass: swap the well-formed string reason for a JSON object.
    body.payload
        .as_object_mut()
        .unwrap()
        .insert("reason".into(), serde_json::json!({"foo": "bar"}));
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("a set reason must be a non-empty string"),
        "a non-string reason must be rejected by the floor, got: {err}"
    );
}
