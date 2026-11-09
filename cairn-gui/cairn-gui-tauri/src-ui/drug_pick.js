// The type-ahead under the allergy form's Substance field (allergy slice 3): while the allergen
// is a DRUG, typing three or more characters offers drugs and drug groups from the node's drug
// list (`drug_search`), so the entry can be CODED and the medication list checked against it.
// Choosing is optional: a name left as typed is recorded exactly as before (no drug list is never
// a reason to refuse an allergy).
//
// The one safety rule here: a coding vouches for THE NAME IT WAS PICKED WITH. Any later edit of
// the name, or leaving "Drug", drops it (`pickedCoding` is null again) — otherwise "penicillin"
// edited to "latex" would still be signed as the penicillin moiety.
//
// Words the clinician sees are pinned in drug_pick_webview_tests.rs. Classic script, loaded before
// allergies.js (which calls `pickedCoding` and `clearPick`), sharing main.js's `el`, `cell`, `invoke`.
"use strict";

const PICK_DELAY_MS = 150;
const PICK_MIN_CHARS = 3;
const NO_DRUG_LIST_HINT = "No drug list on this terminal: the name is recorded as typed";
const GROUP_SYSTEM = "drugref-class";

/** The option held (`{system, code, display}`), or null: nothing picked, or the name edited since. */
let heldCoding = null;
let pickOptions = [];
let pickActive = -1;
let pickTimer = null;
let pickRevision = 0; // an older answer is dropped

/** Whether the pressed allergen kind is Drug — read where it matters, never kept in a flag that a
 *  programmatic press (Correct's `pressChoice`) would leave out of step. */
function pickIsDrug() {
  return pressedChoice("allergen_kind") === "drug";
}

/** What `allergy_record` sends as `coding`: the held option, only while the kind is drug. */
function pickedCoding() {
  return pickIsDrug() ? heldCoding : null;
}

/** Hold `coding` (a line's, from Correct) as if picked; null holds nothing. */
function holdPick(coding) {
  heldCoding = coding;
  closePick();
}

/** The option's words in the list: a group says so. */
function pickLabel(option) {
  return option.system === GROUP_SYSTEM ? option.display + " (group)" : option.display;
}

/** Close the list and the hint (the options are forgotten). */
function closePick() {
  clearTimeout(pickTimer);
  pickRevision += 1;
  pickOptions = [];
  pickActive = -1;
  el("allergy-pick").replaceChildren();
  el("allergy-pick").hidden = true;
  el("allergy-pick-hint").hidden = true;
  el("allergy-term").setAttribute("aria-expanded", "false");
}

/** Forget the held coding and close (the form is emptied, or the name was edited). */
function clearPick() {
  heldCoding = null;
  closePick();
}

/** Mark option `index` as the one Enter would choose. */
function activatePick(index) {
  pickActive = index;
  [...el("allergy-pick").children].forEach((li, i) => li.setAttribute("aria-selected", String(i === index)));
}

/** Choose option `index`: the name becomes its display and the coding is held. */
function choosePick(index) {
  const option = pickOptions[index];
  if (!option) return;
  el("allergy-term").value = option.display;
  heldCoding = { system: option.system, code: option.code, display: option.display };
  closePick();
}

/** Draw an answer (`{available, options}`): the options, or the hint when there is no drug list. */
function drawPick(answer) {
  closePick();
  if (!answer.available) {
    el("allergy-pick-hint").textContent = NO_DRUG_LIST_HINT;
    el("allergy-pick-hint").hidden = false;
    return;
  }
  pickOptions = answer.options;
  el("allergy-pick").replaceChildren(
    ...pickOptions.map((option, i) => {
      const li = cell("li", pickLabel(option), { role: "option", "aria-selected": "false" });
      li.addEventListener("click", () => choosePick(i));
      return li;
    }),
  );
  el("allergy-pick").hidden = pickOptions.length === 0;
  el("allergy-term").setAttribute("aria-expanded", String(pickOptions.length > 0));
}

/** Ask the node for options for `text` (a stale answer, or one after the kind changed, is dropped). */
async function askPick(text) {
  const revision = ++pickRevision;
  try {
    const answer = await invoke("drug_search", { text: text });
    if (revision === pickRevision && pickIsDrug()) drawPick(answer);
  } catch (failure) {
    // The pick is optional: a failed search leaves the name to be recorded as typed.
    if (revision === pickRevision) closePick();
  }
}

el("allergy-term").addEventListener("input", () => {
  heldCoding = null; // any edit of the name drops the coding
  closePick();
  const text = el("allergy-term").value;
  if (pickIsDrug() && text.trim().length >= PICK_MIN_CHARS) {
    pickTimer = setTimeout(() => void askPick(text), PICK_DELAY_MS);
  }
});

el("allergy-term").addEventListener("keydown", (event) => {
  const n = pickOptions.length;
  if (event.key === "Escape") closePick();
  else if (n === 0) return;
  else if (event.key === "ArrowDown") activatePick((pickActive + 1) % n);
  else if (event.key === "ArrowUp") activatePick((pickActive - 1 + n) % n);
  else if (event.key === "Enter" && pickActive >= 0) choosePick(pickActive);
  else return;
  event.preventDefault();
});

// Leaving "Drug" drops the held coding and the list (ruling T5-d: the node refuses a coding on a
// non-drug entry). `pickedCoding` also checks the kind at signing, so this is the tidy-up, not the guard.
document.querySelector('.allergy-choice[data-field="allergen_kind"]').addEventListener("click", (event) => {
  const button = event.target.closest("button");
  if (button && button.dataset.value !== "drug") {
    heldCoding = null;
    closePick();
  }
});
