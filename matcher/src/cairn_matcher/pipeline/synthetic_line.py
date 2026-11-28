# matcher/src/cairn_matcher/pipeline/synthetic_line.py
"""The line between synthetic patients and real ones, as the matcher sees it (ADR-0115).

A patient is marked synthetic (invented, for testing or training) at registration. The floor
refuses a link between a synthetic chart and a real one whoever signs it (db/018), so a
proposal for such a pair is a worklist item no human can ever act on. This module is how the
matcher avoids making one. Three places use it:

- pair generation (`db.generate_candidate_pairs`, `targeted.candidate_pairs_for`) splits every
  blocking group along the line BEFORE the block-size cap is applied, so that a loaded training
  cohort cannot push a group of real patients over the cap and silently cost them their
  duplicate check;
- `runner.assess` refuses to band a pair the floor's veto says crosses the line, which also
  retracts a proposal written before one chart's registration arrived;
- `sweep` counts what it proposed between two synthetic charts, so the duplicate sweep's yield
  (the matcher's miss-rate measure, ADR-0014) can be read without the training cohort in it.

THREE kinds, never two. A chart whose registration this node does not hold is `unknown`, and
`unknown` is on NEITHER side: it pairs with everything. Treating it as real would be a guess,
and treating it as synthetic would hide a real patient's duplicate. If its registration later
shows the pair crosses the line, `runner.assess` retracts the proposal then.

TWO SYNTHETIC CHARTS ARE STILL A PAIR. Duplicate-repair is one of the things a training cohort
is for. Nothing here excludes a chart for being synthetic, only a pair for crossing.

Which chart is which kind is decided in ONE place, the floor's `cairn_chart_synthetic`
(db/088), read by `load_kinds_for` below. Everything else here is pure: it takes that answer
as a plain mapping. The module imports no psycopg (`load_kinds_for` is handed an open
connection), because the per-chart check uses it and must import without the `pipeline` extra
(tests/test_pure_modules_import_without_psycopg.py).
"""

from collections.abc import Iterable, Mapping, Sequence
from uuid import UUID

SYNTHETIC = "synthetic"
REAL = "real"
UNKNOWN = "unknown"

# The `veto_kind` of the row `cairn_match_veto` (db/088) returns for a pair across the line.
SYNTHETIC_LINE_VETO = "synthetic_line"

# Appended to the blocking value of an oversized block's synthetic half when it is reported,
# so a log line never shows two different blocks under one name.
SYNTHETIC_SIDE_LABEL = " [synthetic patients]"


def load_kinds_for(conn, patient_ids) -> dict[str, str]:
    """Which charts are of synthetic patients, which of real ones, in ONE query (ADR-0115).

    Returns {canonical lowercase uuid text: 'synthetic' | 'real' | 'unknown'} with an entry for
    EVERY id given. The answer is the floor's own `cairn_chart_synthetic` (db/088): the matcher
    never reads `patient_registration` for itself, so there is one spelling of "is this chart
    invented" and the matcher cannot drift from the door that enforces it. 'unknown' is a chart
    whose registration this node does not hold; see `pipeline.synthetic_line` for why it is
    kept apart from 'real'.
    """
    ids = sorted({str(UUID(str(p))) for p in patient_ids})
    if not ids:
        return {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT p, cairn_chart_synthetic(p) FROM unnest(%s::uuid[]) AS p",
            (ids,),
        )
        return {str(pid): kind for pid, kind in cur.fetchall()}


def kind_of(kinds: Mapping[str, str], patient) -> str:
    """The kind of `patient` in `kinds`; `unknown` when the mapping does not say.

    The mapping is keyed by canonical lowercase uuid text (as `load_kinds_for` builds it),
    so the id is canonicalised here: an upper-case or braced id must not read as unknown.
    """
    return kinds.get(str(UUID(str(patient))), UNKNOWN)


def straddles(a, b, kinds: Mapping[str, str]) -> bool:
    """True when one chart is known synthetic and the other known real.

    The same rule as the floor's `cairn_charts_straddle_synthetic_line`: an unknown chart on
    either side is False.
    """
    return {kind_of(kinds, a), kind_of(kinds, b)} == {SYNTHETIC, REAL}


def both_synthetic(a, b, kinds: Mapping[str, str]) -> bool:
    """True when both charts are known synthetic (a training pair)."""
    return kind_of(kinds, a) == SYNTHETIC and kind_of(kinds, b) == SYNTHETIC


def sides(members: Sequence, kinds: Mapping[str, str]) -> list[tuple[str, list]]:
    """Split one symmetric blocking group into the sub-groups that may be paired within.

    Returns `[(label, members), ...]`. `label` is "" for a group that needed no split and for
    the real half of one that did; it is SYNTHETIC_SIDE_LABEL for the synthetic half.

    A group with charts on only one side of the line comes back whole. A group with both comes
    back as two: the real charts, and the synthetic charts, each WITH the unknown ones, because
    an unknown chart may be paired with either. A pair of two unknown charts then appears in
    both halves; the caller collects pairs in a set, so it is produced once.

    The caller applies the block-size cap to each half separately. That is the point of
    splitting before the cap instead of filtering pairs after it.
    """
    real = [m for m in members if kind_of(kinds, m) == REAL]
    synthetic = [m for m in members if kind_of(kinds, m) == SYNTHETIC]
    if not real or not synthetic:
        return [("", list(members))]
    unknown = [m for m in members if kind_of(kinds, m) == UNKNOWN]
    return [("", real + unknown), (SYNTHETIC_SIDE_LABEL, synthetic + unknown)]


def members_on_the_anchors_side(anchor, members: Iterable, kinds: Mapping[str, str]) -> list:
    """The members of an anchored block that may be paired with its anchor.

    An anchored block only ever pairs anchor x member, so there is nothing to split: the
    members across the line from the anchor are dropped. An unknown anchor keeps them all.
    """
    return [m for m in members if not straddles(anchor, m, kinds)]


def crosses_synthetic_line(vetoes: Iterable) -> bool:
    """True when the floor's veto rows say this pair crosses the line.

    Reads the `veto_kind` the floor returned and decides nothing itself: the matcher never
    re-implements a floor rule (see `db.match_veto`).
    """
    return any(v.veto_kind == SYNTHETIC_LINE_VETO for v in vetoes)
