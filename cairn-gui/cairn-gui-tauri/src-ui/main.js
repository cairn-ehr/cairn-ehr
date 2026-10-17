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

/**
 * The chart SET the drawn list was read over (the view's `charts`, an array of chart ids) — set
 * by `render`, cleared by `clearChart`. A chart linked to others reads as one combined list
 * (ADR-0076), and sign-off and cease send this set back beside `renderedPatient`: if a link or
 * unlink changed the set while the list was on screen, the backend refuses rather than signing a
 * list nobody reviewed.
 */
let renderedCharts = null;

/**
 * Chart-read generations (#800). "Is it for the chart on screen?" alone let an older answer land
 * over a newer one: two re-reads of the SAME chart can overlap, and a clinician can go A → front
 * door → A. `chartReadRevision` is the newest read STARTED (`refresh` bumps it before it sends);
 * `drawnRevision` the newest DRAWN. `clearChart` (a chart change) and chart_stale.js's `holdOwnAct`
 * (an own act that moved the held set) set both to a fresh floor, outdating every read in flight.
 */
let chartReadRevision = 0;
let drawnRevision = 0;

/**
 * May an answer to the read that took `revision` for `patient` be drawn? While that chart is on
 * screen and nothing NEWER has been drawn. Not "nothing newer started": the older of two
 * overlapping re-reads usually lands first, and dropping it kept an even older list up if the
 * newer then failed (PR #802 review). Asked by `refresh` and duplicates.js's `redrawBannerAlone`.
 */
function mayDraw(patient, revision) {
  return patient === displayedPatient && revision >= drawnRevision;
}

/**
 * May a FAILED read for `patient` say so? Only if it is the newest started: a newer read speaks
 * for the chart now, and an older read's "Could not read the chart" must not pre-empt it.
 */
function isLatestRead(patient, revision) {
  return patient === displayedPatient && revision === chartReadRevision;
}

/**
 * Said in the chart's warnings when a list was READ but could not be DRAWN (#801): a fault in this
 * window (a payload it does not understand), not a failed read. Nothing of the list stays up.
 */
const LIST_NOT_SHOWN =
  "The medication list was read but could not be shown, so none of it is on screen and " +
  "nothing on this list can be signed off or stopped here. This is a fault in this window: ";

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

/**
 * The linked charts' own identity lines, under the identity header. Label and list are hidden
 * together when there are none: a chart linked to nothing must not announce a link.
 */
function renderMembers(members) {
  const list = el("linked-charts");
  list.replaceChildren();
  for (const member of members) {
    list.append(cell("li", member.text));
  }
  const linked = members.length > 0;
  list.hidden = !linked;
  el("linked-charts-label").hidden = !linked;
}

/**
 * The links joining this record's charts, one line each, each with ONE door into the review
 * panel (R2b-2, R6). The button's label comes from Rust (`recordLink.button`: "Review this
 * link…" or "Review this judgement…"); `recordLink.kind` says which line it is, and the panel
 * offers the verdicts that fit. The accessible name carries the link's own text, so a screen
 * reader hears WHICH line it opens, not ten identical buttons.
 */
function renderLinks(links) {
  const list = el("record-links");
  list.replaceChildren(
    ...links.map((recordLink) => {
      const li = cell("li", recordLink.text + " ");
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = recordLink.button;
      b.setAttribute("aria-label", recordLink.button + " " + recordLink.text);
      b.dataset.low = recordLink.low;
      b.dataset.high = recordLink.high;
      b.dataset.kind = recordLink.kind;
      // unlink.js loads after this file and may not be present (fixture-only builds).
      b.addEventListener("click", () => {
        if (typeof compareLinked === "function") compareLinked(recordLink.low, recordLink.high, recordLink.kind, recordLink.attested);
      });
      li.append(b);
      return li;
    }),
  );
  const any = links.length > 0;
  list.hidden = !any;
  el("record-links-label").hidden = !any;
}

function renderRow(row) {
  const tr = document.createElement("tr");
  // Marked in the DOM, not only by colour: colour alone is invisible to a screen reader
  // and to a colour-blind clinician. The text says it too — see the signature cell.
  if (row.will_be_signed) tr.setAttribute("data-will-sign", "true");
  if (row.status_label === "ceased") tr.setAttribute("data-ceased", "true");

  tr.append(
    // On a combined list the source chart is part of the drug's name, so a screen reader hears
    // WHERE it was recorded in the same utterance as WHAT it is. `source` is present only then.
    cell(
      "th",
      row.source ? row.primary + " — recorded on chart(s) " + row.source : row.primary,
      { scope: "row" },
    ),
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

/**
 * Draw a `med_list` answer in two parts, each under its own guard, so a fault drawing one never
 * leaves the other partial nor is misreported by `refresh` as a failed READ: the list (`drawList`;
 * #801), then the banner (`drawDuplicates` says its fault on the banner). The banner comes second
 * because it reads `renderedPatient` to decide whether Review is offered: only over a drawn list.
 */
function render(pane, patient) {
  try {
    drawList(pane, patient);
  } catch (failure) {
    showUnshownList(failure);
  }
  drawDuplicates(pane.duplicates); // a fault drawing it is said on the banner, never thrown here
}

/** The list half of `render`: the drawn chart and set, linked charts, links, warnings, rows. */
function drawList(pane, patient) {
  const view = pane.list;
  renderedPatient = patient;
  renderedCharts = view.charts;
  holdCharts(view.charts); // what the clinician now holds (chart_stale.js)
  setMessage(el("chart-unshown"), ""); // a list that draws takes down a previous fault's warning
  setMessage(el("chart-stale"), ""); // and an out-of-date list's (chart_stale.js, #797)
  renderMembers(pane.members);
  setMessage(el("linked-charts-error"), pane.members_error);
  renderLinks(pane.links);
  setMessage(el("record-links-error"), pane.links_error);
  renderWarnings(view);

  const body = el("med-rows");
  const typed = typedReasons();
  body.replaceChildren();
  for (const row of view.rows) {
    body.append(...renderRow(row));
  }
  restoreReasons(typed);

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
 * A list that was read but could not be drawn (#801). What `drawList` drew before the fault may
 * be only part of the list, so ALL of it is taken down: a partial list under a live "Sign off N"
 * would pass part of the list off as all of it — an untruth that looks complete (principle 4;
 * ADR-0060: partial completion is reported, never implied). The clinician is told in the warnings,
 * above the table — never on the outcome line, which may be saying how a signed act went.
 */
function showUnshownList(failure) {
  console.error(failure); // the stack, for whoever debugs it; the warning says the rest
  // No list on screen, so nothing may compare or sign over one (PR #802 review): the banner,
  // drawn next, offers no Review, and `compare` refuses — as for a failed or in-flight read.
  renderedPatient = null;
  renderedCharts = null;
  holdCharts(null);
  clearDrawnList();
  setMessage(el("chart-unshown"), LIST_NOT_SHOWN + String(failure));
  el("chart-warnings").hidden = false;
  const button = el("sign-off");
  button.disabled = true;
  button.textContent = "Nothing to sign off — the list could not be shown";
}

/**
 * Take down everything `drawList` draws but sign-off, which is left to the caller. Shared by
 * `clearChart` and `showUnshownList`: one list of what `drawList` draws, not two to drift apart.
 */
function clearDrawnList() {
  renderMembers([]);
  setMessage(el("linked-charts-error"), "");
  renderLinks([]);
  setMessage(el("record-links-error"), "");
  el("med-rows").replaceChildren();
  setMessage(el("chart-unshown"), "");
  setMessage(el("chart-stale"), "");
  setMessage(el("chart-incomplete"), "");
  setMessage(el("chart-withheld"), "");
  el("chart-warnings").hidden = true;
  setMessage(el("empty-message"), "");
}

/**
 * Empty the chart view: no rows, no warnings, and a sign-off button that cannot fire. Called
 * whenever the chart changes, so one patient's list is never on screen under another patient's
 * header — not even for the moment a read is in flight, and not after a read that failed. Every
 * read still in flight is outdated too (#800): the fresh floor fails both `mayDraw` and
 * `isLatestRead` for any of them.
 */
function clearChart() {
  drawnRevision = ++chartReadRevision;
  renderedPatient = null;
  renderedCharts = null;
  holdCharts(null);
  clearDuplicates(); // duplicates.js loads right after this file, before funnel.js's boot
  clearDrawnList();
  const button = el("sign-off");
  button.disabled = true;
  button.textContent = "Loading…";
}

/**
 * Re-read the open chart's medication list.
 *
 * `lead` is the outcome sentence of the act that prompted the re-read (a sign-off, a cease, a
 * link), when there was one. A failed re-read is reported AFTER it, never instead of it: the
 * outcome of a signed act is never overwritten by what happened next (PR #707 review — a
 * "Linked — …" sentence was replaced by "Could not read the chart"). #797: `behind` = the list is
 * known behind the record (a stale refusal prompted it); changes are named against the held set.
 */
async function refresh(lead, behind) {
  const patient = displayedPatient;
  if (patient === null) return;
  if (behind) knowListBehind(); // window state: whichever read fails last marks the list
  // Taken BEFORE the read is sent: from here on, any older read still in flight is outdated.
  const revision = ++chartReadRevision;
  try {
    const answer = await invoke("med_list", { patientId: patient, seenCharts: heldCharts });
    // A read for a chart the clinician has since left, or older than what is drawn, is dropped,
    // never rendered (#800).
    if (!mayDraw(patient, revision)) return;
    drawnRevision = revision;
    const marked = listMarkedStale(); // before `render`, whose `drawList` takes the mark down
    render(answer, patient);
    // Only over a list that DREW: a drawing fault took it down (`showUnshownList`), nothing to review.
    if (renderedPatient === patient) sayAfterDraw(lead, revision, marked, answer);
  } catch (e) {
    // A failure is said only by the newest read started: that read speaks for the chart now.
    if (!isLatestRead(patient, revision)) return;
    sayReadFailed(lead, e); // chart_stale.js: remembered, so "Read this chart again" can retry it
    // A failed FIRST read (`clearChart` left no list drawn): sign-off must stop saying "Loading…",
    // or the clinician waits for a read that is not happening (PR #798 review). It stays disabled.
    // A failed RE-read keeps the list it already drew, and that list's own button.
    if (renderedPatient === null) el("sign-off").textContent = "Nothing to sign off — no list read";
    // After a stale refusal that list is known out of date: marked BEFORE the await below,
    // after which a newer read may have drawn a fresh list.
    if (listKnownBehind() && renderedPatient !== null) markListStale();
    // The banner needs only the chart set, so it is read on its own (#745): a failed list read
    // must not take the duplicate warning down, nor leave a just-judged pair up with a live Review.
    await redrawBannerAlone(patient); // duplicates.js, loaded after this file
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
  if (report.charts_message) parts.push(report.charts_message);
  say(parts.join(" "));
}

async function signOff() {
  if (renderedPatient === null) {
    say("No medication list is on screen to sign off.");
    return;
  }
  try {
    reportSignOff(
      await invoke("sign_off", { patientId: renderedPatient, charts: renderedCharts }),
    );
  } catch (failure) {
    // The backend's ErrorView ({text, retry}), not a string: `failureText` reads its text.
    say("Sign-off failed: " + failureText(failure));
    await refresh(el("outcome").textContent, behindTheRecord(failure));
    return;
  }
  await refresh(el("outcome").textContent);
}

async function cease(groupId, reason) {
  try {
    const report = await invoke("cease", {
      groupId: groupId,
      reason: reason,
      patientId: renderedPatient,
      charts: renderedCharts,
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
  } catch (failure) {
    say("Could not stop this drug: " + failureText(failure));
    await refresh(el("outcome").textContent, behindTheRecord(failure));
    return;
  }
  await refresh(el("outcome").textContent);
}

function renderLockState(lock) {
  if (lock.mock) {
    el("lock-state").textContent =
      "Fixture data — this window is showing a sample chart and cannot write to a record.";
    el("unlock-form").hidden = true;
    // Fixture mode holds no signing key at all (link.js's `linkCompared` refuses to write in
    // it regardless), so the Link button reads exactly as a locked one does — never as if a
    // key were unlocked when there is none to unlock.
    if (typeof updateLinkLock === "function") updateLinkLock(false);
    return;
  }
  el("lock-state").textContent = lock.unlocked
    ? "Signing as " + lock.kid + " — your key is unlocked."
    : "Your signing key is locked. Unlock it to sign off or stop a drug.";
  el("unlock-form").hidden = Boolean(lock.unlocked);
  // `link.js` loads after this file and may not always be present (a future front-end could
  // reuse `main.js` without it) — guarded rather than assumed, the same as `unlock-form`.
  if (typeof updateLinkLock === "function") updateLinkLock(lock.unlocked);
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
//
// The FIRST lock poll waits for DOMContentLoaded (#782). Started here, while this file is still
// loading, its answer could land before link.js (loaded after this file) defines
// `updateLinkLock`: `renderLockState` then skipped it, and the verdict buttons showed link.js's
// default "locked" state until the next poll, up to LOCK_POLL_MS later. DOMContentLoaded fires
// only after every classic, synchronous script has run, so every reader of the lock exists by
// then (pinned in `link/lock_hint_tests.rs`, which also keeps every script tag classic).
document.addEventListener("DOMContentLoaded", () => void pollLock());
// The key re-locks on a timer in the backend; the window must not learn about it only when
// a signature is refused (state is ambient, never modal).
//
// This poll deliberately does NOT extend the session: `lock_state` reads the lock without
// counting as activity, because nobody is at the keyboard when a timer fires. It used to go
// through the touching accessor, which meant the window reset its own idle clock every 10
// seconds and the key never re-locked at all. Only a clinical act (sign-off, cease) counts.
setInterval(pollLock, LOCK_POLL_MS);
