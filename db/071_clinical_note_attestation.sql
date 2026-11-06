-- 071_clinical_note_attestation.sql — the later vouch for one part of a progress note,
-- `clinical.note-attestation.asserted` (sign-in slice 3c-2; ADR-0091).
--
-- WHY A VOUCH IS ITS OWN EVENT. A note part may go into the record unsigned: the quarantine timer
-- committed it (db/067), or its writer was not known. A part is never edited (principle 1), so a
-- signature given LATER is a separate, human-signed event that names the part: either the part's
-- own writer signing it alongside a new addendum (`addendum`), or a clinician signing a finished
-- note off (`sign-off`). The vouch names the part by id AND by content address, so a vouch for one
-- version of a part can never be read as a vouch for another.
--
-- WIRE SHAPE (`clinical.note-attestation/1`, built by cairn_event::note_attestation):
--   payload       {"target": <uuid>, "target_address": <lower hex of the target's
--                  event_log.content_address>, "act": "addendum" | "sign-off"}
--   contributors  [{"actor_id": K, "role": "attested" | "co-signed", "responsibility": {"held_by": K}}]
--   signer_key_id K (the human signs AND attests: db/005's gate demands the token)
--   patient_id    the TARGET's chart; t_effective none.
--
-- THE TWO DOORS. Both run the structural rules on a /1 body. The STRICT door (submit_event) also
-- checks the vouch against the part it names, because only there is it this node's own act: a
-- raw-SQL client must not forge "Dr B co-signed" or "the writer signed it later" for a part that
-- does not support it (Review Focus 3). The REMOTE door checks structure only (ADR-0091 decision
-- 3): set-union sync may deliver a vouch before its note, and where a note lives is node-local
-- knowledge, so judging it there would fork the event set (the db/034 #689 argument). The read
-- (cairn_node::note::read) counts a vouch only when its target_address is the part's own and its
-- token is verified, so a structurally sound forgery admitted remotely is never shown as a
-- signature.
--
-- Replay: the loader re-runs every db/*.sql on each connect, so each statement converges.
BEGIN;

-- 1. Register the type (fail-closed classification registry). Additive; it targets no other
--    author. It stays OFF db/048's `cairn_event_type_has_no_thread` list, as notes do: the
--    conservative sensitivity bound applies (pinned in db/tests/071).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.note-attestation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- "The same person": two signing keys share an actor (ADR-0011: a rotate-key row keeps the actor
-- id, so a re-keyed member is still the same person). Which actor a key names cannot be
-- ambiguous: db/004's cairn_key_actor_id_conflict (#166) refuses an enroll or rotate-key that
-- would bind a key already bound to a different actor, so each key has at most one. (Were it
-- otherwise, "ANY shared actor" would fail closed only for the co-sign use below, where a match
-- refuses; for the addendum use, where a match admits, it would fail open.) Owner-only: the floor
-- calls it from inside submit_event (SECURITY DEFINER).
CREATE OR REPLACE FUNCTION cairn_note_keys_share_actor(p_a text, p_b text) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM actor_event a JOIN actor_event b ON b.actor_id = a.actor_id
                    WHERE a.signing_key_id = p_a AND a.op IN ('enroll', 'rotate-key')
                      AND b.signing_key_id = p_b AND b.op IN ('enroll', 'rotate-key'));
$$;
REVOKE EXECUTE ON FUNCTION cairn_note_keys_share_actor(text, text) FROM PUBLIC;

-- 2. The floor. Each rule raises its own message so the mirror can tell which one fired.
CREATE OR REPLACE FUNCTION cairn_check_note_attestation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p           jsonb   := b -> 'payload';
    v_authoring boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_signer    text    := b ->> 'signer_key_id';
    v_bearing   jsonb;
    v_role      text;
    v_target    event_log%ROWTYPE;
BEGIN
    -- (1) Ruling R8, as db/064/db/068: at the REMOTE door a newer profile is carried, not judged
    --     (refusing it would wedge sync on every node older than its author); the applier below
    --     projects /1 alone. The AUTHORING door writes only what this node knows.
    IF NOT v_authoring AND b ->> 'schema_version' IS DISTINCT FROM 'clinical.note-attestation/1' THEN
        RETURN;
    END IF;
    IF p IS NULL THEN
        RAISE EXCEPTION 'note attestation: missing payload';
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.note-attestation/1' THEN
        RAISE EXCEPTION 'note attestation: schema_version must be clinical.note-attestation/1, got %', b ->> 'schema_version';
    END IF;

    -- (2) Exactly one contributor carries responsibility (the predicate is db/005's gate's own,
    --     `? 'responsibility'`), it vouches as attested or co-signed, and it is the signer. With
    --     db/005's #195 binding (the bearing contributor is the verified attester) this makes the
    --     signer, the attester and the vouching human one key. The array guard keeps a DIRECT
    --     caller's non-array off jsonb_array_elements (db/034 M1).
    IF jsonb_typeof(b -> 'contributors') IS DISTINCT FROM 'array'
       OR (SELECT count(*) FROM jsonb_array_elements(b -> 'contributors') e
            WHERE e ? 'responsibility') <> 1 THEN
        RAISE EXCEPTION 'note attestation: exactly one contributor must carry responsibility (the vouching human)';
    END IF;
    SELECT e INTO v_bearing FROM jsonb_array_elements(b -> 'contributors') e WHERE e ? 'responsibility';
    v_role := v_bearing ->> 'role';
    IF v_role IS NULL OR v_role NOT IN ('attested', 'co-signed') THEN
        RAISE EXCEPTION 'note attestation: the vouching contributor must be attested or co-signed, got %', v_role;
    END IF;
    IF v_bearing ->> 'actor_id' IS DISTINCT FROM v_signer THEN
        RAISE EXCEPTION 'note attestation: the vouching contributor must be the event''s signer';
    END IF;
    -- At the STRICT door the contributors are exactly that one entry: the node builds nothing
    -- else. The REMOTE door tolerates extra contributory entries, because refusing a verifiable
    -- peer event for an addition this node does not need would break set-union losslessness
    -- (ADR-0051); the read only ever looks at the bearing one.
    IF v_authoring AND jsonb_array_length(b -> 'contributors') <> 1 THEN
        RAISE EXCEPTION 'note attestation: contributors must be exactly the one vouching human';
    END IF;

    -- (3) The payload's three keys.
    IF jsonb_typeof(p -> 'target') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'note attestation: target must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'target')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'note attestation: target must be a valid uuid';
    END;
    -- Even length because the strict arm and the applier decode() it as bytea (db/034's rule).
    IF jsonb_typeof(p -> 'target_address') IS DISTINCT FROM 'string'
       OR (p ->> 'target_address') !~ '^[0-9a-fA-F]+$'
       OR length(p ->> 'target_address') % 2 <> 0 THEN
        RAISE EXCEPTION 'note attestation: target_address must be a non-empty even-length hex string';
    END IF;
    IF p ->> 'act' IS NULL OR p ->> 'act' NOT IN ('addendum', 'sign-off') THEN
        RAISE EXCEPTION 'note attestation: act must be addendum or sign-off';
    END IF;
    -- An addendum is the writer's own FIRST signature of a part, so it is never a second one.
    IF p ->> 'act' = 'addendum' AND v_role <> 'attested' THEN
        RAISE EXCEPTION 'note attestation: an addendum act carries attested, never co-signed';
    END IF;

    -- (4) STRICT DOOR ONLY: the vouch against the part it names. db/020 raises cairn.remote_apply
    --     before it runs this floor, and submit_event pins it off (#602), so a client cannot
    --     switch these refusals off. A plain RAISE is P0001, the "fix the request" class.
    IF NOT v_authoring THEN
        RETURN;
    END IF;
    SELECT * INTO v_target FROM event_log
     WHERE event_id = (p ->> 'target')::uuid AND event_type = 'clinical.note.asserted';
    IF NOT FOUND THEN
        RAISE EXCEPTION 'note attestation: target % is not a note held on this node', p ->> 'target';
    END IF;
    -- Filed under the part's own chart: a signature recorded against the wrong patient is the
    -- #689 failure, and a linked set puts another chart's notes on screen.
    IF v_target.patient_id IS DISTINCT FROM (b ->> 'patient_id')::uuid THEN
        RAISE EXCEPTION 'note attestation: target % lives on chart %, but this vouch is filed under chart %',
            p ->> 'target', v_target.patient_id, b ->> 'patient_id';
    END IF;
    IF v_target.content_address IS DISTINCT FROM decode(p ->> 'target_address', 'hex') THEN
        RAISE EXCEPTION 'note attestation: target_address is not the content address of note % (a vouch names the exact version it vouches for)',
            p ->> 'target';
    END IF;
    -- An addendum vouch is the part's own writer signing it later: the part names a `drafted`
    -- contributor who is the same person as the signer (compared by actor, so a re-keyed writer
    -- still matches).
    IF p ->> 'act' = 'addendum' AND NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(v_target.contributors) d
             WHERE d ->> 'role' = 'drafted'
               AND cairn_note_keys_share_actor(d ->> 'actor_id', v_signer)) THEN
        RAISE EXCEPTION 'note attestation: an addendum vouch is for a part its signer drafted; % was drafted by someone else',
            p ->> 'target';
    END IF;
    -- A co-signature is a SECOND signature: only a part a human signed, and someone else. An
    -- unsigned part is attested, never co-signed.
    IF v_role = 'co-signed' THEN
        IF NOT cairn_note_key_is_human(v_target.signer_key_id) THEN
            RAISE EXCEPTION 'note attestation: a co-signature is for a part a human signed; % was not signed by a human',
                p ->> 'target';
        END IF;
        IF cairn_note_keys_share_actor(v_target.signer_key_id, v_signer) THEN
            RAISE EXCEPTION 'note attestation: a co-signature is for a part someone else signed; % carries the signer''s own signature',
                p ->> 'target';
        END IF;
    END IF;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382).
REVOKE EXECUTE ON FUNCTION cairn_check_note_attestation(text, jsonb) FROM PUBLIC;

-- 3. Register the floor + hard twin requirement (#173). DO UPDATE with the IS DISTINCT FROM
--    guard (#214): the row converges to this text yet a steady-state replay writes nothing.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.note-attestation.asserted', 'cairn_check_note_attestation', 'note attestation requires a non-empty authored twin (§3.13/§3.19)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 4. The projection: one row per vouch. The read joins it to the part by `target` and counts it
--    only when `target_address` is the part's own content address. content_address is what a
--    shred scrubs by (db/037), as for every clinical projection.
CREATE TABLE IF NOT EXISTS clinical_note_attestation (
    event_id        UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    target          UUID NOT NULL,
    target_address  BYTEA NOT NULL,
    act             TEXT NOT NULL,
    role            TEXT NOT NULL,
    signer_key_id   TEXT NOT NULL,
    t_effective     TIMESTAMPTZ,
    hlc_wall        BIGINT,
    hlc_counter     INTEGER,
    origin          TEXT,
    content_address BYTEA NOT NULL
);
GRANT SELECT ON clinical_note_attestation TO cairn_agent;
CREATE INDEX IF NOT EXISTS clinical_note_attestation_target_idx ON clinical_note_attestation (target);
CREATE INDEX IF NOT EXISTS clinical_note_attestation_content_address_idx ON clinical_note_attestation (content_address);

-- 5. Fold a vouch into the table. Reads the CLEAR payload (ADR-0052): NULL means this node holds
--    no custody for the sealed body, so there is nothing to project (the late-custody path re-runs
--    this when a key lands — heal_safe below). That heal can reach a body the floor never saw (a
--    sealed vouch admitted remotely without its DEK skips the floor, db/020 step 7), so the
--    projection re-imposes the floor's closed vocabularies itself. It SKIPS (never raises, never
--    a table CHECK, either of which would fail the heal) a body whose responsibility-bearing role
--    is missing or is not attested/co-signed, whose act is not addendum/sign-off, whose addendum
--    carries co-signed, whose target is not a uuid, or whose target_address is not even-length
--    hex. The table therefore holds only what the read's VouchRole/VouchAct can name, and an
--    unprojected vouch is never read as a signature.
CREATE OR REPLACE FUNCTION clinical_note_attestation_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := cairn_clear_payload(e);
    v_role text;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- Ruling R8: project ONLY the profile the floor checked (see the floor's rule 1).
    IF e.schema_version IS DISTINCT FROM 'clinical.note-attestation/1' THEN RETURN; END IF;
    SELECT c ->> 'role' INTO v_role FROM jsonb_array_elements(
               CASE WHEN jsonb_typeof(e.contributors) = 'array' THEN e.contributors ELSE '[]' END) c
     WHERE c ? 'responsibility' LIMIT 1;
    IF v_role IS NULL
       OR v_role NOT IN ('attested', 'co-signed')
       OR p ->> 'act' IS NULL
       OR p ->> 'act' NOT IN ('addendum', 'sign-off')
       OR (p ->> 'act' = 'addendum' AND v_role <> 'attested')
       OR NOT pg_input_is_valid(p ->> 'target', 'uuid')
       OR COALESCE(p ->> 'target_address', '') !~ '^([0-9a-fA-F]{2})+$' THEN
        RETURN;
    END IF;
    INSERT INTO clinical_note_attestation
        (event_id, patient_id, target, target_address, act, role, signer_key_id, t_effective,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (e.event_id, e.patient_id, (p ->> 'target')::uuid, decode(p ->> 'target_address', 'hex'),
            p ->> 'act', v_role, e.signer_key_id, e.t_effective,
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge vouch rows.
REVOKE EXECUTE ON FUNCTION clinical_note_attestation_apply(event_log) FROM PUBLIC;

-- 6. Register the applier (db/005, ADR-0057). heal_safe: insert-or-nothing per event.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.note-attestation.asserted', 'clinical_note_attestation_apply', ARRAY['clinical_note_attestation'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- 7. The reader doors the note read and the window need (ADR-0091). Definer wrappers because the
--    window role reads neither actor_event's actor/pinned columns nor vault_member (db/062); each
--    answers NULL for what it does not know (principle 4: never a guess).
--
-- A key's actor, as lower hex: the actor whose enroll or rotate-key row carries the key, so every
-- key a member ever held answers the same person. NULL for an unknown key, and for a key
-- registered to several actors (attribution honestly unknown, as db/005 stamps it).
CREATE OR REPLACE FUNCTION cairn_note_key_actor(p_key text) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT CASE WHEN count(DISTINCT ae.actor_id) = 1 THEN encode(min(ae.actor_id), 'hex') END
      FROM actor_event ae
     WHERE ae.signing_key_id = p_key AND ae.op IN ('enroll', 'rotate-key');
$$;

-- That actor's pinned role (its enroll row's `pinned ->> 'role'`). Pinned determinants are
-- immutable per actor, so a student who re-enrols as a doctor is a NEW actor and the parts they
-- signed as a student keep reading "(student)".
CREATE OR REPLACE FUNCTION cairn_note_key_role(p_key text) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT ae.pinned ->> 'role' FROM actor_event ae
     WHERE ae.actor_id = decode(cairn_note_key_actor(p_key), 'hex') AND ae.op = 'enroll'
     ORDER BY ae.seq LIMIT 1;
$$;

-- The actor of the vault member a who's-here claim names, as lower hex; NULL if no such member.
-- The staff id is normalised by the vault's own one definition (db/061), never a copy of it.
CREATE OR REPLACE FUNCTION cairn_note_staff_actor(p_staff_id text) RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT encode(m.actor_id, 'hex') FROM vault_member m
     WHERE m.staff_id = cairn_vault_staff_id(p_staff_id);
$$;

REVOKE EXECUTE ON FUNCTION cairn_note_key_actor(text), cairn_note_key_role(text),
    cairn_note_staff_actor(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_note_key_actor(text), cairn_note_key_role(text),
    cairn_note_staff_actor(text) TO cairn_window;

COMMIT;
