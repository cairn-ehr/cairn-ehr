# ADR-0054 — Actor-registry federation: admit-and-dispute, the signed actor-event wire shape, and the adjudication algebra

- **Status:** Accepted (refines [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md), [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md), [ADR-0046](0046-enroll-fail-closed-on-key-actor-dual-mapping.md); applies [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)/[ADR-0040](0040-signing-context-domain-separation.md)/[ADR-0045](0045-collation-independent-projection-tiebreaks.md)/[ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)/[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md)/[ADR-0027](0027-trusted-time-anchoring.md); the interim-attribution surface extends [ADR-0007](0007-authorship-and-accountability.md)'s consumer side)
- **Date:** 2026-07-19

## Context

[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) §4 promises **partition-safe
enrolment** — "enroll locally during an outage; the event syncs upstream like any other."
[ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md)/[ADR-0046](0046-enroll-fail-closed-on-key-actor-dual-mapping.md)
then made `enroll_actor` **fail closed bidirectionally** (≤ 1 key per `actor_id`, ≤ 1 `actor_id` per
key, whole-history). Two partitioned nodes can each locally-validly create what is *jointly* a
violation:

- **A-direction:** two different people whose pinned sets are degenerately identical → the same
  `actor_id` under two keys (each node saw only one enrolment, so neither refusal fired).
- **B-direction:** one signing key under two `actor_id`s — most commonly **one clinician enrolled at
  two facilities during a partition** whose nodes minted slightly divergent pinned sets. (Identical
  pinned sets compute the *same* `actor_id` and dedupe harmlessly; only divergence conflicts.)

On reconnect, set-union delivers both signed events. A sync-apply door that mirrors the enroll
checks as refusals **refuses signed history** — violating lossless custody
([sync §6.3/§6.5](../sync.md#63-failure-modes-designed-for), "never reject, never drop") — while
accepting both into today's `actor_current` (`DISTINCT ON (actor_id)`, latest wins) **recreates the
silent merge / attribution loss** the guards exist to prevent. No prior ADR specified the
resolution (the 2026-07-15 review, finding C4 / issue #205). Meanwhile the attestation and
suppression owner-gates trust only the **local** registry (issue #154), so a human's vouch is not
portable across nodes at all — and everything accountability-shaped now routes through
`actor_current`.

Two facts sharpen the design. First, the **threat model is benign-dominant**: the common conflict
producer is provisioning sloppiness or the locum-at-two-sites case; hostile key-reuse is the rare
tail. So the mechanism must make the benign case cheap to adjudicate while **never auto-resolving
either case** (the [ADR-0014](0014-locale-pluggable-matcher-comparators.md) posture: uncertainty may
only *withhold*; a hard conflict forces a human decision, never an auto-pick). Second, today's
`actor_event` rows are **not wire events at all** — no signature, no content address, no HLC, no
origin node — and `actor_current` orders by node-local `(recorded_at, seq)`, which cannot converge
across nodes *even without conflicts*. The wire shape is therefore a day-one, can't-retrofit
obligation of this ADR (the [ADR-0042](0042-concrete-attachment-reference-shape.md) pattern).

## Decision

**1. The actor event becomes a first-class signed wire event (day-one shape).**
COSE_Sign1 ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)) under a
**new dedicated signing context** for the actor plane
([ADR-0040](0040-signing-context-domain-separation.md) domain separation — registry bytes can never
replay as clinical or node events). **Signer = the enrolling node** (the ceremony authority — the
[ADR-0053](0053-per-write-human-authorship.md) `{node, recorded}` posture); the enrollee's public
key, pinned set, any human co-signature, and the [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md)
person-distinguishing determinant are *content*. **Content-addressed** (multihash of the signed
bytes — the wire dedupe/idempotence key). **HLC-stamped** (`t_recorded`, graded per
[ADR-0027](0027-trusted-time-anchoring.md)) with origin `node_id`. The cross-node winner order for
`actor_current` becomes **`(HLC, content_address)`** — deterministic, clock-collision-proof,
collation-independent ([ADR-0045](0045-collation-independent-projection-tiebreaks.md); this
winner-rule change is itself ADR-gated per [sync §6.1](../sync.md#61-mechanism), and this is that
ADR). Local `seq` survives only as the intra-node tiebreak for pre-wire local rows. The `actor_id`
derivation (content-address of the pinned set alone) is **unchanged**. **Pre-wire unsigned rows
never sync** — the apply door refuses unsigned registry history; dev/PoC rigs are wiped, never
synced through (the [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)/[ADR-0052](0052-born-sealed-clinical-bodies.md)
posture; we are pre-clinical, so this is stated discipline, not migration).

**2. Transport: the actor stream rides the node plane.**
Deny-all, trusted-peer admission ([ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)):
**registry trust is node trust** — a node-signed enrolment is inherited exactly as far as the peer
is trusted, and a peer that turns out bad is answered by the
[ADR-0018](0018-federation-revocation-cascade-and-the-anchor-as-power.md) revocation cascade. The
registry is small trust-plane state, so it replicates **fully within the trust neighborhood** —
which is what makes a human's vouch portable. Per-peer seq cursor and the existing node-plane
quarantine pen (re-offer floor, `acked` human exclusion — `db/022`'s contract) are reused verbatim
for unverifiable bytes.

**3. The apply door admits, then detects — never refuses verifiable history.**
`apply_remote_actor_event`: verify signature + signing context (unverifiable → pen) → origin
admission (untrusted → skip-and-sweep: not applied now, revisited by a background sweep when peer
admission changes — never silently dropped) → content-address dedupe (idempotent) → **INSERT
unconditionally** (custody is total) → re-evaluate the **derived dispute state** (point 4) and
raise a salience-routed worklist item ([identity §5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor))
on a newly-disputed `actor_id`/key — at *both* implicated nodes, never a modal (principle 3). The
local `enroll_actor` door keeps its fail-closed whole-history stance **unchanged**; loud-at-creation
remains the first line. Admission ≠ endorsement: safety lives in the projection.

**4. Disputed is derived, never declared — and computed over live bindings.**
An `actor_id`/key is *disputed* iff the conflict predicates detect a violation among **live**
bindings in the local log — a binding is live until a later algebra event closes it (`supersede`,
`revoke`, or `rotate-key` rotating the key away; without the rotate-key closer, every routine
rotation would read as a false two-key dispute): ≥ 2 live keys under one `actor_id`, or
≥ 2 live `actor_id`s under one key. No event creates or clears the dispute; adjudication changes
the *log* the state derives from ([ADR-0010](0010-additive-vs-suppressing-classification.md)
derived-not-declared). Consequence: **log convergence ⇒ state convergence** — no dispute flag to
race, no "who noticed first." Projection surface: `actor_current` stops picking winners for a
disputed `actor_id`; a sibling projection (`actor_registry_state`) carries
`clean | disputed(candidate_set) | revoked` per `actor_id` and per key, enumerating the conflicting
rows. **The predicate split is deliberate and load-bearing:** the *derived state* uses live
bindings so adjudication clears it by construction; the *enroll-door* predicates stay whole-history
(anti-resurrection, fail-closed). Without the split, the A-case repair deadlocks — re-enrolling a
key under its proper successor would trip the whole-history B-check; `supersede` is the sanctioned
repair path the doors recognize.

**5. Content never waits; permissions always wait.**
No strict cross-plane ordering is promised (registry and clinical streams have independent
cursors; promising more would lie at the first partial sync). Instead, at the consuming doors:
*(a)* a clinical event citing a key the local registry doesn't know yet, or one in dispute,
**applies normally** — availability over consistency — with attribution honestly degraded: unknown
key → "key not yet resolved," disputed key → the **candidate set** (*"one of Dr X / Dr Y — registry
dispute pending"*), a first-class [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
authorship state (db/005's `NULL`-on-ambiguity survives only as the fallback the candidate set
cannot express). *(b)* An operation where the registry **grants authority** — a suppression's
owner-gate, an attestation's authority check — is **penned** (delayed-never-lost, re-offered) until
the cited registry state arrives and is clean; disputed = not clean = withheld. Fail-safe by
direction: a held suppression keeps the note *visible*; a held attestation reads *un-vouched* —
never the reverse. A hostile fabricated enrolment therefore buys a worklist entry and **zero
authority**. This rule — *registry uncertainty may withhold a permission, never withhold content* —
is principle 4 operationalized, and it closes issue #154 structurally. Attribution re-derives
retroactively and losslessly the moment the registry catches up or the dispute resolves; the
dispute window stays auditable in the log forever. (In the A-direction, per-event attribution is
unambiguous *by signing key* the whole time — only the registry identity of each key is disputed —
so the candidate set is temporary by construction; genuine permanent ambiguity exists only in the
stolen-key tail, resolved by `revoke` + the [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)
§6 contamination cascade, unchanged.)

**6. Adjudication is `supersede` by audited human ceremony — no new verbs.**

- **B-case (join):** two `supersede` events, each old `actor_id` → one successor (corrected pinned
  set, proper determinants). A join expressed as links — never a merge; both lineages survive.
- **A-case (per-key fork):** the degenerate binding is superseded per key; each person gets a
  successor `actor_id` (distinguishing determinants added) binding their key. Attribution then
  re-derives exactly, per signing key. (The supersede content therefore cites the **specific
  enrolment binding** it closes, never an `actor_id` wholesale — an additive wire-shape field,
  canonical in [data-model §3.12](../data-model.md#312-actor-identity-in-the-registry).)
- **Hostile tail:** `revoke` with compromise-time + contamination cascade.

The ceremony is audited with a **mandatory recorded human adjudicator** (the
[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) §4 backstop); *who* may
adjudicate is policy (principle 9). Adjudication events sync as ordinary actor events. If two nodes
adjudicate the same dispute **differently** during another partition, the live-binding derivation
sees conflicting supersedes and the state honestly reads **disputed again**, worklist escalated —
never auto-picked. Rare, honest, convergent.

**7. Considered and rejected.**

- **Pen-outside + adjudicated admission** (mirror the checks as refusals into a quarantine pen):
  registries *diverge* until adjudication — each node holds its own enrolment and pens the other's —
  so the conflict has no convergent representation; the adjudication event must cite bytes outside
  the log; candidate-set attribution cannot be computed legibly from a pen row; and the pen was
  built for *unverifiable* bytes, not unwelcome verifiable history — stretching it there is exactly
  the "reject signed history" the sync contract forbids.
- **Deterministic tiebreak + auto-supersede** (HLC/content-address picks a winner): an **automated
  identity resolution** — the move principle 2 and [ADR-0014](0014-locale-pluggable-matcher-comparators.md)
  forbid. A tiebreak that silently hands Dr X's authorship to Dr Y because of a clock value is the
  failure mode this project exists to refuse.

**Canonical homes:** [security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)
(registry federation + dispute + adjudication), [sync §6.9](../sync.md#69-the-actor-registry-stream)
(the actor stream + ordering contract), [data-model §3.12](../data-model.md#312-actor-identity-in-the-registry)
(wire shape + convergent ordering), [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
(candidate-set authorship state).

## Consequences

- **Easier:** the C4 contradiction dissolves — custody is total (both signed events, every trusted
  node, byte-for-byte), "never reject" holds, and the silent merge is unreachable because no
  projection picks a winner under dispute. A human's enrolment/vouch becomes portable (closes #154
  structurally). The sync-door half of the #172 mirror obligation is discharged *by specification*
  (the door runs detection, not refusal — deliberately). Log convergence ⇒ dispute-state
  convergence, for free.
- **Harder:** `actor_current` and every accountability-shaped consumer must learn the disputed
  state — a real projection blast radius. The "≤ 1 key per `actor_id`" invariant becomes a
  steady-state *projection* claim, not a table invariant. A second predicate family (live-bindings)
  joins the whole-history one; the split must stay legible or a future door will call the wrong one.
- **Operational:** pre-wire unsigned `actor_event` rows never sync — wipe dev/PoC rigs before
  federating them. The `rotate-key`/`supersede` *local* door obligation (#172's other half) is
  restated with one qualification: it must mirror **both whole-history checks applied
  lineage-aware** — a key may re-bind only along its own supersede lineage. (Lineage-blind
  mirroring would refuse every key-preserving supersession and deadlock the point-6 repairs;
  lineage-aware, the forks and joins pass while a foreign key-grab still refuses.) The human
  key-loss recovery ceremony remains an open gap needing its own ADR before any pilot enrolls
  real humans
  ([security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody),
  issue #94); nothing here forecloses it.
- **Blast radius (§9):** the wire shape, apply door, dispute derivation, and adjudication ceremony
  are **safety-critical** (in-DB/Rust, reviewer-legible, TDD). The dispute derivation is a pure
  function of the log — property-test that **admission order never changes it** (set-union
  commutativity + idempotence over arbitrary interleavings), plus multi-node convergence
  (partition → dual enrol → reconnect → both disputed → adjudicate once → both clean).
- **The bet:** that a derived, live-bindings disputed state plus supersede-only repair absorbs
  every registry conflict a partition can mint — with no new verbs and no auto-resolution. We would
  know it is wrong if a real conflict class cannot be expressed as live-binding supersedes, if the
  dispute state flaps pathologically under normal provisioning, or if withheld-while-disputed
  permissions block a clinically-urgent workflow (which would indicate the permission, not the
  dispute, is mis-scoped).
- **No new founding principle.** This is principles 1, 2, and 4 applied to the actor registry:
  append-only custody, never-merge/always-link, and acknowledged uncertainty — now at the trust
  anchor itself.
