-- Cairn — globalise the authored legibility twin (ADR-0039, refines ADR-0012/0034).
--
-- Every event type now carries an author-materialised §3.13/§4.5 plaintext twin. The floor
-- PREFERS the authored twin; for non-demographic types it degrades HONESTLY to a derived
-- skeleton when the author omitted it (older / non-conformant peer), so set-union convergence
-- is never broken. Demographic types keep ADR-0034's HARD requirement. submit_event (db/005)
-- is reused verbatim — only the cairn_event_twin hook changes (single-source door, no drift).

BEGIN;

-- Improved mechanical fallback: now renders the PAYLOAD too (closes the db/005 TODO), so a
-- derived twin is still genuinely legible. Crude + deterministic by design.
-- NOTE: this is a LOCAL projection — another node's renderer may produce a different derived twin
-- for the same twin-less event; the signed body (not the twin) is the convergent set-union artifact.
CREATE OR REPLACE FUNCTION cairn_twin_skeleton(p_type text, b jsonb)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT format('[%s] %s for patient %s%s',
                  p_type,
                  b ->> 'schema_version',
                  b ->> 'patient_id',
                  CASE WHEN b -> 'payload' IS NULL THEN ''
                       ELSE E'\n' || jsonb_pretty(b -> 'payload') END);
$$;

-- (The per-type twin dispatch moved to the db/005 registry dispatcher — #173/ADR-0048.
--  This migration keeps the improved skeleton + the twin-provenance read surfaces below.)

-- Read-time provenance: was the twin author-materialised, or derived by the floor? Recovered
-- from the immutable signed body (the author either signed a non-empty plaintext_twin or did
-- not), so no stored flag is needed. cairn_body is the pgrx COSE/CBOR parser (db/005 dependency).
CREATE OR REPLACE FUNCTION cairn_twin_is_authored(p_signed bytea)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT cairn_twin_is_present(t)
    FROM (SELECT cairn_body(p_signed) ->> 'plaintext_twin' AS t) s;
$$;

-- Both provenance facts from ONE verify+parse (issue #109 review): a row's `verifiable`
-- and `twin_authored` both derive from the single `cairn_body` call, so the view does one
-- full COSE+Ed25519 verification per row, not two. (The naive form —
-- `cairn_twin_is_authored(x)` AND `cairn_verify(x)` — verifies each row TWICE, since
-- cairn_twin_is_authored already verifies via cairn_body.) PL/pgSQL, not SQL: it holds the
-- body in a variable so the planner cannot re-inline cairn_body into two calls.
-- `verifiable := body IS NOT NULL` means "verifies AND parses" — a hair stricter than a bare
-- signature check, but the difference (a signed body that fails to re-serialize) is
-- unreachable for a well-formed EventBody and degrades SAFE (surfaced, never hidden).
CREATE OR REPLACE FUNCTION cairn_twin_provenance_of(p_signed bytea)
RETURNS TABLE(twin_authored boolean, verifiable boolean)
LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_body jsonb := cairn_body(p_signed);
    v_twin text  := v_body ->> 'plaintext_twin';
BEGIN
    twin_authored := cairn_twin_is_present(v_twin);
    verifiable    := v_body IS NOT NULL;
    RETURN NEXT;
END;
$$;

-- Worklist surface for a future re-authoring / duplicate-sweep / audit pass: which stored
-- events carry an author-faithful twin vs a best-effort derived one.
--
-- `twin_authored` folds a verification failure into "not authored": for a row whose bytes no
-- longer verify (a pre-ADR-0040 legacy row in an upgraded-in-place dev DB), cairn_body returns
-- NULL and the row reports twin_authored=false — indistinguishable from a genuine author-omitted
-- twin. A worklist that then re-derived skeletons would clobber genuinely-authored twins. So the
-- view ALSO exposes `verifiable` (issue #109): consumers filter on `WHERE verifiable` (or handle
-- `verifiable=false` as "no longer verifies", NOT "author omitted the twin"). Columns stay in the
-- prior (event_id, twin_authored, verifiable) order so CREATE OR REPLACE VIEW is additive.
CREATE OR REPLACE VIEW event_twin_provenance AS
    SELECT el.event_id, p.twin_authored, p.verifiable
    FROM event_log el
    CROSS JOIN LATERAL cairn_twin_provenance_of(el.signed_bytes) p;

GRANT SELECT ON event_twin_provenance TO cairn_agent;

-- #453 — the db/015 half of the cairn_twin_% REVOKE family (db/005 carries the other two).
--
-- Neither is an exposure: both take the signed bytes as an ARGUMENT, so a caller must already
-- hold what they parse. What the REVOKE buys is the same thing #382 and #443 bought — a
-- convention a reader can verify, instead of one followed by all-but-two of a family.
REVOKE EXECUTE ON FUNCTION cairn_twin_is_authored(bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_twin_provenance_of(bytea) FROM PUBLIC;

-- ...and the two grants that keep the surface above WORKING, which are the load-bearing half.
--
-- PostgreSQL checks *table* access inside a normal view against the VIEW OWNER, but a
-- *function* called inside that view against the INVOKING user. (CREATE VIEW's documentation
-- says so: "functions called in the view are treated the same as if they had been called
-- directly from the query using the view".) Carrying the table rule across to functions is the
-- easy mistake, so this was MEASURED on PG 18.1 rather than reasoned about — as cairn_agent,
-- with the REVOKE above in place and no grant:
--
--     ERROR:  permission denied for function cairn_twin_provenance_of
--
-- and, with only the outer function granted, the INNER call is checked too:
--
--     ERROR:  permission denied for function cairn_twin_is_present
--     CONTEXT:  PL/pgSQL function cairn_twin_provenance_of(bytea) line 6 at assignment
--
-- Hence TWO grants for a one-function view. cairn_twin_is_present is defined and revoked in
-- db/005; its grant lives here because the NEED lives here.
--
-- This is still a narrowing: PUBLIC (every role on the node, including ones a future migration
-- adds) loses EXECUTE; one named role keeps it. cairn_node is deliberately NOT granted — it
-- does not hold SELECT on the view either, so granting it would widen past the declared
-- surface, and #425 owns the question of which role the runtime should be.
--
-- Rejected: making cairn_twin_provenance_of SECURITY DEFINER to dodge the inner check. That
-- points owner privilege at a function taking arbitrary caller-supplied bytea into the pgrx
-- COSE parser, and adds another search_path-pinning obligation (#426) — a worse trade for what
-- is a legibility fix.
--
-- Pinned by crates/cairn-node/tests/floor_execute_grants.rs
-- (the_declared_twin_provenance_read_surface_still_works): delete either grant and it fails.
GRANT EXECUTE ON FUNCTION cairn_twin_provenance_of(bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_twin_is_present(text) TO cairn_agent;

COMMIT;
