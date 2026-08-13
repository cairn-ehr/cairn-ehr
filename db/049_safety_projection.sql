-- 049_safety_projection.sql — §5.9 part B (ADR-0063): the de-identified safety signal.
--
-- WHY: a sealed clinical body still owes a future clinician a warning. A sealed pregnancy
-- termination implies Rhesus sensitisation the next antenatal clinician must act on. This
-- file carries the ladders that decide HOW MUCH of that warning is published, the local-door
-- floor check on its shape, and the deployment-populated (EMPTY-shipped) class lookup that
-- the AUTHORING node consults pre-seal.
--
-- WHAT IS NOT HERE: the event_log column and the door wiring (db/005, db/020), and the read
-- model (this file's sections 6-7 add it). Nothing in this file withholds any content:
-- §5.9 part B emits and coarsens a SIGNAL. Enforcement is part C (#376).

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The severity ladder.
--
--    !!! THE `ELSE` IS THE DECISION, NOT AN OVERSIGHT !!!
--    An unrecognised severity ranks MAX — "assume the worst" — and that is the SAFE
--    direction for a safety signal, exactly as it is for a sensitivity grade (ADR-0062
--    decision 2). It deliberately differs from cairn_clock_grade_rank's ELSE 0 (db/040),
--    where an unknown value withholds REJECT power and 0 is safe. Here 0 would mute a
--    warning this node cannot interpret, on precisely the events most likely to matter.
--    "Fixing" this into consistency with db/040 reopens that hole.
--
--    Open vocabulary, no CHECK domain: a future peer's severity is admitted verbatim
--    (principle 11, additive-only). Gaps of 10 leave room to interpose terms later.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_severity_rank(s text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE s
        WHEN 'none'     THEN 0
        WHEN 'low'      THEN 10
        WHEN 'moderate' THEN 20
        WHEN 'high'     THEN 30
        WHEN 'critical' THEN 40
        ELSE 2147483647            -- unknown ⇒ most severe. See the comment above.
    END;
$$;

-- ---------------------------------------------------------------------------
-- 2. The disclosure ladder. Higher rank = COARSER = less disclosed.
--
--    Same ELSE discipline, pointed the other way and for the same reason: a rung this
--    node does not recognise must be treated as the COARSEST, never as "show everything".
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_rung_rank(r text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE r
        WHEN 'precise'   THEN 0
        WHEN 'kind'      THEN 10
        WHEN 'existence' THEN 20
        ELSE 2147483647            -- unknown ⇒ disclose nothing.
    END;
$$;

-- ---------------------------------------------------------------------------
-- 3. Sensitivity rank -> disclosure rung. §5.9 calls this ladder "policy-configured";
--    this slice ships the monotone default and files the deployment override.
--
--    KEYED ON THE RANK, NOT THE GRADE STRING, on purpose: ADR-0062's grade vocabulary is
--    open and its unknown-ranks-MAX inversion lives in cairn_sensitivity_rank. Keying on
--    the rank inherits both for free — a future grade interposed at rank 15 lands on
--    'kind', one at 25 lands on 'existence', and an unrecognised one lands on 'existence'
--    without anyone remembering to add it. Safe-default-by-omission, the same discipline
--    ADR-0062 decisions 2 and 10 use.
--
--    MONOTONE NON-DECREASING BY CONSTRUCTION: a higher grade can never disclose more.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_rung_for_rank(p_rank int)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN p_rank IS NULL  THEN 'existence'   -- no answer ⇒ disclose nothing
        WHEN p_rank <= 0     THEN 'precise'     -- routine, or no standing assertion at all
        WHEN p_rank <= 10    THEN 'kind'        -- sensitive
        ELSE                      'existence'   -- restricted, sequestered, unrecognised
    END;
$$;

-- ---------------------------------------------------------------------------
-- 4. The structural floor on the CLEAR safety field.
--
--    CALLED FROM db/005 (submit_event) ONLY — DELIBERATELY NOT FROM db/020.
--
--    ADR-0062 E2 says a STRUCTURAL check (the shape of the claim) is safe at both doors
--    while a CEREMONY check (who authored it) must stay local. Read naively this check is
--    structural, so it would belong at both. THAT READING IS WRONG HERE, AND THE REASON IS
--    BLAST RADIUS.
--
--    A sensitivity assertion IS an event: refusing a malformed one drops one assertion.
--    The safety signal is a FIELD ON A CLINICAL EVENT: refusing it at the apply door drops
--    the medication assertion it rides on off this node's chart. A defect in a
--    de-identified ADVISORY signal would then destroy CLINICAL CONTENT — ADR-0060's "a
--    defect on one line never invalidates another", and its harder corollary: the system
--    may fail to record an order, but it may never cancel one.
--
--    So this follows the clock_grade precedent (db/040): constrained where MINTED, read
--    permissively where it ARRIVES. A peer that sent a self-contradictory signal has
--    already published those bytes; refusing at apply un-discloses nothing, forks the event
--    set (#342), and costs clinical content as well. Section 7's read model is total
--    instead, and never surfaces a class the rung forbids.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_check_safety_signal(b jsonb) RETURNS void
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    s    jsonb := b -> 'safety';
    rung text;
BEGIN
    IF s IS NULL OR jsonb_typeof(s) = 'null' THEN
        RETURN;   -- absent: the overwhelmingly common case, and always legal.
    END IF;
    -- jsonb_typeof is checked POSITIVELY (= 'object'), never as a NOT-something: the
    -- fail-OPEN pattern issue #346 catalogues comes from comparing a NULL typeof.
    IF jsonb_typeof(s) <> 'object' THEN
        RAISE EXCEPTION 'safety: the signal must be a JSON object, got %', jsonb_typeof(s);
    END IF;

    rung := s ->> 'rung';
    IF COALESCE(rung, '') = '' THEN
        RAISE EXCEPTION 'safety: the signal must carry a non-empty rung (ADR-0063)';
    END IF;

    IF rung = 'precise' THEN
        IF COALESCE(btrim(s ->> 'class'), '') = '' THEN
            RAISE EXCEPTION 'safety: rung "precise" must carry a non-empty class — a precise rung with nothing precise in it is a claim about nothing (ADR-0063)';
        END IF;
    ELSIF s ? 'class' THEN
        -- The disclosure guard. A body claiming {"rung":"existence","class":"..."}
        -- publishes the class while asserting it is concealed, and a reader trusting the
        -- rung would render it as concealed while the class sat in the row.
        RAISE EXCEPTION 'safety: rung "%" must not carry a class — it would publish exactly what the rung says is withheld (ADR-0063)', rung;
    END IF;

    IF s ? 'severity' AND COALESCE(btrim(s ->> 'severity'), '') = '' THEN
        RAISE EXCEPTION 'safety: severity, when present, must be a non-empty string';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 5. The class lookup — the AUTHORING node's drug-knowledge seam.
--
--    Ships EMPTY, and the SQL mirror asserts it stays empty. Cairn ships the lookup
--    MECHANISM, never the drug knowledge: a seeded row would be an un-reviewable clinical
--    policy choice smuggled into "infrastructure" (principle 9) — the same discipline
--    db/048's sensitivity_category_map keeps. This table is also the seam the future
--    drugref slice populates.
--
--    KEYED ON THE PAIR (system, code), NEVER A BARE CODE: once drugref-clinical-drug
--    exists beside drugref-moiety, a bare-code key would collide across composition-tree
--    levels (ADR-0059 decision 5's argument, unchanged).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS safety_class_map (
    system   TEXT NOT NULL,
    code     TEXT NOT NULL,
    class    TEXT NOT NULL,
    severity TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (system, code)
);
GRANT SELECT ON safety_class_map TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON safety_class_map FROM PUBLIC;

--    A PURE lookup that yields a CANDIDATE. It authors nothing and is called ONLY
--    pre-seal, by the node that is writing the event — which by construction had a coding
--    authority in hand. A READER must never call it: a reader that re-derives makes the
--    §5.9 floor depend on holding drugref after all, which is precisely the failure
--    ADR-0059 decision 4 / #294 exist to prevent.
CREATE OR REPLACE FUNCTION cairn_safety_class_candidate(p_coding jsonb)
RETURNS TABLE (class text, severity text)
LANGUAGE sql STABLE AS $$
    SELECT m.class, m.severity
    FROM safety_class_map m
    WHERE m.system = (p_coding ->> 'system')
      AND m.code   = (p_coding ->> 'code');
$$;

-- Postgres grants EXECUTE to PUBLIC by default, and every role is a member of PUBLIC, so
-- an un-REVOKEd function is directly callable by a below-the-floor adversary with raw SQL
-- (the db/037 note, and issue #382's finding about the cairn_check_* family).
REVOKE EXECUTE ON FUNCTION cairn_check_safety_signal(jsonb) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) TO cairn_agent;

COMMIT;
