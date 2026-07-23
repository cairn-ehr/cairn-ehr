//! Integration coverage for §5.5(a)/§5.7 `repudiate` + the known-alias pool (db/025):
//! the FIRST *suppressing* identity event. A fabricated-persona name marked known-false
//! leaves the display winner (patient_name_current anti-joins the name_repudiation overlay),
//! stays in db/012's retained set as evidence, and surfaces to the matcher as a reusable
//! alias (patient_alias_pool). suppressing-mode forces the db/005 human-attestation gate
//! (§5.7 "Human"). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`. Fuses the name-assertion harness (demographics_names.rs) with
//! the human-attestation harness (attestation.rs).
use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_event::identity::{
    render_repudiate_twin, repudiation_assertion_body, RepudiationAssertion,
};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement (see identity_dispute.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical + identity tables; enroll one agent signer + one human attester
/// (distinct keys — a suppressing repudiation needs a human token). Returns
/// (agent sk, agent kid, human sk, human kid). name_repudiation is created by db/025, so
/// it is truncated behind a to_regclass guard (the identity_identify.rs pattern).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.name_repudiation') IS NOT NULL THEN TRUNCATE name_repudiation; END IF; \
         END $$;").await.unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Submit one name (demographic.field.asserted, field=name) — the retained-set feeder.
#[allow(clippy::too_many_arguments)]
async fn submit_name(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    value: &str,
    use_: &str,
    prov: &str,
) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: name_assertion_body(value, Some(use_), prov),
        attachments: vec![],
        plaintext_twin: Some(render_name_twin(value, Some(use_), prov)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("name assertion accepted");
}

/// Build a repudiation EventBody (agent-signed). `value`/`reason` are passed verbatim so a
/// test can drive the floor with an empty string; `twin` toggles the authored twin.
fn repudiation_body(
    kid_a: &str,
    subject: Uuid,
    value: &str,
    reason: &str,
    twin: bool,
) -> EventBody {
    let s_s = subject.to_string();
    let a = RepudiationAssertion {
        subject: &s_s,
        value,
        reason,
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(), // a repudiation is "about" its subject's chart
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall: 100,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid_a.into(),
        contributors: serde_json::json!([{"actor_id": kid_a, "role": "recorded"}]),
        payload: repudiation_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: if twin {
            Some(render_repudiate_twin(&a))
        } else {
            None
        },
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// Submit a repudiation WITH a valid human attestation token (the normal, accepted path),
/// at a caller-chosen HLC wall so overlay recency can be exercised.
#[allow(clippy::too_many_arguments)]
async fn repudiate(
    c: &Client,
    sk_a: &SigningKey,
    kid_a: &str,
    sk_h: &SigningKey,
    kid_h: &str,
    subject: Uuid,
    value: &str,
    reason: &str,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    let mut body = repudiation_body(kid_a, subject, value, reason, true);
    body.hlc.wall = wall;
    let signed = sign(&body, sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, kid_h, "attested", sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk_h],
    )
    .await
}

async fn current_name(c: &Client, p: Uuid) -> Option<String> {
    let p_str = p.to_string();
    c.query_opt(
        "SELECT value FROM patient_name_current WHERE patient_id::text=$1",
        &[&p_str],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

async fn name_count(c: &Client, p: Uuid) -> i64 {
    let p_str = p.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_name WHERE patient_id::text=$1",
        &[&p_str],
    )
    .await
    .unwrap()
    .get(0)
}

/// Is (patient, value) in the matcher-facing alias pool? The view is deliberately
/// reason-free (ADR-0006 — no forensic free-text on the name-searchable cross-patient
/// surface), so the pool check is presence-only.
async fn in_alias_pool(c: &Client, p: Uuid, value: &str) -> bool {
    let p_str = p.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_alias_pool WHERE patient_id::text=$1 AND value=$2",
        &[&p_str, &value],
    )
    .await
    .unwrap()
    .get::<_, i64>(0)
        == 1
}

/// The struck value's `reason`, read from the base overlay (where it is confined — the
/// alias pool view does not expose it). None if no overlay row exists.
async fn repudiation_reason(c: &Client, p: Uuid, value: &str) -> Option<String> {
    let p_str = p.to_string();
    c.query_opt(
        "SELECT reason FROM name_repudiation WHERE subject::text=$1 AND value=$2",
        &[&p_str, &value],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

// --- the strike-through: display suppression + evidence retention + alias pool ---

#[tokio::test]
async fn repudiated_name_leaves_the_winner_but_a_surviving_name_takes_over() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    // A fabricated legal name (newer, so it would win) + a true alias (older).
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        2,
        "John Smith",
        "legal",
        "patient-stated",
    )
    .await;
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        1,
        "Real Alias",
        "alias",
        "patient-stated",
    )
    .await;
    assert_eq!(
        current_name(&c, p).await.as_deref(),
        Some("John Smith"),
        "the fabricated legal name is the winner before repudiation"
    );

    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "John Smith",
        "confessed fabricated persona",
        3,
    )
    .await
    .expect("human-attested repudiation accepted");

    assert_eq!(
        current_name(&c, p).await.as_deref(),
        Some("Real Alias"),
        "the struck name leaves the winner; the surviving name takes over"
    );
    // Evidence retention: the struck name is NOT deleted from the retained set (principle 1).
    assert_eq!(
        name_count(&c, p).await,
        2,
        "the struck name stays in the retained set as evidence"
    );
    // Alias pool: the struck value is now a reusable known alias for the matcher (presence
    // only — the pool view is reason-free); the forensic reason stays in the base overlay.
    assert!(
        in_alias_pool(&c, p, "John Smith").await,
        "the struck name enters the known-alias pool"
    );
    assert_eq!(
        repudiation_reason(&c, p, "John Smith").await.as_deref(),
        Some("confessed fabricated persona"),
        "the reason is retained in the base overlay (confined; not on the matcher view)"
    );
    // Confidentiality split: the matcher-facing view must NOT carry `reason` (ADR-0006).
    let has_reason: bool = c
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_name='patient_alias_pool' AND column_name='reason')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !has_reason,
        "patient_alias_pool must not expose the forensic reason free-text"
    );
}

#[tokio::test]
async fn repudiating_the_only_name_yields_no_winner_not_a_lie() {
    // §5.5(a)/principle 4: if the chart's ONLY name is the false one, the winner is EMPTY —
    // honest (name genuinely unknown-now); the §5.4 callsign/unconfirmed rendering handles
    // "show something" one layer up, never by displaying the known-false name.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        1,
        "Jane Doe",
        "legal",
        "patient-stated",
    )
    .await;
    assert_eq!(current_name(&c, p).await.as_deref(), Some("Jane Doe"));
    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "Jane Doe",
        "fabricated",
        2,
    )
    .await
    .unwrap();
    assert_eq!(
        current_name(&c, p).await,
        None,
        "no surviving name → no winner (honest), never the struck name"
    );
    assert_eq!(
        name_count(&c, p).await,
        1,
        "the struck name is retained as evidence"
    );
}

#[tokio::test]
async fn reassert_is_idempotent_and_reason_is_hlc_latest_wins() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_name(&c, &sk_a, &kid_a, p, 1, "Al Ias", "legal", "patient-stated").await;

    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "Al Ias",
        "first reason",
        10,
    )
    .await
    .unwrap();
    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "Al Ias",
        "refined reason",
        20,
    )
    .await
    .unwrap();
    // An OLDER re-assert must not overwrite the newer standing reason.
    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "Al Ias",
        "stale reason",
        5,
    )
    .await
    .unwrap();

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM name_repudiation WHERE subject::text=$1",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "re-repudiating the same (subject,value) is one standing overlay row"
    );
    assert_eq!(
        repudiation_reason(&c, p, "Al Ias").await.as_deref(),
        Some("refined reason"),
        "the highest-HLC reason wins; an older re-assert does not clobber it"
    );
}

#[tokio::test]
async fn newer_reassertion_does_not_unstrike_a_repudiated_name() {
    // The anti-join is deliberately HLC-BLIND (db/025): a standing repudiation strikes its
    // value even against a STRICTLY-NEWER re-assertion of the same string. Re-typing a
    // known-false name (e.g. from the same old insurance card) must NOT resurrect it — a
    // mistaken repudiation is undone only by an explicit reversal event (deferred), never by
    // re-assertion. This test PINS that intended semantics so a future HLC-aware change (or a
    // reversal slice) has to flip it consciously.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        3,
        "John Smith",
        "legal",
        "patient-stated",
    )
    .await;
    repudiate(
        &c,
        &sk_a,
        &kid_a,
        &sk_h,
        &kid_h,
        p,
        "John Smith",
        "believed fabricated",
        5,
    )
    .await
    .unwrap();
    assert_eq!(
        current_name(&c, p).await,
        None,
        "the struck sole name leaves the winner"
    );
    // A strictly-newer re-assertion of the SAME value (wall=10 > the repudiation's 5).
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        10,
        "John Smith",
        "legal",
        "document-verified",
    )
    .await;
    assert_eq!(current_name(&c, p).await, None,
               "a newer re-assertion does NOT un-strike a standing repudiation (reversal is the only recourse)");
    assert_eq!(
        name_count(&c, p).await,
        1,
        "the re-assertion updates the one retained member, still struck"
    );
}

// --- the §5.7 "Human" floor: suppressing-mode forces attestation ---

#[tokio::test]
async fn unattested_repudiation_is_refused() {
    // The whole point of suppressing-mode: no token → the db/005 gate refuses it. This is
    // §5.7's "Human" enforced structurally, not as workflow policy.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        1,
        "No Token",
        "legal",
        "patient-stated",
    )
    .await;

    // submit_event WITHOUT the token/attester params — the 1-arg door.
    let body = repudiation_body(&kid_a, p, "No Token", "no human vouched", true);
    let signed = sign(&body, &sk_a).unwrap();
    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("requires attestation"),
        "an un-attested suppressing repudiation must be refused: {}",
        db_msg(&err)
    );
    // The floor held: the name still displays (nothing was struck).
    assert_eq!(current_name(&c, p).await.as_deref(), Some("No Token"));
}

#[tokio::test]
async fn agent_attested_repudiation_is_refused() {
    // The "Human" in §5.7 is specifically ENFORCED: a valid, correctly-bound token from an
    // enrolled AGENT (not a human) is refused (db/005 gate check #3). Without this, an agent
    // could self-attest a suppressing repudiation and strike a clinical name with no human
    // vouching — the exact failure the suppressing-mode gate exists to prevent. (attestation.rs
    // proves this generically for salience.downgrade; here it is pinned for repudiate itself,
    // so a future identity-type special-case that skipped the human check would fail HERE.)
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_name(
        &c,
        &sk_a,
        &kid_a,
        p,
        1,
        "Agent Vouch",
        "legal",
        "patient-stated",
    )
    .await;

    // A valid token, correctly bound to THIS event — but signed by the AGENT key, and the
    // agent's own verifying key presented as attester.
    let body = repudiation_body(&kid_a, p, "Agent Vouch", "agent self-attested", true);
    let signed = sign(&body, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let agent_tok = sign_attestation(&ca, &kid_a, "attested", &sk_a).unwrap();
    let vk_a = sk_a.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(
            "SELECT submit_event($1,$2,$3)",
            &[&signed.signed_bytes, &agent_tok, &vk_a],
        )
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("not an enrolled human actor"),
        "an agent-attested repudiation must be refused (§5.7 'Human'): {}",
        db_msg(&err)
    );
    assert_eq!(
        current_name(&c, p).await.as_deref(),
        Some("Agent Vouch"),
        "nothing was struck"
    );
}

// --- structural floor rejections (each a distinct legible exception) ---

#[tokio::test]
async fn empty_value_is_rejected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let err = repudiate(&c, &sk_a, &kid_a, &sk_h, &kid_h, p, "   ", "reason", 1)
        .await
        .unwrap_err();
    let m = db_msg(&err);
    assert!(m.contains("value"), "empty value must be refused: {m}");
    assert!(
        !m.contains("attestation"),
        "must reject at the FLOOR, not the attestation gate: {m}"
    );
}

#[tokio::test]
async fn empty_reason_is_rejected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let err = repudiate(&c, &sk_a, &kid_a, &sk_h, &kid_h, p, "A Name", "", 1)
        .await
        .unwrap_err();
    let m = db_msg(&err);
    assert!(m.contains("reason"), "empty reason must be refused: {m}");
    assert!(
        !m.contains("attestation"),
        "must reject at the FLOOR, not the attestation gate: {m}"
    );
}

#[tokio::test]
async fn bad_subject_is_rejected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    // A payload whose subject is not a uuid must be a legible reject, not a crash.
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid_a.clone(),
        contributors: serde_json::json!([{"actor_id": kid_a, "role": "recorded"}]),
        payload: serde_json::json!({"subject": "not-a-uuid", "value": "X", "reason": "r"}),
        attachments: vec![],
        plaintext_twin: Some("name repudiated: x — \"X\" (r)".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(
            "SELECT submit_event($1,$2,$3)",
            &[&signed.signed_bytes, &token, &vk_h],
        )
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("subject"),
        "a non-uuid subject must be refused legibly: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    // The identity floor HARD-requires an authored twin (legible-critical).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let body = repudiation_body(&kid_a, p, "Twinless", "reason", false); // no authored twin
    let signed = sign(&body, &sk_a).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk_h = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute(
            "SELECT submit_event($1,$2,$3)",
            &[&signed.signed_bytes, &token, &vk_h],
        )
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("authored twin"),
        "a twin-less repudiation must be refused: {}",
        db_msg(&err)
    );
}
