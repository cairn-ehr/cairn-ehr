# matcher/src/cairn_matcher/pipeline/queue_db.py
"""The worker's SQL over db/056's notice log and state row (repair path R4).

None of these commit unless the docstring says so: check_chart and run_bulk own the transaction,
so a chart's proposals and the delete of its notices land together or not at all.
"""

# The statuses the MATCHER may still revise: no human has decided them. 'pending' is the
# matcher's own proposal; 'review' is auto_apply.rs's machine kick (a veto appeared, or another
# writer's un-attested unlink stands — ADR-0078). A human's 'accepted'/'rejected'/'applied' and
# the matcher's 'auto_applied' are never revised here (#743 part 1).
AWAITING_HUMAN = ("pending", "review")

_ALL_CHARTS_SQL = (
    "SELECT patient_id FROM patient_chart UNION SELECT patient_id FROM patient_name "
    "UNION SELECT patient_id FROM patient_demographic "
    "UNION SELECT patient_id FROM patient_identifier"
)


def charts_waiting(conn) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT count(DISTINCT patient_id) FROM match_pending")
        return cur.fetchone()[0]


def next_charts(conn, limit: int, exclude: list[str]) -> list[str]:
    """Up to `limit` waiting charts, NEWEST change first (patient uuid text).

    Newest first so a fresh registration is checked next, ahead of older changes (the worker
    re-reads the queue after every chart, Settings.batch = 1). At 10 000 charts one check takes
    ~0.8 s (p50; p95 1.4 s) on the development machine (#725), and a sweep already running
    finishes first.
    `exclude` is the RetryBook's held charts, so a poison chart never starves the rest.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id::text FROM match_pending "
            "WHERE NOT (patient_id = ANY(%s::uuid[])) "
            "GROUP BY patient_id ORDER BY max(id) DESC LIMIT %s",
            (exclude, limit),
        )
        return [p for (p,) in cur.fetchall()]


def awaiting_pairs_involving(conn, patient) -> list[tuple[str, str]]:
    """Proposals involving `patient` that the matcher may still revise (AWAITING_HUMAN)."""
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_low::text, patient_high::text FROM match_proposal "
            "WHERE status = ANY(%s) AND (patient_low = %s::uuid OR patient_high = %s::uuid)",
            (list(AWAITING_HUMAN), patient, patient),
        )
        return [(lo, hi) for lo, hi in cur.fetchall()]


def notice_ids(conn, patient=None) -> list[int]:
    """The notice ids VISIBLE right now, for one chart or (patient=None) for all.

    Capture these BEFORE reading the projections, then delete exactly them with
    clear_notices. Never delete by `id <= highest`: a bigserial is assigned at INSERT, not
    commit, so a slow transaction can hold a LOWER id and commit mid-check; an `id <=` delete
    would remove that notice although the check never saw its change.
    """
    with conn.cursor() as cur:
        if patient is None:
            cur.execute("SELECT id FROM match_pending ORDER BY id")
        else:
            cur.execute("SELECT id FROM match_pending WHERE patient_id = %s::uuid ORDER BY id",
                        (patient,))
        return [int(i) for (i,) in cur.fetchall()]


def clear_notices(conn, ids: list[int], keep=()) -> int:
    """Delete exactly the notices in `ids`, except those of the charts in `keep` (charts in a
    pair a sweep failed to score stay "not yet checked"). Does NOT commit."""
    with conn.cursor() as cur:
        cur.execute(
            "DELETE FROM match_pending WHERE id = ANY(%s::bigint[]) "
            "AND NOT (patient_id = ANY(%s::uuid[]))",
            (list(ids), list(keep)),
        )
        return cur.rowcount


def stamp_drained(conn) -> None:
    with conn.cursor() as cur:
        cur.execute("UPDATE match_worker_state SET last_drained_at = clock_timestamp()")


def ensure_version(conn, version: str) -> bool:
    """Record the running matcher_version; when it differs (or no worker ever ran), queue every
    chart for a 'config' re-check. One transaction; COMMITS. Returns True when it re-queued.

    The first run on a node also lands here (no state row): charts created before db/056 have no
    notices and have never been checked.
    """
    with conn.cursor() as cur:
        cur.execute("SELECT matcher_version FROM match_worker_state FOR UPDATE")
        row = cur.fetchone()
        if row is not None and row[0] == version:
            conn.rollback()
            return False
        cur.execute(f"INSERT INTO match_pending (patient_id, reason) "
                    f"SELECT patient_id, 'config' FROM ({_ALL_CHARTS_SQL}) charts")
        cur.execute(
            "INSERT INTO match_worker_state (singleton, matcher_version) VALUES (TRUE, %s) "
            "ON CONFLICT (singleton) DO UPDATE SET matcher_version = EXCLUDED.matcher_version",
            (version,),
        )
    conn.commit()
    return True
