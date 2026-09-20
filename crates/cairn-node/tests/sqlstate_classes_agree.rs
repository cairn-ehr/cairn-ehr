//! #621 — two planes, one list of SQLSTATE classes.
//!
//! `cairn-node`'s `deterministic_apply_failure` (the node puller) and `cairn-sync`'s
//! `apply_failure_is_local` (the clinical requeue) answer the same question in opposite polarity:
//! *is this failure this node's own trouble, or the event's?* They must claim the same classes,
//! because the answer is a property of PostgreSQL, not of a plane.
//!
//! They are two functions today for one boring reason — `cairn-sync`'s lives inside its binary
//! crate's `main.rs`, where `cairn-node` cannot import it. Merging them into one home is part of
//! #626, which brings the clinical PULL arm onto the same classifier. Until then this guard is
//! what stops the copy drifting: a class added to one list and not the other means the two planes
//! disagree about whether a deadlock is a peer's fault.
//!
//! **Source-level on purpose.** The values are `const`-shaped matches inside two private
//! functions, so there is nothing to ask at runtime; what a reader must be stopped from doing is
//! editing one list. The guard is non-vacuous by construction: it fails if either extraction
//! finds nothing, which is what would otherwise turn a renamed function into a silent pass.
//!
//! **What it does NOT guard, stated so nobody relies on it for more** (PR #627 review, finding 5):
//! the two functions have OPPOSITE polarity (`deterministic_apply_failure` negates the match and
//! answers `false` for `None`; `apply_failure_is_local` does neither), so this compares the SETS
//! they claim and nothing about what they then do with them. A refactor that dropped the `!` would
//! invert the node plane's whole answer and leave both lists identical here. That inversion is
//! what `node_pull_refusal_class.rs` exists for — it asserts the meaning, class by class.

use std::fs;
use std::path::PathBuf;

fn crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("crates/ dir")
}

/// The SQLSTATE literals inside the body of `fn <name>` in `file`: two-character CLASSES, and
/// five-character full codes for the exceptions claimed ahead of the class match (`XX001` /
/// `XX002`). Both lengths matter — an exception added on one plane and not the other is exactly
/// the drift this guard exists to catch, and a classes-only extractor would not see it.
///
/// The body is taken from the function's signature to the first line that is exactly `}` at
/// column 0 — the shape rustfmt guarantees for a top-level item, and the same convention the
/// other source guards in this tree use. Only double-quoted literals count, so the surrounding
/// prose (which names codes like `40001` and `53100`) cannot contribute.
fn classes_in(file: &str, name: &str) -> Vec<String> {
    let src =
        fs::read_to_string(crates_dir().join(file)).unwrap_or_else(|e| panic!("read {file}: {e}"));
    let at = src.find(&format!("fn {name}(")).unwrap_or_else(|| {
        panic!(
            "{file} no longer declares {name} — if it was renamed, rename it here too; \
             this guard exists to notice"
        )
    });
    let body: &str = &src[at..];
    let end = body
        .find("\n}\n")
        .unwrap_or_else(|| panic!("could not find the end of {name} in {file}"));
    let body = &body[..end];

    let mut out: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find('"') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('"') else { break };
        let literal = &rest[..close];
        let shaped = matches!(literal.len(), 2 | 5);
        if shaped && literal.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push(literal.to_string());
        }
        rest = &rest[close + 1..];
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn the_node_and_clinical_planes_claim_the_same_local_sqlstates() {
    let node = classes_in("cairn-node/src/sync.rs", "deterministic_apply_failure");
    let clinical = classes_in("cairn-sync/src/main.rs", "apply_failure_is_local");

    // Non-vacuity first: an empty extraction would make the equality below trivially true, which
    // is how a guard like this dies silently (the #586 shape — a source guard that stopped seeing
    // the code it guards).
    // The inventory is 9 (seven classes + XX001/XX002). The floor is deliberately set just BELOW
    // it rather than AT it (PR #627 review, second pass): at 9 a legitimate removal of one class
    // would fail here, with a message telling the author to fix the GUARD, when what they want is
    // the drift message below or no failure at all. Eight is far enough above a broken
    // extractor's 0–2 to catch the #586 shape and far enough below the inventory to stay out of
    // the way of a real edit. Raise it if the inventory grows a lot.
    assert!(
        node.len() >= 8 && clinical.len() >= 8,
        "the extraction found too few classes to be believable — node: {node:?}, \
         clinical: {clinical:?}. The guard, not the code, is what to fix."
    );
    assert_eq!(
        node, clinical,
        "the two planes disagree about which SQLSTATE classes are THIS NODE's own trouble. \
         Whichever list was edited, the other plane now treats the same failure differently: on \
         the node plane a wrong answer freezes a peer's cursor forever (#621), on the clinical \
         plane it halts a requeue run at the same row every time (#480). Change both, or merge \
         them (#626)."
    );
}
