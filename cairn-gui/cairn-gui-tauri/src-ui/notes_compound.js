// A note and all its later parts, drawn as ONE note (sign-in slice 3c-1), and "Add to this note".
//
// On paper, a note someone wrote more on — a second writer finishing it, a clinician adding a
// line an hour later — is one entry on the page. The node groups the signed events into such a
// compound and Rust words it (`notes_compound.rs`'s `CompoundView`): the root's heading above, a
// state line when the parts went in differently, the parts' texts run together as one text with
// a seam line before each later part (who added it, when it applies), and a "{n} parts"
// disclosure holding each part's own heading. Every word of the compound is Rust's, set as text;
// only the "Add to this note" and "Sign off this note" buttons' labels and their accessible-name
// prefixes are this file's.
//
// "Add to this note" opens the editor on a fresh draft that names the compound's ROOT as the note
// it adds to (`noteDraft.addendumTo`); every save, Sign and Discard sends that target back, so a
// press inside the save debounce still makes an addendum. Pinned in
// notes_compound_webview_tests.rs.
//
// "Sign off this note" (slice 3c-2) is the co-signature at the foot of a paper page: one press
// signs every part of the note the presser has not yet signed. It sends back the part ids the
// pane SHOWED (`shown_parts`), so the node refuses a note that changed since it was read, rather
// than sign a part nobody saw. Pinned in notes_sign_off_webview_tests.rs.
//
// Classic script, loaded right before notes.js, sharing main.js's global scope (`cell`,
// `displayedPatient`, `invoke`, `refresh`), notes.js's `noteItem`, `bylineOf`, `noteDraft`,
// `startNote`, `noteStatus` and its in-flight flags (`noteSigning`, `noteSignInFlight`,
// `signInFlight`), signer.js's `signerCredential`/`clearSigner`, funnel.js's `failureText`,
// chart_stale.js's `behindTheRecord` and whos_here.js's `claimAsked`/`askWhosHere` — all read
// only when a chart is read or a button pressed, long after every script has loaded.
"use strict";

/**
 * One compound note (Rust's `CompoundView`). A one-part note is drawn as any note is (`noteItem`:
 * its text, then its byline, heard in its spoken form, #973). A note of two or more parts is
 * HEADED by its root's byline (spec §3.7) — under the text, the root writer's "signed by Nurse A"
 * would sit beneath the last writer's words and read as signing them — then its state line, then
 * the text a part at a time, each later part after its seam line (who added it, and when it
 * applies), then the parts behind a `<details>` disclosure (keyboard-operable by default, closed
 * until opened). Its buttons follow (`withButtons`).
 */
function compoundItem(compound) {
  if (compound.parts.length === 0) return withButtons(noteItem(compound), compound);
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
  return withButtons(li, compound);
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
 * `li`, with its buttons appended while the editor is free: "Add to this note" when this chart's
 * note may be added to, "Sign off this note" when Rust says it still needs a sign-off.
 */
function withButtons(li, compound) {
  if (compound.can_add && noteDraft === null) {
    // The accessible name says WHICH note: a list of notes is otherwise a list of identical buttons.
    const add = cell("button", "Add to this note", { type: "button", "aria-label": "Add to this note: " + compound.when });
    add.addEventListener("click", () => addToNote(compound));
    li.append(add);
  }
  if (compound.can_sign_off && noteDraft === null) li.append(signOffButton(compound));
  return li;
}

/**
 * "Add to this note": open the editor on a new draft adding to `compound`'s root, saying Rust's
 * "Adding to…" line for it (`adds_line` — the same words the draft reads back with once it
 * exists), and below it what its signature also covers (`covers_line`, with the ids `covers`).
 * Behind the same gate as New note: somebody must have said who is writing, so the picker asks
 * first and the editor opens once it is answered. Nothing opens while a press is in flight
 * (a sign-off holds the strip's credential with the editor free).
 */
function addToNote(compound) {
  if (displayedPatient === null || noteDraft !== null || signInFlight()) return;
  if (!claimAsked()) {
    askWhosHere(() => addToNote(compound));
    return;
  }
  startNote(compound.root_event_id, compound.adds_line, compound.covers_line, compound.covers);
}

/** The "Sign off this note" button; its accessible name says WHICH note, as Add's does. */
function signOffButton(compound) {
  const button = cell("button", "Sign off this note", { type: "button", "aria-label": "Sign off this note: " + compound.when });
  button.addEventListener("click", () => pressSignOff(compound, button));
  return button;
}

/**
 * A press of "Sign off this note". Behind the same gate as Add: somebody must have said who is
 * here, so the picker asks first and the press goes once it is answered. One strip credential
 * serves one act at a time, so nothing goes while the editor holds a draft or a Sign, Discard or
 * other sign-off is in flight, and the button is down until its own press settles. The press
 * rides the note pane's in-flight flags, so idle.js's re-lock and whos_here.js's switch wait for
 * it as for a Sign, and it settles with its outcome ("signed-off", "not-signed-off" or
 * "sign-off-unknown"), which the switch's picker words (`signOutcomeLine`): the switch closes
 * the chart, so without it the clinician would never learn how the press went.
 */
function pressSignOff(compound, button) {
  if (displayedPatient === null || noteDraft !== null || signInFlight()) return;
  if (!claimAsked()) {
    askWhosHere(() => pressSignOff(compound, button));
    return;
  }
  button.disabled = true;
  noteSignInFlight = true;
  noteSigning = signOffNote(compound)
    .catch((failure) => {
      console.error(failure);
      return "sign-off-unknown"; // a fault in this window mid-press: whether it signed off is not known
    })
    .finally(() => {
      noteSignInFlight = false;
      button.disabled = false;
    });
}

/**
 * Sign off `compound`: the part ids the pane showed and the strip's credential go to the node;
 * the strip is cleared both ways out (remembered as a signer only on success); then Rust's words
 * are said — signed off, refused (nothing signed), or NOT known (#814) — and the chart is read
 * again on every path, so the note's state line shows what the record now holds. Answers the
 * outcome for the picker: "signed-off", "not-signed-off" (nothing was written: a refusal, the
 * key's words, a node failure) or "sign-off-unknown" (an interrupted commit MAY have signed).
 */
async function signOffNote(compound) {
  const patient = displayedPatient;
  let said;
  let behind;
  let outcome;
  try {
    const report = await invoke("note_sign_off", {
      patientId: patient,
      rootEventId: compound.root_event_id,
      shownParts: compound.shown_parts,
      credential: signerCredential(),
    });
    clearSigner(true);
    said = report.text; // worded in Rust: `notes::sign_off::note_sign_off_impl`
    behind = report.behind_the_record;
    outcome = behind ? "sign-off-unknown" : "signed-off";
  } catch (failure) {
    clearSigner();
    said = failureText(failure); // already a whole sentence in Rust's words
    behind = behindTheRecord(failure);
    outcome = behind === "unknown" ? "sign-off-unknown" : "not-signed-off";
  }
  noteStatus(said);
  await refresh(said, behind);
  return outcome;
}
