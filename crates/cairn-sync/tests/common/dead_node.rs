//! The dead-node fixture every `requeue` suite is built on (#568, #578).
//!
//! # Why this is shared rather than copied
//!
//! Provisioning a node that can pen a real born-sealed clinical record is not a small fixture: it
//! enrols a device actor, registers this node's unwrap key, registers a chart so the §5.3/§5.8
//! precedence door will accept anything about it (#345), authors one medication event through the
//! PRODUCTION orchestrator, reads the DEK and twin back off the node, then wipes the clinical tier
//! to simulate the dead disk. Roughly four hundred lines, every one of which is load-bearing in a
//! way a second copy would get subtly wrong — and #568's own header records two fixture mistakes
//! that each made a test pass for the wrong reason.
//!
//! `requeue_releases_custody.rs` (#568, the release path) and
//! `requeue_retains_unlanded_custody.rs` (#578, the retention path) both need exactly this node.
//! Pulled in with `#[path]`, the same convention `common/serve.rs` uses for the serve harness.
//!
//! **This module carries no `#[cfg(test)] mod tests` of its own, deliberately.** Cargo compiles
//! `tests/*.rs` with `--test`, so self-tests here would compile and run once per including binary.
//! The convention in this crate is a separate `*_shared.rs` file; these fixtures have no pure logic
//! worth pinning that way — their correctness is asserted by the suites that use them, each of
//! which fails loudly if a fixture stops doing its job (see `author_sealed_record`'s own
//! anti-vacuity check).

// Each including binary uses a different subset, and an unused helper here is not a defect — it is
// a helper the OTHER suite needs. Without this, adding a fixture for one file warns in the other.
#![allow(dead_code)]

use cairn_event::keys::Secret32;
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::medication::{assert_medication, AssertMedicationInput};
// IMPORTED, never re-spelled: a penned row must be identifiable as a restore's rather than
// blending into an unnamed link, and a literal here would drift from the door's own sentinel.
use cairn_node::restore::clinical::RESTORE_PEER_SENTINEL;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use tokio_postgres::Client;
use uuid::Uuid;

/// HLC wall for the chart's birth act — mid-2026, comfortably before the real clock the production
/// orchestrators tick from, so the registration never sorts after the events authored on it
/// (`patient_registration_current` picks the EARLIEST).
pub const REG_WALL: i64 = 1_780_000_000_000;

/// The medium sequence a penned record carries. A restore passes the record's own `source_seq`
/// (`sync_quarantine.refused_seq` is NOT NULL), so a fixture that passed NULL would be exercising a
/// row shape no restore can produce — which is how the first draft of this file failed.
pub const PENNED_SOURCE_SEQ: i64 = 1;

pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Everything a test needs about the one sealed record, read off the node BEFORE its disk "died".
///
/// Of these four, the backup medium carries exactly TWO verbatim — `signed_bytes` and
/// `dek_wrapped` (a real `MediumRecord` also carries attestation, attester key and source seq,
/// which this fixture does not need). The `digest` is NOT carried: a restore re-derives it from the
/// bytes with `event_address`. And the `twin` is precisely what a medium never carries, because it
/// is what we are trying to get back. Do not read this struct as the medium's format.
pub struct DeadNodeRecord {
    pub signed_bytes: Vec<u8>,
    pub digest: Vec<u8>,
    pub dek_wrapped: Vec<u8>,
    pub twin: String,
}

/// Truncate everything this suite touches, so each run starts from a genuinely empty node.
///
/// `node_unwrap_key` MUST be included: it is a SINGLETON that refuses a second, different key
/// (`cairn_register_unwrap_key` — rotation is a separate ceremony), and every run here authors
/// under a FRESH key whose derived unwrap key differs from the last run's. Left standing, the
/// prior run's singleton collides at the first sealed author and every test fails for a reason
/// that has nothing to do with requeue. (Same reasoning as `clinical_pull.rs`'s `reset`.)
pub async fn reset(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, medication_statement, \
         medication_cessation, medication_dose_event, medication_dose_correction, \
         medication_reconciliation, medication_group_member, medication_projection_flag, \
         medication_attestation, medication_patient_conflict_flag, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log, \
         sync_state, sync_quarantine RESTART IDENTITY CASCADE",
    )
    .await
    .expect("truncate the tables this suite owns");
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .expect("reset the HLC");
}

/// Write a signing key in the EXACT on-disk shape the `cairn-sync` binary reads: a hex-encoded
/// 32-byte Ed25519 seed (the daemon does `hex::decode(text.trim())`).
///
/// NOT the same shape as `cairn_keystore::load` (raw bytes / sealed CBOR) — the CLI verbs use the
/// daemon's own hex loader, so a file in the other shape is rejected by the process before any of
/// this test's assertions could run. House rule 6: the seed is generated at runtime by
/// `generate_key`, never a literal.
pub fn write_key_file(dir: &Path, name: &str, sk: &SigningKey) -> String {
    let path = dir.join(name);
    std::fs::write(&path, hex::encode(sk.to_bytes())).expect("write hex-seed key file");
    path.to_str().expect("utf-8 key path").to_string()
}

/// This node's unwrap secret, DERIVED from its signing seed.
///
/// **Derived deliberately — do not switch this to `generate_unwrap_secret`.** `cairn-sync` has no
/// `establish-unwrap-key` command: `unwrap_key::resolve_at_startup` finds its secret either in a
/// `<key>.unwrap` sibling file or, absent one, by deriving it from the signing seed and checking
/// that it matches what `node_unwrap_key` has registered (the pre-ADR-0066 fallback — the decision
/// table on `unwrap_key::resolve` is the durable statement of it, and ADR-0066 the reasoning;
/// HANDOVER's trap list says the same thing but is renumbered as traps are minted, so it is not a
/// citable address). Registering a DERIVED key here is what lets these tests drive the shipped binary
/// with `--key` alone. A generated key would satisfy the authoring path and then silently fail to
/// resolve at requeue time — turning a loud failure into a quiet wrong answer.
pub fn derived_unwrap_secret(sk: &SigningKey) -> Secret32 {
    cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()))
}

/// Register this node's own DEK-unwrap public key — the provisioning act ADR-0066 decision 6 made
/// explicit. Without it `ensure_unwrap_key` refuses the first sealed write, so nothing can be
/// authored to pen in the first place.
pub async fn register_unwrap_key(c: &Client, sk: &SigningKey) {
    let public = cairn_event::seal::unwrap_public(&derived_unwrap_secret(sk));
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&public.as_bytes().as_slice()],
    )
    .await
    .expect("register this node's custody key");
}

/// Register a chart so events may be authored about it (§5.3/§5.8, #345, ADR-0061).
///
/// Since #345 the STRICT door refuses the first event carrying a `patient_id` unless it is that
/// chart's registration — exactly as a clerk must make a folder before anything can be filed in
/// it. Written out here rather than shared because `cairn-node/tests/common/` belongs to the other
/// crate's test target.
pub async fn register_chart(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid) {
    let tokens = [patient.to_string()];
    let a = cairn_event::registration::RegistrationAssertion {
        class: cairn_event::registration::RegistrationClass::Standard,
        basis: None,
        search: Some(cairn_event::registration::SearchAttestationInput {
            terms: cairn_event::registration::SearchTerms {
                name_tokens: &tokens,
                birth_date: None,
                identifiers: &[],
            },
            displayed: &[],
            incomplete: false,
        }),
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: cairn_event::registration::REGISTRATION_EVENT_TYPE.into(),
        schema_version: cairn_event::registration::REGISTRATION_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: REG_WALL,
            counter: 0,
            node_origin: "dead-node".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: cairn_event::registration::registration_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(cairn_event::registration::render_registration_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk).expect("sign the registration");
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("registration accepted");
}

/// Author ONE real born-sealed medication event and read back everything the medium would carry.
///
/// Deliberately the production orchestrator (`assert_medication`) and not a hand-built body: the
/// point of the slice is that a REAL sealed chart comes back, and a fixture that sealed its own
/// payload could drift from what the strict door actually writes.
///
/// The anti-vacuity check lives here, at the source: if the authoring node cannot read its own
/// chart, every later "the body opens" assertion is measuring nothing.
pub async fn author_sealed_record(c: &mut Client, sk: &SigningKey, kid: &str) -> DeadNodeRecord {
    let patient = Uuid::now_v7();
    register_chart(c, sk, kid, patient).await;

    let input = AssertMedicationInput {
        term: "amoxicillin",
        coding: None,
        formulation: Some("capsule"),
        dose_amount: Some("500"),
        dose_unit: Some("mg"),
        sig: Some("one TDS"),
        info_source: "patient-reported",
        started: Some("2026"),
        started_precision: Some("year"),
    };
    assert_medication(c, sk, kid, "dead-node", patient, &input, None, None)
        .await
        .expect("the strict door admits a born-sealed medication assertion");

    let row = c
        .query_one(
            "SELECT e.signed_bytes, e.content_address, d.dek_wrapped, k.twin \
               FROM event_log e \
               JOIN event_dek d ON d.event_id = e.event_id \
               JOIN event_clear k ON k.event_id = e.event_id \
              WHERE e.event_type = 'clinical.medication.asserted'",
            &[],
        )
        .await
        .expect("the authoring node holds its own sealed event with custody and a clear view");

    let record = DeadNodeRecord {
        signed_bytes: row.get(0),
        digest: row.get(1),
        dek_wrapped: row.get(2),
        twin: row.get(3),
    };
    assert!(
        record.twin.contains("amoxicillin"),
        "ANTI-VACUITY: the authoring node must be able to read its own chart before this suite \
         can mean anything by 'the body opens'. Got twin: {:?}",
        record.twin
    );
    record
}

/// Wipe the clinical tier, leaving the node enrolled and its custody key registered.
///
/// This is the state a restore reaches: the machine is new, the schema is loaded, the operator has
/// installed custody, and not one clinical row exists yet.
pub async fn wipe_clinical_tier(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, event_dek, event_clear, erasure_shred_log, patient_chart, \
         medication_statement CASCADE",
    )
    .await
    .expect("wipe the clinical tier");
}

/// Pen a record through the REAL door (`db/052`), as a restore does — never a raw INSERT.
///
/// NOT *exactly* as a restore does, and the gap is worth naming: a restore passes the medium's
/// `attestation` / `attester_key` (`restore::clinical`), where this fixture passes NULL for both,
/// because a locally-authored medication assertion carries neither. The DOOR is the same, which is
/// the property being preserved; the row is a restore-shaped subset, not a replica.
///
/// `dek` is passed separately from the record, and is an `Option`, for two reasons:
///   * `Some(other)` pens a DEK this node cannot open — the foreign-medium arm — without having to
///     fake the rest of the row;
///   * `None` pens the KEYLESS row an ordinary `pull` creates for a plaintext event, which is the
///     modal production shape and the one input pair `do_requeue`'s `_` arm absorbs silently.
pub async fn pen(c: &Client, record: &DeadNodeRecord, dek: Option<&[u8]>) {
    // ⚠️ THE DOOR'S BOOLEAN IS `acked`, NOT "was it penned". A fresh row comes back FALSE, a
    // re-offer of an already-acked one TRUE, and a pen it genuinely refuses RAISEs rather than
    // returning anything. An earlier draft of this fixture read it as success and asserted the
    // wrong thing; the proof that the row is really there is the read-back below.
    let acked: bool = c
        .query_one(
            "SELECT cairn_quarantine_event($1, $2, NULL, NULL, $3, $4, $5, $6, NULL, NULL)",
            &[
                &record.digest,
                &record.signed_bytes,
                &RESTORE_PEER_SENTINEL,
                &PENNED_SOURCE_SEQ,
                &"restore: custody could not be installed, so this record is held with its key",
                &dek,
            ],
        )
        .await
        .expect("the pen door accepts a restore-originated row")
        .get(0);
    assert!(
        !acked,
        "a freshly penned row is not an acked human decision"
    );

    let held: Option<Vec<u8>> = c
        .query_one(
            "SELECT dek_wrapped FROM sync_quarantine WHERE content_digest = $1",
            &[&record.digest],
        )
        .await
        .expect("the penned row is there")
        .get(0);
    assert_eq!(
        held.as_deref(),
        dek,
        "the pen must be holding exactly the key this test meant to give it — a custody arm tested \
         over the wrong pen contents proves nothing about either"
    );
}

/// Did the EVENT come back, whatever happened to its key?
///
/// The twin answers "did the CHART come back", and in every degraded arm the answer is a correct
/// `None` — which is equally consistent with the event never having been applied at all. So the
/// degraded arms need this second, POSITIVE question, or their central claim ("a recovery command
/// must still recover the event") rests on nothing but `released`, a counter inside the very
/// function under test. A release path that deleted the pen row without applying the event would
/// otherwise report `released: 1` over a record that no longer exists anywhere.
pub async fn event_survived(c: &Client, record: &DeadNodeRecord) -> bool {
    c.query_one(
        "SELECT EXISTS (SELECT 1 FROM event_log WHERE content_address = $1)",
        &[&record.digest],
    )
    .await
    .expect("ask whether the event is in the log")
    .get(0)
}

/// How many rows are left in the pen. A released row must LEAVE it; a row still sitting there
/// after a `released` count means the two halves of the release disagree.
pub async fn pen_rows(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM sync_quarantine", &[])
        .await
        .expect("count the pen")
        .get(0)
}

/// The `event_clear.twin` for this record, or `None` when the door withheld custody.
///
/// This is THE assertion of the whole file. A row in `event_log` says the event survived; a row
/// here says the CHART did.
pub async fn twin_after_release(c: &Client, record: &DeadNodeRecord) -> Option<String> {
    c.query_opt(
        "SELECT k.twin FROM event_clear k \
           JOIN event_log e ON e.event_id = k.event_id \
          WHERE e.content_address = $1",
        &[&record.digest],
    )
    .await
    .expect("read the clear view")
    .map(|r| r.get(0))
}

/// Run the shipped binary's `requeue` verb and return (success, stdout, stderr).
///
/// `--metrics` so the counts come back as the JSON object a monitor would parse, which is also the
/// only place `released` / `still_quarantined` are stated machine-readably.
pub fn run_requeue(conn: &str, key_path: &str) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-sync"))
        .args(["requeue", "--conn", conn, "--key", key_path, "--metrics"])
        .output()
        .expect("run the cairn-sync binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Parse the `--metrics` object off stdout. A run that printed nothing parseable is itself a
/// finding, so this panics with both streams rather than returning an Option nobody would read.
pub fn metrics(stdout: &str, stderr: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("requeue --metrics must print one JSON object ({e})\nstdout: {stdout}\nstderr: {stderr}")
    })
}

/// The single stderr line carrying `needle`, panicking with the whole stream if there is none.
///
/// **Why a LINE and not `stderr.contains`.** `requeue` prints one line per record plus one for the
/// run, and a bare `contains` over the whole stream lets a DIFFERENT line satisfy an assertion.
/// That is not hypothetical here: the success line `"requeue: <digest> released through the apply
/// door"` names the same record on the same run, so asserting the digest and the custody-failure
/// phrase separately would pass even if the digest were stripped out of the failure message —
/// precisely the property arm 3 exists to pin. Both halves must land on ONE line or neither counts.
pub fn stderr_line_with<'a>(stderr: &'a str, needle: &str) -> &'a str {
    stderr
        .lines()
        .find(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no stderr line contains {needle:?}\nstderr was:\n{stderr}"))
}

/// Provision a node and leave it holding one penned sealed record, ready for `requeue`.
///
/// Returns the node's key-file path, its signing key, the record, and the TempDir whose drop
/// removes the key files (so no `node.key` litter lands in the crate's working directory).
pub async fn dead_node_with_a_penned_record(
    c: &mut Client,
) -> (TempDir, String, SigningKey, DeadNodeRecord) {
    reset(c).await;
    let (sk, kid) = cairn_event::generate_key().expect("generate this node's signing key");
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"ward-terminal\"}', $1)",
        &[&kid],
    )
    .await
    .expect("enroll the authoring device");
    register_unwrap_key(c, &sk).await;

    let record = author_sealed_record(c, &sk, &kid).await;
    wipe_clinical_tier(c).await;

    let dir = tempfile::tempdir().expect("temp dir for key files");
    let key_path = write_key_file(dir.path(), "node.key", &sk);
    (dir, key_path, sk, record)
}
