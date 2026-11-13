//! The settle suite's fixtures (batch charting slice 2b, Task 5): a chart with charted lines and
//! set-aside versions, restore lines in the draft list, and the restore / stop / read helpers.
//! Split out of `medication_version_settle.rs` for the 500-line cap.
//!
//! Include it with `#[macro_use] #[path = "common/version_settle_kit.rs"] mod version_settle_kit;`
//! AFTER `mod common;`.
#![allow(dead_code)] // a helper only some tests use

use crate::common::note_setup;
use cairn_event::medication::{Charting, SourceNote};
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::draft::{
    add_restore_line, chart_draft_line, read_draft_lines, RestoreContent,
};
use cairn_node::medication::draft_versions_sign::{set_aside_draft_line, VersionToSign};
use cairn_node::medication::version_settle::{check_version, RestoreToSign, VersionCheck};
use cairn_node::medication::version_settle_read::version_for_restore;
use cairn_node::medication::{
    cease_medication, AssertMedicationInput, AuthorParams, CeaseMedicationInput,
};
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

pub struct Rig {
    pub c: Client,
    pub nsk: SigningKey,
    pub nkid: String,
    pub hsk: SigningKey,
    pub hkid: String,
    pub chart: Uuid,
}

pub async fn rig(base: &str) -> Rig {
    let c = db::connect_and_load_schema(base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute(
        "TRUNCATE medication_draft_line, medication_set_aside, medication_restore, \
         medication_set_aside_checked",
    )
    .await
    .unwrap();
    Rig {
        c,
        nsk,
        nkid,
        hsk,
        hkid,
        chart,
    }
}

/// Sign `check` on chart `patient` as the note-setup human.
pub async fn sign_check(
    r: &mut Rig,
    patient: Uuid,
    check: &VersionCheck<'_>,
) -> anyhow::Result<cairn_node::db_diagnosis::Committed<Uuid>> {
    let author = AuthorParams {
        human_sk: &r.hsk,
        human_kid: &r.hkid,
    };
    check_version(
        &mut r.c,
        &r.nsk,
        &r.nkid,
        "test-node",
        patient,
        check,
        &author,
    )
    .await
}

pub fn input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "Lipitor",
        coding: None,
        formulation: None,
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: None,
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
        source_note: None,
    }
}

/// The drug a source reported that the clinician did not believe.
pub fn old_drug() -> cairn_event::medication::MedicationAssertion<'static> {
    cairn_event::medication::MedicationAssertion {
        medication_id: "",
        term: "Lipitor",
        coding: None,
        formulation: None,
        dose_amount: Some("25"),
        dose_unit: Some("mg"),
        sig: None,
        info_source: "external-record",
        started: None,
        started_precision: None,
        safety: None,
        charting: None,
        source_note: Some(SourceNote {
            source_detail: Some("GP letter"),
            why: Some("packet says 25 mg"),
        }),
    }
}

pub async fn saved_at(r: &Rig, line: Uuid) -> String {
    read_draft_lines(&r.c, r.chart, Some("notewriter"))
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.line_id == line)
        .unwrap()
        .saved_at
}

/// Save a draft line (optionally marked `set_aside_of`), return its `saved_at`.
pub async fn save(r: &Rig, line: Uuid, set_aside_of: Option<String>) -> String {
    let parts = json!([{"term": "Lipitor", "coding": null}]).to_string();
    let a: String = r
        .c
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', \
                    'started-here', 'Lipitor', $3::text::jsonb, NULL, false, '[]'::jsonb, false, '', '', \
                    false, NULL, NULL, NULL, $4)",
            &[&line.to_string(), &r.chart.to_string(), &parts, &set_aside_of],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(a, "saved");
    saved_at(r, line).await
}

/// Sign draft `line` (read at `at`) with `charting`, optionally as a restore.
pub async fn sign(
    r: &mut Rig,
    line: Uuid,
    at: &str,
    charting: &Charting<'_>,
    restore: Option<&RestoreToSign<'_>>,
) -> anyhow::Result<cairn_node::db_diagnosis::Committed<(Uuid, Vec<Uuid>)>> {
    let author = AuthorParams {
        human_sk: &r.hsk,
        human_kid: &r.hkid,
    };
    chart_draft_line(
        &mut r.c,
        &r.nsk,
        &r.nkid,
        "test-node",
        r.chart,
        "pc-1",
        Some("notewriter"),
        line,
        at,
        &input(),
        charting,
        &author,
        &[],
        restore,
    )
    .await
}

/// Chart a line on its own (with `charting`); returns its thread.
pub async fn charted_as(r: &mut Rig, charting: &Charting<'_>) -> Uuid {
    let line = Uuid::now_v7();
    let at = save(r, line, None).await;
    sign(r, line, &at, charting, None).await.unwrap().value.0
}

pub async fn charted(r: &mut Rig) -> Uuid {
    charted_as(r, &Charting::default()).await
}

/// Set a version aside beside the charted thread `thread`; returns its version_id.
pub async fn version_beside(r: &mut Rig, thread: Uuid) -> Uuid {
    let line = Uuid::now_v7();
    let at = save(r, line, Some(format!("med:{thread}"))).await;
    let v = VersionToSign {
        line,
        saved_at: &at,
        drug: old_drug(),
    };
    let author = AuthorParams {
        human_sk: &r.hsk,
        human_kid: &r.hkid,
    };
    set_aside_draft_line(
        &mut r.c,
        &r.nsk,
        &r.nkid,
        "test-node",
        r.chart,
        "pc-1",
        Some("notewriter"),
        &v,
        thread,
        &author,
    )
    .await
    .expect("the version signs")
    .value
}

/// Add a restore line of `version` to the list; returns (line, saved_at).
pub async fn restore_line(r: &Rig, version: Uuid) -> (Uuid, String) {
    let line = Uuid::now_v7();
    let parts = json!([{"term": "Lipitor", "coding": null}]);
    let content = RestoreContent {
        charted_as: "pre-existing",
        line_text: "Lipitor 25 mg",
        typed_drug: "Lipitor",
        directions: "25 mg",
        parts: &parts,
    };
    let word = add_restore_line(
        &r.c,
        line,
        r.chart,
        "pc-1",
        Some("notewriter"),
        version,
        &content,
    )
    .await
    .unwrap();
    assert_eq!(word, "saved");
    (line, saved_at(r, line).await)
}

/// Restore `version`, stopping `stop` (each with the plan's reason).
pub async fn restore(
    r: &mut Rig,
    version: Uuid,
    stop: &[Uuid],
) -> anyhow::Result<cairn_node::db_diagnosis::Committed<(Uuid, Vec<Uuid>)>> {
    let (line, at) = restore_line(r, version).await;
    let rs = RestoreToSign {
        version_id: version,
        stop: stop
            .iter()
            .map(|s| (*s, "replaced by the GP letter version"))
            .collect(),
    };
    sign(r, line, &at, &Charting::default(), Some(&rs)).await
}

pub async fn cease(r: &mut Rig, thread: Uuid) {
    cease_medication(
        &mut r.c,
        &r.nsk,
        &r.nkid,
        "test-node",
        r.chart,
        thread,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("someone else stopped it"),
        },
        None,
        None,
    )
    .await
    .unwrap();
}

pub async fn count(c: &Client, table: &str) -> i64 {
    c.query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .unwrap()
        .get(0)
}

pub async fn stop_reason(c: &Client, thread: Uuid) -> Option<Option<String>> {
    c.query_opt(
        "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
        &[&thread.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// What currently stands for `companion` on the rig's chart (the version's own read).
pub async fn standing(r: &Rig, version: Uuid) -> Vec<Uuid> {
    version_for_restore(&r.c, r.chart, version)
        .await
        .unwrap()
        .expect("the version is on this chart")
        .standing
}

macro_rules! db_test {
    ($base:ident) => {
        let Some($base) = common::cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _g = db::test_serial_guard(&$base).await.unwrap();
    };
}
