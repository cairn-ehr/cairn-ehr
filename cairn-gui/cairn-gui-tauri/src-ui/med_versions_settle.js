// Batch charting slice 2b on the charted list (design §3.3, §3.4): settling a set-aside version
// from under its line. Each version of the OPENED chart offers two buttons — "Use this version
// instead" (a restore line, pre-filled, opened in the list's editor; the list's Sign charts it and
// stops what stands) and "Checked: not current" (an inline row, signed on its own Sign). Under a
// line a restore charted, what it replaced (`replacedRows`, called by main.js's `renderRow`).
//
// What this file decides: nothing clinical. Whether a version may be settled from here
// (`can_settle`: its own chart is the opened one, and it is not restored), whether it is still to
// check (`open`), and every settle line's words are Rust's (chart_versions.rs, row_view.rs); the
// restore and the check are re-derived in Rust on a fresh read of the OPENED chart before anything
// is written (med_list_restore.rs, med_version_check.rs). What it guards is the screen: a button
// is down while a Sign or a list act is in flight (`medSigning`) — the restore also while the list
// is another terminal's or not read — the check row signs nothing until its own Sign, and a
// refusal is said beside the version it is about, never only in a status bar. No dialog.
//
// The table is drawn again on every chart read (main.js's `render`), so what must outlive a
// redraw lives here, not in the DOM: the open check row and its typed words (`settleCheck`), each
// version's last refusal (`settleSaid`), and which lines' versions are open (`versionsOpen`). A
// chart change clears all three (`clearSettle`, from main.js's `clearChart`).
//
// Classic script, loaded after med_list_versions.js (`presetOptions`, `FROM_LABEL`, `WHY_LABEL`)
// and med_list_sign.js (`drawSignedTable`, `medListSigning`, `medListSignInFlight`), sharing
// med_list.js's `medList`, `medLines`, `medSigning`, `medListRevision`, `editMedLine`,
// `drawMedRead`, `drawMedList`; main.js's `el`, `cell`, `setMessage`, `invoke`, `renderedPatient`,
// `renderedCharts`, `displayedPatient`, `refresh`; signer.js's `signerCredential`, `clearSigner`;
// notes.js's `signInFlight`; chart_stale.js's `behindTheRecord`; funnel.js's `failureText` — all
// read inside functions, after every script loaded.
"use strict";

const RESTORE = "Use this version instead";
const CHECK = "Checked: not current";
const CHECK_SIGN = "Sign";
const CHECK_CANCEL = "Cancel";
/** A fault in this window mid-check: what went in is the pane's next read. */
const CHECK_FAULT = "Whether the version was recorded as checked is not known — read the chart again.";

/** The one open check row: `{ versionId, preset, detail, why }` as typed, or null. */
let settleCheck = null;
/** The last refusal said beside each version, by version id. */
const settleSaid = new Map();
/** The lines (group ids) whose versions the clinician opened: kept open across a redraw. */
const versionsOpen = new Set();

/** The chart changed: nothing typed or said about one chart's versions follows to the next. */
function clearSettle() {
  settleCheck = null;
  settleSaid.clear();
  versionsOpen.clear();
}

/** Whether `act` ("restore", "check", "sign") is down now: a Sign or a list act is in flight;
 *  a restore adds a line to the list, so it also waits for a list read here and on this terminal. */
function settleBlocked(act) {
  if (medSigning) return true;
  return act === "restore" && (medList === null || medList.elsewhere !== null);
}

/** Every version button, re-decided (drawListVersions calls this whenever the list's are). */
function syncSettleButtons() {
  for (const button of document.querySelectorAll("#med-rows [data-settle]")) {
    button.disabled = settleBlocked(button.dataset.settle);
  }
}

/** The versions' `<details>` of line `groupId`: open if the clinician left it open. */
function keepOpen(box, groupId) {
  box.open = versionsOpen.has(groupId);
  box.addEventListener("toggle", () => (box.open ? versionsOpen.add(groupId) : versionsOpen.delete(groupId)));
}

/** A button on version `version`, named for it, down while `act` is (`settleBlocked`). */
function settleButton(label, act, version, press) {
  const button = cell("button", label, {
    type: "button",
    class: "med-version-act",
    "data-settle": act,
    "aria-label": label + ": " + version.line,
  });
  button.disabled = settleBlocked(act);
  button.addEventListener("click", press);
  return button;
}

/** What goes in a version's `li` after its words (med_list_versions.js's `versionItem`): the two
 *  buttons where Rust offers them, the refusal last said about it, and its check row if open. */
function settleControls(version, groupId) {
  const said = cell("p", settleSaid.get(version.version_id) || "", { class: "med-version-said allergy-field-error", "data-said": version.version_id });
  said.hidden = !settleSaid.has(version.version_id);
  if (!version.can_settle) return [said];
  const restore = settleButton(RESTORE, "restore", version, () => void pressRestore(version, groupId));
  const check = settleButton(CHECK, "check", version, (event) => openCheck(version, groupId, event.target));
  const out = [restore, check, said];
  if (settleCheck && settleCheck.versionId === version.version_id) out.push(checkRow(version, groupId));
  return out;
}

/** Say `text` beside version `versionId`, now and on every redraw until its next press. */
function sayBeside(versionId, text) {
  settleSaid.set(versionId, text);
  for (const said of document.querySelectorAll(".med-version-said")) {
    if (said.dataset.said === versionId) setMessage(said, text);
  }
}

/** A press on a version: its last refusal comes down, and its line's versions stay open. */
function pressed(version, groupId) {
  sayBeside(version.version_id, "");
  settleSaid.delete(version.version_id);
  versionsOpen.add(groupId);
}

/**
 * "Use this version instead": Rust adds a restore line pre-filled from the version to who's
 * here's list (`med_list_restore_open`); its answer, the list read again, is drawn, and the ONE
 * editor opens on the new line so the reviewer can change what has changed since. Nothing is
 * signed here: the list's Sign stops what stands and charts the line, in one transaction. An
 * add whose list could not be read again answers no list and words (`said`) to show instead.
 */
async function pressRestore(version, groupId) {
  if (settleBlocked("restore")) return;
  pressed(version, groupId);
  const patient = renderedPatient;
  const revision = ++medListRevision;
  medSigning = true; // a list act in flight: every list and version control is down
  drawMedList();
  let opened;
  try {
    opened = await invoke("med_list_restore_open", {
      patientId: patient,
      charts: renderedCharts,
      versionId: version.version_id,
    });
  } catch (failure) {
    if (patient === renderedPatient) sayBeside(version.version_id, failureText(failure));
    return;
  } finally {
    medSigning = false; // before the editor, which refuses while a list act is in flight
    drawMedList();
  }
  if (revision !== medListRevision || patient !== renderedPatient) return;
  // Added, but the list could not be read again: say so, never a refusal (a second press would
  // add a second line).
  if (!opened.list) return sayBeside(version.version_id, opened.said);
  drawMedRead(opened.list, patient);
  const line = medLines.find((l) => l.id === opened.line_id);
  if (line) editMedLine(line);
}

/** "Checked: not current": its inline row opens under the version (one open at a time); it signs
 *  nothing until its own Sign. */
function openCheck(version, groupId, opener) {
  if (settleBlocked("check")) return;
  pressed(version, groupId);
  for (const open of document.querySelectorAll(".med-version-check")) open.remove();
  settleCheck = { versionId: version.version_id, preset: "", detail: "", why: "" };
  const form = checkRow(version, groupId);
  opener.closest("li").append(form);
  form.querySelector("select").focus();
}

/** Close the check row, signing nothing; focus back on the version's Check button. */
function closeCheck(form) {
  const back = form.closest("li") && form.closest("li").querySelector('[data-settle="check"]');
  settleCheck = null;
  form.remove();
  if (back) back.focus();
}

/** The check row from `settleCheck`'s words: From: (2a's presets, mapped in Rust as the list's
 *  "From:" is), the document or person, Why, its Sign and Cancel. Every keystroke is kept in
 *  `settleCheck`, so a redraw of the pane rebuilds the row as typed. */
function checkRow(version, groupId) {
  const words = settleCheck;
  const form = cell("div", "", { class: "med-version-check", role: "group", "aria-label": CHECK + ": " + version.line });
  const preset = cell("select", "", { "aria-label": "From, for the check of " + version.line });
  presetOptions(preset);
  preset.value = words.preset;
  const detail = cell("input", "", { type: "text", autocomplete: "off", "aria-label": "The document or person, for the check of " + version.line });
  detail.value = words.detail;
  const why = cell("input", "", { type: "text", autocomplete: "off" });
  why.value = words.why;
  const keep = () => Object.assign(words, { preset: preset.value, detail: detail.value, why: why.value });
  for (const field of [preset, detail, why]) {
    field.addEventListener("input", keep);
    field.addEventListener("change", keep);
  }
  const fromLabel = cell("label", FROM_LABEL + " ");
  fromLabel.append(preset);
  const whyLabel = cell("label", WHY_LABEL + " ");
  whyLabel.append(why);
  const sign = settleButton(CHECK_SIGN, "sign", version, () => pressCheckSign(version, groupId));
  const cancel = cell("button", CHECK_CANCEL, { type: "button" });
  cancel.addEventListener("click", () => {
    if (!medSigning) closeCheck(form);
  });
  form.append(fromLabel, detail, whyLabel, sign, cancel);
  return form;
}

/** The check row's Sign: one press, one act; a who's-here switch and a re-lock wait for it, as for
 *  the list's Sign (notes.js's `signsSettled`). */
function pressCheckSign(version, groupId) {
  if (signInFlight() || settleBlocked("sign")) return;
  medListSignInFlight = true;
  medListSigning = signCheck(version, groupId)
    .catch((failure) => {
      console.error(failure);
      return { words: CHECK_FAULT };
    })
    .finally(() => {
      medListSignInFlight = false;
    });
}

/**
 * Sign the check with the row's words and the strip's credential, on the chart drawn, over the
 * set it was read in. Signed: the row closes and the pane Rust read again is drawn (its version
 * now reads "checked by …"). Refused: the words stay in the row and the refusal is said beside the
 * version; a list behind the record is read again. Resolves `{ words }` for `signsSettled`.
 */
async function signCheck(version, groupId) {
  const patient = renderedPatient;
  const charts = renderedCharts;
  const words = settleCheck;
  pressed(version, groupId);
  medSigning = true;
  drawMedList();
  try {
    let answer;
    try {
      answer = await invoke("med_version_check", {
        patientId: patient,
        charts: charts,
        versionId: version.version_id,
        preset: words.preset || null,
        detail: words.detail.trim() || null,
        why: words.why.trim() || null,
        credential: signerCredential(),
      });
      clearSigner(true);
    } catch (failure) {
      clearSigner();
      if (patient !== displayedPatient) return { words: failureText(failure) };
      sayBeside(version.version_id, failureText(failure));
      const behind = behindTheRecord(failure);
      if (behind) await refresh(failureText(failure), behind);
      return { words: failureText(failure) };
    }
    if (settleCheck === words) settleCheck = null;
    if (patient === displayedPatient) drawSignedTable(answer, patient);
    return { words: answer.outcome };
  } finally {
    medSigning = false;
    drawMedList();
  }
}

/** Under a line a restore charted: what it replaced, in Rust's words, one row each. */
function replacedRows(row) {
  return row.replaced.map((words) => {
    const tr = document.createElement("tr");
    tr.setAttribute("data-replaced", "true");
    const td = cell("td", words);
    td.colSpan = 5;
    tr.append(td);
    return tr;
  });
}
