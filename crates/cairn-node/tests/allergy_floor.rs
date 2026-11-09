//! Allergy slice 1, Task 2 — the two allergy streams end to end below the window: an entry
//! (`clinical.allergy.asserted`) and a "none known" statement (`clinical.allergy-none.asserted`),
//! each built by `cairn_event::allergy`, sealed, signed by a HUMAN author (ADR-0053), admitted by
//! the strict door's floor (db/074), and projected in the clear into `clinical_allergy` /
//! `clinical_allergy_none`; a shred then scrubs each projection row (db/037).
//!
//! `db/tests/074_clinical_allergy_test.sql` pins the floor rule by rule; this file proves the real
//! pipeline produces bodies that floor admits, and that the projections hold what was signed.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::allergy::{
    allergy_none_payload, allergy_payload, render_allergy_none_twin, render_allergy_twin,
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
    ALLERGY_EVENT_TYPE, ALLERGY_NONE_EVENT_TYPE, ALLERGY_NONE_SCHEMA_VERSION,
    ALLERGY_SCHEMA_VERSION,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use serde_json::Value;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

/// Assemble a clear allergy-stream body, device-shaped (`seal_sign_submit` rewrites it to human
/// authorship). The note_body skeleton (`note/sign.rs`): no `t_effective` (the reaction's time
/// is `onset`, inside the body) and no safety signal (design §4).
fn allergy_body(
    event_id: Uuid,
    patient: Uuid,
    node_kid: &str,
    hlc: cairn_event::Hlc,
    (event_type, schema_version): (&str, &str),
    payload: Value,
    twin: String,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// The node's signing key and key id, as `common::medication_setup` enrolled them.
struct Node {
    sk: SigningKey,
    kid: String,
}

/// A registered chart with one human-signed entry and one human-signed "none known drug
/// allergies" on it. Returns the node (to sign a later shred) and the two event ids.
async fn chart_with_both(c: &mut Client, term: &str) -> (Node, Uuid, Uuid) {
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(c).await;
    let patient = Uuid::now_v7();
    // #345: a chart's first event is its registration.
    common::submit_registration(c, &node_sk, &node_kid, patient, 0).await;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };

    let entry = AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term,
        kind: EntryKind::Allergy,
        category: ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: Onset::Year("2019".into()),
        source: Source::PatientStated,
        corrects: None,
        coding: None,
    };
    let none = NoneScope::Drug;
    let bodies = [
        (
            (ALLERGY_EVENT_TYPE, ALLERGY_SCHEMA_VERSION),
            allergy_payload(&entry),
            render_allergy_twin(&entry),
        ),
        (
            (ALLERGY_NONE_EVENT_TYPE, ALLERGY_NONE_SCHEMA_VERSION),
            allergy_none_payload(none),
            render_allergy_none_twin(none),
        ),
    ];
    let mut ids = Vec::new();
    for (types, payload, twin) in bodies {
        let hlc = db::next_hlc(c, &node_kid).await.unwrap();
        let id = Uuid::now_v7();
        let body = allergy_body(id, patient, &node_kid, hlc, types, payload, twin);
        let recorded = seal_sign_submit(c, &node_sk, body, Some(&author), None)
            .await
            .unwrap_or_else(|e| panic!("the strict door admits a {} body: {e:#}", types.0));
        assert_eq!(recorded, id);
        ids.push(id);
    }
    let node = Node {
        sk: node_sk,
        kid: node_kid,
    };
    (node, ids[0], ids[1])
}

/// Rows in projection `table` for one event.
async fn rows(c: &Client, table: &str, id: Uuid) -> i64 {
    c.query_one(
        &format!("SELECT count(*) FROM {table} WHERE event_id = $1::text::uuid"),
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_sealed_entry_submits_and_projects() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, id, _) = chart_with_both(&mut c, "amoxicillin").await;

    let r = c
        .query(
            "SELECT term, allergen_kind, kind, reaction_category, reaction_text IS NULL, \
                    onset_precision, onset_value, source, coding IS NULL, refs::text \
               FROM clinical_allergy WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(r.len(), 1, "one projected row per entry");
    let r = &r[0];
    assert_eq!(r.get::<_, String>(0), "amoxicillin", "the signed term");
    assert_eq!(r.get::<_, String>(1), "drug");
    assert_eq!(r.get::<_, String>(2), "allergy");
    assert_eq!(r.get::<_, String>(3), "anaphylaxis");
    assert!(r.get::<_, bool>(4), "no reaction text");
    assert_eq!(r.get::<_, String>(5), "year");
    assert_eq!(r.get::<_, String>(6), "2019");
    assert_eq!(r.get::<_, String>(7), "patient-stated");
    assert!(r.get::<_, bool>(8), "uncoded reads as SQL NULL");
    assert_eq!(r.get::<_, String>(9), "[]");

    let sealed: bool = c
        .query_one(
            "SELECT body::text NOT LIKE '%amoxicillin%' FROM event_log WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(sealed, "the entry is born sealed (ADR-0052)");
}

#[tokio::test]
async fn a_none_statement_submits_and_projects() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, id) = chart_with_both(&mut c, "penicillin").await;

    let scope: Vec<String> = c
        .query(
            "SELECT scope FROM clinical_allergy_none WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(scope, ["drug"], "one projected row, its scope as signed");
}

#[tokio::test]
async fn a_shred_scrubs_both_projections() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node, entry, none) = chart_with_both(&mut c, "codeine").await;
    assert_eq!(rows(&c, "clinical_allergy", entry).await, 1);
    assert_eq!(rows(&c, "clinical_allergy_none", none).await, 1);

    for id in [entry, none] {
        shred_event(
            &mut c,
            &node.sk,
            &node.kid,
            &node.kid,
            id,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");
    }
    assert_eq!(
        rows(&c, "clinical_allergy", entry).await,
        0,
        "the entry's term is clinical plaintext: the shred scrubs its row"
    );
    assert_eq!(
        rows(&c, "clinical_allergy_none", none).await,
        0,
        "the none statement is clinical plaintext too"
    );
}
