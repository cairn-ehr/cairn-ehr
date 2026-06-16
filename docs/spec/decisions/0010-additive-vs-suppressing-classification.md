# ADR-0010 — Additive-vs-suppressing classification: structural derivation, the demotion boundary, and automation-complacency detection

- **Status:** Accepted (refines [ADR-0007](0007-authorship-and-accountability.md))
- **Date:** 2026-06-15

## Context

[ADR-0007](0007-authorship-and-accountability.md) established that an un-owned (un-attested) clinical
output is safe-by-construction only when it is **strictly additive** — one that "can only *raise* signal …
and can never reduce, defer, de-prioritise, auto-file, or auto-resolve something a human would otherwise
act on" — and recorded that "the additive-vs-suppressing nature of an output is a recordable, projectable
property." It deferred *how* that property is derived, validated, and enforced as "the sharpest of the
follow-ons." [ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) then
made it operationally load-bearing: noise reduction *is* suppression, so the demotion-versus-suppression
boundary now governs the notification economy as much as AI authorship. This ADR makes the property
concrete.

Forces:

- **Classification must be unforgeable.** A producer that self-declares *"I am additive"* is exactly the
  flag ADR-0007 rejected ("something a human must remember to set … binary"); a buggy or careless producer
  would declare additive while suppressing. Self-declaration cannot be the authority.
- **Suppression is frequently *desirable*.** Clinicians drown in results — thousands in objectively-normal
  range, or uniformly abnormal but rarely clinically relevant. Lowering the priority of plainly-normal
  values so attention concentrates on the likely-relevant ones is a *wanted* capability, not a hazard. The
  design must not treat all reduction of signal as dangerous.
- **A formally-additive output can be *practically* suppressing.** Automation complacency: a relied-upon,
  formally-additive alert (it only raises a flag, hides nothing, is always overridable) atrophies the
  independent human process it was meant to backstop, so its false-negative becomes *total* — the
  relied-upon sepsis alert whose miss is worse than paper, where everyone screened by hand. The structural
  classifier sees the output's direct effect, not its second-order effect on behaviour.
- **Mechanism, not policy (principle 9).** Cairn must make trend-aware, context-aware triage *possible*
  without implementing or regulating the rules.

Validating cases, from emergency practice:

- **The result flood.** Thousands of objectively-normal results are the noise baseline; demoting them is
  the wanted behaviour.
- **Trend beats instantaneous value.** eGFR 90 → 70 → 30 is an **ALERT** (high priority); 30 → 35 → 38 is
  **TREND IMPROVING** (low priority) — the same latest value, opposite salience. A rule classifier reads the
  time series; AI oversight extends it with medication, past history, and recent consults to set the
  interpretive context.

## Decision

Additive-vs-suppressing is **structurally derived, enforced in the trusted surface, and never abolished —
only relocated**. Canonical home: [data-model §3.9](../data-model.md#39-authorship-and-accountability);
the salience-scoring seam is [identity §5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor),
the consumer-side atrophy signal is [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side).

1. **Derived, not declared: additive ≡ overlay, suppressing ≡ foreclosure.** The distinction is the
   **append-only principle (1) applied to the attention/decision layer** rather than the data layer. An
   *additive* output adds a layer a human still sees and can act on (a candidate, a warning, a priority);
   it is source-preserving, always-overridable, and monotone — its failure mode is *noise*, and its worst
   case is paper (which had no decision support at all). A *suppressing* output removes, hides, defers,
   auto-acknowledges, auto-files, auto-resolves, or otherwise forecloses what a human would have seen or
   done; its failure mode is *silence*, and its worst case is worse than paper. The falsifiable test is
   ADR-0007's obligation made operational: **could a human still independently see and act on everything
   they would have without this output?** Yes → additive; no → suppressing.

2. **Demotion is additive; only hiding or auto-deciding is suppressing.** The primary, safe noise-reduction
   tool is **priority-lowering, not removal**: the result flood is tamed by demoting objectively-normal
   values to very-low priority while they remain fully reachable and acknowledgeable
   ([ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) demotion/digest).
   Demotion needs no owner. The boundary is crossed only when demotion becomes **hide-to-nothing**, or when
   an assessment is **auto-acted** (auto-file out of a queue, auto-acknowledge, auto-resolve, auto-substitute,
   auto-decline). This is what makes the user's *"low-priority for plainly normal results"* both desirable
   and safe — it never leaves the additive side.

3. **The suppressing set is closed and enumerated** — the same discipline as principle 1's "small,
   explicitly enumerated set of clinically-reasoned merge policies." Members: auto-acknowledge, auto-resolve,
   auto-file / route-out, filter-hide, below-threshold-suppress-to-nothing, auto-substitute, auto-decline.
   **Additive is the open complement and the runtime default.** The set is curated at the trusted surface
   ([§9](../language-substrate.md)) with a **suppressing-until-proven-additive review discipline**: a new
   operation must affirmatively demonstrate information-preservation to earn the additive (un-owned-eligible)
   class — the safety-asymmetric stance (false-merge ≫ false-split, [identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)) applied to the development process, so
   there is never an unclassified operation at runtime.

4. **Enforcement is structural and in-database.** The trusted apply/projection layer **refuses to apply a
   suppressing-class operation that carries no responsible owner** — not a runtime check on a declared flag,
   but a constraint on what is representable: an un-owned producer is confined to the additive vocabulary by
   construction. Same shape as [§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows) barring
   Tier-1 reattribution of executed-effect events, and [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)
   refusing to let routing withhold a result from a present clinician.

5. **Conservation of responsibility.** There is never truly un-owned suppression: accountability is either
   at the **event** (a responsibility-bearing contributor, [§3.9](../data-model.md#39-authorship-and-accountability))
   or, where policy permits a *class* of un-owned suppression, at the **explicit, audited configuration act
   that permitted it** (ADR-0007's "an override toward permitting it is itself an explicit, audited, owned
   configuration act"). Policy can *relocate* the owner — never abolish it. This is the same relocation seen
   in the ADR-0005 deniable-deletion rung (cover migrates to a self-held copy) and the ADR-0008 sign-as
   salvage (authorship relocates without vanishing).

6. **Declaration is a one-way caution ratchet** — the answer to "author-declared, derived, or both." Derived
   sets the floor; a responsible human may declare a formally-additive output *more* suppressing (subjecting
   it to the owner gate), **never less**. This is the concrete handle for de-facto suppression: when a
   department will lean on a formally-additive triage as gospel, its owner marks it "treat as suppressing,"
   pulling it into the accountability regime even though its direct effect is only-additive. The ratchet
   mirrors every other safety floor in the spec (you may blur a signal coarser, never finer
   [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope); raise
   priority, never lower the floor [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)).

7. **Triage is a salience-scoring extension point — mechanism, not policy.** Cairn ships the seam for
   **trend-aware rule classifiers** (deterministic: an eGFR-slope rule emits *ALERT* for 90→70→30, *TREND
   IMPROVING* for 30→35→38) and **optional AI oversight** (interpretive context from medication, past
   history, recent consults), wired to the [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)
   salience dial. The classifier's output is an **authored, additive event** with a contributor set —
   `{rule-classifier | AI, graded | triaged}`, the [§3.9](../data-model.md#39-authorship-and-accountability)
   contributory roles that exist for exactly this — and is therefore safe un-owned *because* additive. The
   rules and the model are policy and deployment configuration; Cairn implements and regulates neither. The
   classifier is fit-for-purpose (a defect mis-prioritises but, being additive, never hides — caught because
   the result still shows); the floor that a demotion can never silently become a hide is safety-critical.

8. **Automation-complacency detection is built now, as an additive meta-signal.** Because the structural
   classifier sees only direct effect, Cairn computes from the audit log
   ([§7](../security.md), [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)
   acknowledgment events) whether **independent human review of a class has collapsed** — humans now only
   acknowledge the automated assessment, never assess first — and surfaces that **atrophy** as its own
   additive, governance-tier warning (*"independent review of X has fallen to near-zero; the automated layer
   is now a single point of failure"*). It is itself additive (it only raises a signal), so it is safe
   un-owned and self-consistent; and it is population/governance-facing (mostly-pull, [ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)),
   not a point-of-care interrupt, and is most meaningful at a tier with enough volume to be statistically
   honest (a single workstation cannot tell complacency from a quiet shift).

## Consequences

- **Easier:** the result-flood "drowning" is solved by additive demotion with no accountability ceremony;
  trend/AI triage falls out of the existing contributor set + salience dial with no new primitive; the
  genuinely dangerous tail (hiding / auto-deciding) is a small closed set behind one structural gate;
  de-facto suppression gets both a concrete handle (the ratchet) and a measurable signal (atrophy
  detection); and "un-owned suppression" is shown to be a contradiction — responsibility is conserved.
- **Harder / trusted surface:** the closed suppressing set, the apply-layer owner-gate, and the
  demotion-cannot-silently-become-a-hide floor are safety-critical (in-database/Rust, [§9](../language-substrate.md));
  the salience classifier (rule + AI) and the atrophy detector are fit-for-purpose/advisory. The seam —
  classifier output → the floor that guarantees additivity — is the one safety-critical path, structurally
  like the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  seal-time and [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
  proximity→stamp seams.
- **The bet:** that overlay≡additive / foreclosure≡suppressing cleanly partitions real outputs, that the
  closed suppressing set stays small and stable, and that additive demotion + trend/AI triage actually tame
  the flood without anyone needing to *hide* (rather than demote) a result un-owned. We would know the bet
  is wrong if the suppressing set keeps growing, if real triage proves unusable unless it may hide un-owned,
  or if atrophy detection is too noisy below population scale to be actionable.
- **Policy-neutral (principle 9):** Cairn ships the classification, the structural gate, the salience-scoring
  seam, the caution ratchet, and the atrophy detector; it takes no side on the triage rules, on which
  suppression classes a deployment permits un-owned, or on the atrophy thresholds.
