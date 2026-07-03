# matcher/tests/test_john_doe_exclusion.py
"""§5.4 — the matcher excludes placeholder (callsign) names from its feature space.

A "John Doe" chart carries a system-generated callsign as a real, displayed name
(use_key='callsign'). §5.4 requires that name be invisible to the matcher: two different
unidentified patients registered at the same site on the same day must NEVER false-match
on their shared callsign tokens. These DB-gated tests seed callsign-use names directly and
assert they contribute ZERO blocking pairs and ZERO scoring features — while a real name on
the same chart still blocks and scores normally (the exclusion is placeholder-only, not
name-wide). Gated on CAIRN_TEST_PG.
"""

import uuid

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def _pairs(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs

    pairs, _skipped = generate_candidate_pairs(conn, **kw)
    return pairs


def test_two_johndoes_sharing_only_a_callsign_token_generate_no_pair(pg_conn):
    # Both callsigns share the tokens "unknown", "ed", "site1", "2026-07-03" — a naive
    # tokenizer would block them together. The placeholder exclusion must drop them entirely.
    seed_patient(pg_conn, PA, callsign_names=[("Unknown-ed-site1-2026-07-03-00ab", 10)])
    seed_patient(pg_conn, PB, callsign_names=[("Unknown-ed-site1-2026-07-03-77cd", 10)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn), (
        "two John Does must never block-match on shared callsign tokens (§5.4)"
    )


def test_callsign_is_excluded_from_the_scoring_feature_space(pg_conn):
    from cairn_matcher.pipeline.db import load_candidate

    seed_patient(pg_conn, PA, callsign_names=[("Unknown-ed-site1-2026-07-03-00ab", 10)])
    rec = load_candidate(pg_conn, uuid.UUID(PA))
    # build_names returns None when no non-placeholder name rows survive the exclusion.
    assert rec.names is None, "a callsign contributes no name feature to the scorer (§5.4)"


def test_a_real_name_on_a_johndoe_chart_still_blocks_and_scores(pg_conn):
    # A chart can hold BOTH a callsign AND a real name (e.g. once partially identified).
    # The exclusion is placeholder-only: the real token must still generate the pair.
    from cairn_matcher.pipeline.db import load_candidate

    seed_patient(
        pg_conn,
        PA,
        names=[("Alex Smith", 20)],
        callsign_names=[("Unknown-ed-site1-2026-07-03-00ab", 10)],
    )
    seed_patient(pg_conn, PB, names=[("Alex Jones", 20)])
    # Blocking: the shared real token "alex" still pairs them.
    assert canonical_pair(PA, PB) in _pairs(pg_conn)
    # Scoring: the real name survives; the callsign does not appear among the features.
    rec = load_candidate(pg_conn, uuid.UUID(PA))
    assert rec.names is not None
    # rec.names.value is a frozenset[Name]; each Name.tokens is {role: (token, ...)}.
    tokens = {
        tok
        for name in rec.names.value
        for group in name.tokens.values()
        for tok in group
    }
    assert "alex" in tokens, "the real name must remain a scoring feature"
    assert "unknown" not in tokens, "the callsign must not leak into the feature space"


def test_a_callsign_token_shared_with_a_real_name_does_not_pair(pg_conn):
    # Defense-in-depth: even if a real name on chart B happened to share a token with chart
    # A's CALLSIGN, that must not pair — A's callsign is excluded, so there is no A-side
    # token to match. (B's "site1" is a real, if odd, name token; A's is a callsign token.)
    seed_patient(pg_conn, PA, callsign_names=[("Unknown-ed-site1-2026-07-03-00ab", 10)])
    seed_patient(pg_conn, PB, names=[("Site1 Person", 20)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)
