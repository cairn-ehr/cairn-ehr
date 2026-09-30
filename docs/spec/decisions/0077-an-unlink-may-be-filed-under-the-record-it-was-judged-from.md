# ADR-0077 — An unlink may be filed under the record it was judged from

- **Status:** Accepted
- **Date:** 2026-09-30
- **Spec version at acceptance:** 0.79
- **Issues:** [#699](https://github.com/cairn-ehr/cairn-ehr/issues/699) (decision (a), maintainer,
  2026-09-28) · [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681)
- **Relates to:** [ADR-0076](0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md)
  (decisions 3–5 are this ADR's setting) · [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md)
  (db/005 step 8b, keyed on the envelope's chart) · [ADR-0053](0053-per-write-human-authorship.md)
- **Design:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (R2b-2)

## Context

R2a's `unlink_charts` refuses an unlink unless one of the two subject charts is held on this node (has a
`patient_chart` row), and files the event under a held subject. (db/005 step 8b is looser: it refuses only a
local event filed under a chart with no history here — `cairn_patient_has_events`, db/001.) #699's
scenario: chart A is held here and its record reads A, B and C through two links (A–B, B–C). B and C are
not held here (their registrations have not arrived). The far link B–C is the wrong one. The clinician
sees all three charts on A's record, judges from it, and is refused — although the record on screen is
exactly the evidence for the judgement. The tool's own advice ("unlink that link too") was untrue on this
node, because the unlink it pointed to was refused here.

## Decision

**An unlink where neither subject is held here may be filed under the chart it was judged from** — the
chart the clinician has open (`unlink-charts --from <chart>`, and the window's displayed chart) — provided
that chart is held here and its record reads **both** subjects (checked before the judgement, and again
inside its transaction after taking db/018's identity lock, before anything is signed — every identity apply
holds that lock until it commits, so a peer's unlink arriving meanwhile cannot leave the event filed under a
record that no longer holds the pair). **A `link` is never filed this way**: a link asserts two charts are one
person and needs both held, so the relaxation cannot reach it. The pair is always the event's **payload**;
the envelope's chart is only the stream the event is filed in.

The maintainer chose this (a) over the alternatives in #699 on 2026-09-28. This ADR's reading of the
choice is deliberately narrow:

- An opened chart that is not a subject must be held here and read both subjects in its record, or the unlink
  is refused — even when a held subject would carry the filing (a stray `--from` must not be silently
  ignored, and the record reported back must be one that held the pair when the clinician judged from it).
  The refusal says what it is about: a chart not held here is this node's state (sync may deliver it); a
  record that does not hold both is the picture judged from — reload the chart and judge again.
- "Still joined?" — the question that decides between *TookEffect* and *StillJoined* — is asked of the
  **subjects**: `high ∈ person_charts(low)`. R2a asked whether the subject the event was *not* filed under
  reads as part of the filed-under chart's record — the same answer whenever the filed-under chart is a
  subject, and wrong the moment it is a third chart: the answer would then turn on which subject was asked
  about, and a successful A–B–C split, judged from A, would read "still joined" whenever that subject was the
  near chart B (which stays in A's record).

## Consequences

An audit of every reader of identity events by chart, done before building (2026-09-30): every projection,
flag, trust view, heal and re-fold pass (db/018, 019, 023–025, 039, 043, 054, 055) and the plaintext twin
read the pair from the **payload**; the sync doors, page selection and the medium (db/020, db/051,
`cairn-sync`, `cairn-medium`, which carries events whole and replays them through the doors) do not read it
at all, and replication has no chart scope. No reader takes the pair from the
envelope; only two key a link event on its envelope chart — db/005 step 8b's admission check and db/048's
chart-scoped sensitivity grade — both addressed below. So:

- **No wire change, no new event type, no SQL object** (`SCHEMA_GENERATION` stays 55). The envelope's chart
  was always free to differ from the payload subjects (neither the db/018 floor nor any door checks it
  against them); no reader's correctness depended on it — only db/048's grade selection follows it (below).
- The event sits in the **opened chart's stream**. For a `RecordOf` filing `cairn_effective_sensitivity`
  (db/048) keys chart-scoped grades on the envelope, so the unlink takes the OPENED chart's grade and
  ignores BOTH subjects' grades (before, one subject's grade always applied). The exposure is bounded:
  the payload and the twin carry no patient data beyond the two subjects' ids, which the opened chart's
  reader already sees as member lines of the same record — the rest is the judgement's provenance (which
  clinician's key judged, and how).
- A receiver that lacks the opened chart admits the event (db/020) and from then on counts that chart as
  having events (`cairn_patient_has_events`, db/001) — the existing pattern for any replicated event that
  precedes its chart's registration — and still applies the unlink from its payload.
- A reprojection reproduces the third-chart unlink from the payload, and a chain split from the opened
  chart reports *TookEffect* while an unlink on a cycle honestly reports *StillJoined*.
- A retry after *Outranked* would normally record a **newer** judgement that overrules the colleague's: a
  peer's identity event arrives through db/020, which admits it unchanged but merges its clock into this
  node's only up to now + 24 h (`cairn_max_hlc_drift_ms()`), so a peer more than 24 h ahead keeps outranking
  a retry until this node's own clock reaches the peer's (db/007, the node-plane door, instead refuses a node event that far ahead; it carries no identity
  events). So the window never says a retry "changes nothing".

## Rejected

- Relaxing `link` as well. A link attaches charts to this person; from a node that holds neither, a typo
  would join a stranger's chart sight unseen — R2a's reason for requiring both charts held for a link. An
  unlink attaches nothing, so that risk does not carry over.
- Silently ignoring an unrelated `--from`.
- A per-member "unlink" that guesses which link is wrong (principle 2: the human picks the edge).
- #699's (b) — keep the refusal and have *StillJoined* name the joining edge and where it can be unlinked (the
  maintainer chose (a)).
