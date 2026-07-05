"""§5.4 identity-pending trust plumbing + the #130 end-to-end (DB-gated).

Gated on CAIRN_TEST_PG via the pg_conn fixture's internal skip (house convention).
psycopg-touching imports stay INSIDE test functions so the pure `uv run pytest` run
(no psycopg installed) still collects this module cleanly.
"""
import uuid

from tests.conftest import seed_identity_pending, seed_patient


def _uid() -> str:
    return str(uuid.uuid4())


def test_load_trust_for_reads_pending_as_unconfirmed(pg_conn):
    from cairn_matcher.pipeline.db import load_trust, load_trust_for

    doe, ordinary = _uid(), _uid()
    seed_patient(pg_conn, doe, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, ordinary, dob=("1985-03-12", 60, "day"))
    seed_identity_pending(pg_conn, doe)

    trust = load_trust_for(pg_conn, [doe, ordinary])
    assert trust.get(doe) == "unconfirmed"
    assert ordinary not in trust  # absent row = confirmed default
    assert load_trust(pg_conn, doe) == "unconfirmed"
    assert load_trust(pg_conn, ordinary) is None


def test_load_trust_for_empty_set_is_empty(pg_conn):
    from cairn_matcher.pipeline.db import load_trust_for

    assert load_trust_for(pg_conn, []) == {}


def test_load_candidate_populates_administrative_sex(pg_conn):
    # The Task-3 widened SELECT, proven against live projection rows (spec §6 DB-gated).
    from cairn_matcher.pipeline.db import load_candidate

    doe = _uid()
    seed_patient(pg_conn, doe, admin_sex=("male", 30))
    rec = load_candidate(pg_conn, doe)
    assert rec.administrative_sex is not None
    assert rec.administrative_sex.value == "male"
    assert rec.administrative_sex.provenance_rank == 30
    assert rec.sex_at_birth is None


def test_pure_age_john_doe_pair_surfaces_as_review_end_to_end(pg_conn):
    # THE #130 headline: a callsign-only John Doe with clinician-observed evidence
    # (estimated-age range + observed administrative-sex, both rank 30) vs their prior
    # chart. Blocks via the anchored dob-range passes; scores ≈1.79 (below review=3.0);
    # the unconfirmed-chart rule forces REVIEW. Blocked -> scored -> SURFACED.
    import json

    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _uid(), _uid()
    seed_patient(
        pg_conn, doe,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-XX-20260705-abcd1234", 30)],
    )
    seed_patient(
        pg_conn, prior,
        dob=("1985-03-12", 60, "day"),
        sex=("male", 60),
        names=[("Robert Menzies", 60)],
    )
    seed_identity_pending(pg_conn, doe)

    result = sweep(pg_conn)

    assert result.errors == []
    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT band, evidence FROM match_proposal "
            "WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        row = cur.fetchone()
    assert row is not None, "the pure-age Doe pair must persist a proposal"
    band_value, evidence = row
    assert band_value == "review"
    entries = evidence if isinstance(evidence, list) else json.loads(evidence)
    marker = next(e for e in entries if e.get("rule") == "identity_pending")
    assert marker["unconfirmed"] == [str(uuid.UUID(doe))]


def test_without_pending_state_the_same_pair_stays_below_threshold(pg_conn):
    # The control: identical evidence, no identity-pending state -> the pair still
    # blocks and scores ≈1.79 but persists NOTHING. Proves the forcing rule (not the
    # sex scoring alone) is what surfaces the headline pair.
    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _uid(), _uid()
    seed_patient(
        pg_conn, doe,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-XX-20260705-abcd1234", 30)],
    )
    seed_patient(
        pg_conn, prior,
        dob=("1985-03-12", 60, "day"),
        sex=("male", 60),
        names=[("Robert Menzies", 60)],
    )

    sweep(pg_conn)

    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT 1 FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        assert cur.fetchone() is None


def test_two_john_does_marker_carries_both_uuids(pg_conn):
    # Two unconfirmed charts (two Does, two sites) whose windows overlap and whose
    # observed sex agrees: the identity_pending marker lists BOTH chart uuids (sorted),
    # so a worklist can group the pair under either Doe.
    import json

    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe_a, doe_b = _uid(), _uid()
    seed_patient(
        pg_conn, doe_a,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-AA-20260705-aaaa1111", 30)],
    )
    seed_patient(
        pg_conn, doe_b,
        dob=("1984/1994", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-BB-20260705-bbbb2222", 30)],
    )
    seed_identity_pending(pg_conn, doe_a)
    seed_identity_pending(pg_conn, doe_b)

    sweep(pg_conn)

    low, high = canonical_pair(doe_a, doe_b)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT band, evidence FROM match_proposal "
            "WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        row = cur.fetchone()
    assert row is not None, "the two-Doe pair must persist a proposal"
    band_value, evidence = row
    assert band_value == "review"
    entries = evidence if isinstance(evidence, list) else json.loads(evidence)
    marker = next(e for e in entries if e.get("rule") == "identity_pending")
    assert marker["unconfirmed"] == sorted([doe_a, doe_b])
