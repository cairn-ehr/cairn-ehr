# matcher/tests/test_john_doe_exclusion.py
"""§5.4 — the matcher excludes placeholder (callsign) names from its feature space.

A "John Doe" chart carries a system-generated callsign as a real, displayed name
(use_key='callsign'). §5.4 requires that name be invisible to the matcher, so two different
unidentified patients can never match via their callsigns. These DB-gated tests seed
callsign-use names directly and assert they contribute ZERO blocking pairs and ZERO scoring
features — while a real name on the same chart still blocks and scores normally (the
exclusion is placeholder-only, not name-wide). Gated on CAIRN_TEST_PG.

Tokenization note (why the blocking tests are shaped as they are): the blocking tokenizer
splits a name on WHITESPACE (`regexp_split_to_table(..., '\\s+')`), and a callsign
(`Unknown-ed-site1-2026-07-03-<suffix>`) is hyphen-joined with NO whitespace — so a whole
callsign is a SINGLE blocking token, not the separate words "unknown"/"ed"/"site1". Two
callsigns therefore only ever share a blocking token when the callsign STRINGS are identical
(the rare same-suffix collision). To actually exercise the blocking exclusion (rather than
pass vacuously on distinct-token charts) these tests force that shared-token condition: an
identical callsign on two charts, and a real name equal to another chart's callsign token.
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


def test_two_johndoes_with_an_identical_callsign_generate_no_pair(pg_conn):
    # The blocking tokenizer splits on whitespace, so a whole callsign is ONE token. Two
    # callsigns collide on a blocking token only when the STRINGS are identical (the rare
    # same-suffix collision). Force that collision: without the placeholder exclusion this
    # shared token would block the pair; with it, both callsign rows are dropped from
    # name_tokens so no token — and no pair — is ever produced.
    same = "Unknown-ed-site1-2026-07-03-00ab"
    seed_patient(pg_conn, PA, callsign_names=[(same, 10)])
    seed_patient(pg_conn, PB, callsign_names=[(same, 10)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn), (
        "two John Does must never block-match via their callsigns, even when identical (§5.4)"
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


def test_a_real_name_equal_to_a_callsign_token_does_not_pair(pg_conn):
    # Defense-in-depth: a callsign is a single whitespace-free token, so the only way a real
    # name could collide with chart A's callsign is by being that exact one-token string.
    # Force it: chart B's LEGAL name is the identical string. Without the exclusion this
    # shared token would block the pair; with it, A's callsign is dropped so there is no
    # A-side token to match, and no pair is produced.
    callsign = "Unknown-ed-site1-2026-07-03-00ab"
    seed_patient(pg_conn, PA, callsign_names=[(callsign, 10)])
    seed_patient(pg_conn, PB, names=[(callsign, 20)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)
