//! Integration coverage for the §5.7 `dispute` + chart trust-state projection (db/023):
//! the dispute/resolve event types, the structural floor, the chart_dispute standing
//! overlay, the chart_trust (confirmed / under-review) projection, and its surfacing on
//! the person_chart unified read. Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`. Mirrors `identity_linkage.rs` (C1).
use cairn_event::identity::{
    dispute_assertion_body, dispute_resolution_body, render_dispute_resolved_twin,
    render_dispute_twin, DisputeAssertion, DisputeResolution,
};
use cairn_event::SigningKey;
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{
    cs, db_msg, person_chart_trust, submit_registration, submit_signed, trust_of, EventSpec,
};

/// The identity-overlay table this suite writes to, truncated per test by `common::setup`.
/// Created by db/023, later than the core clinical tables — hence the `to_regclass` guard
/// `setup` applies.
const OVERLAY_TABLES: [&str; 1] = ["chart_dispute"];

/// Sign + submit one dispute-open OR dispute-resolve event through the real submit_event
/// door. `wall` is the HLC wall clock (higher = newer). Returns the submit result so a
/// test can assert acceptance or a specific rejection. `descriptive` is the reason (open)
/// or resolution (resolve); passed verbatim so an empty string exercises the floor.
#[allow(clippy::too_many_arguments)] // mirrors the `submit_link_prov` helper in identity_linkage.rs
async fn submit_dispute(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dispute_id: Uuid,
    subject: Uuid,
    wall: i64,
    is_open: bool,
    descriptive: &str,
) -> Result<u64, tokio_postgres::Error> {
    let (d_s, s_s) = (dispute_id.to_string(), subject.to_string());
    let (etype, sver, payload, twin) = if is_open {
        let d = DisputeAssertion {
            dispute_id: &d_s,
            subject: &s_s,
            reason: descriptive,
        };
        (
            "identity.dispute.asserted",
            "identity.dispute.asserted/1",
            dispute_assertion_body(&d),
            render_dispute_twin(&d),
        )
    } else {
        let d = DisputeResolution {
            dispute_id: &d_s,
            subject: &s_s,
            resolution: descriptive,
        };
        (
            "identity.dispute.resolved",
            "identity.dispute.resolved/1",
            dispute_resolution_body(&d),
            render_dispute_resolved_twin(&d),
        )
    };
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: subject, // an identity dispute is "about" its subject's chart
            event_type: etype,
            schema_version: sver,
            payload,
            plaintext_twin: Some(twin),
            wall,
        },
    )
    .await
}

/// Convenience: open a dispute with a canned reason.
async fn open_dispute(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dispute_id: Uuid,
    subject: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_dispute(
        c,
        sk,
        kid,
        dispute_id,
        subject,
        wall,
        true,
        "patient states never attended",
    )
    .await
}

/// Convenience: resolve a dispute with a canned resolution.
async fn resolve_dispute(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dispute_id: Uuid,
    subject: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_dispute(
        c,
        sk,
        kid,
        dispute_id,
        subject,
        wall,
        false,
        "dismissed — no evidence",
    )
    .await
}

/// Apply a dispute through the REMOTE door, so it can name a chart this node has never seen.
///
/// The local door refuses a first event that is not a registration (#345); the remote door
/// deliberately does not, because set-union sync has no ordering and a peer legitimately holds
/// the chart this dispute is about. Signs the same body `submit_dispute` builds — the point is
/// which door it enters by, not what it says.
async fn apply_dispute_remotely(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dispute_id: Uuid,
    subject: Uuid,
    wall: i64,
) {
    let (d_s, s_s) = (dispute_id.to_string(), subject.to_string());
    let d = DisputeAssertion {
        dispute_id: &d_s,
        subject: &s_s,
        reason: "patient states never attended",
    };
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(),
        event_type: "identity.dispute.asserted".into(),
        schema_version: "identity.dispute.asserted/1".into(),
        hlc: cairn_event::Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: dispute_assertion_body(&d),
        attachments: vec![],
        plaintext_twin: Some(render_dispute_twin(&d)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = cairn_event::sign(&body, sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("a peer's dispute about a chart we do not hold must be admitted");
}

/// The standing state of a dispute row, or None if no row exists.
async fn dispute_state(c: &Client, dispute_id: Uuid) -> Option<String> {
    let d_s = dispute_id.to_string();
    c.query_opt(
        "SELECT state FROM chart_dispute WHERE dispute_id = $1::text::uuid",
        &[&d_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

// --- acceptance + overlay behaviour ---

#[tokio::test]
async fn valid_dispute_is_accepted() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    open_dispute(&c, &sk, &kid, d, subj, 100)
        .await
        .expect("valid dispute accepted");
    assert_eq!(dispute_state(&c, d).await.as_deref(), Some("open"));
}

#[tokio::test]
async fn newer_resolve_overlays_open() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    open_dispute(&c, &sk, &kid, d, subj, 100).await.unwrap(); // open @100
    resolve_dispute(&c, &sk, &kid, d, subj, 200).await.unwrap(); // resolve @200 (newer)
    assert_eq!(dispute_state(&c, d).await.as_deref(), Some("resolved"));
}

#[tokio::test]
async fn older_open_does_not_reopen_resolved() {
    // Out-of-order arrival must converge: a resolution that lands before the open it
    // closes wins by HLC, and the later-arriving-but-older open does not reopen it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    resolve_dispute(&c, &sk, &kid, d, subj, 200).await.unwrap(); // resolve @200 lands first
    open_dispute(&c, &sk, &kid, d, subj, 100).await.unwrap(); // open @100 lands later (older)
    assert_eq!(
        dispute_state(&c, d).await.as_deref(),
        Some("resolved"),
        "an older open must not reopen a newer resolution"
    );
}

#[tokio::test]
async fn reassert_same_dispute_is_one_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    open_dispute(&c, &sk, &kid, d, subj, 100).await.unwrap();
    open_dispute(&c, &sk, &kid, d, subj, 105).await.unwrap(); // a second, later open of the same dispute
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM chart_dispute WHERE dispute_id = $1::text::uuid",
            &[&d.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "re-opening the same dispute_id is one standing row, not two"
    );
}

#[tokio::test]
async fn local_subject_change_is_rejected() {
    // A dispute_id names ONE chart for its whole life. On the LOCAL submit door a second
    // assertion re-binding the same dispute_id to a DIFFERENT subject is a caller bug and
    // must be refused loudly (nothing is accepted yet — no data lost). The sync-apply path
    // deliberately does NOT raise (it converges by HLC, so honest nodes never fork); that
    // asymmetry is covered by the db/020 apply tests, not here.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj_a, subj_b) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    // #345: BOTH charts are registered. `subj_a` because the first open must succeed — and
    // `subj_b` because the refusal under test lives in the PROJECTION apply (db/023, after the
    // event_log INSERT), while the precedence rule sits before it: an unregistered `subj_b`
    // would be refused for having no chart, and this test would stop exercising the
    // one-dispute-one-chart rule it exists for. Re-binding a dispute to a different REAL chart
    // is also the only version of this bug a caller can actually commit.
    submit_registration(&c, &sk, &kid, subj_a, 1).await;
    submit_registration(&c, &sk, &kid, subj_b, 1).await;
    open_dispute(&c, &sk, &kid, d, subj_a, 100).await.unwrap();
    let err = open_dispute(&c, &sk, &kid, d, subj_b, 110)
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("subject cannot change"),
        "rebinding a dispute_id to a new subject must be refused: {}",
        db_msg(&err)
    );
    // The original binding is untouched — the reject changed nothing.
    assert_eq!(dispute_state(&c, d).await.as_deref(), Some("open"));
    assert_eq!(trust_of(&c, subj_a).await.as_deref(), Some("under-review"));
    assert_eq!(
        trust_of(&c, subj_b).await,
        None,
        "the rejected subject never entered the overlay"
    );
}

// --- the trust-state projection ---

#[tokio::test]
async fn open_marks_chart_under_review() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    submit_registration(&c, &sk, &kid, subj, 100).await;
    open_dispute(&c, &sk, &kid, d, subj, 110).await.unwrap();
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("under-review"));
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("under-review"),
        "the unified read must surface the under-review trust state"
    );
}

#[tokio::test]
async fn resolve_returns_to_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    submit_registration(&c, &sk, &kid, subj, 100).await;
    open_dispute(&c, &sk, &kid, d, subj, 110).await.unwrap();
    resolve_dispute(&c, &sk, &kid, d, subj, 120).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await,
        None,
        "resolved dispute leaves no under-review row"
    );
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("confirmed"),
        "a chart with no standing open dispute reads confirmed"
    );
}

#[tokio::test]
async fn no_dispute_reads_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, subj, 100).await;
    assert_eq!(trust_of(&c, subj).await, None);
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("confirmed"),
        "the default trust state is confirmed"
    );
}

#[tokio::test]
async fn resolve_one_of_two_stays_under_review() {
    // Two concurrent disputes on one chart; resolving one leaves the chart under-review
    // while the other stays open (each dispute is independently resolvable). Resolving
    // both returns the chart to confirmed.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d1, d2, subj) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_registration(&c, &sk, &kid, subj, 100).await;
    open_dispute(&c, &sk, &kid, d1, subj, 110).await.unwrap();
    open_dispute(&c, &sk, &kid, d2, subj, 111).await.unwrap();
    resolve_dispute(&c, &sk, &kid, d1, subj, 120).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("under-review"),
        "one dispute still open → chart stays under-review"
    );
    resolve_dispute(&c, &sk, &kid, d2, subj, 121).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await,
        None,
        "all disputes resolved → confirmed"
    );
}

#[tokio::test]
async fn dispute_before_chart_still_under_review() {
    // Offline-first: a dispute naming a subject with no patient_chart row yet still
    // reports under-review for that subject (the safety signal exists without the body,
    // mirroring §5.9). person_chart only lists it once the chart arrives, which is
    // correct for a *chart* read.
    //
    // #345 changed HOW this state is reached, not whether it exists. A chart this node has
    // never seen can no longer be disputed by a LOCAL author — the precedence rule refuses a
    // first event that is not a registration — so the dispute now arrives the way it really
    // would: through `apply_remote_event`, from a peer that holds the chart. That is the
    // lenient door working as designed (ADR-0061 decision 3), and it makes this test a
    // stronger statement than before: the safety signal survives even when the disputing
    // event and the chart it names are on different nodes.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    apply_dispute_remotely(&c, &sk, &kid, d, subj, 100).await; // no registration for subj
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("under-review"),
        "a dispute reports under-review even before the disputed chart has synced"
    );
    assert_eq!(
        person_chart_trust(&c, subj).await,
        None,
        "person_chart lists the chart only once its patient_chart row exists"
    );
}

// --- structural floor rejections (each a distinct legible exception) ---

#[tokio::test]
async fn empty_reason_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    let err = submit_dispute(&c, &sk, &kid, d, subj, 100, true, "   ")
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("reason"),
        "empty reason must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn empty_resolution_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    let err = submit_dispute(&c, &sk, &kid, d, subj, 100, false, "")
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("resolution"),
        "empty resolution must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    // #345: the disputed chart exists before it can be disputed.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    // Build a dispute event with NO authored twin — the identity floor HARD-requires one.
    let (d_s, s_s) = (d.to_string(), subj.to_string());
    let da = DisputeAssertion {
        dispute_id: &d_s,
        subject: &s_s,
        reason: "r",
    };
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: subj,
            event_type: "identity.dispute.asserted",
            schema_version: "identity.dispute.asserted/1",
            payload: dispute_assertion_body(&da),
            plaintext_twin: None, // the omission under test
            wall: 100,
        },
    )
    .await
    .unwrap_err();
    assert!(
        db_msg(&err).contains("authored twin"),
        "twin-less dispute event must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn bad_dispute_id_is_rejected() {
    // A payload whose dispute_id is not a uuid must be a legible reject, not a crash.
    // Built by hand because the pure builder only takes valid strings by convention.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: subj,
            event_type: "identity.dispute.asserted",
            schema_version: "identity.dispute.asserted/1",
            payload: serde_json::json!({"dispute_id": "not-a-uuid", "subject": subj.to_string(), "reason": "r"}),
            plaintext_twin: Some("dispute opened: x — r (dispute x)".into()),
            wall: 100,
        },
    )
    .await
    .unwrap_err();
    assert!(
        db_msg(&err).contains("dispute_id"),
        "a non-uuid dispute_id must be refused legibly: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn missing_subject_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let d = Uuid::now_v7();
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: Uuid::now_v7(),
            event_type: "identity.dispute.asserted",
            schema_version: "identity.dispute.asserted/1",
            payload: serde_json::json!({"dispute_id": d.to_string(), "reason": "r"}), // no subject
            plaintext_twin: Some("dispute opened: ? — r (dispute d)".into()),
            wall: 100,
        },
    )
    .await
    .unwrap_err();
    assert!(
        db_msg(&err).contains("subject"),
        "a dispute with no subject must be refused legibly: {}",
        db_msg(&err)
    );
}
