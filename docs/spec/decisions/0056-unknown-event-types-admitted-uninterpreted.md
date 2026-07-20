# ADR-0056 — Unknown event types are admitted uninterpreted: custody is total, power is deferred

- **Status:** Accepted (refines [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)'s forward-compatibility claim and [ADR-0022](0022-validated-submit-surface-the-write-path.md)'s remote door; extends [ADR-0054](0054-actor-registry-federation-admit-and-dispute.md)'s content-plane posture from actor conflicts to unknown types; upholds [ADR-0010](0010-additive-vs-suppressing-classification.md)'s derived-not-declared rule and [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)'s strict-submit/lenient-apply asymmetry; reuses [ADR-0039](0039-globalise-authored-legibility-twin.md)'s honest-degradation twin)
- **Date:** 2026-07-20

## Context

[Sync §6.5](../sync.md#65-schema-evolution-two-planes-and-lossless-forwarding) states a **lossless
forwarding invariant**: a node receiving an event authored under a *newer, unseen* schema "stores,
re-propagates, and exports it byte-for-byte — never rejecting, dropping, down-converting, or
re-serializing it." That is the mechanism by which the two planes move independently and there is
**no lockstep fleet upgrade** ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)).

The implementation contradicts it. The classification step of `apply_remote_event`
(`db/020_apply_remote_event.sql`) fails closed on an `event_type` absent from `event_type_class` —
correct-looking for the safety floor, since additive-vs-suppressing cannot be guessed for an unknown
type ([ADR-0010](0010-additive-vs-suppressing-classification.md)), but it means the event is **never
stored at all**. The invariant holds for unknown *fields* under a known type; for unknown *types* it
is false. `event_type_class` is populated by migrations only, so every new slice's events are refused
by any peer that has not taken the code-plane update.

Two further gaps sit underneath it. [§6.3](../sync.md#63-failure-modes-designed-for) claims refused
bytes "are quarantined *verbatim* by digest"; both pens (`sync_quarantine`, `node_event_quarantine`)
hold **unverifiable** bytes only, so a door refusal on verifiable bytes persists nothing — the
clinical-pull test asserting `penned == 0` on verifying bytes
(`crates/cairn-sync/tests/clinical_pull.rs`) states exactly this. And the two planes diverge: the
node plane's skip-and-advance arm for a deliberate (`P0001`) door refusal re-offers only on the
`FULL_SWEEP_EVERY` sweep (`crates/cairn-node/src/sync.rs`), while the clinical puller's freeze arm
halts its cursor and **still exits success** — its `PullIntegrityError` condition tests only
`skipped_unverifiable` and `pen_refused`, never `frozen` (`crates/cairn-sync/src/main.rs`). One
unclassifiable event from an upgraded peer therefore wedges the whole clinical pull from that peer,
silently.

*(Code is cited by symbol here, not by line: this ADR is immutable, and the very line that carries
the contradiction is the one [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) deletes. The
line-level evidence as investigated sits in the dated design note under `docs/superpowers/specs/`.)*

**The failure that decided this.** Under fail-closed refusal, a phone-tier node carrying a chart
between two upgraded facilities — the [§6.1](../sync.md#61-mechanism) sneakernet "carry the chart
with the patient" path, paper-parity-exact and the case Cairn exists for — acquires **nothing** past
the first unknown-type event. A future `clinical.medication.recall` is not merely unrendered in that
chart; it is absent. This is not a rendering limitation: `cairn_event_twin` already falls through to
`cairn_twin_skeleton` (`db/005_submit.sql`), which yields a mechanical twin for *any* type,
registered or not.

The 2026-07-15 review filed this as finding B5 / issue [#200](https://github.com/cairn-ehr/cairn-ehr/issues/200),
supposing the intended design was "refusal + durable re-offer **is** the contract." It is not — that
is only the residual rule.

## Decision

**1. An unknown `event_type` is not a refusal — admit-and-defer.** The remote door admits a
verifiable event of an unclassifiable type: stored verbatim in `event_log`, re-propagated to peers,
exported, and rendered down the [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
legibility ladder via the existing skeleton twin. It yields **no projection rows** and confers **no
power**. *Coarseness varies; existence never disappears* — now true of types, not only of fields.

**2. The strict door keeps failing closed.** `submit_event` still refuses an unclassifiable type: a
node may not **author** a type it has no code for, only **carry** one. This is
[ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)'s
strict-submit/lenient-apply asymmetry applied to types, and it keeps classification an honest
code-plane property rather than something a writer can invent at runtime.

**3. The floor gates effect, not presence.** For a type-unknown event:

- **The remote door still refuses, regardless of type:** invalid signature; unenrolled or revoked
  signer; malformed envelope; oversize beyond the admission ceiling; `t_effective` after the
  `t_recorded` HLC ceiling ([ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)
  tier-1); never-lawful contributor shapes
  ([ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)). Each is
  decidable from the envelope alone, with no reference to the type's classification.
- **Not a remote-door refusal — strict door only:** the born-sealed scope rule
  ([ADR-0052](0052-born-sealed-clinical-bodies.md)). `submit_event` refuses a sealed non-`clinical.*`
  body and an unopenable sealed body; `apply_remote_event` deliberately does **not** mirror either,
  because a refusal there would freeze the watermark on a verifiable event — the same reasoning this
  ADR generalises. A foreign or pre-ADR-0052 plaintext clinical body is likewise admitted. Custody,
  not admission, is what a missing DEK withholds.
- **Moot:** the suppressing⇒attestation gate, because suppressing power is withheld anyway. An
  unclassified event cannot suppress, so it cannot suppress unattested.

**4. Power is granted at reclassification, never retroactively assumed.** When the node takes the
code-plane update that classifies the type, it **re-runs the floor checks that classification gates,
and only then reprojects.** The order is load-bearing and the checks are not optional: admitting an
event uninterpreted necessarily *skips* every refusal derived from its mode or its target
relationship — the attestation gate, the overlay-target-exists check, and the ADR-0043 cross-author
suppression refusal among them. Those are deferred with the interpretation, not waived by it, so
reclassification is **re-adjudication first, backfill second**; a reprojection that only rebuilt
rows would grant power that never passed the gate. An event that then proves to be **suppressing
without a valid attestation**, or to target an event this node does not hold, stays powerless and is
flagged legibly; it is never silently promoted. So *no unattested suppression* holds **at every
instant** — it is never violated-then-repaired. This is the
[#208](https://github.com/cairn-ehr/cairn-ehr/issues/208) reprojection mechanism doing its ordinary
job, with the deferred gates run ahead of it: a classification fix ships with its backfill.

Corollary for implementations: the deferred state must be **explicit**, never inferred from a null
classification lookup falling through the gates by three-valued logic. A node records that an event
was admitted uninterpreted, and that marker — not the absence of a row — is what reclassification
consumes.

**5. Where refusal genuinely remains, the contract is refusal + durable re-offer.** For the point-3
refusals: the bytes are penned verbatim by digest, the refusal is answered legibly, and the
watermark does not advance past an unresolved refusal, so an upgraded floor later admits what an
older one refused. The single deliberate exception stays an operator **ack** — a recorded human
decision, never an automatic drop. This is #200's original statement, kept as the **residual** rule
rather than the general one, and it is today only half-built (§6.3 overstates the pen's coverage;
the clinical freeze is silent) — see the follow-ons below.

**6. The failure direction is the safe one.** Admission cannot hide anything; refusal can. An old
node showing *more* than a new one is what paper does — a struck-through entry stays visible with
its strike ([principle 3](../index.md#founding-principles-the-lens-for-every-decision)) — and a
clinician reading a twin they cannot fully parse is strictly better served than one whose chart
never received the event ([principle 5](../index.md#founding-principles-the-lens-for-every-decision),
availability over consistency).

**7. No wire change; no self-declared mode.** [ADR-0010](0010-additive-vs-suppressing-classification.md)'s
derived-not-declared rule stands: the envelope gains **no** field declaring additive-vs-suppressing,
because a declaration can lie and the floor would then be trusting the writer it exists to bound.
Classification stays derived from code the node holds. Nothing in this ADR is can't-retrofit.

## Consequences

- **§6.5's invariant becomes true as written** — for types as well as fields. The spec stops
  over-promising by the code catching up, not by the promise shrinking.
- **A carrier node stops being a propagation barrier.** Store-and-forward and sneakernet
  ([§6.1](../sync.md#61-mechanism)) work across version skew, which is the partition case the
  topology is built for.
- **The posture triad completes.** Content plane: **admits-and-disputes**
  ([ADR-0054](0054-actor-registry-federation-admit-and-dispute.md), actor conflicts) and
  **admits-and-defers** (this ADR, unknown types). Code plane: **verifies-or-refuses**
  ([ADR-0055](0055-distribution-trust-root-governance-chained-root-document.md)). The content plane
  never refuses verifiable history; the code plane always does. Same corpus, opposite postures, for
  the same reason: content withheld is a safety failure, code admitted is a compromise.
- **`event_type_class` stays migration-only.** Classification remains a code-plane property
  travelling the [§7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)
  distribution plane under [ADR-0055](0055-distribution-trust-root-governance-chained-root-document.md).
- **DoS posture unchanged.** Admitting unknown types gives an enrolled-but-hostile peer no exposure
  it lacks with known types; the honest unbounded-pen limit in §6.3 stands, and a cap or expiry
  remains a policy rung on the same mechanism
  ([principle 9](../index.md#founding-principles-the-lens-for-every-decision)).
- **Honest current limits (follow-ons, not decisions).** The remote door still fail-closes today;
  door refusals are not yet penned; a frozen clinical watermark still exits success; the node plane
  still skips-and-advances. Filed as [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265)–[#270](https://github.com/cairn-ehr/cairn-ehr/issues/270).

## Alternatives considered

- **Ratify refusal + durable re-offer as the general contract** (the shape #200 supposed): keep
  fail-closed, narrow §6.5 to fields only, and make the pen genuinely durable. Cheapest, and it
  would have been honest — but it ratifies the carrier-node propagation barrier, and it narrows a
  promise that the architecture actually wants to keep. Rejected: the spec was right and the code
  was wrong, not the reverse.
- **Split the contract by plane** — content plane custody-total, node/actor plane fail-closed.
  Rejected: the node plane's divergence is a defect to fix, not a design to ratify, and a second
  contract is a second thing to reason about at every door.
- **A self-declared mode field in the envelope**, so an old node can classify a new type from the
  wire. Rejected under [ADR-0010](0010-additive-vs-suppressing-classification.md): a writer that can
  declare its own mode can declare itself additive to evade the attestation gate.
- **Guess the mode from the type-name prefix.** Rejected: a naming convention is not a safety floor,
  and the first type that breaks the convention breaks it silently.
