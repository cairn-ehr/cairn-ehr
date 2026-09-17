//! #594 — **the exit vocabulary `cairn-node restore` speaks, pinned at unit level.**
//!
//! A restore has three things to say and, until #594, only two codes to say them with:
//!
//! | Code | Meaning | Who reads it |
//! |---|---|---|
//! | **0** | Every record the medium carried is usable in the log | a cron drill's `&&` |
//! | **3** | The ceremony finished; records did NOT come back | a monitoring script |
//! | **1** | The ceremony was BLOCKED (a refused local-state bundle, a database fault) | the same |
//!
//! Every test here runs with **no database and no `CAIRN_TEST_PG` gate** — because the decision of
//! what "incomplete" means is a decision about five booleans and counters, and it should be readable
//! and falsifiable without a disaster rehearsal to run it against. All but one are pure; the
//! exception spawns `restore --help`, which needs no database either. The end-to-end pins live in
//! `restore_torn_medium_cli.rs`, `restore_cli_applies_nothing_untrusted.rs` and
//! `restore_cli_surface.rs`; this file pins the rule they each exercise one arm of.
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

/// **The ORDER the causes are named in, which `notice()` documents as deliberate and nothing
/// pinned.** (PR #612 review.)
///
/// `notice()`'s own comment: *"Ordered by how little the operator can do about it: the two that no
/// retry reaches come first, so the causes that matter most are not buried under the ones a
/// command fixes."* That is a real clinical judgement — an operator skimming a verdict mid-disaster
/// reads the top of it — and a refactor that reordered the five `push_str` branches would bury the
/// two unrecoverable causes under the three a command fixes, silently, with every other test in
/// this file still green.
#[test]
fn the_verdict_names_the_unrecoverable_causes_first() {
    let all = Unrestored {
        past_chain_break: 2,
        unknown_plane: 5,
        torn_tail: true,
        penned: 3,
        no_registry: true,
    };
    let notice = all.notice().expect("a cause must produce a notice");
    let at = |needle: &str| {
        notice
            .find(needle)
            .unwrap_or_else(|| panic!("the verdict must name {needle:?}:\n{notice}"))
    };
    // TORN and the chain break are the two nothing recovers; the pen is the one a single
    // `requeue` empties. Between them sit the two that need a second restore or a newer build.
    let order = [
        (
            "TORN",
            "a torn tail — nothing recovers what is not on this copy",
        ),
        (
            "last verified chain link",
            "records past a chain break — no retry of any command reaches them",
        ),
        (
            "NOT restored at all",
            "no registry — recoverable, but only by a whole second restore",
        ),
        (
            "cannot route",
            "an unroutable plane — recoverable by a newer build",
        ),
        (
            "HELD in the quarantine pen",
            "the pen — the most recoverable of the five, so it comes LAST",
        ),
    ];
    for pair in order.windows(2) {
        let (earlier, why_earlier) = pair[0];
        let (later, why_later) = pair[1];
        assert!(
            at(earlier) < at(later),
            "the verdict must name causes by how LITTLE the operator can do about them: \
             {why_earlier} must come before {why_later}. An operator skimming the top of this \
             notice would otherwise meet the fixable causes first and stop.\n{notice}"
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
    // A FULL struct literal — deliberately NOT `..Default::default()`. The exhaustiveness is the
    // whole mechanism: a sixth field makes this an E0063 missing-field error HERE, at the test
    // that enumerates the causes, rather than silently going unpinned. Adding the `..` spread
    // that would make this a "struct update" disarms the guard completely, so do not "tidy" it in.
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

/// **The vocabulary has to be learnable from the command.** (PR #612 review, finding 3.)
///
/// ADR-0071 is written for the author of a cron wrapper around a disaster-recovery drill. That
/// person reads `--help`, not `docs/spec/decisions/`. `cairn-sync requeue`'s usage text has printed
/// *"exit 3 = INCOMPLETE … exit 1 = the run itself failed"* since #578, and `restore` — the command
/// that FILLS the pen `requeue` empties — said nothing about its own statuses at all.
///
/// Spawns the binary because clap's help is assembled at runtime from the doc comment: asserting
/// against the source text would pass while the `--help` a human reads stayed silent.
#[test]
fn restore_help_names_the_exit_statuses() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["restore", "--help"])
        .output()
        .expect("cairn-node restore --help");
    // clap writes long help to stdout; take both so a clap version that moves it cannot make this
    // pass vacuously by finding nothing to search.
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // `--help` is itself part of the contract: `restore --help || exit 1` is a normal thing for a
    // wrapper's pre-flight to do, and help that prints but exits non-zero would break it while
    // every substring assertion below still passed (PR #612 second review round).
    assert!(
        out.status.success(),
        "`restore --help` must exit 0; got {:?}. Help:\n{help}",
        out.status.code()
    );
    assert!(
        help.contains("restore") && help.len() > 200,
        "positive control: this must be the real long help for `restore`, not an error or an \
         empty buffer — otherwise every assertion below is vacuous. Got:\n{help}"
    );
    for (needle, why) in [
        (
            "exit 3",
            "the INCOMPLETE status a drill's wrapper branches on",
        ),
        (
            "INCOMPLETE",
            "the word, so a human reading the log knows it is not a failure",
        ),
        (
            "exit 1",
            "the FAILED status, which means something different since ADR-0071",
        ),
        (
            "exit 0",
            "so 'nothing was left behind' is stated, not inferred from silence",
        ),
    ] {
        assert!(
            help.contains(needle),
            "`restore --help` must name {needle:?} — {why}. A cron wrapper cannot learn this \
             vocabulary anywhere else. Help:\n{help}"
        );
    }
}

/// **Exit 0's stated limits must match what exit 0 actually does** (#614, ADR-0072).
///
/// The EXIT STATUS block used to disclose #614 as a known limit of exit 0: *"a record this build
/// cannot CLASSIFY is in the log and counted, but yields no chart until this node is upgraded"*.
/// Once the summary REPORTS those records, that sentence is stale — it tells a cron-wrapper
/// author that the command is silent about something it now names.
///
/// This is the fourth instance of a pattern PR #612 caught three times: **a fix written under the
/// pressure of a finding is itself unreviewed code, and `--help` is part of the contract.** Round
/// 2's fix wrote round 3's contradiction; round 3's fix was falsified by an issue round 3 had
/// filed an hour earlier. The cheap mechanical form of the defence is to re-ask the original
/// question of the fix's own diff — and, for a published contract, to pin it with a test.
///
/// Spawned, never source-matched, for the reason the test above states.
#[test]
fn restore_help_says_a_deferred_record_is_reported_not_merely_admitted() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["restore", "--help"])
        .output()
        .expect("cairn-node restore --help");
    let help = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "`restore --help` must exit 0; got {:?}. Help:\n{help}",
        out.status.code()
    );
    assert!(
        help.contains("exit 0") && help.len() > 200,
        "positive control: this must be the real long help, or the assertion below is vacuous. \
         Got:\n{help}"
    );
    assert!(
        help.contains("cairn-node deferred"),
        "exit 0's stated limits must name the command that LISTS the records they are about, now \
         that the summary reports them. A contract that still calls this a silence is a contract \
         its own code falsifies. Help:\n{help}"
    );
}
