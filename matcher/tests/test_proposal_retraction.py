"""§5.2 stale-proposal retraction when a pair drops below the review floor (issue #135, DB-gated).

The §5.4 forcing rule persists a REVIEW proposal conditioned on `chart_trust='unconfirmed'`
— a state designed to be TRANSIENT (every John Doe is meant to be identified). Before this
fix, once the Doe was identified the pair banded None on the next sweep and `propose()`
simply rolled back, leaving a permanent `band='review'`, `status='pending'` row whose
`identity_pending` marker misreported a now-resolved chart on the hub worklist forever.

The fix: when a pair bands None, `propose()` retracts a surviving PENDING row
(status -> 'retracted') — append-only-friendly (no DELETE), preserving any human/applied
disposition, and reversible: a genuine later re-proposal reverts 'retracted' -> 'pending'.

Gated on CAIRN_TEST_PG via pg_conn's internal skip; psycopg-touching imports stay inside
the test functions so the pure `uv run pytest` run still collects this module cleanly.
"""

import uuid

from tests.conftest import seed_identity_pending, seed_patient


def _uid() -> str:
    return str(uuid.uuid4())


def _seed_forced_review_pair(conn) -> tuple[str, str]:
    """Seed the #130/#135 pair: a callsign-only Doe (estimated-age range + observed sex)
    vs a prior chart. Scores ≈1.79 (below review=3.0); only the unconfirmed-chart forcing
    rule surfaces it. Identical to test_identity_pending_pipeline's headline shape."""
    doe, prior = _uid(), _uid()
    seed_patient(
        conn, doe,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-XX-20260705-abcd1234", 30)],
    )
    seed_patient(
        conn, prior,
        dob=("1985-03-12", 60, "day"),
        sex=("male", 60),
        names=[("Robert Menzies", 60)],
    )
    return doe, prior


def _identify(conn, subject) -> None:
    """Simulate the clinician identifying the Doe: the chart is no longer flagged
    unconfirmed, so its chart_identity_state row is gone (confirmed = absent-row default).
    Bypasses the C4 `identify` floor on purpose — these tests exercise CONSUMPTION of the
    resolved trust state, the same rationale as conftest.seed_identity_pending."""
    with conn.cursor() as cur:
        cur.execute("DELETE FROM chart_identity_state WHERE subject=%s", (subject,))
    conn.commit()


def _proposal_status(conn, low, high) -> str | None:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT status FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        row = cur.fetchone()
    return row[0] if row else None


def test_forced_review_proposal_is_retracted_once_the_doe_is_identified(pg_conn):
    """The headline #135 fix: a pending forced-REVIEW row transitions to 'retracted'
    (not deleted, not left pending) once the Doe is identified and re-proposed."""
    from cairn_matcher.pipeline.banding import Band
    from cairn_matcher.pipeline.runner import canonical_pair, propose

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)

    # Sweep 1 equivalent: the forcing rule surfaces the sub-threshold pair as REVIEW.
    assert propose(pg_conn, doe, prior) is Band.REVIEW
    low, high = canonical_pair(doe, prior)
    assert _proposal_status(pg_conn, low, high) == "pending"

    # The Doe is identified; the transient forcing condition is gone.
    _identify(pg_conn, doe)

    # Sweep 2 equivalent: the pair now bands None -> the stale pending row is retracted.
    assert propose(pg_conn, doe, prior) is None
    assert _proposal_status(pg_conn, low, high) == "retracted", (
        "the now-resolved pending row must be retracted, not left on the worklist"
    )


def test_retraction_preserves_a_human_disposition(pg_conn):
    """A human's decision (accepted/rejected/applied) must NEVER be overwritten by
    retraction — only still-'pending' advisory rows transition."""
    from cairn_matcher.pipeline.runner import canonical_pair, propose

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)
    propose(pg_conn, doe, prior)
    low, high = canonical_pair(doe, prior)

    # A reviewer accepts the proposal.
    with pg_conn.cursor() as cur:
        cur.execute(
            "UPDATE match_proposal SET status='accepted' "
            "WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
    pg_conn.commit()

    _identify(pg_conn, doe)
    assert propose(pg_conn, doe, prior) is None
    assert _proposal_status(pg_conn, low, high) == "accepted", (
        "a human disposition must survive a band-None re-proposal"
    )


def test_retracted_proposal_reverts_to_pending_on_a_genuine_reproposal(pg_conn):
    """Retraction is reversible: if the pair later legitimately bands non-None again,
    the row re-surfaces as 'pending' (a resurrected match must not stay hidden)."""
    from cairn_matcher.pipeline.banding import Band
    from cairn_matcher.pipeline.runner import canonical_pair, propose

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)
    propose(pg_conn, doe, prior)
    low, high = canonical_pair(doe, prior)

    _identify(pg_conn, doe)
    assert propose(pg_conn, doe, prior) is None
    assert _proposal_status(pg_conn, low, high) == "retracted"

    # The chart is flagged unconfirmed again (e.g. a fresh Doe episode): the forcing rule
    # re-surfaces the pair, and the retracted row must come back onto the worklist.
    seed_identity_pending(pg_conn, doe)
    assert propose(pg_conn, doe, prior) is Band.REVIEW
    assert _proposal_status(pg_conn, low, high) == "pending", (
        "a genuinely re-proposed pair must revert from 'retracted' to 'pending'"
    )


def test_sweep_retracts_forced_review_after_identification_end_to_end(pg_conn):
    """The full issue #135 scenario through the real sweep(): sweep 1 surfaces the forced
    REVIEW, the Doe is identified, and sweep 2 retracts the now-stale pending row."""
    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _seed_forced_review_pair(pg_conn)
    seed_identity_pending(pg_conn, doe)

    assert sweep(pg_conn).errors == []
    low, high = canonical_pair(doe, prior)
    assert _proposal_status(pg_conn, low, high) == "pending"

    _identify(pg_conn, doe)

    result = sweep(pg_conn)
    assert result.errors == []
    # Guard against a vacuous pass: the pair must still be GENERATED in sweep 2 (it blocks
    # on the same dob-range window), otherwise propose() would never run and this would
    # pass for the wrong reason.
    assert result.generated >= 1
    assert _proposal_status(pg_conn, low, high) == "retracted"
