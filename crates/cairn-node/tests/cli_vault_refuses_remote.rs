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
