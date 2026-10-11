from cairn_matcher.agreement import AgreementLevel, Context


def test_levels_are_ordinal_strongest_is_exact():
    assert AgreementLevel.INSUFFICIENT_DATA < AgreementLevel.DISAGREE
    # NEAR: a clash that may be one keying error — weaker than a consistent coarsening,
    # stronger than a clash (spec 2026-10-11).
    assert AgreementLevel.DISAGREE < AgreementLevel.NEAR < AgreementLevel.PARTIAL
    assert AgreementLevel.PARTIAL < AgreementLevel.EDIT_DISTANCE
    assert AgreementLevel.EDIT_DISTANCE < AgreementLevel.PHONETIC
    assert AgreementLevel.PHONETIC < AgreementLevel.NICKNAME
    assert AgreementLevel.NICKNAME < AgreementLevel.EXACT
    # max() over a set of levels picks the strongest agreement
    assert max(AgreementLevel.DISAGREE, AgreementLevel.EXACT) is AgreementLevel.EXACT


def test_levels_persist_by_name_never_by_number():
    # Inserting NEAR renumbered every level above it. That is safe ONLY because nothing
    # persists the integer. Checked on what is actually written, not on source text: an
    # IntEnum handed to json.dumps would serialise as a NUMBER, which a grep cannot see.
    import json

    from cairn_matcher.eval import model_io
    from cairn_matcher.orchestrator import DEFAULT_CONFIG
    from cairn_matcher.pipeline.banding import (
        DEFAULT_THRESHOLDS,
        Band,
        _config_fingerprint,
        build_payload,
    )
    from cairn_matcher.scoring import DEFAULT_WEIGHTS, FieldEvidence, MatchScore

    s = MatchScore(total=2.0, fields=(FieldEvidence("dob", AgreementLevel.NEAR, 60, 2.0),))
    evidence = build_payload(s, (), Band.REVIEW).evidence
    assert json.loads(json.dumps(evidence))[0]["level"] == "NEAR"

    fingerprint = _config_fingerprint(DEFAULT_WEIGHTS, DEFAULT_THRESHOLDS, DEFAULT_CONFIG)
    assert '"NEAR"' in fingerprint

    table = json.loads(json.dumps(model_io._weights_to_json(DEFAULT_WEIGHTS)))
    assert "NEAR" in table["dob"]
    assert model_io._weights_from_json(table) == DEFAULT_WEIGHTS


def test_context_has_default_edit_distance_threshold():
    assert Context().edit_distance_threshold == 0.90
