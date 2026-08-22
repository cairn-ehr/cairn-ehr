//! A frozen cursor must not produce a line that reads like a healthy cycle.
//! (PR #478 review, finding 6.)
//!
//! `pull_into` has three freeze paths — the quarantine pen at quota, a pen WRITE that
//! failed, and a transient database fault while applying. All three `break` out of the
//! loop and then return `Ok(stats)`, which is correct: freezing is the deliberate,
//! availability-preserving choice (never advance past an unresolved refusal), and the
//! cycle itself did not fail.
//!
//! But `run`'s `Ok` arm printed an ordinary summary for them. A `53100` disk-full during
//! apply produced:
//!
//! ```text
//! run: pull 10.0.0.3:9443: full_sweep=false received=5 admitted=4 rejected=0 quarantined=0 pending=0
//! ```
//!
//! — indistinguishable from success, with `pending=0`, while the cursor sat frozen. The
//! new `LOCAL FAULT` / `PARTITION` classification cannot see any of it, because the cycle
//! returned `Ok`: a monitor keyed on those two tokens would watch a stuck node forever and
//! never fire. The only trace was a one-row arithmetic gap nothing stated.
//!
//! The line is composed by a **pure** function so the sentence an operator reads is pinned
//! here, with no database and no peer — the same reason #475 extracted `load_migration`
//! and #471 extracted `requeue_interrupted_message`.

use cairn_node::sync::{frozen_cursor_line, PullStats};

/// The freeze must name the seq it stopped at: that is the number an operator carries to
/// `cairn-node quarantine`, and the one that says whether the next cycle made progress.
#[test]
fn a_frozen_cursor_names_the_peer_and_the_seq() {
    let line = frozen_cursor_line("10.0.0.3:9443", 42);

    assert!(line.contains("10.0.0.3:9443"), "which peer: {line}");
    assert!(line.contains("42"), "…and where it stopped: {line}");
}

/// The word a monitor greps for. A frozen cursor is a stuck node, and the line has to say
/// so in a token that is not also produced by a healthy cycle.
#[test]
fn a_frozen_cursor_says_frozen() {
    let line = frozen_cursor_line("10.0.0.3:9443", 42);
    assert!(
        line.contains("FROZEN"),
        "a monitor needs one greppable token, as `LOCAL FAULT` and `PARTITION` are: {line}"
    );
}

/// The line must point at the reason, which was already printed by the freeze site itself
/// — the operator needs to know to look up, not to re-derive the cause from a seq number.
#[test]
fn a_frozen_cursor_points_at_the_reason_above_it() {
    let line = frozen_cursor_line("10.0.0.3:9443", 42);
    assert!(
        line.to_lowercase().contains("above"),
        "the cause is on the preceding line; say so rather than leaving it to be guessed: {line}"
    );
}

/// A healthy cycle carries no freeze, and `Default` is how every `PullStats` in this crate
/// is built — so the quiet case must be the default one.
#[test]
fn a_healthy_cycle_reports_no_freeze() {
    assert!(
        PullStats::default().frozen.is_none(),
        "silence is the default; a freeze is the exception that has to be set"
    );
}
