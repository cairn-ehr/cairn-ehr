//! Integration coverage for `db/046`'s `cairn_search_candidates`: the §5.8 ADVISORY
//! three-pass candidate search (shared identifier / exact DOB / shared name token) a
//! clerk's search-before-create funnel calls to find charts already on file. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` (the shared-DB + TRUNCATE pattern every suite in this
//! directory uses). This function is advisory (ADR-0014): it never blocks, never vetoes,
//! and a miss produces only a false SPLIT — §5.2's explicitly safe direction, backstopped
//! by the hub-tier duplicate sweep. Matching/veto scoring is a separate subsystem and is
//! NOT exercised here.
mod common;

use cairn_event::demographics::{
    dob_assertion_body, identifier_assertion_body, name_assertion_body, render_dob_twin,
    render_identifier_twin, render_name_twin, IdentifierAssertion,
};
use cairn_event::SigningKey;
use cairn_node::{db, john_doe};
use common::{cs, setup, submit_signed, EventSpec};
use std::collections::BTreeSet;
use tokio_postgres::Client;
use uuid::Uuid;

/// The projections this suite reads, beyond `common::setup`'s default clinical core
/// (`patient_identifier`, `patient_demographic` are already truncated there).
/// `patient_name` holds the retained name set pass 3 tokenises; `chart_identity_state`
/// is the John-Doe-registration overlay the callsign test composes onto (mirrors
/// `john_doe.rs`'s `OVERLAY_TABLES`).
const EXTRA_TABLES: [&str; 2] = ["patient_name", "chart_identity_state"];

/// Submit one §4.4 identifier assertion for `patient`, built by the typed builder.
async fn submit_identifier(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    a: &IdentifierAssertion<'_>,
) -> Result<u64, tokio_postgres::Error> {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.identifier.asserted",
            schema_version: "demographic.identifier/1",
            payload: identifier_assertion_body(a),
            plaintext_twin: Some(render_identifier_twin(a)),
            wall,
        },
    )
    .await
}

/// Submit one §4.2 `demographic.field.asserted` event (dob or name) — the payload and
/// twin are already rendered by the caller, since dob and name twins render differently.
async fn submit_field(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    payload: serde_json::Value,
    twin: String,
) -> Result<u64, tokio_postgres::Error> {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload,
            plaintext_twin: Some(twin),
            wall,
        },
    )
    .await
}

/// Call `cairn_search_candidates` and read back `(patient_id, matched_pass)` for every
/// row. Every argument is `Option` so a test can drive the true-SQL-NULL path for an
/// omitted term (rather than an empty-but-present value), exercising the same
/// `COALESCE(..., <empty>)` branches the function itself is built on.
///
/// UUID / jsonb BINDING: `cairn-node` does not enable tokio-postgres's `with-uuid-1` or
/// `with-serde_json-1` features (project-wide convention — see `common/mod.rs` and
/// `authorship_binding.rs`), so the patient id is read back as `::text` and the
/// identifiers argument is bound as a `text` literal and cast with `$3::text::jsonb`
/// (never a bare `$3::jsonb`, which silently no-ops on an untyped parameter and would
/// false-green this helper).
async fn search_candidates(
    c: &Client,
    name_tokens: Option<&[&str]>,
    birth_date: Option<&str>,
    identifiers_json: Option<&str>,
) -> Vec<(Uuid, String)> {
    let tokens: Option<Vec<String>> =
        name_tokens.map(|ts| ts.iter().map(|t| t.to_string()).collect());
    let rows = c
        .query(
            "SELECT patient_id::text, matched_pass \
             FROM cairn_search_candidates($1, $2, $3::text::jsonb)",
            &[&tokens, &birth_date, &identifiers_json],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|r| {
            let id: String = r.get(0);
            let pass: String = r.get(1);
            (
                Uuid::parse_str(&id).expect("patient_id is a valid uuid"),
                pass,
            )
        })
        .collect()
}

#[tokio::test]
async fn the_identifier_pass_finds_a_chart_by_system_and_value() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "12345",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("identifier assertion accepted");

    let identifiers = serde_json::json!([{"system": "MRN", "value": "12345"}]).to_string();
    let rows = search_candidates(&c, None, None, Some(&identifiers)).await;
    assert_eq!(
        rows,
        vec![(p, "identifier".to_string())],
        "the identifier pass must find the chart by (system, match_key)"
    );
}

#[tokio::test]
async fn the_dob_pass_finds_a_chart_by_exact_birth_date() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    let dob = "1980-01-01";
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        dob_assertion_body(dob, "day", None, "document-verified"),
        render_dob_twin(dob, "day", "document-verified"),
    )
    .await
    .expect("dob assertion accepted");

    let rows = search_candidates(&c, None, Some(dob), None).await;
    assert_eq!(
        rows,
        vec![(p, "dob".to_string())],
        "the dob pass must find the chart by an exact string match on the projected value"
    );
}

#[tokio::test]
async fn the_name_token_pass_finds_a_chart_by_one_shared_token() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        name_assertion_body("John Smith", Some("legal"), "patient-stated"),
        render_name_twin("John Smith", Some("legal"), "patient-stated"),
    )
    .await
    .expect("name assertion accepted");

    // Only "smith" typed — one shared token in any position, no name-order model.
    let rows = search_candidates(&c, Some(&["smith"]), None, None).await;
    assert_eq!(
        rows,
        vec![(p, "name".to_string())],
        "the name pass must find the chart by ONE shared token"
    );
}

#[tokio::test]
async fn a_chart_matching_two_passes_is_returned_once() {
    // Union + dedup. A duplicate row would double-count a candidate in the attestation.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // One chart, matched via BOTH the identifier pass and the dob pass.
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "999",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("identifier assertion accepted");
    let dob = "1975-05-05";
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        dob_assertion_body(dob, "day", None, "document-verified"),
        render_dob_twin(dob, "day", "document-verified"),
    )
    .await
    .expect("dob assertion accepted");

    let identifiers = serde_json::json!([{"system": "MRN", "value": "999"}]).to_string();
    let rows = search_candidates(&c, None, Some(dob), Some(&identifiers)).await;

    // The two passes legitimately label their evidence differently — matched_pass is
    // 'identifier' on one row and 'dob' on the other, so the raw UNION correctly keeps
    // both (they are not literal duplicate rows). What must NOT happen is the same
    // candidate appearing as more than one DISTINCT patient_id — that would double-count
    // one chart as if it were two separate candidates in the registration attestation.
    let distinct_patients: BTreeSet<Uuid> = rows.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        distinct_patients.len(),
        1,
        "one chart matching two passes must still be ONE candidate: {rows:?}"
    );
    assert_eq!(
        rows.len(),
        2,
        "each pass that genuinely matched contributes its own labelled row, no more: {rows:?}"
    );
}

#[tokio::test]
async fn a_john_doe_callsign_chart_is_returned_by_its_callsign_token() {
    // Load-bearing: the chart a clerk needs when the family arrives with a name.
    // Contrast with matcher/pipeline/db.py, which EXCLUDES callsigns from its feature
    // space — correct there (a callsign is not evidence of identity) and wrong here
    // (a clerk searching "Unknown-ED" must find the chart in front of them).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let (pid, call, _ord) = john_doe::register_john_doe(
        &mut c,
        &sk,
        &kid,
        "n",
        "ED",
        "site1",
        "2026-07-03",
        "unconscious ED arrival, no ID",
    )
    .await
    .expect("john doe registration accepted by the floor");

    // The callsign contains no whitespace (dash-joined by `sanitize_part`), so it is a
    // single token on both sides of the comparison: the retained name's own tokenisation
    // (pass 3's `regexp_split_to_table`) and the clerk-typed query term alike.
    let token = call.to_lowercase();
    let rows = search_candidates(&c, Some(&[token.as_str()]), None, None).await;
    assert_eq!(
        rows,
        vec![(pid, "name".to_string())],
        "the callsign is a real name row, unlike in the matcher's excluded feature space"
    );
}

#[tokio::test]
async fn an_empty_query_returns_no_rows_rather_than_every_chart() {
    // The failure that would matter: a no-term query degenerating into a full scan that
    // "displays" the entire patient population into an attestation.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Several real charts on file, so an accidental full scan would be obviously wrong
    // rather than passing by accident on an empty database.
    for (i, wall) in (0u8..3).zip(1i64..) {
        let p = Uuid::now_v7();
        let name = format!("Patient {i}");
        submit_field(
            &c,
            &sk,
            &kid,
            p,
            wall,
            name_assertion_body(&name, Some("legal"), "patient-stated"),
            render_name_twin(&name, Some("legal"), "patient-stated"),
        )
        .await
        .expect("name assertion accepted");
    }

    // Every argument true-SQL-NULL: no name tokens, no birth date, no identifiers.
    let rows = search_candidates(&c, None, None, None).await;
    assert!(
        rows.is_empty(),
        "an empty query must return no rows, not the whole population: {rows:?}"
    );
}
