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
    reconcile_medications, resolve_correction_target, separate_medications, AssertMedicationInput,
    AttestParams, ChangeDoseInput, CorrectDoseInput, ReconcileInput,
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
    // ADR-0052: register THIS node's unwrap key (derived from the device/node key) so the
    // strict door can wrap every sealed event's DEK into custody — including human-signed
    // attestation events, which are clinical.* and born-sealed too. A node has exactly ONE
    // unwrap key regardless of who signs individual events; the human key never derives it
    // (that would collide with the device key on the node_unwrap_key singleton). This also
    // covers the orphan-attestation case, where no content event ever runs the verb path.
    let secret = cairn_event::seal::derive_unwrap_secret(&sk_d.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
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
    // The production contributor shape: one responsibility-bearing entry. This is what
    // makes the db/005 gate demand a valid human token (v_bears) AND what the db/034
    // floor's M1 hardening now requires for the type to be structurally valid.
    let contributors = serde_json::json!([
        {"actor_id": human_kid, "role": "attested",
         "responsibility": {"held_by": human_kid}}
    ]);
    build_attestation_body_with_contributors(
        event_id,
        patient,
        payload,
        human_kid,
        contributors,
        hlc,
    )
}

/// Same as `build_attestation_body` but with a CALLER-SUPPLIED contributor set — used
/// by the M1 hostile-client test to author an attestation with NO responsibility
/// contributor (which a well-behaved client never does; the production Rust builder
/// always carries one).
fn build_attestation_body_with_contributors(
    event_id: Uuid,
    patient: Uuid,
    payload: serde_json::Value,
    signer_kid: &str,
    contributors: serde_json::Value,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-attestation.asserted".into(),
        schema_version: "clinical.medication-attestation/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: signer_kid.into(),
        contributors,
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
    // ADR-0052: the attestation event is clinical.* and must be born-sealed. Seal the
    // clear body (payload + twin under a fresh per-event DEK, outer stub twin), sign the
    // SEALED form (so the content_address the token binds to covers the ciphertext and
    // survives a shred), and submit through the 4-arg strict door carrying the human token
    // AND the DEK. The node unwrap key was registered in setup(), so the door wraps the
    // DEK into custody. `body` is borrowed and reused by some callers, so seal a clone.
    let mut sealed = body.clone();
    let twin = sealed
        .plaintext_twin
        .take()
        .expect("attestation body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&sealed.payload, &twin, &sealed.event_id)
            .expect("seal the attestation body");
    sealed.payload = container;
    sealed.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&sealed.event_type));
    let signed = sign(&sealed, human_sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, human_kid, "attested", human_sk).unwrap();
    let vk_h = human_sk.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1, $2, $3, $4)",
        &[&signed.signed_bytes, &token, &vk_h, &dek.as_slice()],
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

/// M1 (issue #181): a hostile/buggy raw-SQL client can author an attestation event
/// with NO responsibility-bearing contributor. Such a body slips past the db/005
/// attestation gate entirely — `v_bears` is false, so no token is demanded and
/// `attester_key` stays NULL — and (before this hardening) failed only later, at the
/// apply trigger's `attester_kid TEXT NOT NULL`, with a cryptic "null value violates
/// not-null constraint". The db/034 floor now rejects it LEGIBLY at
/// `cairn_check_medication_attestation`, because a responsibility-bearing contributor
/// is exactly what this event type exists to carry: its absence is a structural floor
/// violation (principle 12 — the floor is the clean hostile-client rejection point,
/// mirroring db/026's legible blob-verify errors), caught BEFORE the trigger. Still
/// fail-closed either way; this only upgrades the message. The production Rust builder
/// always carries the contributor, so no legitimate event is affected.
#[tokio::test]
async fn floor_rejects_attestation_without_responsibility_contributor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Otherwise structurally valid (valid uuid, valid even-length hex, non-negative
    // count), so the ONLY thing wrong is the missing responsibility contributor — this
    // isolates the M1 floor check. The contributor carries a plain `role` and NO
    // `responsibility` key, so the db/005 gate never asks for a token.
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "reviewed_commitment": placeholder_commitment_hex(),
        "reviewed_count": 1,
    });
    let contributors = serde_json::json!([{"actor_id": kid_d, "role": "recorded"}]);
    let body = build_attestation_body_with_contributors(
        Uuid::now_v7(),
        patient,
        payload,
        &kid_d,
        contributors,
        hlc,
    );

    // Submitted through the 4-arg door with NO token — exactly what a raw client would do,
    // since with no responsibility marker the gate never demands one. An enrolled DEVICE
    // key signs it. ADR-0052: it must be born-sealed to clear the born-sealed floor and
    // actually REACH the db/034 responsibility-contributor check this test pins (an
    // unsealed clinical body would be refused earlier, losing this coverage). The check
    // runs at the twin dispatch, before the custody path, so it still fails there legibly.
    let mut sealed = body;
    let twin = sealed
        .plaintext_twin
        .take()
        .expect("attestation body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&sealed.payload, &twin, &sealed.event_id)
            .expect("seal the attestation body");
    sealed.payload = container;
    sealed.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&sealed.event_type));
    let signed = sign(&sealed, &sk_d).unwrap();
    let res = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_slice()],
        )
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("requires a responsibility-bearing contributor"),
        "expected a legible floor rejection, got: {err}"
    );
    assert!(
        !err.contains("null value"),
        "the floor must reject BEFORE the trigger's cryptic NOT NULL error: {err}"
    );

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the responsibility-less attestation never landed");
}

/// M1 defense-in-depth (issue #181; door-level gap tracked in #184): the floor check's
/// `jsonb_typeof(...) IS DISTINCT FROM 'array'` guard, exercised where it is actually
/// load-bearing — a DIRECT call of `cairn_check_medication_attestation` with a NON-array
/// `contributors`. Through the two submit doors this branch is unreachable: both compute
/// v_bears (`jsonb_array_elements` over `contributors`) BEFORE the floor, so a non-array
/// is rejected upstream with a cryptic "cannot extract elements from a scalar" (the #184
/// gap). But the check fn is a public SQL function a future door could call first, so the
/// guard must short-circuit the OR — `jsonb_array_elements` never runs on a non-array —
/// yielding the legible responsibility message rather than the scalar-extract error. This
/// pins that short-circuit (which PostgreSQL does not contractually guarantee, so it is
/// worth a test) and turns the guard from a dead branch into a covered one.
#[tokio::test]
async fn floor_check_fn_directly_rejects_non_array_contributors() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A non-array `contributors` (a bare string). `payload` is present + non-null so the
    // fn passes its payload-null guard and reaches the contributors check (which runs
    // before the medication_id/commitment/count checks). No signing needed — this calls
    // the floor check fn directly, exactly the direct-caller path the guard protects.
    let body = serde_json::json!({ "payload": {}, "contributors": "not-an-array" }).to_string();
    let res = c
        .execute(
            "SELECT cairn_check_medication_attestation('clinical.medication-attestation.asserted', $1::text::jsonb)",
            &[&body],
        )
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("requires a responsibility-bearing contributor"),
        "the type guard must short-circuit to the legible message, got: {err}"
    );
    assert!(
        !err.contains("cannot extract elements"),
        "the guard must prevent the cryptic scalar-extract error: {err}"
    );
}

/// The responsibility-separation guarantee at THIS event type: only an enrolled
/// kind='human' actor may vouch. A DEVICE key that self-signs+self-attests an
/// otherwise-well-formed attestation (real thread, real commitment, valid floor) is
/// refused by the db/005 3-arg human gate — signature proves origin, but attestation
/// confers responsibility, and a device carries none (principle 10). This guards
/// against a bespoke client forging a human vouch with a device key. db/005 is the
/// real, unchanged enforcement; this test locks the guarantee at the
/// medication-attestation surface (previously only the CLI e2e smoke covered it).
#[tokio::test]
async fn device_key_cannot_attest_only_humans_vouch() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // A real thread with genuine content, so the floor + twin all pass and the ONLY
    // thing that can reject the vouch is the human gate — isolating the guarantee.
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
    // Build + sign + attest with the DEVICE key (kid_d), not a human.
    let body = build_attestation_body(Uuid::now_v7(), patient, payload, &kid_d, hlc);
    let res = sign_attest_submit(&c, &body, &sk_d, &kid_d).await;

    let err = db_msg(&res.unwrap_err());
    assert!(
        err.contains("not an enrolled human actor"),
        "a device key must not be able to vouch (only humans confer responsibility); got: {err}"
    );
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'clinical.medication-attestation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the forged-human attestation never landed");
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

/// Build a vouch pinning `reviewed_commitment_hex` with the given `reviewed_count` and
/// an EXPLICIT `hlc` (so a caller can force an equal-HLC collision), sign+attest+submit
/// through the 3-arg human door, and return its content_address — the byte-order key the
/// `medication_thread_attestation` DISTINCT ON tiebreaks on. Used by the equal-HLC test.
#[allow(clippy::too_many_arguments)] // explicit HLC + commitment + count + human key/kid, mirrors submit_attestation
async fn submit_vouch_returning_ca(
    c: &Client,
    patient: Uuid,
    medication_id: Uuid,
    reviewed_commitment_hex: &str,
    reviewed_count: i32,
    sk_h: &SigningKey,
    kid_h: &str,
    hlc: Hlc,
) -> Vec<u8> {
    let payload = serde_json::json!({
        "medication_id": medication_id.to_string(),
        "reviewed_commitment": reviewed_commitment_hex,
        "reviewed_count": reviewed_count,
    });
    // ADR-0052: seal the clear body (payload + twin) under a fresh DEK, sign the sealed
    // form, and submit through the 4-arg strict door with the human token AND the DEK.
    let mut body = build_attestation_body(Uuid::now_v7(), patient, payload, kid_h, hlc);
    let twin = body
        .plaintext_twin
        .take()
        .expect("attestation body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal the attestation body");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1, $2, $3, $4)",
        &[&signed.signed_bytes, &token, &vk_h, &dek.as_slice()],
    )
    .await
    .expect("an equal-HLC-but-distinct-event vouch is accepted (backdating is legal)");
    ca
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
            // ADR-0052: sealed content carries ciphertext in body — read the thread key
            // through cairn_clear_payload (the event_clear shadow) to find the assert.
            // CAUTION (ADR-0049 false-fresh, Tasks 8/9): a cairn_clear_payload thread
            // lookup is CUSTODY-dependent — a partial-custody node sees fewer content
            // events than the true set, so any staleness signal derived from it can read
            // FALSE-FRESH. Tasks 8/9 MUST gate the staleness view (readable content count <
            // reviewed_count => stale/unknown); see cairn_medication_thread_commitment.
            "SELECT min(hlc_wall) FROM event_log WHERE event_type IN ( \
                 'clinical.medication.asserted', \
                 'clinical.medication-cessation.asserted', \
                 'clinical.medication-dose-change.asserted', \
                 'clinical.medication-dose-correction.asserted') \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
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
    let mut body = build_dose_change_body(event_id, medication_id, patient, &ch, &kid_d, low_hlc);
    // ADR-0052: a clinical dose change is born-sealed — seal it and submit 4-arg (device-
    // additive, no token). The node unwrap key was registered in setup(), so custody wraps.
    let twin = body
        .plaintext_twin
        .take()
        .expect("dose-change body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal the dose-change body");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, &sk_d).unwrap();
    let res = c
        .execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_slice()],
        )
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
    let event_id =
        attest_medication_thread(&mut c, &sk_d, "test-node", &params, patient, medication_id)
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
    let (sk_d, _kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();
    let medication_id = Uuid::now_v7(); // never asserted -> no local content events

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: None,
        note: None,
    };
    let err = attest_medication_thread(&mut c, &sk_d, "test-node", &params, patient, medication_id)
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
    attest_medication_thread(&mut c, &sk_d, "test-node", &params, patient, medication_id)
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
        effective: None,
        effective_precision: None,
        reason: None,
        strike: &[],
        note: Some("mis-keyed on entry"),
        info_source: None,
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
    attest_medication_thread(&mut c, &sk_d, "test-node", &params, patient, medication_id)
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

// ---------------------------------------------------------------------------
// Test gap 1 (issue #181, highest-value): the two-subject verbs (reconcile /
// separate) attest BOTH threads in ONE transaction after submitting the verb event
// (reconciliation.rs). The single-thread atomic drop-on-error is already proven
// (author_time_rejection_rolls_the_verb_back); these prove the SECOND subject's
// attestation failing rolls back the FIRST subject's already-authored attestation AND
// the verb event too. Forced cleanly by making the second subject an ORPHAN (no local
// content), so `attest_thread_in_tx` bails "nothing to vouch for" only on the second
// leg — after the first subject's vouch has already been written inside the txn.
// ---------------------------------------------------------------------------

/// Shared assertion for the two verbs: the op errored on the orphan second subject, so
/// NOTHING from the atomic block survives — no attestation for `patient`, no verb event
/// of `verb_event_type` — while `real_subject`'s own pre-existing assert (committed in a
/// separate earlier txn) is untouched, proving the rollback is scoped to the verb txn.
async fn assert_second_subject_rollback(
    c: &Client,
    patient: Uuid,
    real_subject: Uuid,
    result: anyhow::Result<Uuid>,
    verb_event_type: &str,
) {
    let err = result.expect_err("the orphan second subject must fail the whole verb");
    assert!(
        err.to_string().contains("nothing to vouch for"),
        "expected the orphan-thread refusal, got: {err}"
    );

    // medication_attestation is append-only / NOT truncated between serialized tests, so
    // both counts are scoped to THIS test's fresh patient.
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
        "the FIRST subject's attestation (authored before the second leg failed) must roll back"
    );

    let verb_events: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = $1 AND patient_id = $2::text::uuid",
            &[&verb_event_type, &patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        verb_events, 0,
        "the verb event rolled back with the attestations"
    );

    // The real subject's own assert (a separate committed txn) is untouched: exactly one
    // statement for the patient — the orphan second subject was never asserted at all.
    let statements: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        statements, 1,
        "the rollback is scoped to the verb txn: {real_subject}'s prior assert survives"
    );
}

#[tokio::test]
async fn reconcile_attest_second_subject_rejection_rolls_back_first() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // subject_a is a real thread (attested FIRST, would succeed); subject_b is an ORPHAN
    // (never asserted -> attested SECOND, bails "nothing to vouch for"). The reconcile
    // floor is structure-only/offline-first, so the reconcile event itself submits fine
    // inside the txn — the failure is purely the second attestation leg.
    let subject_a = assert_medication(
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
    let subject_b = Uuid::now_v7(); // orphan: no local content events

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
    let r = reconcile_medications(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        subject_a,
        subject_b,
        &input,
        Some(&params),
    )
    .await;

    assert_second_subject_rollback(
        &c,
        patient,
        subject_a,
        r,
        "clinical.medication-reconciliation.asserted",
    )
    .await;
}

#[tokio::test]
async fn separate_attest_second_subject_rejection_rolls_back_first() {
    // Same guarantee for the separation verb, which shares reconcile's exact
    // submit-verb-then-attest-both-in-one-txn shape.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let subject_a = assert_medication(
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
    let subject_b = Uuid::now_v7(); // orphan

    let params = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("separation review"),
        note: None,
    };
    let input = ReconcileInput {
        provenance: "clinician-judgment",
        reason: None,
    };
    let r = separate_medications(
        &mut c,
        &sk_d,
        &kid_d,
        "test-node",
        patient,
        subject_a,
        subject_b,
        &input,
        Some(&params),
    )
    .await;

    assert_second_subject_rollback(
        &c,
        patient,
        subject_a,
        r,
        "clinical.medication-separation.asserted",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Test gap 2 (issue #181): medication_group_attestation branches the existing
// group_current_only_when_all_members_current test never reaches — the
// `ta.medication_id IS NULL` (unattested member) filter and the plain singleton
// reduction (group_id = medication_id for a never-reconciled thread). Asserting
// `unattested_members`, which is computed by the view but was never checked.
// ---------------------------------------------------------------------------

/// Read the group rollup for a group_id, or None if it does not surface at all.
async fn group_attestation(c: &Client, group_id: Uuid) -> Option<(bool, i64, i64)> {
    let rows = c
        .query(
            "SELECT attested_current, unattested_members, stale_members \
             FROM medication_group_attestation WHERE group_id = $1::text::uuid",
            &[&group_id.to_string()],
        )
        .await
        .unwrap();
    rows.first().map(|r| (r.get(0), r.get(1), r.get(2)))
}

#[tokio::test]
async fn group_rollup_flags_an_unattested_member() {
    // Reconcile A and B into a group, then attest ONLY A. The `ta.medication_id IS NULL`
    // branch fires for B: the group is NOT attested_current, and unattested_members = 1
    // (stale_members stays 0 — B is unattested, not stale). This is the conservative
    // rollup's "a reconciled group is current only when EVERY member is" in its
    // partially-attested state, which the all-members-attested test cannot reach.
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

    // Attest ONLY member A; B stays unattested.
    attest_current(&c, patient, a, 1, &sk_h, &kid_h).await;

    let group_id = std::cmp::min(a, b);
    let (attested_current, unattested_members, stale_members) = group_attestation(&c, group_id)
        .await
        .expect("the reconciled group surfaces in the rollup");
    assert!(
        !attested_current,
        "a group with an unattested member is not attested_current"
    );
    assert_eq!(unattested_members, 1, "exactly member B is unattested");
    assert_eq!(
        stale_members, 0,
        "B is unattested, not stale — the two branches are distinct"
    );
}

#[tokio::test]
async fn singleton_group_reduces_to_its_thread() {
    // A never-reconciled thread is its own singleton group (group_id = medication_id via
    // medication_thread_group's COALESCE, db/033). Before any vouch it surfaces as
    // unattested (the `ta.medication_id IS NULL` branch, unattested_members = 1); after
    // one vouch it reduces trivially to attested_current with zero unattested/stale
    // members. Covers the singleton reduction the group test (which only ever reconciles)
    // never exercises.
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

    // Unattested singleton: group_id = medication_id, one unattested member, not current.
    let (attested_current, unattested_members, stale_members) =
        group_attestation(&c, medication_id)
            .await
            .expect("a never-reconciled thread is its own singleton group");
    assert!(!attested_current, "an unattested singleton is not current");
    assert_eq!(unattested_members, 1, "the lone member is unattested");
    assert_eq!(stale_members, 0);

    // Attest it: the singleton reduces trivially to attested & current.
    attest_current(&c, patient, medication_id, 1, &sk_h, &kid_h).await;
    let (attested_current2, unattested_members2, stale_members2) =
        group_attestation(&c, medication_id).await.unwrap();
    assert!(
        attested_current2,
        "an attested singleton reduces to attested_current"
    );
    assert_eq!(unattested_members2, 0);
    assert_eq!(stale_members2, 0);
}

// ---------------------------------------------------------------------------
// Test gap 4 (issue #181): the `content_address DESC` sub-tiebreak of
// medication_thread_attestation's DISTINCT ON, for two vouches on one thread sharing
// an identical (hlc_wall, hlc_counter). Near-impossible on a single node (next_hlc is
// monotonic) but NATURAL across nodes, and NOT merely cosmetic: it is
// convergence-load-bearing — without a total, node-independent tiebreak, two nodes
// could pick DIFFERENT standing vouches for the same thread and silently disagree on
// attester_kid/stale (the divergence ADR-0045 protects the projection layer from).
// content_address is the SHA-256 multihash of the signed bytes: identical on every
// node, and bytea DESC is unsigned-byte order == Rust Vec<u8> order, so the winner is
// predictable here.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn equal_hlc_vouches_resolve_deterministically_by_content_address() {
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
        &sample_assert("atorvastatin"),
        None,
    )
    .await
    .unwrap();
    let commitment = thread_commitment(&c, medication_id)
        .await
        .expect("the fresh thread has a commitment");
    let commitment_hex = hex::encode(&commitment);

    // ONE HLC, reused verbatim for BOTH vouches -> an equal-(wall,counter,origin)
    // collision. Distinct reviewed_count -> distinct payload -> distinct content_address,
    // the ONLY key the DISTINCT ON can then order by. Both pin the CURRENT commitment, so
    // both are non-stale; which one stands is decided purely by content_address DESC.
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();

    let ca1 = submit_vouch_returning_ca(
        &c,
        patient,
        medication_id,
        &commitment_hex,
        1,
        &sk_h,
        &kid_h,
        hlc.clone(),
    )
    .await;
    let ca2 = submit_vouch_returning_ca(
        &c,
        patient,
        medication_id,
        &commitment_hex,
        2,
        &sk_h,
        &kid_h,
        hlc.clone(),
    )
    .await;
    assert_ne!(
        ca1, ca2,
        "distinct payloads must yield distinct content addresses"
    );

    // Both vouches landed (append-only, keyed by event_id — the equal HLC does not dedup).
    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(landed, 2, "both equal-HLC vouches are retained");

    // The standing vouch is the one with the LARGER content_address (bytea DESC).
    let expected_standing_count: i32 = if ca1 > ca2 { 1 } else { 2 };
    let (standing_count, stale): (i32, bool) = {
        let row = c
            .query_one(
                "SELECT reviewed_count, stale FROM medication_thread_attestation \
                 WHERE medication_id = $1::text::uuid",
                &[&medication_id.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(
        standing_count, expected_standing_count,
        "the larger-content_address vouch stands (deterministic DESC tiebreak, convergent across nodes)"
    );
    assert!(
        !stale,
        "both vouches pinned the current commitment -> standing is not stale"
    );
}
