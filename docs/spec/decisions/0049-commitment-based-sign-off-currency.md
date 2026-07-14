# ADR-0049 — Commitment-based sign-off currency: a separable attestation overlay, superseded never retracted

- **Status:** Accepted
- **Date:** 2026-07-14
- **Refines:** principle 10 (authorship is compositional; accountability is separable); [ADR-0007](0007-authorship-and-accountability.md) (authorship and accountability — attestation confers responsibility); [ADR-0022](0022-validated-submit-surface-the-write-path.md) (the validated `submit_event` write path — the db/005 attestation gate this overlay goes through unchanged); [ADR-0045](0045-collation-independent-projection-tiebreaks.md) (collation-independent positions, reused for the ancillary "last changed" read). Advances [#163](https://github.com/cairn-ehr/cairn-ehr/issues/163) (asserted-since vs confirmed-current-as-of).

## Context

Medication — and every future clinical stream — needs a way to record that a **human took clinical
responsibility** for a piece of the record, plus an honest signal for whether that vouch is **still
current** ([#163](https://github.com/cairn-ehr/cairn-ehr/issues/163)). Every medication event authored by
slices 1–3 is **device-additive**: signed by the node key, contributor `{"role":"recorded"}`, no human
takes responsibility for the clinical claim. That is correct for the *recording* act (the device faithfully
records what it was told, offline-first) but it means the medication list carries no accountability signal:
a clinician cannot distinguish "a drug a responsible clinician has reviewed and vouches for" from "a drug
the device recorded from some source, unvouched." In a real ED/hospital this distinction *is* the
medication-reconciliation workflow: on admission, and again at each transition of care, a clinician reviews
the list and takes responsibility for it, drug by drug — a first-class clinical act with medico-legal
weight, whose paper equivalent is the signed reconciliation form.

The obvious first design for "is this vouch still current?" — pin the reviewed thread's **head position**
(the latest HLC / `content_address` at review time) and treat anything at-or-below that position as
"reviewed" — is unsound. Sync is set-union, and set-union can deliver a **lower-HLC event after the
sign-off**: a GP's earlier-wall-clock update that had not yet reached this node when the clinician signed
off. That event is causally *below* the pinned head, so a head-position pin **silently absorbs it as "still
reviewed"** even though the clinician never saw it. This is a real offline-first failure mode, not a
hypothetical corner case — it is exactly what set-union sync is built to deliver.

## Decision

**A responsibility-bearing sign-off is a separable per-thread attestation overlay, and "still current?" is
a convergent set-commitment compare, never a position compare.**

1. **Responsibility is separable authorship** (principle 10, [ADR-0007](0007-authorship-and-accountability.md)).
   One new event type, `clinical.medication-attestation.asserted`, carries a responsibility-bearing human
   contributor (`{"role":"attested","responsibility":"attested"}`) referencing an existing `medication_id`
   thread. The device-signed medication events themselves are **unchanged** — a *different* human may
   vouch, possibly *later*, without touching who recorded the fact. The `responsibility` key is what trips
   the **existing** db/005 attestation gate (the 3-arg `submit_event` door, requiring an enrolled
   `kind='human'` actor) — no floor special-case, no db/005 change.

2. **A sign-off pins a convergent commitment of the exact content-event set it reviewed, not a position.**
   `reviewed_commitment` is the sorted-concat-hash of the thread's content-event `content_address`es,
   computed by **one SQL function** used at both author time and read time — never a Rust value compared
   against a SQL value, so there is no cross-language framing to keep in sync.

3. **Staleness = current-set commitment ≠ pinned commitment.** This is sound *because* thread content is
   append-only / grow-only (never substituted) — the set only ever grows, so **any** later event, at
   **any** HLC position, including one lower than the sign-off's, changes the set and therefore the
   commitment. A head-position pin cannot make this guarantee; a set-commitment compare can. The same
   compare also catches a divergent set on another node (same count, different membership) as stale —
   erring toward re-review, the safe direction.

4. **Responsibility is superseded by correcting the record, never retracted — there is no de-attestation
   event, and there deliberately never will be.** A clinician who vouched in error does not un-vouch; they
   author a corrective clinical event (a dose correction, a cessation, a new assertion), which changes the
   thread's content set, which flips the prior attestation stale, which prompts re-review and re-vouch. The
   erroneous vouch **stays in the record** — the accountability trail reads *"Dr X vouched for the wrong
   value at T1, corrected and re-vouched at T2,"* never an erasure (principles 1/2).

## Consequences

**Now guaranteed:** the lower-HLC late-arrival gap a head-position pin would silently absorb is closed; the
cross-node divergent-set case is also closed (errs toward re-review); [#163](https://github.com/cairn-ehr/cairn-ehr/issues/163)
advances with a working mechanism, not just terminology; every future clinical stream's "re-affirmation /
sign-off currency" need has a reusable precedent to build on instead of each stream inventing (or
mis-inventing) its own position-based staleness check; a reconciled group ([ADR-0047](0047-medication-reconciliation-resolution.md))
reads attested-current only when **every** active member thread has its own current attestation — a
conservative rollup that never reports a false "current" from partial review.

**Accepted costs / deferred:**
- **Honest residual:** a commitment cannot cover an event that exists on no reachable node yet — that is
  reviewing the future, not a gap this mechanism can close. The §5.13 background re-review sweep remains
  the defense-in-depth backstop.
- **Whole-list sign-off is composed** from N thread attestations, not a distinct event type; a summary
  "list reviewed at T" event is a future convenience if a worklist wants one.
- `reviewed_count` is a legibility hint only, never a staleness authority — a disagreeing count cannot
  cause unsoundness, but it also cannot resolve one.
- The commitment view recomputes a hash per thread per read; negligible at today's scale (tens of threads,
  a handful of events each) — memoize into the overlay table if a hot path emerges.

**How we would know the bet is failing:** the §5.13 re-review sweep's advisory worklist runs persistently
high (thread content is changing faster than clinicians can review it, suggesting the per-thread
granularity or the sweep cadence is wrong for real workloads), or clinicians routinely need to know *which*
specific event un-staled a vouch (a diff, not a boolean) badly enough to justify carrying more than a hash.

**Not a new founding principle.** This is principle 10 (authorship is compositional; accountability is
separable) plus the append-only / grow-only guarantee already load-bearing elsewhere in the architecture,
applied to the question "is a human review still valid." Reuses the db/005 gate verbatim and
[ADR-0045](0045-collation-independent-projection-tiebreaks.md)'s collation-independent position discipline
for the ancillary "last changed" legibility read.
