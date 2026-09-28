// "Same person as…" (repair path R2b-1). Renders and decides nothing: every sentence comes
// from Rust (`link/view.rs`), every check from the backend (`link/mod.rs`). Classic script,
// loaded after main.js and funnel.js, sharing their scope: `el`, `cell`, `setMessage`, `say`,
// `refresh`, `displayedPatient`, `renderedPatient`, `renderedCharts` (main.js) and
// `failureText` (funnel.js — its doc comment explains why a bare IPC failure needs its own
// wording, not just Rust's).
//
// EVERY MESSAGE GOES THROUGH `setMessage`. `#link-status` is hidden whenever it is empty
// (`clearComparison`), and style.css makes `[hidden]` win over everything: a bare
// `.textContent = …` would put the words in the DOM but never on screen, and a hidden live
// region is never announced (final review C1 — the Outranked sentence and every refusal were
// invisible that way).
"use strict";

/** Browse answers older than the last keystroke — or than the last open/close — are dropped. */
let linkRevision = 0;
/**
 * Bumped on every `compare` click; an answer for an older token is dropped. Two candidates
 * can be clicked before either `compare_records` round trip returns, and they need not
 * resolve in the order they were sent — without this, the slower answer could land LAST and
 * silently overwrite what the clinician most recently asked to see, so the panel would show
 * one candidate's comparison while `compared` (what Link would act on) still matched it, but
 * neither matched the click the clinician thinks they just made. Same pattern as
 * `browseRevision`/`registerRevision` in funnel.js.
 */
let compareToken = 0;
/**
 * What the panel compared — BOTH records, as the comparison was read over them. Link sends
 * exactly these back, never the window's current `renderedCharts`: a re-read between Compare
 * and Link (a sign-off's `refresh()`) can grow this record, and the judgement must name the
 * set the clinician actually compared, so the backend can refuse a changed one (final review
 * I1). `null` whenever there is nothing a Link could honestly be signed over.
 */
let compared = null; // { patientId, charts, otherId, otherCharts }
/**
 * Whether the signing key is unlocked, as last reported by `renderLockState` through
 * `updateLinkLock`. Kept here (not read off the DOM) because a comparison can render before
 * the first lock poll lands, and `renderComparison` needs an answer either way.
 */
let keyUnlocked = false;

/**
 * Open the panel: clear whatever an earlier visit left behind, then focus its heading — the
 * same reason `enterChart` (funnel.js) focuses the patient heading: a screen reader must be
 * told what just appeared, not left pointed at the button that opened it.
 */
function openLinkPanel() {
  forgetInFlight();
  compared = null;
  // A fresh visit, not a resumed one: a name typed before an earlier close, or that search's
  // summary line, must not sit next to an empty candidate list and read as still current.
  el("link-name").value = "";
  el("link-dob").value = "";
  setMessage(el("link-search-status"), "");
  el("link-candidates").replaceChildren();
  clearComparison();
  el("link-panel").hidden = false;
  el("link-heading").focus();
}

/**
 * Hide the panel and forget what it held.
 *
 * `returnFocus` exists because this is called from two different situations that want
 * opposite focus behaviour (controller ruling). When a clinician acts ON the panel itself —
 * the "Close comparison" button, Escape — focus belongs back on `#same-person`, the control
 * that opened it. But this is ALSO called from funnel.js whenever the chart underneath the
 * panel is about to change (`enterChart`, `closeChart`), so a panel from the old chart can
 * never stay open over the new one's header; in that case funnel.js is about to move focus
 * itself (to the new patient heading, or to the front door), and `#same-person` may not even
 * exist yet there — so this function must NOT touch focus, and callers pass `false`.
 */
function closeLinkPanel(returnFocus) {
  el("link-panel").hidden = true;
  forgetInFlight();
  compared = null;
  clearComparison();
  if (returnFocus) el("same-person").focus();
}

/**
 * Drop every compare and search answer still in flight (final review M2): an answer from before
 * an open or close belongs to a visit that is over, and must not draw into the next one.
 */
function forgetInFlight() {
  compareToken += 1;
  linkRevision += 1;
}

/** Empty every part of the panel that a fresh search or a fresh comparison must replace. */
function clearComparison() {
  for (const id of ["link-findings", "link-table", "link-other-meds-section", "link-confirm", "link-problems"]) {
    el(id).hidden = true;
  }
  el("link-findings").replaceChildren();
  el("link-table").tHead.replaceChildren();
  el("link-table").tBodies[0].replaceChildren();
  setMessage(el("link-status"), "");
}

/** Search other charts for a possible match, as the clerk types. */
async function runLinkSearch() {
  const revision = ++linkRevision;
  const form = { revision, raw_name: el("link-name").value, birth_date: el("link-dob").value };
  const list = el("link-candidates");
  if (!form.raw_name.trim() && !form.birth_date.trim()) {
    list.replaceChildren();
    // Nothing is shown any more, so nothing may still be SAID about it — a status line
    // reporting the last search's count next to an empty list would describe a list that is
    // no longer on screen (principle 4). `linkRevision` was already bumped above, so any
    // browse answer still in flight for the cleared search is dropped when it lands.
    setMessage(el("link-search-status"), "");
    return;
  }
  try {
    const view = await invoke("browse", { form });
    if (view.revision !== linkRevision) return; // a newer search is on its way
    // Charts already in this record are left off: there is nothing to compare them against.
    const inRecord = new Set(renderedCharts || []);
    list.replaceChildren(
      ...view.candidates
        .filter((c) => !inRecord.has(c.patient_id))
        .map((c) => {
          const li = document.createElement("li");
          const b = document.createElement("button");
          b.type = "button";
          b.textContent = "Compare: " + c.name + " — " + c.age + " — identity " + c.trust;
          b.addEventListener("click", () => compare(c.patient_id));
          li.append(b);
          return li;
        }),
    );
    setMessage(el("link-search-status"), view.summary);
  } catch (failure) {
    if (revision !== linkRevision) return;
    list.replaceChildren();
    setMessage(el("link-search-status"), failureText(failure));
  }
}

/** Read both records and render the comparison — findings, table, and the other chart's meds. */
async function compare(otherId) {
  const token = ++compareToken;
  // Captured NOW, synchronously: the chart this comparison is about. Nothing read later
  // (`renderedPatient` after a re-read, or after a switch) may stand in for it.
  const patientId = renderedPatient;
  clearComparison();
  compared = null; // until this answer lands, there is nothing a Link could be signed over
  try {
    const view = await invoke("compare_records", {
      patientId,
      charts: renderedCharts,
      otherId,
    });
    if (token !== compareToken) return; // a newer compare, or a close, came after this one
    renderComparison(view);
    // Both sets as the BACKEND read them for this comparison — Link sends these back.
    compared = { patientId, charts: view.left_charts, otherId, otherCharts: view.other_charts };
  } catch (failure) {
    if (token !== compareToken) return;
    setMessage(el("link-status"), failureText(failure));
  }
}

/**
 * Draw a `ComparisonView`. DOM ORDER IS CLINICAL, and it is already right in the markup
 * (index.html): problems, then findings, then the table — so this function only fills each
 * part, never reorders them.
 */
function renderComparison(view) {
  const problems = el("link-problems");
  problems.textContent = view.problems.join(" ");
  problems.hidden = view.problems.length === 0;

  const findings = el("link-findings");
  findings.replaceChildren(...view.findings.map((f) => cell("li", f)));
  findings.hidden = view.findings.length === 0; // never a "no conflicts" line

  const table = el("link-table");
  const groups = document.createElement("tr");
  groups.append(cell("td", ""));
  groups.append(cell("th", "This record", { scope: "colgroup", colspan: String(view.left_count) }));
  const right = view.columns.length - view.left_count;
  if (right > 0) groups.append(cell("th", "Other record", { scope: "colgroup", colspan: String(right) }));
  const heads = document.createElement("tr");
  heads.append(cell("td", ""));
  for (const col of view.columns) heads.append(cell("th", col.heading, { scope: "col" }));
  table.tHead.replaceChildren(groups, heads);
  table.tBodies[0].replaceChildren(
    ...view.rows.map((row) => {
      const tr = document.createElement("tr");
      tr.append(cell("th", row.label, { scope: "row" }));
      for (const text of row.cells) tr.append(cell("td", text));
      return tr;
    }),
  );
  table.hidden = view.columns.length === 0;

  el("link-other-meds-notes").replaceChildren(...view.other_medication_notes.map((n) => cell("li", n)));
  el("link-other-meds").replaceChildren(...view.other_medications.map((m) => cell("li", m)));
  el("link-other-meds-section").hidden = false;

  // No Link button unless the whole comparison was read (design: a judgement needs the whole
  // picture) — and, when it IS shown, its label must reflect the CURRENT lock state, not
  // whatever the button said the last time it was visible.
  el("link-confirm").hidden = !view.can_link;
  updateLinkLock(keyUnlocked);
}

/**
 * Called by `renderLockState` (main.js) whenever the signing-key lock state is read, so the
 * Link button never claims a key is unlocked (or locked) longer than it actually is — the
 * same "state is ambient, never modal" rule main.js already applies to `#lock-state`.
 */
function updateLinkLock(unlocked) {
  keyUnlocked = Boolean(unlocked);
  el("link-confirm").textContent = keyUnlocked
    ? "Link — same person"
    : "Link — same person (unlock your signing key first)";
}

/**
 * Where a link's answer is reported, decided when it LANDS (final review M1). A signed act's
 * outcome is never dropped — but it is only drawn into the panel if the panel still shows the
 * comparison it was signed over.
 *
 * - `"panel"`: same chart, same comparison still open — the ordinary case.
 * - `"chart"`: same chart, but the panel was closed or a newer comparison replaced it — the
 *   outcome goes on the chart's own `#outcome` line instead.
 * - `"elsewhere"`: the clinician has since left that chart. The outcome is still reported, named
 *   with the chart it was about, and the chart now open is NOT re-read for it: nothing about
 *   the new chart changed.
 */
function linkAnswerPlace(sent, token) {
  if (displayedPatient !== sent.patientId) return "elsewhere";
  if (token !== compareToken || el("link-panel").hidden) return "chart";
  return "panel";
}

/**
 * Say a link outcome on whatever surface is showing now. On the front door there is no
 * `#outcome` on screen (it lives in the hidden chart view), so the front door's own status
 * line carries it.
 */
function sayAnywhere(text) {
  if (el("chart-view").hidden) el("browse-status").textContent = text;
  else say(text);
}

/** Send the Link judgement for whatever `compare` last rendered. */
async function linkCompared() {
  if (compared === null) return;
  // What this click signs over, captured before the round trip: nothing that changes while
  // the answer is in flight may change what was sent, or where its answer is reported.
  const sent = compared;
  const token = compareToken;
  const button = el("link-confirm");
  // Disabled for the whole round trip, not just relabelled: a double click before the first
  // answer lands must never send a second judgement (ADR-0053 — a click IS a signature).
  button.disabled = true;
  try {
    const report = await invoke("link_records", {
      patientId: sent.patientId,
      charts: sent.charts,
      otherId: sent.otherId,
      otherCharts: sent.otherCharts,
    });
    const place = linkAnswerPlace(sent, token);
    if (place === "elsewhere") {
      sayAnywhere("For chart " + sent.patientId + ": " + report.sentence);
    } else if (report.reload) {
      // Took effect: the record just changed under this chart, so the outcome belongs on the
      // chart itself (`#outcome`, via `say`) — the panel that reported it is about to close,
      // same chart, same `#same-person` button — and the medication list is re-read.
      say(report.sentence);
      if (!el("link-panel").hidden) closeLinkPanel(true);
      await refresh();
    } else if (place === "chart") {
      say(report.sentence);
    } else {
      // Outranked: the panel stays open, so its own `#link-status` says this ONCE — not also
      // through `say()`, which would read as two different outcomes rather than one.
      setMessage(el("link-status"), report.sentence);
    }
  } catch (failure) {
    const place = linkAnswerPlace(sent, token);
    const text = failureText(failure);
    if (place === "elsewhere") {
      sayAnywhere("For chart " + sent.patientId + ": " + text);
    } else if (place === "chart") {
      say(text);
    } else {
      setMessage(el("link-status"), text);
      // A verdict (or a node that needs an operator) will refuse the same comparison again:
      // take the button away rather than invite a second identical click (final review M3).
      // The comparison and the sentence stay on screen — what was refused is still legible.
      // A bare IPC failure (no `retry`) and an outage (`"now"`) keep it: the outcome is unknown
      // and a second identical link is harmless.
      if (failure && (failure.retry === "never" || failure.retry === "after_operator")) {
        compared = null;
        button.hidden = true;
      }
    }
  } finally {
    button.disabled = false;
  }
}

el("same-person").addEventListener("click", openLinkPanel);
el("link-close").addEventListener("click", () => closeLinkPanel(true));
el("link-confirm").addEventListener("click", linkCompared);
el("link-search").addEventListener("input", runLinkSearch);
el("link-search").addEventListener("submit", (e) => e.preventDefault());
el("link-panel").addEventListener("keydown", (e) => {
  if (e.key === "Escape") closeLinkPanel(true);
});
