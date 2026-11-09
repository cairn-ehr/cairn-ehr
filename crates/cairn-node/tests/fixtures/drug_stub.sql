-- A minimal stand-in for drugref's schema, for the drug read door's tests (db/076, allergy slice 3).
--
-- WHY A STUB: drugref is OPTIONAL and co-resident — a deployment may install it into Cairn's own
-- database as the `drugref` schema (ADR-0098 M1), and most never will. The door must work against
-- the real thing and degrade honestly without it, so the tests build just enough of it: the six
-- tables the door reads, with ONLY the columns it reads (named and typed as in the real schema,
-- checked against `drugref_profiles` on 2026-11-09), and the rows the measured amoxicillin case
-- needs (M3).
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
--   moieties   amoxicillin, amoxicillin sodium, amoxicillin anhydrous, metformin — each with its
--              current UNII claim; metformin also carries one SUPERSEDED claim.
--   composition (M3, as measured) amoxicillin's and amoxicillin sodium's UNIIs both have the active
--              component amoxicillin anhydrous — so the two share a family. Metformin's superseded
--              UNII names amoxicillin anhydrous too: if the door ever read a superseded claim,
--              metformin would join amoxicillin's family. And amoxicillin sodium's UNII names
--              metformin as an INACTIVE component: if the door ever walked an inactive one, the
--              two would join.
--   classes    Penicillin-class Antibacterial [EPC] (amoxicillin anhydrous is its one member, as
--              in drugref), under Anti-Bacterial Agents [PA], under Anti-Infective Agents [PA] —
--              two levels, so the subtree walk is proven recursive. Renal Excretion [PK] (D4: a PK
--              class is never offered), with amoxicillin as a member.
CREATE SCHEMA drugref;

CREATE TABLE drugref.substance_moiety (
    moiety_uuid  uuid PRIMARY KEY,
    display_name text NOT NULL
);
CREATE TABLE drugref.identity_claim (
    moiety_uuid   uuid NOT NULL,
    scheme        text NOT NULL,
    value         text NOT NULL,
    superseded_by bigint
);
CREATE TABLE drugref.substance_class (
    class_uuid   uuid PRIMARY KEY,
    class_name   text NOT NULL,
    concept_type text NOT NULL
);
CREATE TABLE drugref.class_parent (
    child_class_uuid  uuid NOT NULL,
    parent_class_uuid uuid NOT NULL
);
CREATE TABLE drugref.class_membership (
    moiety_uuid  uuid NOT NULL,
    class_uuid   uuid NOT NULL,
    relationship text NOT NULL
);
CREATE TABLE drugref.substance_composition (
    substance_unii      text NOT NULL,
    component_moiety    uuid NOT NULL,
    relation            text NOT NULL,
    is_active_component boolean
);

INSERT INTO drugref.substance_moiety (moiety_uuid, display_name)
SELECT md5('stub moiety ' || n)::uuid, n
  FROM unnest(ARRAY['amoxicillin', 'amoxicillin sodium', 'amoxicillin anhydrous', 'metformin']) n;

INSERT INTO drugref.identity_claim (moiety_uuid, scheme, value, superseded_by) VALUES
    (md5('stub moiety amoxicillin')::uuid,           'UNII', '804826J2HU', NULL),
    (md5('stub moiety amoxicillin sodium')::uuid,    'UNII', '544Y3D6MYH', NULL),
    (md5('stub moiety amoxicillin anhydrous')::uuid, 'UNII', '9EM05410Q9', NULL),
    (md5('stub moiety metformin')::uuid,             'UNII', '9100L32L2N', NULL),
    (md5('stub moiety metformin')::uuid,             'UNII', 'STUBOLDUNII', 1);

INSERT INTO drugref.substance_composition (substance_unii, component_moiety, relation, is_active_component) VALUES
    ('804826J2HU',  md5('stub moiety amoxicillin anhydrous')::uuid, 'SALT_SOLVATE', TRUE),
    ('544Y3D6MYH',  md5('stub moiety amoxicillin anhydrous')::uuid, 'SALT_SOLVATE', TRUE),
    ('STUBOLDUNII', md5('stub moiety amoxicillin anhydrous')::uuid, 'SALT_SOLVATE', TRUE),
    ('544Y3D6MYH',  md5('stub moiety metformin')::uuid,             'SALT_SOLVATE', FALSE);

INSERT INTO drugref.substance_class (class_uuid, class_name, concept_type)
SELECT md5('stub class ' || n)::uuid, n, t
  FROM (VALUES ('Penicillin-class Antibacterial [EPC]', 'EPC'),
               ('Anti-Bacterial Agents', 'PA'),
               ('Anti-Infective Agents', 'PA'),
               ('Renal Excretion [PK]', 'PK')) AS r(n, t);

INSERT INTO drugref.class_parent (child_class_uuid, parent_class_uuid) VALUES
    (md5('stub class Penicillin-class Antibacterial [EPC]')::uuid, md5('stub class Anti-Bacterial Agents')::uuid),
    (md5('stub class Anti-Bacterial Agents')::uuid,                md5('stub class Anti-Infective Agents')::uuid);

INSERT INTO drugref.class_membership (moiety_uuid, class_uuid, relationship) VALUES
    (md5('stub moiety amoxicillin anhydrous')::uuid, md5('stub class Penicillin-class Antibacterial [EPC]')::uuid, 'has_EPC'),
    (md5('stub moiety amoxicillin')::uuid,           md5('stub class Renal Excretion [PK]')::uuid,                 'has_PK');
