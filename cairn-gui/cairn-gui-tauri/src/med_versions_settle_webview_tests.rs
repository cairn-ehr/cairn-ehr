//! Batch charting slice 2b, Task 7: settling a set-aside version from under its line
//! (`med_versions_settle.js`, `med_list_versions.js`'s `versionRows`, `chart_versions.rs`).
//!
//! Two halves. What a version SAYS and whether it is offered the buttons is Rust's
//! (`chart_versions::attach_versions`), so it is tested here on Rust's own answer: a version of
//! the opened chart offers both, a linked chart's none, a restored one none, a checked one keeps
//! both. What the webview DOES with that answer is pinned on the scripts' source, the rig of
//! `med_list_versions_webview_tests.rs` (no JS test rig: no npm, by design) — and VISIBILITY, not
//! text: each element sits in a row the script draws un-hidden, under the line's `<details>`, and
//! no stylesheet rule hides it. The walk itself (open the versions, press each button, sign the
//! check, see the words beside the version) was also driven headlessly with a stub `invoke`,
//! asserting computed display and hidden ancestors (task report).
use crate::chart_versions::{attach_versions, VersionsRead};
use crate::commands::tests::fields_read_in;
use crate::med_chart_webview_tests::not_hidden_by_css;
use crate::source_guard::{js_const, js_fn, position};
use cairn_gui_tab_medications::{build_view, MedListRowView, VersionView};
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::MedicationStatus;
use cairn_node::medication::set_aside_read::SetAsideRow;
use cairn_node::medication::version_settle_read::{Settlement, SettlementRow};
use chrono::{Local, TimeZone, Utc};
use uuid::Uuid;

const SETTLE_JS: &str = include_str!("../src-ui/med_versions_settle.js");
const VERSIONS_JS: &str = include_str!("../src-ui/med_list_versions.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The opened chart of `sample_chart` (every member thread lives on it).
fn opened() -> Uuid {
    Uuid::from_u128(1)
}

/// A version beside thread `beside`, on `chart`.
fn version(id: u128, beside: u128, chart: Uuid) -> SetAsideRow {
    SetAsideRow {
        version_id: Uuid::from_u128(id),
        alternative_to: Uuid::from_u128(beside),
        patient_id: chart,
        drug_line: "metoprolol 25 mg (external-record)".into(),
        source: "GP letter".into(),
        why: None,
        t_recorded: Utc::now(),
    }
}

fn settled(v: &SetAsideRow, settlement: Settlement, who: Option<&str>) -> SettlementRow {
    SettlementRow {
        version_id: v.version_id,
        patient_id: v.patient_id,
        settlement,
        who: who.map(Into::into),
        at: Utc.with_ymd_and_hms(2026, 11, 13, 22, 30, 0).unwrap(),
    }
}

/// `sample_chart`'s rows as the pane shows them, with `read` attached (opened chart 1).
fn pane(read: &VersionsRead) -> Vec<MedListRowView> {
    let chart = sample_chart();
    let mut shown = build_view(&chart).rows;
    attach_versions(&mut shown, &chart.rows, read, opened(), &Local::now());
    shown
}

/// The shown row whose member thread is `thread`.
fn line_of(shown: &[MedListRowView], thread: u128) -> &MedListRowView {
    let chart = sample_chart();
    let at = chart
        .rows
        .iter()
        .position(|r| {
            r.members
                .iter()
                .any(|m| m.medication_id.as_u128() == thread)
        })
        .expect("a line with that thread");
    &shown[at]
}

/// No dialog stands anywhere in this script (principle 3).
fn no_dialog() {
    for banned in ["confirm(", "alert(", "prompt(", "<dialog", "showModal"] {
        assert!(!SETTLE_JS.contains(banned), "{banned}");
    }
}

/// Each version's `li` carries its settle lines and, through the hook, its buttons; the row the
/// versions sit in is never hidden, nor is any element the clinician reads or presses.
fn drawn_in_the_versions_row() {
    let rows = js_fn(VERSIONS_JS, "versionRows");
    assert!(
        rows.contains(
            "list.append(...row.versions.map((version) => versionItem(version, row.group_id)));"
        ),
        "{rows}"
    );
    assert!(!rows.contains(".hidden"), "{rows}");
    let item = js_fn(VERSIONS_JS, "versionItem");
    let line = position(item, "const li = cell(\"li\", version.line);");
    let states = position(item, "for (const words of version.settled) li.append(cell(\"span\", words, { class: \"med-version-settled\" }));");
    let hook = position(item, "li.append(...settleControls(version, groupId));");
    assert!(line < states && states < hook, "{item}");
    for class in [
        ".med-versions",
        ".med-version-settled",
        ".med-version-act",
        ".med-version-said",
        ".med-version-check",
    ] {
        not_hidden_by_css(class);
    }
}

/// A version of the OPENED chart, never settled, offers both buttons, each named for its version
/// (a screen reader hears WHICH version, not a list of identical buttons), each down while a Sign
/// or a list act is in flight.
#[test]
fn a_version_of_the_opened_chart_shows_both_buttons() {
    let v = version(500, 10, opened());
    let shown = pane(&(vec![v], vec![]));
    let version = &line_of(&shown, 10).versions[0];
    assert!(version.can_settle && !version.restored, "{version:?}");
    assert!(version.settled.is_empty());
    assert_eq!(version.version_id, Uuid::from_u128(500).to_string());
    assert_eq!(js_const(SETTLE_JS, "RESTORE"), "Use this version instead");
    assert_eq!(js_const(SETTLE_JS, "CHECK"), "Checked: not current");
    let controls = js_fn(SETTLE_JS, "settleControls");
    let none = position(controls, "if (!version.can_settle) return [said];");
    let restore = position(controls, "settleButton(RESTORE, \"restore\", version,");
    let check = position(controls, "settleButton(CHECK, \"check\", version,");
    assert!(none < restore && restore < check, "{controls}");
    let button = js_fn(SETTLE_JS, "settleButton");
    for step in [
        "\"aria-label\": label + \": \" + version.line",
        "button.disabled = settleBlocked(act);",
    ] {
        assert!(button.contains(step), "{step}: {button}");
    }
    assert!(!button.contains("hidden"), "{button}");
    let blocked = js_fn(SETTLE_JS, "settleBlocked");
    for step in [
        "if (medSigning) return true;",
        "return act === \"restore\" && (medList === null || medList.elsewhere !== null);",
    ] {
        assert!(blocked.contains(step), "{step}: {blocked}");
    }
    // Re-decided whenever the list's own controls are (drawMedList → drawListVersions).
    let sync = js_fn(SETTLE_JS, "syncSettleButtons");
    assert!(
        sync.contains("button.disabled = settleBlocked(button.dataset.settle);"),
        "{sync}"
    );
    assert!(js_fn(VERSIONS_JS, "drawListVersions").contains("syncSettleButtons();"));
    drawn_in_the_versions_row();
    no_dialog();
}

/// A version on a LINKED chart (its own `patient_id` is not the opened chart) offers no button:
/// settling it is that chart's act (spec §2 non-goal) — and Rust refuses it anyway (`VERSION_GONE`).
#[test]
fn a_linked_charts_version_shows_none() {
    let other = Uuid::from_u128(2);
    let shown = pane(&(vec![version(501, 10, other)], vec![]));
    let version = &line_of(&shown, 10).versions[0];
    assert!(!version.can_settle, "{version:?}");
    assert!(js_fn(SETTLE_JS, "settleControls").contains("if (!version.can_settle) return [said];"));
}

/// A restored version reads "restored by {who}, {when}" as a second line of its `li`, and offers
/// no button (it is the current line now, under its own line).
#[test]
fn a_restored_version_shows_restored_and_no_buttons() {
    let v = version(502, 40, opened());
    let restore = Settlement::Restored {
        line: Uuid::from_u128(10),
        replaced_line: "metformin 500 mg".into(),
    };
    let s = settled(&v, restore, Some("Dr Bea Smith"));
    let shown = pane(&(vec![v], vec![s]));
    let version = &line_of(&shown, 40).versions[0];
    assert!(version.restored && !version.can_settle && !version.open);
    assert_eq!(version.settled.len(), 1);
    assert!(
        version.settled[0].starts_with("restored by Dr Bea Smith, "),
        "{:?}",
        version.settled
    );
    drawn_in_the_versions_row();
}

/// A checked version names who checked it, when, from what and why — and keeps BOTH buttons: a
/// check locks nothing (spec §3.4). It is no longer "to check". An unnamed signer is said so.
#[test]
fn a_checked_version_shows_who_and_keeps_both_buttons() {
    let v = version(503, 10, opened());
    let check = |why: Option<&str>| Settlement::Checked {
        source: "Pharmacy".into(),
        why: why.map(Into::into),
    };
    let first = settled(&v, check(Some("old packet")), Some("Dr Bea Smith"));
    let second = settled(&v, check(None), None);
    let shown = pane(&(vec![v], vec![first, second]));
    let version = &line_of(&shown, 10).versions[0];
    assert!(version.can_settle && !version.open && !version.restored);
    assert_eq!(version.settled.len(), 2, "one line per check, oldest first");
    assert!(version.settled[0].starts_with("checked by Dr Bea Smith, "));
    assert!(version.settled[0].ends_with(": not current — from Pharmacy — old packet"));
    assert!(version.settled[1].starts_with("checked by signer not known, "));
    assert!(version.settled[1].ends_with(": not current — from Pharmacy"));
    drawn_in_the_versions_row();
}

/// The summary: "{n} other version(s)", then ", {u} to check" while any version is open — Rust's
/// `open` (never checked nor restored, its line current), counted here, never re-derived.
#[test]
fn the_summary_counts_versions_to_check() {
    let open = version(504, 10, opened());
    let checked = version(505, 10, opened());
    let beside_stopped = version(506, 40, opened());
    let check = Settlement::Checked {
        source: "Patient".into(),
        why: None,
    };
    let s = settled(&checked, check, None);
    let shown = pane(&(vec![open, checked, beside_stopped], vec![s]));
    let opens: Vec<bool> = line_of(&shown, 10)
        .versions
        .iter()
        .map(|v| v.open)
        .collect();
    assert_eq!(opens, [true, false]);
    let stopped = sample_chart()
        .rows
        .into_iter()
        .find(|r| r.members.iter().any(|m| m.medication_id.as_u128() == 40));
    assert_eq!(stopped.unwrap().status, MedicationStatus::Ceased);
    assert!(
        !line_of(&shown, 40).versions[0].open,
        "beside a stopped line"
    );
    let start = position(VERSIONS_JS, "const TO_CHECK =");
    let decl = &VERSIONS_JS[start..start + position(&VERSIONS_JS[start..], ";\n")];
    let pieces: Vec<&str> = decl.split('"').skip(1).step_by(2).collect();
    assert_eq!(
        format!(
            "2{}{}{}",
            js_const(VERSIONS_JS, "OTHER_VERSIONS"),
            pieces[0],
            1
        ) + pieces[1],
        "2 other version(s), 1 to check"
    );
    let rows = js_fn(VERSIONS_JS, "versionRows");
    for step in [
        "const toCheck = row.versions.filter((version) => version.open).length;",
        "const said = row.versions.length + OTHER_VERSIONS + (toCheck > 0 ? TO_CHECK[0] + toCheck + TO_CHECK[1] : \"\");",
        "box.append(cell(\"summary\", said), list);",
    ] {
        assert!(rows.contains(step), "{step}: {rows}");
    }
}

/// "Use this version instead": Rust adds the pre-filled restore line (`med_list_restore_open`, on
/// the opened chart, over the charts on screen); the list it answers is drawn and the ONE editor
/// opens on the new line. A refusal (the version gone, the list on another terminal) is said
/// beside the version. Nothing is signed: the line waits for the list's Sign.
#[test]
fn restore_opens_the_prefilled_line_in_the_editor() {
    let press = js_fn(SETTLE_JS, "pressRestore");
    let guard = position(press, "if (settleBlocked(\"restore\")) return;");
    let sent = position(press, "invoke(\"med_list_restore_open\", {");
    assert!(guard < sent, "{press}");
    for arg in [
        "patientId: patient,",
        "charts: renderedCharts,",
        "versionId: version.version_id,",
    ] {
        assert!(press.contains(arg), "{arg}: {press}");
    }
    let refused = position(
        press,
        "sayBeside(version.version_id, failureText(failure));",
    );
    let drawn = position(press, "drawMedRead(opened.list, patient);");
    // An add that landed with no list read again is said beside the version, never drawn.
    let unread = position(
        press,
        "if (!opened.list) return sayBeside(version.version_id, opened.said);",
    );
    assert!(unread < drawn, "{press}");
    let found = position(
        press,
        "const line = medLines.find((l) => l.id === opened.line_id);",
    );
    let edit = position(press, "if (line) editMedLine(line);");
    assert!(
        sent < refused && refused < drawn && drawn < found && found < edit,
        "{press}"
    );
    // The editor refuses while a list act is in flight: the press's own flag is down first.
    assert!(position(press, "medSigning = false;") < edit, "{press}");
    assert!(
        position(
            press,
            "if (revision !== medListRevision || patient !== renderedPatient) return;"
        ) < drawn
    );
    // The restore line's mark, Rust's words, drawn on the unsigned list as given — kept while
    // the line is typed (its view is down then).
    let draw = js_fn(VERSIONS_JS, "drawLineVersions");
    for step in [
        "if (line.view) line.restoresWords = line.view.restores_words || \"\";",
        "setMessage(own.restores, line.restoresWords || \"\");",
    ] {
        assert!(draw.contains(step), "{step}: {draw}");
    }
    assert!(js_fn(VERSIONS_JS, "versionControls")
        .contains("restores: cell(\"p\", \"\", { class: \"med-list-restores\" }),"));
    not_hidden_by_css(".med-list-restores");
    // Loaded after the scripts whose functions it calls.
    for before in ["med_list.js", "med_list_sign.js", "med_list_versions.js"] {
        assert!(
            position(INDEX_HTML, &format!("src=\"{before}\""))
                < position(INDEX_HTML, "src=\"med_versions_settle.js\""),
            "{before}"
        );
    }
    no_dialog();
}

/// "Checked: not current" opens ONE inline row under its version — From: (2a's presets), the
/// document or person, Why, its own Sign and Cancel — and signs nothing until that Sign. The
/// Sign sends the row's words with the strip's credential, closes the row on success and draws
/// the pane Rust read again; a refusal is said beside the version, the words kept. Its typed
/// words outlive a re-read of the pane (`settleCheck`), never the chart.
#[test]
fn the_check_row_signs_with_its_source_and_why() {
    let open = js_fn(SETTLE_JS, "openCheck");
    assert!(!open.contains("invoke("), "opening signs nothing: {open}");
    assert!(open.contains("settleCheck = { versionId: version.version_id, preset: \"\", detail: \"\", why: \"\" };"), "{open}");
    let row = js_fn(SETTLE_JS, "checkRow");
    for step in [
        "presetOptions(preset);",
        "const fromLabel = cell(\"label\", FROM_LABEL + \" \");",
        "const whyLabel = cell(\"label\", WHY_LABEL + \" \");",
        "settleButton(CHECK_SIGN, \"sign\", version, () => pressCheckSign(version, groupId))",
        "form.append(fromLabel, detail, whyLabel, sign, cancel);",
    ] {
        assert!(row.contains(step), "{step}: {row}");
    }
    assert_eq!(js_const(SETTLE_JS, "CHECK_SIGN"), "Sign");
    let sign = js_fn(SETTLE_JS, "signCheck");
    let sent = position(sign, "invoke(\"med_version_check\", {");
    for arg in [
        "patientId: patient,",
        "charts: charts,",
        "versionId: version.version_id,",
        "preset: words.preset || null,",
        "detail: words.detail.trim() || null,",
        "why: words.why.trim() || null,",
        "credential: signerCredential(),",
    ] {
        assert!(sign.contains(arg), "{arg}: {sign}");
    }
    let closed = position(sign, "settleCheck = null;");
    let drawn = position(sign, "drawSignedTable(answer, patient);");
    assert!(sent < closed && closed < drawn, "{sign}");
    let said = position(sign, "sayBeside(version.version_id, failureText(failure));");
    assert!(sent < said && said < closed, "{sign}");
    assert!(sign.contains("clearSigner(true);") && sign.contains("clearSigner();"));
    // One press, one act, and a who's-here switch waits for it, as for the list's Sign.
    let press = js_fn(SETTLE_JS, "pressCheckSign");
    for step in [
        "if (signInFlight() || settleBlocked(\"sign\")) return;",
        "medListSignInFlight = true;",
        "medListSigning = signCheck(version, groupId)",
    ] {
        assert!(press.contains(step), "{step}: {press}");
    }
    // Words beside the version: in its own `li`, kept for the next draw of the pane.
    let beside = js_fn(SETTLE_JS, "sayBeside");
    assert!(
        beside.contains("settleSaid.set(versionId, text);"),
        "{beside}"
    );
    let controls = js_fn(SETTLE_JS, "settleControls");
    assert!(
        controls.contains("const said = cell(\"p\", settleSaid.get(version.version_id) || \"\""),
        "{controls}"
    );
    assert!(
        controls.contains("said.hidden = !settleSaid.has(version.version_id);"),
        "{controls}"
    );
    assert!(js_fn(MAIN_JS, "clearChart").contains("clearSettle();"));
    drawn_in_the_versions_row();
    no_dialog();
}

/// The line a restore charted says what it replaced, under it: "replaced: {old line} — {who},
/// {when}" (Rust's words; ruling R7: the version's companion), drawn as a row spanning the table.
#[test]
fn the_new_line_shows_what_it_replaced() {
    let v = version(507, 40, opened());
    let restore = Settlement::Restored {
        line: Uuid::from_u128(10),
        replaced_line: "metformin 500 mg (patient-reported)".into(),
    };
    let s = settled(&v, restore, Some("Dr Bea Smith"));
    let shown = pane(&(vec![v], vec![s]));
    let new = &line_of(&shown, 10).replaced;
    assert_eq!(new.len(), 1);
    assert!(
        new[0].starts_with("replaced: metformin 500 mg (patient-reported) — Dr Bea Smith, "),
        "{new:?}"
    );
    assert!(
        line_of(&shown, 40).replaced.is_empty(),
        "never under the line it stopped"
    );
    let render = js_fn(MAIN_JS, "renderRow");
    assert!(
        position(render, "rows.push(...replacedRows(row));")
            < position(render, "rows.push(...versionRows(row));")
    );
    let rows = js_fn(SETTLE_JS, "replacedRows");
    for step in [
        "return row.replaced.map((words) => {",
        "tr.setAttribute(\"data-replaced\", \"true\");",
        "td.colSpan = 5;",
    ] {
        assert!(rows.contains(step), "{step}: {rows}");
    }
    assert!(!rows.contains("hidden"), "{rows}");
    not_hidden_by_css("tr[data-replaced");
}

/// Every field these scripts read off a charted row and its versions is one Rust sends.
#[test]
fn every_field_read_is_one_rust_sends() {
    let chart = sample_chart();
    let row = serde_json::to_value(&build_view(&chart).rows[0]).unwrap();
    let js = format!("{SETTLE_JS}{VERSIONS_JS}");
    for field in fields_read_in(&js, "row") {
        assert!(row.get(&field).is_some(), "row.{field} is not sent: {row}");
    }
    let sent = serde_json::to_value(VersionView {
        line: "l".into(),
        version_id: "v".into(),
        settled: vec![],
        open: false,
        can_settle: true,
        restored: false,
    })
    .unwrap();
    for field in fields_read_in(&js, "version") {
        assert!(
            sent.get(&field).is_some(),
            "version.{field} is not sent: {sent}"
        );
    }
}
