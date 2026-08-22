//! #474 item 3 — a failed pull cycle must not call this node's own database a partition.
//!
//! `cairn-sync` learned this as issue #469: `run`'s catch-all sent an operator to the WAN
//! to look for a link fault while the link was healthy and a local `UPDATE` had failed,
//! and it charged the Bet A availability figure for it. `cairn-node` has the same
//! catch-all over the same kind of failure — `checkpointing sync cursor`, `counting
//! unacked node quarantine rows`, `snapshotting active peer pubkeys` are all THIS node's
//! database, reached after the peer has already answered.
//!
//! The classifier is pure, so the mapping is pinned here with no database and no peer.
//! That matters: the alternative is a test that can only be written by breaking a live
//! database mid-cycle, which is why the defect survived this long in `cairn-sync`.

use cairn_node::db_diagnosis::LocalDbFault;
use cairn_node::sync::{pull_failure_class, PullFailureClass};

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
/// TLS/TCP and never through libpq.
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
