// Charting slice 2 on the charted list (design §3.6): a stat dose, ordered then given. Under a stat
// order's drug line, its stat line ("stat · ordered … · not yet given", or "· given …"), each give
// entered in error as a line of its own, and two buttons — "Given" (an inline row: "Time given",
// pre-filled with now, signed on its own Sign) and "Entered in error" on each standing give (an
// inline row: "Why (optional)", signed likewise). `statRows` is called by main.js's `renderRow`.
//
// The paper counterpart: the nurse initials the stat box with the time given; a dose initialled in
// error is struck through with the striker's initials, never erased.
//
// What this file decides: nothing clinical. Every word of the line, whether Given or Entered in
// error is offered (`can_give`, `can_strike`: the opened chart only, never a linked chart's), and
// whether the line reads "given" are Rust's (chart_stat.rs, the tab crate's stat_view.rs); both
// acts are re-derived in Rust on a fresh read of the OPENED chart before the key (med_stat.rs).
// What it guards is the screen: a button is down while a Sign or a list act is in flight
// (`medSigning`, through med_versions_settle.js's `settleBlocked` — the buttons carry
// `data-settle="stat"`, so `syncSettleButtons` re-decides them on every list draw); a row signs
// nothing until its own Sign; a partly typed time is refused here, because it reaches this script
// as an EMPTY value and Rust reads empty as now (`pressStatSign`); and a refusal is said beside
// the line, never only in a status bar. No dialog.
//
// The table is drawn again on every chart read, so what must outlive a redraw lives here, not in
// the DOM: the open row and its typed words (`statOpen`) and each line's last refusal
// (`statSaid`). A chart change clears both (`clearStat`, from main.js's `clearChart`).
//
// Classic script, loaded after med_versions_settle.js (`settleBlocked`) and med_list_sign.js
// (`drawSignedTable`, `medListSigning`, `medListSignInFlight`), sharing med_list.js's `medSigning`,
// `drawMedList`; main.js's `el`, `cell`, `setMessage`, `invoke`, `renderedPatient`,
// `renderedCharts`, `displayedPatient`, `refresh`; signer.js's `signerCredential`, `clearSigner`;
// notes.js's `signInFlight`; chart_stale.js's `behindTheRecord`; funnel.js's `failureText` — all
// read inside functions, after every script loaded.
"use strict";

const GIVE = "Given";
const STRIKE = "Entered in error";
const GIVE_TIME_LABEL = "Time given";
const STRIKE_WHY_LABEL = "Why (optional)";
const STAT_SIGN = "Sign";
const STAT_CANCEL = "Cancel";
/** Said instead of signing a time only begun (the notes pane's words, for a dose). */
const PARTLY_TYPED_GIVEN = "the time given is only partly typed — finish it, or clear it if the dose was given now";
/** A fault in this window mid-sign: what went in is the pane's next read. */
const STAT_FAULT = "Whether this was recorded is not known — read the chart again.";

/**
 * The one open row: `{ kind: "give" | "strike", medicationId, eventId, time, why }` as typed, or
 * null. `time` is null while the field holds a partly typed time (`keep`).
 */
let statOpen = null;
/** The last refusal said beside each stat line, by the order's medication id. */
const statSaid = new Map();

/** The chart changed: nothing typed or said about one chart's stat doses follows to the next. */
function clearStat() {
  statOpen = null;
  statSaid.clear();
}

/** Above the list, beside the versions' warning: the stat orders could not be read (Rust's
 *  words), so no line says whether it was given. Made here, once, so index.html stays markup. */
function statNotRead(text) {
  let p = el("stat-not-read");
  if (!p) {
    p = cell("p", "", { id: "stat-not-read", role: "status" });
    el("versions-not-read").after(p);
  }
  setMessage(p, text);
}

/** `d` as a `datetime-local` value in this machine's zone, to the minute (notes_time's
 *  `datetime_local` shape): what the field shows is what is signed. */
function localMinute(d) {
  const two = (n) => String(n).padStart(2, "0");
  return d.getFullYear() + "-" + two(d.getMonth() + 1) + "-" + two(d.getDate()) + "T" + two(d.getHours()) + ":" + two(d.getMinutes());
}

/** A button on `row`'s stat line, named for its drug, down while a Sign or a list act is. */
function statButton(label, row, press) {
  const button = cell("button", label, {
    type: "button",
    class: "med-stat-act",
    "data-settle": "stat",
    "aria-label": label + ": " + row.primary,
  });
  button.disabled = settleBlocked("stat");
  button.addEventListener("click", press);
  return button;
}

/** The stat line under `row`, as one table row: Rust's words, each struck give, the buttons Rust
 *  offers, the refusal last said about it, and its open row. Nothing for a line that is no stat
 *  order. */
function statRows(row) {
  const stat = row.stat;
  if (!stat) return [];
  const tr = document.createElement("tr");
  tr.setAttribute("data-stat", "true");
  const td = cell("td", "");
  td.colSpan = 5;
  td.append(cell("span", stat.line, { class: "med-stat-line" }));
  for (const words of stat.struck) td.append(cell("span", words, { class: "med-stat-struck" }));
  if (stat.can_give) td.append(statButton(GIVE, row, (event) => openStat("give", stat, null, row, event.target)));
  for (const give of stat.gives) {
    if (!give.can_strike) continue;
    td.append(statButton(STRIKE, row, (event) => openStat("strike", stat, give, row, event.target)));
  }
  const said = cell("p", statSaid.get(stat.medication_id) || "", { class: "med-stat-said allergy-field-error", "data-stat-said": stat.medication_id });
  said.hidden = !statSaid.has(stat.medication_id);
  td.append(said);
  if (stillOffered(stat)) td.append(statRow(stat, row));
  tr.append(td);
  return [tr];
}

/** Whether the open row belongs on `stat`, and its button is still offered there: a redraw after
 *  the line moved (given elsewhere, struck since) drops the row rather than keep a stale act. */
function stillOffered(stat) {
  if (!statOpen || statOpen.medicationId !== stat.medication_id) return false;
  if (statOpen.kind === "give") return stat.can_give;
  return stat.gives.some((give) => give.can_strike && give.event_id === statOpen.eventId);
}

/** Say `text` beside stat line `medicationId`, now and on every redraw until its next press. */
function sayStat(medicationId, text) {
  if (text) statSaid.set(medicationId, text);
  else statSaid.delete(medicationId);
  for (const said of document.querySelectorAll(".med-stat-said")) {
    if (said.dataset.statSaid === medicationId) setMessage(said, text);
  }
}

/** "Given" or "Entered in error": its inline row opens under the line (one open at a time); it
 *  signs nothing until its own Sign. The time given is now, at the press, to the minute. */
function openStat(kind, stat, give, row, opener) {
  if (settleBlocked("stat")) return;
  sayStat(stat.medication_id, "");
  for (const open of document.querySelectorAll(".med-stat-row")) open.remove();
  statOpen = {
    kind: kind,
    medicationId: stat.medication_id,
    eventId: give ? give.event_id : null,
    time: kind === "give" ? localMinute(new Date()) : "",
    why: "",
  };
  const form = statRow(stat, row);
  opener.closest("td").append(form);
  form.querySelector("input").focus();
}

/** Close the open row, signing nothing; focus back on the line's first button. */
function closeStat(form) {
  const back = form.closest("td") && form.closest("td").querySelector(".med-stat-act");
  statOpen = null;
  form.remove();
  if (back) back.focus();
}

/** Keep what the open row's field holds. A partly typed `datetime-local` has value "" and
 *  `validity.badInput`: kept as null, never as "" (which Rust reads as now). */
function keep(field) {
  if (!statOpen) return; // a field removed with its row may still report a blur
  if (statOpen.kind === "give") statOpen.time = field.validity.badInput ? null : field.value;
  else statOpen.why = field.value;
}

/** The open row from `statOpen`'s words: "Time given" (a `datetime-local`, `max` now) or "Why
 *  (optional)", its Sign and Cancel. Every keystroke is kept, so a redraw rebuilds it as typed. */
function statRow(stat, row) {
  const words = statOpen;
  const give = words.kind === "give";
  const form = cell("div", "", { class: "med-stat-row", role: "group", "aria-label": (give ? GIVE : STRIKE) + ": " + row.primary });
  const field = cell("input", "", give ? { type: "datetime-local", max: localMinute(new Date()) } : { type: "text", autocomplete: "off" });
  field.value = give ? words.time || "" : words.why;
  for (const type of ["input", "change", "keyup", "blur"]) field.addEventListener(type, () => keep(field));
  const label = cell("label", (give ? GIVE_TIME_LABEL : STRIKE_WHY_LABEL) + " ");
  label.append(field);
  const sign = statButton(STAT_SIGN, row, () => pressStatSign(stat, field));
  const cancel = cell("button", STAT_CANCEL, { type: "button" });
  cancel.addEventListener("click", () => {
    if (!medSigning) closeStat(form);
  });
  form.append(label, sign, cancel);
  return form;
}

/** The open row's Sign: one press, one act; a who's-here switch and a re-lock wait for it, as for
 *  the list's Sign (notes.js's `signsSettled`). A partly typed time is refused here, beside the
 *  line, and nothing is invoked: sent as "", it would sign as now. */
function pressStatSign(stat, field) {
  if (signInFlight() || settleBlocked("stat")) return;
  const words = statOpen;
  keep(field);
  if (words.kind === "give" && words.time === null) return sayStat(stat.medication_id, PARTLY_TYPED_GIVEN);
  medListSignInFlight = true;
  medListSigning = signStat(stat, words)
    .catch((failure) => {
      console.error(failure);
      return { words: STAT_FAULT };
    })
    .finally(() => {
      medListSignInFlight = false;
    });
}

/**
 * Sign the give or the strike with the row's words and the strip's credential, on the chart
 * drawn, over the set it was read in. Signed: the row closes and the pane Rust read again is
 * drawn. Refused: the words stay in the row and the refusal is said beside the line; a list behind
 * the record is read again. Resolves `{ words }` for `signsSettled`.
 */
async function signStat(stat, words) {
  const patient = renderedPatient;
  const charts = renderedCharts;
  sayStat(stat.medication_id, "");
  medSigning = true;
  drawMedList();
  try {
    let answer;
    try {
      answer =
        words.kind === "give"
          ? await invoke("med_stat_give", {
              patientId: patient,
              charts: charts,
              medicationId: stat.medication_id,
              givenAt: words.time || null,
              credential: signerCredential(),
            })
          : await invoke("med_stat_strike", {
              patientId: patient,
              charts: charts,
              givenEvent: words.eventId,
              why: words.why.trim() || null,
              credential: signerCredential(),
            });
      clearSigner(true);
    } catch (failure) {
      clearSigner();
      if (patient !== displayedPatient) return { words: failureText(failure) };
      sayStat(stat.medication_id, failureText(failure));
      const behind = behindTheRecord(failure);
      if (behind) await refresh(failureText(failure), behind);
      return { words: failureText(failure) };
    }
    if (statOpen === words) statOpen = null;
    if (patient === displayedPatient) drawSignedTable(answer, patient);
    return { words: answer.outcome };
  } finally {
    medSigning = false;
    drawMedList();
  }
}
