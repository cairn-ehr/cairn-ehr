# Suppression is self-only; disagreement is additive — design

**Date:** 2026-07-09 · **Status:** design approved, pre-implementation ·
**Lands as:** ADR-0043 + a floor change to `db/005_submit.sql` and `db/020_apply_remote_event.sql` ·
**Closes:** the last open sub-item of [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99)
(suppression owner-gate).

## Problem

The validated write floor has an owner-gate hole flagged by the 2026-07-02 comprehensive review
(finding A10) and tracked as #99. A suppressing overlay that forecloses on a specific prior event —
`salience.downgrade` / `visibility.suppress` (the only two `suppressing + targets_other_author=TRUE`
types) — is admitted once *some* enrolled human attests it (step 4) and the target event exists
(step 5). Nothing constrains **whose** event is being suppressed. So any enrolled clinician can
downgrade or hide **any** other author's clinical content.

`db/005:162–178` marks this `DEFERRED`, calling real owner/authority semantics "an ADR-level design
question." This document resolves that question.

### #99 is mostly already closed

Three of #99's four sub-items were fixed on 2026-07-02 and are cited in-code:

| Sub-item | Status |
|---|---|
| Recall epoch bug (`events_by_actor_epoch` joined `actor_current`) | ✅ done — `db/006` resolves against `actor_event` history |
| `recall_overlay` FK on `target_event_id` | ✅ done — `db/006:18–31` |
| `actor_current` wall-clock tiebreak | ✅ done — `db/004` orders on `(recorded_at, seq)` |
| **Suppression owner-gate** | ❌ this design |

## The decision

**A suppressing overlay that targets another author's event is refused at the floor. Suppression is
self-only. Disagreement with another author's content is expressed additively — a note/advisory that
references their event — never by touching their event.**

This is the principle-correct answer, not a compromise:

- **Principle 1/2 (append-only; never erase, always overlay).** Reaching into another author's content
  to hide or demote it is the violation. You overlay *your own* view additively.
- **Principle 3 (paper-parity), read correctly.** On paper you cannot un-write another clinician's ink.
  You write your own note: "I disagree with the above; the plan is X." Their entry stays legible beside
  yours. Striking a colleague's entry to *illegibility* is the silent-falsification malfeasance that
  paper-parity **explicitly excludes** — so the earlier "senior strikes through the junior's note"
  framing was wrong, and a permission/notification gate to *enable* cross-author suppression would have
  been machinery in service of a violation.
- **ADR-0010 (conservation of responsibility).** Suppression is always owned by its attester. Self-only
  makes the owner and the authored-content owner the same person — the tightest form of "responsibility
  is conserved."

Self-suppression stays allowed: the digital equivalent of an author drawing a line through their own
erroneous entry and initialling it. The original event stays in the log, legible under the overlay
(append-only, never erase).

### Deliberate divergence from ADR-0010 §2

ADR-0010 §2 classifies *demotion* as additive ("only hiding-to-nothing or auto-deciding is
suppressing"). This design keeps `salience.downgrade` in the **gated (self-only)** set anyway: demoting
another author's content still *de-facto buries* it (ADR-0010 §5's own concern), so cross-author
demotion is refused on the same footing as cross-author hiding. `db/005` already classifies
`salience.downgrade` as `suppressing`, so no reclassification is needed — the ADR records the reasoning.

## Scope carve-outs (stated explicitly in the ADR)

1. **§5.9 sensitivity / visibility-scope sealing** genuinely hides cross-author content (sealing a
   sensitive episode), but through a **separate, specifically-authorized mechanism** (ADR-0006) that
   emits a de-identified, severity-graded safety projection so nothing safety-relevant is lost. That is
   **not** the generic clinician `visibility.suppress` overlay gated here. This ADR governs only the
   generic suppression overlays; §5.9 keeps its own authority path.
2. **`repudiate` (C5, `identity.repudiate.asserted`)** is `suppressing` but `targets_other_author=FALSE`
   — it is value-grained (strikes a known-false *name*, not a target *event*). The gate fires only when
   `targets_other_author AND payload carries target_event_id`, so repudiate is untouched (regression
   guard in the test set).

## Mechanism

Language tier: **in-DB floor** (SQL / PL-pgSQL) — safety-critical, defect could silently corrupt the
record, so it goes in the smallest, most reviewer-legible surface (§9).

### Shared helper

```
cairn_suppression_author_ok(p_target uuid, p_attester_key bytea) RETURNS boolean   -- STABLE
```

Returns TRUE iff the attester (the responsible human of the suppressing event) is the author of the
target event. "Author of the target" is any of:

- `encode(p_attester_key,'hex') = target.signer_key_id` — the attester's key signed the target; **or**
- the attester's resolved `actor_id` (via `actor_current`, `kind='human'`) `= target.actor_id`; **or**
- the attester's `actor_id` appears among `target.contributors[]` with a responsibility role
  (`e ? 'responsibility'`).

**Safe-refuse (returns FALSE):** a NULL/ambiguous attester actor (key maps to several actors), or no
match on any branch. A same-person-different-key case therefore reads as cross-author and is refused —
the safe direction (the clinician re-expresses as a note; mild friction, never a silent bury). The
failure mode of the whole gate is over-refusal, never over-permission.

Defined in `db/005` (before `submit_event`) so both doors — `submit_event` (db/005) and
`apply_remote_event` (db/020), which runs later in the schema array — can call the one helper and never
drift.

### Door change (both doors, identical)

At step 5, after the existing target-existence check, when
`v_mode = 'suppressing' AND v_targets_other AND (b -> 'payload' ? 'target_event_id')`:

```
IF NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
    RAISE EXCEPTION 'submit_event: cross-author suppression refused — you may only suppress your own '
        'events; express disagreement additively (a note referencing the target). (ADR-0043)';
END IF;
```

(`apply_remote_event` raises with its own function-name prefix.) `p_attester_key` is guaranteed
non-NULL here: step 4 already refused a suppressing event that carries no valid human attestation token.

Both files are **edited in place** — the established floor-hardening pattern (e.g. `db/006` was edited
in place for #99's other three items). Pre-clinical posture: no production data, so hardening a floor
function is an in-place `CREATE OR REPLACE`, not a new re-declaration of the whole ~180-line door.

### Principle 12: both doors, one floor

The remote-apply door (db/020) enforces the identical gate via the same helper, so a **replicated**
cross-author suppress faces the same refusal a locally-authored one does — a peer cannot launder a
cross-author suppression in over the wire.

## Testing (TDD, DB-gated)

Failing test first, then the floor code. Against PG18 + `cairn_pgx` (`CAIRN_TEST_PG`). Cases:

1. **self-suppression accepted, signer-key path** — the author `salience.downgrade`s their own event
   (attester key = target `signer_key_id`).
2. **self-suppression accepted, contributor-role path** — attester is a responsibility-bearing
   contributor of the target (not its signer).
3. **cross-author `salience.downgrade` refused** — human B attests a downgrade of human A's event.
4. **cross-author `visibility.suppress` refused** — same, for the hiding op.
5. **remote-apply door refuses cross-author suppress (db/020)** — a synced cross-author suppress is
   rejected at apply (principle 12).
6. **`repudiate` unaffected** — a `targets_other_author=FALSE` suppressing event is not gated
   (regression guard).
7. **safe-refuse edge** — an attester key that resolves to no / ambiguous actor is refused.

## Deliverables (one PR)

- `db/005_submit.sql` — helper + step-5 gate (in place).
- `db/020_apply_remote_event.sql` — step-5 gate calling the shared helper (in place).
- DB-gated integration tests (the seven cases above).
- `docs/spec/decisions/0043-suppression-self-only-disagreement-is-additive.md` — the immutable ADR.
- Spec bump: `docs/spec/index.md` version + the §9.6 (submit surface) / §3.9 prose line pointing at
  ADR-0043; ADR index rows in ADR README + CLAUDE.md-adjacent indices as the project does per ADR.
- Update #99: note 3/4 sub-items already closed and this reframed resolution of the 4th; close on merge.

## Honest limits / non-goals

- No role hierarchy, care-team, or delegation model is introduced (and none is needed — the answer is
  "no cross-author suppression," not "cross-author suppression under authority X").
- No notification tier is required (there is nothing to notify about — the suppression simply doesn't
  happen; the disagreeing clinician authors their own note through the normal additive path).
- "Author of target" is keyed on signing-key / actor identity; a robust person↔multiple-keys identity
  model is out of scope, and the conservative same-person-different-key → cross-author refusal is the
  safe placeholder until one exists.
- SCHEMA version: no new table; whether the `SCHEMA` array constant bumps is decided during
  implementation per the db.rs convention (the migration files are edited in place, not appended).
```
