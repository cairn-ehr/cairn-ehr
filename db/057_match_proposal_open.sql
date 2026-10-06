-- db/057_match_proposal_open.sql
-- Repair path R5a (#680; design page "R5a — the banner, designed 2026-10-06").
--
-- WHAT: the proposals that still need a human. R4's worker writes match_proposal rows (db/017);
-- a human answers one from the possible-duplicate banner (R5a) or, later, the worklist (R5b).
-- This view is the ONE predicate both read, so they can never disagree on what is "open".
--
-- A ROW IS OPEN WHEN, all three:
--   1. its status is one a human judgement may still move — exactly chart_link.rs's
--      OPEN_PROPOSAL_STATUSES (pinned by tests/match_proposal_open.rs);
--   2. its two charts do NOT read as one record (same person_member.person_id, db/018). Any
--      standing link joins them — attested or not; a doubted un-attested link is R1b's to show,
--      not this view's to hide behind. A chart never touched by linkage has no person_member
--      row and so is never "the same record" as anything;
--   3. there is NO ATTESTED unlink for the pair in patient_link. ONLY attested: unlinks are not
--      veto-gated and the ADR-0030 agent writer can author one, so counting an un-attested
--      unlink would let any unreviewed writer silently clear a duplicate banner (the mirror of
--      R1b's ruling that an un-attested unlink is not a doubt). `attested` is the STORED column
--      (ADR-0076 decision 5) — never re-derive it through event_log.
--
-- CONVERGENCE WITHOUT SYNCING match_proposal: the proposal table is node-local and does not
-- replicate. A colleague's attested judgement arriving by sync changes patient_link /
-- person_member, and this view drops the pair at read time — no status write anywhere. A local
-- judgement also moves the row's status (chart_link.rs), which (1) catches first.
--
-- An explicit column list, never mp.*: every db/*.sql replays on every connect, so an expanded
-- * would silently widen this view the day match_proposal gains a column. Its shape changes
-- only when this file says so, never as a side effect of a table ALTER.
CREATE OR REPLACE VIEW match_proposal_open AS
SELECT mp.patient_low, mp.patient_high, mp.score_total, mp.band, mp.veto_findings,
       mp.evidence, mp.matcher_version, mp.status, mp.created_at, mp.updated_at
  FROM match_proposal mp
 WHERE mp.status IN ('pending', 'accepted', 'review')
   AND NOT EXISTS (
         SELECT 1
           FROM person_member a
           JOIN person_member b ON b.person_id = a.person_id
          WHERE a.patient_id = mp.patient_low
            AND b.patient_id = mp.patient_high)
   AND NOT EXISTS (
         SELECT 1
           FROM patient_link pl
          WHERE pl.low = mp.patient_low
            AND pl.high = mp.patient_high
            AND pl.state = 'unlink'
            AND pl.attested);

-- The view is not security_invoker, so it reads with its owner's rights. It adds no reach for
-- cairn_agent beyond what db/017 and db/018 already grant it — SELECT on match_proposal,
-- person_member and patient_link (pinned by tests/match_proposal_open.rs).
GRANT SELECT ON match_proposal_open TO cairn_agent;
