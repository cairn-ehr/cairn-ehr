//! Integration coverage for submit_event's attestation ACCEPT branch and the
//! valid-token-but-bad-binding rejections (the half Spike 0002 never exercised;
//! the honest gap carried into ADR-0030). Real Postgres, gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern,
//! identical to admission.rs). Tokens are minted directly via cairn_event here; the
//! CLI path is covered separately by the Python harness (Task 3).
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

const SUBMIT3: &str = "SELECT submit_event($1,$2,$3)";

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the advisory-write tables and enroll one human attester + one agent
/// signer (distinct keys). Returns (agent sk, agent kid, human sk, human kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    // Pass the pinned JSON as a literal text cast — tokio-postgres has no built-in
    // jsonb binding without the with-serde_json feature; embedding the literal in
    // the SQL string is the zero-dependency workaround.
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"epoch-a\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build an agent-authored EventBody. `resp_kid` (if Some) adds a contributor
/// carrying a `responsibility` key for THAT key (the v_bears attestation trigger on
/// an additive event) beside the agent's own plain contributor — since #195 the
/// responsibility-holder must be the verified attester, so accept-path tests pass the
/// human's kid and the binding-refusal tests pass a different one. `target` (if Some)
/// is written as payload.target_event_id (suppress target).
fn body(
    event_type: &str,
    patient: Uuid,
    kid_a: &str,
    resp_kid: Option<&str>,
    target: Option<&str>,
) -> EventBody {
    let contrib = match resp_kid {
        Some(rk) => serde_json::json!([
            {"actor_id": kid_a, "role": "triaged"},
            {"actor_id": rk, "role": "attested", "responsibility": {"held_by": rk}}
        ]),
        None => serde_json::json!([{"actor_id": kid_a, "role": "triaged"}]),
    };
    let payload = match target {
        Some(t) => serde_json::json!({ "target_event_id": t }),
        None => serde_json::json!({ "text": "seen, stable" }),
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: "advisory/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "agent".into(),
        },
        t_effective: None,
        signer_key_id: kid_a.into(),
        contributors: contrib,
        payload,
        attachments: vec![],
        plaintext_twin: None,
    }
}

#[tokio::test]
async fn accepts_responsibility_bearing_additive_event_with_valid_human_token() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // P1: a note.added carrying `responsibility` triggers the attestation gate on an
    // additive event (no target/provenance machinery) — isolates the accept.
    let b = body("note.added", patient, &kid_a, Some(&kid_h), None);
    let signed = sign(&b, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk_h])
        .await;
    assert!(r.is_ok(), "valid human attestation must be accepted: {r:?}");
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='note.added'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the attested event is appended");
}

#[tokio::test]
async fn accepts_suppressing_event_with_valid_human_token() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Baseline additive note (no token) to be the suppress target — step-5 needs it.
    let baseline = body("note.added", patient, &kid_a, None, None);
    let baseline_signed = sign(&baseline, &sk_a).unwrap();
    c.execute("SELECT submit_event($1)", &[&baseline_signed.signed_bytes])
        .await
        .unwrap();

    // P2: salience.downgrade (suppressing) targeting the baseline, human-attested.
    let supp = body(
        "salience.downgrade",
        patient,
        &kid_a,
        None,
        Some(&baseline.event_id),
    );
    let supp_signed = sign(&supp, &sk_a).unwrap();
    let ca = event_address(&supp_signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c
        .execute(SUBMIT3, &[&supp_signed.signed_bytes, &token, &vk_h])
        .await;
    assert!(
        r.is_ok(),
        "valid human-attested suppress must be accepted: {r:?}"
    );
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='salience.downgrade'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the suppressing event is appended");
}

#[tokio::test]
async fn rejects_bad_attestations_and_keeps_the_floor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // One baseline target + one suppress event reused across all rejections (none
    // append, so there is no idempotency interaction).
    let baseline = body("note.added", patient, &kid_a, None, None);
    let baseline_signed = sign(&baseline, &sk_a).unwrap();
    c.execute("SELECT submit_event($1)", &[&baseline_signed.signed_bytes])
        .await
        .unwrap();
    let supp = body(
        "salience.downgrade",
        patient,
        &kid_a,
        None,
        Some(&baseline.event_id),
    );
    let supp_signed = sign(&supp, &sk_a).unwrap();
    let ca = event_address(&supp_signed.signed_bytes);
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    let vk_a = sk_a.verifying_key().to_bytes().to_vec();

    // N1: a valid human token bound to a DIFFERENT event's address.
    let wrong = sign_attestation(
        &event_address(b"a different event"),
        &kid_h,
        "attested",
        &sk_h,
    )
    .unwrap();
    let r = c
        .execute(SUBMIT3, &[&supp_signed.signed_bytes, &wrong, &vk_h])
        .await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(
        e.contains("not bound to this event"),
        "N1 wrong-address: {e}"
    );

    // N2: a valid token with one byte flipped (signature no longer verifies).
    // N1 and N2 deliberately assert the SAME needle: the gate (db/005:88) emits one
    // message — "invalid or not bound to this event" — for both a bad signature and a
    // wrong binding, since cairn_attestation_ok conflates them into a single bool. The
    // two cases are therefore distinguished by construction here, not by the error.
    let good = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let mut tampered = good.clone();
    let m = tampered.len() / 2;
    tampered[m] ^= 0x01;
    let r = c
        .execute(SUBMIT3, &[&supp_signed.signed_bytes, &tampered, &vk_h])
        .await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(e.contains("not bound to this event"), "N2 tampered: {e}");

    // N3: a VALID token, correctly bound, but the attester is an enrolled AGENT,
    // not a human (gate check #3, db/005:88-91).
    let agent_tok = sign_attestation(&ca, &kid_a, "attested", &sk_a).unwrap();
    let r = c
        .execute(SUBMIT3, &[&supp_signed.signed_bytes, &agent_tok, &vk_a])
        .await;
    let e = format!("{:?}", r.unwrap_err());
    assert!(
        e.contains("not an enrolled human actor"),
        "N3 non-human attester: {e}"
    );

    // The floor held: not one suppressing event was appended.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='salience.downgrade'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "no rejected suppress leaked into the log");
}

// ---------------------------------------------------------------------------
// Issue #195 (finding A7) — the responsibility claim is BOUND to the attester.
//
// The token proves *some* enrolled human vouched for these bytes, but the signed
// body could name responsibility for a DIFFERENT actor — the immutable record then
// permanently carries an unverified responsibility claim about a person who never
// touched the event. Every production flow (apply_proposal, medication attestation,
// identify --link) already names the attester as the responsibility-bearing
// contributor, so the gate now enforces it: contributor claims `responsibility` ⇒
// its actor_id IS the verified attester key.
// ---------------------------------------------------------------------------

/// A body claiming responsibility for someone OTHER than the verified attester
/// (here: the agent names ITSELF while the human vouches) is refused legibly.
#[tokio::test]
async fn responsibility_claim_not_bound_to_attester_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // The agent signs a body naming the AGENT's own key as responsibility-holder,
    // while the HUMAN's token accompanies it.
    let b = body("note.added", patient, &kid_a, Some(&kid_a), None); // responsibility = kid_a
    let signed = sign(&b, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c
        .execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk_h])
        .await;
    assert!(
        r.is_err(),
        "a responsibility claim about a non-attester must be refused (kid_a={kid_a}, attester={kid_h})"
    );
    let m = r
        .unwrap_err()
        .as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_default();
    assert!(
        m.contains("responsibility"),
        "the refusal must name the unbound responsibility claim, got: {m}"
    );
}

/// The SAME binding at the remote apply door (principle 12).
#[tokio::test]
async fn remote_apply_responsibility_claim_not_bound_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let b = body("note.added", patient, &kid_a, Some(&kid_a), None); // responsibility = kid_a
    let signed = sign(&b, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();

    let r = c
        .execute(
            "SELECT apply_remote_event($1,$2,$3)",
            &[&signed.signed_bytes, &token, &vk_h],
        )
        .await;
    assert!(
        r.is_err(),
        "the apply door must refuse the unbound responsibility claim identically"
    );
}
