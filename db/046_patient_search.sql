-- Cairn — §5.8 search-before-create: advisory candidate generation.
--
-- ADVISORY, NOT A FLOOR (design §2.1, ADR-0014). A missed candidate produces a false
-- SPLIT — §5.2's explicitly safe direction — and ADR-0014 already names the standing
-- backstop: the hub-tier background duplicate sweep. So this function never blocks,
-- never vetoes and never decides; it offers rows to a human.
--
-- SQL rather than a call into the advisory Python tier because a registration path must
-- beat paper (§1.2) and §5.11's latency limb is explicit ("type a few chars and enter, no
-- spinner"). Coupling two services on that path buys no safety.
--
-- DRIFT NOTE: the three blocking keys below mirror matcher/pipeline/db.py's three-pass
-- disjunction. They are NOT the same query — the sweep blocks all-by-all, this maps
-- query -> set — so only the KEY EXTRACTION is shared. Convergence is tracked as an issue;
-- if you change a key here, check the matcher.
BEGIN;

CREATE OR REPLACE FUNCTION cairn_search_candidates(
    p_name_tokens text[],
    p_birth_date  text,
    p_identifiers jsonb          -- [{"system": "...", "value": "..."}]
) RETURNS TABLE (patient_id uuid, matched_pass text)
LANGUAGE sql STABLE
SET search_path = public
AS $$
    -- Pass 1: shared identifier. Highest precision — the same system and the same
    -- match_key is near-conclusive, which is why it is also a db/016 hard-veto axis.
    SELECT DISTINCT pi.patient_id, 'identifier'::text
      FROM patient_identifier pi
      JOIN jsonb_array_elements(COALESCE(p_identifiers, '[]'::jsonb)) q
        ON pi.system = (q ->> 'system')
       AND pi.match_key = (q ->> 'value')
    UNION
    -- Pass 2: exact DOB. No date parsing, no range logic — an exact string compare on the
    -- projected value, matching the deliberately parse-free db/016 discipline.
    SELECT DISTINCT pd.patient_id, 'dob'::text
      FROM patient_demographic pd
     WHERE p_birth_date IS NOT NULL
       AND pd.field = 'dob'
       AND pd.value = p_birth_date
    UNION
    -- Pass 3: shared name token. Culture-neutral: EXACT token equality in ANY position, so
    -- a name typed in a different order still finds the chart, with no name-order model.
    --
    -- The tokenising expression is COPIED VERBATIM from matcher/src/cairn_matcher/pipeline/
    -- db.py's _GROUPS_SQL: `regexp_split_to_table(lower(normalize(value, NFC)), '\s+')`.
    -- Same key extraction, so a chart the sweep would pair is a chart this search finds.
    -- NFC normalisation is load-bearing, not decoration: without it a composed and a
    -- decomposed "José" are different tokens and the chart is silently unfindable.
    --
    -- Exact equality, NOT `LIKE '%token%'`: a leading-wildcard match cannot use an index at
    -- all, and the §7 budget is 5 s to find an existing chart. Equality keeps the door open
    -- to an expression index on the same expression when a node grows large enough to need
    -- one.
    --
    -- Callsigns ARE included here, unlike in the matcher (which excludes them via
    -- `use_key <> ALL(...)`). Both are right: a callsign is not evidence of identity, so it
    -- must not feed the scorer — but a clerk must be able to find the John Doe in front of
    -- them.
    SELECT DISTINCT pn.patient_id, 'name'::text
      FROM patient_name pn
      CROSS JOIN LATERAL regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS tok
      JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
        ON tok = lower(normalize(t, NFC))
$$;

REVOKE EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) TO cairn_agent;

COMMIT;
