// The possible-duplicate banner (repair path R5a, #680). Renders and decides nothing: every
// sentence comes from Rust (`duplicates/view.rs`), every check from the backend
// (`duplicates/mod.rs`). Classic script, loaded after main.js, funnel.js, link.js and unlink.js,
// sharing their scope.
//
// Its one piece of state is which buttons are visible. Review reuses the "Same person as…"
// panel (link.js) pre-filled with the banner's chart; only a comparison opened HERE shows
// "Different people" — `clearComparison` hides it again for any other comparison.
"use strict";

/** Draw `pane.duplicates`. Hidden only when there is truly nothing to say. */
function renderDuplicates(section) {
  setMessage(el("duplicates-error"), section.error || "");
  el("duplicates-entries").replaceChildren(...section.entries.map(duplicateItem));
  setMessage(el("duplicates-more"), section.more || "");
  el("duplicates-checks").replaceChildren(...section.check_lines.map((t) => cell("li", t)));
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
  for (const line of entry.identity_lines) li.append(cell("p", line));
  for (const note of entry.notes) li.append(cell("p", note));
  li.append(cell("h4", entry.medications_heading));
  const meds = document.createElement("ul");
  meds.append(...entry.medications.map((t) => cell("li", t)));
  meds.append(...entry.medication_notes.map((t) => cell("li", t)));
  li.append(meds);
  const review = document.createElement("button");
  review.type = "button";
  review.textContent = "Review";
  review.addEventListener("click", () => reviewDuplicate(entry.review_chart));
  li.append(review);
  return li;
}

/**
 * Open the comparison on the banner's chart. The backend admits it only while an open proposal
 * still joins it to this record (`duplicates::admit_other`). "Different people" is shown only
 * when THIS comparison is the one now on the panel (a newer click, or a close, wins).
 */
async function reviewDuplicate(otherId) {
  openLinkPanel();
  await compare(otherId);
  el("link-different").hidden = !(compared !== null && compared.otherId === otherId);
}

el("link-different").addEventListener("click", () =>
  sendJudgement("record_different_people", el("link-different")),
);
