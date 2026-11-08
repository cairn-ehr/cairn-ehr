// The allergy list on the chart (allergy slice 1): the paper folder's red sticker. One terse line
// in the patient identity header (`#allergy-strip`), always on screen with the name it belongs to;
// "Show all" opens, in place, the full lists, the two "none known" buttons and the form for one
// entry. Never a dialog (principle 3).
//
// What the node does, not this file (cairn-gui-tauri's `allergies.rs`, tested there): every
// sentence on the strip and in the lists, which list an entry belongs to and whether it is
// prominent, every verdict on what was typed (no substance, no kind, no category, an onset it
// cannot read), and every outcome. What IS decided here, each pinned in allergies_webview_tests.rs:
// that nothing is pressed for "Allergy or Adverse effect" or "What happened" until the clinician
// presses it; which field a refusal is shown under; and that a write that MAY have recorded an
// entry never leaves a live Sign that would record it twice.
//
// No markup handling, on purpose: allergy text is plaintext end to end, so every line is drawn
// with `textContent` (main.js's `cell`) and nothing is escaped or parsed.
//
// Classic script, loaded after notes_discard.js and before funnel.js (whose boot can clear the
// chart), sharing main.js's global scope (`el`, `cell`, `setMessage`, `invoke`, `refresh`,
// `displayedPatient`), signer.js's `signerCredential` and `clearSigner`, funnel.js's
// `failureText` and chart_stale.js's `behindTheRecord` — all read only when a chart is read or a
// button is pressed, long after every script has loaded. main.js's `refresh` calls
// `drawAllergies`; its `clearChart` calls `clearAllergies`.
"use strict";

/**
 * What the form presses on load and after every signed entry. `kind` and `category` are null —
 * nothing pressed — on purpose (design §3.2's anti-goal): a default would sign an intolerance as
 * an allergy, or "not known" as a reaction, by a press that chose nothing.
 */
const ALLERGY_DEFAULTS = {
  allergen_kind: "drug",
  kind: null,
  category: null,
  source: "patient-stated",
};

/** The strip while a chart's allergies are being read: never blank, never mistaken for "none". */
const ALLERGY_READING = "Reading the allergy list…";

/**
 * Which field line a refusal of the form is shown under, by a fragment of Rust's own sentence
 * (`allergies::entry_before_the_key` and `parse_onset`; an `ErrorView` carries text only). Any
 * other refusal is said on the form's status line. Each fragment is pinned against allergies.rs.
 */
const FIELD_REFUSALS = [
  ["Name the substance.", "allergy-term-error"],
  ["Choose Allergy or Adverse effect.", "allergy-kind-error"],
  ["Choose what happened, or Not known.", "allergy-category-error"],
  ["is not a date this window can read", "allergy-onset-error"],
  ["is after today", "allergy-onset-error"],
];

/** The chart the drawn allergy list was read for; every write names it. Null while none is drawn. */
let allergyPatient = null;
/**
 * The chart SET that list was read over (the pane's `charts`), sent back with every write so the
 * node refuses one over a set that has changed since. Null whenever no list is drawn, or the one on
 * screen is known behind the record: every write needs it, so with none no write can be pressed.
 */
let allergyCharts = null;
/** The newest allergy read started; an older answer is dropped. */
let allergyRevision = 0;
/** A Sign or a "none known" press is in flight: one press, one signed act. */
let allergyBusy = false;

/** Read the allergy pane for `patient` (main.js's `refresh`, beside the notes and the medication list). */
async function drawAllergies(patient) {
  const revision = ++allergyRevision;
  try {
    const pane = await invoke("allergy_list", { patientId: patient });
    if (patient === displayedPatient && revision === allergyRevision) renderAllergies(pane, patient);
  } catch (failure) {
    // Read on its own: a failed allergy read never takes the medication list down. Nothing of an
    // older list stays up — the strip says the read failed, never an old "none known".
    if (patient === displayedPatient && revision === allergyRevision) {
      allergyCharts = null;
      clearAllergyLists();
      stripSays("Could not read the allergy list: " + failureText(failure), false);
      syncAllergyButtons();
    }
  }
}

/**
 * Draw an `allergy_list` answer (Rust's `AllergyPane`) for `patient`: the strip (prominent when it
 * names an allergy), the strip's own two "none known" buttons when nothing has been asked at all
 * (design §3.4), and the expanded lists.
 */
function renderAllergies(pane, patient) {
  allergyPatient = patient;
  allergyCharts = pane.charts;
  const named = pane.drug_allergies.concat(pane.other_allergies).some((line) => line.prominent);
  stripSays(pane.strip, named);
  el("allergy-strip-none").hidden = !(pane.drug === "not-yet-asked" && pane.other === "not-yet-asked");
  fillAllergyList("allergy-drug", pane.drug_allergies.map(lineItem));
  fillAllergyList("allergy-other", pane.other_allergies.map(lineItem));
  fillAllergyList("allergy-adverse", pane.adverse_effects.map(lineItem));
  fillAllergyList("allergy-previously", pane.previously_none.map((text) => cell("li", text, { class: "allergy-quiet" })));
  syncAllergyButtons();
}

/** The strip's line, and whether it names an allergy (the red sticker: shown prominent). */
function stripSays(text, prominent) {
  el("allergy-strip-text").textContent = text;
  el("allergy-strip").classList.toggle("allergy-prominent", Boolean(prominent));
}

/**
 * One line of an expanded list: Rust's text (a doubted link's mark included), then when and by
 * whom it was recorded. An allergy is prominent, anything else quiet; a line from a chart whose
 * link is in doubt is marked by style too (the text already says it).
 */
function lineItem(line) {
  const li = cell("li", line.text, { class: line.prominent ? "allergy-prominent" : "allergy-quiet" });
  if (line.doubted_chart) li.classList.add("allergy-doubted");
  const by = line.signer ? " by " + line.signer : "";
  li.append(cell("span", " · recorded " + line.recorded + by, { class: "allergy-quiet" }));
  return li;
}

/** Fill list `id` with `items`, hiding it and its label (`<id>-label`, if any) when empty. */
function fillAllergyList(id, items) {
  el(id).replaceChildren(...items);
  el(id).hidden = items.length === 0;
  const label = el(id + "-label");
  if (label) label.hidden = items.length === 0;
}

/** Take every list down (the strip's text is the caller's). */
function clearAllergyLists() {
  for (const id of ["allergy-drug", "allergy-other", "allergy-adverse", "allergy-previously"]) {
    fillAllergyList(id, []);
  }
  el("allergy-strip-none").hidden = true;
}

/** The row of toggle buttons for wire field `field` (`.allergy-choice[data-field]`). */
function choiceGroup(field) {
  return document.querySelector('.allergy-choice[data-field="' + field + '"]');
}

/** The wire word pressed in `field`'s row, or null when nothing is. */
function pressedChoice(field) {
  const button = choiceGroup(field).querySelector('button[aria-pressed="true"]');
  return button ? button.dataset.value : null;
}

/** Press `value` in `field`'s row and release the others (null releases all). */
function pressChoice(field, value) {
  for (const button of choiceGroup(field).querySelectorAll("button")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === value));
  }
}

/** Empty the form and press its defaults (`ALLERGY_DEFAULTS`); its refusals go with it. */
function resetAllergyForm() {
  for (const [field, value] of Object.entries(ALLERGY_DEFAULTS)) pressChoice(field, value);
  for (const id of ["allergy-term", "allergy-reaction-text", "allergy-onset"]) el(id).value = "";
  clearAllergyErrors();
}

/** Take down every refusal shown under a field. */
function clearAllergyErrors() {
  for (const [, id] of FIELD_REFUSALS) setMessage(el(id), "");
}

/** The entry as typed, in Rust's `EntryInput` shape: wire words, text exactly as typed. */
function typedAllergy() {
  return {
    allergen_kind: pressedChoice("allergen_kind"),
    term: el("allergy-term").value,
    kind: pressedChoice("kind"),
    category: pressedChoice("category"),
    reaction_text: el("allergy-reaction-text").value,
    onset: el("allergy-onset").value,
    source: pressedChoice("source"),
  };
}

/**
 * Sign the entry in the form, on the chart whose list is drawn. Nothing is checked here: Rust
 * says every verdict on what was typed (no kind pressed answers "Choose Allergy or Adverse
 * effect."), before it asks for the key, and the refusal lands under its field.
 */
async function recordAllergy() {
  const patient = allergyPatient;
  let answer;
  try {
    answer = await invoke("allergy_record", {
      patientId: patient,
      charts: allergyCharts,
      entry: typedAllergy(),
      credential: signerCredential(),
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    await afterRefused(failure, patient, true);
    return;
  }
  afterSigned(answer, patient, true);
}

/** Sign "no known allergies" (`scope` "all") or "no known drug allergies" ("drug"). */
async function recordNoneKnown(scope) {
  const patient = allergyPatient;
  let answer;
  try {
    answer = await invoke("allergy_none", {
      patientId: patient,
      charts: allergyCharts,
      scope: scope,
      credential: signerCredential(),
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    await afterRefused(failure, patient, false);
    return;
  }
  afterSigned(answer, patient, false);
}

/**
 * A write SIGNED (ruling R11: an Ok is always a landed sign). `wasEntry`: it signed the form's
 * entry, which is emptied so it cannot be signed twice. With the list read again (`answer.pane`)
 * it is drawn. Without it, nothing of the old list may stay up — it lacks what was just signed —
 * so the strip says Rust's outcome (which names "Read this chart again", the way back) and no
 * write can be pressed until a list draws. Nothing is drawn for a chart the clinician has left.
 */
function afterSigned(answer, patient, wasEntry) {
  // A chart the clinician has left: its next read says the truth, and the chart on screen's
  // own read in flight must not be outdated by an answer that is not about it.
  if (patient !== displayedPatient) return;
  allergyRevision += 1; // a read sent before this act cannot draw over its answer
  if (wasEntry) resetAllergyForm();
  if (answer.pane === null) {
    allergyCharts = null;
    clearAllergyLists();
    stripSays(answer.outcome, false); // said once, where the list was
    sayAllergy("");
    syncAllergyButtons();
    return;
  }
  sayAllergy(answer.outcome);
  renderAllergies(answer.pane, patient);
}

/**
 * A write that answered an `ErrorView` (`failure`). Behind the record (`behindTheRecord`): the
 * chart is read again (its sentence is that re-read's lead, on `#outcome`) and no write is live
 * until it draws; an outcome NOT known (#814) may have recorded the entry, so the form is
 * emptied too — a live Sign over it could record it twice.
 * Any other refusal recorded nothing: shown under its field (`FIELD_REFUSALS`) or on the status
 * line, with what was typed left as it is.
 */
async function afterRefused(failure, patient, wasEntry) {
  if (patient !== displayedPatient) return;
  const text = failureText(failure);
  const behind = behindTheRecord(failure);
  if (behind === "unknown") {
    if (wasEntry) resetAllergyForm();
  }
  if (behind) {
    allergyCharts = null;
    syncAllergyButtons();
    // Said once, as the re-read's lead on `#outcome`: chart_stale.js keeps it there (a failed
    // re-read is said after it; a read that overtakes this one still says it).
    await refresh(text, behind);
    return;
  }
  const field = FIELD_REFUSALS.find(([fragment]) => text.includes(fragment));
  if (field) setMessage(el(field[1]), text);
  else sayAllergy(text);
}

/** Say how the form's last act went. Never a dialog. */
function sayAllergy(text) {
  el("allergy-status").textContent = text || "";
}

/** Whether a write may be pressed now: a list is drawn for the chart on screen, nothing in flight. */
function allergyMayWrite() {
  return !allergyBusy && allergyCharts !== null && allergyPatient === displayedPatient;
}

/** Put every write button (Sign, the four "none known") and the text fields in step with the state. */
function syncAllergyButtons() {
  const down = !allergyMayWrite();
  el("allergy-sign").disabled = down;
  for (const button of document.querySelectorAll(".allergy-none")) button.disabled = down;
  for (const id of ["allergy-term", "allergy-reaction-text", "allergy-onset"]) el(id).readOnly = allergyBusy;
}

/**
 * Run one write press: refused at once if another is in flight or none may be pressed, every
 * button down until it is answered. A fault in this window mid-press means whether it recorded is
 * not known: said so, and handled as an unknown outcome is (no live write until a list draws).
 */
async function pressOnce(act) {
  if (!allergyMayWrite()) return;
  allergyBusy = true;
  clearAllergyErrors();
  sayAllergy("");
  syncAllergyButtons();
  try {
    await act();
  } catch (fault) {
    console.error(fault);
    allergyCharts = null;
    allergyRevision += 1; // a read already in flight must not draw and re-enable Sign
    resetAllergyForm();
    sayAllergy('This window failed while recording (' + String(fault) + '). Press "Read this chart again" to see what was recorded.');
  } finally {
    allergyBusy = false;
    syncAllergyButtons();
  }
}

/**
 * The chart changed (main.js's `clearChart`): take the pane down, so one patient's allergies are
 * never under another's header, and empty the form, so an entry typed for one patient is never
 * signed on the next. Every read in flight is outdated.
 */
function clearAllergies() {
  allergyRevision += 1;
  allergyPatient = null;
  allergyCharts = null;
  clearAllergyLists();
  stripSays(ALLERGY_READING, false);
  resetAllergyForm();
  sayAllergy("");
  el("allergy-details").open = false;
  syncAllergyButtons();
}

resetAllergyForm();
syncAllergyButtons();
for (const group of document.querySelectorAll(".allergy-choice")) {
  group.addEventListener("click", (event) => {
    const button = event.target.closest("button");
    if (button && !allergyBusy) pressChoice(group.dataset.field, button.dataset.value);
  });
}
// Enter in a field must not submit the page (the form's one act is Sign).
el("allergy-form").addEventListener("submit", (event) => event.preventDefault());
el("allergy-sign").addEventListener("click", () => void pressOnce(recordAllergy));
for (const button of document.querySelectorAll(".allergy-none")) {
  button.addEventListener("click", () => void pressOnce(() => recordNoneKnown(button.dataset.scope)));
}
