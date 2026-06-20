# ADR-0027 — Trusted-time anchoring: the clock-confidence grade, the bracketed `t_recorded`, and the pluggable multi-anchor

- **Status:** Accepted
- **Date:** 2026-06-20
- **Refines:** [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md), [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md), [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)

## Context

[ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) made `t_recorded` the **objective ceiling**
on a freely-backdatable `t_effective` and the basis for causal ordering. But the HLC
([data-model §3.2](../data-model.md#32-identity-time)) gives causal *ordering* tolerant of skewed wall clocks —
**not wall-clock truth.** A node with a drifting RTC and no time sync can place `t_recorded` arbitrarily, and a
malicious or careless clock can manufacture a ceiling that looks authoritative but is fiction. The medico-legal
consequence is real: decades on, a clinician's only defence is that a record was created when it claims and not
backdated to cover a mistake — and a bare signed timestamp on a node's own say-so cannot prove that. This is the
last genuinely open architecture question (former §11.14).

The forces:

- **Offline-first is the whole point** ([principle 7](../index.md#founding-principles-the-lens-for-every-decision),
  the [ADR-0001](0001-fat-postgres-thin-daemon.md) availability floor). Any trusted-time scheme that *requires*
  a network round-trip at write time is disqualified — the genuinely solo rural node on an intermittent satellite
  link is a first-class deployment, not a degenerate one.
- **Anti-capture** ([principle 7](../index.md#founding-principles-the-lens-for-every-decision)). A single
  mandatory time authority is a trust, availability, capture, *and* privacy point — submission timing leaks
  clinic-activity patterns — and a Cairn-owned root is exactly the lock-in the mission forbids.
- **Acknowledged uncertainty** ([principle 4](../index.md#founding-principles-the-lens-for-every-decision),
  [§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)). The honest answer is
  rarely "the time is X"; it is "the time is somewhere in this interval, and here is how much I trust it." An
  imprecise near-truth beats a precise untruth.
- **Crypto-agility across decades** ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)).
  A time-attestation made today must remain verifiable, or be renewable, after its signature algorithm ages.

Three realizations collapse this into existing canon rather than a new subsystem, and one reframe avoids importing
a 2001-era assumption:

- **The reframe — two different problems hide under "trusted time."** *Clock-setting* (what time is it **now**,
  trustably? — bounds `t_recorded` from **below**, fights the drifting RTC) and *existence-proof* (prove this event
  existed by time T, tamper-evidently — bounds `t_recorded` from **above**, fights backdating). A classic
  RFC-3161 timestamping notary answers only the second. Cairn needs both, and together they **bracket**
  `t_recorded` into an interval.
- **Trusted time is [principle 4](../index.md#founding-principles-the-lens-for-every-decision) applied to
  wall-clock truth.** `t_recorded` stops being a point and becomes a **graded interval** — *literally the §3.7
  uncertainty-capable time type already in the model.* Trusted-time anchoring is just what **populates the
  bounds and the grade**; it adds no new value-typing machinery.
- **The modern existence-proof pattern is the transparency log, not the trusted notary** — and Cairn's event
  store *is already* an append-only, signed, Merkle-izable log synced by gossip (set-union). So time-attestation
  is **the same machinery pointed at time**, not a foreign appendage. You stop trusting a notary's honesty and
  instead verify inclusion/consistency proofs and gossip signed tree-heads — the
  [ADR-0014](0014-locale-pluggable-matcher-comparators.md)/[ADR-0023](0023-native-api-contract-capability-and-conformance.md)
  signed-content-addressed-registry posture, applied to time.
- **An anchor is a node role on the trust-anchor spectrum** ([ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md))
  and **re-notarization is overlay** ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)
  re-attestation-as-overlay). Neither is new mechanism.

## Decision

Specify **trusted-time anchoring** as acknowledged uncertainty applied to wall-clock truth, riding the existing
event/sync/anchor machinery. Canonical home: [data-model §3.17](../data-model.md#317-trusted-time-anchoring-the-clock-confidence-grade-and-the-bracketed-t_recorded)
(the grade, the interval, the envelope field), with the anchor/notary node role in
[security §7.11](../security.md#711-trusted-time-anchoring-the-notary-anchor-node-role) and the gossip/offline
mechanism in [sync §6.8](../sync.md#68-time-attestation-rides-the-gossip-plane). **No new founding principle** —
it is [principle 4](../index.md#founding-principles-the-lens-for-every-decision) applied to time. It adds **one
day-one, can't-retrofit requirement**: the clock-confidence grade and the `t_recorded` interval must be born on
every event from the first write.

1. **`t_recorded` is a graded interval, not a point.** It uses the existing §3.7 uncertainty-capable time type:
   a `[lower, upper]` bound plus a **clock-confidence grade**. It remains the [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)
   objective ceiling for `t_effective` (the `t_effective ≤ t_recorded` invariant now reads against the interval's
   bound), and the HLC remains the sole basis for causal **ordering**. Ordering (HLC) and wall-clock **truth**
   (this interval) are orthogonal and compose — they never collide.

2. **The clock-confidence grade is a single ordered ladder, best-corroboration-wins** (the same shape as the
   §7.1 severity, §3.13 legibility, and §3.14 retrievability ladders):

   `unknown < self-asserted (RTC) < network-synced (NTS / Roughtime) < hardware-sourced (GNSS / TPM secure clock) < externally-anchored (notary / transparency-log token) < multi-anchor-corroborated`

   A reader decades on can therefore tell whether a `t_recorded` was independently anchored or merely
   self-asserted. **`self-asserted` is the honest default** when nothing is configured: a self-signed timestamp
   proves *integrity* (the event existed in this form) but **not external time** — it must be graded
   self-asserted and **never displayed as a trusted timestamp.**

3. **Envelope floor + overlay refine.** The **initial grade and interval are a mandatory day-one envelope field**
   — a node declares, at mint, the best clock provenance it had. **Later anchor tokens are overlays that upgrade
   the grade and tighten the interval** (an async notary/log token arrives after the write), and renewal before
   algorithm obsolescence is [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)
   re-attestation-as-overlay. Envelope for the floor, overlay for the refinement — exactly how `t_recorded` is the
   immutable ceiling while certainty refines upward by overlay ([§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).

4. **Two planes, both pluggable; neither mandatory.**
   - **Clock-setting (lower bound)** — pluggable authenticated current-time sources: **NTS** (authenticated NTP,
     RFC 8915), **Roughtime** (multi-server, with built-in proof of a lying server), **GNSS/GPS** (authenticated
     UTC with no network — relevant to remote nodes where satellite uplink already implies sky view), and a
     **TPM/Secure-Enclave monotonic clock** for offline lower-bounding and ordering (composes with the
     [ADR-0026](0026-node-durability-and-disaster-recovery.md) hardware-bound keys). These bound RTC drift and set
     the lower edge.
   - **Existence-proof (upper bound)** — transparency-log-shaped, multi-anchor time-attestation over the event
     log: a node submits a hash (or, see point 6, a Merkle **root**) and receives a verifiable token bounding
     existence from above.

5. **Anchors are a node role on the [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
   spectrum — no Cairn-owned root, multi-anchor by default.** Self-signed → a practice transparency log → a
   national notary → *(named but unshipped — see Consequences)* a public-chain anchor. **RFC-3161 TSA is one
   supported anchor type** for interop with existing infrastructure. A **threshold notary via the FROST already
   earmarked in [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)** removes the
   single-notary trust/availability/capture point structurally (a quorum co-signs; no one server can backdate or
   withhold). Which anchors (if any) a deployment uses is its choice, exactly as trust anchors are elsewhere.

6. **Offline is a bracket, not a degradation — and the same machinery does it.** Because the event log is already
   gossip-synced:
   - **Peer cross-attestation gives the lower bound offline.** A received anchor (a peer's token, or a notary
     token a peer forwarded) is a **causal lower bound**: any event a node authors *after* receiving an anchor
     timestamped T has `t_recorded.lower ≥ T`. No network needed at write time.
   - **Deferred Merkle-root batch notarization gives the upper bound on reconnect.** A reconnecting node notarizes
     the **Merkle root** of a batch of pending events, not each event — which is simultaneously the **privacy
     fix**: the anchor learns only *"a batch of this size existed by T,"* never per-event clinic-activity
     metadata. The token is overlaid signed data ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)),
     and a single inclusion proof upgrades every event under the root.
   - A solo node with no anchor at all still gets an honest interval (TPM monotonic floor + RTC) graded
     `self-asserted` or `hardware-sourced`, with the HLC giving ordering. **Confidence is graded, never required**
     — the offline-first guarantee is preserved by honesty, not by blocking the write
     ([principle 4](../index.md#founding-principles-the-lens-for-every-decision)).

7. **Clock-confidence is a first-class honest-assembly fact**, like sync freshness
   ([sync §6.2](../sync.md#62-consistency-model)) and backup health
   ([§7.10](../security.md#710-node-durability-and-disaster-recovery)). A node running on a self-asserted clock,
   or one whose anchor has not been reachable, must surface that — a wide, honestly-graded interval is the truthful
   display, never a fake-precise timestamp.

## Consequences

- **Easier.** The last open architecture question closes by **reuse, not new subsystem**: the §3.7 uncertainty
  type carries the interval, the gossip plane carries the attestations, the [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
  anchor spectrum carries the notary role, the [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)
  overlay/FROST machinery carries renewal and threshold signing. Offline nodes get an honest *bracket* instead of
  a silent fiction; the medico-legal anti-backdating need is met by independently-verifiable upper bounds without
  any mandatory cloud or central index.
- **Harder / new trusted surface.** The **grade + interval envelope field**, the **best-corroboration
  composition**, and **anchor-token verification** are safety-critical
  ([§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius), Rust / in-DB, reviewer-legible): a
  forged or mis-verified time anchor corrupts the medico-legal record. The clock-setting clients
  (NTS/Roughtime/GNSS daemons), the notary/transparency-log **server** software, and anchor-submission scheduling
  are fit-for-purpose. The genuinely hard engineering is **not the protocol** — it is the anchor's own time
  provenance, its signing-key protection, and long-term token renewal before algorithm obsolescence.
- **Deliberately named but unshipped.** **Public-chain (blockchain) anchoring** (e.g. OpenTimestamps-style Merkle
  commitment to a public ledger) is the strongest no-trusted-party upper bound and fits the Merkle-root submission
  shape exactly — but it ties a health record's time-proof to a public ledger's continued existence and carries
  governance/longevity optics the mission should not adopt by default. It is recorded as a **pluggable future
  anchor type, never a default and never a dependency.** **VDFs** (proving elapsed real time) are noted and
  dismissed as overkill for this need.
- **The bet.** That a single ordered clock-confidence grade plus a bracketed `t_recorded` honestly captures
  trusted-time across the whole deployment spectrum — and that the transparency-log-over-gossip shape scales from a
  solo node to a national mesh without a central authority. We would know it is wrong if a real deployment needed a
  *legally binding precise* `t_recorded` that no available anchor could supply offline (which would be a signal to
  add a deliberate online-attestation requirement for that event class — never to fake precision on a self-asserted
  clock).
- **No new founding principle; no new event stream** (anchor tokens and grade-upgrades ride the existing overlay
  and gossip planes). One **day-one, can't-retrofit requirement**: the clock-confidence grade and the
  `t_recorded` interval are born on every event from the first write. Refines
  [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) (time), leans on
  [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md) (overlay/FROST),
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md) (anchor spectrum), and
  [ADR-0001](0001-fat-postgres-thin-daemon.md) (availability floor).
