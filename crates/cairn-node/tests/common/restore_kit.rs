//! A dead clinic and the machine that replaces it — the shared fixture for tests of `restore`.
//!
//! # Why this file exists
//!
//! `restore_cli_surface.rs` (#572/#570) built the clinic, the export and the wipe privately.
//! Slice 2d's remaining design tests (#593) need the same dead clinic, the same export and the
//! same wiped replacement machine from four more suites — and those fixtures have each already
//! had a way to make a test pass for the wrong reason: the wipe that forgot `actor_event`, the
//! wipe that forgot `node_unwrap_key` (#593 review), the export that was not really beside the
//! medium. A fixture with that history should exist ONCE, which is the convention `clinic_kit.rs`
//! set for #567. (`restore_reads_the_clinical_plane.rs` still carries older private copies,
//! including a same-named wipe that does less; #598 tracks moving it onto this kit.)
//!
//! # How a restore test uses it
//!
//! 1. [`provisioned_clinic`] — the dead node while it was alive: identity, enrolled actors, and a
//!    registered unwrap key.
//! 2. [`author_sealed_clinical_event`] — a real chart, sealed through the strict door.
//! 3. [`capture`] (once or more) and [`write_export_beside`] — what the nightly `backup` leaves
//!    on the operator's drive. [`medium_with_export`] is both in one call.
//! 4. [`wipe_to_a_fresh_dr_machine`] — the disaster.
//! 5. [`restore_cli`] — the real `cairn-node restore`, stdout and stderr piped, stdin closed.
//!
//! Include it with `#[path = "common/restore_kit.rs"] mod restore_kit;`. The including binary
//! must ALSO declare `mod common;`, because the clinic is built through `common::medication_setup`
//! and each chart is registered through `common::submit_registration` (#345: a chart's first
//! event must be its registration).
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::seal::{seal_event_payload, seal_stub_twin, Secret32};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{parse_any, segment_commitment, serialize_v3, MediumImage, Plane, Segment};
use cairn_node::{backup, db, identity, localstate};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tokio_postgres::Client;
use uuid::Uuid;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A `Command` for the freshly-built `cairn-node` binary under test. Private: every suite drives
/// `restore` through [`restore_cli`], so its flags are spelled in one place.
fn cairn_node() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cairn-node"))
}

/// The operator passphrase for a fixture's local-state escrow.
///
/// Derived at runtime rather than written as a literal (house rule 6a): a literal in a crypto
/// context trips CodeQL's `rust/hard-coded-cryptographic-value` as a recurring critical false
/// positive that blocks the scan until a human dismisses it (#146).
pub fn an_op_passphrase(lineage: u8) -> String {
    (0..24u8)
        .map(|i| (b'a' + ((i.wrapping_mul(5).wrapping_add(lineage)) % 26)) as char)
        .collect()
}

/// A recovery code for a fixture, derived at runtime for the same reason as above.
///
/// `normalize_recovery_code` strips spacing and case before the unwrap, so the exact alphabet
/// does not matter here — only that the same string goes into the seal and comes back out of
/// the file.
pub fn a_recovery_code(lineage: u8) -> String {
    let alphabet: Vec<char> = ('A'..='Z').chain('2'..='7').collect();
    (0..32usize)
        .map(|i| alphabet[(i * 7 + lineage as usize) % alphabet.len()])
        .collect()
}

/// A provisioned solo clinic: node identity, an enrolled device and human actor, and a
/// registered unwrap key derived from the device key. Mirrors
/// `restore_reads_the_clinical_plane.rs::provisioned_clinic`.
pub async fn provisioned_clinic(c: &Client) -> (SigningKey, String) {
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(c).await;
    identity::provision(c, &sk, &kid, "solo-clinic", "127.0.0.1:7941")
        .await
        .unwrap();
    (sk, kid)
}

/// This node's registered unwrap SECRET, as a restore inherits it.
///
/// The fixture registers a key derived from the device signing key (`medication_setup`), so
/// this reproduces the same derivation rather than reading a keystore file the test never
/// wrote. **Not a widening of `derive_unwrap_secret`'s allow-list** — that guard sweeps
/// PRODUCTION trees only, and this is a test reconstructing what the fixture already did.
pub fn fixture_unwrap_secret(sk: &SigningKey) -> Secret32 {
    cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()))
}

/// One chart as the dead node held it: what a restore must bring back, and what a test needs
/// in order to forge a rival to it (`restore_one_event_id_one_body.rs`).
pub struct Authored {
    pub event_id: String,
    pub patient: Uuid,
    /// The twin text, so finding it on the far side proves the body was UNSEALED rather than
    /// merely that a row exists.
    pub twin: String,
    pub signed_bytes: Vec<u8>,
}

/// Build a born-sealed `clinical.medication.asserted` body and the DEK it is sealed under.
/// **Pure** apart from two fresh values: the DEK `seal_event_payload` mints, and the
/// `medication_id` this function takes from `Uuid::now_v7()`.
///
/// `twin` is sealed INSIDE the container, beside the payload (ADR-0052), so its length is what
/// sets the size of the signed bytes — `restore_pen_is_uncapped.rs` pads it to cross the pen's
/// byte quota.
///
/// Separate from [`author_sealed_clinical_event`] so a test can build a body WITHOUT submitting
/// it — a record that must reach a medium but never the dead node's own log (a forged rival
/// under an existing `event_id`, or an event carried in a plane this build cannot route).
pub fn sealed_assert_body(
    kid: &str,
    patient: Uuid,
    event_id: &str,
    twin: &str,
    hlc: Hlc,
) -> (EventBody, Secret32) {
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let (container, dek) = seal_event_payload(&payload, twin, event_id).unwrap();
    let body = EventBody {
        event_id: event_id.into(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    (body, dek)
}

/// Submit ONE real born-sealed clinical event, on a FRESH chart, through the STRICT door.
///
/// A production-door body, never a hand-built row: the `event_dek` custody a restore test reads
/// has to be what the real writer produces, or the restore is being tested against a fixture
/// rather than against the system.
pub async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> Authored {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;

    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let event_id = Uuid::now_v7().to_string();
    let twin = format!("amoxicillin — asserted for {patient}");
    let (body, dek) = sealed_assert_body(kid, patient, &event_id, &twin, hlc);
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
    .expect("a sealed body with its DEK is admitted");

    // ANTI-VACUITY: the clear view really exists on the SOURCE node, so "it came back" is a
    // statement about the restore rather than about a body that was never readable.
    assert_eq!(
        twin_of(c, &event_id).await.as_deref(),
        Some(twin.as_str()),
        "the source node can read its own chart"
    );
    Authored {
        event_id,
        patient,
        twin,
        signed_bytes: signed.signed_bytes,
    }
}

/// The clear twin of `event_id` on this node, or `None` when no readable body exists.
pub async fn twin_of(c: &Client, event_id: &str) -> Option<String> {
    c.query_opt(
        "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
        &[&event_id],
    )
    .await
    .unwrap()
    .map(|row| row.get(0))
}

/// Whether `event_id` is in this node's `event_log` at all — readable or not.
pub async fn in_event_log(c: &Client, event_id: &str) -> bool {
    c.query_one(
        "SELECT EXISTS(SELECT 1 FROM event_log WHERE event_id = $1::text::uuid)",
        &[&event_id],
    )
    .await
    .unwrap()
    .get(0)
}

/// The reason the quarantine pen holds these exact signed bytes for, or `None` when it does not
/// hold them. `content_digest` is the pen's primary key, so there is at most one row.
pub async fn pen_reason(c: &Client, signed_bytes: &[u8]) -> Option<String> {
    c.query_opt(
        "SELECT reason FROM sync_quarantine WHERE content_digest = $1",
        &[&cairn_event::event_address(signed_bytes)],
    )
    .await
    .unwrap()
    .map(|row| row.get(0))
}

/// Whether the quarantine pen holds these exact signed bytes.
pub async fn in_pen(c: &Client, signed_bytes: &[u8]) -> bool {
    pen_reason(c, signed_bytes).await.is_some()
}

/// How many `medication_statement` rows `patient`'s chart shows.
///
/// A readable body in `event_clear` is not yet a chart: the projection runs in the AFTER INSERT
/// trigger on `event_log`, and until #584 a sealed event admitted WITHOUT its body left the chart
/// empty even once custody landed later — the door now projects that landing (ADR-0070). This is
/// the number a clinician sees.
pub async fn medication_rows(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A medium record as a capture taken WITHOUT a key writes it: no attestation, no wrapped DEK.
/// **Pure.**
///
/// The missing DEK is usually the point of the call — a keyless record is admitted with no
/// custody, where one carrying a DEK is refused in Rust when no custody key is installed.
pub fn keyless_record(signed_bytes: Vec<u8>, source_seq: i64) -> cairn_medium::MediumRecord {
    cairn_medium::MediumRecord {
        signed_bytes,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq,
    }
}

/// Whether the export carries the node's unwrap SECRET, or only its custody rows and registry.
///
/// [`Self::Missing`] is not a contrived fixture: it is exactly what
/// `seal_and_write_local_state_export` writes when the keystore's `.unwrap` file cannot be
/// loaded — it warns and carries on, because the export is optional and the medium is the
/// load-bearing copy. A restore from such an export installs the actor registry but no custody
/// key, so every record carrying one is PENNED with its key rather than admitted.
#[derive(Clone, Copy)]
pub enum ExportCustody {
    Carried,
    Missing,
}

/// Run one capture into `dir/cairn.medium` — appending to it when it already exists, exactly
/// as a second night's `backup` does — and return the medium's path.
pub async fn capture(c: &Client, sk: &SigningKey, kid: &str, dir: &Path) -> PathBuf {
    let medium_path = dir.join("cairn.medium");
    backup::backup_to(
        c,
        &medium_path,
        &dir.join("backup-status.json"),
        0,
        Some((sk, kid)),
    )
    .await
    .expect("the backup ceremony succeeds");
    medium_path
}

/// Write the sealed `CAIRNL1` export sibling beside `medium`, built by the same two functions the
/// `backup` command uses after its capture.
///
/// The export carries the dead node's unwrap secret and its actor registry, so a restore without
/// one recovers rows it can never open. Building it through `read_local_state` +
/// `build_export_container` — the two functions `seal_and_write_local_state_export` itself calls
/// — keeps this honest while letting the recovery code be a value the test chose rather than one
/// scraped from a banner. What it skips is the command's bookkeeping around them: the
/// read-after-write check and the health sidecar's `export_covers_seq`, neither of which a
/// restore reads.
pub async fn write_export_beside(
    c: &Client,
    sk: &SigningKey,
    medium: &Path,
    op: &str,
    code: &str,
    custody: ExportCustody,
) {
    let unwrap_secret = fixture_unwrap_secret(sk);
    let carried = match custody {
        ExportCustody::Carried => Some(&unwrap_secret),
        ExportCustody::Missing => None,
    };
    let bundle = localstate::read_local_state(c, carried)
        .await
        .expect("reading this node's local state");
    let wraps = localstate::establish_lsk(op, code).expect("establishing the LSK escrow");
    let container = localstate::build_export_container(&wraps, op, &bundle)
        .expect("sealing the local-state export");
    let export_path = localstate::localstate_path_for(medium);
    std::fs::write(&export_path, &container).unwrap();

    // ANTI-VACUITY. If the export is not really beside the medium, `restore` never enters the
    // recovery-code path at all and every assertion about custody is about a code nothing read.
    assert!(
        export_path.exists(),
        "the fixture must place a CAIRNL1 export beside the medium"
    );
}

/// [`capture`] then [`write_export_beside`]: one night's complete disaster-recovery kit.
pub async fn medium_with_export(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    dir: &Path,
    op: &str,
    code: &str,
    custody: ExportCustody,
) -> PathBuf {
    let medium = capture(c, sk, kid, dir).await;
    write_export_beside(c, sk, &medium, op, code, custody).await;
    medium
}

/// Put the database in the state a disaster-recovery machine is in: no clinical tier, no
/// medication chart, no federation identity, **no registered unwrap key**, and **no actor
/// registry**.
///
/// ⚠️ **Three tables survive a clinical-tier truncate, and each one left behind makes a restore
/// test weaker than it looks.** None has a foreign key to `event_log`, so `CASCADE` never reaches
/// them:
///
/// - **`actor_event`.** A wipe that omitted it left the enrolled signers from `medication_setup`
///   in place, and the apply door would then have accepted every record on its own, whether or
///   not the export's registry ever arrived. A real replacement machine has an empty
///   `actor_event`, which is exactly why `restore_actor_registry` (db/052) exists at all.
/// - **`node_unwrap_key`** (#593 review). `medication_setup` registers the dead node's key. Left in
///   place, db/020 wraps custody to it and writes the readable `event_clear` row whether or not the
///   restore registered anything, so "the body opens" passed even against a restore that had
///   stopped registering custody. With no registration the door admits a sealed event WITHOUT its
///   body (db/020 step 9's lenient arm), which is what a real replacement machine would do.
/// - **The medication projection tables.** A chart assertion made after a restore would otherwise
///   read the dead node's surviving rows. The list mirrors `common::medication_setup`'s, which
///   #340 tracks consolidating — do not tidy the two into one without reading it.
///
/// `actor_event` is append-only (db/004 refuses DELETE by trigger), so this disables that
/// trigger for the duration. A test-fixture act, never something a node does — the door's own
/// fence is what protects a real registry, and it is pinned in the SQL mirror.
pub async fn wipe_to_a_fresh_dr_machine(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, event_dek, event_clear, erasure_shred_log, patient_chart, \
         node_unwrap_key CASCADE",
    )
    .await
    .expect("wiping the clinical tier and its custody key, as a fresh DR machine would have it");
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN TRUNCATE medication_dose_correction; END IF; \
           IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN TRUNCATE medication_reconciliation; END IF; \
           IF to_regclass('public.medication_group_member') IS NOT NULL THEN TRUNCATE medication_group_member; END IF; \
           IF to_regclass('public.medication_projection_flag') IS NOT NULL THEN TRUNCATE medication_projection_flag; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
           IF to_regclass('public.medication_attestation') IS NOT NULL THEN TRUNCATE medication_attestation; END IF; \
         END $$;",
    )
    .await
    .expect("wiping the medication chart, as a fresh DR machine would have it");
    c.batch_execute("DELETE FROM sync_quarantine")
        .await
        .unwrap();
    c.batch_execute(
        "ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update;
         DELETE FROM actor_event;
         ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;",
    )
    .await
    .expect("clearing the actor registry, as a fresh DR machine has it");
    db::reset_node_federation_tables(c).await.unwrap();
}

/// Write the DEAD node's recovery code to a file in `dir`, where `--old-recovery-code-file` reads
/// it, and return the file's path.
pub fn old_recovery_code_file(dir: &Path, code: &str) -> PathBuf {
    let path = dir.join("old-recovery-code");
    std::fs::write(&path, code).unwrap();
    path
}

/// Run the real `cairn-node restore` against `conn`: stdout and stderr piped, stdin closed.
///
/// That is what `Command::output()` gives a child, and it is not quite "no terminal": it does not
/// detach a controlling terminal, so a local run started from a shell could still open `/dev/tty`.
/// CI has no terminal, which is where a regression back to a tty prompt (#572) would show. A
/// future test cannot feed a code through `--old-recovery-code-file /dev/stdin` with this helper —
/// the read would find stdin already closed.
///
/// `--insecure-plaintext` keeps the new key unsealed, so no passphrase or freshly minted
/// recovery code is involved (that branch is taken before a passphrase is ever read);
/// `code_file` is the DEAD node's recovery code, which opens the export beside the medium
/// (ADR-0069). `None` means the export is never opened — or there is none to open.
pub fn restore_cli(conn: &str, new_key: &Path, medium: &Path, code_file: Option<&Path>) -> Output {
    let mut cmd = cairn_node();
    cmd.args(["--conn", conn, "--key"])
        .arg(new_key)
        .args(["restore", "--from"])
        .arg(medium);
    if let Some(file) = code_file {
        cmd.arg("--old-recovery-code-file").arg(file);
    }
    cmd.arg("--insecure-plaintext").output().unwrap()
}

/// Parse the CAIRNB3 medium at `path`, let `edit` change its segments, write it back, and return
/// whatever `edit` returns — so a test can count what it changed, by hand, in the same pass.
///
/// The segments are re-serialized as edited: an attestation the edit invalidated stays invalid
/// rather than being quietly re-signed, so each test decides for itself what kind of damage it
/// is modelling.
pub fn rewrite_medium<R>(path: &Path, edit: impl FnOnce(&mut Vec<Segment>) -> R) -> R {
    let MediumImage::V3(mut m) = parse_any(&std::fs::read(path).unwrap()).unwrap() else {
        panic!("a capture writes CAIRNB3, never a legacy container")
    };
    let result = edit(&mut m.segments);
    std::fs::write(path, serialize_v3(&m.segments).unwrap()).unwrap();
    result
}

/// How many records `segments` carry on the clinical plane, counted by hand. **Pure.**
///
/// A test's expected counts come from here rather than from `plane_records_with_accounting`, so
/// the code under test is never the thing that decides what the right answer is.
pub fn clinical_record_count(segments: &[Segment]) -> usize {
    segments
        .iter()
        .filter(|s| s.plane == Plane::Clinical)
        .map(|s| s.records.len())
        .sum()
}

/// A new UNSIGNED segment chained correctly onto the end of `segments` — the next index, linked
/// to the last segment's records, naming the same node — so the chain stays intact and only
/// what the test puts IN the segment is under test. **Pure.**
///
/// An unsigned segment is not a fault: it is what a capture taken without the signing key writes.
///
/// **It links to the FILE's last segment, not the last VERIFIED one.** Production appends through
/// `cairn_medium::chain_tail`, which follows the last segment whose link held; the two agree only
/// while the medium is intact. Rebuilding the link from the file tail is deliberate: a test that
/// has already broken the chain on purpose (test 14) appends AFTER the damage, and needs the new
/// segment's own link to be intact so the only reason it is untrusted is what sits before it.
pub fn chained_segment(
    segments: &[Segment],
    plane: Plane,
    records: Vec<cairn_medium::MediumRecord>,
) -> Segment {
    let last = segments
        .last()
        .expect("a capture wrote at least one segment");
    Segment {
        plane,
        index: u32::try_from(segments.len()).unwrap(),
        prev_commitment: segment_commitment(&last.records),
        self_node_id_hex: last.self_node_id_hex.clone(),
        attestation: None,
        records,
    }
}
