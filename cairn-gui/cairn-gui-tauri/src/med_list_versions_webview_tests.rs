//! Batch charting slice 2a, Task 7: the webview half of "From:", a line's own source and "Why",
//! "Set aside" with its "Version of which line?" pick, the set-aside line, and a charted line's
//! versions collapsed under it (`med_list_versions.js`).
//!
//! The rig of `med_list_webview_tests.rs` (no JS test rig: no npm, by design): these read the
//! scripts' source and pin the CHOICE or ORDER a property is. VISIBILITY, not only text: each
//! element the clinician must see sits in the pane the script un-hides, is un-hidden by the
//! script itself, and no stylesheet rule hides it; the behaviour was also walked headlessly in a
//! browser with a stub `invoke`, asserting computed display and hidden ancestors (task report).
use crate::commands::tests::fields_read_in;
use crate::med_chart_webview_tests::not_hidden_by_css;
use crate::med_list_webview_tests::{in_the_pane, LIST_JS};
use crate::source_guard::{js_const, js_fn, position};
use cairn_gui_tab_medications::{build_view, VersionView};
use cairn_medication_view::fixtures::sample_chart;
use cairn_node::medication::draft_versions::info_source_of;

const VERSIONS_JS: &str = include_str!("../src-ui/med_list_versions.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// No dialog stands anywhere in this script (principle 3).
fn no_dialog() {
    for banned in ["confirm(", "alert(", "prompt(", "<dialog", "showModal"] {
        assert!(!VERSIONS_JS.contains(banned), "{banned}");
    }
}

/// "From:" sits above the list, shown whenever a list is read, and offers exactly the six
/// presets, each tied to the category Rust's `info_source_of` gives it (one table, pinned both
/// ways). A change first saves every line typed before it (so it never stamps them, ruling R12),
/// then saves "From:" under the writer the list was READ under, over the charts on screen, and
/// draws the list Rust answers (whose new lines then carry the source, Rust's `stamp`).
#[test]
fn from_sets_the_next_lines_source() {
    assert_eq!(js_const(VERSIONS_JS, "FROM_LABEL"), "From:");
    for id in [
        "med-list-from",
        "med-list-from-preset",
        "med-list-from-detail",
    ] {
        in_the_pane(id);
    }
    assert!(INDEX_HTML.contains("<label for=\"med-list-from-preset\">From:</label>"));
    assert!(position(INDEX_HTML, "id=\"med-list-from\"") < position(INDEX_HTML, "id=\"med-list\""));
    not_hidden_by_css("#med-list-from");
    let draw = js_fn(VERSIONS_JS, "drawListVersions");
    assert!(
        draw.contains("el(\"med-list-from\").hidden = medList === null;"),
        "{draw}"
    );
    assert!(js_fn(LIST_JS, "drawMedList").contains("drawListVersions();"));
    // The table: six labels in the maintainer's order, each with Rust's category.
    let table: Vec<String> = {
        let pieces = js_const(VERSIONS_JS, "FROM_PRESETS");
        let start = position(VERSIONS_JS, "const FROM_PRESETS =");
        let decl = &VERSIONS_JS[start..start + position(&VERSIONS_JS[start..], "};\n")];
        assert!(!pieces.is_empty());
        decl.split('"')
            .skip(1)
            .step_by(2)
            .map(String::from)
            .collect()
    };
    let labels: Vec<&String> = table.iter().step_by(2).collect();
    assert_eq!(
        labels,
        [
            "Patient",
            "Carer / family",
            "GP letter",
            "Pharmacy",
            "Packets / Webster pack seen",
            "Other"
        ]
    );
    for pair in table.chunks(2) {
        assert_eq!(
            info_source_of(Some(&pair[0])),
            Some(pair[1].as_str()),
            "{pair:?}"
        );
    }
    // The save: lines first, then "From:" under the read writer, the answer drawn.
    let save = js_fn(VERSIONS_JS, "saveFrom");
    let flushed = position(save, "await flushMedLines();");
    let sent = position(save, "invoke(\"med_list_from_save\", {");
    assert!(flushed < sent, "{save}");
    assert!(
        position(
            save,
            "if (!ownsTheList(writer) || patient !== renderedPatient) return;"
        ) < sent
    );
    for arg in [
        "patientId: patient,",
        "charts: renderedCharts,",
        "writer: writer,",
    ] {
        assert!(save.contains(arg), "{arg}: {save}");
    }
    assert!(position(save, "if (revision === medListRevision && patient === renderedPatient) drawMedRead(fresh, patient);") > sent);
    for wire in [
        "el(\"med-list-from-preset\").addEventListener(\"change\", () => void saveFrom());",
        "el(\"med-list-from-detail\").addEventListener(\"change\", () => void saveFrom());",
    ] {
        assert!(VERSIONS_JS.contains(wire), "{wire}");
    }
    assert!(js_fn(LIST_JS, "drawMedRead").contains("from: fresh.from"));
    // The control shows what Rust read, never over what is being typed in it.
    assert!(
        draw.contains("if (document.activeElement !== preset) preset.value = from.preset || \"\";")
    );
}

/// Each line's "Why" (with its own "From:") is a `<details>` the script never opens: closed until
/// the clinician opens it, and never hidden. A typed why is saved as the line's own (it rides in
/// `line.content`, so every save sends it back). The line says its source and why without being
/// opened.
#[test]
fn the_why_field_is_closed_until_opened() {
    assert_eq!(js_const(VERSIONS_JS, "WHY_LABEL"), "Why");
    let make = js_fn(VERSIONS_JS, "versionControls");
    for step in [
        "const box = cell(\"details\", \"\", { class: \"med-list-why\" });",
        "box.append(cell(\"summary\", WHY_LABEL), fromLabel, detail, why);",
        "why.addEventListener(\"input\", () => ownChanged(line, { why: why.value }));",
        "r.li.append(own.source, own.setAside, own.which, box);",
    ] {
        assert!(make.contains(step), "{step}: {make}");
    }
    for opening in [".open = true", ".open=true", "open: true", "\"open\""] {
        assert!(!VERSIONS_JS.contains(opening), "{opening}");
    }
    let draw = js_fn(VERSIONS_JS, "drawLineVersions");
    assert!(
        !draw.contains("box.hidden"),
        "the Why is never hidden: {draw}"
    );
    assert!(
        draw.contains("setMessage(own.source, sourceWords(line.content));"),
        "{draw}"
    );
    not_hidden_by_css(".med-list-why");
    not_hidden_by_css(".med-list-source");
    // drawLine fills the controls last, after its own words and the offers.
    let line = js_fn(LIST_JS, "drawLine");
    assert!(position(line, "drawDelabelOffers(line);") < position(line, "drawLineVersions(line);"));
}

/// "Set aside" is offered only where Rust found a companion (`companions`, for the SAVED words:
/// none while the line is typed), or on a set-aside line, to undo it.
#[test]
fn set_aside_appears_only_with_a_companion() {
    assert_eq!(js_const(VERSIONS_JS, "SET_ASIDE"), "Set aside");
    let draw = js_fn(VERSIONS_JS, "drawLineVersions");
    for step in [
        "const saved = line.view && medEditingLine !== line;",
        "const marks = saved ? line.view.companions : [];",
        "own.setAside.hidden = !(aside || marks.length > 0);",
        "own.setAside.setAttribute(\"aria-pressed\", String(aside));",
    ] {
        assert!(draw.contains(step), "{step}: {draw}");
    }
    not_hidden_by_css(".med-list-set-aside");
}

/// One candidate is taken at the press; with two or more the pick asks which line, in Rust's
/// words for each (`companion_words`), as buttons under the line, never a dialog.
#[test]
fn two_candidates_ask_which_line() {
    assert_eq!(
        js_const(VERSIONS_JS, "WHICH_LINE"),
        "Version of which line?"
    );
    let press = js_fn(VERSIONS_JS, "pressSetAside");
    let one = position(
        press,
        "else if (marks.length === 1) ownChanged(line, { set_aside_of: marks[0] });",
    );
    let more = position(press, "else if (marks.length > 1) {");
    assert!(one < more, "{press}");
    assert!(
        press[more..].contains("line.picking = !line.picking;"),
        "{press}"
    );
    let draw = js_fn(VERSIONS_JS, "drawLineVersions");
    assert!(
        draw.contains("const words = saved ? line.view.companion_words : [];"),
        "{draw}"
    );
    assert!(
        draw.contains("own.which.hidden = !(line.picking && !aside && marks.length > 1);"),
        "{draw}"
    );
    let which = js_fn(VERSIONS_JS, "drawWhich");
    for step in [
        "cell(\"span\", WHICH_LINE + \" \")",
        "cell(\"button\", words[i] || mark, { type: \"button\" })",
        "() => ownChanged(line, { set_aside_of: mark })",
    ] {
        assert!(which.contains(step), "{step}: {which}");
    }
    not_hidden_by_css(".med-list-which");
    no_dialog();
}

/// A set-aside line has no tick box and no reason row, and reads "set aside — not current" where
/// the tick's words were. Pressing "Set aside" again undoes the mark: an ordinary line, HELD, as
/// the door leaves it (ruling R1). Every change to the mark, source or why is a content change, as
/// the door counts it: the hand tick goes and the line is drawn unticked until it is read again.
#[test]
fn a_set_aside_line_has_no_tick_and_reads_not_current() {
    assert_eq!(
        js_const(VERSIONS_JS, "SET_ASIDE_MARK"),
        "set aside — not current"
    );
    let draw = js_fn(VERSIONS_JS, "drawLineVersions");
    for step in [
        "r.tick.hidden = aside;",
        "if (aside) r.reasonRow.hidden = true;",
        "if (aside) setMessage(r.state, SET_ASIDE_MARK);",
    ] {
        assert!(draw.contains(step), "{step}: {draw}");
    }
    // drawLine words the state first; the mark is written over it.
    let line = js_fn(LIST_JS, "drawLine");
    assert!(
        position(line, "setMessage(r.state, words.join(")
            < position(line, "drawLineVersions(line);")
    );
    let press = js_fn(VERSIONS_JS, "pressSetAside");
    assert!(
        press.contains("if (isSetAside(line)) ownChanged(line, { set_aside_of: null });"),
        "{press}"
    );
    let set = js_fn(VERSIONS_JS, "setOwn");
    for step in [
        "Object.assign(line.content, fields);",
        "line.handTicked = false;",
        "line.held = fields.set_aside_of === null;",
        "line.tickPending = true;",
        "line.version += 1;",
    ] {
        assert!(set.contains(step), "{step}: {set}");
    }
    // A set-aside line beside a CHARTED line signs alone, so it enables Sign; it is never ticked.
    assert!(js_fn(LIST_JS, "drawMedList").contains("!medLines.some(signsAtSign)"));
    let signs = js_fn(VERSIONS_JS, "signsAtSign");
    assert!(
        signs.contains(
            "drawnTicked(line) || (line.content.set_aside_of || \"\").startsWith(\"med:\")"
        ),
        "{signs}"
    );
}

/// The charted list draws each line's set-aside versions under it in ONE closed `<details>`: the
/// summary "{n} other version(s)", then each version in Rust's words (`VersionView::line`, built by
/// `version_line`), never worded here. A line with none draws no row.
#[test]
fn a_charted_line_shows_its_versions_collapsed() {
    assert_eq!(js_const(VERSIONS_JS, "OTHER_VERSIONS"), " other version(s)");
    let render = js_fn(MAIN_JS, "renderRow");
    assert!(
        position(render, "rows.push(...allergyRows(row));")
            < position(render, "rows.push(...versionRows(row));")
    );
    let rows = js_fn(VERSIONS_JS, "versionRows");
    for step in [
        "if (row.versions.length === 0) return [];",
        "const box = cell(\"details\", \"\", { class: \"med-versions\" });",
        "cell(\"summary\", row.versions.length + OTHER_VERSIONS)",
        "row.versions.map((version) => cell(\"li\", version.line))",
        "td.colSpan = 5;",
    ] {
        assert!(rows.contains(step), "{step}: {rows}");
    }
    not_hidden_by_css(".med-versions");
    // Every field read off a charted row and its versions is one Rust sends.
    let sent = serde_json::to_value(VersionView { line: "l".into() }).unwrap();
    assert_eq!(sent, serde_json::json!({ "line": "l" }));
    let row = &build_view(&sample_chart()).rows[0];
    let row = serde_json::to_value(row).unwrap();
    for field in fields_read_in(VERSIONS_JS, "row") {
        assert!(row.get(&field).is_some(), "row.{field} is not sent: {row}");
    }
    assert_eq!(
        fields_read_in(VERSIONS_JS, "version"),
        ["line".to_string()].into()
    );
    // Loaded with the list's scripts, before any chart is drawn.
    assert!(
        position(INDEX_HTML, "src=\"med_list_sign.js\"")
            < position(INDEX_HTML, "src=\"med_list_versions.js\"")
    );
}

/// Review Focus 3: removing a companion line unmarks its versions here at once — ordinary, held
/// lines, as the delete door leaves them — so no mark names a line that is gone, not even one on a
/// version whose own change has not reached the node; the re-read after the delete draws the
/// node's truth.
#[test]
fn removing_a_companion_unmarks_its_version() {
    let remove = js_fn(LIST_JS, "removeMedLine");
    assert!(
        position(remove, "dropLine(line);") < position(remove, "unmarkVersionsOf(line);"),
        "{remove}"
    );
    let unmark = js_fn(VERSIONS_JS, "unmarkVersionsOf");
    for step in [
        "line.content.set_aside_of !== \"line:\" + gone.id",
        "setOwn(line, { set_aside_of: null });",
        "void queueSave(line);",
    ] {
        assert!(unmark.contains(step), "{step}: {unmark}");
    }
    assert!(js_fn(LIST_JS, "deleteLine").contains("void readMedDraft();"));
}

/// Task 4/6 carry: every save sends back ALL FOUR of the line's own fields, as the read gave them
/// or the clinician changed them — the door clears one left out, and clearing is a content change
/// that drops a hand tick. They ride in `line.content` (`contentOf`), which every save spreads,
/// including the save that splits words still in the editor.
#[test]
fn an_autosave_echoes_the_lines_source_why_and_mark() {
    let content = js_fn(LIST_JS, "contentOf");
    for field in [
        "info_source: v.info_source",
        "source_detail: v.source_detail",
        "why: v.why",
        "set_aside_of: v.set_aside_of",
    ] {
        assert!(content.contains(field), "{field}: {content}");
    }
    let send = js_fn(LIST_JS, "sendLine");
    assert!(
        send.contains("{ ...line.content, ...(await lineContent(line.content.charted_as, snap)) }"),
        "{send}"
    );
    assert!(
        send.contains("line: { line_id: line.id, ...content,"),
        "{send}"
    );
    // Read back into the line whenever nothing local is pending, as its words are.
    assert!(js_fn(LIST_JS, "mergeLine").contains("line.content = contentOf(v);"));
}

/// Ruling R14: a Sign records the set-aside versions too (beside charted lines, with nothing
/// ticked), so the pane says so near Sign whenever the list holds one.
#[test]
fn sign_says_it_also_records_set_aside_versions() {
    in_the_pane("med-list-versions-note");
    assert!(
        position(INDEX_HTML, "id=\"med-list-versions-note\"")
            < position(INDEX_HTML, "id=\"med-list-sign\"")
    );
    not_hidden_by_css("#med-list-versions-note");
    let start = position(VERSIONS_JS, "const SIGN_ALSO =");
    let decl = &VERSIONS_JS[start..start + position(&VERSIONS_JS[start..], ";\n")];
    let pieces: Vec<&str> = decl.split('"').skip(1).step_by(2).collect();
    assert_eq!(
        format!("{}{}{}", pieces[0], 2, pieces[1]),
        "Sign also records 2 set-aside version(s), as not current"
    );
    let draw = js_fn(VERSIONS_JS, "drawListVersions");
    for step in [
        "const count = medLines.filter(isSetAside).length;",
        "setMessage(el(\"med-list-versions-note\"), count > 0 ? SIGN_ALSO[0] + count + SIGN_ALSO[1] : \"\");",
    ] {
        assert!(draw.contains(step), "{step}: {draw}");
    }
}
