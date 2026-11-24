// An older window against a newer database (#909 slice C1, design §4.4–§4.5). The node's database
// can be upgraded under a window that is still running; the database then says how old a window
// it still serves (its floor). This script is what the window shows about that, and when it
// restarts into a newer program:
//
// - an update WAITING (still at or above the floor): one calm line and Update now; nothing else
//   changes, and the update installs when the terminal is next idle;
// - BELOW the floor: a persistent alert naming what this window would not show. The chart in hand
//   stays open to be finished; the window's Rust side refuses to open another or to register
//   (`crate::update::refuse_another_chart`). It restarts once that chart closes or the terminal
//   locks.
//
// A restart is the idle re-lock's own sequence (idle.js): every act in flight settles, the chart
// closes with its queued saves awaited, and the program restarts. But it first ASKS the program
// installed here for its generation: one unchanged, or one this database would refuse, is never
// restarted into — the window says so and keeps the chart.
//
// The poll is its own, once a minute, on the window's one connection (never reopening it); the
// 10 s lock poll (main.js) touches no database. A poll is not a person, so at the lock screen
// the window restarts only after a whole idle period with no touch (idle.js's `lastActivityAt`):
// someone typing the terminal passphrase is never restarted under.
//
// Classic script, loaded after idle.js and leave_chart.js, sharing main.js's `el` and `invoke`,
// notes.js's `signsSettled`, leave_chart.js's `closeChart` and idle.js's `idleLockMs` /
// `lastActivityAt` — all read only when called.
"use strict";

/** How often the window re-reads the database's floor (ms): a minute bounds how late it learns. */
const UPDATE_POLL_MS = 60000;

/** Said when Update now (or an idle restart) finds the same program installed. */
const UPDATE_NOT_INSTALLED =
  "No newer version is installed on this terminal yet — ask whoever installs Cairn here.";
/** Said when the program installed here would be refused, or told to update, by this database. */
const UPDATE_DOES_NOT_FIT = "The version installed here does not fit this node yet.";

/** The last view the poll rendered (`update_state`): its kind decides every restart below. */
let updateView = { kind: "current", reason: null };
/** A restart attempt is running: a second one (a close during an idle restart) does nothing. */
let restarting = false;

/** The alert's sentence: what this window would not show, in the stamp's words when it gave any. */
function belowFloorText(reason) {
  if (!reason) {
    return "This terminal needs its update: it may not show newer records correctly. Finish what you are doing.";
  }
  return "This terminal needs its update: it does not show " + reason + ". Finish what you are doing.";
}

/**
 * Show what the poll learned. Text is set only when it changed, so an assistive technology does
 * not re-announce the alert every minute. It never closes or restarts anything (design §4.5).
 */
function renderUpdate(view) {
  if (view.kind !== updateView.kind) el("update-answer").textContent = "";
  updateView = view;
  const below = view.kind === "below_floor";
  el("update-line").hidden = view.kind === "current";
  el("update-calm").hidden = below;
  el("update-alert").hidden = !below;
  const alert = below ? belowFloorText(view.reason) : "";
  if (el("update-alert").textContent !== alert) el("update-alert").textContent = alert;
}

/** The answer to an attempt that did not restart, beside the button. */
function sayUpdate(text) {
  el("update-answer").textContent = text;
}

/** Ask the database again; at an untouched lock screen with an update to take, take it. */
async function pollUpdate() {
  try {
    renderUpdate(await invoke("update_state"));
  } catch (failure) {
    console.error("the update check could not run", failure);
    return;
  }
  if (
    !el("terminal-lock").hidden &&
    idleLockMs > 0 &&
    Date.now() - lastActivityAt >= idleLockMs &&
    updateView.kind !== "current"
  ) {
    void restartIfFits();
  }
}

/**
 * Restart into the installed program if it fits this database. Asked first; unchanged or not
 * fitting, say so and keep the chart. Otherwise the idle re-lock's sequence, then the restart.
 */
async function restartIfFits() {
  if (restarting) return;
  restarting = true;
  try {
    const installed = await invoke("installed_program");
    if (installed.kind !== "fits") {
      sayUpdate(installed.kind === "same" ? UPDATE_NOT_INSTALLED : UPDATE_DOES_NOT_FIT);
      return;
    }
    await signsSettled();
    await closeChart();
    await invoke("restart_window");
  } catch (failure) {
    console.error("the update could not be installed", failure);
  } finally {
    restarting = false;
  }
}

/** The Update now button: the same restart, now. */
async function updateNow() {
  sayUpdate("");
  await restartIfFits();
}

/** The idle re-lock finished (idle.js): an update waiting, or a floor passed, installs now. */
function updateAfterRelock() {
  if (updateView.kind !== "current") void restartIfFits();
}

/**
 * A chart closed (leave_chart.js). Below the floor that was the chart being finished, so restart.
 * With an update merely waiting, only when the lock is showing: someone at the front door is
 * still working.
 */
function updateAfterClose() {
  if (
    updateView.kind === "below_floor" ||
    (updateView.kind === "waiting" && !el("terminal-lock").hidden)
  ) {
    void restartIfFits();
  }
}

el("update-now").addEventListener("click", () => void updateNow());
// After every classic script ran, as the lock poll's first call (main.js, #782).
document.addEventListener("DOMContentLoaded", () => void pollUpdate());
setInterval(pollUpdate, UPDATE_POLL_MS);
