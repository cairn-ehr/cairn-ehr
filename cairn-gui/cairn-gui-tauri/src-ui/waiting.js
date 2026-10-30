// The waiting bar (#875 option 2): a command still unanswered after WAIT_SHOWN_AFTER_MS shows ONE
// window-wide bar that empties towards NODE_ANSWERS_WITHIN_MS, the moment the window's
// connection cuts a stuck command off (`NodeConnection::ANSWER_WITHIN`). Nearly every command
// answers long before the bar would appear, so it is rarely seen, and never flashes.
//
// Loaded BEFORE main.js, which sends every command through `watchWaits(...)`: one wrapper, so
// no call site can forget it. The bar decides nothing; the answer, when it comes, is drawn by
// the command's own caller as before.
"use strict";

/** How long a command may go unanswered before the bar shows (ms): the maintainer's 2 s. */
const WAIT_SHOWN_AFTER_MS = 2_000;

/**
 * When the window's connection cuts a stuck command off (ms). MUST equal
 * `NodeConnection::ANSWER_WITHIN` (pinned by `waiting_webview_tests.rs`).
 */
const NODE_ANSWERS_WITHIN_MS = 15_000;

/** One entry per command still waiting, holding when it was sent. */
const pendingWaits = new Set();

/** The oldest wait the bar is drawn for (or null), so a redraw for it does not restart it. */
let drawnWait = null;

/** The timer that turns the bar's line to "Reconnecting…" when it runs out. */
let runOutTimer = null;

/** The timer that draws the bar for a wait not yet at the delay (see `drawWait`). */
let drawAgainTimer = null;

/**
 * `send` (Tauri's invoke), with every call it makes counted as a wait. The bar is drawn 2 s
 * after a call if it is still pending, and redrawn whenever one ends.
 */
function watchWaits(send) {
  return async (...args) => {
    const wait = { sentAt: performance.now() };
    pendingWaits.add(wait);
    const showTimer = setTimeout(drawWait, WAIT_SHOWN_AFTER_MS);
    try {
      return await send(...args);
    } finally {
      clearTimeout(showTimer);
      pendingWaits.delete(wait);
      drawWait();
    }
  };
}

/** The pending wait sent first, or null. Pure over `waits`. */
function oldestWait(waits) {
  let oldest = null;
  for (const wait of waits) {
    if (oldest === null || wait.sentAt < oldest.sentAt) oldest = wait;
  }
  return oldest;
}

/**
 * Show the bar for the oldest wait once it has run WAIT_SHOWN_AFTER_MS, or hide it. The bar's
 * fill empties over what is left of NODE_ANSWERS_WITHIN_MS since that wait was SENT. A press
 * queued behind a stuck one can wait past it (the cut-off, then up to 5 s of reopen), so a bar
 * that runs out says "Reconnecting…" and stays until the answer, rather than lying.
 */
function drawWait() {
  const bar = document.getElementById("node-wait");
  const line = document.getElementById("node-wait-line");
  const fill = document.getElementById("node-wait-fill");
  const oldest = oldestWait(pendingWaits);
  const waited = oldest === null ? 0 : performance.now() - oldest.sentAt;
  clearTimeout(drawAgainTimer);
  if (oldest === null || waited < WAIT_SHOWN_AFTER_MS) {
    bar.hidden = true;
    drawnWait = null;
    clearTimeout(runOutTimer);
    // Come back when the oldest wait reaches the delay. Its own timer may have fired a
    // millisecond early, and nothing else would draw the bar until another call ended
    // (PR #892 review).
    if (oldest !== null) drawAgainTimer = setTimeout(drawWait, WAIT_SHOWN_AFTER_MS - waited);
    return;
  }
  if (oldest === drawnWait) return;
  drawnWait = oldest;
  const left = Math.max(0, NODE_ANSWERS_WITHIN_MS - waited);
  line.textContent = "Waiting for the node…";
  // Start full, then empty linearly over what is left: one transition, no ticking count.
  fill.style.transition = "none";
  fill.style.width = `${(100 * left) / NODE_ANSWERS_WITHIN_MS}%`;
  bar.hidden = false;
  void fill.offsetWidth; // apply the start width before the transition begins
  fill.style.transition = `width ${left}ms linear`;
  fill.style.width = "0%";
  clearTimeout(runOutTimer);
  runOutTimer = setTimeout(() => {
    line.textContent = "Reconnecting…";
  }, left);
}
