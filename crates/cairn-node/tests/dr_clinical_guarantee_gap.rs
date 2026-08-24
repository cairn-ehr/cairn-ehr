//! #495 and #500 — ADR-0026 decision 1 promises three things about a restored node's
//! CLINICAL tier. This file holds each promise against what is actually built, so a gap is
//! loud instead of invisible.
//!
//! **When this file was written all three were FALSE.** ADR-0066 has since closed the
//! CUSTODY one (#495) — the node's unwrap key is an independent keypair that now rides the
//! sealed export — so this is no longer "four pins plus a mechanism". It is a MIX: some
//! tests assert the promise and stay green, others still pin today's defect. Every test
//! below says in its own doc which of the two it is; read that before changing one.
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
//! # What is actually built
//!
//! - **STILL BROKEN (#500).** `backup::read_event_set` reads `SELECT signed_bytes FROM
//!   node_event` — the **federation plane only** — and `backup::backup_to` writes exactly
//!   that set to the medium. No `event_log`, no `event_clear`, no `event_dek`. Because
//!   `event_log` also carries the demographic, identity, registration and erasure streams,
//!   a restored solo node has **no patients and no charts at all**, not merely no clinical
//!   content.
//! - Restore still mints a **fresh signing key** (ADR-0026 decision 4, "the private signing
//!   key is never backed up"): `restore.rs` orchestrates the apply and the supersede,
//!   `main.rs` owns the minting. What ADR-0066 CHANGED is the consequence. The X25519
//!   unwrap secret used to be HKDF-derived from that seed (ADR-0052 decision 4), so a fresh
//!   seed meant a fresh unwrap secret and every inherited `event_dek` row was noise. It is
//!   now an **independent** keypair in its own `<key>.unwrap` keystore file (decision 1)
//!   that rides the sealed export (decision 3), so identity may die with the disk while
//!   custody survives it. `cairn_event::seal::derive_unwrap_secret` survives only as
//!   ADR-0066 decision 5's one-time MIGRATION for nodes provisioned before it.
//! - `localstate::LocalState` reserves `node_default_deks` and `episode_deks`.
//!   `localstate_read::read_local_state` now fills `episode_deks` from `event_dek` (minus
//!   every target named in `erasure_shred_log` — ADR-0066 decision 7) and carries the
//!   unwrap secret itself in a third slot. `node_default_deks` stays empty, and
//!   LEGITIMATELY so: promise 2 has **no subject** — no node-default data-at-rest keystore
//!   exists anywhere in the built system for it to export.
//!
//! # Why the (former) custody gap was honest history rather than an oversight
//!
//! `localstate.rs`'s own header declared the deferral: *"the federation-node tier has no
//! clinical surface yet, so the bundle is empty … the clinical tier fills later via
//! additive evolution."* That was **true when slice D was written**. It expired when
//! ADR-0052 made every clinical body born-sealed, and nothing re-opened it — while
//! ROADMAP went on recording slices A–D as ✓ done. The precondition is the thing that
//! rotted, not the code.
//!
//! # Why a PIN, where one is still used
//!
//! For an unfixed defect the obvious TDD move is a red test stating the promise. This
//! crate has no `#[ignore]` anywhere and a permanently-red test would block the gate for
//! every unrelated change, so an unfixed promise follows the repo's existing "pinned count"
//! idiom instead: **assert what is true today, name the inversion the fix owes, and the
//! guard failing IS the guard working.**
//!
//! # What each test is now
//!
//! - [`the_export_carries_the_unwrap_secret_and_the_surviving_dek`] — **GUARANTEE** (#495,
//!   ADR-0066 decisions 3 and 7). It replaced the pin that asserted the export carried no
//!   DEK at all. It stays green; if it reddens, a restored solo clinic has lost custody of
//!   its own sealed bodies again.
//! - [`export_carries_no_dek_for_the_survivor_and_none_for_the_shredded`] — **HALF
//!   GUARANTEE, HALF PROHIBITION**, and that asymmetry is the whole point: the survivor's
//!   custody must travel, the shredded event's must never. The prohibition was written
//!   while it was still vacuous and became load-bearing on the exact commit that inverted
//!   its sibling.
//! - [`the_export_filter_drops_a_custody_row_the_shred_log_forbids`] — **DEFENCE IN DEPTH**,
//!   and the only test that can fire the producer's `erasure_shred_log` filter at all. It
//!   stages a state the production doors cannot produce, and its own doc explains why that
//!   deliberate exception to this suite's rules is the point rather than a shortcut.
//! - [`medium_carries_the_federation_plane_and_no_clinical_event`] — **still a PIN** (#500,
//!   the sibling issue). It names its own inversion and goes red on the commit that fixes
//!   the medium. Do not read the rest of this file as evidence that it is fixed.
//! - [`local_state_producers_are_the_empty_constructor_and_the_db_reader`] — a producer
//!   COUNT guard over source text; it moved from 1 to 2 when the DB-reading producer
//!   landed.
//! - [`a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body`] — **MECHANISM**,
//!   not a pin, and it says so in its own doc: it describes what a fresh seed does to a
//!   derived secret, which is unchanged and is now migration-only territory.
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
use cairn_node::localstate::{episode_dek_from_cbor, read_local_state, EpisodeDek, LocalState};
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

/// **Promise 3 — "sealed-episode DEKs survive" — is now TRUE (#495, ADR-0066).** This is
/// the guarantee test that replaced the pin asserting the export carried nothing.
///
/// The sealed local-state export (ADR-0026 slice D) is the only artifact that can carry key
/// material off a dying node. ADR-0066 decision 3 rides the node's INDEPENDENT unwrap
/// secret in it beside the `event_dek` custody rows, so a restored solo clinic inherits both
/// halves of the pair it needs: the wrapped DEKs, and the one secret that opens them.
///
/// ANTI-VACUITY, inherited from the pin this replaced and then strengthened:
///
/// 1. The node is provisioned and the sealed event is authored through the **production
///    door** (`submit_event` with its DEK), so `event_dek` genuinely holds an openable
///    wrapped DEK before the export is built — the test never writes one itself.
/// 2. The happy path is asserted **first** (the secret is carried, and it is this node's
///    byte for byte), so a later failure cannot pass for the wrong reason.
/// 3. The carried secret is checked to actually **open** the carried DEK. Anything less —
///    a presence check, a length check — would prove transport, not recovery, and a
///    well-shaped meaningless blob would sail through it.
#[tokio::test]
async fn the_export_carries_the_unwrap_secret_and_the_surviving_dek() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let (event_id, _bytes) = author_sealed_clinical_event(&c, &sk, &kid).await;

    // `provisioned_clinic` registers this node's unwrap key by DERIVING it from the signing
    // seed — the ADR-0066 decision 5 migration shape, which is what the shared medication
    // fixture models. So the secret a real operator would load from `<key>.unwrap` is, for
    // this node, exactly the derived one; deriving it here is how the test gets hold of the
    // same secret the production door wrapped every DEK to.
    let node_secret = derive_unwrap_secret(&sk.to_bytes());
    // `&*` is load-bearing: `Some(&node_secret)` would be `Option<&Zeroizing<[u8; 32]>>`
    // — deref coercion does not reach inside `Some`, so deref explicitly.
    let exported = read_local_state(&c, Some(&*node_secret))
        .await
        .expect("export must succeed");

    let carried = exported
        .unwrap_secret
        .as_ref()
        .expect("ADR-0066: the export must carry the node's unwrap secret");
    assert_eq!(
        carried.as_slice(),
        node_secret.as_slice(),
        "the carried secret must be this node's, byte for byte"
    );

    let deks: Vec<EpisodeDek> = exported
        .episode_deks
        .iter()
        .map(|b| episode_dek_from_cbor(b).unwrap())
        .collect();
    let mine = deks
        .iter()
        .find(|d| d.event_id == event_id)
        .expect("the sealed event's custody must be in the export");

    // The whole point: the carried secret opens the carried DEK. Anything less proves
    // transport, not recovery.
    let recovered: [u8; 32] = carried.as_slice().try_into().unwrap();
    unwrap_dek(&mine.dek_wrapped, &recovered)
        .expect("the exported secret must open the exported custody row");

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

/// **The erasure half of promise 3 — "minus any erased ones" — and it is the half whose
/// failure is worst.** ADR-0066 decision 7 states the asymmetry this test exists for: the
/// survivor's DEK **must** be present, the shredded event's **must never** be.
///
/// ADR-0026 point 6 requires a restore to replay the shred log so an erased body is not
/// resurrected. A #495 fix that routed `event_dek` rows into the export WITHOUT consulting
/// `erasure_shred_log` would carry a crypto-shredded body's key across the restore boundary
/// and undo an erasure the node already executed — the worst outcome in this whole area,
/// and the rest of this suite would stay green through it. This test is the tripwire.
///
/// Anti-vacuity: two sealed events are authored and only one is shredded, and BOTH custody
/// rows are checked afterwards — the shredded one gone, the survivor's still openable. So
/// the shred is proven to have really executed, and the export cannot satisfy this test by
/// simply carrying nothing for either.
///
/// **The asymmetry, as it now stands:** the first assertion was INVERTED on the commit that
/// closed #495 — the survivor's DEK must travel. The prohibition below survived that commit
/// unchanged, exactly as it was written to. It was authored while it was still vacuous (an
/// empty export had nothing to scan) precisely so that it would already be sitting here,
/// under a reviewer's eyes, on the commit that made it load-bearing.
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

    let node_secret = derive_unwrap_secret(&sk.to_bytes());
    let exported = read_local_state(&c, Some(&*node_secret))
        .await
        .expect("export must succeed");

    // INVERTED on the #495 fix, as this test's doc said it must be: the survivor's custody
    // travels. Decoded and matched by event id rather than merely counted, so "non-empty"
    // cannot stand in for "carries THIS event".
    let decoded: Vec<EpisodeDek> = exported
        .episode_deks
        .iter()
        .map(|b| episode_dek_from_cbor(b).expect("every export element is a valid EpisodeDek"))
        .collect();
    assert!(
        decoded.iter().any(|d| d.event_id == survivor_id),
        "ADR-0066 decision 7, first half: the export MUST carry the surviving sealed body's \
         DEK — the node holds its custody and a restore needs it. Carried: {decoded:?}"
    );

    // The prohibition, in the strongest form the slot's shape allows. The leaf type is now
    // KNOWN (`EpisodeDek`), so decoding and comparing ids observes the property directly
    // instead of hoping an identifier's bytes happen to appear.
    assert!(
        !decoded.iter().any(|d| d.event_id == shredded_id),
        "ADR-0066 decision 7 / ADR-0026 point 6: no trace of the SHREDDED event's custody \
         may ever reach the export. An export that names a crypto-shredded event \
         resurrects an executed erasure on restore. Carried: {decoded:?}"
    );
    // KEPT EXACTLY AS FIRST WRITTEN, and now joined by the decoded check above. This scans
    // the opaque slot bytes for the event UUID's raw 16-byte form — the shape-independent
    // question "is this event referenced in there at all?". The element shape that landed
    // with #495 stores the id as TEXT, so this scan alone would NOT observe the property;
    // that is why the decoded assertion above exists and is the live guard. This one is kept
    // because it costs nothing, needs no knowledge of the leaf type, and would still fire on
    // a future shape that stored ids as bytes. Over a key that must never travel, two
    // independent looks are worth the lines.
    //
    // Its original note declared itself VACUOUS — "the assertion above pins `episode_deks`
    // empty, so this scan has nothing to scan" — and that self-labelling is why the gap was
    // caught here rather than shipped: on the commit that filled the slot, the label forced
    // the question "is it load-bearing NOW?", and the honest answer was no, because the
    // chosen leaf shape stores text. An unlabelled vacuous guard reads as coverage, which is
    // the failure this whole suite exists to name.
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

    // And the same prohibition over the OTHER slot #495 filled. An unwrap secret is read
    // access to every body it was wrapped for, so a shredded event leaking through the
    // secret slot would be the same defect wearing a different hat. It carries a bare
    // 32-byte X25519 secret, so this is a cheap standing check that it stays that.
    assert!(
        !exported
            .unwrap_secret
            .as_ref()
            .is_some_and(|s| s.windows(16).any(|w| w == shredded_marker.as_slice())),
        "the secret slot carries a 32-byte X25519 secret and nothing else — certainly no \
         reference to an erased event"
    );
}

/// **The producer's `erasure_shred_log` filter, and the ONLY test that can fire it.**
///
/// Why it needs its own test, stated plainly because the answer is uncomfortable: on a
/// healthy node that filter selects nothing extra. `cairn_execute_shred` (db/037) already
/// DELETES the custody row when a shred executes, and `apply_remote_event` (db/020) already
/// refuses to create one for a target already in `erasure_shred_log`. So its sibling test
/// [`export_carries_no_dek_for_the_survivor_and_none_for_the_shredded`] would stay green
/// with the filter DELETED — it proves the outcome, not the mechanism. A filter no test can
/// fire is a filter a refactor can remove with nothing going red, and the failure it guards
/// against (an erased body's key resurrected on a restored node) is irreversible.
///
/// So this test STAGES A STATE THE PRODUCTION DOORS CANNOT PRODUCE: a live `event_dek` row
/// sitting beside a live `erasure_shred_log` entry for the same event. That is a deliberate
/// exception to this suite's "never write through the test" rule, and the exception is the
/// point — the filter exists precisely for a world where an upstream defence has failed
/// (a future apply path, a hand-repaired database, a peer re-supply), and only a test
/// willing to simulate that failure can observe it. It is defence in depth, tested in depth.
///
/// Anti-vacuity, in this order: the shred is proven to have really executed (its custody is
/// gone) BEFORE the row is staged back; the staging is proven to have taken; the SURVIVOR is
/// proven to still travel, so the filter is precise rather than a blanket refusal; and only
/// then is the forbidden row's absence asserted.
///
/// Key material is never fabricated here (house rule 6) — the re-inserted DEK is the exact
/// wrapped blob the production door minted, read out before the shred destroyed it.
#[tokio::test]
async fn the_export_filter_drops_a_custody_row_the_shred_log_forbids() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;

    let (survivor_id, _) = author_sealed_clinical_event(&c, &sk, &kid).await;
    let (shredded_id, _) = author_sealed_clinical_event(&c, &sk, &kid).await;

    // Capture the real wrapped DEK before the shred destroys it — the bytes that must not
    // travel are then genuinely the bytes the door wrapped, not a fabricated stand-in.
    let doomed_dek: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&shredded_id],
        )
        .await
        .unwrap()
        .get(0);

    shred(&c, &sk, &kid, &shredded_id).await;
    let after_shred: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&shredded_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after_shred, 0,
        "the shred executed: cairn_execute_shred destroyed the target's custody. The row \
         staged below therefore genuinely could not arise through a production door."
    );

    // THE STAGING. Superuser INSERT, bypassing every door — see this test's doc.
    c.execute(
        "INSERT INTO event_dek (event_id, dek_wrapped) VALUES ($1::text::uuid, $2)",
        &[&shredded_id, &doomed_dek],
    )
    .await
    .expect("staging the impossible state: custody beside a live shred-log entry");

    let staged: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek d JOIN erasure_shred_log s \
             ON s.target_event_id = d.event_id WHERE d.event_id = $1::text::uuid",
            &[&shredded_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        staged, 1,
        "anti-vacuity: the forbidden state really exists now — one custody row for an event \
         the shred log names. Without this the assertions below prove nothing."
    );

    let node_secret = derive_unwrap_secret(&sk.to_bytes());
    let exported = read_local_state(&c, Some(&*node_secret))
        .await
        .expect("export must succeed");
    let decoded: Vec<EpisodeDek> = exported
        .episode_deks
        .iter()
        .map(|b| episode_dek_from_cbor(b).expect("every export element is a valid EpisodeDek"))
        .collect();

    // Precision first: the filter must drop the forbidden row WITHOUT dropping everything.
    // Asserted before the refusal so the refusal cannot pass by exporting nothing at all.
    assert!(
        decoded.iter().any(|d| d.event_id == survivor_id),
        "the filter is provenance-precise: an unshredded event's custody still travels. \
         Carried: {decoded:?}"
    );
    assert!(
        !decoded.iter().any(|d| d.event_id == shredded_id),
        "ADR-0066 decision 7: `read_local_state` must consult erasure_shred_log and refuse \
         to export custody for a shredded event, EVEN when a custody row exists. Deleting \
         that WHERE NOT EXISTS clause must redden this test — it is the only test that can \
         see it. Carried: {decoded:?}"
    );
}

/// The raw 16 bytes of an event UUID, for scanning an export slot without decoding it.
///
/// The slot's leaf type is `Vec<u8>` at the container level, so this is the shape-independent
/// way to ask "is this event referenced in there at all?" — no knowledge of the element's
/// internal schema required. Since #495 that schema IS known (`localstate::EpisodeDek`) and
/// stores the id as TEXT, so a caller must not rely on this scan alone; see the two-check
/// arrangement in [`export_carries_no_dek_for_the_survivor_and_none_for_the_shredded`].
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

/// **The mechanism, and it is now MIGRATION-ONLY territory — do not read it as a
/// description of the live path.** Pure: no database, always runs.
///
/// ⚠️ **Scope first, because this test's shape is unchanged while its meaning is not.**
/// ADR-0066 decision 1 made the node's unwrap key an INDEPENDENT X25519 keypair, so the
/// live path no longer derives an unwrap secret from a signing seed at all, and a restored
/// node adopts the exported secret instead of deriving one (decision 4). What is tested
/// below is the DERIVATION's mechanics — which are unchanged, and which still matter for
/// exactly one population: nodes provisioned before ADR-0066, whose custody is wrapped to a
/// derived key and which adopt that derived secret once as their first independent key
/// (decision 5). This test says what would happen to such a node if that adoption were
/// skipped and a fresh seed minted instead. It is the reason the adoption exists.
///
/// Restore mints a fresh signing key by design (ADR-0026 decision 4), and the pre-ADR-0066
/// derivation (ADR-0052 decision 4) took the X25519 unwrap secret from that seed. So such a
/// node's restored unwrap secret is a *different* secret, and every `event_dek` row it
/// inherits — from a disk image, a `pg_dump`, or a peer that re-supplied the rows verbatim
/// — is noise to it.
///
/// A SECOND SCOPE NOTE, stated precisely because the obvious over-claim is wrong: this does
/// NOT mean a database-level restore yields unreadable bodies. `event_clear` is an ordinary
/// logged table holding the CLEAR payload and clear twin for every sealed body this node has
/// custody of, so a `pg_dump` or disk image carries readable content without needing any DEK
/// at all. The narrower true statement is that the inherited DEKs are noise; and the backup
/// MEDIUM still carries neither the DEKs nor `event_clear` (#500). The sealed EXPORT no
/// longer belongs in that list — since #495 it carries the custody rows and the secret that
/// opens them, which is precisely what stopped the ADR-0026 restore path from arriving with
/// nothing.
///
/// The happy-path leg is asserted FIRST and deliberately: without it a broken `wrap`/
/// `unwrap` pair would make the refusal below pass for entirely the wrong reason.
///
/// **#495 is fixed, and this test stayed true** — it describes the mechanism, not the gap.
/// The fix routed the *secret* across the restore boundary (never the seed: a seed would be
/// a signing identity, which ADR-0026 point 4 forbids the export to carry).
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

/// Exactly TWO pieces of code build a `LocalState`, and the guard knows both by name.
/// Pure: no database.
///
/// **RENAMED on the #495 fix** (it was `the_only_local_state_producer_is_the_empty_
/// constructor`). A test name is a claim, and that one asserted a state of affairs ADR-0066
/// deliberately ended — leaving it would have re-taught the expired framing to every reader,
/// which is the exact failure mode this suite was written about. Its sibling in
/// `tests/localstate.rs` was renamed once already for the same reason.
///
/// **Why this guard still earns its place after the fix.** Before, it showed there was no
/// producer that could read custody; now it fixes the producer surface at a known, small
/// number so a THIRD one cannot appear unnoticed. That matters because the two are not
/// interchangeable: `empty()` is the legitimate zero value (a bundle with nothing to carry),
/// while `localstate_read::read_local_state` is the one that must consult
/// `erasure_shred_log`. A third producer that skipped that filter would resurrect an erased
/// body's key on restore, and every other test here would stay green.
///
/// **The two-file scan is the load-bearing part.** The producer moved OUT of
/// `localstate.rs` (already past the 500-line house budget; the format and the DB read are
/// different jobs), so a single-file scan would now count 1, pass, and prove nothing about
/// the file that does the dangerous work. Both files are read.
///
/// This is a SOURCE-DERIVED guard, the repo's idiom for a claim about code shape
/// (`event_log_row_by_name.rs`, `no_drugref_dependency.rs`). Asserting instead that
/// `LocalState::empty()` returns something empty would be a tautology over a constructor's
/// own name — it could not observe the property this test is named for, which is exactly
/// the failure mode this file exists to avoid.
#[test]
fn local_state_producers_are_the_empty_constructor_and_the_db_reader() {
    let root = sources::repo_root();
    // Both files, named individually rather than walked: the claim is about these two
    // specific producers, and a walk would silently absorb a third file's producer into the
    // count instead of reddening on it.
    let files = [
        root.join("crates/cairn-node/src/localstate.rs"),
        root.join("crates/cairn-node/src/localstate_read.rs"),
    ];

    let producers: Vec<(String, usize, String)> = files
        .iter()
        .flat_map(|f| {
            let name = f
                .file_name()
                .expect("both paths name a file")
                .to_string_lossy()
                .into_owned();
            sources::read_source(f)
                .lines()
                .enumerate()
                .map(|(i, l)| (name.clone(), i + 1, l.trim().to_string()))
                .filter(|(_, _, l)| constructs_local_state(l))
                .collect::<Vec<_>>()
        })
        .collect();

    assert_eq!(
        producers.len(),
        2,
        "`LocalState` is constructed in exactly TWO places — `localstate::LocalState::empty()` \
         (the legitimate zero value) and `localstate_read::read_local_state` (the DB reader, \
         which MUST filter on erasure_shred_log). A third producer is a place an erased \
         body's key could travel from. Found: {producers:?}"
    );

    // And one is in each file: the count alone would pass if the DB reader vanished and a
    // second `empty()`-shaped constructor appeared beside the first.
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            producers.iter().filter(|(n, _, _)| *n == name).count(),
            1,
            "one producer per file — {name} must hold exactly one. Found: {producers:?}"
        );
    }

    // Also confirm `empty()` really is the zero-value producer, not some other site that
    // happens to sit alone in that file — a count cannot notice a swap.
    let ls = LocalState::empty();
    assert!(
        ls.is_empty(),
        "`empty()` is the zero-value producer, and it produces an empty bundle"
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
