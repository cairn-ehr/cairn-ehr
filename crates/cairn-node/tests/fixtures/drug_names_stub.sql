-- drugref's searchable moiety names, for the drug read door's tests (db/076, charting slice 3).
--
-- WHY A SECOND STUB: drugref grew the names only in its db/070 + db/071 (drugref#231, 2026-11-13).
-- A deployment may run any older drugref, and the door must then answer exactly as before. So
-- `drug_stub.sql` alone IS the older drugref, and a test loads THIS file after it, in the same
-- transaction, to get the newer one. Like `drug_stub.sql` it has no BEGIN/COMMIT: load it only
-- inside a transaction that ends in ROLLBACK.
--
-- The two tables and the view are drugref's, copied from origin/main (db/070 §2, db/071 §2–§3,
-- 2026-11-15) with the column names and types kept and the `ingest_run` foreign key dropped (the
-- stub has no ingest_run table). The door reads only the view.
--
-- The rows are the names `drugref_synonyms` (a real drugref on :5532) holds for these moieties:
--   mesh_synonym   Adrenaline → epinephrine, Lignocaine → lidocaine, Amoxycillin → amoxicillin
--                  (amoxicillin is ALSO found by its display name: listed once), Cefalexin →
--                  cephalexin, and Racemic Adrenaline + "Adrenaline, Racemic Mixture" → the
--                  racemate epinephrine,(+/-)- (two names of one drug, only one starting "adren")
--   unii_official  ACETAMINOPHEN → paracetamol, CEFALEXIN → cephalexin anhydrous (so "cefalexin"
--                  names TWO moieties: the pick lists both)
-- Every uuid is computed (md5 of a label), never written out (house rule 6). amoxicillin is
-- `drug_stub.sql`'s row; the other five moieties are added here.
INSERT INTO drugref.substance_moiety (moiety_uuid, display_name)
SELECT md5('stub moiety ' || n)::uuid, n
  FROM unnest(ARRAY['epinephrine', 'lidocaine', 'paracetamol', 'cephalexin',
                    'cephalexin anhydrous', 'epinephrine,(+/-)-']) n;

CREATE TABLE drugref.moiety_official_name (
    moiety_uuid uuid   NOT NULL REFERENCES drugref.substance_moiety(moiety_uuid),
    name        text   NOT NULL,
    ingest_run  bigint NOT NULL,
    PRIMARY KEY (moiety_uuid, name)
);
CREATE TABLE drugref.moiety_mesh_synonym (
    moiety_uuid uuid   NOT NULL REFERENCES drugref.substance_moiety(moiety_uuid),
    name        text   NOT NULL,
    mesh_ui     text   NOT NULL,
    ingest_run  bigint NOT NULL,
    PRIMARY KEY (moiety_uuid, name)
);

INSERT INTO drugref.moiety_official_name (moiety_uuid, name, ingest_run)
VALUES (md5('stub moiety paracetamol')::uuid, 'ACETAMINOPHEN', 1),
       (md5('stub moiety cephalexin anhydrous')::uuid, 'CEFALEXIN', 1);
INSERT INTO drugref.moiety_mesh_synonym (moiety_uuid, name, mesh_ui, ingest_run)
VALUES (md5('stub moiety epinephrine')::uuid, 'Adrenaline', 'D004837', 1),
       (md5('stub moiety lidocaine')::uuid, 'Lignocaine', 'D008012', 1),
       (md5('stub moiety amoxicillin')::uuid, 'Amoxycillin', 'D000658', 1),
       (md5('stub moiety cephalexin')::uuid, 'Cefalexin', 'D002506', 1),
       -- Two other names of ONE drug where "adren" starts only the LONGER one (final review I1):
       -- the door must take the one the text starts, or Sign keeps the unfinished "adren".
       (md5('stub moiety epinephrine,(+/-)-')::uuid, 'Racemic Adrenaline', 'D004837', 1),
       (md5('stub moiety epinephrine,(+/-)-')::uuid, 'Adrenaline, Racemic Mixture', 'D004837', 1);

-- drugref db/071 §3's view, verbatim.
CREATE VIEW drugref.moiety_name AS
SELECT moiety_uuid, display_name AS name, 'display'::text AS name_kind
FROM drugref.substance_moiety
UNION ALL
SELECT moiety_uuid, name, 'unii_official'::text
FROM drugref.moiety_official_name
UNION ALL
SELECT s.moiety_uuid, s.name, 'mesh_synonym'::text
FROM drugref.moiety_mesh_synonym s
WHERE NOT EXISTS (SELECT 1 FROM drugref.substance_moiety m
                  WHERE m.moiety_uuid = s.moiety_uuid
                    AND lower(m.display_name) = lower(s.name))
  AND NOT EXISTS (SELECT 1 FROM drugref.moiety_official_name o
                  WHERE o.moiety_uuid = s.moiety_uuid
                    AND lower(o.name) = lower(s.name));
