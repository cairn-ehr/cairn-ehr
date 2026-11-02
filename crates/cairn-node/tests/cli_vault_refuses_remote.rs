//! #899: the vault's console verbs carry a passphrase, so they must refuse a database on another
//! machine BEFORE connecting. 192.0.2.1 is TEST-NET (unroutable): if the verb tried to connect it
//! would hang, so a quick refusal proves the check ran first.
use std::process::Command;
use std::time::{Duration, Instant};

fn refuses(verb_args: &[&str], pass_env: &str) {
    // Built at runtime: a usable passphrase, so the refusal under test is the remote one.
    let pass = format!("{}-{}", "a long enough", "test phrase");
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", "host=192.0.2.1 dbname=x"])
        .args(verb_args)
        .env(pass_env, &pass)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("another machine"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn set_terminal_passphrase_refuses_a_remote_database() {
    refuses(
        &["vault-set-terminal-passphrase"],
        "CAIRN_TERMINAL_PASSPHRASE",
    );
}

#[test]
fn bootstrap_superuser_refuses_a_remote_database() {
    refuses(
        &[
            "vault-bootstrap-superuser",
            "--staff-id",
            "s1",
            "--display-name",
            "Test Person",
        ],
        "CAIRN_VAULT_PASSPHRASE",
    );
}

/// No secret here, but the same posture as its siblings: a site's policy is set at the node's
/// own console, never across the network.
#[test]
fn set_unconfirmed_expiry_refuses_a_remote_database() {
    refuses(
        &["vault-set-unconfirmed-expiry", "--days", "30"],
        "CAIRN_VAULT_PASSPHRASE",
    );
}

/// The verb's argument rules are refused at parse time, before any connection: a bare verb (which
/// must never parse as "no expiry" and silently clear a site's policy), both flags at once, and a
/// limit outside 1 to 36500 days. Unroutable `--conn` again, so a quick refusal proves no connect.
#[test]
fn set_unconfirmed_expiry_parses_strictly() {
    for args in [
        &["vault-set-unconfirmed-expiry"][..],
        &["vault-set-unconfirmed-expiry", "--days", "5", "--none"],
        &["vault-set-unconfirmed-expiry", "--days", "0"],
        &["vault-set-unconfirmed-expiry", "--days", "36501"],
    ] {
        let started = Instant::now();
        let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
            .args(["--conn", "host=192.0.2.1 dbname=x"])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "a clap usage error: {args:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "{args:?}");
    }
}
