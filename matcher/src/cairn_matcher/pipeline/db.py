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

# §5.4 placeholder-name exclusion. A "John Doe" chart carries a system-generated CALLSIGN
# (`Unknown-ED-<site>-<date>-<suffix>`, cairn-event::john_doe) as a real, displayed name so
# the header is never blank — but §5.4 requires the matcher EXCLUDE placeholder names from
# its feature space so two unidentified patients can never match via their callsigns. The
# carrier is the name's `use` facet (db/012 folds it to `use_key`); `callsign` is a
# system-set, culture-neutral reserved token. This is an ADVISORY exclusion (the matcher owns
# its feature space, §5.2/§5.13) — the callsign stays a normal name in `patient_name`; it is
# only withheld from SCORING (load_candidate) and BLOCKING (the name_tokens CTE) here.
#
# Which of the two is load-bearing: the SCORING exclusion. The blocking tokenizer splits on
# WHITESPACE and a callsign is hyphen-joined with none, so a whole callsign is a SINGLE
# token; two distinct callsigns thus never share a blocking token to begin with. The blocking
# exclusion earns its keep only in the rare IDENTICAL-callsign collision (same site/day/suffix
# — see cairn-node SUFFIX_HEX_LEN) and as cheap defense-in-depth; it is NOT what stops two
# ordinary John Does from grouping (they already don't). The scoring exclusion is what keeps
# a callsign out of the scorer's name feature.
#
# A reserved SET (not one literal) so a future placeholder kind joins by adding one member —
# additive, never a rewrite. This is the hand-maintained mirror of
# cairn-event::john_doe::CALLSIGN_USE, with NO mechanical guard coupling the two (a deferred
# item). Drift is NOT recall-safe in the dangerous direction: if a use Rust emits as a
# placeholder is MISSING here, those callsign names UNDER-exclude — they re-enter the feature
# space, and two same-site/same-day John Does can then block+score+auto-band into a FALSE
# MERGE (§5.2's "false merge >> false split"). So an addition on the Rust side MUST be
# mirrored here; the safe-failure framing ("only lost recall") does not hold for an omission.
PLACEHOLDER_NAME_USES = frozenset({"callsign"})
# The parameter form psycopg binds to a Postgres text[] for `use_key <> ALL(%s)`. Sorted so
# the bound array is deterministic (readability/log stability); order is irrelevant to ALL.
_PLACEHOLDER_USES_PARAM = sorted(PLACEHOLDER_NAME_USES)


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
        # Exclude placeholder-use names (callsigns) from the scoring feature space (§5.4).
        cur.execute("SELECT value, provenance_rank FROM patient_name "
                    "WHERE patient_id=%s AND use_key <> ALL(%s)",
                    (patient_id, _PLACEHOLDER_USES_PARAM))
        name_rows = cur.fetchall()
        cur.execute("SELECT system, match_key FROM patient_identifier WHERE patient_id=%s",
                    (patient_id,))
        identifier_rows = cur.fetchall()
    return candidate_from_rows(
        dob_row=dob_row, sex_row=sex_row, name_rows=name_rows, identifier_rows=identifier_rows
    )


def load_aliases(conn, patient_id) -> frozenset[str]:
    """Read one chart's repudiated known-alias name strings from patient_alias_pool (db/025).

    The view is reason-free (ADR-0006 confidentiality split): only the name `value` is
    exposed, never the forensic `reason`. The matcher recognises a returning fabricated
    persona (§5.5(a)) by these values; how a value got there (the C5 suppressing event
    floor) is not this module's concern — it only reads the projection.

    Single-pair path only. A BATCH driver must not call this per pair: it would re-fetch
    the same chart's aliases once per pair the chart appears in, and issue two empty SELECTs
    for every pair in the (overwhelmingly common) no-repudiation case. `load_aliases_for`
    below reads the whole candidate set once instead.
    """
    with conn.cursor() as cur:
        cur.execute("SELECT value FROM patient_alias_pool WHERE patient_id=%s", (patient_id,))
        return frozenset(row[0] for row in cur.fetchall())


def load_aliases_for(conn, patient_ids) -> dict[str, frozenset[str]]:
    """Read the repudiated known-aliases for a whole set of charts in ONE query.

    The batch counterpart to `load_aliases`. A sweep scores every candidate pair, and a
    chart appears in many pairs; loading its aliases per pair (as `load_aliases` does) is
    redundant I/O, and in the common no-repudiation case it is two empty SELECTs on every
    pair. Instead the sweep pre-loads the aliases for its whole candidate-patient set once
    and hands the lookup to `propose` (which then hits the DB zero extra times per pair).

    Scoped to the given `patient_ids` (never the fleet-wide pool, which grows with every
    repudiation ever synced): the `WHERE patient_id = ANY(...)` probes the base table's
    (subject, value) PK on its leading `subject` column, so this stays an index probe.
    Charts with no repudiated alias are simply absent from the returned dict; the caller
    treats a miss as the empty frozenset. Keys are canonical lowercase uuid text, matching
    `str(patient_id)` at the call site.
    """
    ids = [str(p) for p in patient_ids]
    if not ids:
        return {}
    out: dict[str, set[str]] = {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id, value FROM patient_alias_pool WHERE patient_id = ANY(%s::uuid[])",
            (ids,),
        )
        for pid, value in cur.fetchall():
            out.setdefault(str(pid), set()).add(value)
    return {pid: frozenset(values) for pid, values in out.items()}


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
# single-token block, which the cap would otherwise drop wholesale). Birth-year is the
# FIRST 4-consecutive-digit run in the stored DOB value (`substring(value FROM '[0-9]{4}')`,
# guarded by `value ~ '[0-9]{4}'`) -- an honest, culture-neutral degrade that parses no date
# and assumes no calendar (principle 4). The 4-digit-run (not leading-4) extraction means a
# day-first import ("12/05/1990") and an ISO value ("1990-05-12") for the same person both
# yield "1990" and group together; a value with no 4-digit run (a 2-digit year "07/15/80",
# a null DOB) simply does not join this pass and stays covered by the single-token 'name'
# pass -- never a false group, only a withheld rescue. Because the run ignores month/day,
# this pass also groups precision-mismatched true matches ("1990" vs "1990-05-12") that the
# exact-DOB pass never groups. This is advisory: a mis-extracted year only ever feeds the
# Python scorer a few extra pairs (which it rejects), never an auto-link, so erring toward
# more grouping is safe. Real-world extraction adequacy is to be revisited on richer data.
_GROUPS_SQL = """
WITH name_tokens AS (
    -- normalize(value, NFC) so a name recorded decomposed (NFD) on one feed and
    -- precomposed (NFC) on another produces the SAME blocking token — otherwise the two
    -- are different code points and a true duplicate is never even grouped. Mirrors the
    -- adapter's _normalize_token (NFC) on the Python comparison side.
    -- Exclude placeholder-use names (callsigns) from BLOCKING (§5.4). A callsign is a
    -- single whitespace-free token, so this bites only when two callsign STRINGS are
    -- identical (the rare same-suffix collision) — defense-in-depth, not what keeps two
    -- ordinary John Does apart (distinct callsigns are already distinct tokens; the
    -- load-bearing exclusion is the scoring one in load_candidate). The `name+year` pass
    -- reads this same CTE, so it inherits the exclusion for free.
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(normalize(value, NFC)), '\\s+') AS token
    WHERE token <> '' AND use_key <> ALL(%s)
),
birth_year AS (
    SELECT patient_id, substring(value FROM '[0-9]{4}') AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '[0-9]{4}'
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
        # The single %s binds the placeholder-use exclusion in the name_tokens CTE.
        cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM,))
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
