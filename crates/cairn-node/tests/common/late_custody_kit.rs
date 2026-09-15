//! A sealed medication event this node holds WITHOUT its key, and the key arriving later — the
//! shared fixture for #584's tests (ADR-0070).
//!
//! # Why this file exists
//!
//! Custody can reach an event after the event itself: a peer serves the bytes before this node is
//! admitted (`pull --full` later re-offers them with a DEK), a restore applies a keyless copy before
//! its keyed one, a `requeue` lands a penned key. Every one of those ends in the same database
//! state — an `event_log` row with no `event_clear` row, then a second apply that writes
//! `event_clear` — so the tests drive that state directly through the two doors, with bytes built
//! by the production medication builder rather than by hand.
//!
//! Include it with `#[path = "common/late_custody_kit.rs"] mod late_custody_kit;`. The including
//! binary must ALSO declare `mod common;`, because the node is set up by `common::medication_setup`.
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::{sign, Hlc, SigningKey};
use cairn_node::medication::{build_assert_body, AssertMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset (the repo-wide self-skip, policed
/// by `tests/db_gate_actually_ran.rs`).
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21): below today, so no clock ceiling trips,
/// and a fixed base so a test can order its events by adding to it.
pub const WALL: i64 = 1_782_000_000_000;

/// The two actors every event here needs: the DEVICE (this node; its key derives the registered
/// unwrap key in `medication_setup`) and the HUMAN who authors and signs (ADR-0053).
pub struct Keys {
    pub sk_device: SigningKey,
    pub kid_device: String,
    pub sk_human: SigningKey,
    pub kid_human: String,
}

/// An empty clinical node with both actors enrolled and its unwrap key registered.
///
/// `medication_setup` truncates `event_log` with CASCADE, which also clears `event_deferred` through
/// its foreign key. The conflict-flag table is not on its list, so it is cleared here.
pub async fn fresh_node(c: &Client) -> Keys {
    let (sk_device, kid_device, sk_human, kid_human) = common::medication_setup(c).await;
    c.batch_execute("TRUNCATE medication_patient_conflict_flag")
        .await
        .unwrap();
    Keys {
        sk_device,
        kid_device,
        sk_human,
        kid_human,
    }
}

/// One sealed `clinical.medication.asserted`, as it travels: signed bytes, and the DEK a custody
/// holder would hand the door beside them.
pub struct SealedAssert {
    pub signed: Vec<u8>,
    /// The plaintext DEK. A test-only copy out of `Secret32`, because it is bound as a query
    /// parameter; production never widens a DEK's lifetime this way.
    pub dek: Vec<u8>,
    /// The payload BEFORE sealing — what `event_clear.body` holds once custody lands. Only
    /// `heal_safe_dispatch.rs` uses it, to write the clear view by hand.
    pub clear_payload: serde_json::Value,
    pub event_id: Uuid,
    pub medication_id: Uuid,
    pub patient: Uuid,
    /// The clear twin sealed inside the container: finding it in `event_clear` proves the body
    /// OPENED rather than that some row exists.
    pub twin: String,
}

/// Build, seal and sign a medication assert through the production builder. **Pure** apart from
/// the DEK `seal_event_payload` mints.
///
/// The caller chooses `event_id` and `medication_id` so a test can build a RIVAL (same event id,
/// different body) or a second event on the SAME thread.
pub fn sealed_assert(
    keys: &Keys,
    patient: Uuid,
    medication_id: Uuid,
    event_id: Uuid,
    term: &str,
    wall: i64,
) -> SealedAssert {
    let input = AssertMedicationInput {
        term,
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let hlc = Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    };
    let body = build_assert_body(
        event_id,
        medication_id,
        patient,
        &input,
        &keys.kid_device,
        hlc,
        None,
    );
    let mut body = cairn_event::contributor::with_human_author(body, &keys.kid_human);
    let twin = body
        .plaintext_twin
        .take()
        .expect("build_assert_body always sets a plaintext twin");
    let clear_payload = body.payload.clone();
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal a well-formed medication payload");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, &keys.sk_human).expect("sign the sealed body");
    SealedAssert {
        signed: signed.signed_bytes,
        dek: dek.as_bytes().to_vec(),
        clear_payload,
        event_id,
        medication_id,
        patient,
        twin,
    }
}

/// The bytes through the REMOTE door with no DEK: admitted sealed, no custody, nothing projected.
pub async fn apply_without_key(c: &Client, e: &SealedAssert) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&e.signed])
        .await
}

/// The same bytes through the REMOTE door WITH the DEK.
pub async fn apply_with_key(c: &Client, e: &SealedAssert) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&e.signed, &e.dek],
    )
    .await
}

/// The same bytes through the STRICT door WITH the DEK.
pub async fn submit_with_key(c: &Client, e: &SealedAssert) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&e.signed, &e.dek],
    )
    .await
}

/// The clear twin of `event_id`, or `None` when this node cannot read the body.
///
/// UUIDs are bound as text and cast in SQL: `cairn-node` does not enable tokio-postgres's
/// `with-uuid-1` feature.
pub async fn clear_twin(c: &Client, event_id: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

async fn count(c: &Client, sql: &str, id: Uuid) -> i64 {
    c.query_one(sql, &[&id.to_string()]).await.unwrap().get(0)
}

/// Rows on the medication list for this thread — the number a clinician sees.
pub async fn statement_rows(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid",
        medication_id,
    )
    .await
}

/// The dose timeline's seed row: `clinical.medication.asserted` has TWO registered appliers, and a
/// fix that ran only one of them would pass a statement-only assertion.
pub async fn dose_seed_rows(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_dose_event \
         WHERE medication_id = $1::text::uuid AND is_initial",
        medication_id,
    )
    .await
}

/// #192 cross-patient flags raised on this thread.
pub async fn conflict_flags(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
        medication_id,
    )
    .await
}

/// Remove the counting appliers, if present. Called at test START (a predecessor that panicked
/// may have left them — the #583 reset-at-start rule) and BEFORE asserting, so a failed assertion
/// never leaves two extra rows in a registry `projection_registry.rs` pins at an exact count.
pub async fn remove_probe(c: &Client) {
    c.batch_execute(
        "DELETE FROM cairn_projection_apply \
           WHERE apply_fn IN ('cairn_test_late_custody_safe', 'cairn_test_late_custody_unsafe'); \
         DROP FUNCTION IF EXISTS cairn_test_late_custody_safe(event_log); \
         DROP FUNCTION IF EXISTS cairn_test_late_custody_unsafe(event_log); \
         DROP TABLE IF EXISTS cairn_test_late_custody_runs;",
    )
    .await
    .unwrap();
}

/// Register two appliers for `event_type` that do nothing but COUNT their runs: one heal-safe, one
/// not. Fault injection without residue — see [`remove_probe`].
///
/// Neither reads custody, so they run identically with or without a body; what differs between
/// them is only the registry's `heal_safe` flag, which is exactly the variable under test.
pub async fn install_probe(c: &Client, event_type: &str) {
    remove_probe(c).await;
    c.batch_execute(
        "CREATE TABLE cairn_test_late_custody_runs (applier text NOT NULL, event_id uuid NOT NULL); \
         CREATE FUNCTION cairn_test_late_custody_safe(e event_log) RETURNS void LANGUAGE sql AS \
           $$ INSERT INTO cairn_test_late_custody_runs VALUES ('safe', e.event_id) $$; \
         CREATE FUNCTION cairn_test_late_custody_unsafe(e event_log) RETURNS void LANGUAGE sql AS \
           $$ INSERT INTO cairn_test_late_custody_runs VALUES ('unsafe', e.event_id) $$;",
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO cairn_projection_apply \
           (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES \
           ($1, 'cairn_test_late_custody_safe',   ARRAY['cairn_test_late_custody_runs'], 900, TRUE), \
           ($1, 'cairn_test_late_custody_unsafe', ARRAY['cairn_test_late_custody_runs'], 900, FALSE)",
        &[&event_type],
    )
    .await
    .unwrap();
}

/// How many times the probe applier named `applier` (`"safe"` or `"unsafe"`) has run.
pub async fn probe_runs(c: &Client, applier: &str) -> i64 {
    c.query_one(
        "SELECT count(*) FROM cairn_test_late_custody_runs WHERE applier = $1",
        &[&applier],
    )
    .await
    .unwrap()
    .get(0)
}
