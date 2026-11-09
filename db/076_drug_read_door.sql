-- 076_drug_read_door.sql — the drug read door (allergy slice 3; ADR-0098 M1): the ONE place
-- Cairn reads drugref.
--
-- WHAT drugref IS HERE. An optional drug knowledge base, installed (when a deployment wants it)
-- into Cairn's own database as the schema `drugref`. Most nodes will never have it, and a node
-- that has it may lose it or be unable to read it. So nothing in the floor, the projections or
-- the sync path may depend on it (ADR-0059 decision 4, guarded by
-- crates/cairn-node/tests/no_drugref_dependency.rs, which exempts THIS FILE ONLY).
--
-- WHAT THE DOOR IS FOR. Two read-only questions the window asks, both advisory:
--   * the PICK: while a clinician types an allergen, which drugs and drug groups match
--     (cairn_drug_search) — so the entry can carry a coding as well as the typed name;
--   * the CHECK: which allergens reach which medications — a salt of the same drug, or a member
--     of a coded group (cairn_drug_facts) — so the medication list can mark them.
-- Neither ever refuses anything or writes anything. An answer of "nothing" is always safe to give:
-- the window then checks by name only and SAYS so ("not fully checked", D8). That is why every
-- door answers nothing — never an error — when drugref is absent or unreadable
-- (cairn_drug_available): a fault here must never stop the medication list drawing.
--
-- WHY DEFINER DOORS. The window's login (cairn_window, db/062) holds no grant on drugref, and
-- should not: the doors are the whole interface, so their shape is the contract a drugref release
-- must keep. Each is SECURITY DEFINER with the path pinned to pg_catalog then pg_temp (pg_temp
-- last, #426), and every drugref and Cairn name inside is schema-qualified. No door's name says
-- "drugref", so the Rust that calls them never has to.
--
-- The schema they read (checked against a real drugref on 2026-11-09):
--   substance_moiety(moiety_uuid, display_name)          one drug
--   identity_claim(moiety_uuid, scheme, value, superseded_by)    its UNII (scheme 'UNII'), current
--                                                        while superseded_by IS NULL
--   substance_composition(substance_unii, component_moiety, is_active_component, …)
--                                                        what a substance is made of (salts, hydrates)
--   substance_class(class_uuid, class_name, concept_type)  a group ('EPC', 'PA', 'PK', …)
--   class_parent(child_class_uuid, parent_class_uuid)    the group tree
--   class_membership(moiety_uuid, class_uuid, relationship)  which drugs a group holds
--
-- Replay-safe: CREATE OR REPLACE with fixed return shapes. plpgsql plans its queries when they
-- RUN, so these functions load on a node with no drugref schema at all.
BEGIN;

-- 1. Is there a drugref this door can read? True only when each of the six tables exists and the
--    door's owner may read it. Never raises.
--
--    WHY THE CATALOGUE AND NOT to_regclass: to_regclass('drugref.x') RAISES "permission denied for
--    schema" when the schema exists but the caller lacks USAGE on it — exactly the "installed but
--    unreadable" case this must answer `false` for. Reading pg_namespace/pg_class never raises,
--    and has_schema_privilege/has_table_privilege on an oid found there cannot either.
--    current_user here is the door's owner (definer rights), so the checks are the owner's.
CREATE OR REPLACE FUNCTION cairn_drug_available()
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT coalesce((
        SELECT has_schema_privilege(n.oid, 'USAGE')
               AND count(*) = 6
               AND bool_and(has_table_privilege(c.oid, 'SELECT'))
          FROM pg_catalog.pg_namespace n
          JOIN pg_catalog.pg_class c ON c.relnamespace = n.oid
         WHERE n.nspname = 'drugref'
           AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
           AND c.relname IN ('substance_moiety', 'identity_claim', 'substance_class',
                             'class_parent', 'class_membership', 'substance_composition')
         GROUP BY n.oid), false);
$$;

-- 2. The pick: drugs and drug groups whose name matches what the clinician typed.
--    * Fewer than 3 characters (after trimming) answers nothing — two letters would otherwise
--      offer most of the table.
--    * The text matches LITERALLY: LIKE's own `%`, `_` and the escape `\` are escaped first, so a
--      typed "%%%" is three percent signs, not "every drug".
--    * A drug matches when its name, or a word in it, starts with the text ("sodium" finds
--      "amoxicillin sodium"; "moxi" finds nothing). A group matches anywhere in its name.
--    * Only EPC and PA groups are offered (D4): "Penicillin-class Antibacterial [EPC]" is a group
--      a clinician means by "allergic to penicillins"; "Renal Excretion [PK]" or
--      "Transpeptidase Inhibitors [MoA]" never is.
--    * Drugs first, then groups; within each, shorter names first (the likeliest intent), then
--      byte order (COLLATE "C", so every node orders alike). At most 20.
--    The answer's `system` is the coding system the entry will carry; `display` is the name to
--    sign with it (the window adds " (group)" to a group's option).
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
                       m.display_name AS display, 0 AS tier
                  FROM drugref.substance_moiety m
                 -- A leading space makes "the start of the name" one more word start.
                 WHERE (' ' || m.display_name) ILIKE ('% ' || v_pat || '%') ESCAPE '\'
                UNION ALL
                SELECT 'drugref-class'::text, c.class_uuid::text, c.class_name, 1
                  FROM drugref.substance_class c
                 WHERE c.concept_type IN ('EPC', 'PA')
                   AND c.class_name ILIKE ('%' || v_pat || '%') ESCAPE '\') o
         ORDER BY o.tier, length(o.display), o.display COLLATE "C"
         LIMIT greatest(0, least(coalesce(p_limit, 20), 20));
END;
$$;

-- 3. The check: which (allergen, medication) pairs drugref connects. Both arrays are drugref
--    codes the window already holds (an allergen's coding, a medication line's moiety).
--
--    A drug's FAMILY (M3) is the drug itself plus the active components of the substance its
--    CURRENT UNII names. That is how salts and hydrates count as the same drug: amoxicillin and
--    amoxicillin sodium both have the active component amoxicillin anhydrous — and amoxicillin
--    anhydrous, not amoxicillin, is what drugref files under "Penicillin-class Antibacterial [EPC]".
--    Without the walk, a penicillin allergy would never warn against amoxicillin.
--
--    The facts:
--      'same'    the allergen is a drug, and its family meets the medication's family;
--      'member'  the allergen is a group, and a member of the group or of any group below it
--                (class_parent, walked to the bottom; any relationship counts, D5) is in the
--                medication's family.
--      'complete' is NEVER produced today (D6): drugref states no group is complete, so every
--                group reads "not fully checked". The fact is named so that the day drugref
--                states completeness, this door says so without a new shape.
--    A pair drugref does not connect produces no row: the window then falls back to the name
--    and to its own "same code" test, which is why "nothing" is always a safe answer here.
CREATE OR REPLACE FUNCTION cairn_drug_facts(p_allergens uuid[], p_meds uuid[])
RETURNS TABLE(fact text, allergen uuid, med uuid)
LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
#variable_conflict use_column
BEGIN
    IF NOT public.cairn_drug_available() THEN
        RETURN;
    END IF;
    RETURN QUERY
        WITH RECURSIVE
        -- Every code asked about, allergen or medication.
        asked AS (
            SELECT DISTINCT u AS moiety FROM unnest(p_allergens || p_meds) u),
        -- (drug, family member) pairs: each drug is its own member, plus the active components
        -- of the substance named by its current UNII claim(s).
        family AS (
            SELECT a.moiety, a.moiety AS member FROM asked a
            UNION
            SELECT ic.moiety_uuid, sc.component_moiety
              FROM drugref.identity_claim ic
              JOIN drugref.substance_composition sc ON sc.substance_unii = ic.value
             WHERE ic.moiety_uuid IN (SELECT a.moiety FROM asked a)
               AND ic.scheme = 'UNII'
               AND ic.superseded_by IS NULL
               AND sc.is_active_component IS TRUE),
        -- (allergen group, group in its subtree), the group itself included. UNION, not UNION
        -- ALL, so a cycle in the tree ends the walk instead of looping.
        subtree AS (
            SELECT c.class_uuid AS root, c.class_uuid AS cls
              FROM drugref.substance_class c
             WHERE c.class_uuid = ANY (p_allergens)
            UNION
            SELECT s.root, cp.child_class_uuid
              FROM subtree s
              JOIN drugref.class_parent cp ON cp.parent_class_uuid = s.cls),
        pairs AS (
            SELECT DISTINCT a AS allergen, m AS med FROM unnest(p_allergens) a, unnest(p_meds) m)
        SELECT 'same'::text, p.allergen, p.med
          FROM pairs p
         WHERE EXISTS (SELECT 1 FROM drugref.substance_moiety sm WHERE sm.moiety_uuid = p.allergen)
           AND EXISTS (SELECT 1 FROM family fa JOIN family fm ON fm.member = fa.member
                        WHERE fa.moiety = p.allergen AND fm.moiety = p.med)
        UNION ALL
        SELECT 'member'::text, p.allergen, p.med
          FROM pairs p
         WHERE EXISTS (SELECT 1 FROM subtree s
                         JOIN drugref.class_membership cm ON cm.class_uuid = s.cls
                         JOIN family fm ON fm.member = cm.moiety_uuid
                        WHERE s.root = p.allergen AND fm.moiety = p.med);
END;
$$;

-- Only the window reads them; PUBLIC (and so any other role) does not.
REVOKE EXECUTE ON FUNCTION cairn_drug_available() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_drug_search(text, int) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_drug_facts(uuid[], uuid[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_drug_available() TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_drug_search(text, int) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_drug_facts(uuid[], uuid[]) TO cairn_window;

COMMIT;
