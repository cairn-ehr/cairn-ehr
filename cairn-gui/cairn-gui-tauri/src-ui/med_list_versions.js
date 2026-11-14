// Batch charting slice 2a on the medication list (design §3.1, §3.5): "From:" above the list;
// each line's own source and "Why" (closed until opened); "Set aside", with its "Version of
// which line?" pick, and the set-aside line itself; and, on the charted list, each line's
// set-aside versions collapsed under it (`versionRows`, called by main.js's `renderRow`), each
// with how it was settled and — slice 2b — its two buttons (med_versions_settle.js's
// `settleControls`); and a restore line's mark (`restores_words`).
//
// What this file decides: nothing clinical. Which lines a line may be set aside beside
// (`companions`), how the pick names them (`companion_words`), what "From:" stamps on a new line,
// and every collapsed version's words are Rust's (med_list.rs, med_list_versions.rs,
// chart_versions.rs); at Sign each version's companion is re-derived from the STORED mark
// (ruling R9). What it guards is the screen and the round trip: the line's four own fields
// (`info_source`, `source_detail`, `why`, `set_aside_of`) ride in `line.content` (med_list.js's
// `contentOf`), so every save sends all four back — the door clears one left out — and a change
// to one is drawn as the door counts it: the hand tick gone, the untick kept (`setOwn`); and a
// read brings the line's own fields back even while it is typed (`takeOwn`, ruling R18).
//
// Classic script, loaded after med_list.js and med_list_sign.js, sharing med_list.js's state
// (`medList`, `medLines`, `medSigning`, `medEditingLine`, `medLineNotes`, `medListRevision`) and
// functions (`drawnTicked`, `flushMedLines`, `touch`, `queueSave`, `drawLine`, `drawMedRead`),
// med_list_sign.js's `ownsTheList`,
// main.js's `el`, `cell`, `setMessage`, `invoke`, `renderedPatient`, `renderedCharts`, and
// funnel.js's `failureText` — all read inside functions, after every script loaded.
"use strict";

const FROM_LABEL = "From:";
const WHY_LABEL = "Why";
const SET_ASIDE = "Set aside";
const SET_ASIDE_MARK = "set aside — not current";
/** The pick, shown only with two or more candidate lines. */
const WHICH_LINE = "Version of which line?";
/** Under a charted line: "{n} other version(s)". */
const OTHER_VERSIONS = " other version(s)";
/** Then, while Rust counts any version still open (slice 2b): ", {u} to check". */
const TO_CHECK = [", ", " to check"];
/** Ruling R14, near Sign: "Sign also records {count} set-aside version(s), as not current". */
const SIGN_ALSO = ["Sign also records ", " set-aside version(s), as not current"];
/** The four fields a line holds of its own, sent back on every save (med_list.js's `contentOf`). */
const OWN_FIELDS = ["info_source", "source_detail", "why", "set_aside_of"];
/** No preset chosen: the press's default stands (list), or the stored category (a line). */
const NO_PRESET = "not set";
/** "From:" was not changed: a line typed before it is not saved yet (`sendFrom`). */
const FROM_WAITS = "From: not changed: a line above is not saved yet; set From: again once it is";
/**
 * The "From:" presets and the info_source category each stands for (the maintainer's table,
 * spec §3.1). The list's "From:" sends the LABEL (Rust maps it, `stamp`); a line's own pick sends
 * the category. Pinned against Rust's `info_source_of` by the webview tests, so they cannot drift.
 */
const FROM_PRESETS = {
  "Patient": "patient-reported",
  "Carer / family": "patient-reported",
  "GP letter": "external-record",
  "Pharmacy": "external-record",
  "Packets / Webster pack seen": "clinician-observed",
  "Other": "unknown",
};

/** The line is set aside: a version, never a line to chart — no tick, and not current. */
function isSetAside(line) {
  return Boolean(line.content.set_aside_of);
}

/** Whether Sign records something for the line: its tick, or — finished and set aside beside
 *  a CHARTED line — the version alone (one beside a list line goes in with that line's tick;
 *  an unfinished one is kept, Rust's `VERSION_UNFINISHED`). Sign enables on it. */
function signsAtSign(line) {
  return drawnTicked(line) || (line.finished && (line.content.set_aside_of || "").startsWith("med:"));
}

/** The list's or a line's own controls may not change now (a Sign in flight, or the list is
 *  another terminal's). */
function ownBlocked() {
  return medSigning || medList === null || medList.elsewhere !== null;
}

/** Fill a preset pick: "not set", then the six labels. */
function presetOptions(select) {
  const labels = Object.keys(FROM_PRESETS).map((label) => cell("option", label, { value: label }));
  select.replaceChildren(cell("option", NO_PRESET, { value: "" }), ...labels);
}

/** "From:" and the R14 line near Sign, drawn with the list (med_list.js's `drawMedList`). */
function drawListVersions() {
  el("med-list-from").hidden = medList === null;
  const preset = el("med-list-from-preset");
  const detail = el("med-list-from-detail");
  preset.disabled = ownBlocked();
  detail.disabled = ownBlocked();
  const from = (medList && medList.from) || {};
  if (document.activeElement !== preset) preset.value = from.preset || "";
  if (document.activeElement !== detail) detail.value = from.detail || "";
  const count = medList ? medList.versionsRecorded : 0; // Rust's, by Sign's grouping (R16)
  setMessage(el("med-list-versions-note"), count > 0 ? SIGN_ALSO[0] + count + SIGN_ALSO[1] : "");
  syncSettleButtons(); // the versions' own buttons, down while the list's are (med_versions_settle.js)
}

/**
 * "From:" changed: every line typed before it is saved first (so it never stamps them, ruling
 * R12), then "From:" is saved under the writer the list was READ under, over the charts on
 * screen, and the list Rust answers is drawn (lines saved after it take it, Rust's `stamp`).
 * Both are queued NOW, in the save queue: a line typed after the change waits behind the
 * "From:" save, so its first save reads the new "From:", even on a slow link.
 */
function saveFrom() {
  if (ownBlocked()) return;
  const patient = medList.patient;
  const writer = medList.writer; // whose list it is: a who's-here switch during the flush ends it
  // Read now: a list read landing during the flush redraws these controls from the old "From:".
  const preset = el("med-list-from-preset").value || null;
  const detail = el("med-list-from-detail").value.trim() || null;
  const flushed = flushMedLines();
  medSaveQueue = medSaveQueue.then(() => sendFrom(patient, writer, preset, detail, flushed));
}

/** The "From:" save, run in the save queue after `flushed` (the lines typed before it). Resolves
 *  true when nothing more is owed, false when the change was not saved (said in the status). */
async function sendFrom(patient, writer, preset, detail, flushed) {
  // A line typed before the change that did not save would be stamped on its retry as a NEW
  // line (R12 broken): the change waits until it saves, and says so.
  if (!(await flushed).every(Boolean)) {
    if (patient === renderedPatient && ownsTheList(writer)) setMessage(el("med-list-status"), FROM_WAITS);
    return false;
  }
  if (!ownsTheList(writer) || patient !== renderedPatient) return true;
  const revision = ++medListRevision;
  try {
    const fresh = await invoke("med_list_from_save", {
      patientId: patient,
      charts: renderedCharts,
      preset: preset,
      detail: detail,
      writer: writer,
    });
    if (revision === medListRevision && patient === renderedPatient) drawMedRead(fresh, patient);
    return true;
  } catch (failure) {
    if (patient === renderedPatient && ownsTheList(writer)) {
      setMessage(el("med-list-status"), failureText(failure));
    }
    return false;
  }
}

/** A line's own controls, made once on its row (never re-made, so focus stays put). */
function versionControls(line) {
  const r = line.row;
  const preset = cell("select", "", { class: "med-list-own-preset" });
  presetOptions(preset);
  const detail = cell("input", "", { type: "text", autocomplete: "off", class: "med-list-own-detail" });
  const why = cell("input", "", { type: "text", autocomplete: "off", class: "med-list-own-why" });
  const fromLabel = cell("label", FROM_LABEL + " ");
  fromLabel.append(preset);
  const box = cell("details", "", { class: "med-list-why" });
  const summary = cell("summary", WHY_LABEL);
  box.append(summary, fromLabel, detail, why);
  const own = {
    box, summary, preset, detail, why,
    source: cell("p", "", { class: "med-list-source" }),
    restores: cell("p", "", { class: "med-list-restores" }),
    setAside: cell("button", SET_ASIDE, { type: "button", class: "med-list-set-aside", "aria-pressed": "false" }),
    which: cell("p", "", { class: "med-list-which" }),
    whichKey: null, // the candidates the pick's buttons were made for
  };
  preset.addEventListener("change", () => sourceChosen(line, true));
  detail.addEventListener("input", () => sourceChosen(line, false));
  why.addEventListener("input", () => ownChanged(line, { why: why.value }));
  own.setAside.addEventListener("click", () => pressSetAside(line));
  r.li.append(own.source, own.restores, own.setAside, own.which, box);
  return own;
}

/** The preset a line's stored source shows as: the label it was stamped with (its label as the
 *  detail, of its category), else the first label of its category; "" only with no category
 *  the presets know (ruling R19: never "not set" beside a stored source). */
function presetOf(content) {
  const labels = Object.keys(FROM_PRESETS);
  const exact = labels.find((label) => label === content.source_detail && FROM_PRESETS[label] === content.info_source);
  return exact || labels.find((label) => FROM_PRESETS[label] === content.info_source) || "";
}

/** What the line says of its source and why, without being opened ("" when it names neither):
 *  the detail, or the preset's label — never a bare category word ("external-record"). */
function sourceWords(content) {
  const words = [];
  const source = content.source_detail || presetOf(content);
  if (source) words.push(FROM_LABEL + " " + source);
  if (content.why) words.push(WHY_LABEL + ": " + content.why);
  return words.join(" · ");
}

/**
 * Fill the line's own controls (med_list.js's `drawLine` calls it last): its source and why,
 * "Set aside" — offered only beside a companion, or to undo — and the pick. A set-aside line
 * has no tick and no reason, and reads `SET_ASIDE_MARK` where the tick's words were.
 */
function drawLineVersions(line) {
  const r = line.row;
  if (!r.own) r.own = versionControls(line);
  const own = r.own;
  const aside = isSetAside(line);
  // Rust's candidates are for the SAVED words: none while the line is typed (as its offers).
  const saved = line.view && medEditingLine !== line;
  const marks = saved ? line.view.companions : [];
  const words = saved ? line.view.companion_words : [];
  setMessage(own.source, sourceWords(line.content));
  // A restore line's mark, Rust's words as given; kept while the line is typed (its view is down).
  if (line.view) line.restoresWords = line.view.restores_words || "";
  setMessage(own.restores, line.restoresWords || "");
  // Named for their line, as its tick and Remove are (ruling R19).
  const text = line.snapshot ? line.snapshot.text : line.content.line_text;
  own.setAside.setAttribute("aria-label", SET_ASIDE + ": " + text);
  own.summary.setAttribute("aria-label", WHY_LABEL + ": " + text);
  own.preset.setAttribute("aria-label", "From, for " + text);
  own.detail.setAttribute("aria-label", "The document or person, for " + text);
  own.why.setAttribute("aria-label", WHY_LABEL + ", for " + text);
  const preset = presetOf(line.content);
  const detail = line.content.source_detail && line.content.source_detail !== preset ? line.content.source_detail : "";
  if (document.activeElement !== own.preset) own.preset.value = preset;
  if (document.activeElement !== own.detail) own.detail.value = detail;
  if (document.activeElement !== own.why) own.why.value = line.content.why || "";
  for (const control of [own.preset, own.detail, own.why, own.setAside]) control.disabled = ownBlocked();
  own.setAside.hidden = !(aside || marks.length > 0);
  own.setAside.setAttribute("aria-pressed", String(aside));
  drawWhich(line, marks, words);
  own.which.hidden = !(line.picking && !aside && marks.length > 1);
  r.tick.hidden = aside;
  if (aside) r.reasonRow.hidden = true;
  if (aside) r.kind.textContent = SET_ASIDE_MARK + ": "; // never "Already taking: " (review B)
  if (aside) setMessage(r.state, SET_ASIDE_MARK);
}

/** The pick's buttons, one per candidate in Rust's words, re-made only when the candidates change
 *  (so a button keeps its focus while the list is drawn again). */
function drawWhich(line, marks, words) {
  const r = line.row;
  const key = marks.join(" ");
  if (r.own.whichKey === key) return;
  r.own.whichKey = key;
  const buttons = marks.map((mark, i) => {
    const button = cell("button", words[i] || mark, { type: "button" });
    button.addEventListener("click", () => ownChanged(line, { set_aside_of: mark }));
    return button;
  });
  r.own.which.replaceChildren(cell("span", WHICH_LINE + " "), ...buttons);
}

/** "Set aside": on a set-aside line it undoes the mark (an ordinary line, held: ruling R1); else
 *  one candidate is taken, and with more the pick asks which (never a dialog). */
function pressSetAside(line) {
  if (ownBlocked()) return;
  const marks = line.view ? line.view.companions : [];
  if (isSetAside(line)) ownChanged(line, { set_aside_of: null });
  else if (marks.length === 1) ownChanged(line, { set_aside_of: marks[0] });
  else if (marks.length > 1) {
    line.picking = !line.picking;
    drawLine(line);
  }
}

/** The line's own "From:" changed (`picked`: the preset list, not the free text). A preset sends
 *  its category and the detail: the free text or, when blank, the label (as Rust's `stamp`).
 *  "Not set" PICKED clears the whole source, the box too (review M3: a stale stamp must come
 *  off; the press's default then stands at Sign); text typed under "not set" is a bare detail. */
function sourceChosen(line, picked) {
  const r = line.row;
  const label = r.own.preset.value;
  if (!label && picked) r.own.detail.value = "";
  const text = r.own.detail.value.trim();
  ownChanged(line, { info_source: FROM_PRESETS[label] || null, source_detail: text || label || null });
}

/** A change on the line's own controls: applied (`setOwn`), drawn, saved a second later. */
function ownChanged(line, fields) {
  if (ownBlocked()) return;
  setOwn(line, fields);
  touch(line);
}

/**
 * Apply `fields` (any of the four) to the line. The door counts it a content change: the hand
 * tick goes, but never the clinician's untick (ruling R15: that is about the drug), and a line
 * whose mark went away is HELD (ruling R1). The line is drawn unticked until it is read again.
 */
function setOwn(line, fields) {
  Object.assign(line.content, fields);
  line.handTicked = false;
  if (fields.set_aside_of === null) line.held = true;
  line.tickPending = true;
  line.picking = false;
  medLineNotes.delete(line.id);
  line.version += 1;
  line.ownVersion = { ...line.ownVersion }; // which version last changed each field (`takeOwn`)
  for (const field of Object.keys(fields)) line.ownVersion[field] = line.version;
}

/**
 * A read of the line (`v`; med_list.js's `mergeLine`): its own fields as the node holds them —
 * the source "From:" stamped on it included — even while it is being typed, except a field
 * changed here since the last save the node took (ruling R18). Without this an open line's next
 * autosave would send the stamp back as cleared, and the door would clear it.
 */
function takeOwn(line, v) {
  for (const field of OWN_FIELDS) {
    if (!((line.ownVersion || {})[field] > line.savedVersion)) line.content[field] = v[field];
  }
}

/**
 * A save answered `saved` with the source "From:" stamped on the line by it (Rust's `SaveAnswer`,
 * ruling R23): it goes on the line at once, so the line's next save carries it even when no read
 * lands between the two (else that save would send no source, and the door would clear it). A
 * source changed here after this save was sent is the clinician's newer choice, and stays.
 */
function takeStamp(line, door, sent) {
  if (!door.info_source) return;
  const own = line.ownVersion || {};
  if (own.info_source > sent.version || own.source_detail > sent.version) return;
  line.content.info_source = door.info_source;
  line.content.source_detail = door.source_detail;
}

/**
 * A line was removed (med_list.js's `removeMedLine`): each version marked beside it is an
 * ordinary, held line again — as the delete door leaves it (Review Focus 3) — so no mark names
 * a line that is gone, even on a version whose own change has not reached the node yet.
 */
function unmarkVersionsOf(gone) {
  for (const line of medLines) {
    if (line === gone || line.content.set_aside_of !== "line:" + gone.id) continue;
    setOwn(line, { set_aside_of: null });
    void queueSave(line);
  }
}

/** A charted line's set-aside versions (Rust's `versions`, worded there), collapsed under it in
 *  one `<details>`, closed until the clinician opens it: "{n} other version(s)" (", {u} to check"
 *  while Rust says any is `open`), then each version. None: no row. */
function versionRows(row) {
  if (row.versions.length === 0) return [];
  const tr = document.createElement("tr");
  tr.setAttribute("data-versions", "true");
  const td = document.createElement("td");
  td.colSpan = 5;
  const box = cell("details", "", { class: "med-versions" });
  keepOpen(box, row.group_id); // open again after a redraw if it was (med_versions_settle.js)
  const list = document.createElement("ul");
  list.append(...row.versions.map((version) => versionItem(version, row.group_id)));
  const toCheck = row.versions.filter((version) => version.open).length;
  const said = row.versions.length + OTHER_VERSIONS + (toCheck > 0 ? TO_CHECK[0] + toCheck + TO_CHECK[1] : "");
  box.append(cell("summary", said), list);
  td.append(box);
  tr.append(td);
  return [tr];
}

/** One version: its line, then each settlement as a further line of the same `li` (Rust's words,
 *  oldest first), then its buttons and its check row (med_versions_settle.js's hook). */
function versionItem(version, groupId) {
  const li = cell("li", version.line);
  for (const words of version.settled) li.append(cell("span", words, { class: "med-version-settled" }));
  li.append(...settleControls(version, groupId));
  return li;
}

presetOptions(el("med-list-from-preset"));
el("med-list-from-preset").addEventListener("change", () => void saveFrom());
el("med-list-from-detail").addEventListener("change", () => void saveFrom());
