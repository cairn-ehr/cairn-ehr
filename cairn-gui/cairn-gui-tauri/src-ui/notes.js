// The progress notes on the chart (sign-in slice 3a): the chart's signed notes, its open drafts,
// and one editor to write, autosave and sign a note — the paper chart's progress-note page, where
// writing and signing are the same pen.
//
// What the node does, not this file (cairn-gui-tauri's `notes.rs`, tested there): every sentence
// but the one below and notes_save.js's two; escaping markup (the RAW text is sent and `sign_note` escapes it once, at
// signing — never escape here); every time of recording (the save door stamps them from the
// database's clock — the only time sent is the writer's STATED one); and whether a draft is this
// terminal's to edit (`draft.here`). What IS decided here, each pinned in notes_webview_tests.rs:
// that Sign cancels a pending save, and that a draft started elsewhere is shown read-only. The save
// queue — the rhythm (ruling R14), a failed save keeping the text, a moved or signed-elsewhere
// draft's unsaved text kept as a new draft — is notes_save.js's, loaded just before this file.
//
// No browser storage, on purpose: a draft lives in the node, behind definer doors. A copy in
// the browser's storage would put draft prose on this machine's disk in the clear. The draft
// being edited is a JS variable only (`noteDraft`).
//
// Classic script, loaded right after signer.js and notes_save.js, sharing main.js's global scope (`el`, `cell`,
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

/** Said instead of signing a time the clinician only began to type (a date, no hour). */
const PARTLY_TYPED_TIME =
  "the time it happened is only partly typed — finish it, or clear it if the note is about now";

/**
 * The draft in the editor, `{ draftId, patientId, statedTime, savedText, writer, addendumTo, covers }`
 * — its own chart, never "whatever is open"; the last stated time and text the node TOOK for its
 * part (a partly typed time is never saved); the staff id of whoever was here when it opened (null
 * for "Don't know"), sent verbatim with every save and the sign; and the root of the note it adds
 * to (slice 3c-1; null for a new note), sent with every save, Sign and Discard; and the ids of the
 * writer's earlier unsigned parts the editor's "also covers" line names (slice 3c-2; empty for a new
 * note), sent with every Sign as `alsoVouches`, so what the strip showed is what Sign vouches. A
 * draft kept as a new one (notes_save.js's `keepUnsavedText`) copies both: text typed as an
 * addendum stays one.
 * Two marks are added later:
 * `mayBeSigned`, once a Sign of it might have signed (a `spent` save then forks nothing), and
 * `keptOnScreen`, once its unsaved text has been kept as a new draft (notes_save.js).
 * Read into a local named `held`, never `draft`: `draft` is a Rust `DraftView` here, and the
 * field-drift guard (notes_webview_tests.rs) reads every `draft.<field>` against it.
 */
let noteDraft = null;
/** The newest note read started; an older answer is dropped. */
let notesRevision = 0;
/**
 * The last Sign or Discard, as a promise of its outcome — a Sign's "signed", "not-signed" or
 * "unknown"; a Discard's "discarded", "not-discarded" or "discard-unknown"; or null when nothing
 * was pressed. It never rejects. The idle re-lock and a switch of who's here wait for it.
 */
let noteSigning = Promise.resolve(null);
/** A Sign or Discard was pressed and has not settled: a switch then says its outcome in the picker. */
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
  // The claim Rust chose `editable` for: the editor writes as THAT claim (final review item 7).
  const claim = pane.claim;
  for (const draft of pane.drafts) {
    if (draft.here && noteDraft === null && readForThisClaim(claim)) openDraft(draft, claim.staff_id);
  }
  // Drawn only now, as the drafts below are: a draft reopened just above holds the editor, and a
  // note drawn before it would offer an "Add to this note" that does nothing.
  el("note-list").replaceChildren(...pane.notes.map(compoundItem)); // notes_compound.js
  setMessage(el("notes-withheld"), pane.withheld_sentence);
  const listed = pane.drafts.filter((draft) => !(noteDraft && noteDraft.draftId === draft.draft_id));
  el("note-drafts").replaceChildren(...listed.map(draftItem));
  renderDiscards(pane); // notes_discard.js: their own list, never among the notes
}

/**
 * Whether a pane was read for the claim this window holds now: only then do its `editable` flags
 * say which part this person may edit. A read that answers after a pick waits for the next one.
 */
function readForThisClaim(claim) {
  return claimAsked() && whosHere().kind === claim.kind && whosHere().staff_id === claim.staff_id;
}

/**
 * One signed note: its text exactly as typed, then who wrote and signed it and when it applies
 * (`bylineOf`, heard in its spoken form, #973).
 */
function noteItem(note) {
  const li = document.createElement("li");
  li.append(cell("p", note.text, { class: "note-text" }), bylineOf(note));
  return li;
}

/**
 * A note's byline and time as one paragraph. A screen reader skips a bare `?`, and a paragraph's
 * aria-label is not read, so the shown byline is hidden from it and the spoken one ("written by
 * unverified Nurse A", #973) from the eye. Shared with
 * notes_compound.js, which draws it ABOVE a compound's text.
 */
function bylineOf(note) {
  const byline = document.createElement("p");
  byline.append(
    cell("span", note.byline, { "aria-hidden": "true" }),
    cell("span", note.byline_spoken, { class: "visually-hidden" }),
    " · " + note.when,
  );
  return byline;
}

/**
 * A draft not in the editor: its words and its parts, read-only. While the editor is free it
 * offers "Continue here" — the one way to write on it (it moves a draft held elsewhere here).
 */
function draftItem(draft) {
  const li = document.createElement("li");
  const parts = document.createElement("ol");
  parts.append(...draft.parts.map(partBlock));
  // An addendum draft says what it adds to on its own line, above its words (Rust's wording).
  if (draft.adding_to) li.append(cell("p", draft.adding_to));
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
 * Put this terminal's draft back in the editor, writing as `writer` — the staff id of the claim
 * the pane chose `editable` for (null for "Don't know"). Only the tail the claim may edit (Rust's
 * `editable`) goes in the textarea; every other part is drawn above it, and with no editable
 * tail the editor is empty: what is typed becomes a new part (M3: an unknown claim over an
 * unknown tail EDITS that tail, which Rust marks editable — never an empty editor over it).
 */
function openDraft(draft, writer) {
  const tail = draft.parts[draft.parts.length - 1];
  const editing = Boolean(tail && tail.editable);
  const above = editing ? draft.parts.slice(0, -1) : draft.parts;
  noteDraft = {
    draftId: draft.draft_id,
    patientId: displayedPatient,
    statedTime: editing ? tail.stated_time || "" : "",
    savedText: editing ? tail.text : "",
    writer: writer,
    addendumTo: draft.addendum_to, // the node's own target, sent back verbatim (null: a new note)
    covers: draft.covers,
  };
  showAddsTo(draft.adding_to);
  showCovers(draft.covers_line);
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
 * Start a note. Writing needs somebody to have said who is here (a name or "Don't know"): if
 * nobody has, the picker asks first and the editor opens once it is answered.
 */
function newNote() {
  if (displayedPatient === null || noteDraft !== null) return;
  if (!claimAsked()) {
    askWhosHere(newNote);
    return;
  }
  startNote(null, null);
}

/**
 * Open the editor on a fresh draft: its id minted here, nothing saved until something is typed.
 * `addendumTo` is the root of the note it adds to (notes_compound.js's `addToNote`), null for a
 * new note (`newNote`); `addsTo` is Rust's "Adding to…" line for it, shown above the text box;
 * `coversLine` the line below it naming what the signature also covers, and `covers` those parts'
 * ids (both absent for a new note, which covers nothing).
 */
function startNote(addendumTo, addsTo, coversLine, covers) {
  noteDraft = {
    draftId: crypto.randomUUID(),
    patientId: displayedPatient,
    statedTime: "",
    savedText: "",
    writer: whosHere().staff_id,
    addendumTo: addendumTo,
    covers: covers || [],
  };
  showAddsTo(addsTo);
  showCovers(coversLine);
  showEditor(true);
  noteStatus("");
  el("note-text").focus();
}

/**
 * Say, above the text box, what the draft in the editor adds to — `line`, worded in Rust — or
 * take the line down: it is shown only while an addendum draft is open.
 */
function showAddsTo(line) {
  const addsTo = el("note-adds-to");
  addsTo.textContent = line || "";
  addsTo.hidden = noteDraft === null || noteDraft.addendumTo === null;
}

/**
 * Say, below the "Adding to…" line, which of the writer's earlier unsigned parts this draft's
 * signature also covers — `line`, worded in Rust — or take it down: shown only when there is one.
 * Information, never a question: Sign asks nothing more.
 */
function showCovers(line) {
  const covers = el("note-covers");
  covers.textContent = line || "";
  covers.hidden = !line;
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
  // From this press on, a `spent` save of this draft may be this Sign's own: its text went with
  // the Sign, so it is never kept again as a new draft (final review I1).
  // What the press carries is remembered, so text typed after it (an unknown outcome leaves the
  // editor live) is still kept as a new draft (notes_save.js `sendDraft`, PR #984 review).
  const wasMaybeSigned = Boolean(held.mayBeSigned);
  const wasPressedText = held.pressedText;
  held.mayBeSigned = true;
  held.pressedText = el("note-text").value;
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
      addendumTo: held.addendumTo,
      alsoVouches: held.covers,
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    setSigning(false);
    // Every refusal but an interrupted commit recorded nothing: this press signed nothing.
    if (behindTheRecord(failure) !== "unknown") {
      held.mayBeSigned = wasMaybeSigned;
      held.pressedText = wasPressedText;
    }
    noteStatus("Could not sign this note: " + failureText(failure));
    // The cancelled save's text is still only on screen: save it now. A draft signed in another
    // window answers `spent`, and what it lacks is kept as a new draft (#945); one this window may
    // have signed writes nothing.
    void saveNoteNow();
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

/**
 * The last Sign or Discard, done, as its outcome (`noteSigning`'s values). idle.js's re-lock and
 * whos_here.js's switch wait for it.
 */
function signsSettled() {
  return noteSigning;
}

/** Whether a Sign or Discard is in flight right now. */
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
 * While a Sign or a Discard is in flight (`on`), both buttons are down and the editor read-only
 * (ruling R13): a key pressed then would be in neither the signed note (or the discard's audit
 * copy) nor any draft.
 */
function setSigning(on) {
  el("note-sign").disabled = on; // one press, one note
  el("note-discard").disabled = on;
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
  showAddsTo(null);
  showCovers(null);
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
  el("note-discards").replaceChildren();
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
// A Discard shares Sign's in-flight flags: idle.js's re-lock and whos_here.js's switch wait on
// `signsSettled()` / `signInFlight()`, and must wait for a discard exactly as for a Sign. Its
// "unknown" is renamed "discard-unknown", so the picker never words it as an unknown SIGNATURE
// ("discarded" and "not-discarded" are already a discard's own).
el("note-discard").addEventListener("click", () => {
  noteSignInFlight = true;
  noteSigning = discardNote()
    .then((outcome) => (outcome === "unknown" ? "discard-unknown" : outcome))
    .catch((failure) => {
      console.error(failure);
      return "discard-unknown"; // a fault in this window mid-discard: whether it discarded is not known
    })
    .finally(() => {
      noteSignInFlight = false;
    });
});
