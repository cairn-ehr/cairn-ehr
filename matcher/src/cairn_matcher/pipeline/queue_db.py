# matcher/src/cairn_matcher/pipeline/queue_db.py
"""The worker's SQL over db/056's notice log and state row (repair path R4).

None of these commit unless the docstring says so: check_chart and run_bulk own the transaction,
so a chart's proposals and the delete of its notices land together or not at all.
"""

_ALL_CHARTS_SQL = (
    "SELECT patient_id FROM patient_chart UNION SELECT patient_id FROM patient_name "
    "UNION SELECT patient_id FROM patient_demographic "
    "UNION SELECT patient_id FROM patient_identifier"
)


def charts_waiting(conn) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT count(DISTINCT patient_id) FROM match_pending")
        return cur.fetchone()[0]


def next_charts(conn, limit: int, exclude: list[str]) -> list[tuple[str, int]]:
    """Up to `limit` waiting charts, NEWEST change first, as (patient, highest notice id).

    Newest first so a fresh registration is checked within seconds even behind a restore's
    backlog. `exclude` is the RetryBook's held charts, so a poison chart never starves the rest.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id::text, max(id) FROM match_pending "
            "WHERE NOT (patient_id = ANY(%s::uuid[])) "
            "GROUP BY patient_id ORDER BY max(id) DESC LIMIT %s",
            (exclude, limit),
        )
        return [(p, int(i)) for p, i in cur.fetchall()]


def pending_pairs_involving(conn, patient) -> list[tuple[str, str]]:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_low::text, patient_high::text FROM match_proposal "
            "WHERE status = 'pending' AND (patient_low = %s::uuid OR patient_high = %s::uuid)",
            (patient, patient),
        )
        return [(lo, hi) for lo, hi in cur.fetchall()]


def clear_chart(conn, patient, upto_id: int) -> int:
    """Delete the chart's notices up to the highest id the check READ — never by patient alone,
    which would lose a change that landed mid-check."""
    with conn.cursor() as cur:
        cur.execute("DELETE FROM match_pending WHERE patient_id = %s::uuid AND id <= %s",
                    (patient, upto_id))
        return cur.rowcount


def watermark(conn) -> int | None:
    with conn.cursor() as cur:
        cur.execute("SELECT max(id) FROM match_pending")
        return cur.fetchone()[0]


def clear_upto(conn, upto_id: int, keep: list[str]) -> int:
    """After a bulk sweep: delete every notice up to the watermark EXCEPT the charts in `keep`
    (charts in a pair the sweep failed to score — they stay "not yet checked")."""
    with conn.cursor() as cur:
        cur.execute(
            "DELETE FROM match_pending WHERE id <= %s AND NOT (patient_id = ANY(%s::uuid[]))",
            (upto_id, keep),
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
