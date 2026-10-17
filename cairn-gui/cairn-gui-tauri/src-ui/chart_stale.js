// What the window says when the record changed under the chart on screen (#797). A refusal that
// says "the screen is behind the record" (`retry: "after_reread"`) re-reads the WHOLE chart, from
// whichever handler it reached (main.js's `signOff`/`cease`, link.js, unlink.js). The re-read then
// says it was read again and what joined or left the record — Rust's sentence (the pane's
// `set_change`, from `chart_set::set_change`), never one guessed here. If that re-read fails, the list stays up,
// marked out of date: a stale list, flagged, is an honest near-truth (principle 4), and every
// write over it is refused by the backend's set check anyway.
//
// Split out of main.js to keep it under 500 lines. Classic script, loaded right after
// stop_reasons.js and sharing the one global scope (`el`, `say`, `setMessage` from main.js).
// Its functions are called only once a chart is read, long after every script has loaded.
"use strict";

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

/** Join the sentences that are present, one space apart. */
function sentences(...parts) {
  return parts.filter(Boolean).join(" ");
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
