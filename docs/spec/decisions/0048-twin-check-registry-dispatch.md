# ADR-0048 — The per-type twin/floor-check registry: one stable dispatcher, register-by-row, unified check-fn signature

- **Status:** Accepted (refines [ADR-0039](0039-globalise-authored-legibility-twin.md), [ADR-0022](0022-validated-submit-surface-the-write-path.md); closes [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173))
- **Date:** 2026-07-13

## Context

`cairn_event_twin(p_type, b)` is the per-event-type hook on the validated write path
([ADR-0022](0022-validated-submit-surface-the-write-path.md)): for a submitted event it runs that
type's **structural floor check** (raising on violation) and returns its **plaintext legibility
twin** — carrying an authored twin verbatim, or raising / degrading to a mechanical skeleton per the
twin policy ([ADR-0039](0039-globalise-authored-legibility-twin.md)). It is a safety-critical
in-database gate: a defect here silently drops a floor check on the write path (§9, [principle 12](../index.md)).

The hook was implemented as a **hand-copied `IF/ELSIF` dispatch chain**. Because each new event-type
slice needs to add exactly one branch, and Postgres `CREATE OR REPLACE FUNCTION` replaces the *whole*
body, every slice re-declared `cairn_event_twin` by **copying the entire growing chain** from the
previous migration and appending its one new `ELSIF`. Eleven migrations each carried a full copy of
the chain.

This is a latent safety-floor regression with **no error surface**. If any migration's copy of the
chain were stale — omitting a branch that a *later* type-slice had added, or transcribing an existing
branch subtly wrong — the last-loaded `CREATE OR REPLACE` wins and **silently drops the floor check
for other event types**. Nothing fails; the check simply stops running, and the omission is only
visible by reading and diffing eleven near-identical copies by eye. `db/031`'s header comment already
warned in prose that this copy pattern was a hazard — the warning is evidence the structure was wrong,
not a mitigation. The forces: the per-type check + twin requirement is genuinely *per-type additive
data*, but it was being expressed as *copied executable dispatch code*, so a data addition kept
rewriting a safety-critical function body.

## Decision

Express the per-type structural check and twin requirement as a **registry row**, not a copied
dispatch branch.

- A locked table **`cairn_event_twin_check(event_type PK, check_fn text, twin_required_msg text)`**
  holds one row per registered event type — the sibling of the existing `event_type_class` classifier
  table. `check_fn` is the name of that type's structural check function (nullable ⇒ no structural
  floor for the type); `twin_required_msg` is the raise message when an authored twin is mandatory and
  absent (nullable ⇒ absent twin degrades honestly to a skeleton per ADR-0039). The two columns are
  independent.
- `cairn_event_twin` is declared **exactly once** (`db/005_submit.sql`) and **never re-declared**. It
  reads the row for `p_type` and **dispatches dynamically** over the registry:
  `EXECUTE format('SELECT %I($1, $2)', check_fn) USING p_type, b`.
- **Every** per-type check function shares **one signature** — `(p_type text, b jsonb) RETURNS void`,
  working by RAISE-on-violation — so a single dynamic call site fits all of them.
- A registered `check_fn` is **validated to exist at load time** by a fail-closed
  `BEFORE INSERT OR UPDATE` trigger on the registry table (`to_regprocedure(check_fn || '(text, jsonb)')
  IS NULL ⇒ RAISE`). A slice that registers a typo'd or not-yet-created check function fails loudly on
  schema load, with nothing to remember.
- A **new event type registers one additive row** in its own migration and **never touches the
  dispatcher**. The table is `REVOKE`d from `PUBLIC` (migration-only, like `event_type_class`); the
  `SECURITY DEFINER` submit path reads it as its owner.

### Alternatives rejected

- **Keep the copied `IF/ELSIF` chain but guard it with a test that diffs the copies.** Detects a stale
  copy after the fact but leaves the wrong structure — a copied safety-critical body — in place; the
  copy hazard is designed out, not policed.
- **Merge `cairn_event_twin_check` into the existing `event_type_class` table** (one classifier row per
  type carrying class + check + twin). Attractive convergence, but it couples an unrelated concern
  (event-class taxonomy) to the floor-check wiring in one migration change, and `event_type_class`
  predates this and is read on other paths. Left as a **deliberate future convergence**, not taken here.

## Consequences

- **Easier:** adding an event type is now a one-line additive `INSERT` in the type's own migration; no
  slice ever copies or re-declares the dispatcher body, so the stale-copy floor-regression class is
  eliminated by construction. A registration mistake fails **loudly at load time** instead of silently
  at some later submit.
- **Binding invariants on all future slices** (this is the load-bearing part):
  1. `cairn_event_twin` is declared in **exactly one** migration (`db/005`). Enforced mechanically by
     the no-DB guard test `crates/cairn-node/tests/twin_dispatch_single_source.rs`, which scans `db/*.sql`
     and fails if more than one file declares the function — catching any reintroduction of the copy
     pattern on every `cargo test` / CI run.
  2. Every per-type check function is `(p_type text, b jsonb) RETURNS void`. A check written to any other
     signature cannot be registered (the load-time trigger rejects it) and cannot be dispatched.
  3. A missing or mis-signed check function **fails closed** — refused at registration by the trigger,
     and (for a signature broken *after* registration) raised at dispatch by the `EXECUTE`. A floor check
     is never silently skipped.
- **New bet — the first dynamic SQL in the in-DB floor.** `cairn_event_twin` now builds and `EXECUTE`s a
  statement, where the rest of the floor is static. This is bounded and safe: the dispatched name comes
  only from the **locked, migration-only** registry (never user input), is `%I`-quoted, and every failure
  mode (unknown name, wrong signature) is **fail-closed** and caught at load time by the validation
  trigger. The bet fails if a future change lets non-migration input reach `check_fn`, or relaxes the
  load-time validation — reviewers of registry changes must hold both.
- **Scope:** floor-wiring refactor only. **No wire / event-format / behaviour / spec-prose change** — the
  same checks run for the same types with the same outcomes (behaviour preservation is carried by the full
  existing suite staying green; the seed rows were transcribed verbatim from the winning chain). This ADR
  sits **below the spec line**: no aspect document changes, and the spec-version bump records the decision,
  not a prose edit.
- **Deliberately not done:** `event_type_class` is **not** merged into the registry (see rejected
  alternative) — a possible future convergence, out of scope here.

**Implementation anchor:** the dispatcher, registry table, and load-time validation trigger live in
`db/005_submit.sql`; the single-source invariant is guarded by
`crates/cairn-node/tests/twin_dispatch_single_source.rs`.
