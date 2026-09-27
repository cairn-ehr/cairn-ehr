-- db/055_link_precedence_refold.sql
-- Cairn — ADR-0076 decision 5 on an EXISTING database: re-decide every patient_link winner
-- under the attested-first order (repair path R2a).
--
-- WHAT THIS FILE DOES, AND WHAT ITS EXISTENCE DOES.
--
-- db/018 now ranks an attested link assertion above an un-attested one. That governs every
-- assertion APPLIED from now on. It does not revisit a winner the OLD order already chose:
-- on a database that has run, a human's unlink may already have been displaced by a later
-- machine link, and that machine link is still the stored winner, with the two charts
-- merged. Filling the new `attested` column cannot fix that — a column fill marks the
-- machine link "not attested" and leaves it standing.
--
-- The fix is to re-apply every link assertion through the new applier. Cairn already has
-- that pass: when a node's recorded schema generation differs from its binary's, the loader
-- runs a heal (cairn_reproject, db/039) that replays every replay-eligible event through its
-- heal-safe appliers. patient_link_apply is heal-safe and its order is total, so replaying
-- every link and unlink over the live table leaves exactly the winner the new order picks,
-- whatever the stored row held before; each replay also recomputes both endpoints'
-- component and the #190 flag from the standing winner.
--
-- So THIS FILE'S EXISTENCE IS LOAD-BEARING: it is the newest migration, so it moves
-- SCHEMA_GENERATION to 55, so every existing node heals on its next connect. Do not fold its
-- content back into db/018 "to tidy up" — without a generation change no heal runs.
--
-- The backfill below makes `attested` truthful for every standing row the moment this file
-- loads, before the heal: a reader between the two (none today; R5's worklist and db/054's
-- doubted-link check are candidates) never sees an attested winner reported as un-attested.
-- It is idempotent: once converged, no row matches and the UPDATE writes nothing.
--
-- Node loader only: cairn-sync loads no identity migration (#284).

BEGIN;

UPDATE patient_link pl
   SET attested = TRUE
  FROM event_log el
 WHERE el.content_address = pl.content_address
   AND NOT pl.attested
   AND el.attester_key IS NOT NULL
   AND cairn_attestation_vouched(el.event_id);

COMMIT;
