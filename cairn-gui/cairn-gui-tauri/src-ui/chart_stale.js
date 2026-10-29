// What the window says when the record changed under the chart on screen (#797). A refusal that
// says "the screen is behind the record" (`retry: { after_reread }`) re-reads the WHOLE chart, from
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
// stop_reasons.js and sharing the one global scope (`el`, `say`, `setMessage`, `displayedPatient`,
// `refresh`, and the read counters `chartReadRevision`/`drawnRevision`, from main.js). Its functions are
// called only once a chart is read, long after every script has loaded.
"use strict";

/** The chart set the clinician holds (see above); null while no list is drawn. */
let heldCharts = null;

/**
 * Said when a re-read draws after the list was known behind the record, or names a change (#811's
 * M5): `sayAfterReread`.
 */
const READ_AGAIN = "The chart has been read again — review it before you act on it.";

/**
 * Said in the chart's warnings when a re-read FAILED while a stale refusal is still open: the list
 * on screen is the old one. It names the control that is the way out — never "reload", which this
 * window lacks — but never orders a press now: beside a reconnect the server refused, the failed
 * read's line says pressing again fails until an operator acts, and this notice cannot tell (#885).
 */
const LIST_OUT_OF_DATE =
  'This list is out of date: the record changed while it was on screen, and reading it ' +
  'again failed. "Read this chart again" brings it up to date once the node can be read.';

/**
 * Said instead of `LIST_OUT_OF_DATE` when no stale refusal is open — only interrupted commits
 * (#713, `"unknown"`) are behind the list: whether anything changed is not known (#811).
 */
const LIST_MAY_BE_OUT_OF_DATE =
  'This list may be out of date: whether your last act was recorded is not known, and ' +
  'reading it again failed. "Read this chart again" brings it up to date once the node ' +
  'can be read.';

/**
 * Whether a refusal says the screen is behind the record, and what was recorded (Rust's
 * `Recorded`): `"no"` — the record changed since it was read, nothing was recorded — or
 * `"unknown"` — a commit was interrupted (#713), so it MAY have been. `null` when it is not
 * behind. Both values are truthy: every handler re-reads on either. The one spelling of the
 * test, so every handler that re-reads on it asks the same question.
 */
function behindTheRecord(failure) {
  const retry = failure && failure.retry;
  return (retry && retry.after_reread) || null;
}

/**
 * The newest read revision sent BEFORE the window learnt that the list on screen is behind the
 * record (a stale refusal), or null when no list is known to be. Window state, not one read's
 * flag (#797 review): the re-read that refusal sends can be overtaken — by "Read this chart
 * again", or by the draw floor an own act raises (`holdOwnAct`) — and an overtaken read's failure
 * is never said (`isLatestRead`), so the read that overtook it must still mark the list if IT
 * fails. Only a list drawn from a read sent AFTER the refusal ends it: an older read that lands
 * late is the record as it was. Taking the list down ends it too (`holdCharts`).
 */
let behindAfter = null;

/**
 * The newest read revision sent before a STALE refusal (`"no"`), or null: for the out-of-date
 * notice only. A stale refusal PROVES the list on screen is out of date, so a later interrupted
 * commit never weakens it (#811 review) — but only until a list read AFTER it draws, which shows
 * the change it was refused over, even while an interrupted commit is still behind the list. Only
 * this words the notice "is out of date": anything else — an interrupted commit, or a value this
 * window does not know — reads as "may be".
 */
let staleAfter = null;

/**
 * The sentences of the interrupted acts behind the list (Rust's `judgement_error_view`: whether
 * the act was recorded is not known, and a change the re-read names may be the clinician's own).
 * Each is said as its own re-read's lead, but that read can be overtaken and never drawn, so they
 * are window state too (#811 review): the read that ends the state says them (`withOwed`).
 */
let unknownLeads = [];

/**
 * The window learnt the list is behind the record — a refusal (`behindTheRecord`), or a recorded
 * "Different people" with a failed pair that was (link.js, the report's `behind_the_record`):
 * called by main.js's `refresh` before its read takes a revision. `recorded`: `"no"` or
 * `"unknown"`; `lead`: the sentence the re-read is sent after.
 */
function knowListBehind(recorded, lead) {
  behindAfter = chartReadRevision;
  if (recorded === "no") staleAfter = chartReadRevision;
  else if (lead) unknownLeads.push(lead);
}

/** The list is no longer known behind the record: a later read drew, or no list is drawn. */
function forgetBehind() {
  behindAfter = null;
  staleAfter = null;
  unknownLeads = [];
}

/** Whether the list on screen is known to be behind the record (asked by a failed read). */
function listKnownBehind() {
  return behindAfter !== null;
}

/**
 * The read that took `revision` drew a list. Returns whether that list ends a known-stale state —
 * it was read after the refusal — so the outcome line says the chart was read again.
 */
function drewRead(revision) {
  // A list read after the stale refusal shows the change it was refused over (#811 review).
  if (staleAfter !== null && revision > staleAfter) staleAfter = null;
  const ended = behindAfter !== null && revision > behindAfter;
  if (ended) forgetBehind();
  return ended;
}

/** Whether the list on screen is marked out of date (either of `markListStale`'s sentences is up). */
function listMarkedStale() {
  const mark = el("chart-stale").textContent;
  return mark === LIST_OUT_OF_DATE || mark === LIST_MAY_BE_OUT_OF_DATE;
}

/**
 * A read that took `revision` drew its list (main.js's `refresh`); `marked`: the list it replaced
 * was marked out of date. Usually the line is then said (`sayAfterReread`). But an OLDER read can
 * land after a newer one already failed and marked the list (`mayDraw` accepts it: a failure never
 * raises the draw floor) — read before the stale refusal, that list is just as behind the record
 * (#797 fix review). So it is marked again, and the outcome line, still saying the refusal and the
 * failed read, is left as it is: never a known-stale list under a live "Sign off N".
 */
function sayAfterDraw(lead, revision, marked, pane) {
  const owed = unknownLeads; // read first: `drewRead` forgets them when this list ends the state
  const ended = drewRead(revision);
  if (marked && !ended) markListStale();
  else sayAfterReread(ended ? withOwed(lead, owed) : lead, ended, pane);
}

/**
 * `lead`, then each interrupted act's sentence (`unknownLeads`) it does not already say (pure). The
 * act's own re-read has its sentence as the lead, so it is said once; a read that overtook it
 * says it too, so a change the clinician's own act made is never named as another source's.
 */
function withOwed(lead, owed) {
  const line = lead || "";
  return sentences(line, ...owed.filter((s) => !line.includes(s)));
}

/**
 * The outcome line after a re-read was DRAWN (called through `sayAfterDraw` only over an answer
 * `mayDraw` accepted, so an overtaken read says nothing). `readAgain`: this list ended a stale
 * refusal (`drewRead`). If it did, OR it names a change, say the lead, that the chart was read
 * again — review it — and what changed: a change is news to review after the clinician's own act
 * too (#811's M5). (After an interrupted Link or Unlink, the lead — Rust's sentence — already says
 * a named change may be the clinician's own, #811's M4.) "Read again" is said once per line: the
 * lead of "Read this chart again" is the line as it stands, which may already say it. Otherwise
 * the line is left alone — it may be saying how a signed act went — unless it still says a read
 * FAILED, which this drawn read has just made untrue: it goes back to its lead (what was said
 * before the failure, possibly nothing).
 */
function sayAfterReread(lead, readAgain, pane) {
  const again = lead && lead.includes(READ_AGAIN) ? null : READ_AGAIN;
  if (readAgain || pane.set_change) say(sentences(lead, again, pane.set_change));
  else if (showsFailedRead(el("outcome").textContent)) say(sentences(lead));
}

/**
 * The last failed chart read said on the outcome line: `{ line, lead }`, the line as said and
 * the lead it was said after. Compared by its TEXT (`showsFailedRead`), so once anything else
 * is said on the line it no longer applies, and nothing has to remember to clear it.
 */
let failedRead = null;

/** A chart read failed: say so after `lead` (main.js's `refresh`), and remember the line. */
function sayReadFailed(lead, error) {
  const line = sentences(lead, "Could not read the chart: " + error);
  say(line);
  failedRead = { line, lead };
}

/** Whether `line` is the failed read last said — so the outcome line still says it. */
function showsFailedRead(line) {
  return failedRead !== null && failedRead.line === line;
}

/**
 * The lead "Read this chart again" re-reads with: the outcome line as it stands, so a change the
 * re-read names is ADDED to it and a partial sign-off report ("Signed 2 …; 1 line(s) could NOT
 * be signed …") is never replaced (ADR-0060: partial completion is reported, never implied).
 * Less a read failure the press is retrying: then the lead is what that failure was said after,
 * so pressing again never piles one "Could not read the chart" onto another.
 */
function rereadLead() {
  const line = el("outcome").textContent;
  return showsFailedRead(line) ? failedRead.lead : line;
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

/**
 * Set the held set to what was just drawn (`charts`), or to null when nothing is drawn — and then
 * no list is on screen to be behind the record either.
 */
function holdCharts(charts) {
  heldCharts = charts;
  if (charts === null) forgetBehind();
}

/**
 * The clinician's own act was RECORDED: move the held set by that act's own change, the moment
 * the report lands and before any re-read is sent — so EVERY later re-read (this act's, a stale
 * Stop's, "Read this chart again") finds it already held and stays silent about it, while a
 * colleague's change still differs and is named. `touched`: the charts the act was sent over;
 * `after`: the report's `charts_after`, read inside the act's own transaction under the link lock
 * — after the node checked `touched`, so it is the act's result and nothing foreign.
 *
 * When the held set MOVES, the draw floor is raised too, as `clearChart` does (fix round 2): a
 * re-read sent BEFORE the act (a sign-off's, "Read this chart again") was answered by Rust against
 * the pre-act set, so drawn now it would name the clinician's own change and overwrite the act's
 * outcome. The act's own re-read is sent after this and is newer, so it draws. Only when the set
 * moved: an act that changed nothing must not drop a colleague's read already in flight.
 */
function holdOwnAct(touched, after) {
  const moved = applyOwnAct(heldCharts, touched, after, displayedPatient);
  if (!sameCharts(moved, heldCharts)) {
    heldCharts = moved;
    drawnRevision = ++chartReadRevision;
  }
}

/** Whether two chart sets (or two nulls) hold the same charts, in any order (pure). */
function sameCharts(a, b) {
  if (a === null || b === null) return a === b;
  return ordered(a).join() === ordered(b).join();
}

/**
 * Pure: the held set `held` after an own act over `touched` that left `after` as the record of the
 * chart it was signed on, for a window showing chart `anchor` (always a member of `held` — the held
 * set is the record of the chart on screen). Every result is a sorted, de-duplicated list.
 *
 * - No held set (nothing drawn) or no recorded set (`after` null: "Different people", an act
 *   that changed nothing — Outranked — or a set that is not the record on screen) → `held` as
 *   it is.
 * - The anchor ended in `after` (a link; an unlink seen from its own side) → the touched charts
 *   are replaced by `after`; held charts the act never touched stay.
 * - The anchor was touched but ended outside `after` (an unlink answered while the clinician is on
 *   the OTHER side) → the touched charts are replaced by what the act left on that side.
 * - The act never touched the anchor → `held` unchanged: an own act never silences anything else.
 *
 * Anchored, rather than `(held − (sent − after)) ∪ (after − sent)`: that form cannot tell which
 * side of the act the window is on, so a Link answered while the other chart was already drawn
 * would announce the clinician's own link back to them (fix round 1 report).
 *
 * The cases it was walked over (headless, a stubbed `invoke`; there is no JS runner, #332):
 *   held      touched  after   anchor → result   the case
 *   null      any      any     any    → null     nothing drawn
 *   {A}       {A,B}    null    A      → {A}      no recorded set (Different people, or not this record)
 *   {A}       {A,B}    {A,B}   A      → {A,B}    own Link from A: silent
 *   {B}       {A,B}    {A,B}   B      → {A,B}    Link answered on B, drawn before its commit: silent
 *   {A,B,C}   {A,B}    {A,B}   B      → {A,B,C}  Link answered on B, drawn after a colleague's C
 *   {A,B}     {A,B}    {A}     A      → {A}      own Unlink seen from A: silent
 *   {A,B}     {A,B}    {A}     B      → {B}      Unlink answered on the other side: silent
 *   {C}       {A,B}    {A,B}   C      → {C}      act never touched the chart on screen: unchanged
 * The unanchored formula fails the fourth row: it keeps {B} and names A, the clinician's own link.
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

// The always-visible "Read this chart again" button (index.html): the way out of any read that
// failed. It sends the held set like every other re-read, and keeps what the outcome line says
// (`rereadLead`). (Wired here, not in main.js, which is at its line budget.)
el("reread").addEventListener("click", () => void refresh(rereadLead()));

/**
 * A re-read failed while the list is known behind the record (a stale refusal, or an interrupted
 * commit): say so above the table and take sign-off away, with its reason — "is" out of date only
 * while a stale refusal proves it (`staleAfter`), "may be" otherwise, on the notice and the button
 * alike (#811). The rows stay — the clinician can still read what they last saw, now flagged. The
 * next list that draws (`drawList`) or is taken down (`clearDrawnList`) clears the warning.
 */
function markListStale() {
  const proven = staleAfter !== null;
  setMessage(el("chart-stale"), proven ? LIST_OUT_OF_DATE : LIST_MAY_BE_OUT_OF_DATE);
  el("chart-warnings").hidden = false;
  const button = el("sign-off");
  button.disabled = true;
  button.textContent = proven
    ? "Nothing to sign off — this list is out of date"
    : "Nothing to sign off — this list may be out of date";
}
