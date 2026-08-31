//! The nil-patient UUID must have exactly ONE declaration in the workspace's production sources.
//!
//! It moved to `cairn-event` (a wire constant belongs with the wire format) and
//! `cairn_node::identity` re-exports it. The failure mode this guards is a future COPY: a second
//! `const NIL_PATIENT = "0000…"` elsewhere compiles, compares equal to this one today, and drifts
//! the moment either is edited — inside signed bytes, on two nodes that no longer agree.
//!
//! # Why this is a source scan and NOT a pointer comparison
//!
//! The obvious guard — assert the two paths are the same pointer — is VACUOUS, and was measured
//! to be so on 2026-08-31: rustc merges identical `&'static str` literals into a single rodata
//! entry, so `ptr::eq` returns true even for two independently-declared copies in different
//! crates. Reverting `identity.rs` to its own literal left that test PASSING. It would have
//! passed forever while catching nothing. **Do not reintroduce it.**

use std::fs;
use std::path::PathBuf;

/// `crates/`, from this test's own manifest dir (`crates/cairn-node`).
fn crates_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()
        .expect("crates/ dir")
}

/// Every `.rs` file under `crates/*/src/`, recursively. Production sources only: a test fixture
/// may legitimately spell the nil UUID, and does today.
fn production_sources() -> Vec<PathBuf> {
    fn walk(dir: &PathBuf, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for e in fs::read_dir(crates_dir()).expect("read crates/").flatten() {
        let src = e.path().join("src");
        if src.is_dir() {
            walk(&src, &mut out);
        }
    }
    out
}

/// Does this line DECLARE the constant, as opposed to using or re-exporting it?
///
/// A declaration binds the name to a value. Pure, so the recogniser itself is tested below —
/// a scan whose recogniser is untested can report "exactly one" because it recognises nothing.
fn declares_nil_patient(line: &str) -> bool {
    let l = line.trim_start();
    (l.starts_with("const NIL_PATIENT") || l.starts_with("pub const NIL_PATIENT"))
        && l.contains('=')
}

#[test]
fn nil_patient_is_declared_exactly_once_in_production_sources() {
    let mut found: Vec<String> = Vec::new();
    for path in production_sources() {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if declares_nil_patient(line) {
                found.push(format!("{}:{}", path.display(), i + 1));
            }
        }
    }
    assert_eq!(
        found.len(),
        1,
        "NIL_PATIENT must be DECLARED exactly once across crates/*/src/ — every other use is a \
         re-export or a reference. It is a WIRE constant: a second declaration compiles, compares \
         equal today, and drifts the moment either is edited, inside signed bytes on nodes that \
         then disagree. Declarations found: {found:?}"
    );
    assert!(
        found[0].contains("cairn-event"),
        "the one declaration must live in cairn-event, with the rest of the wire format; found {}",
        found[0]
    );
}

#[test]
fn nil_patient_declaration_recogniser_is_not_vacuous() {
    assert!(declares_nil_patient(
        "pub const NIL_PATIENT: &str = \"00000000-0000-0000-0000-000000000000\";"
    ));
    assert!(declares_nil_patient(
        "    const NIL_PATIENT: &str = \"00000000-0000-0000-0000-000000000000\";"
    ));
    assert!(
        !declares_nil_patient("pub use cairn_event::NIL_PATIENT;"),
        "a re-export is not a declaration — recognising it would make the scan permanently red"
    );
    assert!(
        !declares_nil_patient("    patient_id: NIL_PATIENT.into(),"),
        "a use site is not a declaration"
    );
    assert!(!declares_nil_patient(
        "            \"00000000-0000-0000-0000-000000000000\","
    ));
}

#[test]
fn nil_patient_reexport_agrees_by_value() {
    assert_eq!(
        cairn_node::identity::NIL_PATIENT,
        cairn_event::NIL_PATIENT,
        "the two paths must name the same value"
    );
}
