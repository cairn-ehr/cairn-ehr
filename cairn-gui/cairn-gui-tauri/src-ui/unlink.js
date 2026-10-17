// The REVIEW panel (repair path R2b-2, R5a, R6). One panel, opened from a line under "How these
// charts are linked", that shows the two charts side by side and offers up to TWO verdicts:
//   - "Unlink — not the same person"  (`#unlink-confirm`, command `unlink_records`), and
//   - the second verdict (`#unlink-same`, command `confirm_link`), "Same person — confirm" over
//     a doubted link or "Same person — link them" over a separated pair; its label and the note
//     beside it (`#unlink-same-note`) come from Rust (`view.same_label`, `view.same_note`).
// Which verdicts appear is Rust's call (`view.can_unlink`, `view.can_confirm`), never ours.
// THE SHARED IN-FLIGHT RULE (R5a): while EITHER verdict is being sent BOTH buttons are disabled,
// and once one lands or is refused for good BOTH are hidden — one human must never sign two
// verdicts milliseconds apart. An `"after_reread"` failure (the screen is behind the record: the
// record changed, or a commit was interrupted) disarms and re-reads the chart, never offers a
// live retry. No confirmation dialog: the click is the act.
// A verdict that LANDS (even as "outranked" by a colleague's) takes BOTH buttons off the panel and
// forgets the comparison; a fresh "Review…" click re-reads the pair. An Outranked or unknown
// answer that lands after a newer Review of the SAME pair disarms that one too
// (`disarmNewerReview`): the pair looks unchanged, so the node would not catch a second press.
// So does any other recorded answer — that newer Review is stale too.
// Renders and decides nothing: every sentence comes from Rust, every check from the backend.
// Classic script, loaded after main.js, funnel.js and link.js, sharing their scope: `el`, `cell`,
// `setMessage`, `say`, `refresh`, `invoke`, `displayedPatient`, `renderedPatient`,
// `renderedCharts` (main.js), `behindTheRecord` (chart_stale.js), `failureText` (funnel.js),
// `sayAnywhere`, `updateLinkLock`, `keyUnlocked`, `closeLinkPanel` (link.js). It mirrors link.js's
// rules (read its header) and is a panel SEPARATE FROM THE LINK PANEL: the link panel judges
// "link / different people" over a searched-for chart; this one reviews a link or judgement that
// already stands. Its two verdicts are guarded together (above), so one human cannot sign both.
//
// EVERY MESSAGE GOES THROUGH `setMessage`: `#unlink-status` is hidden whenever empty and
// `[hidden]` wins over everything, so a bare `.textContent = …` would never reach the screen.
"use strict";

/** Bumped on every open, click on a link, or close; an answer for an older token is dropped. */
let unlinkToken = 0;
/**
 * What the panel compared, as the backend read it: BOTH verdicts (Unlink and Same person) send
 * exactly these back, never the window's current `renderedCharts`, so the judgement names what
 * the clinician looked at. `canUnlink`/`canConfirm` say which verdict this read allows.
 * `null` until a WHOLE comparison (`can_unlink` or `can_confirm`) has landed. `standing` is the
 * pair's content address, opaque: sent back unchanged with EITHER verdict so the node can refuse
 * one made over a pair that changed since.
 */
let unlinkCompared_ = null; // { patientId, charts, low, high, kind, standing, canUnlink, canConfirm }
/** The line (low/high/kind) whose button opened the panel: Close returns focus to that button. */
let unlinkOpener = null;

/** Empty every part of the panel that a fresh comparison must replace. */
function clearUnlinkComparison() {
  for (const id of ["unlink-findings", "unlink-table", "unlink-confirm", "unlink-same", "unlink-same-note", "unlink-problems"]) {
    el(id).hidden = true;
  }
  el("unlink-findings").replaceChildren();
  el("unlink-table").tHead.replaceChildren();
  el("unlink-table").tBodies[0].replaceChildren();
  setMessage(el("unlink-status"), "");
  setMessage(el("unlink-same-note"), "");
  el("unlink-same").dataset.label = ""; // updateLinkLock must never redraw an old comparison's label
}

/** Disable (or re-enable) BOTH verdicts: one human must never sign two verdicts ms apart. */
function setVerdictsDisabled(disabled) {
  el("unlink-confirm").disabled = disabled;
  el("unlink-same").disabled = disabled;
}

/** Hide BOTH verdicts once one has landed or been refused for good. */
function hideVerdicts() {
  el("unlink-confirm").hidden = true;
  el("unlink-same").hidden = true;
}

/**
 * Open the panel for the link between charts `low` and `high`, read both charts and draw the
 * comparison: problems, then findings, then the table (DOM order is clinical, already right in
 * the markup). Focus goes to the heading so a screen reader is told what appeared. `kind` and
 * `attested` are the line's own: Rust refuses a pair judged since the line was drawn.
 */
async function compareLinked(low, high, kind, attested) {
  const token = ++unlinkToken;
  // Captured NOW, synchronously: the chart this comparison is about.
  const patientId = renderedPatient;
  const charts = renderedCharts;
  unlinkCompared_ = null; // until a whole answer lands, there is nothing a verdict could be signed over
  unlinkOpener = { low, high, kind };
  // The two panels are mutually exclusive: never one verb's button over the other's comparison.
  if (typeof closeLinkPanel === "function" && !el("link-panel").hidden) closeLinkPanel(false);
  clearUnlinkComparison();
  el("unlink-panel").hidden = false;
  el("unlink-heading").focus();
  try {
    const view = await invoke("compare_linked", { patientId, charts, low, high, kind, attested });
    if (token !== unlinkToken) return; // a newer click, or a close, came after this one
    renderUnlinkComparison(view);
    unlinkCompared_ =
      view.can_unlink || view.can_confirm
        ? {
            patientId,
            charts: view.charts,
            low: view.low,
            high: view.high,
            kind: view.kind,
            standing: view.standing,
            canUnlink: view.can_unlink,
            canConfirm: view.can_confirm,
          }
        : null;
  } catch (failure) {
    if (token !== unlinkToken) return;
    const text = failureText(failure);
    setMessage(el("unlink-status"), text);
    // The screen is behind the record (#797): the line changed, its pair was judged since, or
    // the link is gone. A Review of a line no longer drawn is a dead end, so the WHOLE chart is
    // re-read, flagged stale — never a retry of the same stale line.
    const reread = behindTheRecord(failure);
    if (reread && patientId === displayedPatient) await refresh(text, reread);
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

  // No verdict unless the whole comparison was read: a judgement needs the whole picture.
  el("unlink-confirm").hidden = !view.can_unlink;
  const same = el("unlink-same");
  // The BASE label; updateLinkLock (link.js) adds the unlock hint to it, below.
  same.dataset.label = view.same_label || "";
  same.hidden = !view.can_confirm;
  setMessage(el("unlink-same-note"), view.same_note || "");
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
    const own = opener && buttons.find((b) => b.dataset.low === opener.low && b.dataset.high === opener.high && b.dataset.kind === opener.kind);
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
  if (unlinkCompared_ === null || !unlinkCompared_.canUnlink) return;
  // What this click signs over, captured before the round trip.
  const sent = unlinkCompared_;
  const token = unlinkToken;
  // BOTH verdicts disabled for the whole round trip: neither a double click nor the other verdict
  // may send a second judgement.
  setVerdictsDisabled(true);
  try {
    let report;
    // ONLY the backend call is caught: once it returns, the judgement is recorded, and a fault in
    // the drawing below must never be reported as the unlink failing (PR #711 review).
    try {
      report = await invoke("unlink_records", {
        patientId: sent.patientId,
        charts: sent.charts,
        low: sent.low,
        high: sent.high,
        // Handed back unchanged: the node refuses the unlink if the pair's standing judgement
        // is no longer this one (a colleague's confirm landed meanwhile).
        standing: sent.standing,
      });
    } catch (failure) {
      await showUnlinkFailure(sent, token, failure);
      return;
    }
    await showUnlinkReport(sent, token, report);
  } finally {
    setVerdictsDisabled(false);
  }
}

/**
 * Send the second verdict (R6): "Same person". A mirror of `unlinkCompared`, over the same
 * comparison, with the pair's `standing` address handed back unchanged.
 */
async function confirmCompared() {
  if (unlinkCompared_ === null || !unlinkCompared_.canConfirm) return;
  const sent = unlinkCompared_;
  const token = unlinkToken;
  setVerdictsDisabled(true);
  try {
    let report;
    // ONLY the backend call is caught (see `unlinkCompared`).
    try {
      report = await invoke("confirm_link", {
        patientId: sent.patientId,
        charts: sent.charts,
        low: sent.low,
        high: sent.high,
        kind: sent.kind,
        standing: sent.standing,
      });
    } catch (failure) {
      await showUnlinkFailure(sent, token, failure);
      return;
    }
    await showUnlinkReport(sent, token, report);
  } finally {
    setVerdictsDisabled(false);
  }
}

/**
 * Disarm a NEWER comparison of the same pair as `sent` — a second Review pressed while `sent`'s
 * answer was in flight. Called once `sent` landed (any recorded answer) or may have (#713's
 * unknown outcome). The case that matters: landed Outranked, the pair's standing judgement is
 * unchanged, so the node's stale-pair check would pass a second press — a second judgement over
 * the colleague's that won. After any other answer the comparison is stale anyway.
 */
function disarmNewerReview(sent) {
  if (
    unlinkCompared_ !== null &&
    unlinkCompared_.low === sent.low &&
    unlinkCompared_.high === sent.high
  ) {
    unlinkCompared_ = null;
    hideVerdicts();
  }
}

/** A recorded verdict (Unlink or Same person): say what it did, where the clinician is now, and re-read if it changed. */
async function showUnlinkReport(sent, token, report) {
  const place = unlinkAnswerPlace(sent, token);
  // Recorded: the clinician now holds what their own verdict left (`charts_after`; null when it
  // changed no set), so no re-read announces it back to them (#797). `sent.charts` is every chart
  // the verdict touched: the pair is always one of the record's own (`resolve_edge` finds it among
  // the set's links), "link them" over a separated pair included.
  holdOwnAct(sent.charts, report.charts_after);
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
    // Outranked (`reload` false — see `LinkReportView::reload`): the panel stays open and says
    // this ONCE. The verdict LANDED, so neither verdict may be signed again over this stale read:
    // forget the comparison and take BOTH buttons away (a fresh Review re-reads the pair).
    setMessage(el("unlink-status"), report.sentence);
    unlinkCompared_ = null;
    hideVerdicts();
  }
  // Wherever it landed, a recorded verdict leaves a newer comparison of this pair stale: after an
  // Outranked the pair looks unchanged, so the node would pass a second press; after anything else
  // the record changed and a fresh Review must re-read it anyway.
  disarmNewerReview(sent);
}

/**
 * A verdict (Unlink or Same person) that failed: refused or never reached (nothing was
 * recorded) — or the screen is behind the record (`"after_reread"`: the record changed, so
 * nothing was recorded, or the commit was interrupted, #713, when it MAY have been).
 */
async function showUnlinkFailure(sent, token, failure) {
  const place = unlinkAnswerPlace(sent, token);
  const text = failureText(failure);
  // Said where the clinician is now; also the re-read's lead, so it keeps its "For chart …".
  const lead = place === "elsewhere" ? "For chart " + sent.patientId + ": " + text : text;
  // #713: an interrupted commit may have recorded the verdict. A second press over a chart that
  // still looks unchanged (the verdict landed as Outranked) would record a second judgement over a
  // colleague's: both verdicts go and the chart is re-read, never a live retry.
  const reread = behindTheRecord(failure);
  if (place === "elsewhere") {
    sayAnywhere(lead);
  } else if (place === "chart") {
    say(text);
  } else {
    setMessage(el("unlink-status"), text);
    // A verdict, or a node whose state must change first, will refuse the same comparison
    // again: take both verdicts away. So does an unknown outcome (above). A bare IPC failure or a
    // locked key keeps them.
    if (
      failure &&
      (failure.retry === "never" || failure.retry === "after_operator" || reread)
    ) {
      unlinkCompared_ = null;
      hideVerdicts();
    }
  }
  // An unknown outcome also disarms a NEWER comparison of the same link (answered "chart" or
  // "elsewhere" above): pressing it could record a second judgement over one that may have landed.
  if (reread) disarmNewerReview(sent);
  if (reread && displayedPatient !== null && sent.charts.includes(displayedPatient)) {
    await refresh(lead, reread);
  }
}

el("unlink-close").addEventListener("click", () => closeUnlinkPanel(true));
el("unlink-confirm").addEventListener("click", unlinkCompared);
el("unlink-same").addEventListener("click", confirmCompared);
el("unlink-panel").addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeUnlinkPanel(true);
});
