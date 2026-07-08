-- Cairn — the attachment floor learns the RENDITION SET (ADR-0042, refines ADR-0013).
--
-- Before ADR-0042 the attachment reference was flat: digest_hex/media_type/byte_len sat
-- on each attachment, and the submit/apply doors learned one blob reference per attachment.
-- ADR-0042 nests those under a rendition set (one logical attachment = N content-addressed
-- renditions), so the doors must learn a reference per BY-REFERENCE rendition. Extracted
-- into one shared helper so the two doors (db/005 submit, db/020 remote-apply) never drift
-- (the single-source discipline db/015 used for the twin hook).

BEGIN;

-- Learn a lazy blob reference (reference-eager, byte-lazy) for every by-reference rendition
-- of every attachment in a signed body `b`. Skips INLINE renditions: their bytes ride the
-- event itself, so there is no lazy blob to fetch (noting one would create a phantom
-- present=FALSE row that never resolves). Idempotent via blob_note_reference's ON CONFLICT.
CREATE OR REPLACE FUNCTION cairn_learn_attachment_refs(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    a jsonb;
    r jsonb;
BEGIN
    FOR a IN SELECT jsonb_array_elements(COALESCE(b -> 'attachments', '[]'::jsonb)) LOOP
        FOR r IN SELECT jsonb_array_elements(COALESCE(a -> 'renditions', '[]'::jsonb)) LOOP
            -- Inline renditions carry their bytes in the event; no lazy blob reference.
            CONTINUE WHEN r ? 'inline';
            PERFORM blob_note_reference(
                decode(r ->> 'digest_hex', 'hex'),
                r ->> 'media_type',
                (r ->> 'byte_len')::bigint);
        END LOOP;
    END LOOP;
END;
$$;

COMMIT;
