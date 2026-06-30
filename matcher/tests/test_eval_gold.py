"""The gold fixture is the regression gate: specific, robust band assertions.

These metrics are a regression/tuning instrument, NOT a statistical accuracy claim — the
set is tiny and hand-authored. Assertions are chosen to be robust to comparator nuance:
- alex-1/alex-2 reach AUTO via shared identifier (~4.0) + exact high-rank DOB (6.0) alone.
- garcia-1/smith-1 share nothing comparable -> NONE.
- rev-a/rev-b (different people) share only an exact DOB + agreeing sex -> 7.0 -> REVIEW,
  demonstrating 'weak coincidence is reviewed, never auto-linked'.
- No cross-entity pair reaches AUTO -> auto_false_link_rate == 0.
"""

from cairn_matcher.eval.dataset import record_to_candidate
from cairn_matcher.eval.loader import load_bundled_gold
from cairn_matcher.eval.scorer_eval import evaluate_scorer
from cairn_matcher.orchestrator import DEFAULT_CONFIG, field_comparisons
from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Band, band
from cairn_matcher.scoring import score


def _band_of(ds, id_a, id_b):
    recs = {r.record_id: r for r in ds.all_records()}
    cmp = field_comparisons(
        record_to_candidate(recs[id_a]), record_to_candidate(recs[id_b]), DEFAULT_CONFIG
    )
    return band(score(cmp), (), DEFAULT_THRESHOLDS)


def test_gold_loads():
    ds = load_bundled_gold()
    assert ds.name == "gold_v1"
    assert len(ds.all_records()) == 10


def test_strong_duplicate_is_auto():
    assert _band_of(load_bundled_gold(), "alex-1", "alex-2") is Band.AUTO_CANDIDATE


def test_unrelated_people_band_to_none():
    assert _band_of(load_bundled_gold(), "garcia-1", "smith-1") is None


def test_weak_coincidence_is_review_never_auto():
    assert _band_of(load_bundled_gold(), "rev-a", "rev-b") is Band.REVIEW


def test_no_non_match_is_auto_linked():
    m = evaluate_scorer(load_bundled_gold())
    assert m.auto_false_link_rate == 0.0
    assert m.confusion.nonmatch_auto == 0
