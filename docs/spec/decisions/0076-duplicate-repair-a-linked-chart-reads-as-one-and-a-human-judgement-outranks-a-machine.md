# ADR-0076 — Duplicate repair: a linked chart reads as one, and a human's judgement outranks a machine's

- **Status:** Accepted
- **Date:** 2026-09-27
- **Spec version at acceptance:** 0.78
- **Issues:** [#679](https://github.com/cairn-ehr/cairn-ehr/issues/679) ·
  [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680) ·
  [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) · folds in
  [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334)
- **Relates to:** [ADR-0075](0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md) (decision 2
  is this ADR's brief) · [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md) ·
  [ADR-0049](0049-commitment-based-sign-off-currency.md) ·
  [ADR-0053](0053-per-write-human-authorship.md) ·
  [ADR-0014](0014-locale-pluggable-matcher-comparators.md) · [ADR-0030](0030-advisory-actor-integration-contract.md)
- **Design:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`
  (the slicing, R1–R5, lives there, not here)

## Context

ADR-0075 accepted that duplicate registrations will happen and moved safety to the **window** between
a duplicate's creation and its repair by `link` — the time in which an allergy can sit on chart A while
a drug is charted on chart B. Its premise was that the repair is already *safe* (`link` is append-only,
auditable, lossless — principle 2) and only needs to be made *easy*.

Surveying the code before designing the repair showed the premise was only half built:

1. **No read follows a link.** The medication read is `WHERE patient_id = $1`; `person_chart` (db/018)
   is "thin by design", its unified read "deliberately out of scope" at C1. After a link, chart A still
   shows none of B's drugs. A link therefore repaired nothing a clinician could see.
2. **"These are two different people" had no home.** The identity algebra is closed; the matcher's
   `match_proposal` worklist (db/017) is node-local and does not replicate, so a locally recorded
   rejection is re-proposed by the hub sweep and by every peer, and the review is repeated.
3. **A machine could override a human.** `patient_link` is latest-HLC-wins. An un-attested link from
   a peer's matcher carrying a later HLC displaced a reviewer's `unlink`; and an un-attested `unlink`
   (the ADR-0030 agent writer can author one — unlinks are not veto-gated) could split a reviewer's
   `link`.
4. **The known reconciliation-group defect (#334) becomes the normal case.** A group spanning two
   charts shows twice on one and not at all on the other; after a link, the same drug recorded on both
   duplicates is exactly that shape — a doubled line on a medication chart.

## Decisions

### 1. A linked chart opens as one combined record

A read of a chart reads the chart **set** — every chart in its link component, answered by one database
function (`cairn_person_charts`) so that every reader agrees on the set. Every row names its **source
chart(s)**. This is two paper folders clipped together: the clinician reads both.

Demographics are **not** combined. The header lists each member's own name, date of birth and chart
id; no winner is chosen across members, because the disagreement is often the typo that produced the
duplicate. Choosing one is a separate question.

A reconciliation group lying wholly inside the set appears once. A group reaching a chart **outside**
the set is still a cross-patient group (#334's real hazard) and is still refused sign-off.

### 2. Writes stay per chart

Linking changes what is *read*, never where anything was *written*. A sign-off attests each medication
thread under the chart the thread lives on (ADR-0049's per-thread attestation is unchanged); a cease
writes to its thread's chart. When a new-content write from a combined view first exists, it goes to
the chart that was opened.

### 3. A chart command names the displayed SET, and refuses when it changed

The funnel's rule — every chart command names the chart on screen — widens from one chart to the set.
A link or unlink landing while a list is under review changes the set; a sign-off naming the old set is
refused with a sentence telling the clinician to reload. Signing a list the human did not see is the
defect this prevents.

### 4. "Different people" is a human-attested `unlink`

The algebra stays closed. An `unlink` asserted on a pair that was never linked records a reviewer's
judgement that the two charts are two people; it replicates, so neither a peer nor the hub sweep
re-proposes the pair, and a later human `link` reverses it.

The funnel design's rule stands and is not contradicted: a candidate *displayed and not chosen* at
registration is **never** recorded as an `unlink`. That is weak evidence at best (ADR-0061); this is a
deliberate side-by-side review, a different act.

### 5. An attested link assertion outranks an un-attested one

`patient_link`'s winner order becomes **attested first**, then the existing
`(hlc_wall, hlc_counter, origin)`, then `content_address`. It is still a total order over the
assertions, so every node converges on the same winner (principle 1). "Attested" has the one definition
db/018's applier already uses for its hard-veto check: an attester key is present and
`cairn_attestation_vouched` holds.

Its effect is exactly two protections: a machine's link never displaces a human's `unlink`, and a
machine's `unlink` never splits a human's `link`. Between two human judgements, or two machine ones,
the latest still wins. The losing assertion stays in the log (never erased) and is surfaced for review.

This is the §5.13 seam — proposal to identity algebra — and it is safety-critical, so it lives in the
database, reached by both write doors through the one `patient_link` applier.

### 6. The step-3 prompt shows one row per person and signs every member chart

Search results collapse by link component: one row per person, listing each member's identity line,
ranked by its best-matching member. The registration's `search.displayed` keeps its shape (`db/045` is
unchanged) and names **every member chart of every row shown**, which is literally what was on screen.

Stated because the reading of a signed field changes: `displayed_count`, and the legibility twin's
*"N near-match(es) displayed"*, count **charts, not people**, from this ADR on.

### 7. The commit-time check proposes; it never links

Every chart is checked by the advisory §5.2 matcher when its identity evidence changes — a node-local
queue filled by the database, drained by a matcher worker that talks only to Postgres (§9: the database
is the integration boundary). The queue can never fail a clinical write. A hit, in any band, becomes a
proposal shown on both charts (§5.2's banner, with the other chart's active medications, not merged in)
and on a worklist; any enrolled human resolves it as a `link` or an `unlink`. Auto-linking at commit
time is not done: a wrong auto-link would put two people's medication lists into one combined record
(decision 1). It may become policy once the worklist yields real precision figures (§5.13 — the
worklist's yield is the metric). A worker that is behind is shown as behind, never as "no duplicates"
(principle 4).

## Consequences

- ADR-0075's claim that repair is safe becomes true of what a clinician sees, not only of the log.
- The matcher gains a one-chart mode and a runner; it must skip pairs already linked or unlinked.
- `patient_link` gains a derived `attested` column; its applier's overlay condition changes.
- #334 is fixed for every chart, linked or not.
- An allergy stream, when it exists, joins the combined read and the banner; nothing here waits for it.
- No wire change and no new event type.

## Rejected

- **Show only the opened chart, with a "linked to B" line.** Keeps the window's hazard alive after the
  repair: a fact recorded only on B stays one click away.
- **Combine only safety content.** Two read models for one chart, and the line between them moves
  with every new stream.
- **Per-chart rows at the front door.** One person takes several of the prompt's five places.
- **Record "different people" in `match_proposal` only.** Node-local; every peer and the hub re-ask.
- **A new identity-algebra member (`distinct`).** Reopens a closed algebra to record what `unlink`
  already records.
- **Run the matcher from the window after `register`.** Misses CLI and synced registrations, and couples
  the UI to a Python install.
- **Port a scorer to Rust or SQL.** Two scorers that drift.
- **Auto-link the auto band at commit time,** plain or with an "unreviewed" marker: the first combines
  two people silently; the second is the same review labour plus a period of combined data on screen
  unconfirmed.
