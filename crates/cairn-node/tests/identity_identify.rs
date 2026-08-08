//! Integration coverage for §5.4/§5.7 identity-pending + `identify` + the *unconfirmed*
//! trust state (db/024): the pending/identify event types, the structural floor, the
//! chart_identity_state standing overlay, and the reworked chart_trust projection that
//! composes under-review (dispute — and, tested in link_veto_floor.rs, the #190
//! link-veto flag) over unconfirmed (pending) by highest severity. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`.
//! Mirrors `identity_dispute.rs` (C3).
use cairn_event::identity::{
    dispute_assertion_body, dispute_resolution_body, identify_assertion_body,
    pending_assertion_body, render_dispute_resolved_twin, render_dispute_twin,
    render_identify_twin, render_pending_twin, DisputeAssertion, DisputeResolution,
    IdentifyAssertion, PendingAssertion,
};
use cairn_event::SigningKey;
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{
    cs, db_msg, person_chart_trust, submit_registration, submit_signed, trust_of, EventSpec,
};

/// The identity-overlay tables this suite writes to, truncated per test by
/// `common::setup`. Created by db/024 and db/023, later than the core clinical tables —
/// hence the `to_regclass` guard `setup` applies.
const OVERLAY_TABLES: [&str; 2] = ["chart_identity_state", "chart_dispute"];

/// Sign + submit one identity-pending OR identify event through the real submit_event door.
/// `wall` is the HLC wall clock (higher = newer). Returns the submit result so a test can
/// assert acceptance or a specific rejection. `descriptive` is the basis (pending) or
/// method (identify); passed verbatim so an empty string exercises the floor.
async fn submit_identity_state(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    subject: Uuid,
    wall: i64,
    is_pending: bool,
    descriptive: &str,
) -> Result<u64, tokio_postgres::Error> {
    let s_s = subject.to_string();
    let (etype, sver, payload, twin) = if is_pending {
        let a = PendingAssertion {
            subject: &s_s,
            basis: descriptive,
        };
        (
            "identity.pending.asserted",
            "identity.pending.asserted/1",
            pending_assertion_body(&a),
            render_pending_twin(&a),
        )
    } else {
        let a = IdentifyAssertion {
            subject: &s_s,
            method: descriptive,
        };
        (
            "identity.identify.asserted",
            "identity.identify.asserted/1",
            identify_assertion_body(&a),
            render_identify_twin(&a),
        )
    };
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: subject, // an identity-state assertion is "about" its subject's chart
            event_type: etype,
            schema_version: sver,
            payload,
            plaintext_twin: Some(twin),
            wall,
        },
    )
    .await
}

/// Convenience: register a subject identity-pending with a canned basis.
async fn mark_pending(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    subject: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_identity_state(
        c,
        sk,
        kid,
        subject,
        wall,
        true,
        "unconscious ED arrival, no ID",
    )
    .await
}

/// Apply an identity-pending marker through the REMOTE door, so it can name a chart this node
/// has never seen. Same body `submit_identity_state` builds; the point is which door it enters
/// by (#345: the local door refuses a first event that is not a registration, the remote door
/// deliberately does not).
async fn apply_pending_remotely(c: &Client, sk: &SigningKey, kid: &str, subject: Uuid, wall: i64) {
    let s_s = subject.to_string();
    let a = PendingAssertion {
        subject: &s_s,
        basis: "unconscious ED arrival, no ID",
    };
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s_s.clone(),
        event_type: "identity.pending.asserted".into(),
        schema_version: "identity.pending.asserted/1".into(),
        hlc: cairn_event::Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: pending_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_pending_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = cairn_event::sign(&body, sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("a peer's pending marker about a chart we do not hold must be admitted");
}

/// Convenience: identify a subject with a canned method.
async fn identify(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    subject: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_identity_state(c, sk, kid, subject, wall, false, "driver's licence").await
}

/// Submit one dispute-open OR dispute-resolve event (reused for the compose/precedence
/// test — the C4 slice must prove chart_trust ranks an open dispute over a pending chart).
///
/// `descriptive` is the reason (open) or resolution (resolve), matching the signature in
/// `identity_dispute.rs`. This file's copy had silently dropped that parameter and hard-coded
/// both strings (#120): the divergence was invisible precisely because the two helpers were
/// separate copies. Kept as a parameter so the two suites stay comparable at a glance.
#[allow(clippy::too_many_arguments)] // mirrors `submit_dispute` in identity_dispute.rs
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
            patient: subject,
            event_type: etype,
            schema_version: sver,
            payload,
            plaintext_twin: Some(twin),
            wall,
        },
    )
    .await
}

/// The standing identity state of a chart, or None if no row exists.
async fn identity_state(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

// --- acceptance + overlay behaviour ---

#[tokio::test]
async fn valid_pending_is_accepted() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    mark_pending(&c, &sk, &kid, subj, 100)
        .await
        .expect("valid pending accepted");
    assert_eq!(identity_state(&c, subj).await.as_deref(), Some("pending"));
}

#[tokio::test]
async fn newer_identify_overlays_pending() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap(); // pending @100
    identify(&c, &sk, &kid, subj, 200).await.unwrap(); // identify @200 (newer)
    assert_eq!(
        identity_state(&c, subj).await.as_deref(),
        Some("identified")
    );
}

#[tokio::test]
async fn older_pending_does_not_reopen_identified() {
    // Out-of-order arrival must converge: an identify that lands before the pending it
    // clears wins by HLC, and the later-arriving-but-older pending does not re-open it.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    identify(&c, &sk, &kid, subj, 200).await.unwrap(); // identify @200 lands first
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap(); // pending @100 lands later (older)
    assert_eq!(
        identity_state(&c, subj).await.as_deref(),
        Some("identified"),
        "an older pending must not re-open a newer identify"
    );
}

#[tokio::test]
async fn newer_pending_reopens_after_identify() {
    // The overlay is a full lifecycle, not one-way: a mis-identification retracted, the
    // chart re-registered identity-pending with a HIGHER HLC re-opens the unconfirmed state.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap();
    identify(&c, &sk, &kid, subj, 200).await.unwrap();
    mark_pending(&c, &sk, &kid, subj, 300).await.unwrap(); // re-registered pending @300 (newest)
    assert_eq!(
        identity_state(&c, subj).await.as_deref(),
        Some("pending"),
        "a newer pending after an identify re-opens the unconfirmed state"
    );
}

#[tokio::test]
async fn reassert_same_pending_is_one_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    mark_pending(&c, &sk, &kid, subj, 100).await.unwrap();
    mark_pending(&c, &sk, &kid, subj, 105).await.unwrap(); // a second, later pending on the same subject
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM chart_identity_state WHERE subject = $1::text::uuid",
            &[&subj.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "re-registering the same subject pending is one standing row, not two"
    );
}

// --- the trust-state projection (the unconfirmed state) ---

#[tokio::test]
async fn pending_marks_chart_unconfirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    assert_eq!(trust_of(&c, subj).await.as_deref(), Some("unconfirmed"));
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("unconfirmed"),
        "the unified read must surface the unconfirmed trust state"
    );
}

#[tokio::test]
async fn identify_returns_to_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    identify(&c, &sk, &kid, subj, 120).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await,
        None,
        "identified chart leaves no unconfirmed row"
    );
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("confirmed"),
        "an identified chart reads confirmed"
    );
}

#[tokio::test]
async fn no_identity_reads_confirmed() {
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
async fn pending_before_chart_still_unconfirmed() {
    // Offline-first: a pending marker naming a subject with no patient_chart row yet still
    // reports unconfirmed for that subject (the safety signal exists without the body,
    // mirroring §5.9 and C3's dispute-before-chart). person_chart only lists it once the
    // chart arrives, which is correct for a *chart* read.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345 changed HOW this state is reached, not whether it exists: a chart this node has
    // never seen can no longer be marked pending by a LOCAL author (the precedence rule
    // refuses a first event that is not a registration), so the marker now arrives the way it
    // really would — through the lenient remote door, from the peer that holds the chart
    // (ADR-0061 decision 3). The claim under test is unchanged and now stronger: the trust
    // signal survives even when the marker and the chart are on different nodes.
    apply_pending_remotely(&c, &sk, &kid, subj, 100).await; // no registration for subj
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("unconfirmed"),
        "a pending marker reports unconfirmed even before the chart has synced"
    );
    assert_eq!(
        person_chart_trust(&c, subj).await,
        None,
        "person_chart lists the chart only once its patient_chart row exists"
    );
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
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let (subj, d) = (Uuid::now_v7(), Uuid::now_v7());
    submit_registration(&c, &sk, &kid, subj, 100).await;
    mark_pending(&c, &sk, &kid, subj, 110).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("unconfirmed"),
        "pending alone → unconfirmed"
    );
    submit_dispute(
        &c,
        &sk,
        &kid,
        d,
        subj,
        120,
        true,
        "suspected identity theft",
    )
    .await
    .unwrap(); // open dispute
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("under-review"),
        "an open dispute outranks the pending state → under-review"
    );
    submit_dispute(
        &c,
        &sk,
        &kid,
        d,
        subj,
        130,
        false,
        "dismissed — no evidence",
    )
    .await
    .unwrap(); // resolve dispute
    assert_eq!(
        trust_of(&c, subj).await.as_deref(),
        Some("unconfirmed"),
        "dispute resolved, pending still standing → back to unconfirmed"
    );
    identify(&c, &sk, &kid, subj, 140).await.unwrap();
    assert_eq!(
        trust_of(&c, subj).await,
        None,
        "identify clears the last source → confirmed"
    );
    assert_eq!(
        person_chart_trust(&c, subj).await.as_deref(),
        Some("confirmed")
    );
}

// --- structural floor rejections (each a distinct legible exception) ---

#[tokio::test]
async fn empty_basis_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    let err = submit_identity_state(&c, &sk, &kid, subj, 100, true, "   ")
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("basis"),
        "empty basis must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn empty_method_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    let err = submit_identity_state(&c, &sk, &kid, subj, 100, false, "")
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("method"),
        "empty method must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let subj = Uuid::now_v7();
    // #345: the chart exists before its identity state is asserted.
    submit_registration(&c, &sk, &kid, subj, 1).await;
    // Build a pending event with NO authored twin — the identity floor HARD-requires one.
    let s_s = subj.to_string();
    let pa = PendingAssertion {
        subject: &s_s,
        basis: "b",
    };
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: subj,
            event_type: "identity.pending.asserted",
            schema_version: "identity.pending.asserted/1",
            payload: pending_assertion_body(&pa),
            plaintext_twin: None, // the omission under test
            wall: 100,
        },
    )
    .await
    .unwrap_err();
    assert!(
        db_msg(&err).contains("authored twin"),
        "twin-less identity-state event must be refused: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn bad_subject_is_rejected() {
    // A payload whose subject is not a uuid must be a legible reject, not a crash.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: Uuid::now_v7(),
            event_type: "identity.pending.asserted",
            schema_version: "identity.pending.asserted/1",
            payload: serde_json::json!({"subject": "not-a-uuid", "basis": "b"}),
            plaintext_twin: Some("identity pending: x — b".into()),
            wall: 100,
        },
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
async fn missing_subject_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &OVERLAY_TABLES).await;
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: Uuid::now_v7(),
            event_type: "identity.pending.asserted",
            schema_version: "identity.pending.asserted/1",
            payload: serde_json::json!({"basis": "b"}), // no subject
            plaintext_twin: Some("identity pending: ? — b".into()),
            wall: 100,
        },
    )
    .await
    .unwrap_err();
    assert!(
        db_msg(&err).contains("subject"),
        "an identity-state assertion with no subject must be refused legibly: {}",
        db_msg(&err)
    );
}
