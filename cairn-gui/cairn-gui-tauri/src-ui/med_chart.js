// The ONE-line editor of a medication line (charting slice 1a): the paper drug chart's line,
// "Amoxycillin 500 mg tds PO", as it is typed. Two buttons above the medication table, "Start
// medication" and "Already taking", add a line to the chart's unsigned list (med_list.js, batch
// charting slice 1); the press is the distinction paper never records (design §3.2), `charted_as`.
//
// What this file does: `MedLineEditor` mounts the one editor on the list line being typed. It
// splits the line into its drug part(s) and its directions (`splitLine`), offers drugs for a part
// as it is typed (`med_drug_search`), draws Rust's reading of the directions (`med_line_read`) and
// Rust's allergy check of the line (`med_line_check`), and tells the list when the line changed
// or was finished (Enter, Escape, or focus leaving it). It signs nothing: med_list.js saves the
// line and its one Sign charts every ticked line. Every reading, verdict and refusal is Rust's
// (med_chart.rs); only labels and hints are worded here.
//
// The one safety rule here: a coding vouches only for THE WORDS IT WAS PICKED WITH. A part whose
// words change loses its coding (`medHeld` is filtered on every input), so "amox" picked as
// amoxicillin and retyped "amoxapine" is sent uncoded and reads "not checked" — never checked-clean.
//
// Never a dialog: warnings, the reason field and refusals all sit on the line. Words the clinician
// sees are pinned in med_chart_webview_tests.rs. Classic script, loaded after drug_pick.js (its
// `PICK_DELAY_MS`) and before med_list.js (whose `appendMedLine` the buttons call, at a press),
// sharing main.js's `el`, `cell`, `setMessage`, `invoke`, `renderedPatient`, `renderedCharts`;
// allergy_tags.js's tag words; signer.js's `fixtureMode`.
"use strict";

const MED_PICK_MIN_CHARS = 4; // design §3.1: "after 4–5 typed letters"
const MED_NO_DRUG_LIST = "No drug list on this terminal: the medication is charted as typed";
/** What the line reads while no check speaks for its words: "not checked", never an old verdict. */
const LINE_NOT_CHECKED = { marks: [], notes: [], unverified: true };

/** The button pressed ("started-here" | "pre-existing"), or null while the line is closed. */
let medPress = null;
/** The line as last split: `{ typedDrug, parts, directions }` (see `splitLine`). */
let medLine = { typedDrug: "", parts: [], directions: "" };
/** The codings held, each `{ text, coding }`: the part's words it was picked with, and the pick. */
let medHeld = [];
/** The last check drawn (Rust's `LineCheckView`). */
let medCheck = LINE_NOT_CHECKED;
/** Words the reader reads on their own (true) or not (false), by lower-case word. */
const wordReads = new Map();
let medPickOptions = [];
let medPickActive = -1;
let medPickPart = null; // the part's words the options are for
let medPickTimer = null;
let medCheckTimer = null;
let medPickRevision = 0; // each counter drops an answer older than the newest asked
let medLineRevision = 0;
let medCheckRevision = 0;
/** The list's callbacks for the line the editor is mounted on (`MedLineEditor`), or null. */
let medEditor = null;
/** The editor's own row: moved into the list line being typed, and home again when it closes. */
const MED_LINE_ROW = el("med-line-row");

/**
 * Split the typed line into its drug parts and its directions. Pure. The drug part runs from the
 * first word up to the first word that starts the directions, which is one of:
 *  - a word led by a digit or "." — a dose or a strength ("500mg", "875/125", "1g", ".5mg");
 *  - a word the reader reads on its own (`isDirectionWord`: a form, route, frequency or flag —
 *    "tab", "IV", "bd", "stat"), so "Augmentin tab 875/125mg" keeps the form in the directions;
 *  - any word after a part the clinician has PICKED (`heldTexts`): the pick named the drug, so
 *    "amox" picked then " cap 500mg" keeps "amox" — and its coding — while "cap" is being typed.
 * A lone "+" separates the parts of a combination ("amox + clav"); the first word of each part is
 * always the drug's (a drug may begin with a digit: "5-fluorouracil"). `typedDrug` is the parts
 * as typed, joined by " + "; `directions` is the rest of the line exactly as typed.
 */
function splitLine(text, isDirectionWord, heldTexts) {
  const parts = [];
  let words = [];
  let end = text.length; // where the directions begin
  for (const match of text.matchAll(/\S+/g)) {
    const word = match[0];
    if (word === "+") {
      if (words.length > 0) parts.push(words.join(" "));
      words = [];
      continue;
    }
    const part = words.join(" ");
    if (words.length > 0 && (/^[\d.]/.test(word) || isDirectionWord(word) || heldTexts.includes(part))) {
      end = match.index;
      break;
    }
    words.push(word);
  }
  if (words.length > 0) parts.push(words.join(" "));
  return { typedDrug: parts.join(" + "), parts: parts, directions: text.slice(end).trim() };
}

// The split's pinned cases, run once at load against a stub reader that knows only these words
// (med_chart_webview_tests.rs checks each reads on its own in the real reader). A failure is a
// fault in this file, said on the console for whoever changed it.
const SPLIT_CASE_WORDS = ["tab", "tds", "bd", "po", "iv", "stat"];
const SPLIT_CASES = [
  ["amox 500mg tds", ["amox"], "500mg tds"],
  ["amox + clav 875/125 bd", ["amox", "clav"], "875/125 bd"],
  ["ceftriaxone 1g IV stat", ["ceftriaxone"], "1g IV stat"],
  ["Augmentin tab 875/125mg 1 bd po", ["Augmentin"], "tab 875/125mg 1 bd po"],
];
for (const [text, parts, directions] of SPLIT_CASES) {
  const got = splitLine(text, (w) => SPLIT_CASE_WORDS.includes(w.toLowerCase()), []);
  console.assert(JSON.stringify([got.parts, got.directions]) === JSON.stringify([parts, directions]), "splitLine", text, got);
}

/** Whether the reader reads `word` on its own (`learnWords` asked it first). */
function isDirectionWord(word) {
  return wordReads.get(word.toLowerCase()) === true;
}

/** Ask the reader about every word of `text` not yet known, so the split asks Rust's vocabulary,
 *  never a copy of it here. A failed ask counts the word as part of the drug's name (logged). */
async function learnWords(text) {
  const fresh = [...new Set(text.toLowerCase().split(/\s+/))].filter((w) => w && !/^[\d.+]/.test(w) && !wordReads.has(w));
  await Promise.all(
    fresh.map(async (word) => {
      try {
        const reading = await invoke("med_line_read", { text: word });
        wordReads.set(word, reading.unread.length === 0);
      } catch (failure) {
        console.error("med_line_read failed:", failure);
      }
    }),
  );
}

/** The line as typed now, split (its words learnt first). */
async function currentLine() {
  const text = el("med-line").value;
  await learnWords(text);
  return splitLine(text, isDirectionWord, medHeld.map((held) => held.text));
}

/** The coding held for a part's exact words, or null. */
function codingFor(part) {
  const match = medHeld.find((held) => held.text === part);
  return match ? match.coding : null;
}

/** The parts as `med_line_check` and `med_chart` take them (Rust's `PartArg`). */
function partArgs(line) {
  return line.parts.map((part) => ({ term: part, coding: codingFor(part) }));
}

/** Show the section over a drawn list on a writable chart only (main.js's `render` calls this). */
function syncMedChart() {
  el("med-chart").hidden = renderedPatient === null || fixtureMode;
  // A re-drawn list may hold a changed chart set or a newly signed allergy: an open line's
  // verdict is about the old one, so it is taken down and asked again.
  if (medPress !== null && !el("med-chart").hidden) recheck();
}

/** Press `value`: shown pressed, the line opened, and the line checked. */
function pressMed(value) {
  medPress = value;
  for (const button of el("med-chart-buttons").querySelectorAll("button")) {
    button.setAttribute("aria-pressed", String(button.dataset.value === value));
  }
  el("med-line-row").hidden = false;
  el("med-line").focus();
  drawCheck(medCheck); // the reason field follows the press
  scheduleCheck();
}

/**
 * Close the editor and forget everything typed in it (the list keeps the line). The list is told
 * nothing more first, and the row goes home first: moving it blurs the field (no "finished"
 * from that), and the list line it sat in may already be gone.
 */
function closeMedLine() {
  medEditor = null;
  el("med-chart").append(MED_LINE_ROW);
  medPress = null;
  medHeld = [];
  medLine = { typedDrug: "", parts: [], directions: "" };
  medLineRevision += 1;
  medCheckRevision += 1;
  clearTimeout(medCheckTimer);
  for (const button of el("med-chart-buttons").querySelectorAll("button")) button.setAttribute("aria-pressed", "false");
  el("med-line").value = "";
  closeMedPick();
  drawCodedAs();
  drawReadBack("");
  drawCheck(LINE_NOT_CHECKED);
  setMessage(el("med-line-status"), "");
  el("med-line-row").hidden = true;
}

/** Enter or Escape with no pick list open (`byKey`), or focus leaving the line: the line is
 *  finished (design §3.2: only a finished line can get the default tick). The list closes the editor. */
function finishMedLine(byKey) {
  if (medEditor) medEditor.finished(byKey);
}

/**
 * Mount the editor on one list line (med_list.js): `container` is the line's slot, `line` its
 * saved words `{ charted_as, line_text, parts }`, `callbacks` `{ changed(), finished(byKey) }`.
 * The parts' picks are held again for their exact words, so a saved coding still vouches only
 * for them. Opening reads the line back and checks it; nothing is saved by opening it.
 */
function MedLineEditor(container, line, callbacks) {
  closeMedLine();
  medHeld = line.parts.filter((part) => part.coding).map((part) => ({ text: part.term, coding: part.coding }));
  container.append(MED_LINE_ROW);
  el("med-line").value = line.line_text;
  medEditor = callbacks;
  pressMed(line.charted_as);
  void currentLine().then((split) => {
    if (medEditor !== callbacks) return; // closed meanwhile
    medLine = split;
    drawCodedAs();
    void readBack(split);
  });
}

/** What the editor holds now, read at once (the list saves it later): the words and the picks. */
function medEditorSnapshot() {
  return { text: el("med-line").value, held: medHeld.slice() };
}

/** The chart changed (main.js's `clearChart`): a line typed for one patient is never signed on the next. */
function clearMedChart() {
  closeMedLine();
  el("med-chart").hidden = true;
}

/** Every keystroke: split, forget codings whose words changed, offer drugs for an edited part,
 *  read the directions back, and check the line again when its drug part changed. */
async function onMedLineInput() {
  if (medEditor) medEditor.changed(); // before any await: a save flushed now carries this keystroke
  const revision = ++medLineRevision;
  setMessage(el("med-line-status"), "");
  const line = await currentLine();
  if (revision !== medLineRevision) return;
  medHeld = medHeld.filter((held) => line.parts.includes(held.text)); // a coding vouches for its words only
  const edited = line.parts.find((part) => !medLine.parts.includes(part) && codingFor(part) === null);
  const drugChanged = line.typedDrug !== medLine.typedDrug;
  medLine = line;
  drawCodedAs();
  closeMedPick();
  if (edited && edited.length >= MED_PICK_MIN_CHARS) {
    medPickTimer = setTimeout(() => void askMedPick(edited), PICK_DELAY_MS);
  }
  void readBack(line);
  if (drugChanged) recheck();
}

/** The line under the field while any part is coded: what each part will be signed as. */
function drawCodedAs() {
  const line = el("med-coded-as");
  const words = medLine.parts.map((part) => (codingFor(part) ? codingFor(part).display : part + " (not coded)"));
  line.textContent = "Coded as: " + words.join(" + ");
  line.hidden = !medHeld.some((held) => medLine.parts.includes(held.text));
}

/** Close the pick list and its hint. */
function closeMedPick() {
  clearTimeout(medPickTimer);
  medPickRevision += 1;
  medPickOptions = [];
  medPickActive = -1;
  medPickPart = null;
  el("med-pick").replaceChildren();
  el("med-pick").hidden = true;
  setMessage(el("med-pick-hint"), "");
  el("med-line").setAttribute("aria-expanded", "false");
}

/** Mark option `index` as the one Enter takes. */
function activateMedPick(index) {
  medPickActive = index;
  [...el("med-pick").children].forEach((li, i) => li.setAttribute("aria-selected", String(i === index)));
}

/** Take option `index` for the part it was offered for, if those words are still on the line. */
function chooseMedPick(index) {
  const option = medPickOptions[index];
  if (!option || !medLine.parts.includes(medPickPart)) return;
  medHeld = medHeld.filter((held) => held.text !== medPickPart).concat([{ text: medPickPart, coding: { system: option.system, code: option.code, display: option.display } }]);
  closeMedPick();
  drawCodedAs();
  recheck(); // every coding change is checked
  if (medEditor) medEditor.changed(); // and saved: the pick is part of the line
}

/** Draw a `med_drug_search` answer: the drugs, top one highlighted, or the no-drug-list hint. */
function drawMedPick(found, part) {
  closeMedPick();
  if (!found.available) {
    setMessage(el("med-pick-hint"), MED_NO_DRUG_LIST);
    return;
  }
  medPickOptions = found.options;
  medPickPart = part;
  el("med-pick").replaceChildren(
    ...medPickOptions.map((option, i) => {
      const li = cell("li", option.display, { role: "option", "aria-selected": "false" });
      li.addEventListener("click", () => chooseMedPick(i));
      return li;
    }),
  );
  el("med-pick").hidden = medPickOptions.length === 0;
  el("med-line").setAttribute("aria-expanded", String(medPickOptions.length > 0));
  if (medPickOptions.length > 0) activateMedPick(0);
}

/** Ask for drugs matching `text` (one part's words); an older answer is dropped. */
async function askMedPick(text) {
  const revision = ++medPickRevision;
  try {
    const found = await invoke("med_drug_search", { text: text });
    if (revision === medPickRevision && medLine.parts.includes(text)) drawMedPick(found, text);
  } catch (failure) {
    // The pick is optional: the line charts as typed. Logged, so a broken search is not "no matches".
    console.error("med_drug_search failed:", failure);
    if (revision === medPickRevision) closeMedPick();
  }
}

/** Read the directions back (Rust's `Directions`); an older answer is dropped. */
async function readBack(line) {
  const revision = medLineRevision;
  let reading; // undefined: nothing read
  try {
    if (line.directions) reading = await invoke("med_line_read", { text: line.directions });
  } catch (failure) {
    console.error("med_line_read failed:", failure);
    // Nothing read is never "nothing typed": the directions show whole, as not read.
    reading = { unread: [line.directions] };
  }
  const drug = line.parts.map((part) => (codingFor(part) ? codingFor(part).display : part)).join(" + ");
  if (revision === medLineRevision) drawReadBack(drug, reading);
}

/** The read-back row: what the line will be signed as, field by field ("—": not read), then the
 *  words the reader could not read. Hidden while nothing is typed. `target`: a list line's own row. */
function drawReadBack(drug, reading = { unread: [] }, target = el("med-readback")) {
  const dose = reading.dose_amount ? [reading.dose_amount, reading.dose_unit].filter(Boolean).join(" ") : null;
  const often = [reading.frequency, reading.prn ? "when needed" : null, reading.stat ? "stat" : null].filter(Boolean).join(", ");
  const fields = [
    ["Drug ", drug],
    ["Form ", reading.form],
    ["Strength ", reading.strength],
    ["Dose ", dose],
    ["Route ", reading.route],
    ["How often ", often],
  ];
  const said = fields.map(([label, value]) => label + (value || "—"));
  if (reading.unread.length > 0) said.push("not read: " + reading.unread.join(" "));
  const any = fields.some(([, value]) => value) || reading.unread.length > 0;
  setMessage(target, any ? said.join(" · ") : "");
}

/** What is checked changed (its words, or a pick): every check in flight is about the old line, so
 *  it is outdated, and the line reads "not checked" AT ONCE — never the old verdict, which may be
 *  clean, for the delay and the round trip (or for ever, if the check hangs) — until the new one answers. */
function recheck() {
  medCheckRevision += 1;
  drawCheck(LINE_NOT_CHECKED);
  scheduleCheck();
}

/** Check the line again after a moment of quiet. */
function scheduleCheck() {
  clearTimeout(medCheckTimer);
  medCheckTimer = setTimeout(() => void checkLine(), PICK_DELAY_MS);
}

/** Ask the node to check the line against the chart's allergies (R16: with the drug as typed). */
async function checkLine() {
  const revision = ++medCheckRevision;
  const line = await currentLine();
  try {
    const check = await invoke("med_line_check", {
      patientId: renderedPatient,
      charts: renderedCharts,
      typedDrug: line.typedDrug,
      parts: partArgs(line),
    });
    if (revision === medCheckRevision) drawCheck(check);
  } catch (failure) {
    console.error("med_line_check failed:", failure);
    if (revision === medCheckRevision) drawCheck(LINE_NOT_CHECKED);
  }
}

/** Draw the editor's check (the reason field is the list line's own, med_list.js). */
function drawCheck(check) {
  medCheck = check;
  drawMarks(check, el("med-line-tag"), el("med-line-marks"));
}

/** Draw a check into `tag` and `list`: the loud tag and each mark, the muted notes, or `not
 *  checked`. Shared with med_list.js, which draws each saved line's check the same way. */
function drawMarks(check, tag, list) {
  let shown = true;
  if (check.marks.length > 0) {
    tag.textContent = ALLERGY_TAG;
    tag.className = "allergy-tag";
  } else if (check.unverified) {
    tag.textContent = NOT_CHECKED_TAG;
    tag.className = "allergy-unchecked-tag";
  } else {
    shown = false;
  }
  tag.hidden = !shown;
  list.replaceChildren(
    ...check.marks.map((mark) => cell("li", mark, { class: "allergy-prominent" })),
    ...check.notes.map((note) => cell("li", note, { class: "allergy-quiet" })),
  );
}

for (const button of el("med-chart-buttons").querySelectorAll("button")) {
  button.addEventListener("click", () => appendMedLine(button.dataset.value)); // med_list.js
}
el("med-line").addEventListener("input", () => void onMedLineInput());
el("med-line").addEventListener("keydown", (event) => {
  const n = medPickOptions.length;
  if (event.key === "Escape") {
    if (n > 0) closeMedPick();
    else finishMedLine(true);
  } else if (event.key === "Enter" && n === 0) finishMedLine(true);
  else if (n === 0) return;
  else if (event.key === "ArrowDown") activateMedPick((medPickActive + 1) % n);
  else if (event.key === "ArrowUp") activateMedPick((medPickActive - 1 + n) % n);
  else if (event.key === "Enter" && medPickActive >= 0) chooseMedPick(medPickActive);
  else return;
  event.preventDefault();
});
// Focus leaving the line finishes it — not a click on a drug (the list keeps the focus in the
// field), nor the window losing focus to another program (the clinician has not left the line).
el("med-pick").addEventListener("mousedown", (event) => event.preventDefault());
MED_LINE_ROW.addEventListener("focusout", (event) => {
  if (document.hasFocus() && !MED_LINE_ROW.contains(event.relatedTarget)) finishMedLine(false);
});
closeMedLine();
