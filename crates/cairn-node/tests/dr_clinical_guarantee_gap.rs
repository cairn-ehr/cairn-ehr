//! #495 and #500 — ADR-0026 decision 1 promises three things about a restored node's
//! CLINICAL tier. All three are FALSE in the built system. This file pins each one
//! against reality, so the gap is loud instead of invisible.
//!
//! # The three promises
//!
//! ADR-0026 (`docs/spec/decisions/0026-node-durability-and-disaster-recovery.md`)
//! decision 1 states the honest guarantee for total hardware loss of a **solo** node,
//! restored from the sealed medium plus its recovery secret:
//!
//! > the **clinical event log survives** (verified on apply; RPO = last stream to the
//! > medium); … **node-default data-at-rest keys survive** (else every ordinary body is
//! > noise, and a solo node has no peer to re-supply them); **sealed-episode DEKs survive
//! > minus any erased ones**
//!
//! and decision 2 says *"Clinical events back up as a cold peer."*
//!
//! **"Solo" is load-bearing, and every claim in this file is scoped to it.** A FEDERATED
//! node that re-peers after a restore DOES recover custody: the serve arm re-wraps each
//! DEK against the puller's CURRENT unwrap cert (`cairn-sync`'s `rewrap_custody_for_peer`,
//! gated on the node-plane trust set since #231). ADR-0026 words the promise the way it
//! does for exactly that reason — the deployment with no such rescue is the solo clinic it
//! opens by naming, the one for which *"replication provides zero durability"*.
//!
//! # What was actually built
//!
//! - `backup::read_event_set` reads `SELECT signed_bytes FROM node_event` — the
//!   **federation plane only** — and `backup::backup_to` writes exactly that set to the
//!   medium. No `event_log`, no `event_clear`, no `event_dek`. Because `event_log` also
//!   carries the demographic, identity, registration and erasure streams, a restored solo
//!   node has **no patients and no charts at all**, not merely no clinical content (#500).
//! - Restore mints a **fresh signing key** (ADR-0026 decision 4, "the private signing key
//!   is never backed up"): `restore.rs` orchestrates the apply and the supersede, `main.rs`
//!   owns the minting. The X25519 unwrap secret is HKDF-derived from that seed
//!   (`cairn_event::seal::derive_unwrap_secret`, ADR-0052 decision 4), so a fresh seed is a
//!   fresh unwrap secret and every `event_dek` row wrapped to the dead node's public half
//!   is unopenable on that node (#495).
//! - `localstate::LocalState` reserves `node_default_deks` and `episode_deks` — the two
//!   slots that would carry the keys across — and `read_local_state` fills neither. It
//!   does not even take a live look: its `_db` parameter is unused.
//!
//! # Why the gap is honest history rather than an oversight
//!
//! `localstate.rs`'s own header declared the deferral: *"the federation-node tier has no
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
//! true today, and the guard failing IS the guard working.**
//!
//! **Four of the five tests here are PINS that go red on the commit that fixes the gap**
//! — [`medium_carries_the_federation_plane_and_no_clinical_event`] (#500),
//! [`local_state_export_carries_no_dek_though_the_database_holds_one`] (#495),
//! [`the_only_local_state_producer_is_the_empty_constructor`] (#495, the producer half),
//! and [`export_carries_no_dek_for_the_survivor_and_none_for_the_shredded`] (#495's
//! erasure dimension). Each names what it must be INVERTED to.
//! [`a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body`] is the odd one
//! out and says so in its own doc: it describes the **mechanism**, so it stays true after
//! the fix. Read that as four pins plus one mechanism test, not as five pins.
//!
//! DB-gated on `$CAIRN_TEST_PG`; the two pure tests always run. The skip is itself
//! guarded — `tests/db_gate_actually_ran.rs` derives its required-variable set from these
//! sources, so an unattended run with no `$CAIRN_TEST_PG` FAILS the crate unless
//! `CAIRN_ALLOW_DB_SKIP` is set affirmatively (#450). Key material is derived at runtime,
//! never a literal (house rule 6).

use cairn_event::seal::{
    derive_unwrap_secret, seal_event_payload, seal_stub_twin, unwrap_dek, unwrap_public,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::localstate::{read_local_state, LocalState};
use cairn_node::{backup, db, identity};
use tokio_postgres::Client;
use uuid::Uuid;
use zeroize::Zeroizing;

// Shared scaffolding, for `submit_registration` (since #345 the first event on a chart must
// be its registration) and for `medication_setup`, which owns the truncation list this
// suite needs — see `provisioned_clinic`.
mod common;

// The shared source walk, for the one guard that inspects code text rather than a database.
#[path = "common/sources.rs"]
mod sources;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Bring the database to the state a real solo clinic node is in the moment before its
/// disk dies: a provisioned node identity (so `node_event` is NON-EMPTY — see the
/// anti-vacuity note in [`medium_carries_the_federation_plane_and_no_clinical_event`]),
/// an enrolled actor, and a registered unwrap key.
///
/// The truncation is delegated to `common::medication_setup` rather than copied: it is the
/// canonical list, and it also sweeps the medication PROJECTION tables, which have no FK to
/// `event_log` and so survive a `TRUNCATE … CASCADE` from it. A local copy of only the
/// core-table half (the shape this suite first shipped with) leaves a `medication_statement`
/// row behind on every run of a shared, serialized database. Issue #340 tracks consolidating
/// the remaining medication truncation lists; this suite deliberately joins the shared one
/// rather than adding a fourth.
///
/// `medication_setup` registers the node's single X25519 unwrap key from the DEVICE key, so
/// this function provisions the node identity with that SAME key. That is not incidental
/// tidiness — the coupling between the node's signing seed and its unwrap secret is the
/// whole subject of [`a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body`],
/// and a fixture that split them would not be modelling the node under test.
async fn provisioned_clinic(c: &Client) -> (SigningKey, String) {
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(c).await;

    // The node's federation identity — this is what the backup medium DOES carry.
    identity::provision(c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
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

/// Submit ONE real born-sealed clinical event on a fresh chart through the strict door.
/// Returns its `event_id` and its signed bytes (the exact bytes a backup medium would have
/// to carry for the clinical log to survive).
///
/// ANTI-VACUITY, and the reason this helper does more than submit: `.execute()` discards
/// `submit_event`'s return value, and the door's `event_log` INSERT is `ON CONFLICT DO
/// NOTHING`, so "no error" is an invariant of a distant door rather than evidence visible
/// here. Every assertion in this file about what the medium and the export do NOT carry is
/// worthless unless the thing they should carry genuinely exists — so this reads the row
/// back before returning.
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

    let landed: Vec<u8> = c
        .query_one(
            "SELECT signed_bytes FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect(
            "anti-vacuity: the event must genuinely BE in event_log, or its absence \
                 from the medium and the export proves nothing",
        )
        .get(0);
    assert_eq!(
        landed, signed.signed_bytes,
        "the log holds the exact bytes this test will look for"
    );

    (event_id, signed.signed_bytes)
}

/// **Promise 1 — "the clinical event log survives" — is FALSE (#500).**
///
/// The medium a solo clinic's whole durability story rests on carries the federation
/// plane and nothing else. After a dead disk, restore rehydrates who this node peered
/// with and recovers zero clinical records — and, since `event_log` is also the home of
/// the demographic, identity and registration streams, zero patients.
///
/// Anti-vacuity, on both sides: the node is provisioned first, so the medium is genuinely
/// NON-EMPTY (otherwise "the medium holds no clinical event" would also pass over an empty
/// export); and `author_sealed_clinical_event` reads its event back out of `event_log`, so
/// there is genuinely something for the medium to be missing. Together they close the
/// 2026-08-23 lesson — a guard that cannot observe the property it names.
///
/// The pin is taken at BOTH seams on purpose. `read_event_set` is where the wrong table is
/// named, but a plausible fix — add a clinical reader and compose both inside `backup_to` —
/// would leave a `read_event_set`-only assertion green while closing the defect. So the
/// medium FILE that `backup_to` actually writes is checked too.
///
/// **When #500 is fixed:** invert the three PINS — the clinical event's signed bytes must
/// appear in the event set and in the medium file, and the set's count must exceed the
/// node-plane count. The fourth assertion, `!medium.is_empty()`, is the anti-vacuity guard
/// and stays exactly as it is.
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
        "PINS #500: the backup medium carries no clinical event. ADR-0026 decision 1 says \
         the clinical event log survives a restore and decision 2 says clinical events back \
         up as a cold peer; backup::read_event_set reads only `node_event`. When #500 is \
         fixed this assertion must be INVERTED."
    );

    // The set is `node_event` verbatim. This equality is structurally guaranteed today
    // (`read_event_set` is an unfiltered SELECT over that table), so it proves nothing
    // about the present — it is here purely as a TRIPWIRE: any fix that widens the medium
    // reddens it, including one that adds clinical events without touching the assertion
    // above.
    let node_events: i64 = c
        .query_one("SELECT count(*) FROM node_event", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        medium.len() as i64,
        node_events,
        "PINS #500: the medium is the `node_event` set exactly — nothing clinical has been \
         added to it"
    );

    // And the same absence in the artifact an operator actually carries off-site: the
    // seam a fix is most likely to touch is `backup_to`, not `read_event_set`.
    let tmp = tempfile::tempdir().unwrap();
    let medium_path = tmp.path().join("cairn.medium");
    let health_path = tmp.path().join("backup-status.json");
    backup::backup_to(&c, &medium_path, &health_path, 0, Some((&sk, &kid)))
        .await
        .expect("the backup ceremony succeeds — which is the point");
    let on_disk = std::fs::read(&medium_path).unwrap();
    assert!(
        !on_disk
            .windows(clinical_bytes.len())
            .any(|w| w == clinical_bytes),
        "PINS #500: the medium FILE `backup_to` writes — the artifact the operator carries \
         off-site, and the one `verify-backup` reports OK over — does not contain the \
         clinical event's bytes anywhere. When #500 is fixed this assertion must be INVERTED."
    );
}

/// **Promises 2 and 3 — "node-default data-at-rest keys survive" and "sealed-episode DEKs
/// survive minus any erased ones" — are FALSE (#495).**
///
/// The sealed local-state export (ADR-0026 slice D) is the only artifact that could carry
/// key material off the dying node. Its two DEK slots exist and stay empty, and
/// `read_local_state` never looks at the database at all — so a node holding real custody
/// exports an empty bundle without noticing.
///
/// Anti-vacuity: the custody read back below is opened with the node's own unwrap secret,
/// so the row is proven to be REAL, OPENABLE and THIS event's — a length check alone would
/// pass over a well-shaped but meaningless blob.
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

    // Anti-vacuity: custody genuinely exists on this node — a wrapped DEK written by the
    // production door, which the node's own unwrap secret opens.
    let wrapped: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    let secret = derive_unwrap_secret(&sk.to_bytes());
    unwrap_dek(&wrapped, &secret)
        .expect("the door wrapped a real, openable DEK into this node's custody");

    let exported = read_local_state(&c).await.expect("export must succeed");

    assert!(
        exported.episode_deks.is_empty(),
        "PINS #495: the sealed local-state export carries no sealed-episode DEK, though \
         event_dek holds an openable one for this very event. ADR-0026 decision 1 promises \
         they survive. When #495 is fixed this assertion must be INVERTED."
    );
    assert!(
        exported.is_empty(),
        "PINS #495: the WHOLE bundle is empty on a node with a live clinical tier. \
         `read_local_state`'s `_db` parameter is unused, so the export cannot see custody \
         even in principle — the seam localstate.rs declared for the clinical tier is \
         still open, and the clinical tier now exists."
    );

    // Promise 2 is UNPINNABLE from the export side, and the reason IS the finding: there
    // is no node-default data-at-rest keystore anywhere in the built system (the only
    // `node_default` in the tree is the empty slot itself). So an assertion that the slot
    // is empty could never redden — not even after a complete fix — and asserting it would
    // be exactly the vacuous guard this file exists to avoid. What IS observable is the
    // absence of the store, so that is what is asserted.
    let node_default_store: i64 = c
        .query_one(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name LIKE '%node_default%'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        node_default_store, 0,
        "PINS #495: promise 2 (\"node-default data-at-rest keys survive\") has no subject — \
         no node-default keystore exists to be exported. When one is built, this assertion \
         must be INVERTED and `exported.node_default_deks` pinned the way episode_deks is \
         above."
    );
}

/// **The erasure half of promise 3 — "minus any erased ones" — has no guard at all today,
/// and it is the half whose failure is worst.**
///
/// ADR-0026 point 6 requires a restore to replay the shred log so an erased body is not
/// resurrected. A fix for #495 that routes `event_dek` rows into the export WITHOUT
/// consulting `erasure_shred_log` would carry a crypto-shredded body's key across the
/// restore boundary and undo an erasure the node already executed — the worst outcome in
/// this whole area, and the rest of this suite would stay green through it. This test is
/// the tripwire.
///
/// Anti-vacuity: two sealed events are authored and only one is shredded, and BOTH custody
/// rows are checked afterwards — the shredded one gone, the survivor's still openable. So
/// the shred is proven to have really executed, and a fix cannot satisfy this test by
/// simply exporting nothing for either.
///
/// **When #495 is fixed:** invert the `episode_deks.is_empty()` assertion — the export must
/// carry the SURVIVOR's DEK — and keep the second exactly as it is: the shredded event's key must
/// still never appear. That asymmetry is the whole point of this test, and it is why the
/// second assertion is written now even though it is VACUOUS today (an empty export has
/// nothing to scan — the code says so at the assertion). It becomes load-bearing on the
/// exact commit that makes the first one false.
#[tokio::test]
async fn export_carries_no_dek_for_the_survivor_and_none_for_the_shredded() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;

    let (survivor_id, _) = author_sealed_clinical_event(&c, &sk, &kid).await;
    let (shredded_id, _) = author_sealed_clinical_event(&c, &sk, &kid).await;
    shred(&c, &sk, &kid, &shredded_id).await;

    // Anti-vacuity: the shred really executed (its custody is gone) and the survivor's
    // custody really remains, so "the export carries neither" is a statement about the
    // export rather than about an empty database.
    let shredded_custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&shredded_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        shredded_custody, 0,
        "the shred executed: cairn_execute_shred destroyed the target's DEK"
    );
    let survivor_custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&survivor_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        survivor_custody, 1,
        "the shred was provenance-precise: the unshredded event keeps its custody"
    );

    let exported = read_local_state(&c).await.expect("export must succeed");
    assert!(
        exported.episode_deks.is_empty(),
        "PINS #495: the export carries no DEK for the SURVIVING sealed body, though the \
         node holds its custody. When #495 is fixed this assertion must be INVERTED — the \
         survivor's DEK must travel."
    );
    // VACUOUS TODAY, AND SAID SO PLAINLY: the assertion above pins `episode_deks` empty, so
    // this scan has nothing to scan. It is not evidence about the present — it is a FORWARD
    // tripwire, placed here because the commit that inverts the assertion above is exactly
    // the commit that could get this one wrong, and a reviewer of that commit will be
    // reading this file. Stating the vacuity is the point: an unlabelled vacuous guard
    // reads as coverage, which is the failure this whole suite exists to name.
    let shredded_marker = shredded_uuid_bytes(&shredded_id);
    assert!(
        !exported
            .episode_deks
            .iter()
            .any(|d| d.windows(16).any(|w| w == shredded_marker.as_slice())),
        "PINS #495 / ADR-0026 point 6: no trace of the SHREDDED event's custody may ever \
         reach the export. This assertion must SURVIVE the fix unchanged — a fix that \
         exports event_dek without consulting erasure_shred_log resurrects an executed \
         erasure on restore."
    );
}

/// The raw 16 bytes of an event UUID, for scanning an opaque export slot. The slot's
/// internal schema is deliberately uncommitted (`Vec<u8>` — no speculative generality), so
/// the only shape-independent way to ask "is this event referenced in there?" is to look
/// for its identifier's bytes.
fn shredded_uuid_bytes(event_id: &str) -> [u8; 16] {
    *Uuid::parse_str(event_id).unwrap().as_bytes()
}

/// Submit an `erasure.shred.asserted` tombstone against `target` through the strict door.
/// Plaintext by design (the tombstone must outlive every key) — mirrors `seal_apply.rs`.
async fn shred(c: &Client, sk: &SigningKey, kid: &str, target: &str) {
    // `::text` on the read: this crate does not enable tokio-postgres's `uuid` feature, so a
    // UUID column cannot be decoded directly — cast it in SQL and carry it as a String, which
    // is what `EventBody::patient_id` wants anyway (the repo-wide read idiom).
    let patient: String = c
        .query_one(
            "SELECT patient_id::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient,
        event_type: "erasure.shred.asserted".into(),
        schema_version: "erasure.shred/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({
            "target_event_id": target,
            "basis": "retention ceiling",
        }),
        attachments: vec![],
        plaintext_twin: Some(format!(
            "shredded medication assertion {target} — basis: retention ceiling"
        )),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, NULL)",
        &[&signed.signed_bytes],
    )
    .await
    .expect("a plaintext erasure tombstone is admitted");
}

/// **The mechanism** — why the promises above cannot be rescued by a database-level
/// restore either. Pure: no database, always runs.
///
/// Restore mints a fresh signing key by design (ADR-0026 decision 4), and ADR-0052
/// decision 4 derives the X25519 unwrap secret from that seed. So the restored node's
/// unwrap secret is a *different* secret, and every `event_dek` row it inherits — from a
/// disk image, a `pg_dump`, or a peer that re-supplied the rows verbatim — is noise to it.
///
/// SCOPE, stated precisely because the obvious over-claim is wrong: this does NOT mean a
/// database-level restore yields unreadable bodies. `event_clear` is an ordinary logged
/// table holding the CLEAR payload and clear twin for every sealed body this node has
/// custody of, so a `pg_dump` or disk image carries readable content without needing any
/// DEK at all. The true and still-damning statement is narrower — the inherited DEKs are
/// noise, and NEITHER the backup medium NOR the sealed export carries the DEKs or
/// `event_clear`, so the ADR-0026 restore path (medium + recovery secret) is the one that
/// arrives with nothing.
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

    // The restored node cannot. Its inherited custody is noise.
    assert!(
        unwrap_dek(&wrapped, &restored_secret).is_err(),
        "PINS #495: a node restored under a fresh identity cannot unwrap custody written \
         before the loss — so the DEKs that reach it, by whatever route, are noise to it"
    );
}

/// The empty bundle is not a runtime accident that a populated node would avoid — there is
/// **no code anywhere that builds a non-empty one**. Pure: no database.
///
/// Kept separate from the DB-gated export test on purpose: that one shows the *reader*
/// ignores custody, this one shows there is no *producer* either. Both would have to change
/// for ADR-0026 decision 1 to become true, and a fix that touched only one would leave the
/// other green.
///
/// This is a SOURCE-DERIVED guard, the repo's idiom for a claim about code shape
/// (`event_log_row_by_name.rs`, `no_drugref_dependency.rs`). Asserting instead that
/// `LocalState::empty()` returns something empty would be a tautology over a constructor's
/// own name — it could not observe the property this test is named for, which is exactly
/// the failure mode this file exists to avoid.
///
/// **When #495 is fixed:** a second producer must exist (the one that reads custody out of
/// the database), so the count below goes to 2 and this assertion reddens. Raise it to the
/// new number and name the new producer in the message.
#[test]
fn the_only_local_state_producer_is_the_empty_constructor() {
    let src =
        sources::read_source(&sources::repo_root().join("crates/cairn-node/src/localstate.rs"));

    let producers: Vec<(usize, String)> = src
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim().to_string()))
        .filter(|(_, l)| constructs_local_state(l))
        .collect();

    assert_eq!(
        producers.len(),
        1,
        "PINS #495: `LocalState` is constructed in exactly ONE place — `empty()` — so the \
         empty bundle is the only bundle this node can build, no matter what the database \
         holds. Found: {producers:?}"
    );

    // Also confirm the one producer really is `empty()`, not some other site that happens
    // to be alone — the count alone would not notice a swap.
    let ls = LocalState::empty();
    assert!(
        ls.is_empty(),
        "the sole producer is `empty()`, and it produces an empty bundle"
    );
}

/// True iff `line` constructs a `LocalState` struct literal.
///
/// Pure, and deliberately fussy about the word boundary: `SealedLocalState { … }` contains
/// `LocalState {` as a substring, and counting it would silently inflate the producer count
/// — a guard that reports the wrong number is worse than no guard. Declaration lines
/// (`pub struct LocalState {`, `impl LocalState {`) are not constructions and are excluded
/// too, as are comments.
fn constructs_local_state(line: &str) -> bool {
    let Some(i) = line.find("LocalState {") else {
        return false;
    };
    if line.starts_with("//") || line.starts_with("pub struct") || line.starts_with("struct") {
        return false;
    }
    if line.starts_with("impl") {
        return false;
    }
    // `SealedLocalState` / `MyLocalState` are different types, not this one: require that the
    // character before the needle is not part of an identifier.
    !matches!(line[..i].chars().next_back(), Some(c) if c.is_alphanumeric() || c == '_')
}
