# ADR-0009 — The notification economy: unbundling "priority," responsibility-routing, and the acknowledgment floor

- **Status:** Accepted
- **Date:** 2026-06-15

## Context

Open question §11.10. The spec already *generates* notifications in several places — the
history-arrival alert ([§5.4](../identity.md#54-unidentified-registration-john-doe-baked-into-the-root),
*"prior history now available — N allergies, M active medications"*), the contamination cascade
([§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows), *"a note you read on patient B
at 14:32 has been moved to patient A"*), the safety-projection warning
([§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope),
*"⚠ Grade X interaction with confidential content — break glass"*), the responsibility-state surfacing
([§5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)), and the freshness /
honest-assembly signals ([§6.2](../sync.md#62-consistency-model)). Each was designed in isolation. The
open question is the *economy*: these are safety-critical but **additive**, and additive signals are
exactly what drown when a system pushes everything. Deployed EHRs are the cautionary tale — clinicians
override the overwhelming majority of interruptive alerts, and the resulting override reflex
(click-through) then defeats the one alert that mattered. **Alert fatigue and the confirmation-dialog
click-through that ADR-0008 designed against are the same disease**: a discrete demand for attention,
repeated until it is dismissed unread.

§11.10 asked for *"a priority taxonomy."* That framing is the trap. **"Priority" is one word carrying
several jobs that run at different frequencies** — the same error ADR-0006 found in *scope*, ADR-0007 in
*signature*, and ADR-0008 in *authentication*.

Validating cases, from emergency practice in a largely locum workforce across several health systems:

- **The critical result whose orderer has already left.** In a locum-heavy department the ordering
  doctor is routinely gone by the time the result lands. Some hospitals have follow-up policy; many have
  none; resource-poor remote hospitals run informally — whoever has time works the queue. There is no
  single universal owner to address the result to.
- **The "orderer must release the result before anyone else sees it" policy.** Witnessed repeatedly to
  *cause* missed important results: the result is withheld from every present clinician until an absent
  doctor reviews it. The architecture must be able to *express* this kind of routing preference but must
  never be able to *enforce it as withholding* from a clinician who is present and looking at the chart.
- **The critical-value telephone callback** — paper's strongest notification: the lab phoned a human,
  who repeated the value back (closed loop), and it was logged; on no-answer it escalated (consultant →
  nursing supervisor). This is the floor that must not regress.

## Decision

The notification economy dissolves into existing primitives. **No new founding principle and no new
event stream.** Canonical home: [identity §5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor);
the minimal data-model invariants are [§3.11](../data-model.md#311-notifications-as-projections-responsibility-routing-and-acknowledgment).

1. **Unbundle "priority" into orthogonal dials.** A notification is characterised by separable
   attributes, not one scale: **salience** (intrinsic clinical importance of the underlying fact),
   **acknowledgment requirement** (none / soft "seen" / **hard** closed-loop), **addressing** (who
   owns acting on it), **delivery modality** (interruptive push / ambient / pull-digest), and
   **escalation** (what happens on non-acknowledgment). The load-bearing split is **salience ≠
   interruptiveness.** A high-*salience* standing fact (a penicillin allergy) belongs *ambient and
   always-visible*, never interruptive — re-popping it on every order is precisely what manufactures
   click-through. A high-*urgency transition* (the K⁺ just resulted) is interruptive *once*, then becomes
   ambient and acknowledged. This is the generalisation, from the arming gesture to all surfaced
   information, of the [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
   insight that ambient/peripheral display is the *opposite* of a confirmation dialog. Collapsing these
   dials into one "priority" that defaults everything to interruptive popup is the entire mechanism of
   alert fatigue.

2. **A notification is a projection, not a mailbox.** It is a *delta*: the event stream evaluated
   against *this clinician's own audit-log history of what they have already viewed and acted on* (the
   contamination cascade is the pure case — *"a note you read moved"*; history-arrival is a delta against
   the previously-empty state). The audit log already records view/act (it powers the
   [§5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows) disclosure-scope query). So the
   "inbox" is a derived projection plus an append-only **acknowledgment** event — never a mutable
   unread-flag that is deleted. Same shape as the link graph
   ([§5.1](../identity.md#51-linkage-layer-never-merge-always-link)), the sensitivity grade
   ([§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)),
   and the trust state. Acknowledgment rides the **existing audit stream** ([§7](../security.md)); no new
   stream.

3. **Noise reduction is suppression, and suppression is accountable (ADR-0007).** To cut noise you must
   hide signal, and hiding signal is the suppressing act [ADR-0007](0007-authorship-and-accountability.md)
   governs. The taxonomy therefore splits cleanly: **demotion / coalescing / digest is additive** (the
   signal still reaches the human, only quieter, batched, or merged — safe by construction, free) versus
   **filtering-out / auto-acknowledge / below-threshold-hiding is suppressing** (owned, audited,
   policy-gated). A machine-authored notification may only ever *raise* signal, never lower it
   ([data-model §3.9](../data-model.md#39-authorship-and-accountability) unchanged); an automated filter
   that *hides* an alert is making an ADR-0007 suppressing decision and someone must answer for it.
   Auto-acknowledgment of a hard-ack class is the silent-falsification line paper-parity excludes — it
   claims a human closed the loop who did not.

3a. **The line between demotion and suppression.** Demotion changes *how* and *when* a signal reaches a
   clinician; suppression decides it *never* reaches them (or decides on their behalf). Batching a
   routine result into an end-of-shift digest is demotion (still reaches them, still acknowledgeable);
   discarding it, or auto-acknowledging it as if seen, is suppression. The default is additive; every
   suppressing rung is an explicit, owned, audited configuration act (principle 9, like the ADR-0005
   erasure rungs and the ADR-0006 disclosure-coarsening rungs).

4. **Responsibility-to-follow-up is a graded, multi-source, append-only overlay; the effective
   responsible set is a projection.** The **co-equal inbox is the infrastructure; policy does the
   prioritisation.** A result bears a **responsibility tag**: the **orderer** is an intrinsic tag and is
   *always* prioritised for the telephone callback; *policy* adds further tags, and **more than one
   clinician may hold a tag at once** — a critical-results default fallback, the covering doctor for an
   orderer who has left or is temporarily absent, and a timeout reassignment when the responsible present
   doctor has not addressed it within a policy window (they may be busy with something more urgent). The
   effective responsible set is the **highest-standing projection** over this overlay — *never merge,
   always overlay* — exactly the shape of the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   sensitivity stream and the [§5.1](../identity.md#51-linkage-layer-never-merge-always-link) link graph.
   Cairn ships the tag mechanism, the timeout-reassignment primitive, and the orderer-default; *which*
   policy assigns the fallbacks, and whose acknowledgment *discharges* the obligation versus merely
   records a view, is policy.

5. **Follow-up responsibility is never a visibility gate — the safety floor.** A new result is
   **always** visible to whoever has just opened the patient; the architecture *never withholds*. The
   "orderer must review/release first" preference is expressible only as *ambient state* (*"not yet
   reviewed by the requesting doctor"*) — the architecture **refuses to enforce withholding** from a
   present clinician. This is the consumer-side mirror of [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md)'s
   *"replication is never the confidentiality boundary"*: **routing decides who owns acting on and
   acknowledging a result, never who may see it.** A dangerous release-gating policy may be configured;
   it can never be a load-bearing part of the architecture.

6. **Acknowledgment is a single explicit human confirm, recorded as an append-only audit event**
   (`{who, when, action-taken?}`). It is **never auto-satisfied** for the hard-ack class. Closed-loop
   read-back (repeat-the-value) is left to UI/policy on top of the confirm. Whose acknowledgment
   *discharges* the follow-up obligation versus merely records a view is policy (ruling 4).

7. **Escalation ladder, never a dead-end.** A hard-ack notification not acknowledged within its policy
   window re-routes down the responsible-set projection (orderer → covering → on-call → the patient's
   current care-context holder, [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage))
   and bottoms out in a determinate, reachable human — **never a silent drop.** This is the
   severity-ladder motif recurring a fourth time (erasure ladder → disclosure-coarsening ladder →
   auth-resilience ladder → now the escalation ladder).

8. **Safety floor: filtering changes a notification's modality, never extinguishes a mandatory-ack
   one** — the direct mirror of [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)'s
   *"secrecy blurs the safety signal, never extinguishes it."* Noise reduction can demote routine traffic
   to a digest; it can never suppress the hard-ack class out of existence.

9. **Partition-honest inbox.** A notification is a local projection over locally-available events; a
   trigger may still be on another node. So *"all caught up / inbox zero"* is never claimed across a
   partition — acknowledged uncertainty applied to notifications, which is just
   [§6.2](../sync.md#62-consistency-model) honest-assembly-state for the inbox. The honest ceiling, echoing
   the erasure ceiling: *"to this node's knowledge, you have seen everything relevant."*

10. **Mostly-pull, selectively-push** is the paper-parity-derived default. Paper was almost entirely
    *pull* (you saw it when you picked up the chart) plus a handful of *pushes* (the critical-value phone
    call, the allergy sticker). Deployed EHRs invert this to everything-push; paper-parity prescribes the
    inversion back.

11. **Mechanism, not policy (principle 9).** Cairn ships the dials, a default class→dial mapping (a
    deployment-populated **blacklist** of which classes are hard-ack / never-filterable, the same shape
    as the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
    sensitivity blacklist), the responsibility-tag overlay, the escalation ladder, acknowledgment as an
    audit event, and the inbox projection. Policy assigns classes, escalation windows, the fallback
    responsibility tags, whose-ack-discharges, and what is filterable — and may *express*, but the
    architecture will never *enforce as withholding*, an orderer-release gate (ruling 5).

## Consequences

- **Easier:** the inbox is a query, not new infrastructure; the history-arrival alert, the contamination
  cascade, the safety-projection warning, and responsibility-state surfacing all become *instances* of
  one model rather than four bespoke features. Noise reduction gets a principled, accountable boundary
  (additive demotion is free; suppression is owned), so a deployment can quieten routine traffic without
  the architecture ever permitting a critical signal to be silently dropped. The locum-heavy
  "orderer-has-left" reality is handled by the co-equal inbox + responsibility overlay rather than a
  brittle single-owner assumption, and the dangerous release-gate is demoted to an un-enforceable
  preference.
- **Harder / new trusted surface:** floor enforcement — that a hard-ack class cannot be filtered to
  nonexistence, that a present clinician is never denied sight of a result, and that escalation fires on
  non-acknowledgment — is safety-critical and belongs in the in-DB/Rust trusted surface
  ([§9 blast-radius](../language-substrate.md)); advisory salience-ranking of routine noise (ML) and the
  digest UI are fit-for-purpose. The seam — an automated filter feeding the floor that guarantees a
  hard-ack notification still escalates — is the one safety-critical path, structurally like the
  [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  seal-time projection seam and the [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
  proximity→stamp seam.
- **The bet:** that salience ≠ interruptiveness + mostly-pull + responsibility-routing actually clears
  paper-parity at ED pace without either drowning clinicians or dropping a critical value. We would know
  the bet is wrong if the digest becomes a place criticals hide, if the responsibility overlay recreates
  the team-inbox diffusion it was meant to cure, or if the ambient channel saturates into a fatigue of
  its own.
- **Policy-neutral (principle 9):** Cairn provides the dials, the responsibility-tag and escalation
  mechanisms, the acknowledgment event, and the projection; it takes no side on which classes are
  hard-ack, the escalation windows, the fallback assignments, or what may be filtered — and it
  structurally refuses the one policy (release-gating that withholds from a present clinician) that would
  violate the safety floor.
