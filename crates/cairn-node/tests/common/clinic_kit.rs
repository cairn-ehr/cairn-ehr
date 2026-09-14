//! A solo clinic node for tests that drive the REAL `cairn-node` binary: a live database, the
//! node's signing identity written to a key FILE (which `--key` needs), and a temporary
//! directory standing in for the operator's backup volume.
//!
//! Moved out of `verify_backup_scope.rs` (#567) so a second suite can share it instead of
//! copying it — `tests/common/dead_node.rs` in `cairn-sync` set that convention: a fixture that
//! has already had two ways to make a test pass for the wrong reason should exist once.
//!
//! Include it with `#[path = "common/clinic_kit.rs"] mod clinic_kit;`. The including binary must
//! ALSO declare `mod common;` — `author_sealed_clinical_event` registers each chart through
//! `common::submit_registration` (#345: a chart's first event must be its registration).
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{MediumImage, MediumRecord, MediumV3, Plane};
use cairn_node::{db, identity, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Fixtures for DB-gated tests that drive the CLI. This file holds no tests itself: it is
// included by `#[path]` into BOTH `verify_backup_scope.rs` and `verify_backup_clinical_plane.rs`.
// Deliberately the same SHAPE as `backup_carries_both_planes.rs`'s `clinic()`, which does not
// include it — but this one writes the node's signing key to a FILE, because the including
// suites spawn the real `cairn-node` binary rather than calling `backup::backup_to` as a
// library function.
// ---------------------------------------------------------------------------

pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Everything one test needs: a live database, this node's signing identity (ALSO written
/// to a key file on disk, unlike the DB-only fixtures elsewhere in this crate), and a
/// temporary directory standing in for the operator's backup volume.
pub struct Clinic {
    pub _guard: Client,
    pub db: Client,
    pub base: String,
    pub sk: SigningKey,
    pub kid: String,
    pub dir: tempfile::TempDir,
}

impl Clinic {
    pub fn key(&self) -> std::path::PathBuf {
        self.dir.path().join("node.key")
    }
    pub fn medium(&self) -> std::path::PathBuf {
        self.dir.path().join("cairn.medium")
    }

    /// A `Command` for the freshly-built `cairn-node` binary, pointed at this fixture's
    /// database and key, with stdin nailed to `/dev/null`. That last part matters: an
    /// unattended run is exactly what several tests in the including suites simulate, and
    /// `resolve_passphrase` falls back to an interactive prompt when neither `--passphrase` nor
    /// `CAIRN_KEY_PASSPHRASE` supplies one — `rpassword::prompt_password` already fails fast
    /// on a non-tty (pinned elsewhere in this crate), and `Stdio::null()` guarantees that is
    /// what it always sees here, in every environment those suites ever run in.
    pub fn cli(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"));
        cmd.args(["--conn", &self.base, "--key"])
            .arg(self.key())
            .stdin(std::process::Stdio::null());
        cmd
    }
}

/// Bring the database to the state a real solo clinic node is in, and write that node's
/// identity to a key file `--key` can load. `None` when `$CAIRN_TEST_PG` is unset — the
/// repo-wide self-skip, policed by `tests/db_gate_actually_ran.rs`.
pub async fn establish_clinic() -> Option<Clinic> {
    let base = cs()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
    // The same truncation list `common::medication_setup` uses, minus the medication
    // projection tables the including suites never touch — they only need `event_log` rows to
    // exist, never their medication-specific shadow.
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&dir.path().join("node.key")).unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    identity::provision(&c, &sk, &kid, "solo-clinic", "127.0.0.1:7961")
        .await
        .unwrap();
    // ADR-0066 device-key-derived unwrap secret — the same test-fixture convention
    // `common::medication_setup` documents (a fixture has no `establish-unwrap-key`
    // ceremony to run, so it registers a deterministic secret reproducible from a key it
    // already holds). Only the PUBLIC half lands in the database HERE; the matching `.unwrap`
    // FILE is written by `write_existing_escrow`, which the tests that need a genuinely
    // restorable kit call. A test that does NOT call it gets a node with a registered public
    // half and no private one — which is a real state (an operator who never ran
    // `establish-unwrap-key`) and, since the Critical 2 fix, one whose export correctly
    // withholds coverage rather than reporting a green kit that opens nothing.
    let secret = cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()));
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret)
            .as_bytes()
            .as_slice()],
    )
    .await
    .unwrap();

    Some(Clinic {
        _guard: guard,
        db: c,
        base,
        sk,
        kid,
        dir,
    })
}

/// Establish a FULL disaster-recovery kit beside `key` — the on-disk effect of
/// `cairn-node establish-local-state-key` AND `establish-unwrap-key`, built directly rather
/// than by running those commands, so each test controls its own op-pass/recovery-code pair.
///
/// **Why the `.unwrap` file is written here and was not before (final review, Critical 2).**
/// This fixture used to write the `.lsk` sidecar alone, on the stated reasoning that a
/// missing unwrap key "degrades exactly the way an operator who has not run
/// `establish-unwrap-key` sees it (a warning, never a failure)" and that this suite tests kit
/// STALENESS rather than custody completeness. That separation does not exist: an export
/// carrying custody rows and no key to open them is not a kit whose staleness is worth
/// asking about — ADR-0066's whole point is that the key and the bytes are useless apart —
/// and `export_covers_seq` is the figure that says whether a restore would work.
///
/// While `backup` recorded coverage unconditionally, the difference was invisible and these
/// tests passed over a keyless export. `export_outcome_for_write` now withholds coverage for
/// exactly that artifact, so a fixture that means "a full kit" has to build one.
///
/// The secret written MUST be the one `establish_clinic` registered in `node_unwrap_key` —
/// `derive_unwrap_secret(sk)` — not a fresh random one: the registrar is a singleton that
/// refuses a differing key, so a mismatched file would fail for an unrelated reason.
pub fn write_existing_escrow(key: &std::path::Path, sk: &SigningKey, op: &str, code: &str) {
    let wraps = cairn_node::localstate::establish_lsk(op, code).unwrap();
    let bytes = cairn_node::localstate::serialize_sidecar(&wraps);
    cairn_node::fsio::atomic_write(
        &cairn_node::localstate::lsk_sidecar_path_for(key),
        &bytes,
        Some(0o600),
    )
    .unwrap();

    let secret = cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()));
    cairn_node::keystore::write_unwrap_sealed(
        &cairn_node::keystore::unwrap_key_path_for(key),
        &secret,
        op,
        code,
    )
    .unwrap();
}

/// Build, seal, sign and submit a `clinical.medication.asserted` event whose custody the
/// backup under test must carry. A real born-sealed body, not a hand-built row — matching
/// `backup_carries_both_planes.rs`'s fixture exactly, since this suite needs the same shape.
pub fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Secret32) {
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

/// Submit ONE real born-sealed clinical event (on a FRESH chart — a new registration each
/// call) through the strict door, and return its signed bytes.
///
/// ANTI-VACUITY: reads the row back out of `event_log` before returning, exactly like the
/// sibling fixture in `backup_carries_both_planes.rs` — `submit_event`'s INSERT ends in
/// `ON CONFLICT DO NOTHING`, so "no error" alone would not prove the event actually landed.
pub async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> Vec<u8> {
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
        .expect("anti-vacuity: the event must genuinely BE in event_log")
        .get(0);
    assert_eq!(landed, signed.signed_bytes);
    signed.signed_bytes
}

/// The CAIRNB3 image, or a panic naming what was found instead. Every suite that includes this
/// file backs up through `backup_to`/the `backup` CLI arm, so a `Legacy` here is a broken writer.
pub fn as_v3(image: &MediumImage) -> &MediumV3 {
    match image {
        MediumImage::V3(m) => m,
        MediumImage::Legacy(_) => {
            panic!("a capture must leave a CAIRNB3 medium, never a legacy container")
        }
    }
}

/// Every record the medium carries on the CLINICAL plane, in file order.
pub fn clinical_records(image: &MediumImage) -> Vec<&MediumRecord> {
    as_v3(image)
        .segments
        .iter()
        .filter(|s| s.plane == Plane::Clinical)
        .flat_map(|s| s.records.iter())
        .collect()
}
