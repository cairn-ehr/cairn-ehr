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

/// Fix round 1, item 1: a write on chart A answered after the clinician opened chart B must not
/// outdate B's allergy read. The revision is bumped only once the answer is known to be for the
/// chart on screen — bumped first, B's in-flight read was dropped and B's sticker stayed
/// "Reading the allergy list…" with every write disabled.
#[test]
fn an_answer_for_a_chart_left_never_outdates_the_chart_on_screen() {
    let after = js_fn(ALLERGIES_JS, "afterSigned");
    assert!(
        position(after, "if (patient !== displayedPatient) return;")
            < position(after, "allergyRevision += 1;"),
        "{after}"
    );
}

/// Fix round 1, item 2: a fault mid-press drops the set; a list read already in flight must not
/// then draw and re-enable Sign before the clinician re-reads. So the drop outdates it too.
#[test]
fn a_fault_mid_press_outdates_every_read_in_flight() {
    let press = js_fn(ALLERGIES_JS, "pressOnce");
    let caught = &press[position(press, "catch (fault) {")..];
    let caught = &caught[..position(caught, "} finally {")];
    for step in ["allergyCharts = null;", "allergyRevision += 1;"] {
        assert!(caught.contains(step), "{step}: {press}");
    }
}

/// Fix round 2: a fault in a press on chart A, after the clinician opened chart B, must leave B
/// alone (ruling R13: an outcome for a left chart is never shown on another). The press captures
/// its chart before it acts, and the catch returns for a chart left BEFORE it touches any state —
/// else B's first read was outdated (its sticker stuck at "Reading…"), B's form wiped, and A's
/// fault said on B.
#[test]
fn a_fault_for_a_chart_left_never_touches_the_chart_on_screen() {
    let press = js_fn(ALLERGIES_JS, "pressOnce");
    assert!(
        position(press, "const patient = allergyPatient;") < position(press, "await act();"),
        "{press}"
    );
    let caught = &press[position(press, "catch (fault) {")..];
    let caught = &caught[..position(caught, "} finally {")];
    let guard = position(caught, "if (patient !== displayedPatient) return;");
    for step in [
        "allergyCharts = null;",
        "allergyRevision += 1;",
        "resetAllergyForm();",
        "sayAllergy(",
    ] {
        assert!(
            guard < position(caught, step),
            "{step} after the guard: {press}"
        );
    }
}

/// Fix round 1, item 3: a refusal under a field is announced (it appears where the clinician is
/// not necessarily looking — at the Sign button) and is tied to its control, so a screen reader
/// hears it with the field it is about.
#[test]
fn every_field_refusal_is_announced_and_describes_its_control() {
    for (error, control) in [
        ("allergy-term-error", "id=\"allergy-term\""),
        ("allergy-kind-error", "data-field=\"kind\""),
        ("allergy-category-error", "data-field=\"category\""),
        ("allergy-onset-error", "id=\"allergy-onset\""),
    ] {
        assert!(
            INDEX_HTML.contains(&format!(
                "<p id=\"{error}\" class=\"allergy-field-error\" role=\"alert\" hidden></p>"
            )),
            "{error} is announced"
        );
        let at = position(INDEX_HTML, control);
        let tag_end = at + position(&INDEX_HTML[at..], ">");
        let tag_start = INDEX_HTML[..at]
            .rfind('<')
            .expect("the control's tag opens");
        assert!(
            INDEX_HTML[tag_start..tag_end].contains(&format!("aria-describedby=\"{error}\"")),
            "{control} is described by {error}"
        );
    }
}

/// Fix round 1, item 4: a behind-the-record refusal is said ONCE — as the chart re-read's lead on
/// `#outcome` (chart_stale.js keeps it there: a failed re-read is said after it, and an
/// overtaking read still says an interrupted act's sentence), never also on `#allergy-status`.
#[test]
fn a_behind_the_record_refusal_is_said_once_as_the_rereads_lead() {
    let refused = js_fn(ALLERGIES_JS, "afterRefused");
    let behind = &refused[position(refused, "if (behind) {")..];
    let behind = &behind[..position(behind, "return;")];
    assert!(behind.contains("await refresh(text, behind);"), "{refused}");
    assert!(!behind.contains("sayAllergy("), "{refused}");
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
        standing_none: vec![],
        unreadable: 0,
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

/// An allergy record this terminal cannot read (`pane.unreadable`) makes the strip prominent even
/// when nothing readable names an allergy — the record may be the anaphylaxis — and keeps the
/// strip's one-press "none known" buttons hidden: they would invite a statement over it.
#[test]
fn an_unreadable_record_marks_the_strip_and_hides_its_none_buttons() {
    let render = js_fn(ALLERGIES_JS, "renderAllergies");
    let prominent = &render[position(render, "const named")..];
    let prominent = &prominent[..position(prominent, ";")];
    assert!(prominent.contains("pane.unreadable > 0"), "{render}");
    let buttons = &render[position(render, "el(\"allergy-strip-none\").hidden")..];
    let buttons = &buttons[..position(buttons, ";")];
    assert!(buttons.contains("pane.unreadable === 0"), "{render}");
}

/// The standing "none known" statement is drawn in the expanded list (with who signed it, in
/// Rust's text), through `cell` like every other line, and taken down with the rest.
#[test]
fn the_standing_statement_is_drawn_and_cleared() {
    let render = js_fn(ALLERGIES_JS, "renderAllergies");
    assert!(
        render.contains("fillAllergyList(\"allergy-standing\", pane.standing_none.map("),
        "{render}"
    );
    let clear = js_fn(ALLERGIES_JS, "clearAllergyLists");
    assert!(clear.contains("\"allergy-standing\""), "{clear}");
    assert!(INDEX_HTML.contains("<ul id=\"allergy-standing\""));
}

/// Files under 500 lines (house rule).
#[test]
fn allergies_js_stays_small() {
    assert!(ALLERGIES_JS.lines().count() < 500);
}
