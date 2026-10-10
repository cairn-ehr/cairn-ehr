//! R5b Task 4: the worklist builds the SAME candidate the search does — `candidates_by_id` and
//! the search share one display read (`read_display_facts`) and one rendering
//! (`DisplayFacts::candidate`). DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`.
mod common;
use cairn_node::db;
use cairn_node::patient::candidate_read::candidates_by_id;
use cairn_node::patient::search::search_patients;
use cairn_patient_search::SearchQuery;
use common::{chart_named, cs, setup};
use uuid::Uuid;

#[tokio::test]
async fn candidates_by_id_equal_what_the_search_returns() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_link", "person_member"]).await;
    let a = chart_named(&c, &sk, &kid, 10, "Wilhelmina Quarrington").await;
    let b = chart_named(&c, &sk, &kid, 20, "Wilhelmina Quarringdon").await;
    let today = "2026-10-08";
    let list = search_patients(&c, &SearchQuery::new("Wilhelmina", None, &[]), today)
        .await
        .unwrap();
    let from_search: Vec<_> = list.charts().cloned().collect();
    let ids: Vec<Uuid> = from_search.iter().map(|c| c.patient_id).collect();
    assert!(
        ids.contains(&a) && ids.contains(&b),
        "setup: the search finds both"
    );
    let by_id = candidates_by_id(&c, &ids, today).await.unwrap();
    assert_eq!(by_id, from_search, "one read, one candidate");
    // A chart this node has never heard of is still a candidate (never dropped).
    let ghost = Uuid::now_v7();
    let one = candidates_by_id(&c, &[ghost], today).await.unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].patient_id, ghost);
}
