# matcher/src/cairn_matcher/pipeline/blocking_sql.py
"""The blocking SQL: the text of the eight blocking passes, and nothing else.

Blocking decides which pairs of charts the scorer ever looks at (a pair never grouped is never
scored, so never proposed). Two callers share this ONE definition, on purpose:
- the whole-population sweep, `pipeline.db.generate_candidate_pairs`, and
- the commit-time per-chart check, `pipeline.targeted.candidate_pairs_for` (repair path R4),
  which wraps these statements in a filter that keeps only the groups containing one chart.
A copy in either place would be blocking drift — a pass that groups in one and not the other.

Why a module of its own: it is pure data (strings), so it needs no psycopg. It used to live in
`pipeline/db.py`, the one psycopg-touching module, which made `targeted` — and through it the
worker, `watch` and the `cairn-matcher` CLI — unimportable without the `pipeline` extra. That
broke CI's pure suite (PR #724); `tests/test_pure_modules_import_without_psycopg.py` pins it.
`db.py` re-exports these names, so existing imports from there keep working.

Binds (psycopg `%s` placeholders, in order):
- `_GROUPS_SQL`: (PLACEHOLDER_USES_PARAM for name_tokens, VALUE_SENTINELS_PARAM for blocking_sex)
- `_RANGE_GROUPS_SQL`: (VALUE_SENTINELS_PARAM,)

The SQL comments below name `load_candidate` and the adapter: those are
`pipeline.db.load_candidate` and `pipeline.adapter`.
"""

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
    -- are different code points and a true duplicate is never even grouped. Mirrors the NFC
    -- fold of the adapter's _normalize_token; but this side folds case with lower() while the
    -- scorer casefold()s (Postgres has no casefold), so the two DIVERGE where casefold expands
    -- a code point lower() leaves alone ("Weiß".casefold()=="weiss" vs "Weiß".lower()=="weiß";
    -- ligatures likewise). Such a pair scores EXACT but never shares a token here — recall loss
    -- only (never a false group), the designed limit of this deliberately-simple tokenizer;
    -- the §5.13 hub duplicate sweep is the declared backstop (issue #211 gap 3, adapter.py).
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
#
# #725 — the sexes RIDE ON THE WINDOW ROWS; no arm joins a sex scan. The '+sex' arm used to join
# blocking_sex to itself on `sex` (sa JOIN sm ON sm.sex = sa.sex). Sex has about two values, so
# the planner -- which has no statistics for a CTE and estimated ~116 000 rows -- built that join
# FIRST and got ~14.7 million rows at 10 000 charts: ~7 s of every per-chart duplicate check and
# of every sweep. Two scans of per-patient sex ARRAYS were cross-joined first in the same way. So
# a chart's set of sexes is attached to its birth window ONCE, by patient_id (sexed_window), the
# overlap rows carry both sets, and the arm is a filter. "Some sex of the anchor equals some sex
# of the member" is exactly "the two sets overlap" (&&). Pinned: tests/test_blocking_sql_shape.py.

_BIRTH_WINDOW_CTE = """birth_window AS (
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
)"""

# One row per patient: its SET of blocking sexes (blocking_sex has a row per patient AND sex --
# a chart can carry two, e.g. a sex-at-birth and a different administrative-sex).
_BLOCKING_SEXES_CTE = """blocking_sexes AS (
    SELECT patient_id, array_agg(sex) AS sexes FROM blocking_sex GROUP BY patient_id
)"""

# Every birth window with its chart's sexes: ONE join, on patient_id. A chart with no blocking
# sex keeps its window (LEFT JOIN) with NULL sexes: it still takes part in the plain 'dob-range'
# pass, and never in the '+sex' arm (NULL && x is not true -- the old inner join's "no row").
_SEXED_WINDOW_CTE = """sexed_window AS (
    SELECT w.patient_id, w.y_min, w.y_max, w.is_range, s.sexes
    FROM birth_window w LEFT JOIN blocking_sexes s USING (patient_id)
)"""

# Window `m` overlaps window `a` (both inclusive). Shared by the overlap join and the per-chart
# statement's relevant_anchor, so the two can never disagree on what "overlaps" means.
_OVERLAP_PREDICATE = """m.y_min <= a.y_max
     AND a.y_min <= m.y_max"""

# The overlap join, anchored on range charts. Deliberately left OPEN after `WHERE a.is_range`:
# the per-chart statement appends _ANCHOR_CLAUSE there; the sweep closes it as it is.
_WINDOW_OVERLAP_OPEN = f"""window_overlap AS (
    SELECT a.patient_id AS anchor, m.patient_id AS member,
           a.sexes AS anchor_sexes, m.sexes AS member_sexes
    FROM sexed_window a
    JOIN sexed_window m
      ON m.patient_id <> a.patient_id
     AND {_OVERLAP_PREDICATE}
    WHERE a.is_range"""

# The two arms. Rows are (pass_name, anchor, members); Python pairs anchor x member only.
_RANGE_ARMS = """SELECT 'dob-range' AS pass_name, anchor, array_agg(DISTINCT member) AS members
FROM window_overlap
GROUP BY anchor
UNION ALL
SELECT 'dob-range+sex', anchor, array_agg(DISTINCT member)
FROM window_overlap
WHERE anchor_sexes && member_sexes
GROUP BY anchor
"""

# The sweep's statement: every range anchor's block. Bind: (VALUE_SENTINELS_PARAM,).
_RANGE_GROUPS_SQL = f"""
WITH {_BIRTH_WINDOW_CTE},
{_BLOCKING_SEX_CTE},
{_BLOCKING_SEXES_CTE},
{_SEXED_WINDOW_CTE},
{_WINDOW_OVERLAP_OPEN}
)
{_RANGE_ARMS}"""

# --- The per-chart statement (repair path R4's commit-time check, #725) ----------------------
# Which range blocks can hold ONE chart? Its own, if it is a range anchor, and the block of every
# range anchor whose window overlaps one of its windows. relevant_anchor is that set (the chart
# itself included: a range window overlaps itself), and window_overlap is computed only for
# those anchors -- about 110 of 10 000 charts in the measured population instead of all 752
# range anchors x the population. Each kept anchor's window is still computed IN FULL (the
# clause restricts `a`, never `m`): its size decides the cap, and the verdict must be the sweep's.
# targeted.py still filters the result to the groups that contain the chart, because a relevant
# anchor's SEX-filtered block need not.
_RELEVANT_ANCHOR_CTE = f"""relevant_anchor AS (
    SELECT DISTINCT a.patient_id
    FROM birth_window a
    JOIN birth_window m
      ON m.patient_id = %s::uuid
     AND {_OVERLAP_PREDICATE}
    WHERE a.is_range
)"""

_ANCHOR_CLAUSE = """
      AND a.patient_id IN (SELECT patient_id FROM relevant_anchor)"""

# The sweep's statement plus relevant_anchor and the clause -- nothing else
# (test_the_anchored_statement_is_the_sweeps_plus_one_cte_and_one_clause).
# Binds, in text order: (the chart's uuid for relevant_anchor, VALUE_SENTINELS_PARAM).
_ANCHORED_RANGE_GROUPS_SQL = f"""
WITH {_BIRTH_WINDOW_CTE},
{_RELEVANT_ANCHOR_CTE},
{_BLOCKING_SEX_CTE},
{_BLOCKING_SEXES_CTE},
{_SEXED_WINDOW_CTE},
{_WINDOW_OVERLAP_OPEN}{_ANCHOR_CLAUSE}
)
{_RANGE_ARMS}"""
