# Design — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power

**Date:** 2026-07-29 · **Issues:** [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265),
[#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) · **Decides:**
[ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) decisions 1 and 4
· **Scope:** the clinical remote door + the reclassification path. The residual refusal contract
(decision 5 — #267/#268/#269/#270) is a separate slice.

## 1. What is being fixed

ADR-0056 was ratified 2026-07-20 with no code. Its own "Consequences" section states the honest
current limits: *"The remote door still fail-closes today; door refusals are not yet penned; a frozen
clinical watermark still exits success; the node plane still skips-and-advances."* This slice closes
the first of those four, plus the reclassification path that must exist before it is safe.

The live defect: `apply_remote_event` (db/020) raises on an `event_type` absent from
`event_type_class`, so the event is **never stored at all**. A phone-tier node carrying a chart
between two upgraded facilities — the §6.1 sneakernet path, the case Cairn exists for — acquires
nothing past the first unknown-type event. `sync.md` §6.5's lossless-forwarding invariant is
therefore false for unknown *types*, and the spec was right while the code was wrong.

The ground was prepared by ADR-0057. `cairn_replay_eligible` (db/005) already exists as a constantly-
TRUE predicate, and its own comment names this slice as its consumer: *"#265's explicit deferred
marker hooks in HERE and only here, so a manual mid-upgrade reproject can never grant power to an
unadjudicated deferred event."* db/039's header likewise names "#266's reclassification scans" as a
user of the `event_log_type_idx`.

## 2. Why re-adjudication is load-bearing, not bookkeeping

Admitting an event uninterpreted necessarily **skips** every floor check derived from its mode or its
target relationship. In db/020 all three sit downstream of the classification lookup:

- the suppressing⇒attestation gate (`v_mode = 'suppressing' OR v_bears`),
- the overlay-target-exists refusal,
- the ADR-0043 cross-author-suppression refusal.

These are *deferred with* the interpretation, not waived by it. If classification arrival only
rebuilt projection rows, a deferred event would gain power having never passed the gate that exists to
bound it. ADR-0056 decision 4 therefore fixes the order — **re-adjudicate first, reproject second** —
so *no unattested suppression* holds at every instant rather than being violated-then-repaired.

## 3. The deferred marker

**A node-local `event_deferred` table.** Node-local derived state, like `reproject_log`,
`node_schema` and `hlc_collision_log`: never signed, never on the wire (principle 12).

Rejected alternatives:

- **A boolean column on `event_log`** — it would have to be UPDATEd on promotion, mutating the
  append-only log.
- **Inferring deferral from the absent `event_type_class` row** — rejected by ADR-0056's own
  corollary: *"the deferred state must be explicit, never inferred from a null classification lookup
  falling through the gates by three-valued logic."* An inferred marker also cannot carry the
  adjudication failure reason, which decision 4 requires to be flagged legibly.

Shape:

| column | meaning |
|---|---|
| `event_id` | PK, FK to `event_log` |
| `event_type` | denormalized: the reclassification scan selects its candidates by joining this against `event_type_class` alone, and the CLI listing reads it without touching `event_log` |
| `admitted_at` | node-local `clock_timestamp()` — operational, never a clinical time |
| `adjudication_error` | NULL until a re-adjudication attempt fails; then the verbatim refusal |
| `last_attempt_at` | NULL until first attempted |

**The row's presence is the invariant.** A row exists ⟺ the event is powerless and its
classification-gated checks have not been passed. Promotion **deletes** the row.

## 4. #265 — the door admits

`apply_remote_event` step 3 stops raising. When the `event_type_class` lookup yields NULL it sets
`v_deferred := true` and skips steps 4 and 5. Everything envelope-decidable still refuses exactly as
today, per ADR-0056 decision 3: size ceiling, signature, `cairn_check_contributors` (never-lawful
contributor shapes), signer enrollment/revocation, `t_effective` parse. The grade-gated ceiling
(ADR-0058) already flags-never-rejects and is untouched. Born-sealed scope is strict-door-only by
deliberate design (db/005's own comment gives the reason — *"a refusal there would freeze the seq
watermark on a verifiable event"* — which is this ADR's argument, already applied), so it is not a
concern here.

The twin needs no work: `cairn_event_twin` (db/005:213) finds no `cairn_event_twin_check` row for an
unregistered type, so `v_fn` and `v_msg` are both NULL and it returns `cairn_twin_skeleton`. It never
raises. Verified before this design was written, because the whole mechanism dead-ends otherwise.

### 4.1 The travelling-attestation trap

A suppressing event's attestation token **travels with it on the sync wire** and is stored into
`event_log.attestation` / `.attester_key` today only on the path where the gate passes. Skip the gate
naively and the token is dropped — and then re-adjudication has nothing to verify, so the event is
powerless **forever**, which silently converts admit-and-defer into a slower fail-closed.

So the deferred arm stores the travelling token unconditionally, and the resulting state is named:

> **An attestation on a row carrying an `event_deferred` marker is *carried*, not *vouched*.**

Implementation owes an audit of every reader of `event_log.attestation` / `.attester_key` against
that invariant; any reader treating presence as proof must exclude deferred rows. The audit result is
recorded in the plan, not assumed here.

## 5. #266 — reclassification re-adjudicates, then reprojects

### 5.1 The replay gate

`cairn_replay_eligible(e event_log)` — today `SELECT TRUE` — becomes:

```sql
SELECT NOT EXISTS (SELECT 1 FROM event_deferred d WHERE d.event_id = e.event_id)
```

`cairn_reproject` routes every candidate through it, so no reprojection path — loader, CLI, or a
hand-run mid-upgrade heal — can grant power to an unadjudicated event. This is the only place the
marker gates replay, exactly as ADR-0057 designed.

### 5.2 The pass

`cairn_readjudicate_deferred()` scans `event_deferred` joined to `event_log` for rows whose
`event_type` now has an `event_type_class` row, and for each re-runs the three deferred gates:

1. **Attestation** — if `mode = 'suppressing'` or any contributor bears responsibility: token
   present, `cairn_attestation_ok` against the stored `content_address`, attester an enrolled human,
   and `cairn_responsibility_bound`.
2. **Overlay target exists** — if `targets_other_author`: `cairn_suppression_target_id` must resolve
   to a held event.
3. **ADR-0043 cross-author suppression** — if both suppressing and targeting another author.

The envelope is recovered with `cairn_body(e.signed_bytes)` — the exact bytes the door saw, so the
predicates receive the identical input they would have at admission. No reconstruction from columns,
which would drift from the door.

- **Pass → DELETE the marker.** The event is now fully in effect and replay-eligible.
- **Fail → keep the marker, record `adjudication_error` + `last_attempt_at`.** Powerless and flagged
  legibly; never silently promoted.

**Per-row exception capture, never a raise.** The pass runs inside the loader; a raise would abort
`connect_and_load_schema` and wedge the node on one bad event — the same failure mode this ADR
exists to remove.

### 5.3 When the pass runs

**Every connect, not only on a generation change.** `event_deferred` is empty on a healthy node, so
the pass costs one indexed probe. The reason it cannot be generation-gated: adjudication can fail for
a reason that later resolves without any code-plane update. The sharp case is `overlay targets
unknown event` — a node takes the code update while the deferred overlay's target is still in flight
from another peer; the target lands minutes later, and under a generation-only trigger that event
stays powerless until the *next* code update, potentially months.

Ordering inside `connect_and_load_schema`, which is load-bearing:

```
load every migration
  → cairn_readjudicate_deferred()        -- needs the new classifications; must precede reproject
  → reproject                            -- generation change: cairn_reproject('', false, 'loader')
                                         -- else, per promoted type: cairn_reproject(<type>, false, 'readjudicate')
  → stamp node_schema
```

The stamp stays last for the reason db/db.rs already documents: stamp-then-heal would let a heal
failure leave the generation advanced, so the next connect skips the heal and the projections stay
*silently* stale.

The targeted reproject uses **heal mode**, not rebuild, so db/039's narrow-prefix rebuild refusal
(the slice-6b lesson: `medication_coding` has three writers) does not apply.

### 5.4 The registration guard

`cairn_check_projection_registry_fn` gains one check: the registering `event_type` must exist in
`event_type_class`.

This closes a hole the marker alone leaves open. The `event_deferred` row is written *after* the
`event_log` INSERT, but the AFTER-INSERT projection dispatcher fires *during* it — so a type that was
projection-registered without being classified would project a deferred event at admission, granting
power the marker was meant to gate. The guard makes that state unreachable at migration time rather
than defending against it at runtime. All 17 currently-registered types already satisfy it (verified
by comparing every `cairn_projection_apply` registration against every `event_type_class` row).

## 6. Legibility

`cairn-node deferred` — a read-only subcommand listing deferred rows with type, admitted-at, and the
current `adjudication_error`, mirroring the existing `cairn-sync quarantine` surface. Decision 4's
"flagged legibly" becomes something an operator can see without psql.

## 7. Testing

TDD throughout — failing test first. The set:

- an unknown-type event is **admitted** by the remote door, yields **no projection rows**, and renders
  via the skeleton twin;
- the **strict** door still refuses an unclassifiable type (decision 2 — a regression pin, since this
  slice's whole risk is over-relaxing);
- `cairn_replay_eligible` blocks a mid-upgrade `cairn_reproject` from touching a deferred event;
- classification arrival **promotes** a passing deferred event (marker deleted, projection rows
  appear);
- a deferred event that **fails** re-adjudication stays powerless, keeps its marker, and carries a
  legible `adjudication_error`;
- the travelling attestation token survives defer→promote (the §4.1 trap, pinned so a future
  refactor cannot silently reintroduce it);
- the registration guard refuses a `cairn_projection_apply` row for an unclassified type.

## 8. Scope boundaries

- **The node/actor plane is out of scope.** `db/007:331` carries the identical fail-closed
  (`apply_remote_node_event: unknown node event_type % (fail closed)`), and the propagation-barrier
  argument transfers: a carrier node that refuses a future `node.rekeyed` never stores it, so it
  cannot forward it to a third node that does have the code. It is not a symmetric fix — `node_event`
  is type-shaped, not generic: `v_op` maps four hardcoded types, each with a bespoke INSERT and
  per-type trust logic, so admitting uninterpreted needs a carried-not-interpreted arm plus an audit
  of every trust projection that reads the table. Filed as its own issue (house rule 5), not left
  silent.
- **The residual refusal contract is the next slice** — #267 (pen door refusals verbatim), #268
  (align the node-plane P0001 skip-and-advance), #269 (the missing heal test), #270 (a frozen
  clinical watermark must fail loud). This slice shrinks their blast radius: once unknown types stop
  refusing, the freeze and skip paths are exercised only by genuine refusals.
- **No wire change.** ADR-0010's derived-not-declared rule stands; nothing here is can't-retrofit.

## 9. Paper-parity (§1.2)

Paper-parity: not clinical-surface — this slice changes only which events a node retains and when
their power is granted; it adds no human act at any layer, and exposes no runnable clinical surface.
Its paper counterpart (a referral letter you cannot fully read still stays in the folder, visible and
forwardable) is the *motivation* for the change rather than a workflow it introduces, and the change
strictly increases what the chart holds — so no workflow becomes slower, harder, or impossible.
