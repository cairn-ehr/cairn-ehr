# #909 slice C1: an upgrade disturbs no one — the window floor, the waiting update, the lock timeout — design

Status: **approved 2026-11-24**, brainstormed with the maintainer (PR #1173 began it; this session finished it).
Parent: [`2026-11-16-pinned-terminal-link-909-design.md`](2026-11-16-pinned-terminal-link-909-design.md) §6, which
made the rollout its own slice. Slice C2 (a terminal fetches its new program from its node) is a later slice with its
own brainstorm (§9).

## 1. Why

Slices A and B let a facility run many terminals against one node. Today every upgrade of that node disturbs all of
them (parent §6, all three checked against the code on 2026-11-24):

1. **Locks.** `connect_and_load_schema` replays each changed `db/*.sql` file with `load_migration`
   (`crates/cairn-node/src/db.rs`), with no `lock_timeout`. A full replay takes ACCESS EXCLUSIVE on `event_log`, the
   identity tables and 34 views (#726). A DDL lock queued behind ONE long reader blocks every later clinical read and
   write behind it, and a press held past the window's 15 s cut-off (#875) is cut.
2. **The stamp.** The window checks the database's stamp once, at launch (`cairn-gui-tauri/src/main.rs:343`,
   `conn_posture::window_schema_verdict`), and passes only an EXACT build match: same generation, same loader, same
   content hash ([ADR-0084](../../spec/decisions/0084-the-window-never-holds-the-owners-keys.md) **decision 4**; the
   parent's §6 says "decision 2" — a slip). So any upgrade, even an in-place fix, refuses every terminal's next launch
   until its program is replaced, and a window already running is never re-checked at all — it goes on calling
   doors that may have changed under it.
3. **Doors are not additive in practice.** 31 `db/*.sql` files carry 62 `DROP FUNCTION`s. A running old window can
   call a function that no longer exists, mid-act. [ADR-0021](../../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
   says the native API evolves additively; nothing enforces it.

A facility's IT replaces terminal programs over days, not in one instant. A clinic with no IT replaces them by hand.

## 2. What the maintainer decided (2026-11-24)

- **D1. An older window keeps working against a newer database for as long as that database still shows it
  correctly** — until replaced, not until a natural break. It stops only when it falls below a **floor**.
- **D2. Idle update.** If nobody is working at a terminal, it saves, updates and restarts by itself. The trigger is
  the existing idle re-lock (`src-ui/idle.js`, `--idle-lock-minutes`, default **15**, configurable per site) — no
  second clock.
- **D3. Someone working is told, not stopped.** The calm line ("an update is waiting") carries an **Update now**
  button, because the idle moment might never come that day.
- **D4. Both ways of getting the new program onto the terminal, in order:** C1 (this design) — someone else installs
  it (IT, the OS package manager) and the window picks the moment to restart into it; then C2 — the terminal fetches
  a signed release from its own node (§9).
- **D5. One floor number**, not per-door or per-area floors (§3).
- **D6. Every replayed DDL file runs under a short `lock_timeout` and is retried**; online patterns
  (`CONCURRENTLY`, `NOT VALID`) only where one migration needs them (§3).

## 3. Approaches considered

**The floor.**
- **A single floor generation (chosen).** The loader stamps "the oldest window generation this database serves
  correctly" beside its stamp. Each generation must say whether it raises the floor and why. Simple to reason about;
  a person judges meaning, a test forces the judgement to be made.
- *Checking each door the window calls* (rejected as the mechanism, kept as a guard, §4.3). It catches a removed
  function but not the dangerous case: the same function now carrying a state the old window does not know (a new
  hold state an old window would show as an active medication). That needs a person's judgement anyway.
- *A floor per area* (medications, notes, …) (rejected): an old window that has lost medications but kept notes is
  harder to reason about and confusing to use.

**Locks.**
- **`lock_timeout` + retry per file (chosen).** A timeout aborts the file's transaction (67 of 84 files carry their
  own `BEGIN;`/`COMMIT;`; db/033, db/034 and db/079 carry two or three); the retry rolls back and re-runs the WHOLE
  file. That is safe even for the multi-transaction files, whose earlier blocks may already be committed, because the
  replay contract is already "every file re-runs whole on every changed connect" — a file must be idempotent today.
  A waiting change then never holds readers more than ~the timeout.
- *Online patterns everywhere* (rejected as a default): most files are `CREATE OR REPLACE` of functions and views,
  which have no online variant; it is per-migration work for where it is needed.
- *A maintenance window* (rejected): fails "disturbs no one".

## 4. The design

### 4.1 The floor, and where it lives

- A constant `WINDOW_FLOOR_GENERATION` beside `SCHEMA_GENERATION` (`cairn_event::schema_generation`), and a
  **ledger**: one entry per generation from the first one carrying the floor, `{generation, floor, reason}`.
  `reason` is a plain sentence for a clinician saying what an older window gets wrong ("does not show a medication
  held since 24 Nov"), or why the floor did not move.
- **Guard (forward-only):** the ledger's last entry is for `SCHEMA_GENERATION`, and its floor equals the constant.
  So a generation bump cannot land without a decision. A floor may only move with a generation bump; an in-place
  edit (ADR-0080, same generation) therefore can never raise it.
- The loader stamps `window_floor`, `window_floor_reason` and **`window_floor_for`** (the generation the floor was
  written for) into `node_schema` with the rest of the stamp. They are read through `to_jsonb(n)` like
  `schema_hash`, so an older database reads NULL. **The floor counts only when `window_floor_for` equals the stamp's
  `version`**; otherwise it reads as NULL, and **NULL means floor = the database's own generation** (today's
  exact-generation behaviour — every doubt toward refusing). *(Builder check; my call.)* This closes the stale-floor
  hole: cairn-sync's `init` stamps `node_schema` with its own `ON CONFLICT` (`crates/cairn-sync/src/main.rs` ≈2240)
  and would leave an older floor beside a newer version.

### 4.2 The window's verdict (pure, replacing `window_schema_verdict`'s exact match)

With `w` = the window's generation, `v` = the database's, `f` = its floor:

| Case | Verdict |
|---|---|
| no stamp, or `v < w` (database older or unprepared) | **refused**, as today (database first, then terminals) |
| `loader_build` is not cairn-node's (e.g. cairn-sync's subset) | **refused**, as today (db.rs `replay_can_be_skipped`) |
| `v == w`, hash differs (an in-place fix) | **current** — inside the floor by §4.1's rule |
| `v == w`, hash equal | **current** |
| `f ≤ w < v` | **update waiting** (§4.4) |
| `w < f` | **below the floor** (§4.5), carrying `window_floor_reason` |

The terminal service (`cairn-node terminal-service`, the same binary as the loader) keeps the exact check: it is
node-side and upgraded with the node (§4.7).

**The same verdict judges a newly installed program before the window restarts into it** (§4.4): `w` is then the
new program's generation, so a program that would refuse this database is never restarted into.

### 4.3 The door surface stays additive within the floor

- **Guard:** a committed snapshot of what `cairn_window` can reach after a full load — every function it may EXECUTE
  (signature **and result type**, so a drop-and-recreate with a new result shape is caught), every table/view/column
  it may SELECT — read from the catalogue with `has_function_privilege` / `has_column_privilege`, so it counts
  inheritance from `cairn_agent` (db/062) and PUBLIC's default EXECUTE. The test compares the final catalogue, not
  the files' `DROP`s, so a drop-and-recreate with the same signature and result is no change. A disappeared entry
  fails the test unless the ledger raised the floor in this generation **or** the entry is moved to a reviewed
  `not_window_facing` list with a one-line reason (an internal helper the window never calls). *(Builder check; my
  call: PUBLIC's default EXECUTE makes the reachable set nearly every function, so without that list every internal
  signature change would force a floor raise.)* Additions just extend the snapshot (regenerated by a documented
  command).
- Stated limit: the guard catches a **removed** door, never a **changed meaning**. That is the ledger's `reason` and
  the reviewer's job; the plan adds a checklist line ("a new state value in anything the window renders raises the
  floor"). A new feature an old window simply does not show (a new tab) does not.

### 4.4 Inside the floor: the calm line and Update now

- The window learns the verdict at **launch**, at each **reconnect** (#807, `cairn-gui-live/src/connection.rs`), and
  on a **new, slower stamp poll** (≈ 60 s) — the upgrade drops no connection, so without a poll a running window
  would never learn of it. *(Builder check: `lock_state` touches no database today, `commands.rs` ≈145, so this is a
  new query on the window's one shared connection; my call: its own cadence, not the 10 s lock poll, and it never
  reopens a lost connection — the reconnect path already checks.)*
- It shows one calm line: "An update is waiting; it installs when this terminal is next idle." with **Update now**.
  Nothing else changes.
- **Update now** (D3) and the **idle re-lock** (D2) do the same thing: wait for any act in flight to settle, close the
  chart through `leave_chart.js`'s `closeChart` (queued saves awaited — drafts are already in the database), forget
  the held key and the claim as the re-lock does, then **restart the window program** (Tauri's
  `AppHandle::restart`, no new crate).
- **C1 cannot install anything.** Restarting into an unchanged program would bring the same line back. So before
  restarting, the window asks the installed program for its generation (a flag that prints the embedded
  `SCHEMA_GENERATION` and exits, connecting to nothing) and runs §4.2's verdict on it against the stamp:
  - an unchanged generation → Update now says "No newer version is installed on this terminal yet — ask whoever
    installs Cairn here." and does not restart; the idle re-lock just locks;
  - a program that would be **refused** (newer than the database, say) → "The version installed here does not fit
    this node yet." and no restart — so a too-new install never turns a working terminal dark *(builder check; my
    call)*;
  - otherwise → restart.

### 4.5 Below the floor

- A **persistent** alert naming the `reason`: "This terminal needs its update: it does not show <reason>. Finish what
  you are doing." with Update now.
- The chart that is open stays open, so the clinician can finish (D3). **This terminal opens no other chart and
  sends no registration until it has updated** — so an ignored alert costs at most one chart, never a shift.
- When that chart closes, or the terminal is idle-locked, or nobody is signed in, it restarts at once (same
  sequence as §4.4) if a newer program is installed; otherwise it stays at the lock showing the alert.
- At **launch** below the floor the window does not open a chart at all (as today's refusal), naming the reason.

### 4.6 The replay's lock timeout

- `load_migration` runs each file under `SET lock_timeout` (≈ 2 s; the figure is the builder's) on the replay
  connection, and on a `55P03` sends `ROLLBACK` (the file's own `BEGIN` leaves the session aborted, so a bare retry
  would fail `25P02`) and retries the whole file with backoff, **indefinitely**, printing on each wait which session
  blocks it (`pg_blocking_pids` → application name, pid, how long) so the owner can decide.
- **Scope** *(builder check; my call)*: the timeout is set AFTER `pg_advisory_lock(SCHEMA_LOAD_LOCK)` is taken (that
  wait is loader-against-loader, blocking as today) and covers every file, `cairn_readjudicate_deferred()` and the
  heal — all retried the same way. `lock_timeout` bounds the WAIT for a lock, never how long a statement runs.
- Ctrl-C mid-loop leaves the same half-replayed state as any failed replay today: no stamp is written until the
  whole loop and heal succeed, and a re-run completes it. Nothing new.
- **The post-upgrade heal** (`cairn_reproject('', false, 'loader')` on a generation change) is one long statement:
  the plan measures what it locks and for how long on the 100 k-event rig, and states it in RUNBOOK. If it holds a
  clinical table past the 15 s cut-off, that is a finding (file it, never widen the budget).

### 4.7 The upgrade order

1. Install the new `cairn-node` binary on the node's machine, then run the upgrade with it (`cairn-node
   provision-window-role` — ADR-0084 decision 2's one owner step for install and upgrade) while the old terminal
   service is still running. Its gateway keeps piping; the lock timeout keeps the replay from stalling it.
2. **New work:** the service re-reads the stamp on each tick (today only at startup, `main.rs` ≈3116; its loop,
   `terminal_service/run.rs` ≈198, never reads it). On any mismatch with its own build it **exits with a distinct
   status**, never running old code against a new schema, and its supervisor (launchd/systemd — RUNBOOK already
   requires one for the node's long-running workers) starts it again from the same path, which is now the new
   binary. *(Builder check; my call over "pause until the owner restarts it": a pause would stall registration and
   the commit tick for as long as nobody notices, against §5's goal.)* The same holds after an in-place fix.
3. Remote windows reconnect through the restarted gateway (#807); an act in flight at that instant gets its existing
   outcome line. The plan measures the blip (RUNBOOK). An unsupervised service that exits stays down: the plan's
   RUNBOOK says so, and `terminal-service`'s startup line says it expects a supervisor.
4. Terminals update as §4.4 describes, over hours or days.

**The first rollout** (the build that brings C1) is the last that refuses relaunches: older windows know only the
exact check. No site runs Cairn yet, so slices A, B and C1 ship together and this costs nothing.

## 5. Goals, non-goals, anti-goals

- **Goals:** no upgrade stalls a clinical read or write past a few seconds; no terminal goes dark because IT has not
  reached it yet; no old window renders a newer database wrongly without saying so; the decision "does this change
  break an old window" is forced at every generation.
- **Non-goals:** installing programs (C2); rolling a database back; the matcher worker's own version check (it is
  advisory, separate); a per-door or per-area floor.
- **Anti-goals:** a confirmation dialog on Update now (house rule — it is a deliberate press, and it loses nothing);
  stopping a clinician mid-act for an update inside the floor; an old window silently showing a state it does not
  understand.

## 6. Risks and stated limits

- **The floor is a human judgement.** §4.3 catches removed doors only; a changed meaning slips through a careless
  ledger entry. The checklist line and review are the defence.
- **The floor will rise often** if most generations add states the window renders; then D1 degrades towards "until
  a natural break" for those generations. Honest, and the alert says why.
- **The heal may hold locks longer than any DDL** (§4.6) — measured, not assumed.
- **Below the floor with no new program installed**, the terminal can finish its open chart and then does nothing
  useful until someone installs the update. In C1 that is the price of a recalled form; C2 removes it.

## 7. Paper-parity

Not a clinical workflow, but it interrupts one. Paper counterpart: a form is revised — old stock stays valid until
used up, unless it is recalled, when staff finish the sheet in hand and take a new pad. Steps: idle update **0**
human acts; Update now **1** press; below the floor **0** extra acts on the open chart, then the update. Budget owed
by the plan's RUNBOOK section: the restart (Update now → chart-ready lock screen) and the service-restart blip each
measured and stated; no clinical press waits past the 15 s cut-off during an upgrade.

## 8. Left to the builder

- The `lock_timeout` figure and the backoff curve; the stamp poll's exact cadence.
- The flag's name that prints the installed program's generation, and how the window finds the installed program
  (on Linux a replaced running executable reads as `(deleted)` in `/proc/self/exe`; check Tauri's `restart()` there).
- The service's exit status number and its startup line.
- The ledger's form (a Rust table, or a file a build script reads) and the snapshot's format and regeneration
  command.
- The exact copy of the three lines (calm, not installed, below the floor), checked against the house words.
- Whether the new ADR amends ADR-0084 decision 4 by supersession (it must: the exact match is reversed) and how it
  cites ADR-0012 and ADR-0021.
- Plan the RUNBOOK section (§7's measurements, two machines, an upgrade under a long reader).

## 9. Slice C2 (later, its own brainstorm)

The terminal fetches its new program from its own node over the slice B link, so the owner upgrades once and every
terminal follows. **It must check the release's signature itself, never trust the node** — otherwise a compromised
node could push code that captures every clinician's passphrase. That is the terminal's share of the
signed-release checks in [security §7.6](../../spec/security.md#76-the-software-distribution-plane-signed-releases-and-extension-load),
none of which is built, and it is safety-critical.
