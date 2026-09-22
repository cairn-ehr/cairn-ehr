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

#[test]
fn only_a_provisioning_ceremony_may_enrol_a_device_actor() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/main.rs"))
        .expect("main.rs must be readable from the crate it belongs to");
    let lines: Vec<&str> = src.lines().collect();

    let mut offenders = Vec::new();
    for (n, line) in lines.iter().enumerate() {
        if !strip_comment(line).contains("enroll_device_actor(") {
            continue;
        }
        let arm = enclosing_arm(&lines, n);
        if !ALLOWED.iter().any(|(a, _)| *a == arm) {
            offenders.push(format!("  {}:{} — in {}", "src/main.rs", n + 1, arm));
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
        .expect("main.rs must be readable");
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
