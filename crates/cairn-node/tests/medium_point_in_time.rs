//! #500 slice 2c — **what a backup MEANS, pinned so nobody "fixes" it.**
//!
//! This file has exactly one test, and it is the most important one in the slice. It is not
//! here to catch a regression in a helper; it is here to stop a future session from
//! mistaking correct, deliberate behaviour for a leak and "repairing" it.
//!
//! # The behaviour
//!
//! A clinical body's wrapped DEK travels on the medium beside the body (design §2.1: custody
//! travels on BOTH paths, medium *and* export). The medium is append-only. So:
//!
//! | the body was crypto-shredded… | in a medium captured BEFORE the shred | in a medium captured AFTER |
//! |---|---|---|
//! | …after that capture | **the key is still there, correctly** | n/a |
//! | …before that capture | n/a | the key was never written |
//!
//! Read the top-left cell cold and it looks like an erasure that failed to propagate. It is
//! not. It is the definition of a backup. The maintainer's framing, which governs
//! (2026-09-05, recorded in
//! `docs/superpowers/specs/2026-09-04-dr-slice-2c-both-planes-captured-design.md` §2.1):
//!
//! > **A backup is only a backup if it can restore the state of the system at the time the
//! > backup was taken. Taking care of invalidated backups is a policy issue, not a core
//! > enforcement one. The core will only guarantee availability and integrity of data.**
//!
//! At the moment that medium was written, the body *was* readable on the live node. A medium
//! that quietly dropped the key later would be reporting a state the node was never in — and
//! it could only do so by rewriting a segment it had already signed, forfeiting the very
//! integrity guarantee the core exists to provide. **So: no future slice may "fix" this by
//! filtering old segments.** That sentence is why this file exists.
//!
//! # Where core ends, and what a practice must be told
//!
//! Erasure does not propagate backwards into media already written. Destroying a key on the
//! live node completes the erasure *there*; completing it across backups is **rotation** —
//! capture a fresh medium, destroy the old — and the rotation interval IS the maximum time an
//! erasure takes to complete across all copies. That number is the clinic's policy call, not
//! Cairn's: founding principle 9 (mechanism, never policy) and ADR-0005's *deletion is
//! best-effort and declared, never guaranteed*. Cairn's obligations are to make the residue
//! legible and to say this plainly; slice 2e's ADR owes the sentence in as many words.
//!
//! # Why this is a separate file
//!
//! `dr_clinical_guarantee_gap.rs` holds the pins that say what is NOT yet built, and
//! `backup_carries_both_planes.rs` holds the capture's guarantees. This is neither: it is a
//! statement about the SEMANTICS of the artifact, and it needs its own name in the test list
//! so that `cargo test --test medium_point_in_time` reads as the question it answers.
//!
//! DB-gated on `$CAIRN_TEST_PG`, the repo-wide pattern policed by `tests/db_gate_actually_ran.rs`.
//! Key material is derived at runtime by the production `seal_event_payload`/`generate_key`
//! paths, never a literal (house rule 6).

use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{MediumImage, MediumRecord};
use cairn_node::{backup, db, identity};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding, for `submit_registration` (since #345 a chart's first event must be its
// registration) and `medication_setup`, which owns the canonical truncation list.
mod common;

// ---------------------------------------------------------------------------
// Fixtures.
//
// Deliberately NOT shared with `dr_clinical_guarantee_gap.rs` or
// `backup_carries_both_planes.rs`, which carry near-identical helpers: integration-test
// binaries in this crate cannot `use` one another, and only `tests/common/mod.rs` crosses
// that boundary. Hoisting these there would grow the shared surface (and its hand-written
// mirror in `identity_scaffolding_shared.rs`) for three call sites; the copies are the
// cheaper of the two costs and this comment is the pointer between them.
// ---------------------------------------------------------------------------

/// A live database plus this node's signing identity, in the state a real solo clinic node is
/// in the moment before its disk dies.
///
/// `_guard` is held in a FIELD, not dropped into a loose `let _`: the DB-gated suites share
/// one PostgreSQL database and each truncates on entry, so the advisory-lock guard must stay
/// alive for the whole test.
struct Clinic {
    _guard: Client,
    db: Client,
    sk: SigningKey,
    kid: String,
    dir: tempfile::TempDir,
}

impl Clinic {
    /// One fixed medium path, because the point of this test is what the SECOND capture does
    /// to what the first one already wrote.
    fn medium(&self) -> std::path::PathBuf {
        self.dir.path().join("cairn.medium")
    }

    fn health(&self) -> std::path::PathBuf {
        self.dir.path().join("backup-status.json")
    }

    /// Parse the medium as it stands on disk right now. Insists on CAIRNB3: every capture
    /// here goes through `backup_to`, so a legacy image would mean the writer regressed.
    fn read_medium(&self) -> MediumImage {
        let bytes = std::fs::read(self.medium()).expect("the backup must have written a medium");
        let image =
            cairn_medium::parse_any(&bytes).expect("the medium `backup_to` wrote must parse");
        assert!(
            matches!(image, MediumImage::V3(_)),
            "a capture must leave a CAIRNB3 medium, never a legacy container"
        );
        image
    }
}

/// `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip.
async fn clinic() -> Option<Clinic> {
    let base = std::env::var("CAIRN_TEST_PG").ok()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
    // Delegated rather than reimplemented: `medication_setup` owns the canonical truncation
    // list, including the medication PROJECTION tables that have no FK to `event_log` and so
    // survive a `TRUNCATE … CASCADE` from it. It also registers this node's single unwrap
    // key, which is what lets the strict door wrap each sealed body's DEK into custody.
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(&c).await;
    identity::provision(&c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();
    Some(Clinic {
        _guard: guard,
        db: c,
        sk,
        kid,
        dir: tempfile::tempdir().unwrap(),
    })
}

/// Build a sealed `clinical.medication.asserted` body plus the DEK the strict door needs — a
/// real born-sealed body, so the custody this test watches is the one the production door
/// actually wrapped, never a fixture's stand-in.
fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Secret32) {
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
/// Returns its `event_id` (needed to shred it) and its signed bytes (how it is found on the
/// medium).
///
/// ANTI-VACUITY: reads the row back before returning. `submit_event`'s INSERT ends in `ON
/// CONFLICT DO NOTHING`, so "no error" is an invariant of a distant door rather than evidence
/// visible here.
async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> (String, Vec<u8>) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;

    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let (body, dek) = sealed_assert_body(kid, patient, hlc);
    let event_id = body.event_id.clone();
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
    .expect("a sealed body with its DEK is admitted");

    let landed: Vec<u8> = c
        .query_one(
            "SELECT signed_bytes FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("anti-vacuity: the event must genuinely be in event_log")
        .get(0);
    assert_eq!(
        landed, signed.signed_bytes,
        "the log holds the exact bytes this test will look for on the medium"
    );
    (event_id, signed.signed_bytes)
}

/// Submit an `erasure.shred.asserted` tombstone against `target` through the strict door, and
/// prove the shred actually EXECUTED (`cairn_execute_shred` destroys the custody row).
///
/// The tombstone is plaintext by design — it must outlive every key it names.
///
/// The post-condition is asserted HERE rather than at each call site because every assertion
/// this file makes about the medium is meaningless if the shred was a no-op: "the medium
/// still carries the key" would then be a statement about a shred that never happened.
async fn shred(c: &Client, sk: &SigningKey, kid: &str, target: &str) {
    // `::text` on the read: this crate does not enable tokio-postgres's `uuid` feature, so a
    // UUID column cannot be decoded directly — cast it in SQL and carry it as a String, the
    // repo-wide read idiom.
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

    let live_custody: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        live_custody, 0,
        "anti-vacuity: the shred EXECUTED — cairn_execute_shred destroyed the live custody \
         row for {target}. Every claim below about what the medium does or does not carry \
         rests on this having really happened."
    );
}

/// The medium's record for the event with these signed bytes, if it carries one at all.
///
/// Returned as the whole record rather than just its `dek_wrapped`, because this test must
/// distinguish two very different facts that a bare `Option<dek>` would flatten into one:
/// *the body is on the medium with no key* (correct, for a body shredded before capture) and
/// *the body is not on the medium at all* (a capture defect).
fn record_for<'a>(image: &'a MediumImage, signed_bytes: &[u8]) -> Option<&'a MediumRecord> {
    match image {
        MediumImage::V3(m) => m
            .segments
            .iter()
            .flat_map(|s| s.records.iter())
            .find(|r| r.signed_bytes == signed_bytes),
        MediumImage::Legacy(_) => None,
    }
}

// ---------------------------------------------------------------------------
// The property.
// ---------------------------------------------------------------------------

/// **A medium reproduces the state of the node at the time it was captured. Both directions.**
///
/// ⚠️ **This test pins DELIBERATE behaviour. It is not a known bug, it is not a leak, and it
/// must not be "fixed".** Read this file's header — and design §2.1 — before changing a line
/// of it. The name says what it asserts on purpose: a name containing "leak" or "gap" would
/// invite exactly the repair that would break the guarantee.
///
/// Body **A** is authored, captured, and shredded *afterwards*. Its wrapped DEK stays in the
/// segment that was already written — and stays there across a LATER capture, which is the
/// assertion that actually bites: the only way to remove it would be to rewrite a segment
/// this node has already signed, forfeiting the integrity guarantee that is the core's job.
///
/// Body **B** is authored and shredded *before* its first capture. Its ciphertext still
/// travels (append-only: the event exists, and a tombstone does not unwrite it) but its key
/// was never written at all — db/051's `event_custody_surviving` filter, applied at capture
/// time, which is the same point-in-time semantic seen from the other side.
///
/// So the two halves are one property, not two: **capture-time state, faithfully.** A restore
/// reads a body if EITHER carrier still holds its key; the medium's half is a snapshot, and
/// the export's whole-file rewrite is the one that carries *current* custody (its job is the
/// long-lived unwrap secret, not point-in-time fidelity).
///
/// ANTI-VACUITY, and the order matters: A's presence WITH its key is asserted before the
/// shred (so the medium had something to keep); `shred` itself proves the live custody row was
/// destroyed (so "still on the medium" is a statement about the medium, not about a shred that
/// silently failed); and B's record is asserted PRESENT before its key is asserted absent (so
/// "no key" cannot be satisfied by a capture that dropped the body entirely).
#[tokio::test]
async fn a_medium_restores_the_state_at_capture_time() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };

    // --- A: readable at capture time, shredded afterwards -------------------
    let (id_a, bytes_a) = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    backup::backup_to(
        &cl.db,
        &cl.medium(),
        &cl.health(),
        1_700_000_000,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect("the first capture succeeds");

    // ANTI-VACUITY: at capture time A was readable, and the medium says so. Without this the
    // assertion after the shred would be about a medium that never held the key.
    let image = cl.read_medium();
    let a_at_capture =
        record_for(&image, &bytes_a).expect("A must be on the medium it was captured onto");
    assert!(
        a_at_capture.dek_wrapped.is_some(),
        "anti-vacuity: A's custody travelled onto the medium at capture time (slice 2c's own \
         guarantee) — everything below is about whether it STAYS there"
    );

    shred(&cl.db, &cl.sk, &cl.kid, &id_a).await;

    // --- B: shredded BEFORE it was ever captured ----------------------------
    let (id_b, bytes_b) = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    shred(&cl.db, &cl.sk, &cl.kid, &id_b).await;

    // The second capture appends B (and the two tombstones) to the medium that already holds
    // A. This is the run under test: a "fix" that filtered already-written segments would have
    // to act HERE, and the assertion about A below is what would catch it.
    backup::backup_to(
        &cl.db,
        &cl.medium(),
        &cl.health(),
        1_700_000_100,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect("the second capture succeeds");

    let image = cl.read_medium();

    // HALF 1 — A's key survives in the segment that was already written.
    let a_now = record_for(&image, &bytes_a)
        .expect("A's body must still be on the medium: the medium is append-only");
    assert!(
        a_now.dek_wrapped.is_some(),
        "the medium reproduces the state at capture time — a later shred does not reach a \
         segment already written, and rewriting one would forfeit the integrity guarantee \
         that is the core's job (design §2.1). This is NOT a leak and must not be 'fixed' by \
         filtering old segments: at the moment this medium was taken, A was readable on the \
         live node, and a backup that denied that would be reporting a state the node was \
         never in. Completing an erasure across backups is ROTATION — capture fresh, destroy \
         old — and that interval is the clinic's policy call, not the core's (principle 9, \
         ADR-0005)."
    );

    // HALF 2 — B was shredded before its first capture, so its key was never written.
    // Presence first, so the absence below is about the KEY and not about a missing body.
    let b_now = record_for(&image, &bytes_b).expect(
        "B's body must be on the medium: a crypto-shred destroys the KEY, never the event. \
         An append-only log that dropped the ciphertext would be unwriting history.",
    );
    assert!(
        b_now.dek_wrapped.is_none(),
        "a body shredded before its first capture never has its DEK written — db/051's \
         `event_custody_surviving` filter runs at capture time, so the medium records the \
         node's state as it stood: B was already unreadable. Same point-in-time semantic as \
         half 1, seen from the other side."
    );
}
