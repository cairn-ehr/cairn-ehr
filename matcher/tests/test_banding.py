"""Banding turns a score + veto findings into an advisory band (or None), and shapes the
persisted proposal payload. Pure — no database.

The band honours db/016 exactly: ANY veto finding (hard_veto OR degrade_hold) caps the
band at REVIEW — a veto never auto-links and never auto-rejects. Below the review
threshold nothing is proposed (the noise floor; the B3 hub sweep is the declared
backstop for missed signal).
"""

from dataclasses import replace

import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import DAY_MONTH_SWAP, compare_dob, compare_exact
from cairn_matcher.orchestrator import DEFAULT_CONFIG, FieldSpec
from cairn_matcher.pipeline.banding import (
    Band,
    ProposalPayload,
    Thresholds,
    VetoFinding,
    _corroborated_positive,
    band,
    build_payload,
    dob_near_miss_evidence,
    matcher_version,
)
from cairn_matcher.records import DateValue
from cairn_matcher.scoring import (
    DEFAULT_WEIGHTS,
    FieldEvidence,
    FieldWeights,
    MatchScore,
    Weights,
)


def _score(total: float) -> MatchScore:
    return MatchScore(total=total, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, total),
    ))


# --- Thresholds review<=auto invariant (issue #211 gap 2) --------------------------------
# band() treats [review, auto) as REVIEW and [auto, inf) as AUTO_CANDIDATE, so an inverted
# Thresholds(review > auto) makes the REVIEW band VANISH: every un-vetoed score >= review
# lands in AUTO_CANDIDATE. That is a swapped-kwargs typo waiting to auto-link. The dataclass
# must refuse the inversion at construction so it can never reach band().


def test_thresholds_rejects_inverted_review_above_auto():
    with pytest.raises(ValueError, match="review"):
        Thresholds(review=5.0, auto=3.0)


def test_thresholds_allows_review_equal_auto():
    # Equal is the degenerate-but-coherent boundary (no REVIEW band, no inversion); the
    # learner never emits it — auto = review + margin, margin > 0 — but it is not unsafe,
    # so the invariant is `<=`, not `<`, and must not raise here.
    assert Thresholds(review=4.0, auto=4.0).review == 4.0


def test_thresholds_allows_ordinary_review_below_auto():
    t = Thresholds(review=3.0, auto=8.0)
    assert (t.review, t.auto) == (3.0, 8.0)


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


# --- matcher_version pins the FULL effective config (issue #100) -------------------------
# ADR-0011/0029 make the config-pinned actor identity the contamination-recall handle:
# "which links did matcher-config vN propose?". A pin that moves only with the weights
# makes a bad threshold or comparator rollout invisible in the proposal table — the recall
# has no key to cascade on. So EVERY knob that can change a band decision must move the
# digest: thresholds, per-field comparator wiring, context params, and weights.


def test_matcher_version_changes_when_thresholds_change():
    custom = Thresholds(review=2.0, auto=8.0)
    assert matcher_version(thresholds=custom) != matcher_version()


def test_matcher_version_changes_when_a_context_param_changes():
    # Loosening edit_distance_threshold changes agreement levels (EDIT_DISTANCE vs
    # DISAGREE on near-miss strings) and therefore band decisions — it must move the pin.
    loosened = tuple(
        replace(spec, context=Context(edit_distance_threshold=0.80))
        for spec in DEFAULT_CONFIG
    )
    assert matcher_version(config=loosened) != matcher_version()


def test_matcher_version_changes_when_a_comparator_is_swapped():
    # The ADR-0014 locale-pack seam: same field, different comparator FUNCTION. The
    # wiring is config (the function bodies are pinned by the package version), so a
    # swap must be visible in the pin.
    swapped = tuple(
        replace(spec, comparator=compare_exact) if spec.field == "name" else spec
        for spec in DEFAULT_CONFIG
    )
    assert matcher_version(config=swapped) != matcher_version()


def test_matcher_version_changes_when_weights_change():
    # The one axis the pre-#100 digest already covered — kept as an explicit regression.
    heavier = Weights(per_field={
        **{f: fw for f, fw in DEFAULT_WEIGHTS.per_field.items() if f != "sex"},
        "sex": FieldWeights({AgreementLevel.EXACT: 1.5, AgreementLevel.DISAGREE: -2.0}),
    })
    assert matcher_version(weights=heavier) != matcher_version()


def test_matcher_version_is_insensitive_to_weights_table_insertion_order():
    # Canonicalization guard: the digest input is sorted, so two EQUAL configs pin
    # identically however their dicts happened to be assembled.
    reordered = Weights(per_field=dict(reversed(list(DEFAULT_WEIGHTS.per_field.items()))))
    assert matcher_version(weights=reordered) == matcher_version()


def _absent_names(rec):
    """A named stand-in extractor: reduces every name comparison to INSUFFICIENT_DATA."""
    return (None, 0)


def test_matcher_version_changes_when_an_extractor_is_swapped():
    # FieldSpec.get decides WHICH record data reaches the comparator — swapping it changes
    # band decisions exactly like swapping the comparator (a locale pack feeding, say,
    # historical names through the same compare_name_set), so it must move the pin too
    # (#100 review finding 1).
    swapped = tuple(
        replace(spec, get=_absent_names) if spec.field == "name" else spec
        for spec in DEFAULT_CONFIG
    )
    assert matcher_version(config=swapped) != matcher_version()


def test_matcher_version_pins_int_and_float_thresholds_identically():
    # Numerically equal configs must not split the recall key on Python numeric TYPE:
    # Thresholds(3, 8) and Thresholds(3.0, 8.0) band every pair identically, so they are
    # the SAME config and must carry the same pin.
    assert matcher_version(thresholds=Thresholds(review=3, auto=8)) == matcher_version()


def test_config_fingerprint_covers_every_declared_config_field():
    # The completeness guard that would have caught #100 itself: every declared field of
    # every config dataclass must appear in the fingerprint. A future field added to
    # FieldSpec/Context/Thresholds that can move a band decision then fails HERE instead
    # of silently vanishing from the recall key.
    import json
    from dataclasses import fields

    from cairn_matcher.orchestrator import FieldSpec
    from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, _config_fingerprint

    fp = json.loads(_config_fingerprint(DEFAULT_WEIGHTS, DEFAULT_THRESHOLDS, DEFAULT_CONFIG))
    assert set(fp["thresholds"]) == {f.name for f in fields(Thresholds)}
    assert len(fp["comparators"]) == len(DEFAULT_CONFIG)
    for entry in fp["comparators"]:
        assert set(entry) == {f.name for f in fields(FieldSpec)}
        assert set(entry["context"]) == {f.name for f in fields(Context)}


def test_config_fingerprint_weights_field_sets_are_pinned():
    # Weights/FieldWeights CANNOT ride the generic dataclasses.fields rendering above:
    # their Mapping-with-enum-keys shape defeats asdict, so _config_fingerprint's weights
    # branch hand-walks per_field/weights. A hand-walk silently drops any FUTURE field
    # added to either dataclass (the #100 failure mode, relocated), so pin both field
    # sets here — the #211/#79 anti-drift pattern. If this assertion fails, you added a
    # field to Weights or FieldWeights: extend _config_fingerprint's weights branch to
    # carry it, THEN update this pin.
    from dataclasses import fields

    from cairn_matcher.scoring import FieldWeights

    assert {f.name for f in fields(Weights)} == {"per_field"}
    assert {f.name for f in fields(FieldWeights)} == {"weights"}


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


def test_build_payload_pins_the_callers_thresholds():
    # The pin must record the config that ACTUALLY banded the pair (issue #100): a caller
    # running custom thresholds produces a different matcher_version than the defaults.
    custom = Thresholds(review=1.0, auto=4.0)
    payload = build_payload(_score(5.0), [], Band.AUTO_CANDIDATE, thresholds=custom)
    assert payload.matcher_version == matcher_version(thresholds=custom)
    assert payload.matcher_version != matcher_version()


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


# --- DOB near miss (spec 2026-10-11): never auto-linked; the proposal names the keying error ---

def _near_score(total: float) -> MatchScore:
    """Name EXACT + dob NEAR (+2) + a shared identifier making up the rest of `total`."""
    return MatchScore(total=total, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 2.0),
        FieldEvidence("identifier", AgreementLevel.EXACT, 60, total - 7.0),
    ))


def test_a_near_miss_dob_never_auto_links_even_with_a_shared_identifier():
    # name 5 + dob NEAR 2 + identifier 8 = 15 >= auto (8), no veto: still REVIEW.
    assert band(_near_score(15.0), ()) is Band.REVIEW


def test_a_near_miss_below_review_is_not_rescued_by_a_shared_identifier():
    # The shared-identifier rescue is for a VETO dragging the score under (ADR-0014 §6). A DOB
    # in doubt is not a veto: below the review threshold such a pair is no proposal, as any
    # pair below it is (band's docstring; ADR-0079 decision 4).
    assert band(_near_score(2.5), ()) is None


def test_a_near_miss_pair_above_review_is_review():
    s = MatchScore(total=8.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 2.0),
        FieldEvidence("sex", AgreementLevel.EXACT, 60, 1.0),
    ))
    assert band(s, ()) is Band.REVIEW


def test_near_is_neither_corroboration_nor_a_clash():
    # §5.4 unconfirmed-chart gate: >= 2 positive fields (>= PARTIAL) and NO DISAGREE.
    # NEAR counts as neither — so name + sex + NEAR still corroborates, name + NEAR does not.
    two_pos = MatchScore(total=8.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("sex", AgreementLevel.EXACT, 60, 1.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 2.0),
    ))
    one_pos = MatchScore(total=7.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", AgreementLevel.NEAR, 60, 2.0),
    ))
    assert _corroborated_positive(two_pos) is True
    assert _corroborated_positive(one_pos) is False


def _dob_scored(level: AgreementLevel) -> MatchScore:
    return MatchScore(total=5.0, fields=(
        FieldEvidence("name", AgreementLevel.EXACT, 60, 5.0),
        FieldEvidence("dob", level, 60, 0.0),
    ))


def test_near_miss_evidence_names_the_pattern():
    a, b = DateValue(1950, 1, 7), DateValue(1950, 7, 1)
    near = _dob_scored(AgreementLevel.NEAR)
    assert dob_near_miss_evidence(near, a, b) == (
        {"kind": "dob_near_miss", "pattern": DAY_MONTH_SWAP},)
    # Unreachable from assess (a scored NEAR has both DOBs) but never a crash: no pattern.
    assert dob_near_miss_evidence(near, a, None) == ({"kind": "dob_near_miss", "pattern": None},)


def test_near_miss_evidence_follows_the_scored_level_not_a_second_reading():
    # The entry is emitted only when the configured comparator actually SCORED the DOB NEAR.
    # A locale pack that grades a day/month swap DISAGREE must not get a payload that says
    # "day and month swapped" beside a dob of -4 (#761 item 1).
    a, b = DateValue(1950, 1, 7), DateValue(1950, 7, 1)
    assert dob_near_miss_evidence(_dob_scored(AgreementLevel.DISAGREE), a, b) == ()
    assert dob_near_miss_evidence(_dob_scored(AgreementLevel.EXACT), a, a) == ()


def test_near_miss_evidence_without_a_default_pattern_still_says_near():
    # A locale pack may grade NEAR on an error the default profile does not name (a day-first
    # DD/MM boundary swap). The reviewer is still told it is a suspected keying error.
    a, b = DateValue(1950, 11, 10), DateValue(1950, 1, 11)
    assert dob_near_miss_evidence(_dob_scored(AgreementLevel.NEAR), a, b) == (
        {"kind": "dob_near_miss", "pattern": None},)


# --- A DOB in doubt (NEAR or DISAGREE) never auto-links (ADR-0079 decision 4) ---------------

def _dob_level_score(level: AgreementLevel, dob_weight: float) -> MatchScore:
    fields = (
        FieldEvidence("name", AgreementLevel.EXACT, 70, 5.0),
        FieldEvidence("sex", AgreementLevel.EXACT, 70, 1.0),
        FieldEvidence("identifier", AgreementLevel.EXACT, 0, 4.0),
        FieldEvidence("dob", level, 0, dob_weight),
    )
    return MatchScore(total=sum(f.weight_contribution for f in fields), fields=fields)


def test_a_disagreeing_dob_never_auto_links_even_with_a_shared_identifier():
    # name 5 + sex 1 + identifier 4 + dob DISAGREE at unknown provenance -2 = 8 >= auto, and
    # db/016 is silent because the DOBs are not both verified. Before ADR-0079's cap this
    # auto-linked two charts whose birth dates have nothing in common.
    assert band(_dob_level_score(AgreementLevel.DISAGREE, -2.0), ()) is Band.REVIEW


def test_an_exact_dob_in_the_same_shape_does_auto_link():
    # Control: it is the DOB level, not the field mix, that holds the two above at review.
    assert band(_dob_level_score(AgreementLevel.EXACT, 3.0), ()) is Band.AUTO_CANDIDATE


def test_build_payload_appends_the_near_miss_evidence_last():
    entry = {"kind": "dob_near_miss", "pattern": DAY_MONTH_SWAP}
    p = build_payload(_score(6.0), (), Band.REVIEW, near_miss_evidence=(entry,))
    assert p.evidence[-1] == entry


# The band order a weaker DOB must never climb above: no proposal < REVIEW < AUTO_CANDIDATE.
_BAND_RANK = {None: 0, Band.REVIEW: 1, Band.AUTO_CANDIDATE: 2}


def _band_rank(name, sex, ident, rank, dob_level, dob_rank) -> int:
    """Score one field mix with the real scorer and DEFAULT_WEIGHTS; return its band's rank."""
    from cairn_matcher.records import FieldComparison
    from cairn_matcher.scoring import score

    s = score([
        FieldComparison("name", name, rank),
        FieldComparison("sex", sex, rank),
        FieldComparison("identifier", ident, 0),
        FieldComparison("dob", dob_level, dob_rank),
    ])
    return _BAND_RANK[band(s, ())]


def test_a_weaker_dob_never_bands_higher_with_everything_else_fixed():
    # Monotonicity over the real scorer and DEFAULT_WEIGHTS, across every mix of the other
    # fields and of provenance. EXACT is the strongest DOB, and a DOB that disagrees outright
    # must never band above one a single keying error off. (PARTIAL and NEAR are deliberately
    # not ordered against each other: +2 for a one-keystroke miss is rarer by chance than
    # +1.5 for a year-only DOB agreeing with a full one.)
    from itertools import product

    L = AgreementLevel
    others = product(
        (L.EXACT, L.EDIT_DISTANCE, L.PARTIAL, L.DISAGREE, L.INSUFFICIENT_DATA),  # name
        (L.EXACT, L.DISAGREE, L.INSUFFICIENT_DATA),                              # sex
        (L.EXACT, L.INSUFFICIENT_DATA),                                          # identifier
        (0, 40, 70),                                                             # their rank
        (0, 40, 70),                                                             # dob rank
    )
    for name, sex, ident, rank, dob_rank in others:
        case = (name, sex, ident, rank, dob_rank)
        at = {lvl: _band_rank(name, sex, ident, rank, lvl, dob_rank) for lvl in L}
        assert at[L.DISAGREE] <= at[L.NEAR], case
        for weaker in (L.NEAR, L.PARTIAL, L.DISAGREE, L.INSUFFICIENT_DATA):
            assert at[weaker] <= at[L.EXACT], (case, weaker)


# --- the no-auto-link cap needs to FIND the DOB (ADR-0079 decision 4) --------------------
# _dob_in_doubt reads the level scored for the field named "dob". A config that names its DOB
# field anything else, or scores two of them, would make the cap pass silently: an auto-link on
# a DOB in doubt with nothing logged. So matcher_version — run when the worker starts and for
# every proposal — refuses such a config, loudly, before a single pair is banded.

def _renamed_dob(rec):
    """A named extractor standing in for a locale pack's own DOB field."""
    return (None, 0)


def test_a_config_whose_dob_field_is_named_otherwise_is_refused():
    renamed = tuple(
        replace(spec, field="birth_date") if spec.field == "dob" else spec
        for spec in DEFAULT_CONFIG
    )
    with pytest.raises(ValueError, match="dob"):
        matcher_version(config=renamed)


def test_a_config_with_two_dob_comparisons_is_refused():
    second = FieldSpec("dob_second", compare_dob, _renamed_dob)
    with pytest.raises(ValueError, match="dob"):
        matcher_version(config=DEFAULT_CONFIG + (second,))
    with pytest.raises(ValueError, match="dob"):
        matcher_version(config=DEFAULT_CONFIG + (replace(second, field="dob"),))


def test_a_config_with_its_own_dob_comparator_under_the_dob_name_is_accepted():
    # The locale-pack seam stays open: any comparator may grade the DOB, as long as the field
    # the cap reads is the one it grades.
    swapped = tuple(
        replace(spec, comparator=compare_exact) if spec.field == "dob" else spec
        for spec in DEFAULT_CONFIG
    )
    assert matcher_version(config=swapped) != matcher_version()
