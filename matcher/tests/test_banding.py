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
    # WEAK positive signal (name only) + a veto -> still nothing to propose. The rescue
    # below is scoped to a SHARED IDENTIFIER; a mere name overlap must not flood the
    # worklist (db/016 fires a veto on any shared blocked name token + verified-dob clash).
    assert band(_score(1.0), [VetoFinding("dob", "hard_veto", "dob", "x")]) is None


def _score_with_identifier_exact(total: float) -> MatchScore:
    # A pair sharing a strong identifier (identifier field graded EXACT) but scored below
    # the review floor because the veto's own subject (e.g. a verified-DOB clash) carries a
    # large negative weight that drags the total down.
    return MatchScore(total=total, fields=(
        FieldEvidence("identifier", AgreementLevel.EXACT, 0, 4.0),
        FieldEvidence("dob", AgreementLevel.DISAGREE, 60, -3.7),
        FieldEvidence("name", AgreementLevel.DISAGREE, 60, -1.3),
    ))


def test_veto_plus_shared_identifier_forces_review_even_below_threshold():
    # ADR-0014 §6: a hard veto must force a HUMAN decision, never a silent auto-reject.
    # Two charts sharing a national identifier yet flagged (verified-dob clash) is the
    # classic wrong-chart / mistyped-identifier contamination signal; the veto suppressing
    # its OWN surfacing (by dragging the score sub-threshold) would be an auto-reject.
    v = [VetoFinding("dob", "hard_veto", "dob", "verified dob clash")]
    assert band(_score_with_identifier_exact(-1.0), v) is Band.REVIEW


def test_shared_identifier_below_threshold_without_a_veto_stays_none():
    # The rescue is veto-gated: strong evidence alone that scores sub-threshold is still
    # the ordinary noise floor (no anomaly to force in front of a human).
    assert band(_score_with_identifier_exact(-1.0), []) is None


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


# --- known-alias signal (§5.5(a) returning fabricated persona) --------------------------
# A match on a name a chart has REPUDIATED as known-false is exactly the "recognised
# returning persona" signal. The matcher cannot tell a returning persona from a real bearer
# of that false name, so §5.7 reserves the call for a Human: always surface for REVIEW,
# never auto-link, never silently drop.


def test_known_alias_below_threshold_is_forced_to_review():
    # Sub-threshold, no veto -> normally None; a known-alias match must not be dropped.
    assert band(_score(1.0), [], has_known_alias=True) is Band.REVIEW


def test_known_alias_high_score_is_capped_at_review_never_auto():
    # A pair must never AUTO-link on the strength of a name one chart declared false.
    assert band(_score(9.0), [], has_known_alias=True) is Band.REVIEW


def test_known_alias_flag_default_off_preserves_existing_behavior():
    # Regression: the flag defaults off; every existing band outcome is unchanged.
    assert band(_score(9.0), []) is Band.AUTO_CANDIDATE
    assert band(_score(1.0), []) is None


def test_build_payload_appends_known_alias_evidence():
    score = _score(1.0)
    alias_ev = ({"kind": "known_alias", "value": "John Fakename", "alias_of": "abc"},)
    payload = build_payload(score, [], Band.REVIEW, alias_evidence=alias_ev)
    # Field evidence still present, with the known-alias entry appended after it.
    assert payload.evidence[0]["field"] == "name"
    assert payload.evidence[-1] == {
        "kind": "known_alias",
        "value": "John Fakename",
        "alias_of": "abc",
    }


# --- unconfirmed-chart forcing rule (§5.4) -----------------------------------------------
# An unconfirmed (still-to-be-identified) chart matched only by age window + sex/belongings
# (or age + identifier, but that case is handled by the veto rescue) — thin but
# uncontradicted evidence — must not be silently dropped. Every corroborated candidate
# must reach the worklist. The rule is structural (field count) not a score floor, to
# resist drift as weights change.


def _evidence(field, level, contribution, rank=30):
    """Helper: construct one field evidence entry for a focused score."""
    return FieldEvidence(field, level, rank, contribution)


def _headline_score():
    """The §5.4 pure-age pair: dob PARTIAL + sex EXACT, everything else absent — ≈1.79,
    below review=3.0."""
    return MatchScore(total=1.79, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.EXACT, 0.71),
        _evidence("name", AgreementLevel.INSUFFICIENT_DATA, 0.0),
        _evidence("identifier", AgreementLevel.INSUFFICIENT_DATA, 0.0),
    ))


def test_unconfirmed_rule_forces_review_on_corroborated_pair():
    assert band(_headline_score(), vetoes=(), unconfirmed=True) is Band.REVIEW


def test_unconfirmed_rule_needs_two_positive_fields():
    # A bare window overlap (ONE positive field) must NOT flood the worklist — and the
    # production score shape always carries all four fields, three at INSUFFICIENT_DATA,
    # so this also pins that absent fields never count toward corroboration.
    one_field = MatchScore(total=1.07, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.INSUFFICIENT_DATA, 0.0),
        _evidence("name", AgreementLevel.INSUFFICIENT_DATA, 0.0),
        _evidence("identifier", AgreementLevel.INSUFFICIENT_DATA, 0.0),
    ))
    assert band(one_field, vetoes=(), unconfirmed=True) is None


def test_unconfirmed_rule_blocked_by_any_disagree():
    # Disagreeing evidence -> the normal scoring path decides; forcing is only for
    # thin-but-uncontradicted evidence.
    contradicted = MatchScore(total=0.4, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.EXACT, 0.71),
        _evidence("name", AgreementLevel.DISAGREE, -1.4),
    ))
    assert band(contradicted, vetoes=(), unconfirmed=True) is None


def test_unconfirmed_rule_fires_even_with_a_veto_never_auto_reject():
    # ADR-0014 §6: a veto forces a HUMAN decision, never an auto-reject. An identifier
    # veto needs NO verified values (db/016: disjoint same-system identifiers — e.g. off
    # a Doe's belongings) and the identifier comparator is positive-only (never DISAGREE),
    # so a corroborated Doe pair CAN carry a veto with zero disagreeing fields. Letting
    # the veto suppress the pair would silently drop the very candidate the unconfirmed
    # chart needs a human to look at. The veto still caps the band at REVIEW elsewhere.
    veto = VetoFinding("identifier", "hard_veto", "mrn", "disjoint same-system ids")
    assert band(_headline_score(), vetoes=(veto,), unconfirmed=True) is Band.REVIEW


def test_unconfirmed_rule_counts_structurally_not_by_weight():
    # The corroboration gate is STRUCTURAL (agreement levels), deliberately independent
    # of the weights table: a B3-learned 0.0 weight on a positive level must not stand
    # the forcing rule down (the docstring's no-drift claim, made real).
    zero_weighted = MatchScore(total=1.07, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.EXACT, 0.0),  # learned weight 0.0 -> 0 contribution
    ))
    assert band(zero_weighted, vetoes=(), unconfirmed=True) is Band.REVIEW


def test_unconfirmed_rule_inert_when_flag_false():
    assert band(_headline_score(), vetoes=(), unconfirmed=False) is None


def test_unconfirmed_rule_never_upgrades_to_auto():
    # Above-auto scores follow the NORMAL path; the forcing rule only ever acts below
    # review and only ever yields REVIEW.
    big = MatchScore(total=9.0, fields=(
        _evidence("identifier", AgreementLevel.EXACT, 8.0),
        _evidence("dob", AgreementLevel.PARTIAL, 1.0),
    ))
    assert band(big, vetoes=(), unconfirmed=True) is Band.AUTO_CANDIDATE  # unchanged path


def test_build_payload_appends_trust_evidence_after_alias_evidence():
    score = _headline_score()
    marker = {"kind": "identity_pending", "unconfirmed": ["11111111-1111-1111-1111-111111111111"]}
    payload = build_payload(score, (), Band.REVIEW, trust_evidence=(marker,))
    assert payload.evidence[-1] == marker
