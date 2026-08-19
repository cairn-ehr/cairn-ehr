//! #435 — `sensitivity-withdraw` reports what its withdrawal actually achieved.
//!
//! The verb used to print `withdrew <hex> on chart <uuid>` unconditionally, whatever
//! happened. A withdrawal that lands INERT is the headline subject of the whole §5.9 slice
//! (ADR-0064: the claim is admitted, the POWER is withheld), so an operator reading
//! "withdrew" had to independently know to run `patient-sensitivity` afterwards to discover
//! that nothing had moved.
//!
//! The unit tests in `sensitivity/render.rs` pin the WORDING of the four outcomes; these
//! pin that the reader is actually fed the right rows — a perfectly-worded read-back over a
//! query that never fires is the same silence one layer down.
//!
//! WHY TWO FACTS AND NOT ONE. db/048's `sensitivity_withdrawal_worklist` is a union whose
//! two arms mean OPPOSITE things, and its `inert` arm additionally merges two states its
//! own comment separates: "nobody accountable stands behind this" and "the target has not
//! replicated here yet". Neither the worklist alone nor the standing set alone can tell the
//! four operator stories apart, which is why `WithdrawOutcome` carries both and the tests
//! below are one per story.
mod common;
use cairn_event::sensitivity::SensitivityWithdrawal;
use cairn_node::sensitivity::readback::{SubjectResolution, TargetState};
use cairn_node::sensitivity::withdraw_readback;
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    content_address_of, cs, enroll_human, setup, submit_registration, withdrawal_body_with_id,
};
use uuid::Uuid;

/// The tables this suite writes through, truncated by `setup`.
const TABLES: &[&str] = &["sensitivity_assertion", "sensitivity_withdrawal"];

/// A well-shaped `content_address` hex that names NO assertion on this node.
///
/// Derived by flipping the last nibble of a real one rather than inventing bytes, so it
/// keeps the exact length and multihash shape db/048 decodes — the test must exercise
/// "absent", never "malformed", which is a different refusal entirely.
fn absent_address(real_hex: &str) -> String {
    let (head, last) = real_hex.split_at(real_hex.len() - 1);
    let flipped = if last == "0" { "1" } else { "0" };
    format!("{head}{flipped}")
}

#[tokio::test]
async fn an_inert_withdrawal_reads_back_as_the_target_still_standing() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, TABLES).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let target_hex = hex::encode(content_address_of(&c, a).await);

    // Un-attested: the LOCAL door refuses this shape outright (db/048's ceremony demands a
    // bound human author), so it can only land through the REMOTE door — which is correct,
    // because ADR-0064 gates EFFECT, never admission. It lands, converges, and moves
    // nothing.
    let w_id = Uuid::now_v7();
    let w = SensitivityWithdrawal {
        withdraws_hex: &target_hex,
        rationale: "consent withdrawn by patient",
    };
    apply_remote_raw(&c, &sk, withdrawal_body_with_id(p, w_id, &kid, &w, 20))
        .await
        .expect("ADMITTED — authority gates effect, never admission");

    let o = withdraw_readback(&mut c, p, w_id, &target_hex)
        .await
        .unwrap();

    assert_eq!(
        o.worklist_reason.as_deref(),
        Some("inert"),
        "the accountability fact: nobody this node can hold responsible stands behind it"
    );
    match o.target {
        TargetState::Held {
            still_standing,
            subject: SubjectResolution::Resolved(r),
        } => {
            assert!(
                still_standing,
                "THE DEFECT #435 EXISTS FOR: the grade did not move, and the verb printed \
                 plain success"
            );
            assert_eq!(
                r.grade, "sequestered",
                "the unmoved grade must be readable, not merely 'something still stands'"
            );
        }
        _ => panic!("the target assertion is held here and its subject kind is a known one"),
    }
}

#[tokio::test]
async fn a_stranger_attested_withdrawal_reads_back_as_having_taken_effect() {
    // The arm that must NEVER be reported as a failure: it cleared ADR-0064's bar and
    // protection WAS removed. It is on the worklist as salience — an unaccountable removal
    // is a thing to see — and the two facts therefore disagree in direction, which is
    // exactly why they are carried separately.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let target_hex = hex::encode(content_address_of(&c, a).await);

    let w_id = Uuid::now_v7();
    let w = SensitivityWithdrawal {
        withdraws_hex: &target_hex,
        rationale: "administrative correction",
    };
    let body = bearing_withdrawal_body(&kid, &kid_h, p, w_id, &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");

    let o = withdraw_readback(&mut c, p, w_id, &target_hex)
        .await
        .unwrap();

    assert_eq!(
        o.worklist_reason.as_deref(),
        Some("stranger-attested"),
        "attested by an enrolled human with no prior presence — NOT inert"
    );
    match o.target {
        TargetState::Held {
            still_standing,
            subject: SubjectResolution::Resolved(r),
        } => {
            assert!(
                !still_standing,
                "the withdrawal TOOK EFFECT — the target must be gone from the standing set"
            );
            assert_eq!(r.grade, "routine", "the protection really came off");
        }
        _ => panic!("the target assertion is held here and its subject kind is a known one"),
    }
}

#[tokio::test]
async fn an_accountable_withdrawal_with_prior_presence_raises_nothing() {
    // The anti-vacuity control. If the ordinary, wholly-correct case also produced a
    // worklist reason, every outcome would carry a warning and the operator would learn to
    // ignore all of them.
    //
    // The human authors the assertion FIRST, which is what gives them the prior presence
    // db/048 section 11's second arm looks for — without it this same withdrawal lands on
    // the worklist as `stranger-attested` (#415).
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;
    let target_hex = hex::encode(content_address_of(&c, a).await);

    let w_id = cairn_node::sensitivity::withdraw_sensitivity(
        &mut c,
        &sk_h,
        &kid_h,
        "test-node",
        p,
        &target_hex,
        "patient consent",
    )
    .await
    .expect("an enrolled human's own withdrawal goes through the local door");

    let o = withdraw_readback(&mut c, p, w_id, &target_hex)
        .await
        .unwrap();

    assert_eq!(
        o.worklist_reason, None,
        "an accountable withdrawal by someone already present on this chart is not a \
         worklist item"
    );
    match o.target {
        TargetState::Held {
            still_standing,
            subject: SubjectResolution::Resolved(r),
        } => {
            assert!(
                !still_standing,
                "the target must be gone from the standing set"
            );
            assert_eq!(r.grade, "routine");
        }
        _ => panic!("the target assertion is held here and its subject kind is a known one"),
    }
}

#[tokio::test]
async fn a_withdrawal_whose_target_never_arrived_says_so_rather_than_guessing() {
    // Routine in ordinary federated operation: set-union sync has no ordering, and db/048
    // deliberately keeps NO foreign key from a withdrawal to its target for exactly this
    // reason. The worklist would list such a row under `inert` — the same word it uses for
    // "nobody accountable" — so only a direct look at `sensitivity_assertion` separates
    // the two, and this node must claim NEITHER outcome.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // Gives the human prior presence, so the ONLY unusual thing about the outcome below is
    // the missing target.
    let a = assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;
    let never_arrived = absent_address(&hex::encode(content_address_of(&c, a).await));

    let w_id = cairn_node::sensitivity::withdraw_sensitivity(
        &mut c,
        &sk_h,
        &kid_h,
        "test-node",
        p,
        &never_arrived,
        "patient consent",
    )
    .await
    .expect("a withdrawal may legitimately precede the assertion it withdraws");

    let o = withdraw_readback(&mut c, p, w_id, &never_arrived)
        .await
        .unwrap();

    assert!(
        matches!(o.target, TargetState::NotHeldHere),
        "nothing is known about an assertion this node does not hold"
    );
}

#[tokio::test]
async fn a_withdrawal_naming_another_charts_assertion_is_not_reported_as_effective() {
    // ADR-0064's KNOWN GAP — recorded there as "not fixed, and not exercised by any test".
    // `cairn_sensitivity_standing` is patient-scoped on BOTH sides, which is what stops a
    // withdrawal authored on chart B from stripping chart A. The cost is that a withdrawal
    // mis-stamped with the wrong chart's patient_id, naming a real assertion that lives on
    // another chart, finds nothing in the standing set of its own chart on any read, ever —
    // and falls out of the worklist's `inert` arm too, which asks whether the target still
    // stands on the WITHDRAWAL's chart, where it never did.
    //
    // Neither door refuses the shape: the ceremony's chart-mismatch checks live in the
    // ASSERTION branch only. So an operator typo produces a signed, replicated, permanently
    // inert act — and, before this read-back, a plain "withdrew" line.
    //
    // A naive membership test would report "no longer stands" here, because the target
    // genuinely is not in THIS chart's standing set. That is the reassuring direction of
    // wrong, which is the dangerous one on a confidentiality surface.
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let chart_a = Uuid::now_v7();
    let chart_b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, chart_a, 1).await;
    submit_registration(&c, &sk, &kid, chart_b, 2).await;

    // The real assertion lives on chart B and must stay standing there throughout.
    let a = assert_chart_grade(&c, &sk_h, &kid_h, chart_b, 10, "sequestered").await;
    let target_hex = hex::encode(content_address_of(&c, a).await);

    // ... but the withdrawal is stamped chart A. The operator typo, in one line.
    let w_id = cairn_node::sensitivity::withdraw_sensitivity(
        &mut c,
        &sk_h,
        &kid_h,
        "test-node",
        chart_a,
        &target_hex,
        "patient consent",
    )
    .await
    .expect("neither door refuses a mis-chart withdrawal — that is the gap");

    let o = withdraw_readback(&mut c, chart_a, w_id, &target_hex)
        .await
        .unwrap();

    assert!(
        matches!(o.target, TargetState::OnAnotherChart),
        "the target is held here but belongs to another chart — reporting it as withdrawn \
         would tell the operator protection came off when nothing moved anywhere"
    );

    // The control that makes the above mean something: chart B is untouched.
    let b = cairn_node::sensitivity::chart_sensitivity(&mut c, chart_b)
        .await
        .unwrap();
    assert_eq!(
        b.chart_grade, "sequestered",
        "the protection this withdrawal named is still standing on its own chart"
    );
}
