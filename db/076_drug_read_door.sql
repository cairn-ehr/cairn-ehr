-- 076_drug_read_door.sql — the drug read door (allergy slice 3; ADR-0098 M1): the ONE place
-- Cairn reads drugref.
--
-- WHAT drugref IS HERE. An optional drug knowledge base, installed (when a deployment wants it)
-- into Cairn's own database as the schema `drugref`. Most nodes will never have it, and a node
-- that has it may lose it or be unable to read it. So nothing in the floor, the projections or
-- the sync path may depend on it (ADR-0059 decision 4, guarded by
-- crates/cairn-node/tests/no_drugref_dependency.rs, which exempts THIS FILE ONLY, and
-- crates/cairn-node/tests/drug_door_callers.rs, which forbids any other db/ file calling it).
--
-- WHAT THE DOOR IS FOR. Read-only questions the window asks, all advisory:
--   * the PICK: while a clinician types an allergen, which drugs and allergy groups match
--     (cairn_drug_search) — so the entry can carry a coding as well as the typed name;
--   * the CHECK: what drugref says about each (allergen, medication) pair (cairn_drug_facts);
--   * the STATUS: whether a coded group has been clinically reviewed (cairn_drug_group_status).
--   * the MEDICATION PICK: which drugs match a typed medication, most charted here first
--     (cairn_medication_drug_search, charting slice 1a).
-- None ever refuses anything or writes anything. An answer of "nothing" is always safe to give:
-- the window then checks by name only and SAYS so (the "not checked" tag, D8). So every door
-- answers nothing — no error — when cairn_drug_available says drugref is not usable: absent, too
-- old (no allergy groups, no allergy_check), reshaped (an allergy_check whose result shape is not
-- drugref db/068's), or unreadable (no USAGE on the schema, no SELECT on the three tables the
-- doors read, no EXECUTE on allergy_check).
--
-- WHAT AVAILABILITY CANNOT SEE, AND SO CALLERS MUST HANDLE. drugref's allergy_check is LANGUAGE
-- sql with invoker rights: it reads drugref's INTERNAL tables (moiety_allergy_fold,
-- allergy_group_member, allergy_mesh_placement, …) as this door's owner. Those are drugref's
-- implementation, not its interface, so this door does not list them — and an owner who may
-- read the three interface tables but not one of those makes cairn_drug_facts RAISE. Every
-- caller MUST therefore map an error from these doors to "unavailable" (pinned by the window's
-- a_drug_list_that_raises_leaves_every_line_not_checked): a fault here must never stop the
-- medication list drawing.
--
-- drugref OWNS THE FOLD (M4). Whether a salt or a hydrate is "the same drug", and which drugs an
-- allergy group holds, is drugref's question, answered by drugref.allergy_check (drugref db/068),
-- which folds EVERY composition component (its owner's ruling: pharmacological activity is not
-- what triggers an allergy). Cairn keeps no fold, membership or class walk of its own: the facts
-- door passes each pair to drugref and returns its rows unchanged. A wrong answer is fixed in
-- drugref, in one place, for every client.
--
-- WHY DEFINER DOORS. The window's login (cairn_window, db/062) holds no grant on drugref, and
-- should not: the doors are the whole interface, so their shape is the contract a drugref release
-- must keep. Each is SECURITY DEFINER with the path pinned to pg_catalog then pg_temp (pg_temp
-- last, #426), and every drugref and Cairn name inside is schema-qualified. No door's name says
-- "drugref", so the Rust that calls them never has to.
--
-- What the doors read (drugref origin/main, db/067 + db/068, 2026-11-10):
--   substance_moiety(moiety_uuid, display_name)            one drug
--   substance_class(class_uuid, class_name)                a class (the door never reads its
--                                                          concept_type: the groups offered are
--                                                          those allergy_group lists, #1058)
--   allergy_group(class_uuid, review_status, …)            the allergy groups, 'structural-seed'
--                                                          or 'reviewed'
--   allergy_check(p_prescribed uuid, p_allergen uuid)      the check, one row per path
--
-- Replay-safe: CREATE OR REPLACE with fixed return shapes (and one guarded DROP, below, for the
-- facts door's changed shape). plpgsql plans its queries when they RUN, so these functions load
-- on a node with no drugref schema at all.
BEGIN;

-- 1. Is there a drugref this door can read? True only when the owner may use the schema, may
--    SELECT the three tables, and may EXECUTE a drugref.allergy_check(uuid, uuid) whose result
--    is EXACTLY drugref db/068's shape (the facts door reads its columns by name, so another
--    shape would raise there). Never raises. A drugref from before allergy groups (no
--    allergy_group, no allergy_check) is "not there". It cannot see allergy_check's internal
--    reads — see the header: callers map a door's error to "unavailable".
--
--    WHY THE CATALOGUE AND NOT to_regclass (ruling T2-a): to_regclass('drugref.x') RAISES
--    "permission denied for schema" when the schema exists but the caller lacks USAGE on it —
--    exactly the "installed but unreadable" case this must answer `false` for. Reading
--    pg_namespace/pg_class/pg_proc never raises, and the has_*_privilege checks on an oid found
--    there cannot either. current_user here is the door's owner (definer rights), so the checks
--    are the owner's.
CREATE OR REPLACE FUNCTION cairn_drug_available()
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT coalesce((
        SELECT has_schema_privilege(n.oid, 'USAGE')
               AND (SELECT count(*) = 3 AND bool_and(has_table_privilege(c.oid, 'SELECT'))
                      FROM pg_catalog.pg_class c
                     WHERE c.relnamespace = n.oid
                       AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
                       AND c.relname IN ('substance_moiety', 'substance_class', 'allergy_group'))
               AND EXISTS (SELECT 1 FROM pg_catalog.pg_proc p
                            WHERE p.pronamespace = n.oid
                              AND p.proname = 'allergy_check'
                              AND pg_catalog.oidvectortypes(p.proargtypes) = 'uuid, uuid'
                              AND has_function_privilege(p.oid, 'EXECUTE')
                              -- on an oid read from pg_proc this cannot raise
                              AND pg_catalog.pg_get_function_result(p.oid) =
                                  'TABLE(outcome text, match_kind text, reason text, '
                                  'group_class_uuid uuid, group_name text, group_mesh_ui text, '
                                  'review_status text, allergen_component uuid, '
                                  'prescribed_component uuid)')
          FROM pg_catalog.pg_namespace n
         WHERE n.nspname = 'drugref'), false);
$$;

-- 2. The pick: drugs and allergy groups whose name matches what the clinician typed.
--    * Fewer than 3 characters (after trimming) answers nothing — two letters would otherwise
--      offer most of the table.
--    * The text matches LITERALLY: LIKE's own `%`, `_` and the escape `\` are escaped first, so a
--      typed "%%%" is three percent signs, not "every drug".
--    * A drug matches when its name, or a word in it, starts with the text ("moxi" does
--      not find amoxicillin). A group matches anywhere in its name.
--    * Only drugref's ALLERGY GROUPS are offered as groups (M5) — never a MED-RT class such as
--      "Penicillin-class Antibacterial [EPC]": allergy_check answers `allergen-not-a-group` for
--      one, so an allergen coded with it could never be checked.
--    * GROUPS first, then drugs (ruling F2, #1057): there are few groups, a class allergy is the
--      commonest intent, and "penicillin" matches far more drugs than the window shows, so a
--      group listed after them was cut off. Within each, shorter names first (the likeliest
--      intent), then byte order (COLLATE "C", so every node orders alike). At most 20.
--    The answer's `system` is the coding system the entry will carry; `code` the drugref uuid
--    (a moiety_uuid, or a group's class_uuid, M2); `display` the name to sign with it (the
--    window adds " (group)" to a group's option unless its name already says "group").
CREATE OR REPLACE FUNCTION cairn_drug_search(p_text text, p_limit int)
RETURNS TABLE(system text, code text, display text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
DECLARE
    v_text text := btrim(p_text);
    v_pat  text;
BEGIN
    IF v_text IS NULL OR length(v_text) < 3 OR NOT public.cairn_drug_available() THEN
        RETURN;
    END IF;
    -- Backslash first: it is the escape character for the two that follow.
    v_pat := replace(replace(replace(v_text, '\', '\\'), '%', '\%'), '_', '\_');
    RETURN QUERY
        SELECT o.system, o.code, o.display
          FROM (SELECT 'drugref-moiety'::text AS system, m.moiety_uuid::text AS code,
                       m.display_name AS display, 1 AS tier
                  FROM drugref.substance_moiety m
                 -- A leading space makes "the start of the name" one more word start.
                 WHERE (' ' || m.display_name) ILIKE ('% ' || v_pat || '%') ESCAPE '\'
                UNION ALL
                SELECT 'drugref-class'::text, c.class_uuid::text, c.class_name, 0
                  FROM drugref.allergy_group g
                  JOIN drugref.substance_class c ON c.class_uuid = g.class_uuid
                 WHERE c.class_name ILIKE ('%' || v_pat || '%') ESCAPE '\') o
         ORDER BY o.tier, length(o.display), o.display COLLATE "C"
         LIMIT greatest(0, least(coalesce(p_limit, 20), 20));
END;
$$;

-- 3. The check: drugref's answer for every (allergen, medication) pair. Both arrays are drugref
--    codes the window already holds (an allergen's coding, a medication line's moiety).
--
--    One row per row drugref.allergy_check(med, allergen) returns — note drugref's argument
--    order, PRESCRIBED first. drugref answers `match` (one row per path: `same-drug` or
--    `same-group`, with the group's name), `no-match` (both sides placed, nothing shared), or
--    `unplaced` (could not be checked). The window treats anything but `match` and `no-match` as
--    "not checked"; this door adds nothing and drops only what the window does not show:
--    drugref's reason, its path uuids, the group's MeSH id and its review status (the last is
--    read on its own, by cairn_drug_group_status).
--
--    The shape changed when the door began delegating (M4): the old one was (fact, allergen, med).
--    CREATE OR REPLACE cannot change a RETURNS TABLE, so any shape other than the current one is
--    dropped first; a converged node's replay leaves the function untouched.
DO $$
BEGIN
    -- On a fresh node to_regprocedure is NULL, so the comparison is NULL and nothing is dropped.
    IF pg_get_function_result(to_regprocedure('public.cairn_drug_facts(uuid[], uuid[])'))
       <> 'TABLE(allergen uuid, med uuid, outcome text, match_kind text, group_name text)' THEN
        DROP FUNCTION public.cairn_drug_facts(uuid[], uuid[]);
    END IF;
END $$;
CREATE OR REPLACE FUNCTION cairn_drug_facts(p_allergens uuid[], p_meds uuid[])
RETURNS TABLE(allergen uuid, med uuid, outcome text, match_kind text, group_name text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
BEGIN
    IF NOT public.cairn_drug_available() THEN
        RETURN;
    END IF;
    RETURN QUERY
        SELECT p.a, p.m, r.outcome, r.match_kind, r.group_name
          -- DISTINCT so a code the window lists twice is asked about once.
          FROM (SELECT DISTINCT a, m FROM unnest(p_allergens) a, unnest(p_meds) m
                 WHERE a IS NOT NULL AND m IS NOT NULL) p
         CROSS JOIN LATERAL drugref.allergy_check(p.m, p.a) r;
END;
$$;

-- 4. The status: each allergy group's review status (M7), for the expanded allergy list. A code
--    that is not an allergy group has no row; the window reads a missing row, or any word but
--    `reviewed`, as "not clinically reviewed" (D11).
CREATE OR REPLACE FUNCTION cairn_drug_group_status(p_groups uuid[])
RETURNS TABLE(group_code uuid, review_status text)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
BEGIN
    IF NOT public.cairn_drug_available() THEN
        RETURN;
    END IF;
    RETURN QUERY
        SELECT g.class_uuid, g.review_status
          FROM drugref.allergy_group g
         WHERE g.class_uuid = ANY (p_groups);
END;
$$;

-- 5. The medication pick (charting slice 1a): drugs whose name matches what the clinician typed,
--    ranked by how often THIS node has charted each one. It lives here, not beside the charting
--    projection in db/077, because it reads drugref and this is the one file that may.
--    * Drugs ONLY, never a group: a medication line is one drug (db/041 refuses a
--      drugref-class coding on a medication, and db/077 on a component).
--    * The same text rules as the allergy pick (§2): fewer than 3 characters answers nothing,
--      LIKE's metacharacters are escaped first, and a match is a word start.
--    * `uses` counts this node's coded lines naming the moiety: one per medication_coding row
--      (the line's single coding) plus one per coded component of a combination
--      (medication_charting.components). It reads every patient's rows, as the door's owner, and
--      answers only a per-drug count, never a patient. Most used first, then shorter names, then
--      byte order (every node orders alike). At most 20.
--    shortcut: the counts are recomputed per keystroke by scanning medication_coding's drugref
--    rows and every combination row; keep a per-moiety counter table if a node's medication list
--    grows large enough for the pick to lag.
CREATE OR REPLACE FUNCTION cairn_medication_drug_search(p_text text, p_limit int)
RETURNS TABLE(code text, display text, uses bigint)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
DECLARE
    v_text text := btrim(p_text);
    v_pat  text;
BEGIN
    IF v_text IS NULL OR length(v_text) < 3 OR NOT public.cairn_drug_available() THEN
        RETURN;
    END IF;
    v_pat := replace(replace(replace(v_text, '\', '\\'), '%', '\%'), '_', '\_');
    RETURN QUERY
        WITH used AS (
            SELECT mc.coding_code AS code
              FROM public.medication_coding mc
             WHERE mc.coding_system = 'drugref-moiety'
            UNION ALL
            SELECT part #>> '{coding,code}'
              FROM public.medication_charting ch
             CROSS JOIN LATERAL pg_catalog.jsonb_array_elements(ch.components) part
             WHERE pg_catalog.jsonb_typeof(ch.components) = 'array'
               AND part #>> '{coding,system}' = 'drugref-moiety'
        ), counts AS (
            SELECT u.code, count(*) AS n FROM used u GROUP BY u.code
        )
        SELECT m.moiety_uuid::text, m.display_name, coalesce(c.n, 0)
          FROM drugref.substance_moiety m
          LEFT JOIN counts c ON c.code = m.moiety_uuid::text
         WHERE (' ' || m.display_name) ILIKE ('% ' || v_pat || '%') ESCAPE '\'
         ORDER BY coalesce(c.n, 0) DESC, length(m.display_name), m.display_name COLLATE "C"
         LIMIT greatest(0, least(coalesce(p_limit, 20), 20));
END;
$$;

-- Only the window reads them; PUBLIC (and so any other role) does not.
REVOKE EXECUTE ON FUNCTION cairn_drug_available() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_drug_search(text, int) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_drug_facts(uuid[], uuid[]) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_drug_group_status(uuid[]) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_medication_drug_search(text, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_drug_available() TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_drug_search(text, int) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_drug_facts(uuid[], uuid[]) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_drug_group_status(uuid[]) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_medication_drug_search(text, int) TO cairn_window;

COMMIT;
