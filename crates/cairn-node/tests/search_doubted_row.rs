//! R6 (#723): the search marks a person row whose links db/054 doubts, and the mark never
//! reaches what a registration signs. DB-gated on $CAIRN_TEST_PG; keys minted at runtime.
mod common;
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::SigningKey;
use cairn_node::chart_link::LinkVerb;
use cairn_node::{db, patient::search::search_patients};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use common::{
    apply_remote_raw, chart_named, cs, link_assertion_event, medication_setup as setup,
    submit_signed, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// A document-verified DOB on `p` (two different ones on two charts trip the hard veto).
async fn verified_dob(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, value: &str, wall: i64) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
            plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
            wall,
        },
    )
    .await
    .expect("dob accepted");
}

async fn find(c: &Client, typed: &str) -> CandidateList {
    search_patients(c, &SearchQuery::new(typed, None, &[]), "2026-10-14")
        .await
        .unwrap()
}

#[tokio::test]
async fn a_set_with_a_doubted_link_is_one_doubted_row_and_signs_the_same_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    // A doubted pair: two DOUBTWELL charts with clashing verified DOBs, machine-linked.
    let a = chart_named(&c, &sk, &kid, 1, "Ann DOUBTWELL").await;
    let b = chart_named(&c, &sk, &kid, 3, "Ann DOUBTWELL").await;
    verified_dob(&c, &sk, &kid, a, "1980-07-15", 5).await;
    verified_dob(&c, &sk, &kid, b, "1975-01-02", 6).await;
    apply_remote_raw(
        &c,
        &sk,
        link_assertion_event(&kid, a, b, LinkVerb::Link, 50, 0, "m", false),
    )
    .await
    .unwrap();
    // A plain linked pair, no clash: the control.
    let p = chart_named(&c, &sk, &kid, 7, "Bo QUIETLINK").await;
    let q = chart_named(&c, &sk, &kid, 9, "Bo QUIETLINK").await;
    apply_remote_raw(
        &c,
        &sk,
        link_assertion_event(&kid, p, q, LinkVerb::Link, 51, 0, "m", false),
    )
    .await
    .unwrap();

    // The charts of earlier runs persist (only the link tables are truncated), so find THIS
    // run's row by its members rather than assuming it is the only one.
    let doubted = find(&c, "DOUBTWELL").await;
    let row = doubted
        .people
        .iter()
        .find(|r| r.members().iter().any(|m| m.patient_id == a))
        .expect("the doubted pair is shown");
    assert!(row.is_doubted());
    let mut in_row: Vec<Uuid> = row.members().iter().map(|m| m.patient_id).collect();
    in_row.sort();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(in_row, both, "one row holds both charts");
    let shown = doubted.displayed_charts();
    assert!(
        shown.contains(&a) && shown.contains(&b),
        "the signed list still names every member"
    );

    let quiet = find(&c, "QUIETLINK").await;
    let quiet_row = quiet
        .people
        .iter()
        .find(|r| r.members().iter().any(|m| m.patient_id == p))
        .expect("the quiet pair is shown");
    assert!(
        !quiet_row.is_doubted(),
        "a linked pair with no doubt is not marked"
    );

    // The mark cannot reach what a registration signs: the attestation built from this list is
    // BYTE-IDENTICAL to the one built from the same list with every mark taken off. (Comparing
    // `displayed` with `displayed_charts()` would be a tautology — that is how it is built.)
    let query = SearchQuery::new("DOUBTWELL", None, &[]);
    let mut unmarked = doubted.clone();
    unmarked.people = unmarked
        .people
        .into_iter()
        .map(|r| r.with_doubt(false))
        .collect();
    assert!(
        !unmarked.people.iter().any(|r| r.is_doubted()),
        "precondition: the unmarked list carries no mark, so the comparison is not vacuous"
    );
    let signed = serde_json::to_vec(&SearchAttestation::from_displayed(&query, &doubted)).unwrap();
    let plain = serde_json::to_vec(&SearchAttestation::from_displayed(&query, &unmarked)).unwrap();
    assert_eq!(signed, plain, "the doubt mark never enters the attestation");
}
