-- Cairn — staff supervision: who supervises whom, who must have every signed note co-signed, and
-- who may assign supervision (sign-in slice 3c-3a; design:
-- docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md
-- §3.1, and the slice plan's Global Constraints, whose matrix this file enforces).
--
-- WHY THIS FILE EXISTS. Until now only a STUDENT's note waited for a co-signature, because the
-- student role is pinned on the actor (ADR-0092 decision 10). A probationary doctor needs the same
-- paper countersignature, and only their supervisor's should count. So the node now keeps a
-- RECORD: "B supervises C", "C needs co-signing", "D may assign supervision". Later tasks read it
-- as of a part's signing time to decide whether "— not co-signed" still shows.
--
-- THE RECORD IS THE ACTOR REGISTRY. Each change is one append-only actor_event row (db/004), never
-- an edit, so it is signed, auditable, carried by backup/restore (db/052) and readable "as of t":
--   * link-supervisor / unlink-supervisor sit on the SUPERVISEE's actor_id and name the supervisor
--     in related_actor. On the supervisee, because every question later asked is about them ("does
--     THIS writer's note need a co-sign, and whose co-sign clears it?"): one actor's rows answer it.
--   * require-cosign / lift-cosign sit on the person; grant-assigner / withdraw-assigner on the
--     holder of the "assigns supervision" right. Neither names a second actor.
-- The state of each family is its LATEST row (by recorded_at, seq, as actor_current orders): a
-- link stands while its latest link/unlink row is a link, and so on. A student needs co-signing by
-- role alone, so nothing is stored for one and require/lift on a student is refused.
--
-- THE TRUST BOUNDARY. cairn_supervision_act is reachable by cairn_agent (which the window's role
-- inherits), so by any raw-SQL client holding it. It never acts without a SIGNED approval checked
-- HERE, as db/063's vault doors do:
--   * the approver rule lives in ONE function, cairn_supervision_approver: a member whose key is
--     current, confirmed unless a superuser, whose token verifies over this act's ADDRESS;
--   * the address binds the op, both staff IDs and the family's PRIOR row (its seq), so a token is
--     good for one change of one state: replayed after the state moved, it verifies against nothing;
--   * who may do what (the matrix) lives in ONE function, cairn_supervision_apply, which the window
--     door and the console door both run, so their refusals cannot drift apart.
-- Nobody acts on their own supervision as the supervisee — not even holding the assigner right:
-- otherwise they could unlink their supervisors and fall into the no-supervisor fallback, where any
-- clinician's co-sign clears theirs. Without the assigner right, stepping away is the one act a
-- link's own party may make; an assigner may also link themselves as someone ELSE's supervisor.
-- Every refusal names its check and never echoes a token.

BEGIN;

-- The address one supervision act is signed over, as db/063's cairn_vault_grant_address: '\x1220'
-- || sha256 of newline-joined fields, domain tag first, staff IDs normalised, '' for an op with no
-- supervisor, the family's prior seq last (cairn_supervision_prior). A NULL op or subject makes the
-- address NULL, which no token verifies against. Pure; PUBLIC keeps it (it reads nothing).
CREATE OR REPLACE FUNCTION cairn_supervision_address(p_op text, p_subject_staff text,
                                                     p_supervisor_staff text, p_prior bigint)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-supervision' || E'\n' || p_op || E'\n' || cairn_vault_staff_id(p_subject_staff)
        || E'\n' || COALESCE(cairn_vault_staff_id(p_supervisor_staff), '') || E'\n' || p_prior::text,
        'UTF8'));
$$;

-- The latest row of p_op's FAMILY (link/unlink, require/lift, grant/withdraw) on this actor — for
-- a link, with THAT supervisor (p_related; NULL for the other families, whose rows carry NULL).
-- No row = never acted on. Invoker, owner only: the doors below read through it.
CREATE OR REPLACE FUNCTION cairn_supervision_latest(p_actor bytea, p_op text, p_related bytea)
RETURNS TABLE(seq bigint, op text) LANGUAGE sql STABLE AS $$
    SELECT e.seq, e.op FROM actor_event e
     WHERE e.actor_id = p_actor AND e.related_actor IS NOT DISTINCT FROM p_related
       AND e.op = ANY (CASE
           WHEN p_op IN ('link-supervisor', 'unlink-supervisor')
               THEN ARRAY['link-supervisor', 'unlink-supervisor']
           WHEN p_op IN ('require-cosign', 'lift-cosign') THEN ARRAY['require-cosign', 'lift-cosign']
           WHEN p_op IN ('grant-assigner', 'withdraw-assigner')
               THEN ARRAY['grant-assigner', 'withdraw-assigner']
       END)
     ORDER BY e.recorded_at DESC, e.seq DESC LIMIT 1;
$$;

-- The supervisors whose link to this supervisee stands NOW (latest link/unlink row per supervisor
-- is a link). Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_supervision_supervisors(p_actor bytea)
RETURNS SETOF bytea LANGUAGE sql STABLE AS $$
    SELECT l.related_actor FROM (
        SELECT DISTINCT ON (e.related_actor) e.related_actor, e.op FROM actor_event e
         WHERE e.actor_id = p_actor AND e.op IN ('link-supervisor', 'unlink-supervisor')
         ORDER BY e.related_actor, e.recorded_at DESC, e.seq DESC) l
     WHERE l.op = 'link-supervisor';
$$;

-- The staff ID an actor goes by: its member row's, else (no member row, as after a restore) the
-- handle pinned on its enroll row. Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_supervision_staff_of(p_actor bytea)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        (SELECT m.staff_id FROM vault_member m WHERE m.actor_id = p_actor ORDER BY 1 LIMIT 1),
        (SELECT e.pinned ->> 'handle' FROM actor_event e
          WHERE e.actor_id = p_actor AND e.op = 'enroll' LIMIT 1));
$$;

-- Which actor a supervisor's staff ID names, for this supervisee: a supervisor whose link STANDS
-- and who goes by that staff ID first (so a link to an actor with no member row any more can still
-- be ended), else the member row's actor. NULL when neither exists. Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_supervision_related(p_subject bytea, p_supervisor_staff text)
RETURNS bytea LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        (SELECT s FROM cairn_supervision_supervisors(p_subject) s
          WHERE cairn_supervision_staff_of(s) = cairn_vault_staff_id(p_supervisor_staff) LIMIT 1),
        (SELECT m.actor_id FROM vault_member m
          WHERE m.staff_id = cairn_vault_staff_id(p_supervisor_staff)));
$$;

-- THE APPROVER RULE for a supervision act — its one home. Returns 'superuser', 'assigner' (a
-- member whose latest assigner row is a grant) or 'member', or RAISES naming the failed check:
--   * the approver must be a vault member, and their key the actor's current key;
--   * anyone but a superuser also needs that key 'confirmed' (or 'outside', predating the
--     mechanism), as an enroller does (db/061's cairn_vault_enroller_problem);
--   * p_token must be the approver's own attestation over p_address — this act, this state.
-- WHAT the kind may do is the matrix's business (cairn_supervision_apply). Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_supervision_approver(p_approver_staff text, p_address bytea,
                                                      p_token bytea)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_staff  text := cairn_vault_staff_id(p_approver_staff);
    v_role   text;
    v_actor  bytea;
    v_key    text;
    v_status text;
BEGIN
    SELECT m.role, m.actor_id, m.signing_key_id INTO v_role, v_actor, v_key
      FROM vault_member m WHERE m.staff_id = v_staff;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'cairn_supervision_act: % is not an enrolled member, so cannot approve',
            COALESCE(v_staff, 'NULL');
    END IF;
    IF NOT EXISTS (SELECT 1 FROM actor_current a
                    WHERE a.actor_id = v_actor AND a.signing_key_id = v_key) THEN
        RAISE EXCEPTION 'cairn_supervision_act: %''s key is not current (revoked, superseded, rotated away or repudiated)', v_staff;
    END IF;
    IF v_role <> 'superuser' THEN
        v_status := cairn_key_confirmation(v_key);
        IF v_status NOT IN ('confirmed', 'outside') THEN
            RAISE EXCEPTION 'cairn_supervision_act: %''s key is % — a supervision act needs a confirmed key, so a superuser must confirm it first',
                v_staff, v_status;
        END IF;
    END IF;
    -- IS NOT TRUE: a NULL token or address yields NULL, which must refuse.
    IF cairn_attestation_ok(p_token, p_address, decode(v_key, 'hex')) IS NOT TRUE THEN
        RAISE EXCEPTION 'cairn_supervision_act: the approver''s attestation is invalid or does not cover this act';
    END IF;
    RETURN CASE
        WHEN v_role = 'superuser' THEN 'superuser'
        WHEN (SELECT l.op FROM cairn_supervision_latest(v_actor, 'grant-assigner', NULL) l)
             = 'grant-assigner' THEN 'assigner'
        ELSE 'member'
    END;
END;
$$;

-- THE MATRIX — the shared body of the window's door (p_console false) and the console's (true).
-- In order: lock the SUBJECT's member row (so two acts on one person cannot both read the same
-- prior), find the supervisor's actor (a link op), read the family's prior row, check the approver
-- over the address (the console approves itself: 'console', no token), then the matrix, then append.
-- Matrix (plan Global Constraints): link/require by a superuser or assigner; unlink also by the
-- supervisor themself; lift also by a current supervisor; grant/withdraw-assigner by a superuser
-- only; and the party/state refusals below. Invoker, owner only — p_console is never reachable
-- from the window, whose door passes false.
CREATE OR REPLACE FUNCTION cairn_supervision_apply(p_op text, p_subject_staff text,
                                                   p_supervisor_staff text, p_approver_staff text,
                                                   p_token bytea, p_console boolean)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_door     text := CASE WHEN p_console THEN 'cairn_supervision_console_act'
                            ELSE 'cairn_supervision_act' END;
    v_link     boolean := p_op IN ('link-supervisor', 'unlink-supervisor');
    v_subject  text := cairn_vault_staff_id(p_subject_staff);
    v_sup_id   text := cairn_vault_staff_id(p_supervisor_staff);
    v_role     text;   -- the subject's role and actor
    v_actor    bytea;
    v_sup      bytea;  -- the supervisor's actor and role (link ops only)
    v_sup_role text;
    v_prior    bigint;
    v_latest   text;   -- the op of the family's latest row: the current state
    v_kind     text;   -- 'console' | 'superuser' | 'assigner' | 'member'
    v_by       bytea;  -- the approver's actor and current key (NULL / 'console' for the console)
    v_by_key   text := 'console';
BEGIN
    IF p_op IS NULL OR p_op NOT IN ('link-supervisor', 'unlink-supervisor', 'require-cosign',
                                    'lift-cosign', 'grant-assigner', 'withdraw-assigner') THEN
        RAISE EXCEPTION '%: % is not a supervision act', v_door, COALESCE(p_op, 'NULL');
    END IF;
    IF v_link AND v_sup_id IS NULL THEN
        RAISE EXCEPTION '%: % needs the supervisor''s staff ID', v_door, p_op;
    ELSIF NOT v_link AND p_supervisor_staff IS NOT NULL THEN
        RAISE EXCEPTION '%: % takes no supervisor — only a link names one', v_door, p_op;
    END IF;

    SELECT m.role, m.actor_id INTO v_role, v_actor
      FROM vault_member m WHERE m.staff_id = v_subject FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION '%: % is not an enrolled member', v_door, COALESCE(v_subject, 'NULL');
    END IF;
    IF v_link THEN
        v_sup := cairn_supervision_related(v_actor, v_sup_id);
        IF v_sup IS NULL THEN
            RAISE EXCEPTION '%: the supervisor % is not an enrolled member', v_door, v_sup_id;
        END IF;
        SELECT m.role INTO v_sup_role FROM vault_member m WHERE m.actor_id = v_sup LIMIT 1;
    END IF;
    SELECT l.seq, l.op INTO v_prior, v_latest FROM cairn_supervision_latest(v_actor, p_op, v_sup) l;

    IF p_console THEN
        v_kind := 'console';
    ELSE
        v_kind := cairn_supervision_approver(
            p_approver_staff,
            cairn_supervision_address(p_op, v_subject, v_sup_id, COALESCE(v_prior, 0)), p_token);
        SELECT m.actor_id, m.signing_key_id INTO v_by, v_by_key
          FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_approver_staff);
    END IF;

    -- Who: nobody acts on their own supervision as the supervisee (header), then the kind's limits.
    IF p_op NOT IN ('grant-assigner', 'withdraw-assigner') AND v_by = v_actor THEN
        RAISE EXCEPTION '%: nobody acts on their own supervision as the supervisee — ask a superuser or another assigner', v_door;
    END IF;
    IF v_kind NOT IN ('console', 'superuser') THEN
        IF p_op IN ('grant-assigner', 'withdraw-assigner') THEN
            RAISE EXCEPTION '%: only a superuser may grant or withdraw the assigns-supervision right', v_door;
        ELSIF v_kind = 'member' AND p_op IN ('link-supervisor', 'require-cosign') THEN
            RAISE EXCEPTION '%: only a superuser or a holder of the assigns-supervision right may %', v_door, p_op;
        ELSIF v_kind = 'member' AND p_op = 'unlink-supervisor' AND v_by IS DISTINCT FROM v_sup THEN
            RAISE EXCEPTION '%: only a superuser, an assigner or the supervisor themself may end this link', v_door;
        ELSIF v_kind = 'member' AND p_op = 'lift-cosign'
              AND NOT EXISTS (SELECT 1 FROM cairn_supervision_supervisors(v_actor) s WHERE s = v_by) THEN
            RAISE EXCEPTION '%: only a superuser, an assigner or a current supervisor of % may lift their co-signing', v_door, v_subject;
        END IF;
    END IF;

    -- What: the parties and the state the act changes.
    IF p_op = 'link-supervisor' AND v_sup = v_actor THEN
        RAISE EXCEPTION '%: nobody is their own supervisor', v_door;
    END IF;
    IF (p_op = 'link-supervisor' AND (v_role IN ('superuser', 'admin') OR v_sup_role IN ('superuser', 'admin')))
       OR (p_op = 'require-cosign' AND v_role IN ('superuser', 'admin')) THEN
        RAISE EXCEPTION '%: a superuser or an admin is never a party to supervision', v_door;
    END IF;
    IF p_op = 'link-supervisor' AND v_sup_role = 'student' THEN
        RAISE EXCEPTION '%: a student cannot supervise', v_door;
    END IF;
    IF p_op IN ('require-cosign', 'lift-cosign') AND v_role = 'student' THEN
        RAISE EXCEPTION '%: a student needs co-signing for as long as they are a student — nothing is required or lifted for one', v_door;
    END IF;
    IF p_op IN ('grant-assigner', 'withdraw-assigner') AND v_role = 'superuser' THEN
        RAISE EXCEPTION '%: % is a superuser, who already holds every assigner power', v_door, v_subject;
    END IF;
    CASE p_op
        WHEN 'link-supervisor' THEN IF v_latest = 'link-supervisor' THEN
            RAISE EXCEPTION '%: % already supervises %', v_door, v_sup_id, v_subject; END IF;
        WHEN 'unlink-supervisor' THEN IF v_latest IS DISTINCT FROM 'link-supervisor' THEN
            RAISE EXCEPTION '%: no link stands between % and supervisor %', v_door, v_subject, v_sup_id; END IF;
        WHEN 'require-cosign' THEN IF v_latest = 'require-cosign' THEN
            RAISE EXCEPTION '%: % already needs co-signing', v_door, v_subject; END IF;
        WHEN 'lift-cosign' THEN IF v_latest IS DISTINCT FROM 'require-cosign' THEN
            RAISE EXCEPTION '%: % does not need co-signing', v_door, v_subject; END IF;
        WHEN 'grant-assigner' THEN IF v_latest = 'grant-assigner' THEN
            RAISE EXCEPTION '%: % already holds the assigns-supervision right', v_door, v_subject; END IF;
        ELSE IF v_latest IS DISTINCT FROM 'grant-assigner' THEN
            RAISE EXCEPTION '%: % does not hold the assigns-supervision right', v_door, v_subject; END IF;
    END CASE;

    PERFORM cairn_actor_append(v_actor, p_op, NULL, v_by_key, CASE WHEN p_console THEN NULL ELSE p_token END, v_sup);
END;
$$;

-- The p_prior the window signs over (cairn_supervision_address): the seq of the family's latest
-- row for this subject (and supervisor), 0 if none or no such member. The window cannot read
-- actor_event, so it asks this; the act door computes the same under the subject's row lock.
CREATE OR REPLACE FUNCTION cairn_supervision_prior(p_op text, p_subject_staff text,
                                                   p_supervisor_staff text)
RETURNS bigint LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE((
        SELECT l.seq FROM vault_member m,
               cairn_supervision_latest(m.actor_id, p_op,
                   CASE WHEN p_op IN ('link-supervisor', 'unlink-supervisor')
                        THEN cairn_supervision_related(m.actor_id, p_supervisor_staff) END) l
         WHERE m.staff_id = cairn_vault_staff_id(p_subject_staff)), 0);
$$;

-- The window's door: one supervision act, approved by p_approver_staff's signed token.
CREATE OR REPLACE FUNCTION cairn_supervision_act(p_op text, p_subject_staff text,
                                                 p_supervisor_staff text, p_approver_staff text,
                                                 p_token bytea)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT cairn_supervision_apply(p_op, p_subject_staff, p_supervisor_staff, p_approver_staff,
                                   p_token, false);
$$;

-- Owner only (the node's console): the same act approved by 'console', no token — the same
-- matrix's party and state refusals, no approver checks. No grant: running it IS the authority,
-- as for db/063's cairn_vault_bootstrap_rekey.
CREATE OR REPLACE FUNCTION cairn_supervision_console_act(p_op text, p_subject_staff text,
                                                         p_supervisor_staff text)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT cairn_supervision_apply(p_op, p_subject_staff, p_supervisor_staff, NULL, NULL, true);
$$;

-- The whole supervision record, oldest first: every one of the six ops, actors as hex. The
-- window's audit view (who changed what, when); granted to cairn_window, as db/071's readers are.
CREATE OR REPLACE FUNCTION cairn_supervision_record()
RETURNS TABLE(op text, actor_id text, related_actor text, recorded_at timestamptz)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT e.op, encode(e.actor_id, 'hex'), encode(e.related_actor, 'hex'), e.recorded_at
      FROM actor_event e
     WHERE e.op IN ('link-supervisor', 'unlink-supervisor', 'require-cosign', 'lift-cosign',
                    'grant-assigner', 'withdraw-assigner')
     ORDER BY e.recorded_at, e.seq;
$$;

-- The assigner's and supervisor's panels: every non-superuser member as of NOW — whether they need
-- co-signing (a student always does, by role), whether they hold the assigner right, and the staff
-- IDs of the supervisors whose link stands (the pinned handle when a member row is gone), sorted.
CREATE OR REPLACE FUNCTION cairn_supervision_staff()
RETURNS TABLE(staff_id text, display_name text, role text, needs_cosigning boolean,
              assigner boolean, supervisors text[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT m.staff_id, m.display_name, m.role,
           m.role = 'student' OR COALESCE((SELECT l.op FROM cairn_supervision_latest(
               m.actor_id, 'require-cosign', NULL) l) = 'require-cosign', false),
           COALESCE((SELECT l.op FROM cairn_supervision_latest(
               m.actor_id, 'grant-assigner', NULL) l) = 'grant-assigner', false),
           ARRAY(SELECT cairn_supervision_staff_of(s) FROM cairn_supervision_supervisors(m.actor_id) s
                  ORDER BY 1)
      FROM vault_member m WHERE m.role <> 'superuser' ORDER BY m.staff_id;
$$;

-- Grants. Every definer and helper is first revoked from PUBLIC; cairn_agent (which cairn_window
-- inherits, db/062) gets the act door, the prior and the staff list; cairn_window the record. The
-- console door and every helper have NO grant: the owner's alone. Only the pure address keeps
-- PUBLIC EXECUTE.
REVOKE EXECUTE ON FUNCTION cairn_supervision_latest(bytea, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_supervisors(bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_staff_of(bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_related(bytea, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_approver(text, bytea, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_apply(text, text, text, text, bytea, boolean) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_prior(text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_act(text, text, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_console_act(text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_record() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_supervision_staff() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION cairn_supervision_prior(text, text, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_supervision_act(text, text, text, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_supervision_staff() TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_supervision_record() TO cairn_window;

COMMIT;
