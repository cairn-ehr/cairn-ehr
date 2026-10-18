//! #820: every CLI reader of `SignOffOutcome` names every field, none behind `..`.
//!
//! A field added to the outcome must break the build at each reader, so a new fact cannot be
//! dropped from the report by default — the way `failed` once was. The compiler enforces that
//! only while no reader writes `..`, and `..` is the one-token fix for the very compile error the
//! rule raises. So the source is read. The window's twin is in `cairn-gui-tauri`'s
//! `commands_tests.rs` (`sign_off_impl`).

/// Each `let …SignOffOutcome { … } =` destructuring pattern in `src`, up to its `} =`.
///
/// A construction (`let x = |…| SignOffOutcome {`) is not a pattern: its line has an `=` before
/// the type name, so it is skipped.
fn destructures(src: &str) -> Vec<&str> {
    src.match_indices("SignOffOutcome {")
        .filter(|(at, _)| {
            let line_start = src[..*at].rfind('\n').map_or(0, |n| n + 1);
            let before = src[line_start..*at].trim_start();
            before.starts_with("let ") && !before.contains('=')
        })
        .map(|(at, _)| {
            let rest = &src[at..];
            &rest[..rest.find("} =").expect("the pattern closes with `} =`")]
        })
        .collect()
}

/// Whether a pattern hides a field behind `..` (comment lines are not code).
fn has_rest(pattern: &str) -> bool {
    pattern
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .any(|l| l.contains(".."))
}

#[test]
fn every_cli_reader_of_the_sign_off_outcome_names_every_field() {
    let found = destructures(include_str!("../src/main.rs"));
    assert_eq!(
        found.len(),
        2,
        "the `sign-off` arm and `nothing_attested_sentence` destructure the outcome: {found:#?}"
    );
    for pattern in found {
        assert!(
            !has_rest(pattern),
            "a `..` lets a new SignOffOutcome field drop out of the report silently; name the \
             field and bind it to `_` with a comment saying why it is not shown: {pattern}"
        );
    }
}

/// The guard above can fail: `..` and a construction are told apart from a full pattern.
#[test]
fn the_guard_sees_a_rest_pattern_and_skips_a_construction() {
    let src = "let a = |x| SignOffOutcome {\n    attested,\n};\n\
               let m::SignOffOutcome {\n    attested,\n    // see .. below\n    ..\n} = out;\n";
    let found = destructures(src);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(has_rest(found[0]));
    assert!(!has_rest(
        "SignOffOutcome {\n    attested,\n    // a comment with .. in it\n"
    ));
}
