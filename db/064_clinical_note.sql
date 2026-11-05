-- 064_clinical_note.sql — the text progress note, `clinical.note.asserted` (ADR-0041, sign-in
-- slice 3a; canonical home data-model §3.19).
--
-- WHAT A NOTE IS ON THE WIRE. One signed, born-sealed event (ADR-0052: every `clinical.%` type
-- is sealed at write) whose payload is `clinical.note/1`:
--   narrative          the markdown narrative, as the window ESCAPED it before signing
--                      (cairn_event::note::escape_note_markup)
--   media              the attachment manifest — [] from this window
--   refs               [{rel, event}] inter-event relationships — [] from this window
--   encounter          the reserved grouping slot — null from this window
--   recording_started  RFC 3339, the database clock at the draft's first save (writing began)
--   recording_ended    RFC 3339, the last save that changed the text (never before the start)
--   effective_basis    'stated' (the writer named a time) | 'recording-started' (they did not)
-- The envelope's t_effective is ALWAYS set: the stated time, or recording_started.
--
-- WHAT THE FLOOR REFUSES, AND WHY ONLY THAT. A signed note is frozen forever and will be handed
-- to renderers nobody here can audit (principle 12), so the floor refuses what ADR-0041
-- excludes FOREVER — raw HTML and load-bearing external URLs — plus what it makes integrity
-- (anchor↔manifest, refs targets, honest times). It refuses nothing the v1 profile merely
-- leaves out (tables, code blocks, footnotes): those render as literal text, degraded but
-- honest (ADR-0041 decision 5). A clinician is never refused for TYPING: the window escapes
-- markup before it signs, so every refusal below is for a raw-SQL client.
--
-- THE TWO DOORS. A rule that only this window's own output obeys (media empty, encounter null)
-- is enforced at the AUTHORING door only (`cairn.remote_apply` <> 'on', the db/031 inn_code
-- idiom): at the remote door a future media-capable peer must never be wedged (ADR-0056), so
-- there the general anchor↔manifest rule applies instead. Every other rule runs at both doors —
-- for a /1 body. A NEWER profile arriving at the remote door is carried unchecked and never
-- projected (ruling R8; the floor's rule (1) and clinical_note_apply say why).
BEGIN;

-- 1. Register the type in the fail-closed classification registry. Additive, never targeting
--    another author (a `correction-of` ref is an additive claim — ADR-0041 decision 8).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.note.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The markup half of the floor (ADR-0041 decision 5), split out so each rule reads alone.
--
-- HOW TO READ THE REGEXES. Every pattern below asks "is this character UNESCAPED?", and in
-- markdown a character is escaped when an ODD number of backslashes precede it. So each one
-- opens with the same prefix:
--     (?<!\\)(?:\\\\)*       "a point NOT preceded by a backslash, then an EVEN run of
--                            backslashes (zero included)"
-- With standard_conforming_strings on (the default), a backslash in '…' is literal, so `(?<!\\)`
-- is the regex "not after a backslash" and `(?:\\\\)*` is "pairs of backslashes".
-- WHY A LOOKBEHIND, not `(^|[^\\])`: that older form CONSUMES the character before the run, so
-- in a global scan a `](` beginning right after the previous match's `)` was skipped —
-- `![![w](cairn:att/m1)](http://evil)` passed with its outer external URL unseen (fix round 1).
-- A lookbehind only looks; it never eats the character a following match needs.
CREATE OR REPLACE FUNCTION cairn_check_note_markup(p_text text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_target text;
BEGIN
    -- Raw HTML: an unescaped `<` followed by a letter, `/`, `!` or `?` — the opening of a tag
    -- (`<b>`, `</div>`), a comment (`<!--`), a processing instruction (`<?`), or an autolink
    -- (`<http://…>`). `BP <90`, `<5 min` and `a < b` pass: a digit or a space after `<` opens
    -- nothing in markdown. The window writes every `<` as `\<`, which this never matches.
    IF p_text ~ '(?<!\\)(?:\\\\)*<[A-Za-z/!?]' THEN
        RAISE EXCEPTION 'note assertion: raw HTML is not allowed in clinical.note/1 (ADR-0041 decision 5) — escape `<` as `\<`';
    END IF;
    -- Email autolinks: an unescaped `<`, then the characters an address's local part may hold,
    -- then `@` — `<9evil@evil.example>` renders as a mailto: link, and a local part may start
    -- with a digit, which the tag rule above lets through. `BP <90` and `<5 min` pass: no `@`
    -- follows before a space. (`''` is a quote doubled inside the SQL string.)
    IF p_text ~ '(?<!\\)(?:\\\\)*<[A-Za-z0-9.!#$%&''*+/=?^_`{|}~-]+@' THEN
        RAISE EXCEPTION 'note assertion: an email autolink (`<name@host>`) is not allowed in clinical.note/1 (ADR-0041 decision 5) — escape `<` as `\<`';
    END IF;

    -- Links and images: every unescaped `](` opens a target. The capture is the target up to
    -- and including its closing `)`. The target must be EXACTLY `cairn:att/<id>)` with an id
    -- of URI-unreserved characters — not merely start with `cairn:att/`: a looser test would let
    -- `[a](cairn:att/m1 [b](http://evil))` through, whose inner `[b](http://evil)` a renderer
    -- shows as an external link. The window writes `](` as `]\(`, which this never matches.
    FOR v_target IN
        SELECT m[1] FROM regexp_matches(p_text, '(?<!\\)(?:\\\\)*\]\(([^)]*\)?)', 'g') AS m
    LOOP
        IF v_target LIKE 'cairn:event/%' THEN
            RAISE EXCEPTION 'note assertion: the event anchor cairn:event/ is reserved and not valid in clinical.note/1, ADR-0041 decision 9';
        END IF;
        IF v_target !~ '^cairn:att/[A-Za-z0-9._~-]+\)$' THEN
            RAISE EXCEPTION 'note assertion: link target "%" must be a cairn:att/<id> media anchor — external URLs are excluded forever (ADR-0041 decision 5)',
                v_target;
        END IF;
    END LOOP;

    -- Reference-style link definitions (ruling R3): `[ref]: http://…` carries an external URL
    -- past the `](` rule above. Refuse any unescaped `]:` — the window writes it as `]\:`.
    IF p_text ~ '(?<!\\)(?:\\\\)*\]:' THEN
        RAISE EXCEPTION 'note assertion: a reference-style link definition (`]:`) is not allowed in clinical.note/1 — escape it as `]\:` (ADR-0041 decision 5)';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_note_markup(text) FROM PUBLIC;

-- 3. Anchor↔manifest integrity (ADR-0041 decision 3), at BOTH doors. Every unescaped anchor
--    `](cairn:att/<id>)` must name exactly one manifest entry, that entry's descriptor must be
--    non-blank, and every anchor to it must be written `![<descriptor>](cairn:att/<id>)` with
--    the descriptor byte-identical — so displayed text, twin and safety projection can never
--    disagree. The last check COUNTS: anchors to this id (unescaped `](cairn:att/<id>)`) must
--    equal occurrences of the exact literal image form. Each literal occurrence contains one
--    such anchor, so equal counts mean no anchor to this id is in any other form (a plain link,
--    other text). A manifest entry with no anchor is legal (paper-clip parity).
CREATE OR REPLACE FUNCTION cairn_check_note_anchors(p_text text, p_media jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_id      text;
    v_anchors bigint;
    v_entries bigint;
    v_desc    text;
    v_literal text;
BEGIN
    FOR v_id, v_anchors IN
        SELECT m[1], count(*)
          FROM regexp_matches(p_text, '(?<!\\)(?:\\\\)*\]\(cairn:att/([A-Za-z0-9._~-]+)\)', 'g') AS m
         GROUP BY m[1]
    LOOP
        SELECT count(*), min(e ->> 'descriptor') INTO v_entries, v_desc
          FROM jsonb_array_elements(p_media) AS e
         WHERE jsonb_typeof(e) = 'object' AND e ->> 'id' = v_id;
        IF v_entries = 0 THEN
            RAISE EXCEPTION 'note assertion: anchor cairn:att/% names no media entry (a dangling anchor, ADR-0041 decision 3)', v_id;
        END IF;
        IF v_entries > 1 THEN
            RAISE EXCEPTION 'note assertion: anchor cairn:att/% must match exactly one media entry, found %', v_id, v_entries;
        END IF;
        IF NOT cairn_twin_is_present(v_desc) THEN
            RAISE EXCEPTION 'note assertion: media entry % needs a non-blank descriptor (ADR-0041 decision 4)', v_id;
        END IF;
        v_literal := '![' || v_desc || '](cairn:att/' || v_id || ')';
        -- Occurrences of a literal: the length it removes, over its own length.
        IF (length(p_text) - length(replace(p_text, v_literal, ''))) / length(v_literal) <> v_anchors THEN
            RAISE EXCEPTION 'note assertion: every anchor to media entry % must read ![<its descriptor>](cairn:att/%) — the anchor text is the descriptor, byte for byte (ADR-0041 decision 3)',
                v_id, v_id;
        END IF;
    END LOOP;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_note_anchors(text, jsonb) FROM PUBLIC;

-- 4. The note floor, registered below for the strict and remote doors. Each rule raises its own
--    message, in the order ADR-0041 and the slice plan list them.
CREATE OR REPLACE FUNCTION cairn_check_note_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p          jsonb   := b -> 'payload';
    v_text     text;
    v_authoring boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_ref      jsonb;
    v_ref_id   uuid;
    v_started  timestamptz;
    v_ended    timestamptz;
    v_basis    text;
BEGIN
    -- (1) The schema tag IS the narrative profile (ADR-0041 decision 5): one id, one version.
    --     Ruling R8: at the REMOTE door a newer profile is CARRIED, not judged. A peer's
    --     clinical.note/2 is admitted and forwarded byte for byte without profile checks
    --     (ADR-0056; ADR-0041 decision 3: an older node forwards a newer-schema event it cannot
    --     profile-check) — refusing it would wedge sync on every node older than its author.
    --     That is safe only because clinical_note_apply projects /1 alone (below): a peer that
    --     relabels a hostile body /2 gets it carried, never on screen. The AUTHORING door writes
    --     only what this node's profile knows, so there anything but /1 is refused.
    IF NOT v_authoring AND b ->> 'schema_version' IS DISTINCT FROM 'clinical.note/1' THEN
        RETURN;
    END IF;
    IF p IS NULL THEN
        RAISE EXCEPTION 'note assertion: missing payload';
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.note/1' THEN
        RAISE EXCEPTION 'note assertion: schema_version must be clinical.note/1, got %', b ->> 'schema_version';
    END IF;

    -- (2) A narrative, not blank. "Blank" is the twin's own Unicode-exact test (db/005), so two
    --     nodes on differently-collated databases agree on the same signed bytes.
    IF jsonb_typeof(p -> 'narrative') IS DISTINCT FROM 'string'
       OR NOT cairn_twin_is_present(p ->> 'narrative') THEN
        RAISE EXCEPTION 'note assertion: narrative must be a non-blank string';
    END IF;
    v_text := p ->> 'narrative';

    -- (3) (4) (4a) No raw HTML, no external link target, no reference definition.
    PERFORM cairn_check_note_markup(v_text);

    -- (5) The media manifest: an array; empty from this window; anchors match entries.
    IF jsonb_typeof(p -> 'media') IS DISTINCT FROM 'array' THEN
        RAISE EXCEPTION 'note assertion: media must be an array';
    END IF;
    IF v_authoring AND jsonb_array_length(p -> 'media') > 0 THEN
        RAISE EXCEPTION 'note assertion: media must be empty at the authoring door (attachments are not built yet)';
    END IF;
    PERFORM cairn_check_note_anchors(v_text, p -> 'media');

    -- (6) refs: a closed rel enum and a target this node holds (ADR-0041 decision 8). Sync is
    --     set-union, not causally ordered, so a target that arrives after its note is refused
    --     at the remote door today (#948).
    IF jsonb_typeof(p -> 'refs') IS DISTINCT FROM 'array' THEN
        RAISE EXCEPTION 'note assertion: refs must be an array';
    END IF;
    FOR v_ref IN SELECT * FROM jsonb_array_elements(p -> 'refs') LOOP
        IF jsonb_typeof(v_ref) IS DISTINCT FROM 'object'
           OR (v_ref ->> 'rel') IS NULL
           OR (v_ref ->> 'rel') NOT IN ('addendum-to', 'correction-of', 'transcript-of') THEN
            RAISE EXCEPTION 'note assertion: each refs entry must be {rel, event} with rel one of addendum-to, correction-of, transcript-of';
        END IF;
        BEGIN
            v_ref_id := (v_ref ->> 'event')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'note assertion: refs event must be a uuid, got %', v_ref ->> 'event';
        END;
        IF v_ref_id IS NULL OR NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_ref_id) THEN
            RAISE EXCEPTION 'note assertion: refs event % is not held on this node (a dangling reference)', v_ref ->> 'event';
        END IF;
        -- (6a) Slice 3c-1: at the AUTHORING door an addendum adds to a NOTE of THIS chart. On
        --      paper you write an addendum under the note it adds to, in that patient's folder;
        --      a raw-SQL client naming another chart's note, or a medication event, is refused.
        --      The REMOTE door keeps the existence check above alone, deliberately: its bare
        --      RAISE is what the sync puller pens and re-offers (#948), and a peer's event is
        --      carried, not re-judged. correction-of and transcript-of are not tightened
        --      (nothing writes them yet).
        IF v_authoring AND (v_ref ->> 'rel') = 'addendum-to' AND NOT EXISTS (
            SELECT 1 FROM event_log
             WHERE event_id = v_ref_id
               AND event_type = 'clinical.note.asserted'
               AND patient_id = (b ->> 'patient_id')::uuid) THEN
            RAISE EXCEPTION 'note assertion: an addendum-to target must be a note on this chart, got %', v_ref ->> 'event';
        END IF;
    END LOOP;

    -- (7) The reserved encounter slot is inside the signed bytes from day one (ADR-0041
    --     decision 9); this window asserts no grouping context, so null.
    IF NOT (p ? 'encounter') THEN
        RAISE EXCEPTION 'note assertion: the encounter key must be present (null when no encounter is asserted)';
    END IF;
    IF v_authoring AND jsonb_typeof(p -> 'encounter') IS DISTINCT FROM 'null' THEN
        RAISE EXCEPTION 'note assertion: encounter must be null at the authoring door';
    END IF;

    -- (8) Both recording times are explicit-offset instants, parsed by the ONE wire-pin
    --     validator the envelope's t_effective uses (db/001), so every node reads the same
    --     instant. 'now' and an offset-less time are refused there: either would project a
    --     different value on each node.
    IF jsonb_typeof(p -> 'recording_started') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'recording_ended') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'note assertion: recording_started and recording_ended must be RFC 3339 strings';
    END IF;
    BEGIN
        v_started := cairn_t_effective(p ->> 'recording_started');
        v_ended   := cairn_t_effective(p ->> 'recording_ended');
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'note assertion: recording_started and recording_ended must be RFC 3339 instants with an explicit offset';
    END;
    IF v_started IS NULL OR v_ended IS NULL THEN
        RAISE EXCEPTION 'note assertion: recording_started and recording_ended must be RFC 3339 instants with an explicit offset';
    END IF;
    IF v_started > v_ended THEN
        RAISE EXCEPTION 'note assertion: recording_started must not be after recording_ended';
    END IF;

    -- (9) The basis is named, and t_effective is set and honest about it: with no stated time,
    --     recording start stands in, so the two must be the same instant (compared as instants,
    --     so `+10:00` and `Z` spellings of one moment agree).
    v_basis := p ->> 'effective_basis';
    IF v_basis IS NULL OR v_basis NOT IN ('stated', 'recording-started') THEN
        RAISE EXCEPTION 'note assertion: effective_basis must be stated or recording-started';
    END IF;
    IF cairn_t_effective(b ->> 't_effective') IS NULL THEN
        RAISE EXCEPTION 'note assertion: t_effective must be set (the stated time, or recording_started)';
    END IF;
    IF v_basis = 'recording-started' AND cairn_t_effective(b ->> 't_effective') <> v_started THEN
        RAISE EXCEPTION 'note assertion: with effective_basis recording-started, t_effective must equal recording_started';
    END IF;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382; db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_note_assertion(text, jsonb) FROM PUBLIC;

-- 5. Register the floor + hard twin requirement (#173). The twin is the narrative as typed
--    (ruling R4); the floor does not re-derive it, it requires one, as medication does.
--    DO UPDATE, not DO NOTHING (#214): the loader replays this file, so the row CONVERGES to
--    the migration text; the IS DISTINCT FROM guard keeps the steady-state replay write-free.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.note.asserted', 'cairn_check_note_assertion', 'note assertion requires a non-empty authored twin (§3.13/§3.19)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 6. The projection: one row per note, keyed by the event itself. A note is never overlaid —
--    an addendum or correction is a NEW note carrying a ref — so there is no winner rule, and a
--    replay converges by ON CONFLICT DO NOTHING. content_address is what a shred scrubs by
--    (db/037); patient_id is how a chart reads its notes.
CREATE TABLE IF NOT EXISTS clinical_note (
    event_id          UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    narrative         TEXT NOT NULL,
    refs              JSONB NOT NULL,
    t_effective       TIMESTAMPTZ NOT NULL,
    effective_basis   TEXT NOT NULL,
    recording_started TIMESTAMPTZ NOT NULL,
    recording_ended   TIMESTAMPTZ NOT NULL,
    hlc_wall          BIGINT,
    hlc_counter       INTEGER,
    origin            TEXT,
    content_address   BYTEA NOT NULL
);
-- The medication posture (db/031): the runtime role reads its projections.
GRANT SELECT ON clinical_note TO cairn_agent;
CREATE INDEX IF NOT EXISTS clinical_note_patient_idx ON clinical_note (patient_id);
CREATE INDEX IF NOT EXISTS clinical_note_content_address_idx ON clinical_note (content_address);

-- 7. Fold a note into clinical_note. Reads the CLEAR payload (ADR-0052): NULL means this node
--    holds no custody for the sealed body — nothing to project, honest degradation; the
--    late-custody path re-runs this when a key lands (heal_safe below).
CREATE OR REPLACE FUNCTION clinical_note_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- Ruling R8: project ONLY the profile this node's floor checked. The remote door admits a
    -- newer schema_version unchecked (see the floor above), so projecting it would put an
    -- unvalidated narrative on screen — raw HTML, an external URL — behind a relabelled tag.
    -- A /2 note is carried and forwarded; a node that knows /2 projects it with its own applier.
    IF e.schema_version IS DISTINCT FROM 'clinical.note/1' THEN RETURN; END IF;
    INSERT INTO clinical_note
        (event_id, patient_id, narrative, refs, t_effective, effective_basis,
         recording_started, recording_ended, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        e.event_id, e.patient_id,
        p ->> 'narrative',
        p -> 'refs',
        e.t_effective,
        p ->> 'effective_basis',
        cairn_t_effective(p ->> 'recording_started'),
        cairn_t_effective(p ->> 'recording_ended'),
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge note rows.
REVOKE EXECUTE ON FUNCTION clinical_note_apply(event_log) FROM PUBLIC;

-- 8. Register the applier with the generic dispatcher (db/005, ADR-0057) and cairn_reproject.
--    heal_safe: one row per event, insert-or-nothing, so re-running it is a no-op. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.note.asserted', 'clinical_note_apply', ARRAY['clinical_note'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
