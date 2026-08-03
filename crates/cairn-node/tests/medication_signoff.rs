//! Whole-list sign-off (#288): ONE human gesture attests every thread on the chart whose
//! vouch is absent or stale, in ONE transaction.
//!
//! The N per-thread attestations are a cryptographic artifact of ADR-0049's commitment
//! model, not N human acts: `attest_thread_in_tx` takes an already-unsealed key by
//! reference, so one unseal and one transaction cover all N.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard. Runtime key material
//! (house rule 6 — no literal seeds/kids anywhere below, including the second human's).
//!
//! GUARD-BEFORE-CONNECT. Every test below acquires `test_serial_guard` BEFORE
//! `connect_and_load_schema`, following this directory's prevailing convention (see
//! `medication_read.rs`, `medication_attestation.rs`, `medication_reconciliation.rs`, and
//! dozens more — guard-first is the norm here, not an exception). `sign_off_medication_list`
//! reads across nearly every medication view (it calls `list_patient_medications` twice),
//! so its lock footprint is wide; acquiring the guard first serializes each test's schema
//! load against its siblings too, avoiding the lock-order deadlock a connect-then-guard
//! ordering would risk.
mod common;

use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, change_dose,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ChangeDoseInput,
};
use common::{attestation_count, cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

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

#[tokio::test]
async fn one_gesture_attests_every_unvouched_thread() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;
    let d = assert_one(&mut c, &sk, &kid, "origin-a", patient, "atorvastatin").await;

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();

    assert_eq!(out.attested.len(), 3, "one gesture, three threads");
    assert_eq!(out.event_ids.len(), 3, "one attestation event per thread");
    for t in [a, b, d] {
        assert_eq!(
            attestation_count(&c, t).await,
            1,
            "thread {t} vouched exactly once"
        );
    }
}

/// THE #288 contract: another clinician's current signature is left exactly as it is.
#[tokio::test]
async fn a_thread_with_a_fresh_vouch_is_left_untouched() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    // A SECOND human — "Dr B" — whose signature must survive the first human's sign-off.
    // `generate_key()` returns (SigningKey, hex kid); the kid is never a literal (rule 6).
    //
    // ADR-0044: `actor_id` content-addresses the PINNED DETERMINANT SET, not the key (a
    // key is mutable across `rotate-key`). `medication_setup` already enrolled a human
    // under `{"role":"clinician"}`; reusing that same bare pinned set here would
    // content-address to the SAME actor_id as that first human, and `enroll_actor` refuses
    // it as a silent identity merge (issue #152) rather than actually creating Dr B. A
    // second, genuinely distinct human needs a person-distinguishing determinant added to
    // the set — `handle` here — so the two humans address to two different actors.
    let (other_sk, other_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"dr-b\"}', $1)",
        &[&other_kid],
    )
    .await
    .unwrap();
    let patient = Uuid::now_v7();

    let signed_by_other = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let unsigned = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;

    let other = AttestParams {
        human_sk: &other_sk,
        human_kid: &other_kid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &other, patient, signed_by_other)
        .await
        .unwrap();

    let me = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &me, patient)
        .await
        .unwrap();

    assert_eq!(
        out.attested,
        vec![unsigned],
        "only the unsigned thread is signed"
    );
    assert_eq!(
        attestation_count(&c, signed_by_other).await,
        1,
        "Dr B's signature is not signed over"
    );
    // Same UUID-binding convention as `attestation_count`: text-bind and cast in SQL.
    let who: String = c
        .query_one(
            "SELECT attester_kid FROM medication_thread_attestation \
             WHERE medication_id = $1::text::uuid",
            &[&signed_by_other.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(who, other_kid, "the drug line still carries Dr B's name");
}

#[tokio::test]
async fn a_ceased_thread_is_not_signed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
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
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();

    assert!(out.attested.is_empty(), "a struck line is not re-signed");
    assert_eq!(attestation_count(&c, thread).await, 0);
    // FIX 4 (#288 review): the ceased row still counts toward `total_rows` — this chart is
    // NOT empty, it just has nothing that currently needs a signature. Distinguishing this
    // from a genuinely empty chart (see `an_empty_list_signs_nothing_without_erroring`
    // below) is exactly what `total_rows` exists for.
    assert_eq!(
        out.total_rows, 1,
        "a ceased row is still a row on the chart, not an empty chart"
    );
    // FIX 3 (#338 review finding 2): `total_rows` alone cannot keep the caller honest here.
    // This chart holds ONE drug and it carries NO signature — so a caller that sees
    // `total_rows > 0` and an empty `attested` and concludes "every drug already carries a
    // current signature" states a falsehood. `active_rows` is what separates "nothing on
    // this chart CURRENTLY NEEDS a signature" from "every current drug HAS one".
    assert_eq!(
        out.active_rows, 0,
        "a ceased-only chart has no current drug at all — never say its drugs are signed"
    );
}

/// An empty chart signs nothing and does NOT error. Recording "nil medications, reviewed"
/// has no record-layer home yet — issue #331.
#[tokio::test]
async fn an_empty_list_signs_nothing_without_erroring() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, _kid, hsk, hkid) = setup(&c).await;

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, Uuid::now_v7())
        .await
        .unwrap();

    assert!(out.attested.is_empty());
    assert!(out.event_ids.is_empty());
    // FIX 4 (#288 review): a genuinely empty chart must report zero `total_rows`, so a
    // caller (the CLI's `MedicationSignOff` handler) can tell "nothing recorded" apart from
    // "everything already vouched" instead of printing the same reassurance for both.
    assert_eq!(
        out.total_rows, 0,
        "a chart with no medications at all must report zero total_rows"
    );
    assert_eq!(out.active_rows, 0, "and no current drugs either");
}

/// FIX 3 (#338 review finding 2), the positive case: a chart whose current drugs really
/// ARE all signed must be distinguishable from the ceased-only chart above. Both report an
/// empty `attested`; only this one may be described as "every current drug already carries
/// a signature", and `active_rows` is the field that licenses that sentence.
#[tokio::test]
async fn a_fully_vouched_chart_reports_its_current_rows() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };

    // First gesture signs it; the second finds nothing left to do.
    sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();

    assert!(out.attested.is_empty(), "nothing left to sign");
    assert_eq!(
        out.active_rows, 1,
        "there IS a current drug here, and it is signed — the one case where \
         'every current drug already carries a signature' is a true statement"
    );
}

/// An unenrolled attester fails EVERY line, and commits nothing.
///
/// The db/005 responsibility gate checks only the attester's KEY, never which medication
/// thread is being attested, so every line fails alike. That is a UNIFORM failure, not
/// collateral damage — no line was refused *because of another line* — so ADR-0060 is not
/// engaged, and the correct outcome is simply "nothing committed, every line reported as
/// failed".
///
/// The verb returns `Ok` with per-line failures rather than `Err`: whole-gesture validation
/// ("is this key an enrolled human?") belongs in the caller's pre-flight, which the CLI does
/// via `resolve_attester`. Reaching this code with a bad key means that pre-flight was
/// skipped, and the honest report is still per line. The CLI exits non-zero when any line
/// failed.
#[tokio::test]
async fn an_unenrolled_attester_fails_every_line_and_commits_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;

    let (stranger_sk, stranger_kid) = generate_key().unwrap();
    let params = AttestParams {
        human_sk: &stranger_sk,
        human_kid: &stranger_kid,
        basis: None,
        note: None,
    };

    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .expect("per-line failures are reported, not raised as a whole-gesture error");
    assert!(out.attested.is_empty(), "nothing may be signed");
    assert_eq!(out.failed.len(), 2, "both lines are reported as failed");
    assert_eq!(attestation_count(&c, a).await, 0);
    assert_eq!(attestation_count(&c, b).await, 0);
}

/// **ADR-0060, at the transaction layer: no collateral damage on rollback.**
///
/// The maintainer's ruling (2026-08-03): *"transaction scope must match the atomicity we
/// discussed — an order must not be refused because another order is invalid or incomplete.
/// Hence db transactions must ensure no collateral damage on rollbacks."*
///
/// Until this test, sign-off bundled all N attestations into ONE transaction, so a failure
/// on any one line rolled back every other line's committed act. That is the ADR-0060
/// anti-pattern living inside the very verb the ADR was written for — a failure on the
/// potassium minibag un-writing the saline.
///
/// THE FIXTURE, and why it needs no test-only seam. `cairn_medication_thread_commitment`
/// (db/034) resolves a thread's content events through `cairn_clear_payload`, which reads
/// the `event_clear` shadow for a sealed body (ADR-0052). Deleting one thread's `event_clear`
/// row reproduces a **partial-custody node** — an event synced without its DEK, a real state
/// this schema anticipates — and makes that thread, and only that thread, uncommittable:
/// `attest_thread_in_tx` refuses with "no local content ... nothing to vouch for". The
/// projection row survives, so the line still displays and is still a sign-off target. This
/// is the asymmetric failure issue #333 said needed an injection seam; it does not.
///
/// THE MIDDLE LINE IS THE BROKEN ONE, deliberately. Commit order is `sign_off_targets`'
/// uuid sort, which for `Uuid::now_v7` is creation order, so breaking the middle drug puts a
/// successful commit BOTH BEFORE and AFTER the failure. That is what makes this a real test
/// of rollback scope: the earlier success must survive a later failure (the case a single
/// transaction got wrong), and the later success must not be pre-empted by an earlier one.
#[tokio::test]
async fn a_line_that_cannot_be_attested_never_rolls_back_the_others() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let first = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let broken = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;
    let last = assert_one(&mut c, &sk, &kid, "origin-a", patient, "warfarin").await;
    // The commit order this test depends on, asserted rather than assumed.
    assert!(
        first < broken && broken < last,
        "v7 uuids sort by creation time, so the broken line must be the MIDDLE commit"
    );

    // Strip the middle thread's clear payload: its statement becomes unreadable, so its
    // commitment resolves to NULL and only that line can no longer be vouched.
    let stripped = c
        .execute(
            "DELETE FROM event_clear WHERE event_id IN ( \
               SELECT el.event_id FROM event_log el \
               WHERE el.event_type = 'clinical.medication.asserted' \
                 AND (cairn_clear_payload(el) ->> 'medication_id')::uuid = $1::text::uuid)",
            &[&broken.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(stripped, 1, "exactly one statement's clear payload removed");

    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .expect("one unattestable line must not fail the gesture");

    // THE PROPERTY: both sound lines are signed and COMMITTED, either side of the failure.
    assert_eq!(
        out.attested,
        vec![first, last],
        "the lines before AND after the failure must both be signed"
    );
    assert_eq!(
        attestation_count(&c, first).await,
        1,
        "the line committed BEFORE the failure must survive it — this is the exact \
         collateral damage a single shared transaction caused"
    );
    assert_eq!(attestation_count(&c, last).await, 1);

    // And the failure is reported per line, naming which line and why.
    assert_eq!(out.failed.len(), 1, "exactly one line failed");
    assert_eq!(out.failed[0].medication_id, broken);
    assert!(
        out.failed[0].error.contains("no local content"),
        "the reported reason must be the real one, got: {}",
        out.failed[0].error
    );
    assert_eq!(
        attestation_count(&c, broken).await,
        0,
        "and the failed line really is unsigned"
    );
}

/// FIX 5(c) (#288 final review): each half of "re-vouch a drug that changed" already has
/// coverage on its own — `medication_read.rs`'s `a_thread_grown_after_attestation_reads_as_stale`
/// pins that a content change flips the vouch to `Stale`, and
/// `one_gesture_attests_every_unvouched_thread` above pins that whole-list sign-off signs
/// unvouched/stale threads — but their COMPOSITION was never exercised: does a SECOND
/// whole-list sign-off actually pick up a thread that changed after the FIRST one signed
/// it? That is ADR-0049's entire point (a vouch is a claim about content, and it must go
/// stale — and be re-signable — the moment the content it vouched for moves).
///
/// A CESSATION deliberately would not exercise this: a ceased thread is excluded from
/// targeting entirely (see `a_ceased_thread_is_not_signed` above), so growing the thread
/// with a dose change — which keeps it ACTIVE — is what actually forces the re-vouch path.
#[tokio::test]
async fn a_dose_change_after_signoff_is_re_signed_by_the_next_whole_list_signoff() {
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

    // First whole-list sign-off: the thread is unvouched, so it is signed.
    let first = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();
    assert_eq!(
        first.attested,
        vec![thread],
        "the first sign-off vouches for the thread"
    );
    assert_eq!(attestation_count(&c, thread).await, 1);

    // Grow the thread's content with a REAL clinical change — a dose change, not a
    // cessation — so the thread stays ACTIVE and therefore stays a sign-off target.
    change_dose(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        patient,
        thread,
        &ChangeDoseInput {
            dose_amount: Some("1000"),
            dose_unit: Some("mg"),
            effective: None,
            effective_precision: None,
            info_source: "clinician",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();

    // Second whole-list sign-off: the vouch from the first is now stale (the thread grew
    // after it), so the whole-list gesture must pick the thread back up and re-attest it.
    let second = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient)
        .await
        .unwrap();
    assert_eq!(
        second.attested,
        vec![thread],
        "the whole-list gesture must re-vouch the thread whose content changed since the last sign-off"
    );
    assert_eq!(
        attestation_count(&c, thread).await,
        2,
        "the dose change earned the thread a SECOND attestation row: the stale one from the \
         first sign-off, plus the fresh re-vouch from the second"
    );
}
