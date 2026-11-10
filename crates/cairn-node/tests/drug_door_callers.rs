//! No floor, projection or sync-path code may call the drug read door (allergy slice 3, ADR-0098 M1).
//!
//! `no_drugref_dependency.rs` keeps every mention of drugref out of the trusted surface, except in
//! the read door itself (`db/076_drug_read_door.sql`). That alone no longer proves the floor is
//! drugref-free: the door's functions are deliberately named WITHOUT "drugref"
//! (`cairn_drug_available`, `cairn_drug_search`, `cairn_drug_facts`, `cairn_drug_group_status`) so
//! the window's Rust can call
//! them, which means a floor check or a projection applier could `PERFORM cairn_drug_facts(…)`
//! and make admission depend on drugref with the word never appearing. This guard closes that:
//! no `db/*.sql` other than the door, and no source under `extensions/` (the pgrx floor), may
//! name a `cairn_drug_` function outside a comment.
//!
//! What it does NOT scan, on purpose: Rust under `crates/*/src` — the window (and the medication
//! check behind it) is the door's legitimate caller, and its answers are advisory there; and any
//! `tests/` directory (the door's own tests call it).
//!
//! STATED LIMITATION: a line scanner, like its sibling. A comment marker inside a string literal
//! ends the line's scan early, so a call written after `'--'` on the same line is missed. It is a
//! regression net for an accidental caller, not a defence against concealment.
#[path = "common/sources.rs"]
mod sources;

use sources::{read_source, repo_root, source_files};

/// The door itself, the one file that defines (and so names) these functions.
const READ_DOOR: &str = "db/076_drug_read_door.sql";

/// The prefix every drug read door function carries.
const DOOR_PREFIX: &str = "cairn_drug_";

/// The lines of `text` that name a door function outside a comment (`--` in SQL, `//` in Rust),
/// trimmed. **Pure.**
fn door_calls(text: &str, is_rust: bool) -> Vec<String> {
    let marker = if is_rust { "//" } else { "--" };
    text.lines()
        .filter(|line| {
            line.split(marker)
                .next()
                .unwrap_or("")
                .contains(DOOR_PREFIX)
        })
        .map(|line| line.trim().to_string())
        .collect()
}

#[test]
fn the_scanner_sees_a_call_and_ignores_a_comment() {
    assert_eq!(
        door_calls("    PERFORM cairn_drug_facts(a, m); -- advisory", false),
        ["PERFORM cairn_drug_facts(a, m); -- advisory"]
    );
    assert!(door_calls("-- the window calls cairn_drug_facts", false).is_empty());
    assert!(door_calls("// see cairn_drug_search", true).is_empty());
    assert_eq!(
        door_calls("let x = \"cairn_drug_available\";", true).len(),
        1
    );
}

#[test]
fn no_floor_or_projection_calls_the_drug_read_door() {
    let root = repo_root();
    let door = root.join(READ_DOOR);
    let mut offenders = Vec::new();
    // What the walk saw, so it cannot pass vacuously (a moved directory, a renamed door): the door
    // itself, some floor SQL and some pgrx Rust.
    let (mut saw_door, mut sql, mut rust) = (false, 0, 0);
    for path in source_files(
        &[root.join("db"), root.join("extensions")],
        &["target", "tests"],
        &["sql", "rs"],
    ) {
        if path == door {
            saw_door = true;
            continue;
        }
        let is_rust = path.extension().and_then(|e| e.to_str()) == Some("rs");
        if is_rust {
            rust += 1;
        } else {
            sql += 1;
        }
        for line in door_calls(&read_source(&path), is_rust) {
            offenders.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(
        saw_door && sql > 0 && rust > 0,
        "the walk must see the door ({saw_door}), db/*.sql ({sql}) and extensions/ Rust ({rust})"
    );
    assert!(
        offenders.is_empty(),
        "the floor, the projections and the sync path must never depend on drugref, so nothing in \
         db/ (but the door, {READ_DOOR}) or extensions/ may call a {DOOR_PREFIX}* function — the \
         door is the window's, and advisory. Offenders:\n{}",
        offenders.join("\n")
    );
}
