// The webview renders and decides nothing. Every clinical display question was already
// answered in Rust under `cargo test` (cairn-gui-tab-medications); re-deriving any of it
// here would put an untested second answer on screen.
//
// WHY PLAIN JAVASCRIPT AND NO BUNDLER. `withGlobalTauri` puts `invoke` on `window`, so this
// file is the whole frontend: no package.json, no node_modules, no build step, nothing to
// audit for licence compatibility, and nothing between what a reviewer reads and what the
// window runs. The cost is that this file is not type-checked; it is kept small and
// logic-free to pay for that, and the JS-toolchain decision stays open (issue #332).
"use strict";

const { invoke } = window.__TAURI__.core;

/** How often the window re-checks whether the held key has re-locked (ms). */
const LOCK_POLL_MS = 10_000;

/**
 * The chart id this window is DISPLAYING, set by funnel.js when a chart opens and cleared when
 * it closes. Every chart command sends it, and the backend refuses unless it is the open chart:
 * a sign-off must sign what the clinician was looking at, never whatever happens to be open by
 * the time the command runs (`AppState::displayed_patient`).
 */
let displayedPatient = null;

/**
 * The chart whose medication list is actually DRAWN in the table — set by `render`, cleared by
 * `clearChart`. Sign-off and cease send THIS, not `displayedPatient`: they act on the list the
 * clinician reviewed. If a late read for chart A were ever drawn under chart B's header (the
 * guard in `refresh` is what prevents that), the command would name A, and the backend — with B
 * open — would refuse it rather than sign B on the strength of A's list.
 */
let renderedPatient = null;

function el(id) {
  return document.getElementById(id);
}

/** A cell with text. `attrs` carries the semantics (e.g. scope="row"). */
function cell(tag, text, attrs) {
  const node = document.createElement(tag);
  node.textContent = text;
  for (const [key, value] of Object.entries(attrs || {})) {
    node.setAttribute(key, value);
  }
  return node;
}

/** Show or hide a paragraph, keeping `hidden` and its text in step. */
function setMessage(node, text) {
  const present = Boolean(text);
  node.textContent = present ? text : "";
  node.hidden = !present;
  return present;
}

function renderWarnings(view) {
  const incomplete = setMessage(el("chart-incomplete"), view.missing_message);
  const withheld = setMessage(el("chart-withheld"), view.withheld_message);
  // The section itself is hidden when both are absent, so a healthy chart carries no
  // "Warnings about this chart" heading for a screen reader to walk into.
  el("chart-warnings").hidden = !(incomplete || withheld);
}

function renderRow(row) {
  const tr = document.createElement("tr");
  // Marked in the DOM, not only by colour: colour alone is invisible to a screen reader
  // and to a colour-blind clinician. The text says it too — see the signature cell.
  if (row.will_be_signed) tr.setAttribute("data-will-sign", "true");
  if (row.status_label === "ceased") tr.setAttribute("data-ceased", "true");

  tr.append(
    cell("th", row.primary, { scope: "row" }),
    cell("td", row.dose),
    cell("td", row.status_label),
    cell(
      "td",
      row.will_be_signed
        ? row.vouch_label + " — will be signed"
        : row.vouch_label,
    ),
  );

  const action = document.createElement("td");
  if (row.can_cease) {
    // Stopping a drug takes a reason, because a cancellation needs an owner AND a rationale
    // (ADR-0060). The field is inline rather than behind a dialog: a confirmation box is
    // explicitly not an acceptable safety mechanism in this project, and typing why is the
    // same act as writing it on the paper chart.
    const reason = document.createElement("input");
    reason.type = "text";
    reason.id = "reason-" + row.group_id;
    reason.setAttribute("aria-label", "Reason for stopping " + row.primary);
    reason.placeholder = "reason";

    const stop = document.createElement("button");
    stop.type = "button";
    stop.id = "cease-" + row.group_id;
    stop.textContent = "Stop";
    // "Stop" alone is ambiguous when a screen reader reads the buttons out of table
    // context, so the accessible name names the drug.
    stop.setAttribute("aria-label", "Stop " + row.primary);
    stop.addEventListener("click", () => cease(row.group_id, reason.value));

    action.append(reason, stop);
  }
  tr.append(action);

  const rows = [tr];
  for (const flag of row.flags) {
    // A flag is a claim about THIS line, so it rides directly under it, spanning the row.
    const note = document.createElement("tr");
    note.setAttribute("data-flag", "true");
    const td = document.createElement("td");
    td.colSpan = 5;
    td.textContent = "! " + flag;
    note.append(td);
    rows.push(note);
  }
  return rows;
}

function render(view, patient) {
  renderedPatient = patient;
  renderWarnings(view);

  const body = el("med-rows");
  body.replaceChildren();
  for (const row of view.rows) {
    body.append(...renderRow(row));
  }

  const button = el("sign-off");
  button.disabled = !view.sign_off_enabled;
  // The count is the number of THREADS, which a reconciled group can make larger than the
  // number of visible rows. Saying the real number is the honest thing to sign.
  button.textContent = view.sign_off_enabled
    ? "Sign off " + view.sign_off_count + " unsigned medication(s)"
    : "Nothing to sign off";

  setMessage(el("empty-message"), view.empty_message);
}

/**
 * Empty the chart view: no rows, no warnings, and a sign-off button that cannot fire. Called
 * whenever the chart changes, so one patient's list is never on screen under another patient's
 * header — not even for the moment a read is in flight, and not after a read that failed.
 */
function clearChart() {
  renderedPatient = null;
  el("med-rows").replaceChildren();
  setMessage(el("chart-incomplete"), "");
  setMessage(el("chart-withheld"), "");
  el("chart-warnings").hidden = true;
  setMessage(el("empty-message"), "");
  const button = el("sign-off");
  button.disabled = true;
  button.textContent = "Loading…";
}

async function refresh() {
  const patient = displayedPatient;
  if (patient === null) return;
  try {
    const view = await invoke("med_list", { patientId: patient });
    // A read for a chart the clinician has since left is dropped, never rendered.
    if (patient !== displayedPatient) return;
    render(view, patient);
  } catch (e) {
    if (patient !== displayedPatient) return;
    say("Could not read the chart: " + e);
  }
}

/** Report an outcome. Never a dialog; never silence. */
function say(text) {
  el("outcome").textContent = text;
}

/**
 * Render a completed act honestly: what happened, and what did not.
 * ADR-0060 decision 2 — partial completion is reported, never implied.
 */
function reportSignOff(report) {
  const parts = ["Signed " + report.signed + " medication thread(s)."];
  if (report.failed.length > 0) {
    parts.push(
      report.failed.length +
        " line(s) could NOT be signed and were left unsigned: " +
        report.failed.join("; "),
    );
  }
  if (report.withheld_message) parts.push(report.withheld_message);
  if (report.missing_message) parts.push(report.missing_message);
  say(parts.join(" "));
}

async function signOff() {
  if (renderedPatient === null) {
    say("No medication list is on screen to sign off.");
    return;
  }
  try {
    reportSignOff(await invoke("sign_off", { patientId: renderedPatient }));
  } catch (e) {
    say("Sign-off failed: " + e);
  }
  await refresh();
}

async function cease(groupId, reason) {
  try {
    const report = await invoke("cease", {
      groupId: groupId,
      reason: reason,
      patientId: renderedPatient,
    });
    let text = "Stopped " + report.ceased + " thread(s) of this drug.";
    if (report.failed.length > 0) {
      text +=
        " " +
        report.failed.length +
        " thread(s) were NOT stopped and are still active: " +
        report.failed.join("; ");
    }
    say(text);
  } catch (e) {
    say("Could not stop this drug: " + e);
  }
  await refresh();
}

function renderLockState(lock) {
  if (lock.mock) {
    el("lock-state").textContent =
      "Fixture data — this window is showing a sample chart and cannot write to a record.";
    el("unlock-form").hidden = true;
    return;
  }
  el("lock-state").textContent = lock.unlocked
    ? "Signing as " + lock.kid + " — your key is unlocked."
    : "Your signing key is locked. Unlock it to sign off or stop a drug.";
  el("unlock-form").hidden = Boolean(lock.unlocked);
}

async function pollLock() {
  try {
    renderLockState(await invoke("lock_state"));
  } catch (e) {
    el("lock-state").textContent = "Could not check the signing key: " + e;
  }
}

el("sign-off").addEventListener("click", signOff);

el("unlock-form").addEventListener("submit", async (event) => {
  event.preventDefault();
  const field = el("passphrase");
  try {
    renderLockState(await invoke("unlock", { passphrase: field.value }));
    say("Signing key unlocked.");
  } catch (e) {
    say(String(e));
  } finally {
    // Clear the field either way: a passphrase left in a DOM node outlives the failure
    // that put it there.
    field.value = "";
  }
});

// No `refresh()` here: whether a chart is showing at all is the front door's decision
// (funnel.js), which calls `refresh()` when it opens one. `refresh` and `say` stay globals —
// both files are classic scripts sharing one scope, loaded main.js first.
void pollLock();
// The key re-locks on a timer in the backend; the window must not learn about it only when
// a signature is refused (state is ambient, never modal).
//
// This poll deliberately does NOT extend the session: `lock_state` reads the lock without
// counting as activity, because nobody is at the keyboard when a timer fires. It used to go
// through the touching accessor, which meant the window reset its own idle clock every 10
// seconds and the key never re-locked at all. Only a clinical act (sign-off, cease) counts.
setInterval(pollLock, LOCK_POLL_MS);
