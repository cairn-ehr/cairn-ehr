//! Integration coverage for §5.4/§5.7 identity-pending + `identify` + the *unconfirmed*
//! trust state (db/024): the pending/identify event types, the structural floor, the
//! chart_identity_state standing overlay, and the reworked chart_trust projection that
//! composes under-review (dispute) over unconfirmed (pending) by highest severity. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`.
//! Mirrors `identity_dispute.rs` (C3).
use cairn_event::identity::{
    dispute_assertion_body, dispute_resolution_body, identify_assertion_body,
    pending_assertion_body, render_dispute_resolved_twin, render_dispute_twin,
    render_identify_twin, render_pending_twin, DisputeAssertion, DisputeResolution,
    IdentifyAssertion, PendingAssertion,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// The Postgres error message text for a failed statement (project convention: see
/// `identity_dispute.rs` — `tokio_postgres::Error`'s Display renders only "db error").
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical + identity tables and enroll one agent signer. Returns (sk, kid).
/// `chart_identity_state` / `chart_dispute` are created by db/024 / db/023, so they are
/// truncated behind a `to_regclass` guard — keeping the single `setup()` helper correct
/// even on a DB migrated only to an earlier stage (the identity_dispute.rs pattern).
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE")
        .await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
         END $$;")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Sign + submit one identity-pending OR identify event through the real submit_event door.
/// `wall` is the HLC wall clock (higher = newer). Returns the submit result so a test can
/// assert acceptance or a specific rejection. `descriptive` is the basis (pending) or
/// method (identify); passed verbatim so an empty string exercises the floor.
async fn submit_identity_state(
    c: &Client, sk: &SigningKey, kid: &str, subject: Uuid,
    wall: i64, is_pending: bool, descriptive: &str,
) -> Result<u64, tokio_postgres::Error> {
    let s_s = subject.to_string();
    let (etype, sver, payload, twin) = if is_pending {
        let a = PendingAssertion { subject: &s_s, basis: descriptive };
        ("identity.pending.asserted", "identity.pending.asserted/1",
         pending_assertion_body(&a), render_pending_twin(&a))
    } else {
        let a = IdentifyAssertion { subject: &s_s, method: descriptive };
        ("identity.identify.asserted", "identity.identify.asserted/1",
         identify_assertion_body(&a), render_identify_twin(&a))
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(), // an identity-state assertion is "about" its subject's chart
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

/// Convenience: register a subject identity-pending with a canned basis.
async fn mark_pending(
    c: &Client, sk: &SigningKey, kid: &str, subject: Uuid, wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_identity_state(c, sk, kid, subject, wall, true, "unconscious ED arrival, no ID").await
}

/// Convenience: identify a subject with a canned method.
async fn identify(
    c: &Client, sk: &SigningKey, kid: &str, subject: Uuid, wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_identity_state(c, sk, kid, subject, wall, false, "driver's licence").await
}

/// Submit one dispute-open OR dispute-resolve event (reused for the compose/precedence
/// test — the C4 slice must prove chart_trust ranks an open dispute over a pending chart).
async fn submit_dispute(
    c: &Client, sk: &SigningKey, kid: &str, dispute_id: Uuid, subject: Uuid,
    wall: i64, is_open: bool,
) -> Result<u64, tokio_postgres::Error> {
    let (d_s, s_s) = (dispute_id.to_string(), subject.to_string());
    let (etype, sver, payload, twin) = if is_open {
        let d = DisputeAssertion { dispute_id: &d_s, subject: &s_s, reason: "suspected identity theft" };
        ("identity.dispute.asserted", "identity.dispute.asserted/1",
         dispute_assertion_body(&d), render_dispute_twin(&d))
    } else {
        let d = DisputeResolution { dispute_id: &d_s, subject: &s_s, resolution: "dismissed — no evidence" };
        ("identity.dispute.resolved", "identity.dispute.resolved/1",
         dispute_resolution_body(&d), render_dispute_resolved_twin(&d))
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

/// The standing identity state of a chart, or None if no row exists.
async fn identity_state(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid", &[&s_s],
    ).await.unwrap().map(|r| r.get::<_, String>(0))
}

/// The effective trust state chart_trust reports for a subject, or None (== confirmed).
async fn trust_of(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid", &[&s_s],
    ).await.unwrap().map(|r| r.get::<_, String>(0))
}

/// person_chart_trust.trust_state for a given patient_id row (the C3 unified read composed
/// on top of C1's person_chart — a chart read, so it lists a subject only once its chart
/// has synced; chart_trust is the authoritative pre-sync safety signal).
async fn person_chart_trust(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM person_chart_trust WHERE patient_id = $1::text::uuid", &[&s_s],
    ).await.unwrap().map(|r| r.get::<_, String>(0))
}

/// Submit a minimal patient.created so the subject has a patient_chart row (so
/// person_chart lists it and its trust_state can be read).
async fn submit_patient_created(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "patient.created".into(),
        schema_version: "patient/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
        attachments: vec![],
        plaintext_twin: None, // non-demographic type → honest-degrade skeleton (db/015)
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
        .expect("patient.created accepted");
}

// --- acceptance + overlay behaviour ---

#[tokio::test]
async fn valid_pending_is_accepted() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    mark_pending(&c, &sk, &kid, subj, 100).await.expect("valid pending accepted");
    assert_eq!(identity_state(&c, subj).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn newer_identify_overlays_pending() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap(); // pending @100
    identify(&c, &sk, &kid, subj, 200).await.unwrap();     // identify @200 (newer)
    assert_eq!(identity_state(&c, subj).await.as_deref(), Some("identified"));
}

#[tokio::test]
async fn older_pending_does_not_reopen_identified() {
    // Out-of-order arrival must converge: an identify that lands before the pending it
    // clears wins by HLC, and the later-arriving-but-older pending does not re-open it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    identify(&c, &sk, &kid, subj, 200).await.unwrap();     // identify @200 lands first
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap(); // pending @100 lands later (older)
    assert_eq!(identity_state(&c, subj).await.as_deref(), Some("identified"),
               "an older pending must not re-open a newer identify");
}

#[tokio::test]
async fn newer_pending_reopens_after_identify() {
    // The overlay is a full lifecycle, not one-way: a mis-identification retracted, the
    // chart re-registered identity-pending with a HIGHER HLC re-opens the unconfirmed state.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap();
    identify(&c, &sk, &kid, subj, 200).await.unwrap();
    mark_pending(&c, &sk, &kid, subj, 300).await.unwrap(); // re-registered pending @300 (newest)
    assert_eq!(identity_state(&c, subj).await.as_deref(), Some("pending"),
               "a newer pending after an identify re-opens the unconfirmed state");
}

#[tokio::test]
async fn reassert_same_pending_is_one_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap();
    mark_pending(&c, &sk, &kid, subj, 105).await.unwrap(); // a second, later pending on the same subject
    let n: i64 = c.query_one("SELECT count(*) FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&subj.to_string()]).await.unwrap().get(0);
    assert_eq!(n, 1, "re-registering the same subject pending is one standing row, not two");
}

// --- the trust-state projection (the unconfirmed state) ---

#[tokio::test]
async fn pending_marks_chart_unconfirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("unconfirmed"));
    assert_eq!(person_chart_trust(&c, subj).await.as_deref(), Some("unconfirmed"),
               "the unified read must surface the unconfirmed trust state");
}

#[tokio::test]
async fn identify_returns_to_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    identify(&c, &sk, &kid, subj, 120).await.unwrap();
    assert_eq!(trust_of(&c, subj).await, None, "identified chart leaves no unconfirmed row");
    assert_eq!(person_chart_trust(&c, subj).await.as_deref(), Some("confirmed"),
               "an identified chart reads confirmed");
}

#[tokio::test]
async fn no_identity_reads_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
    assert_eq!(trust_of(&c, subj).await, None);
    assert_eq!(person_chart_trust(&c, subj).await.as_deref(), Some("confirmed"),
               "the default trust state is confirmed");
}

#[tokio::test]
async fn pending_before_chart_still_unconfirmed() {
    // Offline-first: a pending marker naming a subject with no patient_chart row yet still
    // reports unconfirmed for that subject (the safety signal exists without the body,
    // mirroring §5.9 and C3's dispute-before-chart). person_chart only lists it once the
    // chart arrives, which is correct for a *chart* read.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap(); // no patient.created for subj
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("unconfirmed"),
               "a pending marker reports unconfirmed even before the chart has synced");
    assert_eq!(person_chart_trust(&c, subj).await, None,
               "person_chart lists the chart only once its patient_chart row exists");
}

// --- the C3 ⊔ C4 compose / precedence proof ---

#[tokio::test]
async fn dispute_outranks_pending_then_resolves_and_identifies() {
    // A chart that is BOTH identity-pending AND under an open dispute reads under-review
    // (severity-max: under-review > unconfirmed). Resolving the dispute leaves the pending
    // standing → unconfirmed. A later identify returns it to confirmed. This is the proof
    // that C3's dispute source and C4's pending source compose in one projection.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (subj, d) = (Uuid::now_v7(), Uuid::now_v7());
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("unconfirmed"),
               "pending alone → unconfirmed");
    submit_dispute(&c, &sk, &kid, d, subj, 120, true).await.unwrap(); // open dispute
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("under-review"),
               "an open dispute outranks the pending state → under-review");
    submit_dispute(&c, &sk, &kid, d, subj, 130, false).await.unwrap(); // resolve dispute
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("unconfirmed"),
               "dispute resolved, pending still standing → back to unconfirmed");
    identify(&c, &sk, &kid, subj, 140).await.unwrap();
    assert_eq!(trust_of(&c, subj).await, None,
               "identify clears the last source → confirmed");
    assert_eq!(person_chart_trust(&c, subj).await.as_deref(), Some("confirmed"));
}

// --- structural floor rejections (each a distinct legible exception) ---

#[tokio::test]
async fn empty_basis_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    let err = submit_identity_state(&c, &sk, &kid, subj, 100, true, "   ").await.unwrap_err();
    assert!(db_msg(&err).contains("basis"), "empty basis must be refused: {}", db_msg(&err));
}

#[tokio::test]
async fn empty_method_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    let err = submit_identity_state(&c, &sk, &kid, subj, 100, false, "").await.unwrap_err();
    assert!(db_msg(&err).contains("method"), "empty method must be refused: {}", db_msg(&err));
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let subj = Uuid::now_v7();
    // Build a pending event with NO authored twin — the identity floor HARD-requires one.
    let s_s = subj.to_string();
    let pa = PendingAssertion { subject: &s_s, basis: "b" };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(),
        event_type: "identity.pending.asserted".into(),
        schema_version: "identity.pending.asserted/1".into(),
        hlc: Hlc { wall: 100, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: pending_assertion_body(&pa),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap_err();
    assert!(db_msg(&err).contains("authored twin"),
            "twin-less identity-state event must be refused: {}", db_msg(&err));
}

#[tokio::test]
async fn bad_subject_is_rejected() {
    // A payload whose subject is not a uuid must be a legible reject, not a crash.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "identity.pending.asserted".into(),
        schema_version: "identity.pending.asserted/1".into(),
        hlc: Hlc { wall: 100, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"subject": "not-a-uuid", "basis": "b"}),
        attachments: vec![],
        plaintext_twin: Some("identity pending: x — b".into()),
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap_err();
    assert!(db_msg(&err).contains("subject"),
            "a non-uuid subject must be refused legibly: {}", db_msg(&err));
}

#[tokio::test]
async fn missing_subject_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "identity.pending.asserted".into(),
        schema_version: "identity.pending.asserted/1".into(),
        hlc: Hlc { wall: 100, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"basis": "b"}), // no subject
        attachments: vec![],
        plaintext_twin: Some("identity pending: ? — b".into()),
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap_err();
    assert!(db_msg(&err).contains("subject"),
            "an identity-state assertion with no subject must be refused legibly: {}", db_msg(&err));
}
