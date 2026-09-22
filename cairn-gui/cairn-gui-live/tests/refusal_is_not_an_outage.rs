//! DB-gated: the in-DB floor's verdicts arrive as VERDICTS (#648).
//!
//! `data_error_from` decides refusal-vs-outage from the SQLSTATE, which `sqlstate_of` digs out
//! of the `anyhow` chain a `cairn-node` orchestrator returns. **Nothing in the type system
//! keeps that chain intact.** A call site anywhere on this path that renders its database
//! error into a string — `anyhow!("…: {}", legible_db_error(&e))` is the idiom that does it —
//! destroys the `tokio_postgres::Error`, and from then on every floor refusal reads as an
//! outage: a retry button on a verdict, offered to a clerk who will press it.
//!
//! Only a real floor can catch that. `tokio_postgres::Error` has no public constructor and a
//! `DbError` cannot be built by hand at all, so the unit tests in `src/error.rs` can pin the
//! RULE but not the EXTRACTION — a mutation that makes `sqlstate_of` read only the outermost
//! error survives every one of them. This file is what kills it.
//!
//! **So this file must never be relaxed into asserting merely that the call failed.** An
//! `is_err()` assertion here would pass under the exact defect the file exists to detect.
mod common;

use cairn_gui_data::port::{DataError, PatientRegistration};
use cairn_gui_funnel::{bound_for_prompt, AttestedSearch, Restored, TokenStore};
use cairn_gui_live::LiveData;
use cairn_patient_search::{CandidateList, SearchQuery};

const ORIGIN: &str = "n";

/// Mint an attested search over `query`, displaying nothing — the shape a registration takes
/// when the step-3 search found no candidates, which is the common case and the one every
/// refusal probe below wants.
fn attest(store: &mut TokenStore, query: SearchQuery) -> AttestedSearch {
    let token = store
        .record(
            query,
            bound_for_prompt(&CandidateList {
                candidates: vec![],
                incomplete: false,
                incomplete_reason: None,
            }),
        )
        .expect("a non-empty query mints a token");
    store.take(token).expect("the token is redeemable")
}

/// Put a failed registration's attestation back, and insist it actually landed.
///
/// `restore` is `#[must_use]` for a reason: its answer is whether the search went back or was
/// dropped as superseded. Every caller here failed a registration while nothing else touched
/// the store, so `Kept` is the only correct answer — and a silent `SupersededAndDropped` would
/// mean the clerk's form quietly lost the search it is about to re-submit. Asserting it is
/// what makes "the form keeps its values" a tested claim rather than a hopeful one.
fn restore_and_expect_kept(store: &mut TokenStore, attested: AttestedSearch) {
    assert_eq!(
        store.restore(attested),
        Restored::Kept,
        "nothing superseded this search, so it must go back on the form"
    );
}

/// How many charts exist. A free function rather than a closure: an async closure taking a
/// borrow cannot name the lifetime its future captures, and spelling that out is not worth a
/// two-line query.
async fn chart_count(c: &tokio_postgres::Client) -> i64 {
    c.query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .expect("count the charts")
        .get(0)
}

/// A signer this node has not enrolled is refused by `db/005`'s door, with a bare
/// `RAISE EXCEPTION` — P0001, the contract `sqlstate_of` reads.
///
/// **Why this probe and not a malformed date of birth**, which was the obvious first choice
/// and is written down here so the next reader does not repeat it: `register_patient`
/// validates the date-of-birth SHAPE in Rust, before any statement reaches Postgres, and
/// returns a plain `anyhow!`. There is no SQLSTATE in that error at all, so it arrives as
/// `Unavailable` — see `a_rust_side_pre_flight_refusal_is_not_yet_told_apart` below, which
/// pins that gap rather than leaving it to be rediscovered.
///
/// An unenrolled signer is the floor's own verdict: deterministic (the same key refuses
/// identically until somebody enrols it), legible (the message names the key), and squarely
/// the clerk's cue to fix the node rather than to click Register again.
#[tokio::test]
async fn a_deterministic_floor_refusal_is_refused_not_unavailable() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    // `setup` enrols ITS key; this port signs with a DIFFERENT one that nothing enrolled, so
    // the door has a real reason to refuse and the refusal is the floor's, not a fixture's.
    let (_enrolled, _kid) = common::setup(&reader).await;
    let stranger: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(17));
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        cairn_event::SigningKey::from_bytes(&stranger),
        ORIGIN.to_string(),
    );

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Refusal Probe", Some("1980-02-03"), &[]),
    );

    let (err, returned) = live
        .register(attested, Some("Refusal Probe"))
        .await
        .expect_err("an unenrolled signer must not be able to create a chart");

    match &err {
        DataError::Refused(text) => assert!(
            text.contains("not an enrolled"),
            "the floor's own message is the only thing that tells the clerk what to change \
             (§9.6), and a refusal that does not name the cause is the same silence one \
             variant over — got: {text}"
        ),
        other => panic!(
            "the floor's verdict arrived as {other:?}. If that is `Unavailable`, the anyhow \
             chain lost its tokio_postgres::Error somewhere on this path and EVERY floor \
             refusal now reads as an outage — find the call site that rendered its error into \
             a string, and see `sqlstate_of`'s doc in src/error.rs."
        ),
    }

    // The other half of the port's contract, and it holds for a refusal exactly as it does for
    // an outage: the attestation comes BACK, so the form keeps its values and the token store
    // can be settled. `commit` would be a lie here — nothing was created — so `restore` is the
    // only truthful end, and the clerk's next edit will `discard` the doomed search anyway.
    restore_and_expect_kept(&mut store, returned);
}

/// A KNOWN GAP, PINNED RATHER THAN LEFT SILENT — [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651).
///
/// `register_patient` refuses some inputs in Rust, before any statement reaches Postgres: the
/// date-of-birth shape is validated up front precisely so a malformed one refuses the whole
/// call with zero side effects (no HLC tick, no partial chart). Those refusals are every bit
/// as deterministic as the floor's — the same string refuses identically forever — but they
/// carry no SQLSTATE, so `data_error_from` cannot tell them from a dropped connection and
/// reports `Unavailable`.
///
/// The clerk is therefore invited to retry a registration that can never succeed, which is the
/// exact harm #648 describes, one layer above where #648 was looking. Fixing it needs a
/// decision in `cairn-node` (a typed error, or a refusal marker on the `anyhow` chain), not a
/// patch here — this crate has nothing to read. See #651 for both candidate shapes.
///
/// **This test asserts today's WRONG behaviour on purpose, so the gap is visible in the diff
/// and in every run.** When it is fixed, this test fails, and that failure is the good news:
/// flip it to expect `Refused` and delete this paragraph.
#[tokio::test]
async fn a_rust_side_pre_flight_refusal_is_not_yet_told_apart() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(common::connect_for_live(&cs).await, sk, ORIGIN.to_string());

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Shape Probe", Some("1980-13-45-99"), &[]),
    );
    let (err, returned) = live
        .register(attested, Some("Shape Probe"))
        .await
        .expect_err("a malformed date of birth must refuse the whole call");
    restore_and_expect_kept(&mut store, returned);

    let DataError::Unavailable(text) = &err else {
        panic!(
            "this now reports {err:?} rather than `Unavailable` — which is the FIX landing. \
             Change this test to expect `Refused` and remove the gap note in its doc."
        );
    };
    assert!(
        text.contains("not a recognised shape"),
        "the message is still the orchestrator's own, which is the one thing that is right \
         about this arm today — got: {text}"
    );
}

/// THE TEST THAT PINS THE CHAIN WALK, and the reason it had to be written separately.
///
/// `sqlstate_of` walks the WHOLE `anyhow` chain. Nothing above proves that it needs to:
/// `register_patient`'s door refusal arrives with the `tokio_postgres::Error` OUTERMOST, so a
/// mutation replacing `e.chain()` with `e.chain().take(1)` — read only the first layer —
/// passes every other test in this crate. That is a live code path with no coverage, one
/// `.context("registering the patient")` away from being the only thing standing between a
/// floor verdict and a retry button.
///
/// A `DbError` cannot be constructed by hand, so the error is BORROWED FROM THE SERVER: a
/// bare `RAISE EXCEPTION` produces exactly the P0001 the floor produces, and then the test
/// wraps it the way an orchestrator would.
#[tokio::test]
async fn a_sqlstate_is_found_under_context_layers_not_only_at_the_top() {
    use anyhow::Context;

    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;

    // A REAL server error with a real SQLSTATE. `RAISE EXCEPTION` with no `USING ERRCODE` is
    // precisely what every refusal in the in-DB floor is, so this is the floor's own shape.
    let pg = reader
        .batch_execute("DO $$ BEGIN RAISE EXCEPTION 'a borrowed verdict'; END $$;")
        .await
        .expect_err("RAISE EXCEPTION fails");

    // Bare: the shape the register path actually produces today.
    let bare = anyhow::Error::new(pg);
    assert_eq!(
        cairn_gui_live::error::sqlstate_of(&bare).as_deref(),
        Some("P0001"),
        "the SQLSTATE must be readable when the database error is outermost"
    );

    // Buried under two context layers: the shape ANY orchestrator produces the moment someone
    // adds a `.context(…)`, which is the ordinary way to make an error legible.
    let buried = Err::<(), _>(bare)
        .context("submitting the registration act")
        .context("registering the patient")
        .unwrap_err();
    assert_eq!(
        cairn_gui_live::error::sqlstate_of(&buried).as_deref(),
        Some("P0001"),
        "a context layer must not hide the verdict: `sqlstate_of` walks the chain precisely \
         so that adding one stays a legibility improvement rather than silently turning every \
         floor refusal into an outage"
    );
    assert!(
        matches!(
            cairn_gui_live::error::data_error_from(&buried),
            cairn_gui_data::port::DataError::Refused(_)
        ),
        "and the whole mapping must follow it"
    );
}

/// A refused registration must leave NO chart behind.
///
/// `register_patient` derives and validates the date-of-birth precision from its SHAPE before
/// ticking any HLC or authoring the registration act, so a refused call is atomic. A partial
/// chart would be a patient record born without the act that licenses it — the state db/005
/// step 8b exists to make impossible (#345) — and it would be invisible, because the clerk was
/// told the registration failed.
#[tokio::test]
async fn a_refused_registration_creates_no_chart() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(common::connect_for_live(&cs).await, sk, ORIGIN.to_string());

    let before = chart_count(&reader).await;

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Refusal Probe Two", Some("not-a-date"), &[]),
    );
    let (_err, returned) = live
        .register(attested, Some("Refusal Probe Two"))
        .await
        .expect_err("a malformed date of birth refuses");
    restore_and_expect_kept(&mut store, returned);

    assert_eq!(
        before,
        chart_count(&reader).await,
        "a refused registration must leave no chart — a half-born one is worse than none, \
         because the clerk was told it failed"
    );
}

/// An OUTAGE must still arrive as an outage, or the variant is decoration.
///
/// The other direction of the same rule, and it needs a real failure rather than a fabricated
/// one: a closed connection is the commonest outage there is, and it produces an error with no
/// SQLSTATE at all — the `None` arm that must never be read as a verdict.
#[tokio::test]
async fn a_dead_connection_is_an_outage_not_a_refusal() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;

    // A connection whose server-side session is terminated underneath it. The next statement
    // fails with no verdict of any kind, which is exactly what a ward's flaky link looks like.
    let victim = common::connect_for_live(&cs).await;
    let pid: i32 = victim
        .query_one("SELECT pg_backend_pid()", &[])
        .await
        .expect("the victim's backend pid")
        .get(0);
    reader
        .execute("SELECT pg_terminate_backend($1)", &[&pid])
        .await
        .expect("terminate the victim's backend");

    let live = LiveData::new(victim, sk, ORIGIN.to_string());
    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Outage Probe", Some("1975-08-09"), &[]),
    );

    let (err, returned) = live
        .register(attested, Some("Outage Probe"))
        .await
        .expect_err("a dead connection cannot register anyone");
    restore_and_expect_kept(&mut store, returned);

    match &err {
        DataError::Unavailable(_) => {}
        other => panic!(
            "a dead connection decided NOTHING about this registration, and arrived as \
             {other:?}. Reporting it as a refusal tells the clerk to change a form that was \
             never the problem, and not to retry the one thing that would have worked."
        ),
    }
}
