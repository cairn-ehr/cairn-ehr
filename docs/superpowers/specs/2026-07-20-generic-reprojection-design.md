# Design — generic reprojection: registration is the wiring (issue #208)

**Date:** 2026-07-20 · **Issue:** [#208](https://github.com/cairn-ehr/cairn-ehr/issues/208)
(2026-07-15 review, finding D3) · **Outcome:** ADR-0057 + code + a measured number ·
**Scope:** design + implementation + Mac-measured full-replay cost (Pi5 re-run is a follow-on).
Load-bearing for [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) (ADR-0056
reclassify-then-reproject).

## 1. The problem as filed

Projections are populated by ~15 `AFTER INSERT … FOR EACH ROW WHEN (event_type = …)` triggers on
`event_log` (`db/010`–`db/034`). A `CREATE OR REPLACE` of a trigger function heals only *future*
inserts; rows already materialized under buggy logic stay wrong. The tree's only backfill is the
bespoke `cairn_demographic_backfill()` (`db/013`), and `db/013` runs it on **every connect**
(`connect_and_load_schema` replays every migration each connect) — a `DISTINCT ON` sequential scan
over `event_log`, which has **no `event_type` index** (`db/001`: only `(hlc_wall, hlc_counter,
node_origin)` and `(patient_id)`). Cost is linear in log size and unmeasured at Bet-B volume
(2,004,000 events). Each future projection fix would force its own ad-hoc migration-embedded
backfill with its own replay-safety analysis. ADR-0045 is the worked example: it shipped
read-side only, and trigger-populated winner tables settled under the old comparison were healed
only where the one per-slice backfill existed.

## 2. What the tree adds to the filing (investigated 2026-07-20)

**F1 — the existing pattern duplicates winner logic in a second dialect.**
`cairn_demographic_backfill()` re-expresses the trigger's winner comparison twice more — as a
`DISTINCT ON` sort *and* as an `ON CONFLICT … WHERE` guard — kept in lock-step with the trigger
by hand and by test. A rule that says "every projection ships a set-based backfill" would
institutionalize that drift risk per slice, forever. `db/033:379` already documents a second
projection that connect-replay does *not* backfill.

**F2 — every Cairn projection is already arrival-order-independent.** Set-union sync applies
events in arbitrary order and the multi-node convergence suites pin exactly that. Two
consequences: (a) a replay of `event_log` through *the same apply logic the live trigger uses*
converges in any scan order; (b) heal-mode replay never needs a downgrade — a stale winner row is
itself some event's tuple, so it is dominated by the true maximum under the corrected comparison.
The one class replay cannot heal is *wrote-garbage* (a projected value derived from no event,
e.g. a mis-parsed field): that needs delete-first rebuild.

**F3 — the spine for "register, don't wire" already exists.** The ADR-0048 registry
(`cairn_event_twin_check`) established per-type register-by-row with registration-time
fail-closed validation; `node_schema` (`db/038`) already gives the loader a recorded-vs-embedded
generation signal — a ready-made "something actually changed" gate.

**F4 — the #266 coupling is a safety ordering, not a convenience.** ADR-0056 decision 4:
reclassification must **re-adjudicate the deferred door gates first, then reproject survivors**.
The reprojection mechanism must therefore be *unable* to grant power to a deferred event, even
when an operator runs it by hand mid-upgrade.

## 3. Decisions

### D1 — One code path: registration is the wiring

A new registry table **`cairn_projection_apply`**:

| column | meaning |
|---|---|
| `event_type` | exact type, or the reserved `'*'` for type-independent projections |
| `apply_fn` | name of a `fn(event_log) RETURNS void` — the projection's entire fold step |
| `projection_tables text[]` | the tables this fn writes (rebuild-mode scope metadata) |
| `run_order` | deterministic dispatch order among one event's several projections |

PK `(event_type, apply_fn)` — one type may feed several projections (e.g.
`clinical.medication.asserted` → `medication_statement` *and* the dose-timeline seed) and one fn
may serve several types (e.g. the identity link/unlink pair). ADR-0048 discipline applies:
registration-time fail-closed validation (the registered `apply_fn(event_log)` must exist —
`to_regprocedure`, same as `check_fn`), `REVOKE INSERT/UPDATE/DELETE … FROM PUBLIC`, and a
row-count guard mirrored in Rust + SQL (the #212 pattern).

**One dispatcher trigger** replaces all per-type projection triggers:
`AFTER INSERT ON event_log FOR EACH ROW EXECUTE FUNCTION cairn_projection_dispatch()` — look up
registry rows for `NEW.event_type` plus `'*'`, call each `apply_fn(NEW)` in `run_order`. The
existing trigger functions are mechanically refactored to callable form (`NEW` → parameter `e`;
bodies otherwise unchanged), the per-type triggers and their `WHEN` clauses are dropped. The
`'*'` rows carry the type-independent projections: the `db/002` twin materialization and the
`db/008` surrogate interning.

An unregistered projection has no way to exist: nothing else fires on `event_log` insert.

### D2 — `cairn_reproject(p_prefix text, p_rebuild boolean DEFAULT false)`

Scan `event_log WHERE event_type LIKE p_prefix || '%'`, feed each event through **the identical
dispatch** the live trigger uses. Returns/records per-type event counts and elapsed time in a
node-local **`reproject_log`** table (the operational record and the test observable — like
`node_schema`, never on the wire).

- **Heal mode** (default): no deletes. Convergence argument is F2(b): insert-or-better applies
  converge to the corrected winner in any order; already-correct rows incur no write.
- **Rebuild mode** (wrote-garbage defects only): `TRUNCATE` a registered projection table only
  when **every** registry row referencing that table falls inside `p_prefix`; otherwise refuse
  with a legible message ("widen the scope or use heal"). This keeps rebuild generic while making
  it impossible to wipe a shared table (e.g. `cairn_event_twin`, owned by `'*'`) under a narrow
  prefix and silently lose the unmatched types' rows.

**New index:** `event_log (event_type text_pattern_ops)` — serves prefix-scoped replay, #266's
exact-type scans, and the twin worklist.

### D3 — The #266 safety seam: `cairn_replay_eligible(e event_log)`

The replay path routes every candidate event through
`cairn_replay_eligible(e) RETURNS boolean`. Today it is constantly `true` (no deferred events can
exist in `event_log` yet — the door still fail-closes on unknown types until #265). #265's
explicit deferred marker hooks in *here, and only here*; #266's flow becomes: re-adjudicate the
deferred gates → clear/flag per event → `cairn_reproject(type)`. A manual mid-upgrade reproject
therefore *cannot* grant power to an unadjudicated deferred event — ADR-0056 decision 4's
"no unattested suppression at every instant" holds by construction, not by operator discipline.
(The live-insert path needs no such filter: an event being inserted through a door was adjudicated
by that door.)

### D4 — Loader integration: heal on generation change, never on every connect

`cairn_demographic_backfill()` and its every-connect call are **deleted** — the generic mechanism
subsumes them. Its original job (the ADR-0012 carried-not-projected catch-up) arises exactly when
new projection capability arrives, and new projection capability arrives only via a code-plane
update — i.e. a schema-generation change.

`connect_and_load_schema` gains one step, inside the existing `SCHEMA_LOAD_LOCK`, after replay +
stamp: **if the recorded generation differed from the embedded one (or was unknown), run
`cairn_reproject('')`** (full heal replay). Unchanged generation → zero reprojection work on
connect. Unknown generation (fresh DB: free no-op; hand-built rig: converges once) errs toward
healing. Manual entry point: **`cairn-node reproject [--prefix P] [--rebuild]`** for operators,
tests, and #266.

Planning must check whether cairn-sync's subset loader loads any projection migrations; if it
does, it gets the same gated step; if not, that fact gets a pinning comment/test.

### D5 — The written rule (ADR-0057, spec §9.6)

> **A projection lives only in its registered apply function; healing is generic replay, run by
> the loader on generation change.** A projection change therefore ships *inside* a
> schema-generation bump and heals automatically; a new projection registers a
> `cairn_projection_apply` row or fails CI.

Structurally enforced, not prose-enforced: a catalog test asserts no `AFTER INSERT` trigger
exists on `event_log` other than the dispatcher (the immutability guard trigger is
`BEFORE UPDATE OR DELETE`, unaffected), and the registry row-count guards pin membership.
ADR-0057 refines ADR-0048 (registry dispatch) and ADR-0045 (whose read-side-only shipping is the
worked failure example), and upholds ADR-0056 decision 4 via D3. Spec home: §9.6 (the validated
write surface / registry), with a short §6.5 cross-reference for the reclassification path.

## 4. Honest flags (raised during design, carried into planning)

- **Hot-path cost.** The dispatcher puts a registry lookup + dynamic `EXECUTE` on the live write
  path, replacing the per-type triggers' static `WHEN` dispatch. Bet-B holds 13× headroom
  (B1 p95 3.99 ms @ 2M events). The measurement (§5) includes a before/after write-path number,
  not just the replay number. **Fallback, if the delta is ugly:** keep per-type triggers as thin
  wrappers over the registered apply fns, use registry dispatch only for replay — retreats on
  shared *dispatch* while keeping the single *logic* path and every other decision intact.
- **Sibling-projection reads.** Dispatch order among one event's projections is deterministic
  (`run_order`), but planning must verify no apply fn reads a *sibling* projection's output for
  the same event; any such dependency gets an explicit `run_order` justification, not an accident
  of trigger-name alphabetics.
- **Retroactive healing posture.** Pre-clinical: no production data exists; rigs that settled
  winners under pre-ADR-0045 comparisons are wiped, not healed. The mechanism is forward-looking.

## 5. Testing and measurement

**TDD order:**
1. Registry registration fail-closed (bogus `apply_fn` refused at load; SQL mirror per #212).
2. Dispatcher equivalence — **the entire existing suite passing unchanged is the primary guard**
   (same projection rows as the old per-type triggers, pinned by every existing projection test).
3. Generic tamper-then-heal: loop every registered type with a sample event, tamper the
   projection row, `cairn_reproject` converges it back (generalizes #214's heal test).
4. Rebuild-scope refusal (narrow prefix over a shared table refuses legibly) +
   truncate-then-rebuild equals trigger-built state.
5. Loader gating observed via `reproject_log` (no reproject when generation unchanged; one when
   it changes).
6. `cairn_replay_eligible` seam pinned (a false-returning stub keeps events out of replay).
7. Catalog guard: no non-dispatcher `AFTER INSERT` trigger on `event_log`.

**Measurement (Mac :5532 now; Pi5 follow-on):** at ~2M events via the walking-skeleton
generator: (a) full `cairn_reproject('')` wall-clock + per-type breakdown from `reproject_log`;
(b) write-path p95 before/after the dispatcher change (B1 methodology). Both go into ADR-0057
with the hardware caveat named; a follow-on issue tracks the authoritative Pi5/NVMe re-run
(the Bet-B rig, so numbers are comparable).

## 6. Out of scope / follow-ons

- **#265/#266 themselves** — the door change and the re-adjudication flow consume this mechanism;
  D3 is the only piece of them built here.
- **Pi5 re-run** of both numbers (follow-on issue at PR time).
- **Per-row generation watermarks** (reproject only the types whose logic changed) — additive
  optimization if the measured full-heal cost warrants it; the registry shape does not preclude it.
- **Set-based `backfill_fn` override** (design Approach C) — additive registry column, only if
  the Pi number demands it for some hot projection.
