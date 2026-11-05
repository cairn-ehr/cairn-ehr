// A note and all its later parts, drawn as ONE note (sign-in slice 3c-1), and "Add to this note".
//
// On paper, a note someone wrote more on — a second writer finishing it, a clinician adding a
// line an hour later — is one entry on the page. The node groups the signed events into such a
// compound and Rust words it (`notes_compound.rs`'s `CompoundView`): the root's heading above, a
// state line when the parts went in differently, the parts' texts run together as one text with
// a seam line before each later part (who added it, when it applies), and a "{n} parts"
// disclosure holding each part's own heading. Every word of the compound is Rust's, set as text;
// only the "Add to this note" button's label and its accessible-name prefix are this file's.
//
// "Add to this note" opens the editor on a fresh draft that names the compound's ROOT as the note
// it adds to (`noteDraft.addendumTo`); every save, Sign and Discard sends that target back, so a
// press inside the save debounce still makes an addendum. Pinned in
// notes_compound_webview_tests.rs.
//
// Classic script, loaded right before notes.js, sharing main.js's global scope (`cell`,
// `displayedPatient`), notes.js's `noteItem`, `bylineOf`, `noteDraft` and `startNote`, and
// whos_here.js's `claimAsked`/`askWhosHere` — all read only when a chart is read or a button
// pressed, long after every script has loaded.
"use strict";

/**
 * One compound note (Rust's `CompoundView`). A one-part note is drawn as any note is (`noteItem`:
 * its text, then its byline, heard in its spoken form, #973). A note of two or more parts is
 * HEADED by its root's byline (spec §3.7) — under the text, the root writer's "signed by Nurse A"
 * would sit beneath the last writer's words and read as signing them — then its state line, then
 * the text a part at a time, each later part after its seam line (who added it, and when it
 * applies), then the parts behind a `<details>` disclosure (keyboard-operable by default, closed
 * until opened). "Add to this note" follows when this chart's note may be added to.
 */
function compoundItem(compound) {
  if (compound.parts.length === 0) return withAdd(noteItem(compound), compound);
  const li = document.createElement("li");
  li.append(bylineOf(compound));
  if (compound.state) li.append(cell("p", compound.state));
  for (const segment of compound.segments) {
    if (segment.seam) li.append(seamLine(segment));
    li.append(cell("p", segment.text, { class: "note-text" }));
  }
  const details = document.createElement("details");
  const parts = document.createElement("ol");
  parts.append(...compound.parts.map(noteItem));
  details.append(cell("summary", compound.parts_label), parts);
  li.append(details);
  return withAdd(li, compound);
}

/**
 * A later part's seam, Rust's words: shown with its `?` mark, heard with "unverified" — the shown
 * words hidden from a screen reader, the spoken ones from the eye, as a byline is (#973).
 */
function seamLine(segment) {
  const line = document.createElement("p");
  line.append(
    cell("span", segment.seam, { "aria-hidden": "true" }),
    cell("span", segment.seam_spoken, { class: "visually-hidden" }),
  );
  return line;
}

/**
 * `li`, with "Add to this note" appended when this chart's note may be added to and the editor
 * is free.
 */
function withAdd(li, compound) {
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
