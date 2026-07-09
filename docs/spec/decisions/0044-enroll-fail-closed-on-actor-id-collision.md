# ADR-0044 — Enroll fails closed on `actor_id` collision; human actors carry a person-distinguishing determinant

- **Status:** Accepted (refines [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md), [ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md))
- **Date:** 2026-07-09

## Context

[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) fixed an actor's identity as the
content-address of its **pinned determinant set**, and [ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md)
named the skill-epoch / served-model digest as pinned determinants. In `db/004_actors.sql`,
`enroll_actor` derives `actor_id = cairn_actor_id(pinned)` accordingly. For an **AI agent** the pinned set
is rich (vendor, model, version, config, skill-epoch, deploying node), so two distinct agents naturally get
distinct `actor_id`s. But ADR-0011 explicitly gives a **human** *no behavioural-config dimension* — and did
not say what else distinguishes two humans. The minimal human pinned set (e.g. `{"role":"clinician"}`) is
therefore **identical across people**, so two different clinicians compute the **same** `actor_id`.

`actor_current` is `DISTINCT ON (actor_id) … ORDER BY (recorded_at, seq) DESC`. When two keys share an
`actor_id`, the later enrollment's key wins and the earlier key **silently drops out of `actor_current`** —
the two humans are merged into one identity, and the survivor's key can author as the merged actor, with
**no error**. This is a silent identity-merge on the trust-anchor floor: a direct violation of principle 2
(*identity is a claim; never merge — always link; never erase — always overlay*). It surfaced during the
[ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) owner-gate work (the tests hit it and
worked around it with distinct pinned tags) and was then confirmed against `db/004`.

The tempting fix — fold the signing key into `actor_id` — is **wrong**: ADR-0011 §5 mandates `rotate-key`,
under which an actor keeps the **same** `actor_id` across a key change (old publics retained for historical
verification). `actor_id` must stay **stable across key rotation**, so the key cannot be one of its
determinants. The distinguishing determinant for a human must be something *other* than the key and stable
across rotation.

## Decision

1. **`enroll_actor` fails closed on an `actor_id` collision with a distinct key.** Before inserting, it
   refuses if any existing `actor_event` row already binds the computed `actor_id` to a **different**
   `signing_key_id` — checked across the **whole history**, including revoked rows. An `actor_id` is
   **immortal**: it is never reusable by a different key, even after `revoke` (principle 2, the immortal-UUID
   rule now applied to actors). The refusal is legible and names the remedy. Idempotent re-enroll with the
   **same** key passes (re-runnable provisioning; the per-epoch matcher-actor re-enroll). The check is a
   small pure `STABLE` predicate `cairn_actor_id_key_conflict(actor_id, key)`, independently testable and
   reusable at the future actor-sync apply door.

2. **A human actor's pinned set carries a person-distinguishing determinant.** A handle / registration id
   (whatever the future §7.5 enrollment surface adopts) keeps two clinicians' `actor_id`s genuinely distinct,
   exactly as two AI agents differ by their rich pinned sets. Cairn does **not** hard-code *which* field
   (ADR-0011 keeps pinned-set **contents** as policy); the floor rejection in (1) makes a forgotten
   determinant **loud** on the second enroll rather than silently merging.

3. **This is enforcement of ADR-0011's existing intent, not a new determinant.** The `actor_id` derivation
   is unchanged; (1) closes an enforcement gap the same way
   [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) closed the suppression owner-gate gap.
   It is scoped to the one implemented write door (`enroll_actor`); there is no remote-apply door for actor
   enrollment yet. When actor-event sync lands (ADR-0011 §4 makes enrollment partition-safe / syncable), the
   same check must be mirrored at that apply door — the analogue of the
   [#154](https://github.com/cairn-ehr/cairn-ehr/issues/154) apply-door caveat.

**Canonical home:** [security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody).

## Consequences

- **Easier:** two real clinicians (or two agents) can never silently merge on the trust anchor; the failure
  is loud, auditable, and repairable by adding a distinguishing determinant — no data loss.
- **Harder:** the future human-enrollment surface must supply a person-distinguishing determinant (a small,
  cheap-at-provisioning obligation). A second write door (actor-event sync) must carry the same check.
- **The bet:** the collision check plus a person-distinguishing human determinant is the whole fix — no change
  to `actor_id` derivation, `actor_current`, or `rotate-key` (which remains the sanctioned same-actor /
  new-key path). We would revisit if a legitimate flow needs two distinct actors to share one `actor_id`
  (none is known).
- **No new founding principle.** This is principle 2 applied to the actor registry — as ADR-0011 already
  established — now enforced at the enroll door.
