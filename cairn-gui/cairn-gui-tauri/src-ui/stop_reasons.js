// The stop reasons a clinician has typed but not yet sent, carried across a chart re-read
// (#799). main.js's `drawList` rebuilds every medication row on each read, and a re-read happens
// after a link, an unlink, a sign-off, a judgement or another drug's cease — none of which the
// clinician is necessarily doing on the line they were typing on. Re-typing a reason is work a
// paper chart never loses (§1.2), so `drawList` reads them out with `typedReasons` before it
// empties the rows and puts them back with `restoreReasons` after it rebuilds them.
//
// It also words what a stop did (`ceaseOutcome`, for main.js's `cease`).
//
// Classic script, loaded right after main.js and sharing its global scope (`el`). Its functions
// are called only when a chart is drawn or a drug stopped, long after every script has loaded.
"use strict";

/**
 * The stop reasons typed but not yet sent (#799), read just before a re-read rebuilds the rows:
 * each non-empty `#reason-<group_id>` field's id and text, and the one being typed in, with its
 * caret. A re-read (after a link, a sign-off, a judgement, another drug's cease) used to wipe them.
 */
function typedReasons() {
  const fields = [...el("med-rows").querySelectorAll('input[id^="reason-"]')];
  const active = document.activeElement;
  const focused = fields.includes(active)
    ? { id: active.id, start: active.selectionStart, end: active.selectionEnd }
    : null;
  const values = fields
    .filter((field) => field.value !== "")
    .map((field) => ({ id: field.id, value: field.value }));
  return { values, focused };
}

/**
 * Put `typedReasons()`'s answer back into the rebuilt rows. A reason goes back only where its
 * line survived the re-read (its `group_id` still has a stop field); a drug that is now stopped,
 * or no longer on the list, has no field, so its reason goes with it.
 */
function restoreReasons(typed) {
  for (const { id, value } of typed.values) {
    const field = document.getElementById(id);
    if (field) field.value = value;
  }
  const focused = typed.focused && document.getElementById(typed.focused.id);
  if (focused) {
    focused.focus();
    focused.setSelectionRange(typed.focused.start, typed.focused.end);
  }
}

/**
 * What one cease did, as the outcome line says it (ADR-0060: partial completion is reported,
 * never implied): the threads stopped, those NOT stopped, and those whose commit was lost
 * mid-flight (#814, Rust's `unknown_message`). "Stopped 0" is left out when a thread's outcome is
 * unknown: that thread may be stopped, so the count would be false.
 */
function ceaseOutcome(report) {
  const parts = [];
  if (report.ceased > 0 || !report.unknown_message) {
    parts.push("Stopped " + report.ceased + " thread(s) of this drug.");
  }
  if (report.failed.length > 0) {
    const notStopped = " thread(s) were NOT stopped and are still active: ";
    parts.push(report.failed.length + notStopped + report.failed.join("; "));
  }
  if (report.unknown_message) parts.push(report.unknown_message);
  return parts.join(" ");
}
