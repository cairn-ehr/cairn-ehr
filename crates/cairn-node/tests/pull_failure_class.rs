//! #474 item 3 — a failed pull cycle must not call this node's own database a partition.
//!
//! `cairn-sync` learned this as issue #469: `run`'s catch-all sent an operator to the WAN
//! to look for a link fault while the link was healthy and a local `UPDATE` had failed,
//! and it charged the Bet A availability figure for it. `cairn-node` has the same
//! catch-all over the same kind of failure — `checkpointing sync cursor`, `counting
//! unacked node quarantine rows` and `auto-releasing a node quarantine row` are all THIS
//! node's database, and the first two are reached after the peer has already answered in
//! full. (Not all of them are: `reading sync cursor` and `reading the node quarantine
//! re-offer floor` run BEFORE the TCP connect, which is why the production doc hedges with
//! "usually" — PR #478 review, I4.)
//!
//! The classifier is pure, so the mapping is pinned here with no database and no peer.
//! That matters: the alternative is a test that can only be written by breaking a live
//! database mid-cycle, which is why the defect survived this long in `cairn-sync`.

use cairn_node::db_diagnosis::LocalDbFault;
use cairn_node::sync::{pull_failure_class, PeerIntegrityError, PullFailureClass};

/// A real `tokio_postgres::Error` with a live `source()`, built with no database and no
/// network — `Config`'s own parser produces one, which is the same trick
/// `db_diagnosis`'s unit tests use.
fn a_real_pg_error() -> tokio_postgres::Error {
    "host=localhost port=not-a-number"
        .parse::<tokio_postgres::Config>()
        .expect_err("a non-numeric port is not a parseable connection string")
}

/// The defect itself: a failed cursor checkpoint is THIS NODE'S DATABASE, with the peer and
/// the link both healthy — the pull had already streamed every event by the time it ran.
#[test]
fn a_failed_cursor_checkpoint_is_a_local_fault_not_a_partition() {
    let e = anyhow::Error::from(LocalDbFault::new(
        "checkpointing sync cursor",
        a_real_pg_error(),
    ));
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::LocalFault,
        "{e:#}"
    );
}

/// …and it survives a `.context()` layer added above it. A classifier that stopped at the
/// outermost error would revert to `partition` the moment anyone wrapped the call — which
/// is the caveat `cairn-sync`'s sibling carries, because `downcast_ref` on a `dyn Error`
/// does not walk the chain. This one does, and that difference must stay pinned.
#[test]
fn a_local_fault_survives_an_added_context_layer() {
    let e = anyhow::Error::from(LocalDbFault::new("reading sync cursor", a_real_pg_error()))
        .context("pull cycle 7");
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::LocalFault,
        "{e:#}"
    );
}

/// A bare `tokio_postgres::Error` with no wrapper at all is still local: every postgres
/// call on this path talks to THIS node's database, because the peer is reached over
/// TLS/TCP and never through a Postgres connection. (Not "never through libpq" — these
/// crates are pure-Rust protocol implementations and libpq is not in the tree at all.)
#[test]
fn a_bare_postgres_error_is_local() {
    let e = anyhow::Error::from(a_real_pg_error());
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::LocalFault,
        "{e:#}"
    );
}

/// The default arm, and it must stay LAST: a failure nobody claimed is, by elimination,
/// one where the peer did not answer. A refused socket during the handshake is the
/// canonical case, and it is a genuine partition.
#[test]
fn an_unrecognised_failure_is_a_partition() {
    let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
    let e = anyhow::Error::from(io).context("mTLS handshake (server pin)");
    assert_eq!(pull_failure_class(&e), PullFailureClass::Partition, "{e:#}");
}

/// The rendered line is the other half of the fix: classifying correctly and then printing
/// `db error` would trade one silent failure for another. The operator needs the class AND
/// the cause, or the line tells them where to look and nothing about what they will find.
#[test]
fn the_local_fault_line_carries_the_cause() {
    let e = anyhow::Error::from(LocalDbFault::new(
        "checkpointing sync cursor",
        a_real_pg_error(),
    ));
    let text = format!("{e}");
    // A statement of the acceptance criterion, NOT a mutation-killer: this fixture is a
    // `Kind::ConfigParse` error, whose `Display` is `invalid connection string`, so it
    // could never render `db error` whatever the code did. The two assertions below are
    // the ones that do the work (PR #478 review).
    assert_ne!(text, "db error", "{text}");
    assert!(
        text.contains("checkpointing sync cursor"),
        "the line must say what was being done: {text}"
    );
    assert!(
        text.contains("port"),
        "…and the cause must survive the wrapping: {text}"
    );
}

// ---------------------------------------------------------------------------
// Issue #482 — the third class: the peer ANSWERED, and its answer is the problem.
// ---------------------------------------------------------------------------
//
// `Partition` was simultaneously the default-by-elimination and a specific operator
// instruction ("go and look at the link"), so every failure the classifier did not
// recognise got that instruction — including failures where the peer demonstrably
// answered. The sharp case is an mTLS pin mismatch: a rotated or REVOKED peer key
// produced a log line indistinguishable from a satellite link being down, and on a
// WAN-sync project the availability figure is charged against `Partition`.
//
// The recogniser is `std::io::ErrorKind::InvalidData` anywhere in the chain, plus the
// typed `PeerIntegrityError` for the protocol checks that have no io::Error to carry.
// Both mean the same thing and neither can be produced by a link that went away:
// `ConnectionRefused`, `UnexpectedEof`, `ConnectionReset` and `TimedOut` are what a
// dead link looks like, and every one of them still classifies `Partition` below.

/// The defect's own sentence: a revoked or rotated peer key is NOT link downtime.
///
/// rustls surfaces a failed certificate check to `tokio_rustls` as an `io::Error` of kind
/// `InvalidData`, and `pull_into` adds `.context("mTLS handshake (server pin)")` over it.
/// The peer completed a TCP connect moments earlier, so the link is demonstrably up; what
/// failed is the peer's *identity*.
#[test]
fn an_mtls_pin_mismatch_is_the_peer_not_the_link() {
    let io = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid peer certificate: the presented key is not in the trust set",
    );
    let e = anyhow::Error::from(io).context("mTLS handshake (server pin)");
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::Integrity,
        "a peer whose key no longer pins has answered; the WAN is not the place to look: {e:#}"
    );
}

/// A response frame too short to carry its own 8-byte seq prefix. There is no `io::Error`
/// here at all — the bytes arrived fine and this node read them — so this is the case the
/// typed marker exists for.
#[test]
fn a_short_response_frame_is_the_peer_not_the_link() {
    let e = anyhow::Error::from(PeerIntegrityError::new(
        "pull: response frame shorter than the 8-byte seq prefix",
    ));
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::Integrity,
        "the peer answered with an unusable frame — its code, not the link: {e:#}"
    );
}

/// …and it survives a `.context()` layer, for the same reason the local-fault arm must.
#[test]
fn a_peer_integrity_failure_survives_an_added_context_layer() {
    let e = anyhow::Error::from(PeerIntegrityError::new("frame shorter than the prefix"))
        .context("reading a response frame");
    assert_eq!(pull_failure_class(&e), PullFailureClass::Integrity, "{e:#}");
}

/// An oversized length prefix: `read_frame` refuses it BEFORE allocating and returns
/// `InvalidData`. A peer serving a frame over `MAX_FRAME_BYTES` is running incompatible
/// code or is hostile; either way the link delivered the bytes faithfully.
#[test]
fn an_oversized_frame_prefix_is_the_peer_not_the_link() {
    let io = std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "frame length 999999999 exceeds the 8388608-byte cap",
    );
    let e = anyhow::Error::from(io).context("reading a response frame");
    assert_eq!(pull_failure_class(&e), PullFailureClass::Integrity, "{e:#}");
}

/// **The ordering guard, and it is the whole safety argument.** `LocalFault` is checked
/// FIRST, so a chain carrying both a `tokio_postgres::Error` and an `InvalidData` io error
/// is local — this node's own database reached over TLS is exactly that shape, and calling
/// it a peer problem would send an operator to a peer that never failed.
///
/// The fixture is synthetic (anyhow stores a `.context()` value in the chain, so an
/// `io::Error` used as context is reachable by `downcast_ref`); it pins the ORDER, not a
/// production error shape.
#[test]
fn a_local_fault_wins_over_a_peer_signal_in_the_same_chain() {
    let e = anyhow::Error::from(a_real_pg_error()).context(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "a TLS failure on the way to this node's OWN database",
    ));
    assert_eq!(
        pull_failure_class(&e),
        PullFailureClass::LocalFault,
        "a postgres error anywhere in the chain outranks every peer signal: {e:#}"
    );
}

/// The four io kinds a link that went away actually produces. None of them may reach the
/// new class — if one did, the availability figure would start UNDER-counting real
/// downtime, which is the mirror image of the defect being fixed.
#[test]
fn the_kinds_a_dead_link_produces_are_still_partitions() {
    for kind in [
        std::io::ErrorKind::ConnectionRefused,
        std::io::ErrorKind::ConnectionReset,
        std::io::ErrorKind::UnexpectedEof,
        std::io::ErrorKind::TimedOut,
    ] {
        let e = anyhow::Error::from(std::io::Error::new(kind, "the link is gone"))
            .context("reading a response frame");
        assert_eq!(
            pull_failure_class(&e),
            PullFailureClass::Partition,
            "{kind:?} is what a dead link looks like: {e:#}"
        );
    }
}
