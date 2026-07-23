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
use cairn_event::{event_address, generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, cease_medication, change_dose, reconcile_medications, AssertMedicationInput,
    AttestParams, CeaseMedicationInput, ChangeDoseInput, ReconcileInput,
};
use cairn_node::shred::shred_event;
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

/// Fixed local ports for A's serve loop — ONE PER TEST in this file, and ALL
/// BELOW the ephemeral-port floor. Two separate constraints meet here:
/// - ONE PER TEST: the serial guard means at most one DB-gated test runs at a
///   time, and the guard below kills the child on drop; but a test re-binding
///   the PREVIOUS test's port can still hit EADDRINUSE from a lingering
///   TIME_WAIT socket of the killed child (std's TcpListener does not set
///   SO_REUSEADDR), so each test owns its own port.
/// - BELOW THE EPHEMERAL FLOOR (issue #263): the kernel assigns local ports for
///   ordinary outbound connections from its ephemeral range (Linux 32768–60999,
///   macOS 49152–65535). The previous 397xx ports sat inside Linux's range, so
///   on CI any transient outbound connection could hold one at bind time — the
///   serve child died with EADDRINUSE and the test burned the full
///   wait_listening ceiling. Ports below 32768 are never auto-assigned; the
///   guard test below enforces the floor.
const LISTEN_CONVERGE: &str = "127.0.0.1:25717";
const LISTEN_FREEZE: &str = "127.0.0.1:25718";
const LISTEN_LOWHLC: &str = "127.0.0.1:25719";
const LISTEN_SWEEP: &str = "127.0.0.1:25720";
const LISTEN_REPULL: &str = "127.0.0.1:25721";
const LISTEN_SEALED: &str = "127.0.0.1:25722";
const LISTEN_GUARD: &str = "127.0.0.1:25723";
const LISTEN_SEALSCOPE: &str = "127.0.0.1:25724";
const LISTEN_FORWARD: &str = "127.0.0.1:25725";

/// Every fixed listen port in this file — a NEW test's port must be added here so
/// the ephemeral-range guard below covers it.
const ALL_LISTEN: [&str; 9] = [
    LISTEN_CONVERGE,
    LISTEN_FREEZE,
    LISTEN_LOWHLC,
    LISTEN_SWEEP,
    LISTEN_REPULL,
    LISTEN_SEALED,
    LISTEN_GUARD,
    LISTEN_SEALSCOPE,
    LISTEN_FORWARD,
];

/// Guard for issue #263: a fixed listen port must sit BELOW every kernel's
/// ephemeral-port floor (Linux defaults to 32768–60999, macOS to 49152–65535).
/// The kernel hands out local ports from that range to ORDINARY OUTBOUND
/// connections — the wait_listening probes, the pull clients, every Postgres
/// session — so a fixed listener inside it can find its port already taken the
/// moment it binds: the serve child dies with EADDRINUSE and the test burns the
/// full wait_listening ceiling before panicking. Ports below the floor can only
/// collide with an explicit listener, which nothing on a CI runner is.
/// Runs without the DB gate, so it holds even where CAIRN_TEST_PG is unset.
#[test]
fn listen_ports_sit_below_every_ephemeral_floor() {
    for addr in ALL_LISTEN {
        let port: u16 = addr
            .rsplit(':')
            .next()
            .expect("addr has a port")
            .parse()
            .expect("port parses");
        assert!(
            port < 32768,
            "{addr}: port {port} is inside an ephemeral range (Linux floor 32768); \
             an outbound connection can steal it before the serve child binds (issue #263)"
        );
    }
}

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
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
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
///
/// The ceiling is deliberately generous (60 s, issue #238): under a parallel
/// `cargo test --workspace` the freshly-spawned serve binary competes with every
/// other suite for CPU, and the original 5 s ceiling flaked intermittently on
/// loaded machines. The poll returns the moment the socket accepts, so a large
/// ceiling costs nothing on the happy path — it only buys headroom on the slow one.
fn wait_listening(addr: &str) {
    for _ in 0..600 {
        if std::net::TcpStream::connect(addr).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("serve did not start listening on {addr} within 60s");
}

/// Count the ADR-0052 custody rows a node holds for one content event: the wrapped
/// DEK (`event_dek`) and the operational clear shadow (`event_clear`). `(1, 1)` means
/// full crypto-shred custody + a projectable clear view; `(0, 0)` means the node holds
/// only the sealed ciphertext (never replicated, or shredded). Mirrors seal_apply.rs.
async fn custody_count(c: &Client, event_id: &str) -> (i64, i64) {
    let dek: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    let clear: i64 = c
        .query_one(
            "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    (dek, clear)
}

/// Count the `medication_statement` projection rows for one immortal thread. The seal
/// scrub is content_address-PRECISE, so this per-thread count is what distinguishes a
/// shredded thread (0) from a surviving sibling (1) on the very same chart.
async fn statement_count_for_med(c: &Client, medication_id: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid",
        &[&medication_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Resolve a thread's `medication_id` to its content event's id via the clear shadow.
/// `assert_medication` returns the THREAD id (immortal, §3.15), but a shred targets the
/// CONTENT EVENT — so the test needs this hop. Reads `event_clear` (present only where
/// the node holds custody), which the author always does after a local sealed submit.
async fn content_event_id_of(c: &Client, medication_id: Uuid) -> String {
    c.query_one(
        "SELECT event_id::text FROM event_clear WHERE body ->> 'medication_id' = $1",
        &[&medication_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
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
    let med1 = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None, None,
    )
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
        None,
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
    let med2 = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &atorva, None, None,
    )
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
    let med3 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None, None)
        .await
        .unwrap();
    let med4 = assert_medication(&mut a, &sk_d, &kid_d, "node-a", patient, &ibu, None, None)
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

/// ADR-0052 born-sealed erasability, proven END TO END through the REAL binaries
/// (issues #189 / #92). This is the walking-skeleton thread the whole slice was for:
///
///   1. A seal-submits a medication assert (born sealed under a fresh per-event DEK);
///      it projects on A with custody.
///   2. serve A / pull B (real `cairn-sync` binary): the custody sidecar re-wraps the
///      DEK for B, so B gains crypto-shred custody, the clear shadow, AND the projection
///      — A→B projection equality for a SEALED body. We also open B's re-wrapped DEK
///      with B's OWN secret in-process, proving the custody is genuinely B's, not A's.
///   3. A crypto-shreds the content event (patient-request basis). Custody, shadow, and
///      projection vanish on A; the append-only log rows and their signatures survive.
///   4. pull B again: the tombstone propagates and B's apply door scrubs B's custody +
///      shadow + projection too — the shred travelled, not just the content.
///   5. RESTORE HALF (§3.8 "restore replays the shred"): wipe B entirely and full-sweep
///      from A. B re-admits the sealed row but A's serve EXCLUDES the shredded DEK (the
///      wire-level half of the guarantee), so B converges to NO custody, NO projection,
///      tombstone + shred-log present. Set-union re-delivery resurrects NOTHING.
///
/// Skips unless BOTH CAIRN_TEST_PG (A) and CAIRN_TEST_PG2 (B) are set.
#[tokio::test]
async fn sealed_medication_syncs_with_custody_then_shred_propagates() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    // --- provision both nodes; enroll the same actors on each (registry does not sync) ---
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_d, kid_d) = generate_key().unwrap();
    let (_sk_h, kid_h) = generate_key().unwrap();
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    // Per-node key files for the custody wire (see write_key_file). A's serve runs under
    // sk_d — the key that SEALS A's medication below — so it can unwrap A's per-event DEK
    // to re-wrap it for B. B pulls under its OWN key and pre-registers the matching unwrap
    // key, so B's apply door takes custody of the sealed body and can project it.
    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    register_unwrap_key(&b, &sk_b).await;

    // --- 1. A seal-submits a medication assert; it projects on A with custody ---
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
    let med = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None, None,
    )
    .await
    .unwrap();
    let content_eid = content_event_id_of(&a, med).await;
    // A holds full custody + the projection, and the log row is sealed ciphertext.
    assert_eq!(
        custody_count(&a, &content_eid).await,
        (1, 1),
        "A holds custody + shadow for its own sealed assert"
    );
    assert_eq!(
        statement_count_for_med(&a, med).await,
        1,
        "A projects the sealed medication through its clear shadow"
    );
    let a_sealed: bool = a
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(a_sealed, "the authored medication row is born sealed");

    // --- 2. the wire: serve A, pull B; the custody sidecar flows ---
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_SEALED,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_SEALED);
    let pull_b = || {
        // A closure so the shred-propagation pull below reuses the exact same wire.
        Command::new(bin)
            .args([
                "pull",
                "--conn",
                &base_b,
                "--peer",
                LISTEN_SEALED,
                "--peer-name",
                "node-a",
                "--key",
                &key_b,
            ])
            .output()
            .expect("run pull")
    };
    let pull = pull_b();
    assert!(
        pull.status.success(),
        "pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull.stdout),
        String::from_utf8_lossy(&pull.stderr)
    );

    // B gained custody, shadow, and projection for the SEALED body.
    assert_eq!(
        custody_count(&b, &content_eid).await,
        (1, 1),
        "B took custody + shadow of the sealed body it replicated"
    );
    let b_sealed: bool = b
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(b_sealed, "B stored the ciphertext, sealed");
    assert_eq!(
        statement_count_for_med(&b, med).await,
        1,
        "B projected the sealed medication via its own clear shadow"
    );
    // A→B projection equality for the sealed event: both nodes read identically.
    assert_eq!(
        snapshot(&a).await,
        snapshot(&b).await,
        "A and B render identical medication read-state for the sealed body"
    );

    // The custody B holds is genuinely B's OWN: open B's re-wrapped DEK with B's secret
    // (derived from sk_b exactly as the daemon derives it), then unseal B's stored
    // ciphertext with it. If A's DEK had merely been copied across, B's secret could not
    // open it — this is the crux of the born-sealed custody wire.
    let wrapped_for_b: Vec<u8> = b
        .query_one(
            "SELECT dek_wrapped FROM event_dek WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    let b_secret = cairn_event::seal::derive_unwrap_secret(&sk_b.to_bytes());
    let b_dek = cairn_event::seal::unwrap_dek(&wrapped_for_b, &b_secret)
        .expect("B's own secret opens the DEK re-wrapped for B");
    let b_body_text: String = b
        .query_one(
            "SELECT body::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    let b_container: serde_json::Value = serde_json::from_str(&b_body_text).unwrap();
    let (_payload, twin) =
        cairn_event::seal::unseal_event_payload(&b_container, &b_dek, &content_eid)
            .expect("B opens the sealed body with its own DEK");
    assert!(
        twin.contains("metformin"),
        "the twin under B's seal is the real clinical text, got: {twin}"
    );

    // --- 3. A crypto-shreds the content event (patient-request basis) ---
    let target: Uuid = content_eid.parse().unwrap();
    shred_event(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        target,
        "patient request — no retention basis",
        None,
    )
    .await
    .expect("A authors + submits the audited crypto-shred");

    // On A: custody, shadow, projection gone; the append-only log rows + signature stay.
    assert_eq!(
        custody_count(&a, &content_eid).await,
        (0, 0),
        "the shred scrubbed A's custody + shadow"
    );
    assert_eq!(
        statement_count_for_med(&a, med).await,
        0,
        "the shred scrubbed A's projection"
    );
    let a_log_rows: i64 = a
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        a_log_rows, 2,
        "A keeps BOTH append-only rows: the sealed target and the tombstone"
    );
    let a_verifies: bool = a
        .query_one(
            "SELECT cairn_verify(signed_bytes) FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        a_verifies,
        "the sealed row's signature over ciphertext survives the shred on A"
    );

    // --- 4. pull B again: the shred propagates and scrubs B's derived surfaces ---
    let pull2 = pull_b();
    assert!(
        pull2.status.success(),
        "second pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&pull2.stdout),
        String::from_utf8_lossy(&pull2.stderr)
    );
    let b_shred_log: i64 = b
        .query_one(
            "SELECT count(*) FROM erasure_shred_log WHERE target_event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        b_shred_log, 1,
        "the tombstone travelled: B logged the shred"
    );
    assert_eq!(
        custody_count(&b, &content_eid).await,
        (0, 0),
        "the propagated shred scrubbed B's custody + shadow"
    );
    assert_eq!(
        statement_count_for_med(&b, med).await,
        0,
        "the propagated shred scrubbed B's projection"
    );
    let b_log_rows: i64 = b
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        b_log_rows, 2,
        "B keeps both append-only rows after the shred (target + tombstone)"
    );
    let b_verifies: bool = b
        .query_one(
            "SELECT cairn_verify(signed_bytes) FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        b_verifies,
        "the sealed row's signature still verifies on B after the shred"
    );

    // --- 5. RESTORE HALF (§3.8): wipe B entirely, full-sweep from A ---
    // A genuinely-empty B re-registers its custody capability (owner ceremony) and pulls
    // everything from seq 0. The proof is that B is FULLY custody-capable, so the ONLY
    // reason it gains no custody for the sealed row is that A's serve refuses to emit a
    // shredded event's DEK — set-union may re-deliver the ciphertext forever, the key
    // never comes back.
    reset(&b).await;
    enroll_actors(&b, &kid_d, &kid_h).await;
    register_unwrap_key(&b, &sk_b).await;
    let full = Command::new(bin)
        .args([
            "pull",
            "--full",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_SEALED,
            "--peer-name",
            "node-a",
            "--key",
            &key_b,
        ])
        .output()
        .expect("run full pull");
    assert!(
        full.status.success(),
        "full pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&full.stdout),
        String::from_utf8_lossy(&full.stderr)
    );

    // The sealed row is re-admitted…
    let readmitted: bool = b
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .expect("the sealed row was re-admitted by set-union")
        .get(0);
    assert!(
        readmitted,
        "the ciphertext re-delivers (append-only set-union)"
    );
    // …but NO custody (A's serve excluded the shredded DEK) and NO projection…
    assert_eq!(
        custody_count(&b, &content_eid).await,
        (0, 0),
        "restore grants NO custody: A's serve excludes a shredded event's DEK (wire-level shred)"
    );
    assert_eq!(
        statement_count_for_med(&b, med).await,
        0,
        "restore resurrects NO projection — nothing to project without the clear shadow"
    );
    // …and the shred log is present again (rebuilt from the re-delivered tombstone).
    let restored_shred_log: i64 = b
        .query_one(
            "SELECT count(*) FROM erasure_shred_log WHERE target_event_id = $1::text::uuid",
            &[&content_eid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        restored_shred_log, 1,
        "restore replays the shred: the tombstone + shred log come back, the content does not"
    );
}

/// Never-over-erase precision pin (Task 9 minor, reviews requested). `cairn_execute_shred`
/// scrubs by `content_address`, so shredding ONE thread must leave a SIBLING thread on the
/// SAME chart fully intact — custody, shadow, and projection. Single node (no sync needed):
/// author two sealed asserts, shred one, assert the other survives.
///
/// Skips unless CAIRN_TEST_PG (node A) is set.
#[tokio::test]
async fn shred_one_thread_leaves_the_sibling_projection_intact() {
    let Some(base_a) = cs_a() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let (sk_d, kid_d) = generate_key().unwrap();
    let (_sk_h, kid_h) = generate_key().unwrap();
    enroll_actors(&a, &kid_d, &kid_h).await;

    let mut a = a;
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
    // Two sealed threads on ONE chart. `victim` will be shredded; `survivor` must not be
    // touched — its content_address is different, and the scrub is content_address-precise.
    let victim = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None, None,
    )
    .await
    .unwrap();
    let atorva = AssertMedicationInput {
        term: "atorvastatin",
        dose_amount: Some("40"),
        ..metformin
    };
    let survivor = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &atorva, None, None,
    )
    .await
    .unwrap();
    let victim_eid = content_event_id_of(&a, victim).await;

    // Both project before the shred.
    assert_eq!(statement_count_for_med(&a, victim).await, 1);
    assert_eq!(statement_count_for_med(&a, survivor).await, 1);

    // Shred ONLY the victim thread's content event.
    shred_event(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        victim_eid.parse().unwrap(),
        "patient request — no retention basis",
        None,
    )
    .await
    .expect("the victim thread is shredded");

    // The victim is scrubbed…
    assert_eq!(
        custody_count(&a, &victim_eid).await,
        (0, 0),
        "the shredded thread loses custody + shadow"
    );
    assert_eq!(
        statement_count_for_med(&a, victim).await,
        0,
        "the shredded thread's projection is gone"
    );
    // …and the sibling survives, whole.
    let survivor_eid = content_event_id_of(&a, survivor).await;
    assert_eq!(
        custody_count(&a, &survivor_eid).await,
        (1, 1),
        "the sibling thread keeps its custody + shadow — the scrub never over-erases"
    );
    assert_eq!(
        statement_count_for_med(&a, survivor).await,
        1,
        "the sibling thread's projection survives untouched (content_address-precise scrub)"
    );
}

/// ADR-0052 wire-level shred guard — ISOLATES the serve-query CASE branch
/// (`CASE WHEN s.target_event_id IS NULL THEN encode(d.dek_wrapped,'hex') END`, the
/// EventsAfterSeq query in main.rs). In the ordinary crypto-shred path `cairn_execute_shred`
/// DELETEs the `event_dek` row, so the LEFT JOIN already yields NULL and the CASE is never
/// the thing that excludes a DEK — it is defense-in-depth with no test, and a maintainer
/// could drop it with every existing test still green. This test constructs the ONE state
/// the CASE alone defends: an event with a LIVE `event_dek` row (custody present) that ALSO
/// carries an `erasure_shred_log` row (the future custody-rotation path where a DEK row can
/// co-exist with a shred-log entry). It drives the REAL serve binary, so removing the CASE
/// from main.rs breaks this test (a copied SQL could not protect against that).
///
///   Phase 1 (non-vacuous baseline): pull with NO shred-log row → B GAINS custody, proving
///     the live `event_dek` row genuinely ships its DEK over this wire.
///   Phase 2 (the guard): manually INSERT an `erasure_shred_log` row for the SAME event
///     WITHOUT deleting `event_dek`, wipe B, pull again → B gains NO custody. Since A's
///     `event_dek` row is still live, the ONLY thing that withheld the DEK is the serve CASE.
///
/// Skips unless BOTH CAIRN_TEST_PG (A) and CAIRN_TEST_PG2 (B) are set.
#[tokio::test]
async fn serve_case_excludes_dek_for_a_shred_logged_event_with_live_custody() {
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
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    register_unwrap_key(&b, &sk_b).await;

    // A authors one sealed medication assert → a LIVE event_dek row on A.
    let mut a = a;
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
    let med = assert_medication(
        &mut a, &sk_d, &kid_d, "node-a", patient, &metformin, None, None,
    )
    .await
    .unwrap();
    let content_eid = content_event_id_of(&a, med).await;
    assert_eq!(
        custody_count(&a, &content_eid).await,
        (1, 1),
        "A holds live custody for its sealed assert"
    );

    // Serve A under its authoring key so DEKs can be re-wrapped for a puller.
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_GUARD,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_GUARD);
    let full_pull_b = || {
        Command::new(bin)
            .args([
                "pull",
                "--full",
                "--conn",
                &base_b,
                "--peer",
                LISTEN_GUARD,
                "--peer-name",
                "node-a",
                "--key",
                &key_b,
            ])
            .output()
            .expect("run pull")
    };

    // --- Phase 1 (non-vacuous baseline): NO shred-log row → the DEK ships, B gets custody ---
    let p1 = full_pull_b();
    assert!(
        p1.status.success(),
        "phase-1 pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&p1.stdout),
        String::from_utf8_lossy(&p1.stderr)
    );
    assert_eq!(
        custody_count(&b, &content_eid).await,
        (1, 1),
        "baseline: with no shred-log row the live event_dek row genuinely ships its DEK, \
         so B gains custody — this is what proves the guard is non-vacuous (the test fails \
         if the CASE is dropped, because phase 2 would then also grant custody)"
    );

    // --- Phase 2 (the guard): shred-log row present, event_dek STILL LIVE on A ---
    // Directly insert the shred-log row WITHOUT calling cairn_execute_shred, so A's
    // event_dek row survives — exactly the state (shred-logged + custody-present) that the
    // ordinary DELETE path never leaves behind, and the CASE alone must defend.
    a.execute(
        "INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis) \
         VALUES ($1::text::uuid, $2::text::uuid, $3)",
        &[
            &content_eid,
            &Uuid::now_v7().to_string(),
            &"custody-rotation path: DEK row intentionally retained (CASE-branch fixture)",
        ],
    )
    .await
    .unwrap();
    // Precondition: A's event_dek row is STILL live (the branch the DELETE path prevents).
    assert_eq!(
        custody_count(&a, &content_eid).await,
        (1, 1),
        "A's event_dek row is intentionally still live alongside the shred-log row"
    );

    // Wipe B and re-arm it as fully custody-capable, so the ONLY difference from phase 1 is
    // the shred-log row on A. If B now gains no custody, the serve CASE — not a deleted DEK
    // row, not B's own apply door (B has no shred-log row) — is what withheld the DEK.
    reset(&b).await;
    enroll_actors(&b, &kid_d, &kid_h).await;
    register_unwrap_key(&b, &sk_b).await;
    let p2 = full_pull_b();
    assert!(
        p2.status.success(),
        "phase-2 pull failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&p2.stdout),
        String::from_utf8_lossy(&p2.stderr)
    );
    assert_eq!(
        custody_count(&b, &content_eid).await,
        (0, 0),
        "the serve CASE excluded the DEK for a shred-logged event even though A's event_dek \
         row is still live — the wire-level shred guarantee does not depend on the DELETE"
    );
    assert_eq!(
        statement_count_for_med(&b, med).await,
        0,
        "no DEK across the wire ⇒ no clear shadow ⇒ no projection on B"
    );
}

/// Build a validly-signed but wrongly-SEALED `demographic.field.asserted` — the ADR-0052 §2
/// never-lawful shape (ONLY clinical.* is born-sealed; demographic bodies are plaintext by
/// necessity, their projections bind on NEW.body directly). Signed by A's device key so B's
/// apply door VERIFIES and admits it; its ciphertext body must NOT detonate B's
/// NEW.body-reading demographic projections. Returns the signed wire bytes.
fn sealed_demographic(sk: &SigningKey, kid: &str, patient: Uuid, wall: i64) -> Vec<u8> {
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "field": "dob",
        "value": "1980",
        "provenance": "document-verified",
    });
    let (container, _dek) =
        cairn_event::seal::seal_event_payload(&payload, "dob 1980", &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "node-a".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(cairn_event::seal::seal_stub_twin(
            "demographic.field.asserted",
        )),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Final-review availability-floor wedge (ADR-0052 §2, issue #10): a validly-signed
/// NON-clinical body carrying `payload.sealed=true` reaches B on the clinical wire. B's
/// apply door ADMITS it (set-union losslessness), but pre-fix a NEW.body-reading demographic
/// projection detonates on the ciphertext (NULL into a NOT NULL column) → a VERIFIABLE event
/// fails to apply → do_pull FREEZES the seq cursor → clinical sync WEDGES, and the legitimate
/// medication authored AFTER it never reaches B. The fix makes every non-clinical projection
/// seal-robust (returns NULL on a sealed row), so the sealed noise is admitted with no
/// projection and the watermark sails right past it.
///
/// Skips unless BOTH CAIRN_TEST_PG (A) and CAIRN_TEST_PG2 (B) are set.
#[tokio::test]
async fn sealed_non_clinical_pull_does_not_freeze_the_watermark() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let mut a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;

    let (sk_d, kid_d) = generate_key().unwrap();
    let (_sk_h, kid_h) = generate_key().unwrap();
    // Both nodes know the authors (the owner ceremony a real site performs): we want B to
    // APPLY the events, so the freeze under test is the SEAL-SCOPE wedge, not unknown-author.
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    register_unwrap_key(&b, &sk_b).await;

    let patient = Uuid::now_v7();

    // seq 1: a legit medication, born sealed + projected on A (with custody).
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
        None,
    )
    .await
    .unwrap();

    // seq 2: the wrongly-sealed demographic. INJECTED onto A with the projection triggers
    // SUPPRESSED (session_replication_role=replica) — the strict door would refuse it and
    // (pre-fix) the apply door would detonate on A too; suppressing the triggers on the
    // INJECTING node isolates the RED to B's PULL, modelling "A received this from a third
    // node". A then SERVES the verbatim signed bytes to B, whose apply door is under test.
    let sealed_demo = sealed_demographic(&sk_d, &kid_d, patient, WALL);
    a.batch_execute("SET session_replication_role = replica")
        .await
        .unwrap();
    let injected = a
        .query_one("SELECT apply_remote_event($1)::text", &[&sealed_demo])
        .await;
    a.batch_execute("SET session_replication_role = origin")
        .await
        .unwrap();
    injected.expect("A holds the sealed demographic (projection triggers suppressed on inject)");

    // seq 3: the SUBSEQUENT legitimate medication that must still reach B if sync is unwedged.
    assert_medication(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        &AssertMedicationInput {
            term: "ibuprofen",
            inn_code: None,
            formulation: Some("tablet"),
            dose_amount: Some("200"),
            dose_unit: Some("mg"),
            sig: Some("PRN"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    // Serve A, pull B once over real TCP.
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_SEALSCOPE,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_SEALSCOPE);
    let pull = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_SEALSCOPE,
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

    // The watermark sailed past the sealed noise: B holds the SUBSEQUENT ibuprofen (seq 3),
    // so the seq cursor was never frozen at the sealed demographic (seq 2). Pre-fix it froze
    // there and ibuprofen never arrived.
    let ibuprofen_on_b: i64 = b
        .query_one(
            "SELECT count(*) FROM medication_statement \
             WHERE patient_id = $1::text::uuid AND term = 'ibuprofen'",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        ibuprofen_on_b, 1,
        "the medication authored AFTER the sealed non-clinical event reached B — \
         the watermark never froze at the sealed event",
    );

    // And the sealed non-clinical body projected NOTHING on B (harmless ciphertext).
    let demo_on_b: i64 = b
        .query_one(
            "SELECT count(*) FROM patient_demographic WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        demo_on_b, 0,
        "the sealed non-clinical body projected nothing on B — admitted as ciphertext noise"
    );

    // Full convergence: identical event set + medication projections on both nodes.
    assert_eq!(
        snapshot(&a).await,
        snapshot(&b).await,
        "A and B converge — the sealed non-clinical event synced as lossless ciphertext noise"
    );
}

/// Build + LOCALLY AUTHOR (via `submit_event`, the STRICT door — already grade-gated by
/// ADR-0058 and unaffected by this fix) a `note.added` whose `t_effective` is ~4.5 years
/// after its own HLC wall, self-asserted grade. A legitimately HOLDS this event so it can
/// SERVE it to B; the door UNDER TEST below is B's REMOTE apply (db/020) when it pulls
/// this event over the wire. Returns the signed bytes + event_id.
async fn author_forward_dated_note(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
) -> (Vec<u8>, String) {
    let event_id = Uuid::now_v7().to_string();
    let body = EventBody {
        event_id: event_id.clone(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "node-a".into(),
        },
        t_effective: Some("2031-01-01T00:00:00Z".into()),
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "forward-dated probe"}),
        attachments: vec![],
        plaintext_twin: Some("Progress note: forward-dated probe".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, sk).unwrap().signed_bytes;
    c.execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect(
            "A holds the forward-dated event (grade-gated STRICT door already admits+flags it)",
        );
    (signed, event_id)
}

/// Issue #216 F1/F2 — the sync-wedge denial-of-service this task closes. Pre-fix,
/// db/020's LENIENT remote-apply door unconditionally RAISEd on ANY signed event whose
/// `t_effective` sits after its own HLC-wall ceiling, regardless of clock_grade. A single
/// such (still VALIDLY SIGNED) event served by a peer made B's apply of it fail, which
/// FREEZES `do_pull`'s seq cursor forever — one forward-dated event wedges ALL
/// subsequent clinical sync from that peer. The fix makes db/020 grade-gated (mirroring
/// db/005 and the pre-existing HLC-drift clamp a few lines below it in db/020): a
/// self-asserted/unknown clock has no standing to call a forward claim impossible, so it
/// is ADMITTED + FLAGGED, never rejected.
///
/// Mirrors `sealed_non_clinical_pull_does_not_freeze_the_watermark`'s two-node harness
/// exactly: seq 1 is a legit medication, seq 2 is the pathological event, seq 3 is a
/// SUBSEQUENT legit medication whose arrival on B is the proof the watermark never froze.
///
/// Skips unless BOTH CAIRN_TEST_PG (A) and CAIRN_TEST_PG2 (B) are set.
#[tokio::test]
async fn forward_dated_event_does_not_wedge_the_pull() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let mut a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;

    let (sk_d, kid_d) = generate_key().unwrap();
    let (_sk_h, kid_h) = generate_key().unwrap();
    // Both nodes know the authors (the owner ceremony a real site performs): we want B to
    // APPLY the events, so the freeze under test is the CEILING wedge, not unknown-author.
    enroll_actors(&a, &kid_d, &kid_h).await;
    enroll_actors(&b, &kid_d, &kid_h).await;

    let keydir = TempDir::new().unwrap();
    let (sk_b, _kid_b) = generate_key().unwrap();
    let key_a = write_key_file(keydir.path(), "node-a.key", &sk_d);
    let key_b = write_key_file(keydir.path(), "node-b.key", &sk_b);
    // The medication events are BORN-SEALED (ADR-0052): without B's own unwrap key
    // registered, its apply door admits them but withholds custody, so NEITHER
    // medication projects on B (a different failure than the one under test) —
    // exactly why the sealed test above registers it too.
    register_unwrap_key(&b, &sk_b).await;

    let patient = Uuid::now_v7();

    // seq 1: a legit medication, authored + projected on A.
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
        None,
    )
    .await
    .unwrap();

    // seq 2: the forward-dated event — self-asserted grade, t_effective ~4.5 years past its
    // own HLC wall. A legitimately holds it (admitted at the already grade-gated STRICT
    // door); B's REMOTE apply is the door under test when it pulls it over the wire.
    let (signed_forward, forward_event_id) =
        author_forward_dated_note(&a, &sk_d, &kid_d, patient, WALL).await;
    let forward_ca = event_address(&signed_forward);

    // seq 3: the SUBSEQUENT legit medication that must still reach B if sync is unwedged.
    assert_medication(
        &mut a,
        &sk_d,
        &kid_d,
        "node-a",
        patient,
        &AssertMedicationInput {
            term: "ibuprofen",
            inn_code: None,
            formulation: Some("tablet"),
            dose_amount: Some("200"),
            dose_unit: Some("mg"),
            sig: Some("PRN"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    // Serve A, pull B once over real TCP.
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args([
                "serve",
                "--conn",
                &base_a,
                "--listen",
                LISTEN_FORWARD,
                "--key",
                &key_a,
            ])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_FORWARD);
    let pull = Command::new(bin)
        .args([
            "pull",
            "--conn",
            &base_b,
            "--peer",
            LISTEN_FORWARD,
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

    // The watermark sailed past the forward-dated event: B holds the SUBSEQUENT
    // ibuprofen (seq 3), so the seq cursor was never frozen at seq 2. Pre-fix it froze
    // there and ibuprofen never arrived — the #216 F1/F2 DoS.
    let ibuprofen_on_b: i64 = b
        .query_one(
            "SELECT count(*) FROM medication_statement \
             WHERE patient_id = $1::text::uuid AND term = 'ibuprofen'",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        ibuprofen_on_b, 1,
        "the medication authored AFTER the forward-dated event reached B — \
         the watermark never froze (issue #216 F1/F2)",
    );

    // The forward-dated event itself APPLIED on B (admitted, never silently dropped)...
    let forward_on_b: i64 = b
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&forward_event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        forward_on_b, 1,
        "the forward-dated event itself applied on B"
    );

    // ...and was recorded as exactly one advisory ceiling flag on B, proving the LENIENT
    // remote door (not just the strict local one) ran the grade-gated classify.
    let flags_on_b: i64 = b
        .query_one(
            "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1",
            &[&forward_ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags_on_b, 1,
        "B's remote apply door recorded the advisory ceiling flag, never rejected"
    );

    // Full convergence: identical event set + medication projections on both nodes.
    assert_eq!(
        snapshot(&a).await,
        snapshot(&b).await,
        "A and B converge — the forward-dated event synced and was admitted identically on both"
    );
}
