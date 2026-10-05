"""The range-blocking SQL keeps the two shapes that make it cheap (#725). Pure: reads strings.

1. **No arm joins a sex scan.** The `dob-range+sex` arm once joined `blocking_sex` to itself on
   `sex`. Sex has about two values, so that join made ~14.7 million rows at 10 000 charts (the
   planner, with no statistics for a CTE, estimated ~116 000) and cost ~7 s per duplicate check.
   A patient's sexes now ride on its birth-window row (`sexed_window`, joined once on
   `patient_id`), and the arm is a filter. So the statement may reference a sex relation exactly
   twice: `blocking_sexes` reading `blocking_sex`, and `sexed_window` joining `blocking_sexes`.
   Any other join of a sex scan hands the planner the cross-product back — even two scans of
   per-patient sex ARRAYS were cross-joined first in the #725 prototype.
2. **The per-chart statement is the sweep's, plus one CTE and one clause.** Shared, not copied:
   a new range arm, or a change to a window's bounds, reaches both or neither.

Neither is a correctness property — tests/test_targeted_blocking.py's drift canary and
tests/test_dob_range_blocking.py pin what the statements RETURN. These pin what they COST.
"""

import re

from cairn_matcher.pipeline import blocking_sql as b

# A relation named after FROM or JOIN; the word boundary keeps `blocking_sex` and
# `blocking_sexes` apart.
_SEX_RELATION = re.compile(r"\b(FROM|JOIN)\s+(blocking_sexes|blocking_sex)\b")


def _without_comments(sql: str) -> str:
    """Drop `-- …` SQL comments, so prose that names a relation is not counted as a join."""
    return "\n".join(line.split("--", 1)[0] for line in sql.splitlines())


def _sex_relations(sql: str) -> list[tuple[str, str]]:
    return _SEX_RELATION.findall(_without_comments(sql))


def test_the_sweeps_range_statement_reads_sexes_once_and_joins_no_sex_scan():
    assert _sex_relations(b._RANGE_GROUPS_SQL) == [
        ("FROM", "blocking_sex"),      # blocking_sexes: one row per patient, its set of sexes
        ("JOIN", "blocking_sexes"),    # sexed_window: attached to the birth window by patient_id
    ]


def test_the_anchored_range_statement_reads_sexes_once_and_joins_no_sex_scan():
    assert _sex_relations(b._ANCHORED_RANGE_GROUPS_SQL) == [
        ("FROM", "blocking_sex"),
        ("JOIN", "blocking_sexes"),
    ]


def test_the_sex_arm_is_a_filter_on_the_window_rows():
    assert "WHERE anchor_sexes && member_sexes" in b._RANGE_GROUPS_SQL


def test_the_anchored_statement_is_the_sweeps_plus_one_cte_and_one_clause():
    anchored = b._ANCHORED_RANGE_GROUPS_SQL
    assert anchored.count(b._RELEVANT_ANCHOR_CTE) == 1
    assert anchored.count(b._ANCHOR_CLAUSE) == 1
    stripped = anchored.replace(b._RELEVANT_ANCHOR_CTE + ",\n", "").replace(b._ANCHOR_CLAUSE, "")
    assert stripped == b._RANGE_GROUPS_SQL


def test_the_anchor_clause_restricts_the_anchor_never_the_member():
    # Each kept anchor's window must be computed IN FULL: its size decides the cap, and the cap's
    # verdict must be the sweep's. Restricting the member side would shrink another chart's
    # window to the one being checked.
    assert "a.patient_id" in b._ANCHOR_CLAUSE
    assert "m.patient_id" not in b._ANCHOR_CLAUSE
