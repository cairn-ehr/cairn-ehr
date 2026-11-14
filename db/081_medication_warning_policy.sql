-- db/081 — the medication warning threshold (batch-charting slice 3, ADR-0104).
--
-- WHAT. When a clinician charts a medication, the window checks the line against the patient's
-- allergy list (ADR-0098, ADR-0099). A finding that is a WARNING withholds the line's default tick
-- (ADR-0100), asks a Start line for a typed reason (ADR-0099 decision 17.5), and is named in the
-- signed `warning_override`. Which findings are warnings is the site's policy, set here.
--
-- THE SCALE, highest first. The stored word is the LOWEST level that warns:
--   'allergy-hit'    only an allergy hit warns (the default: no row, or a NULL);
--   'adverse-effect' a previous adverse effect warns too.
-- Nothing sits above 'allergy-hit', so no word can switch allergy warnings off: the threshold can
-- only be LOWERED from the default (design §3.5, "lowerable only"). A later check (interactions,
-- renal dosing) adds its own level on this scale; the CHECK below grows additively.
--
-- WHO. The policy is the administrator's, never a clinician's preference: set only at the node's
-- console (`cairn-node medication-set-warning-threshold`), so no runtime role holds the setter.
-- The window reads it through one definer door at every check, on the draft path only: a line
-- already charted is shown as ADR-0098 shows it, whatever the threshold (maintainer, 2026-11-14).
--
-- THE LIMIT. Like note_policy (db/067) and vault_policy (db/061) this row is custody state, not
-- the record: it is in no backup, so a restore comes back with the DEFAULT, and a site that had
-- lowered the threshold must set it again (ADR-0104; the issue it names covers all three rows).
--
-- Replay-safe: this file re-runs on every schema replay.

CREATE TABLE IF NOT EXISTS medication_warning_policy (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    -- NULL (or no row) = the default, 'allergy-hit'.
    threshold text CONSTRAINT medication_warning_policy_known_level
                   CHECK (threshold IN ('allergy-hit', 'adverse-effect'))
);

-- Nobody but the owner reads or writes it; the doors below are the only way in (db/061's posture).
REVOKE ALL ON medication_warning_policy FROM PUBLIC, cairn_agent, cairn_node;

-- Set the site's threshold (the console verb's door). NULL = back to the default. An unknown word
-- raises, so a typo at the console is told, never stored. No grant to any runtime role.
CREATE OR REPLACE FUNCTION cairn_medication_set_warning_threshold(p_word text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF p_word NOT IN ('allergy-hit', 'adverse-effect') THEN
        RAISE EXCEPTION 'cairn_medication_set_warning_threshold: the threshold is allergy-hit or adverse-effect (got "%"); NULL means the default, allergy-hit', p_word;
    END IF;
    INSERT INTO medication_warning_policy (singleton, threshold) VALUES (true, p_word)
    ON CONFLICT (singleton) DO UPDATE SET threshold = EXCLUDED.threshold;
END;
$$;

-- The site's threshold: the stored word, else 'allergy-hit'. A definer, because nobody but the
-- owner may read the table; the window calls it at each check, the console verb to read back.
CREATE OR REPLACE FUNCTION cairn_medication_warning_threshold()
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE((SELECT p.threshold FROM medication_warning_policy p), 'allergy-hit');
$$;

REVOKE EXECUTE ON FUNCTION cairn_medication_set_warning_threshold(text),
    cairn_medication_warning_threshold() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_medication_warning_threshold() TO cairn_window;
