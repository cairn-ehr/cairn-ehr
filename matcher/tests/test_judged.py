"""R4 Task 3, refined by #741: a pair is never proposed when it is in one record or has an
ATTESTED patient_link row."""

import hashlib
import uuid

from cairn_matcher.pipeline.blocking import canonical_pair
from cairn_matcher.pipeline.judged import drop_judged, judged_pairs, judged_partners

A, B, C, D = (str(uuid.UUID(int=i)) for i in (1, 2, 3, 4))
E = str(uuid.UUID(int=5))


def _link(conn, x, y, state, attested=False):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address, attested) VALUES (%s,%s,%s,1,0,'seed','test:link',%s,%s)",
            (low, high, state, b"\x12\x20" + hashlib.sha256(f"{low}{high}".encode()).digest(),
             attested))
    conn.commit()


def _propose(conn, x, y):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (low, high))
    conn.commit()


def _member(conn, patient, person):
    with conn.cursor() as cur:
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s)",
                    (patient, person))
    conn.commit()


def test_drop_judged_keeps_only_pairs_whose_other_side_is_unjudged():
    pairs = [canonical_pair(A, B), canonical_pair(A, C), canonical_pair(A, D)]
    assert drop_judged(pairs, A, frozenset({B, C})) == [canonical_pair(A, D)]


def test_drop_judged_reads_the_other_side_when_the_patient_is_the_high_side():
    # D is the HIGH side of (A, D) and (C, D): the other side is the pair's low member.
    pairs = [canonical_pair(A, D), canonical_pair(C, D)]
    assert drop_judged(pairs, D, frozenset({A})) == [canonical_pair(C, D)]


def _spellings(patient):
    """The same uuid as a caller might hand it over: braced, upper-case, unhyphenated."""
    return ["{" + patient + "}", patient.upper(), uuid.UUID(patient).hex]


def test_drop_judged_reads_any_spelling_of_the_patient_as_the_same_chart():
    # The partners always include the chart itself (judged_partners). If `me` were compared as
    # spelled, `p[0] == me` would never hold, the chart's OWN side would be read as "the other",
    # and every pair where it is the low side would be dropped as judged -- half the check gone.
    pairs = [canonical_pair(A, B), canonical_pair(A, D)]
    for spelled in _spellings(A):
        assert drop_judged(pairs, spelled, frozenset({A, B})) == [canonical_pair(A, D)], spelled


def test_partners_are_the_component_and_every_attested_link_row(pg_conn):
    # A–B–C one record; A–D a human's (attested) unlink; A–E an agent's (un-attested) unlink.
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link", attested=True)
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink", attested=True)
    _link(pg_conn, A, E, "unlink", attested=False)
    assert judged_partners(pg_conn, A) == frozenset({A, B, C, D})
    assert judged_partners(pg_conn, D) == frozenset({D, A})
    assert judged_partners(pg_conn, E) == frozenset({E}), "nobody judged A–E"


def test_judged_pairs_cover_members_and_attested_unlinks_only(pg_conn):
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link", attested=True)
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink", attested=True)
    _link(pg_conn, A, E, "unlink", attested=False)
    want = {canonical_pair(x, y) for x, y in [(A, B), (B, C), (A, C), (A, D)]}
    assert judged_pairs(pg_conn) == frozenset(want)


def test_judged_is_exactly_what_db057_does_not_hold_open(pg_conn):
    """#741's drift guard: the matcher's skip rule and db/057's openness are ONE rule.

    Six pairs, one per case. A `pending` proposal on each. A pair is judged (never proposed)
    exactly when db/057 does NOT hold its proposal open — no case may be judged by one and open
    by the other, or a pair is silently never shown (#741) or proposed forever.

    The sixth case — an UN-attested link whose two charts read as one record — is judged even
    though no human attested anything: the pair is ONE record already, so there is nothing left
    to propose. That is exactly what ADR-0078's first title ("judged only by a human") got wrong;
    whether the machine's link is doubted is R1b's question, not this rule's.
    """
    p = [str(uuid.UUID(int=i)) for i in range(21, 33)]
    cases = {
        "one record": (p[0], p[1]),
        "attested link": (p[2], p[3]),
        "attested unlink": (p[4], p[5]),
        "un-attested unlink": (p[6], p[7]),
        "no row": (p[8], p[9]),
        "un-attested link, one record": (p[10], p[11]),
    }
    _member(pg_conn, p[0], p[0])
    _member(pg_conn, p[1], p[0])
    _member(pg_conn, p[2], p[2])
    _member(pg_conn, p[3], p[2])
    _link(pg_conn, p[2], p[3], "link", attested=True)
    _link(pg_conn, p[4], p[5], "unlink", attested=True)
    _link(pg_conn, p[6], p[7], "unlink", attested=False)
    _member(pg_conn, p[10], p[10])
    _member(pg_conn, p[11], p[10])
    _link(pg_conn, p[10], p[11], "link", attested=False)
    for x, y in cases.values():
        _propose(pg_conn, x, y)
    with pg_conn.cursor() as cur:
        cur.execute("SELECT patient_low::text, patient_high::text FROM match_proposal_open")
        still_open = {(lo, hi) for lo, hi in cur.fetchall()}
    pg_conn.rollback()
    judged = judged_pairs(pg_conn)
    for name, (x, y) in cases.items():
        pair = canonical_pair(x, y)
        assert (pair in judged) == (pair not in still_open), name
        assert (y in judged_partners(pg_conn, x)) == (pair in judged), name
    # And the expected split, so the guard cannot pass by both sides being wrong together.
    assert still_open == {canonical_pair(*cases["un-attested unlink"]),
                          canonical_pair(*cases["no row"])}
