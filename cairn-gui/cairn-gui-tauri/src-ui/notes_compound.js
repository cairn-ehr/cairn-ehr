// A note and all its later parts, drawn as ONE note (sign-in slice 3c-1), and "Add to this note".
//
// On paper, a note someone wrote more on — a second writer finishing it, a clinician adding a
// line an hour later — is one entry on the page. The node groups the signed events into such a
// compound and Rust words it (`notes_compound.rs`'s `CompoundView`): the parts' texts run together
// as one text under the root's heading, a state line when the parts went in differently, and a
// "{n} parts" disclosure holding each part's own heading. Every word is Rust's, set as text.
//
// "Add to this note" opens the editor on a fresh draft that names the compound's ROOT as the note
// it adds to (`noteDraft.addendumTo`); every save, Sign and Discard sends that target back, so a
// press inside the save debounce still makes an addendum. Pinned in
// notes_compound_webview_tests.rs.
//
// Classic script, loaded right before notes.js, sharing main.js's global scope (`cell`,
// `displayedPatient`), notes.js's `noteItem`, `noteDraft` and `startNote`, and whos_here.js's
// `claimAsked`/`askWhosHere` — all read only when a chart is read or a button pressed, long after
// every script has loaded.
"use strict";

/**
 * One compound note (Rust's `CompoundView`): drawn first as a plain note — its fields are a
 * one-part note's own, so `noteItem` shows its text and byline (heard in its spoken form, #973) —
 * then its state line, its parts behind a `<details>` disclosure (keyboard-operable by default,
 * closed until opened), and "Add to this note" when this chart's note may be added to and the
 * editor is free.
 */
function compoundItem(compound) {
  const li = noteItem(compound);
  if (compound.state) li.append(cell("p", compound.state));
  if (compound.parts.length > 0) {
    const details = document.createElement("details");
    const parts = document.createElement("ol");
    parts.append(...compound.parts.map(noteItem));
    details.append(cell("summary", compound.parts_label), parts);
    li.append(details);
  }
  if (compound.can_add && noteDraft === null) {
    // The accessible name says WHICH note: a list of notes is otherwise a list of identical buttons.
    const add = cell("button", "Add to this note", { type: "button", "aria-label": "Add to this note: " + compound.when });
    add.addEventListener("click", () => addToNote(compound));
    li.append(add);
  }
  return li;
}

/**
 * "Add to this note": open the editor on a new draft adding to `compound`'s root, saying Rust's
 * "Adding to…" line for it (`adds_line` — the same words the draft reads back with once it
 * exists). Behind the same gate as New note: somebody must have said who is writing, so the picker
 * asks first and the editor opens once it is answered.
 */
function addToNote(compound) {
  if (displayedPatient === null || noteDraft !== null) return;
  if (!claimAsked()) {
    askWhosHere(() => addToNote(compound));
    return;
  }
  startNote(compound.root_event_id, compound.adds_line);
}
