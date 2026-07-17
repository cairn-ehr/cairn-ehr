//! Issue #199 (review finding B4) — the clinical-plane A→B pull, end to end.
//!
//! The set-union convergence guarantee ("author on A, sync to B, identical
//! projections") is Cairn's flagship property, and until this test it was verified
//! only by hand on the walking-skeleton rigs. This drives the REAL binary — `serve`
//! on node A, `pull` on node B — over real TCP, with events authored through the
//! production medication orchestrators on A, and then asserts the two nodes read
//! byte-identically through every medication projection. It also proves the slice-4
//! attestation token TRAVELS the wire (issue #91's parallel arrays) and re-verifies
//! at B's apply door, so a human vouch replicates without weakening. A second test
//! keeps the review's RED check alive: a pull whose events are refused at the door
//! freezes the watermark (never fakes convergence, never loses events) and the
//! next pull after the repair converges.
//!
//! Skips unless BOTH `CAIRN_TEST_PG` (node A) and `CAIRN_TEST_PG2` (node B) are set.
//! Serialized cluster-wide via cairn-node's `db::test_serial_guard` (both DBs live on
//! the same cluster in CI, and this file TRUNCATEs shared tables on both).
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, cease_medication, change_dose, reconcile_medications, AssertMedicationInput,
    AttestParams, CeaseMedicationInput, ChangeDoseInput, ReconcileInput,
};
use std::path::Path;
use std::process::{Child, Command};
use tempfile::TempDir;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs_a() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}
fn cs_b() -> Option<String> {
    std::env::var("CAIRN_TEST_PG2").ok()
}

/// Fixed local ports for A's serve loop — ONE PER TEST in this file. The serial
/// guard means at most one DB-gated test runs at a time, and the guard below kills
/// the child on drop; but a test re-binding the PREVIOUS test's port can still hit
/// EADDRINUSE from a lingering TIME_WAIT socket of the killed child (std's
/// TcpListener does not set SO_REUSEADDR), so each test owns its own port.
const LISTEN_CONVERGE: &str = "127.0.0.1:39717";
const LISTEN_FREEZE: &str = "127.0.0.1:39718";
const LISTEN_LOWHLC: &str = "127.0.0.1:39719";
const LISTEN_SWEEP: &str = "127.0.0.1:39720";
const LISTEN_REPULL: &str = "127.0.0.1:39721";

/// A realistic PAST HLC wall (ms since epoch, ≈ 2026-06-20) — safely below "now",
/// so A's remote-apply door accepts it (the drift ceiling bounds FUTURE walls only).
const WALL: i64 = 1_782_000_000_000;

/// A validly-signed `note.added` at a CHOSEN HLC wall, from a foreign "node-c"
/// signer — the multi-hop event whose HLC can sit BELOW a node's advanced cursor.
/// The HLC lives inside the signed body (freely set by the signer); the receiving
/// node assigns `seq` at insert. So applying this to A gives it a low HLC but a
/// fresh HIGH seq — exactly the #196 skip trigger. Returns (signed_bytes, event_id).
fn foreign_note(sk: &SigningKey, kid: &str, wall: i64, text: &str) -> (Vec<u8>, String) {
    let event_id = Uuid::now_v7().to_string();
    let body = EventBody {
        event_id: event_id.clone(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "node-c".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": text}),
        attachments: vec![],
        plaintext_twin: Some(format!("Progress note: {text}")),
    };
    (sign(&body, sk).unwrap().signed_bytes, event_id)
}

/// Enroll a foreign signer as a `device` actor on one node (so its `note.added`
/// events pass that node's apply door). Actor enrollment does not travel the
/// clinical plane (#205), so each node does it independently — as a real
/// deployment does.
async fn enroll_device(c: &Client, kid: &str) {
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"node-c\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
}

/// Kill the spawned `serve` child when the test ends (pass or panic) so a leaked
/// listener can never wedge a later run on the fixed port.
struct ServeGuard(Child);
impl Drop for ServeGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Truncate the log + the medication projections + the sync bookkeeping on one node
/// and reset its HLC, so each run starts from a genuinely empty pair of nodes.
async fn reset(c: &Client) {
    // RESTART IDENTITY resets event_log.seq to 1 so the #196 seq-cursor tests see
    // deterministic serving seqs (1, 2, 3 …) rather than a cluster-wide running total.
    //
    // The ADR-0052 custody plane (node_unwrap_key / event_dek / event_clear /
    // erasure_shred_log) MUST be truncated too. node_unwrap_key is a SINGLETON that
    // refuses a second, different key (`cairn_register_unwrap_key` — rotation is a
    // separate ceremony), and each run authors under a FRESH device key whose derived
    // unwrap key differs from the last run's. Without wiping it here, the prior run's
    // singleton survives the truncate and the fresh key collides at the first sealed
    // author (ensure_unwrap_key), failing every sealed-authoring test. event_dek /
    // event_clear are the per-event custody + clear-view rows those authored events
    // populate; erasure_shred_log is the anti-resurrection ledger — all three are
    // per-run state that a genuinely-empty node must not inherit.
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, medication_statement, \
         medication_cessation, medication_dose_event, medication_dose_correction, \
         medication_reconciliation, medication_group_member, medication_projection_flag, \
         medication_attestation, medication_patient_conflict_flag, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log, \
         sync_state, sync_quarantine RESTART IDENTITY CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
}

/// Enroll the same device + human keys on one node. B must know both actors before
/// the pull: actor enrollment does not travel the clinical plane (the registry's
/// sync semantics are #205), so the test performs the owner ceremony on each node —
/// exactly what a real two-site deployment does today.
async fn enroll_actors(c: &Client, kid_device: &str, kid_human: &str) {
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"ward-terminal\"}', $1)",
        &[&kid_device],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_human],
    )
    .await
    .unwrap();
}

/// Write a signing key into `dir` in the EXACT format the `cairn-sync` binary's
/// serve/pull commands read — a hex-encoded 32-byte Ed25519 seed (see the daemon's
/// `load_or_create_key`, which does `hex::decode(text.trim())`). Note this is NOT the
/// same on-disk shape as `keystore::load` (raw bytes / sealed CBOR): the serve/pull
/// verbs use the daemon's own hex loader, so the file MUST be hex or the process
/// rejects it. Returns the `--key` path (a child of the caller's TempDir, which
/// auto-cleans on drop — no node.key litter in the crate cwd). House rule 6: the seed
/// is generated at runtime by `generate_key`, never a literal.
///
/// WHY per-node key files matter here (ADR-0052 born-sealed custody): the serve process
/// on A must run under the SAME key that SEALED A's events — each sealed event's DEK is
/// wrapped for that key's derived unwrap secret, so ONLY that key can unwrap it to
/// re-wrap custody for a puller. The pull process on B runs under B's OWN key so B
/// unwraps the re-wrapped DEK and its apply door gains crypto-shred custody (and hence
/// the clear view it projects from). A shared random key — the old default node.key —
/// is neither, so serve could not unwrap A's DEKs, no custody crossed the wire, and B
/// received the events but could not project the sealed bodies.
fn write_key_file(dir: &Path, name: &str, sk: &SigningKey) -> String {
    let path = dir.join(name);
    std::fs::write(&path, hex::encode(sk.to_bytes())).expect("write hex-seed key file");
    path.to_str().expect("utf-8 key path").to_string()
}

/// Register a node's OWN DEK-unwrap public key so its apply door can take custody of
/// (and later crypto-shred) the sealed events it pulls. Derived from the node's signing
/// seed exactly as the daemon derives it (`derive_unwrap_secret` → `unwrap_public`) and
/// registered through the same in-DB singleton the author-side `ensure_unwrap_key` uses.
/// Without this, the door ADMITS a sealed event but WARNS and withholds custody (no
/// `event_clear` row is written), so the node never projects the sealed body — exactly
/// the born-sealed floor (#92). A real second site performs this owner ceremony once,
/// just like actor enrollment; the custody plane does not replicate.
async fn register_unwrap_key(c: &Client, sk: &SigningKey) {
    let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    let public = cairn_event::seal::unwrap_public(&secret);
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&public.as_slice()],
    )
    .await
    .unwrap();
}

/// One node's complete medication read-state, rendered to comparable strings:
/// the event set (ids + content addresses + whether an attestation is held), the
/// current/past medication views, the group collapse, and the standing vouches.
/// Two nodes holding the same event set MUST render this identically (set-union
/// convergence + ADR-0045 deterministic winners).
async fn snapshot(c: &Client) -> Vec<String> {
    let mut out = Vec::new();
    for row in c
        .query(
            "SELECT event_id::text, encode(content_address, 'hex'), \
                    coalesce(encode(attestation, 'hex'), '-'), \
                    coalesce(encode(attester_key, 'hex'), '-') \
             FROM event_log ORDER BY event_id",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "event {} {} att={} akey={}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2),
            row.get::<_, String>(3)
        ));
    }
    for row in c
        .query(
            "SELECT patient_id::text, medication_id::text, term, \
                    coalesce(dose_amount, '-'), coalesce(dose_unit, '-') \
             FROM patient_medication_current ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "current {} {} {} {}/{}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2),
            row.get::<_, String>(3),
            row.get::<_, String>(4)
        ));
    }
    for row in c
        .query(
            "SELECT patient_id::text, medication_id::text, term \
             FROM patient_medication_past ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "past {} {} {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, String>(2)
        ));
    }
    for row in c
        .query(
            "SELECT medication_id::text, group_id::text \
             FROM medication_group_member ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "group {} -> {}",
            row.get::<_, String>(0),
            row.get::<_, String>(1)
        ));
    }
    for row in c
        .query(
            "SELECT medication_id::text, attester_kid, stale \
             FROM medication_thread_attestation ORDER BY medication_id::text",
            &[],
        )
        .await
        .unwrap()
    {
        out.push(format!(
            "vouch {} by {} stale={}",
            row.get::<_, String>(0),
            row.get::<_, String>(1),
            row.get::<_, bool>(2)
        ));
    }
    out
}

/// Wait until A's serve loop accepts TCP (readiness poll, bounded).
fn wait_listening(addr: &str) {
    for _ in 0..50 {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("serve did not start listening on {addr}");
}

#[tokio::test]
async fn a_to_b_pull_converges_projections_and_ships_the_attestation() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    // --- provision both nodes and enroll the same actors on each ---
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    // Per-node key files for the ADR-0052 custody wire (see write_key_file). A's serve
    // runs under sk_d — the device key that SEALS A's medication events below — so it
    // can unwrap A's per-event DEKs and re-wrap them for the puller. B pulls under its
    // OWN key and pre-registers the matching unwrap key, so B's apply door takes custody
    // of every sealed body it pulls and can therefore project it (without this, B would
    // receive the events by set-union but render an empty chart).
    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    register_unwrap_key(&b, &sk_b).await;

    // --- author a realistic little chart on A through the production orchestrators ---
    let mut a = a; // the orchestrators take &mut Client (they open transactions)
    let patient = Uuid::now_v7();
    let metformin = AssertMedicationInput {
        term: "metformin",
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("500"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2023"),
        started_precision: Some("year"),
    };
    let med1 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None)
        .await
        .unwrap();
    // Dose change WITH an atomic human vouch: the attestation covers assert+change,
    // so the standing vouch must read non-stale on BOTH nodes after sync.
    let vouch = AttestParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
        basis: Some("ward-round review"),
        note: None,
    };
    change_dose(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med1,
        &ChangeDoseInput {
            dose_amount: Some("1000"),
            dose_unit: Some("mg"),
            effective: Some("2026-06"),
            effective_precision: Some("month"),
            info_source: "clinician",
            reason: Some("HbA1c above target"),
        },
        Some(&vouch),
    )
    .await
    .unwrap();
    // A ceased thread (exercises the past view across the wire).
    let atorva = AssertMedicationInput {
        term: "atorvastatin",
        dose_amount: Some("40"),
        ..metformin
    };
    let med2 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &atorva, None)
        .await
        .unwrap();
    cease_medication(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med2,
        &CeaseMedicationInput {
            stopped: Some("2026-05"),
            stopped_precision: Some("month"),
            reason: Some("myalgia"),
        },
        None,
    )
    .await
    .unwrap();
    // A reconciled duplicate pair (exercises the group collapse across the wire).
    let ibu = AssertMedicationInput {
        term: "ibuprofen",
        dose_amount: Some("400"),
        ..metformin
    };
    let med3 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None)
        .await
        .unwrap();
    let med4 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None)
        .await
        .unwrap();
    reconcile_medications(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        med3,
        med4,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
    )
    .await
    .unwrap();

    // --- the wire: serve on A, one pull on B, through the real binary ---
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    // serve under A's authoring key (--key key_a): only that key can unwrap A's DEKs to
    // re-wrap sealed-body custody for the puller (ADR-0052).
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_CONVERGE,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_CONVERGE);
    // pull under B's own key (--key key_b): B presents the matching unwrap cert and
    // gains custody of the sealed bodies it replicates.
    let pull = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_CONVERGE,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("run pull");
    assert!(
        pull.status.success(),
        "pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull.stdout),
        String::from_utf8_lossy(&pull.stderr)
    );

    // --- the flagship assertion: A and B read identically, projection by projection ---
    let snap_a = snapshot(&a).await;
    let snap_b = snapshot(&b).await;
    assert_eq!(
        snap_a, snap_b,
        "A and B must render identical medication read-state after one pull"
    );

    // Non-vacuous floor: the snapshot really carries the chart we authored.
    let joined = snap_a.join("\n");
    assert!(
        joined.contains("metformin 1000/mg"),
        "the synced dose change drives B's current dose:\n{joined}"
    );
    assert!(
        joined.contains("past") && joined.contains("atorvastatin"),
        "the ceased thread reads as past on both nodes:\n{joined}"
    );
    assert!(
        joined.contains(&format!("vouch {med1} by {kid_h} stale=false")),
        "the human vouch travelled, re-verified, and reads current:\n{joined}"
    );
    let ibu_groups: Vec<&str> = snap_a
        .iter()
        .filter(|l| l.starts_with("group"))
        .map(|s| s.as_str())
        .collect();
    assert_eq!(
        ibu_groups.len(),
        2,
        "both duplicate threads joined one reconciled group:\n{joined}"
    );
    // …and it is ONE group: both membership lines name the same group id
    // (snapshot lines read "group <member> -> <group_id>").
    let gids: Vec<&str> = ibu_groups
        .iter()
        .map(|l| l.rsplit(' ').next().unwrap())
        .collect();
    assert_eq!(
        gids[0], gids[1],
        "both duplicate threads collapsed into the SAME group:\n{joined}"
    );

    // The quarantine stayed empty: nothing on this wire needed penning.
    let penned: i64 = b
        .query_one("SELECT count(*) FROM sync_quarantine", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(penned, 0, "a clean pull pens nothing");
}

#[tokio::test]
async fn refused_apply_freezes_the_watermark_and_recovers_without_loss() {
    // The RED check the PR #221 review asked to keep as a standing test: prove the
    // convergence test DISCRIMINATES (a pull whose events cannot apply must not fake
    // convergence), pinned to the A1 watermark discipline. Events authored by an
    // actor B has not enrolled VERIFY on the wire (the signature is self-described)
    // but are REFUSED at B's apply door — that is a freeze, not an exclusion: the
    // pull completes, nothing applies, nothing is penned (the quarantine is for
    // UNVERIFIABLE bytes only), and the watermark holds at the contiguous applied
    // prefix so the refused events stay on the wire. Fix the cause and the very
    // next pull converges — delayed, never lost.
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_d, kid_d) = generate_key().unwrap();
    let (_sk_h, kid_h) = generate_key().unwrap();
    // Enroll on A ONLY — B does not know these actors yet (the owner ceremony a
    // real second site performs before its first pull, deliberately omitted).
    enroll_actors(&a, &kid_d, &kid_h).await;

    // Per-node key files for the ADR-0052 custody wire (see write_key_file). A's serve
    // runs under sk_d (which seals A's metformin); B pulls under its own key with its
    // unwrap key pre-registered. The custody plane is orthogonal to the actor-enrollment
    // freeze under test: the FIRST pull still refuses everything (author unknown at B)
    // BEFORE the custody step, so nothing applies; once the author is enrolled the
    // re-offered sealed event applies WITH custody and B projects it — the delayed, never
    // lost guarantee, now proven all the way through a sealed body.
    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    register_unwrap_key(&b, &sk_b).await;

    let mut a = a;
    let patient = Uuid::now_v7();
    assert_medication(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        &AssertMedicationInput {
            term: "metformin",
            inn_code: None,
            formulation: Some("tablet"),
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "patient-reported",
            started: Some("2023"),
            started_precision: Some("year"),
        },
        None,
    )
    .await
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    // serve under A's authoring key so the custody sidecar can travel (ADR-0052).
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_FREEZE,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_FREEZE);
    // pull under B's own key so the recovered pull gains sealed-body custody.
    let pull_cmd = || {
        Command::new(bin)
            .args([
                "pull",
                "--conn",
                &base_b,
                "--peer",
                LISTEN_FREEZE,
                "--peer-name",
                "node-a",
                "--key",
                &key_b,
            ])
            .output()
            .expect("run pull")
    };

    // --- pull while B does not know the author: refuse-and-hold, loudly logged ---
    let pull = pull_cmd();
    assert!(
        pull.status.success(),
        "a freeze is an availability decision, not an integrity failure — the pull \
         itself completes\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull.stdout),
        String::from_utf8_lossy(&pull.stderr)
    );
    let (applied, penned, cursor): (i64, i64, i64) = {
        let row = b
            .query_one(
                "SELECT (SELECT count(*) FROM event_log), \
                        (SELECT count(*) FROM sync_quarantine), \
                        last_seq \
                 FROM sync_state WHERE peer = 'node-a'",
                &[],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(
        applied, 0,
        "nothing applied while the author is unknown at B"
    );
    assert_eq!(
        penned, 0,
        "nothing penned — these bytes VERIFY; the pen is for unverifiable bytes"
    );
    assert_eq!(
        cursor, 0,
        "the seq cursor FROZE at the contiguous applied prefix, so the refused \
         events stay on the wire (issue #196)"
    );

    // --- fix the cause (enroll the actors on B) and pull again: no loss ---
    enroll_actors(&b, &kid_d, &kid_h).await;
    let pull2 = pull_cmd();
    assert!(
        pull2.status.success(),
        "post-repair pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull2.stdout),
        String::from_utf8_lossy(&pull2.stderr)
    );
    assert_eq!(
        snapshot(&a).await,
        snapshot(&b).await,
        "after the repair the next pull converges — the refusal was a delay, never a loss"
    );
}

/// Issue #196 (the headline regression guard): an event that lands on A with an HLC
/// BELOW B's advanced cursor — a multi-hop arrival from a third node — must still
/// converge to B. The old HLC watermark skipped it forever (it sorts below B's
/// watermark, so B never re-serves it); the seq cursor cannot, because the late
/// arrival got a fresh high seq on A regardless of its low HLC.
#[tokio::test]
async fn low_hlc_below_cursor_still_converges() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;

    // A foreign "node-c" signer, enrolled on BOTH nodes so each door admits its notes.
    let (sk_c, kid_c) = generate_key().unwrap();
    enroll_device(&a, &kid_c).await;
    enroll_device(&b, &kid_c).await;

    // Per-node key files (ADR-0052). These events are UNSEALED (foreign note.added via
    // apply_remote_event), so no custody travels and no unwrap key is registered — but
    // serve/pull still run under DISTINCT per-node keys, mirroring a real two-site
    // deployment and keeping the wire uniform with the sealed-body tests above.
    let keydir = TempDir::new().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &generate_key().unwrap().0);
    let key_b = write_key_file(keydir.path(), "node-b.key", &generate_key().unwrap().0);

    // Two HIGH-HLC events land on A (seqs 1, 2) via A's own apply door.
    let (hi1, _) = foreign_note(&sk_c, &kid_c, WALL + 20_000, "high one");
    let (hi2, _) = foreign_note(&sk_c, &kid_c, WALL + 30_000, "high two");
    for e in [&hi1, &hi2] {
        a.execute("SELECT apply_remote_event($1)", &[e])
            .await
            .unwrap();
    }

    // Serve A; pull B once. B applies both and checkpoints last_seq(node-a) = 2.
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_LOWHLC,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_LOWHLC);
    let pull1 = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_LOWHLC,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("run pull 1");
    assert!(
        pull1.status.success(),
        "pull 1: {}",
        String::from_utf8_lossy(&pull1.stderr)
    );
    let cursor: i64 = b
        .query_one("SELECT last_seq FROM sync_state WHERE peer='node-a'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(cursor, 2, "B checkpointed the seq cursor at 2");

    // The multi-hop event: a LOW HLC (below both pulled events) lands on A LATE,
    // getting a fresh seq (3). This is the exact event the HLC watermark skipped.
    let (low, low_id) = foreign_note(&sk_c, &kid_c, WALL + 10_000, "late low-HLC arrival");
    a.execute("SELECT apply_remote_event($1)", &[&low])
        .await
        .unwrap();

    // Pull B again (incremental). On the OLD HLC code B fetches hlc >= watermark, so
    // the low-HLC event is never served and this count stays 0. On the seq cursor B
    // fetches seq > 2 → seq 3 → the event applies.
    let pull2 = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_LOWHLC,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("run pull 2");
    assert!(
        pull2.status.success(),
        "pull 2: {}",
        String::from_utf8_lossy(&pull2.stderr)
    );
    let present: i64 = b
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id::text = $1",
            &[&low_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        present, 1,
        "the low-HLC multi-hop event converged to B (issue #196)"
    );
}

/// Issue #196: the full sweep is the correctness floor for the residual BIGSERIAL
/// out-of-order-commit skip. Simulate the skip by forcing B's cursor PAST an event's
/// seq; an incremental pull then cannot fetch it (`seq > cursor` excludes it), and a
/// `--full` sweep (seq > 0) reconciles it.
#[tokio::test]
async fn full_sweep_reconciles_a_skipped_seq() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_c, kid_c) = generate_key().unwrap();
    enroll_device(&a, &kid_c).await;
    enroll_device(&b, &kid_c).await;

    // Per-node key files (ADR-0052). Unsealed events, so no custody travels — distinct
    // per-node serve/pull keys only, uniform with the sealed-body tests.
    let keydir = TempDir::new().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &generate_key().unwrap().0);
    let key_b = write_key_file(keydir.path(), "node-b.key", &generate_key().unwrap().0);

    // Two events on A (seqs 1, 2). e2 is the one B will "skip".
    let (e1, _) = foreign_note(&sk_c, &kid_c, WALL + 10_000, "one");
    a.execute("SELECT apply_remote_event($1)", &[&e1])
        .await
        .unwrap();
    let (e2, id2) = foreign_note(&sk_c, &kid_c, WALL + 20_000, "two");
    a.execute("SELECT apply_remote_event($1)", &[&e2])
        .await
        .unwrap();

    // Force B's cursor to 2 WITHOUT applying either event (the commit-race skip).
    b.execute(
        "INSERT INTO sync_state (peer, last_seq) VALUES ('node-a', 2)
         ON CONFLICT (peer) DO UPDATE SET last_seq = 2",
        &[],
    )
    .await
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_SWEEP,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_SWEEP);

    // Incremental pull: seq > 2 fetches nothing → e2 stays missing.
    let inc = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_SWEEP,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("incremental pull");
    assert!(
        inc.status.success(),
        "inc pull: {}",
        String::from_utf8_lossy(&inc.stderr)
    );
    let before: i64 = b
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id::text = $1",
            &[&id2],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        before, 0,
        "incremental cannot reach a seq below the forced cursor"
    );

    // Full sweep: seq > 0 fetches everything → e2 reconciled.
    let full = Command::new(bin)
        .args([
            "pull",
            "--full",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_SWEEP,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("full pull");
    assert!(
        full.status.success(),
        "full pull: {}",
        String::from_utf8_lossy(&full.stderr)
    );
    let after: i64 = b
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id::text = $1",
            &[&id2],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after, 1,
        "the full sweep is the correctness floor (issue #196)"
    );
}

/// ADR-0004 "the watermark is a hint": a full sweep from seq 0 re-applies the whole
/// log as set-union no-ops and reaches an identical read-state (idempotent).
#[tokio::test]
async fn repull_from_zero_converges() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_c, kid_c) = generate_key().unwrap();
    enroll_device(&a, &kid_c).await;
    enroll_device(&b, &kid_c).await;

    // Per-node key files (ADR-0052). Unsealed events, so no custody travels — distinct
    // per-node serve/pull keys only, uniform with the sealed-body tests.
    let keydir = TempDir::new().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &generate_key().unwrap().0);
    let key_b = write_key_file(keydir.path(), "node-b.key", &generate_key().unwrap().0);

    for i in 0..3 {
        let (e, _) = foreign_note(&sk_c, &kid_c, WALL + 10_000 * (i + 1), "n");
        a.execute("SELECT apply_remote_event($1)", &[&e])
            .await
            .unwrap();
    }
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_REPULL,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_REPULL);
    let pull = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_REPULL,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("pull");
    assert!(
        pull.status.success(),
        "pull: {}",
        String::from_utf8_lossy(&pull.stderr)
    );
    let count1: i64 = b
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);

    // Rewind the cursor to 0 and full-sweep: must be a set-union no-op.
    b.execute(
        "UPDATE sync_state SET last_seq = 0 WHERE peer='node-a'",
        &[],
    )
    .await
    .unwrap();
    let full = Command::new(bin)
        .args([
            "pull",
            "--full",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_REPULL,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("full pull");
    assert!(
        full.status.success(),
        "full pull: {}",
        String::from_utf8_lossy(&full.stderr)
    );
    let count2: i64 = b
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count1, count2, "re-pull from 0 is idempotent (ADR-0004)");
    assert!(count1 >= 3, "non-vacuous: the chart really replicated");
}
