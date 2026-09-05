//! Task 12 of #500 DR slice 2c — a restorable DR kit is TWO artifacts (the CAIRNB3 medium
//! and the sealed `CAIRNL1` export beside it), and this codebase has so far treated the
//! second one as optional in every way that matters. This file closes the two remaining
//! gaps in the export half:
//!
//! 1. **`kit_verdict`** (pure, in `cairn_node::backup`) decides whether the kit — medium
//!    PLUS export, together — is actually restorable, by diffing the export's last-achieved
//!    coverage against the medium's OWN newest clinical seq. It is pure so this decision is
//!    testable with no database, no medium and no CLI (Step 1 below).
//! 2. **`verify-backup` now refuses a kit `kit_verdict` calls anything but `Restorable`**,
//!    printing which case it is and the matching remedy (Steps 2 & the two CLI-level tests
//!    at the bottom of this file, which exercise the real binary end to end).
//! 3. **The export gets a read-after-write check**, mirroring what the medium has had since
//!    slice B (`refuse_unsound` in `backup_to`) — the export is the ONLY artifact carrying
//!    this node's custody key off the machine, and it never had one until now. This is also
//!    where `export_covers_seq` first starts advancing at all: before this task nothing
//!    ever wrote `ExportOutcome::Written`, so `kit_verdict` would have reported
//!    `ExportMissing` forever, on every node, however well its exports were actually going.
//!
//! **The deliberate asymmetry, pinned explicitly (Step 3 test, `backup_still_exits_...`).**
//! `backup` keeps warning and exiting 0 when the export is skipped: it already wrote a good
//! medium, and failing the command would page an operator over a success, and would make
//! the clinical capture depend on a passphrase an unattended cron run cannot supply — `M >
//! N`, an architecture defect (house rule 7). `verify-backup` is the cron HEALTH CHECK, and
//! it is the one that refuses. Both halves of that asymmetry are pinned here, not just the
//! refusal, because pinning only the refusal would let a future change re-couple them
//! without any test noticing.

use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{parse_any, MediumImage, MediumRecord, MediumV3, Plane};
use cairn_node::backup::{self, kit_verdict, KitVerdict};
use cairn_node::{db, identity, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding (`submit_registration` — #345 needs the first event on a chart to be
// its registration). Not `medication_setup`: this suite drives the real CLI binary against
// a signing key that must live in a FILE on disk, which `medication_setup`'s in-memory-only
// keypair cannot provide — see `establish_clinic` below for the from-scratch equivalent.
mod common;

// ---------------------------------------------------------------------------
// Step 1 (TDD): `kit_verdict` is a PURE function — testable with no database, no medium,
// and no CLI. These four are the failing tests the task brief specifies, written FIRST.
// (One function name is written in real snake_case rather than the brief's SCREAMING
// fragment: `warnings = "deny"` at the workspace root turns `non_snake_case` into a build
// failure, and `a_medium_with_clinical_events_and_NO_export_...` does not compile clean.)
// ---------------------------------------------------------------------------

#[test]
fn an_export_older_than_the_medium_is_stale() {
    assert_eq!(
        kit_verdict(Some(900), Some(500)),
        KitVerdict::ExportStale {
            medium_seq: 900,
            export_seq: Some(500)
        }
    );
}

#[test]
fn an_export_that_covers_the_medium_is_restorable() {
    assert_eq!(kit_verdict(Some(900), Some(900)), KitVerdict::Restorable);
    // Ahead is fine: the export is written after the capture in the same run.
    assert_eq!(kit_verdict(Some(900), Some(950)), KitVerdict::Restorable);
}

#[test]
fn a_medium_with_no_clinical_events_is_not_stale() {
    // A fresh node that has never written a clinical event has nothing uncovered. Calling
    // that stale would train an operator to ignore the one signal this adds.
    assert_eq!(kit_verdict(None, None), KitVerdict::Restorable);
}

#[test]
fn a_medium_with_clinical_events_and_no_export_is_missing_not_stale() {
    // Different remedies: "run backup with a passphrase" (an escrow that already works, just
    // behind) vs "recover the export" (no coverage was EVER achieved). The #502 lesson —
    // merging unreadable/absent into one class named a remedy that refuses while the file
    // exists — is exactly why these stay two variants rather than one.
    assert!(matches!(
        kit_verdict(Some(900), None),
        KitVerdict::ExportMissing(_)
    ));
}

// ---------------------------------------------------------------------------
// Fixtures for the DB-gated tests below. Deliberately the same SHAPE as
// `backup_carries_both_planes.rs`'s `clinic()` rather than shared with it (integration-test
// binaries in this crate cannot `use` another test binary's private helpers) — but this one
// writes the node's signing key to a FILE, because the tests below spawn the real
// `cairn-node` binary rather than calling `backup::backup_to` as a library function.
// ---------------------------------------------------------------------------

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Everything one test needs: a live database, this node's signing identity (ALSO written
/// to a key file on disk, unlike the DB-only fixtures elsewhere in this crate), and a
/// temporary directory standing in for the operator's backup volume.
struct Clinic {
    _guard: Client,
    db: Client,
    base: String,
    sk: SigningKey,
    kid: String,
    dir: tempfile::TempDir,
}

impl Clinic {
    fn key(&self) -> std::path::PathBuf {
        self.dir.path().join("node.key")
    }
    fn medium(&self) -> std::path::PathBuf {
        self.dir.path().join("cairn.medium")
    }

    /// A `Command` for the freshly-built `cairn-node` binary, pointed at this fixture's
    /// database and key, with stdin nailed to `/dev/null`. That last part matters: an
    /// unattended run is exactly what several tests below simulate, and `resolve_passphrase`
    /// falls back to an interactive prompt when neither `--passphrase` nor
    /// `CAIRN_KEY_PASSPHRASE` supplies one — `rpassword::prompt_password` already fails fast
    /// on a non-tty (pinned elsewhere in this crate), and `Stdio::null()` guarantees that is
    /// what it always sees here, in every environment this suite ever runs in.
    fn cli(&self) -> std::process::Command {
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
async fn establish_clinic() -> Option<Clinic> {
    let base = cs()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
    // The same truncation list `common::medication_setup` uses, minus the medication
    // projection tables this suite never touches — this file only needs `event_log` rows to
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
    // already holds). Only the PUBLIC half lands in the database here; no `.unwrap` FILE is
    // ever written to `dir`, so the export step under test degrades exactly the way an
    // operator who has not run `establish-unwrap-key` sees it (a warning, never a failure —
    // this suite is testing kit STALENESS, not custody completeness).
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

/// Establish a local-state escrow beside `key` — the on-disk effect of
/// `cairn-node establish-local-state-key`, built directly rather than by running that
/// command, so each test controls its own op-pass/recovery-code pair.
fn write_existing_escrow(key: &std::path::Path, op: &str, code: &str) {
    let wraps = cairn_node::localstate::establish_lsk(op, code).unwrap();
    let bytes = cairn_node::localstate::serialize_sidecar(&wraps);
    cairn_node::fsio::atomic_write(
        &cairn_node::localstate::lsk_sidecar_path_for(key),
        &bytes,
        Some(0o600),
    )
    .unwrap();
}

/// Build, seal, sign and submit a `clinical.medication.asserted` event whose custody the
/// backup under test must carry. A real born-sealed body, not a hand-built row — matching
/// `backup_carries_both_planes.rs`'s fixture exactly, since this suite needs the same shape.
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

/// Submit ONE real born-sealed clinical event (on a FRESH chart — a new registration each
/// call) through the strict door, and return its signed bytes.
///
/// ANTI-VACUITY: reads the row back out of `event_log` before returning, exactly like the
/// sibling fixture in `backup_carries_both_planes.rs` — `submit_event`'s INSERT ends in
/// `ON CONFLICT DO NOTHING`, so "no error" alone would not prove the event actually landed.
async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> Vec<u8> {
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

/// The CAIRNB3 image, or a panic naming what was found instead. Every test in this file
/// backs up through `backup_to`/the `backup` CLI arm, so a `Legacy` here is a broken writer.
fn as_v3(image: &MediumImage) -> &MediumV3 {
    match image {
        MediumImage::V3(m) => m,
        MediumImage::Legacy(_) => {
            panic!("a capture must leave a CAIRNB3 medium, never a legacy container")
        }
    }
}

/// Every record the medium carries on the CLINICAL plane, in file order.
fn clinical_records(image: &MediumImage) -> Vec<&MediumRecord> {
    as_v3(image)
        .segments
        .iter()
        .filter(|s| s.plane == Plane::Clinical)
        .flat_map(|s| s.records.iter())
        .collect()
}

// ---------------------------------------------------------------------------
// Step 3: the asymmetry. `backup` must NOT start failing when the export is skipped.
// ---------------------------------------------------------------------------

/// Deliberate asymmetry, and the §1.2 constraint in test form: `backup` wrote a good medium,
/// and failing it would page an operator over a success — and would make the clinical
/// capture depend on a passphrase an unattended cron run cannot supply, which is `M > N`
/// (house rule 7, an architecture defect). `verify-backup` is the cron HEALTH CHECK, and it
/// is the one that refuses (see the two tests at the bottom of this file).
#[tokio::test]
async fn backup_still_exits_zero_when_the_export_is_skipped() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    std::env::remove_var("CAIRN_KEY_PASSPHRASE");
    let health_path = backup::health_path_for(&cl.key());
    let report = backup::backup_to(&cl.db, &cl.medium(), &health_path, 1_700_000_000, None).await;
    assert!(
        report.is_ok(),
        "a passphrase-less capture must still write the medium"
    );
    let image = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap();
    assert!(
        !clinical_records(&image).is_empty(),
        "and it must still carry the clinical plane"
    );
}

// ---------------------------------------------------------------------------
// Steps 2 & 4, end to end via the real binary: `verify-backup`'s new refusal, and the
// export read-after-write that makes `export_covers_seq` trustworthy enough to drive it
// (before this task nothing ever advanced it past `None` at all).
// ---------------------------------------------------------------------------

/// The realistic disaster this task exists to make visible: tonight's medium beside a
/// weeks-old export. A first backup WITH a passphrase leaves the kit `Restorable`; a second
/// clinical event lands and a second backup runs WITHOUT one (the unattended-cron shape) —
/// the medium's watermark advances, the export's does not, and `verify-backup` must now
/// refuse where it used to print a green "sealed export present" line forever.
#[tokio::test]
async fn verify_backup_is_restorable_then_refuses_once_the_export_falls_behind() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    write_existing_escrow(&cl.key(), "op-pass", "REC-CODE");

    // First backup, WITH a passphrase: medium and export are written in the SAME run, so
    // the export covers exactly what the medium holds.
    let out = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .args(["--passphrase", "op-pass"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the first backup must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("local-state exported"),
        "the export must actually have been written this run: stdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );

    let v1 = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        v1.status.success(),
        "a fresh, fully-covered kit must verify; stderr:\n{}",
        String::from_utf8_lossy(&v1.stderr)
    );

    // A second clinical event lands, and this run's backup cannot seal a fresh export — a
    // WRONG passphrase (rather than an omitted one), the same technique
    // `cli_localstate.rs::backup_degrades_when_the_export_cannot_be_sealed` already uses:
    // `build_export_container` fails deterministically trying to unwrap the LSK under it,
    // with no interactive prompt in play (`resolve_passphrase` never reaches one when a
    // non-empty `--passphrase` was given at all). This stands in for the real unattended-cron
    // shape — no passphrase available — without this test depending on how `rpassword`
    // behaves when it cannot find a controlling terminal.
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let out2 = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .args(["--passphrase", "wrong-op"])
        .env_remove("CAIRN_KEY_PASSPHRASE")
        .output()
        .unwrap();
    assert!(
        out2.status.success(),
        "the medium capture must still succeed even though the export cannot be sealed \
         (§1.2); stderr:\n{}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out2.stderr).contains("local-state export skipped"),
        "and it must say so: stderr:\n{}",
        String::from_utf8_lossy(&out2.stderr)
    );

    let v2 = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        !v2.status.success(),
        "a kit whose export has fallen behind the medium must fail the health check"
    );
    let stderr2 = String::from_utf8_lossy(&v2.stderr);
    assert!(
        stderr2.contains("STALE"),
        "the refusal must name the STALE case, not merely fail: {stderr2}"
    );
}

/// The OTHER failure mode `kit_verdict` distinguishes: clinical events exist and NO export
/// has EVER covered them (no escrow was ever established — `EscrowRead::Absent`, which used
/// to be non-fatal because this command "could not tell" a legitimate absence from a lost
/// custody channel). Task 12 gives it a way to tell: the medium DOES carry clinical events,
/// so an export that has never once recorded coverage can no longer be waved through.
#[tokio::test]
async fn verify_backup_refuses_when_clinical_events_have_no_export_at_all() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // No `.lsk` escrow ever established. `backup` degrades honestly (warns, exits 0) and
    // writes no export sibling at all.
    let out = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .env_remove("CAIRN_KEY_PASSPHRASE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the medium capture must succeed even with no escrow at all; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !cairn_node::localstate::localstate_path_for(&cl.medium()).exists(),
        "no export sibling should exist in this scenario"
    );

    let v = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        !v.status.success(),
        "clinical events with NO export at all must now fail the health check (Task 12) — \
         this used to be the case this command \"could not tell apart\" from a legitimate \
         escrow-less node, and stayed non-fatal; `kit_verdict` can now tell, because the \
         medium DOES carry clinical events"
    );
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        stderr.contains("INCOMPLETE"),
        "the refusal must be reachable and distinct from the STALE case: {stderr}"
    );
}
