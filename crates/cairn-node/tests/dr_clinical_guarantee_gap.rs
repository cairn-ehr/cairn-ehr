//! #495 — ADR-0026 decision 1 promises three things about a restored node's CLINICAL
//! tier. All three are FALSE in the built system. This file pins each one against
//! reality, so the gap is loud instead of invisible.
//!
//! # The three promises
//!
//! [ADR-0026](../../../docs/spec/decisions/0026-node-durability-and-disaster-recovery.md)
//! decision 1 states the honest guarantee for total hardware loss of a solo node,
//! restored from the sealed medium plus its recovery secret:
//!
//! > the **clinical event log survives** (verified on apply; RPO = last stream to the
//! > medium); … **node-default data-at-rest keys survive** (else every ordinary body is
//! > noise, and a solo node has no peer to re-supply them); **sealed-episode DEKs survive
//! > minus any erased ones**
//!
//! and decision 2 says *"Clinical events back up as a cold peer."*
//!
//! # What was actually built
//!
//! - `backup::read_event_set` reads `SELECT signed_bytes FROM node_event` — the
//!   **federation plane only**. No `event_log`, no `event_clear`, no `event_dek`.
//! - `restore` mints a **fresh signing key** (ADR-0026 decision 4, "the private signing
//!   key is never backed up"), and the X25519 unwrap secret is HKDF-derived from that
//!   seed (`cairn_event::seal::derive_unwrap_secret`, ADR-0052 decision 4). A fresh seed
//!   is therefore a fresh unwrap secret, and every `event_dek` row wrapped to the dead
//!   node's public half is permanently unopenable.
//! - `localstate::LocalState` reserves `node_default_deks` and `episode_deks` — the two
//!   slots that would carry the keys across — and `read_local_state` fills neither. It
//!   does not even take a live look: its `_db` parameter is unused.
//!
//! # Why the gap is honest history rather than an oversight
//!
//! `localstate.rs`'s own header declares the deferral: *"the federation-node tier has no
//! clinical surface yet, so the bundle is empty … the clinical tier fills later via
//! additive evolution."* That was **true when slice D was written**. It expired when
//! ADR-0052 made every clinical body born-sealed, and nothing re-opened it — while
//! ROADMAP went on recording slices A–D as ✓ done. The precondition is the thing that
//! rotted, not the code.
//!
//! # Why these tests assert the DEFECT rather than the guarantee
//!
//! The obvious TDD move is a red test stating the promise. This crate has no `#[ignore]`
//! anywhere and a permanently-red test would block the gate for every unrelated change,
//! so the pin follows the repo's existing "pinned count" idiom instead: **assert what is
//! true today, and the guard failing IS the guard working.** Whoever closes #495 will see
//! these four tests go red on the same commit that makes them wrong, with a header saying
//! exactly what to replace each assertion with. Nothing here can be quietly satisfied by
//! the fix landing half-way.
//!
//! DB-gated on `$CAIRN_TEST_PG`; the two mechanism tests are pure and always run. Key
//! material is derived at runtime, never a literal (house rule 6).

use cairn_event::seal::{
    derive_unwrap_secret, seal_event_payload, seal_stub_twin, unwrap_dek, unwrap_public,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::localstate::{read_local_state, LocalState};
use cairn_node::{backup, db, identity};
use tokio_postgres::Client;
use uuid::Uuid;
use zeroize::Zeroizing;

// Shared scaffolding, for `submit_registration`: since #345 the first event on a chart
// must be its registration, so every suite that mints a patient arranges one.
mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Bring the database to the state a real solo clinic node is in the moment before its
/// disk dies: a provisioned node identity (so `node_event` is NON-EMPTY — see the
/// anti-vacuity note in [`medium_carries_the_federation_plane_and_no_clinical_event`]),
/// one enrolled actor, and a registered unwrap key.
///
/// The custody-plane tables have no FK to `event_log`, so `CASCADE` from it does not reach
/// them — they must be truncated by name, or a previous suite's node key collides with
/// this one's at `cairn_register_unwrap_key` (the singleton refuses a different key).
async fn provisioned_clinic(c: &Client) -> (SigningKey, String) {
    db::reset_node_federation_tables(c).await.unwrap();
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();

    let (sk, kid) = generate_key().unwrap();

    // The node's federation identity — this is what the backup medium DOES carry.
    identity::provision(c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();

    // The clinical actor that authors the sealed event below.
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();

    // The node's X25519 public half, so the strict door can wrap DEKs into custody.
    let secret = derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();

    (sk, kid)
}

/// Build a sealed `clinical.medication.asserted` body plus the DEK the strict door needs.
/// Mirrors `seal_submit.rs`'s fixture — a real born-sealed body, not a hand-built row, so
/// the `event_dek` custody these tests read is produced by the production door.
fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Zeroizing<[u8; 32]>) {
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    (body, dek)
}

/// Author one real born-sealed clinical event through the strict door. Returns its
/// `event_id` and its signed bytes (the exact bytes a backup medium would have to carry
/// for the clinical log to survive).
async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> (String, Vec<u8>) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;

    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let (body, dek) = sealed_assert_body(kid, patient, hlc);
    let event_id = body.event_id.clone();
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("a sealed body with its DEK is admitted");

    (event_id, signed.signed_bytes)
}

/// **Promise 1 — "the clinical event log survives" — is FALSE.**
///
/// The medium a solo clinic's whole durability story rests on carries the federation
/// plane and nothing else. After a dead disk, restore rehydrates who this node peered
/// with and recovers zero clinical records.
///
/// Anti-vacuity: the node is provisioned first, so the medium is genuinely NON-EMPTY.
/// Without that, "the medium holds no clinical event" would also pass over an empty
/// export, and the test would prove nothing (the 2026-08-23 lesson: a guard that cannot
/// observe the property it names).
///
/// **When #495 is fixed:** invert the two `assert!`s below — the clinical event's signed
/// bytes MUST appear in the medium, and the medium's count must exceed the node-plane
/// count.
#[tokio::test]
async fn medium_carries_the_federation_plane_and_no_clinical_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (_event_id, clinical_bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;

    let medium = backup::read_event_set(&c).await.unwrap();

    // Anti-vacuity: the export is real and non-empty, so the absence below is a real
    // absence rather than "there was nothing to look at".
    assert!(
        !medium.is_empty(),
        "the node was provisioned, so the medium must carry at least the genesis event — \
         an empty medium would make the assertions below vacuous"
    );

    // Promise 1, as built: the clinical event is simply not there.
    assert!(
        !medium.contains(&clinical_bytes),
        "PINS #495: the backup medium carries no clinical event. ADR-0026 decision 1 says \
         the clinical event log survives a restore and decision 2 says clinical events back \
         up as a cold peer; backup::read_event_set reads only `node_event`. When #495 is \
         fixed this assertion must be INVERTED."
    );

    // And it is exactly the node plane, event for event — so the gap is the whole clinical
    // tier, not a filter that happened to drop one row.
    let node_events: i64 = c
        .query_one("SELECT count(*) FROM node_event", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        medium.len() as i64,
        node_events,
        "PINS #495: the medium is the `node_event` set exactly — the clinical log is absent \
         wholesale, not partially filtered"
    );
}

/// **Promises 2 and 3 — "node-default data-at-rest keys survive" and "sealed-episode DEKs
/// survive minus any erased ones" — are FALSE.**
///
/// The sealed local-state export (ADR-0026 slice D) is the only artifact that could carry
/// key material off the dying node. Its two DEK slots exist and stay empty, and
/// `read_local_state` never looks at the database at all — so a node holding real custody
/// exports an empty bundle without noticing.
///
/// **When #495 is fixed:** `episode_deks` must be non-empty here, and `is_empty()` false.
#[tokio::test]
async fn local_state_export_carries_no_dek_though_the_database_holds_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, _bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;

    // Anti-vacuity: custody genuinely exists on this node — a real 104-byte wrapped DEK
    // written by the production door, not a row this test inserted.
    let wrapped: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        wrapped.len(),
        104,
        "the door wrapped a real DEK into custody"
    );

    let exported = read_local_state(&c).await.expect("export must succeed");

    assert!(
        exported.episode_deks.is_empty(),
        "PINS #495: the sealed local-state export carries no sealed-episode DEK, though \
         event_dek holds one for this very event. ADR-0026 decision 1 promises they \
         survive. When #495 is fixed this assertion must be INVERTED."
    );
    assert!(
        exported.node_default_deks.is_empty(),
        "PINS #495: the export carries no node-default data-at-rest key either — the other \
         half of ADR-0026 decision 1's key-survival promise"
    );
    assert!(
        exported.is_empty(),
        "PINS #495: the WHOLE bundle is empty on a node with a live clinical tier. \
         `read_local_state`'s `_db` parameter is unused, so the export cannot see custody \
         even in principle — the seam localstate.rs declared for the clinical tier is \
         still open, and the clinical tier now exists."
    );
}

/// **The mechanism** — why the promises above cannot be rescued by a database-level
/// restore either. Pure: no database, always runs.
///
/// `restore` mints a fresh signing key by design (ADR-0026 decision 4), and ADR-0052
/// decision 4 derives the X25519 unwrap secret from that seed. So the restored node's
/// unwrap secret is a *different* secret, and every `event_dek` row it inherits — from a
/// disk image, a `pg_dump`, or a peer that re-supplied the rows — is noise.
///
/// The happy-path leg is asserted FIRST and deliberately: without it a broken `wrap`/
/// `unwrap` pair would make the refusal below pass for entirely the wrong reason.
///
/// **When #495 is fixed:** this test stays true — it describes the mechanism, not the
/// gap. What must change is that the fix routes the *secret* (or the seed) across the
/// restore boundary, which is the decision #495 asks for.
#[test]
fn a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body() {
    // House rule 6: key material is DERIVED at runtime, never a literal — a byte-array
    // literal in a crypto context is a recurring CodeQL critical false positive (#146).
    let dead_seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(11));
    let restored_seed: [u8; 32] =
        std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(29));
    assert_ne!(
        dead_seed, restored_seed,
        "the two seeds must genuinely differ, or the refusal below proves nothing"
    );

    let dead_secret = derive_unwrap_secret(&dead_seed);
    let restored_secret = derive_unwrap_secret(&restored_seed);
    assert_ne!(
        dead_secret.as_slice(),
        restored_secret.as_slice(),
        "a fresh signing seed derives a fresh unwrap secret (ADR-0052 decision 4)"
    );

    // A per-event DEK, as the seal path mints one, wrapped to the DEAD node's public half.
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3).wrapping_add(5));
    let wrapped = cairn_event::seal::wrap_dek_for(&dek, &unwrap_public(&dead_secret)).unwrap();

    // The node that authored it can open it — so the refusal below is about the KEY, not
    // about a broken wrap.
    let opened = unwrap_dek(&wrapped, &dead_secret).expect("the authoring node opens its own DEK");
    assert_eq!(opened.as_slice(), dek.as_slice());

    // The restored node cannot. Every born-sealed body on that node is dark.
    assert!(
        unwrap_dek(&wrapped, &restored_secret).is_err(),
        "PINS #495: a node restored under a fresh identity cannot unwrap custody written \
         before the loss — so even a database-level restore leaves every born-sealed \
         clinical body permanently unreadable"
    );
}

/// The empty bundle is not a runtime accident that a populated node would avoid — it is
/// the only bundle `LocalState` can currently be built into. Pure: no database.
///
/// Kept separate from the DB-gated export test on purpose: that one shows the *reader*
/// ignores custody, this one shows there is no *producer* either. Both would have to
/// change for ADR-0026 decision 1 to become true, and a fix that touched only one would
/// leave the other green.
///
/// **When #495 is fixed:** `LocalState::empty()` may well stay empty (it is the
/// constructor for "nothing to carry"), but a populated constructor must exist and be
/// named here.
#[test]
fn the_empty_bundle_is_the_only_one_the_node_can_build() {
    let ls = LocalState::empty();
    assert!(ls.node_default_deks.is_empty());
    assert!(ls.episode_deks.is_empty());
    assert!(
        ls.is_empty(),
        "PINS #495: `LocalState::empty()` is the only bundle the node constructs, and \
         `is_empty()` is documented as \"the only valid state at this tier\" — a sentence \
         written before the clinical tier existed"
    );
}
