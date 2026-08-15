//! `cairn_claim_authority` (db/005) — what makes a claim authoritative.
//!
//! Authority is a HUMAN actor this node can hold responsible, by either of two routes:
//! R1 a vouched human attestation, R2 human self-withdrawal of one's own claim. Everything
//! else is 'unverified'. See ADR-0064 and
//! docs/superpowers/specs/2026-08-15-claim-authority-at-the-apply-door-design.md.
//!
//! CONTROLLER RULING (see the task-1 brief's ruling section, 2026-08-15): db/048's
//! `cairn_sensitivity_ceremony_ok` refuses a withdrawal at the LOCAL door (`submit_event`)
//! unless a contributor claims RESPONSIBILITY and a valid human attestation token verifies
//! it. A "recorded"-role withdrawal (the shape a genuinely un-attested write carries) never
//! sets that state — the attestation-storage branch itself is guarded by the SAME
//! responsibility claim — so it can never clear the ceremony, attested or not, at EITHER
//! door (`apply_remote_event`'s own attestation gate reads the identical `v_bears` test).
//! That is why the two UN-ATTESTED-withdrawal tests here (R2) go through the REMOTE door:
//! it is the only door that will ever admit that shape.
//!
//! The R1 (attested) test's withdrawal is a DIFFERENT shape — the same
//! responsibility-bearing contributor `crates/cairn-node/src/sensitivity.rs`'s
//! `withdraw_sensitivity` builds, which the LOCAL door accepts too (it is the only shape
//! the product ever writes locally). Test 3 still goes through the remote door, not
//! because the local door would refuse it, but because `cairn_claim_authority` is
//! door-agnostic — it reads only `event_log.attester_key` / `.actor_id`, columns both
//! doors populate identically for an admitted event — and because #380's exposure is
//! specifically the CROSS-NODE case, which the remote door is what actually exercises.
mod common;
use cairn_event::sensitivity::*;
use cairn_event::{ClockGrade, EventBody, Hlc, SigningKey};
use common::{
    apply_remote_attested, apply_remote_raw, body_from_spec, content_address_of, cs, enroll_human,
    setup, submit_attested, submit_registration, withdrawal_body_with_id, EventSpec,
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

/// The effective grade of the chart-wide assertion's own event — what
/// `cairn_effective_sensitivity` (db/048 section 11) reports once `cairn_sensitivity_standing`
/// (section 9) has factored authority into "what still applies". Task 2's tests below are the
/// first in this file to care about the CONSEQUENCE of a withdrawal, not just the predicate's
/// own verdict — [`authority`] answers "is this claim authoritative", `effective` answers "did
/// it actually move the grade".
async fn effective(c: &tokio_postgres::Client, event: Uuid) -> String {
    c.query_one(
        "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
        &[&event.to_string()],
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

/// A signed `EventBody` of a TYPE this node has no `event_type_class` row for — the exact
/// case ADR-0056 decision 1 exists for (a peer authors something this node's code predates).
/// `apply_remote_event` admits it uninterpreted (the "deferred" arm, db/020:239-242) and —
/// critically for the R1 conjunct this probes — stores whatever attestation token travelled
/// with it WITHOUT EVER CALLING `cairn_attestation_ok`, which only runs in the SEPARATE,
/// non-deferred branch just below it. Never classified by any migration, so it stays
/// deferred for the life of the test database.
fn deferred_probe_body(kid: &str, patient: Uuid, event_id: Uuid, wall: i64) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "cairn_test.claim_authority_deferred_probe".into(),
        schema_version: "cairn_test.claim_authority_deferred_probe/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"probe": "unclassified event for the R1 vouch conjunct"}),
        attachments: vec![],
        plaintext_twin: Some(
            "an unclassified probe event for cairn_claim_authority's R1 test".into(),
        ),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

#[tokio::test]
async fn a_carried_but_unverified_token_on_a_deferred_event_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    // No submit_registration: apply_remote_event's precedence rule (db/005 step 8b) is
    // LOCAL-DOOR-ONLY (#345), so a chart known only from a peer's event is the honest
    // fixture here too — mirrors deferred_admission.rs's own module note on why that
    // suite registers no charts either.
    let p = Uuid::now_v7();
    let eid = Uuid::now_v7();
    let body = deferred_probe_body(&kid, p, eid, 30);
    // The human's token is a REAL, correctly-bound attestation (apply_remote_attested
    // signs it properly against this event's content address) — but the event's type is
    // unclassified, so the door's deferred arm stores the token WITHOUT ever calling
    // cairn_attestation_ok to verify it. A genuinely valid token, simply never checked.
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("an unclassified type must still be admitted, uninterpreted (ADR-0056)");

    // Preconditions — the MIRROR IMAGE of an_event_with_no_attestation_at_all_is_unverified's
    // precondition: THIS time a token did travel and get stored (attester_key is set), but
    // it is marked UNVOUCHED (event_attestation_unvouched carries a row for it, db/020:472-486).
    let attester_key_is_set: bool = c
        .query_one(
            "SELECT attester_key IS NOT NULL FROM event_log WHERE event_id = $1::text::uuid",
            &[&eid.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        attester_key_is_set,
        "precondition: a token travelled and was stored on this deferred row"
    );
    let vouched: bool = c
        .query_one(
            "SELECT cairn_attestation_vouched($1::text::uuid)",
            &[&eid.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !vouched,
        "precondition: a carried-but-never-verified token must NOT read as vouched"
    );

    // THE SECURITY-RELEVANT R1 CONJUNCT: `attester_key IS NOT NULL` alone is not enough —
    // the token must actually be VOUCHED. Drop the cairn_attestation_vouched(...) call from
    // R1 and this carried, never-verified peer token grades 'attested' — the precise
    // cross-node forgery #380 exists to close (an enrolled-but-hostile peer ships a
    // classification-predating type carrying a self-minted or borrowed token, betting the
    // receiving node will trust "a token is present" over "a token was checked").
    assert_eq!(authority(&c, eid, None).await, "unverified");
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

    // A peer's honestly-completed ceremony: the device relays, the human vouches. This
    // bearing-contributor shape IS accepted by the LOCAL door too — it's the same shape
    // `sensitivity::withdraw_sensitivity` writes there in production. It lands here via
    // the REMOTE door instead because #380's exposure is the cross-node case and the
    // predicate is door-agnostic (see the module header).
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

// ===========================================================================
// Task 2 (#380): the seam. `cairn_claim_authority` alone judges nothing until
// `cairn_sensitivity_standing` (db/048 section 9) actually consults it — these tests are
// the first in this file to check the CONSEQUENCE (does the grade move), not just the
// predicate's own verdict.
// ===========================================================================

#[tokio::test]
async fn an_unattested_withdrawal_lands_and_converges_but_does_not_lower() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;
    let target = content_address_of(&c, a).await;

    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "strip it",
    };
    let wid = Uuid::now_v7();
    // Un-attested withdrawals can only ever land through the REMOTE door: db/048's
    // ceremony refuses every un-attested withdrawal at the LOCAL door unconditionally
    // (see this file's module header), and `withdrawal_body_with_id` builds exactly the
    // "recorded"-role shape a genuinely un-attested peer write carries.
    apply_remote_raw(&c, &sk, withdrawal_body_with_id(p, wid, &kid, &w, 20))
        .await
        .expect("ADMITTED — authority gates EFFECT, never admission; a refusal would fork");

    // BOTH halves matter. Assert admission first: if the door started refusing, the
    // "does not lower" assertion below would pass for entirely the wrong reason.
    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE withdraws = $1",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(landed, 1, "the withdrawal must land and converge");

    assert_eq!(
        effective(&c, a).await,
        "sequestered",
        "an un-attested withdrawal must not lower the grade (#380)"
    );
}

#[tokio::test]
async fn an_attested_cross_node_withdrawal_lowers() {
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

    // A peer's honestly-completed ceremony (the same bearing shape
    // `a_vouched_human_attestation_is_attested` above already lands), reused here to also
    // assert the CONSEQUENCE — the grade actually falls — not just the predicate's verdict.
    // Cross-node, so the remote door, exactly like a real replicated write.
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

    assert_eq!(
        effective(&c, a).await,
        "routine",
        "no deadlock: attesting is the remedy"
    );
}

#[tokio::test]
async fn a_locally_authored_withdrawal_always_lowers() {
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

    // The LOCAL door already demands a bound human author for a withdrawal (db/048's
    // ceremony, ADR-0062 decision 7 / ADR-0053), so anything it accepts clears the bar BY
    // CONSTRUCTION. This pins the no-deadlock claim instead of asserting it in prose. Uses
    // the BEARING contributor shape (`bearing_withdrawal_body`) — the only shape the local
    // door's ceremony ever admits for a withdrawal (see this file's module header); it is
    // the same shape production's `sensitivity::withdraw_sensitivity` writes there.
    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "clinician lowered it",
    };
    let wid = Uuid::now_v7();
    let body = bearing_withdrawal_body(&kid, &kid_h, p, wid, &w, 20);
    submit_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("the local ceremony accepted it, so authority must too");
    assert_eq!(effective(&c, a).await, "routine");
}

// ===========================================================================
// Task 3 (#380): arrival-order independence. Computing authority at READ rather than
// stamping it at apply is what makes these pass with NO new production code — Tasks 1/2
// already compute at read, so a withdrawal inert today because a piece it needs has not
// yet replicated self-heals the moment that piece lands, with no re-apply and no second
// event.
// ===========================================================================

#[tokio::test]
async fn a_withdrawal_inert_because_its_target_has_not_replicated_heals_when_it_lands() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Set-union sync has no ordering: the withdrawal legitimately arrives FIRST
    // (ADR-0062 decision 3). Its target's event_id is knowable — it is content-addressed —
    // but the row is not here yet, so R2 cannot resolve. R1 must carry it alone.
    let future_assert_id = Uuid::now_v7();
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sequestered",
        source: "human",
        rationale: Some("protected witness"),
    };
    let assert_body = body_from_spec(
        future_assert_id,
        &kid,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall: 10,
        },
    );
    let target_ca =
        cairn_event::event_address(&cairn_event::sign(&assert_body, &sk).unwrap().signed_bytes);

    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target_ca),
        rationale: "consented",
    };
    let wid = Uuid::now_v7();
    // The bearing shape (R1-eligible) through the remote door — the withdrawal is a peer's
    // event here, exactly like Task 2's cross-node test.
    let body = bearing_withdrawal_body(&kid, &kid_h, p, wid, &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .unwrap();

    // R2 cannot resolve (no target row), but R1 stands on its own.
    assert_eq!(authority(&c, wid, None).await, "attested");

    // Now the target lands, as a peer's event arriving on a later sync cycle. The
    // withdrawal must take effect — a delete-at-apply design would have dropped it on the
    // floor the moment it was inert, instead of leaving it to be re-evaluated at read.
    apply_remote_raw(&c, &sk, assert_body).await.unwrap();
    assert_eq!(effective(&c, future_assert_id).await, "routine");
}

// THE OTHER ARRIVAL-ORDER AXIS — an attester unknown to THIS node — is NOT reachable via
// apply_remote_event for a CLASSIFIED type, so no second test exists here. sensitivity.grade-
// withdrawal.asserted IS classified (db/048 section 2), which forces db/020_apply_remote_event's
// non-deferred branch: a bearing (R1-eligible) contributor's attester is checked against
// actor_current, and an unenrolled attester is refused OUTRIGHT with "attester is not an
// enrolled human actor" — the event never lands at all, so it can never sit "inert" waiting to
// heal. (The deferred arm that stores an attestation token WITHOUT verifying it — the shape
// `a_carried_but_unverified_token_on_a_deferred_event_is_unverified` above exercises — is reached
// only by an UNclassified type, which a withdrawal never is.) Verified empirically, not inferred:
// a throwaway test drove exactly this shape through apply_remote_attested and observed the
// refusal. This finding belongs in ADR-0064's Known limitations (Task 9) once that ADR exists.
