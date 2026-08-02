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
    assert_medication, attest_medication_thread, cease_medication, AssertMedicationInput,
    AttestParams, CeaseMedicationInput,
};
use common::{cs, medication_setup as setup};
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

/// How many attestation rows exist for a thread.
///
/// UUID BINDING: this crate does not enable tokio-postgres's `with-uuid-1` feature (see
/// `medication/read.rs`'s "UUID BINDING" module comment), so `thread` is bound as text and
/// cast in SQL rather than passed as a `Uuid` parameter directly.
async fn attestation_count(c: &Client, thread: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
        &[&thread.to_string()],
    )
    .await
    .unwrap()
    .get(0)
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
}

/// A refused attester leaves ZERO attestation rows. The db/005 responsibility gate checks
/// only the attester's KEY (never enrolled here), never which medication thread is being
/// attested, so it refuses on the very FIRST thread `sign_off_medication_list` attempts —
/// thread `b` is never even reached — and the whole verb returns an error before any
/// thread is signed.
///
/// WHAT THIS DOES NOT PROVE. It does not demonstrate the stronger, more interesting
/// property one-transaction sign-off actually promises: a thread that already succeeded
/// EARLIER in the same transaction being rolled back because a LATER thread's attestation
/// fails. This failure mode is not per-thread-asymmetric (an unenrolled key is refused for
/// every thread alike, not just some), so it can't exercise that path. Proving the
/// asymmetric case needs a failure that succeeds on thread A and fails on thread B within
/// one transaction, which needs a test-only injection seam this crate does not have yet —
/// tracked as issue #333, which also covers the untested double-read-mismatch refusal
/// branch in `signoff.rs`.
#[tokio::test]
async fn a_refused_attestation_signs_nothing_at_all() {
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

    // Never enrolled: the db/005 responsibility gate refuses this attester on the first
    // thread it tries — see the doc comment above for exactly what that does and does not
    // demonstrate.
    let (stranger_sk, stranger_kid) = generate_key().unwrap();
    let params = AttestParams {
        human_sk: &stranger_sk,
        human_kid: &stranger_kid,
        basis: None,
        note: None,
    };

    let err = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient).await;
    assert!(err.is_err(), "an unenrolled attester must be refused");
    assert_eq!(
        attestation_count(&c, a).await,
        0,
        "zero attestation rows after a refused attester"
    );
    assert_eq!(
        attestation_count(&c, b).await,
        0,
        "zero attestation rows after a refused attester"
    );
}
