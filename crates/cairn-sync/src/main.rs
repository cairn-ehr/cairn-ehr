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

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_event::{
    blob_address, materialise_generic_twin, resolve_twin, sign, sign_attestation,
    verify_self_described, AttestationBody, EventBody, Hlc, SigningKey, CTX_EVENT,
};
use serde::{Deserialize, Serialize};

// A slice (not a fixed-size array) so appending a migration is a one-line change
// — the hand-counted length annotation bought nothing and taxed every migration.
//
// DRIFT GUARD: the `schema_subset_tests` module at the bottom of this file loads
// ONLY this list into a wiped database and drives every SQL entry point it ships,
// so a future door→function edge into an unlisted migration fails a test with the
// production error message instead of shipping a first-write outage (issue #198).
const SCHEMA: &[(&str, &str)] = &[
    ("001_envelope", include_str!("../../../db/001_envelope.sql")),
    (
        "002_projection",
        include_str!("../../../db/002_projection.sql"),
    ),
    ("003_blobs", include_str!("../../../db/003_blobs.sql")),
    ("004_actors", include_str!("../../../db/004_actors.sql")),
    ("005_submit", include_str!("../../../db/005_submit.sql")),
    ("006_recall", include_str!("../../../db/006_recall.sql")),
    // The clinical-plane sync apply door (issue #91): replicated events enter
    // event_log only through the in-DB floor, never a daemon-side raw INSERT.
    (
        "020_apply_remote_event",
        include_str!("../../../db/020_apply_remote_event.sql"),
    ),
    // Durable quarantine + re-offer floor for unverifiable pulled events
    // (issue #108): a refused event leaves a durable, re-processable trace and
    // pins the fetch floor so its slot keeps being re-offered — never silent,
    // never lost.
    (
        "021_sync_quarantine",
        include_str!("../../../db/021_sync_quarantine.sql"),
    ),
    // The blob self-verification floor (ADR-0013 point 11): the whole-blob
    // BLAKE3-vs-address check this daemon performs before flipping present is
    // restated in-DB (cairn_blob_verify, cairn_pgx >= 0.3.0) so a bypassing
    // raw-SQL writer cannot mark wrong bytes present either.
    (
        "026_blob_verify_floor",
        include_str!("../../../db/026_blob_verify_floor.sql"),
    ),
    // Both write doors PERFORM cairn_learn_attachment_refs unconditionally (db/005
    // + db/020), and PL/pgSQL late binding means omitting its defining migration
    // loads cleanly and fails only at the FIRST write — a total write outage on a
    // fresh `cairn-sync init` database (issue #198, review finding B3).
    (
        "027_attachment_rendition_references",
        include_str!("../../../db/027_attachment_rendition_references.sql"),
    ),
    // Same late-binding trap: the db/002 patient_chart trigger calls the #157
    // Byzantine HLC-collision predicate/recorder on every patient.created — the
    // first demographic write fails without this file (issue #198 again).
    (
        "029_hlc_collision_log",
        include_str!("../../../db/029_hlc_collision_log.sql"),
    ),
    // The clinical-plane seq cursor (issue #196): event_log.seq +
    // sync_state.last_seq + sync_quarantine.refused_seq. do_pull cursors on seq
    // (never the skip-prone HLC watermark) + cmd_run does a periodic full sweep.
    (
        "036_clinical_sync_seq",
        include_str!("../../../db/036_clinical_sync_seq.sql"),
    ),
    // ADR-0052 born-sealed custody plane: node_unwrap_key / event_dek /
    // erasure_shred_log / event_clear + the shred machinery. Two doors in this
    // subset already HARD-depend on it: the apply door (db/020) references
    // erasure_shred_log inside a top-level IF that Postgres plans on EVERY apply
    // (sealed or not), and the seq serve arm below LEFT JOINs event_dek +
    // erasure_shred_log unconditionally for the custody sidecar. Omitting it would
    // fail the FIRST serve/apply on a fresh `cairn-sync init` database — exactly the
    // #198 late-binding first-write outage this list guards against. Loads standalone
    // because it lands AFTER db/020, which creates the cairn_node role it grants to.
    (
        "037_born_sealed",
        include_str!("../../../db/037_born_sealed.sql"),
    ),
    // The node's recorded schema generation (issue #188): the table behind the
    // downgrade-refusal guard in load_schema below. This subset MUST keep carrying
    // this file — `init` on a fresh database stamps node_schema right after the
    // replay, so the table's migration has to be part of the subset (the unit test
    // in schema_generation_tests pins that). The generation stamped is the repo-wide
    // cairn_event SCHEMA_GENERATION, never derived from this subset's tail — the
    // subset legitimately LAGS db/'s newest file whenever a node-only migration
    // lands (see cairn_event::schema_generation module docs).
    (
        "038_node_schema",
        include_str!("../../../db/038_node_schema.sql"),
    ),
    // #208/ADR-0057: cairn_reproject + reproject_log + the event_type index. In
    // BOTH lists: each loader's gated heal step (generation change) calls it.
    (
        "039_projection_registry",
        include_str!("../../../db/039_projection_registry.sql"),
    ),
];

const SLICE_BYTES: usize = 256 * 1024; // window/slice granularity (tuned; amortizes bao tree overhead)

/// Per-peer bounds on the quarantine pen (PR #110 review finding 2). Identical
/// re-offers dedupe onto one row, so only a peer shipping ever-DIFFERENT garbage
/// (nondeterministic corruption, or malice) can grow the pen — and remote bytes
/// must never be able to fill the clinical node's disk (the ADR-0013
/// resource-isolation stance applied to the quarantine). At the cap the pen
/// refuses further inserts and the pull freezes the watermark instead: delayed,
/// never lost — and loud.
const MAX_QUARANTINE_ROWS_PER_PEER: i64 = 10_000;
const MAX_QUARANTINE_BYTES_PER_PEER: i64 = 64 * 1024 * 1024;

/// Full-sweep cadence (issue #196, mirroring cairn-node's FULL_SWEEP_EVERY): the
/// clinical pull does an incremental seq-cursor pull each cycle and a full sweep
/// (after_seq = 0) every FULL_SWEEP_EVERY cycles. The sweep is the correctness
/// floor — it reconciles any event a residual hazard (BIGSERIAL out-of-order
/// commit) caused incremental to skip. Incremental = optimization; sweep = floor.
///
/// KNOWN COST (issue #101, unpaginated batches): a sweep re-ships the ENTIRE
/// peer log, hex-inflated, in ONE JSON frame inside the 30 s read window. Once a
/// node's history outgrows that window the sweep fails loudly every cadence —
/// the correctness floor stops floor-ing exactly on the largest-history nodes.
/// #101 pagination is the fix; until it lands this cadence assumes a small log.
const FULL_SWEEP_EVERY: u64 = 10;

type R<T> = Result<T, Box<dyn Error>>;

/// The minimum `cairn_pgx` version this daemon requires. Bumped to 0.2.0 for the
/// ADR-0040 signing-context wire format: a pre-0.2.0 `.so` verifies the OLD
/// (uncontextualized) bytes and would reject every event this daemon now signs —
/// a total, silent write outage whose only symptom is a generic "signature
/// verification failed". Gating on the loaded version turns that into a legible
/// "rebuild the extension" at connect time instead (issue #109). Bumped to 0.3.0
/// for the db/026 blob self-verification floor: its trigger guard calls
/// `cairn_blob_verify`, which a 0.2.x `.so` lacks. The guard is PL/pgSQL, so that
/// call is LATE-BOUND — a stale `.so` does NOT fail the schema load; without a
/// gate it surfaces as an illegible `undefined function` only at the first
/// present-flip write. Two layers make it legible instead: db/026 itself refuses
/// to load when `cairn_blob_verify` is absent (a `to_regprocedure` gate, binding
/// every loader including cairn-node), and this connect-time floor catches the
/// `.so`-swapped-after-init skew on the commands that write events or blobs.
const REQUIRED_PGX_FLOOR: &str = "0.3.0";

/// Parse an `"X.Y.Z"` version string into a comparable tuple. Returns `None` for
/// anything that is not exactly three dot-separated non-negative integers — a
/// pre-release suffix or garbage is treated as unparseable so the caller can fail
/// closed rather than guess. Pure (no I/O) so it is unit-testable.
fn parse_pgx_version(s: &str) -> Option<(u64, u64, u64)> {
    let mut parts = s.trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None; // more than three components — not a plain X.Y.Z
    }
    Some((major, minor, patch))
}

/// True iff the `loaded` cairn_pgx version is at least the `floor`. Fails CLOSED
/// (returns false) when either string is unparseable: an unrecognizable version is
/// "cannot confirm compatibility", never silently accepted. Pure — unit-testable.
fn pgx_version_ok(loaded: &str, floor: &str) -> bool {
    match (parse_pgx_version(loaded), parse_pgx_version(floor)) {
        (Some(l), Some(f)) => l >= f,
        _ => false,
    }
}

/// The actionable rejection message for a stale/too-old extension — one place so
/// the `cmd_init` and `connect_checked` call sites read identically.
fn pgx_floor_message(loaded: &str) -> String {
    format!(
        "cairn_pgx {loaded} is loaded, but this cairn-sync requires >= {REQUIRED_PGX_FLOOR} \
         (the ADR-0040 signing-context wire format + the db/026 blob verify floor). The installed \
         extension library is stale — \
         rebuild + reinstall it: `cargo pgrx install` against this cluster's PostgreSQL, then retry."
    )
}

/// Fail fast if the LOADED cairn_pgx `.so` is older than the wire-format floor this
/// daemon needs. Distinct from `connect_checked`'s schema probe: the SQL migrations
/// can be current while the compiled verify library is stale (a `\dx`-invisible skew
/// after a rebuild without reinstall). A pre-0.2.0 library lacks `cairn_pgx_version()`
/// entirely — that missing-function error IS the stale-library signal, so we translate
/// it into the same actionable message rather than leaking a raw "function does not exist".
fn assert_pgx_floor(client: &mut postgres::Client) -> R<()> {
    let loaded: String = match client.query_one("SELECT cairn_pgx_version()", &[]) {
        Ok(row) => row.get(0),
        Err(e) if e.code() == Some(&postgres::error::SqlState::UNDEFINED_FUNCTION) => {
            // The function is unresolvable. The common cause is a stale/pre-0.2.0 library
            // (which lacks it), but a current extension installed in a schema off this
            // role's search_path presents the same 42883 — so name both rather than
            // sending a fine install down a needless rebuild (issue #109 review).
            return Err(format!(
                "cairn_pgx_version() is not callable, so the loaded cairn_pgx cannot be \
                 confirmed at the >= {REQUIRED_PGX_FLOOR} floor this cairn-sync requires \
                 (the ADR-0040 signing-context wire format + the db/026 blob verify floor). \
                 Most likely the installed extension library is stale/pre-0.2.0 — rebuild + reinstall it \
                 (`cargo pgrx install` against this cluster's PostgreSQL); if it is current, \
                 check that cairn_pgx's schema is on this connection role's search_path."
            )
            .into());
        }
        Err(e) => return Err(e.into()),
    };
    if !pgx_version_ok(&loaded, REQUIRED_PGX_FLOOR) {
        return Err(pgx_floor_message(&loaded).into());
    }
    Ok(())
}

/// A pull that FAILED LOUDLY for data-integrity reasons (unverifiable events
/// quarantined, quarantine pen full, or declared signing-context skew) rather
/// than transport reasons. Distinguished from a plain transport error so:
///   * `run` can log it as an integrity condition, NOT a partition — a peer
///     that answers every request is not "unreachable", and the bet_a harness
///     counts the `partition` flag as link downtime (review finding 6);
///   * the per-cycle pull metrics survive into the log line even though the
///     cycle failed (valid events DID apply; the watermark DID advance).
#[derive(Debug)]
struct PullIntegrityError {
    message: String,
    /// The same metrics JSON a successful pull returns (may be `null` for the
    /// pre-loop skew refusal, where no per-event work happened yet).
    metrics: serde_json::Value,
}

impl std::fmt::Display for PullIntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PullIntegrityError {}

/// One decoded wire entry: (signed event bytes, attestation, attester key).
type WireEntry = (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);

// ---------------------------------------------------------------------------
// Wire protocol — one JSON request, one JSON response, per connection.
// ---------------------------------------------------------------------------
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op")]
enum Request {
    /// Clinical plane, HLC-cursored (legacy). KEPT so an older puller still works;
    /// a new puller uses EventsAfterSeq. Every event at or after this HLC watermark.
    EventsAfter { wall: i64, counter: i32 },
    /// Clinical plane, seq-cursored (issue #196): every event whose serving-node
    /// `seq` is strictly greater than `after_seq`, in `seq` order. `after_seq = 0`
    /// returns the full set (the full-sweep path). `seq` is the server's LOCAL
    /// insertion order — the only ordering where newly-learned events always sort
    /// above a puller's cursor, so incremental can never silently skip (#196).
    /// Additive (principle 12): the older EventsAfter variant stays served.
    ///
    /// `unwrap_cert` (ADR-0052 custody sidecar) is the puller's signed unwrap-key
    /// certificate (hex CBOR): it binds the puller's X25519 unwrap public key to its
    /// Ed25519 identity. When present, the server re-wraps each sealed event's DEK
    /// for that key so the puller gains crypto-shred custody of what it replicates
    /// (see rewrap_custody_for_peer). Additive (serde default): an old puller omits
    /// it and the server serves the events with no custody — sealed rows still admit
    /// structurally at the apply door, so nothing fails to sync.
    EventsAfterSeq {
        after_seq: i64,
        #[serde(default)]
        unwrap_cert: Option<String>,
    },
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
    /// Per-event serving-node `seq` (issue #196), PARALLEL to `events`. The puller
    /// checkpoints its per-peer cursor on the max handled seq. Additive (serde
    /// default): an older peer's response decodes with an empty vec — a new puller
    /// that sent EventsAfterSeq treats an events-without-seqs response as a
    /// wire-format error rather than checkpointing blindly (see do_pull).
    #[serde(default)]
    seqs: Vec<i64>,
    /// The ADR-0040 signing context this server's events are minted under
    /// (issue #108). Lets the puller tell deterministic wire-format skew ("your
    /// events are signed for a context I don't speak") from tampering BEFORE
    /// burning a whole batch on per-event verify failures. Additive (serde
    /// default): a response from a peer predating this field decodes as None —
    /// "undeclared" — and the puller falls back to the all-unverifiable
    /// heuristic for the mixed-version diagnosis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signing_context: Option<String>,
    /// Per-event wrapped DEK (hex), PARALLEL to `events` (ADR-0052 custody sidecar).
    /// `wrapped_deks[i]` is the sealed event's data-encryption key RE-WRAPPED for the
    /// pulling peer's unwrap key (from the cert in its request) — the puller opens it
    /// with its own unwrap secret and hands it to the apply door as the 4th arg, so a
    /// replicated sealed event becomes crypto-shreddable on the puller too. A slot is
    /// None whenever no custody travels: the event is unsealed, this node holds no
    /// DEK for it, it has been SHREDDED here (the serve SQL nulls a shredded row's DEK
    /// — the wire-level half of the shred guarantee), or the peer sent no/invalid
    /// cert. Additive (serde default): an old peer omits the field entirely and it
    /// decodes to an empty vec — the puller then applies every event without custody
    /// (sealed rows still admit structurally at the door).
    #[serde(default)]
    wrapped_deks: Vec<Option<String>>,
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

/// Read-side frame cap (issue #202, porting the cairn-node `MAX_FRAME_BYTES`
/// discipline). The 4-byte length prefix is attacker-controlled on both wire ends —
/// the server reads request frames from ANY client that can reach the port (WireGuard
/// is the assumed perimeter, not authentication), and the puller reads response frames
/// from its peer — so an unchecked prefix lets one hostile/corrupt u32 demand a 4 GiB
/// allocation. Unlike the node plane (one frame per event, 8 MiB), the events response
/// here is deliberately UNPAGINATED (issue #101: a full sweep ships the whole log
/// suffix as one hex-encoded JSON frame), so the cap is batch-scale: 64 MiB holds
/// ~20k typical events (~1.5 KiB signed, hex-doubled on the wire) with room to spare.
/// A log that outgrows it fails the sweep LOUDLY with this cap named in the error —
/// pagination (#101) is the real fix for that, tracked there.
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

fn write_frame(s: &mut impl Write, b: &[u8]) -> io::Result<()> {
    // Refuse at the SOURCE, mirroring read_frame's cap (PR #225 review): an over-cap
    // frame would cross the wire in full only to be refused by the peer's read cap,
    // with nothing in the SERVING node's log to say why its peer stopped converging.
    // The decision (cap + u32-truncation-unreachable) lives in the shared
    // cairn_event::framing core (#212); refusing before the prefix is written stays
    // here — a bare length prefix with no body would wedge the reader.
    // A log that outgrows the cap needs pagination: issue #101.
    let prefix =
        cairn_event::framing::encode_len_prefix(b.len(), MAX_FRAME_BYTES).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("refusing to send: {e} (pagination: issue #101)"),
            )
        })?;
    s.write_all(&prefix)?;
    s.write_all(b)?;
    s.flush()
}

fn read_frame(s: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len)?;
    // Refuse BEFORE allocating: the prefix is untrusted input (see MAX_FRAME_BYTES);
    // the decision is the shared cairn_event::framing core (#212).
    let n = cairn_event::framing::decode_len_prefix(len, MAX_FRAME_BYTES)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
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
    let Some(time) = t.get(11..) else {
        return false;
    };
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
    // ADR-0052 custody sidecar: the sealed event's DEK, already unwrapped by the
    // puller for its own key (or None — not a custody holder, a byte-lazy pull, or
    // an unsealed event). Handed straight to the door's 4th arg. With it, the door
    // runs the full clear-view floor and records local custody; without it the
    // sealed row is admitted on structural checks only (set-union losslessness).
    dek: Option<&[u8]>,
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
            "SELECT apply_remote_event($1, $2, $3, $4)",
            &[
                &signed_bytes.to_vec(),
                &attestation.map(|a| a.to_vec()),
                &attester_key.map(|k| k.to_vec()),
                &dek.map(|d| d.to_vec()),
            ],
        )
        // Surface the door's legible RAISE text AND its DETAIL: postgres::Error's Display is
        // just "db error", and message() alone drops the cairn_verify_error DETAIL the doors
        // now attach (issue #109) — the skew-vs-tampering reason that would otherwise reach
        // only a direct psql caller. Appending it here carries it into the freeze/skip log
        // lines, the quarantine reason, and last_requeue_error.
        .map_err(|e| -> Box<dyn Error> {
            match e.as_db_error() {
                Some(db) => match db.detail() {
                    Some(detail) => format!("{} ({detail})", db.message()).into(),
                    None => db.message().to_string().into(),
                },
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
    // `CREATE EXTENSION IF NOT EXISTS` will NOT upgrade an already-installed extension,
    // so a stale library survives it silently. Check the loaded version now — init is the
    // operator's first action, the right place to surface a stale `.so` before it becomes
    // a mystery write outage (issue #109).
    assert_pgx_floor(&mut client)?;
    load_schema(&mut client)
}

/// The schema generation this binary carries — the repo-wide constant, NOT a value
/// derived from this loader's own list. This list is a deliberate SUBSET (issue #198:
/// it must satisfy both write doors standing alone) and it legitimately LAGS db/'s
/// newest file whenever a node-only migration lands — which is the normal case. A
/// per-list derivation would then make this binary report an older generation than
/// the cairn-node that stamped the database, and `init` would refuse every healthy
/// node in the fleet: a downgrade guard that bricks the sync daemon (see
/// `cairn_event::schema_generation` module docs). The constant is kept honest by
/// cairn-event's fs-derived guard test; the subset-shape invariants (contains 038,
/// never exceeds the constant) are pinned in schema_generation_tests below.
fn embedded_schema_version() -> i32 {
    cairn_event::schema_generation::SCHEMA_GENERATION
}

/// Replay the embedded SCHEMA subset — guarded against DOWNGRADE (issue #188).
///
/// `CREATE OR REPLACE` cuts both ways: replayed by an OLDER binary against a NEWER
/// database it silently rewrites newer function bodies — including the in-DB
/// safety-floor checks — back to their older versions. So read the generation the
/// last successful loader recorded (db/038 `node_schema`) and refuse when it exceeds
/// ours; stamp our own only AFTER the full replay succeeded. An absent table or row
/// means "generation unknown" (a pre-#188 database, or a rig loaded by hand via
/// psql) and the replay proceeds — the guard stops known downgrades, it does not
/// lock out hand-built rigs. Mirrors cairn-node db::connect_and_load_schema.
fn load_schema(client: &mut postgres::Client) -> R<()> {
    // Serialize the whole check→replay→stamp against every OTHER loader on this
    // database (2026-07-19 review of PR #251, finding 1): without it the guard is
    // check-then-act — an old and a new binary connecting together can interleave so
    // the old one reads a stale generation, passes, and still replays over the
    // schema the new one just loaded. BLOCKING and session-level: a concurrent
    // loader waits its turn, then reads the now-current record.
    client.execute(
        "SELECT pg_advisory_lock($1)",
        &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
    )?;
    let result = load_schema_under_lock(client);
    // Release on success AND refusal — the caller's client outlives this call, and a
    // lock held for its lifetime would park every later loader forever. If the load
    // failed because the session itself died, the unlock fails too; the load error
    // is the one that must surface (the dead session releases the lock server-side).
    let unlock = client.execute(
        "SELECT pg_advisory_unlock($1)",
        &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
    );
    result?;
    unlock?;
    Ok(())
}

/// The #188 guard + replay + stamp, assumed to run under `SCHEMA_LOAD_LOCK` — only
/// `load_schema` above may call this.
fn load_schema_under_lock(client: &mut postgres::Client) -> R<()> {
    let embedded = embedded_schema_version();
    // Two round-trips, not one CASE: SQL naming node_schema is checked at plan time,
    // so a single statement referencing it errors on exactly the databases (fresh or
    // pre-#188) that must PASS the guard.
    let table_exists: bool = client
        .query_one("SELECT to_regclass('public.node_schema') IS NOT NULL", &[])?
        .get(0);
    if table_exists {
        // query_opt: an absent ROW is a legitimate "generation unknown", but a real
        // query error must still fail loudly.
        if let Some(row) = client.query_opt("SELECT version FROM node_schema", &[])? {
            let recorded: i32 = row.get(0);
            if recorded > embedded {
                return Err(format!(
                    "refusing to load schema: this database was last loaded at schema \
                     generation {recorded}, but this binary embeds only generation \
                     {embedded}. Replaying an older schema would silently downgrade \
                     the in-DB safety floor (issue #188 / ADR-0012). Upgrade this \
                     binary, or point it at a database of its own generation."
                )
                .into());
            }
        }
    }
    for (name, sql) in SCHEMA {
        client.batch_execute(sql)?;
        eprintln!("applied {name}");
    }
    client.execute(
        "INSERT INTO node_schema (version, loader_build) VALUES ($1, $2)
         ON CONFLICT (id) DO UPDATE
           SET version = EXCLUDED.version,
               loaded_at = now(),
               loader_build = EXCLUDED.loader_build",
        &[
            &embedded,
            &concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION")),
        ],
    )?;
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
        // ADR-0051 ratified vocabulary: the node RECORDED this event (contributory);
        // naming the authoring human is the #204 attribution slice, not a role claim.
        // These events cross apply_remote_event (db/020) on every pulling peer, so the
        // entry must carry actor_id + a ratified role or the floor refuses it.
        contributors: serde_json::json!([{ "actor_id": kid, "role": "recorded" }]),
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
    println!(
        "wrote {} {} for patient {}",
        event_type, body.event_id, patient_id
    );
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

/// The two fingerprint aggregation queries, extracted so the collation drift guard
/// (`fingerprint_orderings_compare_under_collate_c`) can assert their ORDER BY text
/// sort keys stay pinned to byte order across future edits.
/// Both orderings pin their TEXT sort keys to byte order with COLLATE "C" (issue
/// #202, the ADR-0045/#69 discipline): the fingerprint exists to PROVE two nodes
/// converged, so a locale-dependent sort — under which two honest nodes with
/// different cluster collations hash identical sets differently — would raise a
/// false divergence alarm from the very tool meant to rule one out.
/// The projection hash interposes '|' between fields (PR #225 review): without a
/// separator, shifting a field boundary — (name 'X', dob '1980') vs (name 'X1',
/// dob '980') — concatenates identically and hashes EQUAL: a false convergence,
/// the inverse failure of the collation alarm above. '|' covers the accidental
/// case; a field embedding '|' itself can still theoretically alias, accepted for
/// a diagnostic (length-prefixing would be proof-grade but unreadable in psql).
const FINGERPRINT_EVENT_HASH_SQL: &str = "SELECT md5(string_agg(encode(content_address,'hex'), ','
     ORDER BY hlc_wall, hlc_counter, node_origin COLLATE \"C\")) FROM event_log";
const FINGERPRINT_PROJECTION_HASH_SQL: &str = "SELECT md5(string_agg(
     patient_id::text || '|' || coalesce(name,'') || '|' || coalesce(dob,'') || '|' ||
     coalesce(sex,'') || '|' || note_count::text, ',' ORDER BY patient_id::text COLLATE \"C\"))
 FROM patient_chart";

/// Emit a convergence/honest-state fingerprint (A1, A3, A6) as JSON. Two nodes
/// have converged iff their `event_hash` and `projection_hash` match.
fn do_fingerprint(client: &mut postgres::Client) -> R<serde_json::Value> {
    let events: i64 = client
        .query_one("SELECT count(*) FROM event_log", &[])?
        .get(0);
    let event_hash: Option<String> = client.query_one(FINGERPRINT_EVENT_HASH_SQL, &[])?.get(0);
    let projection_hash: Option<String> = client
        .query_one(FINGERPRINT_PROJECTION_HASH_SQL, &[])?
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
    emit_event(
        &mut client,
        node,
        &sk,
        &kid,
        "patient.created",
        &pid,
        "patient/1",
        serde_json::json!({"name":"Bench Patient","dob":"1980-01-01","sex":"U"}),
        None,
    )?;

    let mut lat = Vec::with_capacity(count);
    for n in 0..count {
        let t = Instant::now();
        emit_event(
            &mut client,
            node,
            &sk,
            &kid,
            "note.added",
            &pid,
            "note/1",
            serde_json::json!({"text": format!("b1 maintenance sample {n}")}),
            None,
        )?;
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
    // BENCHMARK ONLY: the fixed nonce reused across every encrypt below is a
    // throughput microbench, not a keystore. NEVER copy this into real DEK-wrap /
    // body-seal code — nonce reuse under XChaCha20Poly1305 (same key + same nonce)
    // is catastrophic for confidentiality. Real sealing draws a fresh random nonce
    // per encryption.
    // House rule 6 (#146): bench key/nonce material is DERIVED at runtime, never a
    // byte literal, so CodeQL's hard-coded-crypto-value query stays literal-free and
    // live for production code. Deterministic on purpose — same bench input every run.
    let kek_bytes: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3).wrapping_add(9));
    let kek = XChaCha20Poly1305::new(Key::from_slice(&kek_bytes));
    let nonce_bytes: [u8; 24] = std::array::from_fn(|i| i as u8);
    let nonce = XNonce::from_slice(&nonce_bytes);
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8) ^ 3);
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
    let body_seal_mbps =
        (dek_iters as f64 * body.len() as f64 / (1 << 20) as f64) / t.elapsed().as_secs_f64();

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

/// A representative ~1.5 KB clear medication payload for the seal microbench. Shaped like
/// a real `clinical.medication.asserted` body (substance / dose / sig / provenance) and
/// padded to ~1.5 KB with a realistic free-text instruction block, so the AEAD measures a
/// clinically-representative body size rather than a toy `{"a":1}`. Pure — no crypto
/// material here (the recipient keypair is generated fresh in `cmd_bench_seal`).
fn representative_medication_payload() -> serde_json::Value {
    // A believable, sizeable sig/notes block — the kind of free text a real medication
    // record carries — repeated to bring the whole body to roughly 1.5 KB.
    let instructions = "Take one tablet by mouth twice daily with food. Do not stop \
        abruptly; review renal function and HbA1c at the next visit. Counsel on \
        hypoglycaemia awareness and GI side effects. "
        .repeat(7);
    serde_json::json!({
        "medication_id": uuid::Uuid::now_v7().to_string(),
        "substance": {"term": "metformin", "inn_code": "6809"},
        "formulation": "tablet",
        "dose": {"amount": "500", "unit": "mg"},
        "sig": "one BD",
        "info_source": "patient-reported",
        "started": {"value": "2023", "precision": "year"},
        "instructions": instructions,
    })
}

/// ADR-0052 seal-plane microbench (Task 13): time the four born-sealed crypto stages a
/// single sealed event traverses on the write+replicate path, over a ~1.5 KB
/// representative medication body, and print ns/op per stage.
///
///   seal   = `seal_event_payload`  (fresh DEK, XChaCha20-Poly1305 over payload + twin)
///   wrap   = `wrap_dek_for`        (ECIES X25519 → HKDF KEK → AEAD over the DEK)
///   unwrap = `unwrap_dek`          (the puller/apply-door opening the re-wrapped DEK)
///   unseal = `unseal_event_payload`(open the sealed body with its DEK)
///
/// This feeds the deferred per-event-vs-per-episode DEK-granularity question (ADR-0005 /
/// ADR-0052) and checks the whole pipeline against the Bet-B ~4 ms p95 latency budget:
/// it is AEAD over ~1.5 KB, so it must land microseconds-scale, orders below the budget.
///
/// House rule 6: the recipient unwrap keypair is DERIVED at runtime from a freshly
/// generated seed (`generate_key`), never a hard-coded byte literal — and the seal/wrap
/// primitives draw their own fresh DEK + nonce from the OS RNG internally, so nothing in
/// this bench presents hard-coded cryptographic material to the scanner.
fn cmd_bench_seal(iters: usize) -> R<()> {
    use cairn_event::seal::{
        derive_unwrap_secret, seal_event_payload, unseal_event_payload, unwrap_dek, unwrap_public,
        wrap_dek_for,
    };

    let payload = representative_medication_payload();
    let payload_bytes = serde_json::to_vec(&payload)?.len();
    let twin = "metformin 500 mg tablet — one BD, patient-reported, started 2023";
    let event_id = uuid::Uuid::now_v7().to_string();

    // Recipient (node) unwrap keypair, derived at runtime from a fresh random seed.
    let (sk, _kid) = cairn_event::generate_key()?;
    let secret = derive_unwrap_secret(&sk.to_bytes());
    let public = unwrap_public(&secret);

    // Stage 1: seal the body under a fresh per-event DEK.
    let t = Instant::now();
    let mut last_seal = seal_event_payload(&payload, twin, &event_id)?;
    for _ in 1..iters {
        last_seal = seal_event_payload(&payload, twin, &event_id)?;
        std::hint::black_box(&last_seal);
    }
    let seal_ns = t.elapsed().as_nanos() as f64 / iters as f64;
    let (container, dek) = last_seal;

    // Stage 2: wrap the DEK for the recipient node (the custody sidecar).
    let t = Instant::now();
    let mut wrapped = wrap_dek_for(&dek, &public)?;
    for _ in 1..iters {
        wrapped = wrap_dek_for(&dek, &public)?;
        std::hint::black_box(&wrapped);
    }
    let wrap_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    // Stage 3: unwrap the DEK with the recipient's secret (apply door / puller).
    let t = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(unwrap_dek(&wrapped, &secret)?);
    }
    let unwrap_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    // Stage 4: unseal the body with its DEK (the clear-view read).
    let t = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(unseal_event_payload(&container, &dek, &event_id)?);
    }
    let unseal_ns = t.elapsed().as_nanos() as f64 / iters as f64;

    println!(
        "{}",
        serde_json::json!({
            "op": "bench_seal",
            "iters": iters,
            "payload_bytes": payload_bytes,
            "seal_ns": seal_ns,
            "wrap_ns": wrap_ns,
            "unwrap_ns": unwrap_ns,
            "unseal_ns": unseal_ns,
            "pipeline_ns": seal_ns + wrap_ns + unwrap_ns + unseal_ns,
        })
    );
    Ok(())
}

fn cmd_put_blob(conn: &str, file: &str, media: &str) -> R<()> {
    let bytes = std::fs::read(file)?;
    let addr = blob_address(&bytes);
    let outboard = cairn_event::blob_outboard(&bytes);
    let len = bytes.len() as i64;
    // Gated connect: this INSERT flips present, which fires the db/026
    // cairn_blob_verify trigger — a stale `.so` must fail here legibly.
    let mut client = connect_checked_apply(conn)?;
    // One atomic, idempotent statement. Deliberate cost: flipping a
    // reference-only row pays the trigger hash twice (BEFORE INSERT fires on the
    // proposed row before conflict detection, then the DO UPDATE re-fires the
    // UPDATE trigger) — accepted on this operator CLI path; the hot path
    // (do_blobd's assembly flip) is a single UPDATE and pays once.
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1,$2,$3,$4,$5,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, outboard=EXCLUDED.outboard, present=TRUE,
                byte_len=EXCLUDED.byte_len, fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &bytes, &outboard],
    )?;
    println!(
        "stored blob {} ({} bytes, {})",
        hex::encode(&addr),
        len,
        media
    );
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
    // Gated connect + same deliberate ON CONFLICT double-hash tradeoff as
    // cmd_put_blob (see the comment there).
    let mut client = connect_checked_apply(conn)?;
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
/// `last_seen`/`seen_count`, and ENRICHING a missing attestation via COALESCE —
/// a token once seen on the wire is never dropped) — repeated cycles against a
/// broken peer must not grow the table. Distinct garbage is bounded by the
/// per-peer quota (`MAX_QUARANTINE_*`, counting UNACKED rows only — issue
/// #197); at the quota this returns Err and the caller freezes the watermark
/// instead — delayed, never lost.
///
/// Returns `acked`: whether a human has already licensed this exact exclusion
/// (`sync_quarantine.acked`). An acked row must not pin the re-offer floor and
/// must not fail the pull — the skip is a recorded human decision.
fn quarantine_event(
    client: &mut postgres::Client,
    peer_name: &str,
    signed_bytes: &[u8],
    attestation: Option<&[u8]>,
    attester_key: Option<&[u8]>,
    refused_seq: i64,
    reason: &str,
) -> R<bool> {
    // Surface the database's own message on failure (postgres::Error's Display
    // is just "db error", which would strip the reason from the freeze logs).
    fn legible(e: postgres::Error) -> Box<dyn Error> {
        match e.as_db_error() {
            Some(db) => db.message().to_string().into(),
            None => e.into(),
        }
    }
    let digest = cairn_event::event_address(signed_bytes);
    // Dedupe first: a re-offer of known bytes always succeeds (it does not grow
    // the pen), even when the peer is over quota.
    let bumped = client
        .query_opt(
            "UPDATE sync_quarantine
            SET last_seen    = clock_timestamp(),
                seen_count   = seen_count + 1,
                attestation  = COALESCE(attestation, $2),
                attester_key = COALESCE(attester_key, $3)
          WHERE content_digest = $1
          RETURNING acked",
            &[&digest, &attestation, &attester_key],
        )
        .map_err(legible)?;
    if let Some(row) = bumped {
        return Ok(row.get(0));
    }
    // New bytes: admit only within the per-peer quota (INCLUDING this event's
    // own size, so one huge frame cannot overshoot the byte budget). The
    // aggregate probes ride the same statement so the check and the insert
    // cannot disagree; a concurrent writer can still overshoot by one row —
    // the cap is a resource budget, not an exact invariant. Only UNACKED rows
    // count (issue #197, mirrors the node plane): an acked row is a resolved
    // human decision, retained as the record of it, never auto-deleted — if it
    // still consumed quota, "ack the held rows" (this function's own documented
    // remedy, below) could never free the pen and a peer that flooded then got
    // acked would wedge the cursor forever, with a manual DELETE the only way out.
    // The accepted flip side: the retained ACKED set is bounded per ack round (each
    // ack licenses another quota's worth of kept bytes), not absolutely — operators
    // may DELETE acked rows to reclaim disk (the db/021 grant exists for this).
    // refused_seq (issue #196) is set on INSERT only; the dedupe UPDATE above leaves
    // it untouched — FORENSICS ("at what serving seq was this first refused"), never
    // a fetch input. The re-offer POSITION is sync_state.quarantine_floor_seq, a
    // separate column do_pull recomputes each cycle precisely so it can self-clear
    // on a clean cycle while this pen row survives as the audit trace. (Deriving the
    // floor from min(refused_seq) here — the node-plane model — was considered and
    // REJECTED: it re-ships from the low seq forever after a one-time corruption
    // heals, until a manual ack. See the db/036 header + the #223 PR description.)
    let inserted = client
        .execute(
            "INSERT INTO sync_quarantine
             (content_digest, signed_bytes, attestation, attester_key, peer, refused_seq, reason)
         SELECT $1,$2,$3,$4,$5,$6,$7
          WHERE (SELECT count(*) FROM sync_quarantine
                   WHERE peer = $5 AND NOT acked) < $8
            AND (SELECT COALESCE(sum(octet_length(signed_bytes)),0)
                   FROM sync_quarantine
                   WHERE peer = $5 AND NOT acked) + octet_length($2::bytea) <= $9
         ON CONFLICT (content_digest) DO NOTHING",
            &[
                &digest,
                &signed_bytes,
                &attestation,
                &attester_key,
                &peer_name,
                &refused_seq,
                &reason,
                &MAX_QUARANTINE_ROWS_PER_PEER,
                &MAX_QUARANTINE_BYTES_PER_PEER,
            ],
        )
        .map_err(legible)?;
    if inserted == 0 {
        // Zero rows means EITHER over-quota OR a concurrent writer penned the
        // same bytes first (ON CONFLICT DO NOTHING). Distinguish them — a false
        // "quota" diagnosis on the safety path would send the operator chasing
        // a condition that does not exist.
        if let Some(row) = client.query_opt(
            "SELECT acked FROM sync_quarantine WHERE content_digest = $1",
            &[&digest],
        )? {
            return Ok(row.get(0)); // lost a benign race: the trace exists
        }
        return Err(format!(
            "quarantine pen for peer '{peer_name}' is at its quota of unacked rows \
             ({MAX_QUARANTINE_ROWS_PER_PEER} rows / {MAX_QUARANTINE_BYTES_PER_PEER} bytes) — \
             refusing to grow it; the watermark freezes instead (delayed, never lost). \
             Inspect with `cairn-sync quarantine` and fix or ack the held rows \
             (acked rows stop counting against the quota)."
        )
        .into());
    }
    Ok(false)
}

/// Pull from `peer` on the clinical plane, seq-cursored (issue #196). Cursors on
/// the serving node's LOCAL event_log.seq (db/036) instead of the HLC watermark, so
/// an event that lands below an advanced watermark — a multi-hop arrival, or an L2
/// self-stamped low HLC — still sorts above the cursor and is never silently
/// skipped. `full_sweep` requests from seq 0 (the periodic correctness floor for the
/// residual BIGSERIAL out-of-order-commit gap); cmd_run drives the cadence.
fn do_pull(
    client: &mut postgres::Client,
    peer: &str,
    peer_name: &str,
    full_sweep: bool,
    // This node's signing key, when the caller wants custody on the wire (ADR-0052).
    // From it we derive our unwrap secret and present the matching CERT to the peer,
    // so the peer can re-wrap each sealed event's DEK for us. `None` (older call
    // paths, DB tests) pulls WITHOUT custody: events still sync, sealed rows admit
    // structurally at the door — custody is simply not gained on this cycle.
    key: Option<&SigningKey>,
) -> R<serde_json::Value> {
    client.execute(
        "INSERT INTO sync_state (peer) VALUES ($1) ON CONFLICT (peer) DO NOTHING",
        &[&peer_name],
    )?;
    // The committed seq cursor + the seq re-offer floor for this peer (db/036).
    // `last_seq` is the highest serving-node event_log.seq we have pulled — the
    // node-LOCAL insertion order, NOT the HLC. `quarantine_floor_seq` (NULL = none)
    // is the seq of the first unresolved refused slot, a SEPARATE persisted column
    // so it self-clears on a clean cycle while the pen row survives as an audit trace.
    let st = client.query_one(
        "SELECT last_seq, quarantine_floor_seq FROM sync_state WHERE peer=$1",
        &[&peer_name],
    )?;
    let last_seq: i64 = st.get(0);
    let floor_seq: Option<i64> = st.get(1);

    // Fetch point: a full sweep pulls everything (after_seq = 0); otherwise from the
    // cursor, pulled back to just BELOW the earliest refused slot when a floor is set
    // — so that slot keeps being re-offered every cycle (deduping onto its pen row)
    // even as the cursor advances for valid events, and a repaired/re-signed version
    // is admitted AUTOMATICALLY. The `-1` is load-bearing: serve streams
    // `seq > after_seq` (STRICT), so fetching from floor_seq itself would skip the
    // very slot we re-offer.
    // saturating_sub: the floor is ≥ 1 for anything persisted by a validated pull
    // (see the seqs guard below), but an inherited/hand-edited row must degrade to
    // "fetch everything" rather than wrap the arithmetic.
    let after_seq: i64 = if full_sweep {
        0
    } else {
        floor_seq.map_or(last_seq, |f| f.saturating_sub(1).min(last_seq))
    };

    // This node's unwrap identity for the pull (ADR-0052 custody sidecar). When a
    // signing key is available we keep our unwrap SECRET (to open re-wrapped DEKs the
    // peer sends back) and present our unwrap CERT (so the peer can re-wrap for us).
    // The cert binds our X25519 unwrap public key to our Ed25519 identity — the same
    // key the DEKs come back wrapped for. Absent a key we pull without custody.
    let unwrap_secret = key.map(|sk| cairn_event::seal::derive_unwrap_secret(&sk.to_bytes()));
    let unwrap_cert: Option<String> = match (key, &unwrap_secret) {
        (Some(sk), Some(secret)) => {
            let public = cairn_event::seal::unwrap_public(secret);
            Some(hex::encode(cairn_event::sign_unwrap_key_cert(sk, &public)?))
        }
        _ => None,
    };

    let started = Instant::now();
    // A transport failure here has one skew-shaped cause worth naming (PR #223
    // review): a pre-#196 serve cannot decode the EventsAfterSeq op — its serde
    // rejects the unknown tag, the connection drops with no response frame, and
    // all this side sees is an EOF. Say so, alongside the plain-partition reading,
    // instead of leaving the operator a bare "failed to fill whole buffer".
    let raw = request(
        peer,
        &Request::EventsAfterSeq {
            after_seq,
            unwrap_cert,
        },
    )
    .map_err(|e| {
        format!(
            "pull {peer_name}: no response to EventsAfterSeq: {e}. If the peer is \
             down this is a plain partition (retry later); but if it is reachable \
             and hangs up without answering, it likely predates the #196 seq-cursor \
             wire (db/036) and cannot decode this request — upgrade the peer binary \
             (an OLD puller against THIS node still works; an old server cannot be \
             seq-pulled)."
        )
    })?;
    let wire_bytes = raw.len();
    let resp: EventsResponse = serde_json::from_slice(&raw)?;

    // Deterministic wire-format skew check (issue #108): a peer that DECLARES a
    // signing context we don't speak would fail verification for every event it
    // ships — refuse the batch up front with an error naming both contexts,
    // rather than burning per-event failures whose generic "unverifiable" reason
    // misdirects the operator toward tampering. Nothing is quarantined and the
    // cursor is untouched: the peer still holds the events, and they apply
    // normally once the skew (one side needs upgrading) is fixed. A peer that
    // declares NOTHING is an older build — per-event verification decides, and
    // the all-unverifiable diagnosis below catches the pure-legacy case.
    // (Per-event verification binds CTX_EVENT cryptographically anyway; this
    // gate adds no new rejection, only a legible one-line diagnosis.)
    if let Some(peer_ctx) = &resp.signing_context {
        if peer_ctx != CTX_EVENT.as_str() {
            return Err(Box::new(PullIntegrityError {
                message: format!(
                    "pull {peer_name}: peer declares signing context '{peer_ctx}' but this \
                     node expects '{}' — wire-format skew, not tampering; upgrade the older \
                     side. Batch refused, cursor untouched.",
                    CTX_EVENT.as_str()
                ),
                metrics: serde_json::Value::Null,
            }));
        }
    }

    // The per-event seq is load-bearing for the cursor (issue #196): a response
    // carrying events but a short/empty seqs array is a malformed or unexpectedly-old
    // serve — fail LOUDLY rather than checkpoint the cursor blind.
    if !resp.events.is_empty() && resp.seqs.len() != resp.events.len() {
        return Err(format!(
            "pull {peer_name}: peer returned {} events but {} seqs — cannot checkpoint the \
             seq cursor safely; the peer serves an incompatible/older wire format",
            resp.events.len(),
            resp.seqs.len()
        )
        .into());
    }
    // …and the VALUES are untrusted wire input that persists into sync_state (the
    // advance-only cursor + the re-offer floor). A well-formed serve (`WHERE seq >
    // $1 ORDER BY seq` over an IDENTITY starting at 1) always produces strictly-
    // ascending positive seqs; the contiguous-prefix freeze below RELIES on the
    // ordering, and the floor's `-1` fetch arithmetic on positivity. A batch
    // violating either (a buggy or hostile peer) is refused loudly with the cursor
    // untouched — wire values must not poison persistent cursor state (PR #223
    // review). (A peer lying HIGH about its own seqs only starves its own
    // incremental serving; the periodic full sweep remains the correctness floor.)
    if resp.seqs.first().is_some_and(|&s| s < 1) || resp.seqs.windows(2).any(|w| w[1] <= w[0]) {
        return Err(format!(
            "pull {peer_name}: peer returned malformed seqs (must be strictly ascending \
             and positive) — refusing to checkpoint the seq cursor from these values"
        )
        .into());
    }

    // Cursor discipline (review fix A1 + issue #108 + the PR #110 review), re-keyed
    // to the node-local seq for #196:
    //   * a VERIFIABLE event that fails to APPLY (transient DB error, deterministic
    //     refusal) FREEZES the cursor at the contiguous handled prefix — retried
    //     next cycle, never skipped (A1). (During a FULL SWEEP a failing event may
    //     sit BELOW the committed cursor; the freeze then leaves the cursor as-is
    //     and the retry rides the NEXT sweep, not the next incremental — up to
    //     FULL_SWEEP_EVERY cycles later. Delayed, never lost.);
    //   * an UNVERIFIABLE entry (bad signature, garbage bytes, non-hex text) is
    //     quarantined durably (db/021) AND pins the re-offer floor at its seq. The
    //     cursor still advances for valid events, but the floor keeps the refused
    //     slot on the wire every cycle, so a later repaired/re-signed version is
    //     admitted automatically — a durable trace alone is NOT a license to move
    //     past an event. Only a human `acked` row or a clean cycle releases the slot;
    //   * if the pen itself refuses (insert failure, per-peer quota), the cursor
    //     freezes exactly as for a valid apply failure — delayed, never lost.
    // Any unacked refusal makes the whole pull FAIL LOUDLY at the end.
    let (mut applied, mut skipped_unverifiable, mut skipped_acked, mut event_bytes) =
        (0usize, 0usize, 0usize, 0usize);
    // Highest CONTIGUOUS handled seq. Starts at the cursor so re-offered low-seq
    // events (below it) never rewind the checkpoint; new events above it advance it.
    let mut max_seq = last_seq;
    let mut frozen = false;
    // First pen failure (if any) — surfaced in the loud error.
    let mut pen_refused: Option<String> = None;
    // The seq of the FIRST unacked refused event this cycle (the stream is
    // seq-ascending, so the first is the lowest) — persisted as the new floor.
    let mut pin: Option<i64> = None;

    for (i, hexed) in resp.events.iter().enumerate() {
        // The serving-node seq for THIS entry (parallel array; length-checked above).
        let seq = resp.seqs[i];
        // Decode the entry and its PARALLEL attestation pair (an older peer, or
        // an un-attested event, yields None — the in-DB door decides what that
        // means). A NON-HEX entry is handled like any other unverifiable frame:
        // the verbatim wire text is quarantined and its slot held on the floor —
        // never a whole-pull abort that would wedge the link on one bad entry
        // (the #110 review's hex-decode finding).
        let decoded: Result<WireEntry, String> = hex::decode(hexed)
            .map_err(|e| format!("event entry is not valid hex: {e}"))
            .and_then(|signed| {
                let att = resp
                    .attestations
                    .get(i)
                    .and_then(|o| o.as_deref())
                    .map(hex::decode)
                    .transpose()
                    .map_err(|e| format!("attestation entry is not valid hex: {e}"))?;
                let akey = resp
                    .attester_keys
                    .get(i)
                    .and_then(|o| o.as_deref())
                    .map(hex::decode)
                    .transpose()
                    .map_err(|e| format!("attester-key entry is not valid hex: {e}"))?;
                Ok((signed, att, akey))
            });

        // (bytes to pen, attestation pair, reason) for a refused entry; None if
        // the entry applied or verifies (freeze case).
        let refused: Option<(WireEntry, String)> = match decoded {
            Err(reason) => Some(((hexed.as_bytes().to_vec(), None, None), reason)),
            Ok((signed_bytes, att, akey)) => {
                event_bytes += signed_bytes.len(); // A5: real clinical-plane payload
                                                   // Open the sidecar-wrapped DEK for THIS slot with our own unwrap
                                                   // secret, if the peer shipped one (ADR-0052). This is INDEPENDENT of
                                                   // event decode: a missing, non-hex, or unopenable DEK just means "no
                                                   // custody" — the door still admits the sealed row structurally, so it
                                                   // is never a reason to drop or freeze the event. The unwrapped DEK is
                                                   // held only for the apply call below (Zeroizing clears it on drop).
                let dek = match (
                    &unwrap_secret,
                    resp.wrapped_deks.get(i).and_then(|o| o.as_deref()),
                ) {
                    (Some(secret), Some(hexed)) => match hex::decode(hexed)
                        .ok()
                        .and_then(|w| cairn_event::seal::unwrap_dek(&w, secret).ok())
                    {
                        Some(d) => Some(d),
                        None => {
                            eprintln!(
                                "pull {peer_name}: sidecar DEK for seq {seq} failed to open — \
                                 admitting the event WITHOUT custody"
                            );
                            None
                        }
                    },
                    _ => None,
                };
                match apply_signed(
                    client,
                    &signed_bytes,
                    att.as_deref(),
                    akey.as_deref(),
                    dek.as_ref().map(|d| &d[..]),
                ) {
                    Ok(new) => {
                        if new {
                            applied += 1;
                        }
                        None
                    }
                    Err(e) => {
                        // Classify: a still-verifiable event that failed to
                        // APPLY must NOT be lost — freeze so it is retried,
                        // never skipped. An unverifiable one goes to the pen.
                        match verify_self_described(&signed_bytes) {
                            Ok(_) => {
                                frozen = true;
                                eprintln!(
                                    "pull {peer_name}: HALTING seq cursor at {max_seq} — a valid \
                                     event failed to apply and must not be skipped: {e}"
                                );
                                None
                            }
                            Err(verr) => Some((
                                (signed_bytes, att, akey),
                                format!("{verr}; apply door said: {e}"),
                            )),
                        }
                    }
                }
            }
        };

        if let Some(((bytes, att, akey), reason)) = refused {
            match quarantine_event(
                client,
                peer_name,
                &bytes,
                att.as_deref(),
                akey.as_deref(),
                seq,
                &reason,
            ) {
                Ok(true) => {
                    // A human licensed this exact exclusion (`acked`): no floor
                    // pin, no loud failure — a recorded, attributable decision.
                    skipped_acked += 1;
                }
                Ok(false) => {
                    skipped_unverifiable += 1;
                    if pin.is_none() {
                        pin = Some(seq);
                    }
                    eprintln!(
                        "pull {peer_name}: unverifiable event quarantined durably \
                         (sync_quarantine), slot held on the re-offer floor at seq {seq}: {reason}"
                    );
                }
                Err(qe) => {
                    frozen = true;
                    eprintln!(
                        "pull {peer_name}: HALTING seq cursor at {max_seq} — an unverifiable \
                         event could not be quarantined, so it must not be skipped: {qe}; \
                         reason: {reason}"
                    );
                    pen_refused.get_or_insert(qe.to_string());
                }
            }
        }

        // Advance over the contiguous HANDLED prefix (applied / penned / acked); a
        // freeze stops the advance below its seq. Relies on serve's ascending
        // `ORDER BY seq` so max_seq tracks the contiguous handled prefix.
        if !frozen && seq > max_seq {
            max_seq = seq;
        }
    }

    // Persist progress FIRST — even a loudly-failing cycle keeps what it
    // legitimately gained (applied events, advanced cursor). The floor (same
    // 3-branch discipline as the HLC version, re-keyed to seq):
    //   * CLEAN cycle (no unacked refusals AND no pen failures) → clear: the
    //     whole suffix from the fetch point was admitted or human-acked, so
    //     nothing is being withheld any more;
    //   * unacked refusals, pen healthy → pin at the first refused slot's seq
    //     (everything below it applied or was acked this cycle, so raising an
    //     older floor to the new pin is safe and shrinks re-shipping);
    //   * ANY pen failure → this cycle's view is unreliable: a re-offered slot
    //     whose pen write failed produced NO pin (skips stayed 0), so blindly
    //     overwriting would CLEAR a floor guarding a slot the cursor is already
    //     above — permanent exclusion. Keep the most conservative of
    //     (existing floor, new pin).
    let new_floor: Option<i64> = if skipped_unverifiable == 0 && pen_refused.is_none() {
        None
    } else if pen_refused.is_none() {
        pin
    } else {
        match (floor_seq, pin) {
            (Some(f), Some(p)) => Some(f.min(p)),
            (Some(f), None) => Some(f),
            (None, p) => p,
        }
    };
    // Advance-only cursor (GREATEST) + the recomputed seq floor. A re-offer cycle
    // whose max_seq did not exceed the committed cursor therefore never rewinds it.
    client.execute(
        "UPDATE sync_state
            SET last_seq = GREATEST(last_seq, $2), last_pull_at = clock_timestamp(),
                quarantine_floor_seq = $3
          WHERE peer = $1",
        &[&peer_name, &max_seq, &new_floor],
    )?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    let metrics = serde_json::json!({
        "op": "pull", "peer": peer_name,
        "shipped": resp.events.len(), "applied_new": applied,
        "skipped_unverifiable": skipped_unverifiable,
        "skipped_acked": skipped_acked,
        "watermark_frozen": frozen,
        "floor_active": new_floor.is_some(),
        "event_bytes": event_bytes, "wire_bytes": wire_bytes,
        "bytes_per_event": if resp.events.is_empty() { 0.0 }
                           else { event_bytes as f64 / resp.events.len() as f64 },
        "elapsed_ms": elapsed_ms,
        "cursor_seq": max_seq, "full_sweep": full_sweep
    });

    // LOUD failure (issue #108, generalised by the #110 review): ANY unacked
    // refusal — not just an all-unverifiable batch — fails the pull, every
    // cycle, until the peer is fixed or a human acks the exclusion. The old
    // `skipped == len` heuristic structurally missed the two common cases (a
    // mixed legacy+new batch, and an already-synced link whose boundary event
    // re-applies idempotently), which is exactly where the silent livelock
    // lived. `run` logs this as an integrity condition, not a partition.
    if skipped_unverifiable > 0 || pen_refused.is_some() {
        let all = !resp.events.is_empty() && skipped_unverifiable == resp.events.len();
        let diagnosis = if all {
            let declared = match &resp.signing_context {
                Some(ctx) => format!("declares signing context '{ctx}'"),
                None => "declares no signing context (a pre-ADR-0040 build would not)".to_string(),
            };
            format!(
                " ALL {} shipped events are unverifiable and the peer {declared} — it \
                 appears to serve pre-ADR-0040 (or corrupt) signatures; re-initialize/\
                 re-sign the peer, or if THIS node was at fault run `cairn-sync requeue` \
                 after fixing it.",
                resp.events.len()
            )
        } else {
            String::new()
        };
        let pen = match &pen_refused {
            Some(qe) => format!(" Quarantine pen refused (cursor frozen): {qe}"),
            None => String::new(),
        };
        return Err(Box::new(PullIntegrityError {
            message: format!(
                "pull {peer_name}: {skipped_unverifiable} unverifiable event(s) this cycle; \
                 each is preserved verbatim in sync_quarantine and its slot is held on the \
                 re-offer floor (nothing lost; valid events still applied).{diagnosis}{pen} \
                 Inspect with `cairn-sync quarantine`; a repaired/re-signed peer is picked \
                 up automatically; to accept a permanent exclusion, ack the row: \
                 UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = …"
            ),
            metrics,
        }));
    }

    Ok(metrics)
}

/// Re-process every quarantined event through the real apply door (issue #108's
/// "inspectable and re-processable"): an event that now verifies — e.g. it was
/// falsely rejected by a version-skewed daemon binary since upgraded — is applied
/// and its row cleared; one that still fails stays held, with the door's CURRENT
/// refusal recorded in `last_requeue_error` — never overwriting `reason`, the
/// verify-time forensics (a transient DB hiccup during requeue must not destroy
/// the skew-vs-tampering diagnosis; #110 review finding 5). Never a raw INSERT:
/// release goes through `apply_remote_event` (db/020), so requeue can only ever
/// ADMIT what the floor admits.
fn do_requeue(client: &mut postgres::Client) -> R<serde_json::Value> {
    // Digests only up front; each row's (possibly large) bytes are fetched one
    // at a time inside the loop, so a pen holding a whole legacy history cannot
    // OOM the recovery path (#110 review: the pen is unbounded-ish by design —
    // quota-capped — and requeue is exactly when it is fullest).
    let digests: Vec<Vec<u8>> = client
        .query(
            "SELECT content_digest FROM sync_quarantine ORDER BY first_seen",
            &[],
        )?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let (mut released, mut still_quarantined) = (0usize, 0usize);
    for digest in &digests {
        // The row can vanish between the listing and here only via operator
        // DELETE — skip it silently in that case.
        let Some(row) = client.query_opt(
            "SELECT signed_bytes, attestation, attester_key
             FROM sync_quarantine WHERE content_digest=$1",
            &[digest],
        )?
        else {
            continue;
        };
        let signed: Vec<u8> = row.get(0);
        let att: Option<Vec<u8>> = row.get(1);
        let akey: Option<Vec<u8>> = row.get(2);
        // No sidecar DEK on the requeue path: the quarantine pen holds only the
        // refused signed bytes + attestation pair, never custody. A re-queued sealed
        // event is admitted structurally without custody; its DEK rides a later
        // normal pull once the peer serves it. Pass None (ADR-0052).
        match apply_signed(client, &signed, att.as_deref(), akey.as_deref(), None) {
            Ok(_) => {
                client.execute(
                    "DELETE FROM sync_quarantine WHERE content_digest=$1",
                    &[&digest],
                )?;
                released += 1;
                eprintln!(
                    "requeue: released {} through the apply door",
                    hex_prefix(digest)
                );
            }
            Err(e) => {
                // Still refused: keep the row and record the door's CURRENT
                // rejection beside (never over) the original reason.
                client.execute(
                    "UPDATE sync_quarantine
                     SET last_seen = clock_timestamp(),
                         last_requeue_at = clock_timestamp(),
                         last_requeue_error = $2
                     WHERE content_digest = $1",
                    &[&digest, &e.to_string()],
                )?;
                still_quarantined += 1;
                eprintln!("requeue: {} still refused: {e}", hex_prefix(digest));
            }
        }
    }
    Ok(serde_json::json!({
        "op": "requeue",
        "examined": digests.len(),
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

/// Connect AND verify the schema is current enough for the quarantine machinery
/// (#110 review finding 4: only `init` applies migrations, so an upgraded binary
/// against a pre-#108 database would otherwise limp into a freeze-livelock at
/// the first refused frame, with only stderr as evidence). Fail fast and
/// legibly instead.
fn connect_checked(conn: &str) -> R<postgres::Client> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    // Probe the NEWEST piece of the schema this binary needs — the db/036 seq
    // cursor (event_log.seq + sync_quarantine.refused_seq) — not just table
    // existence: a DB created by an earlier revision would pass a bare to_regclass
    // check and then fail at runtime (the do_pull cursor reads these columns).
    let ok: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                            WHERE table_name='event_log'
                              AND column_name='seq')
                AND EXISTS (SELECT 1 FROM information_schema.columns
                            WHERE table_name='sync_quarantine'
                              AND column_name='refused_seq')",
            &[],
        )?
        .get(0);
    if !ok {
        return Err(
            "this database predates the clinical seq-cursor schema (db/036) this binary \
             requires — run `cairn-sync init --conn <same URI>` (idempotent) to apply \
             the migrations, then retry"
                .into(),
        );
    }
    Ok(client)
}

/// `connect_checked` PLUS the loaded-`cairn_pgx` version floor — for the commands whose
/// writes NEED a current `cairn_pgx`: the ones that APPLY events (`pull`/`run`/`requeue`,
/// where a stale verify library is a silent write outage on every apply — issue #109)
/// and, since db/026, the ones that WRITE blobs (`put-blob`/`gen-blob`/`blobd`, whose
/// present-flips fire the `cairn_blob_verify` trigger and would otherwise die with the
/// illegible `undefined function` this gate exists to prevent). Deliberately NOT used by
/// read-only `quarantine`: the version gate must not block an operator listing the pen
/// during exactly that outage (issue #109 review — a pure SELECT over sync_quarantine
/// needs no `cairn_pgx`).
fn connect_checked_apply(conn: &str) -> R<postgres::Client> {
    let mut client = connect_checked(conn)?;
    assert_pgx_floor(&mut client)?;
    Ok(client)
}

fn cmd_requeue(conn: &str, metrics: bool) -> R<()> {
    // Requeue re-applies through the in-DB door, so it needs a current cairn_pgx.
    let mut client = connect_checked_apply(conn)?;
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

/// One JSON value per quarantine row (oldest first) — the queryable core of
/// `cmd_quarantine`, split out so the DB-gated tests can drive the exact
/// listing an operator sees.
fn quarantine_listing(client: &mut postgres::Client) -> R<Vec<serde_json::Value>> {
    let rows = client.query(
        "SELECT encode(content_digest,'hex'), peer, reason, octet_length(signed_bytes),
                first_seen::text, last_seen::text, seen_count,
                last_requeue_error, last_requeue_at::text, acked
         FROM sync_quarantine ORDER BY first_seen",
        &[],
    )?;
    Ok(rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "digest": r.get::<_, String>(0),
                "peer": r.get::<_, String>(1),
                "reason": r.get::<_, String>(2),
                "bytes": r.get::<_, i32>(3),
                "first_seen": r.get::<_, String>(4),
                "last_seen": r.get::<_, String>(5),
                "seen_count": r.get::<_, i32>(6),
                "last_requeue_error": r.get::<_, Option<String>>(7),
                "last_requeue_at": r.get::<_, Option<String>>(8),
                "acked": r.get::<_, bool>(9)
            })
        })
        .collect())
}

/// List the quarantine (one JSON line per row, oldest first) so an operator can
/// see exactly which events a link refused, from which peer, and why — without
/// psql.
fn cmd_quarantine(conn: &str) -> R<()> {
    // Read-only pen listing: plain connect_checked (schema probe only), NOT the pgx-version
    // gate — an operator must be able to inspect the pen during a stale-cairn_pgx outage
    // (issue #109 review). This is a pure SELECT over sync_quarantine; it calls no cairn_pgx.
    let mut client = connect_checked(conn)?;
    let listing = quarantine_listing(&mut client)?;
    for row in &listing {
        println!("{row}");
    }
    eprintln!("{} event(s) in quarantine", listing.len());
    Ok(())
}

fn cmd_pull(
    conn: &str,
    peer: &str,
    peer_name: &str,
    metrics: bool,
    full: bool,
    key_path: &str,
) -> R<()> {
    let mut client = connect_checked_apply(conn)?;
    // Load this node's signing key so the pull presents an unwrap cert and gains
    // custody of any sealed events it replicates (ADR-0052 custody sidecar).
    let (sk, _kid) = load_or_create_key(key_path)?;
    // A manual one-shot pull defaults to incremental; `--full` requests a sweep
    // from seq 0 (an explicit "reconcile everything now", the same path cmd_run
    // takes on cadence — issue #196).
    let m = do_pull(&mut client, peer, peer_name, full, Some(&sk))?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "pulled from {peer_name}: {} shipped, {} new, {} skipped-unverifiable, frozen={}",
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
            .query(
                "SELECT chunk_index FROM blob_chunk WHERE blob_address=$1",
                &[&addr],
            )?
            .iter()
            .map(|r| r.get::<_, i32>(0))
            .collect();
        let todo: VecDeque<usize> = (0..n_chunks)
            .filter(|i| !have.contains(&(*i as i32)))
            .collect();

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
                                &Request::BlobSlice {
                                    addr_hex: addr_hex.clone(),
                                    offset,
                                    len,
                                },
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
            .query_one(
                "SELECT count(*) FROM blob_chunk WHERE blob_address=$1",
                &[&addr],
            )?
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
                eprintln!(
                    "fetched blob {} ({} bytes, verified)",
                    &addr_hex[..16],
                    buf.len()
                );
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

fn cmd_blobd(conn: &str, peers: &[String], window: usize, budget_ms: u64, metrics: bool) -> R<()> {
    // Gated connect: the assembly flip (present := TRUE) fires the db/026
    // cairn_blob_verify trigger — a stale `.so` must fail here legibly, matching
    // the process-level gate `run` already gives its in-loop blob thread.
    let mut client = connect_checked_apply(conn)?;
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

/// `own_key` (ADR-0052): this node's signing key, wrapped in an `Arc` so a clone can
/// move into each per-connection thread where `serve_conn` derives the unwrap secret
/// to re-wrap DEKs. `None` serves without custody (events still sync).
fn cmd_serve(conn: String, listen: &str, corrupt: bool, own_key: Option<Arc<SigningKey>>) -> R<()> {
    let listener = TcpListener::bind(listen)?;
    eprintln!(
        "serving on {listen}{}",
        if corrupt {
            " (CORRUPT: test fault injection)"
        } else {
            ""
        }
    );
    for stream in listener.incoming() {
        let stream = stream?;
        let conn = conn.clone();
        let own_key = own_key.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_conn(&conn, stream, corrupt, own_key) {
                eprintln!("connection error: {e}");
            }
        });
    }
    Ok(())
}

/// Format the byte-tier thread's per-pass failure line (issue #202). Pure so the
/// log contract is unit-testable: the line must name the subsystem, carry the
/// underlying cause, and say the loop retries.
fn blobd_error_line(e: &dyn Error) -> String {
    format!("blobd pass failed (will retry next interval): {e}")
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
    key_path: &str,
) -> R<()> {
    // Load the node signing key ONCE up front (ADR-0052): the serve thread and the
    // pull loop must share the SAME key — deriving it twice would race to create the
    // file and could leave serve and pull on different identities. One Arc feeds both.
    let node_key = Arc::new(load_or_create_key(key_path)?.0);
    {
        let (c, l) = (conn.to_string(), listen.to_string());
        let own_key = Arc::clone(&node_key);
        std::thread::spawn(move || {
            if let Err(e) = cmd_serve(c, &l, false, Some(own_key)) {
                eprintln!("serve thread exited: {e}");
            }
        });
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    let mut client = connect_checked_apply(conn)?;
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
        let peers = if blob_peers.is_empty() {
            vec![peer.to_string()]
        } else {
            blob_peers.clone()
        };
        let counter = Arc::clone(&blobs_fetched);
        std::thread::spawn(
            move || match postgres::Client::connect(&conn, postgres::NoTls) {
                Ok(mut bclient) => loop {
                    match do_blobd(&mut bclient, &conn, &peers, window, budget_ms) {
                        Ok(m) => {
                            counter.fetch_add(
                                m["blobs_completed"].as_u64().unwrap_or(0),
                                Ordering::Relaxed,
                            );
                        }
                        // Never fatal (the next pass retries) but never SILENT either
                        // (issue #202): without this line a permanently failing pass —
                        // bad conn string after a DB restart, schema skew — is
                        // indistinguishable from "no blobs to fetch" for the life of
                        // the process. Everything else in the run loop logs; so does this.
                        Err(e) => eprintln!("{}", blobd_error_line(e.as_ref())),
                    };
                    std::thread::sleep(Duration::from_millis(interval_ms));
                },
                Err(e) => eprintln!("blob thread could not connect: {e}"),
            },
        );
    }

    let start = Instant::now();
    let mut cycle: u64 = 0;
    loop {
        cycle += 1;
        let mut line = serde_json::json!({ "ts": now_ms(), "cycle": cycle });
        let mut status = format!("cycle {cycle}");

        // Full sweep on cadence (issue #196): incremental each cycle, a full sweep
        // (after_seq = 0) every FULL_SWEEP_EVERY cycles as the correctness floor for
        // the residual BIGSERIAL out-of-order-commit gap. `% == 0` (not
        // is_multiple_of, stabilized only in Rust 1.87) keeps within the MSRV 1.74.
        #[allow(clippy::manual_is_multiple_of)]
        let full_sweep = cycle % FULL_SWEEP_EVERY == 0;
        match do_pull(
            &mut client,
            peer,
            peer_name,
            full_sweep,
            Some(node_key.as_ref()),
        ) {
            Ok(m) => {
                status += &format!(": pull {} shipped / {} new", m["shipped"], m["applied_new"]);
                line["pull"] = m;
            }
            Err(e) => {
                // Two loud failure classes, kept DISTINCT in the machine-readable
                // log (the bet_a harness counts `partition` as link downtime —
                // #110 review finding 6):
                //   * integrity (unverifiable events / skew / pen refusal): the
                //     peer answered; the DATA is the problem. The per-cycle
                //     metrics still exist and are logged.
                //   * anything else (retries exhausted) = a partition.
                status += &format!(": PULL FAILED: {e}");
                line["pull_error"] = serde_json::json!(e.to_string());
                match e.downcast_ref::<PullIntegrityError>() {
                    Some(ie) => {
                        line["integrity"] = serde_json::json!(true);
                        if !ie.metrics.is_null() {
                            line["pull"] = ie.metrics.clone();
                        }
                    }
                    None => {
                        line["partition"] = serde_json::json!(true);
                    }
                }
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

/// Re-wrap each served event's DEK for the pulling peer — the custody half of the
/// clinical wire (ADR-0052). Pure so the wire contract is unit-testable.
///
/// This node stores each sealed event's DEK wrapped for its OWN unwrap key
/// (`event_dek.dek_wrapped`, hex-encoded into `local_deks[i]` by the serve SQL). A
/// DEK is symmetric, so it can never be shipped as-is: we UNWRAP it with our own
/// secret, then RE-WRAP it for the requester's X25519 public key (from its verified
/// cert). The requester unwraps with ITS secret and hands the DEK to the apply door.
/// Custody thus **follows admission** — it does not widen WHICH events a peer
/// replicates — but the DEK is precisely what makes a sealed body READABLE, so handing
/// it over confers clinical-data read access (it populates event_clear / opens the
/// plaintext), not merely a later crypto-shred capability. What currently gates who may
/// obtain it is the trust-note at the serve site (transport boundary only, for now).
///
/// `local_deks[i]` is None whenever no custody must travel: the event is unsealed,
/// this node holds no DEK for it, OR it has been SHREDDED here (the serve SQL nulls a
/// shredded row's DEK — the wire-level half of the shred guarantee: a shredded event
/// NEVER re-emits its key). A per-slot re-wrap failure degrades that one slot to None
/// (no custody for it), never the whole batch. With no requester key or no local
/// unwrap secret, every slot is None — the events still sync, custody just does not.
fn rewrap_custody_for_peer(
    local_deks: &[Option<String>],
    requester_pub: Option<&[u8; 32]>,
    own_secret: Option<&[u8; 32]>,
) -> Vec<Option<String>> {
    let (Some(requester_pub), Some(own_secret)) = (requester_pub, own_secret) else {
        return vec![None; local_deks.len()];
    };
    local_deks
        .iter()
        .enumerate()
        .map(|(i, slot)| {
            // A None slot means no custody must travel (unsealed / no local DEK / shredded):
            // that is normal, so stay silent.
            let hexed = slot.as_deref()?;
            // A PRESENT local DEK that fails to re-wrap, though, is an operator-visible
            // degradation — the event still syncs but its custody silently vanishes, blanking
            // the sealed projection on every puller. The usual cause is a serve `--key` whose
            // derived unwrap secret does not match the registered node_unwrap_key (a misconfig,
            // or an un-re-wrapped key rotation). Log it rather than fail silently — mirrors the
            // pull-side "sidecar DEK failed to open — admitting WITHOUT custody" line.
            let rewrap = || -> Option<String> {
                let local = hex::decode(hexed).ok()?;
                let dek = cairn_event::seal::unwrap_dek(&local, own_secret).ok()?;
                let rewrapped = cairn_event::seal::wrap_dek_for(&dek, requester_pub).ok()?;
                Some(hex::encode(rewrapped))
            };
            if let Some(s) = rewrap() {
                Some(s)
            } else {
                eprintln!(
                    "cairn-sync serve: DEK re-wrap failed for custody slot {i} — serving this \
                     event WITHOUT custody. Check the serve --key matches the node's registered \
                     unwrap key (a mismatched key or un-re-wrapped rotation blanks sealed \
                     projections on every puller)."
                );
                None
            }
        })
        .collect()
}

/// `own_key` (ADR-0052): this node's signing key, from which we derive the unwrap
/// SECRET used to open our locally-stored DEKs before re-wrapping them for a pulling
/// peer. `None` serves without custody (events still sync). One `Arc` is cloned into
/// each per-connection thread; the secret is derived per connection (a cheap HKDF).
fn serve_conn(
    conn: &str,
    mut stream: TcpStream,
    corrupt: bool,
    own_key: Option<Arc<SigningKey>>,
) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    // Our own unwrap secret, derived once per connection from this node's signing
    // seed (ADR-0026 escrow: the secret is never in the DB). Used only to open our
    // locally-stored DEKs so the EventsAfterSeq arm can re-wrap them for the peer.
    let own_secret = own_key
        .as_ref()
        .map(|k| cairn_event::seal::derive_unwrap_secret(&k.to_bytes()));
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
                // Legacy HLC arm ships no per-event seq: an old-style puller ignores
                // it, and a new puller never sends EventsAfter (it uses EventsAfterSeq).
                seqs: vec![],
                // Declare the context we mint under (issue #108) so a skewed
                // puller can refuse the batch deterministically and legibly.
                signing_context: Some(CTX_EVENT.as_str().to_string()),
                // Legacy HLC arm carries NO custody sidecar (ADR-0052): it cannot
                // receive an unwrap cert, so it never re-wraps DEKs. Empty = no
                // custody; sealed events still sync (admitted structurally).
                wrapped_deks: vec![],
            })?
        }
        Request::EventsAfterSeq {
            after_seq,
            unwrap_cert,
        } => {
            // Serve LOCAL insertion order (seq), STRICTLY above the puller's cursor
            // (issue #196). The seq prefix is transport metadata; signed_bytes are
            // the untouched signed core (principle 12). `after_seq = 0` = full sweep.
            // ORDER BY seq is load-bearing: the puller freezes its cursor at the
            // contiguous handled prefix and relies on strictly-ascending arrival.
            // Unpaginated (issue #101): the whole suffix — the whole LOG on a sweep —
            // ships in one frame; see the FULL_SWEEP_EVERY note for the known cost.
            // The 5th column is THIS node's own wrapped DEK for the event, hex-encoded
            // — but ONLY when the event has NOT been shredded here (ADR-0052). The
            // LEFT JOIN to erasure_shred_log + `CASE WHEN s.target_event_id IS NULL`
            // is the WIRE-LEVEL half of the shred guarantee: a shredded event NEVER
            // ships its DEK, so custody can never be reconstituted from a peer's serve
            // after a local crypto-shred. A non-sealed event (or one this node holds
            // no custody for) has no event_dek row, so dek_hex is NULL there too.
            let rows = client.query(
                "SELECT e.seq,
                        encode(e.signed_bytes,'hex'),
                        encode(e.attestation,'hex'),
                        encode(e.attester_key,'hex'),
                        CASE WHEN s.target_event_id IS NULL
                             THEN encode(d.dek_wrapped,'hex') END AS dek_hex
                 FROM event_log e
                 LEFT JOIN event_dek d          ON d.event_id = e.event_id
                 LEFT JOIN erasure_shred_log s  ON s.target_event_id = e.event_id
                 WHERE e.seq > $1
                 ORDER BY e.seq",
                &[&after_seq],
            )?;
            let seqs = rows.iter().map(|r| r.get::<_, i64>(0)).collect();
            let events = rows.iter().map(|r| r.get::<_, String>(1)).collect();
            let attestations = rows.iter().map(|r| r.get::<_, Option<String>>(2)).collect();
            let attester_keys = rows.iter().map(|r| r.get::<_, Option<String>>(3)).collect();
            // Each event's DEK still wrapped for OUR key (None = no custody / shredded).
            let local_deks: Vec<Option<String>> =
                rows.iter().map(|r| r.get::<_, Option<String>>(4)).collect();

            // Verify the pulling peer's unwrap cert (ADR-0052): signature + kid
            // self-binding only. `verify_unwrap_key_cert` confirms the cert's kid
            // signed it, so `requester_pub` provably belongs to that identity.
            //
            // TODO(follow-up, filed in Task 14): pin this kid against the node-plane
            // trust set. The skeleton confirms the cert is internally consistent (the
            // kid signed it, so `requester_pub` provably belongs to that identity) but
            // does NOT yet check the kid is a TRUSTED peer. Be honest about what that
            // leaves standing: until cert-kid trust-set pinning lands, the transport
            // boundary (WireGuard / mTLS on the serve port) is the SOLE access control
            // on who obtains sealed-body custody. ANY self-signed unwrap cert that
            // reaches this port has its DEKs re-wrapped and thereby obtains READ-custody
            // of every non-shredded sealed body this node serves — the DEK is what
            // populates event_clear and opens the sealed plaintext, so custody confers
            // clinical-data READ, not merely a future shred capability. This is the
            // sanctioned ADR-0052 erasability-by-default skeleton (custody is designed
            // to follow admission), but its current floor honestly stated is "the link
            // is the access control"; trust-set pinning is the named hardening that
            // makes admission the boundary in the DB too, not just on the wire. An
            // absent or invalid cert simply yields no custody (below), never a refused
            // pull.
            let requester_pub = unwrap_cert.as_deref().and_then(|hexed| {
                hex::decode(hexed)
                    .ok()
                    .and_then(|c| cairn_event::verify_unwrap_key_cert(&c).ok())
                    .map(|(_kid, pubk)| pubk)
            });
            let wrapped_deks =
                rewrap_custody_for_peer(&local_deks, requester_pub.as_ref(), own_secret.as_deref());
            serde_json::to_vec(&EventsResponse {
                events,
                attestations,
                attester_keys,
                seqs,
                signing_context: Some(CTX_EVENT.as_str().to_string()),
                wrapped_deks,
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
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics] [--full] [--key PATH]
              (--key: this node's key; presents an unwrap cert so sealed events arrive with shred custody — ADR-0052)
  quarantine  --conn URI    (list refused events: digest, peer, reason, requeue error, acked)
  requeue     --conn URI [--metrics]
              (re-process quarantined events through the apply door after fixing the cause)
  blobd       --conn URI (--peer HOST:PORT | --blob-peer HOST:PORT ...) [--window N] [--budget-ms N] [--metrics]
  serve       --conn URI --listen HOST:PORT [--corrupt] [--key PATH]
              (--key: this node's key; re-wraps sealed events' DEKs for pulling peers — ADR-0052)
  fingerprint --conn URI    (convergence/honest-state JSON for the harness)
  run         --conn URI --listen HOST:PORT --peer HOST:PORT --peer-name NAME
              [--blob-peer HOST:PORT ...] [--window N] [--interval-ms N] [--budget-ms N] [--log PATH] [--duration-s N] [--key PATH]
              (unattended: serve+pull+blob, logs one JSON line/cycle, survives drops; --key enables custody both ways — ADR-0052)
  bench-insert --conn URI --node NAME --key PATH [--count N]   (Bet B B1: maintained-write latency)
  chart       --conn URI --patient UUID                        (Bet B B2: chart-read latency)
  bench       [--hash-mb N] [--sig-iters N] [--dek-iters N]    (Bet B B3/B4: crypto throughput, no DB)
  bench-seal  [--iters N]                                      (ADR-0052: seal/wrap/unwrap/unseal ns/op, no DB)
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
            flag(&args, "--patients")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10),
            flag(&args, "--count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100),
            flag(&args, "--rate")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0),
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
            flag(&args, "--count")
                .and_then(|s| s.parse().ok())
                .unwrap_or(200),
        )?,
        "chart" => cmd_chart(&need(conn), &need(flag(&args, "--patient")))?,
        "bench" => cmd_bench(
            flag(&args, "--hash-mb")
                .and_then(|s| s.parse().ok())
                .unwrap_or(256),
            flag(&args, "--sig-iters")
                .and_then(|s| s.parse().ok())
                .unwrap_or(20000),
            flag(&args, "--dek-iters")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100000),
        )?,
        "bench-seal" => cmd_bench_seal(
            flag(&args, "--iters")
                .and_then(|s| s.parse().ok())
                .unwrap_or(10000),
        )?,
        "pull" => cmd_pull(
            &need(conn),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            args.iter().any(|a| a == "--metrics"),
            args.iter().any(|a| a == "--full"),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
        )?,
        "quarantine" => cmd_quarantine(&need(conn))?,
        "requeue" => cmd_requeue(&need(conn), args.iter().any(|a| a == "--metrics"))?,
        "gen-blob" => cmd_gen_blob(
            &need(conn),
            flag(&args, "--size-mb")
                .and_then(|s| s.parse().ok())
                .unwrap_or(8),
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
                flag(&args, "--window")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(4),
                flag(&args, "--budget-ms")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20),
                args.iter().any(|a| a == "--metrics"),
            )?
        }
        "serve" => {
            // Load this node's key so the serve arm can re-wrap sealed events' DEKs
            // for pulling peers (ADR-0052). Defaults to node.key like the other verbs.
            let own_key = Arc::new(
                load_or_create_key(&flag(&args, "--key").unwrap_or_else(|| "node.key".into()))?.0,
            );
            cmd_serve(
                need(conn),
                &need(flag(&args, "--listen")),
                args.iter().any(|a| a == "--corrupt"),
                Some(own_key),
            )?
        }
        "run" => cmd_run(
            &need(conn),
            &need(flag(&args, "--listen")),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            flags(&args, "--blob-peer"),
            flag(&args, "--window")
                .and_then(|s| s.parse().ok())
                .unwrap_or(4),
            flag(&args, "--interval-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(2000),
            flag(&args, "--budget-ms")
                .and_then(|s| s.parse().ok())
                .unwrap_or(20),
            &flag(&args, "--log").unwrap_or_else(|| "cairn-run.jsonl".into()),
            flag(&args, "--duration-s")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
            &flag(&args, "--key").unwrap_or_else(|| "node.key".into()),
        )?,
        "sign-stdin" => {
            cmd_sign_stdin(&flag(&args, "--key").unwrap_or_else(|| "agent.key".into()))?
        }
        "attest-stdin" => {
            cmd_attest_stdin(&flag(&args, "--key").unwrap_or_else(|| "human.key".into()))?
        }
        "key-id" => cmd_key_id(&flag(&args, "--key").unwrap_or_else(|| "agent.key".into()))?,
        _ => usage(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{event_address, generate_key, verify_attestation};

    #[test]
    fn parse_pgx_version_accepts_plain_triples_and_rejects_the_rest() {
        assert_eq!(parse_pgx_version("0.2.0"), Some((0, 2, 0)));
        assert_eq!(parse_pgx_version(" 1.20.3 "), Some((1, 20, 3)));
        // Not exactly three numeric components → unparseable (fail-closed input).
        assert_eq!(parse_pgx_version("0.2"), None);
        assert_eq!(parse_pgx_version("0.2.0.1"), None);
        assert_eq!(parse_pgx_version("0.2.0-rc1"), None);
        assert_eq!(parse_pgx_version("garbage"), None);
        assert_eq!(parse_pgx_version(""), None);
    }

    #[test]
    fn pgx_version_ok_enforces_the_floor_and_fails_closed() {
        // At or above the floor passes; below fails.
        assert!(pgx_version_ok("0.2.0", "0.2.0"), "exact floor is OK");
        assert!(pgx_version_ok("0.2.1", "0.2.0"), "patch above floor is OK");
        assert!(pgx_version_ok("0.3.0", "0.2.0"), "minor above floor is OK");
        assert!(pgx_version_ok("1.0.0", "0.2.0"), "major above floor is OK");
        assert!(
            !pgx_version_ok("0.1.9", "0.2.0"),
            "the pre-ADR-0040 line is refused"
        );
        assert!(
            !pgx_version_ok("0.1.0", "0.2.0"),
            "an older library is refused"
        );
        // Unparseable EITHER side → refused, never silently accepted.
        assert!(!pgx_version_ok("nonsense", "0.2.0"));
        assert!(!pgx_version_ok("0.2.0", "nonsense"));
    }

    #[test]
    fn required_pgx_floor_is_itself_a_valid_triple() {
        // Guards against a typo in the const turning every floor check into a
        // fail-closed refusal of a perfectly good library.
        assert!(parse_pgx_version(REQUIRED_PGX_FLOOR).is_some());
    }

    #[test]
    fn attest_token_hex_is_verifiable_and_address_bound() {
        // The CLI core must produce a token the verifier accepts for the right
        // key+address and rejects for a different address (the binding guarantee).
        let (sk, kid) = generate_key().unwrap();
        let vk = sk.verifying_key();
        let ca = event_address(b"some signed event bytes");
        let input = format!(
            r#"{{"content_address_hex":"{}","attester_key_id":"{}","role":"attested"}}"#,
            hex::encode(&ca),
            kid
        );

        let token_hex = attestation_token_hex(&input, &sk).unwrap();
        let token = hex::decode(&token_hex).unwrap();

        assert!(
            verify_attestation(&token, &ca, &vk),
            "token verifies for right key + address"
        );
        let other = event_address(b"a different event");
        assert!(
            !verify_attestation(&token, &other, &vk),
            "token is bound to its content-address"
        );
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
        // …and for the issue #196 per-event seq array: absent → empty vec.
        assert!(resp.seqs.is_empty(), "missing seqs defaults empty");
    }

    #[test]
    fn events_after_seq_request_round_trips() {
        let req = Request::EventsAfterSeq {
            after_seq: 42,
            unwrap_cert: None,
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        match serde_json::from_slice::<Request>(&bytes).unwrap() {
            Request::EventsAfterSeq {
                after_seq,
                unwrap_cert,
            } => {
                assert_eq!(after_seq, 42);
                assert!(unwrap_cert.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    // ---- issue #202: wire-frame cap + fingerprint collation + byte-tier legibility ----

    #[test]
    fn read_frame_refuses_an_over_cap_length_prefix() {
        // A length prefix is attacker-controlled on BOTH sides of the wire: the
        // server reads request frames from any client that can reach the port
        // (WireGuard is the assumed perimeter, not authentication), and the puller
        // reads response frames from its peer. A hostile/corrupt u32 prefix of up
        // to 4 GiB must be refused BEFORE the read buffer is allocated — as
        // InvalidData with a legible message, never a doomed multi-GiB allocation
        // that surfaces as an opaque UnexpectedEof.
        let mut hostile = std::io::Cursor::new(u32::MAX.to_be_bytes().to_vec());
        let err = read_frame(&mut hostile).expect_err("an over-cap prefix must be refused");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "cap refusal must be InvalidData, got: {err}"
        );
        assert!(
            err.to_string().contains("cap"),
            "the refusal names the cap so an operator can tell it from line noise: {err}"
        );

        // The boundary is exact: one byte over the cap is refused too.
        let over = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let mut s = std::io::Cursor::new(over.to_vec());
        assert_eq!(
            read_frame(&mut s).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn read_frame_round_trips_an_in_cap_frame() {
        // The cap must never break a legitimate exchange: an in-cap frame still
        // round-trips byte-identically through write_frame/read_frame.
        let payload = vec![0xAB_u8; 1024];
        let mut wire = Vec::new();
        write_frame(&mut wire, &payload).unwrap();
        let mut r = std::io::Cursor::new(wire);
        assert_eq!(read_frame(&mut r).unwrap(), payload);
    }

    #[test]
    // The asserts ARE on constants — deliberately: this is a standing bounds guard
    // on MAX_FRAME_BYTES itself (same class as required_pgx_floor_is_itself_a_valid
    // _triple), so a future edit of the const outside the #101-safe window fails a
    // named test instead of silently shipping.
    #[allow(clippy::assertions_on_constants)]
    fn frame_cap_holds_a_realistic_event_batch() {
        // The events response is deliberately UNPAGINATED (issue #101): a full
        // sweep ships the whole log suffix as ONE hex-encoded JSON frame, so the
        // node plane's per-event 8 MiB cap cannot be ported verbatim. The cap must
        // sit far above a realistic harness batch (~1.5 KiB/event, hex doubling →
        // ~3 KiB/event on the wire) while still bounding a hostile 4 GiB prefix.
        // If a deployment's log outgrows the cap, the sweep fails LOUDLY with the
        // cap message — pagination (#101) is the real fix, tracked there.
        assert!(
            MAX_FRAME_BYTES >= 16 * 1024 * 1024,
            "cap must hold a realistic unpaginated batch (issue #101)"
        );
        assert!(
            MAX_FRAME_BYTES <= 256 * 1024 * 1024,
            "cap must still bound a hostile 4 GiB prefix to a refusable size"
        );
    }

    #[test]
    fn fingerprint_orderings_compare_under_collate_c() {
        // ADR-0045 (#69) discipline applied to the convergence PROBE itself
        // (issue #202): both fingerprint hashes aggregate in an order that
        // includes a TEXT sort key (node_origin; patient_id::text). Without
        // COLLATE "C" two honest nodes with different cluster collations report
        // different hashes for IDENTICAL sets — a false divergence alarm in the
        // very tool meant to prove convergence. Standing drift guard, same class
        // as name_winner_order_drift.rs.
        assert!(
            FINGERPRINT_EVENT_HASH_SQL.contains(r#"node_origin COLLATE "C""#),
            "event_hash ORDER BY must pin the TEXT tiebreak to byte order"
        );
        assert!(
            FINGERPRINT_PROJECTION_HASH_SQL.contains(r#"patient_id::text COLLATE "C""#),
            "projection_hash ORDER BY must pin the TEXT key to byte order"
        );
    }

    #[test]
    fn blobd_error_line_is_legible_and_names_the_retry() {
        // #202: the byte-tier thread's failure arm was a bare `Err(_) => 0` — a
        // permanently failing blobd pass (bad conn string after a DB restart,
        // schema skew) was indistinguishable from "no blobs to fetch" for the
        // life of the process. The logged line must carry the underlying error
        // AND say the pass retries, so an operator reading the log can tell a
        // transient blip from a wedge without reading the source.
        let e: Box<dyn Error> = "connection refused".into();
        let line = blobd_error_line(e.as_ref());
        assert!(
            line.contains("blobd"),
            "names the failing subsystem: {line}"
        );
        assert!(
            line.contains("connection refused"),
            "carries the cause: {line}"
        );
        assert!(line.contains("retr"), "says the loop retries: {line}");
    }

    #[test]
    fn write_frame_refuses_an_over_cap_frame() {
        // PR #225 review: the read cap alone is asymmetric — a serving node whose
        // log outgrew MAX_FRAME_BYTES would serialize and SHIP the whole over-cap
        // response, which then fails only at the peer's read cap: the bytes cross
        // the wire for nothing and the serving operator's own log shows no error.
        // Refusing at the source puts the failure next to its cause (and past
        // u32::MAX the length prefix would silently truncate — the write cap makes
        // that unreachable). Nothing may hit the wire before the refusal: a bare
        // length prefix with no body would wedge the reading peer.
        let payload = vec![0u8; MAX_FRAME_BYTES + 1];
        let mut wire = Vec::new();
        let err = write_frame(&mut wire, &payload).expect_err("an over-cap frame must be refused");
        assert_eq!(
            err.kind(),
            std::io::ErrorKind::InvalidData,
            "cap refusal must be InvalidData, got: {err}"
        );
        assert!(
            err.to_string().contains("cap"),
            "the refusal names the cap so the operator can tell it from an I/O fault: {err}"
        );
        assert!(
            wire.is_empty(),
            "nothing may be written before the refusal (a bare prefix would wedge the peer)"
        );

        // The boundary is exact: a frame of exactly MAX_FRAME_BYTES still ships.
        let at_cap = vec![0u8; MAX_FRAME_BYTES];
        let mut wire = Vec::new();
        write_frame(&mut wire, &at_cap).expect("an at-cap frame must still ship");
        assert_eq!(wire.len(), 4 + MAX_FRAME_BYTES);
    }

    #[test]
    fn events_response_seqs_field_is_additive() {
        // A hand-built response WITHOUT `seqs` (an older serve) decodes to empty.
        let legacy = serde_json::json!({
            "events": ["deadbeef"], "attestations": [null], "attester_keys": [null]
        });
        let r: EventsResponse =
            serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(
            r.seqs.is_empty(),
            "missing seqs decodes to empty (serde default)"
        );
        // A response WITH `seqs` round-trips.
        let with = EventsResponse {
            events: vec!["deadbeef".into()],
            attestations: vec![None],
            attester_keys: vec![None],
            seqs: vec![7],
            signing_context: None,
            wrapped_deks: vec![None],
        };
        let back: EventsResponse =
            serde_json::from_slice(&serde_json::to_vec(&with).unwrap()).unwrap();
        assert_eq!(back.seqs, vec![7]);
    }

    // -----------------------------------------------------------------------
    // ADR-0052 custody sidecar — the clinical wire carries a re-wrapped DEK so a
    // pulling peer can gain crypto-shred custody of a sealed event it replicates.
    // Both new fields are ADDITIVE (principle 12 / ADR-0012): an old peer omits
    // them and everything still syncs (sealed rows admit structurally, no custody).
    // -----------------------------------------------------------------------

    #[test]
    fn events_response_wrapped_deks_field_is_additive() {
        // Old responder: no `wrapped_deks` field → empty vec (serde default). The
        // puller then treats every slot as "no custody shipped" (see do_pull's
        // resp.wrapped_deks.get(i)).
        let old = r#"{"events":[],"attestations":[],"attester_keys":[],"seqs":[]}"#;
        let r: EventsResponse = serde_json::from_str(old).unwrap();
        assert!(
            r.wrapped_deks.is_empty(),
            "missing wrapped_deks decodes to empty (serde default)"
        );
    }

    #[test]
    fn events_after_seq_unwrap_cert_is_additive() {
        // Request is INTERNALLY tagged (`#[serde(tag = "op")]`), so an old puller's
        // seq request is `{"op":"EventsAfterSeq","after_seq":0}` with no unwrap_cert.
        // It must decode to None (serde default) — the server then serves without
        // custody rather than refusing the pull.
        let old = r#"{"op":"EventsAfterSeq","after_seq":0}"#;
        let r: Request = serde_json::from_str(old).unwrap();
        match r {
            Request::EventsAfterSeq { unwrap_cert, .. } => assert!(
                unwrap_cert.is_none(),
                "missing unwrap_cert decodes to None (serde default)"
            ),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn unwrap_key_cert_round_trip_binds_kid() {
        // The cert the puller presents must bind its X25519 unwrap public key to its
        // Ed25519 identity (the kid): the server re-wraps DEKs for `xpub`, trusting
        // that only the holder of the matching signing key controls it.
        let (sk, _kid) = cairn_event::generate_key().unwrap();
        let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
        let xpub = cairn_event::seal::unwrap_public(&secret);
        let cert = cairn_event::sign_unwrap_key_cert(&sk, &xpub).unwrap();
        let (kid, got) = cairn_event::verify_unwrap_key_cert(&cert).unwrap();
        assert_eq!(kid, hex::encode(sk.verifying_key().to_bytes()));
        assert_eq!(got, xpub);
    }

    // ---------------------------------------------------------------------
    // rewrap_custody_for_peer — the load-bearing custody re-wrap on the serve
    // path (ADR-0052). Pure (no DB), so unit-testable directly. ALL key material
    // is DERIVED at runtime (house rule 6): a literal 32-byte array in a crypto
    // context trips CodeQL's hard-coded-cryptographic-value query (issue #146).
    // ---------------------------------------------------------------------

    /// Deterministic-but-computed 32-byte fixture. `tag` distinguishes each
    /// role/secret so no two fixtures collide, and nothing is a byte literal.
    fn derived_bytes(tag: u8) -> [u8; 32] {
        std::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(tag))
    }

    #[test]
    fn rewrap_custody_round_trips_to_the_requesters_secret() {
        // (a) A slot re-wrapped for the requester must open with the REQUESTER's
        //     secret and yield the ORIGINAL dek — the whole contract: custody
        //     travels to the peer, readable only by the peer.
        let own_secret = cairn_event::seal::derive_unwrap_secret(&derived_bytes(0x11));
        let own_pub = cairn_event::seal::unwrap_public(&own_secret);
        let peer_secret = cairn_event::seal::derive_unwrap_secret(&derived_bytes(0x22));
        let peer_pub = cairn_event::seal::unwrap_public(&peer_secret);
        let dek = derived_bytes(0x33);

        // This node stores the DEK wrapped for its OWN unwrap key (the serve SQL
        // hands rewrap_custody_for_peer exactly this hex string).
        let local_wrapped = cairn_event::seal::wrap_dek_for(&dek, &own_pub).unwrap();
        let local_deks = vec![Some(hex::encode(local_wrapped))];

        let out = rewrap_custody_for_peer(&local_deks, Some(&peer_pub), Some(&*own_secret));
        assert_eq!(out.len(), 1);
        let rewrapped = hex::decode(out[0].as_ref().expect("slot was re-wrapped")).unwrap();

        // Opens with the PEER's secret and recovers the original DEK …
        let recovered = cairn_event::seal::unwrap_dek(&rewrapped, &peer_secret).unwrap();
        assert_eq!(&*recovered, &dek, "requester recovers the original DEK");
        // … and is NOT re-openable by the serving node (the wrap is bound to the peer).
        assert!(
            cairn_event::seal::unwrap_dek(&rewrapped, &own_secret).is_err(),
            "the re-wrap is bound to the requester, not the server"
        );
    }

    #[test]
    fn rewrap_custody_leaves_a_none_slot_none() {
        // (b) A None local slot (unsealed event / no custody here / shredded) stays
        //     None — no DEK is fabricated for a slot that never carried one.
        let own_secret = cairn_event::seal::derive_unwrap_secret(&derived_bytes(0x44));
        let peer_pub = cairn_event::seal::unwrap_public(&cairn_event::seal::derive_unwrap_secret(
            &derived_bytes(0x55),
        ));

        let out = rewrap_custody_for_peer(&[None], Some(&peer_pub), Some(&*own_secret));
        assert_eq!(out, vec![None], "a None local slot ships no custody");
    }

    #[test]
    fn rewrap_custody_ships_nothing_without_a_requester_key() {
        // (c) No requester public key (absent / invalid cert → None) means EVERY slot
        //     is None: sealed events still sync, custody simply does not travel. Even a
        //     populated local slot must not leak when there is no recipient to bind to.
        let own_secret = cairn_event::seal::derive_unwrap_secret(&derived_bytes(0x66));
        let own_pub = cairn_event::seal::unwrap_public(&own_secret);
        let local_wrapped =
            cairn_event::seal::wrap_dek_for(&derived_bytes(0x77), &own_pub).unwrap();
        let local_deks = vec![Some(hex::encode(local_wrapped)), None];

        let out = rewrap_custody_for_peer(&local_deks, None, Some(&*own_secret));
        assert_eq!(
            out,
            vec![None, None],
            "no requester key means all-None custody"
        );
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

    // pub(super): shared with the sibling `fingerprint_db_tests` module (same file).
    pub(super) fn cs() -> Option<String> {
        std::env::var("CAIRN_TEST_PG").ok()
    }

    /// A realistic HLC wall (≈2026) so ceiling checks compare against a sane instant.
    const WALL_2026: i64 = 1_782_000_000_000;

    /// Connect + take the cluster-wide test advisory lock (same 'CARN' key every
    /// DB-gated suite uses), then (re)apply the schema and reset the tables this
    /// suite touches. The returned client HOLDS the lock until dropped.
    /// pub(super): shared with the sibling `fingerprint_db_tests` module.
    pub(super) fn locked_client(base: &str) -> postgres::Client {
        let mut c = postgres::Client::connect(base, postgres::NoTls).unwrap();
        c.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();
        c.batch_execute("CREATE EXTENSION IF NOT EXISTS cairn_pgx;")
            .unwrap();
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
            hlc: Hlc {
                wall,
                counter: 0,
                node_origin: "peer-src".into(),
            },
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
                let Ok((mut s, _)) = listener.accept() else {
                    break;
                };
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
            // Canned serve = events already in seq order; assign 1-based seqs so the
            // puller has a per-event cursor to checkpoint/pen on (issue #196).
            seqs: (1..=events.len() as i64).collect(),
            signing_context: signing_context.map(str::to_string),
            // These quarantine/cursor tests don't exercise custody; ship no DEKs.
            wrapped_deks: vec![None; events.len()],
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
        .map(|r| QRow {
            peer: r.get(0),
            reason: r.get(1),
            seen_count: r.get(2),
        })
        .collect()
    }

    /// The per-peer seq cursor (issue #196): sync_state.last_seq.
    fn cursor(c: &mut postgres::Client, peer: &str) -> i64 {
        c.query_one("SELECT last_seq FROM sync_state WHERE peer=$1", &[&peer])
            .unwrap()
            .get(0)
    }

    /// The per-peer seq re-offer floor (NULL = no unresolved quarantine).
    fn floor(c: &mut postgres::Client, peer: &str) -> Option<i64> {
        c.query_one(
            "SELECT quarantine_floor_seq FROM sync_state WHERE peer=$1",
            &[&peer],
        )
        .unwrap()
        .get(0)
    }

    /// db/036 (issue #196): the clinical seq cursor. event_log.seq (the monotonic
    /// node-local insertion order the pull cursors on), sync_state.last_seq (the
    /// per-peer checkpoint), and sync_quarantine.refused_seq (the seq-keyed
    /// re-offer floor) must all exist after loading the SCHEMA subset.
    #[test]
    fn db036_adds_seq_columns() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base); // loads the whole SCHEMA subset
        let ok: bool = c
            .query_one(
                "SELECT
                   EXISTS (SELECT 1 FROM information_schema.columns
                           WHERE table_name='event_log'       AND column_name='seq')
               AND EXISTS (SELECT 1 FROM information_schema.columns
                           WHERE table_name='sync_state'      AND column_name='last_seq')
               AND EXISTS (SELECT 1 FROM information_schema.columns
                           WHERE table_name='sync_state'      AND column_name='quarantine_floor_seq')
               AND EXISTS (SELECT 1 FROM information_schema.columns
                           WHERE table_name='sync_quarantine' AND column_name='refused_seq')",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(
            ok,
            "db/036 must add event_log.seq, sync_state.last_seq, sync_quarantine.refused_seq"
        );
    }

    /// Unwrap a pull that must fail as a PullIntegrityError; returns (message, metrics).
    fn pull_integrity_err(
        c: &mut postgres::Client,
        addr: &str,
        peer: &str,
    ) -> (String, serde_json::Value) {
        let err = do_pull(c, addr, peer, false, None).unwrap_err();
        let ie = err
            .downcast_ref::<PullIntegrityError>()
            .expect("pull must fail as an INTEGRITY error, not transport");
        (ie.message.clone(), ie.metrics.clone())
    }

    /// A mixed batch (valid · garbage · valid): the garbage event is quarantined
    /// DURABLY (verbatim bytes + legible reason), the valid events STILL apply and
    /// the watermark STILL advances (progress) — but the pull FAILS LOUDLY and the
    /// re-offer floor pins below the refused slot, so the slot stays on the wire
    /// every cycle (a durable trace alone is not a license to move past an event —
    /// the #110 review's mixed-batch finding). A re-offer of the same bytes dedupes
    /// onto the same row; a REPAIRED slot is admitted automatically and clears the
    /// floor.
    #[test]
    fn pull_pens_unverifiable_pins_floor_and_recovers_when_peer_repaired() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let garbage = b"not a COSE_Sign1 at all".to_vec();
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&e1, &garbage, &e2], Some(CTX_EVENT.as_str()));

        // Cycle 1: loud integrity failure, but with full progress preserved.
        let addr = serve_canned(raw.clone(), 2);
        let (msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert!(
            msg.contains("re-offer floor"),
            "error explains the floor, got: {msg}"
        );
        assert_eq!(
            m["applied_new"], 2,
            "both valid events applied despite the loud failure"
        );
        assert_eq!(m["skipped_unverifiable"], 1);
        assert_eq!(m["watermark_frozen"], false, "penned, not frozen");
        assert_eq!(m["floor_active"], true);

        let events: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(events, 2);

        // The durable trace: verbatim bytes + peer + a legible reason.
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "exactly the garbage event is quarantined");
        assert_eq!(rows[0].peer, "peer-a");
        assert!(
            !rows[0].reason.trim().is_empty(),
            "reason must be legible, got empty"
        );
        assert_eq!(rows[0].seen_count, 1);
        let held: Vec<u8> = c
            .query_one("SELECT signed_bytes FROM sync_quarantine", &[])
            .unwrap()
            .get(0);
        assert_eq!(held, garbage, "quarantine holds the verbatim wire bytes");

        // Cursor advanced over the whole handled prefix (seq 3, the last event),
        // while the floor pins AT the refused slot's seq (garbage = seq 2).
        assert_eq!(cursor(&mut c, "peer-a"), 3);
        assert_eq!(floor(&mut c, "peer-a"), Some(2));

        // Cycle 2 (same canned batch = what a floor-fetch re-offers): idempotent
        // re-applies, deduped pen row, STILL loud, floor stays at the slot's seq.
        let (_msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(m["applied_new"], 0, "set-union no-op on re-apply");
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "same bytes dedupe onto one row");
        assert_eq!(rows[0].seen_count, 2, "re-offer bumps seen_count");
        assert_eq!(floor(&mut c, "peer-a"), Some(2));

        // Cycle 3 — the peer REPAIRS the slot (re-signed bytes at the same HLC,
        // e.g. after fixing a pre-ADR-0040 history): the fixed event is admitted
        // automatically, the pull succeeds, and the floor clears. The pen row
        // remains as the historical trace of what was once refused.
        let repaired = peer_note(&sk, &kid, WALL_2026 + 1_500);
        let raw = response_json(&[&e1, &repaired, &e2], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let m = do_pull(&mut c, &addr, "peer-a", false, None).unwrap();
        assert_eq!(
            m["applied_new"], 1,
            "the repaired event is admitted automatically"
        );
        assert_eq!(m["skipped_unverifiable"], 0);
        assert_eq!(m["floor_active"], false);
        assert_eq!(
            floor(&mut c, "peer-a"),
            None,
            "clean cycle clears the floor"
        );
        assert_eq!(
            quarantine_rows(&mut c).len(),
            1,
            "the trace survives as history"
        );
    }

    /// A human `acked` row is a recorded license to exclude: the same garbage
    /// re-offered after acking no longer pins the floor and no longer fails the
    /// pull — the skip has become an attributable operator decision (db/021).
    #[test]
    fn acked_row_releases_floor_and_pull_succeeds() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let garbage = b"peer serves this corrupt frame forever".to_vec();
        let raw = response_json(&[&e1, &garbage], Some(CTX_EVENT.as_str()));

        let addr = serve_canned(raw.clone(), 2);
        let (_msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(m["skipped_unverifiable"], 1);
        assert!(floor(&mut c, "peer-a").is_some());

        // Operator inspects and licenses the exclusion.
        c.execute("UPDATE sync_quarantine SET acked = TRUE", &[])
            .unwrap();

        // Same wire content again: quiet success, floor released.
        let m = do_pull(&mut c, &addr, "peer-a", false, None).unwrap();
        assert_eq!(m["skipped_unverifiable"], 0);
        assert_eq!(
            m["skipped_acked"], 1,
            "the acked skip is still counted, honestly"
        );
        assert_eq!(floor(&mut c, "peer-a"), None);
    }

    /// A peer whose ENTIRE batch is unverifiable (and that declares no signing
    /// context — the pre-ADR-0040 legacy shape) must fail the pull LOUDLY instead
    /// of silently skipping and livelocking, while still preserving every event
    /// durably. The watermark must not move.
    #[test]
    fn pull_fails_loud_when_every_event_is_unverifiable() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        let g1 = b"legacy or corrupt blob one".to_vec();
        let g2 = b"legacy or corrupt blob two".to_vec();
        // Legacy peer shape: NO signing_context field (pre-ADR-0040), but the seqs
        // travel (a #196 serve always ships them) so the puller reaches the
        // per-event verify + all-unverifiable diagnosis rather than the seq guard.
        let raw = serde_json::to_vec(&serde_json::json!({
            "events": [hex::encode(&g1), hex::encode(&g2)],
            "seqs": [1, 2],
        }))
        .unwrap();

        let addr = serve_canned(raw, 2);
        let (err, _m) = pull_integrity_err(&mut c, &addr, "peer-legacy");
        assert!(
            err.contains("pre-ADR-0040"),
            "diagnosis must name the likely cause (mixed-version peer), got: {err}"
        );
        assert!(
            err.contains("unverifiable"),
            "diagnosis must say what happened, got: {err}"
        );

        // Loud, but nothing lost: both events preserved durably. The cursor advances
        // over the penned (handled) slots, but the floor re-offers them from seq 0.
        assert_eq!(quarantine_rows(&mut c).len(), 2);
        assert_eq!(cursor(&mut c, "peer-legacy"), 2);
        assert_eq!(
            floor(&mut c, "peer-legacy"),
            Some(1),
            "re-offer floor pins the first slot"
        );

        // The next cycle fails loudly AGAIN (no silent livelock) and the
        // quarantine dedupes rather than growing without bound.
        assert!(do_pull(&mut c, &addr, "peer-legacy", false, None).is_err());
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter().all(|r| r.seen_count == 2),
            "re-offers bump, never duplicate"
        );
    }

    /// The silent-livelock case the old `skipped == len` heuristic missed (#110
    /// review finding 1b): an ALREADY-SYNCED link whose post-watermark events are
    /// all unverifiable. The boundary event re-applies idempotently, making the
    /// batch mixed — the pull must STILL fail loudly (any unacked refusal is loud).
    #[test]
    fn pull_fails_loud_on_synced_link_with_unverifiable_tail() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // Sync the link cleanly first.
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let raw = response_json(&[&e1], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let m = do_pull(&mut c, &addr, "peer-a", false, None).unwrap();
        assert_eq!(m["applied_new"], 1);

        // Now the peer's new tail is garbage; the boundary event re-ships (a
        // floor-fetch re-includes it), so the batch is MIXED.
        let garbage = b"corrupt tail after the watermark".to_vec();
        let raw = response_json(&[&e1, &garbage], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let (_err, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(m["skipped_unverifiable"], 1);
        assert_eq!(quarantine_rows(&mut c).len(), 1);
        // Floor pinned at the tail slot's seq (garbage = seq 2) so it stays on the wire.
        assert_eq!(floor(&mut c, "peer-a"), Some(2));
    }

    /// A non-hex entry must not abort the pull (the old `hex::decode(..)?` wedged
    /// the whole link on one bad entry — #110 review finding 7): it is penned like
    /// any other unverifiable frame (verbatim wire text), valid events still apply.
    #[test]
    fn pull_pens_non_hex_entry_instead_of_wedging() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let raw = serde_json::to_vec(&serde_json::json!({
            "events": [hex::encode(&e1), "zz-not-hex-at-all"],
            "seqs": [1, 2],
            "signing_context": CTX_EVENT.as_str(),
        }))
        .unwrap();

        let addr = serve_canned(raw, 1);
        let (_err, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(m["applied_new"], 1, "the valid event still applied");
        assert_eq!(m["skipped_unverifiable"], 1);

        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].reason.contains("not valid hex"),
            "legible reason: {}",
            rows[0].reason
        );
        let held: Vec<u8> = c
            .query_one("SELECT signed_bytes FROM sync_quarantine", &[])
            .unwrap()
            .get(0);
        assert_eq!(
            held,
            b"zz-not-hex-at-all".to_vec(),
            "verbatim wire text preserved"
        );
    }

    /// Issue #196 (puller side, direct seq bookkeeping): the cursor checkpoints the
    /// max HANDLED seq (applied OR penned), the re-offer floor pins the refused
    /// slot's seq, and the pen row records refused_seq as forensics. A mixed batch
    /// [valid, garbage, valid] with serve-assigned seqs 1/2/3.
    #[test]
    fn pull_checkpoints_seq_cursor_and_reoffers_on_refused_seq() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let garbage = b"not a COSE_Sign1".to_vec();
        let e3 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&e1, &garbage, &e3], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let (_msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        // e1(seq1) and e3(seq3) applied; garbage(seq2) penned with refused_seq=2.
        assert_eq!(m["applied_new"], 2);
        assert_eq!(m["cursor_seq"], 3, "checkpoint at the max handled seq");
        assert_eq!(cursor(&mut c, "peer-a"), 3, "cursor persisted");
        assert_eq!(floor(&mut c, "peer-a"), Some(2), "floor = the refused seq");
        let rs: i64 = c
            .query_one("SELECT refused_seq FROM sync_quarantine", &[])
            .unwrap()
            .get(0);
        assert_eq!(rs, 2, "the pen row records refused_seq as forensics");
    }

    /// PR #223 review hardening: `seqs[]` is untrusted wire input that persists into
    /// sync_state (the advance-only cursor + the re-offer floor). The contiguous-
    /// prefix freeze logic RELIES on ascending order, and the floor's `-1` fetch
    /// arithmetic on positive values — so a batch violating either (a buggy or
    /// hostile peer) must be refused loudly, cursor untouched, nothing admitted.
    #[test]
    fn pull_rejects_malformed_seqs() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        // VALID events under out-of-order seqs: were the guard missing, both would
        // apply and the pull would succeed — this test then fails, not just weakens.
        for seqs in [serde_json::json!([2, 1]), serde_json::json!([0, 1])] {
            let raw = serde_json::to_vec(&serde_json::json!({
                "events": [hex::encode(&e1), hex::encode(&e2)],
                "seqs": seqs,
                "signing_context": CTX_EVENT.as_str(),
            }))
            .unwrap();
            let addr = serve_canned(raw, 1);
            let err = do_pull(&mut c, &addr, "peer-a", false, None)
                .unwrap_err()
                .to_string();
            assert!(
                err.contains("ascending"),
                "malformed seqs must be named, got: {err}"
            );
            assert_eq!(cursor(&mut c, "peer-a"), 0, "cursor untouched");
        }
        assert!(quarantine_rows(&mut c).is_empty(), "nothing penned");
        let n: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(n, 0, "no event admitted from a malformed batch");
    }

    /// PR #223 review: a peer that ACCEPTS the connection but hangs up without a
    /// response frame is the signature of a pre-#196 serve — its serde decode of the
    /// unknown `EventsAfterSeq` op fails and the connection drops. The pull must
    /// fail with a diagnosis naming that likely cause and the remedy (upgrade the
    /// peer), never a bare EOF the operator can only read as a partition.
    #[test]
    fn pull_from_pre_seq_server_fails_legibly() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        // The "old serve": read the request frame, then close without replying.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            // request() retries 4 times; hang up on every attempt.
            for _ in 0..4 {
                let Ok((mut s, _)) = listener.accept() else {
                    break;
                };
                let _ = read_frame(&mut s);
                // Dropping the stream closes it with no response frame written.
            }
        });
        let err = do_pull(&mut c, &addr, "peer-old", false, None)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("db/036"),
            "must name the likely pre-#196 peer and the remedy, got: {err}"
        );
        assert_eq!(cursor(&mut c, "peer-old"), 0, "cursor untouched");
    }

    /// At the per-peer quota the pen refuses to grow (#110 review finding 2 —
    /// remote bytes must never fill the clinical node's disk): the watermark
    /// freezes instead (delayed, never lost) and the failure is loud and legible.
    #[test]
    fn pen_quota_freezes_watermark_instead_of_growing() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // Fill the pen to the row quota with synthetic traces from this peer.
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
             SELECT int4send(i), '\\x00'::bytea, 'peer-flood', 'filler'
             FROM generate_series(1, $1::int) i",
            &[&(MAX_QUARANTINE_ROWS_PER_PEER as i32)],
        )
        .unwrap();

        let fresh_garbage = b"yet another distinct corrupt frame".to_vec();
        let raw = response_json(&[&fresh_garbage], None);
        let addr = serve_canned(raw, 1);
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-flood");
        assert!(err.contains("quota"), "error names the quota, got: {err}");
        assert_eq!(
            m["watermark_frozen"], true,
            "over quota = freeze, never skip"
        );

        let count: i64 = c
            .query_one(
                "SELECT count(*) FROM sync_quarantine WHERE peer='peer-flood'",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(count, MAX_QUARANTINE_ROWS_PER_PEER, "the pen did not grow");
    }

    /// The BYTE half of the quota: a pen already near its byte budget refuses a
    /// frame that would overshoot it (the admission check counts the incoming
    /// event's own size, so one huge frame cannot blow past the cap).
    #[test]
    fn pen_byte_quota_refuses_overshooting_frame() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // One filler row 1 KiB under the byte budget for this peer.
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
             VALUES ('\\x0042', convert_to(repeat(' ', $1::int), 'UTF8'), 'peer-fat', 'filler')",
            &[&((MAX_QUARANTINE_BYTES_PER_PEER - 1024) as i32)],
        )
        .unwrap();

        // A 2 KiB garbage frame would overshoot: must be refused, not penned.
        let fat_garbage = vec![0u8; 2048];
        let raw = response_json(&[&fat_garbage], None);
        let addr = serve_canned(raw, 1);
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-fat");
        assert!(err.contains("quota"), "error names the quota, got: {err}");
        assert_eq!(m["watermark_frozen"], true);
        let rows: i64 = c
            .query_one(
                "SELECT count(*) FROM sync_quarantine WHERE peer='peer-fat'",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(rows, 1, "the overshooting frame was not penned");
    }

    /// Issue #197 (2026-07-15 review, B2): acked rows must NOT consume the ROW
    /// quota. The quota error's own remedy is "fix or ack the held rows" — if
    /// acked rows still counted, following that instruction would change
    /// nothing: every new refused frame would still hit Err(quota) and the
    /// cursor would stay frozen forever, with a manual DELETE the only
    /// (undocumented) way out. An acked row is a resolved human decision,
    /// retained as the record of it — the budget bounds only the UNACKED rows
    /// still awaiting one (the node plane learned this first: see
    /// quarantine_node_event in cairn-node/src/sync.rs).
    #[test]
    fn acked_rows_do_not_consume_row_quota() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // A peer floods the pen to its row quota; the operator then follows the
        // documented remedy and acks every held row.
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason, acked)
             SELECT int4send(i), '\\x00'::bytea, 'peer-acked-flood', 'filler', TRUE
             FROM generate_series(1, $1::int) i",
            &[&(MAX_QUARANTINE_ROWS_PER_PEER as i32)],
        )
        .unwrap();

        // A fresh corrupt frame must now be PENNED — the pull still fails
        // loudly on the unacked refusal (normal quarantine discipline), but
        // NOT as a pen-quota freeze.
        let fresh_garbage = b"first frame after the operator acked the flood".to_vec();
        let raw = response_json(&[&fresh_garbage], None);
        let addr = serve_canned(raw, 1);
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-acked-flood");
        assert!(
            !err.contains("quota"),
            "acked rows must not consume quota, got: {err}"
        );
        assert_eq!(
            m["watermark_frozen"], false,
            "the pen accepted the frame — nothing to freeze"
        );
        let unacked: i64 = c
            .query_one(
                "SELECT count(*) FROM sync_quarantine
                  WHERE peer='peer-acked-flood' AND NOT acked",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(unacked, 1, "the fresh frame was penned");
    }

    /// Issue #197, the BYTE half: acked rows must not consume the byte budget
    /// either — same remedy, same wedge if they did.
    #[test]
    fn acked_rows_do_not_consume_byte_quota() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // One ACKED filler row 1 KiB under the byte budget for this peer.
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason, acked)
             VALUES ('\\x0042', convert_to(repeat(' ', $1::int), 'UTF8'), 'peer-fat-acked',
                     'filler', TRUE)",
            &[&((MAX_QUARANTINE_BYTES_PER_PEER - 1024) as i32)],
        )
        .unwrap();

        // A 2 KiB garbage frame would overshoot only if the acked row still
        // counted — it must be penned.
        let fat_garbage = vec![0u8; 2048];
        let raw = response_json(&[&fat_garbage], None);
        let addr = serve_canned(raw, 1);
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-fat-acked");
        assert!(
            !err.contains("quota"),
            "acked bytes must not consume quota, got: {err}"
        );
        assert_eq!(m["watermark_frozen"], false);
        let unacked: i64 = c
            .query_one(
                "SELECT count(*) FROM sync_quarantine
                  WHERE peer='peer-fat-acked' AND NOT acked",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(unacked, 1, "the fresh frame was penned");
    }

    /// #197 follow-on (PR #224 review): the quota probes filter on
    /// `peer = … AND NOT acked`, and the acked rows they exclude from the COUNT
    /// still sit in the SCAN — with only the content_digest PK they seq-scan the
    /// whole pen on every refused frame, and the retained-acked set is the one
    /// part of the table the quota no longer bounds. db/021 must ship a partial
    /// index matching the probes' predicate so they stay O(unacked).
    #[test]
    fn db021_partial_index_backs_unacked_quota_probes() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base); // replays db/021 — also proves idempotency
        let ok: bool = c
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_indexes
                                WHERE tablename = 'sync_quarantine'
                                  AND indexname = 'sync_quarantine_peer_unacked_idx')",
                &[],
            )
            .unwrap()
            .get(0);
        assert!(
            ok,
            "db/021 must create sync_quarantine_peer_unacked_idx ON (peer) WHERE NOT acked"
        );
    }

    /// The floor must SURVIVE a cycle whose pen write fails (fresh-eyes review
    /// of this fixup): after a slot is refused and the watermark advances past
    /// it, a later cycle where quarantine_event errors (here: row deleted by an
    /// operator while the peer sits at quota) produces NO pin — blindly
    /// overwriting would clear the floor and permanently release the slot.
    #[test]
    fn floor_survives_pen_failure_on_reoffer_cycle() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // Cycle 1: garbage penned, floor pinned, watermark advances past it.
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let garbage = b"slot the floor must keep guarding".to_vec();
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&e1, &garbage, &e2], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw.clone(), 2);
        let (_msg, _m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(floor(&mut c, "peer-a"), Some(2));
        assert_eq!(cursor(&mut c, "peer-a"), 3);

        // Sabotage the pen for the re-offer: delete the row AND fill the peer's
        // quota, so the re-offered garbage can be neither bumped nor inserted.
        c.execute("DELETE FROM sync_quarantine", &[]).unwrap();
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
             SELECT int4send(i), '\\x00'::bytea, 'peer-a', 'filler'
             FROM generate_series(1, $1::int) i",
            &[&(MAX_QUARANTINE_ROWS_PER_PEER as i32)],
        )
        .unwrap();

        // Cycle 2 (re-offer): pen fails → loud, watermark frozen, and the floor
        // is RETAINED even though this cycle produced no pin.
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert!(err.contains("quota"), "pen failure surfaces, got: {err}");
        assert_eq!(m["watermark_frozen"], true);
        assert_eq!(
            floor(&mut c, "peer-a"),
            Some(2),
            "a pen-failure cycle must never clear the floor"
        );
    }

    /// A peer that DECLARES a different signing context is deterministic wire-format
    /// skew: refuse the whole batch up front with a legible error naming both
    /// contexts — don't burn per-event verify failures or quarantine anything
    /// (the peer still holds the events; they apply after the skew is fixed).
    #[test]
    fn pull_refuses_declared_context_mismatch_deterministically() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let raw = response_json(&[&e1], Some("application/cairn-event+cbor;v=999"));

        let addr = serve_canned(raw, 1);
        let (err, m) = pull_integrity_err(&mut c, &addr, "peer-skew");
        assert!(
            err.contains("application/cairn-event+cbor;v=999") && err.contains(CTX_EVENT.as_str()),
            "error must name BOTH contexts so the operator sees the skew, got: {err}"
        );
        assert!(
            m.is_null(),
            "refused before any per-event work — no metrics"
        );

        let events: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(events, 0, "nothing applied from a batch refused for skew");
        assert!(
            quarantine_rows(&mut c).is_empty(),
            "skew-refused batch is not quarantined"
        );
        assert_eq!(cursor(&mut c, "peer-skew"), 0);
    }

    /// Re-processing after the operator fixes the cause (the issue's "inspectable
    /// and re-processable"): a quarantined event that NOW verifies (e.g. it was
    /// falsely rejected by a version-skewed daemon binary since upgraded) is
    /// released through the real apply door and its row cleared; one that still
    /// fails stays held — with the door's refusal in `last_requeue_error` and the
    /// ORIGINAL verify-time reason untouched (#110 review finding 5).
    #[test]
    fn requeue_releases_quarantined_events_once_cause_is_fixed() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // Simulate a past false rejection: a perfectly valid event sitting in
        // quarantine (as if the daemon that pulled it was version-skewed), plus
        // one genuinely corrupt blob that can never be released.
        let good = peer_note(&sk, &kid, WALL_2026 + 5_000);
        let junk = b"permanently corrupt".to_vec();
        for (bytes, why) in [
            (&good, "simulated version-skew rejection"),
            (&junk, "corrupt"),
        ] {
            c.execute(
                "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, reason)
                 VALUES ($1, $2, 'peer-a', $3)",
                &[&cairn_event::event_address(bytes), bytes, &why],
            )
            .unwrap();
        }

        let m = do_requeue(&mut c).unwrap();
        assert_eq!(m["examined"], 2);
        assert_eq!(
            m["released"], 1,
            "the now-valid event goes through the apply door"
        );
        assert_eq!(m["still_quarantined"], 1);

        let events: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(events, 1, "released event landed in event_log via the door");
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "released row is cleared, corrupt row stays");
        assert_eq!(
            rows[0].reason, "corrupt",
            "the ORIGINAL verify-time reason survives requeue untouched"
        );

        // The door's CURRENT refusal is recorded beside it, and the operator
        // listing (the exact `cairn-sync quarantine` output) carries both.
        let listing = quarantine_listing(&mut c).unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0]["reason"], "corrupt");
        assert!(
            listing[0]["last_requeue_error"]
                .as_str()
                .unwrap()
                .contains("verification"),
            "door refusal recorded in last_requeue_error: {}",
            listing[0]["last_requeue_error"]
        );
        assert_eq!(listing[0]["acked"], false);
    }

    /// An upgraded binary against a database that predates db/036 (the clinical seq
    /// cursor) must fail fast and legibly (point at `init`), not limp into a runtime
    /// failure when do_pull reads the missing seq columns (#110 review finding 4,
    /// re-pointed at the db/036 marker for #196).
    #[test]
    fn connect_checked_fails_legibly_on_pre_seq_schema() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // Knock the seq cursor off event_log — the shape a pre-db/036 DB is in.
        // connect_checked only CONNECTS + probes (it does not reload the schema),
        // so the missing column is not silently re-added under it.
        c.batch_execute("ALTER TABLE event_log DROP COLUMN seq")
            .unwrap();
        // (No unwrap_err: postgres::Client is not Debug, so destructure by hand.)
        let err = match connect_checked(&base) {
            Ok(_) => panic!("connect_checked must refuse a pre-db/036 schema"),
            Err(e) => e.to_string(),
        };
        assert!(
            err.contains("cairn-sync init"),
            "error must tell the operator the remedy, got: {err}"
        );
        assert!(
            err.contains("db/036"),
            "error must name the missing migration, got: {err}"
        );

        // Restore for whatever suite runs next under the shared lock.
        c.batch_execute(include_str!("../../../db/036_clinical_sync_seq.sql"))
            .unwrap();
        assert!(
            connect_checked(&base).is_ok(),
            "schema restored, probe passes"
        );
    }

    /// The loaded cairn_pgx on the test rig satisfies the ADR-0040 wire-format floor —
    /// the happy path of the #109 startup skew check. The stale-library FAILURE path
    /// can't be exercised without installing an old `.so`; its parse/compare logic and
    /// the missing-`cairn_pgx_version()` translation are covered in `mod tests`.
    #[test]
    fn assert_pgx_floor_passes_on_the_current_rig() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        assert_pgx_floor(&mut c).expect("the installed cairn_pgx meets the required floor");
    }
}

/// PR #225 review — the fingerprint consts must EXECUTE, not just string-match.
/// The `fingerprint_orderings_compare_under_collate_c` drift guard pins the ORDER BY
/// collation, but a typo anywhere else in the extracted SQL would load cleanly and
/// surface only at an operator's first field `fingerprint` run; and the projection
/// hash's field concatenation must not be boundary-ambiguous. Real Postgres, gated
/// on `$CAIRN_TEST_PG`, serialized via the shared advisory lock (connection helpers
/// borrowed from `quarantine_tests` — same file, same discipline).
#[cfg(test)]
mod fingerprint_db_tests {
    use super::quarantine_tests::{cs, locked_client};
    use super::*;

    #[test]
    fn fingerprint_event_hash_query_executes_on_the_real_schema() {
        // Standing execution guard (green by construction today, like the bounds
        // guard on MAX_FRAME_BYTES): a future edit that breaks the const's SQL —
        // column rename, quoting slip — becomes a CI failure here instead of a
        // runtime error in the field. An empty event_log fingerprints as NULL;
        // that is the documented "no events" shape, not a failure.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base); // loads schema, truncates event_log
        let hash: Option<String> = c
            .query_one(FINGERPRINT_EVENT_HASH_SQL, &[])
            .expect("the event-hash fingerprint SQL must parse and run on the real schema")
            .get(0);
        assert!(
            hash.is_none(),
            "an empty event_log must fingerprint as NULL (the no-events shape)"
        );
    }

    #[test]
    fn fingerprint_projection_hash_distinguishes_field_boundaries() {
        // PR #225 review: without separators between the concatenated fields,
        // (name='X', dob='1980') and (name='X1', dob='980') hash IDENTICALLY — a
        // false CONVERGENCE (a missed divergence), the exact inverse of the false
        // alarm the COLLATE "C" pin fixed. Two chart states that differ only in
        // where one field ends and the next begins must hash differently.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base); // loads schema, truncates patient_chart
        let pid = uuid::Uuid::now_v7().to_string();
        let mut hash_for = |name: &str, dob: &str| -> Option<String> {
            c.execute("TRUNCATE patient_chart", &[]).unwrap();
            c.execute(
                "INSERT INTO patient_chart (patient_id, name, dob, sex, note_count)
                 VALUES (($1::text)::uuid, $2, $3, '', 0)",
                &[&pid, &name, &dob],
            )
            .unwrap();
            c.query_one(FINGERPRINT_PROJECTION_HASH_SQL, &[])
                .expect("the projection-hash fingerprint SQL must parse and run")
                .get(0)
        };
        let boundary_a = hash_for("X", "1980");
        let boundary_b = hash_for("X1", "980");
        assert!(boundary_a.is_some() && boundary_b.is_some());
        assert_ne!(
            boundary_a, boundary_b,
            "shifting a field boundary must change the projection hash"
        );
    }
}

/// Issue #198 (review finding B3) — the SCHEMA subset must satisfy its own doors,
/// STANDING ALONE.
///
/// `SCHEMA` above is a mirror of a subset of `db/*.sql`, and PL/pgSQL late binding
/// means an omitted migration does not fail the load — it fails at the first write,
/// as `function ... does not exist` inside `submit_event`/`apply_remote_event` (a
/// total write outage on a fresh `cairn-sync init` database, the documented
/// walking-skeleton flow). Every other DB-gated suite in this workspace runs against
/// a database that cairn-node's FULL 35-file loader has already visited, so the gap
/// is structurally invisible there: this test is the drift guard. It wipes the
/// second test database (`$CAIRN_TEST_PG2`), loads ONLY `SCHEMA`, and drives every
/// SQL entry point the subset ships — the two the 2026-07-15 review found dangling:
///
///   * `submit_event` → `cairn_learn_attachment_refs` (db/027) — unconditional on
///     EVERY submit, exercised end-to-end here with a by-reference attachment whose
///     lazy blob reference must land in `blob_store`;
///   * the db/002 `patient_chart` trigger → `cairn_hlc_triple_collision` +
///     `cairn_record_hlc_collision` (db/029) — parsed on the first
///     `patient.created`, EXECUTED here by applying a genuine Byzantine pair (two
///     different signed bodies under one HLC triple) through `apply_remote_event`;
///
/// and the two db/006 recall-ceremony doors (`recall_event`,
/// `events_by_actor_epoch`), which were loaded-but-undriven in the first cut of
/// this test — the exact shape the late-binding trap needs (PR #222 review).
/// db/021 ships only the `sync_quarantine` TABLE (the quarantine writes are
/// daemon-side SQL, not PL/pgSQL), so there is no quarantine function to drive:
/// with these four (plus `enroll_actor` in the setup), every caller-facing entry
/// point the subset ships is executed. Internal helpers are covered transitively
/// along the driven event shapes only — an edge reachable solely from an event
/// type this test does not author (e.g. a suppression overlay) still needs its
/// path added here when such a call is introduced.
///
/// Adding a call from any of these doors to a function in a NOT-yet-listed
/// migration will fail this test with the exact production error message, instead
/// of shipping a first-write outage. Serialized against every other DB-gated suite
/// via the shared advisory-lock key; the wipe is safe because every suite that
/// shares `$CAIRN_TEST_PG2` (federation, sync_watermark, clinical_pull) reloads
/// the full schema on connect.
#[cfg(test)]
mod schema_subset_tests {
    use super::*;
    use cairn_event::{Attachment, Rendition};

    /// Build + sign one `patient.created` event. The body mirrors what `emit_event`
    /// authors (payload name/dob/sex is what the db/002 projection reads), with the
    /// HLC triple caller-chosen so the Byzantine-collision case can be constructed.
    fn signed_patient_created(
        sk: &SigningKey,
        kid: &str,
        patient_id: &str,
        name: &str,
        wall: i64,
        origin: &str,
        attachments: Vec<Attachment>,
    ) -> Vec<u8> {
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: patient_id.to_string(),
            event_type: "patient.created".into(),
            schema_version: "patient/1".into(),
            hlc: Hlc {
                wall,
                counter: 0,
                node_origin: origin.into(),
            },
            t_effective: None,
            signer_key_id: kid.into(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: serde_json::json!({"name": name, "dob": "1980-01-01", "sex": "U"}),
            attachments,
            plaintext_twin: None,
        };
        // ADR-0039: author the twin into the signed body, as every production author does.
        let body = materialise_generic_twin(body);
        sign(&body, sk).unwrap().signed_bytes
    }

    #[test]
    fn schema_subset_alone_satisfies_every_door() {
        let (Some(base), Some(base2)) = (
            std::env::var("CAIRN_TEST_PG").ok(),
            std::env::var("CAIRN_TEST_PG2").ok(),
        ) else {
            eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
            return;
        };

        // Serialize against every other DB-gated suite (same 'CARN' advisory-lock key,
        // taken on the primary database so it serializes cluster-wide regardless of
        // which database each suite touches). Held until `lock` drops at test end.
        let mut lock = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        lock.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();

        // Wipe the second database down to a fresh `cairn-sync init` starting point.
        // One batch = one implicit transaction: the schema drop and the extension
        // re-create land together, so a failure part-way cannot strand the database
        // extension-less for the suites that share it.
        let mut c = postgres::Client::connect(&base2, postgres::NoTls).unwrap();
        c.batch_execute(
            "DROP SCHEMA public CASCADE;
             CREATE SCHEMA public;
             CREATE EXTENSION cairn_pgx;",
        )
        .unwrap();

        // Load ONLY the subset — exactly what `cairn-sync init` installs.
        for (name, sql) in SCHEMA {
            c.batch_execute(sql)
                .unwrap_or_else(|e| panic!("loading {name} from the subset alone: {e}"));
        }

        // Honesty guard on the fixture itself: if the wipe ever stops working, this
        // database silently reverts to full-schema and the test proves nothing.
        // Three canaries from three different non-subset migrations must ALL be
        // absent — one alone could silently vanish from the full schema (renamed or
        // dropped) and leave this check vacuously green (PR #222 review finding 2):
        // db/016's match-veto function, db/010's patient_identifier table, db/031's
        // medication_statement table.
        let residue: i64 = c
            .query_one(
                "SELECT (SELECT count(*) FROM pg_proc  WHERE proname = 'cairn_match_veto')
                      + (SELECT count(*) FROM pg_class WHERE relname IN
                             ('patient_identifier', 'medication_statement'))",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            residue, 0,
            "wipe failed: full-schema residue present, the subset-only property is gone"
        );

        // One enrolled signer for all three events (the Byzantine pair below models a
        // broken/hostile signer REUSING its HLC triple, not a second identity).
        let (sk, kid) = cairn_event::generate_key().unwrap();
        c.execute(
            "SELECT enroll_actor('agent', '{\"model\":\"subset-test\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
            &[&kid],
        )
        .unwrap();

        let patient_id = uuid::Uuid::now_v7().to_string();

        // ── Door 1: the LOCAL door, with a by-reference attachment ────────────────
        // submit_event PERFORMs cairn_learn_attachment_refs(b) unconditionally
        // (db/005); with db/027 missing this is the first-write outage the review
        // predicted. The attachment also proves the 027 path end-to-end: the lazy
        // blob reference must land in blob_store (reference-eager, byte-lazy).
        let photo_bytes = b"subset-test-attachment-bytes";
        let local = signed_patient_created(
            &sk,
            &kid,
            &patient_id,
            "Subset Alice",
            now_ms(),
            "subset-local",
            vec![Attachment::single(
                "photo: identifying mark, right forearm",
                Rendition::reference("original", photo_bytes, "image/png"),
            )],
        );
        // (::text — this binary reads UUIDs as text, same as the projection queries.)
        let local_event_id: String = c
            .query_one("SELECT submit_event($1)::text", &[&local])
            .expect("submit_event must succeed against the subset alone")
            .get(0);
        let (blob_rows, blob_lazy): (i64, i64) = {
            let row = c
                .query_one(
                    "SELECT count(*), count(*) FILTER (WHERE NOT present) FROM blob_store",
                    &[],
                )
                .unwrap();
            (row.get(0), row.get(1))
        };
        assert_eq!(
            blob_rows, 1,
            "the attachment's lazy blob reference must be learned"
        );
        assert_eq!(
            blob_lazy, 1,
            "reference-eager, byte-lazy: bytes not yet present"
        );

        // ── Door 2: the REMOTE door, overlaying the same patient ──────────────────
        // apply_remote_event PERFORMs cairn_learn_attachment_refs too (db/020), and
        // with a current demographic winner standing, the db/002 trigger now parses
        // AND executes the db/029 collision predicate (FOUND = true, collision false).
        let wall_b = now_ms() + 1;
        let remote = signed_patient_created(
            &sk,
            &kid,
            &patient_id,
            "Subset Alice (amended at B)",
            wall_b,
            "subset-peer",
            vec![],
        );
        c.query_one("SELECT apply_remote_event($1, NULL, NULL)", &[&remote])
            .expect("apply_remote_event must succeed against the subset alone");
        let name: String = c
            .query_one(
                "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
                &[&patient_id],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            name, "Subset Alice (amended at B)",
            "the HLC-later remote event must win the demographic overlay"
        );

        // ── The Byzantine pair: EXECUTE the db/029 recorder, not just parse it ─────
        // A different signed body under the SAME (wall, counter, origin) triple as the
        // standing winner — provably a broken or hostile signer (#157). The apply must
        // still converge (content-address tiebreak) AND record the advisory signal.
        let byzantine = signed_patient_created(
            &sk,
            &kid,
            &patient_id,
            "Subset Mallory",
            wall_b,
            "subset-peer",
            vec![],
        );
        c.query_one("SELECT apply_remote_event($1, NULL, NULL)", &[&byzantine])
            .expect("the Byzantine twin must still be admitted (availability over consistency)");
        let collisions: i64 = c
            .query_one(
                "SELECT count(*) FROM hlc_collision_log WHERE overlay = 'patient_chart'",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            collisions, 1,
            "the HLC-triple collision must be recorded as an advisory signal (db/029)"
        );

        // ── Door 3: the db/006 recall ceremony (PR #222 review finding 1) ─────────
        // recall_event and events_by_actor_epoch were loaded-but-undriven in the
        // first cut — the exact shape the late-binding trap needs. Drive both so a
        // future edge from either into an unlisted migration fails here too. The
        // connecting role owns recall_overlay (it just ran the migrations), so the
        // db/006 REVOKE floor does not block this operator-style call.
        c.query_one(
            "SELECT recall_event($1::text::uuid, 'subset drift guard: drive the recall door')",
            &[&local_event_id],
        )
        .expect("recall_event must succeed against the subset alone");
        let recalls: i64 = c
            .query_one("SELECT count(*) FROM recall_overlay", &[])
            .unwrap()
            .get(0);
        assert_eq!(
            recalls, 1,
            "the recall must land in the append-only overlay"
        );

        // The contamination-cascade query: one enrollment of (key, epoch 'e')
        // preceded all three admissions, so every event is selected, stamped 'pinned'.
        let cascade: i64 = c
            .query_one(
                "SELECT count(*) FROM events_by_actor_epoch($1, 'e') WHERE attribution = 'pinned'",
                &[&kid],
            )
            .expect("events_by_actor_epoch must succeed against the subset alone")
            .get(0);
        assert_eq!(
            cascade, 3,
            "the cascade must select every event this key/epoch authored"
        );

        // Three admitted events total — nothing quarantined, nothing lost.
        let events: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(events, 3);
    }
}

/// Issue #188 — `init`'s schema replay is the SECOND door (beside cairn-node's
/// connect_and_load_schema) through which an older binary could silently downgrade a
/// newer database's safety floor. Same refusal rule, same db/038 record; mirrors
/// crates/cairn-node/tests/schema_version_guard.rs.
#[cfg(test)]
mod schema_generation_tests {
    use super::*;

    /// The two subset-shape invariants the #188 guard leans on. (1) The subset must
    /// CARRY db/038_node_schema: `init` stamps `node_schema` right after its replay,
    /// so on a fresh database the table's migration has to be in the subset or every
    /// init fails at the stamp. (2) The subset may LAG the repo generation (that is
    /// the normal case — node-only migrations never enter it) but can never EXCEED
    /// it: a subset entry newer than SCHEMA_GENERATION means the constant was not
    /// bumped alongside a new migration (cairn-event's fs guard catches that from
    /// the db/ side; this catches it from the list side).
    #[test]
    fn subset_carries_node_schema_and_never_exceeds_the_repo_generation() {
        assert!(
            SCHEMA.iter().any(|(name, _)| *name == "038_node_schema"),
            "the subset must include db/038_node_schema.sql: init stamps node_schema \
             immediately after its replay, so a fresh database needs the table"
        );
        let newest = cairn_event::schema_generation::newest_migration_prefix(
            SCHEMA.iter().map(|(name, _)| *name),
        )
        .expect("SCHEMA is never empty and every entry has a numeric prefix");
        assert!(
            newest <= embedded_schema_version(),
            "subset entry {newest} is newer than SCHEMA_GENERATION {}: bump the \
             constant in crates/cairn-event/src/schema_generation.rs in the same \
             commit that adds the migration",
            embedded_schema_version()
        );
    }

    #[test]
    fn load_schema_stamps_the_generation_and_refuses_a_newer_db() {
        let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        // Serialize against every other DB-gated suite (same 'CARN' key). Held until
        // `lock` drops at test end.
        let mut lock = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        lock.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();
        let embedded = embedded_schema_version();

        let mut c = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        c.batch_execute("CREATE EXTENSION IF NOT EXISTS cairn_pgx;")
            .unwrap();
        // Heal residue from a previously aborted run: a stale future generation would
        // wedge every loader on this shared database.
        c.batch_execute(&format!(
            "DO $$ BEGIN
               IF to_regclass('public.node_schema') IS NOT NULL THEN
                 UPDATE node_schema SET version = LEAST(version, {embedded});
               END IF;
             END $$;"
        ))
        .unwrap();

        // 1. Happy path: the guarded replay stamps this binary's generation.
        load_schema(&mut c).expect("replay at own generation must succeed");
        let (version, build): (i32, String) = {
            let row = c
                .query_one("SELECT version, loader_build FROM node_schema", &[])
                .unwrap();
            (row.get(0), row.get(1))
        };
        assert_eq!(
            version, embedded,
            "a successful replay must stamp the record"
        );
        assert!(
            build.contains("cairn-sync"),
            "loader_build must identify WHICH tool stamped: {build}"
        );

        // 2. Old binary, new DB: refuse rather than downgrade the floor. Restore
        //    BEFORE asserting so a panic cannot strand the shared database claiming
        //    a future generation.
        c.execute("UPDATE node_schema SET version = $1", &[&(embedded + 1)])
            .unwrap();
        let refused = load_schema(&mut c);
        c.execute("UPDATE node_schema SET version = $1", &[&embedded])
            .unwrap();
        let err = refused
            .expect_err("an older binary must refuse a newer database (issue #188)")
            .to_string();
        assert!(
            err.contains(&format!("{}", embedded + 1)) && err.contains(&format!("{embedded}")),
            "the refusal must name both generations: {err}"
        );

        // 3. The restored database replays again — only genuine downgrades refuse.
        load_schema(&mut c).expect("replay after restore must succeed");
    }

    /// The guard must read the recorded generation UNDER the loaders' advisory
    /// load-lock (2026-07-19 review of PR #251, finding 1) — mirrors
    /// guard_check_happens_under_the_load_lock in cairn-node's
    /// schema_version_guard.rs; see there for the full interleaving argument. An
    /// admin session stands in for a concurrent newer loader mid-replay: it holds
    /// the lock, we spawn this binary's loader, and only then bump the recorded
    /// generation and release. A correct loader parks and sees the bump (refusal);
    /// a check-first loader has already read the stale record and succeeds — which
    /// this test turns into a failure.
    #[test]
    fn guard_check_happens_under_the_load_lock() {
        let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut lock = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        lock.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();
        let embedded = embedded_schema_version();

        let mut admin = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        admin
            .batch_execute("CREATE EXTENSION IF NOT EXISTS cairn_pgx;")
            .unwrap();
        // Heal tamper residue from any previously aborted run (same clamp as above).
        admin
            .batch_execute(&format!(
                "DO $$ BEGIN
                   IF to_regclass('public.node_schema') IS NOT NULL THEN
                     UPDATE node_schema SET version = LEAST(version, {embedded});
                   END IF;
                 END $$;"
            ))
            .unwrap();
        // Baseline: schema present and stamped at this binary's generation.
        load_schema(&mut admin).expect("baseline replay must succeed");

        // The "concurrent newer loader": holds the load-lock while mid-replay.
        admin
            .execute(
                "SELECT pg_advisory_lock($1)",
                &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
            )
            .unwrap();
        let base2 = base.clone();
        // Box<dyn Error> is not Send, so the thread reports only Ok/Err-as-String.
        let loader = std::thread::spawn(move || {
            let mut c = postgres::Client::connect(&base2, postgres::NoTls).unwrap();
            load_schema(&mut c).map_err(|e| e.to_string())
        });
        // Long enough for a check-first (buggy) loader to run to completion; a
        // correct loader is parked on the lock, so this cannot flake when green.
        std::thread::sleep(std::time::Duration::from_secs(2));
        // The "newer loader" finishes: stamp a newer generation, release the lock.
        admin
            .execute("UPDATE node_schema SET version = $1", &[&(embedded + 1)])
            .unwrap();
        admin
            .execute(
                "SELECT pg_advisory_unlock($1)",
                &[&cairn_event::schema_generation::SCHEMA_LOAD_LOCK],
            )
            .unwrap();
        let result = loader.join().unwrap();
        // Restore BEFORE asserting so a failure cannot strand the shared database.
        admin
            .execute("UPDATE node_schema SET version = $1", &[&embedded])
            .unwrap();
        assert!(
            result.is_err(),
            "load_schema read the recorded generation BEFORE taking the load-lock: \
             check-then-act TOCTOU — a concurrent old binary can still downgrade the floor"
        );
    }
}
