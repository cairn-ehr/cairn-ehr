//! The two-way drift guard between `main.rs`'s `generate_handler!` list and the webview's calls
//! (moved out of `commands_tests.rs`, which had passed 500 lines, #1139).
//!
//! - Every command the webview calls is registered: a missing one fails only in the running
//!   window (#745).
//! - Every registered command is called by the webview: a registered command nothing calls is
//!   still reachable from any script in the webview, so a dead one is a second, unreviewed door
//!   (#1139: slice 1a's lone-line `med_chart` Sign outlived its caller and lacked the list's R1
//!   seen-set rule).
use std::collections::BTreeSet;

/// The command names `main.rs` registers (the last path segment of each `generate_handler!` row).
fn registered() -> BTreeSet<&'static str> {
    let main = include_str!("main.rs");
    let start = main
        .find("generate_handler![")
        .expect("main.rs registers commands");
    let block = &main[start..start + main[start..].find(']').expect("the list closes")];
    rows_of(block)
}

/// The command name on each row of a `generate_handler!` block. A row this cannot read (no
/// trailing comma, a trailing comment, two rows on a line) fails rather than being dropped: a
/// dropped row is a registered command the converse check never sees.
fn rows_of(block: &str) -> BTreeSet<&str> {
    block
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let path = line
                .strip_suffix(',')
                .filter(|path| !path.contains([',', ' ', '/']))
                .unwrap_or_else(|| {
                    panic!("a generate_handler! row this guard cannot read: {line}")
                });
            path.rsplit("::").next().unwrap_or(path)
        })
        .collect()
}

/// The command names the webview calls. Every call shape is read: a literal `invoke("name"`, and
/// the commands passed by name to the two wrappers that `invoke` a variable, link.js's
/// `sendJudgement` and superuser.js's `superuserAct`. `include_str!` needs literal paths,
/// so the scripts are written out — and checked against the scripts index.html actually loads, so
/// a new script cannot go unscanned (PR #798 review).
fn webview_calls() -> BTreeSet<&'static str> {
    let webview = [
        ("waiting.js", include_str!("../src-ui/waiting.js")),
        ("main.js", include_str!("../src-ui/main.js")),
        ("signer.js", include_str!("../src-ui/signer.js")),
        ("notes_save.js", include_str!("../src-ui/notes_save.js")),
        (
            "notes_compound.js",
            include_str!("../src-ui/notes_compound.js"),
        ),
        ("notes.js", include_str!("../src-ui/notes.js")),
        (
            "notes_discard.js",
            include_str!("../src-ui/notes_discard.js"),
        ),
        (
            "allergy_overlays.js",
            include_str!("../src-ui/allergy_overlays.js"),
        ),
        ("allergy_tags.js", include_str!("../src-ui/allergy_tags.js")),
        ("drug_pick.js", include_str!("../src-ui/drug_pick.js")),
        ("med_chart.js", include_str!("../src-ui/med_chart.js")),
        ("med_list.js", include_str!("../src-ui/med_list.js")),
        (
            "med_list_delabel.js",
            include_str!("../src-ui/med_list_delabel.js"),
        ),
        ("delabel_pick.js", include_str!("../src-ui/delabel_pick.js")),
        (
            "med_list_sign.js",
            include_str!("../src-ui/med_list_sign.js"),
        ),
        (
            "med_list_versions.js",
            include_str!("../src-ui/med_list_versions.js"),
        ),
        (
            "med_versions_settle.js",
            include_str!("../src-ui/med_versions_settle.js"),
        ),
        ("med_stat.js", include_str!("../src-ui/med_stat.js")),
        ("allergies.js", include_str!("../src-ui/allergies.js")),
        ("person_menu.js", include_str!("../src-ui/person_menu.js")),
        ("whos_here.js", include_str!("../src-ui/whos_here.js")),
        ("idle.js", include_str!("../src-ui/idle.js")),
        ("enrol.js", include_str!("../src-ui/enrol.js")),
        ("rekey.js", include_str!("../src-ui/rekey.js")),
        ("superuser.js", include_str!("../src-ui/superuser.js")),
        ("supervision.js", include_str!("../src-ui/supervision.js")),
        ("reseal.js", include_str!("../src-ui/reseal.js")),
        ("stop_reasons.js", include_str!("../src-ui/stop_reasons.js")),
        ("holds.js", include_str!("../src-ui/holds.js")),
        ("chart_stale.js", include_str!("../src-ui/chart_stale.js")),
        ("duplicates.js", include_str!("../src-ui/duplicates.js")),
        ("worklist.js", include_str!("../src-ui/worklist.js")),
        ("funnel.js", include_str!("../src-ui/funnel.js")),
        ("leave_chart.js", include_str!("../src-ui/leave_chart.js")),
        ("update.js", include_str!("../src-ui/update.js")),
        ("link.js", include_str!("../src-ui/link.js")),
        ("unlink.js", include_str!("../src-ui/unlink.js")),
    ];
    let html = include_str!("../src-ui/index.html");
    let opener = "<script src=\"";
    let loaded: BTreeSet<&str> = html
        .match_indices(opener)
        .map(|(at, _)| {
            let rest = &html[at + opener.len()..];
            &rest[..rest.find('"').expect("the src closes")]
        })
        .collect();
    assert_eq!(
        loaded,
        webview.iter().map(|(name, _)| *name).collect(),
        "index.html loads a script this guard does not scan (or no longer loads one it does)"
    );
    webview
        .into_iter()
        .flat_map(|(file, js)| calls_in(file, js))
        .collect()
}

/// Where a call takes a variable, not a literal: each wrapper's definition, and the `invoke`
/// inside it. Every other `invoke(`, `sendJudgement(` or `superuserAct(` must name its
/// command as a `"literal"`.
const VARIABLE_CALLS: [(&str, &str); 4] = [
    ("link.js", "sendJudgement(command)"),
    ("link.js", "invoke(command,"),
    ("superuser.js", "superuserAct(command, args)"),
    ("superuser.js", "invoke(command,"),
];

/// The command names one script calls. A call this cannot read the name of (a single-quoted or
/// template name, a name on the next line, a new wrapper) fails rather than being skipped: a
/// skipped call is a command neither direction of the guard sees.
fn calls_in<'a>(file: &str, js: &'a str) -> Vec<&'a str> {
    let mut called = Vec::new();
    for opener in ["invoke(", "sendJudgement(", "superuserAct("] {
        for (at, _) in js.match_indices(opener) {
            let rest = &js[at + opener.len()..];
            if let Some(name) = rest.strip_prefix('"') {
                called.push(&name[..name.find('"').expect("the name closes")]);
            } else {
                let here = &js[at..];
                assert!(
                    VARIABLE_CALLS
                        .iter()
                        .any(|(known, shape)| *known == file && here.starts_with(shape)),
                    "{file}: a call this guard cannot read the command name of: {}",
                    &here[..here.find('\n').unwrap_or(here.len())]
                );
            }
        }
    }
    called
}

/// Every command the webview calls is one `main.rs` registers. A command missing from
/// `generate_handler!` compiles, passes every `*_impl` test, and fails only in the running window
/// — where the call rejects and its surface says it "could not reach its backend". (Found adding
/// `duplicate_banner`, #745: nothing pinned its registration.)
#[test]
fn every_command_the_webview_calls_is_registered() {
    let registered = registered();
    let called = webview_calls();
    assert!(called.contains("duplicate_banner") && called.contains("record_different_people"));
    assert!(
        called.contains("judge_key"),
        "superuserAct's calls are read"
    );
    for name in called {
        assert!(
            registered.contains(name),
            "the webview calls `{name}`, main.rs never registers it"
        );
    }
}

/// Every command `main.rs` registers is one the webview calls (#1139). Delete a command whose
/// caller is gone, with the tests that exist only for it; never keep it "in case".
#[test]
fn every_registered_command_is_one_the_webview_calls() {
    let called = webview_calls();
    let uncalled: Vec<&str> = registered()
        .into_iter()
        .filter(|name| !called.contains(name))
        .collect();
    assert!(
        uncalled.is_empty(),
        "main.rs registers commands the webview never calls: {uncalled:?}"
    );
}

/// The parsers refuse a shape they cannot read instead of skipping it (PR #1145 review).
#[test]
#[should_panic(expected = "cannot read the command name of")]
fn a_call_whose_name_is_not_a_literal_fails() {
    calls_in("x.js", "invoke('med_x', {})");
}

#[test]
#[should_panic(expected = "cannot read the command name of")]
fn a_new_wrapper_invoking_a_variable_fails() {
    calls_in("x.js", "function fooAct(c) { return invoke(c, {}); }");
}

#[test]
#[should_panic(expected = "row this guard cannot read")]
fn a_handler_row_with_a_trailing_comment_fails() {
    rows_of("generate_handler![\n    med_x::foo, // why\n");
}

#[test]
#[should_panic(expected = "row this guard cannot read")]
fn a_handler_row_without_a_comma_fails() {
    rows_of("generate_handler![\n    med_x::foo,\n    med_x::bar\n");
}
