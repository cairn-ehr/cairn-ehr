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
//! That is why EVERY un-attested-withdrawal test here (R2, and the R2-adjacent negatives)
//! goes through the REMOTE door: it is the only door that will ever admit that shape. Do
//! not count them in this comment — an earlier version said "the two", and the file has
//! grown past that twice since (#410 review).
//!
//! The R1 (attested) test's withdrawal is a DIFFERENT shape — the same
//! responsibility-bearing contributor `crates/cairn-node/src/sensitivity.rs`'s
//! `withdraw_sensitivity` builds, which the LOCAL door accepts too (it is the only shape
//! the product ever writes locally). `a_vouched_human_attestation_is_attested` still goes
//! through the remote door — named rather than numbered, because the ordinal drifts every
//! time a test is inserted above it (#410 review) — not
//! because the local door would refuse it, but because `cairn_claim_authority` is
//! door-agnostic — it reads only `event_log.attester_key` / `.actor_id`, columns both
//! doors populate identically for an admitted event — and because #380's exposure is
//! specifically the CROSS-NODE case, which the remote door is what actually exercises.
mod common;
use cairn_event::sensitivity::*;
use cairn_event::{ClockGrade, EventBody, Hlc};
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    body_from_spec, content_address_of, cs, enroll_human, enroll_human_with_role, setup,
    submit_attested, submit_registration, withdrawal_body_with_id, EventSpec,
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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;

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

/// R2 is SELF-withdrawal, and "self" is an EQUALITY — not merely "some human".
///
/// This is the negative twin of `a_human_withdrawing_their_own_assertion_is_self` above, and
/// it exists because that test alone leaves R2's `c.actor_id = t.actor_id` conjunct
/// (db/005) completely unexercised: every OTHER un-attested-withdrawal fixture in this file
/// uses the DEVICE as both asserter and withdrawer, so R2 already dies on `kind = 'human'`
/// and never reaches the equality. Deleting that equality therefore left the whole suite
/// green (#410 review finding C1, found by mutation testing).
///
/// What the missing coverage was hiding is the entire #380 attack, restored: with the
/// equality gone, ANY enrolled human on ANY peer reads 'self' against ANY assertion, so an
/// un-attested cross-node withdrawal strips protection from a chart the withdrawer has no
/// relationship to. That is precisely what ADR-0064 exists to close, and it is why the
/// assertion below checks BOTH the verdict AND the consequence: a verdict test alone would
/// still pass if the seam stopped consulting the predicate.
#[tokio::test]
async fn a_different_human_cannot_self_withdraw_another_humans_assertion() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    // A SECOND human, distinct in the only way that matters to R2: a different `actor_id`.
    // `enroll_human` twice would collide (same pinned set -> same actor), so the role varies.
    let (sk_h2, kid_h2) = enroll_human_with_role(&c, "locum-clinician").await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // Human ONE raises the protection, so the target's actor_id is human one's.
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;

    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "not mine to lower",
    };
    let wid = Uuid::now_v7();
    // Human TWO signs the withdrawal un-attested — the same remote-door route the sibling
    // self-withdrawal test uses, so the ONLY difference between the two fixtures is whose
    // actor authored the target. That isolation is what makes this a pin on the equality
    // rather than on any of R2's other conjuncts.
    let body = withdrawal_body_with_id(p, wid, &kid_h2, &w, 20);
    apply_remote_raw(&c, &sk_h2, body)
        .await
        .expect("ADMITTED — authority gates effect, never admission (ADR-0064)");

    // R2 must not fire: human two is a human, and their claim is well-formed, but the
    // assertion they target is not theirs.
    assert_eq!(authority(&c, wid, Some(a)).await, "unverified");

    // Positive control on the CONSEQUENCE, not just the verdict: the grade must still stand.
    // Without this, a regression that stopped consulting `cairn_claim_authority` at the seam
    // entirely would leave the verdict assertion above passing while protection was stripped.
    assert_eq!(effective(&c, a).await, "sequestered");

    // Tripwire against the fixture silently degenerating: if a future edit made both
    // withdrawals resolve to the SAME actor, this test would still pass for the wrong
    // reason (R2 failing on something other than the equality). Pin the premise.
    let (actor_target, actor_withdrawal): (Option<Vec<u8>>, Option<Vec<u8>>) = {
        let row = c
            .query_one(
                "SELECT (SELECT actor_id FROM event_log WHERE event_id = $1::text::uuid),
                        (SELECT actor_id FROM event_log WHERE event_id = $2::text::uuid)",
                &[&a.to_string(), &wid.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(
        actor_target.is_some() && actor_withdrawal.is_some(),
        "both events must resolve an actor, or R2 fails on the NULL guards instead"
    );
    assert_ne!(
        actor_target, actor_withdrawal,
        "the two humans must be genuinely distinct actors, or this test pins nothing"
    );
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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    // SECURITY DEFINER is load-bearing, not stylistic (see the SQL header): without it,
    // cairn_agent — the role the product's actual read path runs as — would get "permission
    // denied" when `cairn_claim_authority` calls `cairn_attestation_vouched`, which is
    // REVOKEd FROM PUBLIC. Reading through `cairn_effective_sensitivity` (not the predicate
    // directly) exercises the COMPOSED path Task 2 created: effective -> standing ->
    // claim_authority.
    //
    // WHAT THIS TEST ACTUALLY PINS, precisely — it is the WEAKER of the two role-switched
    // pins, and the comment here used to overstate it (#410 review finding A3). This
    // fixture builds NO withdrawal, so `sensitivity_withdrawal` is empty, the seam's
    // `NOT EXISTS` subquery matches nothing, and the predicate's BODY never executes:
    // `cairn_attestation_vouched` is never reached here. What survives is Postgres's
    // executor-start ACL check — cairn_agent must hold EXECUTE on every function named in
    // the plan, `cairn_claim_authority` included — which is a real regression guard (a
    // dropped GRANT fails before the predicate would ever run) but NOT the dependency
    // chain. Note also that `cairn_claim_authority` carries `SET search_path`, which alone
    // blocks SQL inlining, so removing only SECURITY DEFINER would leave this test green.
    //
    // The STRONGER pin — the one that actually runs the predicate under the role against
    // live data — is `claim_authority_worklist.rs::the_worklist_is_readable_as_cairn_agent`,
    // whose fixture carries a real inert withdrawal. If either is ever simplified, re-anchor
    // to whichever still lands real data through the role switch (ADR-0064's own warning).
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let grade: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&a.to_string()],
        )
        .await
        .expect("cairn_agent must be able to read the effective grade without a permission error")
        .get(0);
    c.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(grade, "sequestered");
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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

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

#[tokio::test]
async fn a_self_withdrawal_lowers_through_the_seam() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // The HUMAN signs the assertion, so actor_id on both rows is that human's actor — R2's
    // precondition. Mirrors a_human_withdrawing_their_own_assertion_is_self, which pins the
    // PREDICATE's 'self' verdict in isolation; this test pins that the SEAM actually acts on
    // it — the mutation check below is what makes that a real distinction, not a restatement
    // (review finding, Important #2).
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;

    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "mine to lower",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &kid_h, &w, 20);
    // R2 exists precisely because a remote withdrawal's attestation may not verify HERE —
    // so an un-attested self-withdrawal must be able to land at all. The local door refuses
    // every un-attested withdrawal unconditionally, so this goes through the remote door.
    apply_remote_raw(&c, &sk_h, body)
        .await
        .expect("an un-attested withdrawal must still land at the remote door");

    // THE COMMITMENT THIS PINS: ADR-0062 examined and rejected self-only withdrawal as
    // deadlocking (the asserting clinician retired, the patient left the practice) — R2 is
    // half of the remedy (R1 attestation is the other half), and it must not require an
    // attester for the ordinary case of a clinician lowering their OWN un-attested claim.
    // If the seam's `<> 'unverified'` ever silently narrowed to `= 'attested'`, this grade
    // would stay 'sequestered' forever — the exact deadlock the design rejected.
    assert_eq!(
        effective(&c, a).await,
        "routine",
        "a human withdrawing their OWN un-attested claim must still lower the grade (R2, ADR-0062)"
    );
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

    // R2 cannot resolve (no target row), but R1 stands on its own. A non-NULL target that
    // names an event genuinely absent from event_log — not `None` — is the shape
    // `cairn_sensitivity_standing` actually calls the predicate with (its own w.event_id,
    // a.event_id pair); passing `None` here would prove R1 alone suffices in a situation
    // the seam never presents (review finding, Minor #4).
    assert_eq!(authority(&c, wid, Some(future_assert_id)).await, "attested");

    // Now the target lands, as a peer's event arriving on a later sync cycle. The
    // withdrawal must take effect — a delete-at-apply design would have dropped it on the
    // floor the moment it was inert, instead of leaving it to be re-evaluated at read.
    apply_remote_raw(&c, &sk, assert_body).await.unwrap();

    // Close the round-trip: prove the target actually projected UNDER THE ADDRESS THE
    // WITHDRAWAL NAMED. Without this, "routine" below is indistinguishable from three
    // different worlds — the withdrawal correctly took effect, the assertion never
    // projected at all, or `target_ca` (recomputed above) never matched the address the
    // landed event actually got — because `cairn_effective_sensitivity`'s COALESCE default
    // is ALSO 'routine' when no assertion applies. Same discipline this file's own
    // `an_unattested_withdrawal_lands_and_converges_but_does_not_lower` already applies to
    // admission (review finding, Important #1).
    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_assertion \
             WHERE patient_id = $1::text::uuid AND content_address = $2",
            &[&p.to_string(), &target_ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        projected, 1,
        "the target must actually project under the address the withdrawal named"
    );
    assert_eq!(effective(&c, future_assert_id).await, "routine");

    // A CONTROL: the arrival order this test is FOR (withdrawal before its target) makes it
    // impossible to observe "sequestered" on future_assert_id BEFORE it lands —
    // cairn_effective_sensitivity requires the event to already be in event_log to resolve
    // its patient/thread, so querying it earlier would just find no row, not a grade. That
    // in-order assert-then-withdraw transition is already pinned by sensitivity_ladder.rs's
    // a_withdrawal_lowers_the_effective_grade_and_the_assertion_survives. What CAN be pinned
    // here, in this exact test context: a second, un-withdrawn assertion on the SAME
    // patient with the SAME grade still reads as 'sequestered' — ruling out a harness bug
    // where cairn_effective_sensitivity silently always answers 'routine' regardless of
    // what stands.
    let control = assert_chart_grade(&c, &sk, &kid, p, 30, "sequestered").await;
    assert_eq!(
        effective(&c, control).await,
        "sequestered",
        "control: an un-withdrawn assertion on the same patient must still read as sequestered"
    );
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

// ---------------------------------------------------------------------------
// #410 review finding C3 — the polarity of the ONE protection-stripping test.
// ---------------------------------------------------------------------------

/// db/005 exactly as this build embeds it, replayed to PUT BACK the predicate the test
/// below deliberately replaces. Restoring from the migration file itself (rather than a
/// hand-copied definition) keeps the restore from drifting away from the thing it restores
/// — the same discipline `safety_overclaim.rs`'s `DB049` keeps.
const DB005: &str = include_str!("../../../db/005_submit.sql");

/// `cairn_claim_authority` replaced by one returning a verdict that does not exist today.
/// Argument NAMES must match db/005's or `CREATE OR REPLACE` refuses.
///
/// This stages the FUTURE, not a hostile act: ADR-0064 says outright that "every future
/// dial" will delegate to this predicate, so a fourth verdict is a routine expected
/// evolution, not an attack. The question this test asks is what the seam does the day one
/// arrives — and the answer must be "withholds", by construction, without anyone having to
/// remember to revisit db/048.
const FOURTH_VERDICT: &str = r#"
CREATE OR REPLACE FUNCTION cairn_claim_authority(p_event_id uuid, p_target_event_id uuid)
RETURNS text LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $future$
    SELECT 'delegated-registry'::text;
$future$;
"#;

#[tokio::test]
async fn an_unrecognised_verdict_withholds_the_power_to_strip() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;

    // A withdrawal that genuinely IS authoritative today, so the fixture proves the seam
    // is live: before the swap it strips, after the swap it must not. Anything less and
    // "still sequestered" could just mean the withdrawal never worked in the first place.
    let withdraws_hex = hex::encode(content_address_of(&c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "mine to lower",
    };
    let wid = Uuid::now_v7();
    apply_remote_raw(&c, &sk_h, withdrawal_body_with_id(p, wid, &kid_h, &w, 20))
        .await
        .expect("an un-attested self-withdrawal must land at the remote door");

    // Precondition: with the REAL predicate this withdrawal is authoritative and the grade
    // has already fallen. This is what makes the post-swap assertion meaningful.
    assert_eq!(authority(&c, wid, Some(a)).await, "self");
    assert_eq!(
        effective(&c, a).await,
        "routine",
        "fixture precondition: the withdrawal must genuinely strip before the swap, or \
         the assertion after it proves nothing"
    );

    c.batch_execute(FOURTH_VERDICT)
        .await
        .expect("stage a future fourth verdict");
    let observed = effective(&c, a).await;
    // Restored BEFORE asserting, so a failure still leaves the database usable.
    c.batch_execute(DB005)
        .await
        .expect("restore db/005 after the staged verdict");

    assert_eq!(
        observed, "sequestered",
        "an unrecognised verdict must WITHHOLD the power to strip, never confer it — a \
         negative `<> 'unverified'` test would hand every future verdict the power to \
         lower a grade, silently and with the suite green (#410 finding C3)"
    );

    // And the restore genuinely put the real predicate back, so this test cannot leave a
    // stub behind that would quietly disarm every sibling test in the file.
    assert_eq!(authority(&c, wid, Some(a)).await, "self");
}
