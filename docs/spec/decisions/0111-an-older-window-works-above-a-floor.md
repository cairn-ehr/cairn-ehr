# ADR-0111 — An older window works above a floor

- **Status:** Accepted (#909 slice C1: an upgrade of the node disturbs no terminal. Slice C2, the terminal fetching a
  signed release from its own node, follows with its own brainstorm)
- **Date:** 2026-11-24
- **Spec version at acceptance:** 0.113
- **Issues:** [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909), PR pending. Design (maintainer-approved
  2026-11-24): `docs/superpowers/specs/2026-11-24-909-slice-c1-rollout-design.md` (D1–D6); plan:
  `docs/superpowers/plans/2026-11-24-909-slice-c1-rollout.md`. Filed by the build:
  [#1175](https://github.com/cairn-ehr/cairn-ehr/issues/1175).
- **Supersedes in part:** [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) decision 4's exact-match launch
  check (decision 9 below says what it now reads). The window still never loads the schema, and every other decision
  of ADR-0084 stands.
- **Builds on:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (the schema version is a
  local node property, so there is no lockstep fleet upgrade) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (the native API evolves additively; decision 3 makes the
  window's share of it a test) · [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (the stamp, its
  content hash, and in-place edits at one generation; a new `db/085`, `SCHEMA_GENERATION` 85) ·
  [ADR-0109](0109-the-terminal-service-and-the-keyless-window.md) and
  [ADR-0110](0110-the-pinned-terminal-gateway.md) (the terminal service and its gateway, whose "the rollout is not
  settled" this slice answers in part)
- **Code:** `db/085_window_floor.sql`; `crates/cairn-event/src/schema_generation.rs`;
  `crates/cairn-node/src/{window_floor,replay_wait,db,conn_posture,main}.rs`, `terminal_service/run.rs`;
  `db/window_door_surface.txt`, `db/window_not_facing.txt`; `cairn-gui/cairn-gui-tauri/src/{update,main,state}.rs`,
  `funnel/commands.rs`; `src-ui/{update,idle,leave_chart}.js`, `src-ui/index.html`

## Context

Slices A and B let a facility run many terminals against one node, and every upgrade of that node disturbed all of
them (design §1). The replay took DDL locks with no `lock_timeout`, so an `ALTER` queued behind one long reader held
every later clinical read and write behind it. The window checked the stamp once, at launch, and passed only an exact
build match (ADR-0084 decision 4), so any upgrade, even an in-place fix, refused every terminal's next launch until
its program was replaced, while a window already running was never re-checked at all. And nothing kept the window's
doors additive: 31 `db/*.sql` files carry `DROP FUNCTION`s. A facility's IT replaces terminal programs over days; a
clinic with no IT replaces them by hand.

## Decision

1. **One floor, recorded per generation** (design D5, §4.1). `cairn_event::schema_generation` carries
   `WINDOW_FLOOR_GENERATION` beside `SCHEMA_GENERATION`, both **85**, and `WINDOW_FLOOR_LEDGER`, one `FloorEntry`
   (`generation`, `floor`, `why`, `old_window_misses`) per generation from 85 on. Its tests force the decision at
   every generation bump: the last entry is for `SCHEMA_GENERATION` and its floor is the constant; generations
   strictly increase, a floor never falls and never exceeds its generation; `old_window_misses` is set exactly when
   an entry raises the floor (the first entry counts as a raise); every entry says why. The first entry is generation
   85, floor 85, why "the first generation that records a floor; older windows know only the exact check", missing
   "this node's update notices".

   `db/085` adds `window_floor`, `window_floor_reason` and `window_floor_for` to `node_schema`, and the loader writes
   them with the rest of the stamp: the constant, `latest_floor_reason()`, and the generation it stamps.
   `read_schema_stamp` reads them through `to_jsonb(n)`, so a database without the columns reads them as absent, and
   it trusts the floor **only when `window_floor_for` equals the stamp's `version`**. Otherwise the floor is absent,
   and an absent floor means the database's own generation: no older window is served. That closes the stale-floor
   hole, since cairn-sync's `init` restamps `version` and `loader_build` without touching these columns.

   The stamped reason is the **latest raising entry's** (plan-time call): `latest_floor_reason()` returns the
   `old_window_misses` of the first entry whose floor equals today's. A window two raises behind therefore sees one
   reason, not both.

2. **The window's verdict is a pure function** (design D1, §4.2). `window_floor::window_verdict(stamp, w)`, with `v`
   the stamp's generation and `f` its floor, decides in this order: no stamp, or `v < w`, is **refused** with
   ADR-0084's "has not been prepared for this window" text; a `loader_build` that does not start with `cairn-node `
   is **refused** with its "prepared by a different build" text; `v == w` is **current**, whatever the hash;
   `f ≤ w < v` is **update waiting**; otherwise, `w < f` or no floor, **below the floor**, carrying the stamp's reason.
   The loader is checked by **name, not version** (plan-time call): `LOADER_BUILD` is
   `cairn-node <CARGO_PKG_VERSION>`, so an exact match would refuse every window after a version bump.

   `window_floor::install_fit(installed, running, stamp)` judges an installed program the same way, with `w` its
   generation: nothing installed, or the identical `(generation, hash)`, is `Same`; a verdict of current or update
   waiting is `Fits`; anything else is `DoesNotFit`. The terminal service keeps ADR-0084's exact check
   (`conn_posture::schema_verdict`): it is node-side and upgraded with the node (decision 8).

3. **The window's doors stay additive within the floor** (design §4.3, ADR-0021). `db/window_door_surface.txt` is a
   committed, sorted snapshot of what `cairn_window` can reach after a full load: `fn <regprocedure> -> <result type>`
   for each `public` function it may `EXECUTE` (`has_function_privilege`, so inheritance from `cairn_agent` and PUBLIC's
   default `EXECUTE` count), and `col <relation>.<column>` for each `public` column it may `SELECT`
   (`has_column_privilege`). The test `window_door_surface.rs` compares that snapshot with the catalogue, so a
   drop-and-recreate with the same signature and result is no change. A vanished entry fails unless it is listed in
   `db/window_not_facing.txt` (doors used by no window at or above the floor) with a reason, or the ledger's last entry
   raised the floor and the snapshot was recorded at an older generation: its first line, `# generation N`, says which.
   Only a regeneration applies that second excuse: the check fails a door it would excuse, saying to regenerate at the
   raise, and the regeneration drops the door and writes the raising generation's header, so an in-place edit at that
   generation (ADR-0080) that drops another door fails like any other. A new entry fails until the snapshot is
   regenerated with `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test window_door_surface` (only `=1`
   regenerates), which writes today's header, adds new entries and drops a vanished one only under those two exemptions.
   **Extension-owned objects are excluded** (build-time ruling): a `pg_depend` row of `deptype = 'e'`, everything
   `cairn_pgx` installs, moves with the extension's version rather than `db/*.sql` and differs between machines. The
   guard catches a removed door, never a changed meaning: `CONTRIBUTING.md`'s migration checklist says a new state value
   in anything the window renders raises the floor, and the ledger's reason and the reviewer carry that.

4. **The window learns the verdict at launch and on a 60 s poll** (design §4.4). At launch it reads the stamp on its
   non-loading connection: refused opens nothing, as before; below the floor opens nothing and says "This window is
   older than the node's database can show correctly: it does not show <reason>. Install the window that matches the
   node." (with no reason, "it may not show newer records correctly"); current and update waiting open, and the window
   keeps the verdict. Then `update.js` asks `update_state` once a minute (`UPDATE_POLL_MS = 60000`), and once at
   startup. It reads the stamp on the window's one shared connection through `lock()`, so **it reopens a lost
   connection** (#807, a reopen taking up to 5 s, none tried within 2 s of a failed one). That reopen is needed: the
   service's exit (decision 8) drops every REMOTE window's connection, which runs only through that service's gateway
   (ADR-0110), and such a window may sit untouched at its lock with nothing else to reopen it. The reconnect re-check
   is **the poll's next tick** (plan-time call): `connection.rs` is over its cap (#1164), so the #807 reopen itself
   checks nothing. **A lost connection or a failed read keeps the last verdict** (build-time ruling): a blip neither
   lifts a below-floor alert nor raises one. The poll also keeps the last stamp it read (`AppState::last_stamp`,
   seeded at launch). A refusal learned mid-session (another loader restamped the database, or it was restored to an
   older generation) is gated as below the floor, the safe direction (build-time ruling), but its alert is the
   refusal's own sentence, not the below-floor copy. `--mock` reads as current and never restarts.

5. **Inside the floor, the calm line and Update now** (design D2, D3). The window shows "An update is waiting; it
   installs when this terminal is next idle." with **Update now**, and nothing else changes. Update now, the idle
   re-lock and the restarts below all run one sequence (`restartIfFits`). First the window asks the program now
   installed at its own path for its generation (`--print-schema-generation`, which prints `<generation> <64 hex>` and
   exits, connecting to nothing), with a **5 s** timeout. The flag and its line are a contract across generations, since
   an older window asks a newer program: neither ever changes, and the program reads the flag from its raw arguments
   before clap parses anything, so no argument a later build requires or refuses can stop the answer; a program that
   fails, hangs or prints anything else reads as not installed. It is judged against a stamp read just now, else the
   last stamp read, and **only a fresh stamp can lead to a restart** (build-time, fix round 2): a window relaunched
   while the node does not answer would exit at its first connect, and the terminal would be gone. A restart from the
   lock screen's poll asks again, after that answer, whether nobody has touched the terminal; an Update now press made
   meanwhile joins that attempt instead. Then:
   - `same` (nothing newer installed, whatever the node says): "No newer version is installed on this terminal yet —
     ask whoever installs Cairn here." and no restart;
   - `does_not_fit` (this database, on a fresh or the cached stamp, would refuse it or tell it to update): "The
     version installed here does not fit this node yet." and no restart, so a too-new install never turns a working
     terminal dark;
   - `unreachable` (it fits only the cached stamp, or no stamp has ever been read): "This terminal cannot reach the node
     right now; the update installs once it can." It closes nothing and does not restart; at an idle lock the next
     poll that reads the stamp tries again;
   - `unknown_install` (a live window that cannot name its own program's path, the symlink limit below): "This
     terminal cannot check which version is installed here; restart it once the update is installed." and no restart;
   - `fits` (on a fresh stamp): the act in flight settles, the chart closes with its queued saves awaited, and the
     window restarts (Tauri's `AppHandle::restart`). Two outcomes are read: that of the act in flight when the sequence
     BEGAN, captured before the program is asked (a note's Sign holds the window's one connection while it writes, so
     the check may answer once nothing is in flight any more), AND that of any act begun during the check (the program
     runs up to 5 s without the connection, time enough for a one-click Stop). Either one that did not settle well (a
     Sign cut off at 15 s, say) keeps its chart on screen and nothing restarts, as "Find another patient" does (#1130):
     its outcome is said only there. The held key and the claim live in the window's memory and go with it.

   With an update waiting the sequence runs after the idle re-lock (giving up, like the poll's, if someone touches the
   lock while the installed program answers), after a chart close while the terminal lock is showing, and at the
   terminal lock after a whole idle period with no touch. That last case covers **a terminal nobody has touched since it
   started** (plan-time call): `idle.js` arms its timer only at a first touch, and `lastActivityAt` starts at the page
   load. Someone typing a passphrase at the lock is never restarted under.

6. **Below the floor, the chart in hand is finished and no other is opened** (design §4.5). A persistent alert says
   "This terminal needs its update: it does not show <reason>. Finish what you are doing." with Update now (with no
   reason, "it may not show newer records correctly"). The open chart's re-reads, saves, Sign and close are never
   gated. `crate::update::refuse_another_chart` refuses opening a DIFFERENT chart and **every** registration with
   "This terminal needs its update before it can open another chart."; a registration is refused before anything
   else is checked. The sequence of decision 5 runs whenever that chart closes (`leaveChart`'s success, by hand or
   by the idle re-lock), after the idle re-lock, or at the terminal lock after a whole idle period; a second run while
   one is in progress does nothing.

7. **The replay waits for its locks** (design D6, §4.6). `connect_and_load_schema` sets `lock_timeout` to
   `REPLAY_LOCK_TIMEOUT` (`2s`) right AFTER it takes the loader-against-loader advisory lock, so that wait stays
   blocking, and runs every replayed file, `cairn_readjudicate_deferred()`, the post-upgrade heal and the stamp through
   one retry loop (`replay_wait::retry_on_lock_timeout`). Only `55P03` is retried: the loop sends `ROLLBACK` (a timeout
   inside a file's own `BEGIN` leaves the session aborted, and a bare retry would fail `25P02`), prints one line naming
   the step and the sessions that **may** be holding the lock, sleeps, and runs the WHOLE step again, indefinitely. The
   pause is 250 ms, doubling, capped at 5 s. Re-running a whole file is safe because every file is already replayed
   whole on every changed connect and so must be idempotent. The blocker line is a heuristic (build-time ruling):
   `pg_blocking_pids` is empty once the timeout has cancelled the wait, so the line lists, oldest first and at most
   five, the sessions in this database holding a table lock inside a transaction open at least as long as the timeout. A
   file's other errors keep exactly the text `load_migration` gives them (#467). After the stamp, `RESET lock_timeout`:
   the loader's client lives on as the daemon's connection.

8. **The terminal service exits for its supervisor on a new schema** (design §4.7). It re-reads the stamp every
   **5 s** (`STAMP_TICK`) with the exact check, so an in-place fix counts too. On a mismatch it prints "the node's
   schema changed (<why>); exiting so the supervisor restarts this service with the new build" and exits with status
   **75** (`SCHEMA_CHANGED_EXIT`, BSD `EX_TEMPFAIL`); its supervisor starts it again from the same path, now the new
   binary. Its startup line says so: "terminal service running (it exits with status 75 when the node's schema
   changes: run it under a supervisor that restarts it)". **A stamp that cannot be read is not a change**
   (build-time ruling): at startup it refuses to start, as before; in the loop it is logged once per distinct message
   and the loop carries on, so a dropped connection never restarts the service.

9. **What the superseded passage now reads.** ADR-0084 decision 4's first paragraph now reads: *the window's launch
   connects with the non-loading `db::connect`, reads the `node_schema` stamp and applies `window_verdict` (this ADR's
   decision 2). An unstamped or older database names `cairn-node provision-window-role` with the owner's connection,
   as before; one at or above the window's generation but stamped by a loader other than cairn-node names "the
   cairn-node built with this window"; one at the window's generation opens, whatever its hash; a newer one opens at
   or above its floor and is refused below it, naming what the window would not show.* Its second paragraph (#912)
   stands.

**The first rollout** (the build that brings this slice) is the last that refuses relaunches: older windows know only
the exact check. No site runs Cairn yet, so slices A, B and C1 ship together and this costs nothing.

**Paper-parity.** The paper counterpart is a revised form: old stock stays valid until used up, and a recalled form
is finished on the sheet in hand, then a new pad is taken. Paper *N* = 1 (take the new pad) → forced *M* = 0 at an
idle update, 1 press (Update now) when someone is working → target *K* = 0 or 1; below the floor, 0 extra acts on the
open chart. `M ≤ N`. The budget: Update now to a ready lock screen ≤ 10 s; no clinical press waits past the 15 s
cut-off during an upgrade; one line of calm text, read once. RUNBOOK §40 measures it, owed by a human.

## Consequences

- An upgrade of the node no longer refuses every terminal's next launch. A window at or above the floor keeps working
  and installs the update at its next idle moment, or at Update now; one below it finishes the chart in hand.
- Every generation bump must say, in the ledger, whether an older window still shows the database correctly.
- A replay step waits at most ~2 s for any ONE lock, so a clinical read or write queued behind that wait is held about
  as long by it. The bound is per wait: a file's transaction keeps the locks it already holds through its later waits
  and its own run, and each early retry can give readers another ~2 s stall. A lock the step already holds is not
  bounded (the heal, below).
- The terminal service must run under a supervisor that restarts it, or an upgrade stops it.

## What is still not guaranteed

- **The floor is a human judgement.** Decision 3 catches a removed door, never a changed meaning; a careless ledger
  entry lets a new state through to an old window. The checklist line and review are the defence.
- **A raise excuses every door it removes.** The guard cannot tell a door the open chart uses from any other, so a
  raise must not remove one: a window below the floor still finishes the chart in hand on it. Raise first, and remove
  the door in a later generation, listed in `db/window_not_facing.txt` since no window at or above the floor uses it.
  "Later" is a sequence, not a time bound: the removal is safe only once no window below the floor can still have a
  chart open, and nothing checks that. A facility that upgrades twice in an afternoon can still take a door from a
  chart in hand.
- **The floor may rise often**, if most generations add states the window renders; D1 then degrades towards "until a
  natural break" for those generations. The alert says why.
- **The heal may hold locks longer than any DDL**, and a lock wait restarts the whole heal, which costs a busy node
  repeated work. Its upserts lock every projection row they touch and hold those row locks to its end, so a WRITE to
  an existing projection row (a Stop on a medication, a demographics edit) waits for the whole heal; a read does not.
  RUNBOOK §40 measures such a write on the 100 k-event rig; past 15 s it is a finding.
- **The re-adjudication swallows its lock timeout** (#1175): db/043 runs each promotion inside `EXCEPTION WHEN OTHERS`,
  which records the `55P03` as the marker's `adjudication_error`, so the retry of decision 7 never sees it. The event
  stays deferred until a later loader verb runs (#912). Nothing is lost.
- **C1 installs nothing.** Below the floor with no newer program installed, the terminal finishes its open chart and
  then does nothing useful until someone installs the update. C2 removes this.
- **A symlink never updates.** Tauri reads the running program's path once, when it loads (`STARTING_BINARY`). On
  Linux it resolves symlinks, so an install that swaps a link leaves the window asking about the old file. On macOS a
  path with a symlink anywhere in it (`/tmp` is one) reads as no path at all, so that window says it cannot check
  which version is installed and never restarts. An install must replace the file, at a path with no symlink in it.
- **A front door below the floor waits.** With no chart open and the terminal unlocked, nothing restarts until the
  idle re-lock or Update now.
- **An unsupervised service that exits stays down**, and so do registration, the commit tick and every remote
  terminal's gateway with it. A supervisor restarting from a path that still holds the OLD binary gets a service that
  refuses to start, again and again.
- **A window learns of an upgrade at its next poll, up to 60 s late.** A remote window, whose connection the
  service's restart drops, learns at its first poll after the gateway is back, plus that poll's reopen (up to 5 s; a
  reopen is not tried within 2 s of a failed one). A poll that falls inside the gateway's restart learns nothing, and
  the next is 60 s later. A reopen another command failed within the previous 2 s also skips that poll's: the 10 s
  lock poll touches no database, but the person menu re-reads every 60 s while someone is named, and a poll that
  follows its failed reopen learns a minute later. A restart reuses the launch's arguments, so a window started with
  `--patient` reopens that chart.
- **The node can still go down between the fresh check and the relaunch.** The check is a few seconds before the
  restart (the program's answer, every Sign settling, the chart's close); a node lost in that gap makes the relaunched
  window fail at launch, like any launch against an unreachable node, and it must be started again by hand.

Pins: `cairn-event`'s `schema_generation` ledger tests;
`crates/cairn-node/src/{window_floor_tests,replay_wait_tests}.rs` and `terminal_service/run.rs`'s unit tests;
`crates/cairn-node/tests/{schema_replay_gate,replay_lock_timeout,window_door_surface,terminal_service_stamp}.rs`;
`cairn-gui-tauri`'s `update_tests.rs` and `update_webview_tests.rs`.
RUNBOOK §40 carries the human measurements: Update now to the lock screen, the service-restart blip, an upgrade under
a long reader, the heal's longest lock, the idle restart and a too-new install.

> [!NOTE]
> **Erratum E1 (2026-11-26).** The consequence *"The re-adjudication swallows its lock timeout (#1175)"* was fixed by
> PR [#1195](https://github.com/cairn-ehr/cairn-ehr/pull/1195): db/043 re-raises `55P03`, so decision 7's retry sees it,
> and `cairn-sync run` re-adjudicates routinely (#912). The trade it bought, one long-held lock failing every pass,
> is [#1197](https://github.com/cairn-ehr/cairn-ehr/issues/1197).
