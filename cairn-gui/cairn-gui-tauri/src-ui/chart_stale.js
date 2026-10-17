// What the window says when the record changed under the chart on screen (#797). A refusal that
// says "the screen is behind the record" (`retry: "after_reread"`) re-reads the WHOLE chart, from
// whichever handler it reached (main.js's `signOff`/`cease`, link.js, unlink.js). The re-read then
// says it was read again and what joined or left the record — Rust's sentence (the pane's
// `set_change`, from `chart_set::set_change`), never one guessed here. If that re-read fails, the
// list stays up, marked out of date: a stale list, flagged, is an honest near-truth (principle 4),
// and every write over it is refused by the backend's set check anyway.
//
// THE HELD SET. "The chart I hold in my hand … I need only to be informed of what changed … that
// I cannot know" (the maintainer's rule, 2026-10-17). What the clinician holds is what the window
// last DREW, moved on by their OWN acts and by nothing else: `heldCharts`. Every re-read sends it
// as `seenCharts`, so Rust names only what differs from it. It is window state, not a re-read's
// argument: a baseline carried by one call let every other re-read compare against the pre-act
// set and announce the clinician's own link back to them (fix round 1). `renderedCharts`
// (main.js) is a different thing — the set every WRITE is checked against — and is untouched.
//
// Split out of main.js to keep it under 500 lines. Classic script, loaded right after
// stop_reasons.js and sharing the one global scope (`el`, `say`, `setMessage`, `displayedPatient`
// from main.js). Its functions are called only once a chart is read, long after every script has
// loaded.
"use strict";

/** The chart set the clinician holds (see above); null while no list is drawn. */
let heldCharts = null;

/** Said after the refusal when a stale refusal's re-read is drawn. */
const READ_AGAIN = "The chart has been read again — review it before you act on it.";

/**
 * Said in the chart's warnings when a stale refusal's re-read FAILED: the list on screen is the
 * old one. It names the control that is the way out — never "reload", which this window lacks.
 */
const LIST_OUT_OF_DATE =
  'This list is out of date: the record changed while it was on screen, and reading it ' +
  'again failed. Press "Read this chart again".';

/**
 * Whether a refusal says the screen is behind the record: the record changed since it was read
 * (nothing was recorded), or a commit was interrupted (#713, so it MAY have been). The one
 * spelling of the test, so every handler that re-reads on it asks the same question.
 */
function behindTheRecord(failure) {
  return Boolean(failure && failure.retry === "after_reread");
}

/**
 * The outcome line after a re-read was DRAWN (called by main.js's `refresh` only over an answer
 * `mayDraw` accepted, so an overtaken read says nothing). After a stale refusal: the refusal
 * (`lead`), that the chart was read again, and what changed. Otherwise only a change is added to
 * the lead; with none, the line is left alone — it may be saying how a signed act went.
 */
function sayAfterReread(lead, staleIfFails, pane) {
  if (staleIfFails) say(sentences(lead, READ_AGAIN, pane.set_change));
  else if (pane.set_change) say(sentences(lead, pane.set_change));
}

/**
 * Join the sentences that are present into one line. Each part is closed with a full stop unless
 * it already ends in one (or `!`/`?`), and the line starts with a capital: Rust's refusals end
 * without a full stop ("… — nothing was done") and are not edited (the node's reach the CLI too),
 * so a bare join read "…nothing was done The chart has been read again…".
 */
function sentences(...parts) {
  const closed = parts
    .filter(Boolean)
    .map((text) => String(text).trim())
    .filter(Boolean)
    .map((part) => (/[.!?]$/.test(part) ? part : part + "."));
  const line = closed.join(" ");
  return line.charAt(0).toUpperCase() + line.slice(1);
}

/** Set the held set to what was just drawn (`charts`), or to null when nothing is drawn. */
function holdCharts(charts) {
  heldCharts = charts;
}

/**
 * The clinician's own act was RECORDED: move the held set by that act's own change, the moment
 * the report lands and before any re-read is sent — so EVERY later re-read (this act's, a stale
 * Stop's, "Read this chart again") finds it already held and stays silent about it, while a
 * colleague's change still differs and is named. `touched`: the charts the act was sent over;
 * `after`: the report's `charts_after`, read inside the act's own transaction under the link lock
 * — after the node checked `touched`, so it is the act's result and nothing foreign.
 */
function holdOwnAct(touched, after) {
  heldCharts = applyOwnAct(heldCharts, touched, after, displayedPatient);
}

/**
 * Pure: the held set `held` after an own act over `touched` that left `after` as the record of the
 * chart it was signed on, for a window showing chart `anchor` (always a member of `held` — the held
 * set is the record of the chart on screen). Every result is a sorted, de-duplicated list.
 *
 * - No held set (nothing drawn) or no recorded set (`after` null: "Different people", or a set
 *   that is not the record on screen) → `held` as it is.
 * - The anchor ended in `after` (a link; an unlink seen from its own side) → the touched charts
 *   are replaced by `after`; held charts the act never touched stay.
 * - The anchor was touched but ended outside `after` (an unlink answered while the clinician is on
 *   the OTHER side) → the touched charts are replaced by what the act left on that side.
 * - The act never touched the anchor → `held` unchanged: an own act never silences anything else.
 *
 * Anchored, rather than `(held − (sent − after)) ∪ (after − sent)`: that form cannot tell which
 * side of the act the window is on, so a Link answered while the other chart was already drawn
 * would announce the clinician's own link back to them (fix round 1 report).
 */
function applyOwnAct(held, touched, after, anchor) {
  if (held === null || !after) return held;
  const untouched = without(held, touched);
  if (after.includes(anchor)) return ordered(untouched.concat(after));
  if (touched.includes(anchor)) return ordered(untouched.concat(without(touched, after)));
  return held;
}

/** The ids of `ids` that are not in `gone` (pure). */
function without(ids, gone) {
  return ids.filter((id) => !gone.includes(id));
}

/** `ids` de-duplicated and sorted, so one set always reads the same way (pure). */
function ordered(ids) {
  return [...new Set(ids)].sort();
}

/**
 * A stale refusal's re-read failed: say so above the table and take sign-off away, with its
 * reason. The rows stay — the clinician can still read what they last saw, now flagged. The next
 * list that draws (`drawList`) or is taken down (`clearDrawnList`) clears the warning.
 */
function markListStale() {
  setMessage(el("chart-stale"), LIST_OUT_OF_DATE);
  el("chart-warnings").hidden = false;
  const button = el("sign-off");
  button.disabled = true;
  button.textContent = "Nothing to sign off — this list is out of date";
}
