//! R3 (ADR-0076 decision 6): the search-before-create front door lists PEOPLE - the link
//! component of charts - not charts. Each test pins one rule of `search_patients`'s grouping:
//! a linked pair is one row; the matched member leads its row; an unmatched member is shown
//! and named (a registration signs "displayed" over every member); people rank by their best
//! member; a member this node does not hold reads `Unknown` and does NOT make the search
//! `incomplete`; and a never-linked search is exactly one row per chart, as before.
mod common;

use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_node::{db, patient::search::search_patients};
use cairn_patient_search::{CandidateList, SearchQuery, TrustState};
use common::{body_from_spec, chart_named, cs, submit_link_event, EventSpec};
use tokio_postgres::Client;
use uuid::Uuid;

const EXTRA: [&str; 6] = [
    "patient_name",
    "patient_link",
    "person_member",
    "link_veto_flag",
    "chart_dispute",
    "patient_registration",
];

async fn find(c: &Client, typed: &str) -> CandidateList {
    search_patients(c, &SearchQuery::new(typed, None, &[]), "2026-10-03")
        .await
        .expect("search runs")
}

/// The chart ids of each row, in display order.
fn rows(list: &CandidateList) -> Vec<Vec<Uuid>> {
    list.people
        .iter()
        .map(|p| p.members().iter().map(|m| m.patient_id).collect())
        .collect()
}

#[tokio::test]
async fn a_linked_pair_is_one_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let a = chart_named(&c, &sk, &kid, 10, "Mary Smith").await;
    let b = chart_named(&c, &sk, &kid, 20, "Mary Smythe").await;
    submit_link_event(&c, &sk, &kid, a, b, 30, true).await;
    let list = find(&c, "mary").await;
    assert_eq!(list.people.len(), 1, "{:?}", rows(&list));
    let ids = rows(&list).remove(0);
    assert!(ids.contains(&a) && ids.contains(&b) && ids.len() == 2);
}

#[tokio::test]
async fn the_member_the_search_matched_leads_its_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let smith = chart_named(&c, &sk, &kid, 10, "Mary Smith").await;
    let smythe = chart_named(&c, &sk, &kid, 20, "Mary Smythe").await;
    submit_link_event(&c, &sk, &kid, smith, smythe, 30, true).await;
    let by_smythe = find(&c, "smythe").await;
    assert_eq!(rows(&by_smythe), vec![vec![smythe, smith]]);
    assert!(!by_smythe.incomplete);
    let by_smith = find(&c, "smith").await;
    assert_eq!(rows(&by_smith), vec![vec![smith, smythe]]);
    assert!(!by_smith.incomplete);
}

#[tokio::test]
async fn a_linked_chart_the_search_did_not_match_is_shown_and_named() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let lee = chart_named(&c, &sk, &kid, 10, "Ann Lee").await;
    let ngo = chart_named(&c, &sk, &kid, 20, "Bea Ngo").await;
    submit_link_event(&c, &sk, &kid, lee, ngo, 30, true).await;
    let list = find(&c, "lee").await;
    assert_eq!(list.people.len(), 1);
    let members = list.people[0].members();
    assert_eq!(members[1].patient_id, ngo);
    assert_eq!(members[1].display_name, "Bea Ngo");
    // What a registration signs as "displayed": BOTH charts, not only the matched one.
    assert_eq!(list.displayed_charts(), vec![lee, ngo]);
}

/// P = {"Jo Kim Park"} linked to {"Jo"}; Q = {"Jo Kim"}, unlinked; query "jo kim park".
/// Chart ranking is P1 (3 tokens), Q (2), P2 (1). P2 alone would sit BELOW Q, so rows
/// `[P1, P2]`, `[Q]` prove the person is placed at its best member.
#[tokio::test]
async fn two_people_rank_by_their_best_member() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let p1 = chart_named(&c, &sk, &kid, 10, "Jo Kim Park").await;
    let p2 = chart_named(&c, &sk, &kid, 20, "Jo").await;
    let q = chart_named(&c, &sk, &kid, 30, "Jo Kim").await;
    submit_link_event(&c, &sk, &kid, p1, p2, 40, true).await;
    let list = find(&c, "jo kim park").await;
    assert_eq!(rows(&list), vec![vec![p1, p2], vec![q]]);
}

#[tokio::test]
async fn a_member_not_held_here_reads_unknown_and_the_search_stays_complete() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let a = chart_named(&c, &sk, &kid, 10, "Rua Tane").await;

    // `elsewhere` was never registered here: its name event and its link arrive through the
    // REMOTE door, which admits a chart's events before its registration (ADR-0061).
    let elsewhere = Uuid::now_v7();
    let name = "Rua Taane";
    let name_event = body_from_spec(
        Uuid::now_v7(),
        &kid,
        EventSpec {
            patient: elsewhere,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: name_assertion_body(name, Some("legal"), "patient-stated"),
            plaintext_twin: Some(render_name_twin(name, Some("legal"), "patient-stated")),
            wall: 20,
        },
    );
    common::apply_remote_raw(&c, &sk, name_event).await.unwrap();
    let link = |x, y, wall| {
        common::link_assertion_event(
            &kid,
            x,
            y,
            cairn_node::chart_link::LinkVerb::Link,
            wall,
            0,
            "n",
            false,
        )
    };
    common::apply_remote_raw(&c, &sk, link(a, elsewhere, 30))
        .await
        .unwrap();

    let list = find(&c, "rua").await;
    assert_eq!(list.people.len(), 1, "{:?}", rows(&list));
    let far = list.people[0]
        .members()
        .iter()
        .find(|m| m.patient_id == elsewhere)
        .expect("the linked chart is shown");
    assert_eq!(far.display_name, "Rua Taane");
    assert_eq!(far.trust, TrustState::Unknown);
    assert!(!list.incomplete, "{:?}", list.incomplete_reason);

    // A linked chart with NO events at all: no name, not held - still not a partial search.
    let ghost = Uuid::now_v7();
    common::apply_remote_raw(&c, &sk, link(a, ghost, 40))
        .await
        .unwrap();
    let list = find(&c, "rua").await;
    let ghost_member = list.people[0]
        .members()
        .iter()
        .find(|m| m.patient_id == ghost)
        .expect("the nameless linked chart is shown");
    assert_eq!(
        ghost_member.display_name,
        "(registration not yet received here)"
    );
    assert_eq!(ghost_member.trust, TrustState::Unknown);
    assert!(!list.incomplete, "{:?}", list.incomplete_reason);
}

#[tokio::test]
async fn a_never_linked_search_is_one_row_per_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &EXTRA).await;
    let one = chart_named(&c, &sk, &kid, 10, "Teo Alpha").await;
    let two = chart_named(&c, &sk, &kid, 20, "Teo Bravo").await;
    let three = chart_named(&c, &sk, &kid, 30, "Teo Charlie").await;
    let list = find(&c, "teo").await;
    assert_eq!(list.people.len(), 3);
    assert!(list.people.iter().all(|p| !p.is_linked()));
    // Equal strength throughout, so the ranking ends on its id tie-break (UUIDv7: oldest first).
    assert_eq!(rows(&list), vec![vec![one], vec![two], vec![three]]);
}
