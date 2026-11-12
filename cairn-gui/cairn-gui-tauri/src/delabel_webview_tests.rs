//! Charting slice 1b, Task 5: the webview half of the same-drug de-label offer — an offer button
//! on a medication-list line (`med_list_delabel.js`) that opens the allergy list's De-label row
//! prefilled (never signing), and that row's drugs-only pick (`delabel_pick.js`).
//!
//! The window has no JS test rig (no npm, by design): these read the scripts' source and pin the
//! CHOICE or ORDER a safety property is, as `med_list_webview_tests.rs` does. VISIBILITY, not only
//! text: each element the clinician must see is un-hidden by the script (or never hidden) and no
//! stylesheet rule hides it; the behaviour was also walked headlessly (see the task report).
use crate::med_chart_webview_tests::not_hidden_by_css;
use crate::source_guard::{js_fn, position};

const OFFER_JS: &str = include_str!("../src-ui/med_list_delabel.js");
const PICK_JS: &str = include_str!("../src-ui/delabel_pick.js");
const OVERLAYS_JS: &str = include_str!("../src-ui/allergy_overlays.js");
const LIST_JS: &str = include_str!("../src-ui/med_list.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// An offer is a plain button whose text is Rust's `words`, drawn un-hidden whenever there is
/// one, and no dialog stands between press and row (principle 3).
#[test]
fn the_offer_is_a_button_with_rusts_words_and_no_dialog() {
    let draw = js_fn(OFFER_JS, "drawDelabelOffers");
    for step in [
        "cell(\"button\", offer.words,",
        "type: \"button\"",
        "r.offers.hidden = offers.length === 0;",
        "line.view.delabel_offers",
    ] {
        assert!(draw.contains(step), "{step}: {draw}");
    }
    for banned in ["confirm(", "alert(", "<dialog", "showModal"] {
        assert!(
            !OFFER_JS.contains(banned) && !PICK_JS.contains(banned),
            "{banned}"
        );
    }
    // drawLine calls it, once, and it is loaded after med_list.js.
    assert!(js_fn(LIST_JS, "drawLine").contains("drawDelabelOffers(line);"));
    assert!(
        position(INDEX_HTML, "src=\"med_list.js\"")
            < position(INDEX_HTML, "src=\"med_list_delabel.js\"")
    );
    assert!(
        position(INDEX_HTML, "src=\"delabel_pick.js\"")
            < position(INDEX_HTML, "src=\"allergy_overlays.js\"")
    );
    not_hidden_by_css(".med-list-delabel");
}

/// A press opens the row (prefilled with the offer's drug, coding and today) and signs nothing:
/// the function never reaches `invoke`.
#[test]
fn a_press_opens_the_delabel_row_prefilled_and_signs_nothing() {
    let press = js_fn(OFFER_JS, "pressDelabelOffer");
    for step in [
        "openDelabel({ event_id: offer.target }, li, delabel,",
        "tolerated: offer.tolerated",
        "coding: offer.coding",
        "date: todayDdMmYyyy()",
        "el(\"allergy-details\").open = true;",
    ] {
        assert!(press.contains(step), "{step}: {press}");
    }
    assert!(!press.contains("invoke("), "a press signs nothing: {press}");
    assert!(
        !OFFER_JS.contains("invoke("),
        "the offer script never calls the node"
    );
    // The row's prefill is applied, and the row still needs its own Sign.
    let open = js_fn(OVERLAYS_JS, "openDelabel");
    assert!(
        open.contains("function openDelabel")
            || OVERLAYS_JS.contains("function openDelabel(line, li, opener, prefill)")
    );
    assert!(position(open, "if (prefill)") < position(open, "onSign(row,"));
}

/// With no drawn live line for the target (or one with no De-label button), the sentence is said
/// and nothing is opened.
#[test]
fn a_press_on_an_allergy_no_longer_drawn_says_so() {
    let press = js_fn(OFFER_JS, "pressDelabelOffer");
    assert!(OFFER_JS.contains("That allergy is no longer on the list; look at it again"));
    assert!(
        position(press, "sayAllergy(OFFER_GONE);") < position(press, "openDelabel("),
        "{press}"
    );
    assert!(press.contains("if (!delabel) {"), "{press}");
    // Only LIVE lists are searched, and allergies.js tags each drawn line with its event id.
    for id in ["allergy-drug", "allergy-other", "allergy-adverse"] {
        assert!(OFFER_JS.contains(&format!("\"{id}\"")), "{id}");
    }
    assert!(!OFFER_JS.contains("allergy-historical"));
    assert!(include_str!("../src-ui/allergies.js").contains("li.dataset.event = line.event_id;"));
}

/// Review Focus 1: any edit of the Tolerated field drops the coding BEFORE anything else, and the
/// "Coded as" line goes with it; every element the clinician reads is in the row template,
/// un-hidden only by the script.
#[test]
fn editing_the_tolerated_field_drops_its_coding() {
    let wire = js_fn(PICK_JS, "wireDelabelPick");
    let handler = &wire[position(wire, "input.addEventListener(\"input\", () => {")..];
    let first = handler.lines().nth(1).expect("a first statement");
    assert!(first.contains("holdDelabelCoding(row, null);"), "{first}");
    let hold = js_fn(PICK_JS, "holdDelabelCoding");
    for step in ["line.hidden = !coding;", "codedAsWords(coding)"] {
        assert!(hold.contains(step), "{step}: {hold}");
    }
    let template = &INDEX_HTML[position(INDEX_HTML, "id=\"allergy-delabel-template\"")..];
    let template = &template[..position(template, "</template>")];
    for note in [
        "data-note=\"coded-as\"",
        "data-note=\"pick-hint\"",
        "data-field=\"pick\"",
    ] {
        assert!(template.contains(note), "{note}");
    }
    not_hidden_by_css(".allergy-overlay-row [data-field=\"pick\"]");
    // Choosing sets the name and the coding together.
    assert!(js_fn(PICK_JS, "wireDelabelPick")
        .contains("holdDelabelCoding(row, { system: option.system"));
}

/// The row signs the held coding, or null: never a stale or invented one.
#[test]
fn the_row_sends_the_held_coding_or_null() {
    let open = js_fn(OVERLAYS_JS, "openDelabel");
    assert!(
        open.contains("toleratedCoding: delabelCoding(row),"),
        "{open}"
    );
    assert!(js_fn(PICK_JS, "delabelCoding").contains("return row.heldCoding || null;"));
}

/// After a signed overlay the allergy list is drawn from the answer, then the medication list
/// reads again (its offers and marks changed with it).
#[test]
fn the_list_rereads_after_a_delabel() {
    let sign = js_fn(OVERLAYS_JS, "signOverlay");
    assert!(
        position(sign, "afterSigned(answer, patient, false);")
            < position(sign, "void readMedDraft();"),
        "{sign}"
    );
}

/// Files under 500 lines (house rule): med_list.js was at 497 and gains one call.
#[test]
fn med_list_js_stays_under_500_lines() {
    for (name, js) in [
        ("med_list", LIST_JS),
        ("med_list_delabel", OFFER_JS),
        ("delabel_pick", PICK_JS),
        ("allergy_overlays", OVERLAYS_JS),
    ] {
        assert!(
            js.lines().count() < 500,
            "{name}.js is {} lines",
            js.lines().count()
        );
    }
}
