//! #909: no window holds the node's signing key or runs the quarantine commit. Both are absences
//! in the source (the window has no launch test, #738, and a reintroduced `keystore::load` passes
//! every behavioural test), so the source of this crate and of `cairn-gui-live` is read.
//! Comment lines are skipped: a doc comment may name what is gone to explain its absence.
use crate::source_guard::sources;

/// Every non-test source file's CODE (comment lines dropped), as (path, code).
fn window_code() -> Vec<(String, String)> {
    let mut files = vec![];
    for dir in ["/src", "/../cairn-gui-live/src"] {
        sources(
            std::path::Path::new(&format!("{}{dir}", env!("CARGO_MANIFEST_DIR"))),
            &mut files,
        );
    }
    assert!(files.len() > 20, "the walk found the sources");
    files
        .into_iter()
        .map(|(name, src)| {
            let code = src
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            (name, code)
        })
        .collect()
}

/// The node's key is never loaded, never taken as `NodeKey::Full`, and never rebuilt from bytes
/// outside `state.rs`, which holds the HUMAN clinician's key for the key-hold.
#[test]
fn the_window_never_loads_the_node_key() {
    for (name, code) in window_code() {
        for token in [
            "keystore::",
            "NodeKey::Full",
            "&cli.key", // not `cli.key_hold_seconds`
            "CAIRN_KEY_PASSPHRASE",
            "load_signing_key",
        ] {
            assert!(!code.contains(token), "{name} contains `{token}`");
        }
        if !name.ends_with("/state.rs") {
            assert!(
                !code.contains("SigningKey::from_bytes"),
                "{name} rebuilds a signing key from bytes"
            );
        }
    }
}

/// The node's service commits overdue drafts; the window only reads what it did.
#[test]
fn the_window_runs_no_quarantine_commit() {
    for (name, code) in window_code() {
        for token in ["commit_overdue", "run_ticks"] {
            assert!(!code.contains(token), "{name} contains `{token}`");
        }
    }
}
