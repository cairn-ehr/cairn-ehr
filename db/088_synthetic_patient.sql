-- db/088 — a patient is marked synthetic at registration (ADR-0115, #1219).
--
-- WHAT THIS FILE IS. ADR-0115 puts "this patient is invented" in one field of the
-- registration event. db/045 (edited in place for this) checks the field at the door and
-- carries it into `patient_registration.synthetic`. THIS file is the two questions everything
-- else asks, and the two existing reads that must give a different answer because of them:
--
--   1. cairn_chart_synthetic(chart)                 -> 'synthetic' | 'real' | 'unknown'
--   2. cairn_charts_straddle_synthetic_line(a, b)   -> boolean
--   3. cairn_match_veto          (db/016), re-declared with one more row
--   4. cairn_person_charts       (db/054), re-declared so a set never crosses the line
--   5. match_proposal            (db/017), which never holds an open proposal across it
--
-- ONE HOME EACH. No door, view or Rust function reads `patient_registration.synthetic` for
-- itself. Two spellings of "is this chart invented" is the mirror-list defect this repo keeps
-- paying for (#182, #404, #441), here with a wrong-chart hazard as the thing mirrored.
--
-- WHY (3) AND (4) ARE RE-DECLARED HERE AND NOT EDITED WHERE THEY LIVE. Both are
-- `LANGUAGE sql`, and Postgres validates a SQL function's body when it is CREATED. db/016 and
-- db/054 load before this file, so on a fresh database a body there that called a function
-- defined here would fail to create. `CREATE OR REPLACE` here supersedes them on every schema
-- replay, the same arrangement db/033 has over db/031's view, and each original carries a
-- comment pointing here. `synthetic_veto_and_chart_set_have_one_live_definition.rs` holds
-- that the earlier bodies are otherwise unchanged, so the two cannot drift apart unseen.
--
-- REPLAY-SAFE: CREATE OR REPLACE only. The column this reads is added by db/045's own ALTER.
BEGIN;

-- 1. Is this chart of an invented patient?
--
--    THREE answers, never two (principle 4). 'unknown' is a chart this node holds no
--    registration for: a peer's clinical event legitimately arrives before the registration
--    that licenses it (db/005 step 8b binds the strict door only). Answering 'real' for it
--    would be a precise untruth, and it is the unsafe one: an invented chart shown as a
--    person. Every caller must decide what to do with 'unknown'; none may fold it into 'real'.
--
--    Reads `patient_registration_current`: the EARLIEST registration is the chart's birth
--    act for every fact it states (db/045 section 6), this one included.
CREATE OR REPLACE FUNCTION cairn_chart_synthetic(p_patient uuid)
RETURNS text
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT COALESCE(
        (SELECT CASE WHEN r.synthetic THEN 'synthetic' ELSE 'real' END
           FROM patient_registration_current r
          WHERE r.patient_id = p_patient),
        'unknown')
$$;
GRANT EXECUTE ON FUNCTION cairn_chart_synthetic(uuid) TO cairn_agent;

-- Do this chart's registrations DISAGREE about the mark? Only possible when more than one
-- registration exists for a chart, which the strict door prevents and a peer (or a
-- non-conforming client) can still deliver. The earliest decides (above); this lets a
-- surface say that the record contradicts itself instead of showing one side in silence.
CREATE OR REPLACE FUNCTION cairn_chart_synthetic_conflict(p_patient uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT count(DISTINCT r.synthetic) > 1
      FROM patient_registration r
     WHERE r.patient_id = p_patient
$$;
GRANT EXECUTE ON FUNCTION cairn_chart_synthetic_conflict(uuid) TO cairn_agent;

-- 2. Do these two charts stand on opposite sides of the line: one invented, one a person?
--
--    TRUE only when BOTH are known and they differ. 'unknown' on either side is FALSE here,
--    on purpose: a door that refused on what it does not know would refuse every link
--    involving a chart whose registration is still in transit, and offline-first means that
--    is an ordinary state. The case is not dropped, it is caught where it CAN be decided:
--    cairn_person_charts (4) asks this again at read time, by which point the registration
--    may have arrived.
CREATE OR REPLACE FUNCTION cairn_charts_straddle_synthetic_line(p_a uuid, p_b uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT (cairn_chart_synthetic(p_a), cairn_chart_synthetic(p_b))
           IN (('synthetic', 'real'), ('real', 'synthetic'))
$$;
GRANT EXECUTE ON FUNCTION cairn_charts_straddle_synthetic_line(uuid, uuid) TO cairn_agent;

-- 3. The match veto, with one more reason (supersedes db/016's definition on replay).
--
--    A pair across the line is a HARD veto, so every existing reader of cairn_has_hard_veto
--    (the auto-apply seam, the duplicate check, ReportTriage's resolver) refuses it without
--    learning anything new. The first three arms are db/016's, verbatim.
--
--    This is the WEAKER of the two rules about such a pair. A hard veto forces a human
--    decision and a human attestation lifts it; db/018 refuses a link across the line
--    whoever signs. The veto row exists so that nothing PROPOSES one.
CREATE OR REPLACE FUNCTION cairn_match_veto(p_a uuid, p_b uuid)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    SELECT * FROM cairn_identifier_veto(p_a, p_b)
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'dob')
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'sex-at-birth')
    UNION ALL
    SELECT 'synthetic_line'::text, 'hard_veto'::text, 'registration'::text,
           'one chart is of a synthetic patient and the other is not (ADR-0115)'::text
     WHERE cairn_charts_straddle_synthetic_line(p_a, p_b);
$$;
GRANT EXECUTE ON FUNCTION cairn_match_veto(uuid, uuid) TO cairn_agent;

-- 4. The person's chart set, which never crosses the line (supersedes db/054's definition).
--
--    THIS IS THE FLOOR UNDER BOTH DOORS. db/018 refuses a link across the line locally and
--    admits one from a peer; either way a link can also arrive BEFORE the registration that
--    would have shown it crosses (the #220 shape). So the rule that matters is here, at READ
--    time: whatever was admitted, and in whatever order, the set of charts read together as
--    one person never holds an invented chart beside a real one.
--
--    The link component (person_member, db/018) is db/054's, unchanged. The added predicate
--    drops any member on the other side of the line FROM THE CHART BEING READ.
--
--    KNOWN LIMIT: for a chart that is itself 'unknown', nothing is dropped (it straddles
--    nothing), so a component A(synthetic)–B(unknown)–C(real) read FROM B holds all three
--    until B's registration arrives. Read from A or from C it is already cut. B is a chart
--    this node cannot yet say anything about, and a surface showing it must say so
--    (cairn_chart_synthetic = 'unknown').
CREATE OR REPLACE FUNCTION cairn_person_charts(p_patient uuid)
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT m.patient_id
    FROM person_member m
    WHERE m.person_id = (SELECT person_id FROM person_member WHERE patient_id = p_patient)
      AND NOT cairn_charts_straddle_synthetic_line(m.patient_id, p_patient)
    UNION
    SELECT p_patient
    ORDER BY 1
$$;
GRANT EXECUTE ON FUNCTION cairn_person_charts(uuid) TO cairn_agent;

-- 5. The match worklist never holds an OPEN proposal across the line.
--
--    A proposal says "a human should decide whether these two charts are one person". For a
--    pair across the line there is nothing to decide: db/018 refuses the link whoever signs
--    it. So the proposal would be a worklist item that can only ever be dismissed, and one
--    that puts an invented chart beside a real patient's on a clinician's screen.
--
--    The matcher leaves such pairs out for itself (matcher/.../pipeline/synthetic_line.py).
--    This is the floor under it, for every OTHER writer: db/017 is a plain table with a GRANT,
--    not a submit door, so a psql session or a third-party matcher writes it directly
--    (principle 12: the floor holds against a client talking raw SQL).
--
--    A TRIGGER, not a CHECK constraint: the answer depends on another table
--    (patient_registration), and a CHECK that reads another table is not re-evaluated when
--    that table changes, and breaks dump/restore ordering.
--
--    WHAT IT ALLOWS, AND WHY IT IS NOT A FLAT REFUSAL. A proposal can be written while one
--    chart's registration is still in transit (kind 'unknown', so the pair does not straddle)
--    and turn out to cross the line when the registration arrives. That row must be able to
--    LEAVE the worklist, so an UPDATE to one of the three closing statuses is admitted:
--    'retracted' (the matcher's next check withdraws it), 'rejected' (a human dismisses it)
--    and 'review' (auto_apply.rs finds the veto and stops). Every other status moves the pair
--    toward a link, or back onto the worklist, and is refused. An INSERT is refused whatever
--    status it carries: a new row across the line has no history to close.
--
--    `status` is deliberately unconstrained in db/017, so this is an allow-list of the closing
--    statuses and not a deny-list of the advancing ones: a status added later is refused for
--    such a pair until someone decides it is a closing one.
CREATE OR REPLACE FUNCTION cairn_match_proposal_stays_on_one_side()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NOT cairn_charts_straddle_synthetic_line(NEW.patient_low, NEW.patient_high) THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE' AND NEW.status IN ('retracted', 'rejected', 'review') THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION
        'no match proposal between a synthetic patient''s chart and a real patient''s: % and % stand on opposite sides and can never be linked (ADR-0115). A row already here may only be closed: retracted, rejected or review',
        NEW.patient_low, NEW.patient_high;
END;
$$;

-- Not a door: nobody calls a trigger function, the table fires it. Revoked from PUBLIC so it is
-- not on the window's reachable surface (db/window_door_surface.txt), as db/056 does for its
-- own. The trigger still fires for every writer: EXECUTE on a trigger function is checked when
-- the trigger is created, not when it fires.
REVOKE EXECUTE ON FUNCTION cairn_match_proposal_stays_on_one_side() FROM PUBLIC;

DROP TRIGGER IF EXISTS match_proposal_stays_on_one_side ON match_proposal;
CREATE TRIGGER match_proposal_stays_on_one_side
    BEFORE INSERT OR UPDATE ON match_proposal
    FOR EACH ROW EXECUTE FUNCTION cairn_match_proposal_stays_on_one_side();

COMMIT;
