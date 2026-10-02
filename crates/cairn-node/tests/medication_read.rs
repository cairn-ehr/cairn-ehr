//! The first clinical READ path (#288 med-list slice): `list_patient_medications` over the
//! existing medication projections.
//!
//! The group/thread asymmetry is what these tests exist for. `patient_medication_current`
//! emits one row per GROUP (reconciled duplicates collapse, ADR-0047) while attestation is
//! per THREAD (ADR-0049). Every test below pins one way that asymmetry can be got wrong.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Key
//! material is minted at runtime (house rule 6).
//!
//! GUARD-BEFORE-CONNECT. The BRIEF's own snippet (Step 3) called `connect_and_load_schema`
//! THEN `test_serial_guard` — that order is backwards. This directory's prevailing
//! convention is the opposite: `test_serial_guard` first, `connect_and_load_schema`
//! second (verified directly: every DB-gated test function in `crates/cairn-node/tests/`
//! that calls both does so guard-first — see `medication_attestation.rs`,
//! `medication_reconciliation.rs`, `identity_dispute.rs`, and dozens more). This file
//! follows that prevailing convention, not a novel one.
//!
//! Why it matters here, concretely: with the brief's connect-then-guard order, every test
//! in this file deadlocked reliably (100% of runs in isolation — `ERROR: deadlock
//! detected`, e.g. relation 64708512 waiting on 64708602 while that session waited on the
//! first). Root cause: `list_patient_medications` reads across nearly every medication
//! view in one call (the whole point of this slice), so its lock footprint spans most of
//! the medication schema; a sibling test's concurrent, unguarded `connect_and_load_schema`
//! (which replays db/031-035's DDL, each statement taking AccessExclusiveLock even when a
//! no-op) can acquire two of those relations' locks in the opposite order, and Postgres
//! detects the cycle. Following the prevailing guard-first convention — acquiring the
//! guard before connecting, so each test's schema load is ALSO serialized against its
//! siblings — closes the window: 4/4 clean runs after the fix (and measurably faster —
//! no more deadlock-abort-retry). This slice's wide reads simply exercise the ordering
//! requirement harder than any narrow single/double-table write in this directory has
//! before.
mod common;

use cairn_event::SigningKey;
use cairn_medication_view::{withheld_group_ids, MedicationStatus, VouchState};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, reconcile_medications,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ReconcileInput, SubstanceCoding,
};
use common::{attestation_count, cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// A uuid list in ascending order — the order `read.rs` returns member threads in, so an
/// expectation can be written from the ids the test minted without depending on which of
/// them happens to sort lower.
fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

/// Assert one medication and return its thread id.
async fn assert_one(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        origin,
        patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

/// Assert one medication with a drug-identity coding (ADR-0059), and return its thread
/// id. Used only by the coding-conflict test (finding 3b): two threads later reconciled
/// together but coded to two different anchors.
async fn assert_coded(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
    code: &str,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        origin,
        patient,
        &AssertMedicationInput {
            term,
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code,
                display: term,
            }),
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_single_unvouched_medication_reads_as_absent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(rows.len(), 1, "one assert, one displayed row");
    assert_eq!(rows[0].term, "metformin");
    assert_eq!(rows[0].status, MedicationStatus::Active);
    assert_eq!(rows[0].members.len(), 1);
    assert_eq!(rows[0].members[0].medication_id, thread);
    assert_eq!(rows[0].members[0].vouch, VouchState::Absent);
    // Negative case for the two advisory flags (finding 3): a single, un-duplicated,
    // un-reconciled, uncoded assert must read as clean on both — otherwise a bug that
    // returns an always-full flag set would go undetected by every other test here, which
    // only ever exercises the positive case.
    assert!(!rows[0].reconciliation_flagged);
    assert!(!rows[0].coding_conflict);
}

#[tokio::test]
async fn an_attested_thread_reads_as_fresh_with_its_attester() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, patient, thread)
        .await
        .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(
        rows[0].members[0].vouch,
        VouchState::Fresh { by: hkid.clone() }
    );
}

/// A reconciled pair is ONE row over TWO member threads — the group/thread asymmetry.
#[tokio::test]
async fn a_reconciled_pair_reads_as_one_row_with_two_members() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "Metformin XR").await;
    // DEVIATION FROM THE BRIEF: the brief's `ReconcileInput { patient, thread_a, thread_b,
    // note }` and a 3-positional-arg `reconcile_medications` do not match the orchestrator
    // that Task 1's review cycle actually landed (`crates/cairn-node/src/medication/
    // reconciliation.rs`). The real shapes are `ReconcileInput { provenance, reason }` and
    // `reconcile_medications(client, node_sk, node_kid, node_origin, patient, subject_a,
    // subject_b, input, author, attest)` — patient and the two subject threads are separate
    // positional arguments, not struct fields. See task-2-report.md for detail.
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        a,
        b,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(
        rows.len(),
        1,
        "a reconciled pair collapses to ONE displayed row"
    );
    let mut members: Vec<Uuid> = rows[0].members.iter().map(|m| m.medication_id).collect();
    members.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(members, expected, "both threads are members of the one row");
}

/// A ceased medication stays VISIBLE, marked ceased — a struck line on a paper chart is
/// not erased (refinement 2 of the plan).
#[tokio::test]
async fn a_ceased_medication_is_retained_and_marked_ceased() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        thread,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("rash"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(rows.len(), 1, "a ceased drug is still on the chart");
    assert_eq!(rows[0].status, MedicationStatus::Ceased);
}

#[tokio::test]
async fn another_patients_medications_are_not_returned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let mine = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, mine, 0).await;
    let theirs = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, theirs, 0).await;

    assert_one(&mut c, &sk, &kid, "origin-a", theirs, "warfarin").await;

    assert!(list_patient_medications(&c, mine)
        .await
        .unwrap()
        .rows
        .is_empty());
}

#[tokio::test]
async fn a_patient_with_no_medications_reads_as_an_empty_list() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _ = setup(&c).await;

    assert!(list_patient_medications(&c, Uuid::now_v7())
        .await
        .unwrap()
        .rows
        .is_empty());
}

/// Finding 1 (review round 1): the single most safety-critical branch in `read.rs` —
/// `VouchState::Stale` — had zero coverage. A stale vouch rendering as fresh is a signed
/// claim the drug was reviewed when it was not, so this pins the database's `stale = true`
/// path end to end rather than trusting the mapping by inspection alone. Growing the
/// thread AFTER attesting it (a cessation event is one of the four content types
/// `cairn_medication_thread_commitment` folds in, db/034) is what makes the recomputed
/// commitment stop matching the vouch's `reviewed_commitment`.
#[tokio::test]
async fn a_thread_grown_after_attestation_reads_as_stale() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, patient, thread)
        .await
        .unwrap();

    // Grow the thread's content AFTER the attestation vouched for it.
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        thread,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("rash"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(
        rows.len(),
        1,
        "the (now-ceased) thread is still the one displayed row"
    );
    assert_eq!(
        rows[0].members[0].vouch,
        VouchState::Stale { by: hkid.clone() },
        "the thread grew after attestation — the vouch must read stale, never fresh"
    );
}

/// Finding 3a (review round 1): `read_reconciliation_flagged_groups` was wired but never
/// exercised — two un-reconciled threads sharing the same duplicate key (here: the same
/// term, neither coded, so `patient_medication_reconciliation_flag`'s `dup_key` falls back
/// to `term:<normalized>`) must both come back `reconciliation_flagged == true`. Left
/// un-reconciled deliberately: reconciling them is exactly what a positive flag should be
/// prompting a clinician to do.
#[tokio::test]
async fn two_un_reconciled_threads_sharing_a_term_are_flagged_for_reconciliation() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;

    assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(
        rows.len(),
        2,
        "two un-reconciled duplicate asserts stay two separate displayed rows"
    );
    assert!(
        rows.iter().all(|r| r.reconciliation_flagged),
        "both rows share an un-reconciled duplicate key and must both be flagged"
    );
}

/// Finding 3b (review round 1): `read_coding_conflict_groups` was wired but never
/// exercised — a reconciled group whose two members carry two DIFFERENT drug-identity
/// codings (ADR-0059 decision 5, a possible mis-reconciliation) must come back
/// `coding_conflict == true`. Mirrors `medication_coding.rs`'s
/// `two_anchors_in_one_group_raise_a_conflict`, read back through this slice's list view
/// instead of a raw count on the underlying view.
#[tokio::test]
async fn a_reconciled_group_with_conflicting_codings_is_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";
    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";

    let a = assert_coded(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        "Lipitor",
        MOIETY_ATORVASTATIN,
    )
    .await;
    let b = assert_coded(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        "Diabex",
        MOIETY_METFORMIN,
    )
    .await;
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        a,
        b,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("reconciliation is a human judgment — never auto-refused over a coding");

    let rows = list_patient_medications(&c, patient).await.unwrap().rows;
    assert_eq!(rows.len(), 1, "the reconciled pair is one displayed row");
    assert!(
        rows[0].coding_conflict,
        "two different anchors in one reconciled group must be flagged"
    );
}

/// Fix 1 (#288 final review), issue #334 — FIXED by the combined read (ADR-0076 R1). A
/// reconciled group whose member threads span TWO unlinked patients is a standing wrong-chart
/// hazard, and it must be visible AND flagged on BOTH charts.
///
/// WHAT CHANGED. This test used to be `a_cross_patient_group_is_missing_from_the_losing_
/// patients_chart` and pinned the defect itself: the read selected rows by the list view's
/// `patient_id`, which is `medication_group_display`'s single `DISTINCT ON` winner, so the
/// group showed on the winner's chart only and the LOSING patient's chart showed nothing —
/// `groups_missing_from_chart` was how that silent gap was surfaced. The read now selects a
/// group through its own member threads, so B's chart shows the line too. Every assertion
/// about withholding and `separation_targets` is kept, on both sides; the "invisible group"
/// half moved to the pure `missing_groups` test in `read.rs`, where the defensive net still
/// lives (by construction it is now empty unless a view drops a group).
///
/// THE DOOR CANNOT PRODUCE THIS VIA THE EVENT PATH. db/033's reconcile door
/// (`medication_reconciliation_apply`) refuses a reconciliation at LOCAL author time
/// whenever BOTH subject threads' patients are already known locally and differ (db/033
/// lines 260-279) — exactly the state this test needs. It never refuses on the SYNC-APPLY
/// path (`cairn.remote_apply = 'on'`), so a peer node's reconciliation event legitimately
/// produces this state here; this test reproduces that arrival by inserting directly into
/// `medication_group_member`, the same projection table the sync-apply path would write,
/// rather than by asserting an event the local door would refuse.
#[tokio::test]
async fn a_cross_patient_group_shows_on_both_charts_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient_a = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, patient_a, 0).await;
    let patient_b = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, patient_b, 0).await;

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", patient_a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "amlodipine").await;

    // Fold both threads into ONE group, with thread_a as the group id — the same shape
    // `cairn_recompute_medication_group` writes for a real reconciled pair. thread_a being
    // the group id makes patient A the view's display "winner"; the point of the test is
    // that winning no longer decides which chart SHOWS the group.
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&thread_a.to_string(), &thread_b.to_string()],
    )
    .await
    .unwrap();

    // Both charts show the group ONCE — deduplicated (the view emits one row per chart the
    // group touches), carrying the cross-patient warning, naming both source charts, and
    // with the group's full membership as the arguments to `medication-separate` (#338
    // review finding 1: each chart's own row lists only its own thread).
    for (patient, own_thread) in [(patient_a, thread_a), (patient_b, thread_b)] {
        let list = list_patient_medications(&c, patient).await.unwrap();
        assert_eq!(
            list.rows.len(),
            1,
            "the group shows, once, on {patient}'s chart"
        );
        let row = &list.rows[0];
        assert_eq!(row.group_id, thread_a);
        assert!(
            row.cross_patient,
            "{patient}'s row must carry the cross-patient warning"
        );
        assert_eq!(row.source_charts, sorted(vec![patient_a, patient_b]));
        assert_eq!(
            row.members
                .iter()
                .map(|m| m.medication_id)
                .collect::<Vec<_>>(),
            vec![own_thread],
            "a chart's line lists only its own thread — the other is someone else's"
        );
        assert!(
            list.groups_missing_from_chart.is_empty(),
            "#334 fixed: a locally-known group is never missing from a chart it touches"
        );
        assert_eq!(
            list.separation_targets.get(&thread_a),
            Some(&sorted(vec![thread_a, thread_b])),
            "the hazardous group must carry BOTH member threads, including the one belonging \
             to the other patient — they are the arguments to `medication-separate`"
        );
    }

    // Sign-off on EITHER chart withholds the line rather than signing it: the dose on it
    // comes from `medication_group_current_dose`, which picks one member across the whole
    // group regardless of patient, so it may be the other patient's dose under this
    // patient's drug name. Withholding is per LINE and REPORTED, never silent (an empty
    // `attested` would read as "nothing needed doing"), and the report carries the remedy's
    // arguments — the other patient's thread is on no other surface of this chart.
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    for patient in [patient_a, patient_b] {
        let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient, None)
            .await
            .expect("a chart with a hazardous line is reported, never refused (#339)");
        assert!(
            out.attested.is_empty(),
            "a cross-patient line must not be signed: its displayed dose may be another patient's"
        );
        assert_eq!(
            withheld_group_ids(&out.withheld),
            vec![thread_a],
            "the withheld line must be REPORTED"
        );
        assert_eq!(
            out.separation_targets.get(&thread_a),
            Some(&sorted(vec![thread_a, thread_b])),
            "the withheld line must carry both member threads for `medication-separate`"
        );
        assert!(out.groups_missing_from_chart.is_empty());
    }
}

/// THE #339 CONTRACT, and the most important test in this file: **a defect on one line
/// never invalidates another.**
///
/// The clinician's ruling (2026-08-03), in their words: *"there is no reason to refuse the
/// whole chart if one single line is not visible or not trustworthy. What matters is that
/// all visible lines in the chart must be signed … or presented as unsigned in the UI."*
/// The paper counterpart is a drug written up but missing a signature — that prompts the
/// nurse to chase the signature before acting on THAT drug; it does not void the chart.
///
/// The worked case they gave, which is why this is a safety property and not a convenience
/// one: a doctor writes up 1 L normal saline over 4 h and signs it, then writes up a
/// 100 mL minibag with 10 mmol potassium and does NOT sign it. The saline must still be
/// giveable. A system that voids the whole chart because the potassium line is unsigned (or
/// invalid, or invisible) withholds fluid from a patient over a defect in a different line.
///
/// Here patient B has an ordinary, unsigned drug of their own PLUS a cross-patient group.
/// The sound drug must be signed; the hazardous line must be withheld and reported, not
/// used as grounds to refuse. (Was `an_incomplete_chart_still_signs_every_line_it_can_show`;
/// since #334 the cross-patient group SHOWS, flagged.) NOT COVERED: the "invisible group"
/// half. `read.rs`'s pure `missing_groups` test pins DETECTION only; sign-off's handling of
/// a non-empty `groups_missing_from_chart` (report, never refuse, union both reads) has no
/// DB coverage, the state being unreachable by construction now (same class as #333).
#[tokio::test]
async fn a_hazardous_line_never_blocks_a_sound_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient_a = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, patient_a, 0).await;
    let patient_b = Uuid::now_v7();
    // #345: every chart this test writes to exists first.
    common::submit_registration(&c, &sk, &kid, patient_b, 0).await;

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", patient_a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "amlodipine").await;
    // Patient B's OWN, entirely ordinary drug — never reconciled with anything, no hazard
    // of its own, and unsigned. On paper B's clinician would simply sign this line.
    let unrelated = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "warfarin").await;

    // Same peer-arrival shape as the test above: one group spanning A and B, id thread_a.
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&thread_a.to_string(), &thread_b.to_string()],
    )
    .await
    .unwrap();

    // B's chart shows both lines: the sound warfarin, and the hazardous group, flagged.
    let b_list = list_patient_medications(&c, patient_b).await.unwrap();
    let row = |g: Uuid| b_list.rows.iter().find(|r| r.group_id == g).unwrap();
    assert_eq!(
        b_list.rows.len(),
        2,
        "B's own drug and the cross-patient line"
    );
    assert!(
        !row(unrelated).cross_patient,
        "the unrelated drug carries no hazard of its own"
    );
    assert!(row(thread_a).cross_patient, "the shared group is flagged");

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient_b, None)
        .await
        .expect("a chart with a hazardous line must never be refused (#339)");

    // THE PROPERTY: the sound line is signed, despite a hazardous line on the same chart.
    assert_eq!(
        out.attested,
        vec![unrelated],
        "the visible, sound drug must be signed — a defect on one line never invalidates \
         another (#339: the saline goes up even when the potassium is unsigned)"
    );
    assert_eq!(
        attestation_count(&c, unrelated).await,
        1,
        "and the attestation is really committed, not merely reported"
    );

    // AND the hazardous line is still surfaced — signing what it can must never become
    // silence about what it cannot.
    assert_eq!(
        withheld_group_ids(&out.withheld),
        vec![thread_a],
        "the hazardous line must be reported alongside the successful sign-off"
    );
    assert_eq!(
        out.separation_targets.get(&thread_a),
        Some(&sorted(vec![thread_a, thread_b])),
        "with the thread ids that make `medication-separate` runnable"
    );
    assert_eq!(
        attestation_count(&c, thread_b).await,
        0,
        "B's own thread on the withheld line is not signed"
    );

    // The other patient's thread is untouched: signing B's chart says nothing about A's.
    assert_eq!(
        attestation_count(&c, thread_a).await,
        0,
        "patient A's thread is not vouched by patient B's sign-off"
    );
}
