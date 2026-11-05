\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/064 — the in-DB mirror for the text progress note's floor (ADR-0041, sign-in slice 3a).
--
-- WHY THIS MIRROR EXISTS: cairn_check_note_assertion is the ONE gate between a raw-SQL client
-- and a signed note that every renderer, forever, must render safely. Each rule below gets one
-- body it refuses (asserting WHICH rule refused it, by message fragment) and, where the rule
-- could plausibly over-refuse, the clinical text it must let through. The Rust suite
-- (crates/cairn-node/tests/note_floor.rs) drives the real seal → sign → submit → project path;
-- this pins the rules one at a time, which the end-to-end path cannot do cheaply.
--
-- The floor is called DIRECTLY (as the schema owner) with hand-built bodies: no signature is
-- needed to exercise a structural check, and the door would only add noise.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits. Query results are sent to
-- /dev/null (each check is a SELECT of a void helper, and ~60 empty result tables would bury
-- the runner's log); a failure is an ERROR, which psql writes to stderr regardless.

\o /dev/null
BEGIN;

-- A valid note body, with `over` merged into its payload. The defaults are what the window
-- writes for a note with no stated time: basis recording-started, t_effective = start.
CREATE FUNCTION pg_temp.note_body(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.note/1',
        't_effective',    '2026-11-03T01:00:00Z',
        'plaintext_twin', 'chest clear',
        'payload', jsonb_build_object(
            'narrative',         'chest clear',
            'media',             '[]'::jsonb,
            'refs',              '[]'::jsonb,
            'encounter',         NULL,
            'recording_started', '2026-11-03T01:00:00Z',
            'recording_ended',   '2026-11-03T01:05:00Z',
            'effective_basis',   'recording-started') || over);
$$;

-- The floor accepts `b`, or this test fails naming the body.
CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_note_assertion('clinical.note.asserted', b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%): %', SQLERRM, b->'payload';
END $$;

-- The floor refuses `b` with a message containing `frag` — refusing for a DIFFERENT reason is
-- a failure too, because it means the rule under test never ran.
CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        PERFORM cairn_check_note_assertion('clinical.note.asserted', b);
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        IF SQLERRM NOT LIKE 'note assertion: %' THEN
            RAISE EXCEPTION 'FAIL: refusal is not the note floor''s own: %', SQLERRM;
        END IF;
        RETURN;
    END;
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse ("%"): %', frag, b->'payload';
END $$;

-- Shorthand: a body whose only change is its narrative.
CREATE FUNCTION pg_temp.said(t text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.note_body(jsonb_build_object('narrative', t));
$$;

-- 0. Registration: the type is known, additive, keeps the conservative sensitivity bound, and
--    its floor and projection are wired.
DO $$
BEGIN
    ASSERT (SELECT mode FROM event_type_class WHERE event_type = 'clinical.note.asserted') = 'additive',
        'clinical.note.asserted is registered additive';
    ASSERT NOT cairn_event_type_has_no_thread('clinical.note.asserted'),
        'a note keeps the conservative sensitivity bound (stays off db/048''s no-thread list)';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.note.asserted')
           = 'cairn_check_note_assertion', 'the floor is registered';
    ASSERT (SELECT heal_safe FROM cairn_projection_apply
             WHERE event_type = 'clinical.note.asserted' AND apply_fn = 'clinical_note_apply'),
        'the projection is registered heal-safe';
END $$;

-- 1. Accepted: the plain body, and every clinical text that merely looks like markup.
SELECT pg_temp.accepts(pg_temp.note_body());
-- Task 1's corpus, as escape_note_markup writes it: `\` before every `<`, before a `(` or `:`
-- that follows `]`, and every typed `\` doubled.
SELECT pg_temp.accepts(pg_temp.said(t)) FROM unnest(ARRAY[
    'BP \<90',
    '\<5 min',
    'pt said "\<b>"',
    '[L] arm (R)',
    'see [x]\(http://e.org)',
    'ends with \\',
    'a\\\<b',
    'plain',
    '[ref]\: http://e.org',
    '[sic]\: he said']) AS t;
-- Unescaped text a raw-SQL client may send that is NOT markup: `<` before a digit or a space,
-- and the v1 profile's own constructs (heading, lists, emphasis, blockquote).
SELECT pg_temp.accepts(pg_temp.said(t)) FROM unnest(ARRAY[
    'BP <90',
    '<5 min',
    'a < b',
    E'# Assessment\n- chest clear\n- HS dual\n1. review in 1/24',
    E'**bold** and *italic*\n> pt states pain 3/10']) AS t;
-- A stated time that differs from recording start is the writer's claim, freely backdated.
SELECT pg_temp.accepts(pg_temp.note_body('{"effective_basis":"stated"}') ||
                       '{"t_effective":"2026-11-02T22:00:00+10:00"}');

-- Rule 0/1: the payload exists and the schema tag is /1.
SELECT pg_temp.refuses(pg_temp.note_body() - 'payload', 'missing payload');
SELECT pg_temp.refuses(pg_temp.note_body() || '{"schema_version":"clinical.note/2"}', 'schema_version');

-- Rule 2: a narrative, not blank.
SELECT pg_temp.refuses(pg_temp.note_body() #- '{payload,narrative}', 'narrative');
SELECT pg_temp.refuses(pg_temp.note_body('{"narrative":42}'), 'narrative');
SELECT pg_temp.refuses(pg_temp.said(E'  \n\t '), 'narrative');

-- Rule 3: no raw HTML — a `<` preceded by an EVEN run of backslashes (zero included) and
-- followed by a letter, `/`, `!` or `?`.
SELECT pg_temp.refuses(pg_temp.said(t), 'raw HTML') FROM unnest(ARRAY[
    'pt said <b>',
    '<script>x</script>',
    E'line one\n<img src=x>',
    '</div>',
    '<!-- hidden -->',
    '<?xml?>',
    'ok \\<b>',
    '<http://e.org>']) AS t;
-- An email autolink renders as a mailto: link (fix round 1): `<` then an address local part and `@`.
SELECT pg_temp.refuses(pg_temp.said('mail <9evil@evil.example> now'), 'autolink');

-- Rule 4: every unescaped link/image target is a cairn:att/<id> anchor.
SELECT pg_temp.refuses(pg_temp.said(t), 'cairn:att/') FROM unnest(ARRAY[
    'see [x](http://e.org)',
    '![wound](file:///etc/passwd)',
    '[x](cairn:attx)',
    '[a](cairn:att/m1 [b](http://e.org))',
    'ok [x\\](http://e.org)']) AS t;
-- Backslash parity decides which `](` is a link: an ODD run escapes the `]`, an even one does not.
-- Here only `b\\](y)` is unescaped, so the note is refused for target `y)` alone.
SELECT pg_temp.refuses(pg_temp.said('a\](x) b\\](y) c\\\](z)'), '"y)"');
SELECT pg_temp.accepts(pg_temp.said('a\](x) c\\\](z)'));
SELECT pg_temp.refuses(pg_temp.said('![prior ECG](cairn:event/01900000-0000-7000-8000-000000000001)'),
                       'ADR-0041 decision 9');

-- Rule 4a: no unescaped reference-style link definition (ruling R3).
SELECT pg_temp.refuses(pg_temp.said(t), 'reference') FROM unnest(ARRAY[
    '[ref]: http://e.org',
    E'see [ref]\n\n[ref]: http://e.org',
    'x \\]: y']) AS t;

-- Rule 5: media is an array, empty at the authoring door; an anchor needs its manifest entry.
SELECT pg_temp.refuses(pg_temp.note_body('{"media":{}}'), 'media');
SELECT pg_temp.refuses(pg_temp.note_body('{"media":[{"id":"m1","descriptor":"wound, L shin"}]}'),
                       'media must be empty');
SELECT pg_temp.refuses(pg_temp.said('![wound, L shin](cairn:att/m1)'), 'media entry');

-- Rule 6: refs is an array of {rel, event} naming a closed rel and an event held here.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
VALUES ('64646464-6464-7464-8464-646464646464', gen_random_uuid(), 'note.added',
        '1', 1, 1, 'n1', '\x64'::bytea,
        '\x1220'::bytea || digest('\x64'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
-- addendum-to is held to a NOTE of the event's own chart at this door (rule 6a, slice 3c-1), so
-- this fixture (a non-note on no chart) is refused for it; db/tests/070 pins 6a both ways.
SELECT pg_temp.accepts(pg_temp.note_body(jsonb_build_object('refs', jsonb_build_array(
    jsonb_build_object('rel', rel, 'event', '64646464-6464-7464-8464-646464646464')))))
FROM unnest(ARRAY['correction-of', 'transcript-of']) AS rel;
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"refs":[{"rel":"addendum-to","event":"64646464-6464-7464-8464-646464646464"}]}'), 'a note on this chart');
SELECT pg_temp.refuses(pg_temp.note_body('{"refs":{}}'), 'refs');
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"refs":[{"rel":"supersedes","event":"64646464-6464-7464-8464-646464646464"}]}'), 'rel');
SELECT pg_temp.refuses(pg_temp.note_body('{"refs":["64646464-6464-7464-8464-646464646464"]}'), 'rel');
SELECT pg_temp.refuses(pg_temp.note_body('{"refs":[{"rel":"addendum-to","event":"not-a-uuid"}]}'),
                       'uuid');
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"refs":[{"rel":"addendum-to","event":"01900000-0000-7000-8000-00000000dead"}]}'), 'not held');

-- Rule 7: the encounter slot is present, and null at the authoring door.
SELECT pg_temp.refuses(pg_temp.note_body() #- '{payload,encounter}', 'encounter');
SELECT pg_temp.refuses(pg_temp.note_body('{"encounter":"01900000-0000-7000-8000-000000000001"}'),
                       'encounter');

-- Rule 8: both recording times are explicit-offset instants, in order.
SELECT pg_temp.refuses(pg_temp.note_body() #- '{payload,recording_started}', 'recording_');
SELECT pg_temp.refuses(pg_temp.note_body(jsonb_build_object('recording_ended', t)), 'recording_')
FROM unnest(ARRAY['yesterday', 'now', '2026-11-03 01:05', '2026-11-03T01:05:00']) AS t;
SELECT pg_temp.refuses(pg_temp.note_body('{"recording_ended":"2026-11-03T00:59:00Z"}'), 'after');

-- Rule 9: the basis is named, and t_effective is set and honest about it.
SELECT pg_temp.refuses(pg_temp.note_body('{"effective_basis":"guessed"}'), 'effective_basis');
SELECT pg_temp.refuses(pg_temp.note_body() - 't_effective', 't_effective');
SELECT pg_temp.refuses(pg_temp.note_body() || '{"t_effective":null}', 't_effective');
SELECT pg_temp.refuses(pg_temp.note_body() || '{"t_effective":"2026-11-03T03:00:00Z"}', 'recording_started');
-- The same instant written with another offset IS recording_started: compared as instants.
SELECT pg_temp.accepts(pg_temp.note_body() || '{"t_effective":"2026-11-03T11:00:00+10:00"}');

-- The remote door (ADR-0056): a media-capable peer is never wedged. A manifest entry with its
-- anchor, and a non-null encounter, are accepted; the anchor↔manifest integrity rule still holds.
SET LOCAL cairn.remote_apply = 'on';
SELECT pg_temp.accepts(pg_temp.note_body(
    '{"narrative":"wound as shown ![wound, L shin](cairn:att/m1) healing",
      "media":[{"id":"m1","descriptor":"wound, L shin"}],
      "encounter":"01900000-0000-7000-8000-000000000001"}'));
-- An entry clipped to the note but never placed inline is legal (paper-clip parity).
SELECT pg_temp.accepts(pg_temp.note_body('{"media":[{"id":"m1","descriptor":"wound, L shin"}]}'));
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"narrative":"![wound, L shin](cairn:att/m2)", "media":[{"id":"m1","descriptor":"wound, L shin"}]}'),
    'media entry');
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"narrative":"![wound](cairn:att/m1)", "media":[{"id":"m1","descriptor":"wound, L shin"}]}'),
    'descriptor');
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"narrative":"![](cairn:att/m1)", "media":[{"id":"m1","descriptor":""}]}'),
    'descriptor');
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"narrative":"![w](cairn:att/m1)",
      "media":[{"id":"m1","descriptor":"w"},{"id":"m1","descriptor":"w"}]}'),
    'exactly one');
-- A second, plain-link anchor to the same entry carries no descriptor: refused.
SELECT pg_temp.refuses(pg_temp.note_body(
    '{"narrative":"![w](cairn:att/m1) and [here](cairn:att/m1)",
      "media":[{"id":"m1","descriptor":"w"}]}'),
    'descriptor');
-- A valid anchor nested inside an external link or image (fix round 1): the outer `](` starts
-- right after the inner anchor's `)`, so a pattern that consumed the character before `]` skipped
-- it. Both render with an external URL — an <img> fetched from a tracker, a badge link.
SELECT pg_temp.refuses(pg_temp.note_body(jsonb_build_object(
    'narrative', t, 'media', '[{"id":"m1","descriptor":"w"}]'::jsonb)), 'cairn:att/')
FROM unnest(ARRAY[
    '![![w](cairn:att/m1)](http://evil.example/px.gif)',
    '[![w](cairn:att/m1)](http://evil)']) AS t;
-- Every other rule still applies at the remote door.
SELECT pg_temp.refuses(pg_temp.said('pt said <b>'), 'raw HTML');
-- Ruling R8: a newer profile is carried, not judged. A peer's clinical.note/2 body is admitted
-- even though it breaks /1's rules (ADR-0041 decision 3: an older node forwards a newer-schema
-- event byte for byte without being able to profile-check it) ...
SELECT pg_temp.accepts(pg_temp.said('pt said <b>') || '{"schema_version":"clinical.note/2"}');
SET LOCAL cairn.remote_apply = '';
-- ... while this node's own authoring door still writes only /1.
SELECT pg_temp.refuses(pg_temp.said('pt said <b>') || '{"schema_version":"clinical.note/2"}',
                       'schema_version');

-- R8's other half: the applier projects /1 only, so relabelling a body /2 at the remote door
-- gets it carried but never on screen. Two unsealed rows inserted as the owner (the floor is
-- not under test here; cairn_clear_payload reads an unsealed row's body as its clear payload):
-- a /1 control that MUST project, proving the applier ran, and a /2 that must not.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, t_effective, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
SELECT id::uuid, gen_random_uuid(), 'clinical.note.asserted', v, 2, n, 'n1',
       '2026-11-03T01:00:00Z', sb, '\x1220'::bytea || digest(sb, 'sha256'),
       pg_temp.said('pt said <b>') -> 'payload', '[]'::jsonb, 'deadbeef', 'pt said <b>'
FROM (VALUES ('64646464-0001-7464-8464-646464646464', 'clinical.note/1', 1, '\x6401'::bytea),
             ('64646464-0002-7464-8464-646464646464', 'clinical.note/2', 2, '\x6402'::bytea))
     AS r(id, v, n, sb);
DO $$
BEGIN
    ASSERT EXISTS (SELECT 1 FROM clinical_note WHERE event_id = '64646464-0001-7464-8464-646464646464'),
        'the /1 control projects (the applier ran)';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note WHERE event_id = '64646464-0002-7464-8464-646464646464'),
        'a clinical.note/2 event is carried but never projected (ruling R8)';
END $$;

ROLLBACK;
\o
