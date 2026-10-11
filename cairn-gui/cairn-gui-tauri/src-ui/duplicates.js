// The possible-duplicate banner (repair path R5a, #680). Words nothing and decides nothing but
// whether "Different people" is visible: every sentence comes from Rust (`duplicates/view.rs`),
// every check from the backend (`duplicates/mod.rs`). Classic script, loaded after main.js and
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

/** Draw `pane.duplicates`. Hidden only when there is truly nothing to say. */
function renderDuplicates(section) {
  setMessage(el("duplicates-error"), section.error || "");
  el("duplicates-entries").replaceChildren(...section.entries.map(duplicateItem));
  setMessage(el("duplicates-more"), section.more || "");
  const checks = el("duplicates-checks");
  checks.replaceChildren(...section.check_lines.map((t) => cell("li", t)));
  // An empty LABELLED list is still announced ("Duplicate check for this record, 0 items").
  checks.hidden = section.check_lines.length === 0;
  el("possible-duplicates").hidden =
    !section.error && section.entries.length === 0 && section.check_lines.length === 0 && !section.more;
}

/** Empty the banner (a chart change; never one patient's banner under another's header). */
function clearDuplicates() {
  renderDuplicates({ entries: [], more: null, error: null, check_lines: [] });
}

/** One possible duplicate: who, why to look, their current drugs, and Review. */
function duplicateItem(entry) {
  const li = document.createElement("li");
  li.append(cell("h3", entry.heading));
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
