//! Allergy slice 1, Task 3 — `allergy::sign`: an entry and a "none known" statement go through the
//! strict door under a human's key (ADR-0053). DB-gated on `$CAIRN_TEST_PG`.
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
};
use cairn_node::allergy::sign::{sign_allergy, sign_allergy_none};
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use uuid::Uuid;

mod common;

#[tokio::test]
async fn signing_an_entry_records_human_authorship() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(&c).await;
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, patient, 0).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let entry = AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term: "penicillin",
        kind: EntryKind::Allergy,
        category: ReactionCategory::Rash,
        reaction_text: Some("itchy"),
        onset: Onset::NotKnown,
        source: Source::PatientStated,
    };
    let id = sign_allergy(&mut c, &node_sk, &node_kid, "n", patient, &entry, &author)
        .await
        .unwrap();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM clinical_allergy WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
    let roles: Vec<(String, String)> = c
        .query(
            "SELECT x->>'actor_id', x->>'role' FROM event_log e, jsonb_array_elements(e.contributors) x \
             WHERE e.event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await.unwrap().iter().map(|r| (r.get(0), r.get(1))).collect();
    assert!(
        roles.contains(&(human_kid.clone(), "authored".into())),
        "{roles:?}"
    );
    assert!(
        roles.contains(&(node_kid.clone(), "recorded".into())),
        "{roles:?}"
    );
}

#[tokio::test]
async fn signing_none_known_records_its_scope() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(&c).await;
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, patient, 0).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let id = sign_allergy_none(
        &mut c,
        &node_sk,
        &node_kid,
        "n",
        patient,
        NoneScope::All,
        &author,
    )
    .await
    .unwrap();
    let scope: String = c
        .query_one(
            "SELECT scope FROM clinical_allergy_none WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(scope, "all");
}
