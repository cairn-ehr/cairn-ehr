// The medication list's ONE Sign and "Continue here" (batch charting slice 1, design §3.3; split
// from med_list.js, which owns the lines, their saves and their drawing).
//
// One Sign charts every line DRAWN ticked when it was pressed: one event per line, under one key
// release, each line in its own transaction (med_list_sign.rs). Rust refuses the whole Sign for a
// changed picture only, before the key (`changed`, or an `ErrorView`), and reports each line it
// did not chart with its reason (`done`). Nothing here re-words a Rust sentence.
//
// Classic script, loaded right after med_list.js, sharing its state (`medList`, `medLines`,
// `medSigning`, `medUnticked`, `medLineNotes`, `medListRevision`, `medEditingLine`) and functions
// (`drawnTicked`, `pending`, `queueSave`, `lineFinished`, `drawMedList`, `drawMedRead`,
// `readMedDraft`), and med_list.js's globals from the other scripts — all read at a press.
"use strict";

/** "Continue here": the list moves to this terminal, and what the node answers is drawn. */
async function continueMedListHere() {
  const patient = renderedPatient;
  if (patient === null || medSigning) return;
  const revision = ++medListRevision;
  try {
    const fresh = await invoke("med_list_take", { patientId: patient });
    if (revision === medListRevision && patient === renderedPatient) drawMedRead(fresh, patient);
  } catch (failure) {
    setMessage(el("med-list-status"), failureText(failure));
  }
}

/**
 * The ONE Sign. The lines sent are those DRAWN ticked at the press; then every pending save
 * lands (a tick or a reason still in its second must be in the node, or Sign reads the line as
 * changed); a line that could not be saved signs nothing. Rust re-derives every tick and check.
 */
async function signMedList() {
  if (medSigning || medList === null || renderedPatient === null) return;
  const patient = renderedPatient;
  const charts = renderedCharts;
  const ticked = medLines.filter(drawnTicked).map((line) => line.id);
  if (medEditingLine) lineFinished(medEditingLine, false);
  medSigning = true;
  drawMedList();
  setMessage(el("med-list-status"), "");
  setMessage(el("med-list-outcome"), "");
  try {
    const saved = await Promise.all(medLines.filter(pending).map(queueSave));
    if (saved.includes(false)) return;
    let answer;
    try {
      answer = await invoke("med_list_sign", {
        patientId: patient,
        charts: charts,
        ticked: ticked,
        credential: signerCredential(),
      });
      clearSigner(true);
    } catch (failure) {
      clearSigner();
      await afterMedListRefused(failure, patient);
      return;
    }
    afterMedListSigned(answer, patient);
  } finally {
    medSigning = false;
    drawMedList();
  }
}

/** A Sign's answer. `changed`: nothing signed, those lines drawn unticked, the list read again
 *  (it carries none). `done`: the outcome, each line not charted with its reason, the table. */
function afterMedListSigned(answer, patient) {
  if (patient !== displayedPatient) return; // a chart left: its next read says the truth
  medLineNotes.clear();
  if (answer.kind === "changed") {
    for (const id of answer.lines) medUnticked.add(id);
    setMessage(el("med-list-status"), answer.message);
    void readMedDraft();
    return;
  }
  for (const [id, reason] of answer.not_charted) medLineNotes.set(id, reason);
  setMessage(el("med-list-outcome"), answer.outcome);
  drawSignedTable(answer, patient);
  void readMedDraft();
}

/** The medication table after a Sign, from the Sign's own answer (as main.js's `readMedList`
 *  draws one), under a fresh revision — or read again, marked behind, without it. */
function drawSignedTable(answer, patient) {
  say(answer.outcome);
  if (!answer.pane) {
    void readMedList(answer.outcome, "no");
    return;
  }
  const revision = ++chartReadRevision;
  drawnRevision = revision;
  const marked = listMarkedStale(); // before `render`, whose `drawList` takes the mark down
  render(answer.pane, patient);
  if (renderedPatient === patient) sayAfterDraw(answer.outcome, revision, marked, answer.pane);
}

/** A refused Sign charted nothing. Behind the record: the chart is read again. Otherwise the
 *  refusal is said under the list, and the list read again (it may have moved). */
async function afterMedListRefused(failure, patient) {
  if (patient !== displayedPatient) return;
  const text = failureText(failure);
  const behind = behindTheRecord(failure);
  if (behind) {
    await refresh(text, behind);
    return;
  }
  setMessage(el("med-list-status"), text);
  void readMedDraft();
}

el("med-list-sign").addEventListener("click", () => void signMedList());
el("med-list-take").addEventListener("click", () => void continueMedListHere());
