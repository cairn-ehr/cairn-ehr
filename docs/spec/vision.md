# 1. Vision & Design Goals

> [!IMPORTANT]
> **0. Paper-Parity Principle (governing law).** No clinical workflow may be slower, more
> difficult, more cognitively demanding — or impossible — compared to its paper-record equivalent.
> Clinician resistance to computerized records is rooted in exactly such regressions. See
> [§1.2](#12-the-paper-parity-test-normative) for the normative form. Sole exclusions: capabilities
> of paper that constitute malfeasance (silent falsification, untraceable backdating, removable
> pages), and friction legally mandated regardless of medium — which must still cost no more than
> its paper equivalent.

1. **Availability over consistency.** During a network partition, a clinician must always be able to read the locally relevant record set and write new clinical data. We explicitly accept eventual consistency (AP in CAP terms) and design the data model so that this is clinically safe.
2. **Fractal topology.** The same software stack runs at every tier — national/regional hub, hospital, department, single practice, individual workstation. A node's role is configuration, not a different product.
3. **Context-scoped replication.** Each node holds the subset of records relevant to its local context ("sync scope").
4. **Resource-proportional deployment.** Runs on Raspberry-Pi-class hardware with intermittent power and 2G/3G-grade connectivity, and scales to a tertiary-care cluster — same codebase.
5. **Vendor independence.** No proprietary services, no mandatory cloud, no license keys. Commodity x86/ARM hardware, standard Linux, PostgreSQL ≥ 18.
6. **Identity is a claim, never a fact.** Patients are sometimes unidentifiable (unconscious) and sometimes deliberately misidentified. Prevention cannot be complete; therefore identity *repair* is a first-class, cheap, fast, forensically clean operation ([§5](identity.md)).
7. **Acknowledged uncertainty.** An imprecise near-truth always beats a precise untruth. The system never forces a clinician to commit data they cannot vouch for; imprecision, ranges, and an explicit *unknown* (distinct from *not-yet-asked* and *refused*) are first-class recordable values, no required field is satisfiable only by fabrication, and certainty is refined later by overlay ([§3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).

## 1.2 The Paper-Parity Test (normative)
- **Falsifiable form:** every clinical workflow must name its paper-era equivalent and benchmark against it in **time, steps, and cognitive load**. A workflow that loses to paper is a design defect and is tracked as one.
- **Honest accounting:** the benchmark measures the *lived* workflow — shared-workstation authentication, system latency under load, and interruption/resumption included. Paper's baseline (grab chart from rack, write) included no login and no lag.
- **Architectural consequence:** parity failures caused by round-trip latency are architecture defects, not UI defects. Local-first reads/writes against the node's own database ([§2](topology.md), [§6](sync.md)) are the structural answer; paper parity and offline-first are the same requirement seen from two angles.
- **Floor, not target:** the principle forbids regressions relative to paper; it never argues against digital gains (simultaneous multi-site access — paper's one unfixable flaw, legibility, search, decision support, [§5.5](identity.md#55-reattribution-one-primitive-tiered-workflows) disclosure-scope queries).
- **Design heuristic:** when a digital workflow breeds errors paper didn't (e.g., the wrong-chart misfile), first ask which physical affordance of paper suppressed the error (possession: one chart in one hand) and restore its semantics, before adding confirmations or alerts.

Existing instances of the principle already in this spec: offline-first operation (paper never had downtime), break-glass access ("pulling the chart"), Tier-1 self-correction (the strike-through, which no hospital countersigns), the append-only event log (how a paper record legally behaves), and the freedom — that paper always allowed — to record an unknown date or "~50 yo" plainly rather than fabricating precision (acknowledged uncertainty, [§3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)). *(A clinician's provisional "?diabetic" differential is also acknowledged uncertainty, but of a different kind — see [§3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types).)*

## Non-goals (for this phase)
- Billing/claims, imaging PACS, full HL7v2 interface engine — integration points, not core.
- Real-time collaborative editing (Google-Docs-style). We need conflict *safety*, not character-level merging.
- Biometric matching in the core (see [§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable) — supported as a pluggable identifier system only).

---

## 12. Guiding Principles (four sentences)

> Make the clinical data model append-only and causally ordered, so that synchronisation becomes set union plus a small, explicitly enumerated list of clinically-reasoned merge policies.

> Treat patient identity as a claim under continuous evaluation — never merge, always link; never erase, always overlay — so that every identity error, accidental or deliberate, is repairable by an auditable event.

> Prefer an acknowledged imprecise near-truth over a forced precise untruth — make uncertainty, imprecision, ranges, and an explicit "unknown" first-class recordable values rather than fields a clinician must fabricate to proceed, so the record stays honest and the matcher is never fed a confident falsehood.

> Select each component's language and substrate by the blast radius of its defects, not by team habit — pushing safety-critical logic into Rust or the database where whole error classes become unrepresentable, and optimizing those layers for reviewer-legibility, because specification and review are now the binding constraint.
