//! Allergy slice 1, Task 3 — `allergy::read::list_patient_allergies`. DB-gated on `$CAIRN_TEST_PG`.
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, Onset, ReactionCategory, Source,
};
use cairn_node::allergy::read::list_patient_allergies;
use cairn_node::allergy::sign::sign_allergy;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use uuid::Uuid;

mod common;

fn entry(term: &str) -> AllergyEntry<'_> {
    AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term,
        kind: EntryKind::Allergy,
        category: ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: Onset::Year("2019".into()),
        source: Source::PatientStated,
    }
}

/// Two registered charts a and x, linked, an entry on each; returns what the tests need.
async fn two_linked(base: &str) -> (tokio_postgres::Client, Uuid, Uuid, Uuid, Uuid) {
    let mut c = db::connect_and_load_schema(base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(&c).await;
    let (a, x) = (Uuid::now_v7(), Uuid::now_v7());
    common::submit_registration(&c, &node_sk, &node_kid, a, 0).await;
    common::submit_registration(&c, &node_sk, &node_kid, x, 0).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let ea = sign_allergy(
        &mut c,
        &node_sk,
        &node_kid,
        "n",
        a,
        &entry("penicillin"),
        &author,
    )
    .await
    .unwrap();
    let ex = sign_allergy(
        &mut c,
        &node_sk,
        &node_kid,
        "n",
        x,
        &entry("sulfa"),
        &author,
    )
    .await
    .unwrap();
    common::submit_link_event(&c, &node_sk, &node_kid, a, x, 10, true).await;
    (c, a, x, ea, ex)
}

#[tokio::test]
async fn a_linked_chart_entries_are_read_with_their_chart() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (c, a, x, ea, ex) = two_linked(&base).await;
    let got = list_patient_allergies(&c, a).await.unwrap();
    let mut pairs: Vec<_> = got.entries.iter().map(|r| (r.event_id, r.chart)).collect();
    pairs.sort();
    let mut want = vec![(ea, a), (ex, x)];
    want.sort();
    assert_eq!(pairs, want);
    assert!(got.doubted_charts.is_empty());
    assert!(got
        .entries
        .iter()
        .all(|r| r.signer.is_some() || r.signer.is_none()));
}

#[tokio::test]
async fn a_doubted_link_names_the_other_charts() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (c, a, x, _, _) = two_linked(&base).await;
    let (lo, hi) = (a.min(x), a.max(x));
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
    let got = list_patient_allergies(&c, a).await.unwrap();
    assert_eq!(got.doubted_charts, vec![x]);
    assert_eq!(got.entries.len(), 2, "shown, not withheld");
}

#[tokio::test]
async fn an_unknown_word_from_a_peer_still_reads() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (c, a, _, ea, _) = two_linked(&base).await;
    // Stand-in for a peer's newer word, projected as the remote door would store it.
    c.execute(
        "UPDATE clinical_allergy SET reaction_category = 'bronchospasm' WHERE event_id = $1::text::uuid",
        &[&ea.to_string()],
    )
    .await
    .unwrap();
    let got = list_patient_allergies(&c, a).await.unwrap();
    let row = got.entries.iter().find(|r| r.event_id == ea).unwrap();
    assert_eq!(row.category, "bronchospasm");
}
