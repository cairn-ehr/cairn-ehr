# Design — `cairn_event_twin` registry-driven dispatch (issue #173)

**Date:** 2026-07-13 · **Issue:** [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173) ·
**ADR:** ADR-0048 (to be written) · **Kind:** internal floor-mechanism refactor, **no behaviour change**

## Problem

Each clinical/identity event slice adds its structural floor check by
`CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)` that **copies the entire, growing
IF/ELSIF dispatch chain verbatim** from the previous latest migration and appends one `ELSIF`. Because
the function is `CREATE OR REPLACE` and the newest migration loads last, its body wins at runtime.

As of today **11 migrations** re-declare the function (db/005, 010, 011, 015, 018, 023, 024, 025, 031,
032, 033), each carrying a full copy of every prior branch (15 event types across 9 check functions).

**Why it is safety-relevant:** the copy is manual. If a future slice copies a **stale** body (one
missing a branch added in an intermediate migration), the newest `CREATE OR REPLACE` **silently drops
the floor check** for those other event types — a floor regression with no error, no failed compile,
caught only if a test happens to exercise the dropped branch. `db/031`'s own header comment warns about
this trap, which is the signal it should be *designed out*, not documented around.

## Goal & non-goals

- **Goal:** eliminate the verbatim-copy hazard. A new slice registers its `(event_type → check_fn,
  twin_required_msg)` **additively** and never re-declares the dispatcher.
- **Hard requirement:** **zero behaviour change.** Every currently-accepted event stays accepted with
  the identical stored twin; every currently-rejected event stays rejected with the identical message.
- **Non-goal (this PR):** merging the sibling `event_type_class` registry into the new table (the
  larger "one per-type floor registry" convergence). Designed *merge-ready*, deferred to its own slice.

## Decision (why Option B)

Chosen mechanism: **a registry table + a single dynamic dispatcher, with all per-type check functions
unified to the signature `(p_type text, b jsonb)`.**

Two variants were weighed (a third, a centralized *static* IF/ELSIF declared once in db/005 and edited
per slice, was rejected: it removes the copy hazard but forces every slice to edit the foundational
door and grows an unbounded chain):

- **A — registry + dynamic dispatch, keep the mixed check-fn signatures** (`(b)` vs `(p_type,b)`) via a
  `check_takes_type` flag column. Zero churn on the check fns; but a *permanent* mixed-convention tax
  paid by every future slice.
- **B — registry + dynamic dispatch, unify all check fns to `(p_type,b)`** (chosen). A *one-time,
  bounded* cost (migrate 4 legacy fns) buys a permanently clean surface — no flag, one dispatch form,
  simplest schema and validation — for the long tail of future clinical types (diagnoses, progress
  notes, prescriptions, referrals, pathology, …). This dispatcher is foundational infra those dozens of
  slices inherit, so paying once for the clean invariant beats a small permanent tax.

**Linchpin making B's cost genuinely one-time:** `(p_type, b)` is the *forever* signature. The per-type
checks validate the event **body structurally** (required fields, valid UUIDs, non-empty provenance) —
culture-neutral, no DB reads; everything they need is in `b` plus `p_type` (to disambiguate a shared
check across a verb-group, e.g. `link`/`unlink`). The stateful gates — attestation, owner-gate,
target-existence — live in `submit_event`/`apply_remote_event` themselves, not the per-type check. So
`(p_type, b)` is sufficient and stable; freezing it now will not force a later re-migration.

### Facts that de-risk B (verified during exploration)

- **All 9 check fns `RETURN void`** — pure RAISE-on-violation validators. So dispatching via
  `EXECUTE format('SELECT %I($1,$2)', fn) USING p_type, b` is the exact dynamic equivalent of the
  current `PERFORM fn(...)`: run, discard result, the check RAISEs on violation. Behaviour-identical.
- **Zero external callers** of the 4 legacy fns — they are called only by the dispatcher being
  replaced. (The `identify.rs:255` occurrence is a *comment*, not a call.) Signature migration breaks
  nothing downstream.
- **`DROP` is safe** — `void` PL/pgSQL bodies create no catalog dependencies; no view/constraint
  depends on these functions.

## Design

### 1. Registry table + self-enforcing validation trigger (db/005)

A sibling to `event_type_class`, created in db/005 so it exists before the first slice INSERT (db/010):

```sql
CREATE TABLE IF NOT EXISTS cairn_event_twin_check (
    event_type         TEXT PRIMARY KEY,
    check_fn           TEXT,   -- nullable: a type may hard-require a twin with no structural check
    twin_required_msg  TEXT    -- nullable: NULL ⇒ honest skeleton-degradation allowed (ADR-0039)
);
```

The two columns are **independent** by design (more general than today, where every checked type also
hard-requires the twin). This latent generality is *not exercised* by the seed data (all 15 rows have
both columns non-null), so it is behaviour-preserving — it just lets a future type opt into
"structural check + honest twin-degradation" without a mechanism change.

A **`BEFORE INSERT OR UPDATE` trigger** validates every registered `check_fn` exists with the unified
signature at registration time — a typo'd or missing check becomes a **fail-closed load-time error**,
for this PR and every future slice, with nothing to remember:

```sql
IF NEW.check_fn IS NOT NULL
   AND to_regprocedure(NEW.check_fn || '(text, jsonb)') IS NULL THEN
    RAISE EXCEPTION 'cairn_event_twin_check: check_fn %(text,jsonb) does not exist (fail closed)', NEW.check_fn;
END IF;
```

Because each slice migration creates its check fn **before** INSERTing its row (enforced ordering
within the file), the trigger always sees the function. Residual (accepted): the trigger fires on
registry writes, not on later function changes; a future migration that broke a check fn's signature
would surface at runtime (fail-closed) rather than load time — a narrow, safe gap.

The table is locked down exactly like `event_type_class` (`REVOKE INSERT/UPDATE/DELETE FROM PUBLIC`):
it is a safety surface (a row pointing a type's check at a no-op would drop its floor); only migrations
(as owner) write it. `submit_event` reads it as its `SECURITY DEFINER` owner, so `cairn_agent` needs no
grant.

### 2. The stable dispatcher — declared **once** (db/005), never re-declared

Absorbs the generic ADR-0039 honest-degradation logic (moved out of db/015):

```sql
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin     text := b ->> 'plaintext_twin';
    v_authored boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin,'\s+','','g')) > 0;
    v_fn       text;
    v_msg      text;
BEGIN
    SELECT check_fn, twin_required_msg INTO v_fn, v_msg
      FROM cairn_event_twin_check WHERE event_type = p_type;

    -- Per-type structural floor (dynamic PERFORM). The check_fn name comes from the locked
    -- registry table (never user input); %I quotes the identifier; a missing fn RAISES
    -- (fail-closed) — but the registry trigger already refused an unregistered fn at load time.
    IF v_fn IS NOT NULL THEN
        EXECUTE format('SELECT %I($1, $2)', v_fn) USING p_type, b;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path, EVERY type).
    IF v_authored THEN
        RETURN v_twin;
    END IF;
    -- Absent/blank twin: a hard-required type RAISES; every other type degrades honestly (ADR-0039).
    IF v_msg IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_msg;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;
```

Notes:
- `v_authored` and the skeleton fallback are generic (not per-type), so they belong in the dispatcher.
- `cairn_twin_skeleton` is resolved at **runtime**, so db/015's payload-rendering improvement still
  applies (PL/pgSQL resolves the call at call time, after all migrations have loaded).
- Message text is preserved **byte-for-byte**, including the pre-existing quirk that db/020's apply
  door also emits the `submit_event:` prefix. No message changes ⇒ existing negative-path tests that
  assert on message text stay green.
- **Dynamic SQL on the floor** is a new pattern here; it is bounded and safe: the function name comes
  from a locked, migration-only table; `%I` quotes the identifier; the outcome is fail-closed; and the
  registry trigger validates existence at load time. This is a net legibility win over a 15-branch chain
  copied 10×.

### 3. Unify the 4 legacy check fns to `(p_type text, b jsonb)`

At their declaration sites, add the added parameter (unused in these four — they validate the body) and
clear the stale overload:

| check fn | declaration site(s) | change |
|---|---|---|
| `cairn_check_identifier_assertion` | db/010 | `DROP …(jsonb)` + `CREATE …(p_type text, b jsonb)` |
| `cairn_check_demographic_field` | db/011 **and** db/014 (live) | `DROP …(jsonb)` at db/011 (earliest); db/014 → `CREATE OR REPLACE …(p_type,b)` |
| `cairn_check_link_assertion` | db/018 | `DROP …(jsonb)` + `CREATE …(p_type,b)` |
| `cairn_check_repudiation_assertion` | db/025 | `DROP …(jsonb)` + `CREATE …(p_type,b)` |

The `DROP FUNCTION IF EXISTS …(jsonb)` clears any stale `(jsonb)` overload lingering on an
upgraded-in-place dev DB; on a fresh DB it is a no-op. The 5 already-`(p_type,b)` fns
(`dispute_assertion`, `identity_state_assertion`, `medication_assertion`, `medication_dose`,
`medication_reconciliation`) are untouched.

### 4. The mechanical sweep — remove every copied chain

Each migration that currently declares `cairn_event_twin` loses that block. **This removal is
mandatory:** any leftover chain-copy loads after db/005 and would override the registry dispatcher.

| migration | change |
|---|---|
| db/005 | new: registry table + trigger + REVOKEs + the stable dispatcher (replaces the trivial skeleton-only one) |
| db/010 | signature unify (identifier) · remove chain · INSERT `demographic.identifier.asserted` |
| db/011 | signature unify (field, with DROP) · remove chain · INSERT `demographic.field.asserted` |
| db/014 | signature unify (field, CREATE OR REPLACE only) — declares no chain |
| db/015 | remove chain declaration only (its ADR-0039 logic now lives in the db/005 dispatcher); **keep** the improved `cairn_twin_skeleton`, `cairn_twin_is_authored`, `cairn_twin_provenance_of`, `event_twin_provenance` |
| db/018 | signature unify (link) · remove chain · INSERT `identity.link.asserted`, `identity.unlink.asserted` |
| db/023 | remove chain · INSERT `identity.dispute.asserted`, `identity.dispute.resolved` |
| db/024 | remove chain · INSERT `identity.pending.asserted`, `identity.identify.asserted` |
| db/025 | signature unify (repudiation) · remove chain · INSERT `identity.repudiate.asserted` |
| db/031 | remove chain + the copy-hazard warning comment · INSERT `clinical.medication.asserted`, `clinical.medication-cessation.asserted` |
| db/032 | remove chain · INSERT `clinical.medication-dose-change.asserted`, `clinical.medication-dose-correction.asserted` |
| db/033 | remove chain · INSERT `clinical.medication-reconciliation.asserted`, `clinical.medication-separation.asserted` |

Each registry INSERT is idempotent (`ON CONFLICT (event_type) DO NOTHING`) and sits **beside** that
slice's existing `event_type_class` INSERT, so "register my event type" reads as one grouped block.
db/020 (apply door) is untouched — it only *calls* the dispatcher.

**The 15 seed rows** are transcribed verbatim from the current *winning* db/033 chain, so the registered
mapping is provably identical to today's runtime behaviour:

| event_type(s) | check_fn | twin_required_msg |
|---|---|---|
| `demographic.identifier.asserted` | `cairn_check_identifier_assertion` | `demographic assertion requires a non-empty authored twin (§4.5)` |
| `demographic.field.asserted` | `cairn_check_demographic_field` | `demographic assertion requires a non-empty authored twin (§4.5)` |
| `identity.link.asserted`, `identity.unlink.asserted` | `cairn_check_link_assertion` | `identity linkage assertion requires a non-empty authored twin (§5.7)` |
| `identity.dispute.asserted`, `identity.dispute.resolved` | `cairn_check_dispute_assertion` | `identity dispute assertion requires a non-empty authored twin (§5.7)` |
| `identity.pending.asserted`, `identity.identify.asserted` | `cairn_check_identity_state_assertion` | `identity-state assertion requires a non-empty authored twin (§5.7)` |
| `identity.repudiate.asserted` | `cairn_check_repudiation_assertion` | `identity repudiation assertion requires a non-empty authored twin (§5.7)` |
| `clinical.medication.asserted`, `clinical.medication-cessation.asserted` | `cairn_check_medication_assertion` | `medication assertion requires a non-empty authored twin (§3.13/§3.15)` |
| `clinical.medication-dose-change.asserted`, `clinical.medication-dose-correction.asserted` | `cairn_check_medication_dose` | `medication dose assertion requires a non-empty authored twin (§3.13/§3.15)` |
| `clinical.medication-reconciliation.asserted`, `clinical.medication-separation.asserted` | `cairn_check_medication_reconciliation` | `medication reconciliation requires a non-empty authored twin (§3.13/§3.15)` |

Types with **no** row (walking-skeleton `patient.created`/`note.added`/… and `identity.evidence.asserted`)
keep today's behaviour: no structural check, honest skeleton twin.

### 5. Testing (TDD, behaviour-preservation first)

- **Strongest signal — regression:** every existing suite stays green (`demographics*`, `identity_*`,
  `medication*`, `twin_globalise`, `apply_remote_event`, `suppression_owner_gate`, …). They already
  exercise all 9 per-type floors end-to-end; green ⇒ the refactor is inert.
- **New `crates/cairn-node/tests/twin_registry.rs` (DB-gated):**
  - per-type dispatch fires the correct check — a malformed body for type *T* raises *T*'s specific
    error (and a valid one passes); a malformed body does **not** raise for an *unregistered* type;
  - twin-absent on a registered type raises its exact `twin_required_msg`;
  - skeleton fallback: an unregistered type with absent twin returns a skeleton, does not raise;
  - **fail-closed:** inserting a registry row whose `check_fn` does not exist (or has the wrong
    signature) is refused by the trigger at insert time.
- **New `crates/cairn-node/tests/twin_dispatch_single_source.rs` (no-DB source guard, mirrors
  `name_winner_order_drift.rs`):** scans `db/*.sql`, asserts `CREATE OR REPLACE FUNCTION cairn_event_twin`
  appears in **exactly one** file (db/005). Directly guards #173's invariant against re-introduction of
  the copy pattern, in every `cargo test`/CI run, no cluster needed.
- **SQL mirror** in `db/tests/` for the dispatch + fail-closed cases (co-located with the floor).

### 6. Invariants established (bind future slices)

1. `cairn_event_twin` is declared **exactly once** (db/005). A new event type registers by INSERTing one
   `cairn_event_twin_check` row in its own migration and **never** touches the dispatcher.
2. Every per-type structural check fn is `(p_type text, b jsonb) RETURNS void`.
3. A missing / mis-signed check fn fails **closed at load time** (registry trigger), never silently
   drops a floor.

These are recorded in **ADR-0048** (short; refines ADR-0039/ADR-0022, spec home §9.6/§3.13) and
enforced by the single-source guard test.

## Documentation / housekeeping

- **ADR-0048** — new, short: "the per-type twin/floor check is a registry row, not a copied dispatch
  branch; the dispatcher is declared once; check fns share one signature." Add to the ADR index
  (`spec/decisions/`) and the mkdocs ADR nav; bump the spec version string in `index.md` per project
  convention (no spec *prose* change — this is below the spec line).
- **HANDOVER.md / ROADMAP.md** updated at session end.
- `db/031`'s copy-hazard warning comment is removed (the hazard is designed out).

## Honest limits / residuals

- Introduces the first dynamic SQL (`EXECUTE`) in the DB floor — accepted, bounded as above.
- The validation trigger covers registration, not later function mutation (runtime fail-closed backstop).
- `event_type_class` is **not** merged in this PR (deliberate scope guard); the two per-type registries
  coexist, written side-by-side per slice — convergence is a future slice.
- Mixed check-fn signatures are resolved to one; no other floor logic changes.
