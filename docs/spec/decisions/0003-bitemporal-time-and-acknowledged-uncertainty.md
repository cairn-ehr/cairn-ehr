# ADR-0003 — Bitemporal event time and acknowledged uncertainty

- **Status:** Accepted
- **Date:** 2026-06-14
- **Supersedes:** —

## Context

Case-mining the ED→ICU transfer (former [§11.3](../open-questions.md)) surfaced a data-model gap
underneath the sync question: **the time a clinical act is *done* is almost never the time it is
*recorded*.** A busy ED physician may write the resuscitation note hours later, after the patient
has moved to ICU or the ward; professionals enter data for the same patient at different times and
places, patient sometimes present, sometimes not, each entry autonomous. There is no way — short of
total audiovisual surveillance — to objectively capture "time performed". A record system must
accept this rather than pretend otherwise.

A second, deeper observation generalized from it: deployed EHRs routinely **force operators to commit
data they cannot be certain of** — a mandatory date-of-birth satisfied only by `01/01/1900`, a yes/no
where the honest answer is "unknown". This manufactures confident falsehoods, which are then trusted
downstream and actively mislead identity matching. The user (an EM physician, founder of an earlier
FOSS Postgres EHR) names this as a primary cause of unreliable real-world records.

These two are the same principle seen twice: **an imprecise near-truth is always preferable to a
precise untruth.** It was elevated this session to a fourth founding principle
([index §1](../index.md), [vision §1](../vision.md), root `README.md`).

## Decision

**1. Bitemporal events** ([data-model §3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)).
Every event carries two times:

- **`t_recorded`** — objective, HLC-assigned, immutable; the basis for causal ordering and sync; the
  **hard ceiling** on effective time. **`t_effective ≤ t_recorded` is an envelope invariant**; a
  violation is *prima facie* falsification, rejected/flagged at write.
- **`t_effective`** — the author's asserted time-performed; defaults to `t_recorded`; freely and
  legitimately backdated; the time **displayed** (with `t_recorded` in brackets).

**2. Two orderings.** Integrity/sync order by `t_recorded`; the clinical narrative is a projection
ordered by `t_effective`. Disagreement between them is the *expected* case (late entries), never a
clash.

**3. Clash detection — flag, never resolve.** A clash is an asserted `t_effective` that is *logically
impossible* against an objective anchor. Two tiers: **Tier 1**, the universal self-ceiling
`t_effective ≤ t_recorded`; **Tier 2**, a small, **closed, explicitly-enumerated** set of clinical
bracket constraints (treated-before-presenting, inpatient-event-after-discharge, …), implemented as a
[§9](../language-substrate.md) coherence check — not an open rules engine, the same discipline as the
identity event algebra. On a clash the system **surfaces and stops**; only the humans reconcile, via a
new overlaying event with full audit trail. The system never picks a winner — that would manufacture a
precise untruth.

**4. Uncertainty-capable value types**
([data-model §3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)):
precision-tagged and interval values; `null` ≠ `unknown` ≠ `refused` preserved distinctly; **no
required field satisfiable only by fabrication** (normative); certainty refined monotonically by
overlay.

**Scope note — two distinct forms of acknowledged uncertainty.** This ADR covers uncertain or absent
*values* (an unknown DOB, an imprecise date, an estimated age). It does **not** cover a clinician's
*provisional/differential assertion* — the `?diabetic` notation, a ranked differential, "probable PE" —
which is an explicitly-flagged clinical *hypothesis* carried in the clinical body, not a value-typing
concern. Both are expressions of principle 4, but they are different mechanisms; representing
differentials/probabilities in the clinical body is deeper content modeling, deferred.

## Consequences

**Easier / gained:**

- Every late entry is automatically *visible as late* (the objective `t_recorded` floor) — the
  malfeasance guard paper cannot provide, since paper lets a backdated note slot invisibly into the
  physical sequence. A paper-parity *surplus*.
- Honest uncertainty *improves* identity matching: an explicit "unknown" is weighted correctly where a
  fabricated value misleads ([§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)).
- Uncertainty resolving over time is just append-only overlay — no new mechanism.

**Harder / the bet:**

- The chart projection must maintain two orderings and render clash flags **without clutter**, at the
  [§1.2](../vision.md#12-the-paper-parity-test-normative) paper-parity pace.
- Tier-2 clinical brackets must stay a **closed, reviewed** set; scope creep into a general temporal
  rules engine would re-grow an unaudited safety surface.
- Every value-bearing type now needs an uncertainty representation in schema *and* UI; forms must
  accept "unknown" everywhere without becoming sloppy.

**How we'd know it's wrong:** if clinicians experience the two-time model or the "unknown" affordances
as *added* friction over paper — which always allowed an unknown date or an estimated age to be written
freely — the
implementation has failed principle 4 / paper-parity and must be simplified.
