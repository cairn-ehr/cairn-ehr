// The waiting list (sign-in slice 3c-3c): the person menu in the who's-here bar, its open-note
// buttons, and the picker's per-person counts.
//
// The words are all Rust's (`person_menu_view.rs`): this file draws what `person_menu` answers and
// writes no sentence a clinician reads. Reading the menu signs nothing and asks for no passphrase.
// Classic script, loaded BEFORE whos_here.js (whose `showClaim` calls `refreshMenuCount`), sharing
// main.js's globals (`el`, `cell`, `setMessage`, `invoke`), funnel.js's `enterChart`, `closeChart`,
// `displayedPatient`, `registering`, `failureText`, and notes.js's `signInFlight`, `signsSettled`,
// `noteDraft`, and whos_here.js's `signOutcomeLine`. Only main.js's `el` is used at load (the listeners at the bottom; main.js loads
// first); the rest are used inside functions, at a press or an answer, after every script loaded.
"use strict";

// One counter per kind of read (the worklist.js pattern, #752): an older answer must never draw
// over a newer one, whichever order the node replies in.
let menuRevision = 0;
let pickerRevision = 0;
/** What the next note pane should scroll to ("part:<id>" or "draft:<id>"), or null. */
let pendingMenuFocus = null;
/** The picker's " · N waiting" words by staff id; kept so a search redraw keeps them. */
let pickerSuffix = {};
/** A Sign's, Discard's, sign-off's or review's outcomes (notes.js's `noteSigning`) that went through. */
const SETTLED_WELL = new Set(["signed", "discarded", "signed-off", "reviewed"]);
/** True while one row's open press is still closing a chart and opening another (a double-click). */
let menuOpening = false;

/**
 * Re-read the menu and redraw the bar: on a new claim, a drawn note pane, a closed chart, a tick.
 * False only when THIS read failed (its line then says why); a superseded read answers true.
 */
async function refreshMenuCount() {
  const revision = ++menuRevision;
  let view;
  try {
    view = await invoke("person_menu");
  } catch (failure) {
    if (revision !== menuRevision) return true; // a newer read is on its way, and will say
    // The list drawn last is no longer known to be true: it leaves; the bar's line says why.
    hideMenu();
    setMessage(el("person-menu-status"), failureText(failure));
    return false;
  }
  if (revision !== menuRevision) return true; // a newer read is on its way
  drawMenu(view);
  return true;
}

/** Draw Rust's `PersonMenuView`: nothing for "Nobody", else the count and the three groups. */
function drawMenu(view) {
  const menu = el("person-menu");
  if (view.kind === "Nobody") {
    setMessage(el("person-menu-status"), "");
    hideMenu();
    return;
  }
  setMessage(el("person-menu-status"), "");
  const summary = el("person-menu-count");
  summary.textContent = view.count_words;
  summary.setAttribute("aria-label", view.count_label);
  el("person-menu-body").replaceChildren(
    group(view.own),
    group(view.cosign),
    reviewGroup(view.review),
  );
  menu.hidden = false;
}

/**
 * Take the menu off the bar, closed and emptied: for nobody named, a failed read, and the moment
 * another person is named (whos_here.js's `showClaim`), so the last person's list is never seen
 * under the next person's name while their read is on its way. The status line is not touched.
 */
function hideMenu() {
  const menu = el("person-menu");
  menu.hidden = true;
  menu.open = false;
  el("person-menu-body").replaceChildren();
}

/** A group's rows (or Rust's "nothing here" line) as a list. */
function groupRows(group) {
  if (group.rows.length === 0) return [cell("p", group.empty || "")];
  const list = document.createElement("ul");
  list.append(...group.rows.map(menuRow));
  return [list];
}

function group(view) {
  const section = document.createElement("section");
  section.append(cell("h3", view.heading), ...groupRows(view));
  return section;
}

/**
 * The review group: a nested `<details>`, closed on every draw and never opened by this file —
 * it is not the clinician's to-do list, so it must not nag.
 */
function reviewGroup(view) {
  const details = document.createElement("details");
  details.append(cell("summary", view.heading), ...groupRows(view));
  return details;
}

/** One row: who and what, when, the state, and the button that opens the note. */
function menuRow(row) {
  const li = document.createElement("li");
  // Visible words are Rust's short ones; the accessible name adds which note (a list of identical
  // buttons would otherwise be unusable by ear), so the visible words stay inside the name.
  const open = cell("button", row.open_words, { type: "button", "aria-label": row.open_label });
  li.append(cell("p", row.patient), cell("p", row.what + " · " + row.when), cell("p", row.state), open);
  open.addEventListener("click", () => void openMenuRow(row));
  return li;
}

/**
 * Open a menu row's note: the queued saves land first (a sign in flight settles, then `closeChart` awaits the
 * saves), THEN the node re-reads the menu and opens the chart only if a row still names it. A sign in
 * flight that did not go through keeps its chart open instead.
 */
async function openMenuRow(row) {
  if (registering) return; // as funnel.js's openChart: registration's own enterChart must not be replaced
  if (menuOpening) return; // a second press would interleave two opens with one close
  menuOpening = true;
  try {
    setMessage(el("person-menu-status"), "");
    const outcome = signInFlight() ? await signsSettled() : null;
    if (outcome && !SETTLED_WELL.has(outcome)) {
      // It did not go through (or is not known to have): stay on its chart. `closeChart` would
      // clear the note pane, the one place that says how it went (whos_here.js's switch keeps it
      // too); the bar's line says it in the switch's words.
      setMessage(el("person-menu-status"), signOutcomeLine(outcome));
      return;
    }
    if (displayedPatient) await closeChart();
    try {
      const header = await invoke("person_menu_open", { patientId: row.patient_id });
      pendingMenuFocus = row.focus;
      el("person-menu").open = false;
      enterChart(header);
    } catch (refusal) {
      // Rust's sentence (a refusal is a string). The list is read again FIRST: a drawn menu clears
      // the status line, so saying the refusal before the re-read lands would erase it. A failed
      // re-read keeps its own line: the refusal would say the list WAS read again.
      if (await refreshMenuCount()) setMessage(el("person-menu-status"), String(refusal));
      // The clicked button is gone with the redraw: focus the line that says why, not <body>.
      el("person-menu-status").tabIndex = -1;
      el("person-menu-status").focus();
    }
  } finally {
    menuOpening = false; // a refusal or a throw must never leave every button dead
  }
}

/** Return and clear the pending focus target (notes.js's `renderNotes` consumes it once). */
function takeMenuFocus() {
  const focus = pendingMenuFocus;
  pendingMenuFocus = null;
  return focus;
}

/** Scroll to and focus the note a menu row named, once the note pane has drawn. */
function focusMenuTarget(focus) {
  if (!focus) return;
  const [kind, id] = focus.split(/:(.*)/s);
  // A medication list: the chart's medication pane, when the chart shows one (else the chart opens as is).
  if (kind === "meds") {
    const pane = el("med-chart");
    if (pane.hidden) {
      pendingMenuFocus = focus; // drawn later: med_list.js asks again once its list is drawn
      return;
    }
    pane.tabIndex = -1;
    pane.scrollIntoView();
    pane.focus();
    return;
  }
  let target = document.querySelector(
    kind === "part" ? `#note-list [data-parts~="${id}"]` : `#note-drafts [data-draft="${id}"]`,
  );
  // A draft this terminal holds is open in the editor, not listed.
  if (!target && kind === "draft" && noteDraft && noteDraft.draftId === id) target = el("note-text");
  if (!target) return;
  target.tabIndex = -1;
  target.scrollIntoView();
  target.focus();
}

/** Remember the picker's counts and redraw it (Rust worded each suffix; "" for none). */
function fillPickerCounts(entries) {
  pickerSuffix = Object.fromEntries(entries.map((entry) => [entry.staff_id, entry.suffix]));
  drawPicker();
}

/** Forget the last opening's counts, as the picker opens. */
function resetPickerCounts() {
  pickerRevision += 1; // an answer still on its way belongs to the last opening
  pickerSuffix = {};
}

/** Read everybody's counts for the picker; the list is already drawn, the suffixes join it. */
async function loadPickerCounts() {
  const revision = ++pickerRevision;
  try {
    const entries = await invoke("whos_here_counts");
    if (revision !== pickerRevision) return;
    fillPickerCounts(entries);
  } catch (failure) {
    // Entries stay without a suffix; Rust's sentence says why.
    if (revision !== pickerRevision) return;
    setMessage(el("whos-here-status"), failureText(failure));
  }
}

// Opening the menu re-reads it: what it showed may be minutes old.
el("person-menu").addEventListener("toggle", () => {
  if (el("person-menu").open) void refreshMenuCount();
});
// The quarantine commit moves drafts into the list by itself, so the count is re-read while
// nobody acts. Only THIS tick is skipped while the menu is open (a redraw would pull focus from a
// button); readNotes and closeChart still re-read, and so redraw, an open menu.
setInterval(() => {
  if (!el("person-menu").open) void refreshMenuCount();
}, 60_000);
