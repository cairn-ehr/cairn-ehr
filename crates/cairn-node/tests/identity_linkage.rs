//! Integration coverage for the §5.1/§5.7 identity linkage core (db/018): the
//! link/unlink event types, the structural floor, the patient_link edge overlay,
//! the person_member connected-component projection, and the person_chart
//! unified-read VIEW. Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`. The advisory matcher (§5.2, Python)
//! and the proposal→apply seam (C2) are separate subsystems, NOT exercised here.
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement. `tokio_postgres::Error`'s
/// Display renders only as the literal "db error" — the actual `RAISE EXCEPTION`
/// message lives in the DbError payload — so every floor-rejection assertion must go
/// through here (project convention: see `twin_globalise.rs`, `admission.rs`).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical + linkage tables and enroll one agent signer. Returns (sk, kid).
/// `patient_link` / `person_member` are created by LATER sections of db/018 (Tasks 3–4),
/// so they are truncated behind a `to_regclass` guard — this keeps the single `setup()`
/// helper correct at every stage as the migration grows, with no cross-task edits.
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Sign + submit one link OR unlink event through the real submit_event door.
/// `wall` is the HLC wall clock (higher = newer); patient_id is set to subject_a
/// by convention (an identity event is "about" subject_a's linkage). Returns the
/// submit result so a test can assert acceptance or a specific rejection.
async fn submit_link(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    a: Uuid,
    b: Uuid,
    wall: i64,
    is_link: bool,
) -> Result<u64, tokio_postgres::Error> {
    submit_link_prov(c, sk, kid, a, b, wall, is_link, "matcher:cfg@test").await
}

/// As `submit_link`, but with an explicit provenance string (for the empty-provenance
/// rejection test). Sends the provenance verbatim; an empty string must be rejected.
#[allow(clippy::too_many_arguments)] // mirrors the `submit` test helper in match_veto.rs
async fn submit_link_prov(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    a: Uuid,
    b: Uuid,
    wall: i64,
    is_link: bool,
    prov: &str,
) -> Result<u64, tokio_postgres::Error> {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: prov,
        confidence: None,
    };
    let (etype, sver, payload, twin) = if is_link {
        (
            "identity.link.asserted",
            "identity.link/1",
            link_assertion_body(&la),
            render_link_twin(&la),
        )
    } else {
        (
            "identity.unlink.asserted",
            "identity.unlink/1",
            unlink_assertion_body(&la),
            render_unlink_twin(&la),
        )
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

#[tokio::test]
async fn valid_link_is_accepted() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true)
        .await
        .expect("valid link accepted");
}

#[tokio::test]
async fn self_link_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let a = Uuid::now_v7();
    let err = submit_link(&c, &sk, &kid, a, a, 100, true)
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("self-link"),
        "self-link must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn empty_provenance_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let err = submit_link_prov(&c, &sk, &kid, a, b, 100, true, "   ")
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("provenance"),
        "empty provenance must be refused: {}",
        db_msg(&err)
    );
}

/// Read the standing edge state for a pair, or None if no edge row exists.
/// Uses `$1::text::uuid` (project convention, see `match_veto.rs`) since
/// tokio-postgres in this project has no uuid `ToSql` feature enabled.
async fn edge_state(c: &Client, a: Uuid, b: Uuid) -> Option<String> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let (lo_s, hi_s) = (lo.to_string(), hi.to_string());
    let row = c
        .query_opt(
            "SELECT state FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo_s, &hi_s],
        )
        .await
        .unwrap();
    row.map(|r| r.get::<_, String>(0))
}

#[tokio::test]
async fn link_creates_a_standing_edge() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    assert_eq!(edge_state(&c, a, b).await.as_deref(), Some("link"));
}

#[tokio::test]
async fn newer_unlink_overlays_older_link() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap(); // link @100
    submit_link(&c, &sk, &kid, a, b, 200, false).await.unwrap(); // unlink @200 (newer)
    assert_eq!(edge_state(&c, a, b).await.as_deref(), Some("unlink"));
}

#[tokio::test]
async fn older_link_does_not_overlay_newer_unlink() {
    // Out-of-order arrival must converge: the highest-HLC assertion wins regardless
    // of the order events land (offline-first set-union).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 200, false).await.unwrap(); // unlink @200 lands first
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap(); // link @100 lands later (older)
    assert_eq!(
        edge_state(&c, a, b).await.as_deref(),
        Some("unlink"),
        "older link must not overlay a newer unlink"
    );
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Build a link event with NO authored twin — the identity floor HARD-requires one.
    let (a_s, b_s) = (a.to_string(), b.to_string());
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "p",
        confidence: None,
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: "identity.link.asserted".into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc {
            wall: 100,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("authored twin"),
        "twin-less identity event must be refused: {}",
        db_msg(&err)
    );
}

/// The person_id a UUID currently projects to, or None if it has no person_member row.
/// UUIDs are passed as text and cast in SQL (`$1::text::uuid`) and read back via
/// `::text` — this project's tokio-postgres has no uuid ToSql/FromSql (project
/// convention: see `match_veto.rs`).
async fn person_of(c: &Client, p: Uuid) -> Option<Uuid> {
    let p_s = p.to_string();
    c.query_opt(
        "SELECT person_id::text FROM person_member WHERE patient_id = $1::text::uuid",
        &[&p_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0).parse().unwrap())
}

#[tokio::test]
async fn linked_pair_shares_min_uuid_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    let expected = a.min(b);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
}

#[tokio::test]
async fn transitive_links_form_one_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, d, 110, true).await.unwrap();
    let expected = a.min(b).min(d);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    assert_eq!(person_of(&c, d).await, Some(expected));
}

#[tokio::test]
async fn diamond_unlink_stays_merged() {
    // A-B, B-C, A-C all linked; unlink A-B. Still connected via A-C-B → one person.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, cc, 120, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 130, false).await.unwrap(); // unlink A-B (not a bridge)
    let expected = a.min(b).min(cc);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    assert_eq!(person_of(&c, cc).await, Some(expected));
}

#[tokio::test]
async fn chain_unlink_splits_component() {
    // Chain A-B-C; unlink A-B (a bridge) → {A} and {B,C}.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 120, false).await.unwrap(); // unlink the A-B bridge
    assert_eq!(
        person_of(&c, a).await,
        Some(a),
        "A now isolated → maps to itself"
    );
    let bc = b.min(cc);
    assert_eq!(person_of(&c, b).await, Some(bc));
    assert_eq!(person_of(&c, cc).await, Some(bc));
}

#[tokio::test]
async fn re_link_is_idempotent() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 105, true).await.unwrap(); // a second, later link of the same pair
    let expected = a.min(b);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    let n: i64 = c
        .query_one("SELECT count(*) FROM patient_link WHERE state='link'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "re-linking the same pair is one standing edge, not two"
    );
}

/// Submit a minimal patient.created so the patient has a patient_chart row to union.
async fn submit_patient_created(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "patient.created".into(),
        schema_version: "patient/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
        attachments: vec![],
        plaintext_twin: None, // non-demographic type → honest-degrade skeleton (db/015)
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("patient.created accepted");
}

#[tokio::test]
async fn person_chart_unions_member_streams() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_patient_created(&c, &sk, &kid, a, 100).await;
    submit_patient_created(&c, &sk, &kid, b, 101).await;
    submit_link(&c, &sk, &kid, a, b, 110, true).await.unwrap();
    let person = a.min(b).to_string();
    // Selecting by the shared person_id returns BOTH member charts.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM person_chart WHERE person_id = $1::text::uuid",
            &[&person],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 2,
        "person_chart must union both member UUIDs' chart rows under one person_id"
    );
}

#[tokio::test]
async fn person_chart_defaults_unlinked_to_self() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let a = Uuid::now_v7();
    submit_patient_created(&c, &sk, &kid, a, 100).await; // never linked → no person_member row
    let a_s = a.to_string();
    let pid: String = c
        .query_one(
            "SELECT person_id::text FROM person_chart WHERE patient_id = $1::text::uuid",
            &[&a_s],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        pid, a_s,
        "a UUID unknown to the link graph is its own person"
    );
}

#[tokio::test]
async fn oversize_component_guard_rejects() {
    // With a tiny cap, the link that would grow the component past it is refused
    // wholesale (fail-loud, never a silent cap). Cap is a per-session GUC.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    c.batch_execute("SET cairn.max_component_size = 3")
        .await
        .unwrap();
    let (a, b, cc, d) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap(); // {A,B} size 2 — ok
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap(); // {A,B,C} size 3 — ok
    let err = submit_link(&c, &sk, &kid, cc, d, 120, true)
        .await
        .unwrap_err(); // size 4 — refuse
    assert!(
        db_msg(&err).contains("exceeds max size"),
        "oversize component must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn component_at_exactly_cap_is_accepted() {
    // The guard is strictly-greater (`> cap`), so a component of exactly `cap` members
    // is accepted. This pins the boundary against a future `>=` regression that would
    // wrongly reject a legitimate at-cap component.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    c.batch_execute("SET cairn.max_component_size = 3")
        .await
        .unwrap();
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap(); // {A,B} size 2
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap(); // {A,B,C} size 3 == cap — accepted
    let expected = a.min(b).min(cc);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    assert_eq!(
        person_of(&c, cc).await,
        Some(expected),
        "a component of exactly cap members is accepted"
    );
}
