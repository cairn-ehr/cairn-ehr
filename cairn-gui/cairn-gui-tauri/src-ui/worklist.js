// The front door's possible-duplicate tray (repair path R5b, #680). Decides nothing, and words
// nothing but the one failure Rust cannot see — the list failing to DRAW (`loadTray`): every other
// sentence comes from Rust (`worklist/view.rs`), every rule from the backend. Classic
// script, loaded after duplicates.js and BEFORE funnel.js, whose `boot()` calls `refreshTray`
// (load order, never a typeof guard — R5a's rule). It uses funnel.js's `openChart`,
// `failureText` and `registering` only at call time.
//
// The tray is a native <details>: closed by default, its <summary> the count. It stays open across
// visits to a chart (the element is never rebuilt), and an open tray re-reads its list on every
// return to the front door — `close_chart` cleared the backend's list of openable charts, and the
// list read admits them again. Until that read lands the list is EMPTY, never the old entries.
"use strict";

/**
 * One counter per read, bumped when a read starts: an answer whose read is no longer the newest
 * is dropped, never drawn over a newer one (#752 point 1). Two reads can be in flight at once —
 * the <details> toggle and a return to the front door both read the list — and a slower, older
 * answer must not win. The same pattern as funnel.js's `browseRevision`.
 */
let trayCountRevision = 0;
let trayListRevision = 0;

/** Re-read the count (front-door show, and every return to it); re-read the list if open. */
async function refreshTray() {
  const revision = ++trayCountRevision;
  // A return to the front door: `close_chart` has emptied the backend's list of openable charts,
  // so entries still on screen would have their Review refused with advice that does not fit the
  // tray ("search again"). They go now, not when the re-read lands (#752 point 2).
  if (el("duplicate-tray").open) clearTrayList();
  let counted;
  try {
    counted = await invoke("duplicate_tray_count");
  } catch (failure) {
    counted = { summary: failureText(failure), status_line: null };
  }
  if (revision !== trayCountRevision) return; // a newer count is on its way
  const tray = el("duplicate-tray");
  tray.hidden = !counted.summary;
  el("duplicate-tray-summary").textContent = counted.summary || "";
  setMessage(el("duplicate-tray-status"), counted.status_line || "");
  if (!tray.hidden && tray.open) await loadTray();
}

/**
 * Read and draw the list. The list is emptied when the read STARTS (#752): until the answer
 * lands, nothing stale is offered. A failure to build the new entries leaves it empty and says
 * so: otherwise the failure would reach only the console (both callers discard this promise).
 */
async function loadTray() {
  const revision = ++trayListRevision;
  // A closed tray keeps its last entries, and opening it shows them: they may be stale (and their
  // Review refused, as above) until this read lands, so they go first.
  clearTrayList();
  let list;
  try {
    list = await invoke("duplicate_worklist");
  } catch (failure) {
    list = { entries: [], more: null, error: failureText(failure) };
  }
  if (revision !== trayListRevision) return; // a newer read is on its way
  let items;
  try {
    items = list.entries.map(trayItem);
  } catch (failure) {
    items = [];
    list = { entries: [], more: null, error: "The possible duplicates could not be shown: " + String(failure) };
  }
  setMessage(el("duplicate-tray-error"), list.error || "");
  el("duplicate-tray-list").replaceChildren(...items);
  setMessage(el("duplicate-tray-more"), list.more || "");
}

/** Empty the list and its two lines; the count and the status line stay. */
function clearTrayList() {
  el("duplicate-tray-list").replaceChildren();
  setMessage(el("duplicate-tray-error"), "");
  setMessage(el("duplicate-tray-more"), "");
}

let trayLineCount = 0; // makes each side's first-line id unique

/**
 * One side of an entry: its label, then each chart as text. It is NOT an open target (the
 * maintainer's decision: Review opens the newer record). Returns the block and the id of its first
 * chart line, which describes Review: the heading names BOTH records (#796), but Review opens only
 * the newer one, so its own description names exactly that (R5a's lesson, R3's person-row pattern).
 */
function traySide(label, side) {
  const div = document.createElement("div");
  div.append(cell("p", label));
  let firstId = null;
  if (side.error) div.append(cell("p", side.error));
  if (side.row) {
    const row = side.row;
    if (row.label) div.append(cell("p", row.label));
    const ul = document.createElement("ul");
    for (const member of row.members) {
      const li = cell("li", member.name + " — " + member.age + " — identity " + member.trust);
      if (firstId === null) {
        firstId = "tray-line-" + ++trayLineCount;
        li.id = firstId;
      }
      ul.append(li);
    }
    div.append(ul);
  }
  return { div, firstId };
}

/** One possible duplicate: both records, the notes, and Review. */
function trayItem(entry) {
  const li = document.createElement("li");
  const newer = traySide(entry.newer_label, entry.newer);
  const older = traySide(entry.older_label, entry.older);
  // The heading names both records, so a screen reader's heading list tells the entries apart
  // (#796); the status ("not yet reviewed", or accepted) is the first line under it.
  li.append(cell("h3", entry.heading), cell("p", entry.status), newer.div, older.div);
  for (const note of entry.notes) li.append(cell("p", note));
  if (entry.open_chart) {
    const review = document.createElement("button");
    review.type = "button";
    review.textContent = "Review";
    // Marked like the funnel's candidate rows, so funnel.js's `setCandidatesEnabled` disables
    // it while a registration saves: funnel.js's `openChart` ignores a click then, and a button
    // that silently did nothing would leave the clerk guessing (funnel.js's `registering`, read
    // at creation like `candidateItem` does).
    review.dataset.candidate = "true";
    review.disabled = registering;
    // A screen reader says "Review, <the newer record's first chart>".
    if (newer.firstId) review.setAttribute("aria-describedby", newer.firstId);
    review.addEventListener("click", async () => {
      // `openChart` writes a refusal into the node's text but knows nothing of `hidden`; the
      // error line starts hidden, so show it only when a refusal actually left words in it.
      const errorLine = el("duplicate-tray-error");
      setMessage(errorLine, "");
      await openChart(entry.open_chart, "duplicate-tray-error");
      errorLine.hidden = !errorLine.textContent;
    });
    li.append(review);
  }
  return li;
}

el("duplicate-tray").addEventListener("toggle", () => {
  if (el("duplicate-tray").open) void loadTray();
});
