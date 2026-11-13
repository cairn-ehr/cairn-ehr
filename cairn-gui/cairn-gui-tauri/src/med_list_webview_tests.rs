//! Batch charting slice 1, Task 6: the medication LIST pane (`med_list.js`) — lines added by
//! "Start medication" / "Already taking", each with its tick and its Remove, saved as typed, and
//! ONE Sign over the ticked lines.
//!
//! The rig of `med_chart_webview_tests.rs` (whose visibility helpers these share): the window has
//! no JS test rig (no npm, by design), so these read the scripts' source and pin the CHOICE or
//! ORDER a safety property is; every field the script reads off a Rust answer is one Rust sends
//! (serialised here). VISIBILITY, not only text: every element the clinician must see sits inside
//! the section the script un-hides, is un-hidden by the script itself, and no stylesheet rule
//! hides it. The behaviour was also walked headlessly in a browser with a stub `invoke`, asserting
//! the computed display of each element and its ancestors (see the task report).
use crate::commands::tests::fields_read_in;
use crate::med_chart::view;
use crate::med_chart_webview_tests::{not_hidden_by_css, section};
use crate::med_list::{ListLineView, MedListView, SaveAnswer, WarningRef};
use crate::med_list_sign::MedListSigned;
use crate::source_guard::{js_fn, position};
use cairn_node::{allergy::check::LineCheck, medication::line::read_directions};
use std::collections::BTreeSet;

/// The list's scripts as one text: med_list.js, its Sign med_list_sign.js, and its versions
/// med_list_versions.js (batch charting slice 2a).
pub(crate) const LIST_JS: &str = concat!(
    include_str!("../src-ui/med_list.js"),
    include_str!("../src-ui/med_list_sign.js"),
    include_str!("../src-ui/med_list_versions.js")
);
const MED_JS: &str = include_str!("../src-ui/med_chart.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const FUNNEL_JS: &str = include_str!("../src-ui/funnel.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");
const STYLE_CSS: &str = include_str!("../src-ui/style.css");

/// `id` is in the `#med-chart` section but NOT inside the editor (`#med-line-row`, shown only
/// while a line is typed): it is on screen whenever the script un-hides it over a drawn chart.
pub(crate) fn in_the_pane(id: &str) {
    let s = section();
    let at = position(s, &format!("id=\"{id}\""));
    let row = position(s, "<div id=\"med-line-row\"");
    let row_end = row + position(&s[row..], "</div>");
    assert!(at < row || at > row_end, "{id} is outside the editor: {s}");
}

/// The top-level keys of every value in `values`.
fn keys_of(values: &[serde_json::Value]) -> BTreeSet<String> {
    values
        .iter()
        .flat_map(|v| v.as_object().expect("an object").keys().cloned())
        .collect()
}

fn line_view() -> ListLineView {
    ListLineView {
        line_id: "l".into(),
        charted_as: "started-here".into(),
        line_text: "amox 500mg tds".into(),
        parts: vec![],
        typed_drug: "amox".into(),
        directions: "500mg tds".into(),
        reason: None,
        hand_ticked: false,
        finished: true,
        check: view(LineCheck::default()),
        read: read_directions("500mg tds"),
        default_ticked: false,
        ticked: false,
        duplicate: false,
        duplicate_of: None,
        held: false,
        unfinished: false,
        against: vec![WarningRef {
            source: "allergy".into(),
            reference: "r".into(),
        }],
        delabel_offers: vec![],
        info_source: None,
        source_detail: None,
        why: None,
        set_aside_of: None,
        companions: vec![],
        companion_words: vec![],
    }
}

/// Every field the script reads off the list's answers is one Rust sends: the list (`fresh`),
/// each line (`v`, `view`), its check, the save's and the Sign's answers.
#[test]
fn every_field_read_is_one_rust_sends() {
    let list = MedListView {
        writer: None,
        writer_name: None,
        elsewhere: None,
        lines: vec![],
        from: None,
        versions_recorded: 0,
    };
    let line = serde_json::to_value(line_view()).unwrap();
    for (binding, sent) in [
        ("fresh", keys_of(&[serde_json::to_value(&list).unwrap()])),
        ("v", keys_of(std::slice::from_ref(&line))),
        ("view", keys_of(&[line])),
        (
            "check",
            keys_of(&[serde_json::to_value(view(LineCheck::default())).unwrap()]),
        ),
        (
            "door",
            keys_of(&[serde_json::to_value(SaveAnswer {
                answer: "saved".into(),
                ..SaveAnswer::default()
            })
            .unwrap()]),
        ),
        (
            "answer",
            keys_of(&[
                serde_json::to_value(MedListSigned::Changed {
                    lines: vec![],
                    message: String::new(),
                })
                .unwrap(),
                serde_json::to_value(MedListSigned::Done {
                    outcome: String::new(),
                    charted: 0,
                    of: 0,
                    not_charted: vec![],
                    unknown: vec![],
                    pane: None,
                    list: None,
                })
                .unwrap(),
            ]),
        ),
    ] {
        let read = fields_read_in(LIST_JS, binding);
        assert!(!read.is_empty(), "{binding} is read");
        for field in read {
            assert!(
                sent.contains(&field),
                "{binding}.{field}: Rust sends {sent:?}"
            );
        }
    }
}

/// A 12-line home list is 12 lines and ONE Sign (the plan's §1.2 benchmark). Each press adds a
/// line to the list and mounts the one editor in it; the line saves a second after its last
/// change, under the writer the list was READ under; Enter or focus leaving it finishes it. The
/// one Sign, under the list, sends the ids of the lines DRAWN ticked at the press — before the
/// pending saves are flushed, so a line that turns ticked while they land is not signed unseen —
/// and signs nothing when a line could not be saved.
#[test]
fn two_lines_then_one_sign() {
    let s = section();
    assert!(
        s.contains(
            "<button id=\"med-list-sign\" type=\"button\" hidden>Sign ticked lines</button>"
        ),
        "{s}"
    );
    in_the_pane("med-list");
    in_the_pane("med-list-sign");
    assert!(
        !INDEX_HTML.contains("id=\"med-sign\""),
        "the lone line's Sign is gone"
    );
    // A press adds a line, and the editor is mounted in it.
    assert!(MED_JS.contains(
        "button.addEventListener(\"click\", () => appendMedLine(button.dataset.value));"
    ));
    let append = js_fn(LIST_JS, "appendMedLine");
    assert!(append.contains("medLines.push(line);"), "{append}");
    assert!(append.contains("editMedLine(line);"), "{append}");
    let edit = js_fn(LIST_JS, "editMedLine");
    assert!(edit.contains("MedLineEditor(line.row.slot,"), "{edit}");
    let mount = js_fn(MED_JS, "MedLineEditor");
    assert!(mount.contains("container.append(MED_LINE_ROW);"), "{mount}");
    assert!(mount.contains("medEditor = callbacks;"), "{mount}");
    // Saved one second after the last change (each change pushes the save back), and at once on
    // a tick or when finished.
    assert!(LIST_JS.contains("const MED_LIST_SAVE_MS = 1000;"));
    let touch = js_fn(LIST_JS, "touch");
    assert!(
        position(touch, "clearTimeout(line.timer);")
            < position(
                touch,
                "setTimeout(() => void queueSave(line), MED_LIST_SAVE_MS)"
            ),
        "{touch}"
    );
    let finished = js_fn(LIST_JS, "lineFinished");
    assert!(finished.contains("line.finished = true;"), "{finished}");
    assert!(finished.contains("void queueSave(line);"), "{finished}");
    let keys = &MED_JS[position(MED_JS, "el(\"med-line\").addEventListener(\"keydown\"")..];
    assert!(
        keys.contains("} else if (event.key === \"Enter\" && n === 0) finishMedLine(true);"),
        "{keys}"
    );
    assert!(MED_JS.contains("MED_LINE_ROW.addEventListener(\"focusout\""));
    // Every save and delete carries the writer the list was READ under (a missing key would be
    // the unnamed list): the line keeps it from the read, and never takes who's here later.
    let send = js_fn(LIST_JS, "sendLine");
    assert!(send.contains("writer: line.writer,"), "{send}");
    assert!(js_fn(LIST_JS, "lineFrom").contains("writer: fresh.writer,"));
    assert!(js_fn(LIST_JS, "deleteLine").contains("writer: line.writer"));
    assert!(js_fn(LIST_JS, "drawMedRead").contains("writer: fresh.writer,"));
    assert_eq!(
        LIST_JS.matches("writer: ").count(),
        5,
        "the list read, lineFrom, sendLine, deleteLine, med_list_versions.js's sendFrom"
    );
    // The one Sign: the ticks drawn at the press, then the flush, then the command.
    let sign = js_fn(LIST_JS, "signMedList");
    let ticked = position(
        sign,
        "const ticked = medLines.filter(drawnTicked).map((line) => line.id);",
    );
    let flush = position(
        sign,
        "const saved = await Promise.all(medLines.filter(pending).map(queueSave));",
    );
    let refused = position(sign, "if (saved.includes(false)) {");
    let sent = position(sign, "invoke(\"med_list_sign\"");
    assert!(
        ticked < flush && flush < refused && refused < sent,
        "{sign}"
    );
    assert!(sign.contains("ticked: ticked,"), "{sign}");
    assert!(LIST_JS.contains("medListSigning = signMedList()"));
    let tag = position(INDEX_HTML, "<script src=\"med_list.js\"></script>");
    assert!(position(INDEX_HTML, "<script src=\"med_chart.js\"></script>") < tag);
    let sign_tag = position(INDEX_HTML, "<script src=\"med_list_sign.js\"></script>");
    assert!(tag < sign_tag);
    assert!(sign_tag < position(INDEX_HTML, "<script src=\"whos_here.js\"></script>"));
    // Mounted by main.js: read over every drawn chart, put away with it; closing a chart waits for
    // the list's queued saves as for the note's.
    assert!(js_fn(MAIN_JS, "render").contains("syncMedList();"));
    assert!(js_fn(MAIN_JS, "clearChart").contains("clearMedList();"));
    let close = js_fn(FUNNEL_JS, "closeChart");
    assert!(
        position(close, "clearChart();") < position(close, "await medListSaved();")
            && position(close, "await medListSaved();")
                < position(close, "invoke(\"close_chart\")"),
        "{close}"
    );
    assert!(js_fn(LIST_JS, "medListSaved").contains("return medSaveQueue;"));
}

/// A line still being typed shows `unfinished` and is NEVER drawn ticked — whatever the last read
/// said about its old words. A change to its words takes its verdict down at once (no verdict is
/// drawn over words the node has not saved and read), and a read that answers before its save
/// landed does not put one back.
#[test]
fn an_unfinished_line_is_never_drawn_ticked() {
    assert!(LIST_JS.contains("const MED_LIST_UNFINISHED = \"unfinished\";"));
    let drawn = js_fn(LIST_JS, "drawnTicked");
    let never = position(
        drawn,
        "if (line.snapshot !== null || !line.finished || medUnticked.has(line.id)) return false;",
    );
    assert!(never < position(drawn, "line.view.ticked"), "{drawn}");
    let typed = js_fn(LIST_JS, "lineTyped");
    for step in [
        "line.finished = false;",
        "line.handTicked = false;",
        "line.view = null;",
        "line.contentVersion = line.version;",
    ] {
        assert!(typed.contains(step), "{step}: {typed}");
    }
    assert!(js_fn(LIST_JS, "mergeLine").contains(
        "if (line.snapshot === null && line.savedVersion >= line.contentVersion) line.view = v;"
    ));
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains("r.tick.checked = drawnTicked(line);"),
        "{draw}"
    );
    assert!(
        draw.contains("if (!line.finished) words.push(MED_LIST_UNFINISHED);"),
        "{draw}"
    );
    assert!(
        draw.contains("if (!r.tick.checked) words.push(MED_LIST_UNTICKED);"),
        "a line not ticked reads visibly unsigned: {draw}"
    );
    assert!(
        draw.contains("setMessage(r.state, words.join(\" · \"));"),
        "{draw}"
    );
    not_hidden_by_css(".med-list-state");
}

/// Design §3.5: a Start line with an allergy mark keeps its tick box DISABLED until its reason
/// field holds non-blank text; the field sits on the line (exact label), shown whenever the line
/// needs it. Already taking never needs one. The node refuses a blank reason anyway.
#[test]
fn a_start_mark_keeps_the_box_disabled_until_a_reason() {
    let needs = js_fn(LIST_JS, "needsReasonOn");
    assert!(
        needs.contains("line.content.charted_as === \"started-here\""),
        "{needs}"
    );
    assert!(needs.contains("line.view.check.reason_needed"), "{needs}");
    let waits = js_fn(LIST_JS, "waitsForReason");
    assert!(
        waits.contains("needsReasonOn(line) && line.reason.trim() === \"\""),
        "{waits}"
    );
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains(
            "r.tick.disabled = blocked || !v || (waitsForReason(line) && !r.tick.checked);"
        ),
        "{draw}"
    );
    assert!(
        draw.contains("r.reasonRow.hidden = !needsReasonOn(line);"),
        "{draw}"
    );
    let row = js_fn(LIST_JS, "medLineRow");
    assert!(
        row.contains("cell(\"label\", \"Reason for charting despite the listed allergy\")"),
        "{row}"
    );
    assert!(
        row.contains("reason.addEventListener(\"input\", () => reasonTyped(line, reason.value));"),
        "{row}"
    );
    // Typing the reason re-draws the line, so the box enables as the first letter lands.
    assert!(js_fn(LIST_JS, "reasonTyped").contains("touch(line);"));
    assert!(js_fn(LIST_JS, "touch").contains("drawLine(line);"));
    // The tick vouches for the warnings DRAWN on the line when it was ticked, never refreshed
    // later (a re-read with a new warning redraws the line before any tick can take it).
    let tick = js_fn(LIST_JS, "tickMedLine");
    assert!(
        tick.contains("line.seen = line.view ? line.view.against : [];"),
        "{tick}"
    );
    assert_eq!(
        LIST_JS.matches("line.seen = ").count(),
        1,
        "only at the tick"
    );
    assert!(
        tick.contains("line.finished = true;"),
        "ticking leaves the line: {tick}"
    );
    assert!(js_fn(LIST_JS, "sendLine").contains("seen: sent.handTicked ? sent.seen : []"));
}

/// `Changed`: nothing was signed. The lines it names are drawn UNTICKED, whatever the next read
/// derives, with their warnings (the read is asked again: `Changed` carries no list) and the
/// message. Each line's check is drawn on the line with the editor's tag words and classes.
#[test]
fn changed_lines_come_back_unticked_with_their_warning() {
    let after = js_fn(LIST_JS, "afterMedListSigned");
    let changed = position(after, "if (answer.kind === \"changed\") {");
    let untick = position(after, "for (const id of answer.lines) medUnticked.add(id);");
    let said = position(
        after,
        "setMessage(el(\"med-list-status\"), answer.message);",
    );
    let reread = position(after, "void readMedDraft();");
    assert!(
        changed < untick && untick < said && said < reread,
        "{after}"
    );
    let draw = js_fn(LIST_JS, "drawLine");
    assert!(
        draw.contains(
            "drawMarks(editing ? NO_MARKS : v ? v.check : LINE_NOT_CHECKED, r.tag, r.marks);"
        ),
        "{draw}"
    );
    let marks = js_fn(MED_JS, "drawMarks");
    assert!(marks.contains("tag.textContent = ALLERGY_TAG;"), "{marks}");
    assert!(marks.contains("tag.hidden = !shown;"), "{marks}");
    in_the_pane("med-list-status");
    not_hidden_by_css("#med-list-status");
    // A hand tick again takes the line out of the untick set; an edit does too.
    // A tick by hand (either way) takes the line out of the Changed set; an edit does too. An
    // untick is stored as `held` (med_list_review_webview_tests.rs), never kept in this set.
    assert!(js_fn(LIST_JS, "tickMedLine").contains("medUnticked.delete(line.id);"));
    assert!(js_fn(LIST_JS, "lineTyped").contains("medUnticked.delete(line.id);"));
}

/// `Done`: `Charted {k} of {n}`, then each line not charted with its reason (Rust's words,
/// `not tried` and the NOT-known lead included), drawn whole on its own lines under the list; each
/// not-charted line also carries its reason on the line itself; the medication table is drawn from
/// the Sign's own answer.
#[test]
fn a_partial_outcome_names_each_line_not_charted() {
    let after = js_fn(LIST_JS, "afterMedListSigned");
    assert!(
        after.contains(
            "for (const [id, reason] of answer.not_charted) medLineNotes.set(id, reason);"
        ),
        "{after}"
    );
    assert!(
        after.contains("setMessage(el(\"med-list-outcome\"), answer.outcome);"),
        "{after}"
    );
    assert!(
        after.contains("drawSignedTable(answer, patient);"),
        "{after}"
    );
    in_the_pane("med-list-outcome");
    not_hidden_by_css("#med-list-outcome");
    assert!(STYLE_CSS.contains("#med-list-outcome {\n  white-space: pre-line;"));
    assert!(js_fn(LIST_JS, "drawLine")
        .contains("setMessage(r.note, medLineNotes.get(line.id) || \"\");"));
    let table = js_fn(LIST_JS, "drawSignedTable");
    for step in [
        "void readMedList(answer.outcome, \"no\");",
        "const revision = ++chartReadRevision;",
        "render(answer.pane, patient);",
        "sayAfterDraw(answer.outcome, revision, marked, answer.pane);",
    ] {
        assert!(table.contains(step), "{step}: {table}");
    }
    // The window never words a line's reason: they are Rust's.
    for rust in ["not charted:", "not tried:", "NOT known"] {
        assert!(!LIST_JS.contains(rust), "{rust} is Rust's word");
    }
}

/// Remove deletes the line at once (no dialog, principle 3): its pending save is cancelled, it
/// leaves the screen, and the delete is queued AFTER any save in flight, under the line's writer.
#[test]
fn removing_a_line_asks_nothing() {
    for text in [LIST_JS, MED_JS, INDEX_HTML] {
        assert!(
            !text.contains("<dialog") && !text.contains("confirm(") && !text.contains("alert("),
            "never a dialog"
        );
    }
    let row = js_fn(LIST_JS, "medLineRow");
    assert!(
        row.contains("cell(\"button\", \"Remove\", { type: \"button\" })"),
        "{row}"
    );
    assert!(row.contains("remove.addEventListener(\"click\", () => removeMedLine(line));"));
    let remove = js_fn(LIST_JS, "removeMedLine");
    let cancel = position(remove, "clearTimeout(line.timer);");
    let gone = position(remove, "dropLine(line);");
    let queued = position(
        remove,
        "medSaveQueue = medSaveQueue.then(() => deleteLine(line));",
    );
    assert!(cancel < gone && gone < queued, "{remove}");
    assert!(js_fn(LIST_JS, "deleteLine").contains("invoke(\"med_list_delete\""));
    // A line removed but not yet deleted is not drawn back by a read in flight.
    assert!(js_fn(LIST_JS, "drawMedRead").contains("!medRemoving.has(v.line_id)"));
}

/// A Remove the list refused (`moved`, `changed`, `not-yours`, …) says so, as a refused save does:
/// the re-read draws the line back, and without a word the clinician would think it was gone
/// (whole-slice review, finding 4). `deleted` and `gone` both mean the line is not in the list.
#[test]
fn a_refused_remove_says_so() {
    let delete = js_fn(LIST_JS, "deleteLine");
    let asked = position(delete, "const door = await invoke(\"med_list_delete\"");
    let judged = position(
        delete,
        "if (door.answer !== \"deleted\" && door.answer !== \"gone\")",
    );
    let said = position(
        delete,
        "setMessage(el(\"med-list-status\"), MED_LIST_NOT_REMOVED + \" (the list answered \" + door.answer + \")\");",
    );
    assert!(asked < judged && judged < said, "{delete}");
    assert!(LIST_JS.contains("const MED_LIST_NOT_REMOVED = \"This line was not removed\";"));
}
