# ADR-0051 — The contributor-role vocabulary floor and the responsibility wire shape

- **Status:** Accepted
- **Date:** 2026-07-16
- **Refines:** [ADR-0028](0028-finalized-closed-contributor-role-enum.md) (the closed contributor-role
  enum — this ADR adds one member by 0028's own extension discipline and makes "closed" a floor
  property); [ADR-0007](0007-authorship-and-accountability.md) (contributor set + separable
  responsibility); [ADR-0049](0049-commitment-based-sign-off-currency.md) (whose wire example used the
  pre-ratification shapes this ADR retires); [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
  (additive-only evolution — the partition-prefix encoding is its application to enum growth).
- **Resolves:** [#203](https://github.com/cairn-ehr/cairn-ehr/issues/203) (2026-07-15 review finding C2),
  [#96](https://github.com/cairn-ehr/cairn-ehr/issues/96) (2026-07-02 review finding B5).

## Context

Two wire windows were closing while production-shaped clinical events accumulated daily:

1. **Every orchestrator minted `role: "recorded"`** — a value in no ADR and not in the
   [ADR-0028](0028-finalized-closed-contributor-role-enum.md) closed enum (6 bearing + 5 contributory);
   ADR-0049's own Context quoted it without noticing. And no validator anywhere checked role membership,
   so "closed" was convention, not floor: `db/005` branched only on the *presence* of a `responsibility`
   key. Signed events are immutable and additive-only — a stray vocabulary can never be excluded later,
   only ratified retroactively or superseded by a second vocabulary
   ([ADR-0040](0040-signing-context-domain-separation.md) set the precedent that pre-production is
   exactly when to fix wire-freeze items).

2. **The responsibility attribute shipped as a flat string** (`"responsibility": "attested"`), while
   [spec §3.9](../data-model.md#39-authorship-and-accountability) specifies the structured
   `{held_by, on_behalf_of}` whose whole point is that the **proxy case** (`on_behalf_of` — load-bearing
   for principle 10 and [ADR-0030](0030-advisory-actor-integration-contract.md)) is expressible from day
   one, so the "AI colleague accountable as proxy for its owner" transition is *"a policy change with no
   schema migration."* On the flat string that case is inexpressible — the exact can't-retrofit gap.

3. **#96's forward-compatibility hole:** the enum is closed **and additive-only**, so a 2031 ADR adds a
   member and a 2026 node then receives it. If validation runs on the *apply* path it rejects the event
   (violates set-union losslessness); if it doesn't, the old node cannot classify the unknown member into
   the **bearing/contributory partition** that safety logic branches on — a properly-vouched note whose
   only human bearing contributor carries the unknown role would read as *un-vouched machine content* (a
   precise untruth, principle 4).

## Decision

Four parts, one theme: **the role vocabulary and the responsibility shape become floor-enforced wire
contracts, with growth encoded so that no future member can ever confuse an old node.**

1. **`recorded` is ratified as the twelfth member, contributory** — *the recording device/system that
   captured and persisted the event.* It asserts capture fidelity, adds no clinical content, and bears no
   clinical responsibility. It meets ADR-0028's bar because the consumer side must branch on it: the
   [§5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side) responsibility-state
   projection renders *device-recorded, no human vouch* (a fidelity-risk profile) differently from
   *AI-drafted/suggested content* (a generation-risk profile), and no existing member honestly describes
   the recording act (`transcribed` would import a spurious ASR-accuracy gap; `authored` claims ownership
   of the words). The recording device remains a legitimate contributor even after human attribution
   ([#204](https://github.com/cairn-ehr/cairn-ehr/issues/204)) lands — mixed sets like
   `{human, ordered} + {node, recorded}` are compositional authorship working as designed. The partition
   is now **6 bearing + 6 contributory**.

2. **Responsibility travels as an object: `{held_by, on_behalf_of?}`** (spec §3.9's shape, verbatim).
   The flat string is retired pre-production, no legacy shim. Floor bindings, extending
   [#195](https://github.com/cairn-ehr/cairn-ehr/issues/195): `held_by` must name the contributor
   entry's own `actor_id`, which in turn must name the verified attester — the chain
   `held_by = actor_id = attester` means the record can never carry a responsibility claim about a
   person who never touched the event. **`on_behalf_of` is wire-expressible from day one but refused at
   the authoring door** until a proxy-grant ADR defines how the principal's consent is verified —
   admitting it unverified would re-open #195's hole one field over (an unverifiable claim *about a
   principal*). At the **apply door it is admitted** as a signed, display-gated, unverified claim: §3.9
   promises the proxy transition with no schema migration, so a refusal there would wedge every future
   lawful proxy event out of the set-union forever (the #201 lesson). This asymmetry is deliberate:
   *author only what you can verify; admit whatever verifiably-signed future the wire brings.*

3. **Future members are partition-prefixed on the wire (#96).** The twelve ratified members travel as
   bare names. Any member a future ADR adds travels as `bearing:<name>` or `contrib:<name>` — the prefix
   is part of the canonical signed value forever, not decoration. An old node classifies an unknown
   member by its prefix; a role that is neither ratified nor prefixed classifies as **vouching-unknown**,
   a first-class honest state (principle 4) that consumers must render as *"a role this node's
   vocabulary cannot classify"* — never collapsed to "un-vouched". The prefix was chosen over a separate
   signed `bears` bit because a redundant bit can contradict its role (a new falsification surface the
   floor would have to adjudicate); a prefix cannot disagree with itself. No new spoof power is created:
   a signed `bearing:selfblessed` carries exactly the weight of a signed `authored` without attestation —
   an attributable claim by its author (principle 2); actual *vouching* still requires the verified
   token + responsibility machinery, which is role-agnostic.

4. **The floor check, one predicate, two doors** (`cairn_check_contributors`, db/005; principle 12):
   - **Submit (strict):** every contributor carries `actor_id` + `role`; every role must be in the
     ratified vocabulary — *a door only authors what it can stand behind* (prefixed future members
     included: locally unauthorable until locally ratified); the contributor set is non-empty;
     `on_behalf_of` refused.
   - **Apply (lenient):** role membership **never** rejects — set-union losslessness. Refusals are
     reserved for the **never-lawful** shapes no conformant door of any schema version could mint (the
     same refusal class as an invalid attestation token): a contributor without `actor_id`/`role`, a
     non-object responsibility, `held_by` naming another actor, and responsibility claimed on a
     non-bearing role (partitions are additive-only and never flip, so that incoherence can never
     become valid).
   - The vocabulary is floor-queryable (`contributor_role(role, bears)`, db/005) with a Rust mirror
     (`cairn-event::contributor::ROLE_VOCABULARY`) under a standing drift guard; `classify_role` is the
     one shared partition classifier for every future consumer-side projection.

**No new founding principle** (this is principles 1/4/10/12 applied to the authorship vocabulary);
**no new event type; no schema migration** (the fields existed from day one — this fixes their value
contracts). Ratifying `recorded` retroactively legalises every event already minted; the responsibility
shape change is a pre-production wire fix with no data to migrate.

## Consequences

- **Easier.** "Closed enum" is now machine-checked at the only place it can be unbypassable — the
  in-DB floor. The proxy case is expressible the day policy wants it, with the admission gate (not the
  wire) as the thing that evolves. Future enum growth is safe by construction: a new member ships as one
  ADR + one `contributor_role` row + one Rust tuple + a prefixed wire value, and every older node in the
  federation classifies it correctly without an upgrade.
- **Harder / trusted surface.** `cairn_check_contributors` joins the reviewed safety floor; its
  strict/lenient split encodes a subtle contract (author-vs-admit) that future maintainers must not
  "simplify" into symmetry — the doc comment carries the warning. The three-way vocabulary lockstep
  (SQL table / Rust mirror / spec prose) is held by a drift-guard test, the same mechanism-class the
  twin registry uses.
- **The bet.** That the recording act plus ADR-0028's eleven cover authorship until real clinical
  workflows demand more, and that partition-prefixing is enough context for an old node to handle any
  future member safely. We would know it is wrong if a future member's safety semantics exceed its
  partition (something an old node must branch on beyond bears/doesn't) — which is the signal for a
  superseding ADR on vocabulary versioning, never for informal widening.
- **Interim honestly named:** with #204 unscheduled at the time of the triggering review, every
  clinical event's only contributor is the recording node — `recorded` makes that reading *accurate*
  (device-recorded, no human authorship claimed) rather than *illegal*. The attribution-token /
  authoring-human slice is now scheduled (ROADMAP) so the interim ends before the next clinical stream.
