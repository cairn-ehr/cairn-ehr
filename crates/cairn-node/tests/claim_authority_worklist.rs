//! The §5.9 withdrawal worklist (ADR-0064). Two rows, two reasons: `inert` is what the
//! gate stopped (transient, self-clearing), `stranger-attested` is what the gate LET
//! THROUGH and nobody would otherwise see.
//!
//! Both `assert_chart_grade` and `bearing_withdrawal_body` are `common/mod.rs` helpers
//! promoted out of `claim_authority.rs`'s file-local copies for this suite (Task 4) — see
//! that module's own doc comments for the full shape rationale.
mod common;
use cairn_event::sensitivity::*;
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    content_address_of, cs, enroll_human, setup, submit_attested, submit_registration,
    withdrawal_body_with_id,
};
use uuid::Uuid;

/// The worklist's `reason` column for every row on `patient`, ordered for a deterministic
/// assertion. Empty is a real, checkable answer — but see `effective_grade` below for why
/// no test in this file trusts an empty list on its own.
async fn reasons(c: &tokio_postgres::Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT reason FROM sensitivity_withdrawal_worklist
          WHERE patient_id = $1::text::uuid ORDER BY reason",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// The chart-wide effective grade for `event` — the positive control every test below
/// pairs with a `reasons()` read. `reasons() == []` is BOTH "the routine case, correctly
/// silent" AND "the fixture never built anything" (the trap task-4's own review finding
/// names); querying the grade a withdrawal claims to have moved is what tells those two
/// apart, the same discipline `claim_authority.rs`'s `effective` helper already applies to
/// the predicate's consequence, re-derived here because this file is a separate suite.
async fn effective_grade(c: &tokio_postgres::Client, event: Uuid) -> String {
    c.query_one(
        "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
        &[&event.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn an_inert_withdrawal_is_listed_and_clears_when_it_becomes_authoritative() {
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
    let target = content_address_of(&c, a).await;

    // An un-attested withdrawal — the plain "recorded" shape, the only shape genuinely
    // un-attested peer traffic ever carries. The LOCAL door refuses this shape outright
    // (db/048's ceremony: "withdrawing a grade requires a bound human author"), so it can
    // only ever land through the REMOTE door — exactly like a real cross-node write with
    // no human behind it yet.
    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "strip",
    };
    apply_remote_raw(
        &c,
        &sk,
        withdrawal_body_with_id(p, Uuid::now_v7(), &kid, &w, 20),
    )
    .await
    .expect("ADMITTED — authority gates effect, never admission");
    assert_eq!(reasons(&c, p).await, vec!["inert"]);

    // Re-issued as a peer's HONESTLY COMPLETED ceremony this time: the same device
    // relays, but a human now vouches. The grade lowers, and the FIRST withdrawal's row
    // disappears from the worklist — not because ITS OWN verdict changed (it cannot:
    // that event's `attester_key` stays NULL forever, so `cairn_claim_authority` on that
    // row alone can never move off 'unverified'), but because the view asks the CURRENT
    // question — is the TARGET still standing — rather than replaying a stamped
    // per-row verdict. Once ANY authoritative withdrawal has stripped the assertion, a
    // sibling inert withdrawal of the SAME target is moot: there is nothing left to heal
    // towards, and leaving it listed would be pure noise.
    let w2 = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "consented",
    };
    let body2 = bearing_withdrawal_body(&kid, &kid_h, p, Uuid::now_v7(), &w2, 30);
    apply_remote_attested(&c, &sk, body2, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");
    assert!(!reasons(&c, p).await.contains(&"inert".to_string()));

    // Positive control: the grade genuinely fell (not just "no longer inert" — actually
    // lowered), and the SECOND withdrawal is exactly why the worklist cannot go fully
    // silent here — `sk_h` has authored nothing else on this chart, so it surfaces as
    // 'stranger-attested'. A view that always answered 'stranger-attested' regardless of
    // input would also pass the `!contains("inert")` check above; this pins the actual
    // remaining row instead of trusting that absence alone.
    assert_eq!(effective_grade(&c, a).await, "routine");
    assert_eq!(reasons(&c, p).await, vec!["stranger-attested"]);
}

#[tokio::test]
async fn an_attested_withdrawal_from_a_stranger_to_the_chart_is_listed() {
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

    // The attesting human has authored nothing else on this chart, and the withdrawal was
    // authored elsewhere. It CLEARS the bar — the grade does lower — and that is exactly
    // why it must be visible: accountability is the control, the gate is only the forcing
    // function (ADR-0064).
    let target = content_address_of(&c, a).await;
    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "consented",
    };
    let body = bearing_withdrawal_body(&kid, &kid_h, p, Uuid::now_v7(), &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");

    assert_eq!(reasons(&c, p).await, vec!["stranger-attested"]);
    // Positive control: the withdrawal actually took effect — the case the worklist
    // exists to surface precisely BECAUSE it succeeded invisibly everywhere else.
    assert_eq!(effective_grade(&c, a).await, "routine");
}

#[tokio::test]
async fn a_local_clinicians_own_withdrawal_is_not_on_the_worklist() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // The human authored content on this chart first (the assertion itself), then
    // withdraws it herself, through the LOCAL door — the same signer-is-attester shape
    // production's own `sensitivity::withdraw_sensitivity` uses ("the SAME human key both
    // signs the event envelope AND mints the attestation token"). This is the ROUTINE
    // case a worklist must stay silent about: a clinician who has been on this chart all
    // along, lowering their own grade with a properly completed ceremony.
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;
    let target = content_address_of(&c, a).await;
    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "consented",
    };
    let body = bearing_withdrawal_body(&kid_h, &kid_h, p, Uuid::now_v7(), &w, 20);
    submit_attested(&c, &sk_h, body, &sk_h, &kid_h)
        .await
        .expect("the local ceremony accepts a bound human author's own withdrawal");

    // POSITIVE CONTROL — the finding this task must not reproduce: `reasons() == []` is
    // BOTH "correctly no rows" and "the fixture never built anything" (`cairn_effective_
    // sensitivity`'s own COALESCE default is ALSO 'routine' when nothing was ever
    // asserted at all). Prove the withdrawal actually landed AND took effect before
    // trusting the silence below.
    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE withdraws = $1",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        landed, 1,
        "precondition: the withdrawal event itself must have landed"
    );
    assert_eq!(
        effective_grade(&c, a).await,
        "routine",
        "precondition: the withdrawal must actually have lowered the grade"
    );

    // The routine case must produce NO noise, or the worklist is unusable on day one
    // (§5.12 alert fatigue — the disease this project names as the enemy).
    assert!(reasons(&c, p).await.is_empty());
}
