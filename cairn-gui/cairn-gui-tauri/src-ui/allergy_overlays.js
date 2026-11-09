// The gestures on a line of the expanded allergy list (allergy slice 2): the overlays paper does
// with a pen on the sticker, done here as signed acts that name the line they are about.
//
//   - a live line:            Correct, De-label, Entered in error
//   - a historical line:      Withdraw de-label (Entered in error, aimed at the de-label it rests
//                             on), and Entered in error (aimed at its own entry: one act strikes it)
//   - a "none known" line:    Entered in error
//   - any line from a chart whose link to this one is in doubt: NOTHING (it may be a stranger's;
//     Rust refuses such a target before the key anyway, review C1)
//
// Correct reuses the entry form (allergies.js): it fills it from the line's words as recorded,
// says "Correcting: <the line>" above it (the label is the page's), and the form's Sign then sends
// `corrects`. A correction whose line has left the list on a re-read is SUSPENDED (its words
// kept, Sign down, the page's sentence saying why) rather than re-sent into a refusal forever.
// De-label and Entered in error open ONE inline row under their line, with its own Sign. The
// buttons and the rows are cloned from the page's `<template>`s, so every label is written in
// index.html, not here (an accessible name adds only ": " and the line's text). Never a dialog,
// never a "are you sure?" (principle 3, paper-parity): the row and one Sign are the whole act,
// and Escape or Cancel closes it signing nothing (once no press is in flight).
//
// What is NOT decided here: every sentence the clinician reads after a press is Rust's (the
// outcome, every refusal); whether the line is still on the record is checked by Rust on a fresh
// read before the key (`allergy_overlays.rs`). Each Sign goes through allergies.js's `pressOnce`
// (one press, one signed act; a fault mid-press is handled there) and lands through its
// `afterSigned` / `afterRefused` (a late answer for a chart left touches nothing, R13) — reused,
// never copied. Text is drawn with `textContent` only (main.js's `cell`); no markup is ever built.
//
// Classic script, loaded BEFORE allergies.js (whose load-time `resetAllergyForm()` calls
// `clearCorrection`), sharing the global scope: allergies.js's state and helpers
// (`allergyPatient`, `allergyCharts`, `allergyBusy`, `pressOnce`, `afterSigned`, `afterRefused`,
// `resetAllergyForm`, `pressChoice`, `syncAllergyButtons`), main.js's `el`, `cell`, `setMessage`
// and `invoke`, signer.js's `signerCredential` and `clearSigner` — every one read only when a
// list is drawn or a button pressed, long after every script has loaded.
"use strict";

/**
 * The live line the entry form is correcting (its `event_id`, sent as `corrects`), or null while
 * the form records a new entry. Cleared whenever the form is emptied (`resetAllergyForm`).
 */
let allergyCorrecting = null;

/**
 * True while that correction is suspended: a list drawn since Correct no longer has its line
 * live (corrected, de-labelled or withdrawn elsewhere). Sign is down until Correct is pressed on
 * a line or the correction cancelled — re-sending it could only be refused, read again, refused.
 */
let correctionSuspended = false;

/**
 * One of a line's buttons: the page's button for `act` (`#allergy-line-buttons`), with an
 * accessible name that says which line it acts on (a screen reader otherwise hears a list of
 * identical Correct buttons). `.allergy-write`: it is down whenever no write may be pressed
 * (`syncAllergyButtons`, called after every draw). `onPress(li, button)` gets its line's <li>.
 */
function actionButton(act, line, onPress) {
  const button = el("allergy-line-buttons").content.querySelector('[data-act="' + act + '"]').cloneNode(true);
  button.setAttribute("aria-label", button.textContent + ": " + line.text);
  button.addEventListener("click", () => onPress(button.closest("li"), button));
  return button;
}

/**
 * The buttons on one line of a drug, other, adverse-effect or historical list (allergies.js's
 * `lineItem`). A line from a doubted chart carries none. A historical line rests on a de-label
 * (`line.challenge`): it is withdrawn through it, or the line itself is struck. A live line is
 * corrected (when its recorded words came with it), de-labelled, or struck.
 */
function lineActions(line) {
  if (line.doubted_chart) return [];
  if (line.challenge) {
    return [
      actionButton("withdraw", line, (li, opener) => openRetraction(line.challenge, li, opener)),
      actionButton("error", line, (li, opener) => openRetraction(line.event_id, li, opener, line)),
    ];
  }
  const buttons = [];
  if (line.words) buttons.push(actionButton("correct", line, () => startCorrection(line)));
  buttons.push(actionButton("delabel", line, (li, opener) => openDelabel(line, li, opener)));
  buttons.push(actionButton("error", line, (li, opener) => openRetraction(line.event_id, li, opener, line)));
  return buttons;
}

/** The button on a standing or previous "none known" line (none from a doubted chart). */
function noneActions(none) {
  if (none.doubted_chart) return [];
  return [actionButton("error", none, (li, opener) => openRetraction(none.event_id, li, opener))];
}

/**
 * Fill the entry form from `line`'s words as recorded and mark it as correcting that line. Each
 * choice goes through `pressChoice`, which presses only a button whose word IS the recorded word:
 * a newer peer's kind or category this window has no button for stays UNPRESSED, so the
 * clinician chooses (Rust refuses a missing one before the key) — an unknown word is never sent
 * back. The onset is recorded in ISO form ("2019", "2019-03", "2019-03-14"), which the form reads.
 */
function startCorrection(line) {
  if (allergyBusy) return;
  closeOverlayRow();
  resetAllergyForm(); // an earlier correction's words never mix into this one's
  const words = line.words;
  pressChoice("allergen_kind", words.allergen_kind);
  pressChoice("kind", words.kind);
  pressChoice("category", words.category);
  pressChoice("source", words.source);
  el("allergy-term").value = words.term;
  el("allergy-reaction-text").value = words.reaction_text || "";
  el("allergy-onset").value = words.onset_value || "";
  allergyCorrecting = line.event_id;
  el("allergy-correcting-text").textContent = line.text;
  el("allergy-correcting-line").hidden = false;
  el("allergy-correcting").hidden = false;
  el("allergy-term").focus();
}

/**
 * The form corrects nothing any more (allergies.js's `resetAllergyForm` calls this — so Correct on
 * a line, Cancel correction and every emptying of the form resume a suspended correction).
 */
function clearCorrection() {
  allergyCorrecting = null;
  correctionSuspended = false;
  el("allergy-correcting-gone").hidden = true;
  el("allergy-correcting-text").textContent = "";
  el("allergy-correcting").hidden = true;
  syncAllergyButtons(); // a suspended correction had Sign down
}

/**
 * Suspend the correction when `pane` (a list just drawn, allergies.js's `renderAllergies`) no
 * longer has its line among the live ones: the typed words stay, Sign goes down
 * (`syncAllergyButtons`), and the page's sentence replaces "Correcting: …".
 */
function suspendLostCorrection(pane) {
  if (allergyCorrecting === null) return;
  const live = pane.drug_allergies.concat(pane.other_allergies, pane.adverse_effects);
  if (live.some((line) => line.event_id === allergyCorrecting)) return;
  correctionSuspended = true;
  el("allergy-correcting-line").hidden = true;
  el("allergy-correcting-gone").hidden = false;
}

/**
 * Open the De-label row under `li`: what was tolerated (EMPTY — the clinician names it; the drug
 * challenged is often not the labelled one, and nothing is signed by default), when (typed as an
 * onset is; empty = not known), an optional note, and Sign.
 */
function openDelabel(line, li, opener) {
  const row = openRow("allergy-delabel-template", li, opener);
  onSign(row, () => void pressOnce(() => signOverlay(row, (base) => invoke("allergy_challenge", Object.assign(base, {
    target: line.event_id,
    tolerated: rowInput(row, "tolerated").value,
    date: rowInput(row, "date").value,
    text: rowInput(row, "text").value,
  })))));
}

/**
 * Open the Entered-in-error row under `li` for event `target` (a line's entry, a "none known"
 * statement, or the de-label a historical line rests on): an optional reason, and Sign. Nothing
 * is struck until it is signed. `line` is passed only when the line's own entry is struck: if it
 * has earlier versions they are struck with it (ruling R-4a), so the page's sentence says so —
 * and, on a line that can be corrected (its words came with it), how to undo a correction instead.
 */
function openRetraction(target, li, opener, line) {
  const row = openRow("allergy-retract-template", li, opener);
  const strikesEarlier = Boolean(line) && line.earlier.length > 0;
  row.querySelector('[data-note="earlier"]').hidden = !strikesEarlier;
  row.querySelector('[data-note="undo"]').hidden = !(strikesEarlier && line.words);
  onSign(row, () => void pressOnce(() => signOverlay(row, (base) => invoke("allergy_retract", Object.assign(base, {
    target: target,
    text: rowInput(row, "text").value,
  })))));
}

/**
 * Clone `templateId`'s row under `li` — the only overlay row open (one at a time) — named as the
 * button that opened it ("De-label: <line>"), its Cancel wired, its first field focused. Cancel
 * waits for a press in flight: closing the row would not stop the sign, and its answer would
 * land in a row no longer on screen. Answers the row.
 */
function openRow(templateId, li, opener) {
  closeOverlayRow();
  const row = el(templateId).content.firstElementChild.cloneNode(true);
  row.setAttribute("aria-label", opener.getAttribute("aria-label"));
  row.querySelector('[data-act="cancel"]').addEventListener("click", () => {
    if (!allergyBusy) closeOverlayRow(li);
  });
  li.append(row);
  syncAllergyButtons();
  row.querySelector("input").focus();
  return row;
}

/** Wire `row`'s Sign to `press`. */
function onSign(row, press) {
  row.querySelector('[data-act="sign"]').addEventListener("click", press);
}

/** The input of `row` for field `name` (`data-field`). */
function rowInput(row, name) {
  return row.querySelector('input[data-field="' + name + '"]');
}

/**
 * Close the open overlay row, signing nothing. With `li` (Cancel, Escape), focus goes back to
 * the line's first button, so a keyboard user is where they were.
 */
function closeOverlayRow(li) {
  for (const row of document.querySelectorAll(".allergy-overlay-row")) row.remove();
  const back = li && li.querySelector("button");
  if (back) back.focus();
}

/**
 * Sign one overlay on the chart whose list is drawn, inside `pressOnce`. `send(base)` is the
 * act's own `invoke` (a literal command name, so commands_tests.rs sees it is registered), given
 * the arguments every write carries: the chart, the chart set the list was read over, the key.
 * Signed: `afterSigned` redraws the list (the row goes with it). Refused: `afterRefused` — a
 * stale list is read again; any other refusal ("Name what was tolerated.", a date it cannot
 * read) is said IN the row, what was typed kept.
 */
async function signOverlay(row, send) {
  const patient = allergyPatient; // the chart this press acts on
  const said = row.querySelector(".allergy-field-error");
  setMessage(said, "");
  let answer;
  try {
    answer = await send({
      patientId: patient,
      charts: allergyCharts,
      credential: signerCredential(),
    });
    clearSigner(true);
  } catch (failure) {
    clearSigner();
    await afterRefused(failure, patient, false, (text) => setMessage(said, text));
    return;
  }
  afterSigned(answer, patient, false);
}

el("allergy-correct-cancel").addEventListener("click", () => {
  if (!allergyBusy) resetAllergyForm();
});
// Escape closes the open row (signing nothing), wherever focus is inside the allergy lists —
// once no press is in flight (as Cancel).
el("allergy-details").addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || allergyBusy) return;
  const row = event.target.closest(".allergy-overlay-row");
  if (row) closeOverlayRow(row.closest("li"));
});
