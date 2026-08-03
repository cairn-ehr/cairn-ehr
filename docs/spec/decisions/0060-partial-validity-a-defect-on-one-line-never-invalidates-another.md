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

6. **Transaction scope must match clinical atomicity — no collateral damage on rollback.** The rule binds
   the **storage layer**, not merely the targeting logic. A workflow acting on N independent lines must not
   bundle them into one database transaction: a failure on any one would then roll back every other line's
   committed act, which is this ADR's own anti-pattern reintroduced one layer down. Each independently-
   actionable line commits in its **own** transaction, and a failed line is rolled back **alone** and
   reported (decision 2). What makes a multi-line gesture *one human act* is the single unseal and the
   single review — never a shared transaction.

**Scope, and what stays atomic.** This binds any workflow over a composite clinical object at any layer —
the in-DB floor, the event core, the API, and the UI. The unit of independence is the **clinical line**, not
the statement count. Three things are therefore *not* split:

- a single **event body**, which remains all-or-nothing at the door ([§3](../data-model.md));
- a single clinical act that inherently spans several threads — a medication *reconciliation* and the
  attestations for both of its subjects ([ADR-0047](0047-medication-reconciliation-resolution.md)) commit
  together, because you cannot half-link two drugs;
- a content event and the vouch authored with it in the same gesture, which is one act plus its signature.

The test is whether the parts are **independently actionable in the clinic**, not whether they were written
in one keystroke. Two drugs on a chart are independently giveable; the two halves of a reconciliation are
not.

> This decision replaced a deliberate deferral. The first draft of this ADR recorded the shared-transaction
> rollback as a *bounded residual* — known-defective lines are excluded before the transaction opens, so a
> rollback is a surprise rather than a foreseen exclusion — and deferred the fix for want of a way to test an
> asymmetric failure ([#333](https://github.com/cairn-ehr/cairn-ehr/issues/333)). The maintainer rejected the
> deferral: *"transaction scope must match the atomicity we discussed — an order must not be refused because
> another order is invalid or incomplete. Hence db transactions must ensure no collateral damage on
> rollbacks."* That is correct, and the deferral had smuggled the anti-pattern back in under a bound. The
> test turned out not to need an injection seam either: a **partial-custody node** — one thread's sealed body
> synced without its DEK, a state the schema already anticipates — makes exactly one line uncommittable while
> its siblings commit normally.

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
- Per-line transactions cost per-line round trips, and they surrender the convenience of "it either all
  happened or none of it did." A caller can now observe a genuinely partial world, which is the point, but it
  means every consumer of a multi-line result must handle three outcomes rather than two.
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
and stand behind, each in its own transaction, and reports `withheld` lines (present but untrustworthy),
`groups_missing_from_chart` (not displayable at all) — both with the thread ids that make the repair command
runnable — and `failed` lines (write errored, rolled back alone). Pinned by two tests:
`an_incomplete_chart_still_signs_every_line_it_can_show`
([#339](https://github.com/cairn-ehr/cairn-ehr/issues/339)) for decisions 1–4, and
`a_line_that_cannot_be_attested_never_rolls_back_the_others` for decision 6 — the latter breaks the
**middle** line of three, so a successful commit sits both before and after the failure and the earlier one
must survive it.
