// Leaving a chart (#1130, #1137): the one way a chart leaves the screen, and the window-wide set
// of acts in flight it waits for. The paper counterpart: the folder goes back in the rack only
// once the pen is off the page — so an act's answer is said on the chart it acted on, never under
// the next patient's header, and never dropped because the chart it was for has gone.
//
// Every chart-leaving path goes through `closeChart`: "Find another patient" (below), a who's-here
// switch (whos_here.js), the idle re-lock (idle.js) and the person menu (person_menu.js). It first
// waits for every act in flight — notes.js's `signsSettled`: a note's Sign, Discard, sign-off or
// review, the medication list's Sign, a stat dose and a version's settle (their own flags), and
// every `chartAct` here. Since nothing else takes a chart off the screen, no answer handler needs
// to ask "is this still my chart?" (`leave_chart_webview_tests.rs` pins the choke point).
//
// Classic script, loaded after funnel.js, sharing main.js's `el`, `invoke`, `say`, `clearChart`,
// `failureText` and `displayedPatient` (WRITTEN here, as in funnel.js's `enterChart`), funnel.js's
// `resetFrontDoor`, notes.js's `signsSettled`/`signInFlight`, notes_save.js's `notesSaved`,
// med_list.js's `medListSaved`, person_menu.js's `SETTLED_WELL`/`refreshMenuCount`,
// worklist.js's `refreshTray` and link.js's / unlink.js's panel closers — all read when called.
"use strict";

/**
 * Every chart act in flight that keeps no flag of its own (notes.js keeps the note's and the
 * medication list's): sign-off and Stop (main.js), Withhold and Release (holds.js), every allergy
 * press (allergies.js). Each entry is a promise of the act's words, settled, never rejecting.
 */
const actsInFlight = new Set();

/** Said for a press this window itself failed in: whether it recorded anything is not known here. */
const CHART_ACT_FAULT = "This window failed during that press — open the chart again to see what was recorded.";

/** The close in progress, or null: an open waits for it (#1137), and a second close joins it. */
let closing = null;

/**
 * Run `run` — a press's whole run: its invoke, the words it says, the re-read after — as one act
 * in flight on the chart on screen. One strip credential serves one act at a time (#1002), so a
 * press made while another act is in flight does nothing and answers `false` (for Withhold and
 * Release, "not spent": their button comes back). Once `run` settles, the act's entry settles as
 * the words it left in element `saidIn` — `{ words }`, which a who's-here switch repeats in its
 * picker after taking the chart away — or null when it said nothing there. Resolves `run`'s own
 * value, so a caller reads it exactly as before.
 */
function chartAct(run, saidIn) {
  if (signInFlight()) return Promise.resolve(false);
  const done = run();
  const settled = done.then(
    () => {
      const said = el(saidIn).textContent;
      return said ? { words: said } : null;
    },
    (fault) => {
      console.error(fault);
      return { words: CHART_ACT_FAULT };
    },
  );
  actsInFlight.add(settled);
  // Registered before any waiter's, so the set is empty again by the time a wait for it resumes.
  void settled.then(() => actsInFlight.delete(settled));
  return done;
}

/**
 * Take the chart off screen. A close already in progress is joined, never run twice: two would
 * interleave their queued saves, and the second's `close_chart` could close the NEXT chart on the
 * backend (#1137).
 */
function closeChart() {
  closing ??= leaveChart().finally(() => {
    closing = null;
  });
  return closing;
}

async function leaveChart() {
  // #1130: every act in flight on this chart settles FIRST, so its answer is said here and its
  // re-read is of this chart. Clearing `displayedPatient` before it would make the answer think
  // its chart was left, and say it nowhere — or on the next patient's chart.
  await signsSettled();
  displayedPatient = null;
  clearChart();
  // As in `enterChart`: the panel must not survive the chart under it. `false` — focus heads for
  // the front door, not `#same-person`, which is hidden with the rest of `#chart-view`.
  if (typeof closeLinkPanel === "function" && !el("link-panel").hidden) closeLinkPanel(false);
  if (typeof closeUnlinkPanel === "function" && !el("unlink-panel").hidden) closeUnlinkPanel(false);
  say("");
  // Every queued note save lands while the node still shows this chart (sent first is not enough).
  if (typeof notesSaved === "function") await notesSaved();
  await medListSaved(); // and every medication line's (med_list.js)
  try {
    await invoke("close_chart");
  } catch (failure) {
    // Stay on the (now empty) chart view and SAY so, rather than leaving a blank view with a
    // "Loading…" button and no word. Every chart command is already refused: nothing is displayed.
    say("Could not return to the front door: " + failureText(failure));
    return;
  }
  el("chart-view").hidden = true;
  el("front-door").hidden = false;
  resetFrontDoor();
  el("browse-name").focus();
  void refreshTray();
  void refreshMenuCount(); // person_menu.js: a saved draft may have moved the count
}

/**
 * "Find another patient". An act still in flight settles first; one that did not go through — or
 * is not known to have — keeps its chart on screen, where it said so, as the person menu does
 * (#1130): leaving would clear the only place that says it. One that went through lets it go.
 */
async function findAnotherPatient() {
  const outcome = signInFlight() ? await signsSettled() : null;
  if (outcome && !SETTLED_WELL.has(outcome)) return;
  await closeChart();
}

el("close-chart").addEventListener("click", () => void findAnotherPatient());
