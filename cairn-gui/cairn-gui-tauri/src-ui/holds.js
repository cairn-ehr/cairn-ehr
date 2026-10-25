// The paper chart's "W" on a medication line (#832, ADR-0081): the hold drawn beside the line's
// status, the reason it reveals, and the two gestures — Withhold (write a W) and Release (end the
// W early). A ceased line's reason, or a current line's stopped member's reason (#861), is revealed
// the same way, on its status word.
//
// Like main.js, this file WORDS nothing. Every sentence — the W's symbol ("W until reviewed"), its
// detail ("Withheld until reviewed by … : BP 85/50"), whether a line may be held or released, the
// instant the list stops being true, and every outcome — is worded in Rust (cairn-gui-tab-
// medications' `hold_view`, the `hold` command module) and tested there. What IS decided here,
// each pinned in holds_webview_tests.rs: turning the clinician's `datetime-local` value into an
// ISO instant (only the browser knows the field's zone); keeping the until select and the time
// field in step (a typed time picks "until a time", "until reviewed" empties the time); disabling
// a pressed button until its gesture is answered; and re-reading the chart when a hold starts or
// ends (`rereadAtNextHoldChange`).
//
// A separate file only to keep main.js small (it must stay under 500 lines). Classic script,
// loaded right after stop_reasons.js and sharing main.js's global scope (`el`, `cell`, `say`,
// `invoke`, `refresh`, `renderedPatient`, `renderedCharts`, funnel.js's `failureText` and
// chart_stale.js's `behindTheRecord` and `rereadLead`, which are read only when a gesture or the
// re-read timer runs, long after every script has loaded). main.js's `renderRow` calls
// `statusCell` and `holdControls`; its `drawList` calls `rereadAtNextHoldChange`.
"use strict";

/** A `datetime-local` value: "2026-10-21T20:00", seconds optional. */
const LOCAL_TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?$/;
/** setTimeout fires AT ONCE past ~24.8 days; a farther boundary is waited for a day at a time. */
const DAY_MS = 24 * 60 * 60 * 1000;
/** After a failed boundary re-read, the next try. */
const HOLD_REREAD_RETRY_MS = 60 * 1000;
let holdChangeTimer = null;

/**
 * A row's status cell: the status word, then one W button per hold. Each button's accessible
 * name is its visible text then its detail ("W until reviewed — Withheld until reviewed by …"),
 * so a voice user can say what they see and a screen reader hears WHY on reaching it. The detail
 * is also revealed beside it on focus or hover, and stays open once clicked ("pinned", kept on
 * the button as `data-pinned`) until clicked again, Escape, or a click elsewhere (`unpinDetails`).
 * Click must work alone: WebKit (Tauri's webview on macOS) does not focus a button a mouse
 * clicks. On a line with a stop reason to reveal (`row.cessation_detail`: a ceased line's, or a
 * current line's stopped member's, #861), the status word is such a button.
 */
function statusCell(row) {
  const td = document.createElement("td");
  // One revealing button and the detail it controls; returns both, to sit side by side.
  const reveal = (text, label, shown, id) => {
    const button = document.createElement("button");
    button.type = "button";
    button.className = "hold-symbol";
    button.textContent = text;
    button.setAttribute("aria-label", label);
    button.setAttribute("aria-controls", id);
    const detail = cell("span", shown, { id: id, class: "hold-detail" });
    detail.hidden = true;
    const pinned = () => button.dataset.pinned === "true";
    const show = (open) => {
      detail.hidden = !open;
      button.setAttribute("aria-expanded", String(open));
    };
    show(false);
    button.addEventListener("focus", () => show(true));
    button.addEventListener("mouseenter", () => show(true));
    button.addEventListener("blur", () => show(pinned()));
    button.addEventListener("mouseleave", () => show(pinned()));
    button.addEventListener("click", () => {
      button.dataset.pinned = String(!pinned());
      show(pinned());
    });
    return [button, detail];
  };
  if (row.cessation_detail) {
    const label = row.status_label + " — " + row.cessation_detail;
    td.append(...reveal(row.status_label, label, row.cessation_detail, "cessation-detail-" + row.group_id));
  } else {
    td.append(row.status_label);
  }
  for (const hold of row.holds) {
    td.append(
      " ",
      ...reveal(hold.symbol, hold.symbol + " — " + hold.detail, hold.detail, "hold-detail-" + hold.withhold_event_id),
    );
  }
  return td;
}

/**
 * Put away every revealed detail but the one `target` is in (null: all of them) — Escape, or a
 * click outside it. A pinned detail sits over the cells beside it (style.css), so it can cover the
 * line's own Stop/Withhold/Release and the line under it; it must never be in the way for long.
 */
function unpinDetails(target) {
  for (const button of document.querySelectorAll('.hold-symbol[aria-expanded="true"]')) {
    const detail = document.getElementById(button.getAttribute("aria-controls"));
    if (target && (button.contains(target) || detail.contains(target))) continue;
    button.dataset.pinned = "false";
    detail.hidden = true;
    button.setAttribute("aria-expanded", "false");
  }
}

// Registered ONCE, here at load — not in `statusCell`, which runs on every redraw and would stack
// a listener per row per read. A click on a W itself reaches its own handler first (it toggles
// its pin), and is skipped here because the target is inside it.
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") unpinDetails(null);
});
document.addEventListener("click", (event) => unpinDetails(event.target));

/**
 * The Withhold and Release controls for a row's action cell, beside Stop — inline, never behind
 * a dialog (principle 3): writing a W and its reason is one act on paper, and is one here.
 * Withhold needs a reason and an end: "until reviewed" (the default, the open-ended paper W) or
 * "until a time". Release's reason is optional. Release sends the ids of the holds THIS row drew,
 * so it can never end a hold the clinician was not shown (ADR-0081 decision 4).
 */
function holdControls(row) {
  const nodes = [];
  if (row.can_withhold) {
    const whyHold = document.createElement("input");
    whyHold.type = "text";
    whyHold.id = "reason-w-" + row.group_id;
    whyHold.setAttribute("aria-label", "Reason for withholding " + row.primary);
    whyHold.placeholder = "W reason";

    const untilKind = document.createElement("select");
    untilKind.id = "until-kind-" + row.group_id;
    untilKind.setAttribute("aria-label", "Withhold " + row.primary + " until");
    for (const [value, text] of [["reviewed", "until reviewed"], ["time", "until a time"]]) {
      untilKind.append(cell("option", text, { value: value }));
    }

    const untilAt = document.createElement("input");
    untilAt.type = "datetime-local";
    untilAt.id = "until-at-" + row.group_id;
    untilAt.setAttribute("aria-label", "Withhold " + row.primary + " until (local time)");
    // Typing a time IS choosing "until a time": a time left under "until reviewed" would be
    // dropped without a word, and the W written open-ended.
    untilAt.addEventListener("input", () => {
      untilKind.value = "time";
    });
    // ...and choosing "until reviewed" back means it: the typed (or partly typed) time is emptied,
    // or the press would still send it and the W lapse silently at that hour.
    untilKind.addEventListener("change", () => {
      if (untilKind.value === "reviewed") {
        untilAt.value = "";
        // WebKit (Tauri's macOS webview) keeps a PARTLY typed time through `value = ""`;
        // resetting the type discards it.
        untilAt.type = "text";
        untilAt.type = "datetime-local";
      }
    });

    // Not named `hold`: that name is a HoldView here, and the drift guard reads `hold.<field>`.
    const holdButton = document.createElement("button");
    holdButton.type = "button";
    holdButton.id = "withhold-" + row.group_id;
    holdButton.textContent = "Withhold";
    holdButton.setAttribute("aria-label", "Withhold " + row.primary);
    holdButton.addEventListener("click", () => {
      // One press, one W: a second press while this one is in flight would read the same
      // fields and write a second W. The re-read after the gesture draws a fresh button; a
      // refused press (nothing written) re-enables this one, as that re-read may have failed —
      // unless the refusal proved the list stale, when a retry would only be refused again.
      holdButton.disabled = true;
      // A partly typed time (a date, no hour) has value "" and fired no `input`, but it is still
      // a time the clinician started — never "until reviewed". Sent as "", Rust refuses it.
      const kind = (untilAt.value || untilAt.validity.badInput) ? "time" : untilKind.value;
      // ...and the select says so, so the re-read after a refusal keeps "until a time" (#799's
      // restore) and a second press is refused again, never an open-ended W (#841).
      untilKind.value = kind;
      void withhold(row.group_id, whyHold.value, kind, untilAt.value).then((spent) => {
        if (!spent) holdButton.disabled = false;
      });
    });

    nodes.push(whyHold, untilKind, untilAt, holdButton);
  }
  if (row.can_release) {
    const whyRelease = document.createElement("input");
    whyRelease.type = "text";
    whyRelease.id = "reason-r-" + row.group_id;
    whyRelease.setAttribute("aria-label", "Reason for releasing " + row.primary);
    whyRelease.placeholder = "release reason (optional)";

    const releaseButton = document.createElement("button");
    releaseButton.type = "button";
    releaseButton.id = "release-" + row.group_id;
    releaseButton.textContent = "Release";
    releaseButton.setAttribute("aria-label", "Release the holds on " + row.primary);
    releaseButton.addEventListener("click", () => {
      releaseButton.disabled = true; // one press, one release, as Withhold
      const sent = release(row.group_id, row.holds.map((hold) => hold.withhold_event_id), whyRelease.value);
      void sent.then((spent) => {
        if (!spent) releaseButton.disabled = false;
      });
    });

    nodes.push(whyRelease, releaseButton);
  }
  return nodes;
}

/**
 * Write a W. `kind` is the until choice; `local` the `datetime-local` value, read in the
 * workstation's zone and sent as an ISO instant. An empty or unreadable time is sent as "" so
 * Rust words the refusal ("the end time is missing or incomplete …") — never turned into
 * "until reviewed".
 * The rest is `cease`'s shape exactly: say the outcome, then re-read the chart. Resolves true
 * when the press is spent and its button must stay down — a report came back (the W recorded,
 * or not known), or a refusal proved the list stale (the re-read draws a fresh button) — and
 * false when a refusal leaves the same press worth retrying.
 */
async function withhold(groupId, reason, kind, local) {
  // Only the field's own shape: a webview that draws `datetime-local` as a plain text field would
  // hand `new Date` "02/03/2026 08:00", which it reads month-first — the wrong end, silently.
  const at = LOCAL_TIME.test(local) ? new Date(local) : NaN;
  const until = kind === "reviewed" ? "reviewed" : isNaN(at) ? "" : at.toISOString();
  let report;
  try {
    report = await invoke("withhold", {
      groupId: groupId,
      reason: reason,
      until: until,
      patientId: renderedPatient,
      charts: renderedCharts,
    });
  } catch (failure) {
    say("Could not withhold this drug: " + failureText(failure));
    await refresh(el("outcome").textContent, behindTheRecord(failure));
    // The fields stay as typed, so the clinician can correct and retry — but a stale refusal
    // keeps the button down: never a live retry over a list known to be behind the record.
    return behindTheRecord(failure) !== null;
  }
  // A report came back (recorded, or not known): clear what was typed for this W BEFORE the
  // re-read, or #799's restore would put it back and a second press write a second W.
  setField("reason-w-" + groupId, "");
  setField("until-kind-" + groupId, "reviewed");
  setField("until-at-" + groupId, "");
  say(report.text); // worded in Rust: `hold::outcome::withhold_outcome`
  await refresh(el("outcome").textContent, report.behind_the_record);
  return true;
}

/** End the holds `withholds` (the ids a row drew) early. `cease`'s shape, as `withhold`. */
async function release(groupId, withholds, reason) {
  let report;
  try {
    report = await invoke("release", {
      groupId: groupId,
      withholds: withholds,
      reason: reason,
      patientId: renderedPatient,
      charts: renderedCharts,
    });
  } catch (failure) {
    say("Could not release this hold: " + failureText(failure));
    await refresh(el("outcome").textContent, behindTheRecord(failure));
    // The reason stays as typed; the button comes back unless the list is stale, as `withhold`.
    return behindTheRecord(failure) !== null;
  }
  setField("reason-r-" + groupId, ""); // as `withhold`: cleared before the re-read
  say(report.text); // worded in Rust: `hold::outcome::release_outcome`
  await refresh(el("outcome").textContent, report.behind_the_record);
  return true;
}

/**
 * Set field `id` of a line to `value`, if the line still has it: a re-read that landed while the
 * gesture was in flight may have redrawn the line without it (a drug now ceased). Never throws —
 * the gesture has already returned, and the re-read after it must still run.
 */
function setField(id, value) {
  const field = el(id);
  if (field) field.value = value;
}

/**
 * Re-read the chart at `next` (the view's `next_hold_change`: when a hold on it starts or ends),
 * while `patient`'s list is the one drawn. A hold changes with the clock, not with an event, so
 * without this a line kept saying "withheld" after its W ended — or "current" after a pending W
 * began — until someone happened to re-read (#838 review). Called by every list drawn, which
 * replaces the previous timer. A failed re-read drew nothing, so nothing re-armed: the screen is
 * past the boundary and still shows what was true before it, so it tries again each minute.
 */
function rereadAtNextHoldChange(next, patient) {
  clearTimeout(holdChangeTimer);
  holdChangeTimer = null;
  if (!next) return;
  // +1 s: the node's clock must be past the instant too when the read reaches it.
  const wait = Math.min(Math.max(0, Date.parse(next) - Date.now()) + 1000, DAY_MS);
  const armed = setTimeout(async () => {
    if (renderedPatient !== patient) return;
    await refresh(rereadLead());
    if (holdChangeTimer === armed && renderedPatient === patient) {
      rereadAtNextHoldChange(new Date(Date.now() + HOLD_REREAD_RETRY_MS).toISOString(), patient);
    }
  }, wait);
  holdChangeTimer = armed;
}
