//! Issue #568 — `cairn-sync requeue` releases a penned sealed event WITH its custody.
//!
//! # The failure this file exists to catch
//!
//! A solo clinic runs an unattended nightly backup. No passphrase is on the machine, so when the
//! restore comes it cannot open the node's custody export, and every sealed clinical record is
//! **penned** in `sync_quarantine` with its wrapped DEK rather than admitted without its key. That
//! is the deliberate behaviour of DR slice 2d (#554, ADR-0067), and every penned reason the restore
//! prints tells the operator the same remedy: recover the export, then run `cairn-sync requeue`.
//!
//! Now the disk is gone and the pen is the only copy of the record in the world. If `requeue` hands
//! the WRAPPED DEK where the door expects the plaintext, or resolves the wrong key file, or reads
//! the wrong column, then the events release **without custody at exit 0 while the pen row is
//! deleted**. That is permanent, silent loss of every sealed chart, arriving inside the mechanism
//! built to prevent exactly that — #500's own shape, one layer down.
//!
//! # Why this had no test before
//!
//! `do_requeue` gained its custody arm in slice 2d, and every call site in the suite passes `None`
//! for the unwrap secret; only production ever passed a real key. `cmd_requeue`'s `--key` /
//! `--unwrap-key` plumbing — *"where a wrong default would live"* — had no test at all.
//!
//! # What these tests assert, and why it is the twin and not a row count
//!
//! The load-bearing assertion is that **a sealed body OPENS**: `event_clear.twin` reads back the
//! exact text the dead node held. A row count is a trace of the property; the twin IS the property,
//! and it stays right against a future door that writes custody before proving it can be used.
//!
//! ⚠️ #568's own wording — *"counting an `event_dek` row is not enough; that is exactly the
//! assertion that would pass under a double-wrap"* — is not true of THIS door, and a reader should
//! not go looking for the case it describes. `db/020` unseals with `p_dek` FIRST; a double-wrapped
//! value fails that unseal, `v_inner` is left NULL, and the whole custody block is skipped, so a
//! double-wrap leaves no `event_dek` row AND no `event_clear` row. The instruction is still the
//! right one, for the better reason above.
//!
//! # The three arms, and why all three are here
//!
//! `do_requeue`'s custody decision has exactly three outcomes, and a suite that reached only the
//! first would be green while proving almost nothing:
//!
//! 1. **Custody resolves and the DEK opens** — the body comes back (`the_body_opens`).
//! 2. **No custody resolves at all** — the event still releases, sealed (`custody_unresolvable`).
//! 3. **Custody resolves but this key does not open THAT DEK** — a foreign medium; the event
//!    releases, sealed, and says so (`a_penned_dek_from_another_node`).
//!
//! Arm 2 is the anti-vacuity twin of arm 1: without it, a suite that never opened anything at all
//! would still pass arm 1's shape.
//!
//! Skips unless `CAIRN_TEST_PG` is set. Serialized via cairn-node's `db::test_serial_guard` —
//! advisory locks are scoped PER DATABASE, not cluster-wide (#476) — because this file TRUNCATEs
//! tables every other DB-gated suite also uses.

use cairn_event::keys::Secret32;
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
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
const REG_WALL: i64 = 1_780_000_000_000;

/// The medium sequence a penned record carries. A restore passes the record's own `source_seq`
/// (`sync_quarantine.refused_seq` is NOT NULL), so a fixture that passed NULL would be exercising a
/// row shape no restore can produce — which is how the first draft of this file failed.
const PENNED_SOURCE_SEQ: i64 = 1;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

/// Everything a test needs about the one sealed record, read off the node BEFORE its disk "died".
///
/// These four values are exactly what the backup medium carries for a sealed event (slice 2c): the
/// signed bytes, their content address, the DEK wrapped for this node's custody key, and — the one
/// a medium does NOT carry, because it is what we are trying to get back — the clear twin.
struct DeadNodeRecord {
    signed_bytes: Vec<u8>,
    digest: Vec<u8>,
    dek_wrapped: Vec<u8>,
    twin: String,
}

/// Truncate everything this suite touches, so each run starts from a genuinely empty node.
///
/// `node_unwrap_key` MUST be included: it is a SINGLETON that refuses a second, different key
/// (`cairn_register_unwrap_key` — rotation is a separate ceremony), and every run here authors
/// under a FRESH key whose derived unwrap key differs from the last run's. Left standing, the
/// prior run's singleton collides at the first sealed author and every test fails for a reason
/// that has nothing to do with requeue. (Same reasoning as `clinical_pull.rs`'s `reset`.)
async fn reset(c: &Client) {
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
fn write_key_file(dir: &Path, name: &str, sk: &SigningKey) -> String {
    let path = dir.join(name);
    std::fs::write(&path, hex::encode(sk.to_bytes())).expect("write hex-seed key file");
    path.to_str().expect("utf-8 key path").to_string()
}

/// This node's unwrap secret, DERIVED from its signing seed.
///
/// **Derived deliberately — do not switch this to `generate_unwrap_secret`.** `cairn-sync` has no
/// `establish-unwrap-key` command: `unwrap_key::resolve_at_startup` finds its secret either in a
/// `<key>.unwrap` sibling file or, absent one, by deriving it from the signing seed and checking
/// that it matches what `node_unwrap_key` has registered (the pre-ADR-0066 fallback, trap 3 in
/// HANDOVER). Registering a DERIVED key here is what lets these tests drive the shipped binary
/// with `--key` alone. A generated key would satisfy the authoring path and then silently fail to
/// resolve at requeue time — turning a loud failure into a quiet wrong answer.
fn derived_unwrap_secret(sk: &SigningKey) -> Secret32 {
    cairn_event::seal::derive_unwrap_secret(&Secret32::from_bytes(sk.to_bytes()))
}

/// Register this node's own DEK-unwrap public key — the provisioning act ADR-0066 decision 6 made
/// explicit. Without it `ensure_unwrap_key` refuses the first sealed write, so nothing can be
/// authored to pen in the first place.
async fn register_unwrap_key(c: &Client, sk: &SigningKey) {
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
async fn register_chart(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid) {
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
async fn author_sealed_record(c: &mut Client, sk: &SigningKey, kid: &str) -> DeadNodeRecord {
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
async fn wipe_clinical_tier(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, event_dek, event_clear, erasure_shred_log, patient_chart, \
         medication_statement CASCADE",
    )
    .await
    .expect("wipe the clinical tier");
}

/// Pen a record through the REAL door (`db/052`), exactly as a restore does — never a raw INSERT.
///
/// `dek` is passed separately from the record so a test can pen a DEK this node cannot open (the
/// foreign-medium arm) without having to fake the rest of the row.
async fn pen_with_custody(c: &Client, record: &DeadNodeRecord, dek: &[u8]) {
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
    assert!(!acked, "a freshly penned row is not an acked human decision");

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
        Some(dek),
        "the pen must be holding the key: a requeue test over a keyless pen row proves nothing"
    );
}

/// The `event_clear.twin` for this record, or `None` when the door withheld custody.
///
/// This is THE assertion of the whole file. A row in `event_log` says the event survived; a row
/// here says the CHART did.
async fn twin_after_release(c: &Client, record: &DeadNodeRecord) -> Option<String> {
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
fn run_requeue(conn: &str, key_path: &str) -> (bool, String, String) {
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
fn metrics(stdout: &str, stderr: &str) -> serde_json::Value {
    serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("requeue --metrics must print one JSON object ({e})\nstdout: {stdout}\nstderr: {stderr}")
    })
}

/// Provision a node and leave it holding one penned sealed record, ready for `requeue`.
///
/// Returns the node's key-file path, its signing key, the record, and the TempDir whose drop
/// removes the key files (so no `node.key` litter lands in the crate's working directory).
async fn dead_node_with_a_penned_record(
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

// ---------------------------------------------------------------------------
// Arm 1 — the headline: custody survives the pen, and the body opens
// ---------------------------------------------------------------------------

/// **The remedy every restore-penned reason advertises, proved end to end.**
///
/// Design test 4 of slice 2d's plan — *"custody survives the pen, via requeue"* — of which only the
/// first half was ever built: `restore_reads_the_clinical_plane.rs` proves the pen HOLDS the key;
/// until now nothing proved `requeue` RELEASES it correctly.
///
/// Watched failing before it was trusted: with `do_requeue`'s unwrap mutated to pass the wrapped
/// bytes straight through (the double-wrap), this fails on the twin assertion — `event_clear` has
/// no row at all, because `db/020` could not unseal the body and skipped the custody block.
#[tokio::test]
async fn a_penned_sealed_record_releases_with_its_custody_and_the_body_opens() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen_with_custody(&c, &record, &record.dek_wrapped).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(ok, "requeue must succeed\nstdout: {stdout}\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the penned record must be released: {m}");
    assert_eq!(m["still_quarantined"], 0, "nothing should stay held: {m}");

    // THE ASSERTION. Not "a row exists" — the clinician's chart is readable again.
    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "THE BODY MUST OPEN. A release that admits ciphertext whose key is gone is permanent, \
         silent loss of the chart at exit 0 — and on a restored solo node the pen was the last \
         copy in the world.\nstderr: {stderr}"
    );

    // And the custody the node now holds is re-wrapped for itself, so a later crypto-shred can
    // still reach this body. Stated after the twin because it is the trace, not the property.
    let stored: Vec<u8> = c
        .query_one(
            "SELECT d.dek_wrapped FROM event_dek d \
               JOIN event_log e ON e.event_id = d.event_id \
              WHERE e.content_address = $1",
            &[&record.digest],
        )
        .await
        .expect("the released event carries custody")
        .get(0);
    cairn_event::seal::unwrap_dek(&stored, &derived_unwrap_secret(&sk))
        .expect("the stored DEK must open with this node's own custody key");

    let left: i64 = c
        .query_one("SELECT count(*) FROM sync_quarantine", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(left, 0, "a released row leaves the pen");
}

// ---------------------------------------------------------------------------
// Arm 2 — no custody resolves at all: the event still comes back, sealed
// ---------------------------------------------------------------------------

/// **The anti-vacuity twin of arm 1, and `cmd_requeue`'s best-effort arm.**
///
/// `requeue` run under a DIFFERENT node's key: the derived secret does not match what
/// `node_unwrap_key` registered, so `resolve_at_startup` refuses and `cmd_requeue` degrades to
/// `None` rather than aborting. That degradation is deliberate and unlike `cmd_pull` (#554 review
/// finding 3): `requeue` is the recovery command a restore's own output points operators at, and a
/// recovery command that aborts before releasing anything is worse than one that releases without
/// custody.
///
/// Two things must both hold, and they pull in opposite directions: the event must still be
/// recovered, and the operator must not be able to read a clean release as a complete one.
#[tokio::test]
async fn without_resolvable_custody_the_record_still_releases_but_stays_sealed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (dir, _key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen_with_custody(&c, &record, &record.dek_wrapped).await;

    // A stranger's key file: a real, well-formed signing key that is simply not this node's.
    let (stranger_sk, _kid) = cairn_event::generate_key().unwrap();
    let stranger_path = write_key_file(dir.path(), "stranger.key", &stranger_sk);

    let (ok, stdout, stderr) = run_requeue(&base, &stranger_path);
    assert!(
        ok,
        "a recovery command must still recover the EVENT when it cannot recover the KEY\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the event is still recovered: {m}");

    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "with no custody the body must stay SEALED — if this reads back, arm 1 is passing for \
         some reason other than the custody arm and the whole file is vacuous"
    );
    assert!(
        stderr.contains("WITHOUT custody"),
        "the operator must be told they did not get custody, or they will read a clean release \
         as a complete one: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Arm 3 — custody resolves, but not for THAT key
// ---------------------------------------------------------------------------

/// **A foreign medium: the pen holds a DEK wrapped for somebody else's node.**
///
/// The only arm that reaches `do_requeue`'s unwrap-failure branch. Arms 1 and 2 pass a good secret
/// and no secret respectively; here the secret is this node's own and correct, and it simply cannot
/// open this key. The event must still release — losing a record because its key belongs to another
/// node would be the worst of both — and the failure must name the record.
#[tokio::test]
async fn a_penned_dek_from_another_node_releases_the_record_and_says_custody_was_lost() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;

    // Re-wrap this record's real DEK for a stranger's custody key: byte-for-byte what a medium
    // written by another node carries. Every key here is generated at runtime (house rule 6).
    let dek = cairn_event::seal::unwrap_dek(&record.dek_wrapped, &derived_unwrap_secret(&sk))
        .expect("the fixture's own DEK opens with the node that sealed it");
    let stranger_secret =
        cairn_event::seal::generate_unwrap_secret().expect("mint a stranger's custody key");
    let foreign = cairn_event::seal::wrap_dek_for(
        &dek,
        &cairn_event::seal::unwrap_public(&stranger_secret),
    )
    .expect("re-wrap for a stranger");
    pen_with_custody(&c, &record, &foreign).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(
        ok,
        "a key this node cannot open is not a reason to lose the record\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the event is still recovered: {m}");

    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "a DEK that does not open must not somehow produce a clear view"
    );
    assert!(
        stderr.contains("did not open with this node's custody key"),
        "the unwrap failure must be reported per record, naming it: {stderr}"
    );
    assert!(
        stderr.contains(&hex::encode(&record.digest)[..8]),
        "and it must name WHICH record, or an operator cannot act on it: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// The plumbing guard
// ---------------------------------------------------------------------------

/// **`requeue` must never MINT a signing key, however wrong the `--key` path is.**
///
/// `cmd_requeue`'s doc names this as the reason it calls `load_existing_key` rather than the
/// `load_or_create_key` the pull path uses: an operator running `requeue` from the wrong directory
/// would otherwise create a stray signing key and then resolve custody against it — which is not
/// merely useless but actively misleading, since the resulting node has a key file that belongs to
/// nothing. Nothing checked it.
///
/// The event must STILL be released: the same best-effort reasoning as arm 2.
#[tokio::test]
async fn requeue_refuses_a_missing_key_file_rather_than_minting_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (dir, _key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen_with_custody(&c, &record, &record.dek_wrapped).await;

    let absent = dir.path().join("not-here").join("node.key");
    let absent_path = absent.to_str().unwrap().to_string();

    let (ok, stdout, stderr) = run_requeue(&base, &absent_path);
    assert!(
        ok,
        "the recovery command still recovers the event\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(metrics(&stdout, &stderr)["released"], 1);
    assert!(
        !absent.exists(),
        "requeue MINTED a signing key at {} — an operator in the wrong directory now has a key \
         file that belongs to nothing, and custody resolved against it",
        absent.display()
    );
    assert!(
        stderr.contains("WITHOUT custody"),
        "and it must say custody was not obtained: {stderr}"
    );
}
