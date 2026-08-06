//! db/045 floor tests: the structural contract of a registration act.
//!
//! What these must prove, beyond "the happy path works":
//!   * absence for the non-standard classes is STRUCTURAL, not merely optional;
//!   * an empty displayed list is ACCEPTED (the normal new-patient case);
//!   * an unattested standard registration is ACCEPTED (ADR-0061 decision 4 — a grade, not a
//!     gate; spec §5.11).
//!
//! # Why the floor is the thing under test, and not the Rust builders
//!
//! `cairn_event::registration::RegistrationAssertion` deliberately PERMITS illegal states:
//! `class: Standard` with `search: None`, and `class: Unidentified` with `search: Some(..)`
//! both compile. That is the twelfth founding principle applied — the safety invariant
//! belongs in the database, unbypassably, so a bespoke client talking raw SQL cannot admit
//! a malformed registration either. `db/045` is the only thing standing between those
//! illegal states and the permanent record, so every rule is tested in BOTH directions: the
//! shape that must be accepted AND the shape that must be refused.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` (the shared-DB + TRUNCATE pattern every suite in this directory
//! uses). `test_serial_guard` is taken BEFORE `connect_and_load_schema`, in that order — a
//! loader replaying `db/*.sql` concurrently with another suite's TRUNCATE is the one
//! interleaving the guard exists to prevent.
mod common;

use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{generate_key, sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_node::db;
use common::{cs, db_msg, setup, submit_signed, EventSpec};
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

/// The projection this suite writes to. Truncated per test by `common::setup`, behind that
/// helper's `to_regclass` guard, so the suite still runs on a database migrated only as far
/// as the core clinical tables (where it self-skips rather than erroring on a missing table).
const EXTRA_TABLES: [&str; 1] = ["patient_registration"];

/// Submit one registration payload through the real `submit_event` door.
///
/// Goes through `common::submit_signed`, so the contributor set is the shared default —
/// a single `recorded` entry naming the AGENT that signed. That is deliberately the
/// no-human-author shape (see `a_standard_registration_with_no_human_author_is_accepted`):
/// most tests here should exercise the floor, not the authorship plane.
///
/// Returns the raw submit result rather than unwrapping, because most of this file asserts
/// a REFUSAL and matches on `db_msg` of the error.
async fn submit_registration(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    payload: Value,
    twin: Option<String>,
) -> Result<u64, tokio_postgres::Error> {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: REGISTRATION_EVENT_TYPE,
            schema_version: REGISTRATION_SCHEMA_VERSION,
            payload,
            plaintext_twin: twin,
            wall,
        },
    )
    .await
}

/// Submit a registration built by the Task-1 TYPED builders.
///
/// This is the seam test in miniature: the wire shape `registration_assertion_body`
/// produces must be exactly the shape the `db/045` floor accepts. A drift between the two
/// (a renamed key, a moved nesting level) shows up here rather than in production.
async fn register(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    a: &RegistrationAssertion<'_>,
) -> Result<u64, tokio_postgres::Error> {
    submit_registration(
        c,
        sk,
        kid,
        patient,
        wall,
        registration_assertion_body(a),
        Some(render_registration_twin(a)),
    )
    .await
}

/// A well-formed STANDARD registration payload as raw JSON.
///
/// Raw rather than builder-produced on purpose: every refusal test below takes this and
/// mutates exactly ONE field, so a failure names the field the floor refused rather than
/// leaving open which of several differences did it. The typed builder cannot express most
/// of these mutations at all — which is the point of having a floor beneath it.
fn standard_payload() -> Value {
    json!({
        "class": "standard",
        "search": {
            "query": {
                "name_tokens": ["smith"],
                "birth_date": "1980-01-01",
                "identifiers": []
            },
            "displayed": [],
            "incomplete": false
        }
    })
}

/// A twin that is valid for every payload in this file. The twin requirement (§3.13) is
/// tested on its own in `a_missing_twin_is_refused`; everywhere else it must not be the
/// reason a submit fails, or a floor test would pass for the wrong reason.
fn good_twin() -> Option<String> {
    Some("Patient registered (standard registration)".to_string())
}

/// Assert the floor REFUSED this submit with a message containing `needle`, AND that
/// nothing reached either the log or the projection.
///
/// The "nothing was written" half matters as much as the refusal: a check that raised
/// AFTER the `event_log` INSERT would still error, but would have already admitted the
/// malformed registration permanently. `patient_id::text = $1` (a string parameter) is the
/// project's binding convention — `cairn-node` does not enable tokio-postgres's
/// `with-uuid-1` feature, so a `Uuid` has no `ToSql`.
async fn assert_refused_and_empty(
    c: &Client,
    result: Result<u64, tokio_postgres::Error>,
    p: Uuid,
    needle: &str,
    label: &str,
) {
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("{label}: the floor accepted a registration it must refuse"),
    };
    let msg = db_msg(&err);
    assert!(
        msg.contains(needle),
        "{label}: expected a refusal containing {needle:?}, got: {msg}"
    );
    let p_str = p.to_string();
    let logged: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(logged, 0, "{label}: nothing may be appended to event_log");
    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_registration WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(projected, 0, "{label}: nothing may be projected");
}

#[tokio::test]
async fn standard_registration_with_a_well_formed_search_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    let displayed = [Uuid::now_v7(), Uuid::now_v7()];
    let tokens = vec!["smith".to_string(), "john".to_string()];
    let identifiers: Vec<(String, String)> = vec![("MRN".into(), "12345".into())];
    let a = RegistrationAssertion {
        class: RegistrationClass::Standard,
        basis: None,
        search: Some(SearchAttestationInput {
            terms: SearchTerms {
                name_tokens: &tokens,
                birth_date: Some("1980-01-01"),
                identifiers: &identifiers,
            },
            displayed: &displayed,
            incomplete: false,
        }),
    };
    register(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("a well-formed standard registration must be accepted");

    // The projection derives displayed_count from the array itself (there is no such field
    // on the wire — two representations of one number is a lie waiting to happen).
    let p_str = p.to_string();
    let row = c
        .query_one(
            "SELECT class, basis, displayed_count, search_incomplete \
             FROM patient_registration WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap();
    let class: String = row.get(0);
    let basis: Option<String> = row.get(1);
    let displayed_count: i32 = row.get(2);
    let incomplete: Option<bool> = row.get(3);
    assert_eq!(class, "standard");
    assert_eq!(basis, None, "a standard registration carries no basis");
    assert_eq!(
        displayed_count, 2,
        "displayed_count is derived from the array"
    );
    assert_eq!(incomplete, Some(false));
}

#[tokio::test]
async fn standard_registration_without_a_search_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // The whole point of §5.8: a standard create act must record the search that preceded
    // it. Without the search there is nothing to answer "was the duplicate on screen?".
    let p = Uuid::now_v7();
    let r = submit_registration(
        &c,
        &sk,
        &kid,
        p,
        1,
        json!({"class": "standard"}),
        good_twin(),
    )
    .await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "standard registration must carry its search",
        "standard-without-search",
    )
    .await;
}

#[tokio::test]
async fn unidentified_registration_carrying_a_search_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // The trap this test exists for: absence must be STRUCTURAL. An implementation that
    // merely made `search` optional would pass every other test in this file and let a
    // John Doe claim a search nobody could have run — an unconscious patient with no name
    // and no identifier cannot be searched for, so a search attestation on that chart is a
    // precise untruth (principle 4).
    let mut payload = standard_payload();
    payload["class"] = json!("unidentified");
    payload["basis"] = json!("unconscious ED arrival, no ID");
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "a search attestation the registrar could not have made",
        "unidentified-with-search",
    )
    .await;
}

#[tokio::test]
async fn a_non_standard_registration_without_a_basis_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Review finding I1: rule 2b shipped with no test that drove it. Deleting the whole
    // `basis` IF block from db/045 left every other test in this file green, because
    // `unidentified_registration_carrying_a_search_is_refused` supplies a VALID basis and
    // is refused for a different reason entirely.
    //
    // Why the rule matters clinically: for `standard`, the class IS the explanation and a
    // mandatory free-text box would be a required field satisfiable only by fabrication
    // (principle 4). For the non-standard classes the reverse holds — "unconscious ED
    // arrival, no ID" is the ONLY record of why this chart was born outside the normal
    // path, and a John Doe chart with no stated reason is unauditable six months later.
    //
    // All three absent/blank/wrong-type shapes are covered: the rule is
    // `jsonb_typeof(basis) IS DISTINCT FROM 'string' OR trim(basis) = ''`, and a test that
    // only omitted the key would leave the blank-string and non-string arms unproven.
    for (label, basis) in [
        ("basis-absent", None),
        ("basis-blank", Some(json!("   "))),
        ("basis-non-string", Some(json!(42))),
    ] {
        let mut payload = json!({"class": "unidentified"});
        if let Some(b) = basis {
            payload["basis"] = b;
        }
        let p = Uuid::now_v7();
        let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
        assert_refused_and_empty(&c, r, p, "non-standard registration states why", label).await;
    }
}

#[tokio::test]
async fn an_unknown_class_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // §5.3's three classes are a CLOSED set, and the class is the discriminant every other
    // rule in the floor keys off. A fourth class admitted here would be a registration no
    // rule applies to at all.
    let mut payload = standard_payload();
    payload["class"] = json!("temporary");
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
    assert_refused_and_empty(&c, r, p, "unknown registration class", "unknown-class").await;
}

#[tokio::test]
async fn a_non_uuid_in_displayed_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // `displayed` NAMES the candidates that were on screen, so a later reviewer can ask
    // "was this duplicate among them?". An element that is not a patient id answers
    // nothing and would silently inflate the count.
    let mut payload = standard_payload();
    payload["search"]["displayed"] = json!(["not-a-uuid"]);
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
    assert_refused_and_empty(&c, r, p, "candidate list malformed", "displayed-non-uuid").await;
}

#[tokio::test]
async fn an_absent_or_wrong_typed_displayed_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Second whole-branch review: only a non-uuid ELEMENT was driven; the key itself being
    // absent (or the wrong type) never was. The absent-key case is the three-valued-logic
    // trap this floor has already fallen to once (the `{}` empty-query fail-open): rule 2f
    // is `jsonb_typeof(v_displayed) IS DISTINCT FROM 'array'`, and a regression to the
    // "obvious" `<>` spelling makes `jsonb_typeof(<absent>)` — SQL NULL — compare to NULL,
    // the branch not taken, the element-check EXISTS over NULL yield zero rows, and the
    // projection COALESCE the count to 0: a standard registration with NO candidate list at
    // all admitted permanently, indistinguishable from a diligent search that found
    // nothing. This test is what makes that one-token regression fail loudly.
    for (label, mutate) in [
        ("displayed-absent", None),
        ("displayed-string", Some(json!("abc"))),
        ("displayed-number", Some(json!(3))),
    ] {
        let mut payload = standard_payload();
        match mutate {
            None => {
                payload["search"]
                    .as_object_mut()
                    .expect("search is an object")
                    .remove("displayed");
            }
            Some(v) => payload["search"]["displayed"] = v,
        }
        let p = Uuid::now_v7();
        let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
        assert_refused_and_empty(&c, r, p, "candidate list malformed", label).await;
    }
}

#[tokio::test]
async fn a_missing_incomplete_flag_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // ADR-0060 decision 2: completeness must be STATED, never assumed by its absence.
    // Defaulting a missing flag to `false` would let a node that knew it could not show
    // everything present the search as exhaustive.
    let mut payload = standard_payload();
    payload["search"]
        .as_object_mut()
        .expect("search is an object")
        .remove("incomplete");
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "completeness must be stated, not assumed",
        "incomplete-missing",
    )
    .await;
}

#[tokio::test]
async fn a_wrong_typed_incomplete_flag_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Second whole-branch review: rule 2g's whole stated reason for `jsonb_typeof ...
    // 'boolean'` (rather than a `::boolean` cast) is refusing Postgres's permissive
    // spellings — `"true"`, `1`, `"yes"` would all cast clean and read as stated-complete.
    // Only ABSENCE was ever driven; these are the wrong-TYPE arms. Without this test, a
    // regression to a presence-only check leaves the `"true"` shape to fail later at the
    // projection's raw cast — an error naming no field, at the door that must never wedge.
    for (label, bad) in [
        ("incomplete-string-true", json!("true")),
        ("incomplete-number-one", json!(1)),
    ] {
        let mut payload = standard_payload();
        payload["search"]["incomplete"] = bad;
        let p = Uuid::now_v7();
        let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
        assert_refused_and_empty(&c, r, p, "a JSON boolean", label).await;
    }
}

#[tokio::test]
async fn an_explicit_search_null_on_a_non_standard_registration_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Rule 2c is a KEY-PRESENCE test (`p ? 'search'`), not a null test, and both db/045's
    // comment and john_doe.rs's module doc lean on that distinction — an explicit
    // `"search": null` is still an author asserting something about a search, and this is
    // the one door that can refuse it. Until this test, no submission ever drove the null
    // shape, so a regression to a typeof-based check (which reads absent and null alike)
    // would have admitted it with the whole suite green.
    let mut payload = json!({
        "class": "unidentified",
        "basis": "unconscious ED arrival, no ID"
    });
    payload["search"] = json!(null);
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, payload, good_twin()).await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "a search attestation the registrar could not have made",
        "unidentified-with-null-search",
    )
    .await;
}

#[tokio::test]
async fn an_empty_query_object_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // A search with no terms cannot have found anything, so "0 candidates displayed" from
    // it is not evidence of anything — it is an attestation with nothing behind it.
    //
    // The `{}` case is the one that caught a real fail-OPEN defect while this slice was
    // being written: `jsonb_typeof(<absent key>)` is SQL NULL, so the floor's term test
    // evaluated to NULL rather than FALSE and `IF NOT NULL` was simply not taken. Keep
    // BOTH cases — the all-keys-absent object and the present-but-blank one — because they
    // exercise different arms of that expression and only the first has ever failed open.
    let mut absent_keys = standard_payload();
    absent_keys["search"]["query"] = json!({});
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, absent_keys, good_twin()).await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "a search with no terms is not a search",
        "empty-query",
    )
    .await;

    let mut blank_terms = standard_payload();
    blank_terms["search"]["query"] = json!({
        "name_tokens": ["   "],
        "birth_date": "",
        "identifiers": [{"system": "MRN", "value": ""}]
    });
    let p2 = Uuid::now_v7();
    let r2 = submit_registration(&c, &sk, &kid, p2, 1, blank_terms, good_twin()).await;
    assert_refused_and_empty(
        &c,
        r2,
        p2,
        "a search with no terms is not a search",
        "blank-terms-query",
    )
    .await;
}

#[tokio::test]
async fn an_empty_displayed_array_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // The NORMAL case for a genuinely new patient. This test exists so nobody later
    // "tightens" the array into a non-empty requirement and makes registering the first
    // patient on a fresh node impossible.
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1, standard_payload(), good_twin())
        .await
        .expect("an empty candidate list is a valid search, not a malformed one");

    let p_str = p.to_string();
    let displayed_count: i32 = c
        .query_one(
            "SELECT displayed_count FROM patient_registration WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(displayed_count, 0, "the search ran and found nothing");
}

/// Read back the whole projected row for `p`, as the tuple the non-standard accept tests
/// assert on: `(class, basis, displayed_count, search_incomplete)`.
///
/// `search_incomplete` is deliberately an `Option<bool>`: NULL is a THIRD value here, not a
/// missing one, and collapsing it to `false` in the reader would destroy the very
/// distinction the tests below exist to pin.
async fn projected_row(c: &Client, p: Uuid) -> (String, Option<String>, i32, Option<bool>) {
    let row = c
        .query_one(
            "SELECT class, basis, displayed_count, search_incomplete \
             FROM patient_registration WHERE patient_id::text = $1",
            &[&p.to_string()],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

#[tokio::test]
async fn a_valid_unidentified_registration_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Review finding I2: before this test the non-standard ACCEPT path was entirely
    // unproven — every non-standard test asserted a refusal. Three things ride on it:
    //
    //   1. This is the §5.4 John Doe birth act: the exact path the whole search-absence
    //      rule exists to protect. A floor that refused it would make an unconscious
    //      patient unregistrable, which is the failure the rule is meant to prevent.
    //   2. It executes the non-standard PROJECTION write for the first time —
    //      `COALESCE(jsonb_array_length(...), 0)` with no search, and the NULL-propagating
    //      `(p -> 'search' -> 'incomplete')::boolean` cast. Both were reasoned about and
    //      neither had ever run.
    //   3. Built through the TASK-1 BUILDER, so it doubles as a seam test: it proves
    //      `RegistrationClass::Unidentified.as_str()` is a member of db/045's closed set.
    //      A one-character divergence between the Rust enum and the SQL `NOT IN (...)`
    //      list would otherwise ship silently.
    let p = Uuid::now_v7();
    let a = RegistrationAssertion {
        class: RegistrationClass::Unidentified,
        basis: Some("unconscious ED arrival, no ID"),
        search: None,
    };
    register(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("a §5.4 John Doe registration must be accepted — there is nothing to search with");

    let (class, basis, displayed_count, incomplete) = projected_row(&c, p).await;
    assert_eq!(class, "unidentified");
    assert_eq!(basis.as_deref(), Some("unconscious ED arrival, no ID"));
    // THIS PAIR IS THE INVARIANT, and neither half means much alone. `displayed_count = 0`
    // is ambiguous on its own — it reads identically for "a search ran and found nothing"
    // and for "no search ran at all". `search_incomplete IS NULL` is what disambiguates
    // it: a standard registration always stores a boolean there (the floor requires the
    // flag), so NULL can ONLY mean no search ran. Assert them together, or a future change
    // that defaulted the flag to FALSE would make the two cases indistinguishable in the
    // projection while every other test stayed green.
    assert_eq!(
        displayed_count, 0,
        "no search ran, so no candidates were displayed"
    );
    assert_eq!(
        incomplete, None,
        "search_incomplete must be NULL — the honest 'not applicable', and the only thing \
         that tells a reader displayed_count = 0 here means 'no search ran' rather than \
         'the search found nothing'"
    );
}

#[tokio::test]
async fn a_valid_pseudonymous_registration_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // The third member of §5.3's closed set, and the ONLY test that exercises it anywhere:
    // without this, `RegistrationClass::Pseudonymous.as_str()` and db/045's `'pseudonymous'`
    // literal were never once compared at runtime. §5.6 legally-sanctioned anonymous or
    // protective care is rare, which is exactly why a silent divergence here would not be
    // found by use — it would be found by a patient under protection failing to register.
    //
    // Also through the Task-1 builder, for the same seam reason as the test above.
    let p = Uuid::now_v7();
    let a = RegistrationAssertion {
        class: RegistrationClass::Pseudonymous,
        basis: Some("court-ordered protective care"),
        search: None,
    };
    register(&c, &sk, &kid, p, 1, &a)
        .await
        .expect("a §5.6 pseudonymous registration must be accepted");

    let (class, basis, displayed_count, incomplete) = projected_row(&c, p).await;
    assert_eq!(class, "pseudonymous");
    assert_eq!(basis.as_deref(), Some("court-ordered protective care"));
    assert_eq!(displayed_count, 0);
    assert_eq!(incomplete, None);
}

#[tokio::test]
async fn a_standard_registration_with_no_human_author_is_accepted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // `setup` enrolls an AGENT signer, and `submit_signed`'s contributor set is a single
    // `recorded` entry naming it — a device-recorded registration with NO human author and
    // no attestation token. Exactly the 03:00 shape below.
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1, standard_payload(), good_twin())
        .await
        .expect(
            "SPEC §2.6 — DO NOT \"FIX\" THIS INTO A REFUSAL. Authorship confidence is a \
             grade, not a gate (§5.11). Gating here would block care documentation at 03:00 \
             when a clerk's key is not unlocked, push named patients through the John Doe \
             path, and produce NO forensic record in the case it fires.",
        );

    // Review finding M4: the acceptance above is necessary but not sufficient. On its own
    // this test was mechanically identical to `an_empty_displayed_array_is_accepted` — it
    // would fail if someone added a gate, but it asserted NOTHING about its own subject, so
    // it could not catch the outcome silently changing in any other way. The reads below
    // pin what "no human author" actually LANDED AS, so the test fails if the shape of an
    // unattested registration changes, not only if submission starts being refused.
    let p_str = p.to_string();
    let row = c
        .query_one(
            "SELECT
                 -- 1. No human vouched: the door stored no verified attester.
                 el.attester_key IS NULL,
                 -- 2. No contributor claims a responsibility-BEARING role. Checked against
                 --    the DB's own ratified vocabulary (contributor_role.bears) rather than
                 --    a hard-coded role list, so ratifying a new bearing role cannot leave
                 --    this assertion silently behind. `bearing:` prefix included for the
                 --    same reason cairn_authorship_bound carries that arm.
                 NOT EXISTS (
                     SELECT 1 FROM jsonb_array_elements(el.contributors) AS e
                     WHERE coalesce(
                               (SELECT r.bears FROM contributor_role r WHERE r.role = e ->> 'role'),
                               (e ->> 'role') LIKE 'bearing:%')),
                 -- 3. ...and none claims a responsibility object either.
                 NOT EXISTS (
                     SELECT 1 FROM jsonb_array_elements(el.contributors) AS e
                     WHERE e ? 'responsibility'),
                 -- 4. The signer is a NON-human actor. This is the fact an authorship gate
                 --    would have keyed on, so naming it here is what makes the §2.6
                 --    decision testable rather than merely asserted in a comment.
                 EXISTS (
                     SELECT 1 FROM actor_current ac
                     WHERE ac.signing_key_id = el.signer_key_id AND ac.kind <> 'human')
             FROM event_log el WHERE el.patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap();
    let (no_attester, no_bearing_role, no_responsibility, signer_is_not_human): (
        bool,
        bool,
        bool,
        bool,
    ) = (row.get(0), row.get(1), row.get(2), row.get(3));
    assert!(
        no_attester && no_bearing_role && no_responsibility && signer_is_not_human,
        "this registration must land as the genuinely UNATTESTED, device-signed case \
         (attester {no_attester}, no-bearing-role {no_bearing_role}, no-responsibility \
         {no_responsibility}, signer-not-human {signer_is_not_human}) — otherwise the test \
         is not exercising §2.6's subject at all and its acceptance proves nothing"
    );

    // And the chart is fully born despite having no human author — the positive half of
    // "a grade, not a gate": the record is usable, not quarantined.
    let (class, _basis, displayed_count, incomplete) = projected_row(&c, p).await;
    assert_eq!(class, "standard");
    assert_eq!(displayed_count, 0);
    assert_eq!(
        incomplete,
        Some(false),
        "a standard registration always states completeness, unattested or not"
    );
}

#[tokio::test]
async fn a_registration_naming_a_registrar_who_did_not_sign_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Proves ADR-0061 decision 4's "unforgeable for free" claim rather than assuming it: the refusal
    // comes from db/005's UNCONDITIONAL cairn_authorship_bound (step 4b), with NO rule added
    // by db/045. That is why the previous test can safely accept an unattested registration
    // — an unattested one claims no author at all, whereas naming a registrar who neither
    // signed nor attested is a forgery, and it is refused whatever db/045 does.
    //
    // The role must be a BEARING one ("authored"); a contributory role (`recorded`) is
    // exempt from the binding by design — a device may record for someone without signing.
    // The named registrar is enrolled as a real human actor so the refusal cannot be
    // mistaken for "unknown actor": they exist, they simply did not authenticate here.
    let (_registrar_sk, registrar_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"registrar\"}', $1)",
        &[&registrar_kid],
    )
    .await
    .unwrap();

    let p = Uuid::now_v7();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: REGISTRATION_EVENT_TYPE.into(),
        schema_version: REGISTRATION_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: json!([{"actor_id": registrar_kid, "role": "authored"}]),
        payload: standard_payload(),
        attachments: vec![],
        plaintext_twin: good_twin(),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, &sk).unwrap();
    let r = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    assert_refused_and_empty(&c, r, p, "forged authorship refused", "named-non-signer").await;
}

#[tokio::test]
async fn a_missing_twin_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // §3.13 legibility across time: a registration must stay readable to someone with no
    // schema at all. The registry row's twin_required_msg is what turns the honest-degrade
    // skeleton (ADR-0039) into a hard requirement for this type.
    let p = Uuid::now_v7();
    let r = submit_registration(&c, &sk, &kid, p, 1, standard_payload(), None).await;
    assert_refused_and_empty(
        &c,
        r,
        p,
        "registration requires a non-empty authored twin",
        "twin-missing",
    )
    .await;
}

#[tokio::test]
async fn the_projection_keeps_every_registration_and_the_view_picks_the_earliest() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Retained set + earliest-wins VIEW: registration is a BIRTH act, so the winner is the
    // earliest by (hlc_wall, hlc_counter, node_origin COLLATE "C", content_address), not
    // the latest as every standing-state overlay uses. Submitted LATEST FIRST so a view
    // that merely kept the first-applied row would pick the wrong one.
    let p = Uuid::now_v7();

    let mut later = standard_payload();
    later["search"]["displayed"] = json!([]);
    submit_registration(&c, &sk, &kid, p, 5, later, good_twin())
        .await
        .expect("the later registration is accepted");

    let mut earlier = standard_payload();
    earlier["search"]["displayed"] = json!([Uuid::now_v7().to_string()]);
    submit_registration(&c, &sk, &kid, p, 2, earlier, good_twin())
        .await
        .expect("the earlier registration is accepted");

    let p_str = p.to_string();
    let retained: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_registration WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        retained, 2,
        "every registration event keeps its own row — the PK includes the content address"
    );

    let row = c
        .query_one(
            "SELECT registered_hlc_wall, displayed_count \
             FROM patient_registration_current WHERE patient_id::text = $1",
            &[&p_str],
        )
        .await
        .unwrap();
    let wall: i64 = row.get(0);
    let displayed_count: i32 = row.get(1);
    assert_eq!(
        wall, 2,
        "the EARLIEST registration is the chart's birth act"
    );
    assert_eq!(
        displayed_count, 1,
        "the winning row's own columns come with it, not the later event's"
    );
}
