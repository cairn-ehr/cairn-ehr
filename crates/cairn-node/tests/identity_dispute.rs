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
    cs, db_msg, person_chart_trust, submit_patient_created, submit_signed, trust_of, EventSpec,
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
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
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
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
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
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
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
    submit_patient_created(&c, &sk, &kid, subj, 100).await;
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
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (d, subj) = (Uuid::now_v7(), Uuid::now_v7());
    open_dispute(&c, &sk, &kid, d, subj, 100).await.unwrap(); // no patient.created for subj
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
