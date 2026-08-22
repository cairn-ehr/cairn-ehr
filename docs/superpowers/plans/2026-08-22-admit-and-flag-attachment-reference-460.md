# Admit and flag — a malformed attachment reference must not sink a replicated clinical event

**Issue:** [#460](https://github.com/cairn-ehr/cairn-ehr/issues/460). Follow-on from #370 (PR #459),
which turned nine freeze paths into legible P0001 refusals at **both** doors and answered #370's
granularity question the wrong way. PR #459's corrected headers already say so; this plan builds the
correction.

**Date:** 2026-08-22 · **SCHEMA:** 49 → 50 · **New migration:** `db/050_attachment_reference_flag.sql`

## Global Constraints

- Paper-parity: not clinical-surface — this changes what an in-DB sync door does with an already-signed
  peer event; no human act is added, removed or reordered at any clinical surface. The change is
  strictly *availability-restoring*: content that a node previously could not see becomes visible.
- TDD: failing test first, then the code. The safety-critical surface (§9) is in-DB SQL.
- AGPL-3.0; no new dependencies.
- Additive-only schema evolution (principle 11 / ADR-0012): a new table and a new function, no
  alteration of an existing signature.

## The decision this implements

**Refuse at `submit_event` (db/005). Admit-and-flag at `apply_remote_event` (db/020).**

The asymmetry is the design, and it is the strict/lenient split the floor already uses (#345's
registration precedence is strict-door-only; the shred target-existence requirement likewise):

- At **submit** the event is not yet a fact of the world, and this node is the only one that can stop a
  permanently-defective event entering an append-only, replicating record. Refuse.
- At **apply** the event is already a fact. Refusing does not un-mint it — it only blinds this node to
  what its peers can read, which forks the event set. Slice 66 settled the same shape one level up:
  *withhold the key, never the bytes*. Here: **withhold the reference, never the event.**

The load-bearing correction: `cairn-sync` re-offers **the same bytes** every cycle and the malformed
field sits **inside the signature**, so the author can never repair it. Release requires this node's
floor to change. Deterministic is why the pen is **permanent**, not why it is safe.

## Design

### 1. A LEDGER, not a view

Slice 68's rule: *flag what cannot self-heal, view what can.* This cannot self-heal — the bytes are
signed and immutable — so it needs a durable row. Modelled on `t_effective_ceiling_flag` (db/040),
which is the closest sibling: a cross-type **door-side write**, not an ADR-0057 projection.

```
attachment_reference_flag(
    flag_id           IDENTITY PK,
    event_id          UUID NOT NULL,     -- from the body; the substitution guard makes it 1:1 with content
    attachment_index  INT  NOT NULL,     -- NAME, never count: which attachment
    rendition_index   INT  NOT NULL,     -- and which rendition within it
    reason            TEXT NOT NULL,     -- the accessor's own refusal text, verbatim
    flagged_at        TIMESTAMPTZ DEFAULT clock_timestamp()
)
UNIQUE (event_id, attachment_index, rendition_index)   -- set-union re-delivery is idempotent
GRANT SELECT TO cairn_agent
```

Survives `cairn_reproject` untouched, for db/040's reason: rebuild replays through the dispatch, never
the doors, and the inputs are immutable.

### 2. `event_deferred` is the WRONG home — stated so it is not attempted

An ADR-0056-deferred event **projects nothing and confers nothing**. Here the event is fully
interpreted and must project normally; only one rendition reference is unlearnable. Reusing
`event_deferred` would suppress the clinical content — nearly as harmful as refusing, which is the
defect being fixed.

### 3. Two entry points, one source of truth

`cairn_learn_attachment_refs(b jsonb)` keeps its **one-argument signature** — PR #459's header explains
why (a second parameter creates an OVERLOAD rather than a replacement, leaving the old unvalidated
function resident in every database that already loaded the file). So the lenient path is a **new,
differently-named** function rather than a new signature.

To avoid the strict and lenient paths drifting into two definitions of "malformed", the lenient one
does not re-implement the checks. It runs the **same accessors** and catches their refusal:

```sql
BEGIN
    PERFORM blob_note_reference(cairn_rendition_address(r, v_door), …);
EXCEPTION WHEN raise_exception THEN          -- P0001 only: our own accessors
    PERFORM cairn_record_attachment_reference_flag(v_event_id, i, j, SQLERRM);
END;
```

`WHEN raise_exception` is narrow and named — **not** `WHEN OTHERS`, which would relabel an unrelated
internal error as bad caller input and which additionally does not catch a statement timeout (57014
`query_canceled` is one of the two codes it excludes — the Slice 68 lesson). The only P0001 source
inside the block is our accessors; `blob_note_reference` is plain SQL and cannot raise it. Cost: one
subtransaction per rendition, negligible at N of 1–3.

### 4. A defect on one rendition never invalidates another

This is ADR-0060's actual principle, applied where it does fit. An event with three renditions, one
malformed, must learn the **two good ones** and flag the one. A test drives exactly that.

### 5. A read surface, because a mechanism nobody can look at is Slice 69's finding

`cairn_patient_attachment_flags(uuid)` — a chart-scoped `SECURITY DEFINER` read granted to **both**
group roles, mirroring db/043's `cairn_patient_deferred_sensitivity` (whose first draft granted to
`cairn_agent` alone and called that "the runtime role", which it is not — #425). Names each flagged
event and reason; never a count.

## Tasks

1. **RED** — extend `crates/cairn-node/tests/attachment_reference_shape.rs`:
   - the apply door **admits** an event whose rendition digest is malformed, and the event is in
     `event_log`;
   - a flag row exists naming the attachment/rendition index and carrying the accessor's reason;
   - a second apply of the same bytes adds no second row (set-union idempotence);
   - an event with one good and one malformed rendition learns the good reference **and** flags the bad;
   - the submit door still refuses the same body with P0001 (unchanged, now load-bearing as the
     asymmetry's other half);
   - the flagged event still **projects** — it is not deferred.
2. **GREEN** — `db/050_attachment_reference_flag.sql`: the table, the recorder, the lenient learner,
   the chart-scoped read. Register in `cairn-node`'s `SCHEMA` (db.rs) **and** `cairn-sync`'s
   `SCHEMA` subset — the apply door needs it, and db/027's helpers must resolve there.
3. Bump `SCHEMA_GENERATION` 49 → 50 (`crates/cairn-event/src/schema_generation.rs`); the guard test
   pins that the migration list carries the repo's newest file.
4. Point `db/020` at the lenient learner. `db/005` unchanged.
5. Mutation-test every new guard: swap `raise_exception` for `OTHERS`; drop the unique index; make the
   lenient path swallow without recording; make it record without learning the good sibling.
6. SQL mirror `db/tests/050_*.sql` — db/050 is a floor file and the mirrors are the in-DB regression
   layer.
7. Correct db/027's and the test module's headers again: the interim state is over; describe what each
   door now does.
8. Whether this warrants an ADR is open — it changes what a door does with clinical content, which
   argues yes. Raise it rather than decide silently.

## Re-verification

`cargo test --workspace` with all three connection strings · `scripts/run-db-sql-tests.sh` ·
`cargo fmt --check` · `cargo clippy --workspace --all-targets -D warnings` · `mkdocs build --strict`.

## What changed during implementation, and why it is recorded here

**1. The framing was wrong, and correcting it removed a task.** Task 8 asked whether this warrants an
ADR. It does not: **ADR-0063 already decides it**, in a table, for the §5.9 `safety` field — *malformed
field: local door REFUSE, remote door ADMIT* — and states the rule generally (*an envelope-level field is
constrained where it is minted and read permissively where it arrives* — the ADR says **graded** field,
and #460 extends the rule past graded ones on the blast-radius argument, not on the sentence), rejecting apply-door refusal on
**blast radius** in words that never mention `safety`. So #460 is not a new decision; it is an existing
rule applied where it already bound, and #370's fix had contradicted an ADR nobody read. What IS missing
is a *findable* name for the rule — three implementations, filed under a fourth thing's title — which is
[#461](https://github.com/cairn-ehr/cairn-ehr/issues/461), documentation-only.

**2. The recorder could raise, and its own header said it could not.** The header claimed db/029's
*structurally non-gating* property — "plain SQL, one INSERT, ON CONFLICT DO NOTHING, it cannot raise" —
which was written before the foreign key to `event_log` was added and survived it unchanged. It raised
**23503** for an event not yet inserted, **inside the handler catching the refusal**, so it would have
propagated and refused the clinical event: the exact harm this file exists to prevent, reintroduced by
the mechanism meant to prevent it. `db/tests/050` caught it on its first run. Fixed with the
`INSERT … SELECT … WHERE EXISTS` guard db/029 actually uses and the header had only cited.

**3. `ON DELETE CASCADE` is unreachable, and the test proved it by failing.** The plan justified the FK
with "a flag must never outlive its event". `event_log` is append-only and db/001's trigger refuses
DELETE outright (principle 1), so the cascade can never fire. The FK still earns its place — it stops a
flag naming an event this node does not hold — but the cascade is now described as what it is. The same
is true of `event_deferred`'s, which this copied.

**4. Task 6's mirror is narrower than planned, on purpose.** It drives the two learners and the ledger
directly rather than re-deriving signed events in SQL: the Rust suite already drives the real doors end
to end, and routing the ledger's own mechanics through `submit_event` would drag in signing, enrolment
and #345's registration precedence — none of which the ledger is being tested for, all of which could
fail it for unrelated reasons.

**5. The shipped table is NOT the one specified above, and the difference is the whole
not-attributable case.** The schema block specifies `attachment_index INT NOT NULL`,
`rendition_index INT NOT NULL` and a plain `UNIQUE`. What shipped makes both indices **nullable** and
the index **`NULLS NOT DISTINCT`** — because there are three legitimate shapes, not one: `(i, j)` for a
rendition, `(i, NULL)` for an attachment whose `renditions` was not a list, and `(NULL, NULL)` for an
`attachments` value that was not a list. `NOT NULL` cannot express the last two, and the default
`NULLS DISTINCT` would make every re-offer of the last one a fresh row, forever. Recorded here because
the block above is exactly the artefact a future reader would use to "simplify" it back.

**6. The review pass rewrote three of this plan's own claims.** Six agents over the finished branch,
every claim re-verified against PG 18.1 before acting:

- **Design §3's "it runs the same accessors" understated what was needed, and the code understated it
  further.** Sharing accessors does not stop the two doors drifting on *traversal* — the list coercion
  and the inline skip. Four files asserted a shared traversal while the strict learner still ran its own
  duplicated loop. Now genuinely shared (`cairn_by_reference_renditions`, db/027, iterated by both) and
  pinned by `db/tests/050` §9 reading `pg_proc`, so the claim cannot outlive the code a second time.
- **Design §4's principle was inverted by the implementation at the next granularity up.** "A defect on
  one rendition never invalidates another" held for accessor faults and failed completely for list-shape
  ones: a PL/pgSQL SRF materialises before its first row, so raising inside it discarded every
  well-formed reference on every *other* attachment. Fault rows fixed it. **No Rust test could have
  caught this** — `EventBody.attachments` is a `Vec` and `sign()` takes a typed body, so a non-list is
  unrepresentable; the list-shape class is the SQL mirror's alone, now stated at the top of that file.
- **§3's "`blob_note_reference` is plain SQL and cannot raise it" was true for the wrong reason.**
  It INSERTs into `blob_store`, which carries db/026's `cairn_blob_present_guard` — a bare `RAISE`,
  i.e. P0001 — held off only by `FOR EACH ROW WHEN (NEW.present)` **in another file**. Now pinned there.
- **A regression this plan did not anticipate:** admitting a non-array `attachments` *stores* it, and
  `read_photo_refs` walks that column with `jsonb_array_elements` (22023). The refusal was relocated,
  not removed — into the §5.3/§5.8 search-before-create funnel. Fixed at both doors with a total
  coercion plus a CHECK.
- Task 5's mutation list was run and found the mirror weaker than it read: the lenient learner replaced
  by `RETURN;` left every section green, because §1 drove it against an `event_id` absent from
  `event_log`.

**Lesson worth carrying past this slice:** *a comment written before a constraint does not update itself
when the constraint lands.* Every defect above is that shape — prose asserting a safety property rather
than code implementing one — and the branch was fully green through all of them. The corollary the
review pass adds: **a claim about two things staying in step needs a guard that reads both**, or it
decays into decoration the moment one of them moves.
