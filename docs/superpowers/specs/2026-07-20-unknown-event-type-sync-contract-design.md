# Design — the sync contract for unknown event types (issue #200)

**Date:** 2026-07-20 · **Issue:** [#200](https://github.com/cairn-ehr/cairn-ehr/issues/200)
(2026-07-15 review, finding B5) · **Outcome:** ADR-0056 · **Scope:** design + spec prose +
follow-on issues; no code this session.

## 1. The problem as filed

The review recorded #200 as a *spec over-promise*: `sync.md` §6.5 says the event format evolves
"forward-compatibly" fleet-wide, while `db/020` fails closed on an event type absent from
`event_type_class`, so every new slice's events are refused by any peer that has not taken the
code-plane update. It guessed the intended design was "refusal + durable re-offer **is** the
contract," needing only an ADR-level statement.

## 2. What the code actually says (investigated 2026-07-20)

The filed framing was too kind. Three findings, all verified against the tree:

**F1 — §6.5's lossless-forwarding invariant is contradicted, not merely imprecise.** The spec
states that a node receiving an event under a newer, unseen schema "stores, re-propagates, and
exports it byte-for-byte — never rejecting, dropping, down-converting, or re-serializing it."
`db/020_apply_remote_event.sql:163-167` refuses an unclassifiable type outright, so the event is
never stored at all. The invariant holds for unknown *fields* under a known type; for unknown
*types* it is false.

**F2 — §6.3's "durable re-offer" row is inaccurate.** It claims refused bytes "are quarantined
*verbatim* by digest." They are not. Both pens (`sync_quarantine`, `node_event_quarantine`) hold
**unverifiable** bytes only. A door refusal on *verifiable* bytes leaves no durable record: the
clinical puller sets `frozen = true` and halts its cursor (`crates/cairn-sync/src/main.rs:1697-1716`),
persisting nothing. `crates/cairn-sync/tests/clinical_pull.rs:766-769` asserts this explicitly
(`penned == 0`, "the pen is for unverifiable bytes").

**F3 — the two planes diverge, and the clinical one fails quietly.** The node plane treats a
verifiable-but-refused event as skip-and-advance, re-offered only on the 10-cycle full sweep
(`crates/cairn-node/src/sync.rs:680-685`). The clinical plane freezes instead — and the pull still
**exits success**, because `PullIntegrityError` fires only on `skipped_unverifiable > 0 ||
pen_refused.is_some()` (`main.rs:1834`), never on a freeze. So one unclassifiable event from an
upgraded peer wedges the entire clinical pull from that peer, accumulating a silent backlog with
only a stderr line as evidence.

**The safety consequence that decided the design.** Under fail-closed refusal, a phone-tier node
carrying a chart between two upgraded facilities (§6.1 sneakernet, the "carry the chart with the
patient" promise) acquires **nothing** past the first unknown-type event. A future
`clinical.medication.recall` would be absent from that chart — not unrenderable, *absent*. It is
not a rendering limitation: `cairn_twin_skeleton` already gives every type a mechanical twin.

## 3. What the change actually costs

Smaller than expected. Everything in the remote door except one line is already type-independent:

| Concern | Status | Evidence |
|---|---|---|
| Re-propagation | free — `serve` reads `event_log` unconditionally | `main.rs:2634` |
| Sealed-scope | already type-independent (`clinical.%` string prefix) | `db/005_submit.sql:662,679` |
| Twin rendering | already degrades for unregistered types | `db/005_submit.sql:96-119` |
| Classification | **the only fail-closed line** | `db/020:163-167` |

## 4. Options considered

**(a) Custody total, power withheld — CHOSEN.** Admit the unclassifiable event's content, withhold
its power until the node has classifying code. Makes §6.5's invariant true, ends the freeze, keeps
"never guess the mode."

**(b) Ratify refusal + durable re-offer.** Keep fail-closed; narrow §6.5 to fields only; fix the pen
so a door refusal is genuinely durable. Cheapest, but leaves a carrier node a propagation barrier —
which is precisely the failure that matters most in the partition cases Cairn is built for.

**(c) Split by plane.** Content plane custody-total, node/actor plane fail-closed. Rejected as a
second contract to reason about; the node plane's own divergence is a defect to fix, not a design
to ratify.

## 5. The design (→ ADR-0056)

**(a) An unknown `event_type` is not a refusal.** The remote door admits it: stored verbatim in
`event_log`, re-propagated, exported, rendered down the twin ladder via the existing skeleton
fallback. It yields no projection rows and confers no power. The **strict door keeps failing
closed** — a node may not *author* a type it has no code for. That asymmetry is ADR-0051's
strict-submit/lenient-apply applied to types.

**(b) The floor gates effect, not presence.** Still refusing regardless of type: bad signature,
unenrolled or revoked signer, malformed envelope, `t_effective` past the HLC ceiling, sealed-scope
violation, never-lawful contributor shapes. Moot for an unclassified event: the
suppressing⇒attestation gate, since suppressing power is withheld anyway.

**(c) Where refusal genuinely remains, the contract is refusal + durable re-offer** — #200's
original claim, kept as the *residual* rule rather than the general one, and today only half-built
(F2, F3).

**Why admission is safe.** Effect is derived at projection time, not granted at admission time. On
upgrade the node reclassifies; an event that turns out to be suppressing without a valid attestation
stays powerless and is flagged legibly. "No unattested suppression" therefore holds at every
instant — it is never violated-then-repaired. And the failure direction is the safe one: an old node
shows *more* than a new one, which is what paper does (a struck-through entry stays visible with its
strike).

**The posture triad.** Content plane: admits-and-disputes (ADR-0054, actor conflicts) and
admits-and-defers (ADR-0056, unknown types). Code plane: verifies-or-refuses (ADR-0055). The content
plane never refuses verifiable history; the code plane always does.

## 6. Scope boundaries

- **No wire change.** ADR-0010's derived-not-declared stands: no self-declared mode field, because a
  declaration can lie. Nothing here is can't-retrofit.
- **`event_type_class` stays migration-only** — classification remains a code-plane property.
- **DoS.** Unknown-type admission adds no exposure an enrolled peer lacks with known types; the
  honest unbounded-pen limit in §6.3 is unchanged and still stands.
- **Couples to [#208](https://github.com/cairn-ehr/cairn-ehr/issues/208).** The upgrade-then-gain-power
  path *is* a reprojection; #208's "a projection fix ships with its backfill" rule governs it.

## 7. Spec homes

- `sync.md` §6.5 — restate the lossless-forwarding invariant to cover types as well as fields; name
  the admitted-uninterpreted state.
- `sync.md` §6.3 — split the refusal row: unknown type is no longer a refusal; genuine refusals keep
  refusal + durable re-offer, with the F2/F3 gaps stated honestly as current limits.
- `data-model.md` §3.13 — the legibility ladder gains its uninterpreted rung.

## 8. Follow-on issues to file

1. Remove the `db/020` unknown-type fail-closed; admit uninterpreted, no projection rows.
2. Door refusals on verifiable bytes must pen verbatim (closes the F2 inaccuracy).
3. A frozen clinical watermark must fail loud, not exit success (F3).
4. Align the node-plane P0001 skip-and-advance with the ratified contract (F3).
5. Reclassify-on-upgrade: deferred events gain power via reprojection (couples to #208).
6. Test gap: no test covers a node-plane skipped event later healing via full sweep.
