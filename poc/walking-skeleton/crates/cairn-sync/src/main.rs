//! Cairn walking skeleton — the thin sync daemon (Spike 0001 §3, §5).
//!
//! Set-union ship/apply over a tiny framed protocol (run over WireGuard; NoTls is
//! deliberate — the link is the transport). Two planes, exactly as the spec
//! separates them:
//!
//!   * **clinical plane** (`serve` events / `pull`): eager, small, high priority —
//!     ships signed event bytes; the receiver *verifies on apply* (Bet A2) and
//!     inserts idempotently (`ON CONFLICT DO NOTHING` — set-union, Bet A1).
//!   * **byte tier** (`serve` blob chunks / `blobd`): lazy, chunked, preemptible,
//!     separately budgeted — must never starve the clinical plane (Bet A4).
//!
//! This daemon carries NO merge logic (ADR-0001/§9.4): convergence is set-union +
//! the in-DB projection trigger. It only ships bytes, verifies, and applies.

use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_event::{blob_address, plaintext_twin, sign, verify_self_described, EventBody, Hlc, SigningKey};
use serde::{Deserialize, Serialize};

const SCHEMA: [(&str, &str); 3] = [
    ("001_envelope", include_str!("../../../db/001_envelope.sql")),
    ("002_projection", include_str!("../../../db/002_projection.sql")),
    ("003_blobs", include_str!("../../../db/003_blobs.sql")),
];

const BLOB_CHUNK: usize = 64 * 1024;

type R<T> = Result<T, Box<dyn Error>>;

// ---------------------------------------------------------------------------
// Wire protocol — one JSON request, one JSON response, per connection.
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize)]
#[serde(tag = "op")]
enum Request {
    /// Clinical plane: every event at or after this HLC watermark.
    EventsAfter { wall: i64, counter: i32 },
    /// Byte tier: a chunk of a blob's bytes.
    BlobChunk {
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
}

#[derive(Serialize, Deserialize)]
struct BlobResponse {
    found: bool,
    total_len: u64,
    bytes_hex: String,
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
    eprintln!("generated new signing key at {path} (kid {})", &kid[..16]);
    Ok((sk, kid))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

// ---------------------------------------------------------------------------
// Apply: insert a verified event idempotently (set-union) and merge the HLC.
// Shared by `pull`. Verification happens HERE (Bet A2 / the §9 seam); in
// production this gate is an in-DB pgrx function so it cannot be bypassed.
// ---------------------------------------------------------------------------
fn apply_signed(client: &mut postgres::Client, signed_bytes: &[u8]) -> R<bool> {
    let body = verify_self_described(signed_bytes)?; // refuse anything that doesn't verify
    let content_address = cairn_event::event_address(signed_bytes);
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let attachments_json = serde_json::to_string(&body.attachments)?;
    let twin = plaintext_twin(&body);

    let mut tx = client.transaction()?;
    let inserted = tx.execute(
        "INSERT INTO event_log
           (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
            node_origin, t_effective, signed_bytes, content_address, body, contributors,
            signer_key_id, plaintext_twin, attachments)
         VALUES ($1::text::uuid,$2::text::uuid,$3,$4,$5,$6,$7,$8::text::timestamptz,$9,$10,
                 $11::text::jsonb,$12::text::jsonb,$13,$14,$15::text::jsonb)
         ON CONFLICT DO NOTHING",
        &[
            &body.event_id,
            &body.patient_id,
            &body.event_type,
            &body.schema_version,
            &body.hlc.wall,
            &body.hlc.counter,
            &body.hlc.node_origin,
            &body.t_effective,
            &signed_bytes.to_vec(),
            &content_address,
            &body_json,
            &contributors_json,
            &body.signer_key_id,
            &twin,
            &attachments_json,
        ],
    )?;

    // Learn any attachment references this event carries (reference-eager).
    for att in &body.attachments {
        if let Ok(addr) = hex::decode(&att.digest_hex) {
            tx.execute(
                "SELECT blob_note_reference($1,$2,$3)",
                &[&addr, &att.media_type, &att.byte_len],
            )?;
        }
    }

    // HLC merge: local clock never falls behind an event we have accepted (A3).
    tx.execute(
        "UPDATE hlc_state SET hlc_wall = GREATEST(hlc_wall, $1),
             hlc_counter = CASE WHEN $1 > hlc_wall THEN $2
                                WHEN $1 = hlc_wall THEN GREATEST(hlc_counter, $2)
                                ELSE hlc_counter END
         WHERE id",
        &[&body.hlc.wall, &body.hlc.counter],
    )?;
    tx.commit()?;
    Ok(inserted == 1)
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------
fn cmd_init(conn: &str) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    for (name, sql) in SCHEMA {
        client.batch_execute(sql)?;
        eprintln!("applied {name}");
    }
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
    };

    let signed = sign(&body, sk)?;
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let twin = plaintext_twin(&body);

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
    let len = bytes.len() as i64;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, present, fetched_at)
         VALUES ($1,$2,$3,$4,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, present=TRUE, byte_len=EXCLUDED.byte_len,
                fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &bytes],
    )?;
    println!("stored blob {} ({} bytes, {})", hex::encode(&addr), len, media);
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

    let (mut applied, mut verify_failures, mut event_bytes) = (0usize, 0usize, 0usize);
    let (mut max_w, mut max_c) = (wall, counter);
    for hexed in &resp.events {
        let signed_bytes = hex::decode(hexed)?;
        event_bytes += signed_bytes.len(); // A5: real clinical-plane payload (the COSE blob)
        match apply_signed(client, &signed_bytes) {
            Ok(new) => {
                if new {
                    applied += 1;
                }
                if let Ok(b) = verify_self_described(&signed_bytes) {
                    if (b.hlc.wall, b.hlc.counter) > (max_w, max_c) {
                        max_w = b.hlc.wall;
                        max_c = b.hlc.counter;
                    }
                }
            }
            // A2: a verification failure is recorded, never applied, never poisons the pull.
            Err(_) => verify_failures += 1,
        }
    }
    client.execute(
        "UPDATE sync_state SET hlc_wall=$1, hlc_counter=$2, last_pull_at=clock_timestamp() WHERE peer=$3",
        &[&max_w, &max_c, &peer_name],
    )?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    Ok(serde_json::json!({
        "op": "pull", "peer": peer_name,
        "shipped": resp.events.len(), "applied_new": applied,
        "verify_failures": verify_failures,
        "event_bytes": event_bytes, "wire_bytes": wire_bytes,
        "bytes_per_event": if resp.events.is_empty() { 0.0 }
                           else { event_bytes as f64 / resp.events.len() as f64 },
        "elapsed_ms": elapsed_ms,
        "watermark_wall": max_w, "watermark_counter": max_c
    }))
}

fn cmd_pull(conn: &str, peer: &str, peer_name: &str, metrics: bool) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let m = do_pull(&mut client, peer, peer_name)?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "pulled from {peer_name}: {} shipped, {} new, {} verify-failures",
            m["shipped"], m["applied_new"], m["verify_failures"]
        );
    }
    Ok(())
}

/// The lazy byte tier (Bet A4): fetch missing blobs in chunks, sleeping a budget
/// between chunks so it is preemptible and cannot saturate the link the clinical
/// plane shares. Crude on purpose — the real tier rate-budgets, this just shows
/// the discipline: byte transfer NEVER blocks clinical sync.
fn do_blobd(client: &mut postgres::Client, peer: &str, budget_ms: u64) -> R<usize> {
    let missing = client.query(
        "SELECT encode(blob_address,'hex'), byte_len FROM blob_store WHERE NOT present",
        &[],
    )?;
    let mut fetched = 0usize;
    for row in missing {
        let addr_hex: String = row.get(0);
        let mut buf: Vec<u8> = Vec::new();
        let mut total: u64 = 0;
        loop {
            let raw = request(
                peer,
                &Request::BlobChunk {
                    addr_hex: addr_hex.clone(),
                    offset: buf.len() as u64,
                    len: BLOB_CHUNK as u64,
                },
            )?;
            let resp: BlobResponse = serde_json::from_slice(&raw)?;
            if !resp.found {
                eprintln!("peer does not have blob {}", &addr_hex[..16]);
                break;
            }
            total = resp.total_len;
            let chunk = hex::decode(&resp.bytes_hex)?;
            if chunk.is_empty() {
                break;
            }
            buf.extend_from_slice(&chunk);
            std::thread::sleep(Duration::from_millis(budget_ms)); // preemptible budget
            if buf.len() as u64 >= total {
                break;
            }
        }
        if buf.len() as u64 != total {
            continue; // resumable: try again next pass, verifying what arrived
        }
        // Content-verify before storing (§4.4 — a wrong-hash blob is never served).
        let got = blob_address(&buf);
        if hex::encode(&got) != addr_hex {
            eprintln!("blob {} failed BLAKE3 verification — discarded", &addr_hex[..16]);
            continue;
        }
        client.execute(
            "UPDATE blob_store SET content=$1, present=TRUE, byte_len=$2, fetched_at=clock_timestamp()
             WHERE blob_address=$3",
            &[&buf, &(buf.len() as i64), &got],
        )?;
        fetched += 1;
        eprintln!("fetched blob {} ({} bytes, verified)", &addr_hex[..16], buf.len());
    }
    Ok(fetched)
}

fn cmd_blobd(conn: &str, peer: &str, budget_ms: u64) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let n = do_blobd(&mut client, peer, budget_ms)?;
    println!("fetched {n} blob(s)");
    Ok(())
}

fn cmd_serve(conn: String, listen: &str) -> R<()> {
    let listener = TcpListener::bind(listen)?;
    eprintln!("serving on {listen}");
    for stream in listener.incoming() {
        let stream = stream?;
        let conn = conn.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_conn(&conn, stream) {
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
    interval_ms: u64,
    budget_ms: u64,
    log_path: &str,
    duration_s: u64,
) -> R<()> {
    {
        let (c, l) = (conn.to_string(), listen.to_string());
        std::thread::spawn(move || {
            if let Err(e) = cmd_serve(c, &l) {
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
        let (c, p) = (conn.to_string(), peer.to_string());
        let counter = Arc::clone(&blobs_fetched);
        std::thread::spawn(move || match postgres::Client::connect(&c, postgres::NoTls) {
            Ok(mut bclient) => loop {
                match do_blobd(&mut bclient, &p, budget_ms) {
                    Ok(n) => counter.fetch_add(n as u64, Ordering::Relaxed),
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
                // A sustained outage (retries exhausted) = a partition. Logged, not fatal.
                status += ": PARTITION (pull unreachable)";
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

fn serve_conn(conn: &str, mut stream: TcpStream) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let raw = read_frame(&mut stream)?;
    let req: Request = serde_json::from_slice(&raw)?;
    let resp: Vec<u8> = match req {
        Request::EventsAfter { wall, counter } => {
            let rows = client.query(
                "SELECT encode(signed_bytes,'hex') FROM event_log
                 WHERE (hlc_wall, hlc_counter) >= ($1,$2)
                 ORDER BY hlc_wall, hlc_counter, node_origin",
                &[&wall, &counter],
            )?;
            let events = rows.iter().map(|r| r.get::<_, String>(0)).collect();
            serde_json::to_vec(&EventsResponse { events })?
        }
        Request::BlobChunk {
            addr_hex,
            offset,
            len,
        } => {
            let addr = hex::decode(&addr_hex)?;
            // substring/octet_length take int4; bind the chunk window as i32.
            // (Skeleton limit: blobs > 2 GiB need bigint offsets — noted in the README.)
            let row = client.query_opt(
                "SELECT encode(substring(content from $2 for $3),'hex'), octet_length(content)
                 FROM blob_store WHERE blob_address=$1 AND present",
                &[&addr, &(offset as i32 + 1), &(len as i32)],
            )?;
            let resp = match row {
                Some(r) => BlobResponse {
                    found: true,
                    total_len: r.get::<_, i32>(1) as u64,
                    bytes_hex: r.get(0),
                },
                None => BlobResponse {
                    found: false,
                    total_len: 0,
                    bytes_hex: String::new(),
                },
            };
            serde_json::to_vec(&resp)?
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

fn usage() -> ! {
    eprintln!(
        "cairn-sync — Cairn walking skeleton (Spike 0001)

USAGE (all take --conn <postgres-uri>):
  init        --conn URI
  write       --conn URI --node NAME --key PATH --type T --patient (UUID|new)
              --schema SV --json '<body>' [--effective ISO8601]
  gen         --conn URI --node NAME --key PATH [--patients N] [--count N] [--rate EV_PER_SEC]
  put-blob    --conn URI --file PATH --media MEDIA_TYPE
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics]
  blobd       --conn URI --peer HOST:PORT [--budget-ms N]
  serve       --conn URI --listen HOST:PORT
  fingerprint --conn URI    (convergence/honest-state JSON for the harness)
  run         --conn URI --listen HOST:PORT --peer HOST:PORT --peer-name NAME
              [--interval-ms N] [--budget-ms N] [--log PATH] [--duration-s N]
              (unattended: serve+pull+blob, logs one JSON line/cycle, survives drops)
  bench-insert --conn URI --node NAME --key PATH [--count N]   (Bet B B1: maintained-write latency)
  chart       --conn URI --patient UUID                        (Bet B B2: chart-read latency)
  bench       [--hash-mb N] [--sig-iters N] [--dek-iters N]    (Bet B B3/B4: crypto throughput, no DB)

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
        "blobd" => cmd_blobd(
            &need(conn),
            &need(flag(&args, "--peer")),
            flag(&args, "--budget-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50),
        )?,
        "serve" => cmd_serve(need(conn), &need(flag(&args, "--listen")))?,
        "run" => cmd_run(
            &need(conn),
            &need(flag(&args, "--listen")),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            flag(&args, "--interval-ms").and_then(|s| s.parse().ok()).unwrap_or(2000),
            flag(&args, "--budget-ms").and_then(|s| s.parse().ok()).unwrap_or(20),
            &flag(&args, "--log").unwrap_or_else(|| "cairn-run.jsonl".into()),
            flag(&args, "--duration-s").and_then(|s| s.parse().ok()).unwrap_or(0),
        )?,
        _ => usage(),
    }
    Ok(())
}
