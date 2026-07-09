//! ADR-0043 / issue #99 — the suppression owner-gate: a suppressing overlay
//! (salience.downgrade / visibility.suppress) that forecloses on a HUMAN author's
//! event is self-only. Cross-human suppression is refused at BOTH write doors;
//! agent-authored / un-owned advisories stay dismissable (clinician-overrides-machine,
//! principle 10). Real Postgres, gated on $CAIRN_TEST_PG, serialized cluster-wide.
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

const SUBMIT1: &str = "SELECT submit_event($1)";
const SUBMIT3: &str = "SELECT submit_event($1,$2,$3)";

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention, see identity_linkage.rs
/// / apply_remote_event.rs's db_msg). try_suppress's bare `e.to_string()` in the brief's
/// first draft only ever produced "db error", losing the RAISE message the cross_human_*
/// tests assert on; this is the same fix apply_remote_event.rs already applies.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Enroll one agent signer + two distinct human actors (A the author, B the
/// would-be cross-author suppressor). Returns their (sk, kid) pairs.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String, SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    let (sk_ag, kid_ag) = generate_key().unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_b, kid_b) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"epoch-a\"}', $1)",
        &[&kid_ag],
    ).await.unwrap();
    // NOTE: the pinned determinant set (not the signing key) is what cairn_actor_id
    // hashes into actor_id (extensions/cairn_pgx: cairn_actor_id = canonical_json_
    // address(pinned)). Two humans enrolled with the IDENTICAL pinned JSON collide
    // into the SAME actor_id; actor_current's `DISTINCT ON (actor_id)` then keeps
    // only the latest-enrolled signing key for that actor_id, silently un-enrolling
    // the other (a real bug this test's first draft hit — both cross-author AND
    // same-author cases failed with "signer is not an enrolled actor"). A distinct
    // `actor` tag keeps A and B's pinned sets — and therefore their actor_ids —
    // genuinely distinct, as two real clinicians would be.
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"A\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"B\"}', $1)",
        &[&kid_b],
    )
    .await
    .unwrap();
    (sk_ag, kid_ag, sk_a, kid_a, sk_b, kid_b)
}

/// Minimal EventBody. `signer_kid` sets signer_key_id; `target` becomes
/// payload.target_event_id; `responsibility` adds a responsibility-bearing contributor.
fn body(
    event_type: &str,
    patient: Uuid,
    signer_kid: &str,
    responsibility: bool,
    target: Option<&str>,
) -> EventBody {
    let contrib = if responsibility {
        serde_json::json!([{"actor_id": signer_kid, "role": "attested", "responsibility": "attested"}])
    } else {
        serde_json::json!([{"actor_id": signer_kid, "role": "author"}])
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
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: signer_kid.into(),
        contributors: contrib,
        payload,
        attachments: vec![],
        plaintext_twin: None,
    }
}

/// Author a plain additive note (no attestation) and return its event_id.
async fn author_note(c: &Client, patient: Uuid, signer_kid: &str, sk: &SigningKey) -> String {
    let b = body("note.added", patient, signer_kid, false, None);
    let s = sign(&b, sk).unwrap();
    c.execute(SUBMIT1, &[&s.signed_bytes]).await.unwrap();
    b.event_id
}

/// Try to submit a human-attested suppress of `target`. Returns Ok(()) on accept.
/// The suppressor signs their own suppress event AND self-attests it (the realistic
/// case: a human authoring a suppress signs it and vouches for it), so one (kid, sk)
/// actor pair drives both the signature and the attestation token.
async fn try_suppress(
    c: &Client,
    patient: Uuid,
    event_type: &str,
    actor_kid: &str,
    actor_sk: &SigningKey,
    target: &str,
) -> Result<(), String> {
    let supp = body(event_type, patient, actor_kid, false, Some(target));
    let signed = sign(&supp, actor_sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, actor_kid, "attested", actor_sk).unwrap();
    let vk = actor_sk.verifying_key().to_bytes().to_vec();
    c.execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .map(|_| ())
        .map_err(|e| db_msg(&e))
}

#[tokio::test]
async fn self_suppression_by_human_signer_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Human A signs a note, then A downgrades A's own note.
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &tgt).await;
    assert!(
        r.is_ok(),
        "author suppressing their own event must be accepted: {r:?}"
    );
}

#[tokio::test]
async fn cross_human_salience_downgrade_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Human A authors; human B tries to downgrade A's note.
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_b, &sk_b, &tgt).await;
    assert!(r.is_err(), "cross-human downgrade must be refused");
    assert!(
        r.unwrap_err().contains("cross-author suppression refused"),
        "legible reason"
    );
}

#[tokio::test]
async fn cross_human_visibility_suppress_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "visibility.suppress", &kid_b, &sk_b, &tgt).await;
    assert!(r.is_err(), "cross-human hide must be refused");
    assert!(
        r.unwrap_err().contains("cross-author suppression refused"),
        "legible reason"
    );
}

#[tokio::test]
async fn self_suppression_by_human_attester_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_ag, kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Target: an AGENT-signed note that human A vouches for (responsibility) — so the
    // target's ONLY human author is the attester A (attester_key = A, signer = agent).
    let b = body("note.added", p, &kid_ag, true, None);
    let signed = sign(&b, &sk_ag).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_a, "attested", &sk_a).unwrap();
    let vk_a = sk_a.verifying_key().to_bytes().to_vec();
    c.execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk_a])
        .await
        .unwrap();
    // A (the human author-of-record) may suppress it.
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &b.event_id).await;
    assert!(
        r.is_ok(),
        "the human attester-of-record may suppress: {r:?}"
    );
}

#[tokio::test]
async fn cross_human_suppress_refused_at_apply_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Human A authors a note through the LOCAL door (submit_event) — establishes
    // A as the target's sole human author-of-record.
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    // Human B's cross-human salience.downgrade of A's note, pushed through the
    // REMOTE-APPLY door (apply_remote_event) exactly as a synced event would arrive:
    // signed bytes + attestation token + attester key travel together (call shape
    // copied verbatim from apply_remote_event.rs::attested_suppress_applies_and_stores_the_token).
    let supp = body("salience.downgrade", p, &kid_b, false, Some(&tgt));
    let signed = sign(&supp, &sk_b).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_b, "attested", &sk_b).unwrap();
    let hkey = hex::decode(&kid_b).unwrap();
    let r = c
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&signed.signed_bytes, &token, &hkey],
        )
        .await
        .map(|_| ())
        .map_err(|e| db_msg(&e));
    assert!(
        r.is_err(),
        "a synced cross-human suppress must be refused at apply"
    );
    assert!(
        r.unwrap_err().contains("cross-author suppression refused"),
        "legible ADR-0043 reason"
    );
}

/// Guards Finding 1 of the final review: the helper's signer branch must resolve
/// human-ness from the append-only actor_event HISTORY, not actor_current. A's
/// original signing key drops out of actor_current the moment A "rotates" (a fresh
/// enroll_actor row under the SAME pinned identity, modelling supersede/re-enroll or
/// a departed colleague's key being revoked and replaced). Under the pre-fix helper
/// (queried actor_current) A's original note.added would then have an EMPTY
/// human-author set, flipping the gate open for ANY enrolled human — over-permission
/// on the safety floor. Under the fix, A stays a human author by history, so B is
/// still refused.
#[tokio::test]
async fn cross_human_suppress_refused_after_author_key_rotation() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();

    // Human A signs a plain note through the local door — establishes A as the
    // target's sole human author-of-record under A's ORIGINAL key (kid_a).
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;

    // "Rotate" A's key: enroll a NEW signing key under the IDENTICAL pinned JSON
    // setup() used for A ({"role":"clinician","actor":"A"}). This inserts another
    // `enroll` row with the same actor_id, so actor_current's DISTINCT ON keeps only
    // the NEW key for A's actor_id — A's original kid_a drops out of actor_current
    // but remains in actor_event history.
    let (_sk_a2, kid_a2) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"A\"}', $1)",
        &[&kid_a2],
    )
    .await
    .unwrap();

    // Human B attempts to downgrade A's note, now that A's ORIGINAL authoring key
    // is no longer current (but is still human-by-history in actor_event).
    let r = try_suppress(&c, p, "salience.downgrade", &kid_b, &sk_b, &tgt).await;
    assert!(
        r.is_err(),
        "cross-human downgrade of a departed/rotated author's note must still be refused: {r:?}"
    );
    assert!(
        r.unwrap_err().contains("cross-author suppression refused"),
        "legible ADR-0043 reason"
    );
}

#[tokio::test]
async fn agent_advisory_dismissable_by_any_human() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_ag, kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Target: an agent-authored, un-owned note (no human author). Human A dismisses it.
    let tgt = author_note(&c, p, &kid_ag, &sk_ag).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &tgt).await;
    assert!(
        r.is_ok(),
        "an agent advisory must be dismissable by any enrolled human: {r:?}"
    );
}
