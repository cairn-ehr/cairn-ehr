-- 068_clinical_note_discard.sql — the audited discard of a draft progress note,
-- `clinical.note-discard.asserted` (ADR-0089 decision 15; sign-in slice 3b-2b).
--
-- WHY A DISCARD IS AN EVENT AT ALL. A draft is autosaved and can be taken over by a colleague,
-- who may have read it and acted on it. If it is then thrown away, "it never existed" would be a
-- quiet falsehood (principle 4). So the discarder signs a record that the draft existed, what it
-- said, and who held it. It is NOT a note: it never enters `clinical_note` (ADR-0087 R8: a
-- chart's notes are read only from there), and its twin says so in its first line.
--
-- WIRE SHAPE (`clinical.note-discard/1`, built by cairn_event::note_discard):
--   parts   a non-empty array, in writing order, of objects:
--             writer             'discarder' | 'not-known'
--             narrative          the part's text, ESCAPED once by the window (as a note's is)
--             stated_time        RFC 3339 instant, or null (the writer named no time)
--             recording_started  RFC 3339 instant, the database clock at the part's first save
--             recording_ended    RFC 3339 instant, the part's last keystroke
-- The envelope's t_effective is the DISCARD INSTANT, set by the door, never typed in a window.
--
-- THE FLOOR (below) follows db/064 rule for rule, and reuses its two narrative checks rather than
-- copying their regexes: a discard's narrative is rendered back to a clinician later, so it must
-- be as inert as a note's. Both doors run every rule for a /1 body; a NEWER profile arriving at
-- the remote door is carried unchecked and never projected (ruling R8, exactly as db/064).
--
-- Replay: the loader re-runs every db/*.sql on each connect, so each statement converges.
BEGIN;

-- 1. Register the type (fail-closed classification registry). Additive; it targets no other
--    author. It stays OFF db/048's `cairn_event_type_has_no_thread` list, as notes do: the
--    conservative sensitivity bound applies (pinned in db/tests/068).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.note-discard.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The floor. Each rule raises its own message so the mirror can tell which one fired.
CREATE OR REPLACE FUNCTION cairn_check_note_discard(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p           jsonb   := b -> 'payload';
    v_authoring boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_part      jsonb;
    v_text      text;
    v_started   timestamptz;
    v_ended     timestamptz;
BEGIN
    -- (1) Ruling R8, as db/064: at the REMOTE door a newer profile is carried, not judged
    --     (refusing it would wedge sync on every node older than its author); the applier below
    --     projects /1 alone, so a relabelled hostile body is never on screen. The AUTHORING door
    --     writes only what this node knows.
    IF NOT v_authoring AND b ->> 'schema_version' IS DISTINCT FROM 'clinical.note-discard/1' THEN
        RETURN;
    END IF;
    IF p IS NULL THEN
        RAISE EXCEPTION 'note discard: missing payload';
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.note-discard/1' THEN
        RAISE EXCEPTION 'note discard: schema_version must be clinical.note-discard/1, got %', b ->> 'schema_version';
    END IF;

    -- (2) At least one part: a discard of nothing is no audit record (an all-blank draft is
    --     deleted with no event at all — the node never builds an empty one).
    IF jsonb_typeof(p -> 'parts') IS DISTINCT FROM 'array' OR jsonb_array_length(p -> 'parts') = 0 THEN
        RAISE EXCEPTION 'note discard: parts must be a non-empty array';
    END IF;

    FOR v_part IN SELECT * FROM jsonb_array_elements(p -> 'parts') LOOP
        IF jsonb_typeof(v_part) IS DISTINCT FROM 'object' THEN
            RAISE EXCEPTION 'note discard: each part must be an object';
        END IF;

        -- (3) The writer is a closed word. It names no key: under the discard rule every named
        --     writer IS the discarder, so a key here would be a second, unchecked claim.
        IF v_part ->> 'writer' IS NULL OR v_part ->> 'writer' NOT IN ('discarder', 'not-known') THEN
            RAISE EXCEPTION 'note discard: each part writer must be discarder or not-known';
        END IF;

        -- (4) A non-blank narrative ("blank" is the twin's own Unicode-exact test, db/005, so
        --     two differently-collated nodes agree), as inert as a note's: no raw HTML, no
        --     external link, and — since a discard carries NO media — every anchor dangles.
        IF jsonb_typeof(v_part -> 'narrative') IS DISTINCT FROM 'string'
           OR NOT cairn_twin_is_present(v_part ->> 'narrative') THEN
            RAISE EXCEPTION 'note discard: each part narrative must be a non-blank string';
        END IF;
        v_text := v_part ->> 'narrative';
        PERFORM cairn_check_note_markup(v_text);
        PERFORM cairn_check_note_anchors(v_text, '[]'::jsonb);

        -- (5) Recording times: explicit-offset instants (the db/001 wire-pin validator, so every
        --     node reads the same instant), in order.
        IF jsonb_typeof(v_part -> 'recording_started') IS DISTINCT FROM 'string'
           OR jsonb_typeof(v_part -> 'recording_ended') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'note discard: recording_started and recording_ended must be RFC 3339 strings';
        END IF;
        BEGIN
            v_started := cairn_t_effective(v_part ->> 'recording_started');
            v_ended   := cairn_t_effective(v_part ->> 'recording_ended');
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'note discard: recording_started and recording_ended must be RFC 3339 instants with an explicit offset';
        END;
        IF v_started IS NULL OR v_ended IS NULL THEN
            RAISE EXCEPTION 'note discard: recording_started and recording_ended must be RFC 3339 instants with an explicit offset';
        END IF;
        IF v_started > v_ended THEN
            RAISE EXCEPTION 'note discard: recording_started must not be after recording_ended';
        END IF;

        -- (6) stated_time: the writer's claim. The key must be present; JSON null means they
        --     named no time (acknowledged uncertainty, principle 4), else it must be an instant.
        IF NOT (v_part ? 'stated_time') THEN
            RAISE EXCEPTION 'note discard: stated_time must be present (null when the writer named no time)';
        END IF;
        IF jsonb_typeof(v_part -> 'stated_time') <> 'null' THEN
            IF jsonb_typeof(v_part -> 'stated_time') <> 'string' THEN
                RAISE EXCEPTION 'note discard: stated_time must be null or an RFC 3339 instant';
            END IF;
            BEGIN
                v_started := cairn_t_effective(v_part ->> 'stated_time');
            EXCEPTION WHEN others THEN
                v_started := NULL;
            END;
            IF v_started IS NULL THEN
                RAISE EXCEPTION 'note discard: stated_time must be null or an RFC 3339 instant with an explicit offset';
            END IF;
        END IF;
    END LOOP;

    -- (7) The envelope's t_effective is the discard instant and must be set.
    IF cairn_t_effective(b ->> 't_effective') IS NULL THEN
        RAISE EXCEPTION 'note discard: t_effective must be set (the discard instant)';
    END IF;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382).
REVOKE EXECUTE ON FUNCTION cairn_check_note_discard(text, jsonb) FROM PUBLIC;

-- 3. Register the floor + hard twin requirement (#173). DO UPDATE with the IS DISTINCT FROM
--    guard (#214): the loader replays this file, so the row converges to this text yet a
--    steady-state replay writes nothing.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.note-discard.asserted', 'cairn_check_note_discard', 'note discard requires a non-empty authored twin (§3.13/§3.19)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 4. The projection: one row per discard, read ONLY for the "this draft was discarded" marker.
--    `parts` keeps the payload's array as is (narratives still ESCAPED; the reader unescapes).
--    content_address is what a shred scrubs by (db/037): the parts ARE clinical plaintext.
CREATE TABLE IF NOT EXISTS clinical_note_discard (
    event_id        UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    parts           JSONB NOT NULL,
    t_effective     TIMESTAMPTZ NOT NULL,
    hlc_wall        BIGINT,
    hlc_counter     INTEGER,
    origin          TEXT,
    content_address BYTEA NOT NULL
);
GRANT SELECT ON clinical_note_discard TO cairn_agent;
CREATE INDEX IF NOT EXISTS clinical_note_discard_patient_idx ON clinical_note_discard (patient_id);
CREATE INDEX IF NOT EXISTS clinical_note_discard_content_address_idx ON clinical_note_discard (content_address);

-- 5. Fold a discard into the table. Reads the CLEAR payload (ADR-0052): NULL means this node
--    holds no custody for the sealed body, so there is nothing to project (the late-custody path
--    re-runs this when a key lands — heal_safe below).
CREATE OR REPLACE FUNCTION clinical_note_discard_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- Ruling R8: project ONLY the profile the floor checked (see the floor's rule 1).
    IF e.schema_version IS DISTINCT FROM 'clinical.note-discard/1' THEN RETURN; END IF;
    INSERT INTO clinical_note_discard
        (event_id, patient_id, parts, t_effective, hlc_wall, hlc_counter, origin, content_address)
    VALUES (e.event_id, e.patient_id, p -> 'parts', e.t_effective,
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge discard rows.
REVOKE EXECUTE ON FUNCTION clinical_note_discard_apply(event_log) FROM PUBLIC;

-- 6. Register the applier (db/005, ADR-0057). heal_safe: insert-or-nothing per event.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.note-discard.asserted', 'clinical_note_discard_apply', ARRAY['clinical_note_discard'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
