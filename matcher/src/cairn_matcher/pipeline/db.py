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
from cairn_matcher.pipeline.blocking import pairs_from_anchor, resolve_enabled_passes
from cairn_matcher.placeholder_uses import PLACEHOLDER_USES_PARAM
from cairn_matcher.records import CandidateRecord

# §5.4 placeholder-name exclusion. A "John Doe" chart carries a system-generated CALLSIGN
# (`Unknown-ED-<site>-<date>-<suffix>`, cairn-event::john_doe) as a real, displayed name so
# the header is never blank — but §5.4 requires the matcher EXCLUDE placeholder names from
# its feature space so two unidentified patients can never match via their callsigns. This is
# an ADVISORY exclusion (the matcher owns its feature space, §5.2/§5.13) — the callsign stays a
# normal name in `patient_name`; it is only withheld from SCORING (load_candidate) and BLOCKING
# (the name_tokens CTE) here.
#
# Which of the two is load-bearing: the SCORING exclusion. The blocking tokenizer splits on
# WHITESPACE and a callsign is hyphen-joined with none, so a whole callsign is a SINGLE
# token; two distinct callsigns thus never share a blocking token to begin with. The blocking
# exclusion earns its keep only in the rare IDENTICAL-callsign collision (same site/day/suffix
# — see cairn-node SUFFIX_HEX_LEN) and as cheap defense-in-depth; it is NOT what stops two
# ordinary John Does from grouping (they already don't). The scoring exclusion is what keeps
# a callsign out of the scorer's name feature.
#
# The reserved set (`PLACEHOLDER_NAME_USES`) and its bound-array form now live in the pure,
# psycopg-free `cairn_matcher.placeholder_uses` module — the single source of truth shared with
# the pure synthetic-eval mirror (`eval/generator.py`), which could not import it from here.
# The Rust↔Python drift guard lives in `tests/test_placeholder_uses_sync.py` (see that module
# for why an omission here is a FALSE-MERGE hazard, not merely lost recall).
_PLACEHOLDER_USES_PARAM = PLACEHOLDER_USES_PARAM


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
    -- year-range values are EXCLUDED: "1981/1991" would otherwise leak its first
    -- 4-digit run (1981) into name+year as if it were a birth year -- a false key
    -- (the window min is not a birth year; principle 4). The anchored range passes
    -- (_RANGE_GROUPS_SQL) own ranges.
    SELECT patient_id, substring(value FROM '[0-9]{4}') AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
)
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
-- Known/deliberate overlap with the anchored 'dob-range' pass (_RANGE_GROUPS_SQL): two
-- charts whose dob VALUES happen to be the IDENTICAL range string (e.g. both "1981/1991")
-- group here by literal string equality, redundant with the anchored pass (which already
-- pairs any two overlapping ranges, identical or not). Harmless -- pair-set dedup in
-- generate_candidate_pairs makes the redundancy invisible to callers -- but it slightly
-- contaminates the A/B toggle (an 'off-range-passes' run can still surface such a pair via
-- this exact-match arm). Left as-is: revisit only if A/B purity between the symmetric and
-- anchored pass sets ever matters.
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

# The two ANCHORED birth-year-range passes (§5.4 slice: design 2026-07-04). Separate
# statement from _GROUPS_SQL because the pair semantics differ: these rows are
# (pass_name, anchor, members) and Python pairs ANCHOR x MEMBER only -- never member x
# member (see pipeline/blocking.py for why all-pairing a birth-year window would
# manufacture C(k,2) noise pairs).
#
# birth_window gives every chart an inclusive birth-year interval:
#   * range rows: facets precision 'year-range', value '<yyyy>/<yyyy>' (slice B's
#     estimated-age window). Guards mirror parse_dob's safe degrade -- a malformed or
#     inverted value is EXCLUDED (never a false group, only a withheld rescue).
#   * point rows: the existing first-4-digit-run rule -> [year, year]. year-range rows
#     are excluded from this branch so a range can never double-enter as a false point
#     [min, min].
# The overlap join (m.y_min <= a.y_max AND a.y_min <= m.y_max) anchored on is_range
# charts yields range<->point AND range<->range (two John Does, two sites -- the only
# key that pair can ever share) from the same predicate.
#
# blocking_sex is the UNION of a chart's sex-at-birth and administrative-sex values:
# recall-first (a trans patient whose administrative-sex matches the clinician's
# observation still groups even though sex-at-birth differs). 'dob-range+sex' is the
# additive RESCUE pass, mirroring name/name+year: in a big DB the plain window block
# exceeds the cap and is skipped+reported; intersecting with a shared sex value roughly
# halves it, so it fires within cap in more settings. Additive-only: a sex mismatch
# merely means the rescue does not fire -- the scorer never sees a suppression.
_RANGE_GROUPS_SQL = """
WITH birth_window AS (
    -- Evaluation-order-proof malformed-range guard: PostgreSQL does NOT guarantee
    -- WHERE-subexpression evaluation order, so a `split_part(...)::int` cast could be
    -- evaluated BEFORE the `value ~ '^[0-9]{4}/[0-9]{4}$'` regex guard that exists to
    -- filter it out -- and a non-numeric value ("about-forty") would then raise
    -- "invalid input syntax for type integer" and crash the whole sweep on exactly the
    -- input this guard exists to degrade safely. `substring(value FROM '^([0-9]{4})/')`
    -- returns NULL on non-match (never raises), so the cast of NULL is safe and the
    -- comparison against NULL is not-true (row filtered) regardless of evaluation order.
    -- The regex guard is kept too (cheap, and documents intent) but correctness must not
    -- -- and no longer does -- depend on it being evaluated first.
    SELECT patient_id,
           substring(value FROM '^([0-9]{4})/')::int AS y_min,
           substring(value FROM '/([0-9]{4})$')::int AS y_max,
           TRUE AS is_range
    FROM patient_demographic
    WHERE field = 'dob'
      AND facets ->> 'precision' = 'year-range'
      AND value ~ '^[0-9]{4}/[0-9]{4}$'
      AND substring(value FROM '^([0-9]{4})/')::int <= substring(value FROM '/([0-9]{4})$')::int
    UNION ALL
    SELECT patient_id,
           substring(value FROM '[0-9]{4}')::int,
           substring(value FROM '[0-9]{4}')::int,
           FALSE
    FROM patient_demographic
    WHERE field = 'dob'
      AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
),
blocking_sex AS (
    -- Exclude the 'unknown' sentinel (principle 4: no-data-is-never-agreement), mirroring
    -- adapter._VALUE_SENTINELS (the SAME sentinel set the Python scoring side treats as
    -- absent-value) and this file's own 'identifier' pass (`system <> 'unknown'`). Without
    -- this, two charts that BOTH merely recorded sex 'unknown' would share a blocking_sex
    -- row and the 'dob-range+sex' rescue would key on mutual ignorance rather than a real
    -- signal. If adapter._VALUE_SENTINELS ever grows beyond {'unknown'}, mirror the addition
    -- here too -- an omission only adds noise pairs / withholds a rescue, it never suppresses
    -- a true one, so drift is safe-direction but should still be kept in sync.
    SELECT DISTINCT patient_id, lower(value) AS sex
    FROM patient_demographic
    WHERE field IN ('sex-at-birth', 'administrative-sex')
      AND value IS NOT NULL AND lower(value) <> 'unknown'
),
window_overlap AS (
    SELECT a.patient_id AS anchor, m.patient_id AS member
    FROM birth_window a
    JOIN birth_window m
      ON m.patient_id <> a.patient_id
     AND m.y_min <= a.y_max
     AND a.y_min <= m.y_max
    WHERE a.is_range
)
SELECT 'dob-range' AS pass_name, anchor, array_agg(DISTINCT member) AS members
FROM window_overlap
GROUP BY anchor
UNION ALL
SELECT 'dob-range+sex', o.anchor, array_agg(DISTINCT o.member)
FROM window_overlap o
JOIN blocking_sex sa ON sa.patient_id = o.anchor
JOIN blocking_sex sm ON sm.patient_id = o.member AND sm.sex = sa.sex
GROUP BY o.anchor
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
    conn, *, max_block_size: int = 100, enabled_passes=None
) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]:
    """Generate canonical candidate pairs via the blocking passes, capping huge blocks.

    Six passes (see blocking.ALL_PASSES): four SYMMETRIC group passes (identifier /
    exact-DOB / name-token / name-token+birth-year, _GROUPS_SQL) and two ANCHORED
    birth-year-range passes (dob-range / dob-range+sex, _RANGE_GROUPS_SQL — the
    §5.4 range-blocking slice).

    `enabled_passes` is the A/B measurement toggle: None runs every pass; a set runs only
    the named ones (unknown names raise — see blocking.resolve_enabled_passes). Filtering
    happens on the returned rows' pass_name, so one run issues the same SQL regardless of
    the subset and the toggle can never change what a pass WOULD have produced.

    Returns (pairs, skipped_blocks). `pairs`: unique canonical (low, high) lowercase-uuid
    tuples from every enabled group with <= max_block_size members. `skipped_blocks`: the
    (pass_name, key, size) of each ENABLED group excluded for exceeding the cap — a block
    shared by hundreds of people is non-discriminating (a group of size k contributes
    C(k,2) pairs; an anchored block of size k contributes k-1), and the §5.13 hub
    duplicate-sweep is the declared backstop for what it drops.

    Read-only — opens a read transaction the CALLER must close (sweep does conn.rollback
    before its write loop, so a long sweep does not pin the xmin horizon).
    """
    enabled = resolve_enabled_passes(enabled_passes)
    pairs: set[tuple[str, str]] = set()
    skipped_blocks: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        # The single %s binds the placeholder-use exclusion in the name_tokens CTE.
        cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM,))
        for pass_name, key, members in cur.fetchall():
            if pass_name not in enabled:
                continue
            size = len(members)
            if size > max_block_size:
                skipped_blocks.append((pass_name, key, size))
            else:
                pairs.update(_pairs_from_members(members))
        # The anchored range passes: pairs are anchor x member ONLY. The cap counts the
        # WHOLE block (members + the anchor itself) so "block size" means the same thing
        # for both pair-generation shapes, and a skipped block is reported under the
        # anchor's uuid (its natural key).
        cur.execute(_RANGE_GROUPS_SQL)
        for pass_name, anchor, members in cur.fetchall():
            if pass_name not in enabled:
                continue
            size = len(members) + 1
            if size > max_block_size:
                skipped_blocks.append((pass_name, str(anchor), size))
            else:
                pairs.update(pairs_from_anchor(anchor, members))
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
