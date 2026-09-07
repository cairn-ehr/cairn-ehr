-- Cairn — the clinical capture's source of truth (#500 slice 2c).
--
-- WHY THIS FILE EXISTS. "A shredded body's key must not travel" had TWO hand-written
-- spellings in two crates — `NOT EXISTS` in the local-state export, `LEFT JOIN … CASE WHEN`
-- in cairn-sync's serve door — and slice 2c's capture would have been a third. That is the
-- mirror-list defect class this repo keeps paying for (#182, #404, #441), with a SAFETY
-- predicate as the mirrored thing. ADR-0001 (fat Postgres, thin daemon) says where it goes:
-- one definition, in the floor, inherited by every caller including one talking raw SQL.
--
-- ⚠️ PRIVILEGE. db/037 REVOKEs event_dek from PUBLIC *and* from cairn_agent, granting SELECT
-- only to cairn_node. A plain Postgres view reads its base tables as the VIEW'S OWNER, so an
-- unguarded view here would hand every role that can select it exactly the custody access
-- db/037 refused. `security_invoker = true` makes the view read as the CALLER, so db/037's
-- grants keep binding through it. Do not remove that option to "simplify"; the guard test
-- `custody_view_does_not_widen_access` exists because this is a decoy path around a floor
-- that looks correct at its own site (the #430/#431 shape).

BEGIN;

CREATE OR REPLACE VIEW event_custody_surviving
    WITH (security_invoker = true) AS
    SELECT d.event_id, d.dek_wrapped
      FROM event_dek d
     WHERE NOT EXISTS (
         SELECT 1 FROM erasure_shred_log s WHERE s.target_event_id = d.event_id
     );

REVOKE ALL ON event_custody_surviving FROM PUBLIC;
GRANT SELECT ON event_custody_surviving TO cairn_node;

-- One page of the clinical plane, in the shape a peer response and a medium segment both
-- need. `page_limit` is BIGINT and NULL means "no limit" — Postgres reads LIMIT NULL as
-- unlimited, so the unpaginated serve path stays the SAME statement with a NULL parameter
-- rather than becoming a second query that could drift from this one.
--
-- The +1 PROBE stays at the CALL SITE, deliberately. `rows.len() == limit` cannot tell "the
-- log ends exactly here" from "there is one more we cut off", and `complete` is the puller's
-- only termination signal (slice 2b). The caller owns that claim, so the caller asks for
-- limit + 1; teaching this function to do it would put the answer in a place that cannot see
-- who is asking.
CREATE OR REPLACE FUNCTION cairn_clinical_page(after_seq BIGINT, page_limit BIGINT)
RETURNS TABLE (seq BIGINT, signed_bytes BYTEA, attestation BYTEA,
               attester_key BYTEA, dek_wrapped BYTEA)
LANGUAGE sql STABLE AS $$
    SELECT e.seq, e.signed_bytes, e.attestation, e.attester_key, c.dek_wrapped
      FROM event_log e
      LEFT JOIN event_custody_surviving c ON c.event_id = e.event_id
     WHERE e.seq > after_seq
     ORDER BY e.seq
     LIMIT page_limit;
$$;

REVOKE EXECUTE ON FUNCTION cairn_clinical_page(BIGINT, BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_clinical_page(BIGINT, BIGINT) TO cairn_node;

COMMIT;
