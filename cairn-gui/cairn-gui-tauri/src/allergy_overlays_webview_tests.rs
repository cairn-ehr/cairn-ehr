//! Allergy slice 2, Task 6: the gestures on the expanded allergy list (`allergy_overlays.js`) —
//! **Correct**, **De-label**, **Entered in error** on a live line, **Withdraw de-label** on a
//! historical one, **Entered in error** on a "none known" line — and the new lists drawn in
//! `#allergy-details` (`allergies.js`).
//!
//! The window's scripts have no test rig (no npm or bundler, issue #332), so, like
//! `allergies_webview_tests.rs`, each guard reads the scripts' source (`crate::source_guard`) and
//! pins the CHOICE or the ORDER a property is. The behaviour was walked headlessly (a stub
//! `invoke`, in Chromium and WebKit) when it was written — see the PR that added this file.
use crate::allergy_pane::{AllergyLine, EnteredInErrorLine, LineWords, NoneLineView};
use crate::commands::tests::fields_read_in;
use crate::source_guard::{js_fn, position};
use std::collections::BTreeSet;

const OVERLAYS_JS: &str = include_str!("../src-ui/allergy_overlays.js");
const ALLERGIES_JS: &str = include_str!("../src-ui/allergies.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The `<details id="allergy-details">` element as written: every list and row of the expanded
/// allergy section lives inside it.
fn details() -> &'static str {
    let at = position(INDEX_HTML, "<details id=\"allergy-details\">");
    &INDEX_HTML[at..at + position(&INDEX_HTML[at..], "</details>")]
}

/// Allergy text is plaintext from end to end (slice 1's rule, durable-rules "No markup, and no
/// escape function"): the new script draws through `cell` / `textContent` only, and escapes nothing.
#[test]
fn overlay_js_never_assigns_inner_html() {
    for markup in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "escape",
    ] {
        assert!(
            !OVERLAYS_JS.contains(markup),
            "allergy_overlays.js uses {markup}"
        );
    }
    let button = js_fn(OVERLAYS_JS, "actionButton");
    assert!(
        button.contains("el(\"allergy-line-buttons\").content.querySelector("),
        "{button}"
    );
    let start = js_fn(OVERLAYS_JS, "startCorrection");
    assert!(
        start.contains("el(\"allergy-correcting-text\").textContent = line.text;"),
        "{start}"
    );
}

/// Correct fills the existing form from the line's words as recorded and says which line it is
/// correcting; Sign then sends `corrects`. Each choice is pressed through `pressChoice`, which
/// presses only a button whose word IS the recorded word — so a newer peer's `kind` or
/// `category` this window has no button for is left UNPRESSED and the clinician must choose,
/// never re-sending a word this node cannot vouch for. Clearing the form clears the correction,
/// and so does every signed act: a correction left over a re-read list is a Sign aimed at a
/// line that may be gone, and clearing it without the form would leave its words to be signed as
/// a NEW entry — a duplicate.
#[test]
fn correct_prefills_and_sends_corrects() {
    let start = js_fn(OVERLAYS_JS, "startCorrection");
    for step in [
        "resetAllergyForm();",
        "pressChoice(\"allergen_kind\", words.allergen_kind);",
        "pressChoice(\"kind\", words.kind);",
        "pressChoice(\"category\", words.category);",
        "pressChoice(\"source\", words.source);",
        "el(\"allergy-term\").value = words.term;",
        "el(\"allergy-onset\").value = words.onset_value || \"\";",
        "allergyCorrecting = line.event_id;",
        "el(\"allergy-correcting\").hidden = false;",
    ] {
        assert!(start.contains(step), "{step}: {start}");
    }
    // Reset first: an earlier correction's words never mix into this one's.
    assert!(
        position(start, "resetAllergyForm();") < position(start, "pressChoice("),
        "{start}"
    );
    let press = js_fn(ALLERGIES_JS, "pressChoice");
    assert!(
        press.contains("String(button.dataset.value === value)"),
        "an unknown word presses nothing: {press}"
    );
    let typed = js_fn(ALLERGIES_JS, "typedAllergy");
    assert!(typed.contains("corrects: allergyCorrecting,"), "{typed}");
    let reset = js_fn(ALLERGIES_JS, "resetAllergyForm");
    assert!(reset.contains("clearCorrection();"), "{reset}");
    let clear = js_fn(OVERLAYS_JS, "clearCorrection");
    for step in [
        "allergyCorrecting = null;",
        "el(\"allergy-correcting\").hidden = true;",
    ] {
        assert!(clear.contains(step), "{step}: {clear}");
    }
    let after = js_fn(ALLERGIES_JS, "afterSigned");
    assert!(
        after.contains("if (wasEntry || allergyCorrecting !== null) resetAllergyForm();"),
        "{after}"
    );
    // The label is the page's, above the form; the script writes only the line's own text.
    let label = position(
        INDEX_HTML,
        "<p id=\"allergy-correcting-line\">Correcting: <span id=\"allergy-correcting-text\"></span></p>",
    );
    assert!(label < position(INDEX_HTML, "<form id=\"allergy-form\""));
    // allergies.js calls `clearCorrection` at load (`resetAllergyForm()`), so this script is
    // loaded first — load ORDER, never a `typeof` guard.
    assert!(
        position(INDEX_HTML, "<script src=\"allergy_overlays.js\"></script>")
            < position(INDEX_HTML, "<script src=\"allergies.js\"></script>")
    );
    assert!(!OVERLAYS_JS.contains("typeof "));
}

/// De-label and Entered in error (Withdraw de-label is the latter, aimed at the de-label) open a
/// row INLINE, under the line it is about — never a dialog, never `confirm(` (paper-parity: the
/// affordance is the row and one Sign). The rows are the page's `<template>`s, so every label is
/// written in the page, not the script; one row is open at a time; Escape or Cancel closes it
/// without signing. "Tolerated" starts empty (review M3).
#[test]
fn delabel_and_withdraw_rows_are_inline_not_dialogs() {
    for text in [OVERLAYS_JS, INDEX_HTML] {
        for banned in ["<dialog", "confirm(", "alert(", "prompt(", "showModal"] {
            assert!(!text.contains(banned), "{banned}");
        }
    }
    let details = details();
    for template in [
        "<template id=\"allergy-delabel-template\">",
        "<template id=\"allergy-retract-template\">",
    ] {
        assert!(
            details.contains(template),
            "{template} is in #allergy-details"
        );
    }
    let open = js_fn(OVERLAYS_JS, "openRow");
    assert!(
        position(open, "closeOverlayRow();") < position(open, "li.append(row);"),
        "one row at a time: {open}"
    );
    let actions = js_fn(OVERLAYS_JS, "lineActions");
    for step in [
        "openDelabel(line,",
        "openRetraction(line.event_id,",
        "openRetraction(line.challenge,",
    ] {
        assert!(actions.contains(step), "{step}: {actions}");
    }
    // Review M3: "Tolerated" starts EMPTY — nothing is signed by default (design anti-goal), and
    // the drug challenged usually differs from the labelled group. Rust refuses an empty one.
    let delabel = js_fn(OVERLAYS_JS, "openDelabel");
    assert!(
        !delabel.contains("rowInput(row, \"tolerated\").value ="),
        "Tolerated is never prefilled: {delabel}"
    );
    assert!(!delabel.contains("words.term"), "{delabel}");
    let nones = js_fn(OVERLAYS_JS, "noneActions");
    assert!(nones.contains("openRetraction(none.event_id,"), "{nones}");
    let keys = &OVERLAYS_JS[position(OVERLAYS_JS, "addEventListener(\"keydown\"")..];
    assert!(
        position(keys, "\"Escape\"") < position(keys, "closeOverlayRow("),
        "{keys}"
    );
}

/// Every Sign of an overlay goes through `pressOnce` (one press, one signed act; a fault mid-press
/// takes the old strip down; a late answer for a chart left touches nothing, R13) and lands
/// through `afterSigned` / `afterRefused` — reused, never copied. The chart is captured before
/// the invoke, as `recordAllergy` does. A refusal of the row (no "Tolerated", a date it cannot
/// read) is said IN the row, never under the entry form's fields.
#[test]
fn overlay_presses_go_through_press_once() {
    for name in ["openDelabel", "openRetraction"] {
        let f = js_fn(OVERLAYS_JS, name);
        assert!(
            f.contains("void pressOnce(() => signOverlay("),
            "{name}: {f}"
        );
    }
    let sign = js_fn(OVERLAYS_JS, "signOverlay");
    assert!(
        position(sign, "const patient = allergyPatient;") < position(sign, "await send({"),
        "{sign}"
    );
    for step in [
        "charts: allergyCharts,",
        "credential: signerCredential(),",
        "afterSigned(answer, patient, false);",
        "await afterRefused(failure, patient, false, sayHere);",
    ] {
        assert!(sign.contains(step), "{step}: {sign}");
    }
    for copied in [
        "allergyBusy = true",
        "allergyRevision += 1",
        "allergyCharts = null",
    ] {
        assert!(
            !OVERLAYS_JS.contains(copied),
            "{copied}: pressOnce/afterSigned's own steps"
        );
    }
    let delabel = js_fn(OVERLAYS_JS, "openDelabel");
    for arg in [
        "invoke(\"allergy_challenge\", Object.assign(base, {",
        "tolerated:",
        "date:",
        "text:",
    ] {
        assert!(delabel.contains(arg), "{arg}: {delabel}");
    }
    let retract = js_fn(OVERLAYS_JS, "openRetraction");
    assert!(
        retract.contains("invoke(\"allergy_retract\", Object.assign(base, {"),
        "{retract}"
    );
    let refused = js_fn(ALLERGIES_JS, "afterRefused");
    assert!(
        position(refused, "if (behind) {") < position(refused, "if (sayHere)"),
        "a stale refusal is still the re-read's lead: {refused}"
    );
    // A write is pressable only when allergyMayWrite says so: every overlay button and row Sign
    // is `.allergy-write`, put in step by syncAllergyButtons with Sign and the "none known" ones.
    let sync = js_fn(ALLERGIES_JS, "syncAllergyButtons");
    assert!(
        sync.contains("document.querySelectorAll(\".allergy-none, .allergy-write\")"),
        "{sync}"
    );
    // The line buttons are `.allergy-write` in the page's template (every_button_label_is_the_pages).
    for template in ["allergy-delabel-template", "allergy-retract-template"] {
        let at = position(INDEX_HTML, &format!("<template id=\"{template}\">"));
        let body = &INDEX_HTML[at..at + position(&INDEX_HTML[at..], "</template>")];
        assert!(
            body.contains("class=\"allergy-write\" data-act=\"sign\""),
            "{body}"
        );
    }
}

/// The new lists of the pane are drawn in `#allergy-details`, muted, through `cell`, and taken
/// down with the rest: de-labelled lines (with Withdraw de-label), entered-in-error lines with
/// their older versions, and overlays that count for nothing. A line's own older versions are a
/// nested, muted list under it. The "none known" lines draw their text (they are objects now).
#[test]
fn historical_and_entered_in_error_lists_are_drawn() {
    let details = details();
    for id in [
        "allergy-historical",
        "allergy-entered-in-error",
        "allergy-ignored",
    ] {
        assert!(
            details.contains(&format!("<ul id=\"{id}\"")),
            "{id} is in #allergy-details"
        );
    }
    let render = js_fn(ALLERGIES_JS, "renderAllergies");
    for step in [
        "fillAllergyList(\"allergy-historical\", pane.historical.map(lineItem));",
        "fillAllergyList(\"allergy-entered-in-error\", pane.entered_in_error.map(withdrawnItem));",
        "fillAllergyList(\"allergy-ignored\", pane.ignored_overlays.map(",
        "pane.standing_none.map((none) => noneItem(none, false))",
        "pane.previously_none.map((none) => noneItem(none, true))",
    ] {
        assert!(render.contains(step), "{step}: {render}");
    }
    let clear = js_fn(ALLERGIES_JS, "clearAllergyLists");
    for id in [
        "\"allergy-historical\"",
        "\"allergy-entered-in-error\"",
        "\"allergy-ignored\"",
    ] {
        assert!(clear.contains(id), "{id}: {clear}");
    }
    let line = js_fn(ALLERGIES_JS, "lineItem");
    for step in ["quietList(line.earlier)", "lineActions(line)"] {
        assert!(line.contains(step), "{step}: {line}");
    }
    let withdrawn = js_fn(ALLERGIES_JS, "withdrawnItem");
    for step in ["withdrawn.text", "quietList(withdrawn.earlier)"] {
        assert!(withdrawn.contains(step), "{step}: {withdrawn}");
    }
    let none = js_fn(ALLERGIES_JS, "noneItem");
    for step in ["cell(\"li\", none.text", "noneActions(none)"] {
        assert!(none.contains(step), "{step}: {none}");
    }
    let quiet = js_fn(ALLERGIES_JS, "quietList");
    assert!(quiet.contains("class: \"allergy-quiet\""), "{quiet}");
}

/// Every button the overlays add has an accessible name that says WHICH line it acts on
/// ("Correct: Allergy: penicillin → anaphylaxis"): a screen reader hears a list of identical
/// "Correct" buttons otherwise.
#[test]
fn every_line_button_names_its_line() {
    let button = js_fn(OVERLAYS_JS, "actionButton");
    assert!(
        button.contains(
            "button.setAttribute(\"aria-label\", button.textContent + \": \" + line.text);"
        ),
        "{button}"
    );
    // A row is named as the button that opened it ("De-label: <line>").
    let open = js_fn(OVERLAYS_JS, "openRow");
    assert!(
        open.contains("row.setAttribute(\"aria-label\", opener.getAttribute(\"aria-label\"));"),
        "{open}"
    );
}

/// Fix round 1, item 5: the four button labels are the page's (a `<template>` in
/// `#allergy-details`), so the header's claim that every label is written in index.html is true
/// and a wording change is one edit in one file.
#[test]
fn every_button_label_is_the_pages() {
    let at = position(details(), "<template id=\"allergy-line-buttons\">");
    let template = &details()[at..at + position(&details()[at..], "</template>")];
    for (act, label) in [
        ("correct", "Correct"),
        ("delabel", "De-label"),
        ("error", "Entered in error"),
        ("withdraw", "Withdraw de-label"),
    ] {
        assert!(
            template.contains(&format!(
                "<button type=\"button\" class=\"allergy-write\" data-act=\"{act}\">{label}</button>"
            )),
            "{label}: {template}"
        );
        assert!(
            !OVERLAYS_JS.contains(&format!("\"{label}\"")),
            "allergy_overlays.js words {label} itself"
        );
    }
}

/// Fix round 1, item 4 (review Minor 1): Cancel and Escape wait for a press in flight. Closing
/// the row mid-press does not stop the sign — it still lands, the clinician having "cancelled"
/// it — and its refusal would be written into a detached paragraph, focus falling to <body>.
#[test]
fn cancel_and_escape_wait_for_a_press_in_flight() {
    let open = js_fn(OVERLAYS_JS, "openRow");
    assert!(
        open.contains("if (!allergyBusy) closeOverlayRow(li);"),
        "{open}"
    );
    let keys = &OVERLAYS_JS[position(OVERLAYS_JS, "addEventListener(\"keydown\"")..];
    assert!(
        position(keys, "if (event.key !== \"Escape\" || allergyBusy) return;")
            < position(keys, "closeOverlayRow("),
        "{keys}"
    );
}

/// Final check (review HIGH-1): a list drawn while a press is in flight — a hold boundary's
/// unattended re-read, "Read this chart again", another pane's act — rebuilds every `<li>`, so
/// the open row and its error paragraph are detached. A refusal written there is never seen, and
/// the row's vanishing is exactly what success looks like ("I struck that NKDA"). So the row says
/// it only while it is still on screen; otherwise the status line does.
#[test]
fn a_refusal_after_a_redraw_mid_press_is_said_on_the_status_line() {
    let sign = js_fn(OVERLAYS_JS, "signOverlay");
    assert!(
        sign.contains("(said.isConnected ? setMessage(said, text) : sayAllergy(text))"),
        "{sign}"
    );
    assert!(
        sign.contains("await afterRefused(failure, patient, false, sayHere);"),
        "{sign}"
    );
}

/// Fix round 1, item 1 (review, ruling R-6a): a correction whose line is no longer a live line
/// of a NEW pane (corrected, de-labelled or withdrawn elsewhere) would livelock — Rust refuses
/// "That allergy line is no longer on this chart", the window re-reads, the correction survives
/// the redraw, and every Sign is refused again. So on any draw where `allergyCorrecting` is not a
/// live line's `event_id`, the correction is SUSPENDED: the typed words stay, Sign is down, and
/// the page's sentence replaces "Correcting: …". A line still listed but now on a doubted
/// chart is lost too (final check, review MEDIUM-3): it carries no act, and Rust refuses one —
/// the same livelock. Correct on a line ends the suspension (via
/// `resetAllergyForm` → `clearCorrection`); Cancel correction clears as before.
#[test]
fn a_correction_whose_target_left_the_pane_is_suspended_not_resent() {
    let render = js_fn(ALLERGIES_JS, "renderAllergies");
    assert!(
        position(render, "suspendLostCorrection(pane);")
            < position(render, "syncAllergyButtons();"),
        "{render}"
    );
    let suspend = js_fn(OVERLAYS_JS, "suspendLostCorrection");
    for step in [
        "if (allergyCorrecting === null) return;",
        "pane.drug_allergies.concat(pane.other_allergies, pane.adverse_effects)",
        "line.event_id === allergyCorrecting && !line.doubted_chart",
        "correctionSuspended = true;",
        "el(\"allergy-correcting-line\").hidden = true;",
        "el(\"allergy-correcting-gone\").hidden = false;",
    ] {
        assert!(suspend.contains(step), "{step}: {suspend}");
    }
    let sync = js_fn(ALLERGIES_JS, "syncAllergyButtons");
    assert!(
        sync.contains("el(\"allergy-sign\").disabled = down || correctionSuspended;"),
        "{sync}"
    );
    let clear = js_fn(OVERLAYS_JS, "clearCorrection");
    for step in [
        "correctionSuspended = false;",
        "el(\"allergy-correcting-gone\").hidden = true;",
        "syncAllergyButtons();",
    ] {
        assert!(clear.contains(step), "{step}: {clear}");
    }
    let start = js_fn(OVERLAYS_JS, "startCorrection");
    assert!(
        start.contains("el(\"allergy-correcting-line\").hidden = false;"),
        "{start}"
    );
    let at = position(INDEX_HTML, "<div id=\"allergy-correcting\" hidden>");
    let block = &INDEX_HTML[at..at + position(&INDEX_HTML[at..], "</div>")];
    assert!(
        block.contains(
            "<p id=\"allergy-correcting-gone\" hidden>The line you were correcting has changed on \
             the record, or its chart's link came into doubt — press Correct on its current line \
             if it has one, or Cancel correction.</p>"
        ),
        "{block}"
    );
    assert!(!OVERLAYS_JS.contains("has changed on the record"));
}

/// Fix round 1, item 3 (ruling (b)): each history list has its own heading, hidden with its list
/// (`fillAllergyList` hides `<id>-label`), so no struck or superseded line sits visually under
/// "Other allergies:".
#[test]
fn every_history_list_has_its_own_heading() {
    for (id, heading) in [
        ("allergy-previously", "Previously recorded"),
        ("allergy-historical", "Historically reported (de-labelled)"),
        ("allergy-entered-in-error", "Entered in error"),
        ("allergy-ignored", "Not counted"),
    ] {
        assert!(
            details().contains(&format!(
                "<p id=\"{id}-label\" hidden>{heading}:</p>\n            <ul id=\"{id}\" aria-labelledby=\"{id}-label\" hidden></ul>"
            )),
            "{id}"
        );
    }
    let fill = js_fn(ALLERGIES_JS, "fillAllergyList");
    assert!(fill.contains("el(id + \"-label\")"), "{fill}");
}

/// Every NESTED field the allergy scripts read off a payload is one Rust sends (Task 5's
/// reviewer hand-off: `every_field_allergies_js_reads_is_one_rust_sends` checks the top-level keys
/// of `pane`, `line` and `answer` only). A misspelt nested field reads as `undefined`: the Correct
/// form would prefill nothing, or a "none known" line would draw blank — silently.
#[test]
fn every_nested_field_the_scripts_read_is_one_rust_sends() {
    let keys = |v: serde_json::Value| -> BTreeSet<String> {
        v.as_object().expect("an object").keys().cloned().collect()
    };
    let words = LineWords {
        allergen_kind: String::new(),
        term: String::new(),
        kind: String::new(),
        category: String::new(),
        reaction_text: None,
        onset_precision: String::new(),
        onset_value: None,
        source: String::new(),
        coding: None,
    };
    let none = NoneLineView {
        event_id: String::new(),
        text: String::new(),
        doubted_chart: false,
    };
    let withdrawn = EnteredInErrorLine {
        text: String::new(),
        earlier: vec![],
    };
    let line = AllergyLine {
        event_id: String::new(),
        text: String::new(),
        prominent: false,
        doubted_chart: false,
        signer: None,
        recorded: String::new(),
        chart: String::new(),
        earlier: vec![],
        challenge: None,
        words: None,
    };
    for (binding, offered) in [
        ("words", keys(serde_json::to_value(&words).unwrap())),
        ("none", keys(serde_json::to_value(&none).unwrap())),
        ("withdrawn", keys(serde_json::to_value(&withdrawn).unwrap())),
        ("line", keys(serde_json::to_value(&line).unwrap())),
    ] {
        let mut read = fields_read_in(OVERLAYS_JS, binding);
        read.extend(fields_read_in(ALLERGIES_JS, binding));
        assert!(!read.is_empty(), "the scripts read `{binding}`");
        for field in read {
            assert!(
                offered.contains(&field),
                "the scripts read `{binding}.{field}`, Rust sends {offered:?}"
            );
        }
    }
}

/// Review C1: a line from a chart whose link to this one is in doubt may be a stranger's; the
/// window offers NO act on it — no Correct, De-label, Entered in error or Withdraw de-label on a
/// line, none on a "none known" line — before anything else is decided. (Rust refuses such a
/// target before the key anyway, `allergy_overlays::target_on`; this keeps the button away.)
#[test]
fn a_doubted_charts_line_carries_no_act() {
    let actions = js_fn(OVERLAYS_JS, "lineActions");
    let guard = position(actions, "if (line.doubted_chart) return [];");
    assert!(guard < position(actions, "actionButton("), "{actions}");
    let nones = js_fn(OVERLAYS_JS, "noneActions");
    let guard = position(nones, "if (none.doubted_chart) return [];");
    assert!(guard < position(nones, "actionButton("), "{nones}");
}

/// ADR-0097 decision 19: a historical (de-labelled) line carries Entered in error aimed at its OWN entry
/// (`line.event_id`) beside Withdraw de-label (aimed at the de-label), so striking a
/// de-labelled line is one act, as on paper.
#[test]
fn a_historical_line_can_be_struck_in_one_act() {
    let actions = js_fn(OVERLAYS_JS, "lineActions");
    let historical = &actions[position(actions, "if (line.challenge) {")..];
    let historical = &historical[..position(historical, "}\n")];
    for step in [
        "actionButton(\"withdraw\", line, (li, opener) => openRetraction(line.challenge, li, opener))",
        "actionButton(\"error\", line, (li, opener) => openRetraction(line.event_id, li, opener, line))",
    ] {
        assert!(historical.contains(step), "{step}: {historical}");
    }
}

/// Review I2: Entered in error on a line with earlier versions strikes them too (ruling R-4a), so
/// its row says so in the page's own words — and, on a line that can be corrected, how to undo a
/// correction instead. The sentence is hidden by default and shown only for such a line.
#[test]
fn striking_a_corrected_line_says_it_strikes_its_earlier_versions() {
    let template =
        &INDEX_HTML[position(INDEX_HTML, "<template id=\"allergy-retract-template\">")..];
    let template = &template[..position(template, "</template>")];
    assert!(
        template.contains(
            "<p class=\"allergy-quiet\" data-note=\"earlier\" hidden>This strikes the line and its \
             earlier versions.<span data-note=\"undo\"> To undo a correction, press Correct and \
             record it as it was.</span></p>"
        ),
        "{template}"
    );
    let open = js_fn(OVERLAYS_JS, "openRetraction");
    for step in [
        "const strikesEarlier = Boolean(line) && line.earlier.length > 0;",
        "row.querySelector('[data-note=\"earlier\"]').hidden = !strikesEarlier;",
        "row.querySelector('[data-note=\"undo\"]').hidden = !(strikesEarlier && line.words);",
    ] {
        assert!(open.contains(step), "{step}: {open}");
    }
    // Only an entry's own strike passes its line; a de-label's withdrawal and a none line do not.
    let nones = js_fn(OVERLAYS_JS, "noneActions");
    assert!(
        nones.contains("openRetraction(none.event_id, li, opener)"),
        "{nones}"
    );
}

/// Files under 500 lines (house rule).
#[test]
fn allergy_overlays_js_stays_small() {
    assert!(OVERLAYS_JS.lines().count() < 500);
}
