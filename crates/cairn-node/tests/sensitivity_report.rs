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
        .withdrawals_needing_review
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
        .withdrawals_needing_review
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
    // The EXACT setting, not `contains("pg_temp")`. `search_path=pg_temp, public` also
    // contains it and is the precise ordering #426 exists to forbid, so the loose assertion
    // was a comment claiming a guarantee it did not provide — the defect class this whole
    // slice is about. db/tests/049 pins the exact string for the same reason and says so.
    // (`search_path_pg_temp.rs` sweeps every definer repo-wide and is the real guard; this
    // pins THIS function so a local edit fails here first, with a message naming the file.)
    assert_eq!(
        cfg, "search_path=public, pg_temp",
        "db/043's definer must pin exactly `public, pg_temp` (#426): {cfg}"
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

/// Insert a raw `event_log` row and mark it deferred, the way
/// `db/tests/043_deferred_readjudication_test.sql` does.
///
/// A DIRECT insert rather than a door, deliberately: the point is to produce the ADR-0056
/// admit-and-defer state for a type this build does not recognise, which by definition
/// cannot be reached through a door that knows the type.
async fn defer_event(c: &tokio_postgres::Client, patient: Uuid, event_id: Uuid, event_type: &str) {
    c.execute(
        "INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         VALUES ($1::text::uuid, $2::text::uuid, $3, 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node',
             ('defer-' || $1)::bytea,
             '\\x1220'::bytea || digest(('defer-' || $1)::bytea, 'sha256'),
             '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe')",
        &[&event_id.to_string(), &patient.to_string(), &event_type],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) VALUES ($1::text::uuid, $2)",
        &[&event_id.to_string(), &event_type],
    )
    .await
    .unwrap();
}

/// The deferred arm, with rows in it.
///
/// Its sibling `the_deferred_reader_is_a_load_bearing_definer_not_decoration` asserts the
/// EMPTY case, which alone would pass over a function body of `WHERE false` — the mutation
/// the review found nothing could catch. This pins the other half: rows reach the report,
/// only this chart's rows do, and only the `sensitivity.%` ones.
#[tokio::test]
async fn a_deferred_sensitivity_event_reaches_the_report_scoped_to_its_own_chart() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    let other = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let mine = Uuid::now_v7();
    defer_event(&c, p, mine, "sensitivity.grade-future.asserted").await;
    // Same chart, NOT a sensitivity type — must not be counted by this block.
    let unrelated = Uuid::now_v7();
    defer_event(&c, p, unrelated, "clinical.future-thing.asserted").await;
    // A sensitivity type on ANOTHER chart — pins that the definer honours its argument.
    let theirs = Uuid::now_v7();
    defer_event(&c, other, theirs, "sensitivity.grade-future.asserted").await;

    let report = chart_sensitivity(&mut c, p).await.unwrap();

    let ids: Vec<Uuid> = report.deferred.iter().map(|d| d.event_id).collect();
    assert!(
        ids.contains(&mine),
        "this chart's deferred sensitivity event must reach the report: {ids:?}"
    );
    assert!(
        !ids.contains(&theirs),
        "another chart's deferred event leaked in — the definer ignored its argument: {ids:?}"
    );
    assert!(
        !ids.contains(&unrelated),
        "a non-sensitivity deferred event was counted as a sensitivity grade: {ids:?}"
    );
    let row = report.deferred.iter().find(|d| d.event_id == mine).unwrap();
    assert_eq!(row.event_type, "sensitivity.grade-future.asserted");
    assert!(
        row.adjudication_error.is_none(),
        "no re-adjudication has failed yet, so the error must be absent, not empty"
    );
}

/// The overclaim arm, with a row in it.
///
/// Reuses `safety_overclaim.rs`'s fixture rather than re-deriving what makes a rung an
/// overclaim, so the two suites cannot drift on it.
#[tokio::test]
async fn a_recorded_safety_overclaim_reaches_the_report() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = common::medication_setup(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    let ca = common::submit_medication_with_raw_safety(
        &c,
        &sk,
        &kid,
        &sk_h,
        &kid_h,
        p,
        20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await
    .expect("ADMITTED — an advisory field may never cancel a clinical write (ADR-0060)");

    let report = chart_sensitivity(&mut c, p).await.unwrap();
    let hex_ca = hex::encode(&ca);
    let row = report
        .overclaims
        .iter()
        .find(|o| o.content_address == hex_ca)
        .expect("the recorded overclaim must reach the report (#405 part 2)");
    // DIRECTION IS THE MEANING: emitted finer than licensed is over-disclosure. Transposing
    // these two columns would report a disclosure incident as an over-cautious one.
    assert_eq!(row.emitted_rung, "precise");
    assert_eq!(row.licensed_rung, "existence");
}

/// A sealed medication event this node cannot open makes the thread list incomplete, and
/// the report must carry that as a MEASURED count rather than inferring it.
#[tokio::test]
async fn sealed_medication_without_custody_is_counted_not_inferred() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    let before = chart_sensitivity(&mut c, p).await.unwrap();
    assert_eq!(
        before.sealed_medication_events_without_custody, 0,
        "nothing sealed yet"
    );

    // A sealed medication event with NO event_clear row: exactly what a node that received
    // this chart by sync but holds no DEK has. medication_statement_apply RETURNs early on
    // a NULL clear payload (db/031), so this projects no thread and the old report called
    // the chart empty.
    let e = Uuid::now_v7();
    c.execute(
        "INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin, sealed)
         VALUES ($1::text::uuid, $2::text::uuid, 'clinical.medication.asserted', 'test-1',
             (extract(epoch from now()) * 1000)::bigint, 0, 'test-node',
             ('sealed-' || $1)::bytea,
             '\\x1220'::bytea || digest(('sealed-' || $1)::bytea, 'sha256'),
             '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe', TRUE)",
        &[&e.to_string(), &p.to_string()],
    )
    .await
    .unwrap();

    let after = chart_sensitivity(&mut c, p).await.unwrap();
    assert_eq!(
        after.sealed_medication_events_without_custody, 1,
        "the node holds one sealed medication event it cannot open"
    );
    assert!(
        after.threads.is_empty(),
        "and it projects no thread for it — which is exactly why the count is needed"
    );
}

/// Both new definers must be reachable by the role the product actually reads as.
///
/// A missing or wrong grant is INVISIBLE under the test superuser every other test in this
/// file runs as — `claim_authority_worklist.rs::the_worklist_is_readable_as_cairn_agent`
/// exists for the same reason. This is the check that would have caught the review's
/// finding that the first draft granted `cairn_patient_deferred_sensitivity` to
/// `cairn_agent` only, while the runtime is provisioned as a `cairn_node` member (#425).
#[tokio::test]
async fn the_new_definers_are_executable_by_both_group_roles() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let p = Uuid::now_v7();

    for role in ["cairn_agent", "cairn_node"] {
        c.batch_execute(&format!("SET ROLE {role}")).await.unwrap();
        let deferred = c
            .query(
                "SELECT event_id FROM cairn_patient_deferred_sensitivity($1::text::uuid)",
                &[&p.to_string()],
            )
            .await;
        let custody = c
            .query_one(
                "SELECT cairn_patient_sealed_medication_without_custody($1::text::uuid)",
                &[&p.to_string()],
            )
            .await;
        c.batch_execute("RESET ROLE").await.unwrap();
        deferred.unwrap_or_else(|e| {
            panic!("{role} cannot execute cairn_patient_deferred_sensitivity: {e}")
        });
        custody.unwrap_or_else(|e| {
            panic!("{role} cannot execute cairn_patient_sealed_medication_without_custody: {e}")
        });
    }
}
