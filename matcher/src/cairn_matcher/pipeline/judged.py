# matcher/src/cairn_matcher/pipeline/judged.py
"""Pairs a human (or the identity algebra) has already judged — never proposed again.

ADR-0076's skip rule for the commit-time check: a pair already in ONE link component is the same
person already; a pair with ANY patient_link row has been judged — a link, or a "not the same
person" unlink (decision 4). Proposing either again would put a settled question back on the
worklist. A pending proposal that predates a judgement is filtered from the worklist by R5's view,
not here.

Requires the optional `pipeline` extra (psycopg) at call time, except drop_judged (pure).
"""

import uuid

from cairn_matcher.pipeline.blocking import canonical_pair


def judged_partners(conn, patient) -> frozenset[str]:
    """Every chart already judged against `patient` (its component, and any link-row partner).

    Includes `patient` itself (cairn_person_charts always returns the chart), which is harmless:
    a self-pair is never generated.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT c::text FROM cairn_person_charts(%s::uuid) AS c "
            "UNION SELECT (CASE WHEN low = %s::uuid THEN high ELSE low END)::text "
            "FROM patient_link WHERE low = %s::uuid OR high = %s::uuid",
            (patient, patient, patient, patient),
        )
        return frozenset(r[0] for r in cur.fetchall())


def judged_pairs(conn) -> frozenset[tuple[str, str]]:
    """Every judged pair node-wide, canonical — for the bulk sweep's skip filter.

    Two members of one component are judged even with no direct link row between them (A–B and
    B–C linked make A–C the same person), so the component self-join is needed as well as the
    link rows.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT a.patient_id::text, b.patient_id::text FROM person_member a "
            "JOIN person_member b ON a.person_id = b.person_id AND a.patient_id < b.patient_id "
            "UNION SELECT low::text, high::text FROM patient_link"
        )
        return frozenset(canonical_pair(x, y) for x, y in cur.fetchall())


def drop_judged(pairs, patient, partners) -> list[tuple[str, str]]:
    """The pairs whose OTHER side is not in `partners` (pure).

    `pairs` and `partners` hold canonical ids (lowercase, hyphenated: canonical_pair's form).
    `patient` may arrive spelled any way uuid.UUID accepts (braced, upper-case, unhyphenated), so
    it is canonicalised first. Without that, `p[0] == me` never holds for a differently spelled
    id, the chart's OWN side reads as "the other", and -- since a chart is always its own partner
    -- every pair where it is the low side is dropped as judged.
    """
    me = str(uuid.UUID(str(patient)))
    return [p for p in pairs if (p[1] if p[0] == me else p[0]) not in partners]
