//! ADR-0076 decision 1: `cairn_person_charts` is the ONE answer to "which charts are this
//! person", read by every combined read. A chart never linked is a set of one; a link
//! component is read whole from any member; an unlink splits it again.
mod common;
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_medication_view::ChartSet;
use cairn_node::db;
use cairn_node::patient::person::{chart_identities, person_charts};
use common::{
    chart_named, cs, medication_setup as setup, submit_link_event, submit_registration,
    submit_signed, EventSpec,
};
use uuid::Uuid;

async fn fresh(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

#[tokio::test]
async fn a_never_linked_chart_is_a_set_of_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
}

#[tokio::test]
async fn a_linked_pair_is_one_set_from_either_side() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let both = ChartSet::new([a, b]).unwrap();
    assert_eq!(person_charts(&c, a).await.unwrap(), both);
    assert_eq!(
        person_charts(&c, b).await.unwrap(),
        both,
        "the same set from the other side"
    );
}

#[tokio::test]
async fn a_transitive_cluster_is_one_set_from_every_member() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    let x = fresh(&c, &sk, &kid).await;
    // a–b and b–x: a and x were never linked to each other, and are still one person.
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, b, x, 11, true).await;
    let all = ChartSet::new([a, b, x]).unwrap();
    for member in [a, b, x] {
        assert_eq!(
            person_charts(&c, member).await.unwrap(),
            all,
            "from {member}"
        );
    }
}

#[tokio::test]
async fn an_unlinked_chart_reads_alone_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
    assert_eq!(person_charts(&c, b).await.unwrap(), ChartSet::single(b));
    // Relinked: the set follows the standing edge, not the first one ever written.
    submit_link_event(&c, &sk, &kid, a, b, 12, true).await;
    assert_eq!(
        person_charts(&c, a).await.unwrap(),
        ChartSet::new([a, b]).unwrap()
    );
}

/// One `demographic.field.asserted` dob event at wall-clock `wall == 5` (fixed by the task
/// brief's literal test body, not chosen here) — a thin wrapper so the test body below reads
/// as the scenario, not the event plumbing.
async fn dob(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: Uuid,
    v: &str,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(v, "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin(v, "day", "patient-stated")),
            wall: 5,
        },
    )
    .await
    .expect("dob accepted");
}

/// ADR-0076 decision 1: the header lists each linked chart's OWN name and dob, never a
/// merged winner. Two duplicates named "SMITH"/"SMYTHE" with day/month-swapped dobs are
/// exactly the case where picking a winner would erase the very discrepancy that revealed
/// the duplicate — and a third, bare chart in the same cluster proves an absent name/dob
/// reads as `None`, never as an empty string or a borrowed value from a linked sibling.
#[tokio::test]
async fn each_member_reports_its_own_name_and_date() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let smith = chart_named(&c, &sk, &kid, 1, "Mary SMITH").await;
    let smythe = chart_named(&c, &sk, &kid, 1, "Mary SMYTHE").await;
    let bare = fresh(&c, &sk, &kid).await; // no name, no date
    dob(&c, &sk, &kid, smith, "1950-07-01").await;
    dob(&c, &sk, &kid, smythe, "1950-01-07").await; // the day/month slip that made the duplicate
    submit_link_event(&c, &sk, &kid, smith, smythe, 10, true).await;
    submit_link_event(&c, &sk, &kid, smythe, bare, 11, true).await;

    let set = person_charts(&c, smith).await.unwrap();
    let lines = chart_identities(&c, &set).await.unwrap();
    assert_eq!(
        lines.iter().map(|l| l.patient_id).collect::<Vec<_>>(),
        set.members().to_vec(),
        "one line per member, in set order"
    );
    let of = |p: Uuid| lines.iter().find(|l| l.patient_id == p).unwrap();
    assert_eq!(of(smith).name.as_deref(), Some("Mary SMITH"));
    assert_eq!(of(smith).birth_date.as_deref(), Some("1950-07-01"));
    assert_eq!(
        of(smythe).birth_date.as_deref(),
        Some("1950-01-07"),
        "never a merged winner"
    );
    assert_eq!(
        of(bare).name,
        None,
        "absence is None, never an empty string"
    );
    assert_eq!(of(bare).birth_date, None);
    assert_eq!(of(smith).trust, "confirmed");
}

/// A member's trust state is READ, not defaulted. The test above only ever sees "confirmed",
/// which the no-row default would produce even if the trust read matched nothing at all. A
/// link this node's hard veto flagged (db/018 `link_veto_flag`) makes both charts read
/// `under-review` in `chart_trust` — exactly the state the combined header must not paper
/// over. The flag is set directly: its lifecycle is db/018's, pinned by `link_veto_floor.rs`.
#[tokio::test]
async fn a_member_under_review_reads_under_review() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let x = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&a.min(x).to_string(), &a.max(x).to_string()],
    )
    .await
    .unwrap();

    let set = person_charts(&c, a).await.unwrap();
    let lines = chart_identities(&c, &set).await.unwrap();
    assert!(lines.iter().all(|l| l.held));
    assert!(lines.iter().all(|l| l.trust == "under-review"), "{lines:?}");
}

/// A link can name a chart whose registration this node does not hold: a link event is not
/// refused for naming an unknown subject, it can sync ahead of that chart's registration, and
/// a scope-limited node (ADR-0004) may never receive the other chart at all. The set still
/// includes it (the link stands), but its line must not claim "identity confirmed" — the
/// no-`chart_trust`-row default is true only of a chart that exists here. Principle 4: an
/// unknown identity is `unknown`, and `held` says why the name and date are absent.
#[tokio::test]
async fn a_member_this_node_does_not_hold_is_not_confirmed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let elsewhere = Uuid::now_v7(); // never registered, nothing about it held here
    submit_link_event(&c, &sk, &kid, a, elsewhere, 10, true).await;

    let set = person_charts(&c, a).await.unwrap();
    assert_eq!(set, ChartSet::new([a, elsewhere]).unwrap());
    let lines = chart_identities(&c, &set).await.unwrap();
    let of = |p: Uuid| lines.iter().find(|l| l.patient_id == p).unwrap();
    assert!(of(a).held);
    assert_eq!(of(a).trust, "confirmed");
    assert!(
        !of(elsewhere).held,
        "its registration is not held on this node"
    );
    assert_eq!(
        of(elsewhere).trust,
        "unknown",
        "never the no-row default of `confirmed`"
    );
    assert_eq!(of(elsewhere).name, None);
}
