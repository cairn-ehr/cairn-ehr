//! Allergy slice 1, Task 6: the allergy strip and its form in the patient header (`allergies.js`).
//!
//! The window's scripts have no test rig (no npm or bundler: main.js's header, issue #332), so,
//! like `notes_webview_tests.rs`, each guard reads the script's source (`crate::source_guard`) and
//! pins the CHOICE or the ORDER a property is. The behaviour was walked headlessly (a stub
//! `invoke` in a browser) when it was written — see the PR that added this file.
use crate::allergies::{AllergyLine, AllergyPane, AllergyWrite};
use crate::commands::tests::fields_read_in;
use crate::source_guard::{js_fn, position};
use std::collections::BTreeSet;

const ALLERGIES_JS: &str = include_str!("../src-ui/allergies.js");
const ALLERGIES_RS: &str = include_str!("allergies.rs");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// Allergy text is plaintext from end to end (global constraints: "no markup handling"): what the
/// clinician typed is drawn with `textContent` only, so no markup sink may appear in the script.
#[test]
fn allergies_js_never_assigns_inner_html() {
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
    ] {
        assert!(!ALLERGIES_JS.contains(markup), "allergies.js uses {markup}");
    }
    let line = js_fn(ALLERGIES_JS, "lineItem");
    assert!(
        line.contains("cell("),
        "a line is drawn through `cell` (textContent): {line}"
    );
}

/// The design's anti-goal (§3.2): "Allergy or Adverse effect" is a choice the clinician MAKES —
/// a default would let an intolerance be signed as an allergy (or the reverse) by a press that
/// chose nothing. The same holds for what happened. So no allergy toggle is pressed in the page as
/// written, and the defaults the script presses (on load and after every signed entry) leave both
/// unpressed; only the allergen kind (drug) and the source (patient stated) have one.
#[test]
fn no_kind_button_starts_pressed() {
    let form = &INDEX_HTML[position(INDEX_HTML, "<form id=\"allergy-form\"")..];
    let form = &form[..position(form, "</form>")];
    assert!(
        !form.contains("aria-pressed=\"true\""),
        "nothing is pressed in the page as written: {form}"
    );
    for value in ["allergy", "adverse-effect"] {
        assert!(
            form.contains(&format!("data-value=\"{value}\" aria-pressed=\"false\"")),
            "the {value} toggle: {form}"
        );
    }
    let at = position(ALLERGIES_JS, "const ALLERGY_DEFAULTS = {");
    let defaults = &ALLERGIES_JS[at..at + position(&ALLERGIES_JS[at..], "};")];
    for pinned in [
        "allergen_kind: \"drug\",",
        "kind: null,",
        "category: null,",
        "source: \"patient-stated\",",
    ] {
        assert!(defaults.contains(pinned), "{pinned}: {defaults}");
    }
    let reset = js_fn(ALLERGIES_JS, "resetAllergyForm");
    assert!(reset.contains("ALLERGY_DEFAULTS"), "{reset}");
}

/// The paper folder's red sticker (design §3.4, maintainer option A): the strip is one line INSIDE
/// the patient identity header, under the identity lines, and "show all" expands in place beside
/// it — never a dialog. A strip outside the header could scroll away from the name it belongs to.
#[test]
fn the_strip_is_inside_the_identity_header() {
    let header = position(INDEX_HTML, "<header id=\"identity\"");
    let id_line = position(INDEX_HTML, "<p id=\"identity-id\">");
    let strip = position(INDEX_HTML, "<div id=\"allergy-strip\" role=\"status\">");
    let details = position(INDEX_HTML, "<details id=\"allergy-details\">");
    let closes = header + position(&INDEX_HTML[header..], "</header>");
    assert!(header < id_line && id_line < strip && strip < details && details < closes);
    for text in [INDEX_HTML, ALLERGIES_JS] {
        assert!(!text.contains("<dialog") && !text.contains("confirm("));
    }
}

/// allergies.js is loaded, after the scripts whose globals it calls at a press (main.js,
/// signer.js, chart_stale.js's helpers are read only later) and BEFORE funnel.js, whose boot can
/// clear the chart — `clearChart` calls `clearAllergies`. main.js reads the strip wherever it reads
/// the chart (`refresh`, which "Read this chart again" calls too) and takes it down with the chart.
#[test]
fn allergies_js_is_listed_and_loaded() {
    let tag = position(INDEX_HTML, "<script src=\"allergies.js\"></script>");
    assert!(position(INDEX_HTML, "<script src=\"signer.js\"></script>") < tag);
    assert!(tag < position(INDEX_HTML, "<script src=\"funnel.js\"></script>"));
    let refresh = js_fn(MAIN_JS, "refresh");
    assert!(
        position(refresh, "void drawAllergies(patient);")
            < position(refresh, "invoke(\"med_list\""),
        "{refresh}"
    );
    let clear = js_fn(MAIN_JS, "clearChart");
    assert!(clear.contains("clearAllergies();"), "{clear}");
    for name in ["drawAllergies", "recordAllergy", "recordNoneKnown"] {
        js_fn(ALLERGIES_JS, name);
    }
}

/// A refusal of the form lands under its field (design §3.2). The command answers an `ErrorView`
/// with text only, so the script finds the field by the sentence's own words: each fragment it
/// matches must still be in Rust's wording, or the refusal would fall through to the status line
/// (still said, but no longer beside the field it is about).
#[test]
fn every_field_refusal_the_script_matches_is_one_rust_says() {
    let at = position(ALLERGIES_JS, "const FIELD_REFUSALS = [");
    let table = &ALLERGIES_JS[at..at + position(&ALLERGIES_JS[at..], "];")];
    let fragments: Vec<&str> = table.split('"').skip(1).step_by(2).collect();
    assert!(fragments.len() >= 10, "{table}");
    for pair in fragments.chunks(2) {
        assert!(
            ALLERGIES_RS.contains(pair[0]),
            "Rust no longer says {:?}",
            pair[0]
        );
        assert!(
            INDEX_HTML.contains(&format!("id=\"{}\"", pair[1])),
            "no field line {:?}",
            pair[1]
        );
    }
}

/// A write that was SIGNED answers Ok (ruling R11). With no pane read back, nothing of the old list
/// may stay up looking current (it lacks the entry just signed) and no Sign may stay live over it:
/// the strip says Rust's outcome, the set is dropped (every write needs one), and the form is
/// emptied so the same entry cannot be signed twice. An outcome-unknown refusal empties the form too.
#[test]
fn a_signed_write_never_leaves_a_live_sign_for_the_same_entry() {
    let after = js_fn(ALLERGIES_JS, "afterSigned");
    let unread = &after[position(after, "if (answer.pane === null) {")..];
    for step in [
        "allergyCharts = null;",
        "stripSays(answer.outcome, false);",
        "clearAllergyLists();",
    ] {
        assert!(unread.contains(step), "{step}: {after}");
    }
    assert!(
        position(after, "resetAllergyForm();") < position(after, "if (answer.pane === null) {"),
        "{after}"
    );
    let refused = js_fn(ALLERGIES_JS, "afterRefused");
    let unknown = &refused[position(refused, "if (behind === \"unknown\")")..];
    assert!(unknown.contains("resetAllergyForm();"), "{refused}");
    assert!(refused.contains("allergyCharts = null;"), "{refused}");
    assert!(
        refused.contains("await refresh(text, behind);"),
        "{refused}"
    );
}

/// Every field allergies.js reads off a payload is one Rust sends. A misspelt field reads as
/// `undefined` and the window would show nothing where an allergy is — silently.
#[test]
fn every_field_allergies_js_reads_is_one_rust_sends() {
    let line = AllergyLine {
        event_id: String::new(),
        text: String::new(),
        prominent: true,
        doubted_chart: false,
        signer: None,
        recorded: String::new(),
    };
    let keys = |v: serde_json::Value| -> BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    let line_keys = keys(serde_json::to_value(&line).unwrap());
    let pane = AllergyPane {
        strip: String::new(),
        drug: "entries",
        other: "entries",
        drug_allergies: vec![line],
        other_allergies: vec![],
        adverse_effects: vec![],
        previously_none: vec![],
        charts: vec![],
    };
    let pane_keys = keys(serde_json::to_value(&pane).unwrap());
    let write_keys = keys(
        serde_json::to_value(AllergyWrite {
            outcome: String::new(),
            pane: None,
        })
        .unwrap(),
    );
    for (binding, offered) in [
        ("pane", &pane_keys),
        ("line", &line_keys),
        ("answer", &write_keys),
    ] {
        let read = fields_read_in(ALLERGIES_JS, binding);
        assert!(!read.is_empty(), "allergies.js reads `{binding}`");
        for field in read {
            assert!(
                offered.contains(&field),
                "allergies.js reads `{binding}.{field}`, Rust sends {offered:?}"
            );
        }
    }
}

/// Files under 500 lines (house rule).
#[test]
fn allergies_js_stays_small() {
    assert!(ALLERGIES_JS.lines().count() < 500);
}
