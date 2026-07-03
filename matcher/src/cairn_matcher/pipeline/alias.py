"""Pure known-alias detection over the §5.5(a) `patient_alias_pool`.

A name established false (a fabricated persona) is struck from the display header but kept
as a **known alias** by db/025's `patient_alias_pool` VIEW, "so that if the same persona
returns the staff recognise it". This module is the matcher's advisory recognition of that:
given a candidate pair's names and known aliases, it emits a `known_alias` evidence entry
whenever one chart's alias is corroborated by the OTHER chart.

FLAG, never suppress. The matcher cannot tell a returning fabricated persona from a real,
different person who merely bears that false name — from the name alone they are identical.
So it only *surfaces* the fact for a human (§5.7 "Human"); the banding layer refuses to
auto-link on it, and never suppresses the true returning-persona recognition §5.5(a) wants.

Pure: no psycopg, no I/O. `pipeline.db.load_aliases` supplies the alias strings and
`pipeline.db.load_candidate` supplies the Names; `runner.propose` wires the two together.
"""

from collections.abc import Iterable

from cairn_matcher.pipeline.adapter import _name_bag
from cairn_matcher.records import Name


def _bag_set(aliases: Iterable[str]) -> dict:
    """Map each raw alias string to its normalized Name bag, keyed by the bag.

    Recognising an alias in NORMALIZED space (NFC + casefold + token bag, via the adapter's
    `_name_bag`) is deliberate: the suppression *floor* (db/025) is exact-string on the
    opaque value because it must be precise, but db/025 explicitly assigns fuzzy/normalized
    recognition to "the advisory matcher's job". Reusing `_name_bag` means "same name" here
    is byte-identical to the scorer's notion, so there is no second, drifting definition.
    """
    return {_name_bag(a): a for a in aliases}


def _corroborators(names: "frozenset[Name] | None", alias_bags: dict) -> set:
    """The set of normalized Name bags the other chart presents (its names ∪ its aliases).

    A chart's own names include struck aliases (db/025 leaves the retained set intact), but
    that is irrelevant here — this is only ever called with the OTHER chart's names, so a
    chart never self-corroborates its own alias.
    """
    out: set = set(names or frozenset())
    out.update(alias_bags.keys())
    return out


def known_alias_evidence(
    id_a: str,
    names_a: "frozenset[Name] | None",
    aliases_a: "frozenset[str]",
    id_b: str,
    names_b: "frozenset[Name] | None",
    aliases_b: "frozenset[str]",
) -> tuple[dict, ...]:
    """Evidence entries for every known alias of one chart corroborated by the other.

    For each raw alias `v` of chart A, `v` is *corroborated* iff its normalized bag appears
    in B's names or B's aliases; each corroborated alias emits
    ``{"kind": "known_alias", "value": v, "alias_of": id_a}`` (and symmetrically for B's
    aliases). The "both charts repudiated the same value" case yields one entry per side.

    Output is deduped on ``(value, alias_of)`` and stable-sorted so the persisted JSON is
    deterministic across runs.
    """
    bags_a = _bag_set(aliases_a)
    bags_b = _bag_set(aliases_b)
    corr_by_b = _corroborators(names_b, bags_b)  # what B presents, to test A's aliases
    corr_by_a = _corroborators(names_a, bags_a)  # what A presents, to test B's aliases

    entries: set[tuple[str, str, str]] = set()
    for bag, value in bags_a.items():
        if bag in corr_by_b:
            entries.add(("known_alias", value, id_a))
    for bag, value in bags_b.items():
        if bag in corr_by_a:
            entries.add(("known_alias", value, id_b))

    return tuple(
        {"kind": kind, "value": value, "alias_of": alias_of}
        for kind, value, alias_of in sorted(entries)
    )
