// The possible-duplicate banner (repair path R5a, #680). Decides nothing but whether "Different
// people" and Review are visible, and words nothing but what Rust cannot see: the banner failing
// to DRAW, its own read failing (#745 — the lead-in to the backend's refusal, or the whole of an
// IPC failure, via funnel.js's `failureText`), and no medication list being on screen
// to compare against (`NO_LIST_TO_COMPARE`, `REVIEW_WAITS_FOR_LIST`). Every other sentence comes
// from Rust (`duplicates/view.rs`), every check from the backend (`duplicates/mod.rs`), and
// whether the banner hides is Rust's `silent`. Classic script, loaded after main.js and
// BEFORE funnel.js (whose last line starts `boot()`, whose first render calls `renderDuplicates`:
// an absent banner must never be possible, so this is guaranteed by load order, not by a typeof
// guard). It shares the global scope; link.js's `sendJudgement`, `compare`, `compared`,
// `compareToken`, `openLinkPanel` and `linkOpener` are used only at call time, long after link.js
// has loaded.
//
// It keeps no state of its own: it sets which buttons are visible, and tells link.js which Review
// opened the panel (`linkOpener`, where Close returns focus). Review reuses the "Same person as…"
// panel (link.js) pre-filled with the banner's chart; only a comparison opened HERE shows
// "Different people" — `clearComparison` hides it again for any other comparison.
"use strict";

/**
 * Said when a comparison (link.js's `compare`, reached from the panel's search) has no drawn
 * medication list to compare against: `renderedPatient` is null, so the backend would be sent no
 * chart. That is so in two cases the window cannot tell apart here — the chart's FIRST read failed
 * (`clearChart` nulled it and nothing re-read), or that read is still in flight — so the sentence
 * is true of both (PR #798 review). Only the window knows this, so it words it.
 */
const NO_LIST_TO_COMPARE =
  "This chart's medication list is not on screen — it could not be read, or is still being " +
  "read — so there is nothing to compare it with; nothing was done.";
/**
 * Under each entry of a banner drawn while no list is on screen, in place of Review. Nothing
 * re-reads a failed first read by itself (#797 tracks a reload control), so it names the way out.
 */
const REVIEW_WAITS_FOR_LIST =
  "Review needs this chart's medication list on screen, and it could not be read or is still " +
  "being read. If it could not be read, close the chart and open it again.";

/**
 * Draw a `DuplicateSection`. Hidden only when Rust says it has nothing to say (`silent`, which is
 * "checked, none open"): never re-derived here from the other fields, where a new field would
 * silently fall outside the rule (#745 point 5).
 */
function renderDuplicates(section) {
  setMessage(el("duplicates-error"), section.error || "");
  el("duplicates-entries").replaceChildren(...section.entries.map(duplicateItem));
  setMessage(el("duplicates-more"), section.more || "");
  const checks = el("duplicates-checks");
  checks.replaceChildren(...section.check_lines.map((t) => cell("li", t)));
  // An empty LABELLED list is still announced ("Duplicate check for this record, 0 items").
  checks.hidden = section.check_lines.length === 0;
  el("possible-duplicates").hidden = section.silent;
}

/** Empty the banner (a chart change; never one patient's banner under another's header). */
function clearDuplicates() {
  renderDuplicates(emptyBanner(null));
}

/**
 * A banner with no entries, saying only `error` (or nothing). The ONE section this file builds
 * itself, so its shape is written once and pinned against what Rust sends
 * (`duplicates/webview_tests.rs`). `silent` is Rust's rule (`DuplicateSection::is_silent`) for a
 * section with no entries, `more` or check lines: hidden only when there is no error to say.
 */
function emptyBanner(error) {
  return { entries: [], more: null, error, check_lines: [], silent: !error };
}

/**
 * Draw the banner, and when DRAWING it fails (a payload drift `fields_read_in` cannot see), say
 * that on the banner itself (#745 point 4). Without this the throw escaped `render` into
 * `refresh`'s catch, which reported a drawing fault as "Could not read the chart" and left the
 * medication rows undrawn. The same shape as worklist.js's `loadTray`.
 */
function drawDuplicates(section) {
  try {
    renderDuplicates(section);
  } catch (failure) {
    console.error(failure); // the stack, for whoever debugs it; the banner says the rest
    renderDuplicates(
      salvagedBanner(section, "The possible duplicates could not be shown: " + String(failure)),
    );
  }
}

/**
 * What a section that failed to draw can still say: the fault, then its own error line, and its
 * own check lines ("Duplicate check status unknown" is a safety line, not decoration). Only plain
 * strings are kept — the fault may be in any field — and the entries are dropped, since one of
 * them may be what failed. Never silent: `fault` is always said.
 */
function salvagedBanner(section, fault) {
  const own = section && typeof section.error === "string" ? " " + section.error : "";
  const lines = section && Array.isArray(section.check_lines) ? section.check_lines : [];
  return {
    ...emptyBanner(fault + own),
    check_lines: lines.filter((line) => typeof line === "string"),
  };
}

/**
 * Re-read and draw the banner ALONE, without the medication list. Three callers: main.js's
 * `refresh` when the chart's list could not be read (#745 points 1 and 2 — the duplicate warning
 * needs only the chart set, so a failed list read must not take it down, nor leave a just-judged
 * pair up with a live Review), and link.js's `compare` and `sendJudgement` after a decided
 * refusal (#745 point 3 — the banner may be stale, and only it is re-read, so a half-typed stop
 * reason survives, #799). If even this read fails, the banner says so in place of the old
 * entries. A read for a chart the clinician has since left is dropped.
 */
async function redrawBannerAlone(patient) {
  let section;
  try {
    section = await invoke("duplicate_banner", { patientId: patient });
  } catch (failure) {
    // A backend refusal is an `ErrorView` and keeps its own words; only an IPC failure (a bare
    // string) is worded "could not reach its backend" by `failureText`.
    section = emptyBanner("Could not check for possible duplicates: " + failureText(failure));
  }
  if (patient !== displayedPatient) return;
  drawDuplicates(section);
}

/** One possible duplicate: who, why to look, their current drugs, and Review. */
function duplicateItem(entry) {
  const li = document.createElement("li");
  // The heading names whose record this is, so a screen reader's heading list tells the entries
  // apart (#737); the status ("not yet reviewed", or accepted) is the first line under it.
  li.append(cell("h3", entry.heading), cell("p", entry.status));
  // The first identity line names whose record this is; Review is described by it, so a screen
  // reader announces "Review, <who>" rather than a bare "Review" repeated per entry.
  const who = entry.identity_lines.map((line) => cell("p", line));
  if (who.length > 0) who[0].id = "duplicate-" + entry.review_chart + "-who";
  li.append(...who);
  for (const note of entry.notes) li.append(cell("p", note));
  li.append(cell("h4", entry.medications_heading));
  const meds = document.createElement("ul");
  // The list's notes ("… withheld", "could not be read") come BEFORE its lines, as in the
  // comparison panel: read top-down, a list must never look complete before its warning.
  meds.append(...entry.medication_notes.map((t) => cell("li", t)));
  meds.append(...entry.medications.map((t) => cell("li", t)));
  li.append(meds);
  // A banner drawn ALONE after the chart's first read failed (`redrawBannerAlone`) has no list
  // on screen to compare with (PR #798 review): Review would send no chart. Say why it is absent
  // rather than offer a button that can only fail. Read at draw time: `render` sets
  // `renderedPatient` before it draws the banner.
  if (renderedPatient === null) {
    li.append(cell("p", REVIEW_WAITS_FOR_LIST));
    return li;
  }
  const review = document.createElement("button");
  review.type = "button";
  review.textContent = "Review";
  review.dataset.reviewChart = entry.review_chart; // where Close returns focus (link.js)
  if (who.length > 0) review.setAttribute("aria-describedby", who[0].id);
  // `compare` words its own failures; this catch is for anything else, so no failure is silent.
  review.addEventListener("click", () =>
    reviewDuplicate(entry).catch((f) => setMessage(el("link-status"), failureText(f))),
  );
  li.append(review);
  return li;
}

/**
 * Open the comparison on the banner's chart. The backend admits it only while an open proposal
 * still joins it to this record (`duplicates::admit_other`). "Different people" is shown
 * only when the entry offers it (not on an accepted pair, #736) and THIS comparison (by token)
 * is the one now on the panel (a newer click, or a close, wins).
 */
async function reviewDuplicate(entry) {
  const otherId = entry.review_chart;
  openLinkPanel();
  linkOpener = otherId;
  // `compare` bumps `compareToken` synchronously, so reading it right after the call names THIS
  // comparison; a newer one (even of the same chart, from the search) changes it.
  const pending = compare(otherId);
  const token = compareToken;
  await pending;
  el("link-different").hidden = !(entry.offers_different_people && token === compareToken && compared !== null);
}

el("link-different").addEventListener("click", () => sendJudgement("record_different_people"));
