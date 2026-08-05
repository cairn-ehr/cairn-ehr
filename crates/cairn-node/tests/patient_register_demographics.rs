//! #350 / Task 8b: `register_patient` must ALSO assert the typed name/dob so the
//! search-before-create funnel it feeds can actually find the chart it just created.
//!
//! Split out of `patient_register.rs` (which covers Task 6's original registration-attestation
//! behaviour) purely to keep both files under the house 500-line limit — this suite shares
//! that file's `common` harness, `EXTRA_TABLES`-style truncation discipline, and general shape,
//! and should be read alongside it.
//!
//! **The bug this closes.** Task 8's own report reproduced it live: `register_patient`
//! authored only the `identity.registration.asserted` act, never a `patient_name`/
//! `patient_demographic` fact — so "register Jane Testpatient" -> "search Jane Testpatient"
//! found NOTHING, and a second registration silently minted a duplicate chart. Filed as
//! issue #350. `a_registered_patient_is_findable_by_a_later_search_on_the_same_name` below is
//! that exact scenario, now fixed, driven through the real `search_patients` entry point.
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

/// Every event this chart carries, ordered by HLC (registration first, by construction) —
/// `(event_type, hlc_wall, hlc_counter)`. Reading straight from `event_log` rather than any
/// projection is deliberate: this is what proves the events actually landed TOGETHER,
/// independent of what each projection separately decided to keep.
async fn events_of(c: &Client, p: Uuid) -> Vec<(String, i64, i32)> {
    c.query(
        "SELECT event_type, hlc_wall, hlc_counter FROM event_log \
         WHERE patient_id::text = $1 ORDER BY hlc_wall, hlc_counter",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}

/// `patient_registration.class` for `p` — used only by the identifier-only test below to
/// confirm registration still succeeded as a normal Standard chart despite asserting no
/// demographic fact.
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
}

#[tokio::test]
async fn neither_name_nor_birth_date_asserts_no_demographic_events() {
    // The identifier-only case: a clerk registering off an MRN card alone, no name spoken,
    // no dob given. Registration must still succeed (principle 4 — no required field may be
    // satisfiable only by fabrication) and assert NOTHING demographic.
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
        &[("MRN".to_string(), "identifier-only-999".to_string())],
    );

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
    let events = events_of(&c, pid).await;
    assert_eq!(
        events.len(),
        1,
        "the registration act alone, no demographic events at all: {events:?}"
    );
    assert_eq!(events[0].0, "identity.registration.asserted");
    assert_eq!(registration_class(&c, pid).await, "standard");
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
    // Strict HLC order, registration first — proves the ticks (and therefore the submits)
    // happened in the documented order, not merely that three rows exist.
    assert!(
        (events[0].1, events[0].2) < (events[1].1, events[1].2),
        "registration must strictly precede the name event: {events:?}"
    );
    assert!(
        (events[1].1, events[1].2) < (events[2].1, events[2].2),
        "name must strictly precede the dob event: {events:?}"
    );
}
