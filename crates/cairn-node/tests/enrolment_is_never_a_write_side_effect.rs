//! ⇒ NOTHING PROVISIONS AN ACTOR ON A WRITE PATH — the guard #654 shipped without.
//!
//! #654's whole value is a rule a reader can trust: `cairn-node init` enrols, `cairn-node
//! enroll-device-actor` is the remedy, and every write subcommand calls `require_device_actor`
//! and REFUSES. Nothing provisions.
//!
//! The PR #661 review measured what enforced that rule and found: nothing. All fifteen write
//! call sites could be deleted and the whole workspace gate stays green — but the mild half of
//! that is the deletion (the command falls through to db/005's own refusal and the operator
//! merely loses the remedy sentence). **The half that matters is the opposite mutation: a
//! sixteenth write command calling `enroll_device_actor` instead of `require_device_actor`**,
//! which silently reintroduces the provisioning-on-write-path shape #654 just closed, in a
//! command nobody was reviewing for it.
//!
//! `require_device_actor`'s own doc anticipates exactly that — *"the asymmetry #654 closed would
//! be back the moment anyone added a sixteenth write command"* — and until this file, nothing
//! detected it.
//!
//! # Why a source guard rather than a behavioural test
//!
//! The behaviour is "an enrolment happened that should not have", which is only observable by
//! driving each of sixteen CLI subcommands against a database and counting `actor_event` rows.
//! That is a rig per command for a property that is a single call-site question. The precedent
//! in this tree is `pen_rows_leave_through_one_door.rs`, written for the same reason in the same
//! shape: a comment-stripped scan with an explicit `ALLOWED` inventory, so that widening the
//! allow-list is a deliberate, reviewable act rather than a silent one.
//!
//! ⚠️ **When this fails, do not add your new call site to `ALLOWED` to make it green.** Ask
//! first whether the code should be calling `require_device_actor` instead. `ALLOWED` is the
//! inventory of the two places provisioning is an owner ceremony, and it should stay that size.
//!
//! # Scope
//!
//! The provisioning scan covers **every shipped `.rs` surface in the repository**, not just
//! `main.rs` — see `only_a_provisioning_ceremony_may_enrol_a_device_actor` for why the
//! `main.rs`-only first version left its own stated mutation undetected, and for the one
//! nesting limit that remains.

#[path = "common/sources.rs"]
mod sources;

/// The only call sites permitted to PROVISION an actor, and why each is an owner ceremony.
///
/// Both are in `main.rs` because both are operator-initiated commands, not writes: `init`
/// provisions a brand-new node (which is what keeps #654's §1.2 step count at `M = 0`), and
/// `enroll-device-actor` exists solely to be the remedy a refusal names.
const ALLOWED: &[(&str, &str)] = &[
    (
        "Cmd::Init",
        "provisioning a brand-new node — the owner ceremony itself",
    ),
    (
        "Cmd::EnrollDeviceActor",
        "the named remedy for a node that never ran init (a restore never does)",
    ),
];

/// Strip a line comment so a mention of the function in prose is not mistaken for a call.
///
/// ⚠️ Truncates at the FIRST `//`, so it **fails open** on a line carrying `//` inside a string
/// literal (a URL in a `context` message) — unlike `enclosing_arm`, which fails closed. No line in
/// the scanned trees has that shape today;
/// [#670](https://github.com/cairn-ehr/cairn-ehr/issues/670) carries the tightening.
fn strip_comment(line: &str) -> &str {
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Which `Cmd::` arm is line `n` inside? The nearest `Cmd::X` at match-arm indentation above it.
///
/// Crude on purpose: `main.rs`'s dispatch is one giant `match` whose arms all sit at exactly
/// eight spaces, so "the last `        Cmd::` line at or above this one" names the arm exactly,
/// and a wrong answer can only ever be a NEIGHBOURING arm — which still fails the test, loudly,
/// with a name a human can check in seconds.
fn enclosing_arm(lines: &[&str], n: usize) -> String {
    for line in lines[..=n].iter().rev() {
        let t = line.trim_start();
        if line.len() - t.len() == 8 && t.starts_with("Cmd::") {
            let name = t.trim_start_matches("Cmd::");
            let end = name
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .unwrap_or(name.len());
            return format!("Cmd::{}", &name[..end]);
        }
    }
    "<outside any Cmd arm>".to_string()
}

/// ⚠️ SCANS EVERY SHIPPED `.rs` SURFACE, NOT JUST `main.rs` — and the widening was the point.
///
/// The first version of this guard read `src/main.rs` alone, which left the mutation its own
/// header names — *"a sixteenth write command calling `enroll_device_actor`"* — undetected
/// anywhere else. `enroll_device_actor` is `pub`, so the reachable callers are every module of
/// `cairn-node` **and every crate that depends on it**, `cairn-gui-live` included. That is not a
/// hypothetical target: `LiveData::new`'s own doc spends three paragraphs arguing that a GUI
/// silently minting a `device` actor is *worse* than the CLI doing it, and that sentence had no
/// enforcement at all. The guard defended the one surface that was already fixed.
///
/// `pub(crate)` would have been stronger, and is what `deliberate_refusal` got one module over —
/// but `src/main.rs` is a SEPARATE crate from `src/lib.rs`, so the binary reaches this function
/// through `cairn_node::`, and `pub(crate)` would break the two sanctioned ceremonies
/// themselves. Hence a widened scan (PR #661 review, converged on by three reviewers).
///
/// **Known limit, stated rather than implied:** `production_rust_files` walks
/// `<tree>/<crate>/src`, one level under each tree, so the nested `cairn-gui/cairn-gui-tabs/*`
/// crates are not covered. No tab crate depends on `cairn-node` today; if one ever does, widen
/// the helper rather than this test.
#[test]
fn only_a_provisioning_ceremony_may_enrol_a_device_actor() {
    let root = sources::repo_root();
    let files: Vec<_> = sources::production_rust_files(&root).collect();
    assert!(
        files.len() > 50,
        "the source sweep collapsed to {} files — a guard that scans nothing passes for the \
         wrong reason",
        files.len()
    );

    let mut offenders = Vec::new();
    for path in &files {
        let src = sources::read_source(path);
        let lines: Vec<&str> = src.lines().collect();
        let rel = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .display()
            .to_string();
        // Arm attribution is a `main.rs` concept: its dispatch is the one giant `match` whose
        // arms `enclosing_arm` reads. ANYWHERE ELSE a call is an offender outright — there is
        // no such thing as a sanctioned provisioning site outside the two CLI ceremonies, and
        // an `ALLOWED` entry must not be able to excuse a call in another file that happens to
        // sit under a like-named arm.
        let is_dispatch = rel.ends_with("cairn-node/src/main.rs");
        for (n, line) in lines.iter().enumerate() {
            let code = strip_comment(line);
            if !code.contains("enroll_device_actor(") {
                continue;
            }
            // The DECLARATION is not a call. `main.rs`-only scanning never had to say this;
            // a sweep that includes the defining module does, or the guard reports the
            // function's own signature as its first offender.
            if code.contains("fn enroll_device_actor(") {
                continue;
            }
            let arm = if is_dispatch {
                enclosing_arm(&lines, n)
            } else {
                "<not the CLI dispatch>".to_string()
            };
            if !(is_dispatch && ALLOWED.iter().any(|(a, _)| *a == arm)) {
                offenders.push(format!("  {}:{} — in {}", rel, n + 1, arm));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a write path is PROVISIONING an actor, which is the shape #654 closed (and ADR-0066 \
         decision 6 forbids one subsystem over, as trap 2).\n\nOffending call sites:\n{}\n\nUse \
         `require_device_actor`, which refuses and names the remedy. If this genuinely IS a new \
         owner ceremony, add it to ALLOWED with its reason — but that is a decision, not a fix.",
        offenders.join("\n")
    );
}

/// Every `ALLOWED` entry must still be a real call site, so the inventory cannot rot into a list
/// of places that no longer enrol — which would quietly let one of them stop provisioning.
///
/// Same discipline as `unwrap_secret_is_not_derived.rs`'s allow-list sweep: when an entry goes
/// stale, DELETE it; never leave it as decoration.
#[test]
fn every_allowed_provisioning_site_is_still_live() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("main.rs must be readable from the crate it belongs to");
    let lines: Vec<&str> = src.lines().collect();

    for (arm, why) in ALLOWED {
        let live = lines.iter().enumerate().any(|(n, line)| {
            strip_comment(line).contains("enroll_device_actor(") && enclosing_arm(&lines, n) == *arm
        });
        assert!(
            live,
            "{arm} is listed as a sanctioned provisioning site ({why}) but no longer calls \
             `enroll_device_actor`. If that is deliberate, delete the ALLOWED entry — a stale \
             allow-list is how a guard stops guarding."
        );
    }
}

/// ⇒ AND THE FIFTEEN WRITE PATHS STILL ASK. The other direction, which nothing pinned.
///
/// The guard above catches a write path that *provisions*. It cannot catch a write path that
/// simply stops *asking* — delete a `require_device_actor` call and the command falls through
/// to db/005's own refusal, which is still safe (every one of the fifteen authors with the
/// node's key as the event signer, so the floor refuses an unenrolled signer with `P0001`
/// regardless). The review confirmed that, and it is why this is the mild half.
///
/// But mild is not nothing: db/005's sentence names a key id and tells nobody what to do about
/// it, which `not_enrolled_refusal`'s doc calls out in so many words. Losing it silently is
/// losing the entire operator-facing value of #654 on that command.
///
/// A count, not a per-command assertion, because the per-command version is fifteen rigs for
/// one call-site question — the same reasoning the module doc gives for the scan above. If you
/// add a sixteenth write subcommand, this number goes UP, and that is the deliberate act.
#[test]
fn every_write_subcommand_still_asks_where_the_key_stands() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("main.rs must be readable from the crate it belongs to");
    let calls = src
        .lines()
        .filter(|line| strip_comment(line).contains("require_device_actor("))
        .count();

    assert_eq!(
        calls, 15,
        "the number of write subcommands asking `require_device_actor` changed. If you ADDED a \
         write subcommand, raise this number. If a call VANISHED, that command has quietly \
         reverted to db/005's key-id refusal — true, legible, and no remedy — which is the \
         operator-facing half of #654 gone. Restore it rather than lowering the number."
    );
}
