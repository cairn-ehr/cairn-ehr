-- 049_safety_projection.sql — §5.9 part B (ADR-0063): the de-identified safety signal.
--
-- WHY: a sealed clinical body still owes a future clinician a warning. A sealed pregnancy
-- termination implies Rhesus sensitisation the next antenatal clinician must act on. This
-- file carries the ladders that decide HOW MUCH of that warning is published, the local-door
-- floor check on its shape, and the deployment-populated (EMPTY-shipped) class lookup that
-- the AUTHORING node consults pre-seal.
--
-- It also carries the READ model (sections 6-7): the grade for an event not yet written,
-- and the total, re-coarsening read that makes the sync door's leniency safe.
--
-- WHAT IS NOT HERE: any withholding of content. §5.9 part B emits and coarsens a SIGNAL;
-- coarsening the signal is not access control, and a caller that holds custody still reads
-- the body exactly as before. Enforcement is part C (#376).

BEGIN;

-- ---------------------------------------------------------------------------
-- 0. The clear signal's home: an additive column on the append-only row.
--
--    WHY A COLUMN AND NOT A PROJECTION TABLE. §5.9 requires the safety projection to
--    OUTLIVE the body it protects — to coarsen but survive a rung-3 crypto-shred. A
--    projection table would have to be explicitly EXEMPTED from cairn_execute_shred's
--    scrub (db/037), which is a standing invitation for a future reviewer to "fix" the
--    inconsistency and silently delete the one signal the spec says must survive. On the
--    append-only row it survives because event_log is never touched by a shred: the
--    guarantee is structural rather than remembered. It also needs no apply function and
--    no ADR-0057 registry entry, so no registry row-count pin moves.
--
--    It is a DERIVED VIEW of the signed bytes, exactly like `body` and `clock_grade` —
--    stored verbatim, never sanitized on the way in. Section 4 explains why the
--    interpretation, not the storage, is where a contradiction is refused.
--
--    ADD COLUMN IF NOT EXISTS does not fire the append-only trigger (that fires on
--    UPDATE/DELETE) — the same additive move db/001 makes for attestation/attester_key.
-- ---------------------------------------------------------------------------
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS safety JSONB;

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
    -- IS DISTINCT FROM, never a bare `<> 'object'`. jsonb_typeof(NULL) is SQL NULL, and
    -- `NULL <> 'object'` evaluates to NULL — which `IF` treats as FALSE, so a NULL typeof
    -- would fall THROUGH this guard into the rung checks below instead of being refused.
    -- That is the fail-OPEN pattern issue #346 catalogues, and it is the repo idiom to
    -- exclude it structurally rather than by argument (db/048 section 3, db/045 section 2).
    --
    -- Today the RETURN above already eliminates every value that could make jsonb_typeof(s)
    -- return NULL, so the bare form would be non-exploitable — which is exactly why it is
    -- written the safe way anyway: the guarantee must not depend on two checks staying
    -- adjacent across a future reordering.
    IF jsonb_typeof(s) IS DISTINCT FROM 'object' THEN
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

    -- The SECOND disclosure guard, mirroring the class guard above one rung down
    -- (2026-08-14 review finding). Section 7 gates severity off at 'existence' — "there is
    -- a safety-relevant signal here and you are not cleared to see what"; a severity beside
    -- it narrows exactly that. Without this arm the door ADMITTED the shape the read model
    -- refuses to show, so the bytes were minted and replicated permanently while every
    -- honest reader declined to surface them — the door and the read model disagreeing
    -- about the same rung, with the door on the side that cannot be undone.
    --
    -- Keyed on the rung reaching or passing 'existence' rather than on the literal string,
    -- so a coarser rung interposed later inherits the guard without anyone remembering —
    -- the same safe-default-by-omission discipline sections 1-3 use.
    --
    -- ADR-0060: this cannot fail a clinical write. `cairn_event::safety::coarsen` is total
    -- over three fixed shapes and its Existence arm emits `{"rung":"existence"}` alone, so
    -- no in-repo builder can construct the refused shape — the identical argument that
    -- licenses the class guard above.
    IF s ? 'severity'
       AND cairn_safety_rung_rank(rung) >= cairn_safety_rung_rank('existence') THEN
        RAISE EXCEPTION 'safety: rung "%" must not carry a severity — it narrows exactly what the rung says is withheld (ADR-0063)', rung;
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

-- ---------------------------------------------------------------------------
-- 6. The PROSPECTIVE grade — the grade for an event that does not exist yet.
--
--    cairn_effective_sensitivity takes an event_id, and at emission time the event has
--    not been written. This is the same computation MINUS the precisely-targeted event
--    arm: an event about to be authored can carry no assertion naming it.
--
--    !!! KEEP IN LOCKSTEP WITH db/048 SECTION 11 !!! The two functions duplicate the
--    chart / thread / catch-all arms. crates/cairn-node/tests/safety_read.rs's
--    prospective_matches_effective_given_the_same_chart_and_thread and
--    a_dangling_event_scoped_assertion_coarsens_prospectively_too are the anti-drift pins.
--
--    WHAT THOSE PINS DO NOT COVER, STATED SO NOBODY TREATS A GREEN RUN AS PROOF
--    (2026-08-14 review). The first passes p_thread = NULL and uses a `note.added`, which
--    db/048 section 10b classifies as thread-free — so it pins
--    prospective(patient, NULL) == effective(event) for a THREAD-LESS event, not the
--    thread arms. Agreement for a thread-BEARING event is unpinned (#399).
--
--    THE THREAD ARM DRIFTED ONCE AND WAS REPAIRED (#404) — do not re-introduce it. It read
--    `p_thread IS NULL OR s.subject_id <> p_thread`, which with the matched arm above was
--    EXHAUSTIVE over thread-scoped assertions, so p_thread was inert and a thread grade
--    coarsened unconditionally. The catch-all now asks db/048's POSITIVE question. The pin
--    is crates/cairn-node/tests/safety_emission.rs's
--    a_grade_on_another_thread_of_the_same_chart_does_not_coarsen_this_one, which lives in
--    that suite rather than safety_read.rs because it needs CUSTODY for
--    cairn_thread_patient to resolve a thread at all.
--
--    Both delegate to cairn_sensitivity_standing, which stays the SINGLE definition of
--    "what still applies" (ADR-0062 decision 3).
--
--    WHY THE DUPLICATION IS NOT REFACTORED AWAY. The obvious "fix" is to make
--    cairn_effective_sensitivity call this one. It cannot: section 11's thread branch is
--    driven by the event's OWN type (the section 10b type gate) and by the thread the event
--    resolves to, neither of which exists before the event does. Two functions with one
--    warning comment and two tests is the honest shape; one function with a nullable
--    event_id would hide the difference behind a branch.
--
--    THE ASYMMETRY THAT MATTERS: this function is allowed to be MORE conservative than
--    section 11, never less. Over-coarsening at emission publishes a vaguer signal than
--    strictly necessary — recoverable, and visible. Under-coarsening publishes a precise
--    class in the clear on the wire that every subsequent read on this very node will then
--    refuse to show: unrecoverable, because bytes already sent cannot be recalled.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_prospective_sensitivity(p_patient uuid, p_thread uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE sql STABLE AS $$
    WITH standing AS (
        SELECT s.* FROM cairn_sensitivity_standing(p_patient) s
    ),
    applicable AS (
        -- chart-scoped, correctly targeted
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s
        WHERE s.subject_kind = 'patient' AND s.subject_id = p_patient
        UNION ALL
        -- thread-scoped, this thread
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s
        WHERE s.subject_kind = 'thread' AND p_thread IS NOT NULL AND s.subject_id = p_thread
        UNION ALL
        -- The catch-all (ADR-0062 erratum E1): an assertion we cannot match to a subject
        -- here still coarsens chart-wide, reported as 'coarsened' rather than echoing its
        -- own kind. Four causes, matching section 11's arm for arm:
        --
        --   * an UNRECOGNISED kind — a future peer's vocabulary (section 11's first clause);
        --   * a 'patient' assertion naming a DIFFERENT chart (mis-target);
        --   * a thread-scoped assertion when we have NO thread to compare against — an
        --     unresolved thread is decision 9's conservative bound, and at emission time it
        --     is the honest reading of "this event may be on that thread";
        --   * an 'event' assertion whose target is not on this chart. THIS ONE IS EASY TO
        --     DROP BY MISTAKE while reading "minus the event arm" as "minus everything
        --     event-shaped". It is not the precisely-targeted arm: it is the arm that fires
        --     when an event-scoped assertion names an event we cannot find here — a wrong
        --     chart, a dangling id, or (most often, and legitimately) an event that has
        --     simply not replicated yet, since set-union sync has no ordering. Section 11
        --     coarsens the WHOLE chart in that case, so every READ of the event we are
        --     about to author will say 'existence'. If emission did not agree, it would
        --     publish a precise class this node then declines to display. Fully computable
        --     before the new event exists, so there is no excuse for the divergence.
        SELECT s.grade, 'coarsened'::text, s.content_address
        FROM standing s
        WHERE s.subject_kind NOT IN ('patient', 'thread', 'event')
           OR (s.subject_kind = 'patient' AND s.subject_id <> p_patient)
           -- THE THREAD ARM ASKS THE POSITIVE QUESTION, LIKE db/048's (#404 fixed this).
           --
           -- It used to read `p_thread IS NULL OR s.subject_id <> p_thread`, which together
           -- with the matched arm above was EXHAUSTIVE over thread-scoped assertions — so a
           -- thread grade coarsened unconditionally and `p_thread` was inert. That made
           -- thread-scoping behave as chart-scoping (what db/048 section 10b's type gate
           -- exists to prevent) and made emission disagree with section 11 on the same node:
           -- emission published `existence` for an event `cairn_effective_sensitivity` calls
           -- `routine`, i.e. a break-glass prompt beside "grade routine".
           --
           -- Now, mirroring db/048's own arm: fire only when the named thread is
           -- DEMONSTRABLY on another chart. `cairn_thread_patient` returning NULL means
           -- "cannot tell" — the NORMAL state on a custody-less node, where
           -- medication_statement is empty — so it coalesces to this chart and stays
           -- silent rather than coarsening every chart everywhere. Asking the 'event' arm's
           -- ABSENCE question here would do exactly that; db/048 spells out why at length.
           --
           -- `p_thread IS NULL` still coarsens: at emission an unresolved thread is decision
           -- 9's conservative bound, the honest reading of "this event MAY be on that
           -- thread".
           --
           -- IT IS DELIBERATELY NOT GATED by `cairn_event_type_has_no_thread` the way
           -- db/048's equivalent arm is — this function takes no event type. A future
           -- thread-FREE clinical verb would therefore inherit the bound and re-open the
           -- #404 divergence through a different door: coarsened to `existence` at emission,
           -- computed `routine` at every read. Harmless TODAY because every event type that
           -- reaches the emission seam is thread-bearing, and that precondition is a
           -- TRIPWIRE, not a hope — safety_ladder.rs's
           -- `every_clinical_event_type_is_thread_bearing_so_the_missing_gate_cannot_bite`
           -- fails the moment it stops holding, and its message says what to do.
           --
           -- WHEN YOU DO ADD THE PARAMETER, READ THIS FIRST. Postgres OVERLOADS on a changed
           -- argument list rather than replacing, and migration replay never drops what a
           -- file stops creating — so the 2-arg definition survives in every existing
           -- database and silently keeps serving any caller you miss. Verified. Use db/005's
           -- `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);` idiom, and update
           -- all five call sites together: this file's REVOKE/GRANT pair (whose signature
           -- string safety_ladder.rs pins via has_function_privilege — it would resolve the
           -- STALE function and pass while the new one kept PUBLIC's default), crates/
           -- cairn-node/src/safety.rs's `prospective_rung`, safety_read.rs's four direct
           -- calls, and safety_emission.rs's BREAK_GRADE_LOOKUP outage rig (a 2-arg raiser
           -- would become an overload BESIDE a working function, so the staged outage would
           -- stop staging anything).
           OR (s.subject_kind = 'thread'
               AND (p_thread IS NULL
                    OR COALESCE(cairn_thread_patient(s.subject_id), p_patient) <> p_patient))
           OR (s.subject_kind = 'event' AND NOT EXISTS (
                   SELECT 1 FROM event_log x
                   WHERE x.event_id = s.subject_id AND x.patient_id = p_patient))
    )
    -- The LEFT JOIN LATERAL over a one-row constant is what makes this return EXACTLY ONE
    -- row even when nothing applies, so callers can use query_one and read 'routine' rather
    -- than distinguishing "no row" from "not sensitive" (db/048 section 11's own idiom,
    -- copied verbatim so the two can be diffed arm by arm).
    SELECT COALESCE(a.grade, 'routine'),
           COALESCE(a.subject_kind, 'none'),
           a.content_address
    FROM (SELECT 1) AS one_row
    LEFT JOIN LATERAL (
        SELECT ap.grade, ap.subject_kind, ap.content_address
        FROM applicable ap
        -- Rank first; content_address breaks a tie between two equally-ranked grades
        -- deterministically (BYTEA has no collation — ADR-0045/#115).
        ORDER BY cairn_sensitivity_rank(ap.grade) DESC, ap.content_address ASC
        LIMIT 1
    ) a ON TRUE;
$$;

-- ---------------------------------------------------------------------------
-- 7. The read model. TOTAL over any stored shape — this is what makes db/020's leniency
--    (section 4) safe rather than merely lenient.
--
--    Three totality rules, each of which must hold whatever the row contains:
--      * an unrecognised OR MISSING rung reads as 'existence' (both rank MAX through
--        cairn_safety_rung_rank's ELSE — `safety ->> 'rung'` is SQL NULL when absent);
--      * a class is surfaced ONLY at rung 'precise' — a class beside a coarser rung is
--        ignored, always;
--      * the rung is the COARSER of what was emitted and what this node's CURRENT grade
--        licenses, because emission cannot control a peer's bytes and read cannot
--        un-publish one. A peer legitimately emits 'precise' when the chart is routine on
--        ITS node; the local grade is the local defence.
--
--    AT 'existence' NEITHER class NOR severity SURVIVES (maintainer ruling, 2026-08-13).
--    'existence' is the claim "there is a safety-relevant signal here and you are not
--    cleared to see what" — a severity beside it would narrow exactly that. severity
--    survives at 'precise' and 'kind' only.
--
--    !! SECURITY DEFINER IS REQUIRED, NOT STYLISTIC (#405 part 1) !!
--    Section 8 withholds `SELECT (safety)` from cairn_agent — that is what makes "the
--    sanctioned way to read the signal" a privilege rather than a convention. A
--    non-definer body runs as the CALLING role whether or not it inlines, so without
--    DEFINER this function fails with 42501 for the very role the product runs as, and
--    the coarsened read would be available to nobody. The pair is load-bearing in BOTH
--    directions: revoking the column without this clause breaks the read path, and this
--    clause without the revoke closes nothing.
--    Pinned by safety_read_grants.rs::the_sanctioned_read_still_works_as_cairn_agent_and_coarsens,
--    whose fixture lands a real signal and a real standing grade, then reads them back
--    through the role switch, so the body actually executes (the weak/strong pin
--    distinction ADR-0064 draws). cairn_agent holds no INSERT on event_log at all
--    (db/005), so the fixture necessarily lands its rows as the owner.
--
--    !! `pg_temp` MUST BE LISTED, AND LISTED LAST (2026-08-16 review) !!
--    `SET search_path = public` alone does NOT exclude the session's temporary schema:
--    Postgres searches pg_temp FIRST for RELATION names whenever the path does not place
--    it explicitly. While these functions were invoker-rights that bought an attacker
--    nothing — they already ran as the caller. Making them SECURITY DEFINER is precisely
--    what turns temp-table shadowing into a trust-boundary crossing, because they are now
--    the only sanctioned read of the signal and therefore the thing a warning surface
--    trusts. cairn_agent retains TEMPORARY on the database, so `CREATE TEMP TABLE
--    event_log (…)` in its own session made `cairn_event_safety` return FABRICATED rows —
--    or, in the direction that actually hurts, ZERO rows, silently suppressing a real
--    warning into main.rs's "no safety signals on file" reassurance path. Listing pg_temp
--    LAST makes public win for every unqualified name here. Pinned by
--    safety_read_grants.rs::a_caller_shadowed_temp_table_cannot_blind_the_sanctioned_read
--    and by the proconfig assertions in db/tests/049. Every OTHER definer in this repo still
--    carries the bare `public` form — audited in #426, not here.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_event_safety(p_event_id uuid)
RETURNS TABLE (rung text, class text, severity text, event_type text,
               grade text, subject_kind text)
LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $$
    WITH ev AS (
        -- No signal ⇒ NO ROW, deliberately. An 'existence' marker synthesised for every
        -- uncoded event would manufacture a warning from nothing, which is worse than
        -- silence: it trains clinicians to ignore the marker (ADR-0059 decision 4's
        -- honest floor). The same applies to a non-object `safety` — a peer that sent an
        -- array or a bare string said nothing this node can act on.
        --
        -- `= 'object'` rather than db/005's `IS DISTINCT FROM` idiom because the polarity
        -- is reversed here: this is a WHERE clause, so a NULL comparison EXCLUDES the row
        -- (no row ⇒ disclose nothing), which is the safe direction. In section 4 the same
        -- NULL would fall THROUGH a guard, which is the fail-open shape #346 catalogues.
        SELECT e.event_id, e.event_type, e.safety
        FROM event_log e
        WHERE e.event_id = p_event_id AND e.safety IS NOT NULL
          AND jsonb_typeof(e.safety) = 'object'
    ),
    graded AS (
        SELECT ev.*, s.grade, s.subject_kind,
               -- The coarser of the two, by rank (higher rank = coarser = less disclosed).
               -- Named `eff_rung` so the CASE below reads as "what may be disclosed", not
               -- "what was claimed".
               CASE WHEN cairn_safety_rung_rank(ev.safety ->> 'rung')
                       >= cairn_safety_rung_rank(cairn_safety_rung_for_rank(cairn_sensitivity_rank(s.grade)))
                    -- The emitted rung wins — but only after NORMALISATION. An emitted
                    -- rung this node cannot interpret (or an absent one) ranks MAX, i.e.
                    -- strictly coarser than the coarsest NAMED rung, and echoing the raw
                    -- string back would hand callers a value no reader knows how to
                    -- render. The test is written against the ladder itself rather than
                    -- against section 2's literal sentinel, so interposing a new rung or
                    -- changing the ELSE value cannot silently break it.
                    THEN CASE WHEN cairn_safety_rung_rank(ev.safety ->> 'rung')
                                   > cairn_safety_rung_rank('existence')
                              THEN 'existence'
                              ELSE ev.safety ->> 'rung' END
                    ELSE cairn_safety_rung_for_rank(cairn_sensitivity_rank(s.grade))
               END AS eff_rung
        -- cairn_effective_sensitivity always returns exactly one row (its own LEFT JOIN
        -- LATERAL over a constant), so this LATERAL never drops or duplicates the event.
        FROM ev, LATERAL cairn_effective_sensitivity(ev.event_id) s
    )
    SELECT g.eff_rung,
           CASE WHEN g.eff_rung = 'precise' THEN g.safety ->> 'class' END,
           CASE WHEN g.eff_rung IN ('precise', 'kind') THEN g.safety ->> 'severity' END,
           g.event_type, g.grade, g.subject_kind
    FROM graded g;
$$;

--    The chart-wide report: every event on the chart that CARRIES a signal, already
--    coarsened. One query, so a UI opening a chart pays one round trip (the §1.2 budget in
--    the slice plan).
--
--    NOT "every STANDING signal" — there is no supersession here (#406). A ceased
--    medication's assert keeps its line, and a corrected coding leaves the RETRACTED
--    class standing because `correct_medication_coding` emits nothing (#401). Thread
--    rollup is the separate design ADR-0063 declines to open; until it exists, currency
--    is the caller's problem and this function must not be described as solving it.
--
--    A plain (INNER) LATERAL, so an event whose `safety` is present but unusable — a JSON
--    array, a bare string, `'null'::jsonb` — contributes no row at all, exactly as
--    cairn_event_safety reports it one event at a time. The two must not disagree: a UI
--    that saw a row in the chart report and none on the detail read would show a warning it
--    could not then explain.
--
--    ORDERING PUTS THE WITHHELD SEVERITIES FIRST, AND THAT IS THE DECISION.
--    cairn_safety_severity_rank ranks an unrecognised severity MAX (section 1), and a
--    coarsened row's severity is SQL NULL — which lands on the same ELSE. So every signal
--    whose severity this reader is not cleared to see sorts ABOVE a known 'critical'.
--    Sorting them last would bury the one class of warning whose content is unknown
--    precisely because it was protected, which is the disclosure-adjacent failure this file
--    exists to avoid. A UI is free to group differently; the default must not hide.
--
--    SECURITY DEFINER for the same reason as cairn_event_safety, and INDEPENDENTLY of it:
--    this function's OWN `WHERE e.safety IS NOT NULL` touches the withheld column, so a
--    fix that made only the per-event reader a definer would leave the chart report broken
--    for cairn_agent. The same test pins both halves. `pg_temp` last for the reason spelled
--    out above cairn_event_safety — and independently here too, since this body names
--    `event_log` in its own FROM clause.
CREATE OR REPLACE FUNCTION cairn_patient_safety(p_patient uuid)
RETURNS TABLE (event_id uuid, rung text, class text, severity text, event_type text,
               grade text, subject_kind text)
LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $$
    SELECT e.event_id, s.rung, s.class, s.severity, s.event_type, s.grade, s.subject_kind
    FROM event_log e, LATERAL cairn_event_safety(e.event_id) s
    WHERE e.patient_id = p_patient AND e.safety IS NOT NULL
    ORDER BY cairn_safety_severity_rank(s.severity) DESC, e.event_id;
$$;

-- ---------------------------------------------------------------------------
-- #405 part 2 — a rung finer than the chart's grade licenses. RECORDED, never refused.
--
-- The door CANNOT refuse it: ADR-0060 forbids an advisory field cancelling a medication
-- assert, and rewriting event_log.safety would make the column disagree with signed_bytes
-- and quietly break the signature's meaning. So it takes ADR-0058's record-a-flag idiom.
--
-- !! LOCAL DOOR ONLY — AND THIS DELIBERATELY BREAKS THE PRECEDENT IT COPIES !!
-- cairn_record_ceiling_flag is called at BOTH doors (db/005:912 and db/020:145), so
-- local-only here reads as an oversight and WILL be tidied into symmetry. It is not:
--   * LOCALLY the node's own grade is authoritative for its own authoring, so a rung finer
--     than it licenses is unambiguously anomalous — apply_safety_rung was bypassed. THAT IS
--     NOT THE ONLY SOURCE, THOUGH: apply_safety_rung's own read (crate::safety::
--     prospective_rung) and this door's read at step 7a — which lives in db/005, NOT in
--     this file — are separate statements over
--     the SAME chart, and crates/cairn-node/src/safety.rs:105-115 declares the resulting
--     race — a chart-wide grade raised in the window between the two makes the daemon emit
--     a correctly-derived rung one step finer than the grade standing by the time this door
--     reads it, and this block records that as an overclaim against the daemon's own
--     correct output. Rare, advisory-only, and still genuine evidence of over-disclosure —
--     but "bypassed" would misname it; nothing was bypassed, the grade just moved.
--   * REMOTELY ADR-0063 decision 2 says this arrives ROUTINELY AND HONESTLY: an older peer
--     predating the slice, a differently-custodial peer computing a lower grade, and a
--     hostile peer all deliver identical bytes and cannot be told apart. Flagging there
--     would fire on ordinary traffic and accuse honest peers — §5.12 alert fatigue, in a
--     ledger nobody could then trust.
-- A clock grade is a claim about the authoring node's own clock and stays meaningful at
-- both doors; a safety rung is a claim about THIS chart's grade, which is node-relative.
-- Same idiom, different question.
--
-- A LEDGER and not a view (ADR-0064's rule): a published byte is permanent and can never
-- improve, so there is nothing to self-heal.
CREATE TABLE IF NOT EXISTS safety_overclaim_flag (
    content_address BYTEA PRIMARY KEY,
    patient_id      UUID        NOT NULL,
    emitted_rung    TEXT        NOT NULL,
    licensed_rung   TEXT        NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON safety_overclaim_flag TO cairn_agent;

-- Idempotent on replay: the PK is the content address, so a re-offered event re-records
-- the same row (the db/048 apply precedent, NOT the #254 bug — a conflict here means the
-- SAME event twice).
CREATE OR REPLACE FUNCTION cairn_record_safety_overclaim_flag(
    p_ca bytea, p_patient uuid, p_emitted text, p_licensed text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO safety_overclaim_flag (content_address, patient_id, emitted_rung, licensed_rung)
    VALUES (p_ca, p_patient, p_emitted, p_licensed)
    ON CONFLICT (content_address) DO NOTHING;
$$;

-- Postgres grants EXECUTE to PUBLIC by default, and every role is a member of PUBLIC, so
-- an un-REVOKEd function is directly callable by a below-the-floor adversary with raw SQL
-- (the db/037 note, and issue #382's finding about the cairn_check_* family).
REVOKE EXECUTE ON FUNCTION cairn_check_safety_signal(jsonb) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) TO cairn_agent;
-- Same posture as cairn_check_safety_signal just above, and for the identical reason: a
-- writer called ONLY from inside submit_event's SECURITY DEFINER context (2026-08-15
-- review, Minor #5 — cairn_record_ceiling_flag, the precedent this function otherwise
-- copies, leaves this open; closing it here rather than re-deriving the gap every read).
-- cairn_agent needs no grant on it at all — submit_event calls it as its owner.
REVOKE EXECUTE ON FUNCTION cairn_record_safety_overclaim_flag(bytea, uuid, text, text) FROM PUBLIC;

-- The read model. REVOKE before GRANT for each, same reason as above: these are the
-- SANCTIONED way to read the safety signal, and a reader that reaches the emitted value by
-- another route gets it UNCOARSENED — so the grant on them must be deliberate, not the
-- PUBLIC default.
--
-- STILL "SANCTIONED", NOT "ONLY" — AND SAYING OTHERWISE WAS WRONG (#405 part 1, 2026-08-16
-- review of the fix itself). Section 8 below closes the CONVENIENT path: db/005 does
-- `GRANT SELECT ON event_log ... TO cairn_agent`, a table-level grant covers columns added
-- later, so cairn_agent could `SELECT safety` raw and skip section 7's re-coarsening. That
-- is now a 42501. What section 8 does NOT close, and what an earlier draft of this comment
-- wrongly claimed it did:
--
--   (a) `event_log.safety` is a VERBATIM COPY of a clear top-level field of the signed body
--       (`b -> 'safety'`, db/005) — not an independent secret. `signed_bytes` is granted
--       (sync must serve it) and `cairn_body` carries PUBLIC's default EXECUTE, so
--       `SELECT cairn_body(signed_bytes) -> 'safety' FROM event_log` returns the
--       uncoarsened rung/class/severity in one statement, to the same role. Demonstrated,
--       not theorised (#424). Withholding a projection while granting its source is not a floor.
--   (b) db/020 grants cairn_node table-level SELECT on event_log and section 8 does not
--       narrow it — and the runtime login role is provisioned as a MEMBER of cairn_node
--       (crates/cairn-node/src/db.rs), not as cairn_agent. So the role the product actually
--       connects as still reads this column raw today (#425).
--
-- Both are tracked; neither is closed here. Read section 8 as raising the cost of the
-- casual read, not as a guarantee — the honest boundary remains emission-time coarsening,
-- exactly as ADR-0063 decision 2 says.
--
-- Note cairn_prospective_sensitivity reads the SENSITIVITY stream and never touches
-- event_log.safety at all; it is grouped here because emission calls it, not because it
-- is part of the safety read model.
REVOKE EXECUTE ON FUNCTION cairn_prospective_sensitivity(uuid, uuid) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_event_safety(uuid) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_patient_safety(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_prospective_sensitivity(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_event_safety(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_patient_safety(uuid) TO cairn_agent;

-- ---------------------------------------------------------------------------
-- 8. The column floor under all of the above (#405 part 1).
--
--    !! A COLUMN-LEVEL REVOKE DOES NOT WORK; THE TABLE GRANT MUST GO FIRST !!
--    `REVOKE SELECT (safety) ON event_log FROM cairn_agent` looks like the fix and is
--    INERT: Postgres tracks table-level and column-level privileges separately, and a
--    column REVOKE removes only a column-level grant. While db/005's table-level
--    `GRANT SELECT ON event_log` stands, it keeps conferring every column, this one
--    included. The only way to withhold one column is to drop to column grants entirely.
--
--    WHY HERE AND NOT IN db/005. The column does not exist yet when db/005 runs (section
--    0 above adds it), and migration replay re-runs every file in order on each connect —
--    so db/005 re-grants the table and this block re-narrows it, every time, idempotently.
--    Keeping the narrowing next to the column's rationale is also the point: a reader who
--    finds the grant must find the reason in the same file.
--
--    THE COST OF THAT PLACEMENT: EACH FILE IS ITS OWN TRANSACTION, so between db/005's
--    COMMIT and this file's, the wide table grant is COMMITTED and visible to every other
--    session. Every schema load therefore reopens the column for the duration of the
--    replay, and a load that aborts anywhere in db/006–db/048 leaves it open indefinitely
--    (the node refuses to serve, but the DATABASE stays fail-OPEN for any other connection).
--    The floor is only as continuous as the replay chain completing (#427).
--
--    IF THIS EVER 42501s A READER YOU BELIEVE IS LEGITIMATE, READ THIS FIRST. The refusal
--    presents as `permission denied for table event_log` — it names neither the column nor
--    the whole-row reference that actually triggered it, because a whole-row `f(el)` needs
--    SELECT on EVERY column. The fix is to add the column to the list below, NEVER to
--    re-issue `GRANT SELECT ON event_log` — that one line silently undoes this whole block.
--    (It would be caught: `safety` flips readable and trips WITHHELD_COLUMNS. Better to
--    know before fighting the guard than after.) db/034's two whole-row readers hit exactly
--    this and became SECURITY DEFINER for it.
--
--    THE LIST IS FAIL-CLOSED BY CONSTRUCTION, AND THAT IS THE DESIGN. A column added to
--    event_log by a FUTURE migration is NOT covered by a column grant, so cairn_agent
--    cannot read it until someone adds it here deliberately. That failure is loud
--    (42501 at the first read) and it forces the disclosure question to be answered at the
--    moment the column is added rather than inherited by default — which is exactly how
--    `safety` became readable in the first place. Generating the list dynamically
--    ("every column except safety") would restore the inheritance this block exists to
--    end. Pinned by safety_read_grants.rs::every_event_log_column_is_a_deliberate_grant_decision,
--    which fails by NAME when event_log gains a column no one has decided about.
--
--    WHAT THIS DOES NOT DO — read this before citing the block as a guarantee. It binds
--    cairn_agent, the role the C1-C5 threat model treats as hostile-capable. Three residuals
--    survive, and only the first is inherent:
--
--      1. An owner/superuser connection reads everything, as it must to run migrations at
--         all. Inherent to "the DB owner can read the DB", not a gap in this floor.
--      2. The VALUE is still reachable by this same role through granted columns:
--         `cairn_body(signed_bytes) -> 'safety'` returns it uncoarsened, because the column
--         is a copy of a clear field of the signed body and `signed_bytes` must stay
--         granted. See the section 7 header for the full statement (#424).
--      3. cairn_node keeps a table-level SELECT on event_log (db/020) that this block does
--         not narrow — and the runtime login role is a MEMBER of cairn_node, so an earlier
--         draft's "the role the runtime connects as" was simply wrong about which role
--         this binds (#425).
--
--    2 (#424) and 3 (#425) are tracked as follow-ups, not closed here. The block is a real narrowing of
--    the casual path; it is not the confidentiality boundary, and ADR-0063 decision 2 —
--    emission-time coarsening — remains the one that binds.
REVOKE SELECT ON event_log FROM cairn_agent;
GRANT SELECT (
    event_id, patient_id, event_type, schema_version,
    hlc_wall, hlc_counter, node_origin, t_effective,
    signed_bytes, content_address, body, contributors,
    signer_key_id, plaintext_twin, sealed, dek_wrapped,
    attachments, recorded_at, attestation, attester_key,
    actor_id, seq, clock_grade
    -- `safety` is deliberately ABSENT. Section 7's functions are the sanctioned read —
    -- and, per that section's header, not the only route to the value.
) ON event_log TO cairn_agent;

COMMIT;
