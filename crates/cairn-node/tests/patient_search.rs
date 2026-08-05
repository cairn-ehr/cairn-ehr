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

use cairn_event::attachment::{Attachment, Rendition, RENDITION_ROLE_ORIGINAL};
use cairn_event::demographics::{
    dob_assertion_body, identifier_assertion_body, name_assertion_body, render_dob_twin,
    render_identifier_twin, render_name_twin, IdentifierAssertion,
};
use cairn_event::identity::{
    dispute_assertion_body, render_dispute_twin, render_repudiate_twin, repudiation_assertion_body,
    DisputeAssertion, RepudiationAssertion,
};
use cairn_event::identity_evidence::{
    photo_evidence_body, render_identity_evidence_twin, IDENTITY_EVIDENCE_EVENT_TYPE,
    IDENTITY_EVIDENCE_SCHEMA_VERSION, PHOTO_EVIDENCE_KIND,
};
use cairn_event::{ClockGrade, EventBody, Hlc, SigningKey};
use cairn_node::{db, john_doe};
use cairn_patient_search::{SearchQuery, TrustState};
use common::{
    cs, enroll_human, setup, submit_attested, submit_patient_created, submit_signed, EventSpec,
};
use std::collections::BTreeSet;
use tokio_postgres::Client;
use uuid::Uuid;

/// The projections this suite reads, beyond `common::setup`'s default clinical core
/// (`patient_identifier`, `patient_demographic` are already truncated there).
/// `patient_name` holds the retained name set pass 3 tokenises; `chart_identity_state`
/// is the John-Doe-registration overlay the callsign test composes onto (mirrors
/// `john_doe.rs`'s `OVERLAY_TABLES`); `chart_dispute` backs the `chart_trust` `under-review`
/// row `a_candidate_carries_name_age_trust_and_last_activity` asserts against;
/// `name_repudiation` backs `a_repudiated_only_name_reads_as_withheld_not_incomplete`.
const EXTRA_TABLES: [&str; 4] = [
    "patient_name",
    "chart_identity_state",
    "chart_dispute",
    "name_repudiation",
];

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

    // The callsign contains no whitespace (dash-joined by `sanitize_part`), so pass 3's
    // OWN tokenisation (`regexp_split_to_table` splits only on `\s+`) treats it as ONE
    // stored token. This test bypasses `SearchQuery::new` entirely and hands db/046 that
    // single finished token directly, to exercise the SQL layer's tokenisation in
    // isolation from the query layer above it. Whether `SearchQuery::new` itself produces
    // that same whole token from raw clerk-typed text is a SEPARATE claim, covered by
    // `an_identity_pending_chart_comes_back_marked_unconfirmed` below, which drives the
    // real entry point end to end — do not read this test as proof of that claim too.
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
    // An OPEN dispute, so `cand.trust` below is asserted against a REAL `chart_trust` row
    // rather than the `map_or` default `read_trust_states` falls back to when a candidate
    // has no row at all — that default reads Confirmed whether or not the trust query ran
    // at all, so asserting it alone would pass even with `read_trust_states` deleted.
    let dispute_id = Uuid::now_v7();
    let dispute = DisputeAssertion {
        dispute_id: &dispute_id.to_string(),
        subject: &p.to_string(),
        reason: "wrong-chart flagged by triage",
    };
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "identity.dispute.asserted",
            schema_version: "identity.dispute.asserted/1",
            payload: dispute_assertion_body(&dispute),
            plaintext_twin: Some(render_dispute_twin(&dispute)),
            wall: 4,
        },
    )
    .await
    .expect("dispute accepted");

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
    assert_eq!(
        cand.trust,
        TrustState::UnderReview,
        "a real chart_trust row must actually be read, not merely defaulted: {cand:?}"
    );
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
    // read path at once. Driven through the REAL entry point, `SearchQuery::new` — this
    // is the only raw-text path any UI or the native API can use, so a test that bypasses
    // it (as this one used to, via a hand-built struct literal) would certify a path that
    // cannot work in production while hiding that fact. `SearchQuery::new` now keeps the
    // whole dash-joined callsign as ONE token (its edge-trimmed "whole" form — see its
    // doc comment) precisely so this works: pass 3 tokenises the STORED callsign on
    // whitespace only, so the intact callsign is what it stores too.
    let query = SearchQuery::new(&call, None, &[]);
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
            .is_some_and(|r| r.starts_with("1 candidate")),
        // `contains('1')` would also pass for "11 candidate(s)" or "21 candidate(s)" —
        // anchoring on the leading count is what actually pins the number down.
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

#[tokio::test]
async fn a_hyphenated_surname_is_found_by_typing_the_surname_alone() {
    // CRITICAL regression coverage (review round 1, #344): typing a compound surname back
    // EXACTLY as printed — "the standard narrowing gesture" — used to fail. The OLD
    // `SearchQuery::new` split on every non-alphanumeric character, so typing
    // "O'Brien-Smith" fragmented into ["o","brien","smith"], none of which equalled the
    // STORED token: pass 3 tokenises the stored name only on whitespace, so
    // "O'Brien-Smith" stays ONE intact stored token ("o'brien-smith"). The signed
    // attestation this search feeds (db/045) would have recorded a diligent-looking but
    // useless search, indistinguishable from a genuine new patient.
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
        name_assertion_body("Anne Marie O'Brien-Smith", Some("legal"), "patient-stated"),
        render_name_twin("Anne Marie O'Brien-Smith", Some("legal"), "patient-stated"),
    )
    .await
    .expect("name accepted");

    // Searching by the surname ALONE, punctuation intact — not the given name, and not a
    // fragment of the surname.
    let query = SearchQuery::new("O'Brien-Smith", None, &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the hyphenated surname, typed alone, must find the chart: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, p);
}

#[tokio::test]
async fn an_identifier_with_a_materialised_key_is_found_by_its_printed_form() {
    // IMPORTANT 1 regression coverage (review round 1, #344): db/046 pass 1 used to match
    // ONLY `patient_identifier.match_key` (= coalesce(normalized, value), db/010) — the
    // MATERIALISED canonical form a §4.4 profile derives (an NHS number's digits-only
    // "9434765919"). A clerk types what is PRINTED on the card ("943 476 5919"), not its
    // normalisation; the query side has no profile to re-derive `normalized` with
    // (ADR-0033), so pass 1 must also match the raw `value` a clerk actually types.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "943 476 5919",
        system: "nhs-number",
        provenance: "document-verified",
        normalized: Some("9434765919"),
        profile: Some("nhs-number@b3-abc"),
        use_: Some("national-id"),
    };
    submit_identifier(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("identifier with a materialised key accepted");

    // The clerk types the number exactly as printed on the card, spaces and all — NOT the
    // digits-only materialised form the registering profile computed.
    let query = SearchQuery::new(
        "",
        None,
        &[("nhs-number".to_string(), "943 476 5919".to_string())],
    );
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the printed-form search must find the chart: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, p);
}

#[tokio::test]
async fn a_candidates_photo_reference_is_the_original_rendition_not_whichever_is_first() {
    // IMPORTANT 2 regression coverage (review round 1, #344): `read_photo_refs` used to
    // index `renditions -> 0` positionally rather than selecting by `role`. ADR-0042
    // exists precisely so ONE attachment can carry N renditions (a thumbnail preview
    // alongside the original), so this event is built with the PREVIEW listed FIRST and
    // the ORIGINAL second — the ordering a positional read gets wrong. Nothing exercised
    // this read path (`identity.evidence.asserted` -> `Candidate::photo_ref`) at all
    // before this test. Built by hand rather than through `photo_evidence::
    // assert_photo_evidence` (which always emits a single "original" rendition, so it
    // cannot construct the two-rendition case this bug needs to be caught at all).
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
        name_assertion_body("Priya Photo", Some("legal"), "patient-stated"),
        render_name_twin("Priya Photo", Some("legal"), "patient-stated"),
    )
    .await
    .expect("name accepted");

    let preview = Rendition::reference("preview", b"thumbnail-bytes", "image/jpeg");
    let original = Rendition::reference(
        RENDITION_ROLE_ORIGINAL,
        b"full-resolution-bytes",
        "image/jpeg",
    );
    let attachment = Attachment {
        descriptor: "frontal face photograph on arrival".to_string(),
        renditions: vec![preview.clone(), original.clone()],
    };
    let twin = render_identity_evidence_twin(PHOTO_EVIDENCE_KIND, Some("on arrival"), &attachment);
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: photo_evidence_body(Some("on arrival")),
        attachments: vec![attachment],
        plaintext_twin: Some(twin),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = cairn_event::sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("two-rendition photo evidence accepted");

    let query = SearchQuery::new("Priya", None, &[]);
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(list.candidates.len(), 1, "the photographed chart: {list:?}");
    assert_eq!(
        list.candidates[0].photo_ref.as_deref(),
        Some(original.digest_hex.as_str()),
        "photo_ref must be the ORIGINAL rendition, found by role, not whichever sits first: {list:?}"
    );
    assert_ne!(
        list.candidates[0].photo_ref.as_deref(),
        Some(preview.digest_hex.as_str()),
        "must never return the preview's digest: {list:?}"
    );
}

#[tokio::test]
async fn two_tied_original_renditions_resolve_to_the_same_digest_every_time() {
    // N1 regression coverage (review round 2, #344): `role` carries no uniqueness
    // constraint (the wire shape, `cairn_event::attachment::Rendition`, or the DB), so two
    // renditions both marked "original" — or two attachments on one event, each with its
    // own original — tie on `(hlc_wall, hlc_counter)`. Without a total tiebreak,
    // `DISTINCT ON`'s pick among tied rows is Postgres's to make, not this query's, and
    // could differ between two runs of the SAME search — intolerable on a
    // wrong-chart-prevention surface, where a clerk must see the same photo every time.
    // `digest_hex` closes the `ORDER BY`, so the pick is content-derived and stable.
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
        name_assertion_body("Tied Original", Some("legal"), "patient-stated"),
        render_name_twin("Tied Original", Some("legal"), "patient-stated"),
    )
    .await
    .expect("name accepted");

    // TWO renditions, BOTH role="original", on one attachment — the exact tie N1
    // describes. Distinct bytes so the two renditions have distinct, comparable digests.
    let original_a =
        Rendition::reference(RENDITION_ROLE_ORIGINAL, b"candidate-bytes-a", "image/jpeg");
    let original_b =
        Rendition::reference(RENDITION_ROLE_ORIGINAL, b"candidate-bytes-b", "image/jpeg");
    // Deliberately array-ORDERED so the FIRST rendition is the one with the LARGER digest —
    // i.e. NOT the one `ORDER BY ... digest_hex` (ascending) should pick. Without this, the
    // test would be non-discriminating: `jsonb_array_elements` walks an array in its own
    // stored order, so "whichever rendition happens to be listed first" is itself a stable,
    // if accidental, order — a query that silently fell back to array position instead of
    // sorting by `digest_hex` could still pass a test that never puts the "wrong" one first.
    let (first_in_array, second_in_array) = if original_a.digest_hex > original_b.digest_hex {
        (original_a.clone(), original_b.clone())
    } else {
        (original_b.clone(), original_a.clone())
    };
    let attachment = Attachment {
        descriptor: "two tied originals on one attachment".to_string(),
        renditions: vec![first_in_array, second_in_array],
    };
    let twin = render_identity_evidence_twin(PHOTO_EVIDENCE_KIND, None, &attachment);
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: photo_evidence_body(None),
        attachments: vec![attachment],
        plaintext_twin: Some(twin),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = cairn_event::sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("two-tied-original photo evidence accepted");

    let query = SearchQuery::new("Tied", None, &[]);
    let expected = [original_a.digest_hex.clone(), original_b.digest_hex.clone()]
        .into_iter()
        .min()
        .unwrap();

    // Run the search TWICE: the point under test is that the SAME digest comes back both
    // times, not merely that a value exists.
    for attempt in 1..=2 {
        let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
            .await
            .expect("search succeeds");
        assert_eq!(
            list.candidates.len(),
            1,
            "attempt {attempt}: the tied-original chart: {list:?}"
        );
        assert_eq!(
            list.candidates[0].photo_ref.as_deref(),
            Some(expected.as_str()),
            "attempt {attempt}: the tie must resolve to the SAME digest every run: {list:?}"
        );
    }
}

#[tokio::test]
async fn a_repudiated_only_name_reads_as_withheld_not_incomplete() {
    // N3 regression coverage (review round 2, #344): a chart whose ONLY name was struck as
    // known-false (db/025) has NO winner row in `patient_name_current` BY DESIGN — showing
    // the known-false name back would be a lie (principle 4). That absence must read as an
    // honest "(name withheld)", NOT count toward `incomplete`: a genuine read failure and an
    // intentional withholding have OPPOSITE remedies for a clerk (search harder vs. accept
    // the chart is real but unnamed), and conflating them would hide exactly the ADR-0060
    // decision-2 signal that tells the clerk the search was not exhaustive.
    //
    // Driven through the REAL suppressing-mode path (a human attestation, via the harness
    // lifted from `identity_repudiate.rs` into `common::enroll_human`/`submit_attested`),
    // not a raw overlay-table INSERT: this codebase's tests exercise the actual floor.
    //
    // PAIRED WITH `a_repudiation_naming_no_asserted_name_still_counts_as_incomplete` below:
    // this test's repudiation strikes a value that IS the chart's own `patient_name` row, so
    // it alone does NOT discriminate the fixed predicate (`read_names_ever_asserted`, which
    // checks `patient_name`) from the review-round-2 predicate it replaced (`patient_alias_
    // pool`, keyed only on the repudiation's subject) — both would answer identically here,
    // since the repudiation's subject IS this chart either way (review round 3 finding).
    // This test covers the honest by-design absence; the other one covers the fail-open case.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    // Matched via an identifier so the chart is a real search candidate once its only name
    // is struck — a repudiation alone, with no other evidence on the chart, would not
    // surface it as a candidate at all, and this test would prove nothing.
    let p = Uuid::now_v7();
    let ident = IdentifierAssertion {
        value: "with-repudiated-name",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &ident)
        .await
        .expect("identifier accepted");
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        name_assertion_body("Fabricated Persona", Some("legal"), "patient-stated"),
        render_name_twin("Fabricated Persona", Some("legal"), "patient-stated"),
    )
    .await
    .expect("name accepted");

    // Strike the chart's ONLY name. `identity.repudiate.asserted` is suppressing-mode, so
    // db/005's attestation gate always demands a responsibility-bearing human (§5.7
    // "Human") — the agent-only key `setup` enrolls cannot sign this off alone.
    let subject_s = p.to_string();
    let rep = RepudiationAssertion {
        subject: &subject_s,
        value: "Fabricated Persona",
        reason: "confessed fabricated persona",
    };
    let repudiation_body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subject_s.clone(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall: 3,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: repudiation_assertion_body(&rep),
        attachments: vec![],
        plaintext_twin: Some(render_repudiate_twin(&rep)),
        clock_grade: ClockGrade::SelfAsserted,
    };
    submit_attested(&c, &sk, repudiation_body, &sk_h, &kid_h)
        .await
        .expect("repudiation accepted with human attestation");

    let query = SearchQuery::new(
        "",
        None,
        &[("MRN".to_string(), "with-repudiated-name".to_string())],
    );
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the chart must still be found, never dropped: {list:?}"
    );
    assert_eq!(list.candidates[0].display_name, "(name withheld)");
    assert!(
        !list.incomplete,
        "a by-design repudiated-name absence must NOT count as a read failure: {list:?}"
    );
    assert!(list.incomplete_reason.is_none());
}

#[tokio::test]
async fn a_repudiation_naming_no_asserted_name_still_counts_as_incomplete() {
    // N3's ACTUAL fail-open case (review round 3, #344): db/025's own structural floor
    // explicitly permits a repudiation to arrive with no matching name evidence on the
    // chart ("a repudiation may legitimately arrive before or independently of the name
    // assertion it strikes" — offline-first sync ordering has no guarantee). So a chart can
    // carry a `name_repudiation` row while its `patient_name` retained set stays EMPTY.
    //
    // The review-round-2 predicate this replaced (`patient_alias_pool`, keyed only on the
    // repudiation's subject) would have answered "yes, repudiated" here purely because a
    // repudiation NAMES this subject — with no requirement that the struck value ever
    // corresponded to anything this chart actually asserted. That is precisely "a chart with
    // zero names ever asserted plus one unrelated repudiation" (this crate's own round-2 doc
    // comment already named the shape) rendering "(name withheld)" and being silently
    // excluded from `incomplete` — a genuine read gap disguised as an intentional one. The
    // FIXED predicate (`read_names_ever_asserted`, checking `patient_name` itself) must NOT
    // make that mistake: zero `patient_name` rows means the honest answer is "unreadable",
    // not "withheld".
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    // Matched via an identifier so the chart is a real search candidate. Deliberately NO
    // `submit_field` name assertion at all — `patient_name` stays empty for this patient.
    let p = Uuid::now_v7();
    let ident = IdentifierAssertion {
        value: "no-name-ever-asserted",
        system: "MRN",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_identifier(&c, &sk, &kid, p, 1, &ident)
        .await
        .expect("identifier accepted");

    // A repudiation naming THIS chart, but for a value it never asserted — the "arrived
    // independently of the name assertion it strikes" case db/025's floor permits.
    let subject_s = p.to_string();
    let rep = RepudiationAssertion {
        subject: &subject_s,
        value: "A Name This Chart Never Asserted",
        reason: "advance notice: known alias, do not trust if it ever appears",
    };
    let repudiation_body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subject_s.clone(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: repudiation_assertion_body(&rep),
        attachments: vec![],
        plaintext_twin: Some(render_repudiate_twin(&rep)),
        clock_grade: ClockGrade::SelfAsserted,
    };
    submit_attested(&c, &sk, repudiation_body, &sk_h, &kid_h)
        .await
        .expect("repudiation with no matching name assertion accepted");

    let query = SearchQuery::new(
        "",
        None,
        &[("MRN".to_string(), "no-name-ever-asserted".to_string())],
    );
    let list = cairn_node::patient::search::search_patients(&c, &query, "2026-06-16")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the chart must still be found, never dropped: {list:?}"
    );
    assert_eq!(
        list.candidates[0].display_name, "(name unavailable)",
        "an unrelated repudiation must NOT be read as this chart's own withheld name: {list:?}"
    );
    assert!(
        list.incomplete,
        "zero patient_name rows is a genuine read gap and MUST count as incomplete: {list:?}"
    );
    assert!(
        list.incomplete_reason
            .as_ref()
            .is_some_and(|r| r.starts_with("1 candidate")),
        "{:?}",
        list.incomplete_reason
    );
}
