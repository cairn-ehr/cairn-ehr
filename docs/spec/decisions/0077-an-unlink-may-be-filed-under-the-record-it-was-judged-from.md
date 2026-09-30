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

R2a's `unlink_charts` refuses an unlink unless one of the two subject charts is held on this node, because
the event is filed under a subject and db/005 step 8b admits only events filed under a held chart. #699's
scenario: chart A is held here and its record reads A, B and C through two links (A–B, B–C). B and C are
not held here (their registrations have not arrived). The far link B–C is the wrong one. The clinician
sees all three charts on A's record, judges from it, and is refused — although the record on screen is
exactly the evidence for the judgement. The tool's own advice ("unlink that link too") was untrue on this
node, because the only way to say it was refused.

## Decision

**An unlink where neither subject is held here may be filed under the chart it was judged from** — the
chart the clinician has open (`unlink-charts --from <chart>`, and the window's displayed chart) — provided
that chart is held here and its record reads **both** subjects (checked in the judgement's own
transaction, not only before it). **A `link` is never filed this way**: a link asserts two charts are one
person and needs both held, so the relaxation cannot reach it. The pair is always the event's **payload**;
the envelope's chart is only the stream the event is filed in.

The maintainer chose this (a) over the alternatives in #699 on 2026-09-28. The window's reading of the
choice is deliberately narrow:

- The filing chart is admitted only when it is **related to the pair** — a subject, or a chart whose record
  holds both. An opened chart unrelated to the pair is refused even when a subject is held (a stray `--from`
  must not be silently ignored, nor shown as a record the event never sat in).
- "Still joined?" — the question that decides between *TookEffect* and *StillJoined* — is asked of the
  **subjects**: `high ∈ person_charts(low)`. It used to be asked of the filed-under chart, which is the same
  answer whenever the filed-under chart is a subject, and wrong the moment it is a third chart (every
  successful A–B–C split would have read "still joined").

## Consequences

An audit of every reader of identity events by chart, done before building (2026-09-30): every projection,
flag, trust view, heal and re-fold pass (db/018, 019, 023–025, 039, 043, 054, 055), the sync doors and page
selection (db/020, db/051, `cairn-sync`), the medium and the plaintext twin read the pair from the
**payload**; none keys a link event on its envelope, and replication has no chart scope. So:

- **No wire change, no new event type, no SQL object** (`SCHEMA_GENERATION` stays 55). The envelope's chart
  was always free to differ from the payload subjects; nothing depended on it not doing so.
- The event sits in the **opened chart's stream**. For a `RecordOf` filing `cairn_effective_sensitivity`
  (db/048) keys chart-scoped grades on the envelope, so the unlink takes the OPENED chart's grade and
  ignores BOTH subjects' grades (before, one subject's grade always applied). The exposure is bounded:
  the payload and the twin carry only the two subjects' ids, which the opened chart's reader already
  sees as member lines of the same record.
- A receiver that lacks the opened chart counts the event as "has events" — an existing pattern for any
  replicated event — and still applies the unlink from its payload.
- A reprojection reproduces the third-chart unlink from the payload, and a chain split from the opened
  chart reports *TookEffect* while an unlink on a cycle honestly reports *StillJoined*.
- A retry after *Outranked* would normally record a **newer** judgement that overrules the colleague's (HLC
  merge at both sync doors, clamped at 24 h of drift, so a peer further ahead keeps outranking), so the window never says a retry "changes nothing".

**Rejected:** relaxing `link` as well (a link across charts nobody here holds is a claim this node cannot
even display); silently ignoring an unrelated `--from`; a per-member "unlink" that guesses which link is
wrong (principle 2: the human picks the edge).
