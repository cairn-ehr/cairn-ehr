//! Batch charting slice 2a, Task 5 — the node signs a set-aside version with its companion.
//!
//! A version beside a DRAFT companion commits in the companion's one transaction (events
//! companion first, spends version rows first: ruling R5); beside an already-CHARTED companion it
//! signs alone. `read_set_asides` shows a version only under a companion on its own chart.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_event::medication::{drug_line_twin, Charting, MedicationAssertion, SourceNote};
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::draft::{chart_draft_line, read_draft_lines, SpendRefused};
use cairn_node::medication::draft_versions_sign::{set_aside_draft_line, VersionToSign};
use cairn_node::medication::set_aside_read::read_set_asides;
use cairn_node::medication::CeaseMedicationInput;
use cairn_node::medication::{cease_medication, AssertMedicationInput, AuthorParams};
use common::note_setup;
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

struct Rig {
    c: Client,
    nsk: SigningKey,
    nkid: String,
    hsk: SigningKey,
    hkid: String,
    chart: Uuid,
}

async fn rig(base: &str) -> Rig {
    let c = db::connect_and_load_schema(base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_draft_line, medication_set_aside")
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

fn input() -> AssertMedicationInput<'static> {
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
    }
}

/// The drug a source reported that the clinician did not believe.
fn old_drug() -> MedicationAssertion<'static> {
    MedicationAssertion {
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

/// Save a draft line (optionally marked `set_aside_of`), return its `saved_at`.
async fn save(r: &Rig, line: Uuid, set_aside_of: Option<String>) -> String {
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
    read_draft_lines(&r.c, r.chart, Some("notewriter"))
        .await
        .unwrap()
        .into_iter()
        .find(|l| l.line_id == line)
        .unwrap()
        .saved_at
}

/// Chart `line` (with `versions`) as the note-setup human.
async fn chart(
    r: &mut Rig,
    line: Uuid,
    at: &str,
    versions: &[VersionToSign<'_>],
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
        &Charting::default(),
        &author,
        versions,
    )
    .await
}

async fn count(c: &Client, table: &str) -> i64 {
    c.query_one(&format!("SELECT count(*) FROM {table}"), &[])
        .await
        .unwrap()
        .get(0)
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

#[tokio::test]
async fn a_line_and_its_version_commit_together_and_leave_the_list() {
    db_test!(base);
    let mut r = rig(&base).await;
    let (comp, ver) = (Uuid::now_v7(), Uuid::now_v7());
    let comp_at = save(&r, comp, None).await;
    // The version is marked against a DRAFT companion: spending the companion's row first would
    // make the version's spend answer `changed` (ruling R5), so this fails if the order flips.
    let ver_at = save(&r, ver, Some(format!("line:{comp}"))).await;
    let before = count(&r.c, "event_log").await;
    let v = VersionToSign {
        line: ver,
        saved_at: &ver_at,
        drug: old_drug(),
    };
    let done = chart(&mut r, comp, &comp_at, &[v])
        .await
        .expect("both sign");
    let (thread, versions) = done.value;
    assert_eq!(versions.len(), 1);
    assert_eq!(count(&r.c, "event_log").await, before + 2);
    assert!(read_draft_lines(&r.c, r.chart, Some("notewriter"))
        .await
        .unwrap()
        .is_empty());
    let alt: Uuid = r
        .c
        .query_one("SELECT alternative_to::text FROM medication_set_aside WHERE version_id = $1::text::uuid", &[&versions[0].to_string()])
        .await
        .unwrap()
        .get::<_, String>(0)
        .parse()
        .unwrap();
    assert_eq!(alt, thread);
}

#[tokio::test]
async fn a_version_spend_that_fails_rolls_back_the_companion_too() {
    db_test!(base);
    let mut r = rig(&base).await;
    let (comp, ver) = (Uuid::now_v7(), Uuid::now_v7());
    let comp_at = save(&r, comp, None).await;
    save(&r, ver, Some(format!("line:{comp}"))).await;
    let before = count(&r.c, "event_log").await;
    let v = VersionToSign {
        line: ver,
        saved_at: "2000-01-01 00:00:00+00",
        drug: old_drug(),
    };
    let err = chart(&mut r, comp, &comp_at, &[v]).await.unwrap_err();
    assert!(err.downcast_ref::<SpendRefused>().is_some(), "{err:#}");
    assert_eq!(count(&r.c, "event_log").await, before);
    assert_eq!(count(&r.c, "medication_set_aside").await, 0);
    assert_eq!(
        read_draft_lines(&r.c, r.chart, Some("notewriter"))
            .await
            .unwrap()
            .len(),
        2
    );
}

/// Chart a companion on its own; returns its thread.
async fn charted(r: &mut Rig) -> Uuid {
    let comp = Uuid::now_v7();
    let at = save(r, comp, None).await;
    chart(r, comp, &at, &[]).await.unwrap().value.0
}

#[tokio::test]
async fn a_version_beside_a_charted_line_signs_alone() {
    db_test!(base);
    let mut r = rig(&base).await;
    let thread = charted(&mut r).await;
    let ver = Uuid::now_v7();
    let at = save(&r, ver, Some(format!("med:{thread}"))).await;
    let v = VersionToSign {
        line: ver,
        saved_at: &at,
        drug: old_drug(),
    };
    let author = AuthorParams {
        human_sk: &r.hsk,
        human_kid: &r.hkid,
    };
    let done = set_aside_draft_line(
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
    .expect("the version signs");
    assert_eq!(count(&r.c, "medication_set_aside").await, 1);
    assert!(read_draft_lines(&r.c, r.chart, Some("notewriter"))
        .await
        .unwrap()
        .is_empty());
    let rows = read_set_asides(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].version_id, done.value);
    assert_eq!(rows[0].alternative_to, thread);
    assert_eq!(rows[0].drug_line, "Lipitor 25 mg (external-record)");
    assert_eq!(rows[0].source, "GP letter");
    assert_eq!(rows[0].why.as_deref(), Some("packet says 25 mg"));
}

#[tokio::test]
async fn a_charted_companion_stopped_since_refuses_and_keeps_the_version() {
    db_test!(base);
    let mut r = rig(&base).await;
    let thread = charted(&mut r).await;
    let ver = Uuid::now_v7();
    let at = save(&r, ver, Some(format!("med:{thread}"))).await;
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
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let before = count(&r.c, "event_log").await;
    let v = VersionToSign {
        line: ver,
        saved_at: &at,
        drug: old_drug(),
    };
    let author = AuthorParams {
        human_sk: &r.hsk,
        human_kid: &r.hkid,
    };
    let err = set_aside_draft_line(
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
    .await;
    let text = format!("{:#}", err.expect_err("a stopped companion refuses"));
    assert!(text.contains("is not a current medication"), "{text}");
    assert_eq!(count(&r.c, "event_log").await, before);
    assert_eq!(
        read_draft_lines(&r.c, r.chart, Some("notewriter"))
            .await
            .unwrap()
            .len(),
        1
    );
}

/// Insert a projected version row directly (the remote door's shape), for the read tests.
async fn plant(c: &Client, patient: Uuid, alt: Uuid) {
    c.execute(
        "INSERT INTO medication_set_aside (content_address, event_id, patient_id, version_id, \
            alternative_to, term, info_source, t_recorded) \
         VALUES (gen_random_bytes(8), gen_random_uuid(), $1::text::uuid, gen_random_uuid(), \
                 $2::text::uuid, 'Planted', 'external-record', now())",
        &[&patient.to_string(), &alt.to_string()],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn read_set_asides_joins_only_present_companions() {
    db_test!(base);
    let mut r = rig(&base).await;
    let thread = charted(&mut r).await;
    plant(&r.c, r.chart, thread).await;
    plant(&r.c, r.chart, Uuid::now_v7()).await; // orphan: its companion is absent here
    let rows = read_set_asides(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].alternative_to, thread);
}

#[tokio::test]
async fn read_set_asides_never_shows_another_charts_version() {
    db_test!(base);
    let mut r = rig(&base).await;
    let thread = charted(&mut r).await;
    let other_chart = Uuid::now_v7();
    // Chart B's version pointing at chart A's thread (a remote flag keeps its own patient_id).
    plant(&r.c, other_chart, thread).await;
    assert!(read_set_asides(&r.c, &[r.chart]).await.unwrap().is_empty());
    assert!(read_set_asides(&r.c, &[other_chart])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn chart_draft_line_with_no_versions_is_unchanged() {
    db_test!(base);
    let mut r = rig(&base).await;
    let comp = Uuid::now_v7();
    let at = save(&r, comp, None).await;
    let before = count(&r.c, "event_log").await;
    let done = chart(&mut r, comp, &at, &[]).await.unwrap();
    assert!(done.value.1.is_empty());
    assert_eq!(count(&r.c, "event_log").await, before + 1);
}

/// Plant a morphine 10 mg version with `route`; return its `drug_line` as read back.
async fn route_line(r: &Rig, thread: Uuid, route: &str) -> String {
    r.c.execute(
        "INSERT INTO medication_set_aside (content_address, event_id, patient_id, version_id, \
            alternative_to, term, dose_amount, dose_unit, route, info_source, t_recorded) \
         VALUES (gen_random_bytes(8), gen_random_uuid(), $1::text::uuid, gen_random_uuid(), \
                 $2::text::uuid, 'Morphine', '10', 'mg', $3, 'external-record', now())",
        &[&r.chart.to_string(), &thread.to_string(), &route],
    )
    .await
    .unwrap();
    read_set_asides(&r.c, &[r.chart])
        .await
        .unwrap()
        .into_iter()
        .map(|x| x.drug_line)
        .next_back()
        .unwrap()
}

#[tokio::test]
async fn a_version_differing_only_by_route_reads_differently() {
    db_test!(base);
    let mut r = rig(&base).await;
    let thread = charted(&mut r).await;
    let oral = route_line(&r, thread, "po").await;
    r.c.batch_execute("TRUNCATE medication_set_aside")
        .await
        .unwrap();
    let iv = route_line(&r, thread, "iv").await;
    assert_ne!(oral, iv);
    // One spelling of a drug line: exactly the twin's.
    let want = |route| {
        drug_line_twin(&MedicationAssertion {
            medication_id: "",
            term: "Morphine",
            coding: None,
            formulation: None,
            dose_amount: Some("10"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "external-record",
            started: None,
            started_precision: None,
            safety: None,
            charting: Some(Charting {
                route: Some(route),
                ..Charting::default()
            }),
            source_note: None,
        })
    };
    assert_eq!(oral, want("po"));
    assert_eq!(iv, want("iv"));
}
