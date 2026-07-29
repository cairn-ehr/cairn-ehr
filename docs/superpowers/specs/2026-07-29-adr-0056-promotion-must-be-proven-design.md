# Design — promotion must be proven: closing three defects in the ADR-0056 floor

**Date:** 2026-07-29 · **Branch:** `feat/adr-0056-admit-uninterpreted-floor-265-266` (PR #302,
pre-merge) · **Amends:**
[the admit-uninterpreted floor design](2026-07-29-adr-0056-admit-uninterpreted-floor-design.md) ·
**Decides:** nothing new — this implements
[ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) decision 4 more
completely. No ADR change, no `SCHEMA_GENERATION` change.

## 1. What is being fixed

Three defects found reviewing PR #302, all confirmed empirically against `cairn_test` before being
written down.

**F1 — promotion never re-runs the per-type structural floor, and the reprojection that follows
permanently wedges `connect_and_load_schema`.** `cairn_readjudicate_deferred` (db/043) re-runs three
gates: attestation, overlay-target-exists, ADR-0043 owner. It does not re-run db/020 step 8 —
`cairn_event_twin`'s dispatch to the type's `check_fn` and `twin_required_msg` — which was skipped at
admission for the same reason the other three were, because it too keys on the type having a registry
row. So that check is genuinely **waived**, not deferred, contradicting db/043's own header.

The consequence is far worse than a bypassed check. `DELETE FROM event_deferred` commits, the event
becomes replay-eligible, the loader reprojects it, and the apply fn raises on a payload the floor
would have refused. The marker is gone and `event_log` is append-only, so nothing can undo it.
Measured, with a peer-signed `clinical.medication.asserted` carrying `{"nonsense": true}` and no
authored twin — an event **both doors would refuse** if the type were known:

```
promoted: 1
markers left after promotion: 0
attempt 1: reproject FAILED -> null value in column "medication_id" of relation
                               "medication_statement" violates not-null constraint
attempt 2: reproject FAILED -> (identical)
```

On the realistic path — classification arrives *with* a migration, so the generation changes too and
the loader takes the full-heal branch — the stamp never advances and every subsequent connect repeats
the same failure:

```
connect attempt 1: FAILED -> post-upgrade heal replay: db error
connect attempt 2: FAILED -> post-upgrade heal replay: db error
connect attempt 3: FAILED -> post-upgrade heal replay: db error
recorded generation after the failures: 42   (never advances)
```

The node never connects again. This is precisely the one-bad-event-wedges-the-node mode ADR-0056
exists to remove, reintroduced one layer up — and `cairn-node deferred`, the only surface that could
diagnose it, calls `connect_and_load_schema` itself, so it is unreachable in exactly that state.

**F2 — on the additive path the carried token is never verified, and promotion then re-opens the
ADR-0043 over-permission PR #302's headline fix closes.** db/020's deferred arm stores
`p_attestation`/`p_attester_key` unconditionally. db/043 verifies them only
`IF r.mode = 'suppressing' OR v_bears`. For `('additive', FALSE)` with no `responsibility`
contributor — the common upgrade case — the token is never checked, yet the marker is deleted. Since
`cairn_suppression_author_ok` keys its exclusion on the marker's *presence*, the unverified key is
then unioned straight into the target's human-author set. Measured, with a **garbage 64-byte blob**
as the attestation and an unrelated enrolled human key:

```
promoted rows: 1
markers left: 0
attester_key still stored: true
MALLORY MAY SUPPRESS ANOTHER HUMAN'S EVENT: true
```

Mallory never signed, authored, or attested anything. The gate is shared by both doors, so she can
author a local `visibility.suppress` against another clinician's event and pass. The mirror harm is
equal: a forged token on an agent-authored advisory makes `human_authors = {forged key}`, locking it
so no clinician but the forger may dismiss it — the clinician-overrides-the-machine path, closed by
an unauthenticated peer.

**F3 — cairn-sync ships db/043 but never calls it.** Both new comments argue the file must land in
cairn-sync's list because cairn-sync runs the door that *writes* markers. The reasoning is right; only
the function ships. `cairn_readjudicate_deferred` has exactly one production caller, in cairn-node. On
a sync-only database — the phone-tier carrier this ADR exists for — markers accumulate and nothing
promotes them.

## 2. The organizing idea

Both safety defects are one mistake in different clothes: **a state was inferred from a proxy with the
wrong lifetime.** `event_deferred` was made to answer two questions —

1. *has this event been adjudicated?* — and
2. *is this event's stored attestation vouched?*

— and it has the right lifetime only for the first. Promotion deletes it, and both the token's trust
and the projection's viability silently inherit that deletion.

So: give the second question its own marker, and stop promoting on assumption. The invariant to reach
is **a promoted event is one that has already projected cleanly** — not one we expect to.

## 3. `event_attestation_unvouched` — naming the state

New in db/001, beside `event_deferred`:

```sql
CREATE TABLE IF NOT EXISTS event_attestation_unvouched (
    event_id UUID PRIMARY KEY REFERENCES event_log(event_id) ON DELETE CASCADE
);
```

No other column: its presence *is* the fact — *this row's `attester_key` was stored without being
verified*. Written by db/020's deferred arm in the same breath as it stores the token, and **only when
a token is actually present** (a deferred event that carried none has nothing unvouched about it);
deleted by db/043 only inside gate 1's success path, where a token has actually been verified.

It lives in db/001, not db/043, for the same reason `event_deferred` does: db/005's predicates are
`LANGUAGE sql`, whose bodies resolve table names at CREATE time.

**Why not the alternatives.** Keeping the carried token in `event_deferred` and moving it into
`event_log` on promotion would be cleanest — the unverified value would never touch the trusted column
— but db/001's append-only trigger refuses UPDATE on `event_log` outright, and relaxing the schema's
most load-bearing invariant to buy this is a bad trade. Verifying every carried token unconditionally
at promotion also works and needs no new table, but it hands a hostile peer a cheap
denial-of-power attack: attach a junk token to an unknown-type event and it is permanently powerless
on every node lagging the code plane, with no path out. Note the asymmetry that makes that wrong — had
the type been *known* and additive, the door would have **dropped** the junk token and admitted the
event at full power.

**Subsumption, and why both markers stay.** The unvouched marker strictly subsumes PR #302's
`event_deferred` exclusion inside `cairn_suppression_author_ok`, so that predicate is **replaced**,
not stacked on top of:

| target's state | `event_deferred` | unvouched | arm counts? |
|---|---|---|---|
| deferred, never adjudicated | present | present | no — correct |
| deferred, re-adjudication failed | present | present | no — correct |
| promoted, gate 1 verified the token | absent | cleared | **yes** — correct |
| promoted, no gate demanded a token | absent | **present** | no — the hole, closed |

The two tables remain distinct because they answer distinct questions; the failed-re-adjudication row
is the case that proves neither subsumes the other in general.

### 3.1 The readers

Three readers of `event_log.attester_key` exclude on the new marker:

- **db/005 `cairn_suppression_author_ok`** — the one reachable today, and the one PR #302 already
  found. Its predicate changes from *"the target is not deferred"* to *"the target's token is
  vouched"*, which is the question it actually meant to ask.
- **db/018 `patient_link_apply`** — newly reachable once gate 4 (below) projects at promotion. It
  reads `attester_key IS NOT NULL` as "human-attested" and uses that to pass a link that
  `cairn_has_hard_veto` would otherwise refuse. Without the exclusion, F1's fix would open a second
  hole while closing the first.
- **db/034 `medication_attestation_apply`** — same class. Its header currently asserts
  `attester_key` is a verified vouch guaranteed non-NULL by the db/005 gate; the exclusion restores
  that, and the header is corrected to name the new precondition.

A new reader of these columns owes the same choice, and the marker's name is what makes that legible
in a way *"is it deferred?"* never was.

## 4. Gate 0 — the per-type structural floor

First gate in db/043's per-row subtransaction, re-running what db/020 step 8 skipped:

```sql
v_clear := cairn_clear_payload(r.el);        -- the existing ONE seam (db/005)
IF v_clear IS NOT NULL THEN                  -- no custody → skip, exactly as db/020 does
    PERFORM cairn_event_twin(r.event_type, jsonb_set(b, '{payload}', v_clear));
END IF;
```

`cairn_clear_payload` is reused rather than reimplementing db/020's sealed/unsealed branching, so the
two paths cannot drift on what a readable body is. A sealed event with no custody skips the check —
identical to the door's behaviour, and gate 4 still proves it can project.

The pass must raise `cairn.remote_apply` for its duration. These are peer-arrived events, and db/041's
coding check reads that marker; without it we would reintroduce the refusal-of-a-verifiable-peer-event
that db/020's own step-8 comment warns about. `cairn_reproject` already does the same for its run, so
this is the established pattern, not a new one.

Beyond correctness, gate 0 buys **legibility**: `adjudication_error` reads *"medication assertion
requires a non-empty authored twin (§3.13/§3.3)"* instead of *"null value in column medication_id …
violates not-null constraint"*. Decision 4's "flagged legibly" is only legible if the flag names a
clinical reason.

## 5. Gate 4 — prove the projection

Last in the same subtransaction, immediately before the marker delete:

```sql
FOR v_fn IN SELECT apply_fn FROM cairn_projection_apply
             WHERE event_type = r.event_type AND heal_safe
             ORDER BY run_order, apply_fn
LOOP
    EXECUTE format('SELECT %I($1)', v_fn) USING r.el;
END LOOP;
DELETE FROM event_deferred WHERE event_id = r.event_id;
```

The loop's row must therefore select `el` as a composite rather than picking out columns.

**Why per-event dispatch is affordable here and not in `cairn_reproject`.** db/039 deliberately uses a
set-based apply — one full-table pass per (type, fn) — because the per-event PL/pgSQL loop it replaced
was measured at ~25% of a 2M-event rebuild's cost at the Pi target (the Bet-B run clocked a full
rebuild at 49 min before that change). The argument does not transfer: the deferred set is empty on a
healthy node and tiny by construction otherwise. This is the one place per-event error isolation is
both necessary and free.

`heal_safe` mirrors heal mode. A type carrying non-heal-safe fns has those reported the way
`reproject_log.skipped_fns` already reports them, rather than being silently unprojected.

**What this buys.** A promoted event is, by construction, one that has already projected cleanly. The
loader's heal can no longer meet an event that wedges it — not for today's payload, and not for a
stricter apply fn written in 2027. Gate 0 closes the reachable case; gate 4 closes the class.

## 6. The loader, and cairn-sync

**`db.rs` gets simpler.** Gate 4 already projected each promoted event, so the `else`-branch targeted
reproject is redundant and is removed, along with its `heal_safe`/prefix-LIKE reasoning. What remains
is the call plus an operator log line when it promotes anything. The generation-change full heal is
untouched, and is now safe by construction.

**cairn-sync gets the call (F3).** Same position as cairn-node's: after the migration loop, before the
gated heal, inside `SCHEMA_LOAD_LOCK`. This had to follow F1 — adding it earlier would have spread the
wedge to the sync daemon. On a subset database gate 4 runs only the projections that node registers,
which is correct: it projects what it knows how to project.

`reproject_log.source`'s column comment — which PR #302 left enumerating `'loader' | 'cli' | 'test' |
'manual'` while introducing a fifth value — needs no update after all: with the targeted reproject
removed, `'readjudicate'` is never written.

## 7. Documentation

The predecessor design doc carries the two claims that produced these defects. Both are corrected in
place, with the reasoning failure recorded rather than quietly overwritten:

- §4 line 80, *"The twin needs no work."* True at **admission** (no registry row ⇒ `check_fn` and
  `twin_required_msg` are NULL ⇒ skeleton, never raises). The question was asked once and never
  re-asked for **promotion**, when the registry row exists. The generalisable rule: every claim of the
  form *"X needs no work because the registry is empty"* has a second lifetime where the registry is
  no longer empty.
- §4.2 line 124, *"Promotion deletes the marker, at which point the now-verified token counts
  normally."* False whenever no gate demanded the token — which is the common case.

No ADR change: ADR-0056 decision 4 says re-adjudicate before power, and this implements it more
completely rather than revising it. `SCHEMA_GENERATION` stays 43 — no new `db/*.sql` file, and neither
db/001 marker table has ever shipped. HANDOVER and ROADMAP Slice 58 are updated in the same branch.

## 8. Testing

TDD throughout: every item below is written failing first, and the failure verified, before the fix.
The four review probes become permanent pins.

| Test | Pins |
|---|---|
| `promotion_refuses_an_event_its_type_floor_rejects` | gate 0 — a junk payload / absent twin stays deferred with the *clinical* message, not a constraint violation |
| `a_promotion_that_cannot_project_never_promotes` | gate 4 at connect level — three consecutive `connect_and_load_schema` succeed and `node_schema.version` advances |
| `a_carried_token_never_widens_the_owner_gate_after_promotion` | F2's direct regression — garbage token + additive type + promotion, gate still refuses |
| `an_unvouched_token_is_not_a_link_attestation` | the db/018 reader — a hard-vetoed link stays refused |
| `a_verified_token_clears_the_marker_and_counts` | the other direction, so the fix is not vacuous |
| cairn-sync loader promotes | F3, mirroring `connect_promotes_and_reprojects_a_deferred_event` |

`db/tests/043` gains the new table's shape and keeps the owner-only privilege assertion.
`a_travelling_token_survives_defer_then_promote` keeps its behaviour but loses its misleading
assertion message — it claims the token "must now VERIFY" on a path where nothing verifies it.

Gate: full `cargo test --workspace` (not per-crate — that is what missed the cross-crate arity gap
before, and `| tail` masks cargo's exit code), `scripts/run-db-sql-tests.sh`, `cargo clippy
--workspace --all-targets -- -D warnings`, `cargo fmt --check`, `mkdocs build`.

## 9. Scope boundaries

Unchanged from the predecessor slice: the node/actor plane still fail-closes on an unmappable type
(#301), and ADR-0056 decision 5 (#267/#268/#269/#270) is untouched.

Deliberately **not** attempted here: making `cairn_reproject` itself resilient to a per-event apply
failure. It would fix a wider class — the ADR-0058 hazard where a violated constraint wedges one event
forever — but it requires reverting db/039's set-based apply to a per-event loop, at the measured 4×
cost on the Pi target, and a heal that silently skips a failing apply is the "silently stale
projection" failure the loader's own ordering comment exists to prevent. Gate 4 gets the safety
without either cost, because it runs only over the deferred set.

## 10. Paper-parity (§1.2)

Paper-parity: not clinical-surface — these are floor corrections beneath the application layer. No
human act is added, removed, or reordered at any layer; the changes affect only which admitted events
gain power and when. The predecessor slice's forced-rationale escape applies unchanged.
