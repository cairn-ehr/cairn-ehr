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
-- BUT THE HEAL ALONE IS NOT ENOUGH, so this file also re-folds the pairs that need it itself
-- (the block at the end). The heal is keyed on a generation number cairn-sync SHARES:
-- `cairn-sync init` runs the same heal and stamps the same SCHEMA_GENERATION, yet loads no
-- identity migration — so on a database both binaries share, a cairn-sync that connects first
-- after the upgrade replays every link through the OLD db/018 applier and stamps 55. cairn-node
-- then reads 55 == 55 and skips its own heal, and a human unlink the old order displaced stays
-- displaced, with nothing logged (PR #698 review). The block below does not depend on which
-- loader got here first.
--
-- The backfill below makes `attested` truthful for every standing row the moment this file
-- loads, before the heal: a reader between the two (none today; R5's worklist and db/054's
-- doubted-link check are candidates) never sees an attested winner reported as un-attested.
-- It is idempotent: once converged, no row matches and the UPDATE writes nothing. It is also
-- MONOTONE — it only ever flips FALSE → TRUE. A spurious TRUE (tampering, or a future bug)
-- is sticky: replaying that event (FALSE) loses to it and nothing here lowers it; only a
-- REBUILD (cairn_reproject with p_rebuild = true) corrects one. Like every
-- migration it runs on EVERY connect (connect_and_load_schema replays all files); the cost
-- once converged is a probe of each un-attested row (an index lookup on content_address),
-- with no writes.
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

-- THE RE-FOLD, for exactly the pairs the old order can have decided differently: those whose
-- standing winner is UN-attested while a VOUCHED link or unlink for the pair (the one
-- definition again) is in the log. If the standing winner is attested it was also the latest
-- attested assertion, so the new order agrees; if no attested assertion exists, the new order
-- IS the old one. Re-applying every vouched assertion of such a pair through
-- patient_link_apply leaves the attested-first winner whatever order they run in (the
-- comparator is total), and recomputes both components and the #190 flag from it.
--
-- Converges: afterwards no un-attested winner has a vouched assertion beside it, so on every
-- later connect the loop's query matches nothing and nothing is written. The loop's query
-- reads the table as it stood when the loop began, so a pair re-decided part-way still has
-- all of its vouched assertions replayed. Lenient apply posture, like the heal (db/039), so a
-- replicated event is never refused here on a node-local cap. A sealed row projects nothing
-- (patient_link_apply's own guard); the CASE keeps the subject casts from ever reading a
-- sealed row's ciphertext.
DO $refold$
DECLARE
    r record;
BEGIN
    PERFORM set_config('cairn.remote_apply', 'on', true);  -- true = transaction-local
    FOR r IN
        WITH vouched AS (
            SELECT el,
                   CASE WHEN el.sealed THEN NULL ELSE
                       LEAST((el.body ->> 'subject_a')::uuid,
                             (el.body ->> 'subject_b')::uuid) END AS lo,
                   CASE WHEN el.sealed THEN NULL ELSE
                       GREATEST((el.body ->> 'subject_a')::uuid,
                                (el.body ->> 'subject_b')::uuid) END AS hi
              FROM event_log el
             WHERE el.event_type IN ('identity.link.asserted', 'identity.unlink.asserted')
               AND el.attester_key IS NOT NULL
               AND cairn_attestation_vouched(el.event_id)
               AND cairn_replay_eligible(el)
        )
        SELECT v.el
          FROM vouched v
          JOIN patient_link pl ON pl.low = v.lo AND pl.high = v.hi
         WHERE NOT pl.attested
    LOOP
        PERFORM patient_link_apply(r.el);
    END LOOP;
END;
$refold$;

COMMIT;
