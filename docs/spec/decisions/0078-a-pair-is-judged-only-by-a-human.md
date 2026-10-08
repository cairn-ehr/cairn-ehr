# ADR-0078 — A pair is judged only by a human

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
put back on the worklist. R5a's db/057 then ruled that only an **attested** unlink closes a possible
duplicate: unlinks are not veto-gated, and the ADR-0030 agent writer can author one, so counting an
un-attested unlink would let any unreviewed writer silently clear a banner. The two rules disagreed
before a proposal existed. A pair whose only row was an agent's un-attested unlink was never
proposed, so it reached neither the banner nor the worklist. No human had judged it, and a drug on
the other chart stayed invisible to the prescriber.

## Decision

1. **A pair is judged — and the matcher never proposes it — only when** its two charts read as one
   record (`person_member`), **or** `patient_link` holds an **attested** row for it (a human's link
   or unlink). This is db/057's openness rule, and a drift test pins the two together.
2. **A pair whose standing row is an un-attested unlink is proposed, and never auto-linked.**
   `auto_apply` moves it to `review` and writes nothing. Otherwise a matcher link would overrule the
   agent's unlink by HLC, which is one machine overruling another where only a human may decide.
3. **The dispute is shown, not hidden.** The banner and the worklist say the pair is "recorded as not
   the same person, without a clinician's confirmation on record here" (principle 4).

## Consequences

- Every pair no human has judged reaches a human.
- An agent's unlink now produces work for a human instead of suppressing it. That is the cost, and it
  is accepted.
- No wire change, no schema change, no new event type.

## Rejected

- **Count an un-attested unlink as judged in db/057 too.** That makes the two rules agree by letting
  any unreviewed writer clear a banner, which is the hazard db/057 exists to close.
- **Auto-apply over an un-attested unlink.** The overlay would then decide between two machine
  assertions by HLC.
