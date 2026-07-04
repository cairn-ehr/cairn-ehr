# Design — §5.4 birth-year-range blocking pass (advisory matcher)

**Date:** 2026-07-04
**Spec home:** §5.2 (probabilistic matcher, blocking) / §5.4 (unidentified registration — John Doe).
**Status:** design approved; ready for implementation plan.

## 1. What this slice is

Slice B (2026-07-04) made a clinician-observed estimated-age range (`dob` value `"<min>/<max>"`,
precision `"year-range"`) a **positive scoring signal** (`compare_dob`: overlap → PARTIAL, never
DISAGREE). But it recorded an honest scope limit: the range never generates a **blocking key**, so
pure-age evidence only helps once a pair is already blocked together by *another* key — and a John
Doe's only name is an excluded callsign. This slice closes that gap: a range-carrying chart is
blocked into candidate pairs by age evidence itself.

Everything here is **advisory tier** (§5.2/§5.13, principle 12): Python in `matcher/`, no `db/`
floor change, no SCHEMA bump, no new event type, no ADR, no spec edit. Blocking is recall-oriented;
the in-DB veto floor (db/016) and the human worklist stay the safety boundary.

Also in scope (a recorded deferred item this slice closes): the **A/B pass-toggle** on
`generate_candidate_pairs`, so any pass change gets a one-command quantitative before/after.

## 2. The pair-semantics problem (why this pass is *anchored*)

The existing four passes are symmetric groups: every within-group pair is generated
(`_pairs_from_members`). Modelling "this John Doe's 11-year window" as such a group would generate
C(k+1, 2) pairs — including k×(k−1)/2 point×point pairs whose only shared feature is *being born
within 11 years of each other*, noise the exact-DOB pass deliberately never produces.

So the range pass is **anchored**: anchors are the (rare) range-carrying charts; pairs are only
ever (anchor × member), never member × member. Cost is bounded by
(#range charts × window population), and the block cap applies per anchor.

## 3. The SQL — one new anchored query (separate from `_GROUPS_SQL`)

New CTEs in `matcher/src/cairn_matcher/pipeline/db.py`:

- **`birth_window(patient_id, y_min, y_max, is_range)`** — every chart's inclusive birth-year
  interval, from `patient_demographic WHERE field='dob'`:
  - *range rows*: `facets->>'precision' = 'year-range' AND value ~ '^\d{4}/\d{4}$'`; split on `/`;
    a malformed value or `y_min > y_max` is **excluded** (safe degrade, mirroring `parse_dob` —
    never a false group, only a withheld rescue);
  - *point rows*: the existing first-4-digit-run rule → `[year, year]`, **excluding**
    `year-range` rows (`facets->>'precision' IS DISTINCT FROM 'year-range'`) so the two branches
    are disjoint — a range value must never double-enter as a false point `[min, min]`.
- **`blocking_sex(patient_id, sex)`** — the **union** of a chart's `sex-at-birth` and
  `administrative-sex` values (lower-cased). Union, not like-for-like: a trans patient whose
  `administrative-sex` matches the clinician's observation still groups even though
  `sex-at-birth` differs. Blocking is recall-oriented; the scorer and the human judge the rest.

Two additive passes over those CTEs, returned as `(pass_name, anchor, members[])`:

- **`dob-range`** — members = every *other* chart whose window overlaps the anchor's
  (`m.y_min <= a.y_max AND a.y_min <= m.y_max`). Range↔range overlap (the two-sites-same-John-Doe
  case — the only key that pair can ever share) falls out of the same join by construction.
- **`dob-range+sex`** — the same join, additionally requiring a shared `blocking_sex` value.
  This is the **rescue** pass, mirroring the name/name+year pattern: in a large DB the plain
  window block exceeds the cap and is skipped+reported; intersecting with sex roughly halves it,
  so it fires within cap in more settings. Additive-only: a sex mismatch merely means the rescue
  doesn't fire — it never suppresses a pair the plain pass or the scorer would surface.

## 4. Pair generation and the cap

A small pure helper `_pairs_from_anchor(anchor, members)` emits only canonical
(low, high) lowercase-uuid pairs of anchor×member (same ordering as `_pairs_from_members`).
Cap semantics match today's: if `len(members) + 1 > max_block_size` the block is recorded in
`skipped_blocks` as `(pass_name, anchor_key, size)`; otherwise its pairs join the cross-pass
canonical dedup set. The §5.13 hub duplicate sweep remains the declared backstop for skips.

## 5. Honesty fix to the existing `birth_year` CTE

Today `"1981/1991"` leaks `1981` into the `name+year` pass via the first-4-digit-run rule — a
**false key**: the window minimum is not a birth year, and grouping on it asserts a precision the
assertion refused (principle 4). The `birth_year` CTE gains
`AND facets->>'precision' IS DISTINCT FROM 'year-range'`; the new pass owns ranges. (John Does are
unaffected either way — their callsign is excluded from name tokens — this is for a *named* chart
carrying an estimated dob.)

## 6. The A/B pass-toggle

`generate_candidate_pairs(conn, *, max_block_size=100, enabled_passes=None)`:

- `None` → all six passes: `identifier`, `dob`, `name`, `name+year`, `dob-range`, `dob-range+sex`.
- An unknown pass name **raises `ValueError` immediately** — a silently-ignored typo would fake an
  A/B measurement (the pass would look "disabled" while actually misspelled).
- Filtering is by `pass_name` on the fetched rows: one SQL round-trip regardless of the subset.
  (Advisory batch; skipping query arms is an optimization we don't need.)

## 7. Eval mirror — deliberately untouched

`eval/generator.py`'s `shares_blocking_key` stays pinned to the base passes. The generator emits
no range DOBs, and an unmirrored pass only makes `_repair` conservative — it under-claims
recoverability and repairs via a name token anyway (the safe direction; the inverse, an
over-claiming mirror, would be the bug). A comment in `shares_blocking_key` records that the two
range passes are deliberately unmirrored until the generator learns to emit estimated-age records
(deferred below).

## 8. Testing (TDD, failing test first)

- **Pure:** `_pairs_from_anchor` — canonical (low, high) order both directions, no member×member
  pair, empty-members → empty set.
- **DB-gated** (new `tests/test_dob_range_blocking.py`, reusing conftest seeding):
  - range chart × point DOB inside window → paired by `dob-range`; outside → not paired by this
    pass (and, with no other shared key, not paired at all);
  - range↔range overlapping windows → paired; disjoint → not;
  - sex rescue: an oversized plain window block is skipped **and reported** while `dob-range+sex`
    still pairs the true candidate within cap;
  - union-sex guard: a member whose only sex row is `administrative-sex` still groups (the trans
    case the like-for-like key would drop);
  - malformed range value (`"1991/1981"`, `"about-40"`) → excluded silently, no crash, no pairs;
  - `name+year` regression: a named chart with a range dob no longer groups on the window-min year;
  - toggle: disabling `dob-range`/`dob-range+sex` removes exactly those pairs; default `None` runs
    all passes (regression against today's output); unknown pass name raises `ValueError`.
- **Suites green before commit:** `cd matcher && uv run pytest` (pure) and
  `CAIRN_TEST_PG=… uv run --extra pipeline pytest` (DB-gated); ruff clean. Rust workspace
  untouched by this slice.

## 9. What this slice deliberately does NOT change

- No `db/` floor file, no SCHEMA bump, no new event type, no ADR, no spec edit.
- `_GROUPS_SQL`'s four symmetric passes keep their shape (only the `birth_year` honesty filter).
- Scoring (`compare_dob`, banding, veto consultation) unchanged — already range-aware and
  positive-only from slice B.
- The Rust producer (`cairn-event::evidence`, `assert-observed-evidence`) unchanged.

## 10. Deferred (recorded, not lost)

- **Generator range-DOB emission** + a range-aware `shares_blocking_key` mirror — the piece that
  turns this pass's contribution into a measured recall number on synthetic volume (the A/B
  toggle landing now is the other half).
- Fuzzy near-window softening (a point one year outside the window neither blocks nor scores —
  binary overlap is the honest v1).
- A dedicated `alias` blocking pass (recorded at C5).
- Hub-tier aggressive sweep consuming the range passes with a larger cap budget.
- **Score-floor gap** ([issue #130](https://github.com/cairn-ehr/cairn-ehr/issues/130)): a pure-age
  John Doe pair now BLOCKS (this slice) but scores below the `review` band threshold (3.0) because
  `administrative-sex` is unscored — the rescue's own sex signal never reaches the scorer. Candidate
  resolutions are enumerated in the issue.
