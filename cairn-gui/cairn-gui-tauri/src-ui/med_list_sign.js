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

/** The last list Sign, done, as `{ words }` (its outcome as said under the list) or null; and
 *  whether one is in flight. notes.js's `signsSettled` / `signInFlight` include it, so a who's-here
 *  switch, a re-lock and the person menu wait for it, and the switch says its words in the picker
 *  (review I1). */
let medListSigning = Promise.resolve(null);
let medListSignInFlight = false;
/** A fault in this window mid-Sign: what went in is the list's next read. */
const MED_LIST_SIGN_FAULT = "Whether the ticked lines were charted is not known — open the chart and look at the list.";

/**
 * The ONE Sign. The lines sent are those DRAWN ticked at the press; then every pending save
 * lands (a tick or a reason still in its second must be in the node, or Sign reads the line as
 * changed); a line that could not be saved signs nothing. Rust re-derives every tick and check.
 */
async function signMedList() {
  if (medSigning || medList === null || renderedPatient === null) return null;
  const patient = renderedPatient;
  const charts = renderedCharts;
  const writer = medList.writer; // whose list this Sign answers for
  const ticked = medLines.filter(drawnTicked).map((line) => line.id);
  if (medEditingLine) lineFinished(medEditingLine, false);
  medSigning = true;
  drawMedList();
  setMessage(el("med-list-status"), "");
  setMessage(el("med-list-outcome"), "");
  try {
    const saved = await Promise.all(medLines.filter(pending).map(queueSave));
    if (saved.includes(false)) {
      clearSigner(); // as every other way out: no passphrase is left in the strip
      return { words: el("med-list-status").textContent }; // the line that could not be saved
    }
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
      await afterMedListRefused(failure, patient, writer);
      return { words: failureText(failure) };
    }
    afterMedListSigned(answer, patient, writer);
    return { words: answer.kind === "changed" ? answer.message : answer.outcome };
  } finally {
    medSigning = false;
    drawMedList();
  }
}

/** A Sign's answer. `changed`: nothing signed, those lines drawn unticked, the list read again
 *  (it carries none). `done`: the outcome, each line not charted with its reason, the table. */
function afterMedListSigned(answer, patient, writer) {
  if (patient !== displayedPatient) return; // a chart left: its next read says the truth
  if (answer.kind === "done") drawSignedTable(answer, patient); // the chart's record, whoever is here
  if (!ownsTheList(writer)) return; // another person's list is on screen: never drawn under it
  medLineNotes.clear();
  if (answer.kind === "changed") {
    for (const id of answer.lines) medUnticked.add(id);
    setMessage(el("med-list-status"), answer.message);
    void readMedDraft();
    return;
  }
  for (const [id, reason] of answer.not_charted) medLineNotes.set(id, reason);
  setMessage(el("med-list-outcome"), answer.outcome);
  void readMedDraft();
}

/** The list on screen is still the one `writer`'s Sign answered for (a who's-here switch takes it
 *  down and reads the next person's). */
function ownsTheList(writer) {
  return medList !== null && medList.writer === writer;
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
async function afterMedListRefused(failure, patient, writer) {
  if (patient !== displayedPatient) return;
  const text = failureText(failure);
  const behind = behindTheRecord(failure);
  if (behind) {
    await refresh(text, behind);
    return;
  }
  if (!ownsTheList(writer)) return;
  setMessage(el("med-list-status"), text);
  void readMedDraft();
}

// As a note's Sign: one press, one act; a note's press in flight holds the strip, so this waits.
el("med-list-sign").addEventListener("click", () => {
  if (signInFlight()) return;
  medListSignInFlight = true;
  medListSigning = signMedList()
    .catch((failure) => {
      console.error(failure);
      return { words: MED_LIST_SIGN_FAULT };
    })
    .finally(() => {
      medListSignInFlight = false;
    });
});
el("med-list-take").addEventListener("click", () => void continueMedListHere());
