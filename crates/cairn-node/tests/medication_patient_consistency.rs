//! Issue #192 (2026-07-15 review, finding A4) + the #177 design decision —
//! medication threads belong to ONE chart for life.
//!
//! Scenario A (rebind): `medication_statement`'s PK is `medication_id` alone and its
//! overlay does `patient_id = EXCLUDED.patient_id`, so a buggy or hostile client
//! re-asserting an existing `medication_id` under another patient silently re-homed the
//! whole thread — including every accumulated dose point, which joins by
//! `medication_id` — onto the other chart, convergently, on every node, unflagged.
//!
//! Scenario B (#177): nothing required a reconciliation's two subject threads to belong
//! to the same patient, so a cross-patient group made `medication_group_status` emit one
//! row per (group, patient) and `patient_medication_current` showed mixed attribution.
//!
//! The fix follows the `chart_dispute` subject-consistency pattern (db/023): FAIL LOUD
//! at the LOCAL door (nothing accepted yet — catch the caller bug at source, lose no
//! data), CONVERGE-AND-FLAG on the sync-apply path (peers already hold the signed
//! event; a node-local veto would fork the event set — the flag surfaces the
//! contradiction for humans instead). Offline-first is preserved: a thread whose
//! standing patient is UNKNOWN locally passes the local door, and the cross-patient
//! read-time view catches the late-arriving contradiction whichever order it lands in.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_assert_body, build_dose_change_body, build_dose_correction_body,
    cease_medication, reconcile_medications, separate_medications, AssertMedicationInput,
    CeaseMedicationInput, ChangeDoseInput, CorrectDoseInput, ReconcileInput,
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

/// ADR-0052: seal a CLEAR clinical EventBody like the node write path, register the
/// node's unwrap key, sign, and submit through the 4-arg strict door. Returns the raw
/// driver Result. House rule 6: the DEK is generated inside seal_event_payload, never a
/// literal.
async fn seal_and_submit(
    c: &Client,
    sk: &SigningKey,
    mut body: EventBody,
) -> Result<u64, tokio_postgres::Error> {
    let twin = body
        .plaintext_twin
        .take()
        .expect("a clinical body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal the clear payload+twin");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk).expect("sign the sealed body");
    let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await?;
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
}

/// Truncate the log + every medication projection + the ADR-0052 custody plane and
/// enroll a fresh device actor (custody tables named explicitly — no FK, no CASCADE).
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
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN TRUNCATE medication_dose_correction; END IF; \
           IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN TRUNCATE medication_reconciliation; END IF; \
           IF to_regclass('public.medication_group_member') IS NOT NULL THEN TRUNCATE medication_group_member; END IF; \
           IF to_regclass('public.medication_projection_flag') IS NOT NULL THEN TRUNCATE medication_projection_flag; END IF; \
           IF to_regclass('public.medication_patient_conflict_flag') IS NOT NULL THEN TRUNCATE medication_patient_conflict_flag; END IF; \
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

/// Sign a re-assert of an EXISTING medication_id under a chosen patient and submit it
/// at the chosen door ('submit_event' or 'apply_remote_event').
async fn reassert_at_door(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    medication_id: Uuid,
    patient: Uuid,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = build_assert_body(
        Uuid::now_v7(),
        medication_id,
        patient,
        &sample_assert("metoprolol"),
        kid,
        hlc,
    );
    match door {
        // The STRICT door (submit_event) demands a born-sealed clinical body (ADR-0052):
        // seal it and pass the DEK as the 4th arg. The #192 patient-consistency guard
        // fires in the projection, on the CLEAR payload the door unseals — fail-loud
        // locally, so a wrong-patient reassert is still refused here.
        "submit_event" => seal_and_submit(c, sk, body).await,
        // The APPLY door (apply_remote_event) stays lenient (set-union) and has no
        // born-sealed floor yet (Tasks 8/9), so a replicated event still arrives plaintext.
        _ => {
            let signed = sign(&body, sk).unwrap();
            c.execute(&format!("SELECT {door}($1)")[..], &[&signed.signed_bytes])
                .await
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario A — the rebind hazard on the assert/cease/dose paths.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_reassert_under_different_patient_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    let r = reassert_at_door(&c, &sk, &kid, "submit_event", med, patient_b).await;
    assert!(
        r.is_err(),
        "re-asserting an existing thread under another patient must be refused locally"
    );
    let m = db_msg(&r.unwrap_err());
    assert!(
        m.contains("patient"),
        "the refusal must name the patient-consistency contract, got: {m}"
    );

    // The thread is untouched — still patient A's.
    let p: String = c
        .query_one(
            "SELECT patient_id::text FROM medication_statement WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        p,
        patient_a.to_string(),
        "the standing thread must be unchanged"
    );
}

#[tokio::test]
async fn local_cessation_under_different_patient_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    let r = cease_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_b,
        med,
        &CeaseMedicationInput {
            stopped: Some("2025"),
            stopped_precision: Some("year"),
            reason: Some("wrong chart"),
        },
        None,
        None,
    )
    .await;
    assert!(
        r.is_err(),
        "ceasing another patient's thread must be refused locally"
    );
}

#[tokio::test]
async fn local_dose_change_under_different_patient_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    let input = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body = build_dose_change_body(Uuid::now_v7(), med, patient_b, &input, &kid, hlc);
    let r = seal_and_submit(&c, &sk, body).await;
    assert!(
        r.is_err(),
        "a dose change naming another patient for the thread must be refused locally"
    );
}

/// Offline-first must survive the guard: an orphan cessation (no local statement)
/// still passes the local door — the standing patient is honestly unknown.
#[tokio::test]
async fn local_orphan_cessation_still_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let unknown_thread = Uuid::now_v7();
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        unknown_thread,
        &CeaseMedicationInput {
            stopped: Some("2025"),
            stopped_precision: Some("year"),
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("an orphan cessation must still be accepted (offline-first)");
}

/// The sync-apply path must NOT raise (a node-local veto of a validly-signed event
/// peers already hold would fork the event set): the thread converges by HLC — and the
/// contradiction is FLAGGED for humans (the 'unflagged' half of finding A4).
#[tokio::test]
async fn remote_reassert_converges_and_flags() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    reassert_at_door(&c, &sk, &kid, "apply_remote_event", med, patient_b)
        .await
        .expect("the apply door must admit the replicated event (converge, never fork)");

    // Converged to the HLC-later claim...
    let p: String = c
        .query_one(
            "SELECT patient_id::text FROM medication_statement WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        p,
        patient_b.to_string(),
        "sync must converge to the HLC winner"
    );

    // ...and the contradiction is on the advisory worklist.
    let flags: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "the cross-patient rebind must be flagged, never silent"
    );
}

// ---------------------------------------------------------------------------
// Scenario B (#177) — cross-patient reconciliation.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_cross_patient_reconcile_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let m1 = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();
    let m2 = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_b,
        &sample_assert("betaloc"),
        None,
        None,
    )
    .await
    .unwrap();

    let r = reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        m1,
        m2,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: Some("looks same"),
        },
        None,
        None,
    )
    .await;
    assert!(
        r.is_err(),
        "reconciling two threads with KNOWN different patients must be refused locally (#177)"
    );
}

/// Offline-first: when the subjects' patients are unknown at submit time the local door
/// passes (never fabricate certainty) — and the standing cross-patient group is then
/// surfaced by the read-time view once the statements land, whatever the arrival order.
#[tokio::test]
async fn cross_patient_group_surfaced_by_view() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let m1 = Uuid::now_v7();
    let m2 = Uuid::now_v7();

    // Reconcile FIRST (both threads unknown locally — passes the local door honestly).
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        m1,
        m2,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("offline-first: unknown subjects must pass the local door");

    // The statements arrive afterwards, on different charts.
    reassert_at_door(&c, &sk, &kid, "apply_remote_event", m1, patient_a)
        .await
        .unwrap();
    reassert_at_door(&c, &sk, &kid, "apply_remote_event", m2, patient_b)
        .await
        .unwrap();

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_group_cross_patient WHERE group_id = $1::text::uuid",
            &[&std::cmp::min(m1, m2).to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "a standing cross-patient group must be surfaced by the advisory view"
    );
}

/// Finding 3 (this-PR review) — the cross-patient view must derive a thread's patient
/// from the SAME source as the write-guard (`cairn_medication_thread_patient`: statement
/// OR orphan cessation), not statements alone. A thread known locally only via an orphan
/// cessation contributes a real patient the guard sees, so a group spanning it is a real
/// cross-patient hazard — but a statement-only join makes it invisible on exactly the
/// read-time surface meant to catch the late-arriving case.
#[tokio::test]
async fn cross_patient_group_via_cessation_only_thread_surfaced() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let m1 = Uuid::now_v7();
    let m2 = Uuid::now_v7();

    // Reconcile FIRST (both threads unknown locally — passes the local door honestly).
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        m1,
        m2,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("offline-first: unknown subjects must pass the local door");

    // m1 lands as a STATEMENT on chart A; m2 is known only via an ORPHAN CESSATION on
    // chart B (no statement ever synced) — exactly what the guard reads through the
    // cessation branch of cairn_medication_thread_patient.
    reassert_at_door(&c, &sk, &kid, "apply_remote_event", m1, patient_a)
        .await
        .unwrap();
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_b,
        m2,
        &CeaseMedicationInput {
            stopped: Some("2025"),
            stopped_precision: Some("year"),
            reason: Some("stopped elsewhere"),
        },
        None,
        None,
    )
    .await
    .expect("an orphan cessation is accepted offline-first");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_group_cross_patient WHERE group_id = $1::text::uuid",
            &[&std::cmp::min(m1, m2).to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "a cross-patient group whose second thread is known only via a cessation must still surface"
    );
}

/// Separation is the REPAIR primitive for a bad cross-patient link — it must never be
/// blocked by the very inconsistency it exists to fix.
#[tokio::test]
async fn separation_of_cross_patient_group_still_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let m1 = Uuid::now_v7();
    let m2 = Uuid::now_v7();
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        m1,
        m2,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    reassert_at_door(&c, &sk, &kid, "apply_remote_event", m1, patient_a)
        .await
        .unwrap();
    reassert_at_door(&c, &sk, &kid, "apply_remote_event", m2, patient_b)
        .await
        .unwrap();

    separate_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        m1,
        m2,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: Some("wrong chart — undoing"),
        },
        None,
        None,
    )
    .await
    .expect("separation (the repair) must always pass, even on a cross-patient group");

    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_group_cross_patient", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "after separation the cross-patient flag view is clear"
    );
}

// ---------------------------------------------------------------------------
// #273 — the dose-CORRECTION verb must carry the same #192 guard as the other
// verbs. db/035's redefinition of medication_dose_correction_apply (file replay
// order) shadowed the guard call #192 added to db/032's body, so a wrong-chart
// correction was admitted silently — on the one verb whose corrected value
// drives current-dose winner selection (ADR-0050).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn local_dose_correction_under_different_patient_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    // A correction targeting SOME dose event of A's thread, but stamped with B's
    // chart (the classic wrong-chart-open slip). The target may be any UUID —
    // corrections are offline-first (the target need not exist locally), so the
    // guard must bind on the THREAD's standing patient, not on target existence.
    let input = CorrectDoseInput {
        dose_amount: Some("60"),
        dose_unit: Some("mg"),
        effective: None,
        effective_precision: None,
        reason: None,
        strike: &[],
        note: Some("mis-keyed"),
        info_source: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body = build_dose_correction_body(
        Uuid::now_v7(),
        med,
        patient_b,
        Uuid::now_v7(),
        &input,
        &kid,
        hlc,
    );
    let r = seal_and_submit(&c, &sk, body).await;
    let err = match r {
        Err(e) => e,
        Ok(_) => panic!(
            "a dose correction naming another patient for the thread must be refused locally (#273)"
        ),
    };
    // The refusal must be the #192 contract, not an incidental failure.
    let msg = db_msg(&err);
    assert!(
        msg.contains("patient cannot change"),
        "unexpected refusal: {msg}"
    );
}

#[tokio::test]
async fn remote_dose_correction_converges_and_flags() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient_a,
        &sample_assert("metoprolol"),
        None,
        None,
    )
    .await
    .unwrap();

    // The same wrong-chart correction arriving over sync: the apply door stays
    // lenient (set-union — peers already hold the signed event), so it must be
    // ADMITTED, and the contradiction must land on the advisory worklist.
    let input = CorrectDoseInput {
        dose_amount: Some("60"),
        dose_unit: Some("mg"),
        effective: None,
        effective_precision: None,
        reason: None,
        strike: &[],
        note: Some("mis-keyed"),
        info_source: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body = build_dose_correction_body(
        Uuid::now_v7(),
        med,
        patient_b,
        Uuid::now_v7(),
        &input,
        &kid,
        hlc,
    );
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the apply door must admit the replicated correction (converge, never fork)");

    // Lossless convergence: the correction row still lands...
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_dose_correction WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the correction must still converge");

    // ...and the cross-patient contradiction is flagged, never silent.
    let flags: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "the cross-patient correction must be flagged, never silent (#273)"
    );
}
