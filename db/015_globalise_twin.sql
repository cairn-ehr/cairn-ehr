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

-- Worklist surface for a future re-authoring / duplicate-sweep / audit pass: which stored
-- events carry an author-faithful twin vs a best-effort derived one.
CREATE OR REPLACE VIEW event_twin_provenance AS
    SELECT event_id, cairn_twin_is_authored(signed_bytes) AS twin_authored
    FROM event_log;

GRANT SELECT ON event_twin_provenance TO cairn_agent;

COMMIT;
