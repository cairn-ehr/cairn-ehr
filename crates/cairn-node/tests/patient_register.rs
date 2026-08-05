//! Integration coverage for §5.3/§5.8 STANDARD patient registration:
//! `patient::register::register_patient` composes the search the clerk actually ran
//! (`SearchQuery`) and what they actually saw (`CandidateList`) into ONE
//! `identity.registration.asserted` event through the real `submit_event` door. Real
//! Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`.
//! Mirrors `john_doe.rs` (the sibling registration orchestrator this composes onto) and
//! `patient_registration.rs` (the db/045 floor this exercises from the Rust side).
//!
//! The load-bearing test in this file is
//! `the_attestation_round_trips_from_the_displayed_list_to_the_stored_body`: it is the ONLY
//! thing that stops `cairn-event` (the wire shape, taking primitives) and
//! `cairn-patient-search` (the read model) drifting apart. A drift there means a registration
//! swearing to candidates the clerk never actually saw on screen — see this crate's
//! `patient::register` module doc for the full argument.
//!
//! `register_patient`'s Task-8b addition (#350 — it must ALSO assert the typed name/dob so a
//! registered chart is actually findable) is covered separately in
//! `patient_register_demographics.rs`, split out purely to keep both files under the house
//! 500-line limit; every call to `register_patient` in THIS file passes `None` for the new
//! `name` parameter, since none of these tests are about the demographic assertions.
mod common;

use cairn_node::db;
use cairn_node::patient::register::register_patient;
use cairn_patient_search::{Candidate, CandidateList, SearchQuery, TrustState};
use common::{cs, setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// The projection this suite writes to, beyond `common::setup`'s clinical core.
const EXTRA_TABLES: [&str; 1] = ["patient_registration"];

/// One minimal candidate for a `CandidateList` — only `patient_id` matters to the
/// attestation; the display fields are filler so the struct is easy to construct per test.
fn candidate(id: Uuid) -> Candidate {
    Candidate {
        patient_id: id,
        display_name: "Some One".into(),
        age: None,
        trust: TrustState::Confirmed,
        last_activity: None,
        locale: None,
        photo_ref: None,
    }
}

/// Read back the projected `patient_registration` row for `p`, as
/// `(class, basis, displayed_count, search_incomplete)` — the same tuple
/// `patient_registration.rs`'s `projected_row` reads, kept local here because this suite
/// only ever exercises the Standard/accepted path (no need to import test-only helpers
/// across a suite boundary Cargo does not share).
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

/// Read back the RAW `search.displayed` array from the stored event body, preserving order.
///
/// Goes through `(... )::text` + `serde_json::from_str` rather than binding jsonb directly:
/// `cairn-node` does not enable tokio-postgres's `with-serde_json-1` feature (the project-wide
/// convention — see `patient/search.rs`'s "JSONB BINDING" module note), so a jsonb value must
/// be cast to its text representation and parsed on the Rust side instead of bound natively.
/// This is deliberately a query against `event_log.body` and NOT the `patient_registration`
/// projection: the projection only stores `displayed_count` (a derived integer, by design —
/// see db/045's own comment on why two representations of one number is a lie waiting to
/// happen), so the only place the actual candidate LIST can be read back from is the signed
/// event body itself. That is exactly what the round-trip test needs to pin.
async fn stored_displayed(c: &Client, p: Uuid) -> Vec<Uuid> {
    let row = c
        .query_one(
            "SELECT (body -> 'search' -> 'displayed')::text AS displayed \
             FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&p.to_string()],
        )
        .await
        .unwrap();
    let raw: String = row.get(0);
    let ids: Vec<String> = serde_json::from_str(&raw).expect("displayed is a JSON array");
    ids.iter()
        .map(|s| Uuid::parse_str(s).expect("each element is a uuid string"))
        .collect()
}

#[tokio::test]
async fn registering_mints_a_chart_and_records_what_was_displayed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Two near-matches genuinely on screen when the clerk chose to create anyway — the
    // ordinary §5.8 shape, not the empty-list "genuinely new patient" case (that is its own
    // test in patient_registration.rs's floor suite).
    let displayed_ids = [Uuid::now_v7(), Uuid::now_v7()];
    let list = CandidateList {
        candidates: displayed_ids.iter().map(|id| candidate(*id)).collect(),
        incomplete: false,
        incomplete_reason: None,
    };
    let query = SearchQuery::new("smith", Some("1980-01-01"), &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &list)
        .await
        .expect("a well-formed standard registration must be accepted");

    let (class, basis, displayed_count, incomplete) = projected_row(&c, pid).await;
    assert_eq!(class, "standard");
    assert_eq!(basis, None, "a standard registration carries no basis");
    assert_eq!(
        displayed_count, 2,
        "displayed_count is derived from the candidate list actually shown"
    );
    assert_eq!(incomplete, Some(false));
}

#[tokio::test]
async fn the_attestation_round_trips_from_the_displayed_list_to_the_stored_body() {
    // GUARDS THE CROSS-CRATE SEAM (ADR-0061 — the one conversion site): cairn-event takes primitives and
    // cairn-patient-search owns the read model, so nothing but this test stops the two
    // drifting. Build a CandidateList -> SearchAttestation -> body -> submit -> read the
    // stored body back -> assert the displayed set is IDENTICAL, in order.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    // Three distinct, freshly-minted UUIDv7s. UUIDv7 sorts close to creation order, so
    // generating them in this exact sequence and then asserting the SAME sequence comes back
    // is a genuine order check, not an accident of a value that happens to equal its own
    // sorted form.
    let ids = [Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7()];
    let list = CandidateList {
        candidates: ids.iter().map(|id| candidate(*id)).collect(),
        incomplete: false,
        incomplete_reason: None,
    };
    let query = SearchQuery::new("jones", None, &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &list)
        .await
        .expect("registration accepted");

    let stored = stored_displayed(&c, pid).await;
    assert_eq!(
        stored,
        ids.to_vec(),
        "the stored displayed set must be IDENTICAL to what the clerk saw, in the SAME order"
    );
}

#[tokio::test]
async fn a_search_the_node_knew_was_partial_is_attested_as_incomplete() {
    // The list's incompleteness must survive all the way into the stored event. A
    // registration must never swear to an exhaustive search over a list known to be partial
    // (ADR-0060 decision 2).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let list = CandidateList {
        candidates: vec![candidate(Uuid::now_v7())],
        incomplete: true,
        incomplete_reason: Some("one chart unreadable".into()),
    };
    let query = SearchQuery::new("baker", None, &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &list)
        .await
        .expect("registration accepted even though the search was partial");

    // Both the projection column (search_incomplete) and the raw stored body must agree —
    // the projection is a read convenience derived from the same body, never a second
    // source of truth (db/045's own comment).
    let (_class, _basis, _displayed_count, incomplete) = projected_row(&c, pid).await;
    assert_eq!(
        incomplete,
        Some(true),
        "the projection must carry the list's own incompleteness through, not silently default to complete"
    );

    let row = c
        .query_one(
            "SELECT (body -> 'search' -> 'incomplete')::text \
             FROM event_log \
             WHERE patient_id::text = $1 AND event_type = 'identity.registration.asserted'",
            &[&pid.to_string()],
        )
        .await
        .unwrap();
    let raw: String = row.get(0);
    assert_eq!(
        raw, "true",
        "the stored body itself must state incomplete=true"
    );
}

#[tokio::test]
async fn registering_with_no_attester_key_succeeds() {
    // SPEC §2.6 — a grade, not a gate. See the identical guard in patient_registration.rs
    // (`a_standard_registration_with_no_human_author_is_accepted`). `common::setup` enrolls
    // only an AGENT signer, and `register_patient`'s contributor set is a single `recorded`
    // entry naming it — a device-recorded registration with NO human author and no
    // attestation token. Exactly the 03:00 shape the brief calls out: this MUST succeed, or
    // care documentation is blocked at the worst possible moment.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;

    let list = CandidateList {
        candidates: vec![],
        incomplete: false,
        incomplete_reason: None,
    };
    let query = SearchQuery::new("nobody-yet", None, &[]);

    let pid = register_patient(&mut c, &sk, &kid, "n", None, &query, &list)
        .await
        .expect(
            "SPEC §2.6 — DO NOT \"FIX\" THIS INTO A REFUSAL. Authorship confidence is a \
             grade, not a gate. Gating here would block care documentation at 03:00 when a \
             clerk's key is not unlocked.",
        );

    // Pin WHAT "no human author" landed as, not merely that submission succeeded — mirrors
    // patient_registration.rs's review-finding-M4 discipline: acceptance alone cannot tell
    // "no gate was ever added" apart from "the outcome silently changed shape".
    let p_str = pid.to_string();
    let row = c
        .query_one(
            "SELECT
                 -- 1. No human vouched: the door stored no verified attester.
                 el.attester_key IS NULL,
                 -- 2. No contributor claims a responsibility-BEARING role, checked against
                 --    the DB's own ratified vocabulary rather than a hard-coded role list.
                 NOT EXISTS (
                     SELECT 1 FROM jsonb_array_elements(el.contributors) AS e
                     WHERE coalesce(
                               (SELECT r.bears FROM contributor_role r WHERE r.role = e ->> 'role'),
                               (e ->> 'role') LIKE 'bearing:%')),
                 -- 3. ...and none claims a responsibility object either.
                 NOT EXISTS (
                     SELECT 1 FROM jsonb_array_elements(el.contributors) AS e
                     WHERE e ? 'responsibility'),
                 -- 4. The signer is a NON-human actor — the fact an authorship gate would
                 --    have keyed on, named here so the §2.6 decision is testable.
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

    // And the chart is fully born despite having no human author.
    let (class, _basis, displayed_count, incomplete) = projected_row(&c, pid).await;
    assert_eq!(class, "standard");
    assert_eq!(displayed_count, 0);
    assert_eq!(incomplete, Some(false));
}
