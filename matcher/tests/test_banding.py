"""Banding turns a score + veto findings into an advisory band (or None), and shapes the
persisted proposal payload. Pure — no database.

The band honours db/016 exactly: ANY veto finding (hard_veto OR degrade_hold) caps the
band at REVIEW — a veto never auto-links and never auto-rejects. Below the review
threshold nothing is proposed (the noise floor; the B3 hub sweep is the declared
backstop for missed signal).
"""

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.pipeline.banding import (
    Band,
    ProposalPayload,
    Thresholds,
    VetoFinding,
    band,
    build_payload,
    matcher_version,
)
from cairn_matcher.scoring import FieldEvidence, MatchScore


def _score(total: float) -> MatchScore:
    return MatchScore(total=total, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, total),
    ))


def test_high_score_no_veto_is_auto_candidate():
    assert band(_score(9.0), []) is Band.AUTO_CANDIDATE


def test_mid_score_no_veto_is_review():
    assert band(_score(4.0), []) is Band.REVIEW


def test_below_review_threshold_is_none():
    assert band(_score(2.9), []) is None


def test_hard_veto_caps_high_score_at_review():
    v = [VetoFinding("dob", "hard_veto", "dob", "verified dob clash")]
    assert band(_score(9.0), v) is Band.REVIEW


def test_degrade_hold_also_caps_high_score_at_review():
    v = [VetoFinding("identifier", "degrade_hold", "mrn:a", "profile absent")]
    assert band(_score(9.0), v) is Band.REVIEW


def test_veto_does_not_resurrect_a_sub_threshold_pair():
    # No positive signal + a veto -> still nothing to propose.
    assert band(_score(1.0), [VetoFinding("dob", "hard_veto", "dob", "x")]) is None


def test_review_threshold_is_inclusive():
    assert band(_score(3.0), []) is Band.REVIEW


def test_auto_threshold_is_inclusive():
    assert band(_score(8.0), []) is Band.AUTO_CANDIDATE


def test_custom_thresholds_apply():
    assert band(_score(5.0), [], Thresholds(review=1.0, auto=4.0)) is Band.AUTO_CANDIDATE


def test_matcher_version_is_deterministic_and_carries_package_version():
    from cairn_matcher import __version__
    v1 = matcher_version()
    v2 = matcher_version()
    assert v1 == v2
    assert v1.startswith(f"{__version__}+")


def test_build_payload_serializes_evidence_and_vetoes():
    score = _score(9.0)
    vetoes = [VetoFinding("dob", "hard_veto", "dob", "verified dob clash")]
    payload = build_payload(score, vetoes, Band.REVIEW)
    assert isinstance(payload, ProposalPayload)
    assert payload.score_total == 9.0
    assert payload.band is Band.REVIEW
    assert payload.evidence[0]["field"] == "name"
    assert payload.evidence[0]["level"] == "EXACT"
    assert payload.veto_findings[0]["severity"] == "hard_veto"
    assert payload.matcher_version == matcher_version()
