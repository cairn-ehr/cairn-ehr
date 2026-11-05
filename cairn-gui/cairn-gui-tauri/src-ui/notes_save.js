// The note editor's save queue (split from notes.js, sign-in slice 3b-1): the one-second
// autosave rhythm (ruling R14), saves sent one after another, what a save that failed says, and
// what is kept when a save finds its draft gone on without this window — moved to another
// terminal, put into the record unsigned by the quarantine timer, or signed in another window.
// notes.js owns the editor (`noteDraft`, the fields, Sign); this file owns getting its text into
// the node, and never losing what the node does not have.
//
// Classic script, loaded right BEFORE notes.js (whose input listeners name `onNoteInput` as it
// loads), sharing main.js's global scope (`el`, `invoke`) and notes.js's `noteDraft`,
// `noteStatus`, `closeEditor` and `readNotes`, funnel.js's `failureText` — all read only when a
// key is pressed or a save answers, long after every script has loaded. No browser storage, on
// purpose (notes.js's header): a draft lives in the node.
"use strict";

/** At most one save a second while the clinician types: often enough to lose little, rarely enough to be cheap. */
const NOTE_SAVE_EVERY_MS = 1000;
/** Said (with the reason) when a save failed. The next keystroke tries again. */
const NOT_SAVED = "not saved yet — your text is still here";
/** Said (with the reason) when the text saved but a new stated time was refused. */
const TIME_NOT_KEPT = "your text is saved, but not the time it happened: ";

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
    // Read after a press (Sign note or Discard draft) of this draft: the text that press carried.
    // A `spent` or `discarded` answer keeps only what differs from it (`sendDraft`).
    pressCarried: held.mayBeSigned ? held.pressedText : null,
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
 * while its draft is still the one on screen. Resolves whether nothing is left to send: saved; a
 * moved, timer-committed or signed-elsewhere draft whose unsaved text was kept as a new draft
 * (`keepUnsavedText`); or a spend or discard this window's own press may have made (the text that
 * press carried went with it; text typed after the press is kept as a new draft). A refusal that
 * wrote nothing — elsewhere, no chart, no such writer — resolves false and leaves the editor
 * UNSAVED: the text exists only on screen, and nothing may treat it as kept (PR #966 review).
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
      addendumTo: sent.held.addendumTo, // a save inside the debounce still makes an addendum's header
    });
    // The draft went on without this window — moved to another terminal, put into the record
    // unsigned by the quarantine timer (slice 3b-2a), or signed in another window (final review
    // I1) or discarded in another window (slice 3b-2b) — so what it lacks is kept as a new draft. A
    // spend or discard this window's own press (Sign or Discard) may have made forks nothing: that
    // text went with the press (`mayBeSigned`). A timer's commit is never this window's press, so
    // `committed` forks even after one.
    if (view.moved || view.committed || ((view.spent || view.discarded) && !sent.held.mayBeSigned)) return keepUnsavedText(sent, view.fork);
    // PR #984 review: this window's own press may have spent or discarded the draft — but it took
    // only the text it carried. An unknown outcome leaves the editor live, so a save read after the
    // press may carry text typed since: that is in no draft, and is kept as a new draft, said with
    // the press's own words. The press saved its text in its own transaction, so that is the line.
    const pressTook = (view.spent || view.discarded) && sent.held.mayBeSigned;
    if (pressTook && typeof sent.pressCarried === "string" && sent.narrative !== sent.pressCarried) {
      sent.held.savedText = sent.pressCarried;
      return keepUnsavedText(sent, view.pressed_fork);
    }
    const kept = view.saved || view.spent || view.discarded; // this window's own press took the text
    if (kept) sent.held.statedTime = asked;
    if (view.saved) sent.held.savedText = sent.narrative;
    if (mine()) {
      noteUnsaved = !kept;
      saySaved(view.saved ? "" : view.sentence);
    }
    return kept;
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
 * What of `text` the draft that went on lacks, and the sentence (Rust's `ForkWords`, from the
 * save's answer) that is true for it. `savedText` is what this window last saved for the part and
 * the node took — it went on with that draft. Typed after it: only the rest is new. Nothing after
 * it: nothing is kept (`kept: null`). The saved text itself edited: all of the text is kept, and
 * said so. Pure.
 */
function unsavedRemainder(text, savedText, forkWords) {
  if (!text.startsWith(savedText)) return { kept: text, said: forkWords.everything };
  const rest = text.slice(savedText.length);
  if (rest.trim() === "") return { kept: null, said: forkWords.nothing_new };
  return { kept: rest, said: forkWords.kept };
}

/**
 * Review Focus 2 and final review I1: a save found its draft moved to another terminal, put into
 * the record unsigned by the quarantine timer, or signed in another window. What the node lacks
 * (`unsavedRemainder`) is kept as a NEW draft of its own (a fresh id, the same writer), sent at
 * once in queue order, so a chart closing behind it still waits for it: nothing typed is lost,
 * and nothing saved is written twice. The editor's text when
 * it still holds the draft (the editor then shows only what the new draft holds), else what the
 * save carried. Once the editor's text has been kept, a save of the old draft still queued
 * carries older text than that, so it keeps nothing again (a second copy). With the editor gone
 * (the chart closed behind two queued saves), each later answer carries NEWER text than the one
 * before: it refills the one new draft the first answer made (`forkedTo`), never a second one
 * holding the same words again (PR #966 review).
 */
async function keepUnsavedText(sent, forkWords) {
  if (sent.held.keptOnScreen) return true;
  const onScreen = noteDraft === sent.held;
  if (onScreen) sent.held.keptOnScreen = true;
  const text = onScreen ? el("note-text").value : sent.narrative;
  const { kept, said } = unsavedRemainder(text, sent.held.savedText, forkWords);
  if (kept === null) {
    if (onScreen) {
      closeEditor();
      noteStatus(said);
      void readNotes(sent.held.patientId); // the note is listed where it went on
    }
    return true;
  }
  const fork = sent.held.forkedTo ||
    { ...sent.held, draftId: crypto.randomUUID(), savedText: "", mayBeSigned: false, keptOnScreen: false, forkedTo: null };
  sent.held.forkedTo = fork;
  if (onScreen) {
    noteDraft = fork;
    el("note-parts").replaceChildren(); // the parts above went on with the other draft
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
