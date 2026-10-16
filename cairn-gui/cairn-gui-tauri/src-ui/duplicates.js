// The possible-duplicate banner (repair path R5a, #680). Decides nothing but whether "Different
// people" is visible, and words nothing but the two failures Rust cannot see — the banner failing
// to DRAW, and its own read failing to reach the backend (`failedBanner`, #745): every other
// sentence comes from Rust (`duplicates/view.rs`), every check from the backend
// (`duplicates/mod.rs`), and whether the banner hides is Rust's `silent`. Classic script, loaded after main.js and
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
  renderDuplicates({ entries: [], more: null, error: null, check_lines: [], silent: true });
}

/** A banner saying only `text`, as its error line: never silent, so never hidden. */
function failedBanner(text) {
  return { entries: [], more: null, error: text, check_lines: [], silent: false };
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
    renderDuplicates(failedBanner("The possible duplicates could not be shown: " + String(failure)));
  }
}

/**
 * The banner ALONE, for the open chart whose medication list could not be read (#745 points 1
 * and 2). The duplicate warning needs only the chart set, so a failed list read must not take it
 * down; and after a judgement, a failed re-read must not leave the judged pair up with a live
 * Review — the banner is re-read on its own, and if even that fails it says so in place of the
 * old entries. A read for a chart the clinician has since left is dropped.
 */
async function redrawBannerAlone(patient) {
  let section;
  try {
    section = await invoke("duplicate_banner", { patientId: patient });
  } catch (failure) {
    section = failedBanner("Could not check for possible duplicates: " + failureText(failure));
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
