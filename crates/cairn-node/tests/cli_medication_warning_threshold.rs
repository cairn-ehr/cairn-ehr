//! Batch-charting slice 3: `cairn-node medication-set-warning-threshold` sets which findings count
//! as a warning when a medication is charted (db/081). It is the administrator's policy, set at
//! the node's own console, so the verb refuses a database on another machine BEFORE connecting,
//! as `note-set-quarantine` does (`cli_note_quarantine.rs`). 192.0.2.1 is TEST-NET (unroutable):
//! a connect attempt would hang, so a quick answer proves no connect was tried.
use std::process::Command;
use std::time::{Duration, Instant};

/// Run the binary against the unroutable database with `args`.
fn run(args: &[&str]) -> (std::process::Output, Duration) {
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", "host=192.0.2.1 dbname=x"])
        .args(args)
        .output()
        .unwrap();
    (out, started.elapsed())
}

#[test]
fn set_threshold_refuses_a_remote_database() {
    for args in [
        &[
            "medication-set-warning-threshold",
            "--level",
            "adverse-effect",
        ][..],
        &["medication-set-warning-threshold", "--default"],
    ] {
        let (out, took) = run(args);
        assert!(!out.status.success(), "{args:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("another machine"),
            "{args:?}"
        );
        assert!(took < Duration::from_secs(5), "{args:?}");
    }
}

/// Exactly one of `--level` or `--default`: a bare verb must never parse as "the default" and
/// silently reset a site's policy. And only the two known levels parse: there is no word that
/// switches allergy warnings off.
#[test]
fn set_threshold_parses_strictly() {
    for args in [
        &["medication-set-warning-threshold"][..],
        &[
            "medication-set-warning-threshold",
            "--level",
            "adverse-effect",
            "--default",
        ],
        &["medication-set-warning-threshold", "--level", "nothing"],
        &["medication-set-warning-threshold", "--level", "none"],
    ] {
        let (out, took) = run(args);
        assert_eq!(out.status.code(), Some(2), "a clap usage error: {args:?}");
        assert!(took < Duration::from_secs(5), "{args:?}");
    }
}
