# ADR-0078 — A machine's unlink does not settle a possible duplicate

- **Status:** Accepted
- **Date:** 2026-10-08
- **Spec version at acceptance:** 0.80
- **Issues:** [#741](https://github.com/cairn-ehr/cairn-ehr/issues/741) (maintainer, 2026-10-07) ·
  [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680)
- **Supersedes:** [ADR-0076](0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)'s
  skip rule wording **only** (its Consequences: "it must skip pairs already linked or unlinked"; and
  the R4 reading of decision 4, "a pair with ANY `patient_link` row has been judged").
- **Design:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (R5b)

## Context

R4's commit-time check skipped any pair with a `patient_link` row, so that a settled question is never
put back on the worklist. R5a's db/057 then ruled that, among unlinks, only an **attested** one closes a
possible duplicate (a pair that reads as one record — through any standing link — is closed too):
unlinks are not veto-gated, and the ADR-0030 agent writer can author one, so counting an
un-attested unlink would let any unreviewed writer silently clear a banner. The two rules disagreed:
the matcher's skip acts when a pair would be proposed, the view when a proposal is read. A pair
whose standing row was an un-attested unlink — an agent's, or any unreviewed writer's — was never
proposed, so it never reached the banner. No human had judged it, and a drug on the other chart
stayed invisible to the prescriber.

## Decision

1. **A pair is judged — and the matcher never proposes it — only when** its two charts read as one
   record (`person_member`), **or** `patient_link` holds an **attested** row for it (a human's link
   or unlink). This is db/057's openness rule (equivalent except where db/018's clamp-and-flag leaves
   `person_member` stale on an oversized component — there the matcher skips while db/057 keeps the
   pair open, so it is still shown), and a drift test pins the two together.
2. **A pair whose standing row is an un-attested unlink is eligible to be proposed like any other,
   and is never auto-linked over a standing un-attested unlink.** `auto_apply` moves the proposal to
   `review` and writes no event and no `patient_link` row. Otherwise a matcher link would overrule
   the agent's unlink by HLC, which is one machine overruling another where only a human may decide.
   An un-attested unlink that commits after auto-apply's check, or arrives later by sync, can still
   lose to an earlier-applied matcher link by HLC order; the pair then reads as one record, and
   R1b's doubted-link rule — not this ADR — governs what the clinician sees.
3. **The dispute is shown, not hidden.** The banner and the worklist say the pair is "recorded as not
   the same person, without a clinician's confirmation on record here" (principle 4).

## Consequences

- Every pair the matcher scores at or above the review floor and that no human has judged — and
  that does not already read as one record — reaches a human; a pair joined by an UN-attested link
  is R1b's doubt to show, not this rule's.
- An agent's unlink now produces work for a human instead of suppressing it. That is the cost, and it
  is accepted.
- No wire change, no schema change, no new event type.

## Rejected

- **Count an un-attested unlink as judged in db/057 too.** That makes the two rules agree by letting
  any unreviewed writer clear a banner, which is the hazard db/057 exists to close.
- **Auto-apply over an un-attested unlink.** The overlay would then decide between two machine
  assertions by HLC.
