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
    # persists the integer: evidence and the config fingerprint store `.name`. If this test
    # ever needs a number, the renumbering broke stored data.
    import inspect

    from cairn_matcher.eval import model_io
    from cairn_matcher.pipeline import banding
    for module in (banding, model_io):
        src = inspect.getsource(module)
        assert "level.value" not in src and "int(level" not in src, module.__name__


def test_context_has_default_edit_distance_threshold():
    assert Context().edit_distance_threshold == 0.90
