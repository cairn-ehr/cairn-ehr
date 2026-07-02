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

-- The generalised per-type twin hook. Demographic types: structural floor + HARD authored-twin
-- requirement (ADR-0034). Every other type: prefer the authored twin; derive+flag if absent
-- (ADR-0039 honest degradation). The authored-vs-derived flag is NOT stored here — it is
-- recoverable from signed_bytes via cairn_twin_is_authored below.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin        text    := b ->> 'plaintext_twin';
    v_authored    boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_demographic boolean := false;
BEGIN
    -- Per-type structural floor (demographics only, for now).
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_demographic := true;
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_demographic := true;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path, EVERY type).
    IF v_authored THEN
        RETURN v_twin;
    END IF;

    -- Absent/blank twin:
    --   demographic types HARD-require it (ADR-0034 — a twin-less demographic event is a
    --     same-version bug; an older node rejects the unknown type at classification).
    --   every other type degrades honestly to a flagged derived skeleton (ADR-0039).
    IF v_demographic THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

-- Read-time provenance: was the twin author-materialised, or derived by the floor? Recovered
-- from the immutable signed body (the author either signed a non-empty plaintext_twin or did
-- not), so no stored flag is needed. cairn_body is the pgrx COSE/CBOR parser (db/005 dependency).
CREATE OR REPLACE FUNCTION cairn_twin_is_authored(p_signed bytea)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT t IS NOT NULL AND length(regexp_replace(t, '\s+', '', 'g')) > 0
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
    twin_authored := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
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

COMMIT;
