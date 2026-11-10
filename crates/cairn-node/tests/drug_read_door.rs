//! Allergy slice 3, Tasks 2 and 2R — the drug read door (db/076) and the opened coding slot (db/074
//! step 3, db/041, db/042), driven over tokio-postgres the way the node drives them.
//!
//! The door is the ONE place Cairn reads drugref, an optional schema a deployment may install into
//! Cairn's own database. It keeps no drug logic of its own: the check is drugref's
//! `allergy_check`, passed through pair by pair (M4). `db/tests/076_drug_read_door_test.sql` pins
//! its rules one at a time; this file is the Rust mirror, one test per section of that file:
//!   0. no drugref → no answers (and only the window may ask), and a replay converges;
//!   1. the pick's search matches literally, finds nothing from fewer than three characters, and
//!      offers allergy groups, never MED-RT classes;
//!   2. the facts are drugref's own rows for every pair, and a group's review status reads back;
//!   3. a drugref the door's owner cannot read, or one without `allergy_check`, is
//!      "unavailable", never a fault;
//!   4. the floor: a coded drug entry signs and projects its coding; nothing else may be coded.
//!
//! drugref is simulated by `tests/fixtures/drug_stub.sql`, loaded only inside a transaction this
//! file rolls back, so no other test on the shared database ever sees a `drugref` schema.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::allergy::{
    allergy_payload, render_allergy_twin, AllergenKind, AllergyEntry, EntryKind, Onset,
    ReactionCategory, Source, ALLERGY_EVENT_TYPE, ALLERGY_SCHEMA_VERSION,
};
use cairn_event::medication::SubstanceCoding;
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use tokio_postgres::{Client, GenericClient};
use uuid::Uuid;

mod common;

/// The drugref stand-in (see its header). Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("fixtures/drug_stub.sql");
/// The stub's allergy group, named as the real drugref names it (#1057).
const PEN: &str = "Penicillins (allergy group)";

/// The four doors, by their full signatures.
const DOORS: [&str; 4] = [
    "cairn_drug_available()",
    "cairn_drug_search(text, integer)",
    "cairn_drug_facts(uuid[], uuid[])",
    "cairn_drug_group_status(uuid[])",
];

/// A connection with the schema loaded, and the serial guard held for the test's life.
async fn rig() -> Option<(Client, Client)> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c))
}

/// A connection inside a transaction with the stub loaded; the caller rolls it back.
macro_rules! with_stub {
    ($c:ident, $tx:ident) => {
        let Some((_g, mut $c)) = rig().await else {
            return;
        };
        let $tx = $c.transaction().await.unwrap();
        $tx.batch_execute(STUB).await.unwrap();
    };
}

/// The door's options for `text`, as "system:display" in the door's order.
async fn options(c: &impl GenericClient, text: Option<&str>, limit: i32) -> Vec<String> {
    c.query(
        "SELECT system || ':' || display FROM cairn_drug_search($1, $2)",
        &[&text, &limit],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// A stub row's uuid, by its name (a drug or a group), as text.
async fn id(c: &impl GenericClient, name: &str) -> String {
    c.query_one(
        "SELECT moiety_uuid::text FROM drugref.substance_moiety WHERE display_name = $1 \
         UNION ALL SELECT class_uuid::text FROM drugref.substance_class WHERE class_name = $1",
        &[&name],
    )
    .await
    .unwrap()
    .get(0)
}

/// The uuids of these stub names, as text (bound as `$n::text[]::uuid[]`).
async fn ids(c: &impl GenericClient, names: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for n in names {
        out.push(id(c, n).await);
    }
    out
}

/// A stub uuid's name, as an SQL expression over the column `u`.
fn name_of(u: &str) -> String {
    format!(
        "(SELECT display_name FROM drugref.substance_moiety WHERE moiety_uuid = {u} \
         UNION ALL SELECT class_name FROM drugref.substance_class WHERE class_uuid = {u})"
    )
}

/// The door's facts for these allergens and medications, as
/// "allergen / med / outcome[ / match_kind[ / group]]" by name, sorted byte-wise.
async fn facts(c: &impl GenericClient, allergens: &[&str], meds: &[&str]) -> Vec<String> {
    let sql = format!(
        "SELECT concat_ws(' / ', {}, {}, f.outcome, f.match_kind, f.group_name) \
         FROM cairn_drug_facts($1::text[]::uuid[], $2::text[]::uuid[]) f",
        name_of("f.allergen"),
        name_of("f.med"),
    );
    let (a, m) = (ids(c, allergens).await, ids(c, meds).await);
    let rows = c.query(&sql, &[&a, &m]).await.unwrap();
    let mut lines: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    lines.sort();
    lines
}

/// The door's review status for these groups, as "group: status" by name, sorted.
async fn group_status(c: &impl GenericClient, groups: &[&str]) -> Vec<String> {
    let sql = format!(
        "SELECT {} || ': ' || s.review_status \
         FROM cairn_drug_group_status($1::text[]::uuid[]) s ORDER BY 1",
        name_of("s.group_code"),
    );
    let g = ids(c, groups).await;
    c.query(&sql, &[&g])
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect()
}

/// Whether the door says a drugref is available.
async fn available(c: &impl GenericClient) -> bool {
    c.query_one("SELECT cairn_drug_available()", &[])
        .await
        .unwrap()
        .get(0)
}

/// Every door answers nothing — false, no options, no facts, no status — and raises nothing.
async fn assert_answers_nothing(c: &impl GenericClient, why: &str) {
    assert!(!available(c).await, "{why}: unavailable");
    assert!(
        options(c, Some("amox"), 20).await.is_empty(),
        "{why}: no options"
    );
    assert!(
        facts(c, &[PEN], &["amoxicillin"]).await.is_empty(),
        "{why}: no facts"
    );
    assert!(group_status(c, &[PEN]).await.is_empty(), "{why}: no status");
}

#[tokio::test]
async fn no_drugref_means_no_answers_and_only_the_window_asks() {
    let Some((_g, c)) = rig().await else { return };
    // Every connect replays all of db/; a second load on the same database must converge on the
    // delegated facts shape (db/076 drops the Task 2 shape only when it is the one present).
    let base = common::cs().unwrap();
    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    let shape: String = c2
        .query_one(
            "SELECT pg_get_function_result('cairn_drug_facts(uuid[], uuid[])'::regprocedure)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        shape,
        "TABLE(allergen uuid, med uuid, outcome text, match_kind text, group_name text)"
    );
    let schema: bool = c
        .query_one("SELECT to_regnamespace('drugref') IS NULL", &[])
        .await
        .unwrap()
        .get(0);
    assert!(schema, "this test needs a database with no drugref schema");
    assert!(!available(&c).await, "no drugref: unavailable");
    assert!(options(&c, Some("amoxicillin"), 20).await.is_empty());
    let n: i64 = c
        .query_one(
            "SELECT (SELECT count(*) FROM cairn_drug_facts(ARRAY[gen_random_uuid()], \
             ARRAY[gen_random_uuid()])) + (SELECT count(*) FROM \
             cairn_drug_group_status(ARRAY[gen_random_uuid()]))",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "no drugref: no facts, no group status");
    for f in DOORS {
        let (public, window): (bool, bool) = c
            .query_one(
                "SELECT has_function_privilege('public', $1, 'EXECUTE'), \
                        has_function_privilege('cairn_window', $1, 'EXECUTE')",
                &[&f],
            )
            .await
            .map(|r| (r.get(0), r.get(1)))
            .unwrap();
        assert!(!public, "PUBLIC must not execute {f}");
        assert!(window, "the window must execute {f}");
    }
}

#[tokio::test]
async fn search_text_is_literal_and_short_text_finds_nothing() {
    with_stub!(c, tx);
    assert!(available(&tx).await, "the stub is a readable drugref");
    assert_eq!(
        options(&tx, Some("amox"), 20).await,
        ["drugref-moiety:amoxicillin"]
    );
    assert_eq!(
        options(&tx, Some("penicillin"), 20).await,
        ["drugref-class:Penicillins (allergy group)"],
        "the allergy group, never the EPC class (M5)"
    );
    assert!(
        options(&tx, Some("antibacterial"), 20).await.is_empty(),
        "a MED-RT class is never offered (M5)"
    );
    for short_or_literal in [
        "am", " am ", "", "am%x", "%%%", "amo_", "___", "amo\\", "pen_",
    ] {
        assert!(
            options(&tx, Some(short_or_literal), 20).await.is_empty(),
            "{short_or_literal:?} must find nothing"
        );
    }
    assert!(options(&tx, None, 20).await.is_empty(), "no text: nothing");
    tx.batch_execute(
        "INSERT INTO drugref.substance_moiety (moiety_uuid, display_name) \
         SELECT md5('stub limit ' || i)::uuid, 'zzlimit ' || i FROM generate_series(1, 25) i",
    )
    .await
    .unwrap();
    assert_eq!(
        options(&tx, Some("zzlimit"), 100).await.len(),
        20,
        "capped at 20 (a negative limit is pinned in the SQL mirror)"
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn the_facts_are_drugrefs_rows_for_every_pair() {
    with_stub!(c, tx);
    assert_eq!(
        facts(
            &tx,
            &[PEN, "flucloxacillin", "amoxicillin", "metformin"],
            &["amoxicillin", "perindopril"],
        )
        .await,
        [
            "Penicillins (allergy group) / amoxicillin / match / same-group / Penicillins (allergy group)",
            "Penicillins (allergy group) / perindopril / no-match",
            "amoxicillin / amoxicillin / match / same-drug",
            "amoxicillin / amoxicillin / match / same-group / Penicillins (allergy group)",
            "amoxicillin / perindopril / no-match",
            "flucloxacillin / amoxicillin / match / same-group / Penicillins (allergy group)",
            "flucloxacillin / perindopril / no-match",
            "metformin / amoxicillin / unplaced",
            "metformin / perindopril / unplaced",
        ],
        "every row allergy_check gives for the 4 × 2 pairs, and nothing else (M4)"
    );
    assert_eq!(
        group_status(&tx, &[PEN, "Penicillin-class Antibacterial [EPC]"]).await,
        ["Penicillins (allergy group): structural-seed"],
        "only an allergy group has a status (M7)"
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn an_unreadable_drugref_is_unavailable_not_a_fault() {
    with_stub!(c, tx);
    // The test database's function owner is a superuser, for whom every privilege check is true
    // (ruling P1). So the four doors are handed to a throwaway non-superuser role, inside this
    // rolled-back transaction, which may read all of the stub — and then not one part of it. No
    // shared role changes. EXECUTE is revoked from PUBLIC so the probe's own grant decides.
    tx.batch_execute(
        "DO $$
         DECLARE r text := 'cairn_drug_probe_' || pg_backend_pid(); f text;
         BEGIN
             EXECUTE format('CREATE ROLE %I NOLOGIN', r);
             FOREACH f IN ARRAY ARRAY['cairn_drug_available()', 'cairn_drug_search(text, integer)',
                                      'cairn_drug_facts(uuid[], uuid[])',
                                      'cairn_drug_group_status(uuid[])'] LOOP
                 EXECUTE format('ALTER FUNCTION %s OWNER TO %I', f, r);
             END LOOP;
             EXECUTE format('GRANT USAGE ON SCHEMA drugref TO %I', r);
             EXECUTE format('GRANT SELECT ON ALL TABLES IN SCHEMA drugref TO %I', r);
             REVOKE EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) FROM PUBLIC;
             EXECUTE format('GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I', r);
         END $$",
    )
    .await
    .unwrap();
    assert!(
        available(&tx).await,
        "control: the probe owner reads it all"
    );
    assert_eq!(
        facts(&tx, &[PEN], &["amoxicillin"]).await.len(),
        1,
        "control"
    );
    // Each grant in turn: revoke it, every door answers nothing, put it back.
    for (what, revoke, grant) in [
        (
            "allergy_check not executable",
            "REVOKE EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) FROM %I",
            "GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I",
        ),
        (
            "allergy_group unreadable",
            "REVOKE SELECT ON drugref.allergy_group FROM %I",
            "GRANT SELECT ON drugref.allergy_group TO %I",
        ),
    ] {
        let probe = |stmt: &str| {
            format!("DO $$ BEGIN EXECUTE format('{stmt}', 'cairn_drug_probe_' || pg_backend_pid()); END $$")
        };
        tx.batch_execute(&probe(revoke)).await.unwrap();
        assert_answers_nothing(&tx, what).await;
        tx.batch_execute(&probe(grant)).await.unwrap();
    }
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn a_drugref_without_allergy_check_is_unavailable() {
    with_stub!(c, tx);
    // A drugref from before allergy groups: its tables are there, its check is not.
    tx.batch_execute("DROP FUNCTION drugref.allergy_check(uuid, uuid)")
        .await
        .unwrap();
    assert_answers_nothing(&tx, "no allergy_check").await;
    tx.rollback().await.unwrap();
}

/// The database's refusal message, or the client error's text.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A registered chart and the keys that sign on it.
struct Chart {
    node_sk: SigningKey,
    node_kid: String,
    human_sk: SigningKey,
    human_kid: String,
    patient: Uuid,
}

/// Sign one coded entry on `chart` through the real seal → sign → submit path (human authorship,
/// ADR-0053), returning its event id or the floor's refusal.
async fn sign_coded(
    c: &mut Client,
    chart: &Chart,
    kind: AllergenKind,
    coding: SubstanceCoding<'_>,
) -> anyhow::Result<Uuid> {
    let entry = AllergyEntry {
        allergen_kind: kind,
        term: "penicillins",
        kind: EntryKind::Allergy,
        category: ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: Onset::NotKnown,
        source: Source::PatientStated,
        corrects: None,
        coding: Some(coding),
    };
    let id = Uuid::now_v7();
    let body = EventBody {
        event_id: id.to_string(),
        patient_id: chart.patient.to_string(),
        event_type: ALLERGY_EVENT_TYPE.into(),
        schema_version: ALLERGY_SCHEMA_VERSION.into(),
        hlc: db::next_hlc(c, &chart.node_kid).await.unwrap(),
        t_effective: None,
        signer_key_id: chart.node_kid.clone(),
        contributors: serde_json::json!([{"actor_id": chart.node_kid, "role": "recorded"}]),
        payload: allergy_payload(&entry),
        attachments: vec![],
        plaintext_twin: Some(render_allergy_twin(&entry)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let author = AuthorParams {
        human_sk: &chart.human_sk,
        human_kid: &chart.human_kid,
    };
    seal_sign_submit(c, &chart.node_sk, body, Some(&author), None).await?;
    Ok(id)
}

#[tokio::test]
async fn the_floor_opens_the_coding_slot_for_drug_entries_only() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(&c).await;
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, patient, 0).await;
    let chart = Chart {
        node_sk,
        node_kid,
        human_sk,
        human_kid,
        patient,
    };
    let code = Uuid::now_v7().to_string();
    let display = "Penicillin-class Antibacterial [EPC]";
    let coding = |system| SubstanceCoding {
        system,
        code: &code,
        display,
    };

    for system in ["drugref-moiety", "drugref-class"] {
        let id = sign_coded(&mut c, &chart, AllergenKind::Drug, coding(system))
            .await
            .unwrap_or_else(|e| panic!("a drug entry coded {system} is admitted: {e:#}"));
        let projected: String = c
            .query_one(
                "SELECT coding::text FROM clinical_allergy WHERE event_id = $1::text::uuid",
                &[&id.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        let projected: serde_json::Value = serde_json::from_str(&projected).unwrap();
        assert_eq!(
            projected,
            serde_json::json!({"system": system, "code": code, "display": display}),
            "the projection holds the signed coding"
        );
    }
    for (kind, system, why) in [
        (
            AllergenKind::Other,
            "drugref-moiety",
            "substance.coding is for a drug entry only",
        ),
        (
            AllergenKind::Drug,
            "drugref-product",
            "must be drugref-moiety or drugref-class, got drugref-product",
        ),
    ] {
        let e = sign_coded(&mut c, &chart, kind, coding(system))
            .await
            .expect_err("the authoring door refuses this coding");
        assert!(format!("{e:#}").contains(why), "wanted {why:?}: {e:#}");
    }

    // A medication is one drug, never a group (D3) — the assertion's coding check, called directly.
    let payload = serde_json::json!({"substance": {"term": "amoxicillin", "coding": {
        "system": "drugref-class", "code": code, "display": display}}});
    let e = c
        .execute(
            "SELECT cairn_check_medication_coding($1::text::jsonb)",
            &[&payload.to_string()],
        )
        .await
        .expect_err("a medication coded as a group is refused");
    assert!(
        db_msg(&e).contains("names a drug group (drugref-class), never one drug"),
        "{}",
        db_msg(&e)
    );
}
