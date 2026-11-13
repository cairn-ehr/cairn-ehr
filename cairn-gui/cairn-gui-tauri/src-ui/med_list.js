// The medication LIST (batch charting slice 1, design §3.1–§3.3): the admission medication
// history's lines, written under each other, and ONE signature at the bottom. "Start medication"
// and "Already taking" add a line; each line has its tick box and its Remove; one Sign charts the
// ticked lines, one event each, under one key release (med_list_sign.js, with "Continue here").
//
// The list is DURABLE and PER WRITER (db/078): each line is saved as typed, a second after its
// last change, into the list of the person named in who's here when the list was read — and only
// that person ever sees or signs it. It is never committed by a timer: only Sign charts a line.
//
// What this file decides: nothing clinical. Every tick, warning, read-back and refusal is Rust's
// (med_list.rs, med_list_sign.rs), re-derived there at Sign on a fresh read. What it guards is the
// screen: a line is sent as ticked only if it was DRAWN ticked when Sign was pressed (`drawnTicked`),
// a line still being typed is never drawn ticked, a tick vouches only for the warnings drawn on
// its line (`seen`), and a who's-here switch takes the last person's list off the screen before
// anyone can type into it (`medListClaimChanged`).
//
// Classic script, loaded after med_chart.js (the one line editor, `MedLineEditor`, and its
// `splitLine`, `learnWords`, `isDirectionWord`, `drawReadBack`, `drawMarks`, `LINE_NOT_CHECKED`,
// `closeMedLine`, `medEditorSnapshot`, `focusPress`, `lineContent`), before med_list_sign.js and
// med_list_versions.js (`drawLineVersions`, `drawListVersions`, `signsAtSign`,
// `unmarkVersionsOf`), and before whos_here.js (whose `showClaim` calls `medListClaimChanged`).
// It shares main.js's `el`, `cell`, `setMessage`, `say`, `invoke`,
// `renderedPatient`, `renderedCharts`, `displayedPatient`, `chartReadRevision`, `drawnRevision`,
// `render`, `readMedList`, `refresh`; chart_stale.js's `listMarkedStale`, `sayAfterDraw`,
// `behindTheRecord`; signer.js's `signerCredential`, `clearSigner`, `fixtureMode`; funnel.js's
// `failureText`; notes_save.js's `NOT_SAVED`; person_menu.js's `pendingMenuFocus`,
// `takeMenuFocus`, `focusMenuTarget` — all read inside functions, after every script loaded.
"use strict";

/** A line saves this long after its last change (the note editor's one-second rhythm). */
const MED_LIST_SAVE_MS = 1000;
/** A line still being typed (Global Constraints' copy). */
const MED_LIST_UNFINISHED = "unfinished";
/** A line not drawn ticked: Sign leaves it in the list. */
const MED_LIST_UNTICKED = "not ticked: Sign leaves it unsigned";
const MED_LIST_ELSEWHERE = "These lines are open on another terminal: ";
/** A Remove the list refused (the door's word follows it). */
const MED_LIST_NOT_REMOVED = "This line was not removed";
const MED_PRESS_WORDS = { "started-here": "Start medication", "pre-existing": "Already taking" };
/** A line's check while the editor shows its own: nothing drawn twice. */
const NO_MARKS = { marks: [], notes: [], unverified: false };

/** The list drawn: `{ patient, writer, writerName, elsewhere }`, or null while none is read. */
let medList = null;
/** The lines, in list order (`lineFrom` names their fields). */
let medLines = [];
/** The line the editor is mounted on, or null. */
let medEditingLine = null;
/** Lines a Sign found changed: drawn unticked until the clinician ticks them again. (An untick
 *  by hand is stored, as `held`: it survives a closed chart, a re-lock and a restart.) */
const medUnticked = new Set();
/** A Sign's reason for each line it did not chart, by line id, drawn on the line. */
const medLineNotes = new Map();
/** Lines removed whose delete has not answered: a read in flight does not draw them back. */
const medRemoving = new Set();
let medListRevision = 0; // a read older than the newest started is dropped
/** Saves and deletes go out one after another; it never rejects (`sendLine` catches all). */
let medSaveQueue = Promise.resolve();
let medSigning = false;

/** A list line from Rust's `ListLineView` `v`, in the list `fresh` read for `patient`. The writer
 *  is the list's as READ, and never changes: every save and delete sends it back. */
function lineFrom(v, fresh, patient) {
  return {
    id: v.line_id,
    patient: patient,
    writer: fresh.writer,
    charts: renderedCharts,
    view: v,
    content: contentOf(v),
    reason: v.reason || "",
    handTicked: v.hand_ticked,
    held: v.held, // the clinician's untick of a default tick, stored (review C1)
    finished: v.finished,
    seen: [], // the warnings drawn on the line when it was ticked by hand
    snapshot: null, // the editor's words not yet split and saved (`medEditorSnapshot`)
    timer: null,
    version: 0, // bumped by every local change; `savedVersion` is the newest the node took
    savedVersion: 0,
    contentVersion: 0, // the version of the last change to the words
    tickPending: false,
    stored: true,
    removed: false,
    row: null,
  };
}

/** The saved words of a line view, as `med_list_save` takes them, and the line's own source, why
 *  and set-aside mark (slice 2a): every save sends all four back, or the door clears them. */
function contentOf(v) {
  return { charted_as: v.charted_as, line_text: v.line_text, typed_drug: v.typed_drug, directions: v.directions, parts: v.parts, info_source: v.info_source, source_detail: v.source_detail, why: v.why, set_aside_of: v.set_aside_of };
}

/** A new, empty line pressed as `chartedAs`, in the list on screen. */
function newLine(chartedAs) {
  const blank = { line_id: crypto.randomUUID(), charted_as: chartedAs, line_text: "", typed_drug: "", directions: "", parts: [], reason: null, hand_ticked: false, held: false, finished: false };
  const line = lineFrom(blank, medList, medList.patient);
  line.view = null;
  line.stored = false;
  return line;
}

/** Something on the line has not reached the node yet. */
function pending(line) {
  return line.version !== line.savedVersion;
}

/**
 * Whether the line is DRAWN ticked — what Sign sends. Never while it is being typed, never one
 * unticked here or found changed by a Sign; the clinician's own tick while its save is on its way;
 * otherwise Rust's `ticked` for the words it read.
 */
function drawnTicked(line) {
  if (line.snapshot !== null || !line.finished || medUnticked.has(line.id)) return false;
  if (line.tickPending) return line.handTicked;
  return Boolean(line.view) && line.view.ticked && !line.view.unfinished;
}

/** A Start line with an allergy mark needs a reason (design §3.5); Already taking never does. */
function needsReasonOn(line) {
  const marked = Boolean(line.view) && line.view.check.reason_needed;
  return line.content.charted_as === "started-here" && (marked || line.reason !== "");
}

function waitsForReason(line) {
  return needsReasonOn(line) && line.reason.trim() === "";
}

/** The drug as Rust read it: each part's pick, or its words. */
function drugWords(parts) {
  return parts.map((part) => (part.coding ? part.coding.display : part.term)).join(" + ");
}

/** Over every drawn chart (main.js's `render`): the list is read again, so it is re-checked. */
function syncMedList() {
  if (renderedPatient !== null && !fixtureMode) void readMedDraft();
}

/** The chart changed (main.js's `clearChart`): its list leaves, its pending saves queued. */
function clearMedList() {
  putAwayMedList();
}

/** Every save and delete queued so far, done (funnel.js's `closeChart` waits for it). */
function medListSaved() {
  return medSaveQueue;
}

/**
 * Take the list off the screen: the editor closed, every pending save queued under the writer
 * and chart it was typed for, every read in flight outdated. Returns the queued saves.
 */
function putAwayMedList() {
  medEditingLine = null;
  closeMedLine();
  const flushed = flushMedLines();
  medList = null;
  medLines = [];
  medUnticked.clear();
  medLineNotes.clear();
  medListRevision += 1;
  el("med-list").replaceChildren();
  setMessage(el("med-list-status"), "");
  setMessage(el("med-list-outcome"), "");
  drawMedList();
  return flushed;
}

/**
 * Who's here names someone else (whos_here.js's `showClaim`). The last person's list leaves the
 * screen AT ONCE, so nothing typed now can land in it; what they typed is saved as theirs; then
 * the new person's list is read. Until it is, nothing can be added.
 */
function medListClaimChanged() {
  if (renderedPatient === null) return;
  const flushed = putAwayMedList();
  void flushed.then(() => readMedDraft());
}

/** Queue a save of every line with a change the node has not taken. */
function flushMedLines() {
  return Promise.all(medLines.filter(pending).map(queueSave));
}

/** Read the list for the chart on screen (the writer is who's here's, in Rust). */
async function readMedDraft() {
  const patient = renderedPatient;
  if (patient === null || fixtureMode) return;
  const revision = ++medListRevision;
  let fresh;
  try {
    fresh = await invoke("med_list_read", { patientId: patient, charts: renderedCharts });
  } catch (failure) {
    if (revision === medListRevision && patient === renderedPatient) setMessage(el("med-list-status"), failureText(failure));
    return;
  }
  if (revision !== medListRevision || patient !== renderedPatient) return;
  drawMedRead(fresh, patient);
}

/**
 * Draw a read (`MedListView`): each line merged into the one on screen (a line typed but not yet
 * saved keeps its words), a line the node no longer holds dropped, a new one added. Rows are
 * updated IN PLACE, so the line being typed keeps its focus.
 */
function drawMedRead(fresh, patient) {
  medList = { patient: patient, writer: fresh.writer, writerName: fresh.writer_name, elsewhere: fresh.elsewhere, from: fresh.from, versionsRecorded: fresh.versions_recorded };
  const byId = new Map(medLines.map((line) => [line.id, line]));
  const next = fresh.lines.filter((v) => !medRemoving.has(v.line_id)).map((v) => mergeLine(byId.get(v.line_id), v, fresh, patient));
  for (const line of medLines) if (!line.stored && !next.includes(line)) next.push(line);
  for (const line of medLines) if (!next.includes(line)) dropLine(line);
  medLines = next;
  drawMedList();
  if (pendingMenuFocus === "meds:") focusMenuTarget(takeMenuFocus()); // person_menu.js, now the pane shows
}

/** Merge a read line `v` into `line` (or make it). A verdict is about the words it read: it is
 *  never drawn over words the node has not taken; local changes not yet saved are kept. */
function mergeLine(line, v, fresh, patient) {
  if (!line) return lineFrom(v, fresh, patient);
  line.stored = true;
  line.charts = renderedCharts;
  if (line.snapshot === null && line.savedVersion >= line.contentVersion) line.view = v;
  if (!pending(line) && medEditingLine !== line) {
    line.content = contentOf(v);
    line.reason = v.reason || "";
    line.handTicked = v.hand_ticked;
    line.held = v.held;
    line.finished = v.finished;
    line.tickPending = false;
  }
  takeOwn(line, v); // its own source, why and mark, even while typed (med_list_versions.js, R18)
  return line;
}

/** A line leaves the screen (removed, signed, or gone from the node). */
function dropLine(line) {
  if (medEditingLine === line) {
    medEditingLine = null;
    closeMedLine();
  }
  clearTimeout(line.timer);
  medLines = medLines.filter((other) => other !== line);
  if (line.row) line.row.li.remove();
}

/** The pane: each row in place, whose list it is, "Continue here", Sign, and the presses. */
function drawMedList() {
  const list = el("med-list");
  for (const line of medLines) {
    if (!line.row) line.row = medLineRow(line);
    if (!line.row.li.isConnected) list.append(line.row.li);
    drawLine(line);
  }
  const any = medLines.length > 0 && medList !== null;
  const whose = any ? "Unsigned lines typed by " + (medList.writerName || "nobody named in who's here") : "";
  setMessage(el("med-list-writer"), whose);
  el("med-list-elsewhere").hidden = !(medList && medList.elsewhere);
  el("med-list-elsewhere-words").textContent = medList && medList.elsewhere ? MED_LIST_ELSEWHERE + medList.elsewhere : "";
  const sign = el("med-list-sign");
  sign.hidden = !any;
  sign.disabled = medSigning || !any || medList.elsewhere !== null || !medLines.some(signsAtSign);
  drawListVersions(); // "From:", and what Sign records beside the ticks (med_list_versions.js)
  for (const button of el("med-chart-buttons").querySelectorAll("button")) {
    button.disabled = medSigning || medList === null || medList.elsewhere !== null;
  }
}

/** One row's elements, made once (`drawLine` fills them): never re-made, so focus stays put. */
function medLineRow(line) {
  const li = document.createElement("li");
  li.dataset.line = line.id;
  const tick = cell("input", "", { type: "checkbox" });
  const text = cell("button", "", { type: "button", class: "med-list-text" });
  const remove = cell("button", "Remove", { type: "button" });
  const label = cell("label", "Reason for charting despite the listed allergy");
  const reason = cell("input", "", { type: "text", autocomplete: "off" });
  label.append(reason);
  const r = {
    li, tick, text, remove, reason,
    kind: cell("span", ""),
    slot: cell("div", ""),
    readback: cell("p", ""),
    tag: cell("span", ""),
    marks: cell("ul", ""),
    reasonRow: cell("p", ""),
    state: cell("p", "", { class: "med-list-state" }),
    note: cell("p", "", { class: "allergy-field-error" }),
  };
  r.reasonRow.append(label);
  li.append(tick, r.kind, text, r.slot, remove, r.readback, r.tag, r.marks, r.reasonRow, r.state, r.note);
  tick.addEventListener("change", () => tickMedLine(line, tick.checked));
  text.addEventListener("click", () => editMedLine(line));
  remove.addEventListener("click", () => removeMedLine(line));
  reason.addEventListener("input", () => reasonTyped(line, reason.value));
  return r;
}

/** Fill one row from the line: its tick, words, Rust's read-back and check, reason and state. */
function drawLine(line) {
  const r = line.row;
  if (!r) return;
  const v = line.view;
  const editing = medEditingLine === line;
  const text = line.snapshot ? line.snapshot.text : line.content.line_text;
  const blocked = medSigning || medList === null || medList.elsewhere !== null;
  r.tick.checked = drawnTicked(line);
  // No tick until the node has read the line's words (`v`): a tick vouches for the warnings drawn
  // on it, and before that read none are drawn, so it would vouch for nothing (review M4).
  r.tick.disabled = blocked || !v || (waitsForReason(line) && !r.tick.checked);
  r.tick.setAttribute("aria-label", "Tick to sign: " + text);
  r.kind.textContent = MED_PRESS_WORDS[line.content.charted_as] + ": ";
  r.text.textContent = text;
  r.text.hidden = editing;
  r.text.disabled = blocked;
  r.slot.hidden = !editing;
  r.remove.disabled = blocked;
  r.remove.setAttribute("aria-label", "Remove: " + text);
  // Rust's reading of the SAVED words; while the line is typed, the editor shows its own.
  if (v && !editing) drawReadBack(drugWords(v.parts), v.read, r.readback);
  else setMessage(r.readback, "");
  drawMarks(editing ? NO_MARKS : v ? v.check : LINE_NOT_CHECKED, r.tag, r.marks);
  r.reasonRow.hidden = !needsReasonOn(line);
  if (document.activeElement !== r.reason) r.reason.value = line.reason;
  const words = [];
  if (!line.finished) words.push(MED_LIST_UNFINISHED);
  if (!r.tick.checked) words.push(MED_LIST_UNTICKED);
  if (v && v.duplicate_of) words.push(v.duplicate_of); // Rust's sentence naming the other line
  setMessage(r.state, words.join(" · "));
  setMessage(r.note, medLineNotes.get(line.id) || "");
  drawDelabelOffers(line); // med_list_delabel.js
  drawLineVersions(line); // its source, Why, Set aside (med_list_versions.js)
}

/** A press: a new line at the end of the list, the editor in it. */
function appendMedLine(chartedAs) {
  if (medList === null || medSigning || medList.elsewhere !== null) return;
  const line = newLine(chartedAs);
  medLines.push(line);
  drawMedList();
  editMedLine(line);
}

/** Mount the one editor on `line` (the line typed before it is finished first). */
function editMedLine(line) {
  if (medSigning || medList === null || medList.elsewhere !== null) return;
  if (medEditingLine && medEditingLine !== line) lineFinished(medEditingLine, false);
  medEditingLine = line;
  drawLine(line);
  const words = line.snapshot
    ? { charted_as: line.content.charted_as, line_text: line.snapshot.text, parts: line.snapshot.held.map((held) => ({ term: held.text, coding: held.coding })) }
    : line.content;
  MedLineEditor(line.row.slot, words, { changed: () => lineTyped(line), finished: (byKey) => lineFinished(line, byKey) });
}

/** The editor's words changed: the tick covered the old words (the door clears it too), the line
 *  is being typed again, and its verdict is taken down until the node reads the new words. */
function lineTyped(line) {
  line.snapshot = medEditorSnapshot();
  line.finished = false;
  line.handTicked = false;
  line.held = false; // the untick covered the old words (the door clears it too)
  line.tickPending = false;
  line.view = null;
  medUnticked.delete(line.id);
  medLineNotes.delete(line.id);
  line.version += 1;
  line.contentVersion = line.version;
  touch(line);
}

/** The reason field changed: saved as typed; the box enables once it holds text. */
function reasonTyped(line, text) {
  line.reason = text;
  line.version += 1;
  touch(line);
}

/** A change: the save waits a second after the last one, and the line is drawn again. */
function touch(line) {
  clearTimeout(line.timer);
  line.timer = setTimeout(() => void queueSave(line), MED_LIST_SAVE_MS);
  drawLine(line);
  drawMedList();
}

/** Enter, Escape or focus leaving the line: the editor closes, and the line, finished, saves now.
 *  By a key, focus goes to the press that started it, so Enter adds the next line. */
function lineFinished(line, byKey) {
  if (medEditingLine === line) {
    medEditingLine = null;
    closeMedLine();
  }
  if (!line.stored && line.version === 0) {
    dropLine(line); // pressed and left without a word typed: nothing to keep
    drawMedList();
    if (byKey) focusPress(line);
    return;
  }
  if (!line.finished) {
    line.finished = true;
    line.version += 1;
  }
  void queueSave(line);
  drawLine(line);
  drawMedList();
  if (byKey) focusPress(line);
}

/** The box, by hand: saved at once, vouching for the warnings drawn on the line NOW. An untick
 *  is stored as the clinician's veto (`held`), so a default tick never comes back by itself. */
function tickMedLine(line, checked) {
  if (medSigning) return;
  if (line.snapshot !== null) void queueSave(line); // the words first, untouched by the tick
  line.seen = line.view ? line.view.against : [];
  line.handTicked = checked;
  line.held = !checked;
  line.finished = true;
  line.tickPending = true;
  medUnticked.delete(line.id);
  medLineNotes.delete(line.id);
  line.version += 1;
  void queueSave(line);
  drawLine(line);
  drawMedList();
}

/** Queue a save of the line as it is NOW (the fields are read here, in order with the others). */
function queueSave(line) {
  clearTimeout(line.timer);
  line.timer = null;
  const sent = { version: line.version, handTicked: line.handTicked, held: line.held, finished: line.finished, reason: line.reason, seen: line.seen };
  medSaveQueue = medSaveQueue.then(() => sendLine(line, sent));
  return medSaveQueue;
}

/** One save. Resolves true when the node holds the line as sent (or nothing was new). */
async function sendLine(line, sent) {
  if (line.removed || sent.version <= line.savedVersion) return true;
  const snap = line.snapshot;
  const words = snap ? await lineContent(line.content.charted_as, snap) : null;
  const content = words ? { ...line.content, ...words } : line.content; // read after the await (review C)
  if (snap && line.snapshot === snap) {
    line.content = content;
    line.snapshot = null;
  }
  try {
    const door = await invoke("med_list_save", {
      patientId: line.patient,
      charts: line.charts,
      writer: line.writer,
      line: { line_id: line.id, ...content, reason: sent.reason.trim() ? sent.reason : null, hand_ticked: sent.handTicked, held: sent.held, finished: sent.finished, seen: sent.handTicked ? sent.seen : [] },
    });
    if (door.answer === "saved") {
      line.stored = true;
      line.savedVersion = Math.max(line.savedVersion, sent.version);
    } else {
      setMessage(el("med-list-status"), NOT_SAVED + " (the list answered " + door.answer + ")");
    }
    if (line.patient === renderedPatient) void readMedDraft(); // the node's verdict on what it took
    return door.answer === "saved";
  } catch (failure) {
    setMessage(el("med-list-status"), NOT_SAVED + " (" + failureText(failure) + ")");
    return false;
  }
}

/** Remove, at once and asking nothing (a line crossed out): no record is kept (design §9 Q3). */
function removeMedLine(line) {
  if (medSigning) return;
  clearTimeout(line.timer);
  line.removed = true;
  medRemoving.add(line.id);
  dropLine(line);
  unmarkVersionsOf(line); // its versions, ordinary held lines again (med_list_versions.js)
  drawMedList();
  medSaveQueue = medSaveQueue.then(() => deleteLine(line));
}

async function deleteLine(line) {
  try {
    const door = await invoke("med_list_delete", { patientId: line.patient, lineId: line.id, writer: line.writer });
    // Moved, changed, not yours: the re-read below draws it back, so say why (as a refused save).
    if (door.answer !== "deleted" && door.answer !== "gone") {
      setMessage(el("med-list-status"), MED_LIST_NOT_REMOVED + " (the list answered " + door.answer + ")");
    }
  } catch (failure) {
    setMessage(el("med-list-status"), failureText(failure));
  }
  medRemoving.delete(line.id);
  if (line.patient === renderedPatient) void readMedDraft(); // a line the node kept comes back
  return true;
}
