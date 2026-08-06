//! #350 / Task 8b: `register_patient` must ALSO assert the typed name/dob so the
//! search-before-create funnel it feeds can actually find the chart it just created.
//!
//! Split out of `patient_register.rs` (which covers Task 6's original registration-attestation
//! behaviour) by RESPONSIBILITY, not file size: that suite exercises the search-attestation
//! act `register_patient` already authored before this task, this suite exercises the
//! name/dob demographic facts #350 adds. This repo does not actually cap test-file length
//! (`medication_attestation.rs` alone runs to 2301 lines) — the split mirrors the existing
//! one-suite-per-concern convention (`john_doe.rs`/`patient_search.rs`/`patient_registration.rs`
//! are already separate files for closely related concerns), not a line-count rule. (Corrected
//! from an earlier, inaccurate "500-line house limit" claim — review round 1, #350 Minor.)
//!
//! **The bug this closes.** Task 8's own report reproduced it live: `register_patient`
//! authored only the `identity.registration.asserted` act, never a `patient_name`/
//! `patient_demographic` fact — so "register Jane Testpatient" -> "search Jane Testpatient"
//! found NOTHING, and a second registration silently minted a duplicate chart. Filed as
//! issue #350. `a_registered_patient_is_findable_by_a_later_search_on_the_same_name` below is
//! that exact scenario, now fixed, driven through the real `search_patients` entry point.
//!
//! **The final review found the same hole one pass over (C1).** #350 closed db/046's passes 2
//! (dob) and 3 (name) and left pass 1 — the HIGHEST-precision one — open: `register_patient`
//! searched on every `--identifier`, signed it into the permanent attestation, and never wrote
//! a `demographic.identifier.asserted` event, the only thing that populates `patient_identifier`
//! (db/010). So an MRN search could not find a chart the funnel itself created, and an
//! identifier-only registration was unreachable on every pass, forever.
//! `a_registered_patient_is_findable_by_a_later_search_on_the_identifier_alone` and
//! `an_identifier_only_registration_asserts_an_identifier_and_no_name_or_dob` below are the
//! end-to-end closure.
mod common;

use cairn_node::db;
use cairn_node::patient::register::register_patient;
use cairn_node::patient::search::search_patients;
use cairn_patient_search::{CandidateList, SearchQuery};
use common::{cs, setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// The projections this suite writes to, beyond `common::setup`'s clinical core
/// (`patient_demographic` is already truncated there; `patient_name`/`patient_registration`
/// are not — both created by later migrations, same `to_regclass`-guarded discipline every
/// suite in this directory uses).
const EXTRA_TABLES: [&str; 2] = ["patient_registration", "patient_name"];

/// An empty `CandidateList` — the "genuinely new patient" case: nothing was on screen for
/// the clerk to attest to, which is the ordinary shape for every test in this file (each
/// registers a never-before-seen name).
fn no_candidates() -> CandidateList {
    CandidateList {
        candidates: vec![],
        incomplete: false,
        incomplete_reason: None,
    }
}

/// The (value, provenance) of every retained name on `p`, ordered by value for determinism.
async fn patient_names(c: &Client, p: Uuid) -> Vec<(String, String)> {
    c.query(
        "SELECT value, provenance FROM patient_name \
         WHERE patient_id = $1::text::uuid ORDER BY value",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

/// `patient_demographic`'s dob row for `p`, as `(value, provenance)`, or `None`.
async fn patient_dob(c: &Client, p: Uuid) -> Option<(String, String)> {
    c.query_opt(
        "SELECT value, provenance FROM patient_demographic \
         WHERE patient_id = $1::text::uuid AND field = 'dob'",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

/// `patient_demographic`'s dob `facets.precision` for `p`, or `None`.
async fn patient_dob_precision(c: &Client, p: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT facets ->> 'precision' FROM patient_demographic \
         WHERE patient_id = $1::text::uuid AND field = 'dob'",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .and_then(|r| r.get(0))
}

/// Every event this chart carries, ordered by HLC (registration first, by construction) —
/// `(event_type, field, hlc_wall, hlc_counter)`. `field` is `body ->> 'field'`: `None` for
/// the registration act (which carries no such key), `Some("name")`/`Some("dob")` for a
/// demographic assertion. Carrying `field` is load-bearing, not decorative (review round 1,
/// #350 Minor): `event_type` alone is IDENTICAL ("demographic.field.asserted") for both the
/// name and dob events, so an ordering assertion that only compares `event_type` cannot tell
/// a name-then-dob run from a dob-then-name one — a swapped `h_name`/`h_dob` tick would still
/// pass. Reading straight from `event_log` rather than any projection is deliberate too: this
/// is what proves the events actually landed TOGETHER, independent of what each projection
/// separately decided to keep.
async fn events_of(c: &Client, p: Uuid) -> Vec<(String, Option<String>, i64, i32)> {
    c.query(
        "SELECT event_type, body ->> 'field' AS field, hlc_wall, hlc_counter FROM event_log \
         WHERE patient_id::text = $1 ORDER BY hlc_wall, hlc_counter",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
    .collect()
}

/// Every retained identifier on `p`, as `(system, value, match_key, provenance)`, ordered
/// for determinism. `match_key` is read explicitly because it is what db/046 pass 1's
/// `pi.match_key = ...` arm compares against — see
/// `an_identifier_is_asserted_with_no_normalized_form_and_no_profile` for why it must equal
/// `value` on this path.
async fn patient_identifiers(c: &Client, p: Uuid) -> Vec<(String, String, String, String)> {
    c.query(
        "SELECT system, value, match_key, provenance FROM patient_identifier \
         WHERE patient_id = $1::text::uuid ORDER BY system, match_key",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
    .collect()
}

/// `patient_registration.class` for `p` — used only by the identifier-only test below to
/// confirm registration still succeeded as a normal Standard chart despite asserting no
/// name and no dob.
async fn registration_class(c: &Client, p: Uuid) -> String {
    c.query_one(
        "SELECT class FROM patient_registration WHERE patient_id::text = $1",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_registered_patient_is_findable_by_a_later_search_on_the_same_name() {
    // THE test that was impossible before this task — see this file's module doc for the
    // exact reproduction it closes. Register, then search through the REAL `search_patients`
    // entry point (the same one `patient-search`/`patient-register` call), and the chart
    // must come back.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Jane O'Brien-Testpatient";
    let dob = "1980-01-01";
    let query = SearchQuery::new(name, Some(dob), &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    // Search again, exactly as `patient-register`'s own pre-write search would: by the
    // surname alone, punctuation intact — the standard narrowing gesture a clerk types.
    let search_query = SearchQuery::new("O'Brien-Testpatient", None, &[]);
    let list = search_patients(&c, &search_query, "2026-08-05")
        .await
        .expect("search succeeds");

    assert_eq!(
        list.candidates.len(),
        1,
        "the chart just registered must be found by a later search on its own name: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, pid);
    assert_eq!(list.candidates[0].display_name, name);

    // And by dob alone too, since a dob event was also asserted.
    let dob_query = SearchQuery::new("", Some(dob), &[]);
    let by_dob = search_patients(&c, &dob_query, "2026-08-05")
        .await
        .expect("search succeeds");
    assert_eq!(by_dob.candidates.len(), 1, "found by dob too: {by_dob:?}");
    assert_eq!(by_dob.candidates[0].patient_id, pid);
}

#[tokio::test]
async fn a_year_only_birth_date_asserts_year_precision_never_a_fabricated_day() {
    // Review round 1, #350, Important 1: `register_patient` used to hardcode "day" for EVERY
    // dob, so `--birth-date 1980` produced a permanent, signed twin claiming a precision
    // nobody has. End-to-end proof the derivation is actually wired in (the `dob_precision`
    // unit tests in `register.rs` cover the pure function in isolation; this proves
    // `register_patient` itself calls it rather than still hardcoding something).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let query = SearchQuery::new("Year Only Patient", Some("1980"), &[]);
    let pid = register_patient(
        &mut c,
        &sk,
        &kid,
        "n",
        Some("Year Only Patient"),
        &query,
        &no_candidates(),
    )
    .await
    .expect("a year-only birth date is a recognised shape and must be accepted");

    assert_eq!(
        patient_dob_precision(&c, pid).await.as_deref(),
        Some("year"),
        "a year-only value must assert year precision, never a fabricated \"day\""
    );

    let query = SearchQuery::new("Month Only Patient", Some("1980-06"), &[]);
    let pid_month = register_patient(
        &mut c,
        &sk,
        &kid,
        "n",
        Some("Month Only Patient"),
        &query,
        &no_candidates(),
    )
    .await
    .expect("a year-month birth date is a recognised shape and must be accepted");
    assert_eq!(
        patient_dob_precision(&c, pid_month).await.as_deref(),
        Some("month")
    );

    // And CLOSE THE LOOP (second whole-branch review): the #350 findability guarantee was
    // proven for names and identifiers but never for the reduced-precision dates this very
    // test legitimises. Searching the same partial date back through the REAL
    // `search_patients` entry point must find exactly the chart registered with it —
    // db/046 pass 2 is an exact string compare, so "1980" finds the year-only chart and
    // does not sweep in the month-only one, and vice versa.
    let by_year = search_patients(&c, &SearchQuery::new("", Some("1980"), &[]), "2026-08-05")
        .await
        .expect("search succeeds");
    assert_eq!(
        by_year.candidates.len(),
        1,
        "a year-only registration must be findable by the same year-only date: {by_year:?}"
    );
    assert_eq!(by_year.candidates[0].patient_id, pid);

    let by_month = search_patients(
        &c,
        &SearchQuery::new("", Some("1980-06"), &[]),
        "2026-08-05",
    )
    .await
    .expect("search succeeds");
    assert_eq!(
        by_month.candidates.len(),
        1,
        "a month-only registration must be findable by the same partial date, and the \
         year-only chart must not leak into it: {by_month:?}"
    );
    assert_eq!(by_month.candidates[0].patient_id, pid_month);
}

#[tokio::test]
async fn an_unrecognised_birth_date_shape_is_refused_not_silently_coerced() {
    // Review round 1, #350, Important 1: the WHOLE call must refuse — no chart minted at
    // all — rather than silently coercing a malformed shape to some guessed precision.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let query = SearchQuery::new("Bad Date Patient", Some("15/06/1980"), &[]);
    let result = register_patient(
        &mut c,
        &sk,
        &kid,
        "n",
        Some("Bad Date Patient"),
        &query,
        &no_candidates(),
    )
    .await;
    assert!(
        result.is_err(),
        "an unrecognised birth-date shape must refuse the whole registration"
    );

    let total: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        total, 0,
        "a refused precision must mint NOTHING — not even the registration act"
    );
}

#[tokio::test]
async fn a_name_with_no_birth_date_asserts_a_name_event_and_no_dob_event() {
    // Principle 4: never fabricate the field that was never supplied. A clerk who typed a
    // name but no dob must get a name assertion and NOTHING pretending to be a dob.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Name Only Patient";
    let query = SearchQuery::new(name, None, &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    // Compared against the LITERAL "registrar-entered", not the imported constant: asserting
    // against the constant would make this test vacuous against a change to the constant's
    // own value (verified non-vacuous by hand — see the task report).
    let names = patient_names(&c, pid).await;
    assert_eq!(
        names,
        vec![(name.to_string(), "registrar-entered".to_string())],
        "exactly one name event, and only one: {names:?}"
    );
    assert_eq!(
        patient_dob(&c, pid).await,
        None,
        "no birth date was supplied — asserting one would be a fabricated placeholder"
    );

    // Confirmed at the wire level too, not only the projection: exactly TWO events
    // (registration + name), never a phantom third.
    let events = events_of(&c, pid).await;
    assert_eq!(events.len(), 2, "registration + name, no dob: {events:?}");
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(events[1].0, "demographic.field.asserted");
    assert_eq!(events[1].1.as_deref(), Some("name"));
}

#[tokio::test]
async fn a_birth_date_with_no_name_asserts_a_dob_event_and_no_name_event() {
    // The mirror image of the test above (review round 1, #350 Minor: this case was
    // missing). A clerk who knows the dob but not the name — e.g. reading it off a document
    // while the patient cannot speak — must get a dob assertion and NOTHING pretending to
    // be a name.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let dob = "1965-09-30";
    let query = SearchQuery::new("", Some(dob), &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &no_candidates())
        .await
        .expect("registration accepted");

    assert!(
        patient_names(&c, pid).await.is_empty(),
        "no name was supplied — no name event must land"
    );
    let (dob_value, dob_provenance) = patient_dob(&c, pid).await.expect("dob row must exist");
    assert_eq!(dob_value, dob);
    assert_eq!(dob_provenance, "registrar-entered");

    let events = events_of(&c, pid).await;
    assert_eq!(events.len(), 2, "registration + dob, no name: {events:?}");
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(events[1].0, "demographic.field.asserted");
    assert_eq!(events[1].1.as_deref(), Some("dob"));
}

#[tokio::test]
async fn an_identifier_only_registration_asserts_an_identifier_and_no_name_or_dob() {
    // The identifier-only case: a clerk registering off an MRN card alone, no name spoken,
    // no dob given. Registration must still succeed (principle 4 — no required field may be
    // satisfiable only by fabrication), must assert the IDENTIFIER that was actually
    // supplied, and must assert nothing pretending to be a name or a dob.
    //
    // THIS TEST PREVIOUSLY ASSERTED THE BUG (final review, C1). It read
    // `assert_eq!(events.len(), 1, "the registration act alone, no demographic events at
    // all")` — locking in a chart with ZERO searchable content on ANY of db/046's three
    // passes: unreachable, forever, by every search this slice ships. The rule was never
    // "assert nothing"; it is "assert only what was actually supplied", and an identifier
    // WAS supplied here.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let identifiers = [("MRN".to_string(), "identifier-only-999".to_string())];
    let query = SearchQuery::new("", None, &identifiers);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &no_candidates())
        .await
        .expect("registration accepted with only an identifier");

    assert!(
        patient_names(&c, pid).await.is_empty(),
        "no name was supplied — no name event must land"
    );
    assert_eq!(
        patient_dob(&c, pid).await,
        None,
        "no dob was supplied — no dob event must land"
    );
    assert_eq!(
        patient_identifiers(&c, pid).await,
        vec![(
            "MRN".to_string(),
            "identifier-only-999".to_string(),
            "identifier-only-999".to_string(),
            "registrar-entered".to_string(),
        )],
        "the identifier the clerk read off the card is the ONLY thing this chart can ever \
         be found by — it must land"
    );
    let events = events_of(&c, pid).await;
    assert_eq!(
        events.len(),
        2,
        "the registration act plus the identifier that was actually supplied: {events:?}"
    );
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(events[1].0, "demographic.identifier.asserted");
    assert_eq!(events[1].1.as_deref(), Some("identifier"));
    assert_eq!(registration_class(&c, pid).await, "standard");

    // And the whole point: this chart, which has no name and no dob at all, is REACHABLE.
    let by_identifier = SearchQuery::new("", None, &identifiers);
    let list = search_patients(&c, &by_identifier, "2026-08-05")
        .await
        .expect("search succeeds");
    assert_eq!(
        list.candidates.len(),
        1,
        "an identifier-only chart must be findable by its identifier, or it is unreachable \
         by every search this slice ships: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, pid);
}

// --- final review, C1: the identifiers the funnel searched on must be PERSISTED ---
//
// `register_patient` authored the registration act, a name and a dob — and discarded every
// `--identifier` it had just searched on and signed into the permanent attestation. So db/046
// pass 1, the HIGHEST-PRECISION pass and the gesture db/045 explicitly blesses as "a complete
// and often better search", could never find a chart the funnel itself created. The tests
// below are the end-to-end closure on that pass.

#[tokio::test]
async fn a_registered_patient_is_findable_by_a_later_search_on_the_identifier_alone() {
    // The C1 failure scenario, driven end to end: a clerk registers from an MRN card, then
    // three weeks later searches that same MRN — the precise, correct gesture — and must
    // find the chart rather than "no candidates found" plus a duplicate whose signed
    // attestation reads as perfectly diligent.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Mrn Card Patient";
    let identifiers = [("MRN".to_string(), "12345".to_string())];
    let query = SearchQuery::new(name, Some("1980-01-01"), &identifiers);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    // Searching on the identifier ALONE — no name, no dob. This is the pass-1 gesture.
    let by_identifier = SearchQuery::new("", None, &identifiers);
    let list = search_patients(&c, &by_identifier, "2026-08-05")
        .await
        .expect("search succeeds");
    assert_eq!(
        list.candidates.len(),
        1,
        "the chart just registered must be found by a later search on the identifier it was \
         registered with: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, pid);
}

#[tokio::test]
async fn a_padded_identifier_at_registration_is_found_by_the_unpadded_search() {
    // The N3 cross-gesture case (ADR-0061, final review, maintainer decision: trim BOTH
    // sides). A clerk pastes an MRN straight off a scanned card, whitespace and all —
    // `patient-register --identifier "MRN= 55512"` — and weeks later a different clerk types
    // the same MRN clean, no padding — `patient-search --identifier MRN=55512`. Before this
    // fix `SearchQuery::new` and `supplied_identifiers` left the value exactly as typed, so
    // the stored value carried the paste's whitespace while the query never did, and db/046
    // pass 1's exact `=` compare silently missed the very chart the funnel had just created
    // and signed a diligent-looking attestation for.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Padded Mrn Patient";
    // Registered from a pasted card: leading/trailing whitespace on both system and value.
    let padded_identifiers = [("  MRN  ".to_string(), "  55512  ".to_string())];
    let query = SearchQuery::new(name, None, &padded_identifiers);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    // What actually landed must be trimmed — the whole point of the fix, checked directly
    // against the projection rather than inferred from the search below.
    let stored = patient_identifiers(&c, pid).await;
    assert_eq!(
        stored,
        vec![(
            "MRN".to_string(),
            "55512".to_string(),
            "55512".to_string(),
            "registrar-entered".to_string(),
        )],
        "the padded identifier must be stored TRIMMED, or a clean later search can never \
         match it: {stored:?}"
    );

    // A different clerk, weeks later, types the same MRN clean — no padding on either side.
    let clean_identifiers = [("MRN".to_string(), "55512".to_string())];
    let by_identifier = SearchQuery::new("", None, &clean_identifiers);
    let list = search_patients(&c, &by_identifier, "2026-08-05")
        .await
        .expect("search succeeds");
    assert_eq!(
        list.candidates.len(),
        1,
        "a clean, unpadded search must find the chart registered from a padded paste: {list:?}"
    );
    assert_eq!(list.candidates[0].patient_id, pid);
}

#[tokio::test]
async fn every_supplied_identifier_lands_and_each_one_finds_the_chart() {
    // A patient handing over two cards (a hospital MRN and a national number) must be
    // findable by EITHER. One dropped identifier is a silently-unfindable chart on the
    // highest-precision pass.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Two Card Patient";
    let identifiers = [
        ("MRN".to_string(), "MC-001".to_string()),
        ("NHI".to_string(), "ZZZ9999".to_string()),
    ];
    let query = SearchQuery::new(name, None, &identifiers);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    let stored = patient_identifiers(&c, pid).await;
    assert_eq!(
        stored.len(),
        2,
        "both supplied identifiers must land, not just the first: {stored:?}"
    );
    assert_eq!(stored[0].0, "MRN");
    assert_eq!(stored[0].1, "MC-001");
    assert_eq!(stored[1].0, "NHI");
    assert_eq!(stored[1].1, "ZZZ9999");

    // Each, on its own, finds the chart.
    for pair in &identifiers {
        let q = SearchQuery::new("", None, std::slice::from_ref(pair));
        let list = search_patients(&c, &q, "2026-08-05")
            .await
            .expect("search succeeds");
        assert_eq!(
            list.candidates.len(),
            1,
            "searching {pair:?} alone must find the chart: {list:?}"
        );
        assert_eq!(list.candidates[0].patient_id, pid);
    }

    // Wire level: registration + name + two identifiers, no phantom extras.
    let events = events_of(&c, pid).await;
    assert_eq!(
        events.len(),
        4,
        "registration + name + two identifiers: {events:?}"
    );
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(events[1].1.as_deref(), Some("name"));
    assert_eq!(events[2].1.as_deref(), Some("identifier"));
    assert_eq!(events[3].1.as_deref(), Some("identifier"));
}

#[tokio::test]
async fn no_identifier_supplied_means_no_identifier_event() {
    // Principle 4, the mirror of the name/dob rule: nothing is invented to fill the gap.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "No Identifier Patient";
    let query = SearchQuery::new(name, None, &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    assert!(
        patient_identifiers(&c, pid).await.is_empty(),
        "no identifier was supplied — asserting one would be a fabricated placeholder"
    );
    let events = events_of(&c, pid).await;
    assert_eq!(events.len(), 2, "registration + name only: {events:?}");
}

#[tokio::test]
async fn an_identifier_is_asserted_with_no_normalized_form_and_no_profile() {
    // The registrar holds no §4.4 comparator profile (ADR-0014/ADR-0033), so claiming one —
    // or materialising a `normalized` key without naming the profile that produced it, which
    // the db/010 floor refuses outright — would be a fabrication. With both absent,
    // `match_key` falls back to `value` (db/010 `COALESCE(norm, value)`), which is exactly
    // what db/046 pass 1's `pi.match_key = ...` arm compares a clerk's typed value against.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let identifiers = [("MRN".to_string(), "943 476 5919".to_string())];
    let query = SearchQuery::new("", None, &identifiers);
    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &no_candidates())
        .await
        .expect("registration accepted");

    let row = c
        .query_one(
            "SELECT normalized, profile, match_key, value FROM patient_identifier \
             WHERE patient_id = $1::text::uuid",
            &[&pid.to_string()],
        )
        .await
        .unwrap();
    let normalized: Option<String> = row.get(0);
    let profile: Option<String> = row.get(1);
    let match_key: String = row.get(2);
    let value: String = row.get(3);
    assert_eq!(
        normalized, None,
        "the registrar materialised no key — claiming one would be a fabrication"
    );
    assert_eq!(profile, None, "the registrar holds no §4.4 profile");
    assert_eq!(
        match_key, value,
        "with no normalized form, match_key must fall back to the as-entered value"
    );
    assert_eq!(value, "943 476 5919");
}

// --- review round 1, #350, Important 2: the blank-name filter is the LIVE CLI shape ---
//
// `main.rs`'s `--name` defaults to `""` and is passed through UNCONDITIONALLY as
// `Some(name.as_str())` — never `None`. So an identifier-only registration through the real
// CLI reaches `register_patient` as `Some("")`, not `None`. The pre-review version of this
// suite covered only the `None` shape (in a since-superseded test — today
// `an_identifier_only_registration_asserts_an_identifier_and_no_name_or_dob` carries the
// `None` case), the library-caller shape a test finds easy to write but the CLI never
// actually sends. The two tests below pin the ACTUAL shape.

#[tokio::test]
async fn a_blank_empty_string_name_the_live_cli_shape_asserts_no_name_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let query = SearchQuery::new(
        "",
        None,
        &[("MRN".to_string(), "cli-blank-name-999".to_string())],
    );
    let pid = register_patient(&mut c, &sk, &kid, "n", Some(""), &query, &no_candidates())
        .await
        .expect("registration accepted despite an empty-string name");

    assert!(
        patient_names(&c, pid).await.is_empty(),
        "Some(\"\") is the CLI's actual empty shape and must assert no name event"
    );
}

#[tokio::test]
async fn a_whitespace_only_name_asserts_no_name_event() {
    // A clerk who fat-fingers a stray space into --name (or a UI that pads a text field)
    // must not get a name event whose "value" is invisible whitespace.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let query = SearchQuery::new(
        "",
        None,
        &[("MRN".to_string(), "cli-whitespace-name-999".to_string())],
    );
    let pid = register_patient(
        &mut c,
        &sk,
        &kid,
        "n",
        Some("   "),
        &query,
        &no_candidates(),
    )
    .await
    .expect("registration accepted despite a whitespace-only name");

    assert!(
        patient_names(&c, pid).await.is_empty(),
        "whitespace-only must trim to blank and assert no name event"
    );
}

#[tokio::test]
async fn name_and_dob_assertions_carry_registrar_entered_provenance() {
    // Not `patient-stated`: see `patient::register`'s module doc for the full reasoning
    // (the registration-desk speaker is often a third party — a parent, a carer — so
    // `patient-stated` would frequently be a precise untruth about WHO stated it).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Provenance Check Patient";
    let dob = "1990-05-05";
    let query = SearchQuery::new(name, Some(dob), &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    // Compared against the LITERAL "registrar-entered", not the imported constant — see the
    // note on the other use of this literal above for why.
    let names = patient_names(&c, pid).await;
    assert_eq!(names.len(), 1);
    assert_eq!(names[0].1, "registrar-entered");

    let (_value, dob_provenance) = patient_dob(&c, pid).await.expect("dob row must exist");
    assert_eq!(dob_provenance, "registrar-entered");
}

#[tokio::test]
async fn registration_name_and_dob_events_land_together_with_the_registration_first() {
    // Atomicity, proved by reading the wire log directly (not a projection, which could
    // paper over a partial write): all three events must be present after ONE call, and
    // strictly HLC-ordered with the registration act first (its own load-bearing invariant,
    // shared with `register_john_doe` — see `john_doe.rs`'s
    // `a_john_doe_chart_begins_with_an_unidentified_registration`).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let name = "Atomic Patient";
    let dob = "1975-03-03";
    let query = SearchQuery::new(name, Some(dob), &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", Some(name), &query, &no_candidates())
        .await
        .expect("registration accepted");

    let events = events_of(&c, pid).await;
    assert_eq!(
        events.len(),
        3,
        "registration + name + dob must ALL land from one call: {events:?}"
    );
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(events[1].0, "demographic.field.asserted");
    assert_eq!(events[2].0, "demographic.field.asserted");
    // `field`, not just `event_type` (review round 1, #350 Minor): the name and dob events
    // are BOTH `demographic.field.asserted`, so an assertion that stopped at `event_type`
    // could not tell a name-then-dob run from a dob-then-name one — a swapped `h_name`/
    // `h_dob` tick in the implementation would still satisfy it. `field` closes that gap.
    assert_eq!(
        events[1].1.as_deref(),
        Some("name"),
        "name must be SECOND (after registration, before dob): {events:?}"
    );
    assert_eq!(
        events[2].1.as_deref(),
        Some("dob"),
        "dob must be THIRD: {events:?}"
    );
    // Strict HLC order, registration first — proves the ticks (and therefore the submits)
    // happened in the documented order, not merely that three rows exist.
    assert!(
        (events[0].2, events[0].3) < (events[1].2, events[1].3),
        "registration must strictly precede the name event: {events:?}"
    );
    assert!(
        (events[1].2, events[1].3) < (events[2].2, events[2].3),
        "name must strictly precede the dob event: {events:?}"
    );
}
