# ADR-0075 — The step-3 prompt is a nudge, not a completeness claim

- **Status:** Accepted
- **Date:** 2026-09-26
- **Spec version at acceptance:** 0.77
- **Issues:** [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671) · filed: #679 · #680 · #681 (the repair path)
- **Relates to:** [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md) ·
  [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) ·
  [ADR-0014](0014-locale-pluggable-matcher-comparators.md)
- **Amends:** nothing in the wire or the floor. It retires one condition from the funnel design
  (`docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md`, *Risks*) and restores
  ADR-0061's meaning of `search.incomplete` where the reference UI had widened it.

## Context

The registration funnel's step-3 prompt shows at most `PROMPT_CAP` (5) candidates, and the new chart's
birth act signs exactly those as `search.displayed` (ADR-0061). The funnel design assumed a
full-name-plus-DOB search returns few candidates, and set its own revisit condition: *if the prompt is
routinely incomplete, the cap is wrong.*

Slice 2c measured it over 50,000 real names
(`cairn-gui/cairn-gui-tauri/results/2026-09-23-funnel-prompt-truncation.md`). `db/046` is a disjunction —
any name token OR the exact DOB — so a search returns a median of ~100 candidates, and because
`bound_for_prompt` OR-ed display truncation into `incomplete`, **92% of registrations signed
`incomplete: true`**. A flag that is nearly always on tells a later reader nothing.

The measurement also showed the limits of any prompt:

- ranked by passes matched, an **exactly-typed** duplicate is first in 500/500 searches;
- a duplicate typed with a **wrong date of birth** is shown in 100/500, ranked or not — it matches the
  name pass alone, which ties with every namesake;
- a duplicate with a **typo in a name token** loses that token's match, so it can be found only through
  its other keys (the remaining name tokens, an exact DOB) — and a name with every token misspelt is
  not found at all.

The obvious fix — show every "strong" candidate and sign that the weak remainder was withheld by rule —
would have withheld exactly the wrong-DOB duplicate, and would still have left the prompt claiming a
completeness it cannot have.

The maintainer's clinical judgement settled the direction: duplicate registration is common, most often
from typos in hard-to-spell names; the person at the desk cannot be made to browse a long list when they
only want to open or create a chart; and people do the right thing most of the time **as long as a
mistake has no serious consequence**. The practical answer is to accept that duplicates happen and make
repairing them easy and safe.

## Decisions

### 1. The step-3 prompt is a best-effort nudge, and truncation is its normal state

It shows the closest few charts to what was typed. It does not claim to show every candidate, and it is
never a gate (the trigger stays advisory, as before). The funnel design's *"if it is routinely
incomplete, the cap is wrong"* condition is **retired**: a cap that truncates is working as designed.

This is the same stance [§5.8](../identity.md) already takes — finding candidates is advisory, a miss is a
false split (§5.2's safe direction) — carried through to what the prompt signs.

### 2. The safety measure is how fast a duplicate is FOUND, not whether the clerk looked

A duplicate is repaired by an auditable, reversible `link` with no data loss (principle 2; §5.7). What
the link cannot repair is anything that happened in the **window** before it — an allergy recorded on
chart A while a drug is charted on duplicate B. So effort goes into shortening that window: a
commit-time local duplicate check by the advisory §5.2 matcher, a duplicate worklist, and a link
gesture that is fast and safe. Those are future slices, filed with this ADR as
[#679](https://github.com/cairn-ehr/cairn-ehr/issues/679) (commit-time check),
[#680](https://github.com/cairn-ehr/cairn-ehr/issues/680) (worklist) and
[#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) (link gesture); nothing here builds them. Until they exist, the backstop is the hub sweep (ADR-0014), which an isolated node may not
reach for a long time — an honest, stated gap.

### 3. `search.incomplete` means the SEARCH was partial — ADR-0061's meaning, restored

ADR-0061 defined it: *"if the node could not read some candidate it found, the attestation says so."*
The reference UI's `bound_for_prompt` additionally set it whenever the prompt cut the list. From this
ADR on it does not: display truncation is not an incompleteness of the search, and the attestation's
`displayed` array — literally true, the ids on screen in order — already states what was shown.

- **No wire change.** `db/045` requires a JSON boolean and is unchanged; so is `db/046`.
- **Stated plainly, since a signed field's reading changes:** bodies signed before 2026-09-26 by the
  reference UI may carry `incomplete: true` meaning *"more matched than were shown"*. Cairn is
  pre-clinical — only test fixtures and measurement corpora carry such bodies — so no clinical record
  is misread. A reader of an older body should treat `incomplete: true` from that UI as *possibly
  truncation only*.
- **ADR-0060 is not relaxed.** Decision 2 (*partial completion is reported, never implied*) is met by
  `displayed` itself, which claims only what was on screen, and by the on-screen line below. Decision 3
  (*under uncertainty, over-report*) does not apply: truncation is known, not uncertain.

### 4. Truncation is shown, not signed

The prompt still tells the person at the desk that more matched — one quiet line, e.g. *"the 5 closest
of 103 shown · type more to narrow"* — carried in the reference UI as a count separate from
`incomplete`. No signed count is added: the size of a disjunctive search (everyone sharing a token or
the birth date) means nothing to a reader years later, and ADR-0061 already rejected counts in favour of
named ids.

### 5. Ranking is improved where it costs nobody a step

The order of the prompt is the one lever that helps without asking anything of the user. It only
reorders — never adds or removes a candidate — by passes matched, then an **identifier match**, then
**name tokens matched** (exactly or as a typed prefix of at least 3 bytes, as `db/046`'s name pass
matches them, and never a §5.4 callsign's parts), then a **DOB near-miss** (day/month swapped, year
±1, the year's last two digits transposed), then **exactly-matched tokens** (so a prefix-only
"Annabel" never ties a typed "Ann"), then chart age. The identifier key, the prefix rule and the
exact-token tie-break came from this ADR's own PR review: without the first two, a chart found only
by its identifier (a nickname and a married surname) and a chart found by a shortened first name
("Alex" for "Alexander") both ranked with, or below, every namesake; the third was needed once the
prefix rule was measured. It is advisory: a ranking computed in Rust that drifts from
`db/046`'s SQL normalisation can only worsen the order, never lose a candidate. The measure is the rig's `--perturb` arms (`dob`, and `name`, `both`,
`dob-any`, `both-any`, added while measuring); the `-any` arms are the near-miss key's
controls and measure where the prompt stops helping. Measured 2026-09-26 over 50,000 real names
(`cairn-gui/cairn-gui-tauri/results/2026-09-26-funnel-prompt-ranking.md`): a DOB slip or a
simply wrong DOB with the name right, and a surname typo with the DOB right or slipped, are all
now shown 500/500, as are a chart found only by its typed MRN (48/500 before the review's
identifier key) and a shortened first name with a wrong DOB (499/500); a surname typo together
with a simply wrong DOB is shown 203/500, and a common full name with a wrong DOB in a twin-heavy
population 72/500. Those residues are Decision 2's.

## Consequences

- The attested `incomplete` flag becomes a real signal again: set only when the node failed to read a
  candidate.
- The prompt stays five rows and asks nothing more of the user.
- Duplicates are expected; the next design thread is the repair path (Decision 2).
- `PROMPT_CAP` is unchanged, and is still named and test-pinned — it is now a layout choice, not a
  completeness boundary.

## Rejected

- **Show every strong candidate and sign the weak remainder as "withheld by rule"** (#671's first
  direction): it withholds the wrong-DOB duplicate, and still claims a completeness the search cannot
  have.
- **A signed count of candidates not shown:** noise to a later reader; ADR-0061 already chose named ids
  over counts.
- **A new signed field instead of restoring `incomplete`'s meaning:** a second field whose only job is
  to contradict the first, for bodies that exist only in test fixtures.
- **Raising `PROMPT_CAP` until the number looks better:** a longer list is exactly what the person at
  the desk will not read.
- **Making the prompt a gate** (must scroll, must confirm): fails paper-parity, and confirmation dialogs
  are not a safety mechanism (principle 3).
