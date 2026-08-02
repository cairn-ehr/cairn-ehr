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
use cairn_medication_view::{MedicationStatus, VouchState};
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
    let theirs = Uuid::now_v7();

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

/// Fix 1 (#288 final review, issue #334): a reconciled group whose member threads span
/// TWO patients is a standing wrong-chart hazard. `medication_group_display`'s
/// `DISTINCT ON (group_id)` always picks exactly ONE patient as the group's displayed
/// owner (see db/033's comment on that view), so `patient_medication_current` shows the
/// group under the WINNING patient's id — TWICE, once per `medication_group_status` row —
/// while the LOSING patient's chart shows no row for it at all, even though the node holds
/// real, locally-known content (a whole medication thread) for that patient.
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
async fn a_cross_patient_group_is_missing_from_the_losing_patients_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", patient_a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "amlodipine").await;

    // Fold both threads into ONE group, with thread_a as the group id — the same shape
    // `cairn_recompute_medication_group` writes for a real reconciled pair. Using thread_a
    // as the group id (rather than relying on which of the two UUIDs happens to sort
    // lower) makes patient A deterministically the "winner" below via
    // `medication_group_display`'s `(s.medication_id = g.group_id) DESC` tiebreak.
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&thread_a.to_string(), &thread_b.to_string()],
    )
    .await
    .unwrap();

    // Patient A wins the tiebreak (its member IS the group id), so patient A's chart shows
    // the group — deduplicated to ONE row by the FIX 1(c) defence in `read.rs`, not the two
    // `medication_group_status` would otherwise emit — and carries the cross-patient
    // warning so the winning chart's reader can see the hazard too.
    let a_list = list_patient_medications(&c, patient_a).await.unwrap();
    assert_eq!(a_list.rows.len(), 1, "the group is deduplicated to one row");
    assert!(
        a_list.rows[0].cross_patient,
        "the winning patient's row must carry the cross-patient warning"
    );
    assert!(
        a_list.groups_missing_from_chart.is_empty(),
        "patient A's own thread is fully accounted for on patient A's chart"
    );

    // Patient B loses the tiebreak: every row `patient_medication_current` emits for this
    // group carries the WINNER's (patient A's) patient_id, so filtering
    // `WHERE patient_id = patient_b` returns nothing — patient B's chart renders empty even
    // though the node holds a real, locally-known drug (thread_b) for patient B. The
    // `groups_missing_from_chart` signal is what catches this silent gap.
    let b_list = list_patient_medications(&c, patient_b).await.unwrap();
    assert!(
        b_list.rows.is_empty(),
        "the group displays under patient A only — patient B's chart shows nothing"
    );
    assert_eq!(
        b_list.groups_missing_from_chart,
        vec![thread_a],
        "the node must surface that a locally-known group is missing from this chart"
    );
    // FIX 1 (#338 review finding 1): naming the group is not enough to ACT on it.
    // `medication-separate` takes TWO THREAD ids, and patient B's own thread_b appears
    // nowhere else on this chart — the rows are empty and `read_member_vouches` is
    // patient-scoped, so without this the operator is told to run a command whose
    // arguments the node never shows them.
    assert_eq!(
        b_list.separation_targets.get(&thread_a),
        Some(&sorted(vec![thread_a, thread_b])),
        "the hazardous group must carry BOTH member threads, including the one belonging \
         to the other patient — they are the arguments to `medication-separate`"
    );

    // The read-path defence this fix exists for: sign-off must REFUSE for patient B rather
    // than silently reporting "nothing to sign off" over a chart the node itself knows is
    // incomplete — that would be a false statement about who is responsible for thread_b.
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let err = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient_b)
        .await
        .expect_err("sign-off must refuse a chart the node knows is missing content");
    // Assert on the BRANCH, not merely on failure: any unrelated error inside the
    // orchestrator would satisfy `is_err()`, so the test would keep passing if the #334
    // refusal were deleted and something else happened to break.
    let message = format!("{err:#}");
    assert!(
        message.contains("#334") && message.contains("do not appear on this chart"),
        "the refusal must be the #334 incomplete-chart branch, got: {message}"
    );
    // FIX 1 (#338 review finding 1): a refusal that names a remedy must name the remedy's
    // ARGUMENTS. The message points the operator at `medication-separate`, which takes two
    // thread ids — so both must appear in the message, or the one exit from this hard
    // refusal is a dead end that can only be cleared with raw SQL.
    assert!(
        message.contains(&thread_a.to_string()) && message.contains(&thread_b.to_string()),
        "the refusal must name BOTH member threads to pass to `medication-separate`, \
         got: {message}"
    );

    // The WINNING patient's side of the same hazard. Patient A's chart DOES show the line,
    // so there is nothing missing to refuse over — but the dose on that line comes from
    // `medication_group_current_dose`, which picks one member across the whole group
    // regardless of patient, so it may be patient B's dose under patient A's drug name.
    // The line is therefore withheld from the gesture instead of signed. Withholding is
    // per LINE: if the fix ever escalated to refusing the whole chart, this call would
    // return Err and the assertion below would fail.
    let a_out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient_a)
        .await
        .expect("the winning patient's chart is complete, so sign-off must not refuse it");
    assert!(
        a_out.attested.is_empty(),
        "a cross-patient line must not be signed: its displayed dose may be another patient's"
    );
    assert_eq!(
        a_out.withheld,
        vec![thread_a],
        "the withheld line must be REPORTED, or the clinician reads an empty result as \
         'nothing needed doing' over a drug that still needs their signature"
    );
    // FIX 1 (#338 review finding 1), the winning chart's side: the withheld line's warning
    // names `medication-separate` too, and patient A's own row lists only patient A's
    // member thread (the vouch read is patient-scoped) — so the outcome must carry the
    // group's FULL membership or A's clinician is given the same unactionable advice.
    assert_eq!(
        a_out.separation_targets.get(&thread_a),
        Some(&sorted(vec![thread_a, thread_b])),
        "the withheld line must carry both member threads for `medication-separate`"
    );
}

/// FIX 2 (#338 review finding 4): the refusal above is only interesting when the losing
/// patient's chart is OTHERWISE SIGNABLE. `a_cross_patient_group_is_missing_from_the_
/// losing_patients_chart` leaves patient B with an empty chart, so refusing costs B
/// nothing there and the test cannot see the price of the decision.
///
/// Here patient B also has an ordinary, unsigned drug of their own. The refusal therefore
/// blocks a drug that is in no way implicated in the cross-patient group — the whole chart
/// is held hostage by one invisible line. That is the CURRENT deliberate design (refuse
/// when a line is *missing*, withhold when a line is *present but untrustworthy*), and
/// this test pins it so the cost is visible in the suite rather than argued in prose.
/// Issue #339 tracks whether the asymmetry is right; if it is ever resolved the other way
/// this test fails loudly instead of the behaviour changing unnoticed.
#[tokio::test]
async fn an_incomplete_chart_refuses_sign_off_even_for_its_unrelated_drugs() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient_a = Uuid::now_v7();
    let patient_b = Uuid::now_v7();

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", patient_a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "amlodipine").await;
    // Patient B's OWN, entirely ordinary drug — never reconciled with anything, no hazard
    // of its own, and unsigned. On paper B's clinician would simply sign this line.
    let unrelated = assert_one(&mut c, &sk, &kid, "origin-a", patient_b, "warfarin").await;

    // Same peer-arrival shape as the test above: thread_a is the group id, so patient A
    // wins `medication_group_display`'s tiebreak and patient B loses it.
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&thread_a.to_string(), &thread_b.to_string()],
    )
    .await
    .unwrap();

    // B's chart is NOT empty: warfarin displays normally and needs a signature.
    let b_list = list_patient_medications(&c, patient_b).await.unwrap();
    assert_eq!(
        b_list.rows.len(),
        1,
        "patient B's own un-reconciled drug still displays"
    );
    assert_eq!(b_list.rows[0].group_id, unrelated);
    assert!(
        !b_list.rows[0].cross_patient,
        "the unrelated drug carries no hazard of its own"
    );
    assert_eq!(
        b_list.groups_missing_from_chart,
        vec![thread_a],
        "the invisible group is still surfaced"
    );

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let err = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient_b)
        .await
        .expect_err("an incomplete chart is refused whole, unrelated drugs included");
    let message = format!("{err:#}");
    assert!(
        message.contains("#334"),
        "the refusal must be the #334 incomplete-chart branch, got: {message}"
    );
    assert!(
        message.contains(&thread_a.to_string()) && message.contains(&thread_b.to_string()),
        "the refusal must name both threads to separate, got: {message}"
    );
    // THE COST, made explicit: warfarin — a drug with nothing wrong with it — is left
    // unsigned by a refusal caused by an entirely different group.
    assert_eq!(
        attestation_count(&c, unrelated).await,
        0,
        "the unrelated drug is left unsigned by the whole-chart refusal (issue #339)"
    );
}
