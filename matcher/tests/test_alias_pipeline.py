"""DB-gated: the matcher consumes patient_alias_pool end-to-end. Skips without CAIRN_TEST_PG.

Scenario (§5.5(a) returning fabricated persona): chart A presented under a false name that
was later established false — repudiated, struck from A's header, kept as a known alias. The
same persona returns and is registered as a NEW chart B under that same reused name. The
matcher must (a) read the alias from patient_alias_pool, (b) recognise B bears A's known
alias, and (c) persist a REVIEW proposal whose evidence carries the known_alias flag — the
paper-registry "known alias" note, surfaced to a human. Flag, never suppress, never auto-link.
"""

from cairn_matcher.pipeline.banding import Band
from cairn_matcher.pipeline.runner import propose
from tests.conftest import seed_patient, seed_repudiation

# NB: cairn_matcher.pipeline.db is imported lazily inside the tests that need it — importing
# it at module top would pull in psycopg at COLLECTION time, breaking the pure `uv run pytest`
# run (which has no psycopg). runner.propose imports db lazily for the same reason.

# A = the original chart carrying the repudiated alias; B = the returning presentation.
A = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
B = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
LOW, HIGH = (A, B) if A < B else (B, A)
FALSE_NAME = "John Fakename"


def _evidence(conn):
    with conn.cursor() as cur:
        cur.execute(
            "SELECT band, evidence FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
            (LOW, HIGH))
        return cur.fetchone()


def test_load_aliases_reads_patient_alias_pool(pg_conn):
    from cairn_matcher.pipeline.db import load_aliases

    seed_repudiation(pg_conn, A, FALSE_NAME)
    assert load_aliases(pg_conn, A) == frozenset({FALSE_NAME})
    assert load_aliases(pg_conn, B) == frozenset()


def test_returning_alias_persists_review_with_known_alias_evidence(pg_conn):
    # A: a real name + the repudiated alias still in its retained set (db/025 keeps it).
    seed_patient(pg_conn, A, names=[("Jane Realname", 20), (FALSE_NAME, 20)])
    seed_repudiation(pg_conn, A, FALSE_NAME)
    # B: the returning persona, registered under the reused false name only.
    seed_patient(pg_conn, B, names=[(FALSE_NAME, 20)])

    result = propose(pg_conn, A, B)
    assert result is Band.REVIEW  # surfaced for a human, never auto-linked

    band_value, evidence = _evidence(pg_conn)
    assert band_value == "review"
    alias_entries = [e for e in evidence if e.get("kind") == "known_alias"]
    assert alias_entries == [{"kind": "known_alias", "value": FALSE_NAME, "alias_of": A}]


def test_no_repudiation_means_no_known_alias_evidence(pg_conn):
    # Same names, but nothing repudiated -> ordinary name match, no known_alias tag.
    seed_patient(pg_conn, A, names=[(FALSE_NAME, 20)])
    seed_patient(pg_conn, B, names=[(FALSE_NAME, 20)])

    propose(pg_conn, A, B)
    row = _evidence(pg_conn)
    if row is not None:  # a name-only match may or may not cross the review threshold
        _, evidence = row
        assert not any(e.get("kind") == "known_alias" for e in evidence)
