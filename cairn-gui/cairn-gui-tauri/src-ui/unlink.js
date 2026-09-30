// "Not the same person" (repair path R2b-2). Renders and decides nothing: every sentence comes
// from Rust, every check from the backend. Classic script, loaded after main.js, funnel.js and
// link.js, sharing their scope: `el`, `cell`, `setMessage`, `say`, `refresh`, `invoke`,
// `displayedPatient`, `renderedPatient`, `renderedCharts` (main.js), `failureText` (funnel.js),
// `sayAnywhere`, `updateLinkLock`, `keyUnlocked`, `closeLinkPanel` (link.js). It mirrors link.js's rules — read its header — and is a SEPARATE
// panel: one panel with two verbs could show one verb's button over the other's comparison.
//
// EVERY MESSAGE GOES THROUGH `setMessage`: `#unlink-status` is hidden whenever empty and
// `[hidden]` wins over everything, so a bare `.textContent = …` would never reach the screen.
"use strict";

/** Bumped on every open, click on a link, or close; an answer for an older token is dropped. */
let unlinkToken = 0;
/**
 * What the panel compared, as the backend read it: Unlink sends exactly these back, never the
 * window's current `renderedCharts`, so the judgement names what the clinician looked at.
 * `null` until a WHOLE comparison (`can_unlink`) has landed.
 */
let unlinkCompared_ = null; // { patientId, charts, low, high }
/** The link (low/high) whose button opened the panel: Close returns focus to that button. */
let unlinkOpener = null;

/** Empty every part of the panel that a fresh comparison must replace. */
function clearUnlinkComparison() {
  for (const id of ["unlink-findings", "unlink-table", "unlink-confirm", "unlink-problems"]) {
    el(id).hidden = true;
  }
  el("unlink-findings").replaceChildren();
  el("unlink-table").tHead.replaceChildren();
  el("unlink-table").tBodies[0].replaceChildren();
  setMessage(el("unlink-status"), "");
}

/**
 * Open the panel for the link between charts `low` and `high`, read both charts and draw the
 * comparison: problems, then findings, then the table (DOM order is clinical, already right in
 * the markup). Focus goes to the heading so a screen reader is told what appeared.
 */
async function compareLinked(low, high) {
  const token = ++unlinkToken;
  // Captured NOW, synchronously: the chart this comparison is about.
  const patientId = renderedPatient;
  const charts = renderedCharts;
  unlinkCompared_ = null; // until a whole answer lands, there is nothing an Unlink could be signed over
  unlinkOpener = { low, high };
  // The two panels are mutually exclusive: never one verb's button over the other's comparison.
  if (typeof closeLinkPanel === "function" && !el("link-panel").hidden) closeLinkPanel(false);
  clearUnlinkComparison();
  el("unlink-panel").hidden = false;
  el("unlink-heading").focus();
  try {
    const view = await invoke("compare_linked", { patientId, charts, low, high });
    if (token !== unlinkToken) return; // a newer click, or a close, came after this one
    renderUnlinkComparison(view);
    unlinkCompared_ = view.can_unlink
      ? { patientId, charts: view.charts, low: view.low, high: view.high }
      : null;
  } catch (failure) {
    if (token !== unlinkToken) return;
    setMessage(el("unlink-status"), failureText(failure));
  }
}

/** Draw a comparison of the link's two charts: problems, findings, a two-column table. */
function renderUnlinkComparison(view) {
  const problems = el("unlink-problems");
  setMessage(problems, view.problems.join(" "));

  const findings = el("unlink-findings");
  findings.replaceChildren(...view.findings.map((f) => cell("li", f)));
  findings.hidden = view.findings.length === 0; // never a "no conflicts" line

  const table = el("unlink-table");
  const heads = document.createElement("tr");
  heads.append(cell("td", ""));
  for (const col of view.columns) heads.append(cell("th", col.heading, { scope: "col" }));
  table.tHead.replaceChildren(heads);
  table.tBodies[0].replaceChildren(
    ...view.rows.map((row) => {
      const tr = document.createElement("tr");
      tr.append(cell("th", row.label, { scope: "row" }));
      for (const text of row.cells) tr.append(cell("td", text));
      return tr;
    }),
  );
  table.hidden = view.columns.length === 0;

  // No Unlink button unless the whole comparison was read: a judgement needs the whole picture.
  el("unlink-confirm").hidden = !view.can_unlink;
  updateLinkLock(keyUnlocked);
}

/**
 * Hide the panel and forget what it held. `returnFocus` only when the clinician acted ON the
 * panel (Close, Escape); funnel.js calls it with `false` when the chart underneath changes,
 * because it is about to move focus itself.
 */
function closeUnlinkPanel(returnFocus) {
  el("unlink-panel").hidden = true;
  unlinkToken += 1;
  unlinkCompared_ = null;
  clearUnlinkComparison();
  const opener = unlinkOpener;
  unlinkOpener = null;
  if (returnFocus) {
    // The button that opened the panel, if it still exists; otherwise the first link's.
    const buttons = [...el("record-links").querySelectorAll("button")];
    const own = opener && buttons.find((b) => b.dataset.low === opener.low && b.dataset.high === opener.high);
    const target = own || buttons[0];
    if (target) target.focus();
  }
}

/** Same rule as `linkAnswerPlace`: where an unlink answer is reported, decided when it LANDS. */
function unlinkAnswerPlace(sent, token) {
  if (displayedPatient !== sent.patientId) return "elsewhere";
  if (token !== unlinkToken || el("unlink-panel").hidden) return "chart";
  return "panel";
}

/** Whether an unlink that changed the record changed the chart open NOW (its list is stale). */
function unlinkChanged(sent, report) {
  if (!report.reload || displayedPatient === null) return false;
  return sent.charts.includes(displayedPatient);
}

/** Send the Unlink judgement for whatever `compareLinked` last rendered. */
async function unlinkCompared() {
  if (unlinkCompared_ === null) return;
  // What this click signs over, captured before the round trip.
  const sent = unlinkCompared_;
  const token = unlinkToken;
  const button = el("unlink-confirm");
  // Disabled for the whole round trip: a double click must never send a second judgement.
  button.disabled = true;
  try {
    const report = await invoke("unlink_records", {
      patientId: sent.patientId,
      charts: sent.charts,
      low: sent.low,
      high: sent.high,
    });
    const place = unlinkAnswerPlace(sent, token);
    if (place === "elsewhere") {
      const text = "For chart " + sent.patientId + ": " + report.sentence;
      sayAnywhere(text);
      if (unlinkChanged(sent, report)) await refresh(text);
    } else if (report.reload) {
      // The record changed under this chart: the outcome goes on the chart's own line, the
      // panel closes, and the list is re-read.
      say(report.sentence);
      // `false`: the list is about to be re-drawn, which destroys any button focus would land
      // on. Focus goes to the patient heading once the re-read is done.
      if (!el("unlink-panel").hidden) closeUnlinkPanel(false);
      await refresh(report.sentence);
      if (!el("unlink-panel").hidden || !el("link-panel").hidden || el("chart-view").hidden) return;
      const heading = el("patient-heading");
      heading.tabIndex = -1;
      heading.focus();
    } else if (place === "chart") {
      say(report.sentence);
    } else {
      // Outranked: the panel stays open and says this ONCE.
      setMessage(el("unlink-status"), report.sentence);
    }
  } catch (failure) {
    const place = unlinkAnswerPlace(sent, token);
    const text = failureText(failure);
    if (place === "elsewhere") {
      sayAnywhere("For chart " + sent.patientId + ": " + text);
    } else if (place === "chart") {
      say(text);
    } else {
      setMessage(el("unlink-status"), text);
      // A verdict, or a node whose state must change first, will refuse the same comparison
      // again: take the button away. A bare IPC failure or a locked key keeps it.
      if (failure && (failure.retry === "never" || failure.retry === "after_operator")) {
        unlinkCompared_ = null;
        button.hidden = true;
      }
    }
  } finally {
    button.disabled = false;
  }
}

el("unlink-close").addEventListener("click", () => closeUnlinkPanel(true));
el("unlink-confirm").addEventListener("click", unlinkCompared);
el("unlink-panel").addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeUnlinkPanel(true);
});
