-- Cairn — the §5.9 sensitivity stream (ADR-0006 decision 3, ADR-0062; issue #232 part A).
--
-- Sensitivity is not a boolean on a body: it is an append-only stream of graded assertions
-- whose EFFECTIVE value is a projection (never merge, always overlay). This file ships the
-- stream, the projection, and nothing else. It ENFORCES NOTHING — a grade computed here
-- withholds no content. Sequester (custody narrowing) is #232 part C and is blocked on #231.
--
-- # Why these bodies are plaintext
--
-- ADR-0052 §2 lists what stays unsealed because the machinery binds on it. Sensitivity
-- assertions join that list: a node must READ the grade in order to coarsen, and coarsening
-- is exactly what a node holding no custody of the graded body must still do. Sealing the
-- grade under the key it governs is circular.
--
-- # What must never appear in these bodies
--
-- The matched blacklist CATEGORY. A plaintext, unconditionally-replicated body carrying
-- `category: "termination-of-pregnancy"` IS the disclosure this whole mechanism exists to
-- prevent (ADR-0006 decision 4).

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The ladder.
--
--    Open TEXT, no CHECK domain: a future grade from an upgraded peer is ADMITTED verbatim
--    (additive-only, principle 11). Gaps of 10 leave room to interpose deployment terms
--    later without renumbering.
--
--    !! READ THIS BEFORE "FIXING" THE ELSE BRANCH !!
--    ELSE is MAX, deliberately INVERTING cairn_clock_grade_rank's ELSE 0 (db/040). There,
--    an unrecognised value ranking 0 is safe because rank 0 WITHHOLDS REJECT POWER. Here,
--    an unrecognised value ranking 0 would WITHHOLD PROTECTION: an older node reading a
--    peer's newer `grade:protected-witness` as "not sensitive" emits an uncoarsened safety
--    projection and renders the body in the clear — a leak on exactly the events that most
--    needed protecting, in code that looks correct because it matches db/040's pattern.
--    The failure mode here must be over-coarsening (honest, repaired by upgrading the node),
--    never disclosure (unrecoverable).
--
--    ABSENCE IS NOT UNKNOWN. No assertion at all contributes nothing and reads as 'routine'
--    (see cairn_effective_sensitivity below); an unparseable or unrecognised GRADE VALUE
--    ranks MAX. Collapsing the two would make every event in the record maximally sensitive
--    — principle 4's not-yet-asked vs unknown.
CREATE OR REPLACE FUNCTION cairn_sensitivity_rank(g text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE g
        WHEN 'routine'     THEN 0
        WHEN 'sensitive'   THEN 10
        WHEN 'restricted'  THEN 20
        WHEN 'sequestered' THEN 30
        ELSE 2147483647    -- unknown / future / NULL: coarsen, never expose
    END;
$$;

COMMIT;
