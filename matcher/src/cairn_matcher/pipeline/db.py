# matcher/src/cairn_matcher/pipeline/db.py
"""The only Postgres-touching module in the matcher. Thin: it loads a patient's
projection rows, calls the in-DB veto floor, and upserts a proposal. All scoring and
banding logic lives in the pure modules; this module just moves data.

Requires the optional `pipeline` extra (psycopg). The pure core never imports it.
"""

import json
import uuid

from psycopg.rows import dict_row

from cairn_matcher.pipeline.adapter import VALUE_SENTINELS_PARAM, candidate_from_rows
from cairn_matcher.pipeline.banding import ProposalPayload, VetoFinding
from cairn_matcher.pipeline.blocking import (
    ANCHORED_PASSES,
    SYMMETRIC_PASSES,
    pairs_from_anchor,
    require_registered,
    resolve_enabled_passes,
)
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

    Reads the winner rows (dob, both sex facets) and the retained sets (names, identifiers).
    Pure shaping is delegated to adapter.candidate_from_rows.
    """
    with conn.cursor(row_factory=dict_row) as cur:
        cur.execute("SELECT value, facets, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field='dob'", (patient_id,))
        dob_row = cur.fetchone()
        # One query for BOTH sex facets (§4.2 sex-at-birth: the birth fact; §5.4
        # administrative-sex: the apparent/phenotypic facet a clinician-observed sex
        # lands on). Split by field name here — 'sex-at-birth'/'administrative-sex'
        # are the projection contract, NOT the scorer's weight key (that is "sex").
        cur.execute("SELECT field, value, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field IN ('sex-at-birth','administrative-sex')",
                    (patient_id,))
        sex_rows = cur.fetchall()
        sex_row = next((r for r in sex_rows if r["field"] == "sex-at-birth"), None)
        admin_sex_row = next((r for r in sex_rows if r["field"] == "administrative-sex"), None)
        # Exclude placeholder-use names (callsigns) from the scoring feature space (§5.4).
        cur.execute("SELECT value, provenance_rank FROM patient_name "
                    "WHERE patient_id=%s AND use_key <> ALL(%s)",
                    (patient_id, _PLACEHOLDER_USES_PARAM))
        name_rows = cur.fetchall()
        cur.execute("SELECT system, match_key FROM patient_identifier WHERE patient_id=%s",
                    (patient_id,))
        identifier_rows = cur.fetchall()
    return candidate_from_rows(
        dob_row=dob_row, sex_row=sex_row, name_rows=name_rows, identifier_rows=identifier_rows,
        admin_sex_row=admin_sex_row
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


def load_trust_for(conn, patient_ids) -> dict[str, str]:
    """§5.7 trust states for a candidate set in ONE query (the load_aliases_for pattern).

    The chart_trust view (db/024) carries rows ONLY for flagged charts (unconfirmed /
    under-review), so an absent key IS the confirmed default — mirrored by
    person_chart_trust's COALESCE. Keys are canonical lowercase uuid text. Scoped to the
    given ids so this stays an index probe, never a scan of every flagged chart in the
    fleet. The single-pair path is the same function over a two-element set — there is
    deliberately no separate singular loader, so this contract lives in exactly one place.
    """
    ids = [str(p) for p in patient_ids]
    if not ids:
        return {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id, trust_state FROM chart_trust WHERE patient_id = ANY(%s::uuid[])",
            (ids,),
        )
        return {str(pid): state for pid, state in cur.fetchall()}


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
# Shared blocking CTE fragments. Both statements (_GROUPS_SQL, _RANGE_GROUPS_SQL) need
# overlapping CTEs, so each CTE body lives ONCE here and the statements compose from these
# constants. This is not premature abstraction: blocking_sex is a load-bearing, sentinel-bound
# normalization, and the module was bitten once by a hand-mirrored sex literal lagging the
# adapter (see the comment inside _BLOCKING_SEX_CTE). Each constant is a CTE BODY only
# ("name AS ( ... )"); the composing statement supplies the leading WITH and comma joins.

_NAME_TOKENS_CTE = """name_tokens AS (
    -- normalize(value, NFC) so a name recorded decomposed (NFD) on one feed and
    -- precomposed (NFC) on another produces the SAME blocking token — otherwise the two
    -- are different code points and a true duplicate is never even grouped. Mirrors the
    -- adapter's _normalize_token (NFC) on the Python comparison side.
    -- Exclude placeholder-use names (callsigns) from BLOCKING (§5.4). A callsign is a
    -- single whitespace-free token, so this bites only when two callsign STRINGS are
    -- identical (the rare same-suffix collision) — defense-in-depth, not what keeps two
    -- ordinary John Does apart (distinct callsigns are already distinct tokens; the
    -- load-bearing exclusion is the scoring one in load_candidate). The name+year and
    -- dob+first-initial passes read this same CTE, so they inherit the exclusion for free.
    SELECT DISTINCT patient_id, token
    FROM patient_name, regexp_split_to_table(lower(normalize(value, NFC)), '\\s+') AS token
    WHERE token <> '' AND use_key <> ALL(%s)
)"""

_BIRTH_YEAR_CTE = """birth_year AS (
    -- year-range values are EXCLUDED: "1981/1991" would otherwise leak its first
    -- 4-digit run (1981) into name+year / dob+first-initial as if it were a birth year --
    -- a false key (the window min is not a birth year; principle 4). The anchored range
    -- passes (_RANGE_GROUPS_SQL) own ranges.
    SELECT patient_id, substring(value FROM '[0-9]{4}') AS year
    FROM patient_demographic
    WHERE field = 'dob' AND value ~ '[0-9]{4}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
)"""

_BLOCKING_SEX_CTE = """blocking_sex AS (
    -- Exclude the uncertainty sentinels (principle 4: no-data-is-never-agreement). The set
    -- is BOUND from adapter.VALUE_SENTINELS_PARAM -- the same set the Python scoring side
    -- treats as absent-value -- so the SQL exclusion can never drift from it (the
    -- placeholder_uses parameter-binding pattern; a hand-mirrored literal here once
    -- lagged the adapter's normalization). Without this, two charts that BOTH merely
    -- recorded sex 'unknown' would share a blocking_sex row and the sex-keyed rescues
    -- (dob-range+sex, name+sex) would key on mutual ignorance rather than a real signal.
    --
    -- The trim approximates the adapter's value.strip() for the whitespace a real feed
    -- plausibly emits: space, tab, LF, CR, FF, VT, NBSP (btrim's DEFAULT trims spaces
    -- ONLY -- it would let a tab-padded sentinel through as a tab-residue key). Python's
    -- strip() also removes rarer Unicode spaces (em-space etc.); those are out of scope:
    -- an exotically-padded sentinel keys on its residue, which at worst adds a noise
    -- pair between two identically-mangled values, never suppresses a true one. lower()
    -- stands in for casefold(); identical for the ASCII values this field carries. The
    -- trimmed form is also the grouping key (padding on a REAL value must not hide a
    -- genuine shared signal); an all-whitespace value trims to '' and is excluded.
    SELECT DISTINCT patient_id, sex FROM (
        SELECT patient_id,
               btrim(lower(value), E' \\t\\n\\r\\f\\u000b\\u00a0') AS sex
        FROM patient_demographic
        WHERE field IN ('sex-at-birth', 'administrative-sex') AND value IS NOT NULL
    ) trimmed
    WHERE sex <> '' AND sex <> ALL(%s)
)"""

_GROUPS_SQL = f"""
WITH {_NAME_TOKENS_CTE},
{_BIRTH_YEAR_CTE},
{_BLOCKING_SEX_CTE}
SELECT 'identifier' AS pass_name, system || ':' || match_key AS key,
       array_agg(patient_id) AS members
FROM patient_identifier WHERE system <> 'unknown'
GROUP BY system, match_key HAVING count(DISTINCT patient_id) >= 2
UNION ALL
-- The exact-'dob' arm is a POINT-dob pass: year-range values are excluded, mirroring the
-- birth_year CTE above. Two reasons. (1) A/B purity: two charts carrying the IDENTICAL
-- range string ("1981/1991" on both) would otherwise group here by literal string
-- equality, so an 'off-range-passes' baseline run would still surface range pairs and
-- understate the anchored passes' measured contribution on exactly the John-Doe
-- population the measurement exists for. (2) Two identical MALFORMED range strings
-- ("about-forty" twice, one buggy writer) would group on garbage. The anchored passes
-- own ranges -- identical or not -- and pair strictly more than string equality did, so
-- with all passes on this exclusion costs no recall.
SELECT 'dob', value, array_agg(patient_id)
FROM patient_demographic WHERE field = 'dob'
  AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
GROUP BY value HAVING count(DISTINCT patient_id) >= 2
UNION ALL
SELECT 'name', token, array_agg(patient_id)
FROM name_tokens
GROUP BY token HAVING count(*) >= 2
UNION ALL
SELECT 'name+year', nt.token || '|' || byr.year, array_agg(nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY nt.token, byr.year HAVING count(DISTINCT nt.patient_id) >= 2
UNION ALL
-- dob+first-initial: birth-year + the first CHARACTER of each name token. substring(token
-- FROM 1 FOR 1) is character-wise in PostgreSQL (first code point after NFC, not first
-- byte). A first-initial RELAXATION of 'name': it groups charts that share a birth-year and
-- a first initial but NO full name token (a misspelling/transposition/diacritic variant), so
-- it rescues true matches the token passes miss. Point-year only (birth_year excludes
-- year-range) -- the anchored dob-range passes own ranges. An empty first initial is
-- impossible: name_tokens excludes '' tokens.
SELECT 'dob+first-initial', substring(nt.token FROM 1 FOR 1) || '|' || byr.year,
       array_agg(DISTINCT nt.patient_id)
FROM name_tokens nt JOIN birth_year byr USING (patient_id)
GROUP BY substring(nt.token FROM 1 FOR 1), byr.year
HAVING count(DISTINCT nt.patient_id) >= 2
UNION ALL
-- name+sex: name token + normalized sex (blocking_sex: the sentinel-excluded UNION of
-- sex-at-birth and administrative-sex). A SUBSET of the 'name' block when uncapped (it adds
-- no pairs there); its value is the CAPPED case -- it splits an oversized unisex-token
-- 'name' block the cap drops wholesale into per-sex sub-blocks that fit. Recall-first union:
-- a trans patient whose administrative-sex matches an observation still groups though
-- sex-at-birth differs. DISTINCT because a chart can carry the same sex on both facets.
SELECT 'name+sex', nt.token || '|' || bs.sex, array_agg(DISTINCT nt.patient_id)
FROM name_tokens nt JOIN blocking_sex bs USING (patient_id)
GROUP BY nt.token, bs.sex HAVING count(DISTINCT nt.patient_id) >= 2
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
_RANGE_GROUPS_SQL = f"""
WITH birth_window AS (
    -- Evaluation-order-proof malformed-range guard: PostgreSQL does NOT guarantee
    -- WHERE-subexpression evaluation order, so a `split_part(...)::int` cast could be
    -- evaluated BEFORE the `value ~ '^[0-9]{{4}}/[0-9]{{4}}$'` regex guard that exists to
    -- filter it out -- and a non-numeric value ("about-forty") would then raise
    -- "invalid input syntax for type integer" and crash the whole sweep on exactly the
    -- input this guard exists to degrade safely. `substring(value FROM '^([0-9]{{4}})/')`
    -- returns NULL on non-match (never raises), so the cast of NULL is safe and the
    -- comparison against NULL is not-true (row filtered) regardless of evaluation order.
    -- The regex guard is kept too (cheap, and documents intent) but correctness must not
    -- -- and no longer does -- depend on it being evaluated first.
    SELECT patient_id,
           substring(value FROM '^([0-9]{{4}})/')::int AS y_min,
           substring(value FROM '/([0-9]{{4}})$')::int AS y_max,
           TRUE AS is_range
    FROM patient_demographic
    WHERE field = 'dob'
      AND facets ->> 'precision' = 'year-range'
      AND value ~ '^[0-9]{{4}}/[0-9]{{4}}$'
      AND substring(value FROM '^([0-9]{{4}})/')::int <= substring(value FROM '/([0-9]{{4}})$')::int
    UNION ALL
    SELECT patient_id,
           substring(value FROM '[0-9]{{4}}')::int,
           substring(value FROM '[0-9]{{4}}')::int,
           FALSE
    FROM patient_demographic
    WHERE field = 'dob'
      AND value ~ '[0-9]{{4}}'
      AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'
),
{_BLOCKING_SEX_CTE},
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

    Eight passes (see blocking.ALL_PASSES): six SYMMETRIC group passes (identifier /
    exact-DOB / name-token / name-token+birth-year / dob+first-initial / name+sex,
    _GROUPS_SQL) and two ANCHORED birth-year-range passes (dob-range / dob-range+sex,
    _RANGE_GROUPS_SQL — the §5.4 range-blocking slice).

    `enabled_passes` is the A/B measurement toggle: None runs every pass; a set runs only
    the named ones (unknown names raise — see blocking.resolve_enabled_passes). Filtering
    happens on the returned rows' pass_name — the SQL is never edited per-subset, so the
    toggle can never change what a pass WOULD have produced. A whole STATEMENT is skipped
    only when none of its passes is enabled (its arms are independent UNION ALL branches,
    so skipping it provably cannot affect any enabled pass — and the A/B baseline run
    with the range passes off must not pay for the un-indexable range overlap join).

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
    # require_registered on every fetched row, against the emitting STATEMENT's declared
    # set: an unregistered (or registered-but-misplaced) SQL arm would otherwise be
    # silently filtered by the `enabled` check on every run (a pass that looks built but
    # contributes zero pairs) — or silently skipped with the wrong statement.
    with conn.cursor() as cur:
        if enabled & SYMMETRIC_PASSES:
            # Two binds now: _PLACEHOLDER_USES_PARAM for name_tokens (first, appears first),
            # VALUE_SENTINELS_PARAM for blocking_sex (second) -- the name+sex arm's sex source.
            cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM))
            for pass_name, key, members in cur.fetchall():
                if require_registered(pass_name, SYMMETRIC_PASSES) not in enabled:
                    continue
                size = len(members)
                if size > max_block_size:
                    skipped_blocks.append((pass_name, key, size))
                else:
                    pairs.update(_pairs_from_members(members))
        if enabled & ANCHORED_PASSES:
            # The anchored range passes: pairs are anchor x member ONLY. The cap counts
            # the WHOLE block (members + the anchor itself) so "block size" means the
            # same thing for both pair-generation shapes, and a skipped block is
            # reported under the anchor's uuid (its natural key). The %s binds the
            # uncertainty-sentinel exclusion in the blocking_sex CTE.
            cur.execute(_RANGE_GROUPS_SQL, (VALUE_SENTINELS_PARAM,))
            for pass_name, anchor, members in cur.fetchall():
                if require_registered(pass_name, ANCHORED_PASSES) not in enabled:
                    continue
                size = len(members) + 1
                if size > max_block_size:
                    skipped_blocks.append((pass_name, str(anchor), size))
                else:
                    pairs.update(pairs_from_anchor(anchor, members))
    return sorted(pairs), skipped_blocks


def upsert_proposal(conn, low, high, payload: ProposalPayload) -> None:
    """Write (or refresh) the advisory proposal for a canonical-ordered pair.

    Latest-wins on (patient_low, patient_high), but a human's decision (accepted / rejected
    / applied / auto_applied / the C2b veto-driven 'review') is PRESERVED — a re-run
    refreshes the score/band/evidence, never a verdict. The ONE matcher-owned exception is
    'retracted' -> 'pending': a row the matcher itself withdrew (band dropped below review,
    see retract_pending_proposal) but now proposes again must re-surface on the worklist, so
    a genuinely resurrected match is never left hidden. Every other status is left untouched.

    Does NOT commit. The caller owns the transaction boundary.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal "
            "(patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) "
            "VALUES (%s,%s,%s,%s,%s,%s,%s) "
            "ON CONFLICT (patient_low, patient_high) DO UPDATE SET "
            "score_total=EXCLUDED.score_total, band=EXCLUDED.band, "
            "veto_findings=EXCLUDED.veto_findings, evidence=EXCLUDED.evidence, "
            "matcher_version=EXCLUDED.matcher_version, updated_at=clock_timestamp(), "
            "status=CASE WHEN match_proposal.status='retracted' THEN 'pending' "
            "ELSE match_proposal.status END",
            (low, high, payload.score_total, payload.band.value,
             json.dumps(list(payload.veto_findings)), json.dumps(list(payload.evidence)),
             payload.matcher_version),
        )


def retract_pending_proposal(conn, low, high) -> int:
    """Withdraw a still-PENDING advisory proposal (status -> 'retracted'); return rows hit.

    Called when a pair the matcher previously surfaced now bands below the review floor —
    most sharply the §5.4 forcing rule, which persisted a REVIEW row while a chart was
    'unconfirmed' (a transient state) that must not linger once the Doe is identified
    (issue #135). Append-only-friendly: a status move, never a DELETE (db/017 grants none),
    so the advisory row's history is preserved and a hub worklist (which filters on
    status='pending') stops grouping a resolved chart under a nonexistent Doe.

    Only 'pending' rows transition — a human's disposition or a matcher auto-application is
    left untouched. A no-op (0 rows) for the common case: a sub-threshold pair that never
    had a proposal. Does NOT commit; the caller owns the transaction boundary.
    """
    with conn.cursor() as cur:
        cur.execute(
            "UPDATE match_proposal SET status='retracted', updated_at=clock_timestamp() "
            "WHERE patient_low=%s AND patient_high=%s AND status='pending'",
            (low, high),
        )
        return cur.rowcount
