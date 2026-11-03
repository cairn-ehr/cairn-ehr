// The progress notes on the chart (sign-in slice 3a): the chart's signed notes, its open drafts,
// and one editor to write, autosave and sign a note — the paper chart's progress-note page, where
// writing and signing are the same pen.
//
// What the node does, not this file (cairn-gui-tauri's `notes.rs`, tested there): every sentence
// but the two below; escaping markup (the RAW text is sent and `sign_note` escapes it once, at
// signing — never escape here); every time of recording (the save door stamps them from the
// database's clock — the only time sent is the writer's STATED one); and whether a draft is this
// terminal's to edit (`draft.here`). What IS decided here, each pinned in notes_webview_tests.rs:
// the debounce, that Sign cancels a pending save, that a failed save keeps the text and says so,
// and that a draft started elsewhere is shown read-only.
//
// No browser storage, on purpose: a draft lives in the node, behind definer doors. A copy in
// the browser's storage would put draft prose on this machine's disk in the clear. The draft
// being edited is a JS variable only (`noteDraft`).
//
// Classic script, loaded right after signer.js, sharing main.js's global scope (`el`, `cell`,
// `setMessage`, `invoke`, `refresh`, `displayedPatient`), signer.js's `signerCredential` and
// `clearSigner`, funnel.js's `failureText` and chart_stale.js's `behindTheRecord` — all read only
// when a chart is read or a key is pressed, long after every script has loaded. main.js's
// `refresh` calls `readNotes`; its `clearChart` calls `clearNotes`.
"use strict";

/** One save a second while the clinician types: often enough to lose little, rarely enough to be cheap. */
const NOTE_SAVE_DEBOUNCE_MS = 1000;
/** Said when a save could not reach the node. The next keystroke tries again. */
const NOT_SAVED = "not saved yet — your text is still here";
/** Said instead of signing a time the clinician only began to type (a date, no hour). */
const PARTLY_TYPED_TIME =
  "the time it happened is only partly typed — finish it, or clear it if the note is about now";

/**
 * The draft in the editor, `{ draftId, patientId }` — its own chart, never "whatever is open".
 * Read into a local named `held`, never `draft`: `draft` is a Rust `DraftView` here, and the
 * field-drift guard (notes_webview_tests.rs) reads every `draft.<field>` against it.
 */
let noteDraft = null;
let noteSaveTimer = null;
/** Saves go out one after another, so an older text can never land after a newer one. */
let noteSaveQueue = Promise.resolve();
/** Saves queued or in flight. With none, a save is SENT at once, not on a later tick. */
let savesInFlight = 0;
/**
 * What a save last said on the status line (or a reopened draft's words): a later save that
 * works takes only THAT down — never a Sign's refusal, which a save sent after it must not hide.
 */
let saveSaid = null;
/** The newest note read started; an older answer is dropped. */
let notesRevision = 0;

/** Read the note pane for `patient` (main.js's `refresh`, beside the medication list). */
async function readNotes(patient) {
  const revision = ++notesRevision;
  try {
    const pane = await invoke("note_list", { patientId: patient });
    if (patient === displayedPatient && revision === notesRevision) renderNotes(pane);
  } catch (failure) {
    // The medication list is read on its own: a note read that failed never takes it down.
    if (patient === displayedPatient && revision === notesRevision) {
      setMessage(el("notes-error"), "Could not read the notes: " + failureText(failure));
    }
  }
}

/**
 * Draw a `note_list` answer (Rust's `NotePane`). A draft started at this terminal reopens into
 * the editor when the editor is free; the draft the editor already holds is not listed again
 * (and its text on screen, possibly newer than the node's, is left alone). Every other draft is
 * listed read-only.
 */
function renderNotes(pane) {
  setMessage(el("notes-error"), "");
  el("note-list").replaceChildren(...pane.notes.map(noteItem));
  setMessage(el("notes-withheld"), pane.withheld_sentence);
  const listed = [];
  for (const draft of pane.drafts) {
    if (draft.here && noteDraft === null) openDraft(draft);
    if (!(noteDraft && noteDraft.draftId === draft.draft_id)) listed.push(draftItem(draft));
  }
  el("note-drafts").replaceChildren(...listed);
}

/** One signed note: its text exactly as typed, then who signed it and when it applies. */
function noteItem(note) {
  const li = document.createElement("li");
  li.append(cell("p", note.text, { class: "note-text" }), cell("p", note.byline + " · " + note.when));
  return li;
}

/** A draft not in the editor: its words and its text, and nothing to press. */
function draftItem(draft) {
  const li = document.createElement("li");
  li.append(cell("p", draft.words), cell("p", draft.narrative, { class: "note-text" }));
  return li;
}

/** Put this terminal's draft back in the editor, as it was saved. */
function openDraft(draft) {
  noteDraft = { draftId: draft.draft_id, patientId: displayedPatient };
  el("note-text").value = draft.narrative;
  el("note-when").value = draft.stated_time || "";
  showEditor(true);
  noteStatus(draft.words); // "unsigned draft · started … · last typed …"
  saveSaid = draft.words; // the first save makes "last typed" untrue, so it takes it down
}

/** Start a note: a fresh draft id, minted here; nothing is saved until something is typed. */
function newNote() {
  if (displayedPatient === null || noteDraft !== null) return;
  noteDraft = { draftId: crypto.randomUUID(), patientId: displayedPatient };
  showEditor(true);
  noteStatus("");
  el("note-text").focus();
}

/** A keystroke in the note or its time: save once the typing pauses. */
function onNoteInput() {
  clearTimeout(noteSaveTimer);
  noteSaveTimer = setTimeout(saveNoteNow, NOTE_SAVE_DEBOUNCE_MS);
}

/**
 * Save the editor's text now, into its own draft and chart. The fields are read HERE, so a save
 * flushed as the chart closes (`clearNotes`) still carries what was typed. Returns the queue.
 */
function saveNoteNow() {
  clearTimeout(noteSaveTimer);
  noteSaveTimer = null;
  const held = noteDraft;
  if (held === null) return noteSaveQueue;
  const when = el("note-when");
  // A partly typed time has value "" (as if none were stated): saved as not stated YET, and the
  // next save, once it is complete, carries it. Sign refuses it instead (`signNote`).
  const sent = { held: held, narrative: el("note-text").value, statedTime: when.value };
  savesInFlight += 1;
  const send = () => sendDraft(sent).finally(() => (savesInFlight -= 1));
  // At once when idle: a save flushed as the chart closes must reach the node BEFORE
  // `close_chart`, or the node, no longer showing that chart, refuses it (found in the walk).
  // shortcut: a flush queued behind a save already in flight still goes after `close_chart`
  // and is refused (the last second of typing lost); hold the close for it if that bites.
  noteSaveQueue = savesInFlight === 1 ? send() : noteSaveQueue.then(send);
  return noteSaveQueue;
}

/** One save, as `saveNoteNow` read it. Says how it went only while its draft is still the one on screen. */
async function sendDraft(sent) {
  try {
    const view = await invoke("note_draft_save", {
      patientId: sent.held.patientId,
      draftId: sent.held.draftId,
      narrative: sent.narrative,
      statedTime: sent.statedTime,
    });
    if (noteDraft === sent.held) saySaved(view.saved ? "" : view.sentence);
  } catch (failure) {
    // Nothing here touches the field: the text stays as typed (Review Focus 2).
    if (noteDraft === sent.held) saySaved(NOT_SAVED);
  }
}

/** A save's word on the status line: a sentence is said; a save that worked clears only its own. */
function saySaved(text) {
  if (text || el("note-status").textContent === saveSaid) noteStatus(text);
  saveSaid = text || null;
}

/**
 * Sign the note: the text ON SCREEN goes with the press (Review Focus 4), so a pending save is
 * cancelled first — it could only say what Sign already carries (Review Focus 3). `cease`'s shape
 * otherwise: the strip's credential, the strip cleared both ways, the outcome said, then the chart
 * read again. Signed: the editor empties and the note is listed. Refused: the text stays, and the
 * status line says why.
 */
async function signNote() {
  const held = noteDraft;
  if (held === null) return;
  clearTimeout(noteSaveTimer);
  noteSaveTimer = null;
  const when = el("note-when");
  if (when.validity.badInput) {
    // Sent as "", it would sign as "not stated": a time the clinician began, silently dropped.
    noteStatus(PARTLY_TYPED_TIME);
    return;
  }
  const button = el("note-sign");
  button.disabled = true; // one press, one note
  let report;
  try {
    report = await invoke("note_sign", {
      patientId: held.patientId,
      draftId: held.draftId,
      narrative: el("note-text").value,
      statedTime: when.value,
      credential: signerCredential(),
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    button.disabled = false;
    noteStatus("Could not sign this note: " + failureText(failure));
    // The cancelled save's text is still only on screen: save it now (a signed draft answers
    // `spent` and nothing is written).
    void saveNoteNow();
    // shortcut: a stale refusal (already signed elsewhere) keeps the dead draft in the editor
    // until the chart is reopened; give it a way out if that turns up outside two-window races.
    await refresh(el("note-status").textContent, behindTheRecord(failure));
    return;
  }
  // Not known whether it was signed (#814): save the text first, so if it was NOT, the draft the
  // re-read reopens holds all of it; if it was, the save answers `spent` and writes nothing.
  if (report.behind_the_record) await saveNoteNow();
  closeEditor();
  noteStatus(report.text); // worded in Rust: `notes::note_sign_impl`
  await refresh(report.text, report.behind_the_record);
}

/** Say how the note pane's last act went. Never a dialog. */
function noteStatus(text) {
  el("note-status").textContent = text || "";
}

/** Show the editor (and Sign), or New note: one draft is written at a time. */
function showEditor(open) {
  el("note-editor").hidden = !open;
  el("new-note").hidden = open;
  el("note-sign").disabled = false;
}

/** Empty the editor and let go of its draft (the draft itself stays in the node). */
function closeEditor() {
  noteDraft = null;
  el("note-text").value = "";
  const when = el("note-when");
  when.value = "";
  // WebKit keeps a PARTLY typed time through `value = ""`; resetting the type discards it (holds.js).
  when.type = "text";
  when.type = "datetime-local";
  showEditor(false);
}

/**
 * The chart changed (main.js's `clearChart`): take the pane down, so one patient's notes are
 * never under another's header. The last second of typing is flushed to its OWN draft's chart
 * first; every note read in flight is outdated.
 */
function clearNotes() {
  if (noteSaveTimer !== null) void saveNoteNow();
  notesRevision += 1;
  closeEditor();
  el("note-list").replaceChildren();
  el("note-drafts").replaceChildren();
  setMessage(el("notes-withheld"), "");
  setMessage(el("notes-error"), "");
  noteStatus("");
}

el("new-note").addEventListener("click", newNote);
el("note-text").addEventListener("input", onNoteInput);
el("note-when").addEventListener("input", onNoteInput);
el("note-sign").addEventListener("click", () => void signNote());
