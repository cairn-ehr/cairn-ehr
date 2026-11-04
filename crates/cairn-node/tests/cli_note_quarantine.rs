//! Sign-in slice 3b-2a: `cairn-node note-set-quarantine` sets how long an untouched note draft
//! waits before it goes into the record unsigned. A site's policy is set at its own console, so
//! the verb refuses a database on another machine BEFORE connecting, as the vault's policy verb
//! does (`cli_vault_refuses_remote.rs`). 192.0.2.1 is TEST-NET (unroutable): a connect attempt
//! would hang, so a quick answer proves no connect was tried.
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
fn set_quarantine_refuses_a_remote_database() {
    for args in [
        &["note-set-quarantine", "--minutes", "60"][..],
        &["note-set-quarantine", "--default"],
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

/// Exactly one of `--minutes` or `--default`: a bare verb must never parse as "the default" and
/// silently reset a site's policy. And the range is 1 to 43200 minutes (30 days).
#[test]
fn set_quarantine_parses_strictly() {
    for args in [
        &["note-set-quarantine"][..],
        &["note-set-quarantine", "--minutes", "60", "--default"],
        &["note-set-quarantine", "--minutes", "0"],
        &["note-set-quarantine", "--minutes", "43201"],
    ] {
        let (out, took) = run(args);
        assert_eq!(out.status.code(), Some(2), "a clap usage error: {args:?}");
        assert!(took < Duration::from_secs(5), "{args:?}");
    }
}
