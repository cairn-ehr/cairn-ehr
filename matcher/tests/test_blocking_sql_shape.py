"""The range-blocking SQL keeps the two shapes that make it cheap (#725). Pure: reads strings.

1. **No arm joins a sex scan.** The `dob-range+sex` arm once joined `blocking_sex` to itself on
   `sex`. Sex has about two values, so that join made ~14.7 million rows at 10 000 charts (the
   planner, with no statistics for a CTE, estimated ~116 000) and cost ~7 s per duplicate check.
   A patient's sexes now ride on its birth-window row (`sexed_window`, joined once on
   `patient_id`), and the arm is a filter. So, outside SQL comments, the sex relations are named
   exactly four times: the two CTE definitions, `blocking_sexes` reading `blocking_sex`, and
   `sexed_window` joining `blocking_sexes`. Any other mention — a JOIN, a comma join, a LATERAL
   subquery — can hand the planner the cross-product back; even two scans of per-patient sex
   ARRAYS were cross-joined first in the #725 prototype.
2. **The per-chart statement is the sweep's, plus one CTE and one clause.** Shared, not copied:
   a new range arm, or a change to a window's bounds, reaches both or neither.

Neither is a correctness property — tests/test_targeted_blocking.py's drift canary and
tests/test_dob_range_blocking.py pin what the statements RETURN. These pin what they COST.
"""

import re

from cairn_matcher.pipeline import blocking_sql as b

# Every mention of a sex relation (the word boundary keeps the two names apart). Counting every
# mention, not just FROM/JOIN, also catches a comma join or a LATERAL subquery.
_SEX_NAME = re.compile(r"\b(blocking_sexes|blocking_sex)\b")
_DEFINES = re.compile(r"\s+AS\s+\(")              # follows a CTE definition's name
_TOKEN_BEFORE = re.compile(r"(\w+|[^\w\s])\s*$")  # the word (or punctuation) before a mention

# The only four mentions a range statement may make, in text order.
_ALLOWED_SEX_MENTIONS = [
    ("blocking_sex", "definition"),    # blocking_sex AS (...): sentinel-excluded union of facets
    ("blocking_sexes", "definition"),  # blocking_sexes AS (...): one row per patient
    ("blocking_sex", "FROM"),          # ...which reads blocking_sex
    ("blocking_sexes", "JOIN"),        # sexed_window: attached to the birth window by patient_id
]


def _without_comments(sql: str) -> str:
    """Drop `-- …` SQL comments, so prose that names a relation is not counted as a join."""
    return "\n".join(line.split("--", 1)[0] for line in sql.splitlines())


def _sex_mentions(sql: str) -> list[tuple[str, str]]:
    """(relation, how it is used) for every mention, in text order.

    "definition" when `AS (` follows the name; otherwise the token just before it -- FROM, JOIN,
    or e.g. "," for a comma join, which then fails the comparison, as it should.
    """
    text = _without_comments(sql)
    out = []
    for m in _SEX_NAME.finditer(text):
        if _DEFINES.match(text, m.end()):
            out.append((m.group(1), "definition"))
        else:
            before = _TOKEN_BEFORE.search(text[: m.start()])
            out.append((m.group(1), before.group(1) if before else ""))
    return out


def test_the_sweeps_range_statement_reads_sexes_once_and_joins_no_sex_scan():
    assert _sex_mentions(b._RANGE_GROUPS_SQL) == _ALLOWED_SEX_MENTIONS


def test_the_anchored_range_statement_reads_sexes_once_and_joins_no_sex_scan():
    assert _sex_mentions(b._ANCHORED_RANGE_GROUPS_SQL) == _ALLOWED_SEX_MENTIONS


def test_the_sex_arm_is_a_filter_on_the_window_rows():
    # Whitespace-tolerant: a cosmetic re-wrap must not trip a test whose message is about cost.
    assert re.search(r"\bWHERE\s+anchor_sexes\s*&&\s*member_sexes\b", b._RANGE_GROUPS_SQL)


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
