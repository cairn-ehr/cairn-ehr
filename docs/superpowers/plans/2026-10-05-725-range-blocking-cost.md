# #725 — the per-chart duplicate check's range-blocking cost

Paper-parity: not clinical-surface — a latency fix inside the advisory duplicate-check worker; it adds and changes no human act (R5, which first shows the check's state to a clinician, owes the §1.2 benchmark).

**Issue:** [#725](https://github.com/cairn-ehr/cairn-ehr/issues/725) (found in R4, #679, PR #724). **Stacked on** PR #724
(R4 is not yet merged; this branch is cut from `feat/r4-commit-time-worker`).

## The finding is narrower than the issue says

The issue blamed "the blocking SQL" as a whole (~7 s of a 9.65 s p95 at 10 000 charts). A diagnostic split it
(scratch, M3 Max, PG 18.1, the `measure_check` population at N = 10 000, five charts):

| statement | per chart |
|---|---|
| symmetric passes (`_GROUPS_SQL`, filtered to the chart) | 90–180 ms |
| range passes (`_RANGE_GROUPS_SQL`, filtered to the chart) | **7.5 s** |

`EXPLAIN ANALYZE` of the range statement: the `dob-range+sex` arm joins `blocking_sex` to ITSELF on `sex` before it
joins the window. Sex has about two values, so that join yields **14.7 million rows** (the planner estimated 116 548 — a
CTE scan has no statistics). The window self-join itself (683 000 rows) costs about 0.6 s. The sweep pays the same
cross-product once per run.

A first prototype (sex arrays + `&&`, still two scans of `blocking_sexes`) did not help: the planner cross-joined the two
array scans first (estimated 200 rows each, actual 5 419). **Any shape that offers the planner two sex scans to join
invites this.**

## The change

1. **Sexes ride on the window rows** (`blocking_sql.py`). `blocking_sexes` (a patient's set of blocking sexes) is joined
   ONCE to `birth_window` on `patient_id` (`sexed_window`); `window_overlap` carries `anchor_sexes` / `member_sexes`;
   the `dob-range+sex` arm becomes a FILTER (`anchor_sexes && member_sexes`) with no join at all. Same semantics: "some
   sex of the anchor equals some sex of the member" ⇔ the two sets overlap; a chart with no blocking sex has NULL sexes,
   and `NULL && x` is not true — exactly the old inner join's "no row".
2. **The per-chart statement is anchored** (`blocking_sql.py`, used by `targeted.py`). A `relevant_anchor` CTE keeps
   the range anchors whose window overlaps the chart's (the chart itself included when it is a range anchor), and
   `window_overlap` is computed only for those. Each kept anchor's window is still computed IN FULL, so its block size —
   and so the cap's verdict — is the sweep's. The outer filter (`anchor = me OR me = ANY(members)`) stays: a relevant
   anchor whose SEX-filtered block lacks the chart must still drop out of the `+sex` arm.
3. **Shared, not copied.** Both statements compose from the same CTE and arm constants; only the anchored one adds
   `relevant_anchor` and one `AND` clause. A new range arm reaches both.

Prototype (same population, 15 charts incl. 5 range anchors; every result identical to the old statement's):
per-chart range statement **75–656 ms** (was 7.5–24 s); sweep range statement 3.1 s (was 8–15 s).

## Tests (TDD)

- **RED → GREEN, pure:** `test_blocking_sql_shape.py` — the range statements reference `blocking_sex(es)` only inside
  the CTEs that build `sexed_window` (no arm joins a sex scan), and the anchored statement differs from the sweep's only
  by `relevant_anchor` and its clause (the sharing pin). Mutation-checked by restoring the old arm.
- **Pin over shipped behaviour, DB:** a range block that holds the chart as a MEMBER and is over the cap is reported
  with the anchor's FULL size (anchoring must not shrink another anchor's window to the chart). Mutation-checked by
  restricting `window_overlap`'s members to the chart.
- **Existing guards that must stay green:** the drift canary (`test_targeted_blocking.py`, targeted == sweep for every
  chart of a range-heavy population), `test_dob_range_blocking.py` (union-of-sexes, unknown/padded sex, toggles),
  `test_eval_generator_sync.py` (the generator mirror's fragments).

## Measure, then retune

Re-run `python -m cairn_matcher.eval.measure_check --sizes 2000 10000` on a vacuumed scratch database; record p50/p95,
the sweep and the break-even in the R4 design page's as-built note and in `targeted.py`; set `Settings.bulk_threshold`
from the new break-even; re-justify `DEFAULT_TARGETED_CAP`. **Stop rule (R4's):** p95 > 2 s at 10 000 → file, don't
tune. The Pi re-measurement is **#728** (hardware).

## Out of scope (named, not dropped)

- The symmetric statement still groups the whole population per chart (90–180 ms at 10 000; it grows with N). Anchoring
  it is the next lever if the Pi needs one.
- The anchored range statement is still O(relevant anchors × population): in the 10 000-record diagnostic one sampled
  chart had 111 of the 752 range anchors relevant (the generator's range-DOB rate is ~7.5 %). A year-indexed key
  projection (the issue's option 2, related to #637) is the lever beyond.
