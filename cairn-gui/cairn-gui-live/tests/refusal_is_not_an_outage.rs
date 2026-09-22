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

use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::{bound_for_prompt, AttestedSearch, Restored, TokenStore};
use cairn_gui_live::LiveData;
use cairn_patient_search::SearchQuery;

/// This node's origin id. Distinctive for the reason the sibling suite spells out: `"n"`
/// is what a port that ignored its `Identity` would hardcode.
const ORIGIN: &str = "gui-live-origin";

/// A fixed clock for the two searches in this file, so nothing here depends on the wall clock.
const TODAY: &str = "2026-09-22";

/// Mint an attested search over `query`, displaying nothing — the shape a registration takes
/// when the step-3 search found no candidates, which is the common case and the one every
/// refusal probe below wants.
fn attest(store: &mut TokenStore, query: SearchQuery) -> AttestedSearch {
    let token = store
        .record(query, bound_for_prompt(&common::nothing_found()))
        .expect("a non-empty query mints a token");
    store.take(token).expect("the token is redeemable")
}

/// Put a failed registration's attestation back, and insist it actually landed.
///
/// `restore`'s result is `#[must_use]` (the attribute sits on `Restored`, not on the
/// function) for a reason: its answer is whether the search went back or was
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
/// validates the date-of-birth SHAPE in Rust, before any statement reaches Postgres. There is
/// no SQLSTATE in that error at all, so THIS test would prove nothing about the SQLSTATE walk:
/// since #651 it is classified by the second discriminator instead — see
/// `a_rust_side_pre_flight_refusal_is_refused_not_unavailable` below, which is the arm that
/// proves the marker, while this one is the arm that proves the chain walk.
///
/// An unenrolled signer is the floor's own verdict: deterministic (the same key refuses
/// identically until somebody enrols it), legible (the message names the key), and squarely
/// the clerk's cue to fix the node rather than to click Register again.
///
/// It is also not a hypothetical shape chosen for convenience. `cairn-node patient-register`
/// enrols its key on first use and `LiveData` deliberately does not, so **this is exactly the
/// refusal the reference window will meet on a node where the CLI never registered anyone** —
/// [#654](https://github.com/cairn-ehr/cairn-ehr/issues/654), and `LiveData::new`'s doc has
/// the argument for why the port must not provision its way out of it.
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
        &common::identity(ORIGIN),
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

/// THE SECOND DISCRIMINATOR, proved against a real floor — [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651).
///
/// `register_patient` refuses some inputs in Rust, before any statement reaches Postgres: the
/// date-of-birth shape is validated up front precisely so a malformed one refuses the whole
/// call with zero side effects (no HLC tick, no partial chart). Those refusals are every bit
/// as deterministic as the floor's — the same string refuses identically forever — but they
/// carry **no SQLSTATE**, so the `P0001` rule the test above proves cannot see them at all.
///
/// Until #651 they reached the clerk as `Unavailable`, and this test pinned that wrong answer
/// on purpose. It now pins the right one: `cairn_node::db_diagnosis::DeliberateRefusal` marks
/// the refusal and `data_error_from` consults **both** discriminators.
///
/// **Why this is the arm that mattered most.** The trigger applies no date format check, and
/// correctly so — a registrar is often told only a year (principle 4) — and db/046's pass 2 is
/// a string compare. So on a desk with no date widget, `3/2/1980` searches fine, finds nothing,
/// and then fails here. That is not an edge case; it is the DEFAULT failure mode of the surface
/// slice 2c builds, and before this it came with a retry button that could never work.
///
/// **The mutation that kills this test:** remove the
/// `|| cairn_node::db_diagnosis::is_deliberate_refusal(e)` arm from `data_error_from` and this
/// goes back to `Unavailable`. Nothing else in either tree notices.
#[tokio::test]
async fn a_rust_side_pre_flight_refusal_is_refused_not_unavailable() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        sk,
        &common::identity(ORIGIN),
    );

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

    let DataError::Refused(text) = &err else {
        panic!(
            "a malformed date of birth is a VERDICT: the same string refuses identically \
             forever, so offering a retry for it is the harm #648 describes, one layer above \
             where #648 was looking (#651). Got {err:?}"
        );
    };
    assert!(
        text.contains("not a recognised shape"),
        "and the orchestrator's own message is the only thing that tells the clerk WHAT to \
         change — a correctly-classified refusal that does not say why is the same silence \
         one variant over; got: {text}"
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
        cairn_gui_live::error::sqlstate_of(&bare),
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
        cairn_gui_live::error::sqlstate_of(&buried),
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

/// A refused registration must leave NO chart behind — and the probe has to reach the server
/// for that to mean anything.
///
/// # Why the probe is an unenrolled signer and NOT a malformed date of birth
///
/// The first cut of this test used `Some("not-a-date")`, and it was **vacuous**. `dob_precision`
/// runs in Rust at `register.rs:448`; the first `next_hlc` is at `:464` and
/// `client.transaction()` at `:523`. A malformed date therefore returns roughly seventy lines
/// before a single statement is sent, so `chart_count` was trivially unchanged and the
/// assertion could not fail for the reason the name claims. Deleting the transaction wrapper
/// from `register_patient` outright left the old version green.
///
/// An unenrolled signer refuses at db/005's door, so the probe now genuinely crosses into
/// `submit_event` and the count assertion is about what the SERVER did.
///
/// # ⚠️ What this still does NOT pin: the multi-event rollback
///
/// Measured, not assumed. `register_patient` wraps its up-to-four `submit_event` calls in one
/// transaction (`register.rs:523`), and replacing that with autocommit — `let tx = &*client;`,
/// no `commit` — **leaves this test green.** The reason is that an unenrolled signer is refused
/// on the FIRST event, so there is never a prior write for a rollback to undo.
///
/// Pinning the rollback needs a refusal on a LATER event: the registration act admitted, then
/// the name or dob assertion refused. This crate cannot produce that — both events are built
/// from one typed string by the same orchestrator and signed by the same key, and the door
/// treats them alike — and no test in the ROOT tree pins it either, which is the more
/// interesting half of the finding. Filed as
/// [#657](https://github.com/cairn-ehr/cairn-ehr/issues/657); it belongs in
/// `crates/cairn-node/tests/patient_register.rs`, where a fault can be injected into one door
/// and not the others.
///
/// (db/005 step 8b / #345 is about first-event PRECEDENCE, not partial writes, so it is not the
/// mechanism that would catch a half-written chart either.)
///
/// What this test DOES guarantee, and it is worth having: a floor refusal reaching the door
/// creates nothing, and the explicit `Refused` assertion below stops the probe silently
/// regressing to a Rust-side bail that never reaches Postgres at all — which is exactly what
/// the first version of this test did.
#[tokio::test]
async fn a_refused_registration_creates_no_chart() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (_enrolled, _kid) = common::setup(&reader).await;
    let stranger: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(5));
    let live = LiveData::new(
        common::connect_for_live(&cs).await,
        cairn_event::SigningKey::from_bytes(&stranger),
        &common::identity(ORIGIN),
    );

    let before = chart_count(&reader).await;

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Refusal Probe Two", Some("1984-05-06"), &[]),
    );
    let (err, returned) = live
        .register(attested, Some("Refusal Probe Two"))
        .await
        .expect_err("an unenrolled signer refuses at the floor's door");
    restore_and_expect_kept(&mut store, returned);

    // Insist the refusal is the FLOOR's, not something that failed earlier. A probe that
    // stopped reaching Postgres would make the count assertion below meaningless again, and
    // would do it silently — this is the guard against that regression.
    assert!(
        matches!(&err, DataError::Refused(t) if t.contains("not an enrolled")),
        "this probe must refuse INSIDE the transaction, at db/005's door, or the rollback is \
         not being tested at all — got {err:?}"
    );

    assert_eq!(
        before,
        chart_count(&reader).await,
        "a refused registration must leave no chart — a half-born one is worse than none, \
         because the clerk was told it failed"
    );
}

/// A FAILED SEARCH IS AN ERROR, NEVER AN EMPTY LIST.
///
/// Every other test in this crate classifies a failure that came through `register`. This one
/// covers `search`, and it is the arm where a wrong answer is worst: on the step-3 prompt an
/// empty candidate list does not read as "something went wrong", it reads as **"no such
/// patient exists — go ahead and create one"**. The clerk then registers a duplicate chart for
/// somebody already in the system, and ADR-0061's attestation permanently signs a claim that a
/// search was run and displayed nobody. An imprecise near-truth beats a precise untruth
/// (principle 4); this is the precise untruth.
///
/// The mutation this kills is the tempting "defensive" one the port's own doc forbids in as
/// many words — `.unwrap_or_else(|_| empty_list())`. Nothing else in the suite notices it.
#[tokio::test]
async fn a_failed_search_is_an_error_not_an_empty_list() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;

    // Same recipe as the outage test below: a connection whose backend is terminated under it.
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

    let live = LiveData::new(victim, sk, &common::identity(ORIGIN));
    let outcome = live
        .search(
            &SearchQuery::new("Search Outage", Some("1970-02-03"), &[]),
            TODAY,
        )
        .await;

    match outcome {
        Err(DataError::Unavailable(_)) => {}
        Ok(list) => panic!(
            "a search that could not run returned Ok({} candidates). An empty list on this \
             screen MEANS 'nobody matched, create a new chart' — reporting a failure as one is \
             how the funnel manufactures the duplicate it exists to prevent.",
            list.candidates.len()
        ),
        Err(other) => {
            panic!("a dead connection decided nothing about this search, and arrived as {other:?}")
        }
    }
}

/// ONE CONNECTION, AND IT HAS TO SURVIVE A REFUSAL.
///
/// `LiveData` holds a single `Client` for the whole life of the window (deliberately — see its
/// doc). When `submit_event` raises inside `register_patient`'s transaction, the `Transaction`
/// is dropped, and tokio-postgres sends its `ROLLBACK` **fire-and-forget**: the send result is
/// discarded and the response is never awaited. In practice it is pipelined ahead of the next
/// statement and the session recovers — but nothing in this crate proved it, because every
/// other refusal test either uses a fresh `LiveData` or refuses before the transaction opens.
///
/// If that rollback ever failed to land, every later call on this window would return `25P02
/// current transaction is aborted` → `Unavailable` → a retry button that can never succeed, for
/// the rest of the session. And #654 makes an unenrolled-signer refusal the FIRST thing a
/// freshly installed node does, so "the window is dead after its first refusal" would be the
/// out-of-the-box experience.
#[tokio::test]
async fn a_refused_registration_leaves_the_connection_usable() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (_enrolled, _kid) = common::setup(&reader).await;

    // Refuse at the floor's door with a stranger's key, on a port that then keeps working.
    let stranger: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(43).wrapping_add(7));
    let db = common::connect_for_live(&cs).await;
    let refusing = LiveData::new(
        db,
        cairn_event::SigningKey::from_bytes(&stranger),
        &common::identity(ORIGIN),
    );

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Reuse Probe", Some("1966-07-08"), &[]),
    );
    let (err, returned) = refusing
        .register(attested, Some("Reuse Probe"))
        .await
        .expect_err("an unenrolled signer refuses inside the transaction");
    restore_and_expect_kept(&mut store, returned);
    assert!(
        matches!(&err, DataError::Refused(t) if t.contains("not an enrolled")),
        "the probe must refuse at the door, inside the transaction — got {err:?}"
    );

    // THE POINT: the same connection, used again. A search first (a plain read proves the
    // session is not stuck in a failed transaction), then a write that must actually land.
    let found = refusing
        .search(&SearchQuery::new("Reuse Probe", None, &[]), TODAY)
        .await
        .expect("the connection must still answer a read after a rolled-back refusal");
    assert!(
        found.candidates.is_empty(),
        "the refusal rolled back, so nothing should have been created: {:?}",
        found.candidates
    );

    // And now a WRITE on that same `Client`. Rather than build a second `LiveData` — which
    // would open a second connection and prove nothing about this one — enrol the stranger's
    // key, which is exactly what the operator does to fix a fresh node (#654). The port is
    // unchanged; only the floor's opinion of its signer is.
    let stranger_kid = hex::encode(
        cairn_event::SigningKey::from_bytes(&stranger)
            .verifying_key()
            .to_bytes(),
    );
    reader
        .execute(
            "SELECT enroll_actor('device', \
             '{\"role\":\"registration-desk\",\"node_key\":\"reuse-probe\"}', $1)",
            &[&stranger_kid],
        )
        .await
        .expect("enrol the formerly-unknown signer");

    let mut store = TokenStore::new();
    let attested = attest(
        &mut store,
        SearchQuery::new("Reuse Probe", Some("1966-07-08"), &[]),
    );
    refusing
        .register(attested, Some("Reuse Probe"))
        .await
        .map_err(|(e, _)| e)
        .expect(
            "the SAME connection that just had a transaction rolled back under it must still \
             be able to write — if this is `25P02`, the rollback did not land and the window \
             is dead after its first refusal",
        );
    store.commit();
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

    let live = LiveData::new(victim, sk, &common::identity(ORIGIN));
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
