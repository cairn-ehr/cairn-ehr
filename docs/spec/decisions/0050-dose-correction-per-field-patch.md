# ADR-0050 — Dose correction is a per-field patch: explicit strike, corrected effective drives the winner

- **Status:** Accepted
- **Date:** 2026-07-15
- **Refines:** [principle 4](../index.md#founding-principles-the-lens-for-every-decision) (acknowledged
  uncertainty — an imprecise near-truth beats a precise untruth; no forced fabrication); the
  [§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time) bitemporal model
  (`t_effective` is the freely-correctable claim). Extends the dose-timeline overlay opened by slice 2
  (`db/032_medication_dose.sql`) on the surface from slice 1. Reuses the existing
  `clinical.medication-dose-correction.asserted` verb — no new event type.

## Context

Slice 2 gave the medication surface a bitemporal dose timeline (point 0 seeded from the assert's `started`,
plus one point per `-dose-change`) and a `-dose-correction` verb that overlays a *targeted* point. But that
overlay carried only `amount`/`unit`/`reason`/`info_source`, and both timeline views computed the effective
date purely from the **original** event. Two gaps followed directly from that shape:

- **A mis-keyed effective date could not be fixed.** This is not cosmetic: the current dose is the
  latest-*effective* point, so a wrong effective date can make the wrong dose "current" or scramble the
  titration trail's chronology. Correcting the date is bitemporal *repair*, not a display-label edit — but
  the verb had no field to carry the fix.
- **The point's clinical reason could not be fixed either**, and a related defect sat underneath: the
  correction row *stored* a `reason` column (meant as "why I'm correcting"), but the dose-history view read
  the *original* event's reason, so the correction's reason was written and never surfaced — dead data.

Slice 2's own correction semantics also had a latent sharp edge: a correction payload with no set fields was
interpreted as "strike the dose to unknown" (omission-as-strike). That collapses two different clinician
intents — *"I have nothing new to say about the dose"* and *"I no longer know the dose"* — onto one wire
shape, which is exactly the ambiguity [principle 4](../index.md#founding-principles-the-lens-for-every-decision)
warns against: uncertainty must be a first-class, *explicit* value, never inferred from what a clinician left
blank.

## Decision

**A dose correction is a per-field patch of three independent groups — `dose` (amount+unit), `effective`
(value+precision), `reason` (the point's clinical reason) — with an explicit `strike` array for set-to-unknown,
and the corrected effective date participates in current-dose winner selection.**

1. **Per-field patch, not whole-point restatement.** For each group *G*: present as a set-key → *touched*,
   value becomes the given value; named in `strike` → *touched*, value becomes unknown; neither → *untouched*,
   the point's existing value is kept. This makes the common case — fix one field, leave the rest — safe:
   correcting a date typo never silently wipes the dose, and vice versa. A group named both as a set-key and
   in `strike` is a contradiction the floor rejects. The `dose` group is patched as one atomic
   `{amount?, unit?}` object (the same shape assert / `-dose-change` use) — per-field keep applies *between*
   groups, not inside the dose quantity, which is one clinical value.
2. **`strike` makes set-to-unknown first-class and explicit.** Slice 2's implicit "omit everything = strike
   the dose" is retired; a correction that touches nothing (no set-key, no `strike` entries) is now a no-op
   the floor rejects. To assert "I don't know the dose," a clinician names `dose` in `strike` — an explicit
   act, not an absence.
3. **The corrected effective date drives current-dose winner selection.** The dose-timeline views' `ORDER BY`
   now sorts on the *corrected* effective value when the `effective` group was touched, falling back to the
   original otherwise. Winner selection is by effective-date order, so fixing a mis-keyed date changes which
   point reads as current and can reorder the titration trail — this is the bitemporal repair the un-correctable
   date made impossible, not a cosmetic label change.
4. **The correction rationale is a separate `note`, distinct from the point's clinical `reason`.** Slice 2's
   correction `reason` meant "why I'm correcting" but was never surfaced. This slice repurposes `reason` to
   mean what it means everywhere else on a dose point — the *clinical* reason ("titration", "renal dosing"),
   now patchable and finally surfaced in the dose history — and adds `note` as the always-additive audit
   rationale for the correction act itself ("mis-keyed the date"). The two answer different questions ("why is
   the dose what it is" vs. "why was this correction made") and neither should overwrite the other. (The CLI
   spells this `--correction-note`, not `--note` — the latter is already the attestation vouch note on
   `--attest-as`, and the two must stay distinguishable at the point of care.)
5. **`schema_version` bumps `clinical.medication-dose-correction/1 → /2`.** Cosmetic in the sense that nothing
   keys off the version number, but honest: the interpretation of an *omitted* field changed from "unknown"
   (v1) to "keep" (v2), and that is exactly the kind of silent meaning-shift a version bump exists to flag. No
   `/1` correction event exists in production (pre-clinical posture), so there is no live-data migration
   concern — only the honesty of the signal itself.

No new event type, no new envelope field, no floor bypass, no `SCHEMA` counter bump — the existing verb, the
existing floor function name, and the existing apply trigger are all reused (extended, not replaced). The
change is confined to one new migration (`db/035_medication_dose_effective_correction.sql`)
that `ALTER TABLE`-extends slice 2's `medication_dose_correction` table and re-defines the correction floor,
apply trigger, and the two dose-timeline views in place; db/031–034 are untouched.

## Consequences

**Now possible:** a mis-keyed effective date or clinical reason on any dose point is correctable without
restating the whole point; the dead correction-reason column is surfaced; set-to-unknown is an explicit,
legible act instead of an inferred one.

**Convergence boundary — must be read alongside the per-field patch.** The overlay keeps slice 2's shape: **one
row per corrected point, highest-HLC-wins wholesale** (`cairn_hlc_overlay_wins`) — this is what keeps set-union
sync convergent without inventing a dangerous field-level merge. Per-field patch therefore applies **within**
one correction event, against the original point; it does **not** compose *across* corrections of the same
point. A later correction of a point **supersedes** an earlier correction of that same point wholesale, rather
than field-merging with it. Concretely: if correction A fixes the effective date and a later correction B (on
the same point) fixes only the dose, B's *effective* group is untouched — meaning "keep the point's own
effective" — and B wins the row outright, so **A's date fix is lost** unless B restates it. To keep an earlier
field-fix alive, a subsequent correction of the same point must restate it. **This is a deliberate, documented
boundary, not an oversight**: true per-field merge across corrections would require per-field HLC tracking
(either multiple overlay rows or a per-field overlay guard) to remain convergent under set-union sync, and that
is a real design cost this slice did not take on. It is left as a **deliberate future refinement** if the
wholesale-supersede boundary proves to bite in practice.

**Accepted costs / deferred:**
- Correcting the medication's top-level `started` date (`patient_medication_current.started_value`, the
  assert's own field) is out of scope — a correction targeting point 0 fixes that point's effective in the
  dose *timeline* only. A future "correct the statement's started/term/sig" slice would handle the assert-level
  field.
- The effective `value` stays an uncertainty-capable free string, matching `-dose-change`; the floor checks
  structure/type only and never parses the date (locale-neutral).
- The backfill that reshapes pre-035 correction rows (`reason` → `note`, touched-flags inferred as
  dose-only-corrected) is guarded and idempotent, but is cosmetic housekeeping only possible because no `/1`
  correction event exists in production yet — this discipline does not generalize once the record carries real
  patient data.

**How we would know the bet is failing:** if clinicians routinely correct the same dose point more than once
and lose earlier field-fixes often enough that restating becomes a recurring workflow tax, that is the signal
to build the per-field-HLC refinement rather than continue relying on "restate to keep." Absent that pressure,
the simpler wholesale-supersede overlay is the right cost/complexity trade for a still-early clinical surface.

**Not a new founding principle.** This is principle 4 (acknowledged uncertainty — explicit strike over
inferred omission) and the existing bitemporal model (§3.6 — effective time is the freely-correctable claim)
applied to a correction overlay already in production. Matches the pattern of the two prior medication ADRs —
[ADR-0047](0047-medication-reconciliation-resolution.md) (reconciliation as a link) and
[ADR-0049](0049-commitment-based-sign-off-currency.md) (attestation as a separable overlay) — each of which
extended the medication surface's overlay machinery without adding new wire shape.
