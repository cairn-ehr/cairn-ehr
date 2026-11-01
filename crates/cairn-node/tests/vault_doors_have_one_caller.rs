//! #898 — the vault's key-releasing doors are called from ONE place: `cairn_node::vault`.
//!
//! # Why this guard exists
//!
//! `vault::release_for_act` judges the connection's power (`conn_posture`, ADR-0084) on every call
//! BEFORE the passphrase is sent, so a window started on the owner's login is refused without a
//! guess being counted. That holds only while nothing else calls `cairn_vault_release` (or the
//! owner-only `cairn_vault_unseal`) directly: a second caller, written next to the raw vault SQL
//! the window already sends for the hint and the terminal lock, would skip the check and pass
//! every behavioural test that never starts a window over an over-powered login. A source guard
//! is the proportionate net.
//!
//! It scans every production `src/**/*.rs` file of every shipped tree (`tests/common/sources.rs`),
//! skipping `*tests.rs` files and comment lines. It is a regression net against an accidental
//! bypass, not a defence against concealment: SQL assembled at runtime from fragments would pass.

#[path = "common/sources.rs"]
mod sources;

use sources::{production_rust_files, read_source, repo_root};

/// The doors whose callers are pinned.
const DOORS: [&str; 2] = ["cairn_vault_release(", "cairn_vault_unseal("];

/// (repo-relative file, door, why it may name the door there).
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "crates/cairn-node/src/vault.rs",
        "cairn_vault_release(",
        "`release_for_act`: THE caller, behind the posture check.",
    ),
    (
        "crates/cairn-node/src/conn_posture.rs",
        "cairn_vault_unseal(",
        "not a call: the posture check asks whether the login may EXECUTE it.",
    ),
    (
        "extensions/cairn_pgx/src/lib.rs",
        "cairn_vault_unseal(",
        "not a call: the extension DEFINES it; only db/061's release door calls it.",
    ),
];

#[test]
fn only_release_for_act_calls_the_vault_doors() {
    let root = repo_root();
    let mut scanned = 0;
    let mut hits = Vec::new();
    for path in production_rust_files(&root) {
        let rel = path
            .strip_prefix(&root)
            .expect("the walk stays under the root")
            .to_string_lossy()
            .into_owned();
        if rel.ends_with("tests.rs") {
            continue;
        }
        scanned += 1;
        let code: String = read_source(&path)
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for door in DOORS {
            if code.contains(door) {
                hits.push((rel.clone(), door));
            }
        }
    }
    assert!(
        scanned > 50,
        "the walk found the shipped sources ({scanned})"
    );
    let unexpected: Vec<_> = hits
        .iter()
        .filter(|(f, d)| !ALLOWED.iter().any(|(af, ad, _)| af == f && ad == d))
        .collect();
    assert!(
        unexpected.is_empty(),
        "a vault door is called outside `vault::release_for_act`, so it skips the posture check \
         (ADR-0084): {unexpected:?}"
    );
    // Positive control: a walk that silently missed the real caller would pass the check above.
    for (f, d, _) in ALLOWED {
        assert!(
            hits.iter().any(|(hf, hd)| hf == f && hd == d),
            "{f} no longer names {d}: update ALLOWED"
        );
    }
}
