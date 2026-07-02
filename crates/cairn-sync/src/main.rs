//! Cairn walking skeleton — the thin sync daemon (Spike 0001 §3, §5).
//!
//! Set-union ship/apply over a tiny framed protocol (run over WireGuard; NoTls is
//! deliberate — the link is the transport). Two planes, exactly as the spec
//! separates them:
//!
//!   * **clinical plane** (`serve` events / `pull`): eager, small, high priority —
//!     ships signed event bytes (plus any attestation token that vouches them); the
//!     receiver applies through the in-DB door `apply_remote_event` (db/020), which
//!     verifies in-DB and inserts idempotently (set-union, Bet A1) — the daemon
//!     itself runs no checks and no raw DML (issue #91 / ADR-0021).
//!   * **byte tier** (`serve` blob slices / `blobd`): lazy, windowed, resumable,
//!     preemptible, separately budgeted — must never starve the clinical plane (Bet A4).
//!
//! This daemon carries NO merge logic (ADR-0001/§9.4): convergence is set-union +
//! the in-DB projection trigger. It only ships bytes, verifies, and applies.

use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_event::{blob_address, materialise_generic_twin, resolve_twin, sign, sign_attestation, verify_self_described, AttestationBody, EventBody, Hlc, SigningKey, CTX_EVENT};
use serde::{Deserialize, Serialize};

const SCHEMA: [(&str, &str); 8] = [
    ("001_envelope", include_str!("../../../db/001_envelope.sql")),
    ("002_projection", include_str!("../../../db/002_projection.sql")),
    ("003_blobs", include_str!("../../../db/003_blobs.sql")),
    ("004_actors", include_str!("../../../db/004_actors.sql")),
    ("005_submit", include_str!("../../../db/005_submit.sql")),
    ("006_recall", include_str!("../../../db/006_recall.sql")),
    // The clinical-plane sync apply door (issue #91): replicated events enter
    // event_log only through the in-DB floor, never a daemon-side raw INSERT.
    ("020_apply_remote_event", include_str!("../../../db/020_apply_remote_event.sql")),
    // Durable quarantine for unverifiable pulled events (issue #108): a skipped
    // event leaves a durable, re-processable trace, never just a stderr line.
    ("021_sync_quarantine", include_str!("../../../db/021_sync_quarantine.sql")),
];

const SLICE_BYTES: usize = 256 * 1024; // window/slice granularity (tuned; amortizes bao tree overhead)

type R<T> = Result<T, Box<dyn Error>>;

// ---------------------------------------------------------------------------
// Wire protocol — one JSON request, one JSON response, per connection.
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize)]
#[serde(tag = "op")]
enum Request {
    /// Clinical plane: every event at or after this HLC watermark.
    EventsAfter { wall: i64, counter: i32 },
    /// Byte tier: a BLAKE3 verified-streaming slice of a blob.
    BlobSlice {
        addr_hex: String,
        offset: u64,
        len: u64,
    },
}

#[derive(Serialize, Deserialize)]
struct EventsResponse {
    /// Verbatim signed_bytes, hex-encoded (skeleton simplification; the real
    /// tier ships raw). The receiver reconstructs everything from these bytes.
    events: Vec<String>,
    /// Per-event attestation token (hex), PARALLEL to `events` (issue #91). A
    /// suppressing event (or asserted responsibility) is admitted at the in-DB
    /// apply door only against its human attestation token, so the token must
    /// travel with the event or a legitimately-attested suppress could never
    /// replicate. Additive field (serde default): an older peer's response
    /// decodes with empty arrays, which simply means "no attestation shipped" —
    /// its suppressing events are then refused fail-closed at the door.
    #[serde(default)]
    attestations: Vec<Option<String>>,
    /// Per-event attester public key (hex), parallel to `attestations`.
    #[serde(default)]
    attester_keys: Vec<Option<String>>,
    /// The ADR-0040 signing context this server's events are minted under
    /// (issue #108). Lets the puller tell deterministic wire-format skew ("your
    /// events are signed for a context I don't speak") from tampering BEFORE
    /// burning a whole batch on per-event verify failures. Additive (serde
    /// default): a response from a peer predating this field decodes as None —
    /// "undeclared" — and the puller falls back to the all-unverifiable
    /// heuristic for the mixed-version diagnosis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signing_context: Option<String>,
}

/// Byte-tier slice response — a **binary** frame, deliberately NOT JSON. The blob
/// tier is throughput-bound on the WAN, so it ships the bao slice as raw bytes
/// rather than hex (hex doubled every transferred byte, halving measured throughput
/// and skewing the §8.2 numbers). Layout: `[found:u8][total_len:u64 BE][slice…]`.
/// The clinical plane stays JSON — it is small and latency-bound, not throughput-bound.
fn encode_blob_slice(found: bool, total_len: u64, slice: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + slice.len());
    out.push(found as u8);
    out.extend_from_slice(&total_len.to_be_bytes());
    out.extend_from_slice(slice);
    out
}

/// Decode a [`encode_blob_slice`] frame into `(found, total_len, slice_bytes)`.
/// A frame shorter than the 9-byte header is malformed and decodes as not-found.
fn decode_blob_slice(raw: &[u8]) -> (bool, u64, &[u8]) {
    if raw.len() < 9 {
        return (false, 0, &[]);
    }
    let found = raw[0] != 0;
    let total_len = u64::from_be_bytes(raw[1..9].try_into().unwrap());
    (found, total_len, &raw[9..])
}

fn write_frame(s: &mut impl Write, b: &[u8]) -> io::Result<()> {
    s.write_all(&(b.len() as u32).to_be_bytes())?;
    s.write_all(b)?;
    s.flush()
}

fn read_frame(s: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    let n = u32::from_be_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    s.read_exact(&mut buf)?;
    Ok(buf)
}

fn try_request(peer: &str, req: &Request) -> R<Vec<u8>> {
    // Bounded connect so a dead link fails fast instead of hanging for minutes.
    let addr = peer
        .to_socket_addrs()?
        .next()
        .ok_or("could not resolve peer address")?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(30)))?;
    write_frame(&mut stream, &serde_json::to_vec(req)?)?;
    Ok(read_frame(&mut stream)?)
}

/// Retry with exponential backoff. A Starlink link drops constantly; a transient
/// failure must not fail the whole pull/fetch — it retries, and only a sustained
/// outage surfaces as an error (which the `run` loop logs as a partition).
fn request(peer: &str, req: &Request) -> R<Vec<u8>> {
    let mut delay = Duration::from_millis(250);
    let mut last: Option<Box<dyn Error>> = None;
    for attempt in 0..4 {
        match try_request(peer, req) {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = Some(e);
                if attempt < 3 {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
            }
        }
    }
    Err(last.unwrap())
}

// ---------------------------------------------------------------------------
// Key handling (skeleton: a per-node key file; the registry is ADR-0011).
// ---------------------------------------------------------------------------
fn load_or_create_key(path: &str) -> R<(SigningKey, String)> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let seed: [u8; 32] = hex::decode(text.trim())?
            .try_into()
            .map_err(|_| "key file is not a 32-byte hex seed")?;
        let sk = SigningKey::from_bytes(&seed);
        let kid = hex::encode(sk.verifying_key().to_bytes());
        return Ok((sk, kid));
    }
    let (sk, kid) = cairn_event::generate_key()?;
    std::fs::write(path, hex::encode(sk.to_bytes()))?;
    // Restrict the private-key file to the owner (0600). std::fs::write creates it 0644 by
    // default, leaving the signing seed world-readable on a shared machine (review finding
    // L12). Set the mode AFTER writing so the bytes are never briefly world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    eprintln!("generated new signing key at {path} (kid {})", &kid[..16]);
    Ok((sk, kid))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// True iff an asserted t_effective string carries an explicit UTC offset — the
/// issue #91/H4 wire pin, checked at AUTHORING time so this node never signs a
/// timestamp every peer's apply door would refuse. (An offset-less timestamp names a
/// different instant on differently-configured nodes.) The strict format validation
/// lives in-DB (db/001 `cairn_t_effective`); this is only the author-side conformance
/// check for the `--effective` CLI flag: after the 10-char date + separator, the
/// string must end with 'Z'/'z' or a ±HH / ±HHMM / ±HH:MM offset.
fn t_effective_has_explicit_offset(t: &str) -> bool {
    if t.ends_with('Z') || t.ends_with('z') {
        return true;
    }
    // Search for the offset sign only AFTER the date part (index 11 on): the date's
    // own '-' separators must not read as an offset.
    let Some(time) = t.get(11..) else { return false };
    match time.rfind(['+', '-']) {
        Some(p) => {
            let off = &time[p + 1..];
            matches!(off.len(), 2 | 4 | 5) && off.chars().all(|c| c.is_ascii_digit() || c == ':')
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Apply: hand a replicated event (and any attestation that travelled with it)
// to the in-DB apply door. Shared by `pull`. Since issue #91 the daemon runs
// ZERO checks and ZERO raw DML here: apply_remote_event (db/020) verifies the
// signature in-DB (pgrx), resolves the signer against the actor registry,
// classifies fail-closed, runs the attestation/twin/t_effective floors, guards
// against event_id substitution, learns attachment references, and merges the
// HLC forward — the same floor local authors face at submit_event (ADR-0021:
// the enforcement floor sits BELOW the inter-node path). Returns Ok(true) iff
// the event was NEW to this node (set-union accounting for the pull metrics).
// ---------------------------------------------------------------------------
fn apply_signed(
    client: &mut postgres::Client,
    signed_bytes: &[u8],
    attestation: Option<&[u8]>,
    attester_key: Option<&[u8]>,
) -> R<bool> {
    // Newness probe for the metrics only: the door itself is idempotent (a re-apply
    // of identical bytes is a silent set-union no-op), so "did we already hold these
    // bytes" is read before knocking. Never a gate — the door decides admission.
    let content_address = cairn_event::event_address(signed_bytes);
    let existed: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM event_log WHERE content_address = $1)",
            &[&content_address],
        )?
        .get(0);
    client
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[
                &signed_bytes.to_vec(),
                &attestation.map(|a| a.to_vec()),
                &attester_key.map(|k| k.to_vec()),
            ],
        )
        // Surface the door's legible RAISE text: postgres::Error's Display is just
        // "db error", which would strip the reason from the freeze/skip log lines.
        .map_err(|e| -> Box<dyn Error> {
            match e.as_db_error() {
                Some(db) => db.message().to_string().into(),
                None => e.into(),
            }
        })?;
    Ok(!existed)
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// Sign an EventBody supplied as JSON on stdin and emit hex COSE_Sign1 on stdout.
/// Lets a non-Rust client (the Python agent stand-in) drive the write contract
/// while Rust owns the canonical encoding + signature (one signer implementation).
fn cmd_sign_stdin(key_path: &str) -> R<()> {
    let (sk, _kid) = load_or_create_key(key_path)?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let body: EventBody = serde_json::from_str(&input)?;
    // Sign exactly what we were given — including a body.signer_key_id that may NOT
    // match this key. That is deliberate: the helper is a dumb signer so the hostile
    // C5.6 (impersonation) case can produce a mismatched event; the in-DB binding
    // gate (verify_self_described) is the floor that rejects it.
    let signed = sign(&body, &sk)?;
    println!("{}", hex::encode(&signed.signed_bytes));
    Ok(())
}

/// Build a hex COSE_Sign1 attestation token from a JSON `AttestationBody` string,
/// signed by `sk`. Pure (no I/O) so it is unit-testable; `cmd_attest_stdin` wraps it
/// with key-load + stdin-read + stdout-print. Mirrors the sign-stdin split so Rust
/// owns the one canonical attestation encoding (no second crypto impl in Python).
fn attestation_token_hex(input: &str, sk: &SigningKey) -> R<String> {
    let body: AttestationBody = serde_json::from_str(input)?;
    let content_address = hex::decode(&body.content_address_hex)?;
    let token = sign_attestation(&content_address, &body.attester_key_id, &body.role, sk)?;
    Ok(hex::encode(&token))
}

/// Sign an `AttestationBody` supplied as JSON on stdin and emit a hex COSE_Sign1
/// attestation token on stdout. Like `sign-stdin`, this is a DUMB signer: it attests
/// whatever `content_address_hex` it is handed, including one bound to no real event.
/// That is deliberate — it is how the wrong-address adversarial test is constructed —
/// and the in-DB floor (`cairn_attestation_ok`) is what rejects a mis-bound token,
/// never this CLI. Do NOT "harden" it to validate the address: that would break the
/// adversarial tests and move a floor check out of the database (ADR-0021/0030).
fn cmd_attest_stdin(key_path: &str) -> R<()> {
    let (sk, _kid) = load_or_create_key(key_path)?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let token_hex = attestation_token_hex(&input, &sk)?;
    println!("{token_hex}");
    Ok(())
}

/// Print the hex Ed25519 public key (the kid) for `key_path`, creating the key if
/// it does not yet exist. Lets a non-Rust client set body.signer_key_id correctly
/// (it must match the signing key — see the binding gate in verify_self_described).
fn cmd_key_id(key_path: &str) -> R<()> {
    let (_sk, kid) = load_or_create_key(key_path)?;
    println!("{kid}");
    Ok(())
}

fn cmd_init(conn: &str) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    // 004/005 call cairn_pgx functions; the extension must exist first.
    client.batch_execute("CREATE EXTENSION IF NOT EXISTS cairn_pgx;")?;
    for (name, sql) in SCHEMA {
        client.batch_execute(sql)?;
        eprintln!("applied {name}");
    }
    Ok(())
}

/// Enroll a signing key as an actor in the LOCAL registry (an owner-privileged
/// ceremony, ADR-0011 — deliberately NOT part of `init` or `pull`). The in-DB apply
/// door (db/020) refuses events whose signer is not enrolled here, and the actor
/// registry does not replicate yet, so an operator enrolls each authoring key on
/// every node that will apply its events (the harness does this for the skeleton).
fn cmd_enroll(conn: &str, key_path: &str, kind: &str) -> R<()> {
    let (_sk, kid) = load_or_create_key(key_path)?;
    // Minimal pinned-determinant set for a node/device key: the key itself. A real
    // agent enrollment pins model/version/skill-epoch (ADR-0029); that ceremony
    // lives with the agent deployment, not this CLI.
    let pinned = serde_json::json!({ "kind": kind, "signing_key": kid }).to_string();
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "SELECT enroll_actor($1, $2::text::jsonb, $3)",
        &[&kind, &pinned, &kid],
    )?;
    println!("enrolled {kind} actor {kid}");
    Ok(())
}

/// Sign and append one local clinical event, advancing this node's HLC under a
/// row lock (the t_recorded ceiling). Returns the clinical-plane byte size of the
/// signed event. Shared by `write` and the `gen` load generator.
#[allow(clippy::too_many_arguments)]
fn emit_event(
    client: &mut postgres::Client,
    node: &str,
    sk: &SigningKey,
    kid: &str,
    event_type: &str,
    patient_id: &str,
    schema_version: &str,
    payload: serde_json::Value,
    t_effective: Option<String>,
) -> R<EventBody> {
    let mut tx = client.transaction()?;
    let row = tx.query_one(
        "SELECT hlc_wall, hlc_counter FROM hlc_state WHERE id FOR UPDATE",
        &[],
    )?;
    let prev_wall: i64 = row.get(0);
    let prev_counter: i32 = row.get(1);
    let phys = now_ms();
    let (wall, counter) = if phys > prev_wall {
        (phys, 0)
    } else {
        (prev_wall, prev_counter + 1)
    };
    tx.execute(
        "UPDATE hlc_state SET hlc_wall=$1, hlc_counter=$2 WHERE id",
        &[&wall, &counter],
    )?;

    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: patient_id.to_string(),
        event_type: event_type.to_string(),
        schema_version: schema_version.to_string(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: node.to_string(),
        },
        t_effective,
        signer_key_id: kid.to_string(),
        contributors: serde_json::json!([{ "role": "author", "kind": "human", "node": node }]),
        payload,
        attachments: vec![],
        plaintext_twin: None,
    };

    // ADR-0039: globalise the authored twin — materialise it into the body BEFORE signing, so
    // this node emits a conformant author-faithful twin rather than relying on receivers to derive.
    let body = materialise_generic_twin(body);
    let signed = sign(&body, sk)?;
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let twin = resolve_twin(&body);

    tx.execute(
        "INSERT INTO event_log
           (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
            node_origin, t_effective, signed_bytes, content_address, body, contributors,
            signer_key_id, plaintext_twin, attachments)
         VALUES ($1::text::uuid,$2::text::uuid,$3,$4,$5,$6,$7,$8::text::timestamptz,$9,$10,
                 $11::text::jsonb,$12::text::jsonb,$13,$14,'[]'::jsonb)",
        &[
            &body.event_id,
            &body.patient_id,
            &body.event_type,
            &body.schema_version,
            &body.hlc.wall,
            &body.hlc.counter,
            &body.hlc.node_origin,
            &body.t_effective,
            &signed.signed_bytes,
            &signed.content_address,
            &body_json,
            &contributors_json,
            &body.signer_key_id,
            &twin,
        ],
    )?;
    tx.commit()?;
    Ok(body)
}

#[allow(clippy::too_many_arguments)]
fn cmd_write(
    conn: &str,
    node: &str,
    key_path: &str,
    event_type: &str,
    patient: &str,
    schema_version: &str,
    json_body: &str,
    t_effective: Option<String>,
) -> R<()> {
    // Author-side wire conformance (issue #91/H4): refuse to SIGN an offset-less
    // t_effective — once signed it is immutable, and every conformant apply door
    // would refuse it, wedging this event out of the fleet forever.
    if let Some(eff) = &t_effective {
        if !t_effective_has_explicit_offset(eff) {
            return Err(format!(
                "--effective '{eff}' must carry an explicit UTC offset \
                 (e.g. 2026-06-20T10:00:00+02:00 or ...T08:00:00Z): an offset-less \
                 timestamp names a different instant on different nodes"
            )
            .into());
        }
    }
    let (sk, kid) = load_or_create_key(key_path)?;
    let payload: serde_json::Value = serde_json::from_str(json_body)?;
    let patient_id = if patient == "new" {
        uuid::Uuid::now_v7().to_string()
    } else {
        patient.to_string()
    };
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let body = emit_event(
        &mut client,
        node,
        &sk,
        &kid,
        event_type,
        &patient_id,
        schema_version,
        payload,
        t_effective,
    )?;
    println!("wrote {} {} for patient {}", event_type, body.event_id, patient_id);
    Ok(())
}

/// Load generator: create `patients` new patients, then append `count` notes
/// spread across them at an optional target `rate` (events/sec). Emits one JSON
/// metrics line so the harness can record throughput.
fn cmd_gen(
    conn: &str,
    node: &str,
    key_path: &str,
    patients: usize,
    count: usize,
    rate: f64,
) -> R<()> {
    let (sk, kid) = load_or_create_key(key_path)?;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;

    let mut pids = Vec::new();
    for i in 0..patients.max(1) {
        let pid = uuid::Uuid::now_v7().to_string();
        emit_event(
            &mut client,
            node,
            &sk,
            &kid,
            "patient.created",
            &pid,
            "patient/1",
            serde_json::json!({"name": format!("Patient {i:04}"), "dob": "1980-01-01", "sex": "U"}),
            None,
        )?;
        pids.push(pid);
    }

    let interval = if rate > 0.0 {
        Some(Duration::from_secs_f64(1.0 / rate))
    } else {
        None
    };
    let start = Instant::now();
    for n in 0..count {
        let pid = &pids[n % pids.len()];
        emit_event(
            &mut client,
            node,
            &sk,
            &kid,
            "note.added",
            pid,
            "note/1",
            serde_json::json!({"text": format!("note {n} from {node}")}),
            None,
        )?;
        if let Some(iv) = interval {
            std::thread::sleep(iv);
        }
    }
    let secs = start.elapsed().as_secs_f64().max(1e-9);
    println!(
        "{}",
        serde_json::json!({
            "op": "gen", "node": node, "patients": patients, "notes": count,
            "elapsed_ms": (secs * 1000.0) as i64,
            "events_per_sec": (count as f64 / secs)
        })
    );
    Ok(())
}

/// Emit a convergence/honest-state fingerprint (A1, A3, A6) as JSON. Two nodes
/// have converged iff their `event_hash` and `projection_hash` match.
fn do_fingerprint(client: &mut postgres::Client) -> R<serde_json::Value> {
    let events: i64 = client
        .query_one("SELECT count(*) FROM event_log", &[])?
        .get(0);
    let event_hash: Option<String> = client
        .query_one(
            "SELECT md5(string_agg(encode(content_address,'hex'), ','
                 ORDER BY hlc_wall, hlc_counter, node_origin)) FROM event_log",
            &[],
        )?
        .get(0);
    let projection_hash: Option<String> = client
        .query_one(
            "SELECT md5(string_agg(
                 patient_id::text || coalesce(name,'') || coalesce(dob,'') ||
                 coalesce(sex,'') || note_count::text, ',' ORDER BY patient_id::text))
             FROM patient_chart",
            &[],
        )?
        .get(0);
    let hlc = client.query_one("SELECT hlc_wall, hlc_counter FROM hlc_state", &[])?;
    let (hlc_wall, hlc_counter): (i64, i32) = (hlc.get(0), hlc.get(1));
    let max_event_hlc: i64 = client
        .query_one("SELECT coalesce(max(hlc_wall),0) FROM event_log", &[])?
        .get(0);
    let max_skew_ms: i64 = client
        .query_one(
            "SELECT coalesce(max(abs(hlc_wall - (extract(epoch FROM recorded_at)*1000)::bigint)),0)
             FROM event_log",
            &[],
        )?
        .get(0);
    let blobs = client.query_one(
        "SELECT count(*) FILTER (WHERE present), count(*) FILTER (WHERE NOT present) FROM blob_store",
        &[],
    )?;
    let (blobs_present, blobs_referenced_only): (i64, i64) = (blobs.get(0), blobs.get(1));

    Ok(serde_json::json!({
        "events": events,
        "event_hash": event_hash,
        "projection_hash": projection_hash,
        "hlc_wall": hlc_wall,
        "hlc_counter": hlc_counter,
        // A3: the local clock must have merged forward past every applied event.
        "hlc_merged_past_max_event": hlc_wall >= max_event_hlc,
        // Max gap between an event's asserted HLC and this node's local recording
        // time — propagation/partition lag plus any true clock skew. Reported and
        // flagged, never auto-resolved (§3.6); the structural invariant is the
        // merge above, not a bound on this gap.
        "max_hlc_record_gap_ms": max_skew_ms,
        // A6: references whose bytes have not (yet) been retrieved.
        "blobs_present": blobs_present,
        "blobs_referenced_only": blobs_referenced_only
    }))
}

fn cmd_fingerprint(conn: &str) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    println!("{}", do_fingerprint(&mut client)?);
    Ok(())
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

/// Bet B (B1) — time `count` projection-maintained single-op writes at the current
/// log size. Each `emit_event` is one transaction whose `AFTER INSERT` trigger folds
/// the event into `patient_chart`, so this measures the exact maintenance path
/// ADR-0001 bets stays cheap. The harness samples at growing log sizes to check the
/// cost does not grow with the log.
fn cmd_bench_insert(conn: &str, node: &str, key_path: &str, count: usize) -> R<()> {
    let (sk, kid) = load_or_create_key(key_path)?;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let log_size: i64 = client
        .query_one("SELECT count(*) FROM event_log", &[])?
        .get(0);
    let pid = uuid::Uuid::now_v7().to_string();
    emit_event(&mut client, node, &sk, &kid, "patient.created", &pid, "patient/1",
        serde_json::json!({"name":"Bench Patient","dob":"1980-01-01","sex":"U"}), None)?;

    let mut lat = Vec::with_capacity(count);
    for n in 0..count {
        let t = Instant::now();
        emit_event(&mut client, node, &sk, &kid, "note.added", &pid, "note/1",
            serde_json::json!({"text": format!("b1 maintenance sample {n}")}), None)?;
        lat.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "{}",
        serde_json::json!({
            "op": "bench_insert", "log_size": log_size, "count": count,
            "p50_ms": pct(&lat, 0.50), "p95_ms": pct(&lat, 0.95), "max_ms": pct(&lat, 1.0)
        })
    );
    Ok(())
}

/// Bet B (B2) — time a full chart read: demographics from the `patient_chart`
/// projection plus the patient's note timeline rendered from the plaintext legibility
/// twins (the version-independent §3.13 substrate). The paper-parity floor: this must
/// beat "grab the paper chart."
fn cmd_chart(conn: &str, patient: &str) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let t = Instant::now();
    let demo = client.query_opt(
        "SELECT name, dob, sex, note_count FROM patient_chart WHERE patient_id=$1::text::uuid",
        &[&patient],
    )?;
    let notes = client.query(
        "SELECT plaintext_twin FROM event_log
         WHERE patient_id=$1::text::uuid AND event_type='note.added'
         ORDER BY hlc_wall, hlc_counter, node_origin",
        &[&patient],
    )?;
    // Touch the rendered text so the assembly is real work, not a lazy cursor.
    let chars: usize = notes.iter().map(|r| r.get::<_, String>(0).len()).sum();
    let elapsed_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "{}",
        serde_json::json!({
            "op": "chart", "patient": patient, "found": demo.is_some(),
            "notes": notes.len(), "rendered_chars": chars, "elapsed_ms": elapsed_ms
        })
    );
    Ok(())
}

/// Bet B (B3/B4) — pure-CPU crypto microbenchmarks (no DB). B4: Ed25519 sign/verify
/// throughput and SHA-256-vs-BLAKE3 hashing throughput (the ARM number that could
/// revisit ADR-0015's provisional blob digest). B3: DEK-wrap and body-seal throughput
/// — the keystore cost of crypto-shredding ([ADR-0005](../spec/decisions/0005...)),
/// from which the harness extrapolates per-event vs per-episode key granularity.
fn cmd_bench(hash_mb: usize, sig_iters: u32, dek_iters: u32) -> R<()> {
    use chacha20poly1305::aead::{Aead, KeyInit};
    use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

    let (sign_per_s, verify_per_s) = cairn_event::bench_sign_verify(sig_iters);
    let (sha_mbps, blake_mbps) = cairn_event::bench_hash_mbps(hash_mb);

    // B3: a KEK wraps a fresh per-body DEK; the DEK seals the body. Crypto-shred =
    // destroy the DEK, so opening a sealed episode is one unwrap per DEK — hence the
    // per-event vs per-episode granularity question this cost feeds.
    //
    // BENCHMARK ONLY: the fixed all-zero nonce reused across every encrypt below is a
    // throughput microbench, not a keystore. NEVER copy this into real DEK-wrap /
    // body-seal code — nonce reuse under XChaCha20Poly1305 (same key + same nonce)
    // is catastrophic for confidentiality. Real sealing draws a fresh random nonce
    // per encryption.
    let kek = XChaCha20Poly1305::new(Key::from_slice(&[9u8; 32]));
    let nonce = XNonce::from_slice(&[0u8; 24]);
    let dek = [3u8; 32];
    let t = Instant::now();
    for _ in 0..dek_iters {
        std::hint::black_box(kek.encrypt(nonce, dek.as_ref()).unwrap());
    }
    let dek_wrap_per_s = dek_iters as f64 / t.elapsed().as_secs_f64();

    let body = vec![0x7Eu8; 1024]; // a representative ~1 KiB clinical body
    let body_kek = XChaCha20Poly1305::new(Key::from_slice(&dek));
    let t = Instant::now();
    for _ in 0..dek_iters {
        std::hint::black_box(body_kek.encrypt(nonce, body.as_ref()).unwrap());
    }
    let body_seal_mbps = (dek_iters as f64 * body.len() as f64 / (1 << 20) as f64)
        / t.elapsed().as_secs_f64();

    println!(
        "{}",
        serde_json::json!({
            "op": "bench",
            // B4
            "ed25519_sign_per_s": sign_per_s,
            "ed25519_verify_per_s": verify_per_s,
            "sha256_mbps": sha_mbps,
            "blake3_mbps": blake_mbps,
            "blake3_faster_than_sha256": blake_mbps >= sha_mbps,
            // B3
            "dek_wrap_per_s": dek_wrap_per_s,
            "body_seal_mbps": body_seal_mbps
        })
    );
    Ok(())
}

fn cmd_put_blob(conn: &str, file: &str, media: &str) -> R<()> {
    let bytes = std::fs::read(file)?;
    let addr = blob_address(&bytes);
    let outboard = cairn_event::blob_outboard(&bytes);
    let len = bytes.len() as i64;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1,$2,$3,$4,$5,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, outboard=EXCLUDED.outboard, present=TRUE,
                byte_len=EXCLUDED.byte_len, fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &bytes, &outboard],
    )?;
    println!("stored blob {} ({} bytes, {})", hex::encode(&addr), len, media);
    Ok(())
}

/// Mint a large local blob (random-ish bytes) and store it present, so a real
/// multi-MB windowed fetch can be driven on the link without shipping a file. The
/// bytes come from a tiny xorshift PRNG (content just needs to be addressable and
/// distinct, not cryptographically random).
fn cmd_gen_blob(conn: &str, size_mb: usize, media: &str) -> R<()> {
    let n = size_mb.max(1) * 1024 * 1024;
    let mut buf = vec![0u8; n];
    let mut x = (now_ms() as u64) | 1;
    for b in buf.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
    let addr = blob_address(&buf);
    let outboard = cairn_event::blob_outboard(&buf);
    let len = buf.len() as i64;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1,$2,$3,$4,$5,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, outboard=EXCLUDED.outboard, present=TRUE,
                byte_len=EXCLUDED.byte_len, fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &buf, &outboard],
    )?;
    println!(
        "{}",
        serde_json::json!({"op":"gen_blob","addr": hex::encode(&addr),"bytes": len,"media": media})
    );
    Ok(())
}

/// Persist an unverifiable pulled event into `sync_quarantine` (db/021, issue
/// #108): verbatim bytes + travelling attestation + the legible verify-failure
/// reason. A re-offer of the same bytes dedupes onto its existing row (bumping
/// `last_seen`/`seen_count`) — repeated cycles against a broken peer must not
/// grow the table. This durable trace is what LICENSES the pull loop to advance
/// the watermark past the event; if this INSERT fails the caller must freeze
/// the watermark instead, exactly as for a valid-but-unapplied event.
fn quarantine_event(
    client: &mut postgres::Client,
    peer_name: &str,
    signed_bytes: &[u8],
    attestation: Option<&[u8]>,
    attester_key: Option<&[u8]>,
    reason: &str,
) -> R<()> {
    client.execute(
        "INSERT INTO sync_quarantine
             (content_digest, signed_bytes, attestation, attester_key, peer, reason)
         VALUES ($1,$2,$3,$4,$5,$6)
         ON CONFLICT (content_digest) DO UPDATE
            SET last_seen  = clock_timestamp(),
                seen_count = sync_quarantine.seen_count + 1,
                reason     = EXCLUDED.reason,
                peer       = EXCLUDED.peer",
        &[
            &cairn_event::event_address(signed_bytes),
            &signed_bytes.to_vec(),
            &attestation.map(|a| a.to_vec()),
            &attester_key.map(|k| k.to_vec()),
            &peer_name,
            &reason,
        ],
    )?;
    Ok(())
}

fn do_pull(client: &mut postgres::Client, peer: &str, peer_name: &str) -> R<serde_json::Value> {
    client.execute(
        "INSERT INTO sync_state (peer) VALUES ($1) ON CONFLICT (peer) DO NOTHING",
        &[&peer_name],
    )?;
    let wm = client.query_one(
        "SELECT hlc_wall, hlc_counter FROM sync_state WHERE peer=$1",
        &[&peer_name],
    )?;
    let (wall, counter): (i64, i32) = (wm.get(0), wm.get(1));

    let started = Instant::now();
    let raw = request(peer, &Request::EventsAfter { wall, counter })?;
    let wire_bytes = raw.len();
    let resp: EventsResponse = serde_json::from_slice(&raw)?;

    // Deterministic wire-format skew check (issue #108): a peer that DECLARES a
    // signing context we don't speak would fail verification for every event it
    // ships — refuse the batch up front with an error naming both contexts,
    // rather than burning per-event failures whose generic "unverifiable" reason
    // misdirects the operator toward tampering. Nothing is quarantined and the
    // watermark is untouched: the peer still holds the events, and they apply
    // normally once the skew (one side needs upgrading) is fixed. A peer that
    // declares NOTHING is an older build — per-event verification decides, and
    // the all-unverifiable diagnosis below catches the pure-legacy case.
    if let Some(peer_ctx) = &resp.signing_context {
        if peer_ctx != CTX_EVENT.as_str() {
            return Err(format!(
                "pull {peer_name}: peer declares signing context '{peer_ctx}' but this \
                 node expects '{}' — wire-format skew, not tampering; upgrade the older \
                 side. Batch refused, watermark untouched.",
                CTX_EVENT.as_str()
            )
            .into());
        }
    }

    // Watermark discipline (review fix A1 + issue #108). The OLD loop advanced the
    // watermark to the max HLC of the *successfully-applied* events and counted every
    // failure as a "verify failure", so a transient DB error (or a deterministic insert
    // failure) on a VALIDLY SIGNED event silently dropped it: the watermark moved past it
    // and the server never offered it again on this link — a permanent, silent set-union
    // violation with only a counter as evidence. Fix: advance the watermark ONLY to the
    // contiguous successfully-applied prefix. The server ships events HLC-ascending, so we
    // FREEZE the watermark at the first still-verifiable event we fail to apply, and never
    // advance past it (it is re-fetched and retried next cycle). An UNVERIFIABLE event is
    // illegitimate (never validly part of the set), so it must not wedge the link — but
    // "skip" is only admissible once the event is QUARANTINED DURABLY (db/021): bytes +
    // reason survive for inspection and requeue, so the skip is recorded, never silent.
    // If even the quarantine INSERT fails, the watermark freezes exactly as for a valid
    // event — delayed, never lost.
    let (mut applied, mut skipped_unverifiable, mut event_bytes) = (0usize, 0usize, 0usize);
    let (mut max_w, mut max_c) = (wall, counter);
    let mut frozen = false;
    for (i, hexed) in resp.events.iter().enumerate() {
        let signed_bytes = hex::decode(hexed)?;
        event_bytes += signed_bytes.len(); // A5: real clinical-plane payload (the COSE blob)
        // The attestation arrays are PARALLEL to events; an older peer (or an
        // un-attested event) yields None, and the in-DB door decides what that means.
        let att = match resp.attestations.get(i).and_then(|o| o.as_deref()) {
            Some(h) => Some(hex::decode(h)?),
            None => None,
        };
        let akey = match resp.attester_keys.get(i).and_then(|o| o.as_deref()) {
            Some(h) => Some(hex::decode(h)?),
            None => None,
        };
        match apply_signed(client, &signed_bytes, att.as_deref(), akey.as_deref()) {
            Ok(new) => {
                if new {
                    applied += 1;
                }
                // Only advance the watermark while the applied prefix is unbroken.
                if !frozen {
                    if let Ok(b) = verify_self_described(&signed_bytes) {
                        if (b.hlc.wall, b.hlc.counter) > (max_w, max_c) {
                            max_w = b.hlc.wall;
                            max_c = b.hlc.counter;
                        }
                    }
                }
            }
            Err(e) => {
                // Classify the failure. A still-verifiable event that failed to APPLY
                // (transient DB error, deterministic insert failure, or a refused
                // substitution) must NOT be lost — freeze the watermark so it is retried
                // and never skipped. An unverifiable event is illegitimate — quarantine
                // it durably, and only a successful quarantine permits skipping on.
                match verify_self_described(&signed_bytes) {
                    Ok(_) => {
                        frozen = true;
                        eprintln!(
                            "pull {peer_name}: HALTING watermark at {max_w}/{max_c} — a valid \
                             event failed to apply and must not be skipped: {e}"
                        );
                    }
                    Err(verr) => match quarantine_event(
                        client,
                        peer_name,
                        &signed_bytes,
                        att.as_deref(),
                        akey.as_deref(),
                        &verr.to_string(),
                    ) {
                        Ok(()) => {
                            skipped_unverifiable += 1;
                            eprintln!(
                                "pull {peer_name}: unverifiable event quarantined durably \
                                 (sync_quarantine): {verr}; apply door said: {e}"
                            );
                        }
                        Err(qe) => {
                            frozen = true;
                            eprintln!(
                                "pull {peer_name}: HALTING watermark at {max_w}/{max_c} — an \
                                 unverifiable event could not be durably quarantined, so it \
                                 must not be skipped: {qe}; verify error: {verr}"
                            );
                        }
                    },
                }
            }
        }
    }

    // Loud mixed-version detection (issue #108): a batch that is unverifiable IN
    // ITS ENTIRETY is not the odd corrupt frame — it is the signature of a peer
    // whose whole history predates ADR-0040 (or a systematically broken link).
    // Silently returning "0 applied" every cycle would livelock the link with no
    // operator-visible failure; fail the pull instead, so `run` logs it as loudly
    // as a partition on every cycle until the peer is fixed. Everything shipped
    // was already preserved durably above, and the watermark never moved (no
    // event verified), so nothing is lost. Note an already-synced link never
    // trips this: the boundary event at the watermark re-ships and re-applies
    // (idempotent Ok), making the batch mixed.
    if !resp.events.is_empty() && skipped_unverifiable == resp.events.len() {
        let declared = match &resp.signing_context {
            Some(ctx) => format!("declares signing context '{ctx}'"),
            None => "declares no signing context (a pre-ADR-0040 build would not)".to_string(),
        };
        return Err(format!(
            "pull {peer_name}: ALL {} shipped events are unverifiable and the peer {declared} — \
             it appears to serve pre-ADR-0040 (or corrupt) signatures. Every event was preserved \
             in sync_quarantine (inspect with `cairn-sync quarantine`); re-initialize/re-sign the \
             peer, or if THIS node was at fault run `cairn-sync requeue` after fixing it. \
             Watermark untouched.",
            resp.events.len()
        )
        .into());
    }

    client.execute(
        "UPDATE sync_state SET hlc_wall=$1, hlc_counter=$2, last_pull_at=clock_timestamp() WHERE peer=$3",
        &[&max_w, &max_c, &peer_name],
    )?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    Ok(serde_json::json!({
        "op": "pull", "peer": peer_name,
        "shipped": resp.events.len(), "applied_new": applied,
        "skipped_unverifiable": skipped_unverifiable,
        "watermark_frozen": frozen,
        "event_bytes": event_bytes, "wire_bytes": wire_bytes,
        "bytes_per_event": if resp.events.is_empty() { 0.0 }
                           else { event_bytes as f64 / resp.events.len() as f64 },
        "elapsed_ms": elapsed_ms,
        "watermark_wall": max_w, "watermark_counter": max_c
    }))
}

/// Re-process every quarantined event through the real apply door (issue #108's
/// "inspectable and re-processable"): an event that now verifies — e.g. it was
/// falsely rejected by a version-skewed daemon binary since upgraded — is applied
/// and its row cleared; one that still fails stays held with its reason refreshed
/// from the door. Never a raw INSERT: release goes through `apply_remote_event`
/// (db/020), so requeue can only ever ADMIT what the floor admits.
fn do_requeue(client: &mut postgres::Client) -> R<serde_json::Value> {
    let rows = client.query(
        "SELECT content_digest, signed_bytes, attestation, attester_key
         FROM sync_quarantine ORDER BY first_seen",
        &[],
    )?;
    let (mut released, mut still_quarantined) = (0usize, 0usize);
    for row in &rows {
        let digest: Vec<u8> = row.get(0);
        let signed: Vec<u8> = row.get(1);
        let att: Option<Vec<u8>> = row.get(2);
        let akey: Option<Vec<u8>> = row.get(3);
        match apply_signed(client, &signed, att.as_deref(), akey.as_deref()) {
            Ok(_) => {
                client.execute(
                    "DELETE FROM sync_quarantine WHERE content_digest=$1",
                    &[&digest],
                )?;
                released += 1;
                eprintln!("requeue: released {} through the apply door", hex_prefix(&digest));
            }
            Err(e) => {
                // Still refused: keep the row, refresh the reason so the operator
                // sees the CURRENT rejection, not the one from quarantine time.
                client.execute(
                    "UPDATE sync_quarantine
                     SET last_seen = clock_timestamp(), reason = $2
                     WHERE content_digest = $1",
                    &[&digest, &e.to_string()],
                )?;
                still_quarantined += 1;
                eprintln!("requeue: {} still refused: {e}", hex_prefix(&digest));
            }
        }
    }
    Ok(serde_json::json!({
        "op": "requeue",
        "examined": rows.len(),
        "released": released,
        "still_quarantined": still_quarantined
    }))
}

/// First 16 hex chars of a content digest — enough to identify a row in logs and
/// to paste into a `WHERE encode(content_digest,'hex') LIKE '…%'` inspection query.
fn hex_prefix(digest: &[u8]) -> String {
    let h = hex::encode(digest);
    h[..h.len().min(16)].to_string()
}

fn cmd_requeue(conn: &str, metrics: bool) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let m = do_requeue(&mut client)?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "requeue: {} examined, {} released, {} still quarantined",
            m["examined"], m["released"], m["still_quarantined"]
        );
    }
    Ok(())
}

/// List the quarantine (one JSON line per row, newest last) so an operator can see
/// exactly which events a link skipped, from which peer, and why — without psql.
fn cmd_quarantine(conn: &str) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let rows = client.query(
        "SELECT encode(content_digest,'hex'), peer, reason, octet_length(signed_bytes),
                first_seen::text, last_seen::text, seen_count
         FROM sync_quarantine ORDER BY first_seen",
        &[],
    )?;
    for r in &rows {
        println!(
            "{}",
            serde_json::json!({
                "digest": r.get::<_, String>(0),
                "peer": r.get::<_, String>(1),
                "reason": r.get::<_, String>(2),
                "bytes": r.get::<_, i32>(3),
                "first_seen": r.get::<_, String>(4),
                "last_seen": r.get::<_, String>(5),
                "seen_count": r.get::<_, i32>(6)
            })
        );
    }
    eprintln!("{} event(s) in quarantine", rows.len());
    Ok(())
}

fn cmd_pull(conn: &str, peer: &str, peer_name: &str, metrics: bool) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let m = do_pull(&mut client, peer, peer_name)?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "pulled from {peer_name}: {} shipped, {} new, {} skipped-unverifiable, watermark-frozen={}",
            m["shipped"], m["applied_new"], m["skipped_unverifiable"], m["watermark_frozen"]
        );
    }
    Ok(())
}

/// The lazy byte tier (§6.6 / §8.2): for each blob whose bytes are missing, fetch
/// its slices with `window` worker threads, each round-robining across the swarm
/// `peers`, each verifying every slice against the content address (§4.4) before
/// persisting it to `blob_chunk`. Verified slices accumulate across passes/drops
/// (resumable); when every index is present the blob is assembled, whole-blob
/// re-verified, and flipped to present. Every worker sleeps `budget_ms` between
/// requests so windowing stays preemptible and never starves clinical sync
/// (ADR-0013 availability floor). Returns metrics for the harness.
fn do_blobd(
    client: &mut postgres::Client,
    conn: &str,
    peers: &[String],
    window: usize,
    budget_ms: u64,
) -> R<serde_json::Value> {
    // Bound the worker pool: each worker opens a PG connection and adds parallel
    // link load, so the effective byte-tier budget is budget_ms * window. Clamp so a
    // large --window can never exhaust connections or breach the availability floor.
    let window = window.clamp(1, 16);

    let missing = client.query(
        "SELECT encode(blob_address,'hex'), byte_len FROM blob_store WHERE NOT present",
        &[],
    )?;

    let mut completed = 0usize;
    let rejected = Arc::new(AtomicU64::new(0));
    let fetched = Arc::new(AtomicU64::new(0));

    for row in missing {
        let addr_hex: String = row.get(0);
        let byte_len: Option<i64> = row.get(1);
        let total = match byte_len {
            Some(n) if n > 0 => n as u64,
            _ => {
                eprintln!(
                    "blob {} referenced but byte_len unknown — skipping until a reference supplies it",
                    &addr_hex[..16]
                );
                continue;
            }
        };
        let addr = hex::decode(&addr_hex)?;
        let n_chunks = total.div_ceil(SLICE_BYTES as u64) as usize;

        // Resume: which indexes are already persisted?
        let have: HashSet<i32> = client
            .query("SELECT chunk_index FROM blob_chunk WHERE blob_address=$1", &[&addr])?
            .iter()
            .map(|r| r.get::<_, i32>(0))
            .collect();
        let todo: VecDeque<usize> = (0..n_chunks).filter(|i| !have.contains(&(*i as i32))).collect();

        if !todo.is_empty() {
            let queue = Arc::new(Mutex::new(todo));
            let mut handles = Vec::new();
            for w in 0..window {
                let queue = Arc::clone(&queue);
                let rejected = Arc::clone(&rejected);
                let fetched = Arc::clone(&fetched);
                let peers = peers.to_vec();
                let addr_hex = addr_hex.clone();
                let addr = addr.clone();
                let conn = conn.to_string();
                handles.push(std::thread::spawn(move || {
                    // Worker returns (); DB/link errors are logged and the worker moves on
                    // (the index stays missing and is retried next pass). A Box<dyn Error>
                    // return would not be Send across the thread boundary.
                    let mut wc = match postgres::Client::connect(&conn, postgres::NoTls) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("blob worker connect failed: {e}");
                            return;
                        }
                    };
                    let root = match cairn_event::blake3_root_from_address(&addr) {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    loop {
                        let idx = match queue.lock().unwrap().pop_front() {
                            Some(i) => i,
                            None => break,
                        };
                        let offset = idx as u64 * SLICE_BYTES as u64;
                        let len = (SLICE_BYTES as u64).min(total - offset);
                        // Try peers (offset by worker+index for swarm spread) until one
                        // returns a slice that VERIFIES. A lying/faulty source is rejected
                        // here and the next source is tried — the per-slice-verify payoff.
                        // try_request (single attempt) fails over fast, unlike request's backoff.
                        let mut got: Option<Vec<u8>> = None;
                        for k in 0..peers.len() {
                            let peer = &peers[(w + idx + k) % peers.len()];
                            std::thread::sleep(Duration::from_millis(budget_ms)); // preemptible budget
                            let raw = match try_request(
                                peer,
                                &Request::BlobSlice { addr_hex: addr_hex.clone(), offset, len },
                            ) {
                                Ok(r) => r,
                                Err(_) => continue, // link drop / dead peer -> next source
                            };
                            let (found, _total, slice) = decode_blob_slice(&raw);
                            if !found {
                                continue;
                            }
                            match cairn_event::verify_slice(slice, &root, offset, len) {
                                Ok(bytes) => {
                                    got = Some(bytes);
                                    break;
                                }
                                Err(_) => {
                                    rejected.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        if let Some(bytes) = got {
                            // chunk_index is i32 (SQL INT): a blob exceeding ~549 GB at
                            // 256 KiB slices would overflow it. Far beyond any DICOM study,
                            // but the dedicated object-store tier (not BYTEA) is where a
                            // wider index would live if that ceiling ever mattered.
                            if let Err(e) = wc.execute(
                                "INSERT INTO blob_chunk (blob_address, chunk_index, content)
                                 VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                                &[&addr, &(idx as i32), &bytes],
                            ) {
                                eprintln!("blob_chunk insert failed: {e}");
                            } else {
                                fetched.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        // If no source verified this index, leave it missing; the next
                        // do_blobd pass retries it from persisted state (resumable).
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        }

        // Assemble if every index is now present.
        let have_now: i64 = client
            .query_one("SELECT count(*) FROM blob_chunk WHERE blob_address=$1", &[&addr])?
            .get(0);
        if have_now as usize == n_chunks && n_chunks > 0 {
            let rows = client.query(
                "SELECT content FROM blob_chunk WHERE blob_address=$1 ORDER BY chunk_index",
                &[&addr],
            )?;
            let mut buf = Vec::with_capacity(total as usize);
            for r in rows {
                let c: Vec<u8> = r.get(0);
                buf.extend_from_slice(&c);
            }
            // Belt-and-suspenders whole-blob verify before serving as present (§4.4).
            if blob_address(&buf) == addr {
                let outboard = cairn_event::blob_outboard(&buf);
                let mut tx = client.transaction()?;
                tx.execute(
                    "UPDATE blob_store SET content=$1, outboard=$2, present=TRUE, byte_len=$3,
                         fetched_at=clock_timestamp() WHERE blob_address=$4",
                    &[&buf, &outboard, &(buf.len() as i64), &addr],
                )?;
                tx.execute("DELETE FROM blob_chunk WHERE blob_address=$1", &[&addr])?;
                tx.commit()?;
                completed += 1;
                eprintln!("fetched blob {} ({} bytes, verified)", &addr_hex[..16], buf.len());
            } else {
                // Per-slice verify should make this unreachable; purge and retry if not.
                client.execute("DELETE FROM blob_chunk WHERE blob_address=$1", &[&addr])?;
                eprintln!("blob {} failed whole-blob verify — purged", &addr_hex[..16]);
            }
        }
    }

    Ok(serde_json::json!({
        "op": "blobd",
        "blobs_completed": completed,
        "slices_fetched": fetched.load(Ordering::Relaxed),
        "slices_rejected": rejected.load(Ordering::Relaxed),
        "window": window,
        "peers": peers.len()
    }))
}

fn cmd_blobd(
    conn: &str,
    peers: &[String],
    window: usize,
    budget_ms: u64,
    metrics: bool,
) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let m = do_blobd(&mut client, conn, peers, window, budget_ms)?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "byte tier: {} blob(s) completed, {} slices fetched, {} rejected",
            m["blobs_completed"], m["slices_fetched"], m["slices_rejected"]
        );
    }
    Ok(())
}

fn cmd_serve(conn: String, listen: &str, corrupt: bool) -> R<()> {
    let listener = TcpListener::bind(listen)?;
    eprintln!("serving on {listen}{}", if corrupt { " (CORRUPT: test fault injection)" } else { "" });
    for stream in listener.incoming() {
        let stream = stream?;
        let conn = conn.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_conn(&conn, stream, corrupt) {
                eprintln!("connection error: {e}");
            }
        });
    }
    Ok(())
}

/// Unattended field runner: serve in the background, then every `interval_ms`
/// pull clinical events, take a blob step, and snapshot a fingerprint — appending
/// one JSON line per cycle to `log_path`. Survives link drops (each pull/blob
/// failure is logged as a partition and the loop continues), so an operator can
/// start it and walk away for hours of real Starlink variability, then analyse the
/// log with `harness/bet_a.py analyze`. Runs until `duration_s` (0 = until killed).
#[allow(clippy::too_many_arguments)]
fn cmd_run(
    conn: &str,
    listen: &str,
    peer: &str,
    peer_name: &str,
    blob_peers: Vec<String>,
    window: usize,
    interval_ms: u64,
    budget_ms: u64,
    log_path: &str,
    duration_s: u64,
) -> R<()> {
    {
        let (c, l) = (conn.to_string(), listen.to_string());
        std::thread::spawn(move || {
            if let Err(e) = cmd_serve(c, &l, false) {
                eprintln!("serve thread exited: {e}");
            }
        });
    }
    let mut log = OpenOptions::new().create(true).append(true).open(log_path)?;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    eprintln!("run: serving on {listen}, pulling {peer_name} ({peer}) every {interval_ms}ms -> {log_path}");

    // The lazy byte tier runs on its OWN thread, never inline in the clinical pull
    // loop. do_blobd fetches a whole blob to completion; inlining it would let a
    // single multi-MB blob over a high-latency link head-of-line-block clinical
    // sync for the entire fetch — the exact availability-floor violation ADR-0013
    // forbids ("byte transfer must never reduce clinical-data availability").
    // Spawned like the serve thread; the main loop below does clinical work only.
    let blobs_fetched = Arc::new(AtomicU64::new(0));
    {
        let conn = conn.to_string();
        let peers = if blob_peers.is_empty() { vec![peer.to_string()] } else { blob_peers.clone() };
        let counter = Arc::clone(&blobs_fetched);
        std::thread::spawn(move || match postgres::Client::connect(&conn, postgres::NoTls) {
            Ok(mut bclient) => loop {
                match do_blobd(&mut bclient, &conn, &peers, window, budget_ms) {
                    Ok(m) => {
                        counter.fetch_add(m["blobs_completed"].as_u64().unwrap_or(0), Ordering::Relaxed)
                    }
                    Err(_) => 0, // peer unreachable: the next pass retries, never fatal
                };
                std::thread::sleep(Duration::from_millis(interval_ms));
            },
            Err(e) => eprintln!("blob thread could not connect: {e}"),
        });
    }

    let start = Instant::now();
    let mut cycle: u64 = 0;
    loop {
        cycle += 1;
        let mut line = serde_json::json!({ "ts": now_ms(), "cycle": cycle });
        let mut status = format!("cycle {cycle}");

        match do_pull(&mut client, peer, peer_name) {
            Ok(m) => {
                status += &format!(": pull {} shipped / {} new", m["shipped"], m["applied_new"]);
                line["pull"] = m;
            }
            Err(e) => {
                // A sustained outage (retries exhausted) = a partition; since issue
                // #108 a pull also fails LOUDLY on wire-format skew / an
                // all-unverifiable peer, so surface the reason on the status line
                // rather than a blanket "unreachable".
                status += &format!(": PULL FAILED: {e}");
                line["partition"] = serde_json::json!(true);
                line["pull_error"] = serde_json::json!(e.to_string());
            }
        }
        // Cumulative blobs fetched by the separate byte-tier thread (informational;
        // never blocks this loop).
        line["blobs_fetched"] = serde_json::json!(blobs_fetched.load(Ordering::Relaxed));
        if let Ok(fp) = do_fingerprint(&mut client) {
            status += &format!(
                ", {} events, blobs {}+{}",
                fp["events"], fp["blobs_present"], fp["blobs_referenced_only"]
            );
            line["fingerprint"] = fp;
        }

        writeln!(log, "{line}")?;
        log.flush()?;
        eprintln!("{status}");

        if duration_s > 0 && start.elapsed().as_secs() >= duration_s {
            break;
        }
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    Ok(())
}

fn serve_conn(conn: &str, mut stream: TcpStream, corrupt: bool) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let raw = read_frame(&mut stream)?;
    let req: Request = serde_json::from_slice(&raw)?;
    let resp: Vec<u8> = match req {
        Request::EventsAfter { wall, counter } => {
            // Ship the attestation token (and attester key) beside each event: the
            // receiver's in-DB apply door re-runs the attestation gate, so a
            // suppressing event without its travelling proof is refused there.
            let rows = client.query(
                "SELECT encode(signed_bytes,'hex'), encode(attestation,'hex'),
                        encode(attester_key,'hex')
                 FROM event_log
                 WHERE (hlc_wall, hlc_counter) >= ($1,$2)
                 ORDER BY hlc_wall, hlc_counter, node_origin",
                &[&wall, &counter],
            )?;
            let events = rows.iter().map(|r| r.get::<_, String>(0)).collect();
            let attestations = rows.iter().map(|r| r.get::<_, Option<String>>(1)).collect();
            let attester_keys = rows.iter().map(|r| r.get::<_, Option<String>>(2)).collect();
            serde_json::to_vec(&EventsResponse {
                events,
                attestations,
                attester_keys,
                // Declare the context we mint under (issue #108) so a skewed
                // puller can refuse the batch deterministically and legibly.
                signing_context: Some(CTX_EVENT.as_str().to_string()),
            })?
        }
        Request::BlobSlice {
            addr_hex,
            offset,
            len,
        } => {
            let addr = hex::decode(&addr_hex)?;
            let row = client.query_opt(
                "SELECT content, outboard, octet_length(content)
                 FROM blob_store WHERE blob_address=$1 AND present AND outboard IS NOT NULL",
                &[&addr],
            )?;
            match row {
                Some(r) => {
                    let content: Vec<u8> = r.get(0);
                    let outboard: Vec<u8> = r.get(1);
                    let total = r.get::<_, i32>(2) as u64;
                    // Clamp the final slice to the blob's end.
                    let len = len.min(total.saturating_sub(offset));
                    let mut slice = cairn_event::extract_slice(&content, &outboard, offset, len)?;
                    // TEST-ONLY fault injection: if started with --corrupt, flip a byte of
                    // every outgoing slice so the receiver's per-slice verify (§4.4) rejects
                    // it. This proves the swarm heals around a lying/faulty source; it is
                    // never enabled in a real node.
                    if corrupt && !slice.is_empty() {
                        let m = slice.len() / 2;
                        slice[m] ^= 0x01;
                    }
                    encode_blob_slice(true, total, &slice)
                }
                None => encode_blob_slice(false, 0, &[]),
            }
        }
    };
    write_frame(&mut stream, &resp)?;
    Ok(())
}

// ---------------------------------------------------------------------------
fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// All values for a repeatable flag, e.g. `--blob-peer A --blob-peer B`.
fn flags(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}

fn usage() -> ! {
    eprintln!(
        "cairn-sync — Cairn walking skeleton (Spike 0001)

USAGE (all take --conn <postgres-uri>):
  init        --conn URI
  enroll      --conn URI --key PATH [--kind human|agent|device]
              (owner ceremony: register the key as an actor so the apply door admits its events)
  write       --conn URI --node NAME --key PATH --type T --patient (UUID|new)
              --schema SV --json '<body>' [--effective ISO8601]
  gen         --conn URI --node NAME --key PATH [--patients N] [--count N] [--rate EV_PER_SEC]
  put-blob    --conn URI --file PATH --media MEDIA_TYPE
  gen-blob    --conn URI [--size-mb N] [--media MEDIA_TYPE]   (mint a large local blob to fetch)
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics]
  quarantine  --conn URI    (list events skipped as unverifiable: digest, peer, reason)
  requeue     --conn URI [--metrics]
              (re-process quarantined events through the apply door after fixing the cause)
  blobd       --conn URI (--peer HOST:PORT | --blob-peer HOST:PORT ...) [--window N] [--budget-ms N] [--metrics]
  serve       --conn URI --listen HOST:PORT [--corrupt]
  fingerprint --conn URI    (convergence/honest-state JSON for the harness)
  run         --conn URI --listen HOST:PORT --peer HOST:PORT --peer-name NAME
              [--blob-peer HOST:PORT ...] [--window N] [--interval-ms N] [--budget-ms N] [--log PATH] [--duration-s N]
              (unattended: serve+pull+blob, logs one JSON line/cycle, survives drops)
  bench-insert --conn URI --node NAME --key PATH [--count N]   (Bet B B1: maintained-write latency)
  chart       --conn URI --patient UUID                        (Bet B B2: chart-read latency)
  bench       [--hash-mb N] [--sig-iters N] [--dek-iters N]    (Bet B B3/B4: crypto throughput, no DB)
  sign-stdin  --key PATH    (read JSON EventBody on stdin, write hex COSE_Sign1 on stdout)
  attest-stdin --key PATH    (read JSON AttestationBody on stdin, write hex COSE_Sign1 token on stdout)
  key-id      --key PATH    (print the hex Ed25519 public key / kid for the key file)

Run over WireGuard; NoTls is intentional (the link is the transport)."
    );
    std::process::exit(2)
}

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("");
    let conn = flag(&args, "--conn");
    let need = |o: Option<String>| o.unwrap_or_else(|| usage());

    match cmd {
        "init" => cmd_init(&need(conn))?,
        "enroll" => cmd_enroll(
            &need(conn),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
            &flag(&args, "--kind").unwrap_or_else(|| "device".into()),
        )?,
        "write" => cmd_write(
            &need(conn),
            &need(flag(&args, "--node")),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
            &need(flag(&args, "--type")),
            &need(flag(&args, "--patient")),
            &flag(&args, "--schema").unwrap_or_else(|| "v1".into()),
            &need(flag(&args, "--json")),
            flag(&args, "--effective"),
        )?,
        "gen" => cmd_gen(
            &need(conn),
            &need(flag(&args, "--node")),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
            flag(&args, "--patients").and_then(|s| s.parse().ok()).unwrap_or(10),
            flag(&args, "--count").and_then(|s| s.parse().ok()).unwrap_or(100),
            flag(&args, "--rate").and_then(|s| s.parse().ok()).unwrap_or(0.0),
        )?,
        "put-blob" => cmd_put_blob(
            &need(conn),
            &need(flag(&args, "--file")),
            &need(flag(&args, "--media")),
        )?,
        "fingerprint" => cmd_fingerprint(&need(conn))?,
        "bench-insert" => cmd_bench_insert(
            &need(conn),
            &need(flag(&args, "--node")),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
            flag(&args, "--count").and_then(|s| s.parse().ok()).unwrap_or(200),
        )?,
        "chart" => cmd_chart(&need(conn), &need(flag(&args, "--patient")))?,
        "bench" => cmd_bench(
            flag(&args, "--hash-mb").and_then(|s| s.parse().ok()).unwrap_or(256),
            flag(&args, "--sig-iters").and_then(|s| s.parse().ok()).unwrap_or(20000),
            flag(&args, "--dek-iters").and_then(|s| s.parse().ok()).unwrap_or(100000),
        )?,
        "pull" => cmd_pull(
            &need(conn),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            args.iter().any(|a| a == "--metrics"),
        )?,
        "quarantine" => cmd_quarantine(&need(conn))?,
        "requeue" => cmd_requeue(&need(conn), args.iter().any(|a| a == "--metrics"))?,
        "gen-blob" => cmd_gen_blob(
            &need(conn),
            flag(&args, "--size-mb").and_then(|s| s.parse().ok()).unwrap_or(8),
            &flag(&args, "--media").unwrap_or_else(|| "application/dicom".into()),
        )?,
        "blobd" => {
            let single = flag(&args, "--peer");
            let mut peers = flags(&args, "--blob-peer");
            if peers.is_empty() {
                peers.push(need(single));
            }
            cmd_blobd(
                &need(conn),
                &peers,
                flag(&args, "--window").and_then(|s| s.parse().ok()).unwrap_or(4),
                flag(&args, "--budget-ms").and_then(|s| s.parse().ok()).unwrap_or(20),
                args.iter().any(|a| a == "--metrics"),
            )?
        }
        "serve" => cmd_serve(
            need(conn),
            &need(flag(&args, "--listen")),
            args.iter().any(|a| a == "--corrupt"),
        )?,
        "run" => cmd_run(
            &need(conn),
            &need(flag(&args, "--listen")),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            flags(&args, "--blob-peer"),
            flag(&args, "--window").and_then(|s| s.parse().ok()).unwrap_or(4),
            flag(&args, "--interval-ms").and_then(|s| s.parse().ok()).unwrap_or(2000),
            flag(&args, "--budget-ms").and_then(|s| s.parse().ok()).unwrap_or(20),
            &flag(&args, "--log").unwrap_or_else(|| "cairn-run.jsonl".into()),
            flag(&args, "--duration-s").and_then(|s| s.parse().ok()).unwrap_or(0),
        )?,
        "sign-stdin" => cmd_sign_stdin(
            &flag(&args, "--key").unwrap_or_else(|| "agent.key".into()),
        )?,
        "attest-stdin" => cmd_attest_stdin(
            &flag(&args, "--key").unwrap_or_else(|| "human.key".into()),
        )?,
        "key-id" => cmd_key_id(
            &flag(&args, "--key").unwrap_or_else(|| "agent.key".into()),
        )?,
        _ => usage(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{event_address, generate_key, verify_attestation};

    #[test]
    fn attest_token_hex_is_verifiable_and_address_bound() {
        // The CLI core must produce a token the verifier accepts for the right
        // key+address and rejects for a different address (the binding guarantee).
        let (sk, kid) = generate_key().unwrap();
        let vk = sk.verifying_key();
        let ca = event_address(b"some signed event bytes");
        let input = format!(
            r#"{{"content_address_hex":"{}","attester_key_id":"{}","role":"attested"}}"#,
            hex::encode(&ca), kid
        );

        let token_hex = attestation_token_hex(&input, &sk).unwrap();
        let token = hex::decode(&token_hex).unwrap();

        assert!(verify_attestation(&token, &ca, &vk), "token verifies for right key + address");
        let other = event_address(b"a different event");
        assert!(!verify_attestation(&token, &other, &vk), "token is bound to its content-address");
    }

    #[test]
    fn t_effective_offset_pin_accepts_explicit_and_refuses_naive() {
        // Conformant: explicit offsets in every accepted shape (H4 wire pin).
        for ok in [
            "2026-06-20T10:00:00Z",
            "2026-06-20t10:00:00z",
            "2026-06-20T10:00:00+02:00",
            "2026-06-20 10:00:00-05:30",
            "2026-06-20T10:00:00.123+0200",
            "2026-06-20T10:00+02",
        ] {
            assert!(t_effective_has_explicit_offset(ok), "should accept {ok}");
        }
        // Non-conformant: offset-less (a different instant on different nodes),
        // date-only, or garbage — the author must not sign these.
        for bad in [
            "2026-06-20T10:00:00",
            "2026-06-20 10:00:00.123",
            "2026-06-20",
            "yesterday",
            "",
        ] {
            assert!(!t_effective_has_explicit_offset(bad), "should refuse {bad}");
        }
    }

    #[test]
    fn events_response_decodes_pre_attestation_wire_format() {
        // Additive wire evolution (ADR-0012 / principle 11): a response from a peer
        // predating the attestation arrays must still decode — the arrays default
        // empty, which the pull loop reads as "no attestation travelled".
        let old = br#"{"events":["deadbeef"]}"#;
        let resp: EventsResponse = serde_json::from_slice(old).unwrap();
        assert_eq!(resp.events.len(), 1);
        assert!(resp.attestations.is_empty(), "missing field defaults empty");
        assert!(resp.attester_keys.is_empty());
        assert_eq!(
            resp.attestations.first().and_then(|o| o.as_deref()),
            None,
            "per-event lookup on the short array reads None (no token shipped)"
        );
        // Same additivity for the issue #108 signing-context declaration: a peer
        // predating it decodes as None ("undeclared"), never an error.
        assert_eq!(resp.signing_context, None);
    }
}

/// Issue #108 integration coverage: durable quarantine + loud mixed-version
/// handling on the clinical-plane pull path. Real Postgres + cairn_pgx, gated on
/// `$CAIRN_TEST_PG`, serialized against every other DB-gated suite via the shared
/// advisory-lock key (see cairn-node `db::test_serial_guard`). Each test serves a
/// CANNED `EventsResponse` from a throwaway local TCP listener, so the exact
/// mixed-batch / all-unverifiable / skewed-context wire shapes are constructed
/// byte-for-byte rather than hoped for.
#[cfg(test)]
mod quarantine_tests {
    use super::*;

    fn cs() -> Option<String> {
        std::env::var("CAIRN_TEST_PG").ok()
    }

    /// A realistic HLC wall (≈2026) so ceiling checks compare against a sane instant.
    const WALL_2026: i64 = 1_782_000_000_000;

    /// Connect + take the cluster-wide test advisory lock (same 'CARN' key every
    /// DB-gated suite uses), then (re)apply the schema and reset the tables this
    /// suite touches. The returned client HOLDS the lock until dropped.
    fn locked_client(base: &str) -> postgres::Client {
        let mut c = postgres::Client::connect(base, postgres::NoTls).unwrap();
        c.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64]).unwrap();
        c.batch_execute("CREATE EXTENSION IF NOT EXISTS cairn_pgx;").unwrap();
        for (_name, sql) in SCHEMA {
            c.batch_execute(sql).unwrap();
        }
        c.batch_execute(
            "TRUNCATE event_log, actor_event, patient_chart, sync_state, sync_quarantine CASCADE;
             UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0;",
        )
        .unwrap();
        c
    }

    /// Enroll a fresh agent signing key so the apply door admits its events.
    fn enrolled_key(c: &mut postgres::Client) -> (SigningKey, String) {
        let (sk, kid) = cairn_event::generate_key().unwrap();
        c.execute(
            "SELECT enroll_actor('agent', '{\"model\":\"quarantine-test-peer\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
            &[&kid],
        )
        .unwrap();
        (sk, kid)
    }

    /// A validly-signed note.added "arriving from a peer" at the given HLC wall.
    fn peer_note(sk: &SigningKey, kid: &str, wall: i64) -> Vec<u8> {
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: uuid::Uuid::now_v7().to_string(),
            event_type: "note.added".into(),
            schema_version: "note/1".into(),
            hlc: Hlc { wall, counter: 0, node_origin: "peer-src".into() },
            t_effective: None,
            signer_key_id: kid.into(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: serde_json::json!({"text": "replicated note"}),
            attachments: vec![],
            plaintext_twin: Some("Progress note: replicated note".into()),
        };
        sign(&body, sk).unwrap().signed_bytes
    }

    /// Serve `raw` (a pre-encoded EventsResponse JSON) to up to `times` connections
    /// on a throwaway local port; returns the address for `do_pull`.
    fn serve_canned(raw: Vec<u8>, times: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            for _ in 0..times {
                let Ok((mut s, _)) = listener.accept() else { break };
                let _ = read_frame(&mut s);
                let _ = write_frame(&mut s, &raw);
            }
        });
        addr
    }

    fn response_json(events: &[&[u8]], signing_context: Option<&str>) -> Vec<u8> {
        serde_json::to_vec(&EventsResponse {
            events: events.iter().map(hex::encode).collect(),
            attestations: vec![None; events.len()],
            attester_keys: vec![None; events.len()],
            signing_context: signing_context.map(str::to_string),
        })
        .unwrap()
    }

    #[derive(Debug, PartialEq)]
    struct QRow {
        peer: String,
        reason: String,
        seen_count: i32,
    }

    fn quarantine_rows(c: &mut postgres::Client) -> Vec<QRow> {
        c.query(
            "SELECT peer, reason, seen_count FROM sync_quarantine ORDER BY first_seen",
            &[],
        )
        .unwrap()
        .iter()
        .map(|r| QRow { peer: r.get(0), reason: r.get(1), seen_count: r.get(2) })
        .collect()
    }

    fn watermark(c: &mut postgres::Client, peer: &str) -> (i64, i32) {
        let row = c
            .query_one("SELECT hlc_wall, hlc_counter FROM sync_state WHERE peer=$1", &[&peer])
            .unwrap();
        (row.get(0), row.get(1))
    }

    /// A mixed batch (valid · garbage · valid): the garbage event is quarantined
    /// DURABLY (bytes + legible reason), the valid events apply, and the watermark
    /// may then advance past the quarantined one — the durable trace is what makes
    /// that admissible (issue #108: no more silent set-union violation). A re-offer
    /// of the same bytes dedupes onto the same row (seen_count bumps).
    #[test]
    fn pull_quarantines_unverifiable_and_advances_watermark() {
        let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let garbage = b"not a COSE_Sign1 at all".to_vec();
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&e1, &garbage, &e2], Some(CTX_EVENT.as_str()));

        let addr = serve_canned(raw.clone(), 1);
        let m = do_pull(&mut c, &addr, "peer-a").unwrap();
        assert_eq!(m["applied_new"], 2, "both valid events applied");
        assert_eq!(m["skipped_unverifiable"], 1);
        assert_eq!(m["watermark_frozen"], false, "quarantine is durable, no freeze");

        let events: i64 =
            c.query_one("SELECT count(*) FROM event_log", &[]).unwrap().get(0);
        assert_eq!(events, 2);

        // The durable trace: verbatim bytes + peer + a legible reason.
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "exactly the garbage event is quarantined");
        assert_eq!(rows[0].peer, "peer-a");
        assert!(!rows[0].reason.trim().is_empty(), "reason must be legible, got empty");
        assert_eq!(rows[0].seen_count, 1);
        let held: Vec<u8> = c
            .query_one("SELECT signed_bytes FROM sync_quarantine", &[])
            .unwrap()
            .get(0);
        assert_eq!(held, garbage, "quarantine holds the verbatim wire bytes");

        // Watermark advanced to the LAST VALID event — past the quarantined one,
        // which is safe now that its durable trace exists.
        assert_eq!(watermark(&mut c, "peer-a"), (WALL_2026 + 2_000, 0));

        // Re-offer of the identical batch: idempotent applies, deduped quarantine.
        let addr = serve_canned(raw, 1);
        let m = do_pull(&mut c, &addr, "peer-a").unwrap();
        assert_eq!(m["applied_new"], 0, "set-union no-op on re-apply");
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "same bytes dedupe onto one row");
        assert_eq!(rows[0].seen_count, 2, "re-offer bumps seen_count");
    }

    /// A peer whose ENTIRE batch is unverifiable (and that declares no signing
    /// context — the pre-ADR-0040 legacy shape) must fail the pull LOUDLY instead
    /// of silently skipping and livelocking, while still preserving every event
    /// durably. The watermark must not move.
    #[test]
    fn pull_fails_loud_when_every_event_is_unverifiable() {
        let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
        let mut c = locked_client(&base);

        let g1 = b"legacy or corrupt blob one".to_vec();
        let g2 = b"legacy or corrupt blob two".to_vec();
        // Legacy peer shape: NO signing_context field at all.
        let raw = serde_json::to_vec(&serde_json::json!({
            "events": [hex::encode(&g1), hex::encode(&g2)],
        }))
        .unwrap();

        let addr = serve_canned(raw.clone(), 1);
        let err = do_pull(&mut c, &addr, "peer-legacy").unwrap_err().to_string();
        assert!(
            err.contains("pre-ADR-0040"),
            "diagnosis must name the likely cause (mixed-version peer), got: {err}"
        );
        assert!(
            err.contains("unverifiable"),
            "diagnosis must say what happened, got: {err}"
        );

        // Loud, but nothing lost: both events preserved durably, watermark untouched.
        assert_eq!(quarantine_rows(&mut c).len(), 2);
        assert_eq!(watermark(&mut c, "peer-legacy"), (0, 0));

        // The next cycle fails loudly AGAIN (no silent livelock) and the
        // quarantine dedupes rather than growing without bound.
        let addr = serve_canned(raw, 1);
        assert!(do_pull(&mut c, &addr, "peer-legacy").is_err());
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.seen_count == 2), "re-offers bump, never duplicate");
    }

    /// A peer that DECLARES a different signing context is deterministic wire-format
    /// skew: refuse the whole batch up front with a legible error naming both
    /// contexts — don't burn per-event verify failures or quarantine anything
    /// (the peer still holds the events; they apply after the skew is fixed).
    #[test]
    fn pull_refuses_declared_context_mismatch_deterministically() {
        let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let raw = response_json(&[&e1], Some("application/cairn-event+cbor;v=999"));

        let addr = serve_canned(raw, 1);
        let err = do_pull(&mut c, &addr, "peer-skew").unwrap_err().to_string();
        assert!(
            err.contains("application/cairn-event+cbor;v=999")
                && err.contains(CTX_EVENT.as_str()),
            "error must name BOTH contexts so the operator sees the skew, got: {err}"
        );

        let events: i64 =
            c.query_one("SELECT count(*) FROM event_log", &[]).unwrap().get(0);
        assert_eq!(events, 0, "nothing applied from a batch refused for skew");
        assert!(quarantine_rows(&mut c).is_empty(), "skew-refused batch is not quarantined");
        assert_eq!(watermark(&mut c, "peer-skew"), (0, 0));
    }

    /// Re-processing after the operator fixes the cause (the issue's "inspectable
    /// and re-processable"): a quarantined event that NOW verifies (e.g. it was
    /// falsely rejected by a version-skewed daemon binary since upgraded) is
    /// released through the real apply door and its row cleared; one that still
    /// fails stays held with a refreshed reason.
    #[test]
    fn requeue_releases_quarantined_events_once_cause_is_fixed() {
        let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // Simulate a past false rejection: a perfectly valid event sitting in
        // quarantine (as if the daemon that pulled it was version-skewed), plus
        // one genuinely corrupt blob that can never be released.
        let good = peer_note(&sk, &kid, WALL_2026 + 5_000);
        let junk = b"permanently corrupt".to_vec();
        for (bytes, why) in [(&good, "simulated version-skew rejection"), (&junk, "corrupt")] {
            c.execute(
                "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
                 VALUES ($1, $2, 'peer-a', $3)",
                &[&cairn_event::event_address(bytes), bytes, &why],
            )
            .unwrap();
        }

        let m = do_requeue(&mut c).unwrap();
        assert_eq!(m["examined"], 2);
        assert_eq!(m["released"], 1, "the now-valid event goes through the apply door");
        assert_eq!(m["still_quarantined"], 1);

        let events: i64 =
            c.query_one("SELECT count(*) FROM event_log", &[]).unwrap().get(0);
        assert_eq!(events, 1, "released event landed in event_log via the door");
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "released row is cleared, corrupt row stays");
        assert!(rows[0].reason.contains("verification"), "reason refreshed from the door");
    }
}
