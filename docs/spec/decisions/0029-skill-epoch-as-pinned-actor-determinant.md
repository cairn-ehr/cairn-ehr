# ADR-0029 — Skill-epoch as a pinned determinant of an agent actor's identity

- **Status:** Accepted (refines [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md))
- **Date:** 2026-06-21
- **Refines:** [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)

## Context

[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) made an AI agent's identity **immutable and version-pinned**: the actor-UUID is determined by a closed set of objectively-recordable behavioural determinants (`vendor, model, version, weights reference, declared inference/decoding config, system-prompt/template, tool & RAG configuration, deploying node`), and any change to a pinned determinant is a **supersession** that mints a new actor-UUID with a `supersede` link. A model recall reuses the contamination cascade, selecting events by actor-UUID.

The [ecosystem evaluation of kastellan/localmail](../../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) surfaced one piece with genuine architectural weight that ADR-0011 did not name explicitly: a **crystallised skill**. A security-first agent runtime promotes a repeatedly well-rated, human-approved run into a **named, frozen protocol** — pinned before it may execute. Because the model parameters and harness are fixed, this is not weight drift; it is distilling behaviour toward determinism and reviewability. A crystallised skill is a content-addressable artifact, so its **digest is one more pinned determinant of the agent's identity** (it sits under the existing "tool & RAG configuration" slot, but is worth naming in its own right). Then *"which advisories did this agent author under skill-set v3?"* stays a first-class query, and if a *skill* — not the model — is later found wanting, the contamination cascade bounds recall to exactly that skill epoch.

The same evaluation surfaced a sibling operational seam: a **shared model-serving fabric must not silently mutate what a pinned actor claims to be**. ADR-0011 pins a *weights reference* and *inference config*; if operations upgrade the inference backend under the hood, every actor pointed at it is mis-describing itself unless that swap forces a supersession.

[Spike 0002](../../spikes/0002-advisory-actor-write-contract.md) demonstrated the mechanism rather than asserting it: an agent actor's identity (`actor_id`) **is the content-address of its pinned-determinant set** — including a `skill_epoch` field — so bumping any pinned determinant mints a new actor by construction, and the spike's C4 bounded recall to a skill epoch (`events_by_actor_epoch` + an append-only contamination overlay). This refinement is now decided on the strength of that demonstration.

## Decision

1. **The crystallised-skill digest (the "skill epoch") is a pinned determinant of an agent actor's identity.** It is added **additively** to the ADR-0011 determinant set (the registry's pinned set was always meant to carry the behavioural determinants; this names skill-epoch as a first-class one rather than folding it silently into "tool & RAG configuration"). Changing the skill epoch is a **supersession** that mints a new actor-UUID — the same closed actor-event algebra (`enroll / supersede / revoke / suspend / rotate-key`), never a mutation.

2. **The skill-promotion (crystallisation) act *is* the audited supersession event.** It records who battle-tested the skill, on what evidence, and who approved the pin — the human backstop ADR-0011 already requires for enrollment, applied to the skill-pinning act. Crystallisation gated by human approval is movement *toward* determinism; it is not drift.

3. **Recall bounds to a skill epoch.** "Which contributions did actor X author under skill epoch E?" is a first-class query, and recalling a flawed skill reuses the existing contamination cascade — select by actor-UUID (which already encodes the epoch), overlay a trust marker, **never erase** (principle 2). Demonstrated as Spike 0002 C4.

4. **The served-model version is observable and pinned per-actor.** The actor records the **model digest it actually ran against**, not merely a logical endpoint name. A backend/inference-fabric swap that changes that digest **forces a supersession**; a shared serving fabric therefore cannot silently mutate a pinned identity. This makes ADR-0011's "weights reference + inference config" determinants concrete against the reality of shared serving.

5. **Drift versus staleness, kept distinct.** Pinning kills *drift* (an actor cannot silently become a different actor). *Staleness* — a pinned skill or model that has aged out of best practice — is handled one layer up by additive overlay, human review, and re-crystallisation (a fresh, audited supersession), never by mutating the aged identity.

**Canonical home:** [security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) (a refinement within the actor-registry section); minimal invariants alongside [data-model §3.12](../data-model.md#312-actor-identity-in-the-registry).

## Consequences

- **Easier:** bounded, auditable recall of agent contributions at skill-epoch granularity; a fleet of specialist advisory actors can each carry an honest, queryable identity; a serving-fabric upgrade can no longer quietly invalidate a pinned actor's self-description.
- **Harder:** the actor-UUID derivation must canonically include the skill-epoch and served-model digests as part of the pinned set. This is **additive** to ADR-0011's set, not a breaking change — but, like all identity shape, it is cheap to get right at provisioning and expensive to retrofit, which is why the demonstration came first.
- **The bet:** the pinned determinant set stays at the right granularity — coarse enough that per-invocation variance rides on the *event* (not a new identity per call, the ADR-0011 ruling), fine enough that a flawed skill or a swapped model is a distinct, recallable identity. A future determinant that is neither pinned nor per-event would signal the granularity needs revisiting.
- **Honest ceiling:** Spike 0002 demonstrated the `actor_id = content-address(pinned set)` mechanism and skill-epoch-bounded recall (C4). It did **not** exercise served-model-digest pinning end-to-end — the stand-in's pinned set was static. Point 4 is decided here on the ecosystem-evaluation reasoning and is to be exercised when a real shared serving fabric is integrated.
- **Closes** the ecosystem/0001 §8 parked item ("skill-epoch as a pinned determinant — ratify if/when adopted") and the corresponding ADR-0011 follow-on. The trigger was Spike 0002's C1–C5 PASS (§6 exit criteria).
- **No new founding principle.** This is principle 2 (*identity is a claim; never merge/erase, always link/overlay*) applied to non-human actor identity — exactly as ADR-0011 already established — now naming two determinants that reality made load-bearing.
