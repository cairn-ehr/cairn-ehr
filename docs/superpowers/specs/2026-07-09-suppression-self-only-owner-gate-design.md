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

**A suppressing overlay that targets another *human author's* event is refused at the floor.
Suppression of human-authored content is self-only. Disagreement with another author's content is
expressed additively — a note/advisory that references their event — never by touching their event.
Agent-authored / un-owned advisories (no responsible human) stay dismissable by any enrolled human —
that is the clinician-overrides-the-machine path, not the burying of a colleague.**

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

### "Author" means *human* author (principle 10)

The rule protects **human-authored** content. An **agent-authored advisory** (ADR-0030 / principle 10:
authorship is compositional but the agent bears no responsibility) is the machine's suggestion, not a
colleague's clinical judgment — so a clinician dismissing it via `salience.downgrade` / `visibility.suppress`
is the intended *human-overrides-the-machine* workflow, not a burying of another author. Suppressing an
un-owned advisory therefore stays open to any enrolled human. Only content that has a **human author**
is self-only. (This keeps the existing `accepts_suppressing_event_with_valid_human_token` test green —
it suppresses an agent-authored baseline — which is the correct behaviour, not an exception.)

A target is **human-authored** iff its `signer_key_id` resolves (via `actor_current`) to a `kind='human'`
actor, **or** it carries a stored human attestation (`event_log.attester_key IS NOT NULL` — the floor
only ever stores a `kind='human'` attester key). The target's set of human authors is
`{signer_key_id if human} ∪ {hex(attester_key) if present}`. This uses stored columns plus one registry
lookup — no fragile contributor-JSON parsing.

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

Returns TRUE iff suppression is permitted:

1. Compute the target's **human authors**: `H = {target.signer_key_id | it resolves to a kind='human'
   actor} ∪ {hex(target.attester_key) | target.attester_key IS NOT NULL}`.
2. **If `H` is empty** — the target is agent-authored / un-owned — return **TRUE** (any enrolled human
   may dismiss an AI advisory).
3. **Otherwise** (target is human-authored) return **TRUE iff `encode(p_attester_key,'hex') ∈ H`** — the
   attester is one of the target's human authors (self-suppression). Else **FALSE** (cross-human
   suppression refused).

**Safe-refuse:** when the target is human-authored and the attester is not among its human authors, the
result is FALSE. A same-human-different-key case therefore reads as cross-author and is refused — the
safe direction (the clinician re-expresses as a note; mild friction, never a silent bury). The failure
mode of the whole gate is over-refusal on human-authored content, never over-permission.

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

1. **self-suppression accepted, human-signer path** — human A `salience.downgrade`s an event A signed
   (attester key = target `signer_key_id`, human).
2. **self-suppression accepted, human-attester path** — human A suppresses a target A authored as a
   responsibility-bearing contributor (target `attester_key` = A, A did not sign it).
3. **agent advisory dismissable** — a human suppresses an agent-authored, un-owned target
   (`H` empty) — accepted (the human-overrides-machine path; also the existing
   `accepts_suppressing_event_with_valid_human_token` regression).
4. **cross-human `salience.downgrade` refused** — human B attests a downgrade of human A's event.
5. **cross-human `visibility.suppress` refused** — same, for the hiding op.
6. **remote-apply door refuses cross-human suppress (db/020)** — a synced cross-human suppress is
   rejected at apply (principle 12).
7. **`repudiate` unaffected** — a `targets_other_author=FALSE` suppressing event is not gated
   (regression guard).

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
- "Human author of target" is keyed on signing-key identity + the stored human attester key + the
  signer's registry `kind`; a robust person↔multiple-keys identity model is out of scope, and the
  conservative same-human-different-key → cross-author refusal is the safe placeholder until one exists.
- The agent/human split rests on the actor registry `kind` at the target's signer resolution. An agent
  key later re-enrolled as a human, or vice versa, is not a modelled case (registry `kind` is fixed per
  key in the current model); if that ever changes, the gate reads the *current* kind — acceptable, since
  the failure direction stays over-refusal on anything that looks human-authored.
- SCHEMA version: no new table; whether the `SCHEMA` array constant bumps is decided during
  implementation per the db.rs convention (the migration files are edited in place, not appended).
```
