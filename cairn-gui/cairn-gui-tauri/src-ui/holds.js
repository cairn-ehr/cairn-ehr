// The paper chart's "W" on a medication line (#832, ADR-0081): the hold drawn beside the line's
// status, the reason it reveals, and the two gestures — Withhold (write a W) and Release (end the
// W early). A ceased line's reason is revealed the same way, on its status word.
//
// Like main.js, this file renders and decides nothing. Every sentence — the W's symbol ("W until
// reviewed"), its detail ("Withheld until reviewed by … : BP 85/50"), whether a line may be held
// or released, and every outcome — is worded in Rust (cairn-gui-tab-medications' `hold_view`, the
// `hold` command module) and tested there. The one thing done here is turning the clinician's
// `datetime-local` value into an ISO instant, because only the browser knows the field's zone.
//
// A separate file only to keep main.js small (it must stay under 500 lines). Classic script,
// loaded right after stop_reasons.js and sharing main.js's global scope (`el`, `cell`, `say`,
// `refresh`, `renderedPatient`, `renderedCharts`, and funnel.js's `failureText` and
// chart_stale.js's `behindTheRecord`, which are read only when a gesture runs, long after every
// script has loaded). main.js's `renderRow` calls `statusCell` and `holdControls`.
"use strict";

/**
 * A row's status cell: the status word, then one W button per hold. Each button's accessible
 * name is its visible text then its detail ("W until reviewed — Withheld until reviewed by …"),
 * so a voice user can say what they see and a screen reader hears WHY on reaching it. The detail
 * is also revealed beside it on focus or hover, and stays open once clicked ("pinned", kept on
 * the button as `data-pinned`) until clicked again, Escape, or a click elsewhere (`unpinDetails`).
 * Click must work alone: WebKit (Tauri's webview on macOS) does not focus a button a mouse
 * clicks. On a ceased line that gave a reason, the word "ceased" is such a button.
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
      // fields and write a second W. The re-read after the gesture draws a fresh button.
      holdButton.disabled = true;
      // A partly typed time (a date, no hour) has value "" and fired no `input`, but it is still
      // a time the clinician started — never "until reviewed". Sent as "", Rust refuses it.
      const kind = (untilAt.value || untilAt.validity.badInput) ? "time" : untilKind.value;
      withhold(row.group_id, whyHold.value, kind, untilAt.value);
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
      release(row.group_id, row.holds.map((hold) => hold.withhold_event_id), whyRelease.value);
    });

    nodes.push(whyRelease, releaseButton);
  }
  return nodes;
}

/**
 * Write a W. `kind` is the until choice; `local` the `datetime-local` value, read in the
 * workstation's zone and sent as an ISO instant. An empty or unreadable time is sent as "" so
 * Rust words the refusal ("choose when the hold ends") — never turned into "until reviewed".
 * The rest is `cease`'s shape exactly: say the outcome, then re-read the chart.
 */
async function withhold(groupId, reason, kind, local) {
  const at = new Date(local);
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
    return; // the fields stay as typed, so the clinician can correct and retry
  }
  // A report came back (recorded, or not known): clear what was typed for this W BEFORE the
  // re-read, or #799's restore would put it back and a second press write a second W.
  setField("reason-w-" + groupId, "");
  setField("until-kind-" + groupId, "reviewed");
  setField("until-at-" + groupId, "");
  say(report.text); // worded in Rust: `hold::outcome::withhold_outcome`
  await refresh(el("outcome").textContent, report.behind_the_record);
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
    return; // the reason stays as typed, so the clinician can retry
  }
  setField("reason-r-" + groupId, ""); // as `withhold`: cleared before the re-read
  say(report.text); // worded in Rust: `hold::outcome::release_outcome`
  await refresh(el("outcome").textContent, report.behind_the_record);
}

/**
 * Set field `id` of a line to `value`, if the line still has it: a re-read that landed while the
 * gesture was in flight may have redrawn the line without it (a drug now ceased). Never throws —
 * the write has already landed, and the re-read after it must still run.
 */
function setField(id, value) {
  const field = el(id);
  if (field) field.value = value;
}
