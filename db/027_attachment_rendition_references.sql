-- Cairn — the attachment floor learns the RENDITION SET (ADR-0042, refines ADR-0013).
--
-- Before ADR-0042 the attachment reference was flat: digest_hex/media_type/byte_len sat
-- on each attachment, and the submit/apply doors learned one blob reference per attachment.
-- ADR-0042 nests those under a rendition set (one logical attachment = N content-addressed
-- renditions), so the doors must learn a reference per BY-REFERENCE rendition. Extracted
-- into one shared helper so the two doors (db/005 submit, db/020 remote-apply) never drift
-- (the single-source discipline db/015 used for the twin hook).
--
-- ===========================================================================
-- ISSUE #370 — every field read here is now SHAPE-CHECKED before it is used.
-- ===========================================================================
--
-- WHY THIS IS AN AVAILABILITY FLOOR, NOT A TIDY-UP
--
-- A signature proves the bytes are what the author signed. It does NOT prove the payload
-- is well formed: a buggy peer signs its own garbage perfectly, and `Rendition.digest_hex`
-- is a plain `String` in crates/cairn-event/src/attachment.rs with no floor anywhere.
--
-- cairn-sync's pull loop reads the SQLSTATE, not the message (`refusal_is_deliberate`,
-- crates/cairn-sync/src/main.rs). **P0001** — the code a bare RAISE EXCEPTION carries —
-- means "this node's floor decided against these bytes": pen them verbatim, ADVANCE the
-- cursor, keep the link alive. Any OTHER code means "something broke, retry": FREEZE the
-- cursor. So one malformed string from a trusted peer froze that peer's *clinical* pull
-- permanently — re-fetched and re-frozen every cycle, and reported to the operator as
-- "transient?", waiting for something that could never clear. Availability over
-- consistency is a governing invariant; the un-checked reads below broke it.
--
-- THE FAMILY IS NINE, NOT ONE (measured on PostgreSQL 18.1 before the fix)
--
--   attachments not an array (incl. JSON null) ... 22023   digest_hex absent ....... 23502
--   renditions not an array (incl. JSON null) .... 22023   a rendition is a scalar . 23502
--   digest_hex not hex / odd length .............. 22023   media_type absent ....... 23502
--   byte_len fractional .......................... 22P02   byte_len beyond bigint .. 22003
--
-- and FOUR shapes that raised nothing and wrote something wrong: an EMPTY digest_hex (the
-- address is blob_store's primary key, so every such rendition from every peer collides
-- into ONE row whose media_type is whichever arrived first), a NEGATIVE byte_len, a BLANK
-- media_type, and an attachment that is a scalar. Repairing digest_hex alone — which is
-- what #370 names — would have left the freeze in place for the other eight.
--
-- THE RULE THE VALIDATORS FOLLOW: refuse what already FAILED, plus what was silently
-- WRONG; accept everything that already worked. Every refusal added here is a new way for
-- a peer's clinical event to be penned, so the shapes the old code happened to accept
-- (uppercase hex, an absent byte_len, a byte_len encoded as a digit STRING) are accepted
-- deliberately and pinned by tests, not left to chance.
--
-- ⚠️ BOTH DOORS REFUSE THE WHOLE EVENT TODAY, AND THAT IS AN INTERIM STATE (issue #460).
--
-- Refusing is strictly better than the freeze it replaces, so it ships now. It is NOT the
-- right end state at the REMOTE door, and the argument that first appeared here for why it
-- was is recorded below as wrong, because a wrong safety argument is worse than none — it
-- disarms the guard it describes.
--
-- What that argument said: "a P0001 refusal is not a loss, because ADR-0056 decision 5 pens
-- the bytes, re-offers them and auto-releases when the refusal stops applying — and a
-- malformed digest is deterministic, exactly the pen's case."
--
-- Why it is wrong: cairn-sync re-offers THE SAME BYTES every cycle, and the malformed field
-- sits INSIDE the signature. The author cannot repair it; the event is immutable. So release
-- requires THIS NODE'S FLOOR to change — a human editing this file. "Deterministic" is not
-- the reason the pen is safe here, it is the reason the pen is permanent. A note event
-- carrying "adrenaline 1 mg IM given" plus one malformed photo reference is withheld from
-- this node's record indefinitely, while every peer that admitted it can read it. Slice 66
-- settled this exact shape one level up — WITHHOLD THE KEY, NEVER THE BYTES, because
-- refusing the bytes forks the event set. Here: withhold the REFERENCE, never the EVENT.
-- ADR-0056 already admits the strictly harder case (an entirely unknown event type), and
-- ADR-0060 decision 2 says partial completion must be REPORTED — an instruction to report,
-- not to refuse.
--
-- THE ASYMMETRY IS THE DESIGN, and #460 builds it: refuse HERE at db/005, where the event is
-- not yet a fact of the world and this node is the only one that can stop a permanently
-- defective event entering an append-only replicating record; ADMIT AND FLAG at db/020,
-- where the event is already a fact and refusing does not un-mint it. That is the strict/
-- lenient split the floor already uses for #345's registration precedence and for the shred
-- target-existence requirement.
--
-- WHY THE VALIDATORS LIVE HERE and not in db/001 like cairn_decode_hex_or_raise: the db/001
-- placement rule (issue #198's late-binding trap) applies to helpers reached from MORE THAN
-- ONE migration subset. These three are called from exactly one place — the loop below, in
-- this file — and db/027 is itself in cairn-sync's subset, so they travel wherever their
-- only caller does. cairn_decode_hex_or_raise is in db/001 because six doors across two
-- planes call it.

BEGIN;

-- ---------------------------------------------------------------------------
-- Three accessors: each VALIDATES one field and RETURNS it.
--
-- Deliberately not "one checker plus the original extraction". A separate check can drift
-- from what the extraction actually reads — validate one spelling, extract another, and the
-- floor is decorative. Returning the value makes the check and the use the same expression,
-- so they cannot come apart. Each is pure (jsonb in, one scalar out), so each is testable on
-- its own; `p_door` is threaded through only so a refusal names where it came from.
--
-- All three check the SHAPE and then convert, rather than converting inside an EXCEPTION
-- handler. That is db/034's idiom and cairn_decode_hex_or_raise's: a handler would relabel
-- an unrelated internal error as bad caller input, and `WHEN OTHERS` additionally does not
-- catch a statement timeout (57014 is one of the two codes it excludes).
-- ---------------------------------------------------------------------------

-- The content address of a by-reference rendition.
--
-- Non-empty is the floor; the LENGTH is deliberately NOT constrained. Today every address
-- is a 34-byte BLAKE3 multihash, but `Rendition.alg` exists precisely so a future digest
-- algorithm is an additive migration (ADR-0012, ADR-0015) — pinning 34 bytes here would
-- make that migration a floor change on every node in the fleet. An EMPTY address is
-- refused because it is not a weaker address, it is not an address at all, and it collides
-- globally on blob_store's primary key.
-- No volatility marker (so: VOLATILE, PostgreSQL's default). The other three accessors here
-- are marked IMMUTABLE because they read nothing but their arguments; this one calls
-- cairn_decode_hex_or_raise, which db/001 declares VOLATILE. PostgreSQL does not check that
-- an IMMUTABLE function only calls immutable ones, so the marker would be a promise this
-- function cannot keep — and a false IMMUTABLE is licence for the planner to fold or cache a
-- call whose behaviour it has been told wrongly.
CREATE OR REPLACE FUNCTION cairn_rendition_address(r jsonb, p_door text)
RETURNS bytea LANGUAGE plpgsql AS $$
DECLARE
    v_hex text := r ->> 'digest_hex';
BEGIN
    IF v_hex IS NULL THEN
        RAISE EXCEPTION '%: a by-reference rendition is missing digest_hex', p_door
            USING DETAIL = 'a rendition with no inline bytes must name the content address '
                           'of the bytes it refers to';
    END IF;
    IF v_hex = '' THEN
        RAISE EXCEPTION '%: digest_hex is empty', p_door
            USING DETAIL = 'an empty content address is not a weaker address, it is none at '
                           'all — and it is the primary key of blob_store, so every empty '
                           'reference from every peer would collide into one row';
    END IF;
    -- Hex shape (non-hex characters, odd length) is cairn_decode_hex_or_raise's job — the
    -- db/001 helper issue #228 added for exactly this, which also raises P0001 and never
    -- echoes the whole value back into a log.
    RETURN cairn_decode_hex_or_raise('digest_hex', v_hex, p_door);
END;
$$;

-- The media type of a by-reference rendition.
--
-- blob_store.media_type is NOT NULL, so an absent one used to surface as a 23502 constraint
-- violation two frames away — a freeze, and a message naming a column rather than a field.
-- Blank is refused as well: it satisfied NOT NULL and told a later reader nothing.
CREATE OR REPLACE FUNCTION cairn_rendition_media_type(r jsonb, p_door text)
RETURNS text LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_mt text := r ->> 'media_type';
BEGIN
    IF v_mt IS NULL OR btrim(v_mt) = '' THEN
        RAISE EXCEPTION '%: a by-reference rendition is missing a usable media_type', p_door
            USING DETAIL = 'media_type must be present and non-blank: it is how a node '
                           'decides whether it can render the bytes it has not fetched yet';
    END IF;
    RETURN v_mt;
END;
$$;

-- The byte length of a by-reference rendition, or NULL when it is not yet known.
--
-- OPTIONAL by design: blob_store.byte_len is nullable because a reference can be learned
-- before anyone knows how long the bytes are (reference-eager, byte-lazy). Absent and JSON
-- null both mean "unknown" and are accepted as NULL.
--
-- The check is on the TEXT form rather than on jsonb_typeof, and that is the point: a JSON
-- *number* is not sufficient (3.5 is a number and its cast to bigint raises 22P02), while a
-- digit STRING is not insufficient (the old `(… ->> 'byte_len')::bigint` accepted "7", so
-- refusing it would be a behaviour change dressed as a bug fix). One regex covers both, and
-- the 18-digit cap keeps the cast below bigint's ceiling without a second range test — no
-- real attachment is 10^18 bytes. Negative is refused: it used to be accepted silently, and
-- a negative length is not an uncertain length, it is a wrong one.
CREATE OR REPLACE FUNCTION cairn_rendition_byte_len(r jsonb, p_door text)
RETURNS bigint LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_len text := r ->> 'byte_len';
BEGIN
    IF v_len IS NULL THEN
        RETURN NULL;                       -- absent, or JSON null: length not yet known
    END IF;
    IF v_len !~ '^[0-9]{1,18}$' THEN
        RAISE EXCEPTION '%: byte_len is not a whole non-negative number of bytes', p_door
            USING DETAIL = 'byte_len must be 1-18 digits: fractional, negative and '
                           'out-of-range values used to reach the bigint cast and raise '
                           'outside P0001, which freezes the pull cursor (issue #370)';
    END IF;
    RETURN v_len::bigint;
END;
$$;

-- A jsonb value that must be a list, coerced to one or refused.
--
-- Absent and JSON null both mean "none" and become an empty array. Treating null as none is
-- the least-refusing honest reading: there is nothing to learn either way, so refusing it
-- would pen a whole clinical event over one encoder's choice between `null` and `[]`. Any
-- OTHER non-array — a string, a number, an object — is a genuine shape error and is refused,
-- because it used to reach jsonb_array_elements and raise 22023.
--
-- THE COALESCE MAKES THIS FUNCTION TOTAL: it always returns a jsonb ARRAY, never SQL NULL.
-- That is the contract, and it is pinned by a test (`the_list_coercion_is_total`), because
-- an earlier draft of this comment claimed something stronger and false — that dropping the
-- COALESCE would make the guard fail OPEN. It would not, today: `jsonb_array_elements(NULL)`
-- yields zero rows rather than raising, so the only immediate effect is that this function
-- stops being total. The reason to keep it is the NEXT reader: `jsonb_typeof(NULL)` is NULL,
-- so any check of the form `jsonb_typeof(x) <> 'array'` written against a possibly-NULL value
-- silently does nothing (issue #346's fail-open pattern). Totality is what makes such a check
-- safe to write here. A wrong safety argument is worse than none — it disarms the guard it
-- describes — so this one is stated as what it is and asserted rather than asserted as what
-- it is not.
CREATE OR REPLACE FUNCTION cairn_json_list_or_raise(v jsonb, p_field text, p_door text)
RETURNS jsonb LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_list jsonb := COALESCE(v, '[]'::jsonb);
BEGIN
    IF jsonb_typeof(v_list) = 'null' THEN
        RETURN '[]'::jsonb;
    END IF;
    IF jsonb_typeof(v_list) <> 'array' THEN
        RAISE EXCEPTION '%: % must be a list, not a %', p_door, p_field, jsonb_typeof(v_list)
            USING DETAIL = 'a non-list here used to reach jsonb_array_elements and raise '
                           'outside P0001, which freezes the pull cursor (issue #370)';
    END IF;
    RETURN v_list;
END;
$$;

-- Learn a lazy blob reference (reference-eager, byte-lazy) for every by-reference rendition
-- of every attachment in a signed body `b`. Skips INLINE renditions: their bytes ride the
-- event itself, so there is no lazy blob to fetch (noting one would create a phantom
-- present=FALSE row that never resolves). Idempotent via blob_note_reference's ON CONFLICT.
--
-- The door name in a refusal is this function, not its caller, because the signature stays
-- one-argument: adding a `p_door` parameter would create an OVERLOAD rather than replace the
-- function (CREATE OR REPLACE matches on the argument list), leaving the old unvalidated
-- one-argument version resident in every database that has already loaded this file. Which
-- of the two doors was running is still visible — PL/pgSQL puts the whole call stack in the
-- error CONTEXT.
CREATE OR REPLACE FUNCTION cairn_learn_attachment_refs(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    a jsonb;
    r jsonb;
    v_door CONSTANT text := 'cairn_learn_attachment_refs';
BEGIN
    FOR a IN SELECT jsonb_array_elements(
                        cairn_json_list_or_raise(b -> 'attachments', 'attachments', v_door)) LOOP
        FOR r IN SELECT jsonb_array_elements(
                            cairn_json_list_or_raise(a -> 'renditions', 'renditions', v_door)) LOOP
            -- Inline renditions carry their bytes in the event; no lazy blob reference.
            CONTINUE WHEN r ? 'inline';
            PERFORM blob_note_reference(
                cairn_rendition_address(r, v_door),
                cairn_rendition_media_type(r, v_door),
                cairn_rendition_byte_len(r, v_door));
        END LOOP;
    END LOOP;
END;
$$;

COMMIT;
