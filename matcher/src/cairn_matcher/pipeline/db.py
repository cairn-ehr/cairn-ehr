# matcher/src/cairn_matcher/pipeline/db.py
"""The only Postgres-touching module in the matcher. Thin: it loads a patient's
projection rows, calls the in-DB veto floor, and upserts a proposal. All scoring and
banding logic lives in the pure modules; this module just moves data.

Requires the optional `pipeline` extra (psycopg). The pure core never imports it.
"""

import json
import uuid

from psycopg.rows import dict_row

from cairn_matcher.pipeline.adapter import candidate_from_rows
from cairn_matcher.pipeline.banding import ProposalPayload, VetoFinding
from cairn_matcher.records import CandidateRecord


def load_candidate(conn, patient_id) -> CandidateRecord:
    """Read one patient's matching-relevant projection rows and shape a CandidateRecord.

    Reads the winner rows (dob, sex-at-birth) and the retained sets (names, identifiers).
    Pure shaping is delegated to adapter.candidate_from_rows.
    """
    with conn.cursor(row_factory=dict_row) as cur:
        cur.execute("SELECT value, facets, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field='dob'", (patient_id,))
        dob_row = cur.fetchone()
        cur.execute("SELECT value, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field='sex-at-birth'", (patient_id,))
        sex_row = cur.fetchone()
        cur.execute("SELECT value, provenance_rank FROM patient_name WHERE patient_id=%s",
                    (patient_id,))
        name_rows = cur.fetchall()
        cur.execute("SELECT system, match_key FROM patient_identifier WHERE patient_id=%s",
                    (patient_id,))
        identifier_rows = cur.fetchall()
    return candidate_from_rows(
        dob_row=dob_row, sex_row=sex_row, name_rows=name_rows, identifier_rows=identifier_rows
    )


def match_veto(conn, a, b) -> list[VetoFinding]:
    """Call the safety-critical in-DB hard-veto floor (db/016) and return its rows.

    The matcher NEVER re-implements this; it only consults it. A pair with any finding
    cannot be auto-linked (banding enforces that).
    """
    with conn.cursor() as cur:
        cur.execute("SELECT veto_kind, severity, subject, detail FROM cairn_match_veto(%s, %s)",
                    (a, b))
        return [VetoFinding(*row) for row in cur.fetchall()]


# Each pass yields rows of (pass_name, key, members) so the cap can be applied uniformly:
# a group is kept (pairs generated) iff cardinality(members) <= cap, else reported skipped.
# Blocking is RECALL-oriented and advisory: the SQL name tokenizer is deliberately simple
# (lower + whitespace split); the Python scorer remains the source of truth for comparison.
#
# The 'name+year' pass is a COMPOUND key (name token + birth-year). It is ADDITIVE: the
# single-token 'name' pass is retained, and pairs are deduped by canonical uuid pair across
# passes, so adding this pass can only RAISE recall (it rescues pairs from an oversized
# single-token block, which the cap would otherwise drop wholesale). Birth-year is taken as
# the leading 4 digits of the stored DOB value ONLY when the value begins with 4 digits
# (`value ~ '^[0-9]{4}'`) -- an honest, culture-neutral degrade that parses no date and
# assumes no calendar (principle 4); a record with a null/non-ISO DOB simply does not join
# this pass and stays covered by the single-token 'name' pass. Because left() truncates,
# this pass also groups precision-mismatched true matches ("1990" vs "1990-05-12") that the
# exact-DOB pass never groups.
_GROUPS_SQL = """
WITH name_tokens AS (
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(value), '\\s+') AS token
    WHERE token <> ''
),
birth_year AS (
    SELECT patient_id, left(value, 4) AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '^[0-9]{4}'
)
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'dob', value, array_agg(patient_id)
FROM patient_demographic WHERE field = 'dob'
GROUP BY value HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'name', token, array_agg(patient_id)
FROM name_tokens
GROUP BY token HAVING count(*) >= 2
UNION ALL
SELECT 'name+year', nt.token || '|' || byr.year, array_agg(nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY nt.token, byr.year HAVING count(DISTINCT nt.patient_id) >= 2
"""


def _pairs_from_members(members: list[str]) -> set[tuple[str, str]]:
    """Every canonical within-group pair (uuid value order), as lowercase-uuid-text.

    Pure: the same uuid ordering as runner.canonical_pair, so a pair has one identity no
    matter which group (or pass) surfaces it. Self-pairs are excluded by the strict order.

    Members are first normalized to canonical lowercase-hyphenated uuid text. In that form
    a plain string compare is order-equivalent to the 128-bit value compare (fixed width,
    lowercase hex, hyphens aligned) == runner.canonical_pair's uuid order — so we order by
    string and avoid re-parsing each uuid inside the O(k^2) inner loop.
    """
    ordered = sorted(str(uuid.UUID(str(m))) for m in members)
    out: set[tuple[str, str]] = set()
    for i, a in enumerate(ordered):
        for b in ordered[i + 1:]:
            out.add((a, b))
    return out


def generate_candidate_pairs(
    conn, *, max_block_size: int = 100
) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]:
    """Generate canonical candidate pairs via four blocking passes (identifier / exact-DOB / name-token / name-token+birth-year), capping huge blocks.

    Returns (pairs, skipped_blocks). `pairs`: unique canonical (low, high) lowercase-uuid
    tuples from every group with <= max_block_size members. `skipped_blocks`: the
    (pass_name, key, size) of each group EXCLUDED for exceeding the cap — a block shared
    by hundreds of people is non-discriminating (a group of size k contributes C(k,2)
    pairs), and the §5.13 hub duplicate-sweep is the declared backstop for what it drops.

    Read-only — opens a read transaction the CALLER must close (sweep does conn.rollback
    before its write loop, so a long sweep does not pin the xmin horizon).
    """
    pairs: set[tuple[str, str]] = set()
    skipped_blocks: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        cur.execute(_GROUPS_SQL)
        for pass_name, key, members in cur.fetchall():
            size = len(members)
            if size > max_block_size:
                skipped_blocks.append((pass_name, key, size))
            else:
                pairs.update(_pairs_from_members(members))
    return sorted(pairs), skipped_blocks


def upsert_proposal(conn, low, high, payload: ProposalPayload) -> None:
    """Write (or refresh) the advisory proposal for a canonical-ordered pair.

    Latest-wins on (patient_low, patient_high), but a non-'pending' status (a human's
    decision) is PRESERVED — a re-run refreshes the score/band/evidence, never a verdict.

    Does NOT commit. The caller owns the transaction boundary.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal "
            "(patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version) "
            "VALUES (%s,%s,%s,%s,%s,%s,%s) "
            "ON CONFLICT (patient_low, patient_high) DO UPDATE SET "
            "score_total=EXCLUDED.score_total, band=EXCLUDED.band, "
            "veto_findings=EXCLUDED.veto_findings, evidence=EXCLUDED.evidence, "
            "matcher_version=EXCLUDED.matcher_version, updated_at=clock_timestamp()",
            (low, high, payload.score_total, payload.band.value,
             json.dumps(list(payload.veto_findings)), json.dumps(list(payload.evidence)),
             payload.matcher_version),
        )
