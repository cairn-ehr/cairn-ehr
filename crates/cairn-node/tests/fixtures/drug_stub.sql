-- A minimal stand-in for drugref, for the drug read door's tests (db/076, allergy slice 3).
--
-- WHY A STUB: drugref is OPTIONAL and co-resident — a deployment may install it into Cairn's own
-- database as the `drugref` schema (ADR-0098 M1), and most never will. The door must work against
-- the real thing and degrade honestly without it, so the tests build just enough of it:
--   * the three tables the door reads, with the columns it reads (named and typed as in
--     drugref's db/067 on origin/main, 2026-11-10) — plus substance_class.concept_type, which
--     the door never reads, kept so a group row looks like drugref's;
--   * `drugref.allergy_check` with drugref db/068's EXACT signature and RETURNS TABLE. The door
--     delegates the whole allergy question to it (M4), so the stub does not reproduce drugref's
--     fold: it answers from a table of canned rows. What the tests pin is that the door passes
--     each pair through faithfully — not drugref's pharmacology, which drugref's own tests own.
--
-- LOAD IT ONLY INSIDE A TRANSACTION THAT ENDS IN ROLLBACK. The test databases are shared: a
-- `drugref` schema left behind would make every other test run against a node that "has" drugref.
-- That is why this file has no BEGIN/COMMIT of its own — the caller owns the transaction.
-- (db/tests/076_drug_read_door_test.sql \ir's it; crates/cairn-node/tests/drug_read_door.rs
-- include_str!'s it.)
--
-- Every uuid is COMPUTED (md5 of a label, read as a uuid), never written out (house rule 6). Tests
-- find a row's uuid by its name.
--
-- The rows, and what each one is for:
--   moieties   amoxicillin, flucloxacillin, perindopril, metformin.
--   classes    Penicillins (allergy group) — an AG, `structural-seed` as every group is today, named
--              as the real drugref names it (#1057: the window must not label it "group" twice) — and
--              Penicillin-class Antibacterial [EPC] — a MED-RT class with NO allergy_group row,
--              which the pick must never offer (M5).
--   answers    for 4 allergens (Penicillins (allergy group), flucloxacillin, amoxicillin, metformin) × 2 meds
--              (amoxicillin, perindopril), every outcome drugref gives: `match` same-group (a group
--              allergen, and a moiety allergen sharing the group), `match` same-drug — which for
--              amoxicillin/amoxicillin comes WITH a same-group row, as drugref gives one row per
--              path — `no-match`, and `unplaced`. Keys are (prescribed, allergen), drugref's
--              argument order, so a door that swapped them would find no answer at all.
CREATE SCHEMA drugref;

CREATE TABLE drugref.substance_moiety (
    moiety_uuid  uuid PRIMARY KEY,
    display_name text NOT NULL
);
CREATE TABLE drugref.substance_class (
    class_uuid   uuid PRIMARY KEY,
    class_name   text NOT NULL,
    concept_type text NOT NULL
);
CREATE TABLE drugref.allergy_group (
    class_uuid    uuid PRIMARY KEY REFERENCES drugref.substance_class (class_uuid),
    review_status text NOT NULL
);
-- The stub's own table (not drugref's): one row per row allergy_check returns for a pair.
CREATE TABLE drugref.stub_allergy_answer (
    prescribed           uuid NOT NULL,
    allergen             uuid NOT NULL,
    outcome              text NOT NULL,
    match_kind           text,
    reason               text,
    group_class_uuid     uuid,
    allergen_component   uuid,
    prescribed_component uuid
);

-- drugref db/068's exact shape. group_name and review_status come from the group's rows, as in
-- drugref; group_mesh_ui is always NULL here (the door does not read it).
CREATE FUNCTION drugref.allergy_check(p_prescribed uuid, p_allergen uuid)
RETURNS TABLE (outcome              text,
               match_kind           text,
               reason               text,
               group_class_uuid     uuid,
               group_name           text,
               group_mesh_ui        text,
               review_status        text,
               allergen_component   uuid,
               prescribed_component uuid)
LANGUAGE sql STABLE AS $$
    SELECT a.outcome, a.match_kind, a.reason, a.group_class_uuid, sc.class_name, NULL::text,
           ag.review_status, a.allergen_component, a.prescribed_component
      FROM drugref.stub_allergy_answer a
      LEFT JOIN drugref.substance_class sc ON sc.class_uuid = a.group_class_uuid
      LEFT JOIN drugref.allergy_group ag ON ag.class_uuid = a.group_class_uuid
     WHERE a.prescribed = p_prescribed AND a.allergen = p_allergen
$$;

INSERT INTO drugref.substance_moiety (moiety_uuid, display_name)
SELECT md5('stub moiety ' || n)::uuid, n
  FROM unnest(ARRAY['amoxicillin', 'flucloxacillin', 'perindopril', 'metformin']) n;

INSERT INTO drugref.substance_class (class_uuid, class_name, concept_type)
SELECT md5('stub class ' || n)::uuid, n, t
  FROM (VALUES ('Penicillins (allergy group)', 'AG'), ('Penicillin-class Antibacterial [EPC]', 'EPC')) AS r(n, t);

INSERT INTO drugref.allergy_group (class_uuid, review_status)
VALUES (md5('stub class Penicillins (allergy group)')::uuid, 'structural-seed');

-- `allergen` is a label: 'stub moiety <name>' or 'stub class <name>'. A match row names the
-- components that met (the allergen's own only when it is a moiety: a group has no fold).
INSERT INTO drugref.stub_allergy_answer
SELECT md5('stub moiety ' || r.med)::uuid, md5(r.allergen)::uuid, r.outcome, r.kind, r.reason,
       md5('stub class ' || r.grp)::uuid,
       CASE WHEN r.outcome = 'match' AND r.allergen LIKE 'stub moiety %' THEN md5(r.allergen)::uuid END,
       CASE WHEN r.outcome = 'match' THEN md5('stub moiety ' || r.med)::uuid END
  FROM (VALUES
      ('amoxicillin', 'stub class Penicillins (allergy group)',     'match',    'same-group', NULL, 'Penicillins (allergy group)'),
      ('perindopril', 'stub class Penicillins (allergy group)',     'no-match', NULL,         NULL, NULL),
      ('amoxicillin', 'stub moiety flucloxacillin', 'match',    'same-group', NULL, 'Penicillins (allergy group)'),
      ('perindopril', 'stub moiety flucloxacillin', 'no-match', NULL,         NULL, NULL),
      ('amoxicillin', 'stub moiety amoxicillin',    'match',    'same-drug',  NULL, NULL),
      ('amoxicillin', 'stub moiety amoxicillin',    'match',    'same-group', NULL, 'Penicillins (allergy group)'),
      ('perindopril', 'stub moiety amoxicillin',    'no-match', NULL,         NULL, NULL),
      ('amoxicillin', 'stub moiety metformin',      'unplaced', NULL, 'allergen-in-no-group', NULL),
      ('perindopril', 'stub moiety metformin',      'unplaced', NULL, 'allergen-in-no-group', NULL)
  ) AS r(med, allergen, outcome, kind, reason, grp);
