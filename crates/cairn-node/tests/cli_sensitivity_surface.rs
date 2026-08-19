//! #387 — CLI-surface regression tests for the §5.9 `--subject-kind` argument, spawning the
//! real `cairn-node` binary (via `CARGO_BIN_EXE_cairn-node`) so clap's own `Command`
//! construction is exercised. No extra test dependency and no database: `--help` and an
//! argument rejection both happen before anything connects.
//!
//! WHY THIS FILE EXISTS. `SubjectKind::ALL` has exactly one user-visible consequence —
//! the accepted values clap prints and enforces — and until now nothing tested it. The
//! library tests cannot: they can only assert `ALL` against itself. The review that closed
//! #387 found the accepted-values list had been hand-checked against the built binary and
//! never pinned, so a drift in the `value_parser` expression (say to `format!("{k:?}")`,
//! yielding `Event`/`Thread`/`Patient`) would have shipped silently and broken every
//! `sensitivity-assert` invocation an operator had in their notes.

use cairn_event::sensitivity::SubjectKind;
use std::process::Command;

/// A `Command` for the freshly-built `cairn-node` binary under test.
fn cairn_node() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cairn-node"))
}

#[test]
fn help_lists_exactly_the_subject_kinds_the_enum_defines() {
    let out = cairn_node()
        .args(["sensitivity-assert", "--help"])
        .output()
        .expect("the binary Cargo just built must be runnable");
    let help = String::from_utf8_lossy(&out.stdout);

    // Derived from the enum on BOTH sides, so this asserts that clap agrees with
    // `SubjectKind`, not that clap agrees with a literal someone retyped here.
    let expected: Vec<&str> = SubjectKind::ALL.iter().map(|k| k.as_str()).collect();
    assert!(
        help.contains(&format!("[possible values: {}]", expected.join(", "))),
        "--help must offer exactly the enum's wire words, in declaration order: {help}"
    );

    // The prose must NOT re-enumerate them: clap prints the doc line and the possible
    // values in the same block, so a second hand-maintained copy there would render as a
    // help page contradicting itself.
    // The OPTIONS entry, not the `Usage:` synopsis — clap renders the flag's prose and its
    // `[possible values: ...]` on the same line, which is exactly the collision under test.
    let doc_line = help
        .lines()
        .find(|l| l.contains("--subject-kind") && l.contains("[possible values:"))
        .expect("the flag must be documented with its accepted values");
    for word in &expected {
        assert_eq!(
            doc_line.matches(word).count(),
            1,
            "{word:?} appears twice on the --subject-kind line — the prose is enumerating \
             the values clap already lists: {doc_line}"
        );
    }
}

#[test]
fn a_subject_kind_this_build_does_not_know_is_refused_before_anything_is_authored() {
    // Refused at the LOCAL boundary only. The apply door must keep ADMITTING an
    // unrecognised kind from a peer (ADR-0056/ADR-0062 — it is interpreted conservatively
    // as chart-wide), so this must never be read as a wire-level rejection. It is the
    // kindness of failing on a typo before an append-only, un-unsayable event is signed.
    let nil = uuid::Uuid::nil().to_string();
    let out = cairn_node()
        .args([
            "--conn",
            "host=127.0.0.1 dbname=definitely-not-used",
            "sensitivity-assert",
            "--patient",
            &nil,
            "--subject-kind",
            "episode",
            "--subject-id",
            &nil,
            "--grade",
            "routine",
        ])
        .output()
        .expect("the binary Cargo just built must be runnable");

    assert!(!out.status.success(), "a typo must not reach the signer");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("episode"),
        "name what was rejected so the operator can see the typo: {err}"
    );
    for k in SubjectKind::ALL {
        assert!(
            err.contains(k.as_str()),
            "and name what IS accepted ({}): {err}",
            k.as_str()
        );
    }
}
