//! Tests for `commands.rs`, the chart surface's commands (moved out of line, #793).
//!
//! Two kinds live here: the wrong-chart pins (each chart command acts only on the chart
//! on SCREEN, tested against `AppState::mock`) and `main.js`'s drift guard. The guard's
//! helper `fields_read_in` is imported by other modules' guards as
//! `crate::commands::tests::fields_read_in`; the `#[path]` declaration in `commands.rs`
//! keeps that module path, so moving the file changed no importer.
use super::*;
use crate::chart_set::MemberLine;
use std::collections::BTreeSet;

/// Every field name `main.js` reads off a payload, in source order of appearance.
///
/// Extracted by scanning for `<binding>.<identifier>` rather than by hand, so the guard
/// below cannot rot into a list nobody updates.
fn fields_read_by_the_webview(binding: &str) -> BTreeSet<String> {
    // chart_stale.js was split out of main.js (#797) and reads the same `med_list` pane.
    let mut read = fields_read_in(include_str!("../src-ui/main.js"), binding);
    read.extend(fields_read_in(
        include_str!("../src-ui/chart_stale.js"),
        binding,
    ));
    // holds.js draws each row's W and sends Withhold/Release (#832), off the same rows.
    read.extend(fields_read_in(include_str!("../src-ui/holds.js"), binding));
    read
}

/// Every field `js` reads off `binding` — the scanner itself, shared by every webview file's
/// drift guard (`crate::commands::tests::fields_read_in`: the front door, link, duplicates and
/// worklist each scan their own `.js`).
pub(crate) fn fields_read_in(js: &str, binding: &str) -> BTreeSet<String> {
    let needle = format!("{binding}.");
    let mut found = BTreeSet::new();
    for (index, _) in js.match_indices(&needle) {
        // Skip `foo.bar` where `foo` is really the tail of a longer identifier
        // (e.g. `lock` inside `unlock.`), which would attribute a field to the wrong
        // payload.
        let preceded_by_word = js[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if preceded_by_word {
            continue;
        }
        let rest = &js[index + needle.len()..];
        let field: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !field.is_empty() {
            found.insert(field);
        }
    }
    found
}

/// The keys a serialized payload actually offers.
fn serialized_keys(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("payload must serialize to an object")
        .keys()
        .cloned()
        .collect()
}

/// THE DRIFT GUARD THIS FILE MOST NEEDS.
///
/// The webview is plain JavaScript with no type checking (the deliberate no-bundler
/// choice), so a Rust field rename does not break the build — it silently produces
/// `undefined` on screen. For most fields that is a visible cosmetic bug. For
/// `missing_message` it is not: `undefined` is falsy, the warning element stays hidden,
/// and the chart claims completeness it does not have. That is the exact ADR-0060
/// failure this slice exists to prevent, arriving through a typo.
///
/// So: every field the JS reads must exist in the payload the Rust actually sends.
#[test]
fn the_webview_reads_no_field_the_backend_does_not_send() {
    let view =
        cairn_gui_tab_medications::build_view(&cairn_medication_view::fixtures::sample_chart());
    let view_json = serde_json::to_value(&view).unwrap();
    let row_json = view_json["rows"][0].clone();
    // holds.js names each of a row's holds `hold`; the fixture chart carries holds (#832).
    let hold_json = view_json["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|row| row["holds"].as_array().and_then(|holds| holds.first()))
        .expect("the fixture chart carries a hold")
        .clone();
    // `med_list` sends the view INSIDE a pane, beside the linked charts' member lines. The
    // JS names the outer payload `pane`, the view `view` and each line `member`.
    let that_link = crate::link::record_links::RecordLinkView {
        low: String::new(),
        high: String::new(),
        text: String::new(),
        kind: crate::link::record_links::PairKind::Link,
        attested: false,
        button: String::new(),
    };
    let member = MemberLine {
        patient_id: String::new(),
        name: String::new(),
        born: String::new(),
        text: String::new(),
    };
    let pane_json = serde_json::to_value(ChartPane {
        list: view,
        members: vec![member.clone()],
        members_error: None,
        links: vec![that_link.clone()],
        links_error: None,
        duplicates: crate::duplicates::view::DuplicateSection::checked_none_open(),
        set_change: None,
    })
    .unwrap();
    let member_json = serde_json::to_value(&member).unwrap();

    let sign_off = serde_json::to_value(SignOffReport {
        signed: 0,
        failed: vec![],
        text: String::new(),
        behind_the_record: None,
    })
    .unwrap();
    let cease = serde_json::to_value(CeaseReport {
        ceased: 0,
        failed: vec![],
        text: String::new(),
        behind_the_record: None,
    })
    .unwrap();
    let hold_report = serde_json::to_value(crate::hold::HoldReport {
        text: String::new(),
        behind_the_record: None,
    })
    .unwrap();
    let lock = serde_json::to_value(LockState {
        held_by: None,
        key_hold_secs: 0,
        mock: false,
    })
    .unwrap();

    // `report` names both outcome payloads in the JS, so the union is the honest
    // comparison: either command may be the one that supplies a given field.
    let mut report_keys = serialized_keys(&sign_off);
    report_keys.extend(serialized_keys(&cease));
    report_keys.extend(serialized_keys(&hold_report));

    for (binding, available) in [
        ("pane", serialized_keys(&pane_json)),
        ("member", serialized_keys(&member_json)),
        (
            "recordLink",
            serialized_keys(&serde_json::to_value(&that_link).unwrap()),
        ),
        ("view", serialized_keys(&view_json)),
        ("row", serialized_keys(&row_json)),
        ("hold", serialized_keys(&hold_json)),
        ("report", report_keys),
        ("lock", serialized_keys(&lock)),
    ] {
        for field in fields_read_by_the_webview(binding) {
            assert!(
                available.contains(&field),
                "a webview script (main.js, chart_stale.js or holds.js) reads \
                 `{binding}.{field}`, which the backend does not send. Available: {available:?}"
            );
        }
    }
}

// ---- Each chart command acts only on the chart on SCREEN (PR #674 review, Critical
// #1). `AppState::displayed_patient` is tested on its own in `funnel::window`; these pin
// that every command actually CALLS it — a refactor back to "whatever chart is open" would
// otherwise stay green while signing chart B on the strength of a review of chart A.

fn open_on_the_fixture_chart() -> (AppState, String) {
    let fixture: Uuid = cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap();
    (AppState::mock(Some(fixture)), fixture.to_string())
}

fn another_chart() -> String {
    Uuid::from_u128(424_242).to_string()
}

#[tokio::test]
async fn the_medication_list_reads_only_the_chart_on_screen() {
    let (state, on_screen) = open_on_the_fixture_chart();
    med_list_impl(&state, &on_screen, None)
        .await
        .expect("the chart on screen is read");
    let err = med_list_impl(&state, &another_chart(), None)
        .await
        .unwrap_err();
    assert!(err.contains("not the chart"), "{err}");
}

/// The first read of a chart (and any caller that does not compare) sends no seen set, and
/// a re-read with nothing to compare says nothing (#797): never a guessed change.
#[tokio::test]
async fn med_list_says_nothing_without_a_seen_set() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let pane = med_list_impl(&state, &on_screen, None).await.unwrap();
    assert_eq!(pane.set_change, None);
    let same = med_list_impl(&state, &on_screen, Some(vec![on_screen.clone()]))
        .await
        .unwrap();
    assert_eq!(same.set_change, None, "an unchanged set says nothing");
}

/// A re-read whose seen set held a chart the record no longer holds names it (#797). The
/// mock has no database to read the departed chart's identity, so it is named by id — said
/// unread, never dropped, and with why (#881).
#[tokio::test]
async fn med_list_names_a_change_from_the_seen_set() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let gone = Uuid::from_u128(99);
    let pane = med_list_impl(
        &state,
        &on_screen,
        Some(vec![on_screen.clone(), gone.to_string()]),
    )
    .await
    .unwrap();
    assert_eq!(
        pane.set_change,
        Some(format!(
            "This record changed while it was on screen. No longer part of it: chart {gone} \
             (its name could not be read here). Why the name could not be read: there is no \
             database to read the linked charts' identities from."
        ))
    );
}

/// Checked FIRST, before fixture mode or the key: whatever else would refuse, a sign-off
/// naming a chart that is not on screen is refused for THAT reason.
#[tokio::test]
async fn sign_off_refuses_a_chart_that_is_not_on_screen() {
    let (state, _) = open_on_the_fixture_chart();
    let err = sign_off_impl(&state, &another_chart(), vec![another_chart()], None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"));
}

#[tokio::test]
async fn cease_refuses_a_chart_that_is_not_on_screen() {
    let (state, _) = open_on_the_fixture_chart();
    let err = cease_impl(
        &state,
        &another_chart(),
        vec![another_chart()],
        &another_chart(),
        "allergy",
        None,
    )
    .await
    .unwrap_err();
    assert!(err.text.contains("not the chart"));
}

/// With the right chart, fixture mode's own refusal is what the clinician reads.
#[tokio::test]
async fn a_write_on_the_chart_on_screen_meets_the_fixture_refusal() {
    let (state, on_screen) = open_on_the_fixture_chart();
    let err = sign_off_impl(&state, &on_screen, vec![on_screen.clone()], None)
        .await
        .unwrap_err();
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
    assert!(err.text.contains("fixture mode"));
}

/// The other direction for the combined read (ADR-0076). Each of these, dropped by the JS,
/// fails SILENTLY in a dangerous way: without `charts` every sign-off names no set and is
/// refused (safe, but the gesture is dead); without `members` a combined record's header
/// reads as a single chart while its rows come from several; without `source` a line
/// recorded on another chart looks like one recorded on this one.
#[test]
fn the_webview_reads_the_linked_set_and_each_rows_source() {
    assert!(fields_read_by_the_webview("view").contains("charts"));
    assert!(fields_read_by_the_webview("pane").contains("members"));
    assert!(fields_read_by_the_webview("pane").contains("members_error"));
    assert!(fields_read_by_the_webview("member").contains("text"));
    assert!(fields_read_by_the_webview("row").contains("source"));
    // #797: dropped, a colleague's link or unlink landing while the list was on screen would be
    // redrawn without a word — the record silently grows or shrinks under the clinician.
    assert!(fields_read_by_the_webview("pane").contains("set_change"));
}

/// The other direction, for the two fields where silence is the dangerous failure.
///
/// A field the backend sends and the JS ignores is usually harmless — but not these
/// two: dropping either means a chart that is missing a drug, or a line that was
/// deliberately left unsigned, renders as if everything were fine (ADR-0060 decision 2).
#[test]
fn the_webview_reads_both_partial_completion_reports() {
    let read = fields_read_by_the_webview("view");
    assert!(read.contains("missing_message"), "got: {read:?}");
    assert!(read.contains("withheld_message"), "got: {read:?}");
    // #848: a drug of this patient's on another chart's line has no row to say it on.
    assert!(read.contains("elsewhere_message"), "got: {read:?}");

    // A sign-off's reports reach the clinician inside Rust's outcome line (#818,
    // `write_failure::sign_off_outcome`, which appends every note it is handed), so the webview
    // must say `text`, and `sign_off_impl` must hand it the withheld and missing reports.
    let reported = fields_read_by_the_webview("report");
    assert!(reported.contains("text"), "got: {reported:?}");
    assert!(reported.contains("behind_the_record"), "got: {reported:?}");
    let sign_off = crate::source_guard::rust_fn(include_str!("commands.rs"), "sign_off_impl");
    for fed in [
        "withheld_report(&withheld,",
        "missing_report(",
        "&groups_missing_from_chart,",
        // #849: this patient's entries on another chart's line, said after the gesture too.
        "elsewhere_report(&elsewhere)",
        "&notes)",
    ] {
        assert!(sign_off.contains(fed), "{fed} not in sign_off_impl");
    }
}

/// #820: `sign_off_impl` names every `SignOffOutcome` field, none behind `..`, so a field added
/// to the outcome breaks this build rather than dropping out of the report. `..` is the one-token
/// fix for that compile error, which is why the source is read. The CLI's twin is the root tree's
/// `crates/cairn-node/tests/signoff_readers_name_every_field.rs`.
#[test]
fn sign_off_impl_names_every_outcome_field() {
    let sign_off = crate::source_guard::rust_fn(include_str!("commands.rs"), "sign_off_impl");
    let start = sign_off
        .find("SignOffOutcome {")
        .expect("sign_off_impl destructures the outcome");
    let pattern = &sign_off[start..start + sign_off[start..].find("} = outcome").unwrap()];
    let hides_a_field = pattern
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .any(|l| l.contains(".."));
    assert!(
        !hides_a_field,
        "name the field and bind it to `_` with a comment saying why it is not shown: {pattern}"
    );
}

/// Every command the webview calls is one `main.rs` registers. A command missing from
/// `generate_handler!` compiles, passes every `*_impl` test, and fails only in the running window
/// — where the call rejects and its surface says it "could not reach its backend". (Found adding
/// `duplicate_banner`, #745: nothing pinned its registration.) Both call shapes are read: a
/// literal `invoke("name"`, and the commands passed by name to link.js's `sendJudgement`.
#[test]
fn every_command_the_webview_calls_is_registered() {
    let main = include_str!("main.rs");
    let start = main
        .find("generate_handler![")
        .expect("main.rs registers commands");
    let block = &main[start..start + main[start..].find(']').expect("the list closes")];
    let registered: BTreeSet<&str> = block
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().strip_suffix(','))
        .map(|path| path.rsplit("::").next().unwrap_or(path))
        .collect();
    // `include_str!` needs literal paths, so the list is written out — and checked against the
    // scripts index.html actually loads, so a new script cannot go unscanned (PR #798 review).
    let webview = [
        ("waiting.js", include_str!("../src-ui/waiting.js")),
        ("main.js", include_str!("../src-ui/main.js")),
        ("signer.js", include_str!("../src-ui/signer.js")),
        ("stop_reasons.js", include_str!("../src-ui/stop_reasons.js")),
        ("holds.js", include_str!("../src-ui/holds.js")),
        ("chart_stale.js", include_str!("../src-ui/chart_stale.js")),
        ("duplicates.js", include_str!("../src-ui/duplicates.js")),
        ("worklist.js", include_str!("../src-ui/worklist.js")),
        ("funnel.js", include_str!("../src-ui/funnel.js")),
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
    let mut called = BTreeSet::new();
    for (_, js) in webview {
        for opener in ["invoke(\"", "sendJudgement(\""] {
            for (at, _) in js.match_indices(opener) {
                let rest = &js[at + opener.len()..];
                called.insert(&rest[..rest.find('"').expect("the name closes")]);
            }
        }
    }
    assert!(called.contains("duplicate_banner") && called.contains("record_different_people"));
    for name in called {
        assert!(
            registered.contains(name),
            "the webview calls `{name}`, main.rs never registers it"
        );
    }
}

/// The lock screen tells "no terminal passphrase is set" apart from "wrong", so an installer is
/// never locked out with no explanation. The words are the webview's contract (`signer.js`).
#[test]
fn the_terminal_verdict_tells_not_set_from_wrong() {
    let phc = cairn_node::vault::terminal_phc("shared pass").unwrap();
    assert_eq!(
        terminal_verdict(Some(&phc), "shared pass"),
        TerminalUnlock::Unlocked
    );
    assert_eq!(terminal_verdict(Some(&phc), "other"), TerminalUnlock::Wrong);
    assert_eq!(terminal_verdict(None, "anything"), TerminalUnlock::NotSet);
    let words = |v| serde_json::to_string(&v).unwrap();
    assert_eq!(words(TerminalUnlock::Unlocked), "\"unlocked\"");
    assert_eq!(words(TerminalUnlock::Wrong), "\"wrong\"");
    assert_eq!(words(TerminalUnlock::NotSet), "\"not-set\"");
}

/// A node fault while reading the terminal PHC is NOT a wrong passphrase: the operator would
/// retype a correct passphrase and conclude they had forgotten it. It gets its own word.
#[test]
fn a_failed_phc_read_is_a_fault_never_wrong() {
    let fault: Result<Option<String>, ()> = Err(());
    assert_eq!(
        terminal_read_verdict(fault, "anything"),
        TerminalUnlock::Fault
    );
    assert_eq!(
        serde_json::to_string(&TerminalUnlock::Fault).unwrap(),
        "\"fault\""
    );
    let phc = cairn_node::vault::terminal_phc("shared pass").unwrap();
    let read: Result<Option<String>, ()> = Ok(Some(phc));
    assert_eq!(terminal_read_verdict(read, "other"), TerminalUnlock::Wrong);
}
