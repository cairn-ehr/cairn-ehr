//! Allergy slice 1, Task 3 — `allergy::read::list_patient_allergies`. DB-gated on `$CAIRN_TEST_PG`.
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
};
use cairn_node::allergy::read::list_patient_allergies;
use cairn_node::allergy::sign::{sign_allergy, sign_allergy_none};
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
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
        corrects: None,
    }
}

/// Two registered charts a and x, linked, an entry on each; returns what the tests need.
async fn two_linked(base: &str) -> (tokio_postgres::Client, Uuid, Uuid, Uuid, Uuid) {
    let mut c = db::connect_and_load_schema(base).await.unwrap();
    // note_setup also enrols the human as a named vault member, so the signer name is populated.
    let (node_sk, node_kid, human_sk, human_kid, a) = common::note_setup(&c).await;
    let x = Uuid::now_v7();
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
        .all(|r| r.signer.as_deref() == Some(common::NOTE_AUTHOR)));
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

fn near_now(t: chrono::DateTime<chrono::Utc>) -> bool {
    (chrono::Utc::now() - t).num_seconds().abs() < 300
}

#[tokio::test]
async fn rows_carry_chart_scope_and_a_recorded_time_near_now() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, a) = common::note_setup(&c).await;
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
    let en = sign_allergy_none(&mut c, &node_sk, &node_kid, "n", a, NoneScope::All, &author)
        .await
        .unwrap();
    let got = list_patient_allergies(&c, a).await.unwrap();
    let n = &got.none[0];
    assert_eq!((n.event_id, n.chart, n.scope.as_str()), (en, a, "all"));
    assert!(near_now(n.recorded_at), "{}", n.recorded_at);
    assert_eq!(n.signer.as_deref(), Some(common::NOTE_AUTHOR));
    let e = &got.entries[0];
    assert_eq!(e.event_id, ea);
    assert!(near_now(e.recorded_at), "{}", e.recorded_at);
}

/// An allergy event this node holds but has no projected row for — no custody of its sealed
/// body, or (as here) shredded — is COUNTED, never silently absent: without the count the strip
/// would say "not yet asked", or a readable NKDA, over a recorded allergy. Readable rows are not
/// counted, and the "none known" stream is counted too.
#[tokio::test]
async fn an_allergy_event_this_node_cannot_read_is_counted() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, a) = common::note_setup(&c).await;
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
    let en = sign_allergy_none(
        &mut c,
        &node_sk,
        &node_kid,
        "n",
        a,
        NoneScope::Drug,
        &author,
    )
    .await
    .unwrap();
    let kept = sign_allergy_none(&mut c, &node_sk, &node_kid, "n", a, NoneScope::All, &author)
        .await
        .unwrap();
    assert_eq!(list_patient_allergies(&c, a).await.unwrap().unreadable, 0);

    for id in [ea, en] {
        shred_event(
            &mut c,
            &node_sk,
            &node_kid,
            &node_kid,
            id,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");
    }
    let got = list_patient_allergies(&c, a).await.unwrap();
    assert_eq!(got.unreadable, 2, "the shredded entry and statement");
    assert!(got.entries.is_empty());
    let readable: Vec<Uuid> = got.none.iter().map(|n| n.event_id).collect();
    assert_eq!(readable, vec![kept]);
}

/// Final review: the unreadable count reaches every LINKED chart, not just the one opened — a
/// linked chart's unreadable allergy must not vanish under this chart's readable strip.
#[tokio::test]
async fn an_unreadable_event_on_a_linked_chart_is_counted() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid, a) = common::note_setup(&c).await;
    let x = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, x, 0).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
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
    assert_eq!(list_patient_allergies(&c, a).await.unwrap().unreadable, 0);
    shred_event(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        ex,
        "retention ceiling",
        None,
    )
    .await
    .expect("the shred is admitted");
    let got = list_patient_allergies(&c, a).await.unwrap();
    assert_eq!(
        got.unreadable, 1,
        "the linked chart's shredded entry, read from a"
    );
    assert!(got.entries.is_empty());
}

/// Final review: a NULL in a nullable projection column fails the read as an error (the window
/// says "Could not read the allergy list"), never a panic in the command.
#[tokio::test]
async fn a_null_projected_column_is_an_error_not_a_panic() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (c, a, _, ea, _) = two_linked(&base).await;
    c.execute(
        "UPDATE clinical_allergy SET term = NULL WHERE event_id = $1::text::uuid",
        &[&ea.to_string()],
    )
    .await
    .unwrap();
    assert!(list_patient_allergies(&c, a).await.is_err());
}
