"""R4 Task 3: a pair already judged — one component, or ANY patient_link row — is never proposed."""

import hashlib
import uuid

from cairn_matcher.pipeline.blocking import canonical_pair
from cairn_matcher.pipeline.judged import drop_judged, judged_pairs, judged_partners

A, B, C, D = (str(uuid.UUID(int=i)) for i in (1, 2, 3, 4))


def _link(conn, x, y, state):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address) VALUES (%s,%s,%s,1,0,'seed','test:link',%s)",
            (low, high, state, b"\x12\x20" + hashlib.sha256(f"{low}{high}".encode()).digest()))
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


def test_partners_are_the_component_and_every_link_row_either_state(pg_conn):
    # A–B linked (one component A,B,C via B–C); A–D a standing human unlink.
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link")
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink")
    assert judged_partners(pg_conn, A) == frozenset({A, B, C, D})
    assert judged_partners(pg_conn, D) == frozenset({D, A})


def test_judged_pairs_cover_transitive_members_and_unlinks(pg_conn):
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link")
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink")
    want = {canonical_pair(x, y) for x, y in [(A, B), (B, C), (A, C), (A, D)]}
    assert judged_pairs(pg_conn) == frozenset(want)
