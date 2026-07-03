"""Pure known-alias detection (pipeline/alias.py) — no database.

A repudiated name (§5.5(a) fabricated persona) stays a *known alias* in patient_alias_pool.
When one chart's known alias is corroborated by the OTHER chart (that chart bears the name,
or independently repudiated the same alias), the matcher tags the advisory proposal with a
`known_alias` evidence entry so a human reviewer sees the recognised-returning-persona signal.

This is FLAG, never suppress: the matcher cannot tell a returning fabricated persona from a
real, different bearer of that false name, so it only surfaces the fact for a human (§5.7).
"""

from cairn_matcher.pipeline.alias import known_alias_evidence
from cairn_matcher.records import Name

A = "11111111-1111-1111-1111-111111111111"
B = "22222222-2222-2222-2222-222222222222"


def _names(*displays: str) -> frozenset[Name]:
    """Normalized token-bag Names, mirroring the adapter's projection shaping."""
    from cairn_matcher.pipeline.adapter import _name_bag

    return frozenset(_name_bag(d) for d in displays)


def test_returning_persona_b_bears_a_known_alias():
    # A repudiated "John Fakename"; B is registered under that same name -> corroborated.
    ev = known_alias_evidence(
        A, _names("Jane Realname"), frozenset({"John Fakename"}),
        B, _names("John Fakename"), frozenset(),
    )
    assert ev == (
        {"kind": "known_alias", "value": "John Fakename", "alias_of": A},
    )


def test_both_charts_repudiated_the_same_alias():
    # Both independently repudiated the same fabricated value -> one entry per side.
    ev = known_alias_evidence(
        A, _names("Jane Realname"), frozenset({"John Fakename"}),
        B, _names("Bob Other"), frozenset({"John Fakename"}),
    )
    assert ev == (
        {"kind": "known_alias", "value": "John Fakename", "alias_of": A},
        {"kind": "known_alias", "value": "John Fakename", "alias_of": B},
    )


def test_uncorroborated_alias_yields_nothing():
    # A's alias appears NOWHERE on B (neither B's names nor B's aliases) -> no signal.
    ev = known_alias_evidence(
        A, _names("Jane Realname"), frozenset({"John Fakename"}),
        B, _names("Someone Else"), frozenset(),
    )
    assert ev == ()


def test_alias_match_is_normalized_case_space_and_unicode():
    # The alias pool value is opaque; the matcher recognises it in NORMALIZED space
    # (NFC + casefold + whitespace fold) — the adapter's notion of "same name".
    ev = known_alias_evidence(
        A, _names("Jane"), frozenset({"John Fakename"}),
        B, _names("  john   FAKENAME "), frozenset(),
    )
    assert ev == (
        {"kind": "known_alias", "value": "John Fakename", "alias_of": A},
    )


def test_own_retained_struck_name_does_not_self_corroborate():
    # Struck names stay in patient_name, so A's own names include its alias. That must NOT
    # count as corroboration — only the OTHER chart corroborates.
    ev = known_alias_evidence(
        A, _names("Jane Realname", "John Fakename"), frozenset({"John Fakename"}),
        B, _names("Totally Different"), frozenset(),
    )
    assert ev == ()


def test_empty_inputs_are_safe():
    assert known_alias_evidence(A, None, frozenset(), B, None, frozenset()) == ()
    assert known_alias_evidence(A, frozenset(), frozenset(), B, _names("x"), frozenset()) == ()
