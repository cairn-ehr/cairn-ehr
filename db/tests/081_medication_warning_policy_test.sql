\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/081 — the medication warning threshold (batch-charting slice 3, Task 1).
--
-- WHY: which findings count as a WARNING when a medication is charted is a site's policy, set by
-- an administrator at the node's console, never by a clinician. The scale has two levels today:
-- 'allergy-hit' (the default: only an allergy hit warns) and 'adverse-effect' (a previous adverse
-- effect warns too). Nothing sits above 'allergy-hit', so no stored word can switch allergy
-- warnings off ("lowerable only"). These cases pin the default, the read-back, the reset, the
-- refused words, and that the setter is the console's alone. Nothing here persists (ROLLBACK).

BEGIN;

-- Start from "no policy row": what a fresh site and a restored one both get.
DELETE FROM medication_warning_policy;

-- default_is_allergy_hit, set_and_reset, refused_words
DO $$
DECLARE w text;
BEGIN
    IF cairn_medication_warning_threshold() <> 'allergy-hit' THEN
        RAISE EXCEPTION 'default_is_allergy_hit: no row gave %', cairn_medication_warning_threshold();
    END IF;
    PERFORM cairn_medication_set_warning_threshold('adverse-effect');
    IF cairn_medication_warning_threshold() <> 'adverse-effect' THEN
        RAISE EXCEPTION 'set_and_reset: adverse-effect read back as %', cairn_medication_warning_threshold();
    END IF;
    PERFORM cairn_medication_set_warning_threshold(NULL);
    IF cairn_medication_warning_threshold() <> 'allergy-hit' THEN
        RAISE EXCEPTION 'set_and_reset: NULL must read back the default';
    END IF;
    PERFORM cairn_medication_set_warning_threshold('allergy-hit');
    IF cairn_medication_warning_threshold() <> 'allergy-hit' THEN
        RAISE EXCEPTION 'set_and_reset: allergy-hit not stored';
    END IF;
    -- Any other word, an off switch included, is refused at the door, not stored.
    FOREACH w IN ARRAY ARRAY['nothing', 'none', '', 'off', 'Allergy-Hit'] LOOP
        BEGIN
            PERFORM cairn_medication_set_warning_threshold(w);
            RAISE EXCEPTION 'refused_words: "%" was stored', w USING ERRCODE = 'P0002';
        EXCEPTION WHEN raise_exception THEN NULL;
        END;
    END LOOP;
    -- And the table itself refuses one, should anyone write it as the owner.
    BEGIN
        INSERT INTO medication_warning_policy (singleton, threshold) VALUES (true, 'nothing')
        ON CONFLICT (singleton) DO UPDATE SET threshold = EXCLUDED.threshold;
        RAISE EXCEPTION 'refused_words: the table stored "nothing"' USING ERRCODE = 'P0002';
    EXCEPTION WHEN check_violation THEN NULL;
    END;
END $$;

-- the_setter_is_the_consoles: no runtime role may set it or reach the table; the window may read
-- it; both definer-run with a pinned search_path; neither PUBLIC.
DO $$
DECLARE f text;
BEGIN
    IF has_function_privilege('cairn_window', 'cairn_medication_set_warning_threshold(text)', 'EXECUTE')
       OR has_function_privilege('cairn_agent', 'cairn_medication_set_warning_threshold(text)', 'EXECUTE')
       OR has_function_privilege('cairn_node', 'cairn_medication_set_warning_threshold(text)', 'EXECUTE') THEN
        RAISE EXCEPTION 'the_setter_is_the_consoles: a runtime role may set the threshold';
    END IF;
    IF has_table_privilege('cairn_window', 'medication_warning_policy', 'SELECT')
       OR has_table_privilege('cairn_window', 'medication_warning_policy', 'UPDATE')
       OR has_table_privilege('cairn_agent', 'medication_warning_policy', 'SELECT')
       OR has_table_privilege('cairn_node', 'medication_warning_policy', 'SELECT') THEN
        RAISE EXCEPTION 'the_setter_is_the_consoles: a runtime role can reach medication_warning_policy';
    END IF;
    IF NOT has_function_privilege('cairn_window', 'cairn_medication_warning_threshold()', 'EXECUTE') THEN
        RAISE EXCEPTION 'the_setter_is_the_consoles: the window cannot read the threshold';
    END IF;
    FOREACH f IN ARRAY ARRAY['cairn_medication_warning_threshold()',
                             'cairn_medication_set_warning_threshold(text)'] LOOP
        IF has_function_privilege('public', f, 'EXECUTE') THEN RAISE EXCEPTION '% must not be PUBLIC', f; END IF;
        IF NOT (SELECT prosecdef AND proconfig @> ARRAY['search_path=public, pg_temp'] FROM pg_proc
                 WHERE oid = f::regprocedure) THEN
            RAISE EXCEPTION '% must be definer-run with a pinned search_path', f;
        END IF;
    END LOOP;
END $$;

ROLLBACK;
