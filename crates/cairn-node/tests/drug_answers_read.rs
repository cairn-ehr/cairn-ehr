//! Allergy slice 3, Task 3 — the Rust side of the drug read door (db/076): an allergy row's coding
//! reads back, and `allergy::drug_facts` turns the door's rows into answers.
//!
//! `drug_read_door.rs` pins the SQL door itself; this file pins what the node makes of its rows:
//!   * the facts fold (any `match` row wins a pair; a pair with no row is absent);
//!   * "no drugref" reads as `Unavailable` / `None`, never as an empty answer;
//!   * the window's own role can call every door (ruling T2-b);
//!   * a coded allergy entry reads back its coding.
//!
//! drugref is the stand-in `tests/fixtures/drug_stub.sql`, loaded only inside a transaction that
//! is rolled back. DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, Onset, ReactionCategory, Source,
};
use cairn_event::medication::SubstanceCoding;
use cairn_node::allergy::drug_facts::{
    read_drug_facts, read_group_status, search_drugs, Answer, DrugFacts,
};
use cairn_node::allergy::read::{list_patient_allergies, Coding};
use cairn_node::allergy::sign::sign_allergy;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use tokio_postgres::{Client, GenericClient};
use uuid::Uuid;

mod common;

/// The drugref stand-in (see its header). Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("fixtures/drug_stub.sql");

/// A connection with the schema loaded, and the serial guard held for the test's life.
async fn rig() -> Option<(Client, Client, String)> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c, base))
}

/// The stub's uuid for a moiety or a class, computed the way the stub computes it.
async fn stub_id(c: &impl GenericClient, label: &str) -> Uuid {
    let t: String = c
        .query_one("SELECT md5($1)::uuid::text", &[&label])
        .await
        .unwrap()
        .get(0);
    t.parse().unwrap()
}

fn kinds(a: &Answer) -> Vec<(String, Option<String>)> {
    let Answer::Match(paths) = a else {
        panic!("expected a match, got {a:?}");
    };
    let mut v: Vec<_> = paths
        .iter()
        .map(|p| (p.kind.clone(), p.group_name.clone()))
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn drug_facts_reads_the_stub() {
    let Some((_g, mut c, _)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    let pen = stub_id(&tx, "stub class Penicillins").await;
    let fluclox = stub_id(&tx, "stub moiety flucloxacillin").await;
    let amox = stub_id(&tx, "stub moiety amoxicillin").await;
    let metformin = stub_id(&tx, "stub moiety metformin").await;
    let perindopril = stub_id(&tx, "stub moiety perindopril").await;
    let stranger = Uuid::from_u128(77);

    let DrugFacts::Available(f) = read_drug_facts(
        &tx,
        &[pen, fluclox, amox, metformin, stranger],
        &[amox, perindopril],
    )
    .await
    .unwrap() else {
        panic!("the stub is a readable drugref");
    };
    let group = Some("Penicillins".to_string());
    assert_eq!(
        kinds(&f[&(pen, amox)]),
        [("same-group".to_string(), group.clone())]
    );
    assert_eq!(f[&(pen, perindopril)], Answer::NoMatch);
    assert_eq!(
        kinds(&f[&(fluclox, amox)]),
        [("same-group".to_string(), group.clone())]
    );
    // drugref gives one row per path: amoxicillin/amoxicillin is BOTH same-drug and same-group,
    // and the pair keeps both.
    assert_eq!(
        kinds(&f[&(amox, amox)]),
        [
            ("same-drug".to_string(), None),
            ("same-group".to_string(), group)
        ]
    );
    assert_eq!(f[&(amox, perindopril)], Answer::NoMatch);
    assert_eq!(f[&(metformin, amox)], Answer::Unplaced);
    assert_eq!(f[&(metformin, perindopril)], Answer::Unplaced);
    assert!(
        !f.contains_key(&(stranger, amox)),
        "a pair with no row is absent"
    );
    assert_eq!(f.len(), 8);
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn drug_facts_without_drugref_is_unavailable() {
    let Some((_g, c, _)) = rig().await else {
        return;
    };
    let none: bool = c
        .query_one("SELECT to_regnamespace('drugref') IS NULL", &[])
        .await
        .unwrap()
        .get(0);
    assert!(none, "this test needs a database with no drugref schema");
    let ids = [Uuid::from_u128(1)];
    assert_eq!(
        read_drug_facts(&c, &ids, &ids).await.unwrap(),
        DrugFacts::Unavailable
    );
    assert!(read_group_status(&c, &ids).await.unwrap().is_empty());
    assert_eq!(search_drugs(&c, "amoxicillin").await.unwrap(), None);
}

#[tokio::test]
async fn group_status_reads_the_stub() {
    let Some((_g, mut c, _)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    let pen = stub_id(&tx, "stub class Penicillins").await;
    let amox = stub_id(&tx, "stub moiety amoxicillin").await;
    let got = read_group_status(&tx, &[pen, amox]).await.unwrap();
    assert_eq!(got.len(), 1, "a moiety is no group");
    assert_eq!(got[&pen], "structural-seed");
    let found = search_drugs(&tx, "penicillin").await.unwrap().unwrap();
    assert_eq!(
        found
            .iter()
            .map(|o| (o.system.as_str(), o.display.as_str()))
            .collect::<Vec<_>>(),
        [("drugref-class", "Penicillins")]
    );
    tx.rollback().await.unwrap();
}

/// Ruling T2-b: a login holding only `cairn_window`'s grants can EXECUTE every door.
#[tokio::test]
async fn the_window_role_can_call_the_doors() {
    let Some((_g, mut c, base)) = rig().await else {
        return;
    };
    let login = common::window_login(&c, &base).await;
    let role = login.rsplit("user=").next().unwrap().to_string();
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    let pen = stub_id(&tx, "stub class Penicillins").await;
    let amox = stub_id(&tx, "stub moiety amoxicillin").await;
    tx.batch_execute(&format!("SET LOCAL ROLE {role}"))
        .await
        .unwrap();
    let who: String = tx
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(who, role, "the walk really runs as the window's login");
    assert!(matches!(
        read_drug_facts(&tx, &[pen], &[amox]).await.unwrap(),
        DrugFacts::Available(f) if f.contains_key(&(pen, amox))
    ));
    assert_eq!(read_group_status(&tx, &[pen]).await.unwrap().len(), 1);
    assert!(search_drugs(&tx, "amox").await.unwrap().is_some());
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn a_coded_entry_reads_back_its_coding() {
    let Some((_g, mut c, _)) = rig().await else {
        return;
    };
    let (node_sk, node_kid, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let code = Uuid::from_u128(0xA110).to_string();
    let entry = |term, coding| AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term,
        kind: EntryKind::Allergy,
        category: ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: Onset::NotKnown,
        source: Source::PatientStated,
        corrects: None,
        coding,
    };
    let coded = entry(
        "amoxicillin",
        Some(SubstanceCoding {
            system: "drugref-moiety",
            code: &code,
            display: "amoxicillin",
        }),
    );
    let coded_id = sign_allergy(&mut c, &node_sk, &node_kid, "n", chart, &coded, &author)
        .await
        .unwrap();
    let plain_id = sign_allergy(
        &mut c,
        &node_sk,
        &node_kid,
        "n",
        chart,
        &entry("sulfa", None),
        &author,
    )
    .await
    .unwrap();
    let got = list_patient_allergies(&c, chart).await.unwrap();
    let coding_of = |id| got.entries.iter().find(|r| r.event_id == id).unwrap();
    assert_eq!(
        coding_of(coded_id).coding,
        Some(Coding {
            system: "drugref-moiety".into(),
            code,
            display: "amoxicillin".into()
        })
    );
    assert_eq!(coding_of(plain_id).coding, None);
}
