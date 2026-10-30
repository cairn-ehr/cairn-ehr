# #875 option 2 — the window's connection answers within 15 s, and a bar shows the wait

**Goal:** a query the window sends on a link that died silently can no longer hang the window. Before this, it held
the window's ONE connection until macOS gave up on the retransmits (minutes), and every other chart and funnel
command queued behind it. Agreed with the maintainer in the 2026-10-30 brainstorm: a 15 s limit, and a waiting bar
that appears only after 2 s. (A workstation's window normally talks to its OWN node on the same machine,
topology.md; the case this bounds is a thin client attached to a nearby node. The by-patient sync lane raised in the
same brainstorm is #891.)

**Architecture:**
- **A watchdog per hold.** `NodeConnection::lock` and `lock_if_open` return a `ConnectionGuard` (derefs to
  `Client`, so no caller changes). Taking it starts a timer; dropping it cancels the timer. If the guard is still
  held after `NodeConnection::ANSWER_WITHIN` (15 s), the watchdog aborts the client's connection TASK. The stuck
  query then fails `connection closed`, exactly as after a server hang-up, so every existing path applies unchanged:
  - a cut-off COMMIT is marked unknown by `cairn_node::db_diagnosis::commit_failure` (no ERROR answer);
  - the client is now `is_closed()`, so the next `lock` reopens it (#807's reopen and backoff).
- **The task's handle.** `cairn_node::db::connect_with_task` returns the client and its connection task's
  `AbortHandle`; `connect` is it with the handle dropped (the CLI and sync are untouched).
  `NodeConnection::reopening` takes the handle; a reopen stores the fresh one. A `fixed` connection has no handle and
  no watchdog: aborting it would kill it for good.
- **Launch.** `build_live_state` loads the schema on one connection (identity read there), drops it, and opens the
  window's own through `connect_with_task`. One extra connect at launch.
- **The bar (webview).** `main.js` wraps the one `invoke`: a call still pending after `WAIT_SHOWN_AFTER_MS` (2000)
  shows one window-wide bar, "Waiting for the node…", emptying towards `NODE_ANSWERS_WITHIN_MS` (15000) from the
  OLDEST pending call. When it runs out it reads "Reconnecting…" and stays until the answer: a press queued behind a
  stuck one can wait up to 15 s + the 5 s reopen. No ticking seconds; one polite live-region announcement.

**Tests (TDD):**
- DB-gated (`tests/answer_limit.rs`), with a short limit: a `pg_sleep` past it is cut off and RELEASES the guard
  (a second lock gets through), the error is a closed connection, the next lock reads on a NEW backend; a COMMIT cut
  off mid-flight (a deferred constraint trigger that sleeps) is marked unknown by `commit_failure`, and the row
  lands anyway — "not known", never "failed". A guard dropped in time leaves the connection open.
- The `ConnectionGuard` stays `Send` (the existing `the_guard_may_be_held_across_an_await` pin).
- Source guards (`waiting_webview_tests.rs`): `NODE_ANSWERS_WITHIN_MS` equals `ANSWER_WITHIN` and the delay is
  2000; every script but main.js leaves `__TAURI__` alone and main.js binds `invoke` through `watchWaits`;
  `waiting.js` loads first; the bar is drawn only by the delayed timer; the line is a live region and the bar
  `aria-hidden`. `every_command_the_webview_calls_is_registered` scans `waiting.js` too.
- **As built, beyond the plan:** the JS has no runner (#332) and the browser tool was down, so the bar's logic
  was walked once in Node with a stubbed DOM and timings scaled down ten times (fast answer, the delay,
  "Reconnecting…", a newer answer leaving an older wait's bar, oldest-first redraw). Throwaway, not committed;
  its look and VoiceOver are RUNBOOK §14 (d).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** you ring the ward clerk for an old chart. If nobody picks up you know at once and try
  another way; you are never left holding a silent line with no idea whether anyone is coming.
- **Steps:** paper **N = 1** (notice there is no answer, act on it). Architecture-forced **M = 1**: read the bar's
  line, or the lost-connection line once it is cut off, then press again. UI bundling target **K = 1**. Nothing is
  added to an action that answers within 2 s, which is nearly all of them. M ≤ N.
- **Time + cognitive load:**
  - Budget: a dead node is said within 15 s of the press (20 s for a press queued behind a stuck one), against an
    unbounded hang before. The bar never appears for an answer inside 2 s.
  - Load drops: a frozen window becomes a stated wait with a visible end.
  - The measurement (the bar's look, VoiceOver's single announcement, a pulled cable to a remote node) belongs to
    RUNBOOK §14's human pass. It cannot be measured headlessly.
