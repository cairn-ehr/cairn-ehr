# ADR-0060 — Partial validity: a defect on one line never invalidates another

- **Status:** Accepted
- **Date:** 2026-08-03
- **Derives from:** principle 3 (paper-parity as governing law,
  [§1.2](../vision.md#12-the-paper-parity-test-normative)) — this is a corollary of it, not a new axiom
- **Applies:** principle 4 (acknowledged uncertainty,
  [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md));
  [ADR-0047](0047-medication-reconciliation-resolution.md) (a displayed row is a group);
  [ADR-0049](0049-commitment-based-sign-off-currency.md) (attestation is per thread)

## Context

This principle is **derivable from paper-parity** ([§1.2](../vision.md#12-the-paper-parity-test-normative)) —
it is not a new axiom. It gets its own ADR because it was violated *by a design that had already accepted
paper-parity*, by engineers reasoning carefully, in the first clinical surface Cairn built. A corollary that
is obvious in hindsight and invisible in the moment is exactly the kind that earns a numbered home.

**How it surfaced.** The `clinical.medication` whole-list sign-off slice
([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)) hit a projection defect
([#334](https://github.com/cairn-ehr/cairn-ehr/issues/334)): a reconciled group whose member threads span two
patients displays on only *one* patient's chart, so the other patient's chart silently omits a real drug. The
first fix made sign-off **refuse the entire chart** whenever the node knew of content it could not display,
reasoning that "a clinician must never vouch for a list the node knows is incomplete."

That reasoning is *whole-list* reasoning. But the same slice had, one review round earlier, deliberately
adopted **per-line** semantics on a clinician's correction: attestation is per thread
([ADR-0049](0049-commitment-based-sign-off-currency.md)), and the paper counterpart is the **drug chart**,
where every line carries the signature of whoever is responsible for *that* drug — not a medication-
reconciliation form signed once at the bottom. Under per-line semantics, signing ten visible lines makes ten
claims, and none of them is falsified by an eleventh line being invisible. The refusal imported a premise the
design had already rejected, and no test caught it because the only scenario exercised left the affected
chart empty, where refusing costs nothing. ([ADR-0047](0047-medication-reconciliation-resolution.md) is what
makes a displayed row a *group* while [ADR-0049](0049-commitment-based-sign-off-currency.md) attests per
*thread*; that group/thread seam is where the defect lived.)

**The clinician's ruling** (2026-08-03), which settled it:

> There is no reason to refuse the whole chart if one single line is not visible or not trustworthy. What
> matters is that all visible lines in the chart must be signed … or presented as unsigned in the UI. The
> paper equivalent — a drug can be written up but is missing a signature — which usually would prompt the
> nurse to chase the signature before acting on the medication.

And the general form, which is what makes this an architecture decision rather than a medication one:

> Even partial orders carry weight. Example — a doctor writes up an infusion of 1 L of normal saline over
> 4 hours, signs it, and writes up a minibag of 10 mmol potassium in 100 ml saline, but doesn't sign it yet.
> It is utmost important that the order on the 1 L saline can be carried out even when the signature on the
> potassium mini bag is missing or the order for it is invalid for whatever reason.

**Why this is a safety property, not a convenience.** A system that voids the chart because the potassium
line is unsigned **withholds fluid from a patient over a defect in a different line.** The all-or-nothing
rule does not fail safe; it manufactures a new harm — omission — while protecting against a harm (over-broad
vouching) that per-line accountability had already dissolved. The clinically-correct behaviour and the
convenient behaviour happen to coincide here, and that coincidence is worth stating plainly, because
all-or-nothing *feels* like the conservative choice and is not.

**Why it will bite harder later.** Sign-off is the mildest possible instance: the cost of a bad refusal is a
clinician's time. The same structure recurs wherever a composite clinical object holds independently-
actionable parts — order sets, infusion regimens, care plans, discharge scripts, result panels, referral
bundles — and there the cost of an all-or-nothing rule is a withheld treatment. The decision is recorded now,
before those surfaces exist, because retrofitting composability into a validation model that assumed
all-or-nothing is far more expensive than starting with it.

## Decision

**A defect on one element of a composite clinical object never invalidates the other elements.** Validity,
signature currency, and actionability are properties of the **individual line**, never of the container.

1. **Never refuse the whole for a defect in a part.** A workflow over a composite object (chart, order set,
   regimen, panel) processes every element it can show and stand behind. An element that is missing,
   invalid, unsigned, untrustworthy, or unreadable is **excluded from the operation — not a veto over it.**

2. **Partial completion must be reported, never implied.** Silence about excluded elements is the failure
   mode this decision creates, and it is worse than the refusal it replaces: "signed off 11 medications" over
   a chart with a twelfth outstanding line reads as *finished*. Every operation that acts on part of a
   composite MUST return, and its caller MUST surface, an explicit account of what it did not act on and why.
   The unsigned line stays **visibly unsigned** — the paper chart's blank signature box, which is what
   prompts the nurse to chase it. This is [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)
   applied to completeness: an acknowledged partial truth beats a precise untruth about the whole.

3. **Under uncertainty, over-report incompleteness rather than under-report it.** Where a defect's extent is
   itself uncertain (e.g. two reads of a projection disagree about what is displayable), report the union.
   Uncertainty may only ever *widen* the account of what was not done; it may never narrow it, and it may
   never expand what was silently signed or actioned.

4. **The exclusion must be actionable.** Naming an excluded element without naming what would repair it —
   including the identifiers a repair command actually takes — leaves the operator with a dead end and no
   route but raw database access. A report that cannot be acted on is a refusal wearing a report's clothes.

5. **What may still be refused: identity of the reviewed object, never its quality.** A workflow may refuse
   when it cannot establish that it is acting on *the same object the human reviewed* — e.g. the target set
   moved between the human's review and the write, so signing would silently substitute a different list.
   That question ("is this what they saw?") is categorically distinct from ("is this perfect?") and is the
   only admissible ground for whole-operation refusal. It is a
   [§1.2](../vision.md#12-the-paper-parity-test-normative) forced-rationale-class friction, not a
   completeness gate.

**Scope.** This binds any workflow over a composite clinical object at any layer — the in-DB floor, the
event core, the API, and the UI. It does **not** license partial application of a single atomic *event*:
event bodies remain all-or-nothing at the door ([§3](../data-model.md)). The unit of independence is the
**clinical line**, not the storage write.

**A named residual, not a resolved one.** A gesture that bundles N per-line acts into one transaction still
commits or aborts as one, so a line that fails *unexpectedly at the door* does roll back its siblings — a
smaller instance of the very pattern this ADR forbids. Two things bound it, and neither dissolves it:
elements with *known* defects are excluded **before** the transaction opens (so the rollback case is a
surprise, not a foreseen exclusion), and an all-or-nothing write is the honest failure mode when the
alternative is a partially-applied gesture whose extent nobody recorded. The principled fix — per-line
commit with a per-line outcome record — is deferred, not decided against; it needs a failure that is
asymmetric across lines to even be testable
([#333](https://github.com/cairn-ehr/cairn-ehr/issues/333)). Anyone extending this to orders or
administration, where the cost of a spurious rollback is a withheld treatment rather than a re-click, should
revisit it there rather than inherit this bound.

## Consequences

**Easier.**
- Composite workflows inherit the offline-first posture: a partly-degraded node still lets a clinician act on
  everything it *can* show, which is the same argument as availability-over-consistency applied to a chart
  rather than to a network.
- Projection defects (like [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334)) degrade to a reported
  gap instead of a workflow outage, so the same class of bug stops being able to deny care while it is being
  fixed.
- The rule is testable and falsifiable: for any composite workflow, inject a defect in one element and assert
  the others still complete *and* that the defect is named in the output.

**Harder.**
- **Every** such workflow now owes a reporting surface, and the reports must be good enough to act on
  (decision 4). That is more design work per workflow than a refusal, which needs none.
- Reports accumulate into an alert-fatigue risk, which §1.2's *mostly-pull, selectively-push* limb governs:
  a partial-completion account belongs where the clinician is already looking (the line itself, rendered
  unsigned), not as an additional push.
- "Excluded but reported" is a third state between success and failure, and callers — including exit codes,
  API responses, and UI affordances — must be able to express it. Boolean success/failure signatures are no
  longer sufficient for composite operations.

**The bet.** That making partial completion *visible* is a stronger safety mechanism than making incomplete
work *impossible*. This inherits paper's own wager: a paper chart cannot enforce completeness either, and it
compensates with a blank signature box that a human notices. We are betting the blank box is enough, as it
has been for a century, and that all-or-nothing enforcement buys less than the omissions it causes.

**How we would know the bet fails.** Partial-completion reports being routinely ignored — outstanding
unsigned lines persisting across many sign-off gestures, or a real incident where a reported-but-unactioned
gap reaches a patient. The [§1.2](../vision.md#12-the-paper-parity-test-normative) benchmark for any
composite workflow should measure whether clinicians actually chase the reported gaps; if they do not, the
answer is a better-placed affordance (paper's blank box is *in the line*, not in a summary line at the
bottom), not a return to all-or-nothing refusal.

**First instance.** `cairn-node`'s `medication/signoff.rs`: whole-list sign-off signs every line it can show
and stand behind, and reports both `withheld` lines (present but untrustworthy) and
`groups_missing_from_chart` (not displayable at all) with the thread ids that make the repair command
runnable. Pinned by `an_incomplete_chart_still_signs_every_line_it_can_show`
([#339](https://github.com/cairn-ehr/cairn-ehr/issues/339)).
