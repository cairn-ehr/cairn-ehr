//! #594 — **the exit vocabulary `cairn-node restore` speaks, pinned at unit level.**
//!
//! A restore has three things to say and, until #594, only two codes to say them with:
//!
//! | Code | Meaning | Who reads it |
//! |---|---|---|
//! | **0** | Everything this build could apply is in the log | a cron drill's `&&` |
//! | **3** | The ceremony finished; records did NOT come back | a monitoring script |
//! | **1** | The ceremony was BLOCKED (a refused local-state bundle, a database fault) | the same |
//!
//! Every test here is **pure** — no database, no spawned binary, no `CAIRN_TEST_PG` gate — because
//! the decision of what "incomplete" means is a decision about five booleans and counters, and it
//! should be readable and falsifiable without a disaster rehearsal to run it against. The
//! end-to-end pins live in `restore_torn_medium_cli.rs`, `restore_cli_applies_nothing_untrusted.rs`
//! and `restore_cli_surface.rs`; this file pins the rule they each exercise one arm of.
//!
//! **Why the value 3 is pinned here rather than compared against `cairn-sync`'s constant.**
//! `cairn-sync` depends on `cairn-node`, never the reverse, so this crate's tests cannot `use
//! cairn_sync`. The equality is asserted from the *other* side instead, by
//! `cairn_sync::requeue::tests::exit_incomplete_matches_cairn_nodes_restore` — the earliest point
//! in the build where both numbers are visible, since `cairn-node` is a **dev**-dependency of that
//! binary-only crate. So the two constants are held together by a test, not by the type system,
//! and **this file is the half that pins the VALUE**: without it, both could be changed to 7
//! together and the equality test would still pass.

use cairn_node::restore::completeness::{Unrestored, EXIT_INCOMPLETE};

/// The number itself. `cairn-sync requeue` has used 3 for INCOMPLETE since #578, and a restore that
/// spoke a different number for the same state would be worse than one that stayed silent: a
/// script would have to learn two vocabularies for one recovery.
#[test]
fn incomplete_is_exit_three_for_both_binaries() {
    assert_eq!(
        EXIT_INCOMPLETE, 3,
        "cairn-sync requeue exits 3 for INCOMPLETE and is held equal to THIS constant by its own \
         test; changing it changes the contract both binaries offer a cron wrapper"
    );
}

/// A restore that left nothing behind is complete, and says nothing.
///
/// The positive control for every row below: without it, a `notice()` that returned `Some(..)`
/// unconditionally would pass all seven negative cases and fail no test.
#[test]
fn a_restore_that_left_nothing_behind_is_complete_and_silent() {
    let clean = Unrestored::default();
    assert!(clean.is_complete(), "{clean:?}");
    assert_eq!(
        clean.notice(),
        None,
        "a complete restore must print no verdict at all — an operator who sees a notice after \
         every successful drill stops reading them"
    );
}

/// Records past a mid-file chain break: on the medium, never offered to the door, and **no retry
/// of any command reaches them**. The most alarming of the five, and until #594 the quietest.
#[test]
fn records_past_a_chain_break_make_the_restore_incomplete() {
    let left = Unrestored {
        past_chain_break: 2,
        ..Default::default()
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    assert!(
        notice.contains('2') && notice.to_lowercase().contains("chain"),
        "the notice must name the count and the cause, not merely that something is wrong: \
         {notice}"
    );
}

/// A plane this build cannot route. Not damage — an upgrade prompt — but the records are not in
/// the log, and a drill that reads exit 0 records a recovery that did not happen.
#[test]
fn records_in_an_unroutable_plane_make_the_restore_incomplete() {
    let left = Unrestored {
        unknown_plane: 5,
        ..Default::default()
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    assert!(
        notice.contains('5') && notice.to_lowercase().contains("upgrade"),
        "the notice must name the count AND the remedy — this is the one cause a newer build \
         actually fixes: {notice}"
    );
}

/// A torn tail. This is the pin that **reverses** #500 slice 2c round 2's exit-0 ruling: the
/// restore still recovers the intact prefix and still refuses nothing (ADR-0068 decision 1
/// stands), but it no longer reports a partial recovery as a clean one.
#[test]
fn a_torn_tail_makes_the_restore_incomplete() {
    let left = Unrestored {
        torn_tail: true,
        ..Default::default()
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    assert!(
        notice.contains("TORN"),
        "the verdict must repeat the tear in the operator's own vocabulary — the early WARNING \
         says TORN and so must this: {notice}"
    );
}

/// Records held in the quarantine pen. Exit 1 before #594, and the most recoverable of the five:
/// `cairn-sync requeue` empties the pen without redoing the restore.
#[test]
fn penned_records_make_the_restore_incomplete() {
    let left = Unrestored {
        penned: 3,
        ..Default::default()
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    assert!(
        notice.contains('3') && notice.contains("cairn-sync requeue"),
        "the notice must name the count and the command that finishes the recovery: {notice}"
    );
}

/// No actor registry: the whole clinical plane was never offered to the door. The remedy is NOT
/// `requeue` — `finalize_identity` has already closed the registry door permanently — and printing
/// the pen's remedy here would be a false promise made to someone mid-disaster (#554 finding 4).
#[test]
fn a_missing_actor_registry_makes_the_restore_incomplete() {
    let left = Unrestored {
        no_registry: true,
        ..Default::default()
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    let lower = notice.to_lowercase();
    assert!(
        lower.contains("registry") && lower.contains("restore again"),
        "the notice must name the registry and send the operator to a SECOND RESTORE, never to \
         requeue: {notice}"
    );
    // Not "must not mention requeue" — it must mention it *and rule it out*. An operator whose
    // restore penned nothing has `cairn-sync requeue` as their obvious next move, and the notice
    // that stays silent about it sends them there anyway. #554 finding 4 is about OFFERING it as
    // the remedy, not about naming it.
    assert!(
        lower.contains("requeue") && lower.contains("not fix"),
        "requeue cannot fix a missing registry, and the notice must say so rather than leave the \
         operator to discover it at an empty pen: {notice}"
    );
}

/// All five at once. A notice that named only the first cause it found would pass every single-cause
/// row above; this is the row that fails it.
#[test]
fn every_cause_is_named_when_several_hold_at_once() {
    let left = Unrestored {
        past_chain_break: 2,
        unknown_plane: 5,
        torn_tail: true,
        penned: 3,
        no_registry: true,
    };
    assert!(!left.is_complete());
    let notice = left.notice().expect("a cause must produce a notice");
    let lower = notice.to_lowercase();
    for cause in ["chain", "upgrade", "torn", "pen", "registry"] {
        assert!(
            lower.contains(cause),
            "a restore with five causes must name all five, not the first one it found — \
             missing {cause:?} in:\n{notice}"
        );
    }
}

/// Whatever the cause, the verdict says the word a human reads and the number a script reads.
///
/// Both matter and they are different readers: the operator needs to know this is not a failure
/// (their charts ARE back, minus what is named), and the cron wrapper needs the integer.
#[test]
fn a_verdict_states_both_the_word_and_the_number() {
    for left in [
        Unrestored {
            past_chain_break: 1,
            ..Default::default()
        },
        Unrestored {
            unknown_plane: 1,
            ..Default::default()
        },
        Unrestored {
            torn_tail: true,
            ..Default::default()
        },
        Unrestored {
            penned: 1,
            ..Default::default()
        },
        Unrestored {
            no_registry: true,
            ..Default::default()
        },
    ] {
        let notice = left.notice().expect("a cause must produce a notice");
        assert!(
            notice.contains("INCOMPLETE"),
            "{left:?} must be reported as INCOMPLETE, the word requeue already uses: {notice}"
        );
        assert!(
            notice.contains(&EXIT_INCOMPLETE.to_string()),
            "{left:?} must name the exit status in the text too — stderr and the status are read \
             by different people: {notice}"
        );
    }
}

/// An ACKED penned row is not a separate cause, and must never become one.
///
/// `ClinicalRestoreReport::penned()` already counts acked rows, and the summary carries their own
/// NOTE (`requeue` deliberately skips them). Adding an `acked` field here would double-count the
/// same rows and let a restore that penned nothing report INCOMPLETE. Pinned so the "completeness"
/// of the struct is not improved into a defect.
#[test]
fn the_cause_list_is_exactly_five() {
    // A struct update from `default()` touching every field compiles only while the field set is
    // what this file believes it is; adding a sixth cause fails to compile HERE, at the test that
    // enumerates them, rather than silently going unpinned.
    let all = Unrestored {
        past_chain_break: 0,
        unknown_plane: 0,
        torn_tail: false,
        penned: 0,
        no_registry: false,
    };
    assert!(
        all.is_complete(),
        "every cause at its zero value is a complete restore: {all:?}"
    );
}
