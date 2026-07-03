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


# --- batch alias preload (the sweep path) -----------------------------------------------
# A sweep must not re-fetch a chart's aliases once per pair; load_aliases_for reads the
# whole candidate set in one scoped query and propose() consumes that map.

C = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def test_load_aliases_for_batch_reads_every_chart_in_one_dict(pg_conn):
    from cairn_matcher.pipeline.db import load_aliases_for

    seed_repudiation(pg_conn, A, FALSE_NAME)
    seed_repudiation(pg_conn, B, "Other Fake")
    got = load_aliases_for(pg_conn, [A, B, C])
    # Each requested chart's aliases, keyed by canonical uuid text; a chart with none absent.
    assert got == {A: frozenset({FALSE_NAME}), B: frozenset({"Other Fake"})}
    assert C not in got


def test_load_aliases_for_empty_input_returns_empty(pg_conn):
    from cairn_matcher.pipeline.db import load_aliases_for

    assert load_aliases_for(pg_conn, []) == {}


def test_propose_reads_preloaded_aliases_not_the_db(pg_conn, monkeypatch):
    # With a preloaded map, propose() must issue NO per-pair alias SELECT. Force
    # db.load_aliases to explode; the known_alias tag must still appear, proving the map path.
    from cairn_matcher.pipeline import db as db_mod

    seed_patient(pg_conn, A, names=[("Jane Realname", 20), (FALSE_NAME, 20)])
    seed_patient(pg_conn, B, names=[(FALSE_NAME, 20)])

    def _boom(*a, **k):
        raise AssertionError("propose() must not call load_aliases when aliases= is supplied")

    monkeypatch.setattr(db_mod, "load_aliases", _boom)
    preloaded = {A: frozenset({FALSE_NAME})}  # B carries none — a plain map miss
    result = propose(pg_conn, A, B, aliases=preloaded)
    assert result is Band.REVIEW

    _, evidence = _evidence(pg_conn)
    assert {"kind": "known_alias", "value": FALSE_NAME, "alias_of": A} in evidence
