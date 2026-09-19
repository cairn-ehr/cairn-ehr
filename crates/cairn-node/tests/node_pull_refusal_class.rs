//! #621 — which failures belong to the EVENT and which belong to THIS NODE.
//!
//! `deterministic_apply_failure` is the whole of the node puller's new decision: a verifiable
//! event whose apply failed without a verdict is PENNED when the failure will recur identically
//! (a cast, a CHECK violation, an `XX000`) and FROZEN when it is this node's own trouble (a
//! deadlock, a timeout, a dropped connection). Pure, so it is pinned here with no database and no
//! peer — the pattern `frozen_cursor_is_loud.rs` set for the loop's other pure decisions.
//!
//! Two properties, and the second is the one that will be argued with later:
//!
//! 1. every class the loop must NOT pen is claimed explicitly;
//! 2. an UNRECOGNISED code is deterministic — it pens. That asymmetry is deliberate and is the
//!    opposite of the node plane's older instinct: a wrongly-penned valid event is delayed, held,
//!    re-offered and auto-released, while a wrongly-frozen link is stuck forever with no remedy.
//!    Flipping the default "to be safe" reinstates exactly the defect #621 reported.

use cairn_node::sync::deterministic_apply_failure;

/// The local classes and codes, each with the failure a reader should picture.
const LOCAL: [(&str, &str); 9] = [
    (
        "08006",
        "connection_failure — the link to our own database went away",
    ),
    ("40001", "serialization_failure — retry succeeds"),
    ("40P01", "deadlock_detected — retry succeeds"),
    (
        "42501",
        "insufficient_privilege — a grant this node is missing",
    ),
    ("53100", "disk_full"),
    ("55P03", "lock_not_available"),
    ("57014", "query_canceled — a statement timeout"),
    // The two exceptions inside class XX, claimed by full code (PR #627 review, finding 2). A
    // corrupt page or index on node_event is THIS machine's disk, and without these the puller
    // would pen a peer's whole log while blaming the peer in every durable row.
    ("XX001", "data_corrupted — a bad heap page under node_event"),
    (
        "XX002",
        "index_corrupted — a torn index, while the pen table stays healthy",
    ),
];

/// The deterministic ones: nothing about them improves by waiting.
const DETERMINISTIC: [(&str, &str); 5] = [
    (
        "22P02",
        "invalid_text_representation — a uuid cast on a peer-supplied field",
    ),
    ("23514", "check_violation — the HLC or role CHECK"),
    (
        "23502",
        "not_null_violation — a missing field reaching a NOT NULL column",
    ),
    ("23505", "unique_violation"),
    (
        "XX000",
        "internal_error — a pgrx function panicking on adversarial bytes, the case that must \
         never be able to wedge a link",
    ),
];

#[test]
fn a_failure_local_to_this_node_is_not_the_events_fault() {
    for (code, why) in LOCAL {
        assert!(
            !deterministic_apply_failure(Some(code)),
            "{code} ({why}) is this node's own trouble: the SAME bytes may well apply on the \
             next cycle, so the cursor must freeze and retry rather than pen a refusal that \
             never happened"
        );
    }
}

#[test]
fn a_failure_that_will_recur_identically_belongs_to_the_event() {
    for (code, why) in DETERMINISTIC {
        assert!(
            deterministic_apply_failure(Some(code)),
            "{code} ({why}) will fail the same way on every retry. Freezing under it holds every \
             later event on that link behind one poison event, forever, with nothing penned and \
             no ack remedy — #621"
        );
    }
}

/// No SQLSTATE at all means the statement never reached the door: a dropped connection, a
/// client-side decode failure. Nothing about the bytes was decided, so this must NOT pen — a pen
/// row would claim a refusal that never happened.
#[test]
fn no_sqlstate_means_nothing_was_decided_about_the_bytes() {
    assert!(!deterministic_apply_failure(None));
}

/// A code too short (or too odd) to carry a class falls to the keep-the-link-moving side.
/// `get(..2)` returning `None` is the path being pinned; it is reachable from a server that is
/// not PostgreSQL, or a future client library that reports a truncated code.
#[test]
fn an_unreadable_code_pens_rather_than_freezes() {
    for code in ["", "4", "é"] {
        assert!(
            deterministic_apply_failure(Some(code)),
            "an unreadable code ({code:?}) must take the cheaper mistake: a pen is delayed, \
             loud and auto-released; a freeze is permanent"
        );
    }
}

/// P0001 never reaches this function — the arm above it in `pull_into` owns every door verdict —
/// but if it ever did, penning is the safe answer and freezing is not. Pinned so a refactor that
/// reorders the arms cannot make the wrong one silently.
#[test]
fn a_door_verdict_would_also_pen_rather_than_freeze() {
    assert!(deterministic_apply_failure(Some("P0001")));
}
