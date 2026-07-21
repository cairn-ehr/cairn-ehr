# ADR-0057 — Generic reprojection: a projection lives only in its registered apply function; healing is generic replay

- **Status:** Accepted (refines [ADR-0048](0048-twin-check-registry-dispatch.md)'s register-by-row dispatch and [ADR-0045](0045-collation-independent-projection-tiebreaks.md)'s read-side-only shipping; upholds [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) decision 4; load-bearing for [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266))
- **Date:** 2026-07-21

## Context

Projections — the [§9.4](../language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)
trigger-maintained incremental tables that turn the append-only `event_log` into the chart, the
demographic winners, the identity connected-component, and the clinical worklists — were each
populated by a per-type `AFTER INSERT … FOR EACH ROW WHEN (event_type = …)` trigger. A
`CREATE OR REPLACE` of one of those trigger functions heals only **future** inserts; every row
already materialised under the old logic stays wrong. Projection logic is
[§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius) safety-critical, so "the
fix ships but the already-projected rows silently keep the old answer" is exactly the failure class
the floor exists to prevent.

[ADR-0045](0045-collation-independent-projection-tiebreaks.md) is the worked example: it corrected
the winner tiebreak but shipped **read-side only**, so trigger-populated winner tables that had
already settled under the old comparison were healed only where a bespoke per-slice backfill
happened to exist. The tree's *only* such backfill was `cairn_demographic_backfill()`, and it ran
on **every connect** — `connect_and_load_schema` replays every migration each connect — as a
`DISTINCT ON` sequential scan over an `event_log` that carried **no `event_type` index**. Cost was
linear in log size, unmeasured at Bet-B volume, and paid on every connect whether or not anything
had changed. Worse, the pattern institutionalised drift: `cairn_demographic_backfill()`
re-expressed its trigger's winner comparison **twice more** (a `DISTINCT ON` sort *and* an
`ON CONFLICT … WHERE` guard), kept in lock-step with the trigger by hand and by test. A rule of
"every projection ships its own set-based backfill" would have made that per-slice drift risk
permanent.

Two properties of the codebase made a single generic mechanism possible instead of fifteen bespoke
backfills:

- **Every Cairn projection is already arrival-order-independent.** Set-union sync applies events in
  arbitrary order and the multi-node convergence suites pin exactly that ([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)).
  So a replay of `event_log` through *the same apply logic the live trigger runs* converges in any
  scan order, and a heal-mode replay never needs a downgrade path: a stale winner row is itself some
  event's tuple, so it is dominated by the true maximum under the corrected comparison.
- **The "register, don't wire" spine already existed.** [ADR-0048](0048-twin-check-registry-dispatch.md)
  established per-type register-by-row with load-time fail-closed validation for the floor-check
  hook, and `node_schema` ([§9.4](../language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon),
  the recorded-vs-embedded schema-generation signal) already gave the loader a ready-made
  "something actually changed" gate.

The mechanism is also load-bearing for [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md)
decision 4: reclassification of a deferred unknown-type event must **re-adjudicate the deferred door
gates first, then reproject the survivors**, and the reprojection step must be structurally *unable*
to grant power to an unadjudicated event even when an operator runs it by hand mid-upgrade.

## Decision

### 1. One code path — registration is the wiring

A projection's fold step is expressed **once**, as a registry row, never as copied dispatch code.

- A locked table **`cairn_projection_apply(event_type, apply_fn, projection_tables text[],
  run_order, heal_safe, PK(event_type, apply_fn))`** holds one row per `(type, projection)` pair.
  `apply_fn` names a `fn(event_log) RETURNS void` — the projection's entire fold step. The composite
  PK carries both fan-outs: one type may feed several projections (a medication assertion feeds
  `medication_statement` *and* the dose-timeline seed) and one fn may serve several types (the
  identity link/unlink pair).
- **One dispatcher trigger** — `cairn_projection_dispatch()` behind
  `cairn_projection_dispatch_trg`, `AFTER INSERT ON event_log FOR EACH ROW` — replaces every
  per-type projection trigger: it looks up the registry rows for `NEW.event_type` and calls each
  `apply_fn(NEW)` in `run_order`. The former trigger bodies were mechanically refactored to callable
  form (`NEW` → parameter `e`, bodies otherwise unchanged) and their `WHEN` clauses dropped. An
  unregistered projection has **no way to fire**: nothing else runs on an `event_log` insert.
- **`heal_safe`** marks whether replaying the fn over an existing projection converges
  (insert-or-better). Idempotent winner projections are `heal_safe = true`; counter-shaped applies
  (e.g. `patient_chart_apply`'s note-count increment) are `heal_safe = false` — re-running them over
  an already-populated table would double-count, so heal mode **skips** them and rebuild mode
  (replay-from-truncate) heals them. New projections should be idempotent; a `heal_safe = false` row
  needs a justifying comment.
- **Exact types only — no `'*'` wildcard.** The design reserved a type-independent apply row, but
  the legibility twin turned out to be door-materialised (a column written by both admission doors),
  not a trigger projection, so there is no type-independent projection. Every apply fn registers
  under the exact types it handles; the wildcard mechanism was dropped (YAGNI, and a wildcard row
  would be a standing invitation to a projection that fires on types it was never reviewed against).
- **[ADR-0048](0048-twin-check-registry-dispatch.md) discipline applies in full.** Registration is
  validated fail-closed at load time — a `BEFORE INSERT OR UPDATE` trigger refuses a row whose
  `apply_fn(event_log)` does not exist (`to_regprocedure`) or whose `projection_tables` name a table
  that does not exist. The table is `REVOKE`d from `PUBLIC` (migration-only). Every converted apply
  fn has `EXECUTE … (event_log)` revoked from `PUBLIC`, and each dynamic-dispatch fn pins
  `SET search_path = public` so a dispatched `%I` name cannot resolve into an attacker-shadowed
  schema. Membership is pinned by a row-count guard mirrored in **both** Rust and SQL (the
  [#212](https://github.com/cairn-ehr/cairn-ehr/issues/212) two-place pattern), so a missed
  registration fails CI rather than drifting silently.

### 2. `cairn_reproject(p_prefix, p_rebuild, p_source)` — heal and rebuild

`cairn_reproject` scans `event_log WHERE event_type LIKE p_prefix || '%'` and feeds each event
through **the identical dispatch the live trigger uses** — the single-logic-path guarantee extends
to replay, not just to live inserts. It records per-type counts and elapsed time in a node-local
**`reproject_log`** (the operational record and the test observable, never on the wire).

- **Heal mode** (default, `p_rebuild = false`): no deletes. Convergence follows from
  arrival-order-independence — insert-or-better applies converge to the corrected winner in any scan
  order, and an already-correct row incurs no write. Heal converges the **wrong-winner** class
  (a projected row that is dominated under corrected logic by an honest event's tuple). It does
  **not** heal *wrote-garbage* — a projected value derived from no event, or an out-of-band tamper
  whose value **ties** the winner comparison rather than losing it — because replay can only insert
  a better candidate, never delete an existing tuple that nothing dominates; that class is the
  province of rebuild. Heal mode additionally **skips** `heal_safe = false` rows, reporting them in
  `reproject_log.skipped_fns` — honest degradation is only honest if it is visible.
- **Rebuild mode** (`p_rebuild = true`, wrote-garbage defects only): `TRUNCATE`s a registered
  projection table only when **every** registry row that writes that table falls inside `p_prefix`;
  otherwise it refuses with a legible message. This keeps rebuild generic while making it impossible
  to wipe a table fed by several types (e.g. `patient_chart`) under a narrow prefix and silently
  lose the unmatched types' rows.
- **Idempotency is a property of the apply fns, achieved by keying on event identity, never on
  observation shape.** Where a projection is an append-only alarm/worklist table (the identity and
  medication conflict flags), replay-safety was secured by adding `content_address` to the row's
  unique key with `NULLS DISTINCT` — so a replayed event dedups against its own prior projection by
  **event identity**, pre-fix legacy rows (null `content_address`) are left untouched and undeleted
  (*never erase, always overlay*, [principle 2](../index.md#founding-principles-the-lens-for-every-decision)),
  and two genuinely distinct events that happen to share an observation shape are **not** collapsed.
  A blanket "`ON CONFLICT DO NOTHING` on the natural key" would have been wrong for these tables — it
  dedups by what was observed, not by which event observed it.
- **Replay runs in the lenient-apply posture.** The [#192](https://github.com/cairn-ehr/cairn-ehr/issues/192)
  patient-consistency helper and the component-size caps RAISE on pathology unless the
  transaction-local `cairn.remote_apply` GUC is on (set by `apply_remote_event`). Replayed events
  include remotely-admitted ones, so `cairn_reproject` sets `cairn.remote_apply = 'on'` for its
  transaction: an event a door already admitted stays admitted; replay heals and clamps-and-flags
  where the strict door would refuse. This is [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md)'s
  *gate the effect, not the presence* applied to replay — reprojection re-runs the projection, it is
  not a second authoring door.
- **The apply fns are invoked set-based, not row-by-row.** For each `(event_type, apply_fn)` pair
  the replay runs one full-table pass — `count(*)` over a subquery whose target list calls the
  apply fn — instead of a per-event PL/pgSQL loop. The apply fn is `VOLATILE`, so the planner can
  never prune it: it still runs **exactly once per eligible row**, but the per-event loop, the
  per-event dynamic `EXECUTE`, and the composite-value marshalling are gone. `ORDER BY` is dropped
  (licensed by arrival-order-independence); fn interleaving becomes per-fn-full-pass (licensed
  because sibling apply fns for one type write disjoint tables). This is the mechanism behind the
  heal-mode speedup in decision 5; it targets *loop* overhead and therefore leaves rebuild cost —
  which is dominated by the apply fns' own write work — essentially unchanged, by construction.

### 3. The [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) safety seam: `cairn_replay_eligible(e event_log)`

Every candidate event on the replay path is routed through
`cairn_replay_eligible(e) RETURNS boolean`. Today it is constantly `true` — no deferred events can
exist in `event_log` yet, because the remote door still fail-closes on unknown types until
[#265](https://github.com/cairn-ehr/cairn-ehr/issues/265). [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md)'s
explicit deferred marker hooks in **here, and only here**; #266's reclassification flow becomes
*re-adjudicate the deferred gates → clear or flag per event → `cairn_reproject(type)`*. A manual
mid-upgrade reproject therefore **cannot** grant power to an unadjudicated deferred event —
ADR-0056 decision 4's *"no unattested suppression at every instant"* holds by construction, not by
operator discipline. The live-insert path needs no such filter: an event inserted through a door was
adjudicated by that door.

### 4. Loader heal on generation change, never on every connect

`cairn_demographic_backfill()` and its every-connect call are **deleted**; the generic mechanism
subsumes them. Its real job — the [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
carried-not-projected catch-up — arises exactly when new projection capability arrives, and new
projection capability arrives only via a code-plane update, i.e. a schema-generation change.

Both loaders — `cairn-node`'s full `connect_and_load_schema` and `cairn-sync`'s subset loader (which
carries the projection migrations its apply path depends on) — gain one gated step inside the
existing `SCHEMA_LOAD_LOCK`: **if the recorded generation differs from the embedded one (or is
unknown), run `cairn_reproject('', false, 'loader')`** (full heal replay). Unchanged generation →
zero reprojection work on connect. Unknown generation (fresh DB: free no-op; hand-built rig:
converges once) errs toward healing.

The gated heal runs **before** the `node_schema` generation stamp, deliberately. If the heal errors,
the stamp never runs, the recorded generation stays at its old value, and the next connect retries
the full replay-then-heal — the same loud, self-retrying failure mode a broken migration file
already has (a bad `db/*.sql` blocks connect until fixed; it never silently half-applies).
Stamp-then-heal would invert this: a heal failure *after* the stamp would leave the generation
already advanced, the next connect would read `recorded == embedded`, skip the heal, and the
projections would stay **silently stale** — the worst failure mode, and the reason the order is
load-bearing rather than cosmetic. Operators, tests, and #266 reach the mechanism by hand via
**`cairn-node reproject [--prefix P] [--rebuild]`**.

### 5. The written rule, structurally enforced

> **A projection lives only in its registered apply function; healing is generic replay, run by the
> loader on generation change.**

A projection change therefore ships *inside* a schema-generation bump and heals automatically; a new
projection registers a `cairn_projection_apply` row or fails CI. This is enforced structurally, not
by prose: a catalog guard asserts that the **only** `AFTER INSERT` trigger on `event_log` is the
dispatcher (the immutability guard is `BEFORE UPDATE OR DELETE`, unaffected), and the registry
row-count guards pin membership in both Rust and SQL.

## Consequences

- **[ADR-0045](0045-collation-independent-projection-tiebreaks.md)'s failure class is closed
  generically.** A future read-side projection fix heals its already-materialised rows automatically
  on the next generation bump, with no per-slice backfill to write, review, or keep in lock-step. The
  drift risk `cairn_demographic_backfill()` embodied — winner logic re-expressed in a second dialect
  — is designed out: there is exactly one expression of each projection's fold.
- **[ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) decision 4 gets its mechanism.**
  The reclassify-then-reproject flow #266 needs is this ADR's `cairn_reproject` behind the
  `cairn_replay_eligible` seam; the "re-adjudicate first, reproject second" ordering is enforced by
  the seam, not by hope.
- **New bet: a registry lookup + dynamic dispatch now sits on the live write path**, replacing the
  per-type triggers' static `WHEN`. The measurement (below) shows the delta is far inside budget. The
  documented fallback, never needed: keep per-type triggers as thin wrappers over the registered
  apply fns and use registry dispatch only for replay — this retreats on shared *dispatch* while
  keeping the single *logic* path and every other decision intact.
- **Reviewers of registry changes must hold three invariants** (inherited from
  [ADR-0048](0048-twin-check-registry-dispatch.md)): load-time validation is fail-closed; the
  registry stays migration-only and `REVOKE`d; dynamic-dispatch fns keep their `search_path` pin. And
  one new one: an apply fn registered `heal_safe = true` must genuinely be insert-or-better —
  validation checks the fn *exists*, not that it is idempotent.
- **Retroactive healing is forward-looking only.** Pre-clinical, no production data exists; rigs that
  settled winners under pre-ADR-0045 comparisons are wiped, not healed. The mechanism heals the fleet
  from here on, not the past.

### Measured cost (Bet-B volume; Mac-measured, Pi5 re-run is the authoritative follow-on)

Measured on an Apple Silicon Mac dev box (PostgreSQL 18.1 / Postgres.app, `cairn_pgx` 0.3.0) over a
walking-skeleton-generated corpus of **2,006,000 events across 200 patients** — composition
`note.added` ≈ 90% (1,805,420), `demographic.field.asserted` ≈ 10% (200,380), `patient.created` 200.
All figures were cross-verified read-only against the persisted `reproject_log` before being trusted.

- **Write-path latency through the live dispatcher** (2,000 single-row inserts against the full
  ~2M-row log): **p50 0.076 ms, p95 0.236 ms.** A single 1278.39 ms max is a checkpoint/autovacuum
  artefact of measuring immediately after a 2M-row bulk load, not a steady-state tail (p95 already
  excludes it). The Bet-B B1 budget this is sanity-bounded against is **p95 3.99 ms @ 2M events on
  Pi-class hardware**, so 0.236 ms clears it by ~17×. This is a *cross-rig* bound, not a same-rig
  before/after: the honest same-rig write-path A/B is the Pi5 re-run.
- **Heal replay** (`cairn_reproject('')`, the operationally relevant path the loader runs): the
  **200,580** events eligible for a registered heal-safe apply fn (`patient.created` +
  `demographic.field.asserted`) in **2.098 s** (~95.6k events/s) with the set-based invocation, down
  from **12.207 s** (~16.4k events/s) with the per-event PL/pgSQL loop it replaced — a **5.8×**
  speedup isolating the loop-dispatch overhead. The 1,805,420 `note.added` rows are correctly skipped
  (`patient_chart_apply` is `heal_safe = false`).
- **Rebuild replay** (`TRUNCATE` + full replay of **all 2,006,000** events, `note.added` included):
  **54 min 32 s** post-optimisation, **49 min 1 s** pre-optimisation — the ~11% gap is run-to-run
  bloat/checkpoint variance from four consecutive full-log operations in one session, not a
  regression. Rebuild cost is **loop-invariant by construction**: it re-applies every event, so its
  cost is dominated by the apply fns' own write work (mostly ~1.8M note-count increments), not by
  loop dispatch — the same corpus loaded through the *live* per-row dispatcher (the door-equivalent
  ingest path) ran at ~1.2 ms/event, so rebuild's ~1.47–1.63 ms/event **is** re-ingest cost, and the
  set-based change (which removes loop overhead, not apply work) had nothing left to remove here.
- **The stop-gate story, recorded honestly.** The plan carried a "~30 min, else STOP and reopen the
  set-based-backfill fallback" gate. It was written against **heal** mode — the default, operational
  path the loader runs — which lands comfortably inside it (2.1 s). The first rebuild measurement
  **tripped** the gate (49 min); rather than reopen the fallback immediately, the per-event loop was
  optimised to the set-based invocation (5.8× on heal), then rebuild was re-measured and **judged
  acceptable as-is**: it is a rare, human-supervised, narrow-prefix recovery operation whose cost
  mirrors ingest by construction, not a hot path anything invokes automatically. Rebuild mode is
  therefore explicitly **not** covered by a low-latency SLA; if a future recovery runbook ever
  invokes it at Bet-B-or-larger volume, that path needs its own budget, and 49–54 min at 2M events is
  the honest number to size against.
- **The Pi5/NVMe re-run is the authoritative follow-on** (the Bet-B B1 rig, so numbers are
  comparable), tracked as [issue #272](https://github.com/cairn-ehr/cairn-ehr/issues/272). These
  Mac numbers establish the shape and clear the budget
  by a wide margin; they are not the same-rig A/B the write-path claim ultimately rests on.

**Implementation anchors:** the registry, dispatcher, load-time validation, and eligibility seam
live in `db/005_submit.sql`; `cairn_reproject`, `reproject_log`, and the `event_log_type_idx`
prefix index live in `db/039`; the gated loader heal lives in both loaders; the sole-dispatcher
catalog guard and registry row-count pins are in `crates/cairn-node/tests`.
