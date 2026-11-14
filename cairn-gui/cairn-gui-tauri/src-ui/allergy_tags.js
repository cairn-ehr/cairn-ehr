// The allergy check's marks on the patient's OWN medication list (allergy slice 3, M6). The node
// decides everything (cairn_node::allergy::check, worded in Rust); this file only DRAWS what a
// `MedicationRow` carries: `allergy_marks` (a confirmed hit: the loud "Allergy" tag and each mark
// as a line under the row), `allergy_notes` (earlier adverse effects: muted), and
// `allergy_unverified` (the small "not checked" tag, so silence never reads as checked).
//
// Everything is VISIBLE TEXT, never a hover-only `title` (#871). Only main.js's `renderRow` calls
// these, so the duplicates and link comparison views (their own renderers, rows that are never
// checked) can never look checked-clean through a shared renderer.
//
// Classic script, loaded after main.js, sharing its global scope (`cell`).
"use strict";

/** The loud tag's words, and the small one's. Pinned in drug_pick_webview_tests.rs. */
const ALLERGY_TAG = "Allergy";
const NOT_CHECKED_TAG = "not checked";
/** A line being charted whose warning is not an allergy (a previous adverse effect, when the
 *  site lowered the threshold, ADR-0104): never labelled "Allergy". Pinned in
 *  med_chart_threshold_tests.rs. */
const WARNING_TAG = "Warning";

/** The line's name cell: the drug (and source chart, on a combined list) then its allergy tag. */
function nameCell(row) {
  const th = cell("th", row.source ? row.primary + " — recorded on chart(s) " + row.source : row.primary, { scope: "row" });
  if (row.allergy_marks.length > 0) {
    th.append(" ", cell("span", ALLERGY_TAG, { class: "allergy-tag" }));
  } else if (row.allergy_unverified) {
    th.append(" ", cell("span", NOT_CHECKED_TAG, { class: "allergy-unchecked-tag" }));
  }
  return th;
}

/** The rows under a line: each mark (loud), each note (muted), spanning the table like a flag. */
function allergyRows(row) {
  const line = (text, attr, klass) => {
    const tr = document.createElement("tr");
    tr.setAttribute(attr, "true");
    const td = cell("td", text, { class: klass });
    td.colSpan = 5;
    tr.append(td);
    return tr;
  };
  return row.allergy_marks
    .map((mark) => line(mark, "data-allergy-mark", "allergy-prominent"))
    .concat(row.allergy_notes.map((note) => line(note, "data-allergy-note", "allergy-quiet")));
}
