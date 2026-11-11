//! Batch charting slice 1, Task 4's store rulings (db/078), beside Task 1's
//! `medication_draft_list_store.rs` (already over 500 lines, so these live apart).
//!
//! 1. The list stores the drug as typed and the directions (`typed_drug`, `directions`): Sign
//!    rebuilds the line in Rust from the stored row, never from the webview, and the split of a
//!    typed line into the two is done only in the window. A change to either clears a hand tick,
//!    as a change to the text does.
//! 2. A hand tick keeps the warnings seen WHEN IT WAS GIVEN. The window autosaves every second
//!    and sends the refs of the moment each time; were the door to take them on every save, a
//!    warning that arrived after the tick would be silently absorbed into it (design §3.2: the
//!    prescriber's responsibility covers what they saw). So the door takes `p_ticked_against`
//!    only on the false→true transition, keeps the stored refs while the tick stands, and clears
//!    them on an untick.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_node::db;
use cairn_node::medication::draft::read_draft_lines;
use common::{note_setup, owner_and_window};
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

/// One allergy ref as the window builds it (a fresh uuid each call: test data, not a constant).
fn allergy_ref(r: Uuid) -> serde_json::Value {
    json!({"source": "allergy", "ref": r.to_string()})
}

/// Save `line` for the notewriter on pc-1: its typed drug, directions, tick and refs.
async fn save(
    c: &Client,
    line: Uuid,
    chart: Uuid,
    (typed, directions): (&str, &str),
    ticked: bool,
    against: &serde_json::Value,
) -> String {
    let text = format!("{typed} {directions}");
    let parts = json!([{"term": typed, "coding": null}]).to_string();
    c.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                'started-here', $3, $4::text::jsonb, NULL, $5, $6::text::jsonb, false, $7, $8)",
        &[
            &line.to_string(),
            &chart.to_string(),
            &text,
            &parts,
            &ticked,
            &against.to_string(),
            &typed,
            &directions,
        ],
    )
    .await
    .expect("the save door answers")
    .get(0)
}

/// The line's stored tick and the refs it was given against.
async fn refs(w: &Client, chart: Uuid) -> (bool, Vec<(String, Uuid)>) {
    let r = read_draft_lines(w, chart, Some("notewriter"))
        .await
        .unwrap();
    (r[0].hand_ticked, r[0].ticked_against.clone())
}

async fn fixture(base: &str) -> (Client, Client, Uuid) {
    let (owner, window) = owner_and_window(base).await;
    let (_, _, _, _, chart) = note_setup(&owner).await;
    owner
        .batch_execute("TRUNCATE medication_draft_line, medication_draft_line_deleted")
        .await
        .unwrap();
    (owner, window, chart)
}

#[tokio::test]
async fn the_typed_drug_and_directions_are_stored_and_a_change_clears_the_tick() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    let none = json!([]);
    assert_eq!(
        save(&w, line, chart, ("amox", "500 mg tds"), true, &none).await,
        "saved"
    );
    let rows = read_draft_lines(&w, chart, Some("notewriter"))
        .await
        .unwrap();
    assert_eq!(
        (rows[0].typed_drug.as_str(), rows[0].directions.as_str()),
        ("amox", "500 mg tds")
    );
    assert!(rows[0].hand_ticked);
    // Same text, same parts — only the directions differ: the tick covered the old directions.
    w.query_one(
        "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                'started-here', 'amox 500 mg tds', $3::text::jsonb, NULL, true, '[]'::jsonb, false, \
                'amox', '250 mg tds')",
        &[
            &line.to_string(),
            &chart.to_string(),
            &json!([{"term": "amox", "coding": null}]).to_string(),
        ],
    )
    .await
    .unwrap();
    let rows = read_draft_lines(&w, chart, Some("notewriter"))
        .await
        .unwrap();
    assert_eq!(rows[0].directions, "250 mg tds");
    assert!(!rows[0].hand_ticked, "a directions change voids the tick");
}

#[tokio::test]
async fn an_autosave_keeps_the_refs_seen_when_ticked() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    let drug = ("warfarin", "5 mg mane");
    let (seen, later) = (Uuid::now_v7(), Uuid::now_v7());
    // Typed, unticked: no refs are kept whatever the window sends.
    save(&w, line, chart, drug, false, &json!([allergy_ref(seen)])).await;
    assert_eq!(refs(&w, chart).await, (false, vec![]));
    // Ticked: the refs of THIS moment are taken.
    save(&w, line, chart, drug, true, &json!([allergy_ref(seen)])).await;
    let at_tick = vec![("allergy".to_string(), seen)];
    assert_eq!(refs(&w, chart).await, (true, at_tick.clone()));
    // An autosave while still ticked sends the refs of ITS moment, a new warning among them:
    // the door keeps what was seen at the tick.
    let now = json!([allergy_ref(seen), allergy_ref(later)]);
    save(&w, line, chart, drug, true, &now).await;
    assert_eq!(refs(&w, chart).await, (true, at_tick));
    // Unticked: cleared.
    save(&w, line, chart, drug, false, &now).await;
    assert_eq!(refs(&w, chart).await, (false, vec![]));
}

/// Save the line with the clinician's veto `held` (Task 6 review C1): the door's word.
async fn save_held(c: &Client, line: Uuid, chart: Uuid, text: &str, ticked: bool, held: bool) {
    let answer: String = c
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                    'pre-existing', $3, '[]'::jsonb, NULL, $4, '[]'::jsonb, true, $3, '', $5)",
            &[&line.to_string(), &chart.to_string(), &text, &ticked, &held],
        )
        .await
        .expect("the save door answers")
        .get(0);
    assert_eq!(answer, "saved");
}

/// The line's stored veto.
async fn held(w: &Client, chart: Uuid) -> bool {
    read_draft_lines(w, chart, Some("notewriter"))
        .await
        .unwrap()[0]
        .held
}

/// Task 6 review C1: unticking a line the DEFAULT tick would give is the clinician's veto, and it
/// is stored (`held`): a chart closed, a re-lock or a restart must never bring the line back
/// ticked. The veto covers the words it was given against: a change to them clears it, as it
/// clears a hand tick; a hand tick clears it too (the two never stand together).
#[tokio::test]
async fn a_held_line_stays_held_until_its_words_change_or_it_is_ticked() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, w, chart) = fixture(&base).await;
    let line = Uuid::now_v7();
    save_held(&w, line, chart, "metformin 500mg bd", false, true).await;
    assert!(held(&w, chart).await, "a new line can be held");
    save_held(&w, line, chart, "metformin 500mg bd", false, true).await;
    assert!(held(&w, chart).await, "an autosave keeps it");
    save_held(&w, line, chart, "metformin 1g bd", false, true).await;
    assert!(
        !held(&w, chart).await,
        "changed words: the veto covered the old ones"
    );
    save_held(&w, line, chart, "metformin 1g bd", false, true).await;
    assert!(held(&w, chart).await);
    save_held(&w, line, chart, "metformin 1g bd", true, true).await;
    assert!(
        !held(&w, chart).await,
        "a hand tick and a veto never stand together"
    );
}
