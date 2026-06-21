# ADR-0030 — The advisory-actor integration contract: L2/L3 attachment and authorship through the in-DB floor

- **Status:** Accepted
- **Date:** 2026-06-21
- **Refines:** [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md), [ADR-0022](0022-validated-submit-surface-the-write-path.md)

## Context

The [ecosystem evaluation of kastellan/localmail](../../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) concluded that an external advisory agent fits Cairn as an **L2/L3 component** ([ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)) that consumes the native node API and writes back **only through `submit_event`** ([ADR-0022](0022-validated-submit-surface-the-write-path.md)) — the same public-API-only discipline the reference UI uses, nothing on the inter-node path. That conclusion was *evaluation*; it minted no ADR. It also left a sharp, load-bearing question unproven: is the integration contract not just *expressible* but *unbypassable* — does the in-DB floor actually hold when the agent is buggy or hostile and has direct database access?

[Spike 0002](../../spikes/0002-advisory-actor-write-contract.md) turned the narrowest load-bearing claim into a demonstration. An external advisory agent registered as an actor, authored an **additive, un-attested, provenance-anchored, recallable** clinical advisory through the validated write path, **and** the in-DB safety floor rejected every attempt a buggy or hostile agent must not be allowed to make — including with direct DB access. Its C1–C5 pass/fail table is now green (run 2026-06-21), with each rejection failing closed and carrying a legible reason and no gate loosened to make a test pass. This ADR promotes the ecosystem/0001 evaluation from *fits* to *decided integration contract*.

## Decision

The **advisory-actor integration contract** — how an external advisory actor (e.g. a triage agent, a pathology-import pipeline) attaches and authors into a Cairn node:

1. **Attachment is L2/L3; the inter-node path is untouched.** An advisory actor is an application-layer client ([ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)) that consumes the native node API. It is never on the L0 wire contract or the L1 enforcement floor — so a buggy or captured agent can produce content *wrong for its clinic* but never a wire-incompatible event (principle 12, *uniform core, plural edges*).

2. **Authorship is only ever through the validated submit surface.** The agent authors via `submit_event` ([ADR-0022](0022-validated-submit-surface-the-write-path.md)) and never writes the event log directly. The **grant floor** makes this safe *by construction* even for a caller with direct DB access: write privileges on the event log are revoked, and the agent's role is granted only `EXECUTE` on the submit door plus read on projections — ADR-0021's *"direct DB access is safe by construction"* made checkable and **confirmed** (Spike 0002 C5.4).

3. **The agent is a registered actor that authors un-vouched by construction.** It enrolls in the actor registry ([ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md), [ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md)) with its own signing identity, and contributes with a **contributory role** (e.g. `triaged`) carrying **no responsibility attribute**. "AI-generated / un-vouched" is therefore *emergent* from the contributor set ([ADR-0007](0007-authorship-and-accountability.md)) — there is no `is_ai` flag anywhere (Spike 0002 C1).

4. **Advisories are additive, never suppressing.** An advisory raises salience; it cannot foreclose another author's signal. A suppressing event authored by an un-attested actor is **refused in-DB** ([ADR-0010](0010-additive-vs-suppressing-classification.md)). Suppression — and any asserted responsibility — requires a **human attestation token** (a signed claim bound to the event's content-address) from an enrolled human actor ([ADR-0007](0007-authorship-and-accountability.md) signature ≠ attestation; the conservation-of-responsibility loop). This is also how an agent's advisory becomes acted-upon: a human attests by overlay, never the agent auto-acting on the irreversible.

5. **Contributions are provenance-anchored and recallable.** An advisory cites its source by content-addressed reference ([ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md); the reference eager, the bytes on the lazy byte tier). Contributions are bounded to an actor and skill-epoch ([ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md)) and recalled via the contamination-cascade overlay — marked, **never erased** (principle 2).

6. **The contract is enforced unbypassably at the L1 floor.** Verification, actor resolution, additive-vs-suppressing classification, the attestation gate, and provenance binding all run in-DB inside the single validated write door ([ADR-0022](0022-validated-submit-surface-the-write-path.md); the in-DB Ed25519 verify is the [ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md) move). A buggy or hostile agent **fails closed with a legible reason**, never silent corruption (Spike 0002 C5).

**Canonical home:** [language-substrate §9.8](../language-substrate.md#98-the-advisory-actor-integration-contract) (extending the §9.5 → §9.6 → §9.7 application-layer arc).

## Consequences

- **Easier:** a *fleet* of specialist advisory actors can attach to a node without bespoke trust plumbing — each is one registered actor dropping advisories through the same public floor every UI uses. Accountability routes to the deployer (`on_behalf_of`), never to a clinician who merely saw an advisory. The reference UI and any third-party agent are citizens of the same contract (anti-capture turned inward).
- **Harder:** the floor's *completeness* is the bet inherited from [ADR-0022](0022-validated-submit-surface-the-write-path.md) — a write that the closed submit surface cannot express would push an integrator toward raw access and reopen the bypass. The in-DB verify must remain reachable from the submit path ([ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md) / fat Postgres).
- **The bet:** the closed write door plus the grant floor stays complete and unbypassable as the advisory ecosystem grows. The recurring trusted-seam motif: `submit_event` is the most safety-critical code in the system, and every advisory actor leans on it.
- **Honest ceiling:** Spike 0002 validated the **rejection** half of the attestation contract — an un-attested suppressing event or un-attested asserted responsibility is refused in-DB (C2, C5). It did **not** exercise the attestation **success** path: a *valid* human attestation token, correctly bound to the event's content-address, being accepted. That handshake (and the author-side flow that produces such a token) remains to be exercised before a production advisory deployment, and is the first thing to build atop this contract.
- **Out of scope:** the agent-*action* gate (kastellan's CASSANDRA-style "no irreversible act without a human") is **complementary, not part of this contract** — it governs what an agent may *do*, while this contract governs what may *become record*. The two compose through additive, un-attested authorship (the agent raises salience; a human attests by overlay). Transport security and the agent runtime's own containment are likewise out of scope here.
- **Promotes** [ecosystem/0001](../../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) from evaluation to decision; that document remains the evaluation record.
- **No new founding principle.** This is principles 8 (*safety-critical logic in Rust/DB*), 10 (*authorship is compositional; accountability is separable*), and 12 (*uniform core, plural edges*) applied to the advisory tier.
