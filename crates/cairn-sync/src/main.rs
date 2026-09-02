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
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cairn_event::{
    blob_address, materialise_generic_twin, resolve_twin, sign, sign_attestation,
    verify_self_described, AttestationBody, ClockGrade, EventBody, Hlc, SigningKey, CTX_EVENT,
};
use cairn_wire::{read_frame, write_frame, EventsResponse, Request, TcpTransport, Transport};

// The node's custody-key resolution (issue #503). A module rather than another
// function in this file: main.rs is ~11,700 lines, and the decision table below is
// the safety argument of the ADR-0066 conversion — it belongs somewhere a reviewer
// can hold in one screen, and it is testable with no database and no filesystem.
mod unwrap_key;

// The per-page and per-cycle pull tallies, and the pure quarantine-floor rule they feed
// (slice 2b Task 5, #500/#101). Lifted out ahead of Task 7's paging loop so the folding
// rules are testable on their own, with no peer and no database — main.rs is already
// ~11.5k lines (#531 tracks its decomposition) and this is where new pull-loop logic goes.
mod pull_page;

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
    // Both write doors learn attachment references unconditionally, through the ONE
    // traversal declared here (cairn_by_reference_renditions) — db/005 via the strict
    // learner, db/020 via db/050's lenient one (#460). PL/pgSQL late binding means
    // omitting this migration loads cleanly and fails only at the FIRST write — a total
    // write outage on a fresh `cairn-sync init` database (issue #198, review finding B3).
    (
        "027_attachment_rendition_references",
        include_str!("../../../db/027_attachment_rendition_references.sql"),
    ),
    // Same late-binding trap: the db/002 patient_chart trigger calls the #157
    // Byzantine HLC-collision predicate/recorder on every patient.amended — the
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
    // db/040 (issue #216): the grade-gated ceiling helpers + t_effective_ceiling_flag +
    // cairn_clock_health. db/020 in this subset references cairn_ceiling_classify /
    // cairn_record_ceiling_flag via late-binding — omitting this file would fail the FIRST
    // apply of a forward-dated event on a fresh `cairn-sync init` DB (the #198 trap).
    (
        "040_clock_confidence_grade",
        include_str!("../../../db/040_clock_confidence_grade.sql"),
    ),
    // db/043 (ADR-0056 decision 4 / #266): cairn_readjudicate_deferred. Unlike the
    // medication files, which this subset legitimately lags on (#284), this one is NOT
    // optional here: db/020 above is the door that WRITES the event_deferred marker, so a
    // cairn-sync database without this file would accumulate deferred rows nothing could
    // ever promote — admitted events, permanently powerless, with no mechanism to notice.
    //
    // Shipping the file is necessary but not sufficient: the loader in this crate must also
    // CALL cairn_readjudicate_deferred, or the function sits unused and the markers pile up
    // anyway (PR #302 review finding F3).
    (
        "043_deferred_readjudication",
        include_str!("../../../db/043_deferred_readjudication.sql"),
    ),
    // db/045 + db/047 (#344/#345, ADR-0061). This subset carried NEITHER while db/005 had no
    // opinion about registration — every identity/demographic projection is legitimately absent
    // here (#284), and a replicated registration was simply admitted-and-deferred (ADR-0056).
    //
    // The §5.3/§5.8 precedence rule ended that: db/005 — which this subset DOES carry — now
    // refuses any first event on a chart that is not a registration, and
    // `identity.registration.asserted` is classified only by db/045. A subset with db/005 and
    // without db/045 would be a door carrying a rule it cannot satisfy: no chart could ever be
    // authored against it, and `schema_subset_tests`' local-door case proves exactly that by
    // failing. db/047 follows db/045 because its projection registration is validated against
    // that classification — and it is also what retires `patient.created` on a database this
    // loader built alone.
    (
        "045_patient_registration",
        include_str!("../../../db/045_patient_registration.sql"),
    ),
    (
        "047_registration_precedence",
        include_str!("../../../db/047_registration_precedence.sql"),
    ),
    (
        "048_sensitivity_stream",
        include_str!("../../../db/048_sensitivity_stream.sql"),
    ),
    (
        "049_safety_projection",
        include_str!("../../../db/049_safety_projection.sql"),
    ),
    // db/050 (#460): the attachment-reference ledger + the LENIENT learner db/020 calls, so a
    // malformed rendition reference from a peer is recorded and skipped instead of refusing an
    // already-signed clinical event (which forks the event set and never releases from the pen).
    (
        "050_attachment_reference_flag",
        include_str!("../../../db/050_attachment_reference_flag.sql"),
    ),
];

// DELIBERATELY ABSENT: db/007 (the node plane). Since issue #231 the serve path READS
// `trust_peer` + `local_node` from it to decide whether a pulling peer may obtain
// read-custody — so this subset now has a SOFT dependency on a migration it does not
// load. That is the Slice 64 lesson in its other direction, and it is resolved
// deliberately rather than by adding db/007 here:
//
//   * The node plane is `cairn-node`'s to provision, and on any real node it has —
//     both binaries share one database, and cairn-node's loader carries db/007.
//   * db/007 re-declares `hlc_state`, which db/001 already creates for this subset. The
//     shapes match today, but reconciling that is a decision of its own (issue #284,
//     subset-vs-full consistency), not something to smuggle in behind a custody fix.
//   * The dependency is SOFT because its absence is an ANSWER, not a fault:
//     `look_up_peer_trust` maps SQLSTATE 42P01 to `TrustLookup::NodePlaneAbsent`, which
//     withholds custody and says so. A door carrying a rule it cannot satisfy would be
//     the defect; a door that fails closed and names the missing provisioning is not.
//
// If you ever make this subset load db/007, delete the NodePlaneAbsent arm with it —
// leaving a dead arm behind is how a floor grows unreachable branches nobody tests.

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
    /// Was THIS NODE'S DATABASE also at fault? (Issue #489 part 1.)
    ///
    /// The mirror image of [`CursorCommitError::also_loud`], and it exists for the same
    /// reason: one cycle can be two conditions, and collapsing it to the outer type's
    /// class drops the one that needs a different human action. Set only by the pen
    /// refusal path, and only when a `postgres::Error` is reachable in the pen's own
    /// error — a pen refused by its per-peer QUOTA is the peer's garbage filling a
    /// budget, not a local fault, and must not charge this node's uptime for it.
    also_local_fault: bool,
}

impl std::fmt::Display for PullIntegrityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for PullIntegrityError {}

/// A pull whose per-event work SUCCEEDED but whose cursor commit did not (issue #469).
///
/// This is a THIRD failure class, and it exists because folding it into either of the
/// other two states a wrong diagnosis:
///
/// * [`PullIntegrityError`] means *the peer answered and its DATA is the problem*;
/// * the `partition` flag means *the LINK is the problem* — and the Bet A harness counts
///   it as link downtime;
/// * this means *this node's own DATABASE is the problem*, with the peer and the link
///   both entirely healthy. Before it existed, a failed `UPDATE sync_state` fell through
///   `run`'s catch-all and was logged as a partition: an operator sent to look at the WAN
///   for a local write failure, and an availability figure charged to the wrong cause.
///
/// It carries the cycle's metrics for the same reason `PullIntegrityError` does, and more
/// urgently: the events DID apply, each in its own transaction, and they are durable. A
/// cycle that says nothing at all about work it actually completed is precisely the
/// silence issue #465 set out to end — and it was the new `references_unlearnable` metric
/// falling into this hole that surfaced it.
///
/// Relationship to [`cycle_is_loud`]: that predicate ranges over the states reachable on a
/// cycle that DID commit its cursor, and this is an `Err` return before it is ever
/// consulted — so this class is loud by construction rather than by that test. It does
/// NOT satisfy the predicate: the canonical case (every event applied cleanly, only the
/// bookmark write failed) passes `(0, 0, false, false)`, for which `cycle_is_loud` is
/// FALSE. An earlier draft of this comment claimed otherwise (PR #472 review, finding 6).
///
/// The two are INDEPENDENT, which is what `also_loud` records: a cycle can quarantine
/// unverifiable events *and* fail its commit, and reporting only one of those hands an
/// operator half a diagnosis — with the wrong remedy attached, since this class's own
/// line says the events re-apply idempotently and a quarantined event does no such thing.
#[derive(Debug)]
struct CursorCommitError {
    message: String,
    /// The same metrics JSON a successful pull returns, with the fields that describe
    /// the FAILED write marked unknown — see [`mark_cursor_outcome_unknown`].
    metrics: serde_json::Value,
    /// Was this ALSO a `cycle_is_loud` cycle? Then the log line carries BOTH class keys
    /// and the message carries both diagnoses; the integrity condition is not swallowed
    /// just because the commit failed on the same pass (PR #472 review, finding 5).
    also_loud: bool,
}

impl std::fmt::Display for CursorCommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for CursorCommitError {}

/// A `requeue` that failed PART-WAY THROUGH its loop, carrying what it had achieved
/// (issue #471; ADR-0060 decision 2).
///
/// The three statements inside `do_requeue`'s loop were bare `?`, so a failure on row 5 of
/// 20 returned a raw `postgres::Error` and the whole metrics object went with it. At that
/// moment rows 1-4 have each been RESOLVED one way or another — released (durable in
/// `event_log`, pen row gone), still held, or vanished; only the released ones are durable
/// — and rows 6-20 were never examined. Reporting nothing about either is
/// precisely the implied completion ADR-0060 decision 2 forbids: an operator re-runs the
/// command (correctly — release is idempotent through the db/020 door) with no way to know
/// from the failed run what it had already done, or whether the rows that vanished from the
/// pen were released or lost.
///
/// Modelled on [`CursorCommitError`], which is the same decision one command over and for
/// the same reason: the metrics have to survive the `Err`, so they travel inside it.
///
/// `Debug` is HAND-WRITTEN to delegate to `Display`, and that is load-bearing rather than
/// tidy: `fn main() -> R<()>` has no error printer, so an `Err` reaches Rust's
/// `Termination`, which prints `Error: {err:?}`. With a derived `Debug` the composed
/// sentence — which is the whole acceptance criterion of #471 — arrived wrapped in struct
/// syntax with the metrics re-dumped in Rust debug form beside the JSON already on stdout
/// (PR #478 review, finding 7).
struct RequeueInterruptedError {
    message: String,
    /// The counts as they stood when the statement failed, with `references_unlearnable`
    /// held at `null` — the #465 report never ran, and `0` would be a claim that it had.
    metrics: serde_json::Value,
}

impl std::fmt::Display for RequeueInterruptedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// See the type's doc: `Termination` prints `Debug`, so `Debug` must be the sentence.
impl std::fmt::Debug for RequeueInterruptedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl Error for RequeueInterruptedError {}

/// Compose the sentence an operator reads when a requeue is interrupted. **Pure.**
///
/// Every clause earns its place: the counts say what is DONE and durable, `at` says where
/// it stopped, `cause` says why — it was the eight characters `db error` before — and the
/// remedy is stated because it is real and non-obvious: release runs through the same
/// in-DB door a pull does, so re-running after fixing the fault re-releases nothing twice.
///
/// # The row it stopped ON is its own case, and saying otherwise is a precise untruth
///
/// `examined` is the size of the up-front LISTING, not a count of rows processed, and the
/// three counters are the COMPLETED ones — so the row named by `at` is in none of them.
/// The first draft therefore said "the remaining rows were never examined", which was
/// false for exactly that row: on a failed release its event is already durable in
/// `event_log` while its pen row survives. Principle 4 makes this the wrong trade — an
/// imprecise near-truth beats a precise untruth — so the row is named as undecided rather
/// than folded into either side of the arithmetic (PR #478 review, finding 12).
fn requeue_interrupted_message(
    examined: usize,
    released: usize,
    still_quarantined: usize,
    vanished: usize,
    at: &str,
    cause: &str,
) -> String {
    format!(
        "requeue: INTERRUPTED at {at}, of {examined} pen row(s) listed — this node's own \
         DATABASE failed: {cause}. Work already done is NOT lost: {released} released \
         (durable in event_log, their pen rows gone), {still_quarantined} still held, \
         {vanished} vanished before release. The row at {at} is counted in none of those \
         three: it was reached, and the failure above is exactly what left its outcome \
         undecided — on a failed release its event may already be durable in event_log \
         while its pen row survives. Every row after it was never examined. The \
         attachment-reference report did not run, so it reports null rather than zero. Fix \
         the local database fault and re-run — release goes through the same in-DB door a \
         pull does, so it is idempotent."
    )
}

/// One decoded wire entry: (signed event bytes, attestation, attester key).
type WireEntry = (Vec<u8>, Option<Vec<u8>>, Option<Vec<u8>>);

// ---------------------------------------------------------------------------
// Wire protocol — one JSON request, one JSON response, per connection.
// ---------------------------------------------------------------------------
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

// ---------------------------------------------------------------------------
// Key handling (skeleton: a per-node key file; the registry is ADR-0011).
// ---------------------------------------------------------------------------
/// Load this node's signing key, creating one only when the path does not exist.
///
/// ⚠️ **A file that exists but does not parse is a REFUSAL, never an overwrite.** This function
/// used to `read_to_string` and fall through to create-and-write on any error — including invalid
/// UTF-8, which is exactly what cairn-node's sealed (binary CBOR) key file produces. Pointing this
/// daemon at a real node's key therefore replaced that node's sealed signing key with a fresh
/// plaintext one. The signing key is never backed up (ADR-0026 decision 4), so the identity was
/// gone for good — caused by nothing more than starting a daemon.
fn load_or_create_key(path: &str) -> R<(SigningKey, String)> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                format!(
                    "{path} exists but is not a hex seed (it looks binary — a sealed cairn-node \
                     key?); refusing to overwrite it. Point --key at this daemon's own key file."
                )
            })?;
            let seed: [u8; 32] = hex::decode(text.trim())
                .map_err(|e| {
                    format!("{path} exists but is not valid hex ({e}); refusing to overwrite it")
                })?
                .try_into()
                .map_err(|_| {
                    format!("{path} exists but is not a 32-byte hex seed; refusing to overwrite it")
                })?;
            let sk = SigningKey::from_bytes(&seed);
            let kid = hex::encode(sk.verifying_key().to_bytes());
            Ok((sk, kid))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The only path that creates. Absence is the one state where writing cannot
            // destroy anything.
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
        // Present but unreadable (permissions, I/O): refuse. "Cannot read" is not "absent",
        // and treating it as absent is how the overwrite happened.
        Err(e) => Err(format!("cannot read {path} ({e}); refusing to overwrite it").into()),
    }
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
// The residual refusal contract (ADR-0056 decision 5) — two pure decisions
// ---------------------------------------------------------------------------

/// The SQLSTATE Postgres assigns to a bare `RAISE EXCEPTION` in PL/pgSQL.
/// Every refusal in `apply_remote_event` (db/020) is one of those, so this code
/// is how the puller tells "the floor decided against these bytes" apart from
/// "the database was briefly unhappy".
const SQLSTATE_RAISE_EXCEPTION: &str = "P0001";

/// Did the apply door DELIBERATELY refuse these bytes, or did something break?
///
/// The distinction is the whole of ADR-0056 decision 5's routing. A deliberate
/// refusal is a verdict about the event: it will refuse identically on every
/// retry until either the event or this node's floor changes, so the right move
/// is to pen the bytes verbatim and keep re-offering the slot. A transient fault
/// (dropped connection, serialization failure, statement timeout, disk full) is
/// a verdict about nothing: the very same bytes may well apply next cycle, so
/// the right move is to freeze the cursor and simply try again.
///
/// Getting this backwards is a real defect in both directions — penning on a
/// transient fault fills the pen with events that were never refused, and
/// freezing on a deliberate refusal wedges the peer link behind one bad event
/// (the #270 failure).
///
/// `None` means the error carried no SQLSTATE at all (a dropped connection,
/// a client-side decode failure) — never a door verdict, so never deliberate.
fn refusal_is_deliberate(sqlstate: Option<&str>) -> bool {
    sqlstate == Some(SQLSTATE_RAISE_EXCEPTION)
}

/// Whose fault was a failed apply — **this node's machine, or the bytes**? **Pure.**
///
/// [`refusal_is_deliberate`] answers a different question (*was this a verdict about the
/// event*), and the `false` half of its answer is not one thing. Two callers need it split,
/// and they need it for opposite reasons (PR #493 review):
///
/// * `do_pull`'s freeze arm must add the `local_fault` class when the apply failed on THIS
///   node's own database. Without it a lock storm reaches `bet_a.py` as `integrity` alone —
///   *the peer answered and its DATA is the problem* — which is issue #489 part 1's own
///   sentence, one match arm above the pen write it was filed against;
/// * `do_requeue` must INTERRUPT for a local fault (the operator has something to fix, and
///   every row behind it would meet the same fault anyway) but ANNOTATE-AND-CONTINUE for a
///   byte-attributable one. Halting there wedges the whole recovery command behind one row
///   forever, and `cairn-sync quarantine` is read-only — there is no CLI remedy. The header
///   of `db/001_envelope.sql` records that exact outcome on the pull plane: a class-22
///   `decode()` raise from one buggy peer froze that peer's cursor PERMANENTLY.
///
/// # Why the CLASS, and why `false` is the default
///
/// The two-character class is the part PostgreSQL documents as stable; individual codes
/// are not always. Classes are claimed EXPLICITLY and everything else answers `false`, so
/// an unrecognised code annotates and the sweep keeps going. The asymmetry is deliberate
/// and is the opposite of `cairn-node`'s `pull_failure_class`: a wrong `true` HALTS a run
/// that had work left to do, while a wrong `false` writes one legible, SQLSTATE-carrying
/// line onto one row. The cheaper mistake is the one that keeps moving.
///
/// `XX` (internal_error) is deliberately NOT claimed as local: a pgrx function panicking on
/// adversarial bytes raises it, and that is precisely the case that must not be able to
/// wedge the pen.
///
/// # Why a DELIBERATE refusal cannot be misread as local here
///
/// It would be, if a door ever raised a verdict with `USING ERRCODE` in one of the classes
/// above. None does, and none may: `db/001_envelope.sql`'s header states that **the P0001 is
/// a contract, not an accident of using `RAISE EXCEPTION`** — precisely because the pull
/// loop routes on it — and forbids adding `USING ERRCODE` to any raise. So every SQLSTATE
/// this function sees came from PostgreSQL itself, which is what makes reading its class
/// meaningful at all.
fn apply_failure_is_local(sqlstate: Option<&str>) -> bool {
    match sqlstate {
        // No SQLSTATE at all: the statement never reached a verdict — a dropped
        // connection, a client-side decode failure. Nothing about the bytes was decided.
        None => true,
        // `get(..2)` rather than a slice: a code shorter than two characters (or one that
        // is not ASCII) answers `None` here and falls to `false`, which is the safe side.
        Some(code) => matches!(
            code.get(..2),
            Some(
                "08"    // connection_exception
                    | "40" // transaction_rollback — serialization failure, deadlock
                    | "42" // access rule violation — a revoked grant, a missing table
                    | "53" // insufficient_resources — disk full, out of memory
                    | "55" // object_not_in_prerequisite_state — lock_not_available
                    | "57" // operator_intervention — statement timeout, shutdown
                    | "58" // system_error — an I/O error underneath the database
            )
        ),
    }
}

/// Must this pull cycle fail LOUDLY (a `PullIntegrityError`) rather than exit 0?
///
/// Issue #270: a cycle whose watermark froze used to exit SUCCESS with nothing
/// but a stderr line to show for it, so a wedged peer link accumulated a silent
/// backlog. Every one of these four states means this node is knowingly NOT
/// holding something a peer offered it, so every one of them is loud:
///
/// * `unverifiable` — bytes penned that cannot verify (existing behaviour);
/// * `refused`      — verifiable bytes the floor deliberately refused (#267);
/// * `frozen`       — the cursor halted, so events behind it are not being read (#270);
/// * `pen_failed`   — a refusal we could not even record durably.
///
/// A human `ack` on a pen row is deliberately NOT counted by the caller: an
/// acked exclusion is a recorded decision, not an unresolved refusal.
///
/// TWO states that fit the wording above are deliberately EXCLUDED, for one shared
/// reason: in both, every event the peer offered DID arrive and applied, and what is
/// missing is a sanctioned degradation ABOVE the event set. Failing every cycle for
/// either would turn a known, recorded gap into a link that reads as broken — and an
/// operator who learns that a red pull means "probably the usual thing" is worse off
/// than one with no signal at all. Each gets its own stderr line and its own metric
/// instead — `custody_withheld` in `do_pull`, `references_unlearnable` in BOTH `do_pull`
/// and `do_requeue`, since both admit remote events through the same db/020 door:
///
/// * `custody_withheld` (issue #231) — this node holds sealed bodies it cannot read,
///   the sanctioned ADR-0052 degradation, usually an un-finished peering ceremony;
/// * `references_unlearnable` (issue #465) — an admitted event carried an attachment
///   reference this node could not learn, so its BYTES are unfetchable here while the
///   clinical content is intact. #460 made the apply door admit-and-flag rather than
///   refuse, which is what moved this state out of `refused` and into its own line.
///
/// Neither exclusion is a licence for a third: the test is that the event set is
/// COMPLETE and the loss is declared elsewhere. Anything that leaves this node not
/// holding an event a peer offered is loud.
fn cycle_is_loud(unverifiable: usize, refused: usize, frozen: bool, pen_failed: bool) -> bool {
    unverifiable > 0 || refused > 0 || frozen || pen_failed
}

/// A quarantine-pen write that was refused, and **whose fault that is** (issue #489).
///
/// The pen refuses for two entirely different reasons, and they call for opposite
/// operator actions:
///
/// * **this node's own database** said no — disk full, a grant revoked by a restore, a
///   lock timeout, a dead connection. The peer and the link are fine; the machine under
///   the operator's hands is not;
/// * **the per-peer quota** said no — a resource budget exhausted by the peer's own
///   garbage. That is the mechanism working as designed (`MAX_QUARANTINE_*`), and it is a
///   fact about the PEER.
///
/// Before this, both produced the single class `integrity` — *the peer answered and its
/// DATA is the problem* — so a failing local disk sent someone to audit a peer's
/// signatures, and the Bet A figures learned nothing about this node's own uptime.
#[derive(Debug, Clone)]
struct PenRefusal {
    /// The already-legible refusal text, as it goes into the operator's freeze line.
    message: String,
    /// True iff a `postgres::Error` is reachable in the refusal's chain.
    local_fault: bool,
}

/// Classify a refused pen write. **Pure**, so the routing is testable without a database
/// that refuses on demand.
///
/// The test is *is a `postgres::Error` reachable*, not *does the message look like SQL*:
/// [`quarantine_event`] renders its database failures through [`LocalDbFault`], which
/// keeps the original error as `source()` precisely so this question can be asked. Its
/// quota refusal is a plain `String` error with no chain, so it answers `false` — which
/// is the right answer, not a fallback.
///
/// ⚠️ This is why [`quarantine_event`] must never "tidy" a `LocalDbFault` into a
/// `format!(…).into()`: a `String` error has no `source()`, so a dead local database
/// would silently start reporting as the peer's problem. That helper used to do exactly
/// that (issue #490 item 2).
fn pen_refusal(e: &(dyn Error + 'static)) -> PenRefusal {
    PenRefusal {
        message: e.to_string(),
        local_fault: chain_reaches_a_postgres_error(e),
    }
}

/// Fold a second (third, …) pen refusal from the same cycle into the one already held.
/// **Pure**, so the rule is testable without a batch that has to fail twice in two
/// different ways.
///
/// # The two halves pull in opposite directions
///
/// The **message** is first-wins. It and the class must describe the same event, or the
/// operator reads one refusal's text under another's class (issue #489).
///
/// The **`local_fault` flag** is not about one event at all — it is a fact about this cycle,
/// and about this node's uptime. Plain first-wins dropped it: a QUOTA refusal arriving first
/// (`local_fault: false`) hid a database failure later in the same batch, and the cycle
/// published `integrity` alone while this node's own disk was the thing that failed (PR #493
/// review). So the flag is OR-ed.
///
/// On its own that would leave the text and the class disagreeing, which is exactly what the
/// first-wins rule exists to prevent — so when a LATER refusal is the one that raises the
/// flag, the message says so. Both stay true, and the operator is told which event is which.
fn merge_pen_refusal(first: Option<PenRefusal>, next: PenRefusal) -> PenRefusal {
    let Some(mut first) = first else { return next };
    if next.local_fault && !first.local_fault {
        first.local_fault = true;
        first.message = format!(
            "{} (and a LATER pen write in the same cycle was refused by this node's OWN \
             database: {})",
            first.message, next.message
        );
    }
    first
}

/// A peer request that produced no usable response, **with the transport error kept
/// reachable** (PR #493 review).
///
/// # Why a type rather than the `format!` that was here
///
/// `do_pull`'s request site rendered its failure with `format!(…: {e})`, which produces a
/// `String` error with no `source()`. That is the exact trap [`pen_refusal`]'s doc warns
/// about two hundred lines up, applied to the transport instead of the database — and it
/// had already cost a classification: [`read_frame`] returns `io::ErrorKind::InvalidData`
/// for a length prefix over [`MAX_FRAME_BYTES`](cairn_wire::MAX_FRAME_BYTES), the identical condition `cairn-node`'s
/// `pull_failure_class` calls `Integrity`, and on this plane it fell to `partition`. The
/// two planes gave one failure two operator words, which is what issue #482 was filed to
/// end.
///
/// Flattening is the worse half of that: with no `source()`, the classifier could never be
/// TAUGHT to recognise it, however obvious the fix looked later.
struct PeerRequestError {
    /// The operator-facing sentence, including the pre-#196 skew reading. It is the whole
    /// of `Display`; the cause is reached through `source()`, not spliced into it.
    message: String,
    /// The transport failure itself, kept REACHABLE. `Box<dyn Error>` rather than the
    /// concrete [`cairn_wire::TransportError`] it always holds today, so this struct
    /// never needs to change shape if a future [`Transport`] impl's failure does — an
    /// address resolution failure and a frame read failure are both legitimate here.
    source: Box<dyn Error>,
}

impl std::fmt::Display for PeerRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// `cmd_pull` is reachable as a one-shot, and `fn main() -> R<()>` has no error printer —
/// an `Err` reaches Rust's `Termination`, which prints `Error: {err:?}`. A DERIVED `Debug`
/// would deliver this diagnosis wrapped in struct syntax with the boxed transport error
/// re-dumped beside it, which is the defect `RequeueInterruptedError` and `LocalDbFault`
/// each had to fix already. `Debug` is therefore the sentence, as it is for both of them.
impl std::fmt::Debug for PeerRequestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl Error for PeerRequestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Does `e`'s chain reach an `io::Error` the PEER caused, rather than the link dying?
///
/// The recogniser is `io::ErrorKind::InvalidData`, exactly as on the node plane — see
/// `cairn_node::sync::pull_failure_class`, whose doc carries the full argument for why a
/// KIND and not message text. On this plane exactly one thing produces it: [`read_frame`]
/// refusing a length prefix over [`MAX_FRAME_BYTES`](cairn_wire::MAX_FRAME_BYTES) before allocating. A link that went
/// away produces `ConnectionRefused` / `ConnectionReset` / `UnexpectedEof` / `TimedOut`,
/// none of which reach here.
///
/// Walks rather than downcasting the outermost type, and bounded at the same depth and for
/// the same reason as [`chain_reaches_a_postgres_error`]: the wrapper that names the failing
/// operation must not be able to hide the cause from the classifier.
fn chain_reaches_a_peer_frame_error(e: &(dyn Error + 'static)) -> bool {
    let mut layer = Some(e);
    for _ in 0..8 {
        let Some(cause) = layer else { return false };
        if cause
            .downcast_ref::<io::Error>()
            .is_some_and(|io| io.kind() == io::ErrorKind::InvalidData)
        {
            return true;
        }
        layer = cause.source();
    }
    false
}

/// Compose the operator-facing text of a loud cycle (see [`cycle_is_loud`]).
///
/// Pure and unit-tested because this message IS the product on this path: it is
/// what a human reads at 3am to decide whether a link is broken, wedged, or merely
/// slow, and each of the four loud states calls for a DIFFERENT action. Assembling
/// it from unconditional clauses is how it went wrong before: a transient-fault
/// freeze announced "0 unverifiable and 0 floor-refused event(s) … each is
/// preserved verbatim in sync_quarantine" and pointed at an empty pen, and a
/// pen-QUOTA freeze said "it clears by itself" two sentences after correctly
/// saying it needs an operator ack. Every clause here is therefore conditional on
/// the state that makes it true.
///
/// * `frozen_at`   — `Some(seq)` when the cursor halted, naming the slot it halted at.
/// * `pen_refused` — the pen's own refusal text (quota/insert failure), when that is
///   what froze the cursor. Its presence changes the freeze from self-clearing to
///   operator-action-required, so it is the one clause that rewrites another.
/// * `diagnosis`   — the pre-composed "all shipped events are unverifiable" hint, or
///   empty. Passed in rather than derived here so this stays free of wire types.
fn loud_pull_message(
    peer_name: &str,
    unverifiable: usize,
    refused: usize,
    frozen_at: Option<i64>,
    pen_refused: Option<&str>,
    diagnosis: &str,
) -> String {
    let penned = unverifiable + refused;
    // The lead. Only claim durable bytes when some were actually penned.
    let lead = if penned > 0 {
        format!(
            "{unverifiable} unverifiable and {refused} floor-refused event(s) this cycle; \
             each is preserved verbatim in sync_quarantine and its slot is held on the \
             re-offer floor (nothing lost; valid events still applied)."
        )
    } else {
        "this cycle ended holding LESS than the peer offered, and nothing was penned to \
         show for it."
            .to_string()
    };
    let pen = match pen_refused {
        Some(qe) => format!(" Quarantine pen refused (cursor frozen): {qe}"),
        None => String::new(),
    };
    // The freeze half (#270). A halted cursor means every event BEHIND the frozen
    // slot is withheld too, so it is the loudest of the states. Whether it clears
    // by itself depends entirely on WHY it froze — a transient fault does, a pen
    // that refused the write does not.
    let freeze = match (frozen_at, pen_refused.is_some()) {
        (Some(at), false) => format!(
            " The seq cursor is FROZEN at {at}: events after that slot are NOT being read \
             from this peer and the backlog grows every cycle. This is a transient-fault \
             hold (a deliberate floor refusal is penned instead), so it clears by itself \
             once the underlying fault does — if it does not, the stderr line above names \
             the failure."
        ),
        (Some(at), true) => format!(
            " The seq cursor is FROZEN at {at}: events after that slot are NOT being read \
             from this peer and the backlog grows every cycle. This hold does NOT clear by \
             itself — the pen refusal above is its cause, and it lifts only once the \
             operator action that message names has been taken."
        ),
        (None, _) => String::new(),
    };
    // The remedies, which are all about the pen — so they belong only where the pen
    // is the thing to act on. A pure transient freeze has nothing to inspect.
    let remedy = if penned > 0 || pen_refused.is_some() {
        " Inspect with `cairn-sync quarantine`; a repaired peer — or a repaired floor here \
         (enrol the author, take the code-plane update) — is picked up automatically; to \
         accept a permanent exclusion, ack the row: UPDATE sync_quarantine SET acked = TRUE \
         WHERE content_digest = …"
    } else {
        ""
    };
    format!("pull {peer_name}: {lead}{diagnosis}{pen}{freeze}{remedy}")
}

/// The operator line for a failed cursor commit (issue #469). Pure, so it is testable.
///
/// Three things this must say, because each redirects a different wrong instinct:
///
/// 1. **the applied work is not lost** — the events applied in their own transactions and
///    are durable in `event_log`; only the bookmark write failed;
/// 2. **it is THIS node, not the link** — the whole reason the error class exists. An
///    operator who reads "pull failed" reaches for the network first;
/// 3. **it self-heals if the fault does** — the next cycle re-offers from wherever the
///    cursor actually is and re-applies idempotently. That is the difference between
///    "watch it" and "act now", and it is not guessable from the failure alone.
///
/// What it must NOT say is that the cursor did not advance. An earlier draft did (PR #472
/// review, finding 4), which flatly contradicted [`mark_cursor_outcome_unknown`] thirty
/// lines below: that function nulls `cursor_seq` precisely BECAUSE a statement may have
/// committed before the connection broke, so this node cannot know. Publishing the null
/// and then asserting the certainty in the prose an operator actually reads is principle 4
/// applied to the metric and dropped where it counts — and it invites a manual cursor
/// reset on a cursor that may well have moved. The claim is hedged; the SELF-HEALING is
/// not, because it holds either way.
///
/// `loud` carries the SEPARATE integrity diagnosis when the same cycle was also
/// `cycle_is_loud`. Both travel or the operator gets half a picture, with this line's
/// "re-applies idempotently" as the remedy for a quarantined event it does not fit.
fn cursor_commit_failure_message(
    peer_name: &str,
    applied: usize,
    attempted_seq: i64,
    cause: &str,
    loud: Option<&str>,
) -> String {
    let also = loud
        .map(|l| format!(" AND, INDEPENDENTLY: {l}"))
        .unwrap_or_default();
    format!(
        "pull {peer_name}: {applied} event(s) applied and durable, but committing the \
         sync_state cursor to seq {attempted_seq} FAILED — this node's own DATABASE, not \
         the peer and not the link: {cause}. The applied events are not lost. Whether the \
         cursor MOVED is unknown — the statement may have committed before the failure — \
         so both cursor fields report null rather than guess; the next cycle re-offers \
         from wherever it actually is and re-applies idempotently either way. Fix the \
         local database fault; if it recurs every cycle the backlog grows.{also}"
    )
}

/// Mark the metrics fields that describe the CURSOR COMMIT as unknown (issue #469).
///
/// `cursor_seq` and `floor_active` are not observations of this cycle's work — they are
/// claims about the state of `sync_state` *after the write that just failed*. Reporting
/// the values that write would have set is a precise untruth in the reassuring direction:
/// a monitor watching `cursor_seq` sees a cursor that advanced when it did not. And if the
/// connection dropped mid-statement, this node genuinely cannot know which happened, which
/// is the case principle 4 exists for.
///
/// So both go to `null` — the same rule, one field over, as `references_unlearnable`'s
/// null-never-zero (a number is a claim; absence of knowledge is not a number). The seq we
/// *tried* to commit is named in [`cursor_commit_failure_message`] instead, where it reads
/// as an attempt rather than an outcome.
///
/// Everything else in the object describes work that genuinely happened before the commit
/// — how many events applied, what was penned, what was withheld — and is left untouched.
fn mark_cursor_outcome_unknown(metrics: &mut serde_json::Value) {
    metrics["cursor_seq"] = serde_json::Value::Null;
    metrics["floor_active"] = serde_json::Value::Null;
}

/// Which failure classes is this, and what metrics survive it? (issue #469.)
///
/// Pure and total, so the mapping is unit-testable over constructed errors instead of
/// being reachable only through a live pull against a broken database. Returns the
/// log-line KEYS to set (`"integrity"` / `"local_fault"` / `"partition"`) and the metrics
/// to publish alongside them — `Null` when the failure carried none.
///
/// **Keys, plural.** A cursor-commit failure on a cycle that ALSO quarantined unverifiable
/// events is genuinely both, and collapsing it to one key drops a condition that needs a
/// human (PR #472 review, finding 5).
///
/// **A raw `postgres::Error` is a LOCAL fault, not a partition** (PR #472 review, finding
/// 3). `do_pull` reaches its peer over a raw `TcpStream`, so the only postgres error that
/// can escape it is one from THIS node's database — and `do_pull`'s first two statements
/// (the `sync_state` upsert and read) are bare `?` on exactly that, before any network I/O
/// happens at all. Without this arm they fell to the default and were logged as link
/// downtime, which is issue #469's own title sentence, two statements above the fix.
/// This matters more than the cursor-commit case it generalises: `cmd_run` builds its
/// client ONCE, outside the loop, so after the database goes away every later cycle fails
/// at that first statement for the life of the process.
///
/// The default arm is `partition`, and it is deliberately LAST: a failure this function
/// does not recognise is, by elimination, one where the peer did not answer. Every failure
/// that IS recognised must therefore be claimed explicitly above, or it silently becomes a
/// partition — which is exactly the defect #469 records.
///
/// The caveat this used to carry — *`downcast_ref` does NOT walk `source()`, so a
/// `.map_err` or a retry wrapper anywhere on this path turns the arms back into
/// `partition`, silently* — is retired for the postgres arm, which now walks the chain
/// (see [`chain_reaches_a_postgres_error`]). It still binds the two arms ABOVE it, and
/// deliberately so: those match the outermost type because a database error found
/// *beneath* a `PullIntegrityError` or a `CursorCommitError` must not re-label it.
fn classify_pull_failure(
    e: &(dyn Error + 'static),
) -> (&'static [&'static str], serde_json::Value) {
    if let Some(ie) = e.downcast_ref::<PullIntegrityError>() {
        // Both, when the pen write was refused by this node's own database (#489 part 1):
        // the peer sent something we could not admit AND we could not record that fact.
        let classes: &'static [&'static str] = if ie.also_local_fault {
            &["integrity", "local_fault"]
        } else {
            &["integrity"]
        };
        return (classes, ie.metrics.clone());
    }
    if let Some(ce) = e.downcast_ref::<CursorCommitError>() {
        let classes: &'static [&'static str] = if ce.also_loud {
            &["local_fault", "integrity"]
        } else {
            &["local_fault"]
        };
        return (classes, ce.metrics.clone());
    }
    if chain_reaches_a_postgres_error(e) {
        // No metrics: these escape BEFORE the metrics object is built, so there is
        // nothing to report about this cycle beyond the class and the error text.
        return (&["local_fault"], serde_json::Value::Null);
    }
    // The peer answered with a frame this build refuses (an over-cap length prefix), which
    // is the node plane's `Integrity` by another name — PR #493 review. Checked LAST, after
    // both typed classes and after the postgres walk, for the reason that walk's own doc
    // gives: an arm that matches on a KIND must never be able to re-label something a more
    // specific arm has already claimed.
    if chain_reaches_a_peer_frame_error(e) {
        return (&["integrity"], serde_json::Value::Null);
    }
    (&["partition"], serde_json::Value::Null)
}

/// Does a `postgres::Error` sit anywhere in `e`'s `source()` chain?
///
/// # Why this walks, when the two arms above do not (issue #479)
///
/// `downcast_ref` on a `dyn Error` inspects the OUTERMOST type only. That was fine while
/// every local database failure on this path was a bare `?` — and it made the one
/// improvement anybody would reach for next, *naming the operation that failed*, into a
/// silent regression: a `LocalDbFault` (or any `.map_err`, or a retry wrapper) pushed the
/// `postgres::Error` one layer down and the classifier fell through to `partition`,
/// sending an operator to the WAN for a dead local database and charging the Bet A
/// availability figure for it. That is #469's own defect, reinstated by its own fix. The
/// classifier's header used to carry this as a caveat to be careful about; walking the
/// chain removes it instead.
///
/// **This is deliberately the LAST class checked**, after `PullIntegrityError` and
/// `CursorCommitError`, both of which are matched on the outermost type and would be
/// wrongly re-labelled if a database error were found beneath them. Order is the whole
/// safety argument here.
///
/// Safe to widen this far because of where `do_pull` sits: it reaches its peer over a raw
/// `TcpStream`, so every non-database failure on this path is an `io::Error` or a `String`
/// — neither of which can carry a `postgres::Error` underneath it. The hop limit matches
/// [`kind_and_causes`] and [`operator_chain`]: a cyclic chain must not hang the daemon.
fn chain_reaches_a_postgres_error(e: &(dyn Error + 'static)) -> bool {
    let mut layer = Some(e);
    for _ in 0..8 {
        let Some(cause) = layer else { return false };
        if cause.is::<postgres::Error>() {
            return true;
        }
        layer = cause.source();
    }
    false
}

/// Write a failed fingerprint probe into the cycle's log line. **Pure.** (PR #486 review.)
///
/// # Why the `eprintln!` was not enough (issue #479, second half)
///
/// #479 replaced `if let Ok(fp) = do_fingerprint(...)` — which had no `else` arm at all — with
/// a `match` whose `Err` arm printed [`fingerprint_error_line`] to stderr. That names the
/// missing thing and its consequence, and [`fingerprint_error_line`]'s own doc claims the gap
/// in the log is thereby self-explaining. It is not: the gap is in `run.jsonl` and the
/// explanation was on stderr. `bet_a.py` reads the JSONL —
///
/// ```python
/// fps = [r["fingerprint"] for r in rows if "fingerprint" in r]
/// ```
///
/// — so under `nohup`, or a systemd unit whose stderr is discarded or separately rotated, a
/// schema skew at cycle 3 leaves it computing a clean-looking convergence series over cycles
/// 1-2 with no evidence in the artifact at all.
///
/// # `null`, never absent
///
/// The same rule [`mark_cursor_outcome_unknown`] applies one field over, for the same reason:
/// a key that VANISHES cannot be told from a key nobody wrote, while an explicit `null` is a
/// claim that we looked and do not know. Absence was precisely #479's defect here — so the
/// failed probe sets the key to `null` AND records why beside it, rather than leaving a reader
/// to infer both from a hole.
///
/// The probe stays non-fatal: a node that cannot fingerprint itself must still pull.
fn record_fingerprint_failure(line: &mut serde_json::Value, e: &(dyn Error + 'static)) {
    line["fingerprint"] = serde_json::Value::Null;
    line["fingerprint_error"] = serde_json::json!(operator_chain(e));
}

/// Write one failed cycle's classification into its log line. (PR #472 review, finding 8.)
///
/// Extracted from `cmd_run` for the same reason `classify_pull_failure` was extracted from
/// it: this is the seam where the whole three-class distinction becomes OBSERVABLE — it
/// is what `bet_a.py` reads — and while the classifier was well tested, the four lines
/// that used it were reachable only by running the daemon. A dropped `line["pull"]`, a
/// hardcoded key, or a swapped class would have passed the entire suite.
fn record_pull_failure(line: &mut serde_json::Value, e: &(dyn Error + 'static)) {
    // `operator_chain`, never `to_string()`: a `postgres::Error`'s own `Display` is the
    // literal two words `db error`, and this key is read by a machine as well as a human
    // (issue #479).
    line["pull_error"] = serde_json::json!(operator_chain(e));
    let (classes, metrics) = classify_pull_failure(e);
    for class in classes {
        line[*class] = serde_json::json!(true);
    }
    // Null only for the classes that failed before any work was measurable; every other
    // class publishes what the cycle actually did.
    if !metrics.is_null() {
        line["pull"] = metrics;
    }
}

/// The operator line for a peer that withheld custody for a batch (issue #231).
///
/// Extracted from `do_pull` and made pure so its ESCAPING is testable (#466 review). The
/// substance is unchanged: this node still applies every event it was offered — withhold
/// the key, never the bytes — but it now knows why the sealed bodies it just stored will
/// not render, instead of logging a cycle byte-identical to a healthy one while a
/// clinician reads an empty medication list. The remedy travels in the line.
///
/// ⚠️ `reason` IS UNBOUNDED PEER PROSE AND IS ESCAPED FOR THAT REASON. It is an
/// `Option<String>` deserialized straight off the wire, deliberately served to a peer the
/// trust set has NOT admitted, with no length bound and no validation. Printed raw, a
/// newline in it forges a whole line in a line-oriented log — and the most valuable line
/// to forge is now `unlearnable_reference_message`'s, because a fabricated
/// "0 attachment reference(s)" reads as an all-clear on the alarm issue #465 just
/// installed. `{:?}` is the same treatment `unlearnable_reference_message` gives the
/// ledger reason, and the peer name needs none: it is this operator's own `--peer-name`.
fn custody_withheld_message(peer_name: &str, reason: &str) -> String {
    format!(
        "pull {peer_name}: the peer WITHHELD CUSTODY for this batch — its sealed bodies \
         replicate here but stay UNREADABLE until this is fixed. The serving node \
         reported: {reason:?}"
    )
}

// ---------------------------------------------------------------------------
// Unlearnable attachment references — the signal admit-and-flag owed (issue #465)
// ---------------------------------------------------------------------------
//
// WHAT CHANGED UNDER US. Before #460, a peer event carrying a malformed rendition
// reference was REFUSED by the apply door (P0001), penned in `sync_quarantine`, counted
// as `refused_verifiable`, and turned this whole cycle into a `PullIntegrityError` an
// operator could not miss. #460 made the remote door ADMIT and FLAG instead — correct,
// because refusing a FIELD at the apply door drops the clinical act it rode on and forks
// the event set (ADR-0063; db/050's header carries the full argument). But the loud
// signal was removed and nothing replaced it: the same defect now produces a successful
// cycle, exit 0, a log line byte-identical to a healthy pull, and a row in a table
// nothing reads.
//
// THE SHAPE, AND WHOSE PRECEDENT IT FOLLOWS. `custody_withheld` (issue #231) is the
// same situation one level up — a degradation this node must know about that must NOT
// fail the cycle — and it got its own metric AND its own stderr line. So does this. It
// is deliberately NOT folded into `cycle_is_loud`: see that function's doc for why a
// second excluded state is not a slippery slope but the same argument twice.
//
// NAME, NEVER COUNT (the Slice 69 rule). "3 events have bad references" cannot tell an
// operator whether to chase one peer's encoder bug or one corrupted import — which is
// the question db/050's own header poses — so the line carries an example event id and
// the accessor's own refusal text, and points at the standing node-wide ledger.

/// What ONE run discovered about attachment references it could not learn.
///
/// `count` counts LEDGER ROWS, which in the common case is one per unlearnable rendition:
/// a single note can carry a whole rendition set (original + preview + extracted text),
/// and how much of it is broken is what distinguishes "one bad file" from "this peer's
/// encoder is emitting garbage".
///
/// It is a FLOOR, not an exact reference count, and the difference is db/050's three row
/// shapes: `(i, NULL)` records one row for an attachment whose whole `renditions` list was
/// unparseable, and `(NULL, NULL)` one row for an unparseable `attachments` — in both, one
/// row stands for an unknown number of lost references. Calling it exact would be the kind
/// of precise untruth principle 4 is about, on the surface that argues for principle 4.
#[derive(Debug)]
struct UnlearnableReferences {
    count: i64,
    /// The EARLIEST flag of this run, ordered by `(flagged_at, flag_id)` — the SAME key
    /// `cairn_attachment_flag_health()` uses (db/050), so an operator who follows this
    /// line to the standing ledger is shown the same event rather than a different one.
    example_event: String,
    /// That flag's recorded reason — the db/027 accessor's own refusal message and its
    /// DETAIL, which is the half that tells truncation from wrong-encoding.
    example_reason: String,
}

/// The operator-facing line for a run that admitted references it cannot fetch.
///
/// Pure and unit-tested for the same reason [`loud_pull_message`] is: this text IS the
/// product on this path. What it must convey, in order: that the events themselves are
/// NOT lost (the opposite of what the refusal it replaces meant), which reference is
/// broken and why, and where to look for the standing picture.
///
/// `context` is the caller's own prefix — `"pull node-a"` or `"requeue"` — because TWO
/// paths admit remote events through db/020 and both can discover an unlearnable
/// reference. Passing it in keeps this function ignorant of which one it is serving.
///
/// ⚠️ THE REASON IS PEER TEXT AND IS PRINTED WITH `{:?}` FOR THAT REASON. db/001's
/// `cairn_decode_hex_or_raise` echoes a bounded PREFIX of the peer's own value into its
/// refusal message (capped at 8 characters, so a short secret is never reproduced whole),
/// and db/050 records that message verbatim. The prefix is not what tells a truncated
/// digest from a wrongly-encoded one — the DETAIL is, and db/050 keeps it; the prefix is
/// there so a `0x` marker or a leading `-` survives into the diagnosis. Either way it is
/// PEER-CHOSEN TEXT, and a newline in it would forge a whole line in a line-oriented log
/// — the Slice 69 finding, on a new surface. `{:?}` escapes control characters (newline,
/// carriage return and ESC alike, so ANSI injection is blocked too) while leaving ordinary
/// text readable — the idiom `sensitivity::render::peer` uses in cairn-node. The event id
/// needs no such treatment: it is a `uuid` rendered by Postgres, not a string a peer chose.
fn unlearnable_reference_message(context: &str, found: &UnlearnableReferences) -> String {
    format!(
        "{context}: {} attachment reference(s) in this run could not be learned. \
         The events themselves ARE in the record and their clinical content is intact — \
         only the referenced bytes are unfetchable here, and stay so until the author \
         re-issues the reference (the malformed field sits inside the signature, so it \
         cannot be repaired in place). Example: event {} — {:?}. Standing picture: \
         `SELECT * FROM cairn_attachment_flag_health()`.",
        found.count, found.example_event, found.example_reason
    )
}

/// Turn the ledger read's outcome into `(metric value, optional operator line)`.
///
/// Pure, so all three outcomes are testable without a database — and the third is the
/// one worth the function: a read that FAILED reports the metric as JSON `null`, never
/// as `0`. Zero is a claim ("I looked, there were none"); after a failed read it is a
/// precise untruth in the reassuring direction on an operator surface, and it would let
/// a monitor alerting on `references_unlearnable > 0` fall silent exactly when this node
/// has stopped being able to see (principle 4: an honest unknown beats a precise
/// untruth). The failed read is itself never silent — it gets a line of its own.
fn unlearnable_report(
    context: &str,
    read: Result<Option<UnlearnableReferences>, String>,
) -> (serde_json::Value, Option<String>) {
    match read {
        Ok(None) => (serde_json::json!(0), None),
        Ok(Some(found)) => (
            serde_json::json!(found.count),
            Some(unlearnable_reference_message(context, &found)),
        ),
        Err(e) => (
            serde_json::Value::Null,
            Some(format!(
                // `{e:?}` for the same reason the sibling arm escapes the reason: a
                // server message can carry DETAIL/HINT lines and, through them, peer
                // text — so the arm that reports a FAILURE must not be the one that
                // forges a line.
                "{context}: could not read the attachment-reference ledger for this \
                 run: {e:?}. The `references_unlearnable` metric is UNKNOWN (null) rather \
                 than zero — the run itself is unaffected and its events are applied. \
                 Check the standing picture directly: \
                 `SELECT * FROM cairn_attachment_flag_health()`."
            )),
        ),
    }
}

/// Produce the metric AND write the operator line, in ONE place.
///
/// The emission lives here rather than at each call site deliberately (issue #466 review):
/// this is the only producer of the `references_unlearnable` value, so a caller cannot
/// keep the metric while dropping the line, and the line itself is unit-testable through
/// `out` instead of being an `eprintln!` no test can see. Callers pass `io::stderr()`; the
/// tests pass a `Vec<u8>`.
///
/// A write failure is swallowed on purpose. This runs AFTER the cursor commit, and a
/// broken stderr must not turn a report about completed work into a failed cycle — the
/// opposite of the "never fail the cycle for this" rule the whole surface is built on.
fn emit_unlearnable_report(
    out: &mut dyn Write,
    context: &str,
    read: Result<Option<UnlearnableReferences>, String>,
) -> serde_json::Value {
    let (metric, line) = unlearnable_report(context, read);
    if let Some(line) = line {
        let _ = writeln!(out, "{line}");
    }
    metric
}

/// A `postgres::Error` rendered for a human instead of the eight characters it gives.
///
/// `postgres::Error`'s `Display` is a bare match on its kind — a server-side failure of
/// ANY sort renders as the literal string `"db error"`, and `Display` does not chain to
/// the source that holds the message, the DETAIL, the HINT and the SQLSTATE. This file has been
/// bitten by that twice before ([`ApplyError::from`], and `quarantine_event`'s local
/// renderer), and #467 records the same species one crate over. On THIS surface it would
/// be worse than elsewhere: the whole point of the `null`-not-zero arm is that a failed
/// read is never silent, and a line that says only `db error` cannot tell a missing
/// db/050 from a revoked grant from a statement timeout — three different remedies.
///
/// The SQLSTATE is included because it is the part an operator can search for, and the
/// DETAIL because on the doors it is where the reason actually lives (issue #109).
/// Flatten a server-supplied string onto ONE line. **Pure.**
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Compose the operator-facing text of a server-side failure. **Pure.**
///
/// `message [SQLSTATE] — DETAIL — HINT: hint`. DETAIL is where this project's own
/// doors put the reason (issue #109); HINT is where POSTGRES puts the remedy, and it is
/// labelled because unlike DETAIL it is advice rather than fact. Each separator travels
/// with its own part, so an absent part leaves no dangling em dash.
///
/// Every part is flattened onto one line: a DETAIL/HINT is routinely multi-line, these
/// renderings land in a one-line-per-event operator log, and a raw newline would let a
/// server — or, through the bounded peer prefixes the doors splice into their messages, a
/// PEER — forge a whole log line. `unlearnable_report` below escapes for the same reason.
///
/// **Twin.** `cairn-node`'s `db_diagnosis::compose_db_diagnosis` composes the byte-identical
/// shape, and `the_twin_composes_the_agreed_shape` here asserts the same strings its
/// `the_agreed_shape_is_exactly_this` does, so drift between the two goes red.
fn compose_db_diagnosis(
    message: &str,
    sqlstate: &str,
    detail: Option<&str>,
    hint: Option<&str>,
) -> String {
    let message = one_line(message);
    let detail = detail
        .map(|d| format!(" — {}", one_line(d)))
        .unwrap_or_default();
    let hint = hint
        .map(|h| format!(" — HINT: {}", one_line(h)))
        .unwrap_or_default();
    format!("{message} [{sqlstate}]{detail}{hint}")
}

/// Name the kind of a non-server failure **and every cause beneath it**. **Pure.**
///
/// `postgres::Error`'s `Display` is a bare match on KIND for EVERY kind, not just
/// `Kind::Db`, and it never consults `source()`. So `error connecting to server` is the
/// whole of what it says about a refused socket, an unresolvable host and a TLS timeout
/// alike — a category, not a diagnosis, and `db error` one kind over. The cause is
/// reachable; it is just not in `Display`. (PR #472 review, finding 1.)
///
/// The hop limit is belt-and-braces: a cyclic source chain would otherwise spin forever,
/// and no diagnostic is worth hanging the daemon over.
fn kind_and_causes(e: &postgres::Error) -> String {
    let mut text = one_line(&e.to_string());
    let mut source = std::error::Error::source(e);
    for _ in 0..8 {
        match source {
            Some(cause) => {
                text.push_str(": ");
                text.push_str(&one_line(&cause.to_string()));
                source = cause.source();
            }
            None => break,
        }
    }
    text
}

fn legible_db_error(e: &postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => compose_db_diagnosis(db.message(), db.code().code(), db.detail(), db.hint()),
        // Not a server error at all (refused socket, connection closed, TLS, encode): the
        // kind names the LAYER that failed, the chain beneath it names the failure.
        None => kind_and_causes(e),
    }
}

/// Render a whole `Box<dyn Error>` chain as ONE operator line, with each layer said once.
/// **Pure.**
///
/// # The defect this exists for (issue #479)
///
/// `postgres::Error`'s `Display` is the literal string `db error`, and a `Box<dyn Error>`
/// delegates its own `Display` straight to it. So when this node's database died, the run
/// loop's terminal line and the machine-readable JSONL that `bet_a.py` reads both said:
///
/// ```text
/// cycle 118: PULL FAILED: db error
/// {"cycle":118,"pull_error":"db error","local_fault":true}
/// ```
///
/// `53300` (connections exhausted), `57P01` (admin shutdown), `42501` (a grant revoked by a
/// restore) and a dead socket were one indistinguishable string — and `cmd_run` builds its
/// client ONCE, outside the loop, so after the database goes away this repeats every cycle
/// for the life of the process. #469 fixed the *class* and never the *message*; its test
/// asserted only the class, which is why this survived it.
///
/// # The rule, in three parts
///
/// * a `postgres::Error` is rendered through [`legible_db_error`], never as its kind;
/// * a layer is **dropped when the layer above already ends with it** — a suffix test, not
///   a type test, so any future self-rendering wrapper inherits it. [`LocalDbFault`] is
///   today's case: its `Display` already embeds the rendered cause;
/// * the walk **stops at the first `postgres::Error`**, because [`legible_db_error`] has
///   already consumed that error's whole source subtree (the `Kind::Db` arm reads the
///   `DbError`'s message, SQLSTATE, DETAIL and HINT; every other arm walks `source()`
///   itself through [`kind_and_causes`]). Descending further printed the server's message
///   twice on one line — measured, and fixed in the `cairn-node` twin in the same commit.
///
/// **Twin.** `cairn-node`'s `db_diagnosis::operator_chain` is the same three rules over an
/// `anyhow` chain. The difference is structural rather than stylistic and is worth knowing
/// before porting between them: `anyhow::Error::chain()` walks `source()` for you, while
/// here the walk is explicit — and `downcast_ref` on a `dyn Error` does NOT walk, which is
/// the caveat [`classify_pull_failure`] carries for the same reason.
///
/// The hop limit matches [`kind_and_causes`]: a cyclic source chain would otherwise spin
/// forever, and no diagnostic is worth hanging the daemon over.
fn operator_chain(e: &(dyn Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut layer = Some(e);

    for _ in 0..8 {
        let Some(cause) = layer else { break };
        let pg = cause.downcast_ref::<postgres::Error>();
        let rendered = match pg {
            Some(pg) => legible_db_error(pg),
            None => one_line(&cause.to_string()),
        };
        // An empty layer would make the suffix test vacuously true for everything after
        // it, so it is dropped outright rather than reasoned about.
        if !rendered.is_empty() && !parts.last().is_some_and(|above| above.ends_with(&rendered)) {
            parts.push(rendered);
        }
        if pg.is_some() {
            // Stop: `legible_db_error` already consumed this error's whole source subtree.
            break;
        }
        layer = cause.source();
    }
    parts.join(": ")
}

/// A failure of **this node's own database**, rendered for a human without throwing away
/// the error underneath it (issue #479). The `cairn-sync` twin of `cairn-node`'s
/// `db_diagnosis::LocalDbFault`.
///
/// # Why a type and not a `format!(…).into()`
///
/// Every other bare `?` on this path could have been fixed by rendering into a `String`
/// error at the site. Not these. [`classify_pull_failure`] tells *this node's database
/// failed* from *the peer did not answer* by looking for a `postgres::Error`, and a
/// `String` error has no `source()` — so rendering into one would have silently reverted
/// every local fault to `partition`, which is #469's defect reinstated inside the fix for
/// its sibling. That is the trap in this change most likely to be sprung in good faith.
///
/// So this type does both jobs at once: `Display` is the legible rendering an operator
/// reads, and `source()` is the original error the classifier can find.
///
/// # Why `Debug` is hand-written
///
/// `fn main() -> R<()>` has no error printer, so an `Err` reaching Rust's `Termination`
/// is printed as `Error: {err:?}`. A derived `Debug` would deliver the diagnosis wrapped
/// in struct syntax with the `postgres::Error` re-dumped beside it — the same defect the
/// PR #478 review found in `RequeueInterruptedError`, which is why that type does this too.
struct LocalDbFault {
    /// What this node was DOING — an operator-facing phrase, not a SQL fragment. A
    /// SQLSTATE names the condition; only the caller knows which statement met it.
    doing: String,
    /// [`legible_db_error`] of [`Self::source`], rendered once at construction.
    rendered: String,
    /// The original error, kept REACHABLE. Dropping it is what breaks the classifier.
    source: postgres::Error,
}

impl LocalDbFault {
    fn new(doing: impl Into<String>, source: postgres::Error) -> Self {
        let rendered = legible_db_error(&source);
        Self {
            doing: doing.into(),
            rendered,
            source,
        }
    }

    /// Box it as the crate's error type, which is what every call site actually wants.
    fn boxed(doing: impl Into<String>, source: postgres::Error) -> Box<dyn Error> {
        Box::new(Self::new(doing, source))
    }
}

impl std::fmt::Display for LocalDbFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.doing, self.rendered)
    }
}

impl std::fmt::Debug for LocalDbFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl Error for LocalDbFault {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

/// The ledger's high-water mark, read BEFORE a run admits anything.
///
/// Half of the "what did THIS run discover" key; see [`unlearnable_references`] for why a
/// watermark alone is not enough and an address list alone is not either. `flag_id` is a
/// `GENERATED ALWAYS AS IDENTITY` primary key, so "greater than the mark" is exactly
/// "recorded after we looked" — no clock is involved, which matters on a node whose
/// wall-clock confidence is itself a graded, uncertain thing (ADR-0027).
///
/// An empty ledger reads as `0`, and `flag_id` starts at 1, so a fresh node reports
/// everything its first run flags rather than nothing.
fn attachment_flag_watermark(client: &mut postgres::Client) -> Result<i64, postgres::Error> {
    Ok(client
        .query_one(
            "SELECT COALESCE(max(flag_id), 0)::bigint FROM attachment_reference_flag",
            &[],
        )?
        .get(0))
}

/// Read db/050's ledger for what THIS run just discovered.
///
/// TWO keys, and neither is sufficient alone — this is the #466 review's finding, and the
/// earlier single-key version got a silent zero exactly where the signal mattered most:
///
/// * **`content_address = ANY(applied)`** attributes exactly. A concurrent pull from a
///   DIFFERENT peer must not have its flags reported under this peer's name; on a surface
///   whose whole job is to name things, that would be a precise untruth.
/// * **`flag_id > since`** is what makes it this run's NEWS. The first version keyed on
///   newly-INSERTED events instead, reasoning that a foreign key stops a flag preceding
///   its event. True, and beside the point: db/020 calls
///   `cairn_learn_attachment_refs_lenient` UNCONDITIONALLY, after its
///   `ON CONFLICT (event_id) DO NOTHING` and with no `v_rows` guard — so the learner runs
///   on an idempotent RE-APPLY too, and can write a brand-new flag row for an event this
///   node already held. Keyed on newness, every such row was invisible: a node upgrading
///   onto db/050 flags its whole pre-#460 back-catalogue on the next full sweep
///   (`FULL_SWEEP_EVERY`) and would have reported `0` for all of it. So the caller now
///   records the address of EVERY event the door admitted, new or re-offered, and the
///   watermark — not newness — is what keeps a re-delivery quiet: re-delivery writes no
///   row (db/050's `ON CONFLICT DO NOTHING`), so its flag stays below the mark.
///
/// Together: one alert when a defect is DISCOVERED, silence on every re-offer after,
/// and never another peer's defect charged to this one.
///
/// Returns `Ok(None)` when nothing new was flagged — including the ordinary case of a run
/// that admitted nothing at all, which skips the query entirely.
fn unlearnable_references(
    client: &mut postgres::Client,
    applied: &[Vec<u8>],
    since_flag_id: i64,
) -> Result<Option<UnlearnableReferences>, postgres::Error> {
    if applied.is_empty() {
        return Ok(None);
    }
    // ORDER BY (flagged_at, flag_id) rather than flag_id alone, to match
    // cairn_attachment_flag_health() (db/050): the line's closing move is to send the
    // operator to that ledger, and two orderings would show them a different example
    // event there than the one they were just told about.
    let row = client.query_one(
        "SELECT count(*)::bigint,
                (array_agg(f.event_id::text ORDER BY f.flagged_at, f.flag_id))[1],
                (array_agg(f.reason         ORDER BY f.flagged_at, f.flag_id))[1]
           FROM attachment_reference_flag f
           JOIN event_log e USING (event_id)
          WHERE e.content_address = ANY($1)
            AND f.flag_id > $2",
        &[&applied, &since_flag_id],
    )?;
    let count: i64 = row.get(0);
    if count == 0 {
        return Ok(None);
    }
    // With count > 0 at least one row aggregated, so neither aggregate can be NULL
    // (db/050 declares both columns NOT NULL). If that ever stops being true, say what
    // happened rather than substituting a plausible-looking value: a bare "unknown" is
    // indistinguishable from a peer whose reason text is literally "unknown", and it
    // would fire a monitor with nothing to chase.
    let missing = || "<ledger returned NULL — read cairn_attachment_flag_health()>".to_string();
    Ok(Some(UnlearnableReferences {
        count,
        example_event: row.get::<_, Option<String>>(1).unwrap_or_else(missing),
        example_reason: row.get::<_, Option<String>>(2).unwrap_or_else(missing),
    }))
}

/// Why the in-DB apply door said no, with the SQLSTATE preserved.
///
/// `apply_signed` used to flatten `postgres::Error` into a `String`, which threw
/// away the one fact [`refusal_is_deliberate`] needs. This keeps both: the
/// legible message (the door's own RAISE text plus its DETAIL — the issue #109
/// skew-vs-tampering diagnosis, which must keep reaching the pen reason and the
/// freeze log lines) and the machine-readable code.
#[derive(Debug)]
struct ApplyError {
    message: String,
    sqlstate: Option<String>,
    /// The original error, kept REACHABLE — the same discipline [`LocalDbFault`] follows
    /// and for the same reason (issue #480).
    ///
    /// `Display` here is the DOOR's vocabulary (`message (detail)`), which is what belongs
    /// in `sync_quarantine.reason` and `last_requeue_error`. When the failure turns out
    /// NOT to be a door verdict at all, the operator needs the *database's* vocabulary
    /// instead — SQLSTATE included — and that is only renderable from the error itself.
    /// **What this field is and is not load-bearing for**, because the first draft of this
    /// doc claimed more than the code does (PR #493 review). It IS what `operator_text()`
    /// renders from, and it is what keeps the `source()` chain unbroken so a future walker
    /// — or `operator_chain`, if an `ApplyError` ever reaches one — finds the database
    /// error rather than a dead end, which is the trap that cost `quarantine_event` its
    /// diagnosis (#490). It is NOT how either of this type's two routing decisions is made:
    /// both `is_deliberate_refusal` and `apply_failure_is_local` read `sqlstate`, which says
    /// strictly more than "a postgres error is reachable".
    source: postgres::Error,
}

impl ApplyError {
    /// True iff the floor decided against these bytes (see [`refusal_is_deliberate`]).
    fn is_deliberate_refusal(&self) -> bool {
        refusal_is_deliberate(self.sqlstate.as_deref())
    }

    /// Render the underlying failure in the ONE format every local database fault in this
    /// daemon uses (`message [SQLSTATE] — DETAIL — HINT`).
    ///
    /// For callers that have already established this was not a door verdict: the
    /// `Display` above deliberately omits the SQLSTATE, because a floor refusal's SQLSTATE
    /// is always `P0001` and says nothing, while a transient fault's SQLSTATE is the whole
    /// diagnosis.
    fn operator_text(&self) -> String {
        legible_db_error(&self.source)
    }
}

impl std::fmt::Display for ApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ApplyError {
    /// Kept reachable so the chain does not dead-end here — see the `source` field's doc
    /// for what this does and does not currently carry.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

impl From<postgres::Error> for ApplyError {
    /// Surface the database's own message AND its DETAIL: `postgres::Error`'s
    /// Display is just "db error", and `message()` alone drops the
    /// `cairn_verify_error` DETAIL the doors attach (issue #109) — the reason
    /// that would otherwise reach only a direct psql caller.
    fn from(e: postgres::Error) -> Self {
        match e.as_db_error() {
            Some(db) => ApplyError {
                message: match db.detail() {
                    Some(detail) => format!("{} ({detail})", db.message()),
                    None => db.message().to_string(),
                },
                sqlstate: Some(db.code().code().to_string()),
                source: e,
            },
            // No `DbError` at all: the statement never reached a verdict (a dropped
            // connection, a client-side decode failure). `e.to_string()` here was
            // `postgres::Error`'s bare kind — `db error` / `error connecting to server` —
            // which is #467's species inside the type that is supposed to carry the
            // door's reasons (issue #480). `legible_db_error` walks the chain instead.
            None => ApplyError {
                message: legible_db_error(&e),
                sqlstate: None,
                source: e,
            },
        }
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
) -> Result<bool, ApplyError> {
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
        // `ApplyError::from` surfaces the door's legible RAISE text AND its DETAIL
        // (issue #109) while KEEPING the SQLSTATE — the one bit that tells a
        // deliberate floor refusal apart from a transient fault (ADR-0056 decision 5;
        // see `refusal_is_deliberate`). Flattening it to a String here was what left
        // the puller unable to route the two differently (#267/#270).
        .map_err(ApplyError::from)?;
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
    let mut client = connect_db(conn)?;
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
/// Apply ONE embedded migration, naming it if it fails (issue #475).
///
/// # Why a named door and not an inline `.map_err`
///
/// The loop this was extracted from ran `client.batch_execute(sql)?` with `name` in scope
/// and unused on the failure path, so an operator whose `cairn-sync init` died had to count
/// `applied \u{2026}` lines to work out which migration it was — on the very first command they
/// ever run. Issue #467 fixed the identical loop in `cairn-node` by extracting exactly this
/// door, for the reason that fix records: **a three-line loop body cannot be tested**, and
/// the acceptance criterion here is a sentence about a *message*. A door can be driven with
/// a deliberately failing body, which pins the criterion instead of restating it.
///
/// Renders through the same [`legible_db_error`] as everything else in this crate, so a
/// node log and a sync log carry one format an operator has to learn once.
fn load_migration(client: &mut postgres::Client, name: &str, sql: &str) -> R<()> {
    client
        .batch_execute(sql)
        .map_err(|e| format!("loading {name}: {}", legible_db_error(&e)).into())
}

/// Connect, naming the reason and NEVER echoing the connection string (issue #475).
///
/// `postgres::Error`'s `Display` renders a refused socket, an unresolvable host and a TLS
/// timeout all as `error connecting to server` — a category, not a diagnosis — and a bare
/// `?` gives an operator nothing else. [`legible_db_error`] walks `source()` for the errno,
/// which is where `Connection refused (os error 61)` actually lives.
///
/// The connection string is deliberately absent from the message: it can carry a password,
/// and an error line is exactly the text that ends up pasted into an issue. This is
/// `cairn-node`'s `db::connect` rule, which these sites did not have.
fn connect_db(conn: &str) -> R<postgres::Client> {
    postgres::Client::connect(conn, postgres::NoTls)
        .map_err(|e| format!("connecting to the database: {}", legible_db_error(&e)).into())
}

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
    // Hoisted to fn scope (was a block-local inside the `if let` below) so the
    // SAME generation reading can drive the gated heal further down this
    // function — without a second round-trip to node_schema.
    let mut recorded: Option<i32> = None;
    if table_exists {
        // query_opt: an absent ROW is a legitimate "generation unknown", but a real
        // query error must still fail loudly.
        if let Some(row) = client.query_opt("SELECT version FROM node_schema", &[])? {
            let v: i32 = row.get(0);
            recorded = Some(v);
            if v > embedded {
                return Err(format!(
                    "refusing to load schema: this database was last loaded at schema \
                     generation {v}, but this binary embeds only generation \
                     {embedded}. Replaying an older schema would silently downgrade \
                     the in-DB safety floor (issue #188 / ADR-0012). Upgrade this \
                     binary, or point it at a database of its own generation."
                )
                .into());
            }
        }
    }
    for (name, sql) in SCHEMA {
        load_migration(client, name, sql)?;
        eprintln!("applied {name}");
    }
    // ADR-0056 decision 4 (#266) / PR #302 finding F3: RE-ADJUDICATE FIRST, REPROJECT
    // SECOND — the same pass, in the same position, as cairn-node's loader. This crate
    // carries db/020, the door that WRITES the event_deferred marker, so without this call
    // a sync-only database (the phone-tier carrier node the ADR exists for) accumulates
    // admitted-but-powerless events that nothing can ever promote.
    //
    // Safe to run unconditionally — NOT because "a promoted event has already projected
    // cleanly" (unqualified, that is false: a type whose apply fns are ALL heal_safe = false,
    // e.g. note.added today, promotes on ZERO apply-fn runs by design, since a counter fn
    // cannot prove itself by replaying). The real reason: db/043's gate 4 selects its apply
    // fns with `WHERE event_type = ... AND heal_safe` (the `FOR v_apply_fn` loop inside
    // cairn_readjudicate_deferred), which is IDENTICAL to the heal filter cairn_reproject
    // (db/039) uses for its own per-type apply-fn aggregation (`FILTER (WHERE p_rebuild OR
    // r.heal_safe)`, with `p_rebuild = false` on this connect path). So the heal below can
    // never invoke an apply fn that gate 4 did not already run to completion on that exact
    // row — THAT identity, not "already projected", is what makes the F1 brick unreachable: a
    // bad event simply keeps its marker instead of taking the schema load down with it. On
    // THIS subset database gate 4 runs only the projections db/002 registers here, which is
    // correct — the node projects what it knows how to project.
    client.execute("SELECT count(*) FROM cairn_readjudicate_deferred()", &[])?;
    // #208/ADR-0057: same gated heal as cairn-node's loader, and BEFORE the stamp
    // below for the same reason. On this SUBSET database only the
    // subset-registered projections exist (db/002's rows); the registry makes
    // that automatic — replay heals exactly what is registered here, nothing
    // more (the old db/013 every-connect backfill is retired by this branch's
    // demographics conversion).
    //
    // Ordered BEFORE the stamp deliberately: if the heal query below errors, the
    // stamp never runs, so the recorded generation stays at its OLD (pre-upgrade)
    // value and the `?` propagates the failure up to the caller. The NEXT connect
    // attempt then sees the same stale `recorded`, so it retries the FULL
    // replay-then-heal — exactly the loud, self-retrying failure mode a broken
    // migration file already has in this loader (a bad `db/*.sql` blocks connect
    // until fixed; it never silently half-applies). Stamp-then-heal would invert
    // this: a heal failure AFTER the stamp leaves the generation already
    // advanced, so the next connect reads `recorded == embedded`, skips the heal
    // entirely, and the projections stay SILENTLY stale — the worst failure
    // mode, and the reason this order is load-bearing, not cosmetic.
    if recorded != Some(embedded) {
        client.execute(
            "SELECT count(*) FROM cairn_reproject('', false, 'loader')",
            &[],
        )?;
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
    let mut client = connect_db(conn)?;
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
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };

    // ADR-0039: globalise the authored twin — materialise it into the body BEFORE signing, so
    // this node emits a conformant author-faithful twin rather than relying on receivers to derive.
    let body = materialise_generic_twin(body);
    let signed = sign(&body, sk)?;
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let twin = resolve_twin(&body);
    // Issue #216 (ADR-0058) Task 6: this INSERT is the author's OWN write path — it
    // never crosses the apply_remote_event door (db/020), so that door's grade-storing
    // logic never runs for locally-authored events. Without stamping the column
    // explicitly here, the author's own row would silently fall back to the table's
    // 'unknown' DEFAULT even though the signed body carries the minted grade — a
    // cross-node metadata inconsistency versus every peer that later pulls this same
    // event through db/020. Serialize ClockGrade to its kebab-case wire string (e.g.
    // "self-asserted") so the column matches exactly what the signed body asserts.
    let grade = serde_json::to_value(body.clock_grade)?
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    tx.execute(
        "INSERT INTO event_log
           (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
            node_origin, t_effective, signed_bytes, content_address, body, contributors,
            signer_key_id, plaintext_twin, attachments, clock_grade)
         VALUES ($1::text::uuid,$2::text::uuid,$3,$4,$5,$6,$7,$8::text::timestamptz,$9,$10,
                 $11::text::jsonb,$12::text::jsonb,$13,$14,'[]'::jsonb,$15)",
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
            &grade,
        ],
    )?;

    // PR #285 review finding 2: this direct INSERT bypasses both doors, so the
    // grade-gated ceiling classify (db/005 step 1b' / db/020 step 1b') never runs for
    // locally-authored events — without this, a forward-dated `t_effective` authored
    // here would carry NO advisory `t_effective_ceiling_flag` row on the author's own
    // node while every peer that pulls the same signed event through db/020 records
    // one (the same cross-node inconsistency the explicit `clock_grade` column above
    // exists to prevent). Run the SAME in-DB classifier + t_effective parser + recorder
    // the doors use (one implementation, one dedup rule), in the same transaction.
    tx.execute(
        "SELECT cairn_record_ceiling_flag($1, $2::bigint, cairn_t_effective($3::text), $4::text, v.verdict) \
           FROM (SELECT cairn_ceiling_classify($2::bigint, $4::text, cairn_t_effective($3::text)) AS verdict) v \
          WHERE v.verdict IN ('flag', 'reject')",
        &[
            &signed.content_address,
            &body.hlc.wall,
            &body.t_effective,
            &grade,
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
    let mut client = connect_db(conn)?;
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
    let mut client = connect_db(conn)?;

    let mut pids = Vec::new();
    for i in 0..patients.max(1) {
        let pid = uuid::Uuid::now_v7().to_string();
        emit_event(
            &mut client,
            node,
            &sk,
            &kid,
            // #345 retired `patient.created`; the bench emits `patient.amended`, whose
            // db/002 branch is the SAME demographic-overlay path, so the Bet-B numbers
            // measured before the retirement stay comparable. `emit_event` INSERTs
            // directly and never passes the submit door, so the precedence rule does not
            // apply to it either way.
            "patient.amended",
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
    let mut client = connect_db(conn)?;
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
    let mut client = connect_db(conn)?;
    let log_size: i64 = client
        .query_one("SELECT count(*) FROM event_log", &[])?
        .get(0);
    let pid = uuid::Uuid::now_v7().to_string();
    emit_event(
        &mut client,
        node,
        &sk,
        &kid,
        // See the seeding loop above: `patient.amended` since #345, same projection path.
        "patient.amended",
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
    let mut client = connect_db(conn)?;
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
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};

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
    // The `(&array).into()` conversions below borrow the fixed-size arrays as the
    // AEAD `Key`/`XNonce` types — chacha20poly1305 0.11 deprecated `from_slice`.
    let kek_bytes: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(3).wrapping_add(9));
    let kek = XChaCha20Poly1305::new((&kek_bytes).into());
    let nonce_bytes: [u8; 24] = std::array::from_fn(|i| i as u8);
    let nonce: &XNonce = (&nonce_bytes).into();
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8) ^ 3);
    let t = Instant::now();
    for _ in 0..dek_iters {
        std::hint::black_box(kek.encrypt(nonce, dek.as_ref()).unwrap());
    }
    let dek_wrap_per_s = dek_iters as f64 / t.elapsed().as_secs_f64();

    let body = vec![0x7Eu8; 1024]; // a representative ~1 KiB clinical body
    let body_kek = XChaCha20Poly1305::new((&dek).into());
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
        "substance": {"term": "metformin"},
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
/// House rule 6: the recipient unwrap secret is GENERATED at runtime from the OS RNG
/// (`generate_unwrap_secret`), never a hard-coded byte literal — and the seal/wrap
/// primitives draw their own fresh DEK + nonce from the OS RNG internally, so nothing in
/// this bench presents hard-coded cryptographic material to the scanner.
fn cmd_bench_seal(iters: usize) -> R<()> {
    use cairn_event::seal::{
        generate_unwrap_secret, seal_event_payload, unseal_event_payload, unwrap_dek,
        unwrap_public, wrap_dek_for,
    };

    let payload = representative_medication_payload();
    let payload_bytes = serde_json::to_vec(&payload)?.len();
    let twin = "metformin 500 mg tablet — one BD, patient-reported, started 2023";
    let event_id = uuid::Uuid::now_v7().to_string();

    // Recipient (node) unwrap keypair, generated fresh for the benchmark. Deliberately
    // NOT derived from a signing seed: this measures crypto cost, so the secret's
    // provenance is irrelevant, and reaching for `derive_unwrap_secret` here would keep
    // a dead coupling alive in a file that no longer has one (ADR-0066, issue #503).
    let secret = generate_unwrap_secret()?;
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

/// Which residual-refusal class a penned entry belongs to (ADR-0056 decision 5).
///
/// Both classes take the SAME mechanism — verbatim bytes penned by digest, the
/// slot pinned on the re-offer floor, the cycle loud — because one contract at
/// the door is cheaper to reason about than two. They are counted and reported
/// separately only because they call for different operator action: unverifiable
/// bytes need the PEER repaired (re-signed history, fixed build); a door refusal
/// needs THIS node changed (enroll the author, take the code-plane update) or the
/// exclusion acked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RefusalClass {
    /// The bytes do not verify: they can never apply without repair at the source.
    Unverifiable,
    /// The bytes verify; this node's floor deliberately refused them.
    DoorRefused,
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
        .map_err(|e| LocalDbFault::boxed("recording a re-offer of already-penned bytes", e))?;
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
        .map_err(|e| LocalDbFault::boxed("penning a refused event in sync_quarantine", e))?;
    if inserted == 0 {
        // Zero rows means EITHER over-quota OR a concurrent writer penned the
        // same bytes first (ON CONFLICT DO NOTHING). Distinguish them — a false
        // "quota" diagnosis on the safety path would send the operator chasing
        // a condition that does not exist.
        if let Some(row) = client
            .query_opt(
                "SELECT acked FROM sync_quarantine WHERE content_digest = $1",
                &[&digest],
            )
            .map_err(|e| LocalDbFault::boxed("distinguishing a full pen from a lost race", e))?
        {
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
    transport: &dyn Transport,
    peer_name: &str,
    full_sweep: bool,
    // This node's custody for the wire (ADR-0052): the signing key that proves who we
    // are and the unwrap secret that opens DEKs a peer re-wraps back for us. Since
    // ADR-0066 the secret is NOT derivable from the key, so it is resolved once at
    // startup (see `unwrap_key::resolve`) and carried here. `None` (older call paths,
    // DB tests) pulls WITHOUT custody: events still sync and sealed rows admit
    // structurally at the door — custody is simply not gained on this cycle.
    custody: Option<&unwrap_key::NodeCustody>,
) -> R<serde_json::Value> {
    // The first two statements happen BEFORE any network I/O, so a failure here is
    // always this node's own database — and `cmd_run` builds its client once, outside the
    // loop, so after the database goes away every later cycle fails right here for the
    // life of the process. `LocalDbFault` names the operation AND keeps the
    // `postgres::Error` reachable, which is what `classify_pull_failure` needs to keep
    // calling this a local fault rather than link downtime (issue #479).
    client
        .execute(
            "INSERT INTO sync_state (peer) VALUES ($1) ON CONFLICT (peer) DO NOTHING",
            &[&peer_name],
        )
        .map_err(|e| LocalDbFault::boxed("registering this peer in sync_state", e))?;
    // The committed seq cursor + the seq re-offer floor for this peer (db/036).
    // `last_seq` is the highest serving-node event_log.seq we have pulled — the
    // node-LOCAL insertion order, NOT the HLC. `quarantine_floor_seq` (NULL = none)
    // is the seq of the first unresolved refused slot, a SEPARATE persisted column
    // so it self-clears on a clean cycle while the pen row survives as an audit trace.
    let st = client
        .query_one(
            "SELECT last_seq, quarantine_floor_seq FROM sync_state WHERE peer=$1",
            &[&peer_name],
        )
        .map_err(|e| LocalDbFault::boxed("reading this peer's sync cursor", e))?;
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

    // This node's unwrap identity for the pull (ADR-0052 custody sidecar). We keep our
    // unwrap SECRET (to open re-wrapped DEKs the peer sends back) and present our unwrap
    // CERT (so the peer can re-wrap for us). The cert binds our X25519 unwrap public key
    // to our Ed25519 identity — the same key the DEKs come back wrapped for.
    let unwrap_secret = custody.map(|c| c.unwrap_secret.clone());
    let unwrap_cert: Option<String> = match custody {
        Some(c) => {
            let public = cairn_event::seal::unwrap_public(&c.unwrap_secret);
            Some(hex::encode(cairn_event::sign_unwrap_key_cert(
                &c.signing_key,
                &public,
            )?))
        }
        None => None,
    };

    let started = Instant::now();
    // A transport failure here has one skew-shaped cause worth naming (PR #223
    // review): a pre-#196 serve cannot decode the EventsAfterSeq op — its serde
    // rejects the unknown tag, the connection drops with no response frame, and
    // all this side sees is an EOF. Say so, alongside the plain-partition reading,
    // instead of leaving the operator a bare "failed to fill whole buffer".
    let raw = transport
        .request(&Request::EventsAfterSeq {
            after_seq,
            unwrap_cert,
            // Slice 2b (#101 item 1): paging lands wired-but-off. This puller does not
            // yet loop on `complete`, so asking for an unbounded batch is still correct
            // — turning this on before the pull loop can honour it would silently
            // truncate a real clinic's log. Task 7 flips it.
            limit: None,
        })
        .map_err(|e| PeerRequestError {
            // THE CAUSE IS THE SUFFIX, and that placement is load-bearing, not stylistic.
            // `operator_chain` drops a layer only when the layer above it ENDS WITH that
            // layer's rendering; with `{e}` mid-sentence (where the first draft of this type
            // put it) the same transport error would be printed twice on the `run` path —
            // the double-render the sweep's own tail had to fix one file over. Keeping it
            // last also makes `Display`/`Debug` self-contained for the one-shot `pull`, whose
            // only error printer is `Termination`.
            message: format!(
                "pull {peer_name}: no usable response to EventsAfterSeq. If the peer is down \
             this is a plain partition (retry later); but if it is reachable and hangs up \
             without answering, it likely predates the #196 seq-cursor wire (db/036) and \
             cannot decode this request — upgrade the peer binary (an OLD puller against \
             THIS node still works; an old server cannot be seq-pulled). The transport \
             reported: {e}"
            ),
            // Kept, never flattened (PR #493 review): an over-cap length prefix arrives here
            // as `InvalidData` and is the PEER's doing, not the link's. See
            // `chain_reaches_a_peer_frame_error`.
            source: Box::new(e),
        })?;
    let wire_bytes = raw.len();
    // The peer ANSWERED and what it sent is unusable — an integrity condition, not a
    // partition (issue #489). A bare `?` here handed `classify_pull_failure` a
    // `serde_json::Error`, which its default arm claimed by elimination was a peer that
    // never answered; `bet_a.py` then counted a truncated or foreign response as link
    // downtime. Nothing was reached beyond the response itself, so there are no metrics.
    let resp: EventsResponse = serde_json::from_slice(&raw).map_err(|e| PullIntegrityError {
        message: format!(
            "pull {peer_name}: the peer answered with {wire_bytes} byte(s) that are not an \
             EventsResponse: {e}. The link delivered them — the peer is serving a corrupt, \
             truncated, or entirely different payload on this port."
        ),
        metrics: serde_json::Value::Null,
        also_local_fault: false,
    })?;

    // The three response-shape guards moved to `validate_page` ahead of Task 7's
    // paging loop (slice 2b Task 6, #500): each page's response has to pass them on
    // its own, independent of how many pages came before it in this cycle.
    validate_page(&resp, peer_name)?;

    // Cursor discipline (review fix A1 + issue #108 + the PR #110 review), re-keyed
    // to the node-local seq for #196:
    //   * an UNVERIFIABLE entry (bad signature, garbage bytes, non-hex text) is
    //     quarantined durably (db/021) AND pins the re-offer floor at its seq. The
    //     cursor still advances for valid events, but the floor keeps the refused
    //     slot on the wire every cycle, so a later repaired/re-signed version is
    //     admitted automatically — a durable trace alone is NOT a license to move
    //     past an event. Only a human `acked` row or a clean cycle releases the slot;
    //   * a VERIFIABLE event this node's floor DELIBERATELY refused (`P0001`:
    //     unenrolled/revoked signer, oversize, t_effective past the ceiling, an
    //     unlawful contributor shape) takes the SAME path — penned verbatim, slot
    //     pinned (ADR-0056 decision 5, issue #267). It used to persist nothing and
    //     freeze, which wedged the whole peer link behind one bad author's event;
    //   * a VERIFIABLE event that fails to apply for any OTHER reason is transient
    //     infrastructure trouble (dropped connection, deadlock, timeout, disk full),
    //     where the very same bytes may apply next cycle. That FREEZES the cursor at
    //     the contiguous handled prefix — retried next cycle, never skipped (A1).
    //     (During a FULL SWEEP a failing event may sit BELOW the committed cursor;
    //     the freeze then leaves the cursor as-is and the retry rides the NEXT sweep,
    //     not the next incremental — up to FULL_SWEEP_EVERY cycles later. Delayed,
    //     never lost.);
    //   * if the pen itself refuses (insert failure, per-peer quota), the cursor
    //     freezes exactly as for a transient apply failure — delayed, never lost.
    // Any unacked refusal — and any freeze (issue #270) — makes the whole pull FAIL
    // LOUDLY at the end.
    // The peer deliberately withheld custody for this batch (issue #231 review). Printed
    // BEFORE applying, so the reason sits above the "N applied" line rather than buried
    // under it; the line itself, and why its peer text is escaped, is
    // `custody_withheld_message`.
    if let Some(reason) = resp.custody_withheld.as_deref() {
        eprintln!("{}", custody_withheld_message(peer_name, reason));
    }
    // The watermark is read HERE, before the first apply, and its failure is carried
    // rather than raised: it feeds the same `Err` arm the ledger read does, so an
    // unreadable ledger reports UNKNOWN instead of costing the pull its progress.
    let flag_watermark = attachment_flag_watermark(client);

    // Everything `apply_page` needs from `do_pull` that isn't `resp` itself (Task 7
    // rebuilds this once per page; `floor_seq` and the custody pair are constant across
    // a whole cycle, so they cost nothing to recompute).
    let ctx = PageContext {
        peer_name,
        unwrap_secret: unwrap_secret.as_deref(),
        floor_seq,
    };
    let page = apply_page(client, &ctx, &resp, wire_bytes, last_seq);
    let mut cycle = pull_page::CycleTally::new(last_seq);
    cycle.fold(page);

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
    let new_floor: Option<i64> = pull_page::quarantine_floor(
        cycle.skipped_unverifiable,
        cycle.refused_verifiable,
        cycle.pen_refused.is_some(),
        cycle.pin,
        floor_seq,
    );
    // BUILT BEFORE THE CURSOR COMMIT (issue #469). The commit is the only `?` between the
    // apply loop and this function's return — it used to sit ABOVE this object, so a bare
    // `?` there threw away every number describing work that had ALREADY happened — events
    // applied and durable, flags written — and the cycle reported nothing at all about it.
    // Moving the object above the commit is the fix; the commit is now the block below.
    // Two fields are filled in afterwards because they are not knowable yet; both are
    // seeded `null` rather than `0` so a path that somehow skipped them can never publish
    // a claim.
    let mut metrics = serde_json::json!({
        "op": "pull", "peer": peer_name,
        "shipped": cycle.shipped, "applied_new": cycle.applied,
        "skipped_unverifiable": cycle.skipped_unverifiable,
        "refused_verifiable": cycle.refused_verifiable,
        "skipped_acked": cycle.skipped_acked,
        "watermark_frozen": cycle.frozen,
        // Deliberately NOT folded into `cycle_is_loud` (issue #231 review). It IS a
        // state in which this node knowingly lacks something the peer holds, which is
        // that predicate's stated test — but the events themselves all arrived, the
        // degradation is the sanctioned ADR-0052 one, and failing every cycle would
        // turn a peering gap into a link that reads as broken. Surfaced as its own
        // metric + its own stderr line instead, so a monitor can alert on it without
        // the pull having to fail. Revisit if a real deployment finds the line alone
        // too quiet to notice.
        "custody_withheld": cycle.custody_withheld,
        // Attachment references admitted-but-unlearnable this cycle (issue #465), for
        // the same reason and on the same terms as `custody_withheld` above: a real
        // degradation this node must know about, which must not fail the cycle. Counts
        // REFERENCES, not events, and is `null` — never 0 — when the ledger could not
        // be read at all.
        // Seeded null and filled in after the cursor commit — see below.
        "references_unlearnable": serde_json::Value::Null,
        "floor_active": new_floor.is_some(),
        "event_bytes": cycle.event_bytes, "wire_bytes": cycle.wire_bytes,
        "bytes_per_event": if cycle.shipped == 0 { 0.0 }
                           else { cycle.event_bytes as f64 / cycle.shipped as f64 },
        "elapsed_ms": serde_json::Value::Null,
        "cursor_seq": cycle.max_seq, "full_sweep": full_sweep
    });

    // Advance-only cursor (GREATEST) + the recomputed seq floor. A re-offer cycle
    // whose max_seq did not exceed the committed cursor therefore never rewinds it.
    //
    // NOT a bare `?` (issue #469): this is the one write whose failure means "the work
    // happened but the bookmark did not move", and it is neither an integrity condition
    // nor a partition. Held as a Result so the report below still runs either way.
    let cursor_commit = client
        .execute(
            "UPDATE sync_state
                SET last_seq = GREATEST(last_seq, $2), last_pull_at = clock_timestamp(),
                    quarantine_floor_seq = $3
              WHERE peer = $1",
            &[&peer_name, &cycle.max_seq, &new_floor],
        )
        .map_err(|e| legible_db_error(&e))
        // A statement that matched NO row is `Ok` and commits nothing: the cursor silently
        // does not move while the cycle reports `cursor_seq` as fact. The upsert at the top
        // of this function makes the row, so this needs a concurrent `DELETE FROM
        // sync_state` or an RLS change to happen at all — but a silent zero is exactly the
        // shape #465/#469 exist to end, and checking a rowcount we already have is free
        // (PR #472 review).
        .and_then(|rows| match rows {
            0 => Err(format!(
                "the sync_state row for peer '{peer_name}' matched no row — it was deleted \
                 mid-cycle, or this role cannot see it"
            )),
            _ => Ok(()),
        });
    metrics["elapsed_ms"] = serde_json::json!(started.elapsed().as_secs_f64() * 1000.0);

    // Issue #465: what did this cycle admit that it cannot fetch the bytes for? Read
    // AFTER the cursor commit on purpose — this is a report about work already done, and
    // a failure to produce it must never cost the pull its progress. Either read failing
    // (the watermark above, or the ledger here) reports UNKNOWN (null), never zero, and
    // says what actually failed rather than "db error" — see `unlearnable_report` and
    // `legible_db_error`.
    //
    // It runs even when the commit above FAILED, and that is deliberate: the events and
    // their flags are durable regardless, so this is still a true report about completed
    // work. If the database is the thing that broke, the read fails too and reports
    // UNKNOWN with the cause named — which is the honest answer, not a missing line.
    metrics["references_unlearnable"] = emit_unlearnable_report(
        &mut io::stderr(),
        &format!("pull {peer_name}"),
        flag_watermark
            .and_then(|since| unlearnable_references(client, &cycle.applied_addresses, since))
            .map_err(|e| legible_db_error(&e)),
    );

    // The loud-integrity diagnosis is composed BEFORE the cursor-commit check, because the
    // two conditions are independent and a cycle can be both. Composing it here (rather
    // than only in the `if` below, as the first draft did) is what stops a failed commit
    // from swallowing an integrity condition that needs a human — PR #472 review, finding
    // 5. `loud_pull_message` is pure, so composing it on a path that may not use it costs
    // one allocation and no I/O.
    let loud = cycle_is_loud(
        cycle.skipped_unverifiable,
        cycle.refused_verifiable,
        cycle.frozen,
        cycle.pen_refused.is_some(),
    );
    let loud_message = loud.then(|| {
        let all = cycle.shipped != 0 && cycle.skipped_unverifiable == cycle.shipped;
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
                cycle.shipped
            )
        } else {
            String::new()
        };
        // Every clause is conditional on the state that makes it true — see
        // `loud_pull_message`, where the four loud states and their (different)
        // operator actions are composed and unit-tested with no database.
        loud_pull_message(
            peer_name,
            cycle.skipped_unverifiable,
            cycle.refused_verifiable,
            cycle.frozen.then_some(cycle.max_seq),
            cycle.pen_refused.as_ref().map(|p| p.message.as_str()),
            &diagnosis,
        )
    });

    // The local-fault class (issue #469). Everything above describes work that genuinely
    // happened; only the two fields describing the failed write are withdrawn.
    if let Err(cause) = cursor_commit {
        mark_cursor_outcome_unknown(&mut metrics);
        return Err(Box::new(CursorCommitError {
            message: cursor_commit_failure_message(
                peer_name,
                cycle.applied,
                cycle.max_seq,
                &cause,
                loud_message.as_deref(),
            ),
            metrics,
            also_loud: loud,
        }));
    }

    // LOUD failure (issue #108, generalised by the #110 review, extended to the
    // freeze by issue #270): ANY state in which this node knowingly does not hold
    // something the peer offered it fails the pull, every cycle, until the cause is
    // fixed or a human acks the exclusion. The old `skipped == len` heuristic
    // structurally missed the two common cases (a mixed legacy+new batch, and an
    // already-synced link whose boundary event re-applies idempotently), which is
    // exactly where the silent livelock lived; the freeze arm then reintroduced one
    // of its own by exiting SUCCESS with a halted cursor. `run` logs this as an
    // integrity condition, not a partition. The message itself was composed above,
    // so that a cycle which ALSO failed its cursor commit still carries it.
    if let Some(message) = loud_message {
        return Err(Box::new(PullIntegrityError {
            message,
            metrics,
            // #489 part 1: a pen write the DATABASE refused makes this cycle a local
            // fault as well as an integrity condition. A quota refusal does not.
            //
            // PR #493 review: so does an APPLY that failed on this node's database, which
            // reaches the loud path through `frozen` and never touches the pen at all —
            // the arm this fix originally missed.
            also_local_fault: cycle.local_apply_fault
                || cycle.pen_refused.is_some_and(|p| p.local_fault),
        }));
    }

    Ok(metrics)
}

/// Everything `apply_page` needs from `do_pull` besides the response itself and the
/// per-page cursor value — grouped so Task 7's loop can rebuild it once per page rather
/// than growing `apply_page`'s argument list (slice 2b Task 6, #500).
struct PageContext<'a> {
    peer_name: &'a str,
    /// This node's unwrap secret (ADR-0052), if this cycle is pulling WITH custody.
    /// `&[u8; 32]`, not a plain slice: `cairn_event::seal::unwrap_dek` takes
    /// `&[u8; 32]`, and keeping the fixed size here means a wrong-length secret is
    /// unrepresentable rather than a runtime `try_from` that could fail into the
    /// "admit without custody" arm and misattribute a LOCAL fault to the peer.
    unwrap_secret: Option<&'a [u8; 32]>,
    /// The seq re-offer floor at the START of this cycle (db/036) — constant across
    /// every page of one cycle, unlike the running cursor `apply_page` also takes.
    floor_seq: Option<i64>,
}

/// The response-shape guards a pull's page must pass before any per-event work begins —
/// moved out of `do_pull` ahead of Task 7's paging loop (slice 2b Task 6, #500), so each
/// page's response validates on its own, independent of how many pages came before it in
/// this cycle. Nothing here touches the database: every violation is refused before any
/// per-event work, so `metrics` is always `null` and `also_local_fault` always `false`.
fn validate_page(resp: &EventsResponse, peer_name: &str) -> Result<(), PullIntegrityError> {
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
            return Err(PullIntegrityError {
                message: format!(
                    "pull {peer_name}: peer declares signing context '{peer_ctx}' but this \
                     node expects '{}' — wire-format skew, not tampering; upgrade the older \
                     side. Batch refused, cursor untouched.",
                    CTX_EVENT.as_str()
                ),
                metrics: serde_json::Value::Null,
                // Nothing was written here: the batch is refused before any per-event
                // work, so this node's database was never asked to do anything.
                also_local_fault: false,
            });
        }
    }

    // The per-event seq is load-bearing for the cursor (issue #196): a response
    // carrying events but a short/empty seqs array is a malformed or unexpectedly-old
    // serve — fail LOUDLY rather than checkpoint the cursor blind.
    if !resp.events.is_empty() && resp.seqs.len() != resp.events.len() {
        // Integrity, not partition (issue #489): the peer answered, and its WIRE FORMAT is
        // the problem. This is the structural sibling of the signing-context skew sixteen
        // lines above, which has returned a `PullIntegrityError` since #108 — the two
        // returned different classes for the same kind of fault until now.
        return Err(PullIntegrityError {
            message: format!(
                "pull {peer_name}: peer returned {} events but {} seqs — cannot checkpoint \
                 the seq cursor safely; the peer serves an incompatible/older wire format",
                resp.events.len(),
                resp.seqs.len()
            ),
            metrics: serde_json::Value::Null,
            // Refused before any per-event work: this node's database was never asked.
            also_local_fault: false,
        });
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
        // Integrity, not partition (issue #489). A buggy or hostile peer is the ONLY thing
        // that produces this — a link cannot reorder a JSON array — so classifying it as
        // link downtime both misdirected the operator and hid a peer worth looking at.
        return Err(PullIntegrityError {
            message: format!(
                "pull {peer_name}: peer returned malformed seqs (must be strictly ascending \
                 and positive) — refusing to checkpoint the seq cursor from these values"
            ),
            metrics: serde_json::Value::Null,
            // Refused before any per-event work: this node's database was never asked.
            also_local_fault: false,
        });
    }

    Ok(())
}

/// One page's worth of apply — the body `do_pull` used to run inline, lifted out
/// ahead of Task 7's paging loop so each page can be applied on its own and the
/// results folded (`pull_page::CycleTally::fold`) rather than accumulated in a dozen
/// local `mut` bindings that outlive one request (slice 2b Task 6, #500). Pure move:
/// every counting/quarantine/freeze rule below is unchanged from `do_pull`'s old
/// inline loop, comments included.
///
/// `max_seq_in` seeds the page's running cursor — the committed cursor for page 1,
/// the PREVIOUS page's `max_seq` from page 2 on (Task 7) — so a page never rewinds
/// what an earlier page in the same cycle already advanced past.
fn apply_page(
    client: &mut postgres::Client,
    ctx: &PageContext<'_>,
    resp: &EventsResponse,
    wire_bytes: usize,
    max_seq_in: i64,
) -> pull_page::PageTally {
    let (mut applied, mut skipped_unverifiable, mut skipped_acked, mut event_bytes) =
        (0usize, 0usize, 0usize, 0usize);
    // Verifiable events the floor refused and we penned this cycle (issue #267).
    let mut refused_verifiable = 0usize;
    // Content addresses of every event the door ADMITTED this cycle — new or re-offered
    // — and the ledger's high-water mark before any of them landed. Together they are the
    // key the attachment-reference ledger is read back on at the end (issue #465; the two
    // keys and why neither alone is enough are in `unlearnable_references`). Held rather
    // than counted because the report has to NAME an event, and because attributing a
    // flag to this peer means proving the event came from this batch.
    let mut applied_addresses: Vec<Vec<u8>> = Vec::new();
    // Highest CONTIGUOUS handled seq. Starts at the cursor so re-offered low-seq
    // events (below it) never rewind the checkpoint; new events above it advance it.
    let mut max_seq = max_seq_in;
    let mut frozen = false;
    // Did any apply failure this cycle land on THIS NODE'S database? (PR #493 review.)
    // Tracked separately from `frozen` because the two answer different questions: `frozen`
    // says the cursor stopped, this says WHOSE fault that was. A freeze caused by a lock
    // storm and a freeze caused by a peer's malformed field both halt the cursor; only the
    // first is a fact about this node's uptime.
    let mut local_apply_fault = false;
    // First pen failure (if any) — surfaced in the loud error.
    let mut pen_refused: Option<PenRefusal> = None;
    // The seq of the FIRST unacked refused event this cycle (the stream is
    // seq-ascending, so the first is the lowest) — persisted as the new floor.
    let mut pin: Option<i64> = None;
    // Aliased so the per-event branching below — moved verbatim from `do_pull` —
    // reads exactly as it did before extraction, with no adaptation anywhere in it.
    let peer_name = ctx.peer_name;
    let floor_seq = ctx.floor_seq;
    let unwrap_secret = ctx.unwrap_secret;

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

        // (bytes to pen, attestation pair, reason, class) for a refused entry;
        // None if the entry applied or hit transient trouble (the freeze case).
        let refused: Option<(WireEntry, String, RefusalClass)> = match decoded {
            Err(reason) => Some((
                (hexed.as_bytes().to_vec(), None, None),
                reason,
                RefusalClass::Unverifiable,
            )),
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
                    // No sidecar DEK for this slot. Silent HERE on purpose: it is the
                    // normal case for every unsealed event, so a line per slot would be
                    // pure noise. When the absence is a deliberate REFUSAL rather than
                    // "nothing to send", the peer says so once per batch in
                    // `resp.custody_withheld`, printed above (issue #231 review).
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
                        // Recorded for EVERY admitted event, NOT only the new ones: db/020
                        // runs the lenient reference learner unconditionally, so a
                        // re-offered event can still produce a first-ever flag row, and
                        // gating this on `new` made exactly those invisible (#466 review).
                        // What keeps an ordinary re-delivery quiet is the flag_id
                        // watermark, not this list — see `unlearnable_references`.
                        //
                        // Recomputed rather than threaded out of `apply_signed` (which
                        // computes and drops the same value): it is a hash of bytes
                        // already in hand, and the alternative changes a return type both
                        // call sites read.
                        applied_addresses.push(cairn_event::event_address(&signed_bytes));
                        // Auto-release (mirrors the node plane's issue #111
                        // behaviour): a re-offered event that NOW applies is no
                        // longer an unresolved refusal, so its pen row must go —
                        // otherwise the pen keeps a full duplicate of a row
                        // event_log already holds, and `cairn-sync quarantine`
                        // shows a resolved refusal forever. Gated on an ACTIVE
                        // floor (this peer has at least one unresolved slot), so
                        // the common forward path does no per-event DELETE.
                        // UNVERIFIABLE bytes can never reach this arm — they never
                        // apply — so the pen's forensic trace is preserved by
                        // construction, not by a special case. An `acked` row IS
                        // released when it reaches here, deliberately (same rule
                        // `do_requeue` uses): an event the floor has now ADMITTED is
                        // held in event_log, so a pen row claiming it is excluded
                        // would be the misleading state. But note the reach — the
                        // floor gate means an acked row only gets here while some
                        // OTHER unresolved slot is still pinning this peer's floor;
                        // an ack that cleared the last floor leaves its row in place
                        // until a full sweep coincides with one. Harmless (the
                        // content is in event_log either way), and the alternative —
                        // a per-event DELETE on every clean forward pull — is a cost
                        // the common path should not pay for a cosmetic tidy-up.
                        //
                        // A failure here is NOT worth aborting the cycle for: the
                        // events already applied (each apply is its own transaction),
                        // and the floor keeps re-offering, so the next cycle retries
                        // the release. Aborting would instead discard the cursor
                        // commit below and be logged as a `partition` — a transport
                        // diagnosis for a tidy-up failure.
                        if floor_seq.is_some() {
                            let digest = cairn_event::event_address(&signed_bytes);
                            if let Err(de) = client.execute(
                                "DELETE FROM sync_quarantine WHERE content_digest = $1",
                                &[&digest],
                            ) {
                                // `{de}` here was the literal two words `db error` —
                                // #479's title sentence, inside the daemon loop (#490
                                // item 1). A grant revoked by a restore (42501) leaves
                                // the pen row unreleased, and without the SQLSTATE there
                                // is nothing to act on.
                                //
                                // HOW LONG THIS LINE REPEATS DEPENDS ON THE FLOOR, and an
                                // earlier draft of this comment said "every cycle forever"
                                // as though it always did (PR #493 review). The DELETE is
                                // gated on `floor_seq.is_some()`, and a failed release
                                // moves none of the three counters `new_floor` reads — so
                                // if this was the LAST unresolved slot, the floor clears at
                                // the end of this very cycle, the gate is false next cycle,
                                // and the line prints exactly ONCE while the row is left
                                // behind. That is the quieter and worse case: the pull
                                // exits 0, `cairn-sync quarantine` shows a resolved refusal
                                // forever, and `cairn-sync requeue` is what clears it.
                                eprintln!(
                                    "pull {peer_name}: seq {seq} applied but its resolved pen \
                                     row could not be released: {} — retried next cycle",
                                    legible_db_error(&de)
                                );
                            }
                        }
                        None
                    }
                    Err(e) => {
                        // Three outcomes, decided by two independent questions:
                        // do the bytes VERIFY, and did the door DELIBERATELY
                        // refuse them (ADR-0056 decision 5)?
                        match verify_self_described(&signed_bytes) {
                            // Verifiable + a deliberate floor refusal (P0001).
                            // Deterministic: it will refuse identically until this
                            // node changes. Pen it verbatim and pin the slot —
                            // freezing here would wedge the whole link behind one
                            // author's event (issue #267).
                            Ok(_) if e.is_deliberate_refusal() => Some((
                                (signed_bytes, att, akey),
                                format!("refused by this node's floor: {e}"),
                                RefusalClass::DoorRefused,
                            )),
                            // Verifiable, but the failure was NOT a door verdict:
                            // transient infrastructure trouble, where the same
                            // bytes may well apply next cycle. Freeze and retry —
                            // penning would record a refusal that never happened.
                            Ok(_) => {
                                frozen = true;
                                // PR #493 review. `frozen` alone made this cycle
                                // `integrity` — *the peer answered and its DATA is the
                                // problem* — for a lock storm on this node's own database.
                                // That is issue #489 part 1's own sentence, one arm above
                                // the pen write it was filed against, and `bet_a.py` both
                                // missed the local fault AND folded the blocked write's
                                // elapsed_ms into the A4 latency percentiles because its
                                // filter is `not r.get("local_fault")`.
                                //
                                // The pen path asks this question by walking for a
                                // `postgres::Error`; here the SQLSTATE is already in hand
                                // and says strictly more, because the non-`P0001` space is
                                // not all local — see `apply_failure_is_local`.
                                local_apply_fault |= apply_failure_is_local(e.sqlstate.as_deref());
                                eprintln!(
                                    "pull {peer_name}: HALTING seq cursor at {max_seq} — a valid \
                                     event failed to apply for a NON-refusal reason (transient?) \
                                     and must not be skipped: {}",
                                    // `operator_text()`, NOT `{e}`: `Display` is the DOOR's
                                    // vocabulary and deliberately omits the SQLSTATE, and
                                    // this arm is reached ONLY by failures that are not door
                                    // verdicts — where the SQLSTATE is the whole diagnosis.
                                    // The node-plane twin already renders `legible_db_error`
                                    // at its equivalent freeze site (#474 item 2), so the
                                    // two crates' most consequential line now agrees.
                                    e.operator_text()
                                );
                                None
                            }
                            Err(verr) => Some((
                                (signed_bytes, att, akey),
                                format!("{verr}; apply door said: {e}"),
                                RefusalClass::Unverifiable,
                            )),
                        }
                    }
                }
            }
        };

        if let Some(((bytes, att, akey), reason, class)) = refused {
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
                    // Both classes pin the floor identically; only the tally and
                    // the wording differ (see RefusalClass).
                    let kind = match class {
                        RefusalClass::Unverifiable => {
                            skipped_unverifiable += 1;
                            "unverifiable event"
                        }
                        RefusalClass::DoorRefused => {
                            refused_verifiable += 1;
                            "floor-refused (but verifiable) event"
                        }
                    };
                    if pin.is_none() {
                        pin = Some(seq);
                    }
                    eprintln!(
                        "pull {peer_name}: {kind} quarantined durably \
                         (sync_quarantine), slot held on the re-offer floor at seq {seq}: {reason}"
                    );
                }
                Err(qe) => {
                    frozen = true;
                    eprintln!(
                        "pull {peer_name}: HALTING seq cursor at {max_seq} — a refused \
                         event could not be quarantined, so it must not be skipped: {qe}; \
                         reason: {reason}"
                    );
                    // First-wins for the message, OR for the class — the rule and the
                    // reasoning live in `merge_pen_refusal` (issue #489, PR #493 review).
                    pen_refused = Some(merge_pen_refusal(
                        pen_refused.take(),
                        pen_refusal(qe.as_ref()),
                    ));
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

    pull_page::PageTally {
        shipped: resp.events.len(),
        applied,
        skipped_unverifiable,
        refused_verifiable,
        skipped_acked,
        event_bytes,
        wire_bytes,
        max_seq,
        frozen,
        local_apply_fault,
        pen_refused,
        pin,
        custody_withheld: resp.custody_withheld.is_some(),
        applied_addresses,
    }
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
    // PR #478 converted the three statements INSIDE the loop and left the one that opens
    // the function: a `SELECT` revoked by a restore gave `Error: "db error"` from the very
    // command #471 was filed against. Nothing has been examined yet, so there is no
    // partial-completion report to make (contrast `RequeueInterruptedError` below) — only
    // a legible reason (issue #479).
    let digests: Vec<Vec<u8>> = client
        .query(
            "SELECT content_digest FROM sync_quarantine ORDER BY first_seen",
            &[],
        )
        .map_err(|e| LocalDbFault::boxed("listing the quarantine pen", e))?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let (mut released, mut still_quarantined, mut vanished) = (0usize, 0usize, 0usize);
    // Every count as it stands RIGHT NOW, for either exit — the `Ok` at the bottom of this
    // function or an interruption inside the loop (issue #471). Stated once so the two can
    // never disagree about what a requeue reports.
    let examined = digests.len();
    let snapshot = |released: usize,
                    still_quarantined: usize,
                    vanished: usize,
                    references_unlearnable: serde_json::Value| {
        serde_json::json!({
            "op": "requeue",
            // examined == released + still_quarantined + vanished on a COMPLETE run. An
            // interrupted one is short by the rows it never reached PLUS ONE — the row it
            // stopped on, which was reached and is deliberately counted nowhere because
            // the failure is what left its outcome undecided. The message states both in
            // words; reconciling the four numbers alone would mislead by exactly that one.
            "examined": examined,
            "released": released,
            "still_quarantined": still_quarantined,
            "vanished": vanished,
            "references_unlearnable": references_unlearnable
        })
    };
    // Built once per failing statement: the FOUR sites below differ only in which
    // statement failed, and every one of them owes the same report (ADR-0060 decision 2).
    // (Three raw statements from #471, plus the apply door itself since #480.)
    //
    // `why` is ALREADY LEGIBLE — `legible_db_error` for a raw `postgres::Error`, or
    // `ApplyError::operator_text()` for the apply door. NOT `ApplyError`'s `Display`, which
    // deliberately omits the SQLSTATE (see that method's doc); an earlier draft of this
    // comment named `Display` and would have taught the next maintainer to make exactly the
    // substitution #480 was filed to prevent. Taking a rendered reason rather than a
    // `postgres::Error` is what lets the apply path reach this closure at all: the door's
    // failures arrive as `ApplyError`, which has already read the `DbError` it needed.
    let interrupted =
        |digest: &[u8], released: usize, still_quarantined: usize, vanished: usize, why: String| {
            RequeueInterruptedError {
                message: requeue_interrupted_message(
                    examined,
                    released,
                    still_quarantined,
                    vanished,
                    &hex_prefix(digest),
                    &why,
                ),
                // null, NEVER 0: the #465 report never ran, and a number here would tell a
                // monitor this run had looked and found nothing (#465's own rule).
                metrics: snapshot(
                    released,
                    still_quarantined,
                    vanished,
                    serde_json::Value::Null,
                ),
            }
        };
    // Content addresses of the events this run put through the apply door (issue #465).
    // `sync_quarantine.content_digest` IS the event content address — the same key the
    // pull path's release DELETE uses — so no re-hashing is needed. As on the pull path,
    // an already-held event is recorded too: the flag_id watermark below, not newness, is
    // what keeps a re-delivery quiet (#466 review; see `unlearnable_references`).
    let mut released_addresses: Vec<Vec<u8>> = Vec::new();
    let flag_watermark = attachment_flag_watermark(client);
    for digest in &digests {
        // The row can vanish between the listing and here — and NOT only via an operator
        // DELETE, which an earlier version of this comment claimed. A concurrent pull
        // auto-releases resolved pen rows (the issue #111 behaviour, above), and a second
        // overlapping `requeue` takes them too; both are ordinary during exactly the
        // recovery session this command is run in. Counted rather than skipped silently,
        // because `examined` comes from the up-front listing: without this counter the
        // four numbers in the result object would not add up and nothing would say why.
        // #471: a bare `?` here discarded the whole report of the rows already released.
        // No counter has moved for THIS row yet — it has not been examined — so the counts
        // passed in are exactly the completed ones.
        let Some(row) = client
            .query_opt(
                "SELECT signed_bytes, attestation, attester_key
             FROM sync_quarantine WHERE content_digest=$1",
                &[digest],
            )
            .map_err(|e| {
                interrupted(
                    digest,
                    released,
                    still_quarantined,
                    vanished,
                    legible_db_error(&e),
                )
            })?
        else {
            vanished += 1;
            eprintln!(
                "requeue: {} left the pen between listing and release (a concurrent pull \
                 auto-released it, another requeue took it, or an operator deleted it) — \
                 examined but neither released nor still held here",
                hex_prefix(digest)
            );
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
                // #471: `released` has NOT been incremented for this row yet, and that is
                // correct — the event is in `event_log`, but the row is not released until
                // its pen entry is gone, and this is the statement that failed. Reporting
                // it as released would be a precise untruth about the pen's contents.
                client
                    .execute(
                        "DELETE FROM sync_quarantine WHERE content_digest=$1",
                        &[&digest],
                    )
                    .map_err(|e| {
                        interrupted(
                            digest,
                            released,
                            still_quarantined,
                            vanished,
                            legible_db_error(&e),
                        )
                    })?;
                released += 1;
                released_addresses.push(digest.clone());
                eprintln!(
                    "requeue: released {} through the apply door",
                    hex_prefix(digest)
                );
            }
            // THIS NODE'S OWN DATABASE failed, and no verdict about these bytes exists
            // (issue #480). `apply_signed` returns `ApplyError` for two entirely different
            // events: the door refusing (a `RAISE EXCEPTION`, SQLSTATE `P0001`) and the
            // statement never reaching the door at all — its FIRST statement is a newness
            // probe on `event_log`, and a dropped connection or a lock storm fails there.
            //
            // Recording the second as `last_requeue_error` claims the in-DB door
            // adjudicated these bytes and rejected them. It did not, and that annotation
            // is what an operator reads while deciding whether an event is corrupt. So it
            // takes the interruption path instead — the same partial-completion report
            // #471 built for the three raw statements around it, and the same routing the
            // PULL path already applies to this exact distinction (`refusal_is_deliberate`
            // → pen vs freeze).
            //
            // THE GUARD IS `apply_failure_is_local`, NOT `!is_deliberate_refusal()` — PR
            // #493 review. The first draft halted on every non-`P0001`, which is a wider
            // set than "our machine broke": a class-22 cast on a peer-supplied field, a
            // constraint violation, an `XX000` from a function fed adversarial bytes are
            // all DETERMINISTIC and attributable to the row itself. Halting on one of those
            // stops every future run at the same row — the listing is `ORDER BY first_seen`
            // — and `cairn-sync quarantine` is read-only, so raw SQL was the only remedy.
            // `db/001_envelope.sql`'s header records that exact failure one plane over.
            //
            // The accepted cost, and it is the pull path's cost too: a LOCAL fault stops
            // every requeue run at the same row rather than annotating it and moving on.
            // That is "delayed, never lost", loud, and it names the row and the SQLSTATE —
            // and it costs nothing, because the row behind it would meet the same broken
            // database anyway.
            Err(e) if apply_failure_is_local(e.sqlstate.as_deref()) => {
                return Err(Box::new(interrupted(
                    digest,
                    released,
                    still_quarantined,
                    vanished,
                    e.operator_text(),
                )));
            }
            Err(e) => {
                // Still not admitted: keep the row and record what happened beside (never
                // over) the original reason.
                // #471, and the distinction a reader will otherwise miss: THE TWO HALVES
                // OF THIS STATEMENT ARE ABOUT DIFFERENT ERRORS. The `e` being STORED
                // describes the APPLY; the error this `map_err` catches is a
                // `postgres::Error` from the UPDATE itself, which renders `db error` —
                // only that one is #467's species. `still_quarantined` has not moved for
                // this row yet, and it stays that way until the outcome is recorded.
                //
                // TWO kinds of `e` reach here (PR #493 review), and they are different
                // findings for the operator reading this column: the door's own `P0001`
                // VERDICT, and a non-deliberate failure that is nevertheless attributable
                // to these bytes and will recur identically forever. Both belong in the
                // pen — neither is fixable by retrying — but writing the second in the
                // first's voice is #480's defect in miniature, so it says what it is, in
                // the DATABASE's vocabulary (SQLSTATE included) rather than the door's.
                let annotation = if e.is_deliberate_refusal() {
                    e.to_string()
                } else {
                    format!(
                        "NOT a deliberate floor refusal — the apply door FAILED on these \
                         bytes and will fail identically on every retry: {}",
                        e.operator_text()
                    )
                };
                client
                    .execute(
                        "UPDATE sync_quarantine
                     SET last_seen = clock_timestamp(),
                         last_requeue_at = clock_timestamp(),
                         last_requeue_error = $2
                     WHERE content_digest = $1",
                        &[&digest, &annotation],
                    )
                    .map_err(|ue| {
                        interrupted(
                            digest,
                            released,
                            still_quarantined,
                            vanished,
                            legible_db_error(&ue),
                        )
                    })?;
                still_quarantined += 1;
                eprintln!(
                    "requeue: {} still not admitted: {annotation}",
                    hex_prefix(digest)
                );
            }
        }
    }
    // A released event goes through the SAME apply door a pulled one does (db/020), so
    // it can discover the same unlearnable references — and this path is even easier to
    // miss, because it prints one cheerful "released" line per row (issue #465).
    let references_unlearnable = emit_unlearnable_report(
        &mut io::stderr(),
        "requeue",
        flag_watermark
            .and_then(|since| unlearnable_references(client, &released_addresses, since))
            .map_err(|e| legible_db_error(&e)),
    );

    Ok(snapshot(
        released,
        still_quarantined,
        vanished,
        references_unlearnable,
    ))
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
    let mut client = connect_db(conn)?;
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
    match do_requeue(&mut client) {
        Ok(m) => {
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
        // #471 / ADR-0060 decision 2: an interrupted run still has a report to give, and it
        // goes out on the SAME channel a successful one uses — a monitor parsing
        // `--metrics` must not lose the object merely because the command failed, since
        // that object is the only record of the events this run DID release. The command
        // still exits non-zero: partial work is not success.
        Err(e) => {
            if metrics {
                if let Some(interrupted) = e.downcast_ref::<RequeueInterruptedError>() {
                    println!("{}", interrupted.metrics);
                }
            }
            Err(e)
        }
    }
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

/// The standing attachment-reference ledger (db/050), grouped as
/// `cairn_attachment_flag_health()` groups it: one row per (event type, refusal reason),
/// commonest first, each naming an example event to go and look at.
///
/// This is the caller that read surface lacked (issue #465). Note what it is NOT: the
/// per-cycle `references_unlearnable` metric answers "what did this pull just discover",
/// while this answers "what does this node stand holding" — the question an operator
/// woken by that line asks next, and the one that separates a single corrupted import
/// from a peer whose encoder is emitting garbage.
///
/// One JSON object per row, like [`quarantine_listing`], which also settles the peer-text
/// problem for free: `reason` embeds a prefix of the peer's own value (see
/// [`unlearnable_reference_message`]), and JSON string escaping means a newline in it can
/// never forge a second output line.
fn attachment_flag_listing(client: &mut postgres::Client) -> R<Vec<serde_json::Value>> {
    let rows = client.query(
        "SELECT event_type, reason, flagged, first_flagged::text, last_flagged::text,
                example_event::text
         FROM cairn_attachment_flag_health()",
        &[],
    )?;
    Ok(rows
        .iter()
        // `Option<_>` on every column: `Row::get` PANICS on a NULL, and this verb is
        // deliberately reachable during an outage (see `cmd_attachment_flags`) — a panic
        // here means no listing at all at the one moment it is wanted. Every column is
        // NOT NULL today, so this changes nothing now; under additive-only schema
        // evolution (principle 11) a widened health function renders as JSON `null`
        // instead of aborting the process. It does NOT cover a TYPE mismatch, which still
        // panics — that one is a build-time disagreement with db/050, not a runtime state.
        .map(|r| {
            serde_json::json!({
                "event_type": r.get::<_, Option<String>>(0),
                "reason": r.get::<_, Option<String>>(1),
                "flagged": r.get::<_, Option<i64>>(2),
                "first_flagged": r.get::<_, Option<String>>(3),
                "last_flagged": r.get::<_, Option<String>>(4),
                "example_event": r.get::<_, Option<String>>(5)
            })
        })
        .collect())
}

/// Print the standing attachment-reference ledger (see [`attachment_flag_listing`]).
///
/// `connect_checked` rather than the pgx-version gate, for `cmd_quarantine`'s reason: an
/// operator must be able to read what this node cannot fetch even during a stale-
/// `cairn_pgx` outage. This is a plain SELECT over a SECURITY DEFINER read; it calls no
/// cairn_pgx.
fn cmd_attachment_flags(conn: &str) -> R<()> {
    let mut client = connect_checked(conn)?;
    let listing = attachment_flag_listing(&mut client)?;
    for row in &listing {
        println!("{row}");
    }
    // Counted on stderr, NAMED on stdout: the count alone is what this surface exists to
    // improve on, so it never appears without the rows above it.
    //
    // BOTH numbers, because the group count alone understates by an unbounded factor —
    // a node holding 4700 flags from three encoder bugs is three groups, and the group
    // count is the number an eye lands on. The total is already in the rows.
    let flagged: i64 = listing.iter().filter_map(|r| r["flagged"].as_i64()).sum();
    eprintln!(
        "{} (event type, reason) group(s), {flagged} unlearnable attachment reference(s) \
         in total",
        listing.len()
    );
    Ok(())
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
    unwrap_key_path: Option<&str>,
) -> R<()> {
    let mut client = connect_checked_apply(conn)?;
    // Load this node's signing key so the pull presents an unwrap cert and gains
    // custody of any sealed events it replicates (ADR-0052 custody sidecar).
    let (sk, _kid) = load_or_create_key(key_path)?;
    // ADR-0066: LOAD this node's provisioned unwrap key rather than deriving one that
    // cannot match it. Refuses to start on a divergence, on a restored node, or on a
    // corrupt key file; warns loudly on the pre-ADR-0066 fallback (issue #503).
    let custody = unwrap_key::resolve_at_startup(&mut client, key_path, unwrap_key_path, sk)?;
    // A manual one-shot pull defaults to incremental; `--full` requests a sweep
    // from seq 0 (an explicit "reconcile everything now", the same path cmd_run
    // takes on cadence — issue #196).
    let m = do_pull(
        &mut client,
        &TcpTransport::new(peer),
        peer_name,
        full,
        Some(&custody),
    )?;
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
                        // #475's species: `postgres::Error`'s `Display` renders a refused
                        // socket, an unresolvable host and a TLS timeout all as `error
                        // connecting to server`. The errno lives in `source()`.
                        Err(e) => {
                            eprintln!("blob worker connect failed: {}", legible_db_error(&e));
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
                            let raw = match TcpTransport::new(peer).try_once(&Request::BlobSlice {
                                addr_hex: addr_hex.clone(),
                                offset,
                                len,
                            }) {
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
                                // Same rendering as the connect arms in this function
                                // (issue #479): `{e}` here is the two words `db error`,
                                // and the byte tier's failures are already invisible
                                // enough — this is the only line a starved chunk gets.
                                eprintln!("blob_chunk insert failed: {}", legible_db_error(&e));
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

/// `custody` (ADR-0052): this node's signing key and unwrap secret, wrapped in an
/// `Arc` so a clone can move into each per-connection thread where `serve_conn` uses
/// the carried secret to re-wrap DEKs. `None` serves without custody (events still
/// sync).
fn cmd_serve(
    conn: String,
    listen: &str,
    corrupt: bool,
    custody: Option<Arc<unwrap_key::NodeCustody>>,
) -> R<()> {
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
        let custody = custody.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_conn(&conn, stream, corrupt, custody) {
                eprintln!("connection error: {e}");
            }
        });
    }
    Ok(())
}

/// Format the byte-tier thread's per-pass failure line (issue #202). Pure so the
/// log contract is unit-testable: the line must name the subsystem, carry the
/// underlying cause, and say the loop retries.
fn blobd_error_line(e: &(dyn Error + 'static)) -> String {
    format!(
        "blobd pass failed (will retry next interval): {}",
        operator_chain(e)
    )
}

/// Format the run loop's "the fingerprint is missing, and here is why" line. **Pure.**
///
/// # The silence this ends (issue #479)
///
/// The call site was `if let Ok(fp) = do_fingerprint(&mut client)` with **no `else`, no
/// log line, nothing**. A schema skew made the `fingerprint` key vanish from every later
/// JSONL line and the status string quietly stop showing event counts, with zero evidence
/// why — and a reader of that log has no way to tell a node that stopped reporting from a
/// node that reported nothing to report.
///
/// The line therefore names three things: the missing artefact, the CONSEQUENCE for every
/// later line (so the gap in the log is self-explaining), and the diagnosis. It does NOT
/// say the loop retries, because a schema skew does not heal on the next cycle — claiming
/// it would be exactly the reassuring untruth principle 4 forbids.
fn fingerprint_error_line(e: &(dyn Error + 'static)) -> String {
    format!(
        "fingerprint failed — this cycle's line and status carry no event/blob counts: {}",
        operator_chain(e)
    )
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
    unwrap_key_path: Option<&str>,
) -> R<()> {
    // Load the node signing key ONCE up front (ADR-0052): the serve thread and the
    // pull loop must share the SAME custody — deriving it twice would race to create
    // the file and could leave serve and pull on different identities. One Arc feeds
    // both.
    let (sk, _kid) = load_or_create_key(key_path)?;
    // Connect and fail fast BEFORE spawning the serve thread. ADR-0066: LOAD this
    // node's provisioned unwrap key rather than deriving one that cannot match it.
    // Refuses to start on a divergence, on a restored node, or on a corrupt key file;
    // warns loudly on the pre-ADR-0066 fallback (issue #503).
    let mut client = connect_checked_apply(conn)?;
    let custody = Arc::new(unwrap_key::resolve_at_startup(
        &mut client,
        key_path,
        unwrap_key_path,
        sk,
    )?);
    {
        let (c, l) = (conn.to_string(), listen.to_string());
        let own_custody = Arc::clone(&custody);
        std::thread::spawn(move || {
            if let Err(e) = cmd_serve(c, &l, false, Some(own_custody)) {
                eprintln!("serve thread exited: {e}");
            }
        });
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
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
                // Same as the worker above: name the errno, not the layer.
                Err(e) => eprintln!("blob thread could not connect: {}", legible_db_error(&e)),
            },
        );
    }

    // Built ONCE, outside the cycle loop: `do_pull` is transport-agnostic (slice 2b,
    // #500) and this daemon's clinical pull always goes to the same TCP peer for the
    // life of the process — the same reuse the signing key above already gets.
    let transport = TcpTransport::new(peer);
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
            &transport,
            peer_name,
            full_sweep,
            Some(custody.as_ref()),
        ) {
            Ok(m) => {
                status += &format!(": pull {} shipped / {} new", m["shipped"], m["applied_new"]);
                line["pull"] = m;
            }
            Err(e) => {
                // THREE loud failure classes, kept DISTINCT in the machine-readable
                // log (the bet_a harness counts `partition` as link downtime —
                // #110 review finding 6):
                //   * integrity (unverifiable events / skew / pen refusal): the
                //     peer answered; the DATA is the problem. The per-cycle
                //     metrics still exist and are logged.
                //   * local_fault (issue #469): THIS NODE'S DATABASE failed —
                //     either after the events applied, failing to record where the
                //     cycle got to, or before the cycle reached the network at all.
                //     The first kind's metrics survive; they describe real work.
                //   * anything else (retries exhausted) = a partition.
                // A cycle can be integrity AND local_fault at once, so the classes
                // are a SET, not a choice. Both the mapping and the line-writing
                // live in `classify_pull_failure` / `record_pull_failure`, pure and
                // unit-tested, because a wrong branch here is a wrong DIAGNOSIS
                // handed to an operator at 3am.
                // Same rendering as the JSONL key below, for the same reason: `{e}`
                // over a boxed `postgres::Error` prints `db error` (issue #479).
                status += &format!(": PULL FAILED: {}", operator_chain(e.as_ref()));
                record_pull_failure(&mut line, e.as_ref());
            }
        }
        // Cumulative blobs fetched by the separate byte-tier thread (informational;
        // never blocks this loop).
        line["blobs_fetched"] = serde_json::json!(blobs_fetched.load(Ordering::Relaxed));
        match do_fingerprint(&mut client) {
            Ok(fp) => {
                status += &format!(
                    ", {} events, blobs {}+{}",
                    fp["events"], fp["blobs_present"], fp["blobs_referenced_only"]
                );
                line["fingerprint"] = fp;
            }
            // Never fatal — the fingerprint is a convergence PROBE, and a node that
            // cannot take one must still pull. But never silent either (issue #479): the
            // `if let Ok(..)` this replaces had no else arm at all, so a schema skew
            // dropped the key from every later line with no evidence anywhere.
            //
            // BOTH surfaces, deliberately: the operator reads stderr, `bet_a.py` reads the
            // JSONL, and the first cut of this arm wrote only to stderr — which left the
            // machine-readable artifact exactly as silent as before (PR #486 review).
            Err(e) => {
                record_fingerprint_failure(&mut line, e.as_ref());
                eprintln!("{}", fingerprint_error_line(e.as_ref()));
            }
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

/// What this node's admitted-peer trust set says about a presented unwrap cert's
/// `kid` (issue #231).
///
/// Deliberately a small closed vocabulary rather than a bare `bool`. Every non-grant
/// arm ends at the same place — no custody — but they are DIFFERENT operator problems
/// with different fixes, and collapsing them into one "custody withheld" line is
/// exactly how a silent replication stall becomes unreadable. Keeping them distinct is
/// what lets `decide_custody` print a remedy the reader can actually run.
///
/// (No count is stated here on purpose: the first review of this code found "five"
/// written in four places when there were already six, and #376's sequester will add
/// another. `decisions/README.md` names a miscounted count as the classic erratum.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrustLookup {
    /// `trust_peer` holds this kid with `status = 'active'` — an admitted peer.
    ActivePeer,
    /// This node once peered this kid, and that subject's latest op is `revoke`.
    ///
    /// Deliberately NOT read off `trust_peer.peer_pubkey`: `peer.revoked` carries only
    /// `peer_node_id_hex` in its payload (`identity::author_unpeer`), so db/007 stores
    /// a NULL `peer_pubkey` on the revoke row, and the view's `DISTINCT ON
    /// (subject_node_id)` lets that row REPLACE the `peer` row that held the key. A
    /// revoked kid therefore vanishes from `trust_peer` entirely. Reading it back needs
    /// the historical `peer` row — see `look_up_peer_trust`. (Review of PR #391: this
    /// arm was unreachable, so revoking a compromised peer printed "not among this
    /// node's admitted peers … admit it out of band" — an instruction to re-admit the
    /// node the operator had just deliberately cut off.)
    RevokedPeer,
    /// The trust set has peers, but none carrying this kid.
    NotAPeer,
    /// The node plane IS provisioned, but nothing has been peered yet — a healthy
    /// new node that simply has not run the pairing ceremony.
    NoPeersAdmitted,
    /// `local_node` is unset: `cairn-node init` was never run here. Kept distinct
    /// from [`Self::NoPeersAdmitted`] because `trust_peer` filters on
    /// `author_node_id = (SELECT node_id FROM local_node WHERE id)` — with
    /// `local_node` empty that subquery is NULL and the view yields zero rows *no
    /// matter how many peer events exist*, so the two states look identical from
    /// `trust_peer` alone and their first command differs (`init`, then `pair`).
    NodePlaneUninitialised,
    /// `trust_peer` does not exist in this database at all (SQLSTATE `42P01`):
    /// `db/007` was never loaded here. `cairn-sync`'s own SCHEMA subset
    /// deliberately excludes it — the node plane is `cairn-node`'s to provision.
    NodePlaneAbsent,
    /// The lookup itself failed for any other reason (permissions, a dropped
    /// session). Distinct from every "answer" above because the honest statement
    /// is *we do not know*, and principle 4 says an acknowledged unknown must not
    /// be dressed up as a finding.
    LookupFailed,
}

impl TrustLookup {
    /// Can the PULLER get back the bodies it already replicated without custody, once
    /// this cause is fixed?
    ///
    /// This is the property the operator line's recovery clause is keyed on, expressed
    /// as an exhaustive match so a NEW arm is a compile error here rather than silently
    /// inheriting someone else's remedy. (Its first version was a substring test over
    /// the message prose — `line.contains("cairn-node pair")` — which could quietly
    /// classify LESS as the wording changed, and did already miss one arm.)
    ///
    /// False for the two arms the puller cannot act on: a node plane that was never
    /// loaded, and a lookup that failed for an unknown reason. Telling a puller to
    /// re-sweep before the SERVING node is fixed is a remedy that runs and does nothing.
    fn puller_can_recover(self) -> bool {
        match self {
            // Nothing was withheld, so there is nothing to recover.
            TrustLookup::ActivePeer => false,
            TrustLookup::RevokedPeer
            | TrustLookup::NotAPeer
            | TrustLookup::NoPeersAdmitted
            | TrustLookup::NodePlaneUninitialised => true,
            TrustLookup::NodePlaneAbsent | TrustLookup::LookupFailed => false,
        }
    }
}

/// The serve side's custody decision for one pulling peer.
///
/// Both arms carry what acting on them requires, so a caller cannot take the decision
/// without also taking its consequence: `Grant` carries the very key the DEKs may be
/// re-wrapped for (so re-wrapping for a key the decision did not admit is a compile
/// error, not a review question), and `Withhold` carries both its typed cause and the
/// operator line that explains it — produced together, by one pure function, so there
/// is no way to log the wrong reason for a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CustodyAdmission {
    /// Re-wrap this node's DEKs for THIS key — the one carried in the verified cert
    /// whose `kid` the trust set just admitted.
    Grant { requester_pub: [u8; 32] },
    /// Serve the events WITHOUT custody, and print this line.
    Withhold {
        cause: TrustLookup,
        operator_line: String,
    },
}

/// Decide whether a pulling peer may obtain read-custody of this node's sealed
/// bodies, from one trust-set lookup (issue #231; the ADR-0052 hardening deferred
/// as "custody is designed to follow admission").
///
/// **Fail closed.** Only `ActivePeer` grants. A DEK is what populates `event_clear`
/// and opens the sealed plaintext, so handing one over confers clinical-data READ —
/// not merely a later crypto-shred capability. Every uncertainty therefore withholds.
///
/// **Withhold, never refuse the pull.** The events still ship: they are sealed
/// ciphertext, harmless without a DEK, and refusing them would wedge replication for
/// an availability gain of nothing (principle: availability over consistency; a
/// refusal here would fork the event set). This is the same degradation the arm
/// already performs for an absent or malformed cert — an unadmitted kid simply joins
/// that path.
///
/// The lines name a remedy the reader can run from what was printed (the Slice 61
/// lesson), and the two provisioning arms name the NODE PLANE rather than the peer,
/// because blaming the puller for this node's un-provisioned state sends the operator
/// hunting the wrong problem.
///
/// **Why the recovery clause names TWO steps, not "pull again".** Withheld custody is
/// repairable, but only by a remedy in two parts, and naming half of it is what makes
/// a safety refusal worse than useless:
///
/// 1. `pull --full`. `apply_remote_event` has no early return for an event already in
///    the log, and its custody insert is `ON CONFLICT (event_id) DO NOTHING`, so a
///    re-offer that *does* carry a DEK fills in the missing `event_dek` / `event_clear`
///    rows. But an incremental pull only asks for `seq > cursor`, and by the time the
///    operator reads this line the cursor is already past the custody-less events. Only
///    the full sweep (`after_seq = 0`) re-offers them. (The periodic `FULL_SWEEP_EVERY`
///    sweep gets there eventually; `--full` is the same thing on demand.)
/// 2. `cairn_reproject`. The sweep restores custody and NOT the chart: the projection
///    dispatcher is an `AFTER INSERT` trigger on `event_log` (db/005), and the re-apply
///    inserts no row (`ON CONFLICT DO NOTHING`, db/020), so it never fires. The
///    projections were built when the events first applied — without a clear view — and
///    a re-apply does not rebuild them. Heal mode (`p_rebuild` false, the default)
///    replays the apply fns over the now-readable events without truncating anything.
///
/// Step 2 is the review finding that made this clause honest: measured, `pull --full`
/// alone took custody from `(0,0)` to `(1,1)` and left `medication_statement` at zero —
/// the clinician's chart still empty after following the printed instruction, which is
/// the Slice 61 failure one layer down. `an_admitted_peer_recovers_the_bodies_it_pulled_without_custody`
/// pins both steps, including the fact that step 1 alone is not enough.
///
/// The one arm with no repair path is a SHRED: `db/020` step 9 refuses custody for a
/// target in `erasure_shred_log` however often it is re-delivered. That is deliberate
/// anti-resurrection, not a gap.
fn decide_custody(kid: &str, requester_pub: [u8; 32], lookup: TrustLookup) -> CustodyAdmission {
    // The recovery clause is shared, and emitted ONLY for the causes the puller can
    // actually act on (`puller_can_recover`) — a typed property, not a grep over this
    // prose. Keeping it out of the per-cause text leaves each line about its CAUSE and
    // stops six copies of the same two-step instruction drifting apart.
    let recovery = if lookup.puller_can_recover() {
        " Once that is done the puller recovers the bodies it already replicated in TWO \
         steps: `cairn-sync pull --full` (an incremental pull cannot reach events below \
         its cursor), THEN `SELECT cairn_reproject()` on the puller as its DB owner — \
         the sweep restores custody but NOT the projections, which were built without a \
         clear view and are not rebuilt by a re-apply."
    } else {
        ""
    };
    // One shared prefix, so every line reads the same way and an operator grepping
    // logs for lost custody finds every cause with one pattern.
    let withhold = |cause_text: &str| CustodyAdmission::Withhold {
        cause: lookup,
        operator_line: format!(
            "cairn-sync serve: custody WITHHELD from puller {kid} — {cause_text} \
             (the events still sync; their sealed bodies stay unreadable there).{recovery}"
        ),
    };
    match lookup {
        TrustLookup::ActivePeer => CustodyAdmission::Grant { requester_pub },
        TrustLookup::RevokedPeer => withhold(
            "this key belongs to a REVOKED peer. If the revocation was intended, \
             nothing to do — this node is refusing custody exactly as asked. If not, \
             re-pair it out of band (`cairn-node pair-offer` / `pair-accept`)",
        ),
        TrustLookup::NotAPeer => withhold(
            "this key is not among this node's admitted peers. Admit it out of band \
             (`cairn-node pair-offer` / `pair-accept`)",
        ),
        TrustLookup::NoPeersAdmitted => withhold(
            "this node has admitted no peers at all yet. Pair this puller out of band \
             (`cairn-node pair-offer` / `pair-accept`)",
        ),
        TrustLookup::NodePlaneUninitialised => withhold(
            "this node's node plane was never initialised (`local_node` is unset, \
             which empties `trust_peer` whatever peer events exist). Run `cairn-node \
             init` here, then pair this puller (`cairn-node pair-offer` / `pair-accept`)",
        ),
        TrustLookup::NodePlaneAbsent => withhold(
            "the lookup raised SQLSTATE 42P01 (undefined_table) — see the serve log \
             line above for which relation. The usual cause is that the node plane \
             (db/007) was never loaded against this database, so no peer can be \
             admitted here; a `search_path` that cannot see it does the same. Provision \
             it with `cairn-node` against this database, then pair this puller",
        ),
        TrustLookup::LookupFailed => withhold(
            "the trust-set lookup itself failed, so admission is UNKNOWN and custody \
             fails closed. Check this connection's SELECT grant on `trust_peer` and \
             the serve log line above for the database error",
        ),
    }
}

/// Ask the node-plane trust set about one unwrap-cert `kid`.
///
/// The admission predicate is the node plane's own —
/// `peer_pubkey = <kid> AND status = 'active'` — matching the set `refresh_trust_set`
/// snapshots for `cairn-node`'s mTLS cert-pin verifier (`transport::pinned` tests
/// membership of that snapshot rather than re-querying, so this is the same trust set
/// under the same `status` grading, not a second definition of who is admitted).
///
/// Four booleans in one round trip, deliberately: one query means one SNAPSHOT, so the
/// four facts cannot disagree with each other the way four round trips could (a peer
/// revoked between the `active` probe and the `revoked` probe). `EXISTS` rather than a
/// row fetch because `trust_peer` is `DISTINCT ON (subject_node_id)`, so one key
/// re-registered under two node ids could yield two rows; any-active wins, matching
/// `refresh_trust_set`, which collects every active `peer_pubkey` into one flat set.
///
/// **The `revoked` probe cannot read `trust_peer.peer_pubkey`** — that is the review
/// defect this shape exists to fix. `peer.revoked` carries only `peer_node_id_hex`
/// (`identity::author_unpeer`), so db/007 stores a NULL `peer_pubkey` on the revoke
/// row, and `DISTINCT ON (subject_node_id) … ORDER BY hlc DESC` lets that row REPLACE
/// the `peer` row that held the key: a revoked kid is simply absent from the view.
/// Measured, `EXISTS (… WHERE peer_pubkey = $1)` returned false for a peer that had
/// just been revoked, so the revoked arm was dead and a revoked peer was told it was
/// "not among this node's admitted peers … admit it out of band". Recovering the fact
/// therefore needs the HISTORICAL `peer` row: resolve the kid to the subject it was
/// peered as, then read THAT subject's current status. (`refresh_trust_set`'s
/// `AND peer_pubkey IS NOT NULL` guard is the same NULL, seen from the other side.)
///
/// The fourth boolean reads `local_node`, which is the ONLY thing that separates
/// "provisioned but nothing peered" from "never initialised" — `trust_peer` is empty
/// in both, because it filters on a `local_node` subquery that is NULL in the second.
///
/// Comparison is exact, not case-folded, deliberately: `peer_pubkey` is stored verbatim
/// from the pairing bundle and every other consumer of the trust set compares it
/// verbatim too, so folding HERE would make custody admit a peer the mTLS pin rejects.
/// Normalising belongs at the pairing door, once, on write — filed as #392.
///
/// Errors never propagate: a failed lookup is an *answer* here (fail closed), not a
/// reason to drop the connection and deny the peer its events.
fn look_up_peer_trust(client: &mut postgres::Client, kid: &str) -> TrustLookup {
    let row = client.query_one(
        "SELECT EXISTS (SELECT 1 FROM trust_peer WHERE peer_pubkey = $1 AND status = 'active'),
                EXISTS (SELECT 1 FROM trust_peer t
                         WHERE t.status = 'revoked'
                           AND t.peer_node_id IN (
                               SELECT ne.subject_node_id FROM node_event ne
                                WHERE ne.op = 'peer' AND ne.peer_pubkey = $1
                                  AND ne.author_node_id = (SELECT node_id FROM local_node WHERE id))),
                EXISTS (SELECT 1 FROM trust_peer),
                EXISTS (SELECT 1 FROM local_node WHERE id)",
        &[&kid],
    );
    match row {
        Ok(r) => {
            // try_get, not get: an EXISTS can never be NULL, so this is unreachable —
            // but `get` PANICS, and a panic here unwinds the per-connection thread
            // WITHOUT `serve_conn`'s error line, leaving the puller a bare EOF it
            // misdiagnoses as an old-binary wire mismatch. Fail closed totally rather
            // than incidentally.
            let probes = (r.try_get(0), r.try_get(1), r.try_get(2), r.try_get(3));
            let (Ok(active), Ok(revoked), Ok(any_peer), Ok(provisioned)): (
                Result<bool, _>,
                Result<bool, _>,
                Result<bool, _>,
                Result<bool, _>,
            ) = probes
            else {
                eprintln!(
                    "cairn-sync serve: trust-set lookup for puller {kid} returned an \
                     unreadable row — treating admission as UNKNOWN"
                );
                return TrustLookup::LookupFailed;
            };
            // The probes form an implication chain (`active` and `revoked` are mutually
            // exclusive, and either implies `any_peer` implies `provisioned`), so the
            // wildcards below absorb combinations the SQL cannot produce rather than
            // untested ones. Order is the precedence: a key that is active SOMEWHERE
            // wins, matching refresh_trust_set's flat any-active set.
            match (active, revoked, any_peer, provisioned) {
                (true, _, _, _) => TrustLookup::ActivePeer,
                (false, true, _, _) => TrustLookup::RevokedPeer,
                (false, false, true, _) => TrustLookup::NotAPeer,
                (false, false, false, true) => TrustLookup::NoPeersAdmitted,
                (false, false, false, false) => TrustLookup::NodePlaneUninitialised,
            }
        }
        Err(e) => {
            // 42P01 = undefined_table. That is a provisioning fact about THIS
            // database, not a fault, so it gets its own arm and its own line rather
            // than being lumped in with a genuine failure.
            //
            // Both arms print the error, and BOTH name the kid: the operator lines
            // point the reader at "the serve log line above", and a serve process runs
            // one thread per connection, so an unattributed error line can be read
            // against the wrong peer's refusal when two pullers overlap.
            let undefined_table = e
                .code()
                .is_some_and(|c| c == &postgres::error::SqlState::UNDEFINED_TABLE);
            // The AUTHORIZATION path for an inbound peer: "why did this peer stop being
            // served" was answered with the eight characters `db error`, on the one line
            // the operator lines above point the reader at. `e.code()` is read one
            // statement up, so the SQLSTATE was in hand and thrown away (issue #479).
            eprintln!(
                "cairn-sync serve: trust-set lookup for puller {kid} failed: {}",
                legible_db_error(&e)
            );
            if undefined_table {
                TrustLookup::NodePlaneAbsent
            } else {
                TrustLookup::LookupFailed
            }
        }
    }
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
/// plaintext), not merely a later crypto-shred capability. Who may obtain it is gated by
/// the caller: since issue #231 the serve arm pins the requester's unwrap-cert `kid` to
/// the node-plane trust set (`decide_custody`) and passes `requester_pub = None` for
/// anyone unadmitted, so this function only ever re-wraps for an admitted peer.
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

/// `custody` (ADR-0052): this node's signing key and unwrap secret, resolved once at
/// startup and shared across connection threads. `None` serves without custody (events
/// still sync). One `Arc` is cloned into each per-connection thread.
fn serve_conn(
    conn: &str,
    mut stream: TcpStream,
    corrupt: bool,
    custody: Option<Arc<unwrap_key::NodeCustody>>,
) -> R<()> {
    let mut client = connect_db(conn)?;
    // Our own unwrap secret, resolved once at startup and shared across connection
    // threads (ADR-0026 escrow: the secret is never in the DB). Used only to open our
    // locally-stored DEKs so the EventsAfterSeq arm can re-wrap them for the peer.
    let own_secret = custody.as_ref().map(|c| c.unwrap_secret.clone());
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
                // …and therefore nothing was WITHHELD either: this arm cannot receive
                // a cert, so there was never an admission decision to report.
                custody_withheld: None,
                // Legacy HLC arm carries NO custody sidecar (ADR-0052): it cannot
                // receive an unwrap cert, so it never re-wraps DEKs. Empty = no
                // custody; sealed events still sync (admitted structurally).
                wrapped_deks: vec![],
                // This arm is unpaginated BY CONSTRUCTION (it has no `limit` to honour
                // and ships the whole suffix), so it always drains the log (slice 2b).
                complete: true,
            })?
        }
        Request::EventsAfterSeq {
            after_seq,
            unwrap_cert,
            limit,
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
            //
            // Fetch ONE MORE than asked for, then serve `limit` (slice 2b, #101 item
            // 1). `rows.len() == limit` cannot distinguish "the log ends exactly here"
            // from "there is a next event we cut off", and `complete` is the puller's
            // only termination signal — a wrong `true` at that boundary strands every
            // event above it, forever, with the cursor checkpointed past them. The
            // extra row answers the question instead of inferring it. `LIMIT NULL` is
            // Postgres for "no limit", so the unpaginated path stays the SAME
            // statement with a NULL parameter rather than a second query.
            let probe: Option<i64> = limit.map(|n| i64::from(n) + 1);
            let mut rows = client.query(
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
                 ORDER BY e.seq
                 LIMIT $2",
                &[&after_seq, &probe],
            )?;
            // Did that fetch drain the log? `None` (unpaginated) always does, by
            // construction. Otherwise: more rows came back than the caller asked for
            // means there is at least one more beyond `limit` — truncate the probe row
            // back off (it must never reach the wire) and report `complete: false`.
            let complete = match limit {
                None => true,
                Some(n) => {
                    let n = n as usize;
                    let more_remain = rows.len() > n;
                    rows.truncate(n);
                    !more_remain
                }
            };
            let seqs = rows.iter().map(|r| r.get::<_, i64>(0)).collect();
            let events = rows.iter().map(|r| r.get::<_, String>(1)).collect();
            let attestations = rows.iter().map(|r| r.get::<_, Option<String>>(2)).collect();
            let attester_keys = rows.iter().map(|r| r.get::<_, Option<String>>(3)).collect();
            // Each event's DEK still wrapped for OUR key (None = no custody / shredded).
            let local_deks: Vec<Option<String>> =
                rows.iter().map(|r| r.get::<_, Option<String>>(4)).collect();

            // Admit the pulling peer to CUSTODY, in two steps (ADR-0052 + issue #231).
            //
            // 1. `verify_unwrap_key_cert` proves the cert is internally consistent:
            //    the kid signed it, and the payload does not lie about that kid, so
            //    `requester_pub` provably belongs to that identity.
            // 2. That identity is then PINNED to this node's admitted-peer trust set
            //    (`trust_peer`, db/007) — the same set the mTLS cert-pin verifier and
            //    the node-plane admission gate consult.
            //
            // Step 2 is what makes admission, not transport, the boundary on
            // read-custody. Before it, ANY self-signed unwrap cert reaching this port
            // had its DEKs re-wrapped and thereby obtained READ-custody of every
            // non-shredded sealed body this node serves — the DEK is what populates
            // event_clear and opens the sealed plaintext, so custody confers
            // clinical-data READ, not merely a future shred capability, and the link
            // (WireGuard / mTLS) was the sole access control.
            //
            // A withheld decision serves the events WITHOUT custody — never a refused
            // pull. The bodies are sealed ciphertext, harmless without a DEK, and
            // refusing them would wedge replication for no confidentiality gain. This
            // is the same degradation an absent or malformed cert already takes.
            //
            // A REJECTED cert is logged, not silently dropped. The benign case (an
            // honest, not-yet-paired peer) used to be the only loud one, which inverted
            // the priority: `CertKidMismatch` is a cert whose payload claims a kid it
            // did not sign — an attempted impersonation of an admitted peer, i.e. an
            // attack on this very pin — and it left no trace at all.
            let verified_cert = match unwrap_cert.as_deref() {
                None => None,
                Some(hexed) => {
                    let parsed = hex::decode(hexed).map_err(|e| e.to_string()).and_then(|c| {
                        cairn_event::verify_unwrap_key_cert(&c).map_err(|e| e.to_string())
                    });
                    match parsed {
                        Ok(pair) => Some(pair),
                        Err(reason) => {
                            eprintln!(
                                "cairn-sync serve: unwrap cert REJECTED — {reason}. No custody \
                                 travels for this pull (the events still sync). A kid mismatch \
                                 or bad signature here is an impersonation attempt, not a \
                                 misconfiguration; a malformed key is usually a corrupt \
                                 `--key` file on the puller."
                            );
                            None
                        }
                    }
                }
            };
            // Only ask the trust set when custody could actually travel. Without this
            // guard the lookup — and its multi-line refusal — fires on EVERY pull from
            // an un-peered node, including batches with no sealed events at all, where
            // nothing was going to be re-wrapped whatever the answer. In `run` mode
            // that is one identical refusal per interval, forever, teaching the operator
            // to filter out the exact `custody WITHHELD` prefix this design chose so
            // every cause could be found with one grep. (Repeats across cycles are left
            // alone deliberately: a standing refusal IS a standing problem, and
            // de-duplicating across per-connection threads would buy quiet with shared
            // mutable state.)
            let custody_could_travel = local_deks.iter().any(Option::is_some);
            let mut custody_withheld: Option<String> = None;
            let requester_pub =
                verified_cert
                    .filter(|_| custody_could_travel)
                    .and_then(|(kid, pubk)| {
                        match decide_custody(&kid, pubk, look_up_peer_trust(&mut client, &kid)) {
                            CustodyAdmission::Grant { requester_pub } => Some(requester_pub),
                            CustodyAdmission::Withhold {
                                operator_line,
                                cause: _,
                            } => {
                                eprintln!("{operator_line}");
                                custody_withheld = Some(operator_line);
                                None
                            }
                        }
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
                // Send the refusal to the party that experiences it (issue #231 review).
                // The line above lands on THIS node's stderr, but the symptom — a chart
                // whose sealed bodies will not render — appears at the puller, often at
                // another site with another operator, and the remedy names steps THEY
                // must run. Additive + serde-default, so an older peer simply ignores it.
                custody_withheld,
                // The limit+1 probe above answered this precisely (slice 2b) — never
                // inferred from `rows.len() == limit`, which cannot tell "drained" from
                // "cut off at the boundary" apart.
                complete,
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
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics] [--full] [--key PATH] [--unwrap-key PATH]
              (--key: this node's signing key; --unwrap-key: its custody key, default <key>.unwrap — ADR-0066)
  quarantine  --conn URI    (list refused events: digest, peer, reason, requeue error, acked)
  attachment-flags --conn URI
              (list attachment references this node admitted but cannot fetch: type, reason, count, example)
  requeue     --conn URI [--metrics]
              (re-process quarantined events through the apply door after fixing the cause)
  blobd       --conn URI (--peer HOST:PORT | --blob-peer HOST:PORT ...) [--window N] [--budget-ms N] [--metrics]
  serve       --conn URI --listen HOST:PORT [--corrupt] [--key PATH] [--unwrap-key PATH]
              (--key: this node's signing key; --unwrap-key: its custody key, default <key>.unwrap — ADR-0066)
  fingerprint --conn URI    (convergence/honest-state JSON for the harness)
  run         --conn URI --listen HOST:PORT --peer HOST:PORT --peer-name NAME
              [--blob-peer HOST:PORT ...] [--window N] [--interval-ms N] [--budget-ms N] [--log PATH] [--duration-s N] [--key PATH] [--unwrap-key PATH]
              (unattended: serve+pull+blob, logs one JSON line/cycle, survives drops;
              --key: this node's signing key; --unwrap-key: its custody key, default <key>.unwrap — ADR-0066)
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
            flag(&args, "--unwrap-key").as_deref(),
        )?,
        "quarantine" => cmd_quarantine(&need(conn))?,
        "attachment-flags" => cmd_attachment_flags(&need(conn))?,
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
            let key_path = flag(&args, "--key").unwrap_or_else(|| "node.key".into());
            let (sk, _kid) = load_or_create_key(&key_path)?;
            // ADR-0066: LOAD this node's provisioned unwrap key rather than deriving one
            // that cannot match it — a one-off connection just to resolve it, since
            // (unlike `pull`/`run`) `serve` otherwise never touches the database until
            // its first accepted connection. Refuses to start on a divergence, on a
            // restored node, or on a corrupt key file; warns loudly on the pre-ADR-0066
            // fallback (issue #503).
            let mut startup_client = connect_checked_apply(&need(conn.clone()))?;
            let custody = Arc::new(unwrap_key::resolve_at_startup(
                &mut startup_client,
                &key_path,
                flag(&args, "--unwrap-key").as_deref(),
                sk,
            )?);
            drop(startup_client);
            cmd_serve(
                need(conn),
                &need(flag(&args, "--listen")),
                args.iter().any(|a| a == "--corrupt"),
                Some(custody),
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
            flag(&args, "--unwrap-key").as_deref(),
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

    /// **#469.** A local database write failure must not be logged as link downtime.
    ///
    /// The three classes are genuinely different operator actions — look at the peer's
    /// data, look at this node's database, look at the network — and the log line is
    /// where that distinction is made. Before this, the second collapsed into the third
    /// through `run`'s catch-all, and the Bet A availability figure was charged for it.
    ///
    /// The metrics half is the other reason this class exists: an integrity failure and a
    /// cursor-commit failure both happen AFTER events have applied, so both must publish
    /// what the cycle did. Only a partition — where nothing was reached — has none.
    #[test]
    fn a_cursor_commit_failure_is_a_local_fault_not_a_partition() {
        let integrity = PullIntegrityError {
            message: "unverifiable".into(),
            metrics: serde_json::json!({"applied_new": 3}),
            also_local_fault: false,
        };
        let (classes, metrics) = classify_pull_failure(&integrity);
        assert_eq!(classes, &["integrity"]);
        assert_eq!(metrics["applied_new"], 3, "the cycle's work is published");

        let local = CursorCommitError {
            message: "cursor commit failed".into(),
            metrics: serde_json::json!({"applied_new": 7}),
            also_loud: false,
        };
        let (classes, metrics) = classify_pull_failure(&local);
        assert_eq!(
            classes,
            &["local_fault"],
            "this node's database, NOT the WAN — the whole point of #469"
        );
        assert_eq!(
            metrics["applied_new"], 7,
            "seven events really did apply and the cycle must say so"
        );

        // The catch-all, unchanged: a transport failure with nothing to report.
        let transport: Box<dyn Error> = "connection refused".into();
        let (classes, metrics) = classify_pull_failure(transport.as_ref());
        assert_eq!(classes, &["partition"]);
        assert!(
            metrics.is_null(),
            "nothing was reached, so nothing is claimed"
        );
    }

    /// **PR #472 review, finding 5.** A cycle can be BOTH, and must be logged as both.
    ///
    /// A failed cursor commit on a cycle that also quarantined unverifiable events used to
    /// return before `cycle_is_loud` was ever consulted, so `integrity` was never set: the
    /// operator lost `loud_pull_message`'s remedies (`ack-quarantine`, `requeue`, re-sign)
    /// and `bet_a.py`'s INTEGRITY counter missed the cycle. Worse, the local-fault line
    /// says the events "re-apply idempotently", which is the WRONG remedy for a
    /// quarantined event — reassuring, and false.
    #[test]
    fn a_cycle_that_is_both_loud_and_locally_broken_reports_both() {
        let both = CursorCommitError {
            message: "cursor commit failed AND, INDEPENDENTLY: 2 unverifiable".into(),
            metrics: serde_json::json!({"applied_new": 1, "skipped_unverifiable": 2}),
            also_loud: true,
        };
        let (classes, metrics) = classify_pull_failure(&both);
        assert!(
            classes.contains(&"local_fault") && classes.contains(&"integrity"),
            "both conditions are true, so both keys are set: {classes:?}"
        );
        assert_eq!(metrics["skipped_unverifiable"], 2, "{metrics}");
    }

    /// **PR #472 review, finding 3.** A bare `postgres::Error` is a LOCAL fault.
    ///
    /// `do_pull` talks to its peer over a raw `TcpStream`, so a postgres error escaping it
    /// can only have come from THIS node's database. Its first two statements — the
    /// `sync_state` upsert and read — are bare `?` on exactly that, and they run BEFORE
    /// any network I/O, so before this arm existed a dead local database was reported as
    /// link downtime every cycle for the life of the process. That is #469's own title
    /// sentence, two statements above where #469 was fixed.
    #[test]
    fn a_bare_postgres_error_is_this_node_not_the_link() {
        // A real `postgres::Error`, obtained without a server: an unparseable connection
        // string fails in Config's own parser. What matters is the TYPE reaching the
        // classifier, not which kind of postgres failure it is.
        let e = "host=localhost port=not-a-number"
            .parse::<postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string");
        let boxed: Box<dyn Error> = Box::new(e);
        let (classes, metrics) = classify_pull_failure(boxed.as_ref());
        assert_eq!(
            classes,
            &["local_fault"],
            "a database error on this node's own client is never a partition"
        );
        assert!(
            metrics.is_null(),
            "these escape before the metrics object exists, so nothing is claimed"
        );
    }

    /// **PR #472 review, finding 8.** The seam where the classification becomes observable.
    ///
    /// `classify_pull_failure` was well tested and the four lines that USED it were not —
    /// yet those are what `bet_a.py` reads. A dropped `line["pull"]`, a hardcoded key or a
    /// swapped class would have passed the whole suite.
    #[test]
    fn the_log_line_carries_the_class_the_error_text_and_the_surviving_metrics() {
        let mut line = serde_json::json!({"ts": 1, "cycle": 4});
        record_pull_failure(
            &mut line,
            &CursorCommitError {
                message: "committing the sync_state cursor to seq 412 FAILED".into(),
                metrics: serde_json::json!({"applied_new": 7}),
                also_loud: false,
            },
        );
        assert_eq!(line["local_fault"], true, "{line}");
        assert!(line.get("partition").is_none(), "not a partition: {line}");
        assert!(
            line["pull_error"]
                .as_str()
                .is_some_and(|t| t.contains("seq 412")),
            "the operator-facing text travels into the machine-readable log: {line}"
        );
        assert_eq!(
            line["pull"]["applied_new"], 7,
            "and the work the cycle really did: {line}"
        );

        // A partition claims nothing, and must not leave a stale `pull` object behind.
        let mut line = serde_json::json!({"ts": 2, "cycle": 5});
        let transport: Box<dyn Error> = "connection refused".into();
        record_pull_failure(&mut line, transport.as_ref());
        assert_eq!(line["partition"], true, "{line}");
        assert!(line.get("pull").is_none(), "nothing was reached: {line}");
    }

    /// The node crate composes the byte-identical shape; this is the sync half of the pair.
    /// `cairn-node`'s `db_diagnosis::tests::the_agreed_shape_is_exactly_this` asserts the
    /// same two strings, so a change to either renderer turns the OTHER crate red rather
    /// than drifting silently (PR #472 review, finding 7 on the twin).
    #[test]
    fn the_twin_composes_the_agreed_shape() {
        assert_eq!(
            compose_db_diagnosis("permission denied for table event_log", "42501", None, None),
            "permission denied for table event_log [42501]"
        );
        assert_eq!(
            compose_db_diagnosis("could not obtain lock", "55P03", Some("why"), Some("how")),
            "could not obtain lock [55P03] — why — HINT: how"
        );
    }

    /// A server message can carry newlines, and these renderings land in a one-line-per-
    /// event log. `unlearnable_report` escapes its cause for this reason; the general
    /// renderer must not be the weaker one (PR #472 review).
    #[test]
    fn a_diagnosis_never_forges_a_log_line() {
        let text = compose_db_diagnosis(
            "refused\npull peer-a: 0 attachment reference(s) unlearnable",
            "P0001",
            Some("first\r\nsecond"),
            None,
        );
        assert!(
            !text.contains('\n') && !text.contains('\r'),
            "no rendering may span two lines: {text:?}"
        );
        assert!(text.contains("second"), "collapse, never drop: {text}");
    }

    /// The message must send the operator to the right place, and say the work survived.
    #[test]
    fn the_cursor_failure_message_names_this_node_not_the_link() {
        let text = cursor_commit_failure_message(
            "peer-a",
            7,
            412,
            "could not serialize access due to concurrent update [40001]",
            None,
        );
        assert!(text.contains("peer-a"), "the peer is named: {text}");
        assert!(
            text.contains("7 event(s) applied"),
            "the work that survived is stated first, so 'PULL FAILED' does not read as \
             'nothing arrived': {text}"
        );
        assert!(text.contains("412"), "the seq it tried to commit: {text}");
        assert!(
            text.contains("DATABASE") && text.contains("not the link"),
            "an operator reading 'pull failed' reaches for the network first — this line \
             exists to stop that: {text}"
        );
        assert!(
            text.contains("[40001]"),
            "and the legible cause travels with it, never 'db error': {text}"
        );
    }

    /// **PR #472 review, finding 4.** The prose may not claim what the metric withholds.
    ///
    /// `mark_cursor_outcome_unknown` nulls `cursor_seq` BECAUSE the statement may have
    /// committed before the connection broke. An earlier draft of this line then told the
    /// operator flatly that "the cursor did not advance" — principle 4 applied to the
    /// machine-readable field and dropped in the sentence a human actually reads, which
    /// invites a manual cursor reset on a cursor that may well have moved.
    ///
    /// What must survive the hedge is the SELF-HEALING claim: that one holds either way,
    /// and it is the difference between "watch it" and "act now".
    #[test]
    fn the_cursor_failure_message_does_not_claim_an_outcome_it_nulled() {
        let text = cursor_commit_failure_message("peer-a", 7, 412, "connection closed", None);
        assert!(
            !text.contains("the cursor did not advance"),
            "the metric says unknown, so the prose may not say certain: {text}"
        );
        assert!(
            text.contains("unknown"),
            "and it says so out loud rather than going quiet: {text}"
        );
        assert!(
            text.contains("re-applies idempotently"),
            "the self-healing claim holds either way and must not be lost to the \
             hedge — without it an operator cannot tell watch-it from act-now: {text}"
        );
    }

    /// **PR #472 review, finding 5.** Both diagnoses travel, and the loud one is marked
    /// as independent so it does not read as an explanation of the commit failure.
    #[test]
    fn a_loud_cycle_that_also_failed_its_commit_carries_both_diagnoses() {
        let text = cursor_commit_failure_message(
            "peer-a",
            1,
            9,
            "connection closed",
            Some("pull peer-a: 2 event(s) UNVERIFIABLE and quarantined; run `ack-quarantine`"),
        );
        assert!(
            text.contains("committing the sync_state cursor"),
            "the local fault is still stated: {text}"
        );
        assert!(
            text.contains("ack-quarantine"),
            "and the integrity remedy is NOT swallowed by it — a quarantined event does \
             not 're-apply idempotently', so losing this line loses the only correct \
             instruction: {text}"
        );
        assert!(
            text.contains("INDEPENDENTLY"),
            "and the two are marked as separate conditions, not cause and effect: {text}"
        );
    }

    /// **#469.** A failed write's intended values are never reported as its outcome.
    ///
    /// `cursor_seq` and `floor_active` describe `sync_state` AFTER the commit. Publishing
    /// the values the failed statement would have set tells a monitor the cursor advanced
    /// when it did not — and if the connection dropped mid-statement, this node cannot
    /// know which happened at all. Principle 4: null is a first-class unknown; a number
    /// is a claim. Everything describing completed work is untouched.
    #[test]
    fn an_uncommitted_cursor_reports_unknown_not_the_value_it_tried() {
        let mut metrics = serde_json::json!({
            "applied_new": 7, "shipped": 9, "cursor_seq": 412, "floor_active": true,
            "references_unlearnable": 2,
        });
        mark_cursor_outcome_unknown(&mut metrics);

        assert!(
            metrics["cursor_seq"].is_null(),
            "the cursor did NOT reach 412: {metrics}"
        );
        assert!(
            metrics["floor_active"].is_null(),
            "the floor write is in the same statement, so it is equally unknown: {metrics}"
        );
        assert_eq!(metrics["applied_new"], 7, "completed work is untouched");
        assert_eq!(metrics["shipped"], 9, "completed work is untouched");
        assert_eq!(
            metrics["references_unlearnable"], 2,
            "the flags were written before the commit and are durable: {metrics}"
        );
    }

    /// ADR-0056 decision 5 routing: only the door's own `RAISE EXCEPTION` (P0001)
    /// is a verdict about the event. Everything else is infrastructure trouble,
    /// where the same bytes may apply on the very next attempt — pen those and
    /// the pen fills with events that were never refused.
    #[test]
    fn only_a_bare_raise_is_a_deliberate_refusal() {
        assert!(refusal_is_deliberate(Some("P0001")), "the door said no");
        // Transient/infrastructure classes: a retry is the correct response.
        assert!(
            !refusal_is_deliberate(Some("40001")),
            "serialization failure"
        );
        assert!(!refusal_is_deliberate(Some("40P01")), "deadlock detected");
        assert!(!refusal_is_deliberate(Some("57014")), "statement timeout");
        assert!(!refusal_is_deliberate(Some("53100")), "disk full");
        assert!(!refusal_is_deliberate(Some("08006")), "connection failure");
        // A raised-with-ERRCODE refusal is NOT P0001 and must stay conservative:
        // freezing delays an event, penning one that was never refused loses the
        // distinction the pen exists to record.
        assert!(!refusal_is_deliberate(Some("23514")), "check violation");
        // No SQLSTATE at all (dropped connection, client-side failure).
        assert!(!refusal_is_deliberate(None));
    }

    /// **PR #493 review.** `refusal_is_deliberate` answers *was this a verdict*. It does
    /// NOT answer *whose fault the failure was*, and both callers of its `false` half need
    /// the second question — one to classify a pull cycle, one to decide whether halting a
    /// recovery sweep is safe.
    ///
    /// The non-`P0001` space is not one thing. A lock timeout, a dropped connection or a
    /// grant a restore has not re-granted is THIS NODE'S infrastructure and clears when the
    /// operator fixes it. A class-22 decode failure or a constraint violation is
    /// DETERMINISTIC and attributable to the bytes — halting on one of those wedges every
    /// row behind it forever.
    #[test]
    fn a_non_deliberate_failure_is_split_by_whose_fault_it_is() {
        for local in [
            "08006", // connection_exception — the connection died
            "40001", // serialization_failure
            "40P01", // deadlock_detected
            "42501", // insufficient_privilege — a grant a pg_restore has not re-granted
            "42P01", // undefined_table — schema skew
            "53100", // disk_full
            "55P03", // lock_not_available — the lock_timeout the tests below force
            "57014", // query_canceled (statement_timeout)
            "58030", // io_error underneath the database
        ] {
            assert!(
                apply_failure_is_local(Some(local)),
                "{local} is this node's own machine, not the peer's bytes"
            );
        }
        // No SQLSTATE at all: the statement never reached a verdict.
        assert!(
            apply_failure_is_local(None),
            "a dropped connection decided nothing about these bytes"
        );
        // Attributable to the BYTES: deterministic, so it will fail identically on every
        // future run and halting the sweep on it strands every row behind it.
        for bytes in [
            "22P02", // invalid_text_representation — a cast on a peer-supplied field
            "22023", // invalid_parameter_value — jsonb_array_length on a scalar (db/050)
            "23514", // check_violation
            "23502", // not_null_violation
            "XX000", // internal_error — a pgrx function panicking on adversarial input
        ] {
            assert!(
                !apply_failure_is_local(Some(bytes)),
                "{bytes} is a failure these bytes earned; halting on it wedges the sweep"
            );
        }
        // A deliberate refusal never reaches this question, but the answer must still not
        // claim the local machine failed.
        assert!(
            !apply_failure_is_local(Some("P0001")),
            "the door deciding is not the machine breaking"
        );
        // Degenerate codes fall to the safe side rather than panicking on a slice.
        assert!(!apply_failure_is_local(Some("")));
        assert!(!apply_failure_is_local(Some("4")));
    }

    /// **PR #493 review.** One cycle can refuse two pen writes for two different reasons,
    /// and a plain first-wins dropped the one that mattered: a QUOTA refusal arriving first
    /// carries `local_fault: false`, so a database failure later in the same batch was
    /// published as `integrity` alone while this node's own disk was what failed.
    ///
    /// The message must still be first-wins — the operator must not read one refusal's text
    /// under another's class — so the fix OR-s the flag and says in the message when a later
    /// event is what raised it. Both halves are pinned here, in both orders.
    #[test]
    fn a_later_database_refusal_is_not_hidden_by_an_earlier_quota_one() {
        let quota = PenRefusal {
            message: "quarantine pen for peer 'peer-a' is at its quota".into(),
            local_fault: false,
        };
        let db = PenRefusal {
            message: "penning a refused event in sync_quarantine: could not serialize \
                      access [40001]"
                .into(),
            local_fault: true,
        };

        // Quota first, database second: the flag must survive, and the text must say which
        // event raised it rather than leaving the two disagreeing.
        let merged = merge_pen_refusal(Some(quota.clone()), db.clone());
        assert!(
            merged.local_fault,
            "the cycle DID meet a local database failure; the earlier quota refusal is \
             not a reason to stop counting it"
        );
        assert!(
            merged
                .message
                .starts_with("quarantine pen for peer 'peer-a'"),
            "first-wins for the message is what keeps text and class about one event: {}",
            merged.message
        );
        assert!(
            merged.message.contains("LATER pen write")
                && merged.message.contains("could not serialize"),
            "…so the later refusal has to be named, or the text and the class disagree: {}",
            merged.message
        );

        // Database first: nothing to upgrade, and the quota refusal must not append noise.
        let merged = merge_pen_refusal(Some(db.clone()), quota.clone());
        assert!(merged.local_fault);
        assert!(
            !merged.message.contains("LATER pen write"),
            "the flag was already true, so there is no upgrade to report: {}",
            merged.message
        );

        // Two quota refusals stay exactly one class — the mirror-image regression, where
        // widening this would charge this node's uptime for a peer flooding the pen.
        let merged = merge_pen_refusal(Some(quota.clone()), quota.clone());
        assert!(!merged.local_fault, "a quota refusal is the PEER's doing");

        // Nothing held yet: the first refusal is taken whole.
        let merged = merge_pen_refusal(None, db.clone());
        assert!(merged.local_fault && merged.message.contains("40001"));
    }

    /// A freeze with an EMPTY pen must not describe penned bytes. The message is
    /// assembled from independent clauses, and the count clause used to render
    /// unconditionally — so a transient-fault freeze announced "0 unverifiable and
    /// 0 floor-refused event(s) this cycle; each is preserved verbatim in
    /// sync_quarantine" and then sent the operator to inspect an empty pen.
    #[test]
    fn a_freeze_only_message_describes_no_penned_bytes() {
        let m = loud_pull_message("peer-a", 0, 0, Some(7), None, "");
        assert!(
            !m.contains("0 unverifiable"),
            "no counts when nothing was penned, got: {m}"
        );
        assert!(
            !m.contains("preserved verbatim"),
            "nothing was preserved, got: {m}"
        );
        assert!(
            !m.contains("cairn-sync quarantine"),
            "the pen is empty — do not send the operator to it, got: {m}"
        );
        assert!(
            m.contains("FROZEN at 7"),
            "it must name the halted slot: {m}"
        );
    }

    /// The pen-quota freeze is NOT transient: it clears only when a human acks or
    /// deletes held rows, which the pen clause itself says. The freeze clause must
    /// not contradict it two sentences later with "clears by itself".
    #[test]
    fn a_pen_refusal_freeze_never_claims_it_clears_by_itself() {
        let m = loud_pull_message("peer-a", 0, 0, Some(7), Some("pen is at its quota"), "");
        assert!(m.contains("pen is at its quota"), "the cause survives: {m}");
        assert!(
            !m.contains("clears by itself"),
            "a pen-refusal hold needs operator action, got: {m}"
        );
        assert!(
            m.contains("does NOT clear by itself"),
            "it must say so plainly, got: {m}"
        );
        assert!(
            m.contains("cairn-sync quarantine"),
            "here the pen IS the thing to inspect, got: {m}"
        );
    }

    /// The penned path keeps everything it always said: both counts, the durability
    /// claim, and the two remedies (repair the peer, or ack the row).
    #[test]
    fn a_penned_cycle_message_names_the_counts_and_both_remedies() {
        let m = loud_pull_message("peer-a", 2, 1, None, None, " DIAGNOSIS.");
        assert!(m.contains("2 unverifiable and 1 floor-refused"), "{m}");
        assert!(m.contains("preserved verbatim"), "{m}");
        assert!(m.contains(" DIAGNOSIS."), "the diagnosis is carried: {m}");
        assert!(m.contains("acked = TRUE"), "the ack remedy survives: {m}");
        assert!(!m.contains("FROZEN"), "nothing froze here: {m}");
    }

    /// Issue #270: a frozen watermark used to exit SUCCESS. Every state in which
    /// this node knowingly does not hold something a peer offered it is loud.
    #[test]
    fn every_unresolved_refusal_state_makes_the_cycle_loud() {
        // (unverifiable, refused, frozen, pen_failed)
        assert!(!cycle_is_loud(0, 0, false, false), "a clean cycle is quiet");
        assert!(
            cycle_is_loud(1, 0, false, false),
            "unverifiable bytes penned"
        );
        assert!(
            cycle_is_loud(0, 1, false, false),
            "a door refusal penned (#267)"
        );
        assert!(
            cycle_is_loud(0, 0, true, false),
            "a frozen watermark (#270)"
        );
        assert!(cycle_is_loud(0, 0, false, true), "the pen itself refused");
        assert!(cycle_is_loud(2, 3, true, true), "several at once");
    }

    /// Issue #465. The line an operator reads when a peer's event carried an attachment
    /// reference this node could not learn must NAME the event, not merely count it.
    ///
    /// This is the Slice 69 rule applied to a new surface: "3 events have bad references"
    /// cannot tell an operator whether to chase one peer's encoder bug or one corrupted
    /// import, and the ledger's own header poses exactly that question. It must also say
    /// what was NOT lost — the events applied, the clinical content is in the record —
    /// because the loud failure this line replaces (#460) meant the opposite.
    #[test]
    fn an_unlearnable_reference_line_names_the_event_and_not_just_a_count() {
        let found = UnlearnableReferences {
            count: 2,
            example_event: "018f0000-0000-7000-8000-00000000abcd".into(),
            example_reason: "digest_hex is not valid hex (6 chars, starts \"ab...\") \
                             — has an odd number of hex digits"
                .into(),
        };
        let m = unlearnable_reference_message("pull peer-a", &found);
        assert!(m.contains("peer-a"), "the peer is named: {m}");
        assert!(
            m.contains("2 attachment reference(s)"),
            "the count is there, and counts REFERENCES: {m}"
        );
        assert!(
            m.contains("018f0000-0000-7000-8000-00000000abcd"),
            "NAME, NEVER COUNT — the example event id must be in the line: {m}"
        );
        assert!(
            m.contains("digest_hex"),
            "the recorded reason travels with it, so the operator can tell an encoder \
             bug from a corrupted import: {m}"
        );
        assert!(
            m.contains("cairn_attachment_flag_health"),
            "the line points at the full ledger: {m}"
        );
    }

    /// The recorded reason carries PEER TEXT, and this line is read line-by-line.
    ///
    /// db/001's `cairn_decode_hex_or_raise` echoes a bounded PREFIX of the peer's own
    /// value into its refusal message, db/050 records that message verbatim, and this
    /// function prints it. A newline in it forges a whole log line: an operator scanning
    /// `journalctl` for pull lines would read a fabricated one as this node's own report.
    /// See `unlearnable_reference_message` for what the prefix is and is not for.
    #[test]
    fn peer_text_in_the_reason_cannot_forge_a_second_line() {
        let found = UnlearnableReferences {
            count: 1,
            example_event: "018f0000-0000-7000-8000-00000000abcd".into(),
            // What a hostile encoder can actually put there: the accessor quotes the first
            // few characters of the value it refused.
            example_reason: "digest_hex is not valid hex (99 chars, starts \"\n\
                             pull peer-b: 0 attachment reference(s)...\")"
                .into(),
        };
        let m = unlearnable_reference_message("pull peer-a", &found);
        assert_eq!(
            m.lines().count(),
            1,
            "peer text must not forge a second line: {m}"
        );
        assert!(
            m.contains("digest_hex is not valid hex"),
            "…while an ordinary reason still reads naturally: {m}"
        );
    }

    /// Three outcomes, and the third is the one that is easy to get wrong: a ledger read
    /// that FAILED must report the metric as `null` (unknown), never as `0` — the
    /// argument is in `unlearnable_report`'s own doc and is not repeated here.
    #[test]
    fn the_unlearnable_report_separates_none_from_could_not_read() {
        // Nothing to report: a plain 0 and no line. The common case, and it must stay silent.
        let (metric, line) = unlearnable_report("pull peer-a", Ok(None));
        assert_eq!(metric, serde_json::json!(0));
        assert!(line.is_none(), "a clean cycle says nothing: {line:?}");

        // Something to report: the count and the line.
        let (metric, line) = unlearnable_report(
            "pull peer-a",
            Ok(Some(UnlearnableReferences {
                count: 1,
                example_event: "018f0000-0000-7000-8000-00000000abcd".into(),
                example_reason: "digest_hex is not hex".into(),
            })),
        );
        assert_eq!(metric, serde_json::json!(1));
        assert!(
            line.as_deref().is_some_and(|l| l.contains("018f0000")),
            "the operator line carries the example: {line:?}"
        );

        // Could not look: UNKNOWN, and said out loud.
        let (metric, line) = unlearnable_report("pull peer-a", Err("connection reset".into()));
        assert!(
            metric.is_null(),
            "a failed read is UNKNOWN, not zero — got {metric}"
        );
        let line = line.expect("a failed read must not be silent");
        assert!(
            line.contains("connection reset"),
            "the cause travels: {line}"
        );
        assert!(
            line.contains("unknown") || line.contains("UNKNOWN"),
            "the line must say the metric is unknown rather than zero: {line}"
        );
    }

    /// **#466 review.** The metric and the operator line come out of ONE function, and
    /// the line is actually WRITTEN — which nothing pinned before.
    ///
    /// The emission used to be an `eprintln!` at each of the two call sites, so deleting
    /// both left every test green: the surface whose entire purpose is "do not produce a
    /// log line byte-identical to a healthy pull" had its log line untested. Putting the
    /// write inside the only producer of the metric closes that — a caller cannot now keep
    /// the count and drop the line, and this test watches the write itself through a sink.
    #[test]
    fn the_report_writes_its_line_and_stays_silent_when_there_is_nothing_to_say() {
        // Nothing to say: a plain 0, and NOT ONE BYTE written. A signal that speaks on a
        // healthy cycle is noise, and noise is what an operator learns to ignore.
        let mut out: Vec<u8> = Vec::new();
        let metric = emit_unlearnable_report(&mut out, "pull peer-a", Ok(None));
        assert_eq!(metric, serde_json::json!(0));
        assert!(out.is_empty(), "a clean cycle writes nothing: {out:?}");

        // Something to say: the count AND the line, on one line, naming the event.
        let mut out: Vec<u8> = Vec::new();
        let metric = emit_unlearnable_report(
            &mut out,
            "pull peer-a",
            Ok(Some(UnlearnableReferences {
                count: 2,
                example_event: "018f0000-0000-7000-8000-00000000abcd".into(),
                example_reason: "digest_hex is not hex".into(),
            })),
        );
        assert_eq!(metric, serde_json::json!(2));
        let written = String::from_utf8(out).expect("the line is UTF-8");
        assert!(
            written.contains("018f0000-0000-7000-8000-00000000abcd"),
            "the written line names the event: {written}"
        );
        assert_eq!(
            written.lines().count(),
            1,
            "exactly one line per report: {written}"
        );
        assert!(
            written.ends_with('\n'),
            "…and it is terminated: {written:?}"
        );

        // Could not look: the failure is written too, never swallowed.
        let mut out: Vec<u8> = Vec::new();
        let metric = emit_unlearnable_report(&mut out, "requeue", Err("42P01: gone".into()));
        assert!(metric.is_null(), "a failed read is UNKNOWN: {metric}");
        let written = String::from_utf8(out).expect("the line is UTF-8");
        assert!(written.contains("42P01"), "the cause is written: {written}");
    }

    /// **#466 review.** `custody_withheld` is UNBOUNDED PEER PROSE, and the line it can
    /// forge is the one issue #465 just installed.
    ///
    /// The reason is an `Option<String>` deserialized straight off the wire from a peer
    /// the trust set has NOT admitted — no length bound, no validation. Printed raw, a
    /// newline in it lets that peer write a whole fabricated line into this node's log,
    /// and the most useful one to fabricate is a `0 attachment reference(s)` all-clear:
    /// an operator grepping for the new alarm reads the forgery and stands down. The
    /// bounded 8-character ledger prefix this PR escaped is the SMALLER of the two
    /// channels; this is the larger one, on the very line cited as its precedent.
    #[test]
    fn custody_withheld_peer_prose_cannot_forge_a_second_line() {
        let hostile = "key policy\npull peer-a: 0 attachment reference(s) in this run \
                       could not be learned.";
        let m = custody_withheld_message("peer-a", hostile);
        assert_eq!(
            m.lines().count(),
            1,
            "unbounded peer prose must not forge a second line: {m}"
        );
        assert!(
            !m.contains("\npull"),
            "specifically, it must not forge a pull line: {m}"
        );
        // …while an ordinary reason still reads naturally, which is the whole point of
        // `{:?}` over a strip-everything sanitiser.
        let m = custody_withheld_message("peer-a", "the unwrap certificate has expired");
        assert!(
            m.contains("the unwrap certificate has expired"),
            "an honest reason survives intact: {m}"
        );
        assert!(m.contains("peer-a"), "and the peer is named: {m}");
    }

    /// The door's legible text (RAISE message + DETAIL, issue #109) and its
    /// SQLSTATE must BOTH survive the trip out of `apply_signed`: the message is
    /// what a human reads in the pen row, the code is what the routing reads.
    #[test]
    fn apply_error_keeps_both_the_legible_message_and_the_sqlstate() {
        let Some(base) = quarantine_tests::cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = quarantine_tests::locked_client(&base);
        // A bare RAISE with a DETAIL, exactly the shape db/020's refusals take.
        let err: ApplyError = c
            .batch_execute(
                "DO $$ BEGIN RAISE EXCEPTION 'apply_remote_event: signer is not enrolled'
                            USING DETAIL = 'cairn_verify_error: unknown key'; END $$;",
            )
            .unwrap_err()
            .into();
        assert!(
            err.message.contains("signer is not enrolled")
                && err.message.contains("cairn_verify_error"),
            "message must carry RAISE text AND DETAIL, got: {}",
            err.message
        );
        assert_eq!(err.sqlstate.as_deref(), Some("P0001"));
        assert!(err.is_deliberate_refusal());
    }

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
            limit: None,
        };
        let bytes = serde_json::to_vec(&req).unwrap();
        match serde_json::from_slice::<Request>(&bytes).unwrap() {
            Request::EventsAfterSeq {
                after_seq,
                unwrap_cert,
                limit,
            } => {
                assert_eq!(after_seq, 42);
                assert!(unwrap_cert.is_none());
                assert!(limit.is_none());
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    // ---- issue #202: wire-frame cap + fingerprint collation + byte-tier legibility ----

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

    // ─── #479: the "db error" species in cairn-sync's OWN run loop ────────────────────
    //
    // #469 fixed the CLASS of a local database failure and never its MESSAGE, and its
    // test asserted only the class — so `cycle 118: PULL FAILED: db error` reached both
    // the terminal and the JSONL `bet_a.py` reads, every cycle for the life of the
    // process. These pin the message.

    /// A real `postgres::Error` with a live `source()`, built with NO database and no
    /// network: `Config`'s own parser is the only way to get one by hand, because
    /// `DbError` cannot be constructed outside `tokio-postgres`. The same fixture trick
    /// `cairn-node`'s `db_diagnosis` uses.
    ///
    /// It is a `Kind::ConfigParse`, not a `Kind::Db` — worth knowing, because those two
    /// arms of [`legible_db_error`] behave differently and only a live server can produce
    /// the second. `clinical_pull.rs` covers the server arm end to end.
    fn a_real_pg_error() -> postgres::Error {
        "host=localhost port=not-a-number"
            .parse::<postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string")
    }

    /// **Issue #480.** `ApplyError` is NOT "legible by construction", and it was
    /// documented as though it were.
    ///
    /// `ApplyError::from`'s `None` arm is taken when there is no `DbError` at all — the
    /// connection died rather than the door deciding — and it fell back to
    /// `postgres::Error`'s own `Display`, which is the two words `db error`. The type's
    /// other job, telling a deliberate floor refusal from a transient fault, is unaffected
    /// and pinned here beside it: no SQLSTATE can never be a verdict.
    #[test]
    fn an_apply_failure_with_no_door_verdict_is_legible_and_is_not_a_refusal() {
        let ae = ApplyError::from(a_real_pg_error());
        assert!(
            !ae.is_deliberate_refusal(),
            "no SQLSTATE means the door never decided, so it is never a verdict"
        );
        let text = ae.to_string();
        // As in the sibling crate's guard: this fixture is a `Kind::ConfigParse` error, so
        // it could not render `db error` whatever the code did. The assertion below it is
        // the one that does the work.
        assert_ne!(text, "db error", "{text}");
        assert!(
            text.contains("port"),
            "the cause must survive rather than being flattened to a kind: {text}"
        );
    }

    /// **PR #493 review, self-review.** `cmd_pull` is reachable as a one-shot, and
    /// `fn main() -> R<()>` has no error printer — `Termination` prints `{err:?}`. A
    /// DERIVED `Debug` on this type would hand an operator struct syntax with the boxed
    /// transport error re-dumped beside the sentence, which is the defect
    /// `RequeueInterruptedError` and `LocalDbFault` each had to fix before it.
    #[test]
    fn a_failed_peer_request_reads_as_a_sentence_under_termination() {
        let e = PeerRequestError {
            message: "pull peer-a: no usable response to EventsAfterSeq. The transport \
                      reported: failed to fill whole buffer"
                .into(),
            source: Box::new(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "failed to fill whole buffer",
            )),
        };
        let debug = format!("{e:?}");
        assert!(
            debug.starts_with("pull peer-a: no usable response"),
            "`Termination` prints Debug; struct syntax is not a sentence: {debug}"
        );
        assert!(
            !debug.contains("PeerRequestError {") && !debug.contains("source:"),
            "the boxed cause must not be re-dumped beside the sentence: {debug}"
        );
    }

    /// A purpose-built chain link, so a multi-layer chain can be written down exactly.
    #[derive(Debug)]
    struct Layer(&'static str, Option<Box<dyn Error + 'static>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.0)
        }
    }

    impl Error for Layer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            self.1.as_deref()
        }
    }

    #[test]
    fn operator_chain_names_every_layer_in_order() {
        let e = Layer(
            "pull cycle 7",
            Some(Box::new(Layer("reading the sync cursor", None))),
        );
        assert_eq!(operator_chain(&e), "pull cycle 7: reading the sync cursor");
    }

    #[test]
    fn operator_chain_drops_a_layer_the_layer_above_already_ends_with() {
        // The `LocalDbFault` shape, written down structurally: a wrapper whose own
        // `Display` already embeds its cause. Printing the cause again is the `{e:#}`
        // defect, and for a server error the trailing copy is the literal `db error`.
        // THREE layers, deliberately: with only two, "dropped the duplicate" and "never
        // walked the chain at all" produce the same string, and the test cannot tell them
        // apart. The outermost layer here is NOT the answer, so a walk has to happen.
        let e = Layer(
            "pull cycle 7",
            Some(Box::new(Layer(
                "reading the sync cursor: permission denied for table sync_state [42501]",
                Some(Box::new(Layer(
                    "permission denied for table sync_state [42501]",
                    None,
                ))),
            ))),
        );
        assert_eq!(
            operator_chain(&e),
            "pull cycle 7: reading the sync cursor: permission denied for table \
             sync_state [42501]",
            "a layer already ending in its cause must not repeat it"
        );
    }

    #[test]
    fn operator_chain_renders_a_postgres_error_rather_than_its_kind() {
        let e = Layer("reading the sync cursor", Some(Box::new(a_real_pg_error())));
        let line = operator_chain(&e);

        assert!(line.starts_with("reading the sync cursor: "), "{line}");
        assert!(
            line.contains("port"),
            "the diagnosis must survive, not just the layer: {line}"
        );
        assert!(
            !line.ends_with("db error") && !line.contains(": db error"),
            "#479's whole subject: {line}"
        );
    }

    #[test]
    fn operator_chain_renders_a_bare_postgres_error() {
        // No wrapper at all — the shape `do_pull`'s first two statements produce today.
        let e = a_real_pg_error();
        let line = operator_chain(&e);
        assert!(!line.is_empty(), "a chain of one must still say something");
        assert_ne!(line, "db error", "{line}");
        assert!(line.contains("port"), "{line}");
    }

    #[test]
    fn operator_chain_never_forges_a_log_line() {
        // A server DETAIL arrives with raw newlines, and this log is line-oriented: a
        // multi-line cause must be collapsed, never dropped. Same rule
        // `compose_db_diagnosis` already obeys — this is the other door into that log.
        let e = Layer(
            "first line\nFATAL: forged second line",
            Some(Box::new(Layer("cause\nsecond forged line", None))),
        );
        let line = operator_chain(&e);
        assert!(!line.contains('\n'), "one line per event: {line}");
        assert!(
            line.contains("forged second line"),
            "collapsed, never dropped: {line}"
        );
        assert!(line.contains("second forged line"), "both layers: {line}");
    }

    #[test]
    fn a_local_db_fault_names_the_operation_and_keeps_its_cause() {
        let fault = LocalDbFault::new("reading the sync cursor", a_real_pg_error());

        let shown = fault.to_string();
        assert!(shown.starts_with("reading the sync cursor: "), "{shown}");
        assert!(shown.contains("port"), "the diagnosis rides along: {shown}");
        assert_ne!(shown, "db error", "{shown}");

        // `Debug` delegates to `Display`, because `main`'s `Termination` prints `{err:?}`.
        assert_eq!(format!("{fault:?}"), shown);

        // The cause stays REACHABLE. Dropping it is what silently reverts every local
        // fault to `partition`.
        assert!(
            Error::source(&fault).is_some_and(|c| c.is::<postgres::Error>()),
            "the postgres error must still be findable through source()"
        );
    }

    #[test]
    fn a_wrapped_local_db_fault_is_still_a_local_fault_not_a_partition() {
        // THE TRAP, pinned. `downcast_ref` on a `dyn Error` does NOT walk `source()`, so
        // naming the operation at a failing statement — which is the whole point of
        // `LocalDbFault` — would otherwise push the error out of the classifier's reach
        // and log this node's dead database as link downtime, charging the Bet A
        // availability figure for it. #469's defect, reinstated by its own fix.
        let e: Box<dyn Error> = LocalDbFault::boxed("reading the sync cursor", a_real_pg_error());
        let (classes, metrics) = classify_pull_failure(e.as_ref());

        assert_eq!(
            classes,
            &["local_fault"],
            "this node's database is gone; the link was never touched: {e}"
        );
        assert!(
            metrics.is_null(),
            "it failed before any work was measurable: {metrics}"
        );
    }

    #[test]
    fn a_failed_pull_records_a_legible_reason_not_db_error() {
        // The JSONL surface `bet_a.py` reads. `record_pull_failure` is the ONE seam both
        // the machine-readable line and the operator's terminal line pass through, so
        // pinning it here pins both.
        let mut line = serde_json::json!({ "cycle": 118 });
        let e: Box<dyn Error> = LocalDbFault::boxed("reading the sync cursor", a_real_pg_error());
        record_pull_failure(&mut line, e.as_ref());

        let reported = line["pull_error"].as_str().expect("a reason is recorded");
        assert_ne!(reported, "db error", "#479's title sentence");
        assert!(
            reported.starts_with("reading the sync cursor: "),
            "the operator learns WHAT FAILED, not only that something did: {reported}"
        );
        assert!(reported.contains("port"), "…and why: {reported}");
        assert_eq!(line["local_fault"], serde_json::json!(true));

        // A BARE `postgres::Error` — the case that can tell `operator_chain` from
        // `to_string()`. The `LocalDbFault` above cannot: its own `Display` already embeds the
        // rendered cause, so both renderings produce the same string, and reverting this key
        // to `{e}` left the entire suite green (measured, PR #486 review). A bare error
        // renders as its KIND alone — the layer that failed, never the reason — and only
        // `operator_chain` reaches the cause beneath it.
        let mut bare = serde_json::json!({ "cycle": 119 });
        let raw: Box<dyn Error> = Box::new(a_real_pg_error());
        record_pull_failure(&mut bare, raw.as_ref());

        let reported = bare["pull_error"].as_str().expect("a reason is recorded");
        assert_ne!(
            reported,
            raw.to_string(),
            "the kind alone is what `{{e}}` gives; this key must carry what is UNDER it"
        );
        assert!(
            reported.contains("port"),
            "…which for this fixture is the cause naming the bad port: {reported}"
        );
        assert_eq!(bare["local_fault"], serde_json::json!(true));
    }

    #[test]
    fn a_failed_fingerprint_probe_is_null_and_says_why_in_the_jsonl() {
        // The half `bet_a.py` actually reads. The stderr line is covered by
        // `a_missing_fingerprint_says_why`; this pins that the ARTIFACT is not silent, which
        // is what a run under nohup leaves behind.
        let mut line = serde_json::json!({ "cycle": 3 });
        let e: Box<dyn Error> = LocalDbFault::boxed("reading the event count", a_real_pg_error());
        record_fingerprint_failure(&mut line, e.as_ref());

        // Present-and-null, never absent — the #479 defect was the key VANISHING, and
        // `"fingerprint" in r` is exactly how bet_a.py asks.
        assert!(
            line.get("fingerprint").is_some(),
            "the key must be present so a reader can tell 'we looked and failed' from \
             'nobody wrote this': {line}"
        );
        assert!(
            line["fingerprint"].is_null(),
            "…and null, never a number: {line}"
        );

        let why = line["fingerprint_error"]
            .as_str()
            .expect("the reason travels in the artifact, not only on stderr");
        assert!(
            why.starts_with("reading the event count: "),
            "the operation is named: {why}"
        );
        assert!(why.contains("port"), "…and the cause beneath it: {why}");
        assert_ne!(why, "db error", "#479's title sentence");
    }

    #[test]
    fn blobd_error_line_carries_a_database_diagnosis() {
        // The line's own doc says it "must carry the underlying cause". Over a
        // `postgres::Error` it carried the two words `db error` — #202's fix meeting
        // #479's species.
        let e: Box<dyn Error> = Box::new(a_real_pg_error());
        let line = blobd_error_line(e.as_ref());
        assert!(line.contains("blobd"), "{line}");
        assert!(line.contains("retr"), "{line}");
        assert!(
            line.contains("port"),
            "the cause must be the DIAGNOSIS, not the kind: {line}"
        );
    }

    #[test]
    fn a_missing_fingerprint_says_why() {
        // `if let Ok(fp) = do_fingerprint(…)` had NO else arm: a schema skew made the
        // `fingerprint` key vanish from every later JSONL line and the status string
        // quietly stop showing event counts, with zero evidence why.
        //
        // The fixture's operation phrase deliberately does NOT contain the word
        // "fingerprint": the line must name the missing thing ITSELF, or it passes on any
        // error that happens to mention one and says nothing when the error does not.
        let e: Box<dyn Error> = LocalDbFault::boxed("reading the event count", a_real_pg_error());
        let line = fingerprint_error_line(e.as_ref());
        assert!(
            line.contains("fingerprint"),
            "names the missing thing: {line}"
        );
        assert!(
            line.contains("counts"),
            "…and its CONSEQUENCE — every later line silently loses the counts, which is \
             the whole reason the silence mattered: {line}"
        );
        assert!(line.contains("port"), "carries the diagnosis: {line}");
        assert!(
            !line.contains('\n'),
            "one line per event, like every other line here: {line}"
        );
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
            custody_withheld: None,
            complete: false,
        };
        let back: EventsResponse =
            serde_json::from_slice(&serde_json::to_vec(&with).unwrap()).unwrap();
        assert_eq!(back.seqs, vec![7]);
    }

    #[test]
    fn events_response_custody_withheld_field_is_additive() {
        // An older serve omits the field entirely → None, which the puller reads as
        // "nothing was deliberately withheld" and prints nothing (issue #231 review).
        let old = r#"{"events":[],"attestations":[],"attester_keys":[],"seqs":[]}"#;
        let r: EventsResponse = serde_json::from_str(old).unwrap();
        assert!(
            r.custody_withheld.is_none(),
            "a response predating the field must decode as 'no refusal reported', never \
             as an empty refusal"
        );
        // …and a reason round-trips verbatim: it is operator prose, so anything that
        // mangled it would hand the reader a remedy they cannot follow.
        let reason = "custody WITHHELD from puller abc — this key is not among …";
        let with = EventsResponse {
            events: vec![],
            attestations: vec![],
            attester_keys: vec![],
            seqs: vec![],
            signing_context: None,
            wrapped_deks: vec![],
            custody_withheld: Some(reason.into()),
            complete: false,
        };
        let back: EventsResponse =
            serde_json::from_slice(&serde_json::to_vec(&with).unwrap()).unwrap();
        assert_eq!(back.custody_withheld.as_deref(), Some(reason));
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
    // decide_custody — the #231 trust-set pin. Pure: it turns ONE trust-set
    // lookup outcome into a grant/withhold decision plus the operator line that
    // explains it. The DB round trip lives in `look_up_peer_trust`; everything
    // that decides anything is here, where it needs no database to test.
    // ---------------------------------------------------------------------

    /// A kid-shaped fixture (hex Ed25519 public key), derived rather than written
    /// as a literal — house rule 6 keeps CodeQL's hard-coded-crypto query live for
    /// production code by never presenting it a literal in a crypto context.
    fn fixture_kid(tag: u8) -> String {
        hex::encode(derived_bytes(tag))
    }

    /// EVERY non-grant arm, as one list with its expected verdict, driven by an
    /// exhaustive `match` so a new `TrustLookup` variant is a COMPILE ERROR here.
    ///
    /// The first version of these tests hand-wrote the variant list in three places
    /// and claimed that "a NEW arm has to come here and state its intent". It did not:
    /// array literals inherit nothing, so an eighth variant — even one mapping to
    /// `Grant` — would have left all six tests green. This closes that.
    fn every_withhold_arm() -> Vec<TrustLookup> {
        [
            TrustLookup::ActivePeer,
            TrustLookup::RevokedPeer,
            TrustLookup::NotAPeer,
            TrustLookup::NoPeersAdmitted,
            TrustLookup::NodePlaneUninitialised,
            TrustLookup::NodePlaneAbsent,
            TrustLookup::LookupFailed,
        ]
        .into_iter()
        .filter(|lookup| match lookup {
            // The ONE arm that grants. Every other variant must appear below, and a
            // new one cannot be added without deciding which side it falls on.
            TrustLookup::ActivePeer => false,
            TrustLookup::RevokedPeer
            | TrustLookup::NotAPeer
            | TrustLookup::NoPeersAdmitted
            | TrustLookup::NodePlaneUninitialised
            | TrustLookup::NodePlaneAbsent
            | TrustLookup::LookupFailed => true,
        })
        .collect()
    }

    #[test]
    fn an_active_peer_is_granted_custody_for_the_key_in_its_own_cert() {
        // The ONLY grant arm: the presented kid is in this node's trust set and
        // its latest op is not `revoke` (#231). The grant carries the key it admits,
        // so the caller cannot re-wrap for a DIFFERENT key than the one decided on.
        let requester_pub = derived_bytes(0x11);
        assert_eq!(
            decide_custody(&fixture_kid(0x01), requester_pub, TrustLookup::ActivePeer),
            CustodyAdmission::Grant { requester_pub }
        );
    }

    #[test]
    fn every_other_lookup_outcome_withholds_custody() {
        // Fail-closed is the whole point: custody confers clinical-data READ, so
        // anything short of a positive `active` match must withhold.
        let arms = every_withhold_arm();
        assert!(!arms.is_empty(), "the arm list must not be empty");
        for lookup in arms {
            assert!(
                matches!(
                    decide_custody(&fixture_kid(0x02), derived_bytes(0x12), lookup),
                    CustodyAdmission::Withhold { .. }
                ),
                "{lookup:?} must withhold custody"
            );
        }
    }

    #[test]
    fn each_withhold_names_the_kid_its_cause_and_a_distinct_remedy() {
        // The Slice 61 lesson made mechanical: a safety refusal is only as good as
        // the escape hatch it names — check the reader can act on what was printed.
        // Every withhold line must (a) carry the full kid, so the operator can paste
        // it into the fix, and (b) name a DISTINCT remedy, because each arm is a
        // different operator problem and one shared line would hide the rest.
        let kid = fixture_kid(0x03);
        let mut lines = Vec::new();
        for lookup in every_withhold_arm() {
            let CustodyAdmission::Withhold {
                operator_line,
                cause,
            } = decide_custody(&kid, derived_bytes(0x13), lookup)
            else {
                panic!("{lookup:?} must withhold");
            };
            // The withhold carries the cause it was decided from — so a caller (and
            // this test) can act on the DECISION rather than parse its prose.
            assert_eq!(cause, lookup, "the withhold must carry its own cause");
            assert!(
                operator_line.contains(&kid),
                "{lookup:?} line must name the full kid so it can be pasted into the fix: \
                 {operator_line}"
            );
            assert!(
                operator_line.contains("custody"),
                "{lookup:?} line must say what was withheld: {operator_line}"
            );
            lines.push(operator_line);
        }
        let distinct: std::collections::HashSet<&String> = lines.iter().collect();
        assert_eq!(
            distinct.len(),
            lines.len(),
            "each withhold cause needs its OWN line — a shared message hides the others: {lines:#?}"
        );
    }

    #[test]
    fn a_recoverable_withhold_names_both_repair_steps_and_the_others_name_neither() {
        // The remedy is TWO steps, and naming only the first is what a review measured
        // as a lie: `pull --full` alone took custody from (0,0) to (1,1) and left the
        // medication projection at ZERO, because the re-apply inserts no event_log row
        // and the projection dispatcher is an AFTER INSERT trigger. An operator who
        // followed the printed line still saw an empty chart.
        //
        // Both directions are asserted, and the classifier is the TYPED property
        // (`puller_can_recover`), never a substring of the message: the previous
        // version grepped its own prose for "cairn-node pair", which silently checked
        // LESS whenever the wording moved — and already missed one arm.
        let kid = fixture_kid(0x07);
        let (mut recoverable, mut terminal) = (0, 0);
        for lookup in every_withhold_arm() {
            let CustodyAdmission::Withhold { operator_line, .. } =
                decide_custody(&kid, derived_bytes(0x17), lookup)
            else {
                panic!("{lookup:?} must withhold");
            };
            if lookup.puller_can_recover() {
                recoverable += 1;
                assert!(
                    operator_line.contains("pull --full"),
                    "{lookup:?}: a recoverable withhold must name the FULL sweep — an \
                     incremental pull cannot reach events already below the cursor: \
                     {operator_line}"
                );
                assert!(
                    operator_line.contains("cairn_reproject"),
                    "{lookup:?}: the sweep restores CUSTODY, not the chart. A line that \
                     stops at `pull --full` sends the operator away believing the \
                     record is lost: {operator_line}"
                );
            } else {
                terminal += 1;
                assert!(
                    !operator_line.contains("pull --full")
                        && !operator_line.contains("cairn_reproject"),
                    "{lookup:?}: the puller cannot fix this, so a pull/reproject \
                     instruction here would run and change nothing: {operator_line}"
                );
            }
        }
        // Non-vacuity in BOTH directions: a classifier that quietly stopped matching
        // anything would otherwise turn this whole test into a no-op that passes.
        assert!(recoverable > 0, "no arm was classified recoverable");
        assert!(terminal > 0, "no arm was classified terminal");
    }

    #[test]
    fn the_remedy_names_only_commands_that_exist() {
        // A review found every withhold line naming `cairn-node pair` — which is not a
        // subcommand (`pair-offer` / `pair-accept` are), so the printed remedy exited
        // with a usage error. The test that was supposed to catch it asserted the
        // substring "cairn-node pair", which is a PREFIX of `cairn-node pair-offer` and
        // therefore could never have failed.
        //
        // Pinned against the real subcommand names. `cairn-node`'s clap definition is
        // in another crate (and another binary), so this cannot introspect it; the list
        // is the thing to update if a subcommand is ever renamed.
        const REAL_SUBCOMMANDS: [&str; 4] = [
            "init",
            "pair-offer",
            "pair-accept",
            "provision-runtime-role",
        ];
        let kid = fixture_kid(0x08);
        for lookup in every_withhold_arm() {
            let CustodyAdmission::Withhold { operator_line, .. } =
                decide_custody(&kid, derived_bytes(0x18), lookup)
            else {
                panic!("{lookup:?} must withhold");
            };
            // Every `cairn-node <word>` the line mentions must be a real subcommand.
            for (i, _) in operator_line.match_indices("cairn-node ") {
                let rest = &operator_line[i + "cairn-node ".len()..];
                let word: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                // A bare `cairn-node` used as a noun ("provision it with `cairn-node`")
                // names no subcommand and needs no check.
                if word.is_empty() {
                    continue;
                }
                assert!(
                    REAL_SUBCOMMANDS.contains(&word.as_str()),
                    "{lookup:?} names `cairn-node {word}`, which is not a subcommand — a \
                     remedy that exits with a usage error is worse than none: {operator_line}"
                );
            }
        }
    }

    #[test]
    fn the_two_unprovisioned_arms_name_the_node_plane_not_the_peer() {
        // These two are NOT security events and must not read like one. A node plane
        // that was never initialised, and a database that never loaded db/007, are
        // operator provisioning gaps; blaming the puller would send the reader
        // hunting the wrong problem.
        for lookup in [
            TrustLookup::NodePlaneUninitialised,
            TrustLookup::NodePlaneAbsent,
        ] {
            let CustodyAdmission::Withhold { operator_line, .. } =
                decide_custody(&fixture_kid(0x04), derived_bytes(0x14), lookup)
            else {
                panic!("{lookup:?} must withhold");
            };
            assert!(
                operator_line.contains("node plane"),
                "{lookup:?} must name the node plane as the gap: {operator_line}"
            );
        }
    }

    #[test]
    fn an_unpeered_node_is_not_told_to_re_initialise_itself() {
        // The defect this test pins, found by reading the serve log of the wire test
        // rather than by any assertion: `trust_peer` is EMPTY both when the node
        // plane was never initialised AND when it is provisioned but has peered
        // nobody, because the view filters on a `local_node` subquery that is NULL in
        // the first case. Reporting the second as the first tells an operator to run
        // `init` on an already-initialised node — a remedy that cannot work, which is
        // worse than no remedy at all (the Slice 61 lesson).
        let kid = fixture_kid(0x06);
        let CustodyAdmission::Withhold {
            operator_line: no_peers,
            ..
        } = decide_custody(&kid, derived_bytes(0x16), TrustLookup::NoPeersAdmitted)
        else {
            panic!("NoPeersAdmitted must withhold");
        };
        assert!(
            !no_peers.contains("cairn-node init"),
            "a provisioned node with no peers must NOT be told to re-initialise: {no_peers}"
        );
        assert!(
            no_peers.contains("pair"),
            "its remedy is pairing, and the line must say so: {no_peers}"
        );

        let CustodyAdmission::Withhold {
            operator_line: uninit,
            ..
        } = decide_custody(
            &kid,
            derived_bytes(0x16),
            TrustLookup::NodePlaneUninitialised,
        )
        else {
            panic!("NodePlaneUninitialised must withhold");
        };
        assert!(
            uninit.contains("cairn-node init"),
            "an uninitialised node plane must name `init` as the FIRST step: {uninit}"
        );
    }

    #[test]
    fn a_revoked_peer_reads_as_revoked_not_as_unknown() {
        // Revocation is an ACT someone performed (ADR-0018's cascade); reporting it
        // as "not a peer" would erase that and send the operator to re-pair a node
        // they deliberately cut off. Principle 2 in miniature: never erase, overlay.
        //
        // This test pins the MESSAGE only. That it is ever REACHED from a real
        // database is a separate fact, and one this test cannot see: as first shipped
        // the arm was unreachable, so a revoked peer got the "not among this node's
        // admitted peers" line while this test passed. `look_up_peer_trust`'s
        // DB-gated tests and the `a_revoked_peer_is_told_it_was_revoked…` wire test
        // are what hold the other half.
        let kid = fixture_kid(0x05);
        let CustodyAdmission::Withhold { operator_line, .. } =
            decide_custody(&kid, derived_bytes(0x15), TrustLookup::RevokedPeer)
        else {
            panic!("a revoked peer must withhold");
        };
        // Case-insensitive on purpose: the line emphasises REVOKED in caps, and a test
        // that pins the casing would fail on a purely cosmetic edit while still not
        // proving the word is there.
        assert!(
            operator_line.to_lowercase().contains("revoked"),
            "a revoked peer must be named as revoked: {operator_line}"
        );
    }

    // ---------------------------------------------------------------------
    // rewrap_custody_for_peer — the load-bearing custody re-wrap on the serve
    // path (ADR-0052). Pure (no DB), so unit-testable directly. ALL key material
    // is DERIVED at runtime (house rule 6): a literal 32-byte array in a crypto
    // context trips CodeQL's hard-coded-cryptographic-value query (issue #146).
    // ---------------------------------------------------------------------

    /// Deterministic-but-computed 32-byte fixture. `tag` distinguishes each
    /// role/secret so no two fixtures collide, and nothing is a byte literal.
    /// pub(super): shared with the sibling `trust_lookup_db_tests` module, which needs
    /// the same house-rule-6 derivation for its node-id and key fixtures.
    pub(super) fn derived_bytes(tag: u8) -> [u8; 32] {
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

    /// A key file that exists but cannot be parsed must STOP the daemon, never be overwritten.
    ///
    /// `read_to_string` fails on invalid UTF-8, and cairn-node's sealed key file is binary CBOR —
    /// so before this guard, pointing cairn-sync at a real node's key file generated a fresh key
    /// and wrote it over the sealed one. The signing key is never backed up (ADR-0026 decision 4),
    /// so that is unrecoverable identity loss caused by a daemon start.
    #[test]
    fn an_unparseable_key_file_is_refused_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("node.key");

        // Bytes shaped like cairn-node's sealed bundle: binary, invalid UTF-8. Derived at
        // runtime (house rule 6) — this is a key file, so no literal key material.
        let sealed_like: Vec<u8> = (0u8..64)
            .map(|i| i.wrapping_mul(3).wrapping_add(0x80))
            .collect();
        std::fs::write(&p, &sealed_like).unwrap();

        let err = load_or_create_key(p.to_str().unwrap())
            .expect_err("an unparseable key file must be refused, never overwritten");
        assert!(
            format!("{err}").contains("refusing to overwrite"),
            "the refusal must say what it is protecting; got: {err}"
        );
        assert_eq!(
            std::fs::read(&p).unwrap(),
            sealed_like,
            "THE POINT: the existing key file must be byte-identical after the refusal"
        );
    }

    /// The create-on-absent path is the intended behaviour and must survive the fix.
    #[test]
    fn an_absent_key_file_is_still_created() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("fresh.key");
        let (_sk, kid) = load_or_create_key(p.to_str().unwrap()).unwrap();
        assert!(!kid.is_empty());
        assert!(p.exists(), "an absent key file is still created");
        // And a second call LOADS it rather than minting a new identity.
        let (_sk2, kid2) = load_or_create_key(p.to_str().unwrap()).unwrap();
        assert_eq!(
            kid, kid2,
            "a second start must reuse the key, not replace it"
        );
    }
}

/// Issue #475: `cairn-sync init` must name the migration that failed.
///
/// A tiny DB-gated module of its own rather than a lodger in `quarantine_tests`, because
/// it shares none of that suite's fixture: it takes no advisory lock, loads no schema and
/// truncates nothing. The whole fixture is a bare connection and a statement the server
/// refuses, so it can run beside anything.
#[cfg(test)]
mod schema_load_diagnosis_tests {
    use super::*;

    fn cs() -> Option<String> {
        std::env::var("CAIRN_TEST_PG").ok()
    }

    /// The acceptance criterion of #467, unmet in the operator's FIRST command until now:
    /// *a schema-load failure names the migration, the SQLSTATE and the server's message +
    /// DETAIL*.
    ///
    /// Drives the extracted door with a body guaranteed to fail, exactly as `cairn-node`'s
    /// `tests/db_diagnosis.rs` does for its half — so what is under test is the composition
    /// an operator meets, not a restatement of it. The SQL is synthetic on purpose: forcing
    /// a real migration to fail would mean planting a decoy object in a shared database,
    /// which is both destructive and coupled to whichever migration tripped over it.
    #[test]
    fn a_failed_migration_names_the_migration_and_the_cause() {
        let Some(conn) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = postgres::Client::connect(&conn, postgres::NoTls).unwrap();

        let err = load_migration(
            &mut c,
            "031_medication",
            r#"DO $$ BEGIN RAISE EXCEPTION 'relation "decoy" does not exist'
                         USING ERRCODE = '42P01', DETAIL = 'this migration never loaded'; END $$;"#,
        )
        .expect_err("a bare RAISE always fails")
        .to_string();

        assert_ne!(err, "db error", "the whole of #467/#475: {err}");
        assert!(
            err.contains("031_medication"),
            "an operator must not have to COUNT `applied \u{2026}` lines to find it: {err}"
        );
        assert!(
            err.contains("[42P01]"),
            "\u{2026}and the SQLSTATE is the part they can search for: {err}"
        );
        assert!(
            err.contains("this migration never loaded"),
            "\u{2026}and the DETAIL, where this project's own doors put the reason (#109): {err}"
        );
    }

    /// A failed connect names the reason and never the connection string. `postgres::Error`
    /// renders a refused socket as `error connecting to server` — a category, not a
    /// diagnosis — and the errno lives in `source()`. Needs no database: nothing listens.
    #[test]
    fn a_failed_connect_names_the_errno_and_never_the_password() {
        // DERIVED, never a literal — house rule 6. A password-shaped literal beside a
        // `password=` key is the shape CodeQL flags (issue #146), and this string exists
        // only to be asserted ABSENT, so it must still look like a real secret.
        let secret: String = (0..16u8)
            .map(|i| char::from(b'a' + (i.wrapping_mul(7) % 26)))
            .collect();
        // `expect_err` would need `postgres::Client: Debug`; a match says the same thing.
        let err = match connect_db(&format!(
            "host=127.0.0.1 port=1 dbname=cairn user=n password={secret}"
        )) {
            Ok(_) => panic!("nothing listens on port 1"),
            Err(e) => e.to_string(),
        };

        assert!(
            !err.contains(&secret),
            "the connection string can carry a password and must never be echoed: {err}"
        );
        assert!(
            err.contains("connecting to the database"),
            "the line must say what was being attempted: {err}"
        );
        assert!(
            err.to_lowercase().contains("refused") || err.contains("os error"),
            "\u{2026}and name the errno, which is the half `Display` drops: {err}"
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

    /// Connect + take the shared test advisory lock (same 'CARN' key every
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
            clock_grade: ClockGrade::SelfAsserted,
            safety: None,
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
            // These quarantine/cursor tests don't exercise custody; ship no DEKs, and
            // report no refusal — nothing was withheld, there was simply nothing to send.
            wrapped_deks: vec![None; events.len()],
            custody_withheld: None,
            // A canned response is the WHOLE answer for these tests — none of them
            // exercise paging (that is Task 7's job) — so it always declares complete.
            complete: true,
        })
        .unwrap()
    }

    /// Run the REAL `serve_conn` against one request and return its response frame. The
    /// existing `serve_canned` serves a pre-encoded response, which cannot exercise the serve
    /// arm itself — and the serve arm is what this test is about.
    fn serve_one_request(conn: &str, req: &Request) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let conn = conn.to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_conn(&conn, stream, false, None).unwrap();
        });
        let raw = TcpTransport::new(&addr).try_once(req).unwrap();
        server.join().unwrap();
        raw
    }

    /// A limited request must return at most `limit` events and must declare `complete`
    /// truthfully at, below and above the boundary. `complete` is the puller's ONLY
    /// termination signal, so a serve that reports it wrongly either strands events
    /// forever or spins the puller.
    #[test]
    fn serve_honours_the_page_limit_and_declares_completeness() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        // Three events, applied locally so this node has something to serve.
        for i in 0..3 {
            let bytes = peer_note(&sk, &kid, WALL_2026 + i);
            c.execute("SELECT apply_remote_event($1, NULL, NULL, NULL)", &[&bytes])
                .unwrap();
        }

        // (limit, expected event count, expected `complete`)
        let cases: [(Option<u32>, usize, bool); 4] = [
            (Some(2), 2, false),
            // THE BOUNDARY. `rows.len() == limit` is ambiguous on its own — the limit+1
            // probe is what makes this case answerable rather than guessable.
            (Some(3), 3, true),
            (Some(9), 3, true),
            (None, 3, true),
        ];
        for (limit, want_len, want_complete) in cases {
            let raw = serve_one_request(
                &base,
                &Request::EventsAfterSeq {
                    after_seq: 0,
                    unwrap_cert: None,
                    limit,
                },
            );
            let resp: EventsResponse = serde_json::from_slice(&raw).unwrap();
            assert_eq!(resp.events.len(), want_len, "limit={limit:?}");
            assert_eq!(resp.seqs.len(), want_len, "limit={limit:?}");
            assert_eq!(resp.complete, want_complete, "limit={limit:?}");
        }
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

    /// **PR #493 review — the wedge the first draft of #480 created.**
    ///
    /// The interruption arm was guarded by `!is_deliberate_refusal()`, which is a far wider
    /// set than "this node's machine broke": a cast on a peer-supplied field (`22P02`), a
    /// constraint violation, an `XX000` from a function fed adversarial bytes are all
    /// DETERMINISTIC. Halting on one stopped every future `requeue` at the same row — the
    /// listing is `ORDER BY first_seen` — and `cairn-sync quarantine` is read-only, so raw
    /// SQL was the operator's only remedy. `db/001_envelope.sql`'s header records exactly
    /// that outcome one plane over, from one buggy peer.
    ///
    /// So a byte-attributable failure ANNOTATES and continues. What it must not do is
    /// annotate in the door's voice: `last_requeue_error` is what an operator reads while
    /// deciding whether an event is corrupt, and "the floor refused this" and "the door
    /// broke on this" are different findings.
    #[test]
    fn a_byte_attributable_apply_failure_annotates_and_does_not_wedge_the_sweep() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        // TWO rows: the claim is about the row BEHIND the failing one, which a halt strands.
        for (n, tag) in [
            (1i64, &b"first penned bytes"[..]),
            (2, &b"second penned bytes"[..]),
        ] {
            c.execute(
                "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, refused_seq, reason)
                 VALUES ($1, $2, 'peer-a', $3, 'unverifiable at first sight')",
                &[&cairn_event::event_address(tag), &tag.to_vec(), &n],
            )
            .unwrap();
        }
        c.batch_execute(
            "CREATE OR REPLACE FUNCTION apply_remote_event(
                 p_signed       BYTEA,
                 p_attestation  BYTEA DEFAULT NULL,
                 p_attester_key BYTEA DEFAULT NULL,
                 p_dek          BYTEA DEFAULT NULL
             ) RETURNS UUID LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'invalid input syntax for type bigint: \"not-a-wall\"'
                       USING ERRCODE = '22P02';
             END $$;",
        )
        .unwrap();

        let metrics = do_requeue(&mut c);

        for (name, sql) in SCHEMA {
            if name.starts_with("020") {
                c.batch_execute(sql).unwrap();
            }
        }
        let metrics = metrics.expect("a deterministic byte failure must not halt the sweep");

        assert_eq!(metrics["examined"], 2, "both rows were listed: {metrics}");
        assert_eq!(
            metrics["still_quarantined"], 2,
            "BOTH rows were judged — the second is the one a halt would have stranded, \
             for a fault no operator can fix by mending a disk: {metrics}"
        );

        let annotations: Vec<Option<String>> = c
            .query(
                "SELECT last_requeue_error FROM sync_quarantine ORDER BY refused_seq",
                &[],
            )
            .unwrap()
            .iter()
            .map(|r| r.get(0))
            .collect();
        for a in &annotations {
            let text = a.as_deref().unwrap_or("");
            assert!(
                text.contains("NOT a deliberate floor refusal"),
                "the door did not adjudicate these bytes and the column must not imply \
                 it did (#480's own claim, in the arm that keeps the row): {text}"
            );
            assert!(
                text.contains("22P02"),
                "…and it must name the condition, in the DATABASE's vocabulary — the \
                 door's own Display omits the SQLSTATE by design: {text}"
            );
        }
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

    /// ADR-0064: db/048 references `cairn_claim_authority`, which lives in db/005. Both
    /// files are in this subset — but `LANGUAGE sql` resolves the reference EAGERLY at
    /// CREATE time, so a subset that carried db/048 without db/005 would fail outright to
    /// create `cairn_sensitivity_standing`, taking clinical sync down entirely. `locked_client`
    /// merely loading both files without erroring already proves the CREATEs succeeded — it
    /// does NOT prove the functions run correctly: a stale search_path or a migration-order
    /// difference at CREATE time could still bind a `LANGUAGE sql` reference to the wrong
    /// definition, or leave it silently unresolved, with the CREATE itself staying silent and
    /// nothing surfacing until CALL time. (A missing GRANT is a SEPARATE axis this suite would
    /// NOT catch — it connects as the owning role, and that axis is already pinned by
    /// `claim_authority.rs`'s `the_read_path_works_as_cairn_agent`.) #386 records exactly the
    /// load-without-drive gap against db/048's OWN earlier subset coverage — loaded into
    /// SCHEMA but never DRIVEN, so the guarantee it appeared to give was untested at runtime.
    /// This test drives both functions with real arguments so the same gap, reopened by this
    /// slice's db/005 addition, does not recur unnoticed.
    #[test]
    fn db048_authority_gate_resolves_in_the_sync_subset() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base); // loads the whole SCHEMA subset
        let verdict: String = c
            .query_one(
                "SELECT cairn_claim_authority(gen_random_uuid(), gen_random_uuid())",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            verdict, "unverified",
            "the predicate must RUN on the sync subset — two fresh random uuids name no \
             real event, so R1/R2 both fail closed"
        );
        // And the seam that calls it (db/048 section 9) must be callable here too, not just
        // creatable — the actual regression shape #386 caught.
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM cairn_sensitivity_standing(gen_random_uuid())",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            n, 0,
            "a patient with no sensitivity events must have an empty standing set"
        );
    }

    // -----------------------------------------------------------------------
    // Issue #480 — a transient local fault must not be recorded as a door refusal.
    // -----------------------------------------------------------------------
    //
    // `ApplyError` was documented as "the door's own refusal, already legible". Neither
    // half held. `ApplyError::from`'s `None` arm — taken when there is no `DbError` at
    // all, i.e. the connection died rather than the door deciding — fell back to
    // `postgres::Error`'s own `Display`, which is `db error`. And `apply_signed`'s FIRST
    // statement is a bare `?` on a newness probe, so a `postgres::Error` from a statement
    // that never reached the door becomes an `ApplyError` too.
    //
    // The consequence is in `do_requeue`, which had no `is_deliberate_refusal()` check on
    // that path: a lock storm, or a `pg_restore` that has not finished re-granting, gets
    // written into `sync_quarantine.last_requeue_error` as though the in-DB door had
    // adjudicated those bytes and rejected them. That annotation is what an operator
    // reads while deciding whether an event is corrupt.

    /// The behavioural half, end to end: a transient local fault during `requeue` must
    /// interrupt with a partial-completion report, NOT annotate the pen row.
    ///
    /// Forced by holding an `ACCESS EXCLUSIVE` lock on `event_log` from a second
    /// connection while the requeue runs under a short `lock_timeout`, so `apply_signed`'s
    /// newness probe — its first statement, and one that never reaches the door — fails
    /// with `55P03`. The lock is released by the `ROLLBACK` (and by the OS if this test
    /// ever panics between the two), so nothing persists into the shared test database.
    #[test]
    fn a_transient_fault_during_requeue_is_not_recorded_as_a_door_refusal() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let garbage = b"penned bytes whose requeue meets a lock storm".to_vec();
        let digest = cairn_event::event_address(&garbage);
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, refused_seq, reason)
             VALUES ($1, $2, 'peer-a', 1, 'unverifiable at first sight')",
            &[&digest, &garbage],
        )
        .unwrap();

        let mut blocker = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        // The BLOCKER gets a timeout too (PR #493 review): without one, anything else
        // holding a lock on this table turns the test into a HANG rather than a failure,
        // and a suite that hangs is one nobody can read the result of.
        blocker
            .batch_execute(
                "SET lock_timeout = '10s'; BEGIN; LOCK TABLE event_log IN ACCESS EXCLUSIVE MODE;",
            )
            .unwrap();
        c.batch_execute("SET lock_timeout = '750ms'").unwrap();

        let err = do_requeue(&mut c).expect_err("the newness probe cannot read event_log");

        c.batch_execute("RESET lock_timeout").unwrap();
        blocker.batch_execute("ROLLBACK;").unwrap();

        let ie = err
            .downcast_ref::<RequeueInterruptedError>()
            .unwrap_or_else(|| panic!("a local fault must INTERRUPT, not refuse: {err}"));
        assert!(
            ie.message.contains("55P03"),
            "the operator line must name the condition that was met: {}",
            ie.message
        );

        // The claim that matters. `last_requeue_error` says "the in-DB door adjudicated
        // these bytes and rejected them" — a door that was never reached must not leave
        // that behind, and `still_quarantined` must not count a row it never judged.
        let annotation: Option<String> = c
            .query_one(
                "SELECT last_requeue_error FROM sync_quarantine WHERE content_digest = $1",
                &[&digest],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            annotation, None,
            "a lock timeout is not a verdict about these bytes: {annotation:?}"
        );
        assert_eq!(
            ie.metrics["still_quarantined"], 0,
            "nothing was judged, so nothing is counted as still refused: {}",
            ie.metrics
        );
    }

    // -----------------------------------------------------------------------
    // Issue #489 part 1 / #490 item 2 — a pen write refused by THIS NODE'S DATABASE.
    // -----------------------------------------------------------------------
    //
    // `pen_refused` is set when the quarantine INSERT fails, and it fed `cycle_is_loud` →
    // `PullIntegrityError` → the single class `integrity`, whose documented meaning is
    // *the peer answered and its DATA is the problem*. So a local disk-full, a grant
    // revoked by a restore, or a lock timeout sent an operator to audit a peer's
    // signatures for a write failure on this node's own disk — #469's species in a third
    // direction.
    //
    // It is genuinely BOTH: the peer really did send something we could not admit (that
    // is why a pen write was attempted at all), AND our database refused to record it.
    // Two classes, exactly as `CursorCommitError::also_loud` already models the mirror
    // case. The pen's OTHER failure mode — the per-peer quota — is not a local fault at
    // all: it is a resource budget exhausted by the peer's own garbage, so it stays
    // `integrity` alone. The two are told apart by whether a `postgres::Error` is
    // reachable in the chain, which is why #490 item 2 (the private `legible()` that
    // flattened DB errors into a `String`, destroying `source()`) had to be fixed first.

    /// The pure half: a loud cycle whose pen write was refused BY THE DATABASE is both.
    #[test]
    fn a_pen_write_refused_by_the_database_is_local_fault_as_well_as_integrity() {
        let both = PullIntegrityError {
            message: "2 unverifiable; Quarantine pen refused (cursor frozen): …".into(),
            metrics: serde_json::json!({"applied_new": 1}),
            also_local_fault: true,
        };
        let (classes, metrics) = classify_pull_failure(&both);
        assert!(
            classes.contains(&"integrity") && classes.contains(&"local_fault"),
            "the peer sent something we could not admit AND our own database refused to \
             record it — both conditions are true: {classes:?}"
        );
        assert_eq!(
            metrics["applied_new"], 1,
            "the cycle's work is still published"
        );

        // …and the quota case, which is the peer's garbage filling a budget, stays one
        // class. Widening this to "any pen refusal is local" would charge this node's
        // uptime for a peer flooding the pen.
        let quota = PullIntegrityError {
            message: "quarantine pen for peer 'peer-a' is at its quota".into(),
            metrics: serde_json::Value::Null,
            also_local_fault: false,
        };
        let (classes, _) = classify_pull_failure(&quota);
        assert_eq!(classes, &["integrity"], "{classes:?}");
    }

    /// **#490 item 2.** `quarantine_event`'s DB failures must stay CLASSIFIABLE.
    ///
    /// Its private `legible()` rendered a server error into `db.message().to_string()` —
    /// a `String`, which has no `source()` — so the SQLSTATE, the DETAIL and the original
    /// error were all gone by the time anything could read them, and the `None` arm fell
    /// back to `postgres::Error`'s own `Display`: the two words `db error`. That is
    /// exactly the trap `LocalDbFault`'s own doc warns about, one file over — and it was
    /// still live in this helper.
    ///
    /// The failure is forced by an ABORTED TRANSACTION: every later statement on that
    /// connection fails `25P02` until it is rolled back. Chosen over a REVOKE or a
    /// trigger for the reason the cursor-commit test gives — those persist in the shared
    /// test database if the test panics; a `ROLLBACK` cannot be left behind, because the
    /// connection is dropped at the end of the test either way.
    #[test]
    fn a_pen_write_refused_by_the_database_keeps_its_sqlstate_and_its_source() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        c.batch_execute("BEGIN").unwrap();
        c.batch_execute("SELECT 1/0")
            .expect_err("division by zero aborts the transaction");

        let err = quarantine_event(&mut c, "peer-a", b"some bytes", None, None, 1, "reason")
            .expect_err("no statement succeeds in an aborted transaction");
        let text = err.to_string();
        c.batch_execute("ROLLBACK").unwrap();

        assert!(
            chain_reaches_a_postgres_error(err.as_ref()),
            "the postgres error must stay REACHABLE — flattening it into a String is what \
             silently reverts a local fault to `partition`: {text}"
        );
        assert_ne!(text, "db error", "{text}");
        assert!(
            text.contains("25P02"),
            "the SQLSTATE is what tells an operator which condition was met: {text}"
        );
        // The dedupe UPDATE is the FIRST statement `quarantine_event` runs, so it is the
        // one that meets an aborted transaction — and naming the operation is half of
        // what `LocalDbFault` exists for.
        assert!(
            text.contains("recording a re-offer of already-penned bytes"),
            "…and the line must say what this node was DOING: {text}"
        );
    }

    /// The seam, end to end: a pen write that the database refuses must reach the
    /// operator (and `bet_a.py`) as BOTH classes.
    ///
    /// Forced with a row lock on the pen row the dedupe UPDATE is about to take, held
    /// from a second connection while the puller runs under a short `lock_timeout` — the
    /// same shape (and the same reasoning about not poisoning the shared test database)
    /// as `a_failed_cursor_commit_keeps_the_cycles_metrics_and_is_not_a_partition`, one
    /// table over. Without this the fix would be pinned only by fixtures the tests build
    /// themselves, and a revert at the freeze site would stay green.
    #[test]
    fn a_pen_write_the_database_refuses_reports_both_classes() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let garbage = b"unverifiable bytes whose pen row is already locked".to_vec();
        let digest = cairn_event::event_address(&garbage);

        // The row must EXIST before it can be locked — so the dedupe UPDATE at the top of
        // `quarantine_event` is the statement that blocks.
        c.execute(
            "INSERT INTO sync_quarantine (content_digest, signed_bytes, peer, refused_seq, reason)
             VALUES ($1, $2, 'peer-a', 1, 'seeded by the test')",
            &[&digest, &garbage],
        )
        .unwrap();
        let mut blocker = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        // The BLOCKER gets a timeout too (PR #493 review): without one, anything else
        // holding a lock on this table turns the test into a HANG rather than a failure,
        // and a suite that hangs is one nobody can read the result of.
        blocker
            .batch_execute(
                "SET lock_timeout = '10s'; BEGIN; \
                 SELECT 1 FROM sync_quarantine WHERE peer='peer-a' FOR UPDATE;",
            )
            .unwrap();
        c.batch_execute("SET lock_timeout = '750ms'").unwrap();

        let addr = serve_canned(response_json(&[&garbage], Some(CTX_EVENT.as_str())), 1);
        let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect_err("the pen write cannot acquire the row lock");

        c.batch_execute("RESET lock_timeout").unwrap();
        blocker.batch_execute("ROLLBACK;").unwrap();

        let (classes, _) = classify_pull_failure(err.as_ref());
        assert!(
            classes.contains(&"local_fault"),
            "the pen write failed on THIS node's database; auditing the peer's signatures \
             is the wrong instruction: {classes:?} — {err}"
        );
        assert!(
            classes.contains(&"integrity"),
            "…and the peer's event is still not held here, which needs a human: \
             {classes:?} — {err}"
        );
        assert!(
            err.to_string().contains("55P03"),
            "the operator line must carry the SQLSTATE that names the condition: {err}"
        );
    }

    // -----------------------------------------------------------------------
    // Issue #489 part 2 — "the peer answered with garbage" is not link downtime.
    // -----------------------------------------------------------------------
    //
    // `classify_pull_failure`'s default arm argued it was safe BY ELIMINATION: "a failure
    // this function does not recognise is, by elimination, one where the peer did not
    // answer". Three sites in `do_pull` falsified that. All three returned a bare
    // `String`/`serde_json::Error`, fell to `partition`, and were counted by `bet_a.py` as
    // link downtime — for a peer that had answered in full over a healthy link.
    //
    // Sixteen lines above the second of them, the structurally identical signing-context
    // skew already returned a `PullIntegrityError`. These tests drive the real `do_pull`
    // against a canned serve so a revert at any of the three sites is caught here, not
    // just in the classifier's own unit tests (which build their fixtures themselves).

    /// A response body that is not JSON at all: truncated, or a peer serving something
    /// else entirely on the sync port.
    #[test]
    fn a_corrupt_response_body_is_the_peer_not_the_link() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let addr = serve_canned(b"{ this is not an EventsResponse".to_vec(), 1);

        let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect_err("a body that will not deserialize cannot be applied");
        let (classes, _) = classify_pull_failure(err.as_ref());
        assert_eq!(
            classes,
            &["integrity"],
            "the peer ANSWERED — sending an operator to the WAN is the wrong \
             instruction, and bet_a.py would charge the outage figure for it: {err}"
        );
    }

    /// A response carrying events but a short `seqs` array (issue #196's own guard). The
    /// peer serves an incompatible or unexpectedly-old wire format; the link is fine.
    #[test]
    fn a_seq_count_mismatch_is_the_peer_not_the_link() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        // One event, ZERO seqs: the puller cannot checkpoint its cursor safely.
        let mut resp: serde_json::Value =
            serde_json::from_slice(&response_json(&[&e1], Some(CTX_EVENT.as_str()))).unwrap();
        resp["seqs"] = serde_json::json!([]);
        let addr = serve_canned(serde_json::to_vec(&resp).unwrap(), 1);

        let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect_err("events without seqs cannot checkpoint the cursor");
        let (classes, _) = classify_pull_failure(err.as_ref());
        assert_eq!(classes, &["integrity"], "{err}");
    }

    /// Seq VALUES that are not strictly ascending and positive — a buggy or hostile peer.
    /// These are untrusted wire input that would otherwise persist into `sync_state`, so
    /// the batch is refused; the refusal must name the peer, not the network.
    #[test]
    fn malformed_seq_values_are_the_peer_not_the_link() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let mut resp: serde_json::Value =
            serde_json::from_slice(&response_json(&[&e1, &e2], Some(CTX_EVENT.as_str()))).unwrap();
        resp["seqs"] = serde_json::json!([7, 3]); // descending: the freeze logic relies on order
        let addr = serve_canned(serde_json::to_vec(&resp).unwrap(), 1);

        let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect_err("descending seqs must not poison the persistent cursor");
        let (classes, _) = classify_pull_failure(err.as_ref());
        assert_eq!(classes, &["integrity"], "{err}");
    }

    /// Unwrap a pull that must fail as a PullIntegrityError; returns (message, metrics).
    fn pull_integrity_err(
        c: &mut postgres::Client,
        addr: &str,
        peer: &str,
    ) -> (String, serde_json::Value) {
        let err = do_pull(c, &TcpTransport::new(addr), peer, false, None).unwrap_err();
        let ie = err
            .downcast_ref::<PullIntegrityError>()
            .expect("pull must fail as an INTEGRITY error, not transport");
        (ie.message.clone(), ie.metrics.clone())
    }

    /// **#469, end to end.** A cursor commit that fails must not take the cycle's report
    /// down with it, and must not be reported as a partition.
    ///
    /// The failure is forced by holding a `FOR UPDATE` row lock from a SECOND connection
    /// while the puller runs under a short `lock_timeout`. That shape is chosen over a
    /// trigger or a `REVOKE` on purpose: those persist in the shared test database if this
    /// test ever panics between setting up and tearing down, poisoning every suite that
    /// runs after it. A row lock is released by the operating system when the second
    /// client drops, so the worst case of a panic here is a released lock.
    ///
    /// What it pins: the event still applied and is durable, the returned error is a
    /// `CursorCommitError` (not a bare `postgres::Error`), its metrics still carry that
    /// work, `classify_pull_failure` calls it a local fault, the message is legible rather
    /// than `"db error"`, and the two fields describing the FAILED write read `null`.
    #[test]
    fn a_failed_cursor_commit_keeps_the_cycles_metrics_and_is_not_a_partition() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let addr = serve_canned(response_json(&[&e1], Some(CTX_EVENT.as_str())), 1);

        // The row must exist before it can be locked; `do_pull` would otherwise create it.
        c.execute(
            "INSERT INTO sync_state (peer) VALUES ('peer-a') ON CONFLICT (peer) DO NOTHING",
            &[],
        )
        .unwrap();
        let mut blocker = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        // The BLOCKER gets a timeout too (PR #493 review): without one, anything else
        // holding a lock on this table turns the test into a HANG rather than a failure,
        // and a suite that hangs is one nobody can read the result of.
        blocker
            .batch_execute(
                "SET lock_timeout = '10s'; BEGIN; \
                 SELECT 1 FROM sync_state WHERE peer='peer-a' FOR UPDATE;",
            )
            .unwrap();
        // Short enough that the test is quick, long enough that a loaded rig does not
        // trip it on some unrelated statement.
        c.batch_execute("SET lock_timeout = '750ms'").unwrap();

        let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect_err("the cursor commit cannot acquire the row lock");

        c.batch_execute("RESET lock_timeout").unwrap();
        blocker.batch_execute("ROLLBACK;").unwrap();

        let ce = err
            .downcast_ref::<CursorCommitError>()
            .unwrap_or_else(|| panic!("must be a CursorCommitError, got: {err}"));
        let (classes, metrics) = classify_pull_failure(err.as_ref());
        assert_eq!(
            classes,
            &["local_fault"],
            "a local write failure is not link downtime: {}",
            ce.message
        );

        // The work really happened, and the cycle says so.
        let applied: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(applied, 1, "the event applied in its own transaction");
        assert_eq!(
            metrics["applied_new"], 1,
            "…and the metrics survived the failure to report it: {metrics}"
        );
        assert_eq!(metrics["shipped"], 1, "{metrics}");
        // The #465 ledger read runs AFTER the commit attempt, including when it failed:
        // the flag rows are durable regardless, so it stays a true report about completed
        // work. Seeded `null`, so `0` here proves the read actually ran rather than the
        // seed surviving @EM@ move `emit_unlearnable_report` above the failure return and
        // this goes red (PR #472 review).
        assert_eq!(
            metrics["references_unlearnable"], 0,
            "the report about durable work still runs when the commit failed: {metrics}"
        );
        // Newly nullable this PR (seeded null, filled after the commit). Nothing else
        // asserts it, and `bet_a.py`'s median(lat) throws on a null.
        assert!(
            metrics["elapsed_ms"].is_number(),
            "every published cycle carries its latency: {metrics}"
        );

        // The write that failed is reported as unknown, never as the value it tried.
        assert!(
            metrics["cursor_seq"].is_null() && metrics["floor_active"].is_null(),
            "the cursor did not move, so neither field may claim it did: {metrics}"
        );
        assert_eq!(
            cursor(&mut c, "peer-a"),
            0,
            "and the persisted cursor really is where it was"
        );

        // And the line an operator reads names the cause (#467's species).
        assert!(
            !ce.message.contains("db error") && ce.message.contains("[55P03]"),
            "lock_not_available, named: {}",
            ce.message
        );
    }

    /// **PR #472 review, finding 3, end to end.** A DEAD local database is a local fault,
    /// and this is the half of #469 that never self-heals.
    ///
    /// #469 fixed the cursor commit. But `do_pull`'s FIRST TWO statements — the `sync_state`
    /// upsert and read — are bare `?` on a `postgres::Error`, and they run before any
    /// network I/O happens at all. `cmd_run` builds its client ONCE, outside the loop
    /// (`connect_checked_apply`), and never reconnects: so when the database goes away
    /// mid-run, every remaining cycle for the life of the process failed at that first
    /// statement and was logged `"partition": true`. An operator sent to look at a
    /// satellite link, and the Bet A availability figure charged for a local outage —
    /// which is issue #469's own title sentence, two statements above where #469 was fixed.
    ///
    /// The failure is forced by terminating the client's own backend from a SECOND
    /// connection. That is chosen over stopping the server (obviously) and over revoking a
    /// grant (which persists in a shared test database if the test panics, poisoning every
    /// later suite): the only thing killed is a connection this test opened, and the
    /// serialization guard is held on a DIFFERENT client so the kill cannot release it.
    ///
    /// The peer address is deliberately unreachable. If this test ever fails by REACHING
    /// it, the ordering assumption above is what broke.
    #[test]
    fn a_dead_local_database_is_a_local_fault_not_a_partition() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        // Guard on its own client, so terminating the victim below cannot drop the
        // advisory lock and let a concurrent suite interleave.
        let _guard = locked_client(&base);

        let mut victim = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        let pid: i32 = victim
            .query_one("SELECT pg_backend_pid()", &[])
            .unwrap()
            .get(0);
        let mut killer = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        // Ignore the result: the backend may be gone before the reply is written.
        let _ = killer.execute("SELECT pg_terminate_backend($1)", &[&pid]);

        // Port 1 is privileged and nothing listens on it: if the pull got this far it
        // would fail as a genuine partition, which is exactly what must NOT happen here.
        let err = do_pull(
            &mut victim,
            &TcpTransport::new("127.0.0.1:1"),
            "peer-a",
            false,
            None,
        )
        .expect_err("the client's backend has been terminated");

        let (classes, metrics) = classify_pull_failure(err.as_ref());
        assert_eq!(
            classes,
            &["local_fault"],
            "this node's database is gone; the link was never touched: {err}"
        );
        assert!(
            metrics.is_null(),
            "it failed before any work was measurable, so it claims nothing: {metrics}"
        );

        // …AND THE MESSAGE, which is the half this test used to leave unasserted — the
        // reason #479 survived the fix that added the class above. This is the end-to-end
        // form of `a_failed_pull_records_a_legible_reason_not_db_error`: not a constructed
        // error but the real one a terminated backend produces, through the real door.
        let mut line = serde_json::json!({ "cycle": 118 });
        record_pull_failure(&mut line, err.as_ref());
        let reported = line["pull_error"].as_str().expect("a reason is recorded");

        assert_ne!(reported, "db error", "#479's title sentence, verbatim");
        assert!(
            reported.starts_with("registering this peer in sync_state: ")
                || reported.starts_with("reading this peer's sync cursor: "),
            "the operator learns WHICH statement met the dead backend — either of the two \
             is correct, since which one is reached first depends on how far the \
             termination had progressed: {reported}"
        );
        assert!(
            !reported.contains('\n'),
            "one line, even end to end: {reported}"
        );
    }

    /// A REAL SERVER ERROR is rendered ONCE by [`operator_chain`] — the `Kind::Db` arm.
    ///
    /// The pure fixtures above build their `postgres::Error` from an unparseable
    /// connection string, which is `Kind::ConfigParse`. That arm renders through
    /// [`kind_and_causes`], which **already walks `source()`**, so the rendered text ends
    /// with the cause's own text, the suffix rule drops the next layer, and the dedupe
    /// appears to work whether or not the walk stops.
    ///
    /// `Kind::Db` — the arm every in-DB refusal takes — behaves differently.
    /// [`legible_db_error`] renders it as `message [SQLSTATE] — DETAIL — HINT`, while
    /// `Error::source()` hands back the `DbError` underneath, whose own `Display` is
    /// `severity: message` + `\nDETAIL:` + `\nHINT:`. Neither is a suffix of the other,
    /// so without the `break` the server's message is printed TWICE on one line — the
    /// exact duplication `operator_chain`'s header rejects `{e:#}` for.
    ///
    /// It needs a live server because a `DbError` cannot be constructed by hand. The
    /// identical test exists in `cairn-node`'s `db_diagnosis` suite, where the same defect
    /// was found and fixed in the same commit.
    #[test]
    fn a_server_error_is_rendered_once_through_the_whole_chain() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        // No schema and no writes: a bare connection is the whole fixture, so this needs
        // neither the serial guard nor a loaded database.
        let mut c = postgres::Client::connect(&base, postgres::NoTls).unwrap();

        // Distinctive strings throughout — a substring that could occur incidentally
        // would make the counts below meaningless.
        let pg = c
            .batch_execute(
                "DO $$ BEGIN RAISE EXCEPTION 'the quarantine pen refused' \
                 USING ERRCODE = 'P0001', DETAIL = 'a distinctive detail', \
                 HINT = 'a distinctive hint'; END $$;",
            )
            .expect_err("the DO block always raises");

        let wrapped: Box<dyn Error> = LocalDbFault::boxed("listing the quarantine pen", pg);
        let line = operator_chain(wrapped.as_ref());

        assert!(
            line.starts_with("listing the quarantine pen: "),
            "the operation leads: {line}"
        );
        assert!(line.contains("[P0001]"), "the SQLSTATE survives: {line}");
        for once in [
            "the quarantine pen refused",
            "a distinctive detail",
            "a distinctive hint",
        ] {
            assert_eq!(
                line.matches(once).count(),
                1,
                "{once:?} must appear EXACTLY once — twice is `{{e:#}}`'s defect, which \
                 this function exists to avoid: {line}"
            );
        }
        assert!(!line.contains('\n'), "still one line: {line}");
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
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
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

    /// A validly-signed peer `note.added` carrying `n` by-reference renditions whose
    /// `digest_hex` is not hex at all — the #460 shape. Returns (signed bytes, event id).
    ///
    /// MALFORMED rather than merely unknown, on purpose: a well-formed digest for bytes
    /// this node has never seen is the NORMAL case and is learned into `blob_store`
    /// (references are eager, bytes are lazy). Only a reference the db/027 accessors
    /// cannot parse reaches the ledger, and it is the only thing this metric counts.
    fn peer_note_with_bad_references(
        sk: &SigningKey,
        kid: &str,
        wall: i64,
        n: usize,
    ) -> (Vec<u8>, String) {
        let event_id = uuid::Uuid::now_v7().to_string();
        let renditions = (0..n)
            .map(|i| cairn_event::Rendition {
                role: format!("original-{i}"),
                alg: "blake3".into(),
                // Not hex, so `cairn_rendition_address` refuses it with P0001 and the
                // LENIENT learner records the refusal instead of re-raising it.
                digest_hex: "0xABC".into(),
                media_type: "image/jpeg".into(),
                byte_len: 1024,
                inline: None,
                seal: None,
            })
            .collect();
        let body = EventBody {
            event_id: event_id.clone(),
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
            payload: serde_json::json!({"text": "note with a photograph"}),
            attachments: vec![cairn_event::Attachment {
                descriptor: "a photograph of the wound".into(),
                renditions,
            }],
            plaintext_twin: Some("Progress note: note with a photograph".into()),
            clock_grade: ClockGrade::SelfAsserted,
            safety: None,
        };
        (sign(&body, sk).unwrap().signed_bytes, event_id)
    }

    /// How many flag rows this node holds for one event id.
    fn flag_count(c: &mut postgres::Client, event_id: &str) -> i64 {
        c.query_one(
            "SELECT count(*) FROM attachment_reference_flag WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .unwrap()
        .get(0)
    }

    /// **Issue #465.** The cycle stays QUIET (admit-and-flag, #460) but must not stay
    /// SILENT: the references it could not learn are counted and named.
    ///
    /// Before #460 this exact batch produced a P0001 refusal, a penned event and a
    /// `PullIntegrityError` an operator could not miss. Admitting is right — refusing
    /// forks the event set and the pen never releases — but the loud signal was removed
    /// without a replacement, and the ledger that replaced it had no caller. This test
    /// pins BOTH halves of the trade: the cycle succeeds AND it reports.
    ///
    /// The batch is deliberately mixed and the bad event carries TWO bad renditions, so
    /// the assertion separates "references" from "events": one event, two unlearnable
    /// references, two other clean events' worth of clinical content untouched.
    #[test]
    fn pull_admits_a_malformed_reference_quietly_and_still_reports_it() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let clean = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let (bad, bad_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026 + 2_000, 2);
        let (bad2, _bad2_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026 + 3_000, 1);
        let raw = response_json(&[&clean, &bad, &bad2], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);

        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect("a malformed attachment reference must NOT fail the cycle (#460)");

        assert_eq!(m["applied_new"], 3, "every event is in the record");
        assert_eq!(m["skipped_unverifiable"], 0, "nothing was penned");
        assert_eq!(m["refused_verifiable"], 0, "the floor refused nothing");
        assert_eq!(m["watermark_frozen"], false, "the cursor moved on");
        assert_eq!(
            m["references_unlearnable"], 3,
            "TWO unlearnable references on one event and one on another — the metric \
             counts REFERENCES, not events, because how much of a rendition set is \
             broken is what separates one bad file from a broken encoder: {m}"
        );
        assert_eq!(
            flag_count(&mut c, &bad_id),
            2,
            "the ledger holds the standing fact, keyed to the event that carries it"
        );

        // The EXAMPLE the operator line names must be the EARLIEST flag, and must not
        // depend on the order the addresses happen to arrive in — an operator re-reading
        // the line has to land on the same event they were just looking at. Asserted
        // through the reader itself, because the example rides the stderr line rather
        // than the metrics object.
        let found = unlearnable_references(
            &mut c,
            &[
                cairn_event::event_address(&bad2),
                cairn_event::event_address(&bad),
            ],
            0,
        )
        .unwrap()
        .expect("three flags were just recorded");
        assert_eq!(found.count, 3, "both events' flags, asked about together");
        assert_eq!(
            found.example_event, bad_id,
            "the example is the earliest flag, whatever order the events are asked about in"
        );
        assert!(
            found.example_reason.contains("digest_hex"),
            "and it carries the accessor's own words: {}",
            found.example_reason
        );

        // **#466 review.** The address filter, pinned DIRECTLY. Ask about ONE of the two
        // flagged events and only its flags come back — a peer's report must never name
        // an event that peer did not ship. This used to be pinned only by
        // `a_flag_from_outside_this_cycle_is_not_charged_to_this_peer`, which the flag_id
        // watermark now also satisfies; without this assertion, dropping
        // `WHERE e.content_address = ANY($1)` would leave the whole family green.
        let only_bad2 = unlearnable_references(&mut c, &[cairn_event::event_address(&bad2)], 0)
            .unwrap()
            .expect("bad2 carries one flag");
        assert_eq!(
            only_bad2.count, 1,
            "asked about one event, told about one event's flags — not the ledger's"
        );

        // **#466 review.** The watermark clause, pinned DIRECTLY: nothing was flagged
        // after the last flag, so the same question asked from the top of the ledger is
        // silent. This is what stops a re-offer re-announcing a defect forever.
        let mark = attachment_flag_watermark(&mut c).unwrap();
        assert!(mark > 0, "three flags were recorded, so the mark moved");
        assert!(
            unlearnable_references(
                &mut c,
                &[
                    cairn_event::event_address(&bad2),
                    cairn_event::event_address(&bad),
                ],
                mark,
            )
            .unwrap()
            .is_none(),
            "nothing is newer than the newest flag"
        );
    }

    /// **#466 review — the silent zero the first version shipped.** A flag can be born on
    /// a RE-APPLY, and the metric must report it.
    ///
    /// db/020 calls `cairn_learn_attachment_refs_lenient` UNCONDITIONALLY: it sits after
    /// the `ON CONFLICT (event_id) DO NOTHING` with no `v_rows` guard, so the learner runs
    /// again on an idempotent re-apply of an event this node already holds. Keying the
    /// report on newly-INSERTED events therefore made every such flag invisible — and the
    /// case that matters is not exotic: a node upgrading onto db/050 flags its entire
    /// pre-#460 back-catalogue on the next full sweep, and would have reported `0` for all
    /// of it. #463 gives a second route, since clearing a flag today means DELETE and the
    /// next sweep re-creates it.
    ///
    /// A silent zero in the reassuring direction, on the one cycle carrying the most news
    /// — which is the exact defect class issue #465 exists to end.
    #[test]
    fn a_flag_born_on_a_re_apply_is_still_this_runs_news() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // The event is ALREADY HELD — admitted by some earlier delivery — and its flag is
        // then cleared, standing in for the pre-db/050 history and for #463's DELETE.
        let (bad, bad_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026, 1);
        c.execute("SELECT apply_remote_event($1)", &[&bad]).unwrap();
        c.execute(
            "DELETE FROM attachment_reference_flag WHERE event_id = $1::text::uuid",
            &[&bad_id],
        )
        .unwrap();
        assert_eq!(flag_count(&mut c, &bad_id), 0, "the ledger starts clear");

        // Now a peer re-offers those same bytes. Nothing is NEW to event_log…
        let raw = response_json(&[&bad], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
        assert_eq!(
            m["applied_new"], 0,
            "set-union no-op: the event was already held"
        );

        // …but the lenient learner ran again and wrote a flag row that did not exist
        // before this cycle. That IS this run's news.
        assert_eq!(
            flag_count(&mut c, &bad_id),
            1,
            "the learner ran on the re-apply"
        );
        assert_eq!(
            m["references_unlearnable"], 1,
            "a flag BORN this cycle is reported even though its event was not new — \
             keying on applied_new alone reported 0 here: {m}"
        );
    }

    /// **#466 review.** The line a failed ledger read produces must SAY WHAT FAILED.
    ///
    /// `postgres::Error`'s `Display` is a bare match on its kind: any server-side failure
    /// renders as the literal string `"db error"`, and `Display` does not chain to the
    /// source holding the message, the DETAIL and the SQLSTATE. So the first version of
    /// this surface — `.map_err(|e| e.to_string())` — emitted "could not read the
    /// attachment-reference ledger for this run: db error", which cannot tell a node that
    /// never loaded db/050 from a revoked grant from a statement timeout. Three different
    /// remedies, one indistinguishable line.
    ///
    /// That matters more here than almost anywhere: the entire justification for reporting
    /// `null` instead of `0` is that the failure is never silent. A line with no
    /// diagnosis in it is silent in every way that helps.
    ///
    /// The failure is forced through `search_path` rather than a role change, so it is
    /// deterministic on any rig regardless of which role the test connects as.
    #[test]
    fn a_failed_ledger_read_names_the_cause_and_not_just_db_error() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        c.batch_execute("BEGIN; SET LOCAL search_path = pg_catalog;")
            .unwrap();
        let err = unlearnable_references(&mut c, &[vec![0u8; 34]], 0)
            .expect_err("the ledger is not on the search_path here");
        let legible = legible_db_error(&err);
        c.batch_execute("ROLLBACK;").unwrap();

        assert_ne!(legible, "db error", "the whole point: {legible}");
        assert!(
            legible.contains("attachment_reference_flag"),
            "the line must name what could not be read: {legible}"
        );
        assert!(
            legible.contains("42P01"),
            "…and carry the SQLSTATE, which is the part an operator can search for: \
             {legible}"
        );

        // And end to end: metric UNKNOWN, one written line, cause inside it.
        let mut out: Vec<u8> = Vec::new();
        let metric = emit_unlearnable_report(&mut out, "pull peer-a", Err(legible));
        assert!(metric.is_null(), "a failed read is never 0: {metric}");
        let written = String::from_utf8(out).unwrap();
        assert!(
            written.contains("42P01") && written.contains("attachment_reference_flag"),
            "the operator line carries the diagnosis: {written}"
        );
        assert_eq!(written.lines().count(), 1, "still one line: {written}");
    }

    /// The metric reports what THIS cycle discovered, not what the node standing holds.
    ///
    /// Set-union sync re-offers the same bytes freely (a full sweep, a re-pull from zero,
    /// a peer serving twice), and the recorder dedupes on re-delivery — so a metric read
    /// off the whole ledger would re-announce the same defect every cycle forever, which
    /// is how an operator learns to ignore a line. The standing view is
    /// `cairn_attachment_flag_health()`; this number is the cycle's own news.
    #[test]
    fn a_re_offered_unlearnable_reference_is_not_reported_a_second_time() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        let (bad, bad_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026 + 1_000, 1);
        let raw = response_json(&[&bad], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 2);

        let first = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
        assert_eq!(first["applied_new"], 1);
        assert_eq!(first["references_unlearnable"], 1, "the cycle's own news");

        let second = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
        assert_eq!(second["applied_new"], 0, "set-union no-op on re-apply");
        assert_eq!(
            second["references_unlearnable"], 0,
            "one alert, not one per cycle forever: {second}"
        );
        assert_eq!(
            flag_count(&mut c, &bad_id),
            1,
            "the ledger keeps the standing fact (and dedupes the re-delivery)"
        );
    }

    /// The standing picture the pull line points at — and the reason db/050's node-wide
    /// read exists at all (issue #465's third acceptance criterion: it must have a caller
    /// outside the tests).
    ///
    /// TWO DIFFERENT QUESTIONS, deliberately answered by two different surfaces. The pull
    /// metric is this cycle's news: "what did I just admit that I cannot fetch?". This is
    /// the standing state: "what does this node hold, from every delivery, ever?" — the
    /// one that tells a single corrupted import from a peer whose encoder is broken,
    /// which is the question db/050's own header poses. A read surface with no caller is
    /// the Slice 69 finding, and this file shipped one.
    #[test]
    fn the_attachment_flags_listing_names_the_type_the_reason_and_an_example() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        assert!(
            attachment_flag_listing(&mut c).unwrap().is_empty(),
            "a healthy node lists nothing — the signal must not start as noise"
        );

        // Two unlearnable renditions on one note: same event type, same refusal text, so
        // they group into ONE row carrying a count of two.
        let (bad, bad_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026, 2);
        c.execute("SELECT apply_remote_event($1)", &[&bad]).unwrap();

        let rows = attachment_flag_listing(&mut c).unwrap();
        assert_eq!(rows.len(), 1, "one (event_type, reason) group: {rows:?}");
        assert_eq!(rows[0]["event_type"], "note.added");
        assert_eq!(rows[0]["flagged"], 2);
        assert_eq!(
            rows[0]["example_event"], bad_id,
            "NAME, NEVER COUNT — the group names an event to go and look at"
        );
        assert!(
            rows[0]["reason"]
                .as_str()
                .is_some_and(|r| r.contains("digest_hex")),
            "and says what was wrong with it: {rows:?}"
        );
        // The two time columns are read with the same non-panicking accessor as the rest
        // (#466 review) — assert they carry a value, so "renders as null" can only ever
        // mean the health function actually returned null.
        assert!(
            rows[0]["first_flagged"].is_string() && rows[0]["last_flagged"].is_string(),
            "the group states when it was first and last seen: {rows:?}"
        );
    }

    /// The count is attributed to the EVENTS THIS CYCLE APPLIED, not read off the whole
    /// ledger — so a defect discovered elsewhere is never charged to this peer.
    ///
    /// This is the assertion that makes the `WHERE e.content_address = ANY(...)` clause
    /// load-bearing rather than decorative: drop it and this test goes red while every
    /// other one in this family stays green, because they all run against a ledger whose
    /// only rows are their own. Two peers pulling concurrently is the real case, and a
    /// line naming another peer's event under this peer's name is exactly the kind of
    /// precise untruth an operator surface must not produce.
    #[test]
    fn a_flag_from_outside_this_cycle_is_not_charged_to_this_peer() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);

        // A flagged event that arrived some OTHER way — a different peer's cycle, or a
        // local hop. Applied directly through the same door, so the ledger row is real.
        let (elsewhere, elsewhere_id) = peer_note_with_bad_references(&sk, &kid, WALL_2026, 1);
        c.execute("SELECT apply_remote_event($1)", &[&elsewhere])
            .unwrap();
        assert_eq!(
            flag_count(&mut c, &elsewhere_id),
            1,
            "the ledger holds a standing flag before this peer's cycle begins"
        );

        // Now a perfectly clean batch from peer-a.
        let clean = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&clean], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();

        assert_eq!(m["applied_new"], 1);
        assert_eq!(
            m["references_unlearnable"], 0,
            "this cycle learned every reference it was offered — the standing ledger is \
             not its news, and another peer's defect is not its fault: {m}"
        );
    }

    /// `requeue` releases penned bytes through the SAME apply door (db/020), so it can
    /// discover the same unlearnable references — and it is the easier path to miss,
    /// because it prints one cheerful "released" line per row and nothing else.
    ///
    /// The setup is the ADR-0056 decision 5 sequence: an event from an author this node
    /// does not know is penned verbatim; enrolling the author repairs the cause; the
    /// release then admits it AND flags the reference it cannot learn.
    #[test]
    fn requeue_reports_a_reference_it_could_not_learn_on_release() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk_x, kid_x) = cairn_event::generate_key().unwrap();

        // Valid signature, unknown author, and a malformed reference riding along.
        let (penned, penned_id) = peer_note_with_bad_references(&sk_x, &kid_x, WALL_2026, 1);
        let raw = response_json(&[&penned], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let (_msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(
            m["refused_verifiable"], 1,
            "the door refused an unknown author"
        );
        assert_eq!(
            m["references_unlearnable"], 0,
            "nothing was ADMITTED, so there was no reference to learn or to flag: {m}"
        );
        assert_eq!(flag_count(&mut c, &penned_id), 0);

        // Repair the cause and release.
        c.execute(
            "SELECT enroll_actor('agent', '{\"model\":\"late-enrolled\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
            &[&kid_x],
        )
        .unwrap();
        let m = do_requeue(&mut c).unwrap();
        assert_eq!(m["released"], 1, "the repaired event is admitted");
        assert_eq!(
            m["references_unlearnable"], 1,
            "…and the reference it could not learn is reported, not swallowed: {m}"
        );
        assert_eq!(flag_count(&mut c, &penned_id), 1);
    }

    /// #471 / ADR-0060 decision 2: the sentence an operator reads after an interrupted
    /// requeue must say what the run ACHIEVED, not merely that it failed. **Pure**, so the
    /// wording is pinned without breaking a database mid-loop.
    #[test]
    fn an_interrupted_requeue_reports_the_work_that_survived_it() {
        let msg = requeue_interrupted_message(
            20,
            4,
            1,
            0,
            "a1b2c3d4e5f60718",
            "permission denied for table sync_quarantine [42501]",
        );
        assert!(
            msg.contains("4 released"),
            "the released events are DURABLE in event_log, and saying nothing about them \
             is the implied-completion ADR-0060 decision 2 forbids: {msg}"
        );
        assert!(
            msg.contains("20"),
            "the size of the run must be named, or the operator cannot tell how much was \
             never examined: {msg}"
        );
        assert!(
            msg.contains("a1b2c3d4e5f60718"),
            "…and WHERE it stopped: {msg}"
        );
        assert!(
            msg.contains("[42501]"),
            "…and the cause, which was the eight characters `db error` before: {msg}"
        );
        assert!(
            msg.contains("idempotent"),
            "the remedy is real — release runs through the same in-DB door a pull does — \
             and an operator who does not know that cannot safely re-run: {msg}"
        );
        // PR #478 review, finding 12. The row it stopped ON is in none of the three
        // counters, so a sentence that folds it into "never examined" is a precise
        // untruth about a row whose event may already be durable (principle 4).
        assert!(
            !msg.contains("the remaining rows were never examined"),
            "the row at `at` WAS reached, so this claim is false about it: {msg}"
        );
        assert!(
            msg.contains("counted in none of those three"),
            "…so the undecided row is named as undecided instead: {msg}"
        );
        assert!(
            msg.contains("listed"),
            "`examined` is the size of the up-front listing, not a count of rows \
             processed; calling it `examined` fought the next clause: {msg}"
        );
    }

    /// #471's acceptance criterion is a SENTENCE AN OPERATOR READS — so the rendering
    /// that actually reaches them has to be the composed one. (PR #478 review, finding 7.)
    ///
    /// `fn main() -> R<()>` has no error printer, so an `Err` reaches Rust's `Termination`,
    /// which prints `Error: {err:?}` — the **`Debug`** impl, not `Display`. With a derived
    /// `Debug` the operator saw the sentence wrapped in struct syntax with the metrics
    /// object re-dumped in Rust debug form beside the JSON already on stdout:
    ///
    /// ```text
    /// Error: RequeueInterruptedError { message: "requeue: INTERRUPTED at …", metrics: Object {…} }
    /// ```
    ///
    /// Both other tests assert `to_string()`, so they passed while production delivered
    /// the other rendering. **Pure.**
    #[test]
    fn the_interrupted_sentence_survives_the_debug_rendering_main_actually_prints() {
        let e = RequeueInterruptedError {
            message: requeue_interrupted_message(
                20,
                4,
                1,
                0,
                "a1b2c3d4e5f60718",
                "permission denied for table sync_quarantine [42501]",
            ),
            metrics: serde_json::json!({ "op": "requeue" }),
        };

        let debug = format!("{e:?}");
        assert!(
            !debug.contains("RequeueInterruptedError {"),
            "`Termination` prints Debug; struct syntax is not a sentence: {debug}"
        );
        assert_eq!(
            debug,
            e.to_string(),
            "Debug delegates to Display, so the operator reads the composed line either way"
        );
        assert!(
            debug.contains("4 released") && debug.contains("[42501]"),
            "…and the whole report survives it: {debug}"
        );
    }

    /// The behavioural half of #471: a requeue interrupted mid-loop still hands back the
    /// four counts, and does NOT report `references_unlearnable` as `0` — a number is a
    /// claim, and the #465 report never ran.
    ///
    /// # How the write failure is forced, and why it is a ROW LOCK
    ///
    /// A second connection holds `SELECT … FOR UPDATE` on the second pen row inside an open
    /// transaction, and this client runs under a short `lock_timeout`, so its release
    /// `DELETE` on that row fails with `55P03`. The `SELECT` above it is unaffected —
    /// MVCC readers do not block — which is exactly right: the row is fetched, the event is
    /// applied through the real door, and only the pen-row DELETE fails. That is the
    /// mid-loop shape the issue is about.
    ///
    /// A trigger or a `REVOKE` would do the same job and would PERSIST if this test
    /// panicked, poisoning every later suite in the shared database. A row lock dies with
    /// its connection.
    #[test]
    fn a_requeue_interrupted_mid_loop_still_reports_what_it_released() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);

        // Two events from an author this node does not know: db/020 refuses both with a
        // bare RAISE, so both are penned verbatim (ADR-0056 decision 5).
        let (sk_x, kid_x) = cairn_event::generate_key().unwrap();
        let a = peer_note(&sk_x, &kid_x, WALL_2026 + 1_000);
        let b = peer_note(&sk_x, &kid_x, WALL_2026 + 2_000);
        let raw = response_json(&[&a, &b], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);
        let (_msg, _m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(quarantine_rows(&mut c).len(), 2, "both events are penned");

        // Repair the cause, so the door will now admit both on release.
        c.execute(
            "SELECT enroll_actor('agent', '{\"model\":\"late-enrolled\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
            &[&kid_x],
        )
        .unwrap();

        // `do_requeue` walks the pen in `first_seen` order; block the SECOND row so the
        // first genuinely releases before the failure. Read the order rather than assuming
        // it — the assertion is about "row 2 of 2", not about which event that is.
        let second: Vec<u8> = c
            .query(
                "SELECT content_digest FROM sync_quarantine ORDER BY first_seen",
                &[],
            )
            .unwrap()[1]
            .get(0);

        let mut blocker = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        // The BLOCKER gets a timeout too (PR #493 review): without one, anything else
        // holding a lock on this row turns the test into a HANG rather than a failure.
        blocker
            .batch_execute("SET lock_timeout = '10s'; BEGIN")
            .unwrap();
        blocker
            .query(
                "SELECT 1 FROM sync_quarantine WHERE content_digest = $1 FOR UPDATE",
                &[&second],
            )
            .unwrap();

        // Short enough that the test is quick, long enough that a loaded rig does not
        // trip it spuriously on the FIRST row (which is unlocked and returns at once).
        c.batch_execute("SET lock_timeout = '750ms'").unwrap();
        let err = do_requeue(&mut c).expect_err("the release DELETE on row 2 must fail");

        // Let the blocker go before asserting: a panic below must not leave a lock held
        // for the rest of the suite.
        blocker.batch_execute("ROLLBACK").unwrap();
        drop(blocker);
        c.batch_execute("SET lock_timeout = 0").unwrap();

        let interrupted = err
            .downcast_ref::<RequeueInterruptedError>()
            .unwrap_or_else(|| panic!("must be a RequeueInterruptedError, got: {err}"));

        assert_eq!(
            interrupted.metrics["released"], 1,
            "row 1 WAS released and its event is durable; a run that says nothing about \
             it is the silence ADR-0060 decision 2 forbids: {}",
            interrupted.metrics
        );
        assert_eq!(
            interrupted.metrics["examined"], 2,
            "the size of the run survives the failure: {}",
            interrupted.metrics
        );
        assert!(
            interrupted.metrics["references_unlearnable"].is_null(),
            "null, NEVER zero: the #465 report never ran, and `0` would tell a monitor \
             this run had looked and found nothing: {}",
            interrupted.metrics
        );
        assert_ne!(
            interrupted.to_string(),
            "db error",
            "#467's species: {interrupted}"
        );
        assert!(
            interrupted.to_string().contains("[55P03]"),
            "the SQLSTATE must name the lock timeout, not hide behind a kind: {interrupted}"
        );
    }

    /// Issue #267 — ADR-0056 decision 5 for bytes that VERIFY but the floor
    /// deliberately refuses (here: an author this node has not enrolled). Before
    /// this, such a refusal persisted NOTHING: the puller froze its cursor, wrote
    /// a stderr line, and exited SUCCESS, so the evidence lived only in a log.
    /// Now it takes exactly the path an unverifiable refusal takes — verbatim
    /// bytes penned by digest with the door's own reason, the slot pinned on the
    /// re-offer floor, the cursor still advancing so other authors' events keep
    /// flowing (principle 5, availability over consistency), and the cycle loud.
    ///
    /// The second half is the "delayed, never lost" proof AND the pen's
    /// self-cleaning property: enroll the author, re-offer the same bytes, and
    /// the event applies while its now-resolved pen row auto-releases.
    #[test]
    fn pull_pens_a_deliberate_door_refusal_and_releases_it_on_repair() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        // A perfectly valid signature from an actor this node does not know:
        // db/020 refuses with a bare RAISE ("signer % is not an enrolled,
        // non-revoked actor"), which is the residual-refusal class ADR-0056
        // decision 5 governs — NOT a verification failure.
        let (sk_x, kid_x) = cairn_event::generate_key().unwrap();

        let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let refused = peer_note(&sk_x, &kid_x, WALL_2026 + 1_500);
        let e2 = peer_note(&sk, &kid, WALL_2026 + 2_000);
        let raw = response_json(&[&e1, &refused, &e2], Some(CTX_EVENT.as_str()));

        let addr = serve_canned(raw.clone(), 2);
        let (msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
        assert_eq!(
            m["applied_new"], 2,
            "one author's refusal must not withhold another author's events"
        );
        assert_eq!(m["refused_verifiable"], 1, "the door refusal is counted");
        assert_eq!(
            m["skipped_unverifiable"], 0,
            "these bytes VERIFY — they are not the unverifiable class"
        );
        assert_eq!(
            m["watermark_frozen"], false,
            "penned and pinned, not frozen"
        );
        assert!(
            msg.contains("floor-refused"),
            "the loud message must name the refusal class, got: {msg}"
        );

        // The durable record: verbatim bytes, keyed by digest, with the door's
        // own words as the reason a human will read in `cairn-sync quarantine`.
        let rows = quarantine_rows(&mut c);
        assert_eq!(rows.len(), 1, "exactly the refused event is penned");
        assert!(
            rows[0].reason.contains("not an enrolled"),
            "the door's reason must survive into the pen, got: {}",
            rows[0].reason
        );
        let held: Vec<u8> = c
            .query_one("SELECT signed_bytes FROM sync_quarantine", &[])
            .unwrap()
            .get(0);
        assert_eq!(held, refused, "the pen holds the VERBATIM signed bytes");
        assert_eq!(cursor(&mut c, "peer-a"), 3, "the cursor advanced past it");
        assert_eq!(
            floor(&mut c, "peer-a"),
            Some(2),
            "…but the floor pins the refused slot, so it stays on the wire"
        );

        // Repair the cause (the ceremony a real second site performs): enroll the
        // author. The floor re-offers the same bytes, they apply, and the pen row
        // — now resolved — is released. Unverifiable rows can never reach this
        // path (their bytes never apply), so the forensic trace is untouched.
        c.execute(
            "SELECT enroll_actor('agent', '{\"model\":\"late-enrolled\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
            &[&kid_x],
        )
        .unwrap();
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
            .expect("the repaired cycle is clean and must not fail");
        assert_eq!(m["applied_new"], 1, "delayed, never lost");
        assert_eq!(m["refused_verifiable"], 0);
        assert_eq!(
            floor(&mut c, "peer-a"),
            None,
            "a clean cycle clears the floor"
        );
        assert_eq!(
            quarantine_rows(&mut c).len(),
            0,
            "a penned event that now applies auto-releases (no stale duplicate of event_log)"
        );
    }

    /// The OTHER half of the ADR-0056 decision 5 routing, end-to-end: a VERIFIABLE
    /// event whose apply fails for a NON-refusal reason must FREEZE the cursor, pen
    /// NOTHING, and still fail loudly (#270). Penning here would record a refusal
    /// that never happened; skipping would lose the event. Only the pure predicate
    /// covered this, so nothing pinned that `do_pull` actually wires it that way.
    ///
    /// The transient fault is induced by swapping the apply door for one that raises
    /// with SQLSTATE `40001` (serialization failure) — the cheapest DETERMINISTIC way
    /// to get a real non-`P0001` database error, and a class that genuinely occurs.
    /// `locked_client` re-applies the whole SCHEMA at the start of every DB-gated
    /// test, so the override cannot leak into another test even if this one panics;
    /// the restore at the end is for the reader, not load-bearing cleanup.
    #[test]
    fn a_transient_apply_failure_freezes_and_pens_nothing() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        // A perfectly good event from an enrolled author: the ONLY reason it will
        // not apply is the induced fault, so the freeze cannot be misattributed.
        let good = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let raw = response_json(&[&good], Some(CTX_EVENT.as_str()));
        let addr = serve_canned(raw, 1);

        // Same signature (Postgres forbids renaming parameters on REPLACE), a
        // non-P0001 SQLSTATE, and no message resembling a floor verdict.
        c.batch_execute(
            "CREATE OR REPLACE FUNCTION apply_remote_event(
                 p_signed       BYTEA,
                 p_attestation  BYTEA DEFAULT NULL,
                 p_attester_key BYTEA DEFAULT NULL,
                 p_dek          BYTEA DEFAULT NULL
             ) RETURNS UUID LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'could not serialize access due to concurrent update'
                       USING ERRCODE = '40001';
             END $$;",
        )
        .unwrap();

        // The error object is kept (rather than going through `pull_integrity_err`)
        // because its CLASS is half of what this test now pins — see the assertion below
        // the metrics — and the canned serve answers exactly once.
        let boxed = do_pull(&mut c, &TcpTransport::new(&addr), "peer-t", false, None)
            .expect_err("a non-refusal apply failure freezes the cycle loudly");
        let ie = boxed
            .downcast_ref::<PullIntegrityError>()
            .unwrap_or_else(|| panic!("pull must fail as an INTEGRITY error: {boxed}"));
        let (msg, m) = (ie.message.clone(), ie.metrics.clone());
        assert_eq!(
            m["watermark_frozen"], true,
            "a non-refusal failure freezes: the same bytes may apply next cycle"
        );
        assert_eq!(m["applied_new"], 0);
        assert_eq!(
            m["refused_verifiable"], 0,
            "the door passed no verdict — this is not the refusal class"
        );
        assert_eq!(m["skipped_unverifiable"], 0, "the bytes verify");
        assert_eq!(
            quarantine_rows(&mut c).len(),
            0,
            "penning here would record a refusal that never happened"
        );
        assert_eq!(
            cursor(&mut c, "peer-t"),
            0,
            "the cursor holds at the contiguous applied prefix, so the event is re-offered"
        );
        assert_eq!(
            floor(&mut c, "peer-t"),
            None,
            "nothing penned, nothing pinned"
        );
        // The message must not describe a pen it did not write (the clause-assembly
        // defect the `loud_pull_message` unit tests guard at value level).
        assert!(
            !msg.contains("preserved verbatim") && !msg.contains("cairn-sync quarantine"),
            "a freeze-only cycle must not send the operator to an empty pen, got: {msg}"
        );
        assert!(
            msg.contains("FROZEN at 0"),
            "it must name the halted slot, got: {msg}"
        );

        // **PR #493 review — the half this test used to leave unasserted.** `40001` is a
        // serialization failure on THIS NODE'S database. Before the fix the cycle reached
        // `bet_a.py` as `integrity` alone — *the peer answered and its DATA is the
        // problem* — so an operator was sent to audit a peer's signatures for a lock
        // storm here, the local-fault rate under-counted, and (because the A4 filter is
        // `not r.get("local_fault")`) the blocked write's elapsed_ms was folded into the
        // pull-latency percentiles. That is issue #489 part 1's own sentence, one match
        // arm above the pen write it was filed against.
        let (classes, _) = classify_pull_failure(boxed.as_ref());
        assert!(
            classes.contains(&"local_fault"),
            "the apply failed on this node's own database: {classes:?} — {msg}"
        );
        assert!(
            classes.contains(&"integrity"),
            "…and the peer's event is still not held here, which needs a human: \
             {classes:?} — {msg}"
        );
        // The freeze LINE itself — the most consequential line in the loop, because the
        // cursor halts here — goes to stderr rather than into `msg`, so it is pinned by
        // SHAPE in `cairn-node/tests/db_errors_stay_legible.rs` instead: `{e}` there
        // rendered `ApplyError`'s `Display`, which deliberately omits the SQLSTATE, and
        // `40001` retries by itself where `53100` does not.

        // Restore the real door so a reader of the next test is not misled.
        for (name, sql) in SCHEMA {
            if name.starts_with("020") {
                c.batch_execute(sql).unwrap();
            }
        }
    }

    /// **The mirror image, and it is what keeps the fix above honest.** A non-`P0001`
    /// failure is not automatically this node's fault: a cast on a peer-supplied field
    /// (`22P02`), a constraint violation, an `XX000` from a function fed adversarial bytes
    /// are all the BYTES' doing and deterministic with it.
    ///
    /// Such a cycle still freezes — the event must not be skipped — and it is still
    /// `integrity`, because the peer offered something this node does not hold. What it
    /// must NOT pick up is `local_fault`: charging this node's uptime for a peer's
    /// malformed field is the mirror of the defect being fixed. (PR #493 review.)
    #[test]
    fn an_apply_failure_the_bytes_earned_is_not_this_nodes_fault() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let good = peer_note(&sk, &kid, WALL_2026 + 1_000);
        let addr = serve_canned(response_json(&[&good], Some(CTX_EVENT.as_str())), 1);

        // `22P02` is what `(b -> 'hlc' ->> 'wall')::bigint` raises on a non-numeric field
        // — a real shape for db/020, and one no operator can fix by mending a disk.
        c.batch_execute(
            "CREATE OR REPLACE FUNCTION apply_remote_event(
                 p_signed       BYTEA,
                 p_attestation  BYTEA DEFAULT NULL,
                 p_attester_key BYTEA DEFAULT NULL,
                 p_dek          BYTEA DEFAULT NULL
             ) RETURNS UUID LANGUAGE plpgsql AS $$
             BEGIN
                 RAISE EXCEPTION 'invalid input syntax for type bigint: \"not-a-wall\"'
                       USING ERRCODE = '22P02';
             END $$;",
        )
        .unwrap();

        let boxed = do_pull(&mut c, &TcpTransport::new(&addr), "peer-b", false, None)
            .expect_err("a non-refusal apply failure freezes the cycle loudly");
        let (classes, _) = classify_pull_failure(boxed.as_ref());

        for (name, sql) in SCHEMA {
            if name.starts_with("020") {
                c.batch_execute(sql).unwrap();
            }
        }

        assert_eq!(
            classes,
            &["integrity"],
            "a cast failing on the PEER'S field is not this node's database breaking; \
             charging our uptime for it is the mirror of the defect: {boxed}"
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
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
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
        assert!(do_pull(
            &mut c,
            &TcpTransport::new(&addr),
            "peer-legacy",
            false,
            None
        )
        .is_err());
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
        let m = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None).unwrap();
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
            let err = do_pull(&mut c, &TcpTransport::new(&addr), "peer-a", false, None)
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
        let boxed =
            do_pull(&mut c, &TcpTransport::new(&addr), "peer-old", false, None).unwrap_err();
        let err = boxed.to_string();
        assert!(
            err.contains("db/036"),
            "must name the likely pre-#196 peer and the remedy, got: {err}"
        );
        // **The mirror-image guard (PR #493 review).** Three sites in `do_pull` moved from
        // `partition` to `integrity` in this PR, and a fourth recogniser (an over-cap frame
        // prefix) was added. Nothing end to end pinned that the remaining partitions did
        // NOT move with them — and this is the case most at risk of a well-meaning
        // widening, because the peer really did accept the connection and read the request.
        // It answered with nothing, so it is still the link's word: `UnexpectedEof`, never
        // `InvalidData`. Under-counting real downtime is this fix's own mirror defect.
        let (classes, _) = classify_pull_failure(boxed.as_ref());
        assert_eq!(
            classes,
            &["partition"],
            "a peer that hangs up without answering is an outage, not a peer-data \
             problem: {err}"
        );
        // **The cause is now REACHABLE, so it can also be printed TWICE.** Keeping the
        // transport error in the chain (so an over-cap frame can be classified at all)
        // means `operator_chain` walks into it — and it drops a layer only when the layer
        // above ENDS WITH that layer's rendering. Put `{e}` mid-sentence, as the first
        // draft of `PeerRequestError` did, and the same error is named twice: the
        // double-render the sweep's own tail had to fix one file over. Driven here rather
        // than over a fixture, because the thing at risk is the PRODUCTION message's shape.
        let cause = boxed
            .source()
            .map(|c| c.to_string())
            .expect("the transport error must stay reachable");
        let line = operator_chain(boxed.as_ref());
        assert_eq!(
            line.matches(cause.as_str()).count(),
            1,
            "the transport failure must be named ONCE — `{cause}` in: {line}"
        );
        assert_eq!(cursor(&mut c, "peer-old"), 0, "cursor untouched");
    }

    /// **PR #493 review — the two planes gave one failure two operator words.**
    ///
    /// A response whose length prefix exceeds [`MAX_FRAME_BYTES`] is refused by
    /// [`read_frame`] BEFORE it allocates (issue #212's rule 1), as `InvalidData`. On the
    /// node plane that is `PullFailureClass::Integrity`, with its own test. Here it was
    /// flattened into a `String` by the request site's `format!` and fell to `partition` —
    /// so the same hostile or incompatible peer read as link downtime on one plane and as
    /// a peer problem on the other, which is what issue #482 was filed to end.
    ///
    /// The flattening was the worse half: with no `source()` the classifier could never be
    /// TAUGHT to recognise it. Driven through the real `do_pull` and the real
    /// `TcpTransport::request` retry loop, so a revert to `format!` at that site is
    /// caught here.
    #[test]
    fn an_oversized_response_frame_is_the_peer_not_the_link() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        // Prefix ONLY, and deliberately no payload: the puller must refuse on the number
        // alone. Sending 4 GiB of bytes to prove that would be the opposite of the point.
        let over_cap = u32::MAX.to_be_bytes().to_vec();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || {
            // `TcpTransport::request` retries four times before giving up; answer every
            // attempt, or the last one becomes an ordinary connection failure and the
            // test proves nothing.
            for _ in 0..4 {
                let Ok((mut s, _)) = listener.accept() else {
                    break;
                };
                let _ = read_frame(&mut s);
                let _ = s.write_all(&over_cap);
                let _ = s.flush();
            }
        });

        let boxed = do_pull(&mut c, &TcpTransport::new(&addr), "peer-fat", false, None)
            .expect_err("a length prefix over the cap is refused before allocating");
        let (classes, _) = classify_pull_failure(boxed.as_ref());
        assert_eq!(
            classes,
            &["integrity"],
            "the peer ANSWERED with a frame this build refuses; the WAN is the wrong \
             place to send an operator, and bet_a.py would charge the outage figure \
             for it: {boxed}"
        );
        assert_eq!(cursor(&mut c, "peer-fat"), 0, "cursor untouched");
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
        // The error object itself is kept (rather than going through
        // `pull_integrity_err`) because its CLASS is half of what this test now pins, and
        // the canned serve answers exactly once.
        let boxed = do_pull(&mut c, &TcpTransport::new(&addr), "peer-flood", false, None)
            .expect_err("a pen at its quota freezes the cycle loudly");
        let ie = boxed
            .downcast_ref::<PullIntegrityError>()
            .unwrap_or_else(|| panic!("pull must fail as an INTEGRITY error: {boxed}"));
        let (err, m) = (ie.message.clone(), ie.metrics.clone());
        assert!(err.contains("quota"), "error names the quota, got: {err}");
        assert_eq!(
            m["watermark_frozen"], true,
            "over quota = freeze, never skip"
        );
        // #489 part 1, the OTHER direction. A quota refusal is the pen mechanism working
        // as designed against a peer's garbage — a fact about the PEER — so it must not
        // pick up the `local_fault` class the same code path now sets when this node's
        // own database refuses the write. Getting this wrong would charge this node's
        // uptime for a peer flooding the pen, which is the mirror image of the defect.
        let (classes, _) = classify_pull_failure(boxed.as_ref());
        assert_eq!(
            classes,
            &["integrity"],
            "a quota refusal is the peer's doing, not this node's database: {err}"
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
        //
        // RENAME, NOT DROP (#296). A drop-then-re-add left the shared test database
        // permanently reordered: the migrations re-add with `ADD COLUMN IF NOT EXISTS`,
        // which appends at the END of the attribute list, so `seq` came back AFTER
        // db/040's `clock_grade` and every later positional `ROW(...)::event_log`
        // construction in any crate silently bound the wrong value into the wrong column.
        // That is the whole cause of the long-carried "recreate the test databases"
        // gotcha. A rename is invisible to the probe in exactly the same way (no column
        // named `seq` exists) but preserves the column's position, its data, and its
        // dependent objects, so renaming back is an EXACT restore.
        c.batch_execute("ALTER TABLE event_log RENAME COLUMN seq TO seq_pre036_probe")
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

        // Restore for whatever suite runs next under the shared lock — exactly, in place.
        c.batch_execute("ALTER TABLE event_log RENAME COLUMN seq_pre036_probe TO seq")
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

    /// Issue #216 (ADR-0058) Task 6: `emit_event` performs its OWN direct `INSERT INTO
    /// event_log` (it never routes through the apply_remote_event door a peer's pull
    /// uses), so Task 1 minting `clock_grade: SelfAsserted` into every authored
    /// `EventBody` is not enough on its own — the direct INSERT's explicit column list
    /// must also carry the column, or the author's own row silently stores the table's
    /// `'unknown'` DEFAULT while every peer that later pulls the same signed event
    /// (through db/020, which DOES read the body's grade) stores `'self-asserted'` —
    /// a cross-node metadata inconsistency for one and the same event.
    #[test]
    fn emit_event_stores_self_asserted_clock_grade_on_its_own_row() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let patient_id = uuid::Uuid::now_v7().to_string();

        let body = emit_event(
            &mut c,
            "test-node",
            &sk,
            &kid,
            "note.added",
            &patient_id,
            "note/1",
            serde_json::json!({"text": "grade check"}),
            None,
        )
        .expect("emit_event authors and stores the event");

        let grade: String = c
            .query_one(
                "SELECT clock_grade FROM event_log WHERE event_id = $1::text::uuid",
                &[&body.event_id],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            grade, "self-asserted",
            "the author's own row must store the minted grade, not the 'unknown' default"
        );
    }

    /// PR #285 review finding 2: `emit_event`'s direct INSERT bypasses both doors, so
    /// the grade-gated ceiling classify (db/005 step 1b' / db/020) never ran for
    /// locally-authored events — a forward-dated `t_effective` authored here got NO
    /// advisory `t_effective_ceiling_flag` row on the author's own node, while every
    /// peer that pulls the same signed event through db/020 records one. That is the
    /// same class of cross-node metadata inconsistency Task 6 fixed for the
    /// `clock_grade` column, reproduced for the flag ledger. `emit_event` must classify
    /// and flag exactly as the doors do.
    #[test]
    fn emit_event_flags_its_own_forward_dated_t_effective() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut c = locked_client(&base);
        let (sk, kid) = cairn_event::generate_key().unwrap();
        let patient_id = uuid::Uuid::now_v7().to_string();

        // Forward-dated: far after the HLC wall emit_event mints (~now) → one flag row.
        let forward = emit_event(
            &mut c,
            "test-node",
            &sk,
            &kid,
            "note.added",
            &patient_id,
            "note/1",
            serde_json::json!({"text": "forward flag check"}),
            Some("2099-01-01T00:00:00Z".to_string()),
        )
        .expect("emit_event authors and stores the forward-dated event");
        let flags: i64 = c
            .query_one(
                "SELECT count(*) FROM t_effective_ceiling_flag f \
                   JOIN event_log e USING (content_address) \
                  WHERE e.event_id = $1::text::uuid",
                &[&forward.event_id],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            flags, 1,
            "the author's own node must record the advisory ceiling flag its peers will"
        );

        // Backdated: the everyday legitimate case → clean, no flag.
        let past = emit_event(
            &mut c,
            "test-node",
            &sk,
            &kid,
            "note.added",
            &patient_id,
            "note/1",
            serde_json::json!({"text": "backdate no-flag check"}),
            Some("2001-01-01T00:00:00Z".to_string()),
        )
        .expect("emit_event authors and stores the backdated event");
        let flags: i64 = c
            .query_one(
                "SELECT count(*) FROM t_effective_ceiling_flag f \
                   JOIN event_log e USING (content_address) \
                  WHERE e.event_id = $1::text::uuid",
                &[&past.event_id],
            )
            .unwrap()
            .get(0);
        assert_eq!(flags, 0, "a backdated t_effective must not be flagged");
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
/// a database that cairn-node's FULL schema loader has already visited, so the gap
/// is structurally invisible there: this test is the drift guard. It wipes the
/// second test database (`$CAIRN_TEST_PG2`), loads ONLY `SCHEMA`, and drives every
/// SQL entry point the subset ships — the two the 2026-07-15 review found dangling:
///
///   * `submit_event` → `cairn_learn_attachment_refs` (db/027) — unconditional on
///     EVERY submit, exercised end-to-end here with a by-reference attachment whose
///     lazy blob reference must land in `blob_store`. Since #460 the remote door reaches
///     `cairn_learn_attachment_refs_lenient` (db/050) instead, so this test also covers
///     db/050's presence in the subset: door 2 below resolves that name or raises 42883;
///   * the db/002 `patient_chart` trigger → `cairn_hlc_triple_collision` +
///     `cairn_record_hlc_collision` (db/029) — parsed on the first
///     `patient.amended`, EXECUTED here by applying a genuine Byzantine pair (two
///     different signed bodies under one HLC triple) through `apply_remote_event`;
///
/// and the two db/006 recall-ceremony doors (`recall_event`,
/// `events_by_actor_epoch`), which were loaded-but-undriven in the first cut of
/// this test — the exact shape the late-binding trap needs (PR #222 review);
/// and db/048's sensitivity read model (`cairn_event_thread`,
/// `cairn_effective_sensitivity`, `cairn_thread_patient`), which was the same
/// shape again — in the SCHEMA list, confirmed to CREATE, and never once CALLED
/// on the medication-less schema its `to_regclass` guards exist for (#386).
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

    /// Build + sign one `identity.registration.asserted` (§5.3 standard class, an empty
    /// displayed set — the normal case for a genuinely new patient). Needed because the
    /// precedence rule (#345) makes registration the only legal FIRST event on a chart, so
    /// every chart these tests write to has to be opened with one of these first.
    fn signed_registration(
        sk: &SigningKey,
        kid: &str,
        patient_id: &str,
        wall: i64,
        origin: &str,
    ) -> Vec<u8> {
        let tokens = [patient_id.to_string()];
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
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: patient_id.to_string(),
            event_type: cairn_event::registration::REGISTRATION_EVENT_TYPE.into(),
            schema_version: cairn_event::registration::REGISTRATION_SCHEMA_VERSION.into(),
            hlc: Hlc {
                wall,
                counter: 0,
                node_origin: origin.into(),
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
        sign(&body, sk).unwrap().signed_bytes.to_vec()
    }

    /// Build + sign one `patient.amended` event (`patient.created` was retired by #345;
    /// the demographic-overlay projection it drives is unchanged). The body mirrors what
    /// `emit_event` authors (payload name/dob/sex is what the db/002 projection reads), with
    /// the HLC triple caller-chosen so the Byzantine-collision case can be constructed.
    fn signed_patient_amended(
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
            event_type: "patient.amended".into(),
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
            clock_grade: ClockGrade::SelfAsserted,
            safety: None,
        };
        // ADR-0039: author the twin into the signed body, as every production author does.
        let body = materialise_generic_twin(body);
        sign(&body, sk).unwrap().signed_bytes
    }

    /// Build + sign one event of a type this schema generation does NOT know
    /// (`observation.vital.recorded`) — the ADR-0056 admit-uninterpreted case.
    ///
    /// Needed by Door 5 for a reason worth stating precisely, because getting it slightly
    /// wrong is how the first version of that fixture came out vacuous.
    ///
    /// The order inside `cairn_event_thread` (db/048) is: the two `to_regclass` probes FIRST,
    /// then the `event_log` lookup, then §10b's thread-free type gate, and only then the
    /// five-table UNION. So on a subset node every event — `patient.`, `identity.`, all of
    /// them — reaches the probes and is stopped THERE; the type gate is what is never reached.
    /// (The first cut of this comment had that backwards, and a maintainer trusting it would
    /// conclude that moving the probes after the gate is free. It is not — #456 review.)
    ///
    /// The consequence is what matters, and it survives the correction: once the stub
    /// `medication_statement` below makes probe 1 pass, a thread-FREE event falls through to
    /// the type gate, which returns NULL before the UNION — so deleting probe 2 would still
    /// look green. Only a type OUTSIDE §10b's list survives the gate and reaches the UNION
    /// that raises. That is what this fixture supplies.
    ///
    /// Not `clinical.%`: ADR-0052 makes those born-sealed, and a plaintext clinical body is a
    /// shape that should not exist. An unknown *non*-clinical type is the honest fixture — a
    /// peer one schema generation ahead is precisely who sends one, and admitting it
    /// uninterpreted is the ADR-0056 floor working as designed.
    fn signed_uninterpreted(
        sk: &SigningKey,
        kid: &str,
        patient_id: &str,
        wall: i64,
        origin: &str,
    ) -> Vec<u8> {
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: patient_id.to_string(),
            event_type: "observation.vital.recorded".into(),
            schema_version: "observation/1".into(),
            hlc: Hlc {
                wall,
                counter: 0,
                node_origin: origin.into(),
            },
            t_effective: None,
            signer_key_id: kid.into(),
            contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
            payload: serde_json::json!({"observation": "from a peer we cannot interpret"}),
            attachments: vec![],
            plaintext_twin: None,
            clock_grade: ClockGrade::SelfAsserted,
            safety: None,
        };
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
        // taken on CAIRN_TEST_PG specifically — PostgreSQL advisory locks are scoped PER
        // DATABASE, so a guard taken on any OTHER database, CAIRN_TEST_PG2 included, would
        // not serialize against these suites at all. This comment used to say the lock
        // serializes "cluster-wide regardless of which database each suite touches", which
        // is the wrong mechanism for the right practice; #476 tracks the rest of the
        // copies). Held until `lock` drops at test end.
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
        //
        // #345: the chart is registered first — the subset carries db/005's precedence rule, so
        // this local write would otherwise be refused as a first event on a chart nobody made.
        // That dependency is precisely why db/045 + db/047 joined this subset (see SCHEMA).
        {
            let reg = signed_registration(&sk, &kid, &patient_id, now_ms(), "subset-local");
            c.query_one("SELECT submit_event($1)::text", &[&reg])
                .expect("the subset alone must be able to register a chart");
        }
        let photo_bytes = b"subset-test-attachment-bytes";
        let local = signed_patient_amended(
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
        // apply_remote_event PERFORMs cairn_learn_attachment_refs_lenient (db/050) — the
        // LENIENT half of the #460 asymmetry, not db/027's strict learner — and
        // with a current demographic winner standing, the db/002 trigger now parses
        // AND executes the db/029 collision predicate (FOUND = true, collision false).
        let wall_b = now_ms() + 1;
        let remote = signed_patient_amended(
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
        let byzantine = signed_patient_amended(
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

        // The contamination-cascade query: one enrollment of (key, epoch 'e') preceded every
        // admission, so all of them are selected and stamped 'pinned'. FOUR, not three: the
        // chart's REGISTRATION (#345) is an event this same key authored, and a cascade over a
        // recalled actor must reach the charts it created, not only the content it wrote.
        let cascade: i64 = c
            .query_one(
                "SELECT count(*) FROM events_by_actor_epoch($1, 'e') WHERE attribution = 'pinned'",
                &[&kid],
            )
            .expect("events_by_actor_epoch must succeed against the subset alone")
            .get(0);
        assert_eq!(
            cascade, 4,
            "the cascade must select every event this key/epoch authored"
        );

        // ── Door 4: db/049's safety read model — CALL it, don't just load it (#386) ───
        // #386 recorded that db/048's subset coverage LOADED that migration into SCHEMA but
        // never DROVE it, so the guarantee it appeared to give was untested at runtime. Door 5
        // below closes that half; this one is the same trap for the NEXT migration.
        // db/049 sits in the exact same trap: it is IN the SCHEMA list above, and Door 1's
        // submit_event calls already exercise cairn_check_safety_signal in passing (db/005
        // §1d PERFORMs it unconditionally on every LOCAL write) — but nothing so far has
        // called a db/049 function and asserted on ITS OWN return value, which is the
        // #386 gap restated for this migration. A subset node writes event_log.safety on
        // every clinical apply (submit_event stores `b -> 'safety'` verbatim, present or
        // not), so the read-model ladder that column feeds must resolve here too, not
        // merely exist upstream where cairn-node's full schema loader has always run.
        let rung: String = c
            .query_one(
                "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered'))",
                &[],
            )
            .expect("db/049's read-model ladder must be callable against the subset alone")
            .get(0);
        assert_eq!(
            rung, "existence",
            "a sequestered grade must coarsen to the least-disclosing rung"
        );

        // ── Door 5: db/048's sensitivity read model — the OTHER half of #386 ─────────
        // db/048 was in the SCHEMA list above and the file was confirmed to CREATE, which is
        // the half that matters for cairn_event_thread's `LANGUAGE plpgsql, NOT sql` decision.
        // Nothing ever CALLED it here. PL/pgSQL binds at first EXECUTION, so until now the
        // `to_regclass(...) RETURN NULL` guard bodies could have been deleted (keeping
        // plpgsql) and the whole workspace would have stayed green — the first client read
        // against a cairn-sync-shaped node would then raise `relation "medication_statement"
        // does not exist`. That is the #198/#227 first-write outage class, on the one file
        // whose comments lean hardest on being guarded against it.
        //
        // A subset node holds NO medication projections, so every one of these must answer the
        // honest "I cannot resolve a thread here", which is the same state as holding no
        // custody: section 11's conservative bound then applies, and the direction is safe.
        let thread: Option<String> = c
            .query_one(
                "SELECT cairn_event_thread($1::text::uuid)::text",
                &[&local_event_id],
            )
            .expect("cairn_event_thread must be CALLABLE against the subset alone (#386)")
            .get(0);
        assert_eq!(
            thread, None,
            "a node with no medication projections cannot resolve a thread — it must answer \
             NULL, not raise"
        );

        let grade: String = c
            .query_one(
                "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
                &[&local_event_id],
            )
            .expect("cairn_effective_sensitivity must be callable against the subset alone")
            .get(0);
        assert_eq!(
            grade, "routine",
            "absence of every assertion reads as routine, never as unknown (ADR-0062)"
        );

        let unknown_thread = uuid::Uuid::now_v7().to_string();
        let thread_patient: Option<String> = c
            .query_one(
                "SELECT cairn_thread_patient($1::text::uuid)::text",
                &[&unknown_thread],
            )
            .expect("cairn_thread_patient must be callable against the subset alone")
            .get(0);
        assert_eq!(thread_patient, None);

        // A THREAD-BEARING event type, admitted uninterpreted (ADR-0056). Every event above
        // carries a §10b thread-FREE prefix (`patient.`, `identity.`), and once the stub below
        // makes the FIRST to_regclass probe pass, such an event is absorbed by the type gate —
        // which sits AFTER both probes — and returns NULL before reaching the UNION that would
        // raise. That is why the window below needs a different event to be non-vacuous. A peer
        // one schema generation ahead sending a type this node cannot interpret is the
        // realistic source of one.
        let uninterpreted =
            signed_uninterpreted(&sk, &kid, &patient_id, now_ms() + 2, "subset-peer");
        c.query_one(
            "SELECT apply_remote_event($1, NULL, NULL)",
            &[&uninterpreted],
        )
        .expect("an uninterpreted type must be ADMITTED, not refused (ADR-0056)");
        let uninterpreted_id: String = c
            .query_one(
                "SELECT event_id::text FROM event_log WHERE event_type = 'observation.vital.recorded'",
                &[],
            )
            .unwrap()
            .get(0);
        let unknown_type_thread: Option<String> = c
            .query_one(
                "SELECT cairn_event_thread($1::text::uuid)::text",
                &[&uninterpreted_id],
            )
            .expect("a thread-bearing type must reach the probes without raising")
            .get(0);
        assert_eq!(
            unknown_type_thread, None,
            "no medication projections exist here, so no thread can resolve"
        );

        // ── The crash-mid-replay window: db/031 present, db/032 absent ────────────────
        // cairn_event_thread probes a representative table from EACH of db/031 and db/032,
        // because the loop that replays db/*.sql is not one atomic transaction — a loader that
        // crashes between them leaves medication_statement existing and medication_dose_event
        // not. NO SCHEMA IN THE TREE HAS THAT SHAPE (cairn-node loads both, cairn-sync
        // neither), so until now the second probe had never decided anything anywhere. This
        // database can model the window exactly, with a stub carrying only the columns the
        // reachable code paths touch.
        //
        // NON-VACUITY, which is the whole difficulty here: "returns NULL" is also what the
        // FIRST probe produces, so the assertion alone cannot tell which probe saved us. The
        // seeded row settles it — cairn_thread_patient has only ONE probe, so reading the
        // patient back out proves to_regclass now SEES medication_statement and the first probe
        // is passing. Only then does cairn_event_thread's NULL become attributable to the
        // second. Delete that second probe and this raises `relation "medication_cessation"
        // does not exist` — the UNION's SECOND branch, which is the first missing relation the
        // parser reaches; the mutation run in PR #456 reports exactly that string. (The first
        // cut of this comment named medication_dose_event, the branch the deleted PROBE
        // mentions, which is not the same thing.)
        // In a TRANSACTION, so the stub cannot outlive a failure (#456 review). `postgres`
        // is autocommit, so the first cut's trailing `DROP TABLE` only ran on the success
        // path: any panic in the window below committed a 3-column `medication_statement`
        // into the shared $CAIRN_TEST_PG2 database — and db/031 creates that table with
        // `IF NOT EXISTS`, so a schema reload does NOT heal it. The real table has 16
        // columns, so `patient_medication` then fails with `column s.term does not exist`
        // for every other suite sharing that database, attributed to the wrong test in a
        // different crate. `to_regclass` sees uncommitted DDL from the same session, so the
        // probes behave identically; and a panic drops the client, which rolls back — which
        // is strictly stronger than a DROP that only runs when nothing failed.
        c.batch_execute("BEGIN")
            .expect("open the transaction the stub lives inside");
        c.batch_execute(
            "CREATE TABLE medication_statement (
                 medication_id   uuid PRIMARY KEY,
                 patient_id      uuid NOT NULL,
                 content_address bytea)",
        )
        .expect("the db/031-without-db/032 window is modelled with a stub");
        c.execute(
            "INSERT INTO medication_statement (medication_id, patient_id)
             VALUES ($1::text::uuid, $2::text::uuid)",
            &[&unknown_thread, &patient_id],
        )
        .unwrap();

        let probed: Option<String> = c
            .query_one(
                "SELECT cairn_thread_patient($1::text::uuid)::text",
                &[&unknown_thread],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            probed.as_deref(),
            Some(patient_id.as_str()),
            "the stub must be VISIBLE to to_regclass, or the assertion below proves nothing"
        );

        let still_null: Option<String> = c
            .query_one(
                "SELECT cairn_event_thread($1::text::uuid)::text",
                &[&uninterpreted_id],
            )
            .expect(
                "with db/031's table present and db/032's absent, the SECOND to_regclass probe \
                 is the only thing standing between this call and a missing-relation error",
            )
            .get(0);
        assert_eq!(still_null, None);

        // Leave the database in the subset-only shape the rest of this test asserted. ROLLBACK
        // rather than DROP: it undoes the whole window, and it is what the backend would have
        // done for us had anything above panicked.
        c.batch_execute("ROLLBACK")
            .expect("the stub must not outlive the window it models");

        // Five admitted events total (registration + the three doors + the uninterpreted
        // observation Door 5 needs) — nothing quarantined, nothing lost.
        let events: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .unwrap()
            .get(0);
        assert_eq!(events, 5);
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

    /// PR #302 review finding F3. This crate embeds db/043 — and the comment beside that
    /// entry argues it MUST, because db/020 (also here) is the door that WRITES the
    /// event_deferred marker. The reasoning was right and only the function shipped: nothing
    /// in this binary called it, so a sync-only database — the phone-tier carrier node
    /// ADR-0056 exists for — accumulated markers that nothing could ever promote.
    #[test]
    fn load_schema_promotes_a_deferred_event() {
        let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut lock = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        lock.execute("SELECT pg_advisory_lock($1)", &[&0x4341524E_i64])
            .unwrap();
        let mut c = postgres::Client::connect(&base, postgres::NoTls).unwrap();
        load_schema(&mut c).expect("baseline replay must succeed");
        c.batch_execute("TRUNCATE event_log CASCADE").unwrap();

        // Hand-write a deferred row AND classify its type. The pass's query INNER JOINs
        // event_deferred to event_type_class (db/043) — a type with NO class row is SKIPPED
        // entirely, so the probe below would never even be reached and adjudication_error
        // would stay NULL forever, whether or not the loader calls the pass at all. The
        // INSERT INTO event_type_class below is therefore load-bearing, not incidental: it is
        // what makes the pass attempt this row, so its refusal (this crate cannot sign, so
        // signed_bytes never parses) lands in adjudication_error — the one column this test
        // can read without a real signature.
        c.batch_execute(
            "DO $$ DECLARE v_id uuid := uuidv7(); v_sb bytea; BEGIN \
               v_sb := ('sync-defer-' || v_id::text)::bytea; \
               INSERT INTO event_log (event_id, patient_id, event_type, schema_version, \
                 hlc_wall, hlc_counter, node_origin, signed_bytes, content_address, \
                 body, contributors, signer_key_id, plaintext_twin) \
               VALUES (v_id, v_id, 'sync.defer.probe', 'test-1', \
                 (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', v_sb, \
                 '\\x1220'::bytea || digest(v_sb, 'sha256'), \
                 '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe'); \
               INSERT INTO event_deferred (event_id, event_type) \
                 VALUES (v_id, 'sync.defer.probe'); \
               INSERT INTO event_type_class (event_type, mode, targets_other_author) \
                 VALUES ('sync.defer.probe', 'additive', FALSE) ON CONFLICT DO NOTHING; \
             END $$;",
        )
        .unwrap();

        load_schema(&mut c).expect("replay must succeed");

        // WHAT THIS ASSERTS, and why it is not "the event was promoted": this crate cannot
        // sign, so the row above carries unparseable `signed_bytes` — and the pass re-derives
        // every event's envelope from those bytes (`cairn_body`), refusing anything that no
        // longer parses. So the probe can never be PROMOTED here, by construction.
        //
        // It can, however, be ADJUDICATED, and that is exactly what F3 is about: whether this
        // loader invokes the pass at all. A pass that ran records its refusal in
        // `adjudication_error`; a loader that never calls it leaves the column NULL forever.
        // That is the discriminator, and it needs no signing. Promotion itself is already
        // pinned by cairn-node's suite, which can sign.
        let err: Option<String> = c
            .query_one(
                "SELECT adjudication_error FROM event_deferred WHERE event_type = $1",
                &[&"sync.defer.probe"],
            )
            .unwrap()
            .get(0);

        // Clean up BEFORE asserting, so a failure cannot strand the shared database — the
        // same discipline load_schema_stamps_the_generation_and_refuses_a_newer_db uses
        // above. `event_type_class` is load-bearing here: no migration knows this probe
        // type, so migration replay would never remove the row.
        c.batch_execute(
            "TRUNCATE event_log CASCADE; \
             DELETE FROM event_type_class WHERE event_type = 'sync.defer.probe'",
        )
        .unwrap();

        assert!(
            err.is_some(),
            "this loader must re-adjudicate: a sync-only node whose loader never calls the \
             pass accumulates admitted-but-permanently-powerless events with no mechanism to \
             notice (PR #302 finding F3)"
        );
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

/// DB-gated: the OTHER half of the #231 custody pin — the half that reads a real
/// database. Skips when `CAIRN_TEST_PG` is unset.
///
/// `decide_custody`'s tests are pure and cover the MESSAGE. These cover the mapping
/// from actual node-plane state to a [`TrustLookup`], and that is where the first
/// review of this code found the defect: the `RevokedPeer` arm was unreachable, so
/// revoking a compromised peer printed "this key is not among this node's admitted
/// peers … admit it out of band". Every pure test passed throughout, because a pure
/// test hand-feeds the enum it is meant to be checking the derivation of.
///
/// The node plane is authored through the REAL door (`identity::author_peer` /
/// `author_unpeer` → `submit_node_event`), never by hand-inserting `node_event` rows.
/// That is the whole point: a hand-built fixture would encode this test's assumption
/// about what the door writes, and the defect WAS a wrong assumption about what the
/// door writes (`peer.revoked` carries no `peer_pubkey`, so the revoke row stores NULL
/// there and replaces the `peer` row in the `DISTINCT ON` view).
#[cfg(test)]
mod trust_lookup_db_tests {
    use super::tests::derived_bytes;
    use super::*;
    use quarantine_tests::{cs, locked_client};

    /// A current-thread runtime, so these SYNC tests can drive `cairn-node`'s ASYNC
    /// identity API. Two connections to one database — the async one authors, the sync
    /// one is the very `postgres::Client` `serve_conn` hands to `look_up_peer_trust`.
    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime")
    }

    /// A distinct 32-byte node-id fixture, DERIVED not written (house rule 6). A node
    /// id is a content address, so any 32 hex-encoded bytes are well-formed as far as
    /// `submit_node_event` is concerned.
    fn derived_node_id(tag: u8) -> String {
        hex::encode(derived_bytes(tag))
    }

    /// The full node plane in one place: a locked SYNC client (the subject under test)
    /// plus an async handle for authoring. `local_node` starts EMPTY — each test
    /// provisions only as far as the state it is pinning.
    struct Plane {
        rt: tokio::runtime::Runtime,
        node_db: tokio_postgres::Client,
        sync_db: postgres::Client,
        sk: SigningKey,
        kid: String,
    }

    impl Plane {
        fn open(base: &str) -> Plane {
            // Take the shared advisory lock FIRST (locked_client does it), then
            // open the async connection unguarded — cairn-node's `test_serial_guard`
            // uses the same key, so guarding twice would deadlock against ourselves.
            let sync_db = locked_client(base);
            let rt = rt();
            let node_db = rt
                .block_on(cairn_node::db::connect_and_load_schema(base))
                .expect("node-plane schema");
            rt.block_on(cairn_node::db::reset_node_federation_tables(&node_db))
                .expect("reset the node plane");
            let (sk, kid) = cairn_event::generate_key().unwrap();
            Plane {
                rt,
                node_db,
                sync_db,
                sk,
                kid,
            }
        }

        /// Run `cairn-node init`'s in-DB half: mint this node's own identity.
        fn provision(&self) {
            self.rt
                .block_on(cairn_node::identity::provision(
                    &self.node_db,
                    &self.sk,
                    &self.kid,
                    "node-under-test",
                    "127.0.0.1:7900",
                ))
                .expect("provision");
        }

        /// Run the in-DB half of `cairn-node pair-accept` for one peer key.
        fn admit(&self, peer_node_id_hex: &str, peer_pubkey_hex: &str) {
            let me = self
                .rt
                .block_on(cairn_node::identity::load_local(&self.node_db))
                .expect("local identity");
            let bundle = cairn_event::PairingBundle {
                node_id_hex: peer_node_id_hex.into(),
                pubkey_hex: peer_pubkey_hex.into(),
                address: "127.0.0.1:7901".into(),
                fingerprint: cairn_event::short_fingerprint(peer_pubkey_hex).unwrap(),
                nonce: "n".into(),
                hlc: Hlc {
                    wall: 0,
                    counter: 0,
                    node_origin: peer_node_id_hex.into(),
                },
            };
            self.rt
                .block_on(cairn_node::identity::author_peer(
                    &self.node_db,
                    &self.sk,
                    &self.kid,
                    &me.node_id_hex,
                    &bundle,
                    Some("peer"),
                ))
                .expect("author peer.added");
        }

        /// Run the in-DB half of `cairn-node unpeer`.
        fn revoke(&self, peer_node_id_hex: &str) {
            let me = self
                .rt
                .block_on(cairn_node::identity::load_local(&self.node_db))
                .expect("local identity");
            self.rt
                .block_on(cairn_node::identity::author_unpeer(
                    &self.node_db,
                    &self.sk,
                    &self.kid,
                    &me.node_id_hex,
                    peer_node_id_hex,
                ))
                .expect("author peer.revoked");
        }

        fn look_up(&mut self, kid: &str) -> TrustLookup {
            look_up_peer_trust(&mut self.sync_db, kid)
        }
    }

    #[test]
    fn an_admitted_peer_reads_as_an_active_peer() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, peer_kid) = cairn_event::generate_key().unwrap();
        plane.admit(&derived_node_id(0x21), &peer_kid);
        assert_eq!(plane.look_up(&peer_kid), TrustLookup::ActivePeer);
    }

    #[test]
    fn a_revoked_peer_reads_as_revoked_not_as_a_stranger() {
        // THE regression test for the review defect. `peer.revoked` carries only
        // `peer_node_id_hex`, so db/007 stores a NULL `peer_pubkey` on the revoke row
        // and `trust_peer`'s `DISTINCT ON (subject_node_id) … ORDER BY hlc DESC` lets
        // that row REPLACE the `peer` row that held the key — the revoked kid is
        // simply absent from the view. A probe of `peer_pubkey = $1` therefore
        // answered "never seen it", and the operator response to a COMPROMISED peer
        // was an instruction to re-admit it. Recovering the fact needs the historical
        // `peer` row, which is what `look_up_peer_trust`'s second probe now joins to.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, peer_kid) = cairn_event::generate_key().unwrap();
        let peer_node = derived_node_id(0x22);
        plane.admit(&peer_node, &peer_kid);
        assert_eq!(
            plane.look_up(&peer_kid),
            TrustLookup::ActivePeer,
            "precondition: it must genuinely be an active peer BEFORE the revoke, or \
             this test could pass without the revoke doing anything"
        );

        plane.revoke(&peer_node);
        assert_eq!(
            plane.look_up(&peer_kid),
            TrustLookup::RevokedPeer,
            "a revoked peer must read as REVOKED. Reading it as NotAPeer erases the \
             act someone performed and tells the operator to re-pair the node they \
             deliberately cut off (principle 2: never erase, always overlay)"
        );
    }

    #[test]
    fn an_unknown_key_reads_as_not_a_peer_when_other_peers_exist() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, admitted) = cairn_event::generate_key().unwrap();
        plane.admit(&derived_node_id(0x23), &admitted);
        let (_sk2, stranger) = cairn_event::generate_key().unwrap();
        assert_eq!(plane.look_up(&stranger), TrustLookup::NotAPeer);
    }

    #[test]
    fn a_provisioned_node_with_no_peers_reads_as_no_peers_admitted() {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, kid) = cairn_event::generate_key().unwrap();
        assert_eq!(plane.look_up(&kid), TrustLookup::NoPeersAdmitted);
    }

    #[test]
    fn an_uninitialised_node_plane_is_distinguished_from_an_unpeered_one() {
        // The pair `trust_peer` alone cannot tell apart: the view filters on
        // `author_node_id = (SELECT node_id FROM local_node WHERE id)`, NULL when
        // `local_node` is empty, so it yields zero rows in BOTH states. Their first
        // operator command differs (`init` vs `pair-accept`), which is why the fourth
        // probe reads `local_node` directly.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        let (_sk, kid) = cairn_event::generate_key().unwrap();
        assert_eq!(
            plane.look_up(&kid),
            TrustLookup::NodePlaneUninitialised,
            "an empty local_node is `init` was never run here"
        );
        plane.provision();
        assert_eq!(
            plane.look_up(&kid),
            TrustLookup::NoPeersAdmitted,
            "…and once provisioned the SAME empty trust set means something else"
        );
    }

    #[test]
    fn a_database_without_the_node_plane_reads_as_node_plane_absent() {
        // The arm the `DELIBERATELY ABSENT: db/007` note at the top of this file rests
        // its whole argument on — a soft dependency on an unloaded migration is only
        // safe because its absence is an ANSWER. Untested, that argument was a claim.
        //
        // The shared test database HAS db/007 (cairn-node's loader put it there), so
        // absence is staged by dropping the view inside a transaction that is rolled
        // back. The failing SELECT aborts the transaction; ROLLBACK restores the view.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, kid) = cairn_event::generate_key().unwrap();

        plane
            .sync_db
            .batch_execute("BEGIN; DROP VIEW trust_peer;")
            .expect("stage a database with no node plane");
        let verdict = plane.look_up(&kid);
        plane
            .sync_db
            .batch_execute("ROLLBACK")
            .expect("restore the view");

        assert_eq!(
            verdict,
            TrustLookup::NodePlaneAbsent,
            "a missing `trust_peer` must map to its OWN arm: it is a provisioning fact \
             about this database, not a fault, and it names a different remedy than a \
             lookup that genuinely failed"
        );
        // The view really is back — otherwise this test would poison every later one.
        assert_eq!(plane.look_up(&kid), TrustLookup::NoPeersAdmitted);
    }

    #[test]
    fn a_lookup_that_fails_for_any_other_reason_reads_as_unknown_and_withholds() {
        // Fail-closed for the state where the honest answer is "we do not know"
        // (principle 4). Staged with a syntactically-valid but type-broken relation:
        // `trust_peer` replaced by one whose `peer_pubkey` cannot be compared to text.
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let mut plane = Plane::open(&base);
        plane.provision();
        let (_sk, kid) = cairn_event::generate_key().unwrap();

        plane
            .sync_db
            .batch_execute(
                "BEGIN;
                 DROP VIEW trust_peer;
                 CREATE VIEW trust_peer AS
                     SELECT NULL::bytea AS peer_node_id, NULL::int AS peer_pubkey,
                            NULL::text AS status;",
            )
            .expect("stage a broken trust_peer");
        let verdict = plane.look_up(&kid);
        plane.sync_db.batch_execute("ROLLBACK").expect("restore");

        assert_eq!(
            verdict,
            TrustLookup::LookupFailed,
            "an error that is NOT 'relation does not exist' must not be reported as a \
             missing node plane — the remedies differ"
        );
        assert!(
            matches!(
                decide_custody(&kid, derived_bytes(0x24), verdict),
                CustodyAdmission::Withhold { .. }
            ),
            "and an unknown admission must WITHHOLD: uncertainty can only ever \
             withhold custody, never confer it"
        );
        assert_eq!(plane.look_up(&kid), TrustLookup::NoPeersAdmitted);
    }
}
