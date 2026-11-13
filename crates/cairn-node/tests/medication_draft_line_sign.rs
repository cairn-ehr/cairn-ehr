//! Batch charting slice 1, Task 3: sign ONE draft line. Its event enters the record and its draft
//! row leaves the list in the same transaction; a line the list no longer holds for this
//! terminal rolls back unwritten. `chart_medication` itself is pinned unchanged by running
//! `medication_chart.rs` unedited.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_event::medication::Charting;
use cairn_node::db;
use cairn_node::medication::draft::{chart_draft_line, read_draft_lines};
use cairn_node::medication::{AssertMedicationInput, AuthorParams};
use common::note_setup;
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

fn input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "Augmentin Duo Forte",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
        source_note: None,
    }
}

/// Save one line on `terminal` under the `notewriter`, ticked against `against` (a json array).
async fn save(c: &Client, line: Uuid, chart: Uuid, terminal: &str, against: &str) {
    let parts = json!([{"term": "Augmentin Duo Forte", "coding": null}]).to_string();
    let a: String = c
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, $3, 'notewriter', \
                    'started-here', 'Augmentin Duo Forte', $4::text::jsonb, NULL, true, \
                    $5::text::jsonb, false, '', '')",
            &[
                &line.to_string(),
                &chart.to_string(),
                &terminal,
                &parts,
                &against,
            ],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(a, "saved");
}

async fn events(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0)
}

/// Sign `line` on `terminal` as the note-setup human, device node origin "test-node".
async fn sign(
    c: &mut Client,
    keys: &(
        cairn_event::SigningKey,
        String,
        cairn_event::SigningKey,
        String,
    ),
    chart: Uuid,
    terminal: &str,
    line: Uuid,
    saved_at: &str,
) -> anyhow::Result<cairn_node::db_diagnosis::Committed<Uuid>> {
    let author = AuthorParams {
        human_sk: &keys.2,
        human_kid: &keys.3,
    };
    chart_draft_line(
        c,
        &keys.0,
        &keys.1,
        "test-node",
        chart,
        terminal,
        Some("notewriter"),
        line,
        saved_at,
        &input(),
        &Charting::default(),
        &author,
        &[],
        None,
    )
    .await
    .map(|c| cairn_node::db_diagnosis::Committed {
        value: c.value.0,
        event_id: c.event_id,
    })
}

#[tokio::test]
async fn a_signed_line_is_in_the_record_and_gone_from_the_list() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_draft_line")
        .await
        .unwrap();
    let line = Uuid::now_v7();
    save(&c, line, chart, "pc-1", "[]").await;
    let before = events(&c).await;
    let at = read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap()[0]
        .saved_at
        .clone();
    let done = sign(
        &mut c,
        &(nsk, nkid, hsk, hkid.clone()),
        chart,
        "pc-1",
        line,
        &at,
    )
    .await
    .expect("the line signs");
    assert_eq!(events(&c).await, before + 1);
    let signer: String = c
        .query_one(
            "SELECT signer_key_id FROM event_log WHERE event_id = $1::text::uuid",
            &[&done.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(signer, hkid, "the clinician signs");
    assert!(read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_line_taken_by_another_terminal_rolls_back_unwritten() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_draft_line")
        .await
        .unwrap();
    let line = Uuid::now_v7();
    save(&c, line, chart, "pc-1", "[]").await;
    let before = events(&c).await;
    let at = read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap()[0]
        .saved_at
        .clone();
    let err = sign(&mut c, &(nsk, nkid, hsk, hkid), chart, "pc-2", line, &at)
        .await
        .expect_err("another terminal's line does not sign");
    assert!(format!("{err:#}").contains("answered"), "{err:#}");
    assert_eq!(events(&c).await, before, "no event was written");
    assert_eq!(
        read_draft_lines(&c, chart, Some("notewriter"))
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn read_draft_lines_parses_ticked_against() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, _, _, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_draft_line")
        .await
        .unwrap();
    let (line, r) = (Uuid::now_v7(), Uuid::now_v7());
    save(
        &c,
        line,
        chart,
        "pc-1",
        &json!([{"source": "allergy", "ref": r.to_string()}]).to_string(),
    )
    .await;
    let got = read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].line_id, line);
    assert_eq!(got[0].ticked_against, vec![("allergy".to_string(), r)]);
    assert!(got[0].hand_ticked);
}

#[tokio::test]
async fn a_save_between_read_and_sign_answers_changed_and_writes_nothing() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_draft_line")
        .await
        .unwrap();
    let line = Uuid::now_v7();
    save(&c, line, chart, "pc-1", "[]").await;
    // Sign's fresh read ...
    let at = read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap()[0]
        .saved_at
        .clone();
    // ... then an autosave lands with the clinician's last edit.
    let parts = json!([{"term": "Amoxil", "coding": null}]).to_string();
    let a: String = c
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', 'started-here', 'Amoxil', $3::text::jsonb, NULL, false, '[]'::jsonb, false, '', '')",
            &[&line.to_string(), &chart.to_string(), &parts],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(a, "saved");
    let before = events(&c).await;
    let err = sign(&mut c, &(nsk, nkid, hsk, hkid), chart, "pc-1", line, &at)
        .await
        .expect_err("a changed line does not sign");
    assert!(format!("{err:#}").contains("changed"), "{err:#}");
    assert_eq!(events(&c).await, before, "no event was written");
    let rows = read_draft_lines(&c, chart, Some("notewriter"))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "the row is intact");
    assert_eq!(
        rows[0].line_text, "Amoxil",
        "with the clinician's last edit"
    );
}
