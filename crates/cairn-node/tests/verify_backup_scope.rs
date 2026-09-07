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
            export_seq: 500
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
// Fix round 1, Minor 3: `clinical_watermark_of` supplies HALF of every `kit_verdict` call
// and had no test of its own — the DB-gated end-to-end tests exercised it only implicitly.
// The two `None` cases are pure (no database, no CLI); the `Some` case needs a real signed
// CAIRNB3 medium and lives with the other DB-gated tests further down this file.
// ---------------------------------------------------------------------------

#[test]
fn clinical_watermark_of_a_legacy_medium_is_none() {
    // CAIRNB1/CAIRNB2 predate the clinical plane entirely — there is nothing to have a
    // watermark over, so this must be the honest `None`, never a guessed `Some(0)`.
    let legacy = MediumImage::Legacy(cairn_medium::Container {
        self_marker: None,
        events: vec![],
    });
    assert_eq!(backup::clinical_watermark_of(&legacy), None);
}

#[test]
fn clinical_watermark_of_a_v3_medium_with_no_segments_is_none() {
    let empty_v3 = MediumImage::V3(MediumV3 {
        segments: vec![],
        truncated_tail: false,
        complete_bytes: 0,
    });
    assert_eq!(backup::clinical_watermark_of(&empty_v3), None);
}

// ---------------------------------------------------------------------------
// Fix round 1, Important 1: a health sidecar's `medium_path` must actually name the medium
// under test, or its coverage figure describes a DIFFERENT artifact — the two-drive
// rotation false green the reviewer traced through by hand. These are the pure path-compare
// tests; the CLI-level rotation scenario lives with the other DB-gated tests below.
// ---------------------------------------------------------------------------

#[test]
fn health_describes_medium_is_true_for_the_identical_path() {
    let dir = tempfile::tempdir().unwrap();
    let medium = dir.path().join("cairn.medium");
    std::fs::write(&medium, b"stand-in bytes").unwrap();
    assert!(backup::health_describes_medium(
        &medium.display().to_string(),
        &medium
    ));
}

#[test]
fn health_describes_medium_canonicalizes_a_relative_path_against_an_absolute_one() {
    let dir = tempfile::tempdir().unwrap();
    let medium = dir.path().join("cairn.medium");
    std::fs::write(&medium, b"stand-in bytes").unwrap();
    // A REAL subdirectory, so walking into it and back out via `..` is something
    // `std::fs::canonicalize` (an actual filesystem walk) can resolve — a fictional
    // intermediate segment would make canonicalization fail on both sides and fall back to
    // the literal compare, proving nothing about the canonicalizing branch at all.
    // `PathBuf`'s own `Eq` does NOT resolve `..` (that needs the filesystem), so the two
    // paths below are genuinely different as plain values — canonicalization is the only
    // thing that can see they name the same file.
    let sibling_dir = dir.path().join("sibling");
    std::fs::create_dir(&sibling_dir).unwrap();
    let via_parent = sibling_dir.join("..").join("cairn.medium");
    assert_ne!(
        medium, via_parent,
        "the two paths must differ as PLAIN values, or this test proves nothing beyond Eq"
    );
    assert!(backup::health_describes_medium(
        &medium.display().to_string(),
        &via_parent
    ));
}

#[test]
fn health_describes_medium_is_false_for_a_genuinely_different_medium() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("drive-a.medium");
    let b = dir.path().join("drive-b.medium");
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    // The two-drive rotation, distilled: a sidecar recorded coverage for `a`; `--from` names
    // `b`. Neither canonicalizes to the other, so this must be `false`.
    assert!(!backup::health_describes_medium(
        &a.display().to_string(),
        &b
    ));
}

#[test]
fn health_describes_medium_falls_back_to_a_literal_compare_when_the_recorded_medium_is_gone() {
    // The recorded medium no longer exists at that path (moved, deleted, or simply never
    // existed on THIS machine) — canonicalization fails, so this can only fall back to a
    // literal string compare. Still correctly says "different" for two distinct paths.
    let from = std::path::Path::new("/tmp/does-not-exist/cairn.medium");
    assert!(!backup::health_describes_medium(
        "/tmp/also-does-not-exist/other.medium",
        from
    ));
    assert!(backup::health_describes_medium(
        "/tmp/does-not-exist/cairn.medium",
        from
    ));
}

// ---------------------------------------------------------------------------
// Fix round 1, Minor 2: `confirm_export_readback` (in `cairn_node::localstate`) is the
// read-after-write's actual check. Both failure branches get a test — before this fix
// round, NEITHER had one.
// ---------------------------------------------------------------------------

#[test]
fn confirm_export_readback_accepts_a_genuinely_sound_readback() {
    let wraps = cairn_node::localstate::establish_lsk("op-pass", "REC-CODE").unwrap();
    let bytes = cairn_node::localstate::build_export_container(
        &wraps,
        "op-pass",
        &cairn_node::localstate::LocalState::empty(),
    )
    .unwrap();
    assert!(cairn_node::localstate::confirm_export_readback(&bytes, &bytes, "op-pass").is_ok());
}

#[test]
fn confirm_export_readback_refuses_a_readback_that_differs_from_what_was_written() {
    let wraps = cairn_node::localstate::establish_lsk("op-pass", "REC-CODE").unwrap();
    let written = cairn_node::localstate::build_export_container(
        &wraps,
        "op-pass",
        &cairn_node::localstate::LocalState::empty(),
    )
    .unwrap();
    let mut readback = written.clone();
    let last = readback.len() - 1;
    readback[last] ^= 0xFF; // a single flipped bit — a torn write or a corrupting disk
    assert!(
        cairn_node::localstate::confirm_export_readback(&written, &readback, "op-pass").is_err()
    );
}

#[test]
fn confirm_export_readback_refuses_a_container_that_does_not_unseal_under_the_given_op_pass() {
    // Byte-identical to itself — the cheap check alone would miss this — but sealed under a
    // DIFFERENT op-pass than the one this call is handed, so the "does it actually unseal"
    // half must be the one that catches it (Minor 2's "stronger still" half).
    let wraps = cairn_node::localstate::establish_lsk("op-pass", "REC-CODE").unwrap();
    let bytes = cairn_node::localstate::build_export_container(
        &wraps,
        "op-pass",
        &cairn_node::localstate::LocalState::empty(),
    )
    .unwrap();
    assert!(
        cairn_node::localstate::confirm_export_readback(&bytes, &bytes, "some-other-pass").is_err()
    );
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
fn write_existing_escrow(key: &std::path::Path, sk: &SigningKey, op: &str, code: &str) {
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
///
/// Fix round 1, Minor 1: this used to call `backup::backup_to` directly, which never had
/// anything to do with the export ceremony at all — that lives entirely in the `Cmd::Backup`
/// CLI arm, one layer up. A test with this name calling the lower layer would stay green
/// even if a FUTURE change made only the arm start bailing on a skipped export (`backup_to`
/// itself would be untouched). Pinned at the CLI now, via the real binary, so the property
/// this test's name promises is the property it actually exercises.
#[tokio::test]
async fn backup_still_exits_zero_when_the_export_is_skipped() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // No `.lsk` escrow ever established — no `--passphrase` needed to reach it, and
    // `resolve_passphrase` is never called on this path (`EscrowRead::Absent` short-circuits
    // before any passphrase resolution), so nothing here depends on env state at all (fix
    // round 1 Nit: the old version's `std::env::remove_var("CAIRN_KEY_PASSPHRASE")` mutated
    // process-global state for no reason — `backup_to` never reads that variable — and could
    // race a sibling test in the same binary).
    let out = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the medium capture must still succeed even with no local-state escrow at all; \
         stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let image = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap();
    assert!(
        !clinical_records(&image).is_empty(),
        "and it must still carry the clinical plane"
    );
}

/// **An export written WITHOUT a custody key must not report a restorable kit**
/// (#500 slice 2c final review, Critical 2).
///
/// The escrow exists and the passphrase is right, so the export seals, writes and passes its
/// read-after-write check — but `<key>.unwrap` is absent, which is the ordinary state of a
/// node provisioned before ADR-0066 decision 5 (and equally what a bit-rotted or
/// differently-sealed unwrap file produces). `backup` warns to stderr and exits 0, correctly:
/// the medium is the load-bearing copy and an optional export must never abort it.
///
/// What must NOT happen is coverage advancing for that artifact. It did:
/// `ExportOutcome::Written(covers_seq)` was recorded unconditionally, so `kit_verdict` saw
/// `(Some(N), Some(N))`, returned `Restorable`, and `verify-backup` exited 0 over a kit whose
/// every sealed body restores as ciphertext, permanently — while `status`, reading the same
/// node, shouted about the missing key.
///
/// This test could not have existed before the fix landed, because `write_existing_escrow`
/// wrote no `.unwrap` file either: THREE tests in this file were passing over exactly this
/// kit. That is why the fixture now builds a full one and this test opts out of it explicitly.
#[tokio::test]
async fn an_export_with_no_custody_key_does_not_report_a_restorable_kit() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // The `.lsk` escrow ONLY — deliberately not `write_existing_escrow`, which would also
    // write the `.unwrap` file this test exists to be missing.
    let wraps = cairn_node::localstate::establish_lsk("op-pass", "REC-CODE").unwrap();
    cairn_node::fsio::atomic_write(
        &cairn_node::localstate::lsk_sidecar_path_for(&cl.key()),
        &cairn_node::localstate::serialize_sidecar(&wraps),
        Some(0o600),
    )
    .unwrap();
    assert!(
        !cairn_node::keystore::unwrap_key_path_for(&cl.key()).exists(),
        "anti-vacuity: the whole point is that this file is absent"
    );

    let out = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .args(["--passphrase", "op-pass"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "backup must still succeed — the medium is the load-bearing copy; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("unwrap key"),
        "and it must WARN that the export carries no key: {stderr}"
    );

    // THE PROPERTY. `verify-backup` must refuse rather than call this kit restorable.
    let check = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    let check_err = String::from_utf8_lossy(&check.stderr).to_string();
    assert!(
        !check.status.success(),
        "a kit whose export carries no custody key is NOT restorable, and exiting 0 here is \
         the false green the coverage ratchet exists to prevent; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&check.stdout),
        check_err
    );
    assert!(
        check_err.contains("ADR-0066"),
        "and the refusal must point at the custody decision, so the operator knows the \
         bodies are the problem rather than the events: {check_err}"
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
    write_existing_escrow(&cl.key(), &cl.sk, "op-pass", "REC-CODE");

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

/// The `Some` case the two pure `clinical_watermark_of_*` tests above cannot reach: a real,
/// signed CAIRNB3 medium carrying a genuine clinical segment. Cross-checked against the
/// database's own `MAX(seq)` — never against a hardcoded number — so this cannot pass by
/// silently agreeing with itself for the wrong reason.
#[tokio::test]
async fn clinical_watermark_of_a_real_medium_matches_the_true_max_seq() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let health_path = backup::health_path_for(&cl.key());
    backup::backup_to(
        &cl.db,
        &cl.medium(),
        &health_path,
        1_700_000_000,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect("the backup ceremony succeeds");

    let true_max: i64 = cl
        .db
        .query_one("SELECT MAX(seq) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let image = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap();
    assert_eq!(backup::clinical_watermark_of(&image), Some(true_max));
}

/// Fix round 1, Important 1, end to end: the reviewer's two-drive rotation. One signing key,
/// two media. Drive B's own export attempt fails (wrong passphrase, the unattended-cron
/// shape) while it holds a real clinical event with genuinely NO coverage at all. Drive A is
/// then backed up successfully, which — because `backup-status.json` is ONE file per key —
/// overwrites the shared sidecar's `medium_path` to A and its `export_covers_seq` to a
/// number that has NOTHING to do with B.
///
/// WITHOUT the Important-1 guard, `verify-backup --from B` would compare B's true watermark
/// (seq 1, one event) against A's coverage figure (also seq 1, coincidentally "enough") and
/// print `Restorable` — on a medium whose own export was never written at all. That is
/// exactly "the exact kit that cannot open its bodies" reading GREEN.
#[tokio::test]
async fn verify_backup_refuses_when_the_sidecar_describes_a_different_medium() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    write_existing_escrow(&cl.key(), &cl.sk, "op-pass", "REC-CODE");

    let drive_a = cl.dir.path().join("drive-a.medium");
    let drive_b = cl.dir.path().join("drive-b.medium");

    // Drive B: a WRONG passphrase, so the medium capture succeeds but the export never
    // lands — B's real, standing coverage is `ExportMissing`, not merely stale.
    let out_b = cl
        .cli()
        .args(["backup", "--to"])
        .arg(&drive_b)
        .args(["--passphrase", "wrong-op"])
        .output()
        .unwrap();
    assert!(
        out_b.status.success(),
        "the medium capture must still succeed; stderr:\n{}",
        String::from_utf8_lossy(&out_b.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out_b.stderr).contains("local-state export skipped"),
        "drive B's export must have failed to seal: stderr:\n{}",
        String::from_utf8_lossy(&out_b.stderr)
    );

    // Drive A: the RIGHT passphrase. This is a completely SEPARATE, fresh medium — its own
    // first capture sweeps the same one clinical event from the beginning — and its
    // successful export overwrites the ONE shared sidecar's `medium_path` to A.
    let out_a = cl
        .cli()
        .args(["backup", "--to"])
        .arg(&drive_a)
        .args(["--passphrase", "op-pass"])
        .output()
        .unwrap();
    assert!(
        out_a.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out_a.stderr)
    );
    assert!(String::from_utf8_lossy(&out_a.stdout).contains("local-state exported"));

    // The sidecar now names A, not B. `verify-backup --from B` must refuse outright rather
    // than read A's coverage figure as if it said anything about B.
    let v = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(&drive_b)
        .output()
        .unwrap();
    assert!(
        !v.status.success(),
        "a medium whose sidecar describes a DIFFERENT artifact must refuse, not read as \
         restorable"
    );
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        stderr.contains("COVERAGE-UNKNOWN"),
        "the refusal must name the mismatch, distinct from STALE/INCOMPLETE: {stderr}"
    );
}

/// Fix round 2's new Important finding: the Important-1 guard above must NOT fire on a
/// medium that carries no clinical events at all, purely because the node-global sidecar
/// happens to name a different path. Two media, neither ever holding a single clinical
/// event (only the federation genesis every `establish_clinic` node has) — the sidecar ends
/// up naming drive A after drive B's own backup already succeeded, so `verify-backup --from
/// B` sees a "mismatched" path exactly like the rotation test above. The difference is that
/// B has NOTHING for an export to cover, so this must verify CLEAN — `kit_verdict`'s own
/// policy (a genuinely-empty medium is `Restorable`, never flagged) must not be overridden
/// by a guard that fires on path disagreement alone.
#[tokio::test]
async fn verify_backup_is_clean_on_an_empty_clinical_medium_even_with_a_mismatched_sidecar() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // Deliberately NO `author_sealed_clinical_event` call. `establish_clinic` already gives
    // this node a federation genesis (via `identity::provision`), so the medium is not
    // EMPTY in the `#502 item 2` sense (federation events exist) — it simply carries no
    // CLINICAL events, which is the `clinical_watermark_of(&image) == None` case this test
    // is actually about.

    let drive_a = cl.dir.path().join("drive-a.medium");
    let drive_b = cl.dir.path().join("drive-b.medium");

    // Back up to B first — no passphrase/escrow needed, since neither medium ever carries a
    // clinical event or an export in this test.
    let out_b = cl
        .cli()
        .args(["backup", "--to"])
        .arg(&drive_b)
        .output()
        .unwrap();
    assert!(
        out_b.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out_b.stderr)
    );

    // Then to A — which, because `backup-status.json` is ONE file per signing key, rewrites
    // the shared sidecar's `medium_path` to A. The sidecar now describes a DIFFERENT medium
    // than B, exactly the shape the Important-1 guard watches for.
    let out_a = cl
        .cli()
        .args(["backup", "--to"])
        .arg(&drive_a)
        .output()
        .unwrap();
    assert!(
        out_a.status.success(),
        "stderr:\n{}",
        String::from_utf8_lossy(&out_a.stderr)
    );

    // B is genuinely empty of clinical events, so the path mismatch above must NOT refuse
    // it — the guard requires `medium_seq.is_some()` precisely so this stays green.
    let v = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(&drive_b)
        .output()
        .unwrap();
    assert!(
        v.status.success(),
        "an empty-clinical medium must verify clean regardless of a mismatched sidecar; \
         stderr:\n{}",
        String::from_utf8_lossy(&v.stderr)
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

// ---------------------------------------------------------------------------
// Final review, Important 2: `verify-backup` and `backup` must reach the SAME verdict on
// the same bytes. Before this, `verify-backup` computed `assess()` (inside
// `clinical_watermark_of`) and threw the composed verdict away, resting its "OK" on
// `verify_events` over the FEDERATION plane alone — so a corrupt CLINICAL record printed
// `federation-plane events OK: N/N verified` and exited 0 on a medium `backup` refused to
// touch. Two commands, opposite verdicts, one file.
// ---------------------------------------------------------------------------

/// Flip one bit inside `needle` where it sits in `haystack`, in place.
///
/// A SURGICAL corruption, and the shape matters: it targets the middle of a known record's
/// signed bytes, so every length prefix and every frame boundary in the container stays
/// exactly as written. The medium therefore still PARSES cleanly — this is a broken
/// signature, not a `Damaged` container — which is precisely the case a narrow
/// federation-plane check cannot see and the composed verdict can.
fn flip_a_bit_inside(haystack: &mut [u8], needle: &[u8]) -> usize {
    let at = haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("the record must be findable on the medium, or this test corrupts nothing");
    let target = at + needle.len() / 2;
    haystack[target] ^= 0x01;
    target
}

/// A corrupt CLINICAL record must fail `verify-backup`, exactly as it fails `backup`.
#[tokio::test]
async fn verify_backup_refuses_a_medium_whose_clinical_plane_is_corrupt() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let signed = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    // A full kit — escrow + passphrase, so the sealed export lands beside the medium and the
    // coverage check further down `verify-backup` is satisfied. Without it the honest medium
    // would already refuse for a DIFFERENT reason (`ExportMissing`), and this test would have
    // nothing to say about soundness.
    write_existing_escrow(&cl.key(), &cl.sk, "op-pass", "REC-CODE");

    let out = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .args(["--passphrase", "op-pass"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the honest capture must succeed first; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // ANTI-VACUITY: the UNCORRUPTED medium must verify clean, or "it refuses after the
    // bit-flip" would be a statement about something else entirely.
    let before = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        before.status.success(),
        "an honest medium must still verify clean; stderr:\n{}",
        String::from_utf8_lossy(&before.stderr)
    );

    let mut bytes = std::fs::read(cl.medium()).unwrap();
    flip_a_bit_inside(&mut bytes, &signed);
    std::fs::write(cl.medium(), &bytes).unwrap();
    // The corruption must not have broken the FRAMING — the whole point is a medium that
    // parses, whose federation plane is untouched, and which is nonetheless unsound.
    assert!(
        matches!(parse_any(&bytes), Ok(MediumImage::V3(_))),
        "the fixture must stay a parseable CAIRNB3 medium: a `Damaged` container would be \
         caught by the old, narrower check too and would prove nothing"
    );

    let after = cl
        .cli()
        .args(["verify-backup", "--from"])
        .arg(cl.medium())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&after.stderr);
    let stdout = String::from_utf8_lossy(&after.stdout);
    assert!(
        !after.status.success(),
        "a corrupt clinical record must fail the cron health check — the federation plane \
         being intact is not the same claim as the medium being sound.\nstdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("UNSOUND"),
        "the refusal must name what it found, not merely exit non-zero: {stderr}"
    );
    assert!(
        !stdout.contains("events OK"),
        "and it must refuse BEFORE printing any all-clear — an operator who sees `OK` scroll \
         past has already been told the wrong thing: {stdout}"
    );

    // THE POINT OF THE WHOLE FINDING: `backup` over these same bytes refuses too, so the
    // two commands can no longer disagree about one file.
    let re_backup = cl
        .cli()
        .args(["backup", "--to"])
        .arg(cl.medium())
        .output()
        .unwrap();
    assert!(
        !re_backup.status.success(),
        "`backup` already refused this medium — that disagreement is the defect: stderr:\n{}",
        String::from_utf8_lossy(&re_backup.stderr)
    );
}
