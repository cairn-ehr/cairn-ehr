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
use cairn_patient_search::{SearchQuery, TrustState};
use common::{cs, setup, submit_patient_created, submit_signed, EventSpec};
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
async fn a_chart_matching_two_passes_returns_one_row_per_pass() {
    // One chart genuinely matching TWO DIFFERENT passes (identifier + dob) legitimately
    // returns TWO rows — matched_pass differs ('identifier' vs 'dob'), so the tuples are
    // not literal duplicates and UNION correctly keeps both. That is the intended shape:
    // it tells a caller building the attestation WHY the chart matched, on each axis it
    // matched on. This test does NOT exercise the within-pass dedup mechanism at all (the
    // two passes' tuples can never collide on matched_pass, so no combination of
    // DISTINCT/UNION-vs-UNION-ALL choices could make this specific test fail) — see
    // `two_identifiers_matching_the_same_patient_are_deduped_to_one_row` and
    // `two_name_rows_matching_the_same_patient_are_deduped_to_one_row` below for tests that
    // exercise a genuine within-pass duplicate.
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
async fn two_identifiers_matching_the_same_patient_are_deduped_to_one_row() {
    // `patient_identifier` is a set-union projection keyed (patient_id, system, match_key),
    // so a patient with two ON-FILE identifiers (MRN *and* Medicare number, both typed by
    // the clerk) legitimately produces TWO rows out of pass 1's
    // `patient_identifier pi JOIN jsonb_array_elements(p_identifiers) q` — one per matching
    // (system, match_key) pair, both labelled 'identifier'.
    //
    // VERIFIED NON-VACUOUS BY HAND (see the fix report for the full derivation): plain SQL
    // `UNION` (as opposed to `UNION ALL`) already re-distincts the ENTIRE final combined
    // result, so removing JUST this pass's own `SELECT DISTINCT` does NOT make this test
    // fail — the outermost `UNION` in the three-branch chain independently catches the
    // same duplicate. The two protections are genuinely redundant with each other; this
    // test only fails once BOTH are gone at once (this pass's `SELECT DISTINCT` AND the
    // final `UNION` becoming `UNION ALL`), which is the real single point of total failure.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    let mrn = IdentifierAssertion {
        value: "12345",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    let medicare = IdentifierAssertion {
        value: "67890",
        system: "Medicare",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &mrn)
        .await
        .expect("MRN accepted");
    submit_identifier(&c, &sk, &kid, p, 2, &medicare)
        .await
        .expect("Medicare accepted");

    // Both identifiers typed by the clerk, so BOTH join rows match this one chart.
    let identifiers = serde_json::json!([
        {"system": "MRN", "value": "12345"},
        {"system": "Medicare", "value": "67890"},
    ])
    .to_string();
    let rows = search_candidates(&c, None, None, Some(&identifiers)).await;
    assert_eq!(
        rows,
        vec![(p, "identifier".to_string())],
        "two matching identifiers on ONE chart must collapse to ONE row, not two: {rows:?}"
    );
}

#[tokio::test]
async fn two_name_rows_matching_the_same_patient_are_deduped_to_one_row() {
    // `patient_name` is a RETAINED SET — one row per distinct (patient, use, value) name —
    // so a patient with a legal name AND an alias that both carry the SAME shared token
    // legitimately produces TWO matching rows under pass 3's
    // `CROSS JOIN LATERAL regexp_split_to_table(...)`, both labelled 'name'.
    //
    // VERIFIED NON-VACUOUS BY HAND (see the fix report): same finding as the identifier
    // test above — the outermost `UNION` in the three-branch chain already re-distincts
    // the whole final result, so this pass's own `SELECT DISTINCT` is independently
    // redundant with it. This test only fails once BOTH this pass's `SELECT DISTINCT` AND
    // the final `UNION` are gone at once.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    // Legal name and an alias, distinct (use, value) retained-set members, both sharing
    // the "smith" token.
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
    .expect("legal name accepted");
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        name_assertion_body("Robert Smith", Some("alias"), "patient-stated"),
        render_name_twin("Robert Smith", Some("alias"), "patient-stated"),
    )
    .await
    .expect("alias accepted");

    let rows = search_candidates(&c, Some(&["smith"]), None, None).await;
    assert_eq!(
        rows,
        vec![(p, "name".to_string())],
        "two retained names sharing one token on ONE chart must collapse to ONE row: {rows:?}"
    );
}

#[tokio::test]
async fn a_stored_name_with_surrounding_whitespace_is_not_matched_by_an_empty_query_token() {
    // The §4.2/§4.4 structural floor only requires a non-BLANK (trimmed) name — it never
    // reformats the authored value — so a stored name with leading/trailing whitespace is
    // legitimately admitted, and `regexp_split_to_table` on that raw value emits an EMPTY
    // string as one of its tokens (confirmed directly against Postgres: splitting
    // `' smith'` on `\s+` yields `('', 'smith')`). Without the `tok <> ''` guard, a stray
    // empty element in `p_name_tokens` (e.g. a caller's own naive split producing a
    // leading/trailing blank) would equal that empty projected token and surface a chart
    // with no typed evidence behind the match at all.
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
        name_assertion_body(" Smith", Some("legal"), "patient-stated"),
        render_name_twin(" Smith", Some("legal"), "patient-stated"),
    )
    .await
    .expect("whitespace-padded name accepted by the floor (non-blank after trim)");

    let rows = search_candidates(&c, Some(&[""]), None, None).await;
    assert!(
        rows.is_empty(),
        "an empty query token must never match an empty projected token: {rows:?}"
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

// ---------------------------------------------------------------------------------------
// `search_patients` — Task 5's orchestrator, the ONE mapping from the projections above
// into the shared `Candidate` model. The nine tests above cover `cairn_search_candidates`
// (the SQL layer) directly; these four cover the Rust assembly on top of it: display-field
// reads, the trust surface, and the two never-negotiable behaviours (never drop a
// candidate; never touch the database on an empty query).
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_candidate_carries_name_age_trust_and_last_activity() {
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
    .expect("name accepted");
    let dob = "1980-06-15";
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
    .expect("dob accepted");
    // Only patient.created/patient.amended/note.added give a chart a `patient_chart` row
    // (`patient_chart_apply`'s registered types) — so last_activity needs one of those,
    // not the demographic assertions above, which land in their own overlay tables only.
    submit_patient_created(&c, &sk, &kid, p, 3).await;

    let query = SearchQuery::new("smith", None, &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "exactly one chart on file: {list:?}"
    );
    let cand = &list.candidates[0];
    assert_eq!(cand.patient_id, p);
    assert_eq!(cand.display_name, "John Smith");
    assert_eq!(
        cand.age.as_ref().map(|a| a.years),
        Some(46),
        "1980-06-15 -> 2026-06-16 is 46 (birthday already passed by a day): {cand:?}"
    );
    assert_eq!(
        cand.age.as_ref().map(|a| a.basis.as_str()),
        Some("document-verified"),
        "the age's basis is the dob assertion's own provenance, not a constant: {cand:?}"
    );
    assert_eq!(cand.trust, TrustState::Confirmed);
    assert!(
        cand.last_activity.is_some(),
        "patient.created gave this chart a patient_chart row: {cand:?}"
    );
    assert!(!list.incomplete, "every field read cleanly: {list:?}");
}

#[tokio::test]
async fn an_identity_pending_chart_comes_back_marked_unconfirmed() {
    // NOT merely "is returned" — the trust state must be visible, or a clerk cannot tell
    // the John Doe from a confirmed chart and the §5.4 identification path breaks.
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

    // The callsign IS the chart's only name (patient_name_current has nothing else to
    // pick), so searching by it is both the realistic clerk gesture and the display-name
    // read path at once. Built as a single already-final token, NOT through
    // `SearchQuery::new` — the callsign is dash-joined (`sanitize_part`) and `new`'s
    // alphanumeric-only splitter would fragment it on every dash, where db/046's pass 3
    // only splits the STORED value on whitespace (`\s+`) and so treats the whole,
    // hyphens-and-all callsign as ONE token. `search_candidates` in this same file (the
    // `db/046` SQL-layer test) hits the identical mismatch and sidesteps it the same way,
    // by supplying the finished token directly rather than through a raw-text splitter.
    let token = call.to_lowercase();
    let query = SearchQuery {
        name_tokens: vec![token],
        birth_date: None,
        identifiers: vec![],
    };
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-07-03")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the john doe chart must be found by its callsign: {list:?}"
    );
    let cand = &list.candidates[0];
    assert_eq!(cand.patient_id, pid);
    assert_eq!(
        cand.trust,
        TrustState::Unconfirmed,
        "an identity-pending chart must read as unconfirmed, never as confirmed: {cand:?}"
    );
    // A registration event alone creates no `patient_chart` row (patient_chart_apply is
    // registered only for patient.created/patient.amended/note.added) — this is an honest
    // absence, not a read failure, and must not flip `incomplete`.
    assert_eq!(cand.last_activity, None);
    assert!(!list.incomplete);
}

#[tokio::test]
async fn a_chart_with_no_readable_name_is_reported_incomplete_never_dropped() {
    // ADR-0060 decision 2 at the read layer: a candidate the node cannot render must
    // surface as `incomplete` with a reason, never vanish. A silently-dropped candidate is
    // the exact duplicate-creating failure the funnel exists to prevent.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // A chart matched by identifier ALONE: no name assertion was ever authored for it, so
    // patient_name_current holds no row for this patient_id at all — the display-name read
    // genuinely cannot succeed, rather than merely being untested.
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "999-no-name",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("identifier accepted");

    let query = SearchQuery::new("", None, &[("MRN".to_string(), "999-no-name".to_string())]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the unreadable candidate must still be present, never dropped: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, p);
    assert!(
        list.incomplete,
        "an unreadable display field must mark the list incomplete: {list:?}"
    );
    assert!(
        list.incomplete_reason
            .as_ref()
            .is_some_and(|r| r.contains('1')),
        "the reason must name how many candidates were affected: {:?}",
        list.incomplete_reason
    );
}

#[tokio::test]
async fn an_empty_query_yields_an_empty_complete_list() {
    // Empty AND complete: "found nothing" is a true, exhaustive answer, not a partial one.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Real charts on file, so a search that failed to short-circuit would visibly return
    // rows instead of passing by accident on an empty database.
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

    let query = SearchQuery::new("", None, &[]);
    assert!(query.is_empty(), "precondition: this query is empty");
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert!(
        list.candidates.is_empty(),
        "an empty query must yield an empty list, not the whole population: {list:?}"
    );
    assert!(
        !list.incomplete,
        "found-nothing is a complete answer, not a partial one: {list:?}"
    );
    assert!(list.incomplete_reason.is_none());
}
