# #909 slice C1 — an upgrade disturbs no one — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An older window keeps working against a newer database above a stamped floor, updates itself at the idle
re-lock or on **Update now**, and no upgrade's DDL stalls a clinical read or write past a few seconds.

**Architecture:** A floor ledger beside `SCHEMA_GENERATION` is stamped into `node_schema` (db/085). A pure verdict
(`window_floor.rs`) replaces the window's exact match; the window polls it, gates new charts below the floor, and
restarts into an installed program only if that program fits. The loader replays each file under a `lock_timeout`
with retry; the terminal service exits for its supervisor on a stamp change. A catalogue snapshot keeps the
window's reachable doors additive.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri, Tauri 2 `AppHandle::restart`), PostgreSQL 18 SQL,
plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-24-909-slice-c1-rollout-design.md`](../specs/2026-11-24-909-slice-c1-rollout-design.md)

## Global Constraints

- Every doubt resolves toward refusing (NULL floor = exact generation), as design §4.1.
- No new crate. No confirmation dialog (house rule; design §5 anti-goals).
- Copy, verbatim (design §4.4/§4.5):
  - calm line: `An update is waiting; it installs when this terminal is next idle.` with a button `Update now`
  - `No newer version is installed on this terminal yet — ask whoever installs Cairn here.`
  - `The version installed here does not fit this node yet.`
  - below the floor: `This terminal needs its update: it does not show <reason>. Finish what you are doing.`
  - a refused open/register below the floor (my call): `This terminal needs its update before it can open another chart.`
- Files stay under 500 lines; `db.rs` (1372) and `connection.rs` (650, #1164) gain no bulk — new logic goes in new
  modules.
- `SCHEMA_GENERATION` becomes **85** (db/085); every later generation adds a ledger entry (Task 1's guard).
- DB-free `cargo test` needs `CAIRN_ALLOW_DB_SKIP=1`; DB-gated tests per `docs/developers` (RUNBOOK §0's
  `scram-sha-256` cluster for the window launch). Never `| tail` a gate.

## Plan-time calls (amend the design, Task 8)

- **`loader_build` is checked by NAME, not version:** it is `cairn-node <CARGO_PKG_VERSION>`, so an exact match
  would refuse every window after a version bump. The window accepts a stamp whose `loader_build` starts with
  `cairn-node `.
- **The reconnect re-check is the poll's next tick (≤ 60 s):** `connection.rs` is over its cap (#1164); the upgrade
  itself drops no connection, so the poll bounds every case equally.
- **A never-touched terminal updates too:** `idle.js` arms its clock only on the first touch, so the update also
  installs when the terminal lock is showing and nobody has touched it for the idle period.
- **The stamped reason is the latest raising entry's** (`shortcut: a window two raises behind sees one reason;
  upgrade if that is ever confusing in practice`).
- The service checks the stamp every **5 s** and exits with status **75** (`EX_TEMPFAIL`).

## Review Focus

1. **A below-floor window must still finish its open chart:** Sign, saves, re-reads of THE OPEN chart are never
   gated — only opening a DIFFERENT patient and registering (Task 6 test).
2. **The replay's `lock_timeout` must not outlive the load:** the loader's client lives on as the daemon's
   connection; a leftover 2 s timeout would fail ordinary statements under contention (Task 3 test: `SHOW
   lock_timeout` is `0` after `connect_and_load_schema`).
3. **Someone typing a passphrase at the lock screen is never restarted under:** the lock-screen restart needs a
   full idle period without a touch (Task 7 test).
4. **A hung or missing installed program never blocks or darkens the window:** `--print-schema-generation` is run
   with a 5 s timeout; failure reads as "not installed" (Task 6 test on the parser and the timeout path).
5. **`--mock` never polls the database or restarts** (Task 6 test: `update_state` in mock is `current`).

---

### Task 1: The floor ledger, db/085, and the stamp

**Files:**
- Modify: `crates/cairn-event/src/schema_generation.rs` (constants + ledger + tests)
- Create: `db/085_window_floor.sql`
- Modify: `crates/cairn-node/src/db.rs` (`SchemaStamp`, `read_schema_stamp`, the stamp `INSERT`, the `SCHEMA` list)
- Modify: every `SchemaStamp { … }` literal in tests (`conn_posture_tests.rs` `stamp()`, `db.rs` tests `stamp()`)
- Test: `crates/cairn-node/tests/schema_replay_gate.rs` (extend)

**Interfaces:**
- Produces: `cairn_event::schema_generation::{WINDOW_FLOOR_GENERATION: i32, FloorEntry, WINDOW_FLOOR_LEDGER:
  &[FloorEntry], latest_floor_reason() -> Option<&'static str>}`;
  `pub struct FloorEntry { pub generation: i32, pub floor: i32, pub why: &'static str, pub old_window_misses:
  Option<&'static str> }`; `db::WindowFloor { pub floor: i32, pub reason: Option<String> }`;
  `SchemaStamp.window_floor: Option<WindowFloor>` (already `None` unless `window_floor_for == version`).

- [ ] **Step 1: Failing ledger tests** in `schema_generation.rs`:
  `ledger_ends_at_the_current_generation` (`last().generation == SCHEMA_GENERATION`),
  `ledger_floor_is_the_constant` (`last().floor == WINDOW_FLOOR_GENERATION`),
  `ledger_is_ordered_and_floors_never_fall` (generations strictly increase; floors non-decreasing; `floor <=
  generation`), `a_raise_names_what_an_old_window_misses` (`old_window_misses.is_some()` exactly when `floor` >
  the previous entry's floor; the first entry counts as a raise), `every_entry_says_why` (`!why.trim().is_empty()`).
- [ ] **Step 2:** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-event schema_generation` → FAIL (undefined).
- [ ] **Step 3:** Add the constants and the first entry: `{ generation: 85, floor: 85, why: "the first generation
  that records a floor; older windows know only the exact check", old_window_misses: Some("this node's update
  notices") }`. `SCHEMA_GENERATION = 85`. `latest_floor_reason()` = `old_window_misses` of the
  FIRST entry whose `floor == WINDOW_FLOOR_GENERATION` (the raise that set today's floor).
- [ ] **Step 4:** Write `db/085_window_floor.sql` (header in db/058's style: WHY/WHAT/node-local/not in cairn-sync's
  subset): `BEGIN; ALTER TABLE node_schema ADD COLUMN IF NOT EXISTS window_floor int, ADD COLUMN IF NOT EXISTS
  window_floor_reason text, ADD COLUMN IF NOT EXISTS window_floor_for int; COMMIT;`. Add it to `db.rs`'s `SCHEMA`.
- [ ] **Step 5: Failing DB test** `the_floor_is_stamped_and_read_only_for_its_own_generation` in
  `schema_replay_gate.rs`: after `connect_and_load_schema`, `read_schema_stamp` gives `window_floor ==
  Some(WindowFloor { floor: 85, reason: Some("this node's update notices") })`; after `UPDATE node_schema SET
  window_floor_for = window_floor_for - 1` it reads `None`; a database without the columns (drop them, then read
  before reload) reads `None`. Restore with `force_schema_replay` + reload.
- [ ] **Step 6:** Extend `read_schema_stamp`'s query through `to_jsonb(n)` (`->> 'window_floor'`, `'window_floor_reason'`,
  `'window_floor_for'`, cast in SQL) and build `window_floor` only when `window_floor_for = version`. Extend the
  stamp `INSERT … ON CONFLICT` with the three columns (`WINDOW_FLOOR_GENERATION`, `latest_floor_reason()`,
  `embedded`).
- [ ] **Step 7:** Run `cargo test -p cairn-event`, `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib`, the
  extended `schema_replay_gate` (DB), and `schema_version_guard` → PASS. Grep the other pinned-count guards that
  name the newest migration (`grep -rn "084" crates/*/tests/*.rs crates/*/src/*.rs`) and update each.
- [ ] **Step 8:** Commit `feat(#909): the window floor ledger and its stamp (db/085)`.

### Task 2: The window verdict (pure)

**Files:**
- Create: `crates/cairn-node/src/window_floor.rs`, `crates/cairn-node/src/window_floor_tests.rs`
- Modify: `crates/cairn-node/src/lib.rs` (module), `crates/cairn-node/src/conn_posture.rs` (`window_schema_verdict`
  removed; its caller moves in Task 6)

**Interfaces:**
- Consumes: `db::{SchemaStamp, WindowFloor}`, `LOADER_BUILD`'s name prefix.
- Produces: `pub enum WindowVerdict { Current, UpdateWaiting, BelowFloor { reason: Option<String> }, Refused(String) }`;
  `pub fn window_verdict(stamp: Option<&SchemaStamp>, window_generation: i32) -> WindowVerdict`;
  `pub enum InstallFit { Same, Fits, DoesNotFit }`;
  `pub fn install_fit(installed: Option<(i32, [u8; 32])>, running: (i32, [u8; 32]), stamp: Option<&SchemaStamp>) -> InstallFit`.

- [ ] **Step 1: Failing table test** `window_verdict_cases` covering design §4.2 row by row plus the name rule:
  no stamp → `Refused`; `v < w` → `Refused` (text keeps today's "has not been prepared for this window … run
  cairn-node provision-window-role"); `loader_build = "cairn-sync 0.1.0"` → `Refused`; `loader_build = "cairn-node
  9.9.9"`, `v == w` → `Current` (any hash, including `None`); `f ≤ w < v` → `UpdateWaiting`; `w < f` →
  `BelowFloor { reason }` carrying the stamp's reason; `v > w` with `window_floor: None` → `BelowFloor { reason:
  None }` (NULL floor = `v`).
- [ ] **Step 2: Failing test** `install_fit_cases`: `None` → `Same` ("not installed" is the safe reading);
  identical `(gen, hash)` → `Same`; a different hash at the running generation → `Fits`; an installed generation the
  verdict refuses (`> v`) → `DoesNotFit`; one below the floor → `DoesNotFit`; `UpdateWaiting`/`Current` → `Fits`.
- [ ] **Step 3:** Run `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib window_floor` → FAIL.
- [ ] **Step 4:** Implement both pure functions; `install_fit` is `window_verdict` with `w` = the installed
  generation. Module doc: why the floor exists (design §1–§2), for a junior reader.
- [ ] **Step 5:** Re-run → PASS. Commit `feat(#909): the window verdict and the install fit`.

### Task 3: The replay waits for its locks instead of stalling readers

**Files:**
- Create: `crates/cairn-node/src/replay_wait.rs`, `crates/cairn-node/src/replay_wait_tests.rs`
- Modify: `crates/cairn-node/src/db.rs` (`connect_and_load_schema`: set after the advisory lock, use for each file,
  the readjudication and the heal, `RESET lock_timeout` after the stamp and on every exit path)
- Test: `crates/cairn-node/tests/replay_lock_timeout.rs` (DB-gated, new)

**Interfaces:**
- Produces: `pub const REPLAY_LOCK_TIMEOUT: &str = "2s"`; `pub fn is_lock_timeout(sqlstate: Option<&str>) -> bool`
  (`55P03`); `pub fn backoff(attempt: u32) -> Duration` (250 ms doubling, capped at 5 s);
  `pub async fn run_waiting(client: &Client, what: &str, sql: &str) -> anyhow::Result<()>` — on `55P03`: `ROLLBACK`
  (ignore its own error), print one line naming the blockers (`pg_blocking_pids(pg_backend_pid())` →
  `application_name`, `pid`, `now() - xact_start`), sleep `backoff`, retry the whole `sql`; any other error is
  returned as `load_migration` returns it today.

- [ ] **Step 1: Failing unit tests** `only_55p03_is_a_lock_timeout`, `backoff_doubles_then_caps` (0→250 ms,
  1→500 ms, 5+→5 s).
- [ ] **Step 2: Failing DB test** `a_waiting_change_lets_readers_through_and_then_lands`: connection R holds `BEGIN;
  SELECT * FROM t` on a scratch table; spawn `run_waiting(c, "test", "BEGIN; ALTER TABLE t ADD COLUMN IF NOT EXISTS
  x int; COMMIT;")` on C (after `SET lock_timeout = '2s'`); a third connection's `SELECT count(*) FROM t` completes
  within **3 s**; R commits; the spawned task returns `Ok` within 10 s and column `x` exists; `SELECT 1` on C works
  (the session is not left aborted).
- [ ] **Step 3: Failing DB test** `the_loaders_connection_keeps_no_lock_timeout`: `connect_and_load_schema(...)`
  then `SHOW lock_timeout` → `"0"`, both on a replaying connect (`force_schema_replay` first) and a skipping one.
- [ ] **Step 4:** Implement; route the file loop, `cairn_readjudicate_deferred()` and the heal through
  `run_waiting`. Keep `load_migration` as the single-try call `run_waiting` wraps (its error text is #467's
  acceptance criterion).
- [ ] **Step 5:** Run the new tests and `schema_replay_gate`, `migration_replay_widening` → PASS. Commit
  `feat(#909): the upgrade replay waits for its locks (lock_timeout + retry)`.

### Task 4: The window's doors stay additive

**Files:**
- Create: `crates/cairn-node/tests/window_door_surface.rs` (DB-gated), `db/window_door_surface.txt`,
  `db/window_not_facing.txt`
- Modify: `CONTRIBUTING.md` (the migration checklist line: *"a new state value in anything the window renders raises
  the floor — add a ledger entry"*)

**Interfaces:**
- Consumes: `WINDOW_FLOOR_LEDGER` (Task 1).
- Produces: the snapshot format — one sorted line per entry: `fn <regprocedure> -> <pg_get_function_result>` for
  each `public` function `has_function_privilege('cairn_window', oid, 'EXECUTE')`; `col <relname>.<attname>` for
  each `public` column `has_column_privilege('cairn_window', attrelid, attnum, 'SELECT')`. `window_not_facing.txt`:
  `<entry>  # <reason>` lines.

- [ ] **Step 1: Write the test** `the_windows_doors_never_vanish_within_the_floor`: load the schema, read the
  catalogue set; FAIL listing each snapshot entry absent from the catalogue unless it is in `window_not_facing.txt`
  or the ledger's last entry raised the floor; FAIL listing each catalogue entry absent from the snapshot, telling
  the reader to run `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test window_door_surface`.
  Regeneration ADDS new entries and removes one only under the same two exemptions — it never forgets a vanished
  door silently.
- [ ] **Step 2:** Run it with `CAIRN_REGEN_DOOR_SURFACE=1` to write the first snapshot; read the file (expect several
  hundred lines; any `cairn_vault_*` grant from db/062 must appear); run again without the variable → PASS.
- [ ] **Step 3: Prove the guard bites:** temporarily `DROP FUNCTION` one window-reachable function in a scratch
  copy of a db file, run → FAIL naming it; revert (copy the file aside first — never `git checkout --`).
- [ ] **Step 4:** Commit `test(#909): the window's reachable doors stay additive within the floor`.

### Task 5: The terminal service exits on a stamp change

**Files:**
- Modify: `crates/cairn-node/src/terminal_service/run.rs` (`Schedule`/`Due` gain `stamp`; `STAMP_TICK = 5 s`;
  the loop returns `Err(StampChanged)`), `crates/cairn-node/src/main.rs` (`TerminalService` arm: exit status, startup
  line)
- Test: `run.rs`'s existing `due` tests; `crates/cairn-node/tests/terminal_service_stamp.rs` (DB-gated, new)

**Interfaces:**
- Produces: `pub const SCHEMA_CHANGED_EXIT: i32 = 75`; `#[derive(Debug)] pub struct StampChanged(pub String)`
  (`std::error::Error`); `pub async fn stamp_still_ours(client: &Client) -> Result<(), StampChanged>` (reads the
  stamp, applies `conn_posture::schema_verdict(…, "terminal service", "provision-terminal-service-role")`).

- [ ] **Step 1: Failing tests** `the_stamp_is_due_every_five_seconds` (pure `due`), and DB-gated
  `a_changed_stamp_is_noticed` (`UPDATE node_schema SET schema_hash = NULL` → `stamp_still_ours` is `Err`; reload →
  `Ok`).
- [ ] **Step 2:** Implement; on `Err`, `run` returns it and `main` prints `the node's schema changed (<why>);
  exiting so the supervisor restarts this service with the new build` and exits `SCHEMA_CHANGED_EXIT`
  (downcast). Startup line becomes `terminal service running (it exits with status 75 when the node's schema
  changes: run it under a supervisor that restarts it)`.
- [ ] **Step 3:** Run → PASS. Commit `feat(#909): the terminal service exits for its supervisor on a new schema`.

### Task 6: The window's Rust side — launch, poll, install check, restart, gate

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/update.rs`, `cairn-gui/cairn-gui-tauri/src/update_tests.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (`Cli`: `--print-schema-generation`, handled first, connecting
  to nothing; `build_live_state` uses `window_verdict`; the three commands in `generate_handler!`), `AppState`
  (`verdict: std::sync::Mutex<WindowVerdict>`, `launched_exe: Option<PathBuf>`), the `open_chart` and `register`
  commands (gate)

**Interfaces:**
- Consumes: Task 2's `window_verdict`, `install_fit`; `embedded_schema_version()`, `embedded_schema_hash()`.
- Produces (Tauri commands, JSON camelCase like the neighbours):
  `update_state() -> UpdateView { kind: "current" | "waiting" | "below_floor", reason: Option<String> }` (reads the
  stamp on the shared connection with `lock_if_open` — never reopens; `current` in mock and when no connection is
  open);
  `installed_program() -> InstallView { kind: "same" | "fits" | "does_not_fit" }`;
  `restart_window(app: tauri::AppHandle)` (calls `app.restart()`).
  Pure helpers: `parse_generation_line(&str) -> Option<(i32, [u8; 32])>` (format `"<gen> <64 hex>"`, exactly what
  `--print-schema-generation` prints); `exe_path(raw: PathBuf) -> PathBuf` (strips a trailing ` (deleted)`).
  `INSTALLED_CHECK_TIMEOUT = 5 s`.

- [ ] **Step 1: Failing tests** in `update_tests.rs`: `the_printed_line_round_trips` (print → parse),
  `a_garbled_line_is_not_installed`, `a_deleted_exe_path_is_the_installed_path`, `mock_update_state_is_current`,
  `a_below_floor_window_refuses_another_chart_but_not_its_own` (state with `BelowFloor`: `open_chart` for a
  DIFFERENT patient → the refusal copy; for the OPEN patient → proceeds; `register` → refused),
  `a_hung_program_reads_as_not_installed` (run the check against a program that sleeps — `/bin/sleep` with args
  `["10"]` behind a `cfg(unix)` — and assert `Same` within `INSTALLED_CHECK_TIMEOUT + 1 s`).
- [ ] **Step 2:** Run `cargo test` in `cairn-gui/` for `update` → FAIL.
- [ ] **Step 3:** Implement. Launch: `Refused(text)` bails as today; `BelowFloor` bails with `This window is older
  than the node's database can show correctly: it does not show <reason>. Install the window that matches the node.`
  (reason `None` → `it may not show newer records correctly`); `Current`/`UpdateWaiting` launch and store the
  verdict. `installed_program` runs `exe_path(launched_exe) --print-schema-generation` with the timeout and returns
  `install_fit`.
- [ ] **Step 4:** Run the cairn-gui gate (clippy `--locked -D warnings`, test, `cargo doc` with `-D warnings`) and
  the DB-gated `launch_posture_tests` → PASS. `command_registry_tests` will fail until Task 7 calls the commands —
  commit Tasks 6 and 7 together if the gate must stay green per commit.
- [ ] **Step 5:** Commit `feat(#909): the window checks its floor, its installed program, and restarts`.

### Task 7: The window's face — the calm line, Update now, the alert, the idle restart

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/update.js`, `cairn-gui/cairn-gui-tauri/src/update_webview_tests.rs`
- Modify: `src-ui/index.html` (`#update-line` with `#update-now`, `#update-alert`; load `update.js` after
  `leave_chart.js`), `src-ui/idle.js` (export `lastActivityAt`; after `relockTerminal`, call `updateAfterRelock()`),
  `src-ui/leave_chart.js` (after a close completes, call `updateAfterClose()`), `src-ui/style.css`

**Interfaces:**
- Consumes: Task 6's three commands; `signsSettled`, `closeChart`, `noteActivity`'s timestamp.
- Produces (JS): `UPDATE_POLL_MS = 60000`; `pollUpdate()`, `renderUpdate(view)`, `updateNow()`,
  `restartIfFits()` (asks `installed_program`; `same`/`does_not_fit` say their copy and return; `fits` → `await
  signsSettled(); await closeChart(); await invoke("restart_window")`), `updateAfterRelock()`, `updateAfterClose()`
  (both `restartIfFits()` only when the last view was not `current`; after a close, only when `below_floor`, or
  when `waiting` and the terminal lock is showing), and a lock-screen check on each poll: lock showing ∧ `Date.now()
  - lastActivityAt ≥ idleLockMs` ∧ not `current` → `restartIfFits()`.

- [ ] **Step 1: Failing source-guard tests** (`crate::source_guard`, as `leave_chart_webview_tests.rs`):
  `the_copy_is_the_designs` (each Global Constraints string appears verbatim in `update.js`/`index.html`),
  `a_restart_waits_for_acts_and_the_close` (`signsSettled` and `closeChart` precede `invoke("restart_window")` in
  `restartIfFits`), `a_restart_asks_first` (`invoke("installed_program")` precedes them),
  `the_lock_screen_restart_needs_a_full_idle_period` (the poll's lock-screen branch reads `lastActivityAt` and
  `idleLockMs`), `the_alert_never_closes_the_open_chart` (`renderUpdate` calls no `closeChart`),
  `update_js_loads_after_leave_chart` (index.html order).
- [ ] **Step 2:** Run → FAIL. Implement. Then `command_registry_tests` → PASS.
- [ ] **Step 3: Walk it headlessly** per the webview mock-walk recipe (stub `invoke` with Rust-shaped payloads;
  assert VISIBILITY, not textContent): `current` (nothing shown), `waiting` (calm line + button), `below_floor`
  (alert with the reason, chart still open), Update now with each `installed_program` kind. Note the walk in the PR.
- [ ] **Step 4:** cairn-gui gate → PASS. Commit `feat(#909): update waiting, Update now, the below-floor alert`.

### Task 8: ADR, spec, RUNBOOK, durable rules, design amendments

**Files:**
- Create: `docs/spec/decisions/0111-an-older-window-works-above-a-floor.md`
- Modify: `docs/spec/decisions/README.md`, `docs/spec/index.md` (**0.113**), `docs/spec/security.md` §7.6
  ("Fail-safe upgrades" gains the window floor sentence), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (**§40**),
  `docs/developers/durable-rules.md` (a "#909 slice C1" block), the design doc (Plan-time calls above),
  `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1:** ADR-0111 supersedes ADR-0084 **decision 4's exact match** only (the window still never loads the
  schema), citing ADR-0012 and ADR-0021; decisions = design D1–D6 + the plan-time calls; "What is still not
  guaranteed" = design §6. Fact-check every sentence against the code before review.
- [ ] **Step 2:** RUNBOOK §40 measures: Update now → lock screen (budget ≤ 10 s, my call); the service-restart
  blip as seen by a remote window (≤ 5 s to a working chart); an upgrade under a 60 s open reader (no clinical press
  waits past 15 s); the heal's longest lock on the 100 k-event rig (stated, a finding if > 15 s); idle restart at
  the lock screen; a too-new install says "does not fit".
- [ ] **Step 3:** Durable-rules block: "never reset the floor ledger"; "a new state value the window renders raises
  the floor"; "`run_waiting` must `ROLLBACK` before retrying"; "the loader resets `lock_timeout`"; "`update_state`
  never reopens"; "below the floor gates only a DIFFERENT chart and registration".
- [ ] **Step 4:** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict` → exit 0; run
  `scripts/check_closing_keywords.py` on the commit message. Commit `docs(#909): ADR-0111, spec v0.113, RUNBOOK §40`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a revised form. Old stock stays valid until used up; a recalled form is finished on the
  sheet in hand, then a new pad is taken.
- **Steps:** paper *N* = 1 (take the new pad) → architecture-forced *M* = 0 at an idle update, 1 press (Update now)
  when someone is working → UI target *K* = 0 or 1. Below the floor, 0 extra acts on the open chart. `M ≤ N`.
- **Time + cognitive load:** budget — Update now to a ready lock screen ≤ 10 s; no clinical press waits past the
  15 s cut-off during an upgrade; one line of calm text, read once. Measured by RUNBOOK §40 (Task 8), owed by a
  human.
