//! §3.15/§3.16 slice-4 medication attestation — FLOOR tests (registration,
//! structural check, twin registry, set-commitment fn; part 1) plus the overlay/
//! projection tests (part 2: `medication_attestation`, its apply trigger, and the
//! `medication_thread_attestation` / `medication_group_attestation` staleness views),
//! plus the Rust orchestrator tests (part 3: the responsibility-gate + the standalone
//! `attest_medication_thread` happy-path/orphan cases, `crates/cairn-node/src/
//! medication/attestation.rs`). Parts 1-2 hand-build the attestation `EventBody`
//! inline, mirroring the exact human self-sign/self-attest pattern
//! `crates/cairn-node/src/identify.rs` and `crates/cairn-node/tests/attestation.rs`
//! already use for a responsibility-bearing event through the 3-arg `submit_event`
//! door; part 3 exercises that same construction through the production orchestrator.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard
//! (shared-DB + TRUNCATE pattern, identical to the other medication test files). Key
//! material is minted at runtime via `generate_key` (getrandom-backed) — never a
//! byte-array/string literal (house rule 6).
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, build_dose_change_body, change_dose, correct_dose,
    reconcile_medications, resolve_correction_target, AssertMedicationInput, AttestParams,
    ChangeDoseInput, CorrectDoseInput, ReconcileInput,
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

/// Truncate the log + every medication projection and enroll a fresh DEVICE actor
/// (mints the medication thread, mirrors `medication_reconciliation.rs::setup_node`)
/// plus a fresh HUMAN actor (signs + attests the attestation event). Returns
/// (device_sk, device_kid, human_sk, human_kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
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
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid_d],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_d, kid_d, sk_h, kid_h)
}

/// A minimal, valid medication assertion input — mints a real thread (rather than a
/// bare UUID) so the commitment-fn test has a genuine content event to hash.
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

/// Hand-build a `clinical.medication-attestation.asserted` `EventBody` carrying a
/// responsibility-bearing contributor (self-signed AND self-attested by the human
/// key, exactly as `apply_proposal::build_attested_link_body` /
/// `identify::identify_patient`'s link path do) — trips the db/005 attestation gate.
/// `payload` is caller-supplied so each test can independently mutate exactly the one
/// field under test (medication_id / reviewed_commitment / reviewed_count).
fn build_attestation_body(
    event_id: Uuid,
    patient: Uuid,
    payload: serde_json::Value,
    human_kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-attestation.asserted".into(),
        schema_version: "clinical.medication-attestation/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: human_kid.into(),
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "attested", "responsibility": "attested"}
        ]),
        payload,
        attachments: vec![],
        plaintext_twin: Some("reviewed and attested the medication thread".into()),
    }
}

/// Sign `body` with the human key, self-attest it (same key signs the event AND the
/// attestation token — the pattern `identify.rs`'s link path uses), and submit through
/// the 3-arg human door. Returns the raw `tokio_postgres` result so callers can assert
/// either acceptance or the exact rejection message.
async fn sign_attest_submit(
    c: &Client,
    body: &EventBody,
    human_sk: &SigningKey,
    human_kid: &str,
) -> Result<u64, tokio_postgres::Error> {
    let signed = sign(body, human_sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, human_kid, "attested", human_sk).unwrap();
    let vk_h = human_sk.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk_h],
    )
    .await
}

/// A syntactically-valid hex string for `reviewed_commitment` in tests that are NOT
/// exercising the hex-shape check itself. Not cryptographic material (house rule 6 is
/// about keys/seeds/nonces/IVs) — just a structural placeholder — but still derived
/// from a runtime-generated UUID rather than typed as a byte-array literal, so there
/// is nothing for a hard-coded-value scanner to flag either way.
fn placeholder_commitment_hex() -> String {
    hex::encode(Uuid::now_v7().as_bytes())
}

#[tokio::test]
async fn floor_accepts_well_formed_attestation() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // A real thread with one content event, so the commitment reflects genuine review.
    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    let commitment: Vec<u8> = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid)",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": medication_id.to_string(),
        "reviewed_commitment": hex::encode(&commitment),
        "reviewed_count": 1,
    });
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_h, hlc);

    let r = sign_attest_submit(&c, &body, &sk_h, &kid_h).await;
    assert!(
        r.is_ok(),
        "well-formed human-attested attestation must be accepted: {r:?}"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the attestation event landed in the log");
}

#[tokio::test]
async fn floor_rejects_bad_medication_id() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": "not-a-uuid",
        "reviewed_commitment": placeholder_commitment_hex(),
        "reviewed_count": 1,
    });
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_h, hlc);

    let res = sign_attest_submit(&c, &body, &sk_h, &kid_h).await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("medication_id must be a valid uuid"),
        "got: {err}"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the rejected attestation never landed");
}

#[tokio::test]
async fn floor_rejects_malformed_commitment() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": "zzzz",
        "reviewed_count": 1,
    });
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_h, hlc);

    let res = sign_attest_submit(&c, &body, &sk_h, &kid_h).await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("reviewed_commitment must be a non-empty even-length hex string"),
        "got: {err}"
    );
}

#[tokio::test]
async fn floor_rejects_odd_length_commitment() {
    // Task-3 review Minor: the floor regex accepted ODD-length hex, which the part-2
    // apply trigger's decode(..., 'hex') would then choke on with a cryptic low-level
    // error. "abc" is valid hex CHARACTERS but odd length — the floor must reject it
    // with a legible message BEFORE it ever reaches the trigger (principle 12: the
    // floor, not the trigger, is the clean hostile-client rejection point).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": "abc",
        "reviewed_count": 1,
    });
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_h, hlc);

    let res = sign_attest_submit(&c, &body, &sk_h, &kid_h).await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("reviewed_commitment must be a non-empty even-length hex string"),
        "got: {err}"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the rejected odd-length-hex attestation never landed");
}

#[tokio::test]
async fn floor_rejects_negative_count() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": placeholder_commitment_hex(),
        "reviewed_count": -1,
    });
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_h, hlc);

    let res = sign_attest_submit(&c, &body, &sk_h, &kid_h).await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("reviewed_count must be a non-negative integer"),
        "got: {err}"
    );
}

#[tokio::test]
async fn commitment_fn_is_deterministic_and_null_when_absent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("metformin"),
        None,
    )
    .await
    .unwrap();

    let first: Option<Vec<u8>> = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid)",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let second: Option<Vec<u8>> = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid)",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        first.is_some(),
        "a thread with one content event has a non-null commitment"
    );
    assert_eq!(
        first, second,
        "the commitment is stable across repeated calls (same event set -> same hash)"
    );

    // An unknown/never-asserted thread has no content events -> NULL (orphan case).
    let unknown = Uuid::now_v7();
    let absent: Option<Vec<u8>> = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid)",
            &[&unknown.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(absent.is_none(), "an unknown thread's commitment is NULL");
}

// ---------------------------------------------------------------------------
// Part 2 (Task 4): the attestation overlay table, its apply trigger, and the
// medication_thread_attestation / medication_group_attestation staleness views.
// ---------------------------------------------------------------------------

/// Read `medication_id`'s CURRENT thread-content commitment straight from the same
/// SQL fn the apply trigger and staleness view both call — the single source of
/// truth these tests pin their attestations against.
async fn thread_commitment(c: &Client, medication_id: Uuid) -> Option<Vec<u8>> {
    c.query_one(
        "SELECT cairn_medication_thread_commitment($1::text::uuid)",
        &[&medication_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Submit a `clinical.medication-attestation.asserted` event pinning an EXPLICIT
/// (caller-supplied) `reviewed_commitment` hex string — used both by the "vouch for
/// what's true now" helper below and by the orphan test, which deliberately pins a
/// commitment for a thread that has no local content at all.
async fn submit_attestation(
    c: &Client,
    patient: Uuid,
    medication_id: Uuid,
    reviewed_commitment_hex: &str,
    reviewed_count: i32,
    sk_h: &SigningKey,
    kid_h: &str,
) -> Result<Uuid, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let event_id = Uuid::now_v7();
    let payload = serde_json::json!({
        "medication_id": medication_id.to_string(),
        "reviewed_commitment": reviewed_commitment_hex,
        "reviewed_count": reviewed_count,
    });
    let body = build_attestation_body(event_id, patient, payload, kid_h, hlc);
    sign_attest_submit(c, &body, sk_h, kid_h).await?;
    Ok(event_id)
}

/// Attest `medication_id`'s CURRENT commitment (a human vouching for exactly what the
/// thread contains right now) — the common case most projection tests need. Panics if
/// the thread has no local content (use `submit_attestation` directly for the orphan
/// case, which deliberately has none).
async fn attest_current(
    c: &Client,
    patient: Uuid,
    medication_id: Uuid,
    reviewed_count: i32,
    sk_h: &SigningKey,
    kid_h: &str,
) -> Uuid {
    let commitment = thread_commitment(c, medication_id)
        .await
        .expect("attest_current requires the thread to already have local content");
    submit_attestation(
        c,
        patient,
        medication_id,
        &hex::encode(&commitment),
        reviewed_count,
        sk_h,
        kid_h,
    )
    .await
    .expect("a well-formed human attestation of the current commitment must be accepted")
}

#[tokio::test]
async fn post_hoc_attestation_shows_attester_and_not_stale() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Assert a real thread, then have the human attest exactly what it now contains.
    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    attest_current(&c, patient, medication_id, 1, &sk_h, &kid_h).await;

    let rows = c
        .query(
            "SELECT attester_kid, stale FROM medication_thread_attestation \
             WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "exactly one standing attestation row");
    let attester_kid: String = rows[0].get(0);
    let stale: bool = rows[0].get(1);
    assert_eq!(
        attester_kid, kid_h,
        "attester_kid is the verified human who vouched"
    );
    assert!(
        !stale,
        "the current commitment matches what was just reviewed"
    );
}

#[tokio::test]
async fn later_change_flips_stale_true() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("metformin"),
        None,
    )
    .await
    .unwrap();
    attest_current(&c, patient, medication_id, 1, &sk_h, &kid_h).await;
    let stale_before: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!stale_before, "freshly attested -> not stale");

    // A normal-HLC dose change lands on the thread after the attestation.
    let ch = ChangeDoseInput {
        dose_amount: Some("1000"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    change_dose(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        medication_id,
        &ch,
        None,
    )
    .await
    .unwrap();

    let stale_after: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stale_after,
        "a later content event changes the thread's commitment -> stale"
    );
}

#[tokio::test]
async fn lower_hlc_late_arrival_flips_stale_true() {
    // THE LOAD-BEARING TEST. A vouch pins a set-commitment, not a head position. If the
    // staleness check instead compared the attested HLC to the thread's MAX(hlc), a
    // content event that arrives LATE but stamped with a LOWER hlc (a device that was
    // offline for a week, syncing an earlier-wall record) would slip in under the
    // attested head and stay silently marked "reviewed" — exactly the failure mode a
    // human-responsibility gate must never allow. The set-commitment catches it because
    // ANY change to the content-event SET (regardless of hlc order) changes the hash.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("warfarin"),
        None,
    )
    .await
    .unwrap();
    attest_current(&c, patient, medication_id, 1, &sk_h, &kid_h).await;
    let stale_before: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!stale_before, "freshly attested -> not stale");

    // The lowest hlc_wall currently on the thread's content events (i.e. the assert's
    // own hlc, the only content event so far) — the floor the late arrival must land
    // BELOW to prove the point.
    let min_wall: i64 = c
        .query_one(
            "SELECT min(hlc_wall) FROM event_log WHERE event_type IN ( \
                 'clinical.medication.asserted', \
                 'clinical.medication-cessation.asserted', \
                 'clinical.medication-dose-change.asserted', \
                 'clinical.medication-dose-correction.asserted') \
               AND (body ->> 'medication_id')::uuid = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    // ~11.5 days earlier — comfortably below anything already on the thread, mirroring
    // a device that synced a week-old backdated record.
    let low_hlc = Hlc {
        wall: min_wall - 1_000_000_000,
        counter: 0,
        node_origin: "test-node".into(),
    };
    let ch = ChangeDoseInput {
        dose_amount: Some("5"),
        dose_unit: Some("mg"),
        effective: Some("2024-01"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("late-arriving backdated dose change"),
    };
    let event_id = Uuid::now_v7();
    let body = build_dose_change_body(event_id, medication_id, patient, &ch, &kid_d, low_hlc);
    let signed = sign(&body, &sk_d).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    assert!(
        res.is_ok(),
        "a lower-HLC event with a distinct content_address is accepted -- \
         backdating is legal, only far-FUTURE hlc is rejected: {res:?}"
    );

    let stale_after: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stale_after,
        "the content SET changed even though the new event's hlc is BELOW the \
         attested head -> stale; the set-commitment closes the gap a head-position \
         pin would miss"
    );
}

#[tokio::test]
async fn group_current_only_when_all_members_current() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    let b = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        a,
        b,
        &input,
        None,
    )
    .await
    .unwrap();

    // Attest both members' current content.
    attest_current(&c, patient, a, 1, &sk_h, &kid_h).await;
    attest_current(&c, patient, b, 1, &sk_h, &kid_h).await;

    let group_id = std::cmp::min(a, b).to_string();
    let (attested_current, stale_members): (bool, i64) = {
        let row = c
            .query_one(
                "SELECT attested_current, stale_members FROM medication_group_attestation \
                 WHERE group_id = $1::text::uuid",
                &[&group_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(
        attested_current,
        "both members attested & current -> group attested_current"
    );
    assert_eq!(stale_members, 0);

    // Now dose-change B: only B goes stale, so the group must flip.
    let ch = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    change_dose(&mut c, &sk_d, &kid_d, "test-node", patient, b, &ch, None)
        .await
        .unwrap();

    let (attested_current2, stale_members2): (bool, i64) = {
        let row = c
            .query_one(
                "SELECT attested_current, stale_members FROM medication_group_attestation \
                 WHERE group_id = $1::text::uuid",
                &[&group_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(
        !attested_current2,
        "B's attestation is now stale -> group is no longer attested_current"
    );
    assert_eq!(stale_members2, 1, "exactly B is stale");
}

#[tokio::test]
async fn orphan_attestation_renders_nothing_until_thread_arrives() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Offline-first: a human attests a medication_id that has NO local content events
    // at all (set-union sync may deliver the attestation before the thread itself).
    let medication_id = Uuid::now_v7();
    submit_attestation(
        &c,
        patient,
        medication_id,
        &placeholder_commitment_hex(),
        1,
        &sk_h,
        &kid_h,
    )
    .await
    .expect("the floor accepts an attestation for a thread not yet locally present");

    // medication_thread_attestation MAY show a row (stale=true, since the current
    // commitment is NULL and IS DISTINCT FROM the pinned hex) -- but the thread is not
    // a member of medication_thread_group (no local assert), so it must not surface in
    // the group rollup.
    let stale: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stale,
        "a NULL current commitment IS DISTINCT FROM any pinned value -> stale"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_group_attestation WHERE group_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "an orphan attestation's thread renders nothing in the group rollup until it arrives"
    );
}

// ---------------------------------------------------------------------------
// Part 3 (Task 5): the Rust orchestrator — `attest_thread_in_tx` /
// `attest_medication_thread`, `crates/cairn-node/src/medication/attestation.rs`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn attestation_requires_a_valid_human_token() {
    // The responsibility-bearing contributor is what trips the db/005 gate — proven
    // here directly against submit_event (not yet through the orchestrator) so the
    // gate behaviour is pinned independently of `attest_medication_thread`'s plumbing.
    // Mirrors `identity_repudiate.rs::unattested_repudiation_is_refused` /
    // `agent_attested_repudiation_is_refused` and `attestation.rs::
    // rejects_bad_attestations_and_keeps_the_floor` (checks N3), applied here to the
    // medication-attestation event type.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Case 1: a well-formed, human-signed attestation submitted through the 1-arg
    // door (no token at all) -> the db/005 gate refuses BEFORE any structural check
    // (the attestation gate runs ahead of the twin/floor dispatch), so a placeholder
    // payload is sufficient here.
    let hlc1 = db::next_hlc(&c, "test-node").await.unwrap();
    let payload1 = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": placeholder_commitment_hex(),
        "reviewed_count": 1,
    });
    let body1 = build_attestation_body(Uuid::now_v7(), patient, payload1, &kid_h, hlc1);
    let signed1 = sign(&body1, &sk_h).unwrap();
    let res1 = c
        .execute("SELECT submit_event($1)", &[&signed1.signed_bytes])
        .await;
    let err1 = db_msg(&res1.unwrap_err());
    assert!(
        err1.contains("requires attestation"),
        "an un-attested responsibility-bearing event must be refused: {err1}"
    );

    // Case 2: a VALID token, correctly bound, but signed/presented by the enrolled
    // DEVICE (agent) actor from `setup`, not a human -> the db/005 kind='human' check
    // refuses it (§5.7 "Human", enforced structurally).
    let hlc2 = db::next_hlc(&c, "test-node").await.unwrap();
    let payload2 = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": placeholder_commitment_hex(),
        "reviewed_count": 1,
    });
    let body2 = build_attestation_body(Uuid::now_v7(), patient, payload2, &kid_d, hlc2);
    let res2 = sign_attest_submit(&c, &body2, &sk_d, &kid_d).await;
    let err2 = db_msg(&res2.unwrap_err());
    assert!(
        err2.contains("attester is not an enrolled human actor"),
        "an agent-attested medication attestation must be refused: {err2}"
    );

    // The floor held: neither rejected attempt landed in the log.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "neither rejected attestation landed");
}

#[tokio::test]
async fn attest_medication_thread_end_to_end() {
    // The standalone post-hoc orchestrator: mint an HLC, open a txn, sign+attest+submit,
    // commit. Exercises the real production path (not the test-local hand-built body).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // A real thread with one content event, so there is something genuine to vouch for.
    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("chart review"),
        note: Some("confirmed dose with patient"),
    };
    let event_id = attest_medication_thread(&mut c, "test-node", &params, patient, medication_id)
        .await
        .expect("a well-formed post-hoc attestation of a real thread must succeed");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid \
             AND event_type = 'clinical.medication-attestation.asserted'",
            &[&event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "the orchestrator's event id is the one actually appended"
    );

    let (attester_kid, stale): (String, bool) = {
        let row = c
            .query_one(
                "SELECT attester_kid, stale FROM medication_thread_attestation \
                 WHERE medication_id = $1::text::uuid",
                &[&medication_id.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(
        attester_kid, kid_h,
        "attester_kid is the verified human who called attest_medication_thread"
    );
    assert!(
        !stale,
        "the orchestrator pinned exactly the thread's current commitment"
    );
}

#[tokio::test]
async fn attest_refuses_orphan_thread_with_clear_message() {
    // Offline-first refusal: a thread with no LOCAL content events has nothing genuine
    // to vouch for, so the orchestrator must bail with a legible message rather than
    // author a meaningless attestation (mirrors `attest_thread_in_tx`'s doc comment).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();
    let medication_id = Uuid::now_v7(); // never asserted -> no local content events

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: None,
        note: None,
    };
    let err = attest_medication_thread(&mut c, "test-node", &params, patient, medication_id)
        .await
        .expect_err("an orphan thread must refuse a post-hoc attestation");
    assert!(
        err.to_string().contains("nothing to vouch for"),
        "got: {err}"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the refused orphan attestation never landed");
}

// ---------------------------------------------------------------------------
// Part 4 (Task 6): author-time `--attest-as` on all six verb orchestrators —
// `assert_medication` / `cease_medication` / `change_dose` / `correct_dose` /
// `reconcile_medications` / `separate_medications`. Each now takes a trailing
// `attest: Option<&AttestParams<'_>>`; when `Some`, the verb's submit AND the
// attestation(s) run in ONE transaction, so the attestation's pinned commitment sees
// the content event the SAME call just submitted. A rejected attestation rolls the
// verb back with it (proven directly against `assert_medication` below; the same
// `attest_thread_in_tx` call inside every other verb shares that guarantee).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn author_time_assert_is_attested_current_in_one_txn() {
    // assert_medication(..., Some(&params)) mints the thread AND attests it in the
    // SAME transaction (Task 6 Step 1's atomic shape) -> medication_thread_attestation
    // shows the new thread with stale=false immediately, and exactly one attestation
    // row exists for it in the append-only medication_attestation table.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("chart review"),
        note: Some("confirmed with patient at time of entry"),
    };
    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        Some(&params),
    )
    .await
    .expect("author-time assert+attest must succeed atomically in one txn");

    let (attester_kid, stale): (String, bool) = {
        let row = c
            .query_one(
                "SELECT attester_kid, stale FROM medication_thread_attestation \
                 WHERE medication_id = $1::text::uuid",
                &[&medication_id.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(
        attester_kid, kid_h,
        "attester_kid is the human named in AttestParams, not the device that signed the assert"
    );
    assert!(
        !stale,
        "the attestation was minted from the just-submitted assert's OWN commitment, \
         seen because both ran in the same txn/snapshot"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "exactly one attestation row for the freshly-minted thread"
    );
}

#[tokio::test]
async fn author_time_rejection_rolls_the_verb_back() {
    // A human key that was NEVER enrolled (mirrors identify.rs's
    // human_precheck_distinguishes_human_from_device_and_unenrolled and
    // link_with_non_human_attester_rolls_back_the_whole_op) -> the db/005 gate's
    // kind='human' check refuses the attestation, and per Task 6's atomic shape that
    // MUST roll the assert back with it: neither the medication_statement row nor the
    // attestation row may survive.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let (unenrolled_sk, unenrolled_kid) = generate_key().unwrap();
    let patient = Uuid::now_v7();

    let params = AttestParams {
        human_sk: &unenrolled_sk,
        human_kid: &unenrolled_kid,
        basis: None,
        note: None,
    };
    let r = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        Some(&params),
    )
    .await;
    assert!(
        r.is_err(),
        "an unenrolled attester must be refused by the db/005 gate"
    );

    // Scope both checks to THIS test's patient: medication_attestation is deliberately
    // NOT in setup()'s TRUNCATE list (it's append-only/audit-retained across tests, see
    // the supersede test below), so an unscoped count would pick up other serialized
    // tests' rows in the same shared DB.
    let statements: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        statements, 0,
        "the whole txn rolled back: the assert must NOT have committed either"
    );
    let attestations: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attestations, 0,
        "no attestation row may survive a rolled-back txn"
    );
}

#[tokio::test]
async fn author_time_dose_change_is_attested_current() {
    // Assert a thread device-additively (attest=None, unchanged path), THEN
    // change_dose(..., Some(&params)) atomically pins the attestation to the
    // POST-change commitment (Task 6 Step 2's atomic shape applied to change_dose).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("metformin"),
        None,
    )
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("titration review"),
        note: None,
    };
    let ch = ChangeDoseInput {
        dose_amount: Some("1000"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    change_dose(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        medication_id,
        &ch,
        Some(&params),
    )
    .await
    .expect("author-time dose-change+attest must succeed atomically in one txn");

    let (attester_kid, stale): (String, bool) = {
        let row = c
            .query_one(
                "SELECT attester_kid, stale FROM medication_thread_attestation \
                 WHERE medication_id = $1::text::uuid",
                &[&medication_id.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(attester_kid, kid_h);
    assert!(
        !stale,
        "the attestation was minted AFTER the dose change, in the same txn, so it \
         pins the post-change commitment as current"
    );
}

#[tokio::test]
async fn reconcile_attest_as_vouches_for_both_threads() {
    // Assert A and B device-additively, then reconcile_medications(A, B, ...,
    // Some(&params)) attests BOTH subject threads in the SAME transaction as the
    // reconcile event (Task 6 Step 3) -> medication_thread_attestation shows
    // non-stale rows for BOTH A and B, and the group rollup is attested_current.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    let b = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("reconciliation review"),
        note: None,
    };
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    reconcile_medications(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        a,
        b,
        &input,
        Some(&params),
    )
    .await
    .expect("author-time reconcile+attest-both must succeed atomically in one txn");

    for (label, thread) in [("A", a), ("B", b)] {
        let stale: bool = c
            .query_one(
                "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
                &[&thread.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!stale, "thread {label}'s attestation must be current");
    }

    let group_id = std::cmp::min(a, b).to_string();
    let attested_current: bool = c
        .query_one(
            "SELECT attested_current FROM medication_group_attestation WHERE group_id = $1::text::uuid",
            &[&group_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        attested_current,
        "both members attested & current in the SAME txn as the reconcile -> group attested_current"
    );
}

#[tokio::test]
async fn supersede_not_retract_correction_flips_prior_vouch_stale() {
    // assert -> attest (stale=false). A dose-correction then changes the thread's
    // content SET, flipping the standing vouch stale -- but the FIRST attestation row
    // is never deleted or mutated (medication_attestation is append-only). Attesting
    // again appends a SECOND row and the standing view flips back to current. This
    // proves responsibility is SUPERSEDED (a later vouch overrides which one is
    // "current"), never RETRACTED (the earlier vouch's own record persists intact).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let medication_id = assert_medication(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        &sample_assert("warfarin"),
        None,
    )
    .await
    .unwrap();

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("initial chart review"),
        note: None,
    };
    attest_medication_thread(&mut c, "test-node", &params, patient, medication_id)
        .await
        .expect("the first, post-hoc attestation must succeed");

    let stale_before: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!stale_before, "freshly attested -> not stale");

    // Author a "acted in error, correct is X" dose-correction against the thread's
    // current point (device-additive; the correction itself carries no attestation).
    let target = resolve_correction_target(&c, medication_id, None)
        .await
        .unwrap();
    let corr = CorrectDoseInput {
        dose_amount: Some("5"),
        dose_unit: Some("mg"),
        info_source: None,
        reason: Some("mis-keyed on entry"),
    };
    correct_dose(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        medication_id,
        target,
        &corr,
        None,
    )
    .await
    .unwrap();

    // The prior vouch is STILL PRESENT (retained, not deleted) even though the thread
    // content changed underneath it.
    let attestations_after_correction: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attestations_after_correction, 1,
        "the correction must not delete or mutate the prior attestation row"
    );
    let stale_after_correction: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stale_after_correction,
        "the correction changed the content SET -> the retained prior vouch is now stale"
    );

    // Attest again: a NEW row is appended (superseding which vouch is "current"),
    // the old one is untouched.
    attest_medication_thread(&mut c, "test-node", &params, patient, medication_id)
        .await
        .expect("re-attesting the corrected thread must succeed");

    let attestations_after_reattest: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attestations_after_reattest, 2,
        "both the original AND the new attestation are retained -- superseded, never retracted"
    );
    let stale_after_reattest: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !stale_after_reattest,
        "the standing (latest) vouch now pins the corrected commitment -> current again"
    );
}
