-- db/019_apply_proposal.sql
-- §5.2/§5.7 C2 apply seam — the additive column linking an applied proposal to the
-- identity event it produced.
--
-- WHAT: one nullable column on the advisory match_proposal worklist (db/017). When the
-- C2 seam (cairn-node::apply_proposal) turns a human-ACCEPTED proposal into a real
-- identity.link.asserted event, it records that event's id here and flips status to
-- 'applied' in the SAME transaction as submit_event. This closes the loop (proposal ->
-- which link event) and makes re-application idempotent (only status='accepted' rows
-- are picked up).
--
-- INVARIANT (enforced by the CHECK at the foot of this file since #704; every seam's single
-- transaction also keeps it):
--   applied_event_id IS NOT NULL  <=>  status IN ('applied','auto_applied').
--   C2 (human-accepted, cairn-node::apply_proposal) sets status='applied'; C2b (matcher
--   auto-apply, cairn-node::auto_apply) sets status='auto_applied' — both record the id of
--   the identity.link.asserted event they produced, and the two statuses stay distinct so
--   the audit trail tells auto from human. A consumer asking "did this proposal produce a
--   link?" must therefore test `applied_event_id IS NOT NULL`, NOT `status='applied'`
--   alone (which silently misses every auto-applied link).
--
-- Additive: no event-format change, no submit_event change, no new event type. The
-- existing GRANT ... UPDATE ON match_proposal TO cairn_agent (db/017) already permits
-- the mark-applied write; no new grant is needed.

ALTER TABLE match_proposal ADD COLUMN IF NOT EXISTS applied_event_id UUID;

-- #704 — the invariant above, in the floor. It used to be held by convention in every writer
-- (the matcher's upsert/retract, auto_apply.rs, chart_link.rs, apply_proposal.rs); any other
-- writer could break it, and a row that is 'pending' while still naming a link event reads as
-- LINKED to every consumer that follows this header's advice to test applied_event_id. The
-- matcher's ADR-0078 reopen (auto_applied -> pending) clears the id in the same UPDATE, which is
-- exactly the shape this admits. Same idiom as db/017's band and status checks: guarded on the
-- deparsed definition (idempotent, write-free once current) and NOT VALID, so a pre-existing
-- bad advisory row cannot abort a schema load. It is not made harmless: the next UPDATE of that
-- row raises 23514 and stalls that pair's duplicate check, loudly, until it is repaired by hand.
-- Only a raw-SQL write could have left one — auto_apply.rs and this file have set the id together
-- since their first commit. Pinned by db/tests/019.
DO $$
DECLARE
    want CONSTANT text :=
        'CHECK (((applied_event_id IS NOT NULL) = (status = ANY (ARRAY[''applied''::text, '
        '''auto_applied''::text]))))';
    have text;
BEGIN
    SELECT regexp_replace(pg_get_constraintdef(oid), '\s+NOT VALID$', '') INTO have
      FROM pg_constraint
     WHERE conname = 'match_proposal_applied_event_check'
       AND conrelid = 'match_proposal'::regclass;
    IF have IS DISTINCT FROM want THEN
        ALTER TABLE match_proposal DROP CONSTRAINT IF EXISTS match_proposal_applied_event_check;
        ALTER TABLE match_proposal
            ADD CONSTRAINT match_proposal_applied_event_check
            CHECK ((applied_event_id IS NOT NULL) = (status IN ('applied', 'auto_applied')))
            NOT VALID;
    END IF;
END $$;
