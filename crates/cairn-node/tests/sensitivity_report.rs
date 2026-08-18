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
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
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
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
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
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
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
        report
            .standing
            .iter()
            .map(|s| &s.content_address)
            .collect::<Vec<_>>()
    );
}

/// The deferred reader is a `SECURITY DEFINER` on purpose, and this pins WHY rather than
/// merely that it works: if `cairn_agent` could read `event_deferred` directly, the definer
/// would be decorative and the next reader would delete it as ceremony.
#[tokio::test]
async fn the_deferred_reader_is_a_load_bearing_definer_not_decoration() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    // 1. cairn_agent genuinely cannot read the table. This is the fact that makes the
    //    definer necessary — and it is #425's territory: the runtime login role reaches it
    //    today only by cairn_node membership, which is exactly what must not be relied on.
    let direct: bool = c
        .query_one(
            "SELECT has_table_privilege('cairn_agent', 'event_deferred', 'SELECT')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !direct,
        "event_deferred is readable by cairn_agent — the definer in db/043 is then \
         decoration, and this test should be replaced by a plain grant"
    );

    // 2. The function is a definer AND pins pg_temp last (#426). A definer reading
    //    event_log unqualified without that clause can be blinded to ZERO ROWS by any
    //    caller, which this surface would render as "nothing is deferred".
    let row = c
        .query_one(
            "SELECT p.prosecdef, COALESCE(array_to_string(p.proconfig, ','), '')
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
              WHERE p.proname = 'cairn_patient_deferred_sensitivity'",
            &[],
        )
        .await
        .unwrap();
    let is_definer: bool = row.get(0);
    let cfg: String = row.get(1);
    assert!(is_definer, "must be SECURITY DEFINER");
    assert!(
        cfg.contains("pg_temp"),
        "search_path must pin pg_temp (#426): {cfg}"
    );

    // 3. And it is scoped: a chart with nothing deferred reports nothing.
    let n: i64 = c
        .query_one(
            "SELECT count(*)::bigint FROM cairn_patient_deferred_sensitivity(
                 '00000000-0000-0000-0000-0000000000ff'::uuid)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "a chart with no deferred sensitivity events must report none"
    );
}
