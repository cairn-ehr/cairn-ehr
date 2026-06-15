# ADR-0011 — The actor registry: version-pinned immutable identity, behavioral-configuration granularity, and key custody

- **Status:** Accepted (refines [ADR-0007](0007-authorship-and-accountability.md))
- **Date:** 2026-06-15

## Context

[ADR-0007](0007-authorship-and-accountability.md) made each event's authorship a **contributor set** whose
`identity` is "a registered actor — human, AI agent (model + version + vendor + deploying node), or device,"
and [security §7.2](../security.md#72-signing-attestation-and-ai-agent-identity) committed that AI agents
are **registered cryptographic identities** whose authorship is **recall-traceable** ("which events did
agent X v2.3 author?") and whose "registry and key custody are part of the trusted base." But the registry
itself — how an actor is enrolled, how AI version-pinning and recall actually work, and how keys are held —
was deferred as the sharpest remaining ADR-0007 follow-on. Node identity (the provisioning ceremony,
[data-model §3.2](../data-model.md#32-identity-time)) and human point-of-care authentication
([identity §5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage),
[security §7.3](../security.md#73-point-of-care-authentication-possession-and-salvage)) exist, but the
durable *actor record* they authenticate against, and AI keying/version-pinning/custody, do not.

Forces:

- **Recall-traceability forbids mutable version state.** If "agent X" is edited in place from v2.3 to v2.4,
  the events still pointing at "agent X" can no longer be partitioned by the version that actually authored
  them — the recall query is destroyed. Version must be *frozen into the identity*.
- **Behavior is determined by objectively-recordable configuration, not just weights.** Under current
  technology the *same* model at different temperature / top-p / top-k / sampling, or with a different system
  prompt or tool/RAG configuration, yields distinguishably different output, and output *consistency* depends
  heavily on those settings. Humans vary too (mood, hunger, sleep deprivation), but there is **no objective
  criterion** to mint distinct entities for "happy Dr X" and "sleep-deprived Dr X," so a human stays one
  identity. The partitioning rule that reconciles both: **identity granularity tracks what is objectively
  recordable** — the same epistemics as [§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)
  (record what can be known objectively; never fabricate the rest).
- **Conservation of responsibility needs a human backstop.** Even a fully un-owned, additive AI output
  ([ADR-0010](0010-additive-vs-suppressing-classification.md)) should trace to *someone who decided this
  agent may write here.*
- **"Key custody" smears two opposite lifecycles.** A signing public must be verifiable forever; a DEK must
  be destroyable on demand ([ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)). Conflating them
  is a latent safety bug.

## Decision

Cairn keeps a **general actor registry** — append-only, version-pinned, projection-shaped, in the trusted
base. AI agents are the forcing case; humans and devices are simpler members. Canonical home:
[security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody); minimal
invariants [data-model §3.12](../data-model.md#312-actor-identity-in-the-registry).

1. **One registry, three actor kinds (human / device / AI agent).** `kind` is a discriminator, not a
   separate subsystem — and a deliberately **de-emphasizable** one: as the actor boundary blurs over time,
   the machinery that does not branch on `kind` keeps working. The contributor-set `identity`
   ([data-model §3.9](../data-model.md#39-authorship-and-accountability)) resolves against this registry; it
   is the durable record that [identity §5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
   / [§7.3](../security.md#73-point-of-care-authentication-possession-and-salvage) authenticate a human
   *against*, and that [§7.2](../security.md#72-signing-attestation-and-ai-agent-identity) signing binds to.

2. **Actor identity is immutable and version-pinned; the registry is a projection over a closed actor-event
   algebra.** The algebra is `enroll / supersede / revoke / suspend / rotate-key` — the same closed,
   append-only, syncable, auditable shape as the [identity §5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)
   patient-identity algebra. Identity is **never mutated**: a version bump mints a *new* actor-UUID with a
   `supersede` link to the prior (append-only lineage, the superseding-ADR rule applied to actors); a
   compromise is a `revoke` overlay carrying a compromise-time, **never a deletion**. *Never merge, never
   erase — always link, always overlay*, now for non-human actors too.

3. **Identity granularity tracks objectively-recordable behavioral determinants.** The AI-agent identity
   pins a tuple of everything objectively recorded that *materially determines behavior*: `vendor, model,
   version, weights reference, declared inference/decoding configuration (temperature, top-p, top-k,
   sampling), system-prompt / template, tool & RAG configuration, deploying node`. A change to any pinned
   determinant is a **supersession** (a new identity), so recall is exact. To avoid identity explosion, the
   split mirrors [§3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)'s
   objective-vs-asserted time: the identity pins the **declared standing configuration**, while any
   **per-invocation parameter variance** an agent exposes is stamped on the *event's* authorship record, not
   minted as a new identity. Both are objectively recorded, so both are queryable for recall (*"which events
   ran at temperature > 1.0?"*). A **human** actor carries no behavioral-config dimension — not because the
   variance is absent but because it is not objectively recordable; there is no honest criterion to split
   "happy Dr X" from "sleep-deprived Dr X," and inventing one would be a fabricated precision principle 4
   forbids. The rule is the same for both kinds; the inputs differ, and the rule self-adjusts as the
   boundary blurs.

4. **Enrollment is an audited ceremony with a mandatory human backstop.** Enrolling an actor mirrors node
   provisioning / mTLS enrolment ([§7](../security.md), [data-model §3.2](../data-model.md#32-identity-time)):
   it mints the UUIDv7, freezes the pinned tuple, binds the keypair, and is itself an **append-only, signed,
   audited event**. For an AI agent it **must record a named responsible human** (the deployer) — the
   *introduction-accountability* backstop that completes [ADR-0010](0010-additive-vs-suppressing-classification.md)'s
   conservation of responsibility: even a fully un-owned agent output traces to a human who decided the agent
   may write here. Whether that human bears *ongoing* responsibility for the agent's individual outputs stays
   **separable and policy** ([ADR-0007](0007-authorship-and-accountability.md): responsibility may be absent,
   held, or proxied). Enrollment is partition-safe (enroll locally during an outage; the event syncs upstream
   like any other); *who* may enroll, and the ceremony's strength, are policy (principle 9).

5. **Key custody, un-conflated — opposite lifecycles.** **Signing publics are immortal**: a historical
   AI-authored (or human-authored) event must remain signature-verifiable forever, so a superseded or revoked
   actor's public key **persists** — `revoke` means *distrust new events after the compromise-time*, never
   *cannot verify old ones*. **DEKs are destroyable** — the [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)
   crypto-shredding keystore, a different store with the opposite guarantee. An agent's **private signing key**
   is node-bound trusted-base custody; because *signature ≠ attestation*
   ([§7.2](../security.md#72-signing-attestation-and-ai-agent-identity)), a stolen AI signing key forges
   *origin*, not *responsibility*, and the blast radius is bounded by un-vouched-by-default output plus
   revocation plus recall. `rotate-key` overlays a new public while retaining the old for historical
   verification.

6. **A model recall reuses the contamination-cascade primitive.** Because every event names the exact pinned
   actor-UUID, a defective-model recall is *"select events authored by these agent-UUIDs"* (optionally further
   filtered by the queryable per-event config) fed into the existing
   [identity §5.5](../identity.md#55-reattribution-one-primitive-tiered-workflows) /
   [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)
   contamination-cascade and notification machinery, with a [§5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
   trust marker overlaid (*"authored by a model version later found defective"*). The affected events are
   **never erased** — re-surfaced for human review. A defective-AI recall is structurally the same operation
   as a misfiled-note contamination cascade.

7. **Blast radius (§9).** The registry projection, the actor-event algebra, and signature verification are
   **safety-critical** (in-database/Rust, beside the [§5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)
   identity algebra) — a defect could let an unregistered or forged actor author, or mis-bind a key, causing
   mis-attribution. The **agent runtime** that generates content is **fit-for-purpose** (its output is
   additive/advisory by default, [ADR-0010](0010-additive-vs-suppressing-classification.md); a defect
   produces bad *content*, caught by human review and the additive-classification gate). The seam — agent
   runtime → trusted signing/registry stamp — is the one safety-critical path, structurally like the
   [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   seal-time and [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
   proximity→stamp seams. Keep the trusted base small and reviewer-legible ([§9.2](../language-substrate.md#92-primary-quality-metric-reviewer-legibility)).

## Consequences

- **Easier:** recall-traceability becomes a trivial query because version (and behavioral config) is frozen
  into immutable identities; a defective-model recall reuses the contamination cascade with no new mechanism;
  the human-backstop closes the accountability chain so "un-owned" never means "un-traceable to any human";
  humans, devices, and AI agents share one registry, ready for the blurring boundary; and the
  signing-public-vs-DEK distinction removes a latent custody bug.
- **Harder / new trusted surface:** the registry projection, the actor-event algebra, and verification join
  the small in-DB trusted base; private signing-key custody (node-bound) is new sensitive local state;
  version-pinning on full behavioral config can proliferate identities if standing config churns (mitigated
  by the standing-config-in-identity / per-call-on-event split). Enrollment ceremonies are new operational
  surface.
- **The bet:** that freezing the objectively-recordable behavioral tuple into immutable, supersession-linked
  identities gives exact recall without identity explosion, and that the general three-kind registry ages
  well as the actor boundary blurs. We would know it is wrong if standing-config churn makes identities
  unmanageable, if real recalls need a behavioral dimension that was *not* objectively recorded (so it cannot
  be pinned or queried), or if node-bound private-key custody proves impractical on Pi-class hardware.
- **Policy-neutral (principle 9):** Cairn ships the registry, the actor-event algebra, the enrollment
  ceremony with its mandatory human-binding, and key custody; it takes no side on *who* may enroll, whether
  an AI agent may ever *hold* responsibility ([§7.2](../security.md#72-signing-attestation-and-ai-agent-identity)),
  ceremony strength, or retention.
