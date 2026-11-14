//! Charting slice 1a, Task 6: the one-line medication entry (`med_chart.js`) — "Start
//! medication" / "Already taking", the drug pick per part, the read-back row, the allergy check on
//! the line, and Sign through the signer strip.
//!
//! Like `drug_pick_webview_tests.rs`, these read the scripts' source (the window has no JS test rig:
//! no npm, by design) and pin the CHOICE or ORDER a property is. Two halves are checked for real:
//! every field the script reads off a Rust answer is one Rust sends (serialised here), and the
//! line split's four pinned cases run in the window at load (`SPLIT_CASES`), over a vocabulary
//! this file checks against the real reader. The behaviour was walked headlessly in a browser with
//! a stub `invoke` (see the task report). VISIBILITY, not only text (an earlier slice shipped
//! messages nobody could see): every element the clinician must see sits inside the section the
//! script un-hides, is un-hidden by the script itself, and no stylesheet rule hides it.
use crate::allergy_check::{CodingArg, DrugSearch};
use crate::commands::tests::fields_read_in;
use crate::med_chart::{view, NEEDS_A_REASON};
use crate::source_guard::{js_fn, position};
use cairn_node::allergy::check::LineCheck;
use cairn_node::medication::line::read_directions;

pub(crate) const MED_JS: &str = include_str!("../src-ui/med_chart.js");
/// The list the editor is mounted in (batch charting slice 1): med_list.js and its Sign.
const LIST_JS: &str = concat!(
    include_str!("../src-ui/med_list.js"),
    include_str!("../src-ui/med_list_sign.js")
);
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");
const STYLE_CSS: &str = include_str!("../src-ui/style.css");

/// The `#med-chart` section of the page, open tag to close.
pub(crate) fn section() -> &'static str {
    let at = position(INDEX_HTML, "<section id=\"med-chart\"");
    &INDEX_HTML[at..at + position(&INDEX_HTML[at..], "</section>")]
}

/// `id` is inside the line (`#med-line-row`, inside `#med-chart`): it is on screen whenever the
/// line is open, and the line is open only while the script shows it.
pub(crate) fn in_the_line(id: &str) {
    let s = section();
    let row = position(s, "<div id=\"med-line-row\"");
    assert!(
        position(s, &format!("id=\"{id}\"")) > row,
        "{id} is inside the line: {s}"
    );
}

/// No stylesheet rule on `selector` hides it (`display: none` / `visibility: hidden` would leave
/// the words in the DOM and off the screen).
pub(crate) fn not_hidden_by_css(selector: &str) {
    let mut from = 0;
    while let Some(at) = STYLE_CSS[from..].find(selector) {
        let start = from + at;
        let block = &STYLE_CSS[start..start + STYLE_CSS[start..].find('}').expect("a rule")];
        for hiding in [
            "display: none",
            "display:none",
            "visibility: hidden",
            "visibility:hidden",
        ] {
            assert!(!block.contains(hiding), "{selector} is hidden: {block}");
        }
        from = start + selector.len();
    }
}

/// The two buttons sit above the medication table, with their exact copy; the section is shown
/// only over a DRAWN list (`renderedPatient`) and never in fixture mode (signer.js's
/// `fixtureMode`), re-decided at every draw and taken down with the chart. The pressed one reads
/// as pressed (aria-pressed, styled without colour alone) and is what Sign sends as `charted_as`.
#[test]
fn both_buttons_show_on_a_writable_chart_and_not_in_fixture_mode() {
    let s = section();
    assert!(
        position(INDEX_HTML, "<section id=\"med-chart\"")
            < position(INDEX_HTML, "<table id=\"med-table\">")
    );
    assert!(s.contains(" hidden>"), "hidden until a list draws: {s}");
    for (value, words) in [
        ("started-here", "Start medication"),
        ("pre-existing", "Already taking"),
    ] {
        assert!(
            s.contains(&format!(
                "data-value=\"{value}\" aria-pressed=\"false\">{words}</button>"
            )),
            "{words}: {s}"
        );
    }
    let sync = js_fn(MED_JS, "syncMedChart");
    assert!(
        sync.contains("el(\"med-chart\").hidden = renderedPatient === null || fixtureMode;"),
        "{sync}"
    );
    assert!(js_fn(MAIN_JS, "render").contains("syncMedChart();"));
    assert!(js_fn(MAIN_JS, "clearChart").contains("clearMedChart();"));
    let press = js_fn(MED_JS, "pressMed");
    assert!(
        press.contains("setAttribute(\"aria-pressed\", String(button.dataset.value === value))"),
        "{press}"
    );
    assert!(
        press.contains("el(\"med-line-row\").hidden = false;"),
        "{press}"
    );
    assert!(
        press.contains("scheduleCheck();"),
        "the check runs on open: {press}"
    );
    // The press is what the list line is charted as (med_list.js's `newLine`), and the editor
    // mounted on a line shows that line's press as pressed.
    assert!(js_fn(LIST_JS, "newLine").contains("charted_as: chartedAs,"));
    assert!(js_fn(MED_JS, "MedLineEditor").contains("pressMed(line.charted_as);"));
    assert!(STYLE_CSS.contains("#med-chart-buttons button[aria-pressed=\"true\"]"));
    // Loaded after drug_pick.js (whose PICK_DELAY_MS it shares) and before funnel.js's boot.
    let tag = position(INDEX_HTML, "<script src=\"med_chart.js\"></script>");
    assert!(position(INDEX_HTML, "<script src=\"drug_pick.js\"></script>") < tag);
    assert!(tag < position(INDEX_HTML, "<script src=\"funnel.js\"></script>"));
    for text in [INDEX_HTML, MED_JS] {
        assert!(
            !text.contains("<dialog") && !text.contains("confirm("),
            "never a dialog"
        );
    }
}

/// The pick: from the fourth character of a part (design §3.1's "4–5"), after the drug pick's
/// quiet delay, drugs only (`med_drug_search`), each option drawn with its FULL display, the top
/// one highlighted so Enter takes what is visibly chosen (Review Focus 2). Plain text, never markup.
#[test]
fn four_letters_list_drugs_with_full_names() {
    assert!(MED_JS.contains("const MED_PICK_MIN_CHARS = 4;"));
    let input = js_fn(MED_JS, "onMedLineInput");
    assert!(
        input.contains("edited.length >= MED_PICK_MIN_CHARS"),
        "{input}"
    );
    assert!(
        input.contains("setTimeout(() => void askMedPick(edited), PICK_DELAY_MS)"),
        "{input}"
    );
    assert!(js_fn(MED_JS, "askMedPick").contains("invoke(\"med_drug_search\", { text: text })"));
    let draw = js_fn(MED_JS, "drawMedPick");
    assert!(
        draw.contains("cell(\"li\", option.display, { role: \"option\""),
        "{draw}"
    );
    assert!(
        draw.contains("el(\"med-pick\").hidden = medPickOptions.length === 0;"),
        "{draw}"
    );
    assert!(draw.contains("activateMedPick(0);"), "{draw}");
    let keys = &MED_JS[position(MED_JS, "el(\"med-line\").addEventListener(\"keydown\"")..];
    for key in ["\"ArrowDown\"", "\"ArrowUp\"", "\"Enter\"", "\"Escape\""] {
        assert!(keys.contains(key), "{key}");
    }
    in_the_line("med-pick");
    not_hidden_by_css("#med-pick");
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
    ] {
        assert!(!MED_JS.contains(markup), "med_chart.js uses {markup}");
    }
    // What the pick answers is what Rust sends.
    let sent = keys_of(&DrugSearch {
        available: true,
        options: vec![coding()],
    });
    for field in fields_read_in(MED_JS, "found") {
        assert!(sent.contains(&field), "found.{field}: Rust sends {sent:?}");
    }
    let sent = keys_of(&coding());
    for field in fields_read_in(MED_JS, "option") {
        assert!(sent.contains(&field), "option.{field}: Rust sends {sent:?}");
    }
}

/// Review Focus 2: a coding vouches only for the words it was picked with. Every input forgets
/// the held coding of any part whose words changed (pick amoxicillin, retype "amoxapine": gone),
/// the "Coded as" line is redrawn from what is still held (so it cannot outlive a coding), the
/// check re-runs — so the line reads "not checked", never checked-clean — and what is sent is
/// looked up by the part's exact words, at the moment of sending.
#[test]
fn editing_a_picked_part_drops_its_coding() {
    let input = js_fn(MED_JS, "onMedLineInput");
    let drop = position(
        input,
        "medHeld = medHeld.filter((held) => line.parts.includes(held.text));",
    );
    assert!(drop < position(input, "drawCodedAs();"), "{input}");
    assert!(drop < position(input, "recheck();"), "{input}");
    let coded = js_fn(MED_JS, "drawCodedAs");
    assert!(coded.contains("\"Coded as: \""), "{coded}");
    assert!(coded.contains("line.hidden = !medHeld.some"), "{coded}");
    in_the_line("med-coded-as");
    let held = js_fn(MED_JS, "codingFor");
    assert!(held.contains("held.text === part"), "{held}");
    let args = js_fn(MED_JS, "partArgs");
    assert!(args.contains("coding: codingFor(part)"), "{args}");
    // A pick is held only for words still on the line.
    let choose = js_fn(MED_JS, "chooseMedPick");
    assert!(
        choose.contains("if (!option || !medLine.parts.includes(medPickPart)) return;"),
        "{choose}"
    );
    // The held codings are written in four places only: declared, dropped, picked, reset.
    assert_eq!(
        MED_JS.matches("medHeld = ").count(),
        5,
        "declaration, drop, pick, reset, mount"
    );
}

/// The read-back row: the directions (the typed line after the drug parts) are read by Rust
/// (`med_line_read`) and drawn as Drug · Form · Strength · Dose · Route · How often, with words
/// the reader could not read said as `not read: …`. The split is one pure function, and its
/// four pinned cases run at load; the vocabulary those cases assume is the real reader's.
#[test]
fn the_read_back_row_shows_the_parse_and_unread_words() {
    let back = js_fn(MED_JS, "readBack");
    assert!(
        back.contains("invoke(\"med_line_read\", { text: line.directions })"),
        "{back}"
    );
    let draw = js_fn(MED_JS, "drawReadBack");
    for label in [
        "\"Drug \"",
        "\"Form \"",
        "\"Strength \"",
        "\"Dose \"",
        "\"Route \"",
        "\"How often \"",
        "\"not read: \"",
    ] {
        assert!(draw.contains(label), "{label}: {draw}");
    }
    in_the_line("med-readback");
    not_hidden_by_css("#med-readback");
    // Every field read off the reading is one Directions serialises.
    let sent = keys_of(&read_directions("tab 875/125mg 1 bd po"));
    let read = fields_read_in(MED_JS, "reading");
    assert!(read.len() >= 8, "{read:?}");
    for field in read {
        assert!(
            sent.contains(&field),
            "reading.{field}: Rust sends {sent:?}"
        );
    }
    // The four cases, pinned in the script and run against `splitLine` at load.
    let cases = &MED_JS[position(MED_JS, "const SPLIT_CASES = [")..];
    let cases = &cases[..position(cases, "];")];
    for (line, parts, directions) in [
        ("amox 500mg tds", "[\"amox\"]", "500mg tds"),
        (
            "amox + clav 875/125 bd",
            "[\"amox\", \"clav\"]",
            "875/125 bd",
        ),
        ("ceftriaxone 1g IV stat", "[\"ceftriaxone\"]", "1g IV stat"),
        (
            "Augmentin tab 875/125mg 1 bd po",
            "[\"Augmentin\"]",
            "tab 875/125mg 1 bd po",
        ),
    ] {
        assert!(
            cases.contains(&format!("[\"{line}\", {parts}, \"{directions}\"]")),
            "{line}: {cases}"
        );
    }
    assert!(MED_JS.contains("for (const [text, parts, directions] of SPLIT_CASES)"));
    // The stub vocabulary the cases run with: each word reads on its own, with nothing unread,
    // exactly as `isDirectionWord` asks the real reader — so the cases test the real split.
    let words = &MED_JS[position(MED_JS, "const SPLIT_CASE_WORDS = [")..];
    let words: Vec<&str> = words[..position(words, "];")]
        .split('"')
        .skip(1)
        .step_by(2)
        .collect();
    assert_eq!(words, ["tab", "tds", "bd", "po", "iv", "stat"]);
    for w in words {
        assert!(read_directions(w).unread.is_empty(), "{w} reads");
    }
    for w in ["amox", "clav", "ceftriaxone", "augmentin"] {
        assert!(
            !read_directions(w).unread.is_empty(),
            "{w} is not a direction word"
        );
    }
    let known = js_fn(MED_JS, "isDirectionWord");
    assert!(
        known.contains("wordReads.get(word.toLowerCase()) === true"),
        "{known}"
    );
    assert!(js_fn(MED_JS, "learnWords").contains("reading.unread.length === 0"));
    // And the directions the split hands on read as the clinician meant.
    let d = read_directions("tab 875/125mg 1 bd po");
    assert_eq!(
        (
            d.form.as_deref(),
            d.strength.as_deref(),
            d.dose_amount.as_deref()
        ),
        (Some("tablet"), Some("875/125 mg"), Some("1"))
    );
}

/// Design §3.5: a Start line with an allergy mark needs a reason. Batch charting slice 1 moved
/// the field from the lone line (whose own Sign waited for it) onto each LIST line, whose tick
/// box waits for it (med_list_webview_tests.rs pins that); the editor keeps none of its own. The
/// node refuses a blank one anyway (`NEEDS_A_REASON`), worded by Rust, never by the window.
#[test]
fn a_start_hit_needs_a_reason_on_its_list_line() {
    assert!(
        !INDEX_HTML.contains("id=\"med-reason\""),
        "the editor has no reason field"
    );
    assert!(!MED_JS.contains("med-reason"), "nor reads one");
    assert!(LIST_JS.contains("cell(\"label\", \"Reason for charting despite the warning\")"));
    assert_eq!(NEEDS_A_REASON, "Charting despite a warning needs a reason");
    for js in [MED_JS, LIST_JS] {
        assert!(
            !js.contains(NEEDS_A_REASON),
            "Rust words the refusal; the window never re-words it"
        );
    }
}

/// An Already-taking line with a mark shows the loud `Allergy` tag and each mark's words on the
/// line, and no Reason field; a note is muted; an unverified line gets `not checked`. Each is
/// VISIBLE text (allergy_tags.js's words and classes), never a hover title.
#[test]
fn an_already_taking_hit_shows_the_tag_but_no_reason() {
    // The editor's check and each list line's are drawn by the one `drawMarks`.
    assert!(js_fn(MED_JS, "drawCheck")
        .contains("drawMarks(check, el(\"med-line-tag\"), el(\"med-line-marks\"));"));
    let draw = js_fn(MED_JS, "drawMarks");
    let mark = position(draw, "check.marks.length > 0");
    let unchecked = position(draw, "check.unverified");
    assert!(
        mark < unchecked,
        "a marked line is not also 'not checked': {draw}"
    );
    assert!(draw.contains("tag.textContent = ALLERGY_TAG;"), "{draw}");
    assert!(
        draw.contains("tag.textContent = NOT_CHECKED_TAG;"),
        "{draw}"
    );
    assert!(draw.contains("tag.className = \"allergy-tag\";"), "{draw}");
    assert!(
        draw.contains("tag.className = \"allergy-unchecked-tag\";"),
        "{draw}"
    );
    assert!(draw.contains("tag.hidden = !shown;"), "{draw}");
    assert!(
        draw.contains(
            "check.marks.map((mark) => cell(\"li\", mark, { class: \"allergy-prominent\" }))"
        ),
        "{draw}"
    );
    assert!(
        draw.contains(
            "check.notes.map((note) => cell(\"li\", note, { class: \"allergy-quiet\" }))"
        ),
        "{draw}"
    );
    // Already taking never needs a reason: the list line asks for started-here.
    let needs = js_fn(LIST_JS, "needsReasonOn");
    assert!(needs.contains("=== \"started-here\""), "{needs}");
    assert!(!needs.contains("pre-existing"), "{needs}");
    in_the_line("med-line-tag");
    in_the_line("med-line-marks");
    not_hidden_by_css("#med-line-tag");
    not_hidden_by_css("#med-line-marks");
    assert!(!MED_JS.contains(".title"), "text, not a hover title");
    // Every field read off the check is one Rust sends; the command gets `typedDrug` (R16).
    let sent = keys_of(&view(LineCheck::default()));
    for field in fields_read_in(MED_JS, "check") {
        assert!(sent.contains(&field), "check.{field}: Rust sends {sent:?}");
    }
    let ask = js_fn(MED_JS, "checkLine");
    assert!(ask.contains("invoke(\"med_line_check\""), "{ask}");
    assert!(ask.contains("typedDrug: line.typedDrug,"), "{ask}");
}

/// No drug list on this terminal (or a search that fails): the hint says so in words, never a
/// refusal, and Sign does not wait on the drug list — the line charts uncoded and reads "not
/// checked". After a Sign that landed, the list is drawn from the write's own answer.
#[test]
fn no_drug_list_says_so_and_still_signs() {
    assert!(MED_JS.contains(
        "const MED_NO_DRUG_LIST = \"No drug list on this terminal: the medication is charted as typed\";"
    ));
    let draw = js_fn(MED_JS, "drawMedPick");
    assert!(
        position(draw, "if (!found.available)") < position(draw, "medPickOptions = found.options;")
    );
    assert!(
        draw.contains("setMessage(el(\"med-pick-hint\"), MED_NO_DRUG_LIST);"),
        "{draw}"
    );
    in_the_line("med-pick-hint");
    not_hidden_by_css("#med-pick-hint");
    assert!(
        !js_fn(LIST_JS, "drawMedList").contains("available"),
        "Sign never waits on the drug list"
    );
    let ask = js_fn(MED_JS, "askMedPick");
    assert!(
        ask.contains("console.error(\"med_drug_search failed:\""),
        "{ask}"
    );
    // The check failing is "not checked", never silence.
    let check = js_fn(MED_JS, "checkLine");
    assert!(check.contains("drawCheck(LINE_NOT_CHECKED);"), "{check}");
    // After the list's Sign, the table is drawn as `readMedList` draws an answer, under a revision
    // taken fresh (every read in flight is outdated), and said through the same `sayAfterDraw`.
    let after = js_fn(LIST_JS, "drawSignedTable");
    let read = js_fn(MAIN_JS, "readMedList");
    for (step, ours) in [
        ("drawnRevision = revision;", "drawnRevision = revision;"),
        ("const marked = listMarkedStale();", "const marked = listMarkedStale();"),
        ("render(answer, patient);", "render(answer.pane, patient);"),
        (
            "if (renderedPatient === patient) sayAfterDraw(lead, revision, marked, answer);",
            "if (renderedPatient === patient) sayAfterDraw(answer.outcome, revision, marked, answer.pane);",
        ),
    ] {
        assert!(read.contains(step), "{read}");
        assert!(after.contains(ours), "{after}");
    }
    assert!(
        position(after, "const revision = ++chartReadRevision;")
            < position(after, "drawnRevision = revision;")
    );
}

fn coding() -> CodingArg {
    CodingArg {
        system: "drugref-moiety".into(),
        code: "c".into(),
        display: "amoxicillin".into(),
    }
}

/// The top-level keys `value` serialises to.
fn keys_of(value: &impl serde::Serialize) -> Vec<String> {
    match serde_json::to_value(value).expect("serialises") {
        serde_json::Value::Object(map) => map.keys().cloned().collect(),
        other => panic!("not an object: {other}"),
    }
}
