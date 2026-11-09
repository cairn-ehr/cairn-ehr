//! Allergy slice 3, Task 6: the drug pick (`drug_pick.js`) and the tags on the medication list
//! (`allergy_tags.js`). Like `allergies_webview_tests.rs`, these read the scripts' source — the
//! window has no test rig (no npm, by design) — and pin the CHOICES a property is. The behaviour
//! was walked headlessly in a browser with a stub `invoke`; see the slice's report.
use crate::source_guard::{js_fn, position};

const DRUG_PICK_JS: &str = include_str!("../src-ui/drug_pick.js");
const TAGS_JS: &str = include_str!("../src-ui/allergy_tags.js");
const ALLERGIES_JS: &str = include_str!("../src-ui/allergies.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Drugs and groups are offered after 150 ms of quiet and from the third character; a group's
/// option says so; arrows, Enter, click and Escape drive the list; it is loaded before allergies.js.
#[test]
fn the_pick_offers_drugs_and_groups() {
    for pinned in [
        "const PICK_DELAY_MS = 150;",
        "const PICK_MIN_CHARS = 3;",
        "invoke(\"drug_search\", { text: text })",
        "option.display + \" (group)\"",
        "\"ArrowDown\"",
        "\"ArrowUp\"",
        "\"Enter\"",
        "\"Escape\"",
        "li.addEventListener(\"click\"",
    ] {
        assert!(DRUG_PICK_JS.contains(pinned), "{pinned}");
    }
    let tag = position(INDEX_HTML, "<script src=\"drug_pick.js\"></script>");
    assert!(tag < position(INDEX_HTML, "<script src=\"allergies.js\"></script>"));
    // The options are plain text (textContent via `cell`), never markup.
    assert!(!DRUG_PICK_JS.contains("innerHTML"));
}

/// Review focus 2: picking a drug then editing the name drops the coding, and so does leaving
/// "Drug" (ruling T5-d) — and the coding is read, at signing, only while the kind is drug.
#[test]
fn editing_the_name_after_a_pick_drops_the_coding() {
    let at = position(DRUG_PICK_JS, "addEventListener(\"input\"");
    let on_input = &DRUG_PICK_JS[at..at + position(&DRUG_PICK_JS[at..], "});")];
    assert!(on_input.contains("heldCoding = null;"), "{on_input}");
    let held = js_fn(DRUG_PICK_JS, "pickedCoding");
    assert!(held.contains("pickIsDrug() ? heldCoding : null"), "{held}");
    let kind = &DRUG_PICK_JS[position(DRUG_PICK_JS, "allergen_kind\"]')")..];
    assert!(kind.contains("heldCoding = null;"), "{kind}");
    // The form's emptying forgets it too, and Record/Correct send what is held.
    assert!(js_fn(ALLERGIES_JS, "resetAllergyForm").contains("clearPick();"));
    assert!(js_fn(ALLERGIES_JS, "recordAllergy").contains("coding: pickedCoding(),"));
}

/// No drug list on this terminal: the hint (exact words), never a refusal; a failed search too
/// leaves the name to be recorded as typed.
#[test]
fn no_drug_list_shows_the_hint() {
    assert!(
        DRUG_PICK_JS.contains("\"No drug list on this terminal: the name is recorded as typed\"")
    );
    let draw = js_fn(DRUG_PICK_JS, "drawPick");
    assert!(
        position(draw, "if (!answer.available)") < position(draw, "pickOptions = answer.options;"),
        "{draw}"
    );
    assert!(draw.contains("NO_DRUG_LIST_HINT"));
}

/// The loud tag and each mark are VISIBLE TEXT on the line (never a hover-only `title`, #871):
/// the tag is the word `Allergy`, in the name cell, and each mark is a table row under the line.
/// The comparison views have their own renderers and never call these.
#[test]
fn a_marked_line_shows_its_tag_and_mark_without_hover() {
    assert!(TAGS_JS.contains("const ALLERGY_TAG = \"Allergy\";"));
    assert!(
        !TAGS_JS.contains("\"title\"") && !TAGS_JS.contains(".title"),
        "a tag is text, not a hover title"
    );
    let name = js_fn(TAGS_JS, "nameCell");
    assert!(name.contains("row.allergy_marks.length > 0"), "{name}");
    assert!(
        name.contains("th.append(\" \", cell(\"span\", ALLERGY_TAG"),
        "{name}"
    );
    let rows = js_fn(TAGS_JS, "allergyRows");
    assert!(rows.contains("row.allergy_marks"), "{rows}");
    assert!(rows.contains("\"allergy-prominent\""), "{rows}");
    assert!(rows.contains("row.allergy_notes"), "{rows}");
    assert!(rows.contains("\"allergy-quiet\""), "{rows}");
    // Drawn from the patient's own pane only: renderRow is main.js's, and no other script uses it.
    let render = js_fn(MAIN_JS, "renderRow");
    assert!(render.contains("nameCell(row)") && render.contains("allergyRows(row)"));
    for other in [
        include_str!("../src-ui/duplicates.js"),
        include_str!("../src-ui/link.js"),
        include_str!("../src-ui/unlink.js"),
    ] {
        assert!(!other.contains("allergy_marks") && !other.contains("nameCell("));
    }
    // The check-failed line sits above the table, outside the hidden warnings section.
    let failed = position(INDEX_HTML, "id=\"allergy-check-failed\"");
    assert!(failed < position(INDEX_HTML, "<table id=\"med-table\">"));
    // After the warnings section closes: it is hidden when empty, this line must not be.
    assert!(failed > position(INDEX_HTML, "<p id=\"chart-elsewhere\" hidden></p>"));
    assert!(js_fn(MAIN_JS, "drawList").contains("setMessage(el(\"allergy-check-failed\")"));
    assert!(js_fn(MAIN_JS, "clearDrawnList").contains("allergy-check-failed"));
}

/// The small tag is the words `not checked`, drawn only when there is no mark (a marked line is
/// not "unverified": the node never sets both, and the draw agrees).
#[test]
fn an_unverified_line_shows_the_small_tag() {
    assert!(TAGS_JS.contains("const NOT_CHECKED_TAG = \"not checked\";"));
    let name = js_fn(TAGS_JS, "nameCell");
    assert!(
        position(name, "row.allergy_marks.length > 0") < position(name, "row.allergy_unverified"),
        "{name}"
    );
    assert!(INDEX_HTML.contains("<script src=\"allergy_tags.js\"></script>"));
}

/// A signed allergy act re-reads the medication list (the check runs inside `med_list`), through
/// the split-out `readMedList`, which `refresh` also uses.
#[test]
fn recording_an_allergy_rereads_the_medication_list() {
    let signed = js_fn(ALLERGIES_JS, "afterSigned");
    let patient_gate = position(signed, "if (patient !== displayedPatient) return;");
    assert!(
        patient_gate < position(signed, "void readMedList();"),
        "{signed}"
    );
    let refresh = js_fn(MAIN_JS, "refresh");
    assert!(
        refresh.contains("await readMedList(lead, behind);"),
        "{refresh}"
    );
    assert!(js_fn(MAIN_JS, "readMedList").contains("invoke(\"med_list\""));
}

/// Fix round T6-a/b: Correct holds the line's coding AFTER setting the term (the input handler
/// drops a coding on edit, and a programmatic `.value =` fires no input event), and whether a
/// coding may be sent follows the pressed kind at the point of use — no separate flag that
/// `pressChoice` (which fires no click) could leave out of step.
#[test]
fn a_correction_keeps_the_coding_and_the_pick_follows_the_pressed_kind() {
    let overlays = include_str!("../src-ui/allergy_overlays.js");
    let start = js_fn(overlays, "startCorrection");
    assert!(
        position(start, "el(\"allergy-term\").value = words.term;")
            < position(start, "holdPick(words.coding);"),
        "{start}"
    );
    let held = js_fn(DRUG_PICK_JS, "pickedCoding");
    assert!(held.contains("pickIsDrug()"), "{held}");
    assert!(
        js_fn(DRUG_PICK_JS, "pickIsDrug").contains("pressedChoice(\"allergen_kind\") === \"drug\"")
    );
    assert!(!DRUG_PICK_JS.contains("pickAllowed"), "no separate flag");
    let at = position(DRUG_PICK_JS, "addEventListener(\"input\"");
    let on_input = &DRUG_PICK_JS[at..at + position(&DRUG_PICK_JS[at..], "});")];
    assert!(on_input.contains("pickIsDrug()"), "{on_input}");
}
