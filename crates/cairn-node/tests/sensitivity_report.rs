//! #388 / ADR-0064 §1.2 — the operator surface answers "why did this withdrawal not take
//! effect?" in ONE query with no raw SQL.
//!
//! The unit tests in `sensitivity/render.rs` pin the WORDING; these pin that the report is
//! actually fed the rows. Both halves are needed: a perfectly-worded report over an empty
//! query is exactly the silence this slice exists to end.
//!
//! Fixtures are the ones `claim_authority_worklist.rs` established for the same two rows —
//! reused rather than re-derived, so a change to what makes a withdrawal inert cannot leave
//! these two suites disagreeing about it.
mod common;
use cairn_event::sensitivity::SensitivityWithdrawal;
use cairn_node::sensitivity::chart_sensitivity;
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    content_address_of, cs, enroll_human, setup, submit_registration, withdrawal_body_with_id,
};
use uuid::Uuid;

#[tokio::test]
async fn an_un_attested_withdrawal_is_reported_as_inert_with_its_reason_and_rationale() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let target = content_address_of(&c, a).await;

    // An un-attested withdrawal — the plain "recorded" shape, the only shape genuinely
    // un-attested peer traffic ever carries. The LOCAL door refuses it outright (db/048's
    // ceremony demands a bound human author), so it can only land through the REMOTE door.
    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "consent withdrawn by patient",
    };
    apply_remote_raw(
        &c,
        &sk,
        withdrawal_body_with_id(p, Uuid::now_v7(), &kid, &w, 20),
    )
    .await
    .expect("ADMITTED — authority gates effect, never admission");

    let report = chart_sensitivity(&mut c, p).await.unwrap();

    let hex_target = hex::encode(&target);
    let row = report
        .ineffective_withdrawals
        .iter()
        .find(|x| x.withdraws == hex_target)
        .expect("the un-attested withdrawal must appear on the report (#380/ADR-0064)");
    assert_eq!(row.reason, "inert");
    assert_eq!(
        row.rationale, "consent withdrawn by patient",
        "the §1.2 budget needs the RATIONALE, not just the fact of failure"
    );
    // And the grade really did NOT drop — the report describes a live state, not a
    // hypothetical one. Without this the test would pass over a report that lists rows
    // nobody is actually being harmed by.
    assert_eq!(report.chart_grade, "sequestered");
}

#[tokio::test]
async fn an_attested_stranger_s_withdrawal_is_reported_with_its_own_reason_and_actor() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let target = content_address_of(&c, a).await;

    // Attested by an enrolled human who has authored nothing else on this chart. It CLEARS
    // the bar — the grade lowers — which is precisely why it must be visible: the gate is
    // the forcing function, accountability is the control (ADR-0064).
    let w = SensitivityWithdrawal {
        withdraws_hex: &hex::encode(&target),
        rationale: "administrative correction",
    };
    let body = bearing_withdrawal_body(&kid, &kid_h, p, Uuid::now_v7(), &w, 20);
    apply_remote_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("a properly attested cross-node withdrawal must land");

    let report = chart_sensitivity(&mut c, p).await.unwrap();

    let row = report
        .ineffective_withdrawals
        .first()
        .expect("an attested-but-stranger withdrawal belongs on the report");
    assert_eq!(
        row.reason, "stranger-attested",
        "attested by an enrolled human, so NOT inert — the two reasons must not collapse"
    );
    assert!(
        row.responsible_actor_id.is_some(),
        "#421: the accountable actor must be named, or the row cannot be acted on"
    );
    // The positive control: this one DID take effect, unlike the inert case above.
    assert_eq!(report.chart_grade, "routine");
}

#[tokio::test]
async fn a_chart_with_standing_assertions_and_no_projected_threads_still_names_them() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    // Authoring no medication events at all is the CHEAP stand-in for a custody-thin node:
    // both produce zero medication_statement rows, which is the condition the report
    // branches on. It does NOT reproduce the custody path itself — a node holding sealed
    // medication events without a DEK — so this pins the BRANCH, not the cause. Stated
    // rather than left for a reader to assume the harder case is covered.
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk, &kid, p, 10, "restricted").await;
    let ca = hex::encode(content_address_of(&c, a).await);

    let report = chart_sensitivity(&mut c, p).await.unwrap();
    assert!(
        report.threads.is_empty(),
        "no medication events were authored, so nothing projects"
    );
    assert!(
        report.standing.iter().any(|s| s.content_address == ca),
        "the standing assertion must be NAMED, not merely counted (#383): {:?}",
        report.standing.iter().map(|s| &s.content_address).collect::<Vec<_>>()
    );
}
