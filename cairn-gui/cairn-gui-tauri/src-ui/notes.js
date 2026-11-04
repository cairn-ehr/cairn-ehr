// The progress notes on the chart (sign-in slice 3a): the chart's signed notes, its open drafts,
// and one editor to write, autosave and sign a note — the paper chart's progress-note page, where
// writing and signing are the same pen.
//
// What the node does, not this file (cairn-gui-tauri's `notes.rs`, tested there): every sentence
// but the two below; escaping markup (the RAW text is sent and `sign_note` escapes it once, at
// signing — never escape here); every time of recording (the save door stamps them from the
// database's clock — the only time sent is the writer's STATED one); and whether a draft is this
// terminal's to edit (`draft.here`). What IS decided here, each pinned in notes_webview_tests.rs:
// the save rhythm (ruling R14), that Sign cancels a pending save, that a failed save keeps the text and says so,
// and that a draft started elsewhere is shown read-only.
//
// No browser storage, on purpose: a draft lives in the node, behind definer doors. A copy in
// the browser's storage would put draft prose on this machine's disk in the clear. The draft
// being edited is a JS variable only (`noteDraft`).
//
// Classic script, loaded right after signer.js, sharing main.js's global scope (`el`, `cell`,
// `setMessage`, `invoke`, `refresh`, `displayedPatient`), signer.js's `signerCredential` and
// `clearSigner`, whos_here.js's `whosHere`/`claimAsked`/`askWhosHere`, funnel.js's `failureText`
// and chart_stale.js's `behindTheRecord` — all read only when a chart is read or a key is
// pressed, long after every script has loaded. main.js's `refresh` calls `readNotes`; its
// `clearChart` calls `clearNotes`.
//
// A draft is a sequence of writers' parts (slice 3b-1). The editor holds only the TAIL, and only
// when it is the current claim's (Rust's `editable`); every other part is drawn read-only above
// it, and what is typed becomes a new part. The writer is captured ONCE, when the editor opens
// (`noteDraft.writer`), and sent with every save and the sign: a save landing after a switch
// still names the person who typed it.
"use strict";

/** At most one save a second while the clinician types: often enough to lose little, rarely enough to be cheap. */
const NOTE_SAVE_EVERY_MS = 1000;
/** Said (with the reason) when a save failed. The next keystroke tries again. */
const NOT_SAVED = "not saved yet — your text is still here";
/** Said (with the reason) when the text saved but a new stated time was refused. */
const TIME_NOT_KEPT = "your text is saved, but not the time it happened: ";
/** Said instead of signing a time the clinician only began to type (a date, no hour). */
const PARTLY_TYPED_TIME =
  "the time it happened is only partly typed — finish it, or clear it if the note is about now";

/**
 * The draft in the editor, `{ draftId, patientId, statedTime, savedText, writer }` — its own
 * chart, never "whatever is open"; the last stated time and text the node TOOK for its part (a
 * partly typed time is never saved); and the staff id of whoever was here when it opened (null
 * for "Don't know"), sent verbatim with every save and the sign.
 * Read into a local named `held`, never `draft`: `draft` is a Rust `DraftView` here, and the
 * field-drift guard (notes_webview_tests.rs) reads every `draft.<field>` against it.
 */
let noteDraft = null;
let noteSaveTimer = null;
/** When the last save was sent (`performance.now()`); -Infinity for a draft not yet saved. */
let noteLastSave = -Infinity;
/**
 * Saves go out one after another, so an older text can never land after a newer one. It never
 * rejects (`sendDraft` catches everything): closing a chart awaits it (`notesSaved`).
 */
let noteSaveQueue = Promise.resolve();
/** The editor holds text the node may not have: typed since a save, or a save that failed. */
let noteUnsaved = false;
/**
 * What a save last said on the status line (or a reopened draft's words): a later save that
 * works takes only THAT down — never a Sign's refusal, which a save sent after it must not hide.
 */
let saveSaid = null;
/** The newest note read started; an older answer is dropped. */
let notesRevision = 0;
/**
 * The last Sign, as a promise of its outcome ("signed", "not-signed", "unknown", or null when
 * nothing was pressed); it never rejects. The idle re-lock and a switch of who's here wait for it.
 */
let noteSigning = Promise.resolve(null);
/** A Sign was pressed and has not settled: a switch then says its outcome in the picker. */
let noteSignInFlight = false;

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
 * the editor when the editor is free and somebody has said who is writing; the draft the editor
 * already holds is not listed again (and its text on screen, possibly newer than the node's, is
 * left alone). Every other draft is listed read-only, with "Continue here" while the editor is
 * free.
 */
function renderNotes(pane) {
  setMessage(el("notes-error"), "");
  el("note-list").replaceChildren(...pane.notes.map(noteItem));
  setMessage(el("notes-withheld"), pane.withheld_sentence);
  for (const draft of pane.drafts) {
    if (draft.here && noteDraft === null && claimAsked()) openDraft(draft);
  }
  const listed = pane.drafts.filter((draft) => !(noteDraft && noteDraft.draftId === draft.draft_id));
  el("note-drafts").replaceChildren(...listed.map(draftItem));
}

/** One signed note: its text exactly as typed, then who signed it and when it applies. */
function noteItem(note) {
  const li = document.createElement("li");
  li.append(cell("p", note.text, { class: "note-text" }), cell("p", note.byline + " · " + note.when));
  return li;
}

/**
 * A draft not in the editor: its words and its parts, read-only. While the editor is free it
 * offers "Continue here" — the one way to write on it (it moves a draft held elsewhere here).
 */
function draftItem(draft) {
  const li = document.createElement("li");
  const parts = document.createElement("ol");
  parts.append(...draft.parts.map(partBlock));
  li.append(cell("p", draft.words), parts);
  if (noteDraft === null) {
    const go = cell("button", "Continue here", { type: "button", "aria-label": "Continue here: " + draft.words });
    go.addEventListener("click", () => void continueHere(draft));
    li.append(go);
  }
  return li;
}

/**
 * One writer's part, read-only: headed by the claimed writer ("?Nurse A", heard "unverified
 * Nurse A" — the `?` is text and spoken, never a glyph alone), then its text exactly as typed.
 */
function partBlock(part) {
  const li = document.createElement("li");
  li.append(
    cell("h3", part.writer_shown, { "aria-label": part.writer_spoken }),
    cell("p", part.text, { class: "note-text" }),
    cell("p", part.words),
  );
  return li;
}

/**
 * Put this terminal's draft back in the editor. Only the tail the claim may edit (Rust's
 * `editable`) goes in the textarea; every other part is drawn above it, and with no editable
 * tail the editor is empty: what is typed becomes a new part (M3: an unknown claim over an
 * unknown tail EDITS that tail, which Rust marks editable — never an empty editor over it).
 */
function openDraft(draft) {
  const tail = draft.parts[draft.parts.length - 1];
  const editing = Boolean(tail && tail.editable);
  const above = editing ? draft.parts.slice(0, -1) : draft.parts;
  noteDraft = {
    draftId: draft.draft_id,
    patientId: displayedPatient,
    statedTime: editing ? tail.stated_time || "" : "",
    savedText: editing ? tail.text : "",
    writer: whosHere().staff_id,
  };
  el("note-parts").replaceChildren(...above.map(partBlock));
  el("note-text").value = editing ? tail.text : "";
  el("note-when").value = noteDraft.statedTime;
  showEditor(true);
  // "unsigned draft · started … · last typed …", added to — never in place of — what the same
  // gesture said: a Sign that worked reopens the next draft here, and "Signed" must stay seen.
  const said = el("note-status").textContent;
  noteStatus(said ? said + " · " + draft.words : draft.words);
  saveSaid = el("note-status").textContent; // the first save makes "last typed" untrue, so it takes it down
}

/**
 * Start a note: a fresh draft id, minted here; nothing is saved until something is typed.
 * Writing needs somebody to have said who is here (a name or "Don't know"): if nobody has, the
 * picker asks first and the editor opens once it is answered.
 */
function newNote() {
  if (displayedPatient === null || noteDraft !== null) return;
  if (!claimAsked()) {
    askWhosHere(newNote);
    return;
  }
  noteDraft = {
    draftId: crypto.randomUUID(),
    patientId: displayedPatient,
    statedTime: "",
    savedText: "",
    writer: whosHere().staff_id,
  };
  showEditor(true);
  noteStatus("");
  el("note-text").focus();
}

/**
 * "Continue here" on a listed draft: a draft held at another terminal is MOVED here (maintainer
 * decision 2), then the pane is read again, which opens it in the editor. Nobody asked yet: the
 * picker asks first.
 */
async function continueHere(draft) {
  if (!claimAsked()) {
    askWhosHere(() => void continueHere(draft));
    return;
  }
  const patient = displayedPatient;
  if (patient === null || noteDraft !== null) return;
  if (!draft.here) {
    try {
      const took = await invoke("note_draft_take", { patientId: patient, draftId: draft.draft_id });
      if (!took.taken) noteStatus(took.sentence);
    } catch (failure) {
      noteStatus("Could not continue this note here: " + failureText(failure));
    }
  }
  await readNotes(patient);
}

/**
 * A keystroke in the note or its time (ruling R14): the first of a burst saves at once, so the
 * node stamps `recording_started` when writing began; later ones wait until a second has passed
 * since the last save. The timer is never pushed back by the next keystroke, so continuous typing
 * still saves every second (a power cut loses at most that), and it reads the fields when it
 * fires, so it is also the save after the last keystroke.
 */
function onNoteInput() {
  noteUnsaved = true;
  if (noteSaveTimer !== null) return; // a save is already due, and will carry this keystroke
  const wait = noteLastSave + NOTE_SAVE_EVERY_MS - performance.now();
  if (wait <= 0) void saveNoteNow();
  else noteSaveTimer = setTimeout(saveNoteNow, wait);
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
  noteLastSave = performance.now();
  const when = el("note-when");
  const sent = {
    held: held,
    narrative: el("note-text").value,
    // A partly typed time reads as "" — saved, it would erase the time the draft holds. `null`
    // sends the last time the node took instead, read by `sendDraft` in queue order, after every
    // earlier save has answered.
    statedTime: when.validity.badInput ? null : when.value,
  };
  noteSaveQueue = noteSaveQueue.then(() => sendDraft(sent));
  return noteSaveQueue;
}

/** Every save queued so far, done (funnel.js's `closeChart` waits for it before `close_chart`). */
function notesSaved() {
  return noteSaveQueue;
}

/**
 * One save, as `saveNoteNow` read it. Says how it went, and marks the editor saved or not, only
 * while its draft is still the one on screen. Resolves whether the node ANSWERED: saved, or a
 * sentence no retry can change (spent, elsewhere, no chart) — either way nothing is left to send.
 */
async function sendDraft(sent) {
  const mine = () => noteDraft === sent.held;
  const taken = sent.held.statedTime;
  const asked = sent.statedTime === null ? taken : sent.statedTime;
  try {
    const view = await invoke("note_draft_save", {
      patientId: sent.held.patientId,
      draftId: sent.held.draftId,
      narrative: sent.narrative,
      statedTime: asked,
      writer: sent.held.writer,
    });
    if (view.moved) return keepMovedText(sent, view);
    sent.held.statedTime = asked;
    if (view.saved) sent.held.savedText = sent.narrative;
    if (mine()) {
      noteUnsaved = false;
      saySaved(view.saved ? "" : view.sentence);
    }
    return true;
  } catch (failure) {
    // A new time the node refused (one this clock never showed) must not hold the TEXT back: the
    // door writes both, so send the text again with the last time the node took, and say why.
    // The editor stays UNSAVED (its time is not in the node), so the next save — or the flush
    // as the chart closes — sends the time again: a passing failure that was not the time heals.
    if (asked !== taken && (await sendDraft({ ...sent, statedTime: taken }))) {
      if (mine()) {
        noteUnsaved = true;
        saySaved(TIME_NOT_KEPT + failureText(failure));
      }
      return true;
    }
    // Nothing here touches the field: the text stays as typed (Review Focus 2).
    if (mine()) {
      noteUnsaved = true;
      saySaved(NOT_SAVED + " (" + failureText(failure) + ")");
    }
    return false;
  }
}

/**
 * What of `text` a moved draft lacks, and the sentence (Rust's, from the save's `view`) that is
 * true for it. `savedText` is what this window last saved for the part and the node took — it
 * went on with the moved note. Typed after it: only the rest is new. Nothing after it: nothing
 * is kept (`kept: null`). The saved text itself edited: all of the text is kept, and said so.
 * Pure.
 */
function movedRemainder(text, savedText, view) {
  if (!text.startsWith(savedText)) return { kept: text, said: view.moved_everything };
  const rest = text.slice(savedText.length);
  if (rest.trim() === "") return { kept: null, said: view.moved_nothing_new };
  return { kept: rest, said: view.sentence };
}

/**
 * Review Focus 2: a save found its draft taken to another terminal. What the node lacks
 * (`movedRemainder`) is kept as a NEW draft of its own (a fresh id, the same writer), sent at
 * once in queue order, so a chart closing behind it still waits for it: nothing typed is lost,
 * and nothing saved is written twice. The editor's text when it still holds the draft (the
 * editor then shows only what the new draft holds), else what the save carried.
 */
async function keepMovedText(sent, view) {
  const onScreen = noteDraft === sent.held;
  const text = onScreen ? el("note-text").value : sent.narrative;
  const { kept, said } = movedRemainder(text, sent.held.savedText, view);
  if (kept === null) {
    if (onScreen) {
      closeEditor();
      noteStatus(said);
      void readNotes(sent.held.patientId); // the note is listed where it went on
    }
    return true;
  }
  const fork = { ...sent.held, draftId: crypto.randomUUID(), savedText: "" };
  if (onScreen) {
    noteDraft = fork;
    el("note-parts").replaceChildren(); // the parts above went on with the moved draft
    el("note-text").value = kept;
    noteStatus(said);
    saveSaid = null; // a later save that works must not take the sentence down
  }
  return sendDraft({ held: fork, narrative: kept, statedTime: sent.statedTime });
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
  if (held === null) return null;
  const when = el("note-when");
  if (when.validity.badInput) {
    // Sent as "", it would sign as "not stated": a time the clinician began, silently dropped.
    // Asked BEFORE the pending save is cancelled, so that save still goes.
    noteStatus(PARTLY_TYPED_TIME);
    return "not-signed";
  }
  clearTimeout(noteSaveTimer);
  noteSaveTimer = null;
  setSigning(true);
  let report;
  try {
    report = await invoke("note_sign", {
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
    noteStatus("Could not sign this note: " + failureText(failure));
    // The cancelled save's text is still only on screen: save it now (a signed draft answers
    // `spent` and nothing is written).
    void saveNoteNow();
    // shortcut: a stale refusal (already signed elsewhere) keeps the dead draft in the editor
    // until the chart is reopened; give it a way out (#945).
    await refresh(el("note-status").textContent, behindTheRecord(failure));
    // An interrupted commit MAY have signed (#713); every other refusal recorded nothing.
    return behindTheRecord(failure) === "unknown" ? "unknown" : "not-signed";
  }
  // Not known whether it was signed (#814): save the text first, so if it was NOT, the draft the
  // re-read reopens holds all of it; if it was, the save answers `spent` and writes nothing. A
  // save that got no answer leaves the text where it is — on screen, with the editor live.
  if (report.behind_the_record && !(await saveNoteNow())) {
    setSigning(false);
    noteStatus(report.text + " · " + NOT_SAVED);
    await refresh(report.text, report.behind_the_record);
    return "unknown";
  }
  closeEditor();
  noteStatus(report.text); // worded in Rust: `notes::note_sign_impl`
  await refresh(report.text, report.behind_the_record);
  return report.behind_the_record ? "unknown" : "signed";
}

/** The last Sign, done, as its outcome (idle.js's re-lock and whos_here.js's switch wait for it). */
function signsSettled() {
  return noteSigning;
}

/** Whether a Sign is in flight right now. */
function signInFlight() {
  return noteSignInFlight;
}

/** Say how the note pane's last act went. Never a dialog. */
function noteStatus(text) {
  el("note-status").textContent = text || "";
}

/** Show the editor (and Sign), or New note: one draft is written at a time. */
function showEditor(open) {
  el("note-editor").hidden = !open;
  el("new-note").hidden = open;
  setSigning(false);
}

/**
 * While a Sign is in flight (`on`), Sign is down and the editor read-only (ruling R13): a key
 * pressed then would be in neither the signed note nor any draft.
 */
function setSigning(on) {
  el("note-sign").disabled = on; // one press, one note
  el("note-text").readOnly = on;
  el("note-when").readOnly = on;
}

/** Empty the editor and let go of its draft (the draft itself stays in the node). */
function closeEditor() {
  clearTimeout(noteSaveTimer);
  noteSaveTimer = null;
  noteDraft = null;
  noteUnsaved = false;
  noteLastSave = -Infinity;
  el("note-parts").replaceChildren();
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
 * never under another's header. Text the node may not have — typed since the last save, or a
 * save that failed — is flushed to its OWN draft's chart first (closeChart awaits it before
 * `close_chart`); every note read in flight is outdated.
 * shortcut: a flush that cannot reach the node either is lost with the editor (#946).
 */
function clearNotes() {
  if (noteUnsaved || noteSaveTimer !== null) void saveNoteNow();
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
el("note-sign").addEventListener("click", () => {
  noteSignInFlight = true;
  noteSigning = signNote()
    .catch((failure) => {
      console.error(failure);
      return "unknown"; // a fault in this window mid-sign: whether it signed is not known
    })
    .finally(() => {
      noteSignInFlight = false;
    });
});
