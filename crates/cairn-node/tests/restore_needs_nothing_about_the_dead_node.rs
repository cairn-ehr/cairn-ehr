//! **The cognitive-load half of #512's §1.2 budget, pinned.**
//!
//! DR slice 1's paper-parity section promises that a restore needs *"one secret and **no
//! knowledge of the dead node's config**"*. The time half of that budget is measured (see
//! `crates/cairn-node/results/`); the clause pinned here is the other half, and it is the
//! one that can regress **silently**. Nothing about adding a required `--node-name` or
//! `--origin` flag to `Cmd::Restore` would fail a single existing test, and the operator it
//! would strand is one whose disk is already dead — the exact person who cannot look the
//! answer up.
//!
//! **What "no knowledge of the dead node's config" means operationally.** Everything the
//! restore needs *about the dead node* must come off the **medium**, never out of the
//! operator's memory or a config file that died with the disk. Two flags are required today
//! and neither is dead-node knowledge:
//!
//! - `--conn` names the **new, freshly-created** database being restored INTO. It is a fact
//!   about the replacement machine, which the operator is standing at.
//! - `--from` is the path to the medium they just attached.
//!
//! `--superseded-node` is the interesting case, and it is why this guard reads
//! *required-ness* rather than merely counting flags: it names a dead node id, so it IS
//! dead-node knowledge — but it is **optional**, auto-detected from a sole enroll, and needed
//! only on a federated medium carrying several nodes' genesis. Optional is the whole
//! distinction. A solo clinic, the forcing case ADR-0026 names, supplies nothing but the two
//! facts above.
//!
//! **Why this drives the real binary rather than reading `main.rs`.** `Cmd` is defined in a
//! binary crate, so an integration test cannot import it (the wall this crate keeps hitting).
//! Spawning `cairn-node restore --help` tests the shipped surface instead of a source-text
//! approximation of it — the same route `restore_torn_medium_cli.rs` and `cli_localstate.rs`
//! already take. No DB, no key, no fixture.
//!
//! **The secret half is deliberately NOT pinned here**, because today it would pin a defect.
//! `restore` needs two secrets, not one: a passphrase for the NEW key (invented at restore
//! time, so not *retained* — it has both `--passphrase` and `CAIRN_KEY_PASSPHRASE`) and the
//! OLD node's recovery code, which unseals the local-state export. Only the second is
//! retained, so the budget's "one secret" holds as written — but the recovery code has
//! **no flag and no environment variable at all**; it is read through `rpassword`, which
//! fails on any non-tty. Filed from the #512 measurement run.

use std::process::Command;

/// Pull the **required** flags out of a clap `--help` listing.
///
/// Pure and total on purpose: it takes text and returns names, so the interesting cases
/// below can be driven from synthetic help text rather than by breaking the real CLI to see
/// the guard fire.
///
/// **How required-ness is read, and why it is the usage line.** clap prints every flag under
/// `Options:` whether required or not, so that section cannot answer the question. The usage
/// line can: clap collapses the optional flags into a single literal `[OPTIONS]` token and
/// prints each **required** one explicitly. So in
/// `Usage: cairn-node --conn <CONN> restore [OPTIONS] --from <FROM>` the required set is
/// exactly `{--conn, --from}` and everything else is inside `[OPTIONS]`.
fn required_flags(help: &str) -> Vec<String> {
    usage_line(help)
        .split_whitespace()
        // `[OPTIONS]` is clap's placeholder for the optional group, and a `[`-wrapped token
        // is an optional argument spelled out. Neither is required.
        .filter(|t| !t.starts_with('['))
        .filter(|t| t.starts_with("--"))
        // A flag may be printed as `--name=<V>`; keep the name.
        .filter_map(|t| t.split('=').next())
        .map(str::to_string)
        .collect()
}

/// Every flag the help listing **documents**, required or not, read from `Options:`.
///
/// Needed because the usage line deliberately hides optional flags behind `[OPTIONS]`, so
/// "is `--superseded-node` still offered, and is it optional?" cannot be answered from the
/// usage line alone. Documented-minus-required is the optional set.
fn documented_flags(help: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_options = false;
    for line in help.lines() {
        if line.trim_start().starts_with("Options:") {
            in_options = true;
            continue;
        }
        if !in_options {
            continue;
        }
        // Option lines are indented and start with the flag (possibly after a short form,
        // e.g. `  -h, --help`). Wrapped description lines are indented further and do not
        // begin with a dash, so they fall through.
        for token in line.split_whitespace().take(2) {
            if let Some(name) = token.trim_end_matches(',').split('=').next() {
                if name.starts_with("--") {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

/// The `Usage:` line of a clap help listing, joined if clap wrapped it across lines.
///
/// clap wraps a long usage line by indenting the continuation, so a naive "first line after
/// `Usage:`" read would silently drop the tail — and dropping the tail is precisely how a
/// newly-added required flag would go unnoticed by this guard. Continuations are folded in.
fn usage_line(help: &str) -> String {
    let mut collecting = false;
    let mut parts: Vec<&str> = Vec::new();
    for line in help.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("Usage:") {
            collecting = true;
            parts.push(rest.trim());
            continue;
        }
        if collecting {
            // A continuation is indented and non-empty; anything else ends the usage line.
            if line.starts_with(' ') && !line.trim().is_empty() {
                parts.push(line.trim());
            } else {
                break;
            }
        }
    }
    parts.join(" ")
}

#[test]
fn the_parser_reads_required_ness_from_the_usage_line() {
    // Synthetic, so the guard's own logic is exercised without breaking the real CLI. This
    // is the real shape: two required flags printed, the rest behind `[OPTIONS]`.
    let help = "Restore a node\n\n\
                Usage: cairn-node --conn <CONN> restore [OPTIONS] --from <FROM>\n\n\
                Options:\n      --from <FROM>\n          the medium\n      \
                --superseded-node <S>\n          the dead node\n  -h, --help\n          Print help\n";
    assert_eq!(required_flags(help), vec!["--conn", "--from"]);
    assert_eq!(
        documented_flags(help),
        vec!["--from", "--superseded-node", "--help"],
        "the Options section lists optional flags the usage line hides behind [OPTIONS]"
    );
}

#[test]
fn the_parser_folds_a_wrapped_usage_line_rather_than_truncating_it() {
    // The failure this protects against: clap wraps, the tail carries a NEW required flag,
    // and a parser that read one line would report the CLI as clean.
    let help = "Usage: cairn-node --conn <CONN> restore --from <FROM>\n           \
                --node-name <NAME> [OPTIONS]\n\nOptions:\n";
    assert!(
        required_flags(help).contains(&"--node-name".to_string()),
        "a required flag on a wrapped continuation line must still be seen"
    );
}

#[test]
fn the_parser_sees_a_newly_required_flag() {
    // Drive the guard's assertion, not just its parser: this is the shape the real check
    // below rejects, and pinning it is what makes a green run meaningful.
    let help = "Usage: cairn-node --conn <CONN> restore [OPTIONS] --from <FROM> --origin <O>\n";
    let unexpected: Vec<String> = required_flags(help)
        .into_iter()
        .filter(|f| f != "--conn" && f != "--from")
        .collect();
    assert_eq!(unexpected, vec!["--origin".to_string()]);
}

/// The `--help` text of `cairn-node restore`, from the real binary.
fn restore_help() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["restore", "--help"])
        .output()
        .expect("the cairn-node binary builds and answers --help");
    assert!(
        out.status.success(),
        "restore --help must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// **The guard.** `restore` may require the medium and the new database, and nothing else.
///
/// A new required flag here is not a style question: `restore` runs after total hardware
/// loss, so any fact it demands beyond these two is a fact the operator may no longer have
/// any way to obtain. If this test fails, the fix is to give the new flag a default or read
/// it off the medium — **not** to widen the list.
#[test]
fn restore_requires_the_medium_and_the_new_database_and_nothing_else() {
    let help = restore_help();
    assert_eq!(
        required_flags(&help),
        vec!["--conn", "--from"],
        "restore must demand only the NEW database and the medium — every fact about the \
         DEAD node has to come off the medium, because the operator's disk is gone (#512's \
         \"no knowledge of the dead node's config\"). Full help:\n{help}"
    );
}

/// The companion clause: `--superseded-node` must stay **optional**.
///
/// Separated from the guard above because the two fail for different reasons, and a reader
/// who sees this one red should not go looking for a new flag. This is the flag that names
/// dead-node knowledge, so its optionality IS the budget clause for a solo clinic — the
/// forcing case ADR-0026 names. Making it required would strand exactly one operator: the
/// one restoring a solo node who never knew its node id.
#[test]
fn the_dead_node_id_stays_optional() {
    let help = restore_help();
    assert!(
        documented_flags(&help).contains(&"--superseded-node".to_string()),
        "restore must still offer --superseded-node for a federated medium:\n{help}"
    );
    assert!(
        !required_flags(&help).contains(&"--superseded-node".to_string()),
        "--superseded-node names a DEAD node's id; requiring it would make a solo restore \
         depend on a fact that died with the disk. It is auto-detected from a sole enroll."
    );
}
