// Discard draft (sign-in slice 3b-2b): throw away the draft in the editor, leaving a signed record
// that it existed — the paper counterpart is crossing out a page you wrote and initialling it.
//
// What the node does, not this file (cairn-gui-tauri's `notes_discard.rs`, tested there): every
// sentence; who may discard (only the writer of every part with text, by KEY); saving the text on
// screen and discarding it in ONE transaction, so the audit copy holds the last second of typing;
// and every time. What IS decided here, each pinned in notes_discard_webview_tests.rs: the press
// cancels a pending save and carries the text on screen, the editor is read-only while it is in
// flight, a refusal saves the text again, and the markers are text in their own list.
//
// No confirmation dialog, on purpose (principle 3): a dialog is a step paper never charges, and it
// is not a safety mechanism. The safety is the record itself — the discarded text is kept, signed.
//
// Classic script, loaded right after notes.js, sharing its globals (`noteDraft`, `noteSaveTimer`,
// `setSigning`, `closeEditor`, `noteStatus`, `PARTLY_TYPED_TIME`), notes_save.js's `saveNoteNow`
// and `NOT_SAVED`, and main.js's/signer.js's/chart_stale.js's helpers — all read only when a key is
// pressed or a chart is read. notes.js wires the button (it shares Sign's in-flight flags) and
// calls `renderDiscards` from `renderNotes`.
"use strict";

/**
 * Discard the draft in the editor. `signNote`'s shape step for step, because a discard is signed by
 * the same person at the same gesture: the text ON SCREEN goes with the press, so a pending save is
 * cancelled first; the strip's credential; the strip cleared both ways; the outcome said; the chart
 * read again. Answers "discarded", "not-discarded", or "unknown" (an interrupted commit).
 */
async function discardNote() {
  const held = noteDraft;
  if (held === null) return null;
  const when = el("note-when");
  if (when.validity.badInput) {
    // The press would carry "" — a time the clinician began, silently dropped from the audit copy.
    // Asked BEFORE the pending save is cancelled, so that save still goes.
    noteStatus(PARTLY_TYPED_TIME);
    return "not-discarded";
  }
  clearTimeout(noteSaveTimer);
  noteSaveTimer = null;
  // From this press on, a save of this draft answering `discarded` may be this press's own: its
  // text went with the discard, so it must never be kept again as a new draft (notes_save.js).
  const wasMaybeSigned = Boolean(held.mayBeSigned);
  held.mayBeSigned = true;
  setSigning(true);
  let report;
  try {
    report = await invoke("note_discard", {
      patientId: held.patientId,
      draftId: held.draftId,
      narrative: el("note-text").value,
      statedTime: when.value,
      writer: held.writer,
      credential: signerCredential(),
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    setSigning(false);
    // Every refusal but an interrupted commit recorded nothing: this press discarded nothing.
    if (behindTheRecord(failure) !== "unknown") held.mayBeSigned = wasMaybeSigned;
    // Worded in Rust, already "Not discarded: …": a prefix here would say it twice (found in the walk).
    noteStatus(failureText(failure));
    // The refused transaction rolled back its own save of the text on screen: save it again, so
    // "your text is still here" stays true past this window.
    void saveNoteNow();
    await refresh(el("note-status").textContent, behindTheRecord(failure));
    return behindTheRecord(failure) === "unknown" ? "unknown" : "not-discarded";
  }
  // Not known whether it was discarded: save the text first, so if it was NOT, the draft the
  // re-read reopens holds all of it; if it was, the save answers `discarded` (this press's own,
  // so it forks nothing). A save that got no answer leaves the text on screen, the editor live.
  if (report.behind_the_record && !(await saveNoteNow())) {
    setSigning(false);
    noteStatus(report.text + " · " + NOT_SAVED);
    await refresh(report.text, report.behind_the_record);
    return "unknown";
  }
  closeEditor();
  noteStatus(report.text); // worded in Rust: `notes::discard::note_discard_impl`
  await refresh(report.text, report.behind_the_record);
  return report.behind_the_record ? "unknown" : "discarded";
}

/**
 * Draw the chart's discarded drafts (Rust's `NotePane.discards`) in their own list, never among
 * the notes: a discarded draft was never entered in the record as a note (ADR-0087 R8).
 */
function renderDiscards(pane) {
  el("note-discards").replaceChildren(...pane.discards.map(discardItem));
}

/**
 * One marker: a one-line `<details>` whose summary says who discarded it and when, opening to
 * each part's heading and its text exactly as typed. Closed by default — it is an audit trace,
 * not something to read on every visit. Every word is Rust's, set as text.
 */
function discardItem(discard) {
  const li = document.createElement("li");
  const details = document.createElement("details");
  details.append(cell("summary", discard.summary));
  for (const discardPart of discard.parts) {
    details.append(cell("h3", discardPart.heading), cell("p", discardPart.text, { class: "note-text" }));
  }
  li.append(details);
  return li;
}
