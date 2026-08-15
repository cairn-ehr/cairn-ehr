//! `cairn_claim_authority` (db/005) — what makes a claim authoritative.
//!
//! Authority is a HUMAN actor this node can hold responsible, by either of two routes:
//! R1 a vouched human attestation, R2 human self-withdrawal of one's own claim. Everything
//! else is 'unverified'. See ADR-0064 and
//! docs/superpowers/specs/2026-08-15-claim-authority-at-the-apply-door-design.md.
//!
//! CONTROLLER RULING (see the task-1 brief's ruling section, 2026-08-15): db/048's
//! `cairn_sensitivity_ceremony_ok` refuses ANY withdrawal at the LOCAL door
//! (`submit_event`) unless a contributor claims RESPONSIBILITY and a valid human
//! attestation token verifies it — a "recorded"-role withdrawal (the shape a genuinely
//! un-attested peer write carries) can never clear that gate, attested or not, because the
//! attestation-storage branch itself is guarded by the SAME responsibility claim. So every
//! withdrawal in this file is submitted through the REMOTE door (`apply_remote_event`),
//! which is exactly the door #380's cross-node exposure travels through.
mod common;
use cairn_event::sensitivity::*;
use cairn_event::{ClockGrade, EventBody, Hlc, SigningKey};
use common::{
    apply_remote_attested, apply_remote_raw, content_address_of, cs, enroll_human, setup,
    submit_registration, withdrawal_body_with_id, EventSpec,
};
use uuid::Uuid;

/// Ask the predicate directly. `target` may be None (R1-only callers pass SQL NULL).
async fn authority(c: &tokio_postgres::Client, event: Uuid, target: Option<Uuid>) -> String {
    c.query_one(
        "SELECT cairn_claim_authority($1::text::uuid, $2::text::uuid)",
        &[&event.to_string(), &target.map(|t| t.to_string())],
    )
    .await
    .unwrap()
    .get(0)
}

/// A standing assertion, submitted by `sk`/`kid`, returning its event id.
async fn assert_grade(
    c: &tokio_postgres::Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
) -> Uuid {
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: patient,
        grade: "sequestered",
        source: "human",
        rationale: Some("protected witness"),
    };
    let id = Uuid::now_v7();
    common::submit_signed_with_id(
        c,
        sk,
        kid,
        id,
        EventSpec {
            patient,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall,
        },
    )
    .await
    .unwrap();
    id
}

/// A withdrawal `EventBody` whose contributor claims RESPONSIBILITY for `attester_kid`
/// rather than for the signer — the ONLY shape either door's attestation gate will
/// validate and STORE. `cairn_responsibility_bound` (db/005, mirrored at db/020) requires
/// the bearing contributor's `actor_id` (and `cairn_check_contributors`'s
/// `responsibility.held_by`) to equal the verified attester's own key, so a device may
/// sign while a human attests, and the token still lands on `event_log.attester_key`.
///
/// `withdrawal_body_with_id` (common/mod.rs) builds the plain "recorded" contributor
/// instead — what a genuinely un-attested peer write looks like, and a shape whose token
/// (if any were even offered) is silently discarded by both doors because the
/// attestation-storage branch itself is gated on the SAME responsibility claim. That shape
/// can never be graded 'attested', so this file needs a second body builder — used by
/// exactly one test, so it stays local rather than joining common/mod.rs's shared surface
/// (that module's own header: "if two suites would write it identically, it goes here").
fn bearing_withdrawal_body(
    kid: &str,
    attester_kid: &str,
    patient: Uuid,
    event_id: Uuid,
    w: &SensitivityWithdrawal,
    wall: i64,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: WITHDRAWAL_EVENT_TYPE.into(),
        schema_version: WITHDRAWAL_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        // "attested" + a responsibility marker naming the ATTESTER (not the signer): the
        // ADR-0051 wire shape both doors' attestation gate demands before it will verify
        // and STORE the token as this event's `attester_key` (mirrors
        // `cairn-node::sensitivity::withdraw_sensitivity`, generalised to a
        // different-signer/different-attester pair).
        contributors: serde_json::json!([{"actor_id": attester_kid, "role": "attested",
                                          "responsibility": {"held_by": attester_kid}}]),
        payload: sensitivity_withdrawal_body(w),
        attachments: vec![],
        plaintext_twin: Some(render_withdrawal_twin(w)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

#[tokio::test]
async fn an_unattested_claim_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // The device key signed it; no attestation rides it, and the signer is not human.
    assert_eq!(authority(&c, a, None).await, "unverified");
}

#[tokio::test]
async fn an_event_with_no_attestation_at_all_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // THE GUARD THIS TEST EXISTS FOR: cairn_attestation_vouched returns TRUE for an event
    // carrying NO attestation, because "vouched" is the ABSENCE of an unvouched marker row.
    // So `attester_key IS NOT NULL` is the actual R1 test; drop it and every unattested
    // event in the log grades 'attested'.
    assert!(
        c.query_one(
            "SELECT cairn_attestation_vouched($1::text::uuid)",
            &[&a.to_string()]
        )
        .await
        .unwrap()
        .get::<_, bool>(0),
        "precondition: an unattested event is vacuously 'vouched'"
    );
    assert_eq!(authority(&c, a, None).await, "unverified");
}

#[tokio::test]
async fn a_vouched_human_attestation_is_attested() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // A peer's honestly-completed ceremony: the device relays, the human vouches. Landed
    // through the REMOTE door — see the controller ruling in the module header for why the
    // local door can never accept a withdrawal built this way (or any way).
    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "patient consented",
    };
    let wid = Uuid::now_v7();
    let body = bearing_withdrawal_body(&kid, &kid_h, p, wid, &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");

    assert_eq!(authority(&c, wid, Some(a)).await, "attested");
}

#[tokio::test]
async fn a_human_withdrawing_their_own_assertion_is_self() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // The HUMAN signs the assertion, so actor_id on both rows is that human's actor.
    let a = assert_grade(&c, &sk_h, &kid_h, p, 10).await;

    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "mine to lower",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &kid_h, &w, 20);
    // R2 exists precisely because a remote withdrawal's attestation may not verify HERE —
    // so an un-attested self-withdrawal must be able to land at all. The local door
    // (db/048's ceremony) refuses every un-attested withdrawal unconditionally, so this
    // goes through the remote door, exactly like a real cross-node write would.
    apply_remote_raw(&c, &sk_h, body)
        .await
        .expect("an un-attested withdrawal must still land at the remote door");

    assert_eq!(authority(&c, wid, Some(a)).await, "self");
}

#[tokio::test]
async fn an_advisory_actor_cannot_self_withdraw_its_own_protective_tag() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // `setup` enrols a DEVICE/agent actor. It auto-tags, then tries to strip its own tag.
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "reconsidered",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &kid, &w, 20);
    // Same remote-door reasoning as the self-withdrawal test above: an un-attested
    // withdrawal can only ever land through the remote door.
    apply_remote_raw(&c, &sk, body)
        .await
        .expect("an un-attested withdrawal must still land at the remote door");

    // ADR-0062 decision 6: dismissing a protective auto-tag is a LOWERING and must route
    // through the ceremony. Without the kind='human' clause on R2 this returns 'self'.
    assert_eq!(authority(&c, wid, Some(a)).await, "unverified");
}

#[tokio::test]
async fn no_target_means_r2_cannot_apply() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk_h, &kid_h, p, 10).await;

    // Same event, NULL target: R2 is unavailable, and this assertion carries no attestation.
    assert_eq!(authority(&c, a, None).await, "unverified");
}

#[tokio::test]
async fn the_read_path_works_as_cairn_agent() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // SECURITY DEFINER is load-bearing, not stylistic (see the SQL header): without it,
    // cairn_agent — the role the product's actual read path (cairn_sensitivity_standing,
    // db/048) runs as — gets "permission denied" the instant it calls this predicate,
    // because cairn_attestation_vouched is REVOKEd FROM PUBLIC. A suite that only ever
    // runs as the connection owner would never see that failure (Slice 62's lesson: test
    // the path the product actually calls, not a stand-in with more privilege than it has).
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let verdict: String = c
        .query_one(
            "SELECT cairn_claim_authority($1::text::uuid, NULL)",
            &[&a.to_string()],
        )
        .await
        .expect("cairn_agent must be able to call the predicate without a permission error")
        .get(0);
    assert_eq!(verdict, "unverified");
}
