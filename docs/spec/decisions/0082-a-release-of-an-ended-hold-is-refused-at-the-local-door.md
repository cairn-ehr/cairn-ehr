# ADR-0082 — A release of an ended hold is refused at the local door

- **Status:** Accepted
- **Date:** 2026-10-23
- **Spec version at acceptance:** 0.84
- **Issues:** [#843](https://github.com/cairn-ehr/cairn-ehr/issues/843) (found in the review of PR #842, #840) — a
  maintainer decision of 2026-10-23
- **Amends:** [ADR-0081](0081-medication-withhold-is-a-time-bounded-hold.md) decision 4's sentence *"A named hold
  whose span already ended is a no-op, not an error"*, at the LOCAL door only, and adds one entry to decision 8's
  list of clinical-state refusals. Everything else in ADR-0081 stands, including that sentence at sync apply.

## Context

ADR-0081 made a release of a hold that had already ended a no-op: the read-time fold ignores it, so the record stays
correct. But the event is still written. A clinician whose screen still showed a W a colleague had just released, or
one that had run out, wrote a second release, and the record then showed two clinicians "releasing" the same W with
nothing to say which of them acted on a stale screen. It also meant the window's rule that a stale refusal wins (#840)
could never fire for a release, because no node refusal took that path.

A cease of a line that is already ceased raises the same question. Its answer is different: the second stop carries
its own reason (an adverse reaction, say), and while Cairn has no allergy stream that reason may be the only record of
it.

## Decision

1. **At the local door, a release naming a hold that has ended by the release's `t_effective` is refused.** Ended
   means a release already ended it at or before that instant, or its `until` has passed (an empty span counts as
   ended). A hold that has not started yet has not ended: releasing it still cancels it (ADR-0081 decision 4).
2. **The refusal is a stale picture**, not an input error: the caller's screen is behind the record, and re-reading
   is the whole remedy. The node refuses before signing, and says why for each hold ("was already released at …",
   "already ended at …").
3. **Sync apply and replay admit it unchanged.** A peer's release of an ended hold is valid on the peer, and a
   node-local veto would fork the event set. The fold makes it a no-op, as before.
4. **A cease of an already-ceased line stays admitted at both doors.** The reference window, which reads the line
   when the Stop is pressed, says the line was already stopped, with the earlier stop's reason, so the new stop is not
   read as the one that stopped it. The CLI's `medication-cease` does not read the line before writing and does not
   say it yet ([#854](https://github.com/cairn-ehr/cairn-ehr/issues/854)).

## Consequences

- One predicate (db/059's `cairn_medication_hold_ended`) answers "has this hold ended at this instant". The
  read-time fold, the floor's refusal and the node's pre-check all call it, so a hold the floor calls ended is never
  still shown. The pre-check asks it only about holds on the release's own thread; a hold id on another thread is
  left to the floor's wrong-thread refusal, which is not stale.
- Two narrow races remain and are accepted for now
  ([#855](https://github.com/cairn-ehr/cairn-ehr/issues/855)): a colleague's release landing between the node's
  pre-check and the floor gets an unscoped refusal for that one press, and two local releases of one hold at the
  same moment can both be admitted, which leaves the same harmless no-op a peer's double release does.
- A release that raced a colleague's now fails, and the window re-reads instead of writing an empty event. The
  record no longer gains releases that released nothing from this node's own door; it can still receive them from
  peers, which is correct.
- The bet: no clinical workflow needs to record a release of a hold that is already over. If one appears (a
  retrospective "this was released at 14:00" written after the hold ran out), it is a new event shape, not a
  reason to reopen this rule.
