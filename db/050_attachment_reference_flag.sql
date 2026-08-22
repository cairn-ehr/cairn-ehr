-- Cairn — a malformed attachment reference must not sink a replicated clinical event (#460).
--
-- ===========================================================================
-- THIS IS NOT A NEW DECISION. IT IS ADR-0063's RULE, APPLIED WHERE IT ALREADY BOUND.
-- ===========================================================================
--
-- #370 (db/027) turned nine ways a malformed rendition reference could FREEZE the clinical
-- pull cursor into legible P0001 refusals. That was the availability fix, and it was right.
-- It also refused at BOTH doors — and that half contradicted an ADR written eight days
-- earlier, which nobody read before reopening the question.
--
-- ADR-0063 §"the floor is at the LOCAL door only and the read model is total" decides this
-- exact shape, in a table, for the §5.9 `safety` field:
--
--     | malformed / self-contradictory field | local door: REFUSE | remote door: ADMIT |
--
-- and names the rule behind it. QUOTED EXACTLY: "an envelope-level GRADED field is constrained
-- where it is minted and read permissively where it arrives." An attachment rendition reference
-- is NOT graded, so #460 EXTENDS that rule rather than merely applying it — and what carries the
-- extension is not the sentence but the rejected-alternatives argument below, which turns on
-- BLAST RADIUS and never mentions `safety`. (An earlier draft of this header dropped the word
-- "graded" from inside the quotation marks, which made an extension read as a citation. That is
-- what issue #461 exists to fix properly.) The rejected alternatives reject precisely what #370
-- shipped:
--
--     "Refusing a malformed signal at the apply door ... fails on blast radius: the safety
--      signal is a field on a clinical event, so refusing it at apply drops the medication
--      assertion — an advisory, de-identified field cancelling clinical content, which
--      ADR-0060 forbids in as many words. It also forks the event set between honest peers
--      running different versions (the #342 trap, hit four times in this project already)."
--
-- AN ATTACHMENT REFERENCE IS THE SAME CATEGORY. A sensitivity assertion IS an event, so
-- refusing a malformed one drops one assertion. `safety`, `clock_grade` and an attachment
-- rendition reference are all FIELDS ON a clinical event: refusing one at the apply door drops
-- the note, the medication assertion, the whole clinical act it rode on. Three instances of
-- one rule (ADR-0058's clock_grade / db/040, ADR-0063's safety, this) — and no ADR names the
-- rule in general, which is how a fourth reader gets it wrong. Proposed as its own ADR in
-- issue #461; until then, this header and ADR-0063's table are where the rule lives.
--
-- THE ASYMMETRY, RESTATED FOR THIS FILE:
--
--   submit_event (db/005) — REFUSE. The field is being MINTED. The event is not yet a fact of
--     the world and this node is the only one that can stop it; admitting writes a
--     permanently-defective event into an append-only replicating record, correctable only by
--     overlay, with the broken original resident for the life of the record.
--
--   apply_remote_event (db/020) — ADMIT AND FLAG. The field has ARRIVED. The event is already
--     a fact; refusing does not un-mint it, it only blinds THIS node to content its peers can
--     read. That is a fork of the event set — the #342 trap for the fifth time.
--
-- One argument #370 made that is worth killing explicitly, because it is seductive: "a P0001
-- refusal is not a loss, because ADR-0056 decision 5 pens the bytes and re-offers them, and a
-- malformed digest is deterministic — exactly the pen's case." THE PEN NEVER RELEASES.
-- cairn-sync re-offers THE SAME BYTES every cycle and the malformed field sits INSIDE the
-- signature, so the author cannot repair it and the event is immutable: release requires THIS
-- NODE'S FLOOR to change, i.e. a human editing db/027. Deterministic is why the pen is
-- PERMANENT, not why it is safe. A note event carrying "adrenaline 1 mg IM given" plus one
-- malformed photo reference was withheld from this node's record indefinitely, while every
-- peer that admitted it could read it.
--
-- Slice 66 settled the shape one level up — withhold the key, never the bytes, because
-- refusing the bytes forks the event set. Here: withhold the REFERENCE, never the EVENT.

BEGIN;

-- ---------------------------------------------------------------------------
-- The ledger.
--
-- A LEDGER, NOT A VIEW, per the Slice 68 rule (flag what cannot self-heal, view what can).
-- This cannot self-heal: the malformed field is inside a signature over an immutable event,
-- so no later arrival ever makes it well-formed. A view derived from event_log would have to
-- re-parse every attachment on every read to say the same thing.
--
-- A cross-type DOOR-SIDE write, NOT an ADR-0057 projection (cairn_projection_dispatch keys on
-- event_type; an unlearnable reference is type-independent). Like t_effective_ceiling_flag
-- (db/040) it therefore survives a `cairn_reproject` rebuild untouched — rebuild replays
-- through the dispatch, never the doors, and the inputs are immutable.
--
-- NAME, NEVER COUNT (the Slice 69 rule): the row says WHICH attachment and WHICH rendition,
-- and carries the accessor's refusal MESSAGE **and its DETAIL** (cairn_flag_reason_text below —
-- SQLERRM alone drops the DETAIL, which is the half that tells truncation from wrong-encoding).
-- "3 events have bad references" cannot tell an operator whether to chase a peer's encoder bug
-- or one corrupted import.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS attachment_reference_flag (
    flag_id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- The FK is what stops a flag naming an event this node does not hold. ON DELETE CASCADE
    -- mirrors event_deferred, and is honestly UNREACHABLE: event_log is append-only and db/001's
    -- trigger refuses DELETE outright (principle 1), so the cascade can never fire. Kept for
    -- consistency with its sibling and as a correct statement of intent, but do not cite it as a
    -- live guarantee — the guarantee is the reference itself.
    event_id         UUID NOT NULL REFERENCES event_log(event_id) ON DELETE CASCADE,
    -- Three shapes, and the NULLs are honest unknowns rather than sentinels (principle 4):
    --
    --   (i, j)      one rendition could not be learned — the common case.
    --   (i, NULL)   attachment i's `renditions` was not a list, so no rendition index exists.
    --               The ATTACHMENT index does exist and is recorded: NAME, NEVER COUNT.
    --   (NULL,NULL) `attachments` itself was not a list. Nothing to point at, and nothing was
    --               learnable, so nothing is lost by not pointing.
    --
    -- A sentinel like -1 would be a precise untruth in all three.
    attachment_index INT,
    rendition_index  INT,
    reason           TEXT        NOT NULL,
    flagged_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- Set-union sync re-offers bytes freely (a full sweep, a re-pull from zero, a peer serving the
-- same event twice), so re-delivery must dedupe rather than grow a row per delivery.
--
-- NULLS NOT DISTINCT is load-bearing and is NOT the default: with the default NULLS DISTINCT
-- every not-attributable row would be unique against every other, and a body with a malformed
-- attachments list would add one row per re-offer forever — the exact unbounded growth this
-- index exists to stop, in the one case that cannot name an index.
CREATE UNIQUE INDEX IF NOT EXISTS attachment_reference_flag_event_rendition_idx
    ON attachment_reference_flag (event_id, attachment_index, rendition_index) NULLS NOT DISTINCT;

CREATE INDEX IF NOT EXISTS attachment_reference_flag_event_idx
    ON attachment_reference_flag (event_id);

GRANT SELECT ON attachment_reference_flag TO cairn_agent;

-- The recorder. STRUCTURALLY non-gating, in db/029's hlc_collision_log idiom: plain SQL, one
-- INSERT ... SELECT with an existence guard in the WHERE, plus ON CONFLICT DO NOTHING. That
-- matters because this function runs INSIDE the handler that is catching a refusal, so anything
-- it raised would escape past the handler and fail the apply door — turning a metadata problem
-- into the clinical-availability problem this entire file exists to prevent. Worse, the outer
-- handler would then call it AGAIN and fail again, this time with nothing above it, leaving a
-- non-P0001 code for cairn-sync to read as transient: a permanent freeze on an event that will
-- never apply. So the property is load-bearing twice over.
--
-- ⚠️ STATE THE PROPERTY AS WHAT IT IS, NOT AS AN ABSOLUTE. The construction closes 23503 (the
-- missing target) and 23505 (re-delivery). It does NOT close 42501: this function is not
-- SECURITY DEFINER, so a caller holding only SELECT on the ledger gets "permission denied for
-- table" — measured on PG 18.1, escaping both handlers. What closes that is a CALLER property
-- (apply_remote_event is SECURITY DEFINER and runs as the owner) plus the REVOKE below, not the
-- INSERT's shape. The previous wording — "it cannot raise, by construction" — was a strictly
-- stronger absolute than the one this same block records as having been proven wrong two
-- paragraphs down, on this same function.
--
-- ⚠️ THE `WHERE EXISTS` IS THE LOAD-BEARING PART, AND THE FIRST DRAFT DID NOT HAVE IT.
-- `event_id` REFERENCES event_log, so a plain VALUES insert raises 23503 (foreign_key_violation)
-- whenever the flag is recorded before its event exists — OUTSIDE the P0001 handler, so it
-- propagates and refuses the event. The db/tests/050 mirror caught it on the first run. The
-- header above it already claimed "it cannot raise": the claim was written before the FK, and
-- survived it. Guarding the insert makes the claim true rather than deleting it.
--
-- The FK stays, and its ON DELETE CASCADE is why: a flag must never outlive its event (the
-- event_deferred precedent). db/020 inserts into event_log well before it learns references, so
-- the guard's skip branch is unreachable there — and `the_apply_door_admits_a_malformed_digest_
-- and_flags_it` fails loudly if that ordering ever changes, which is the check that keeps the
-- skip from becoming a silent hole.
CREATE OR REPLACE FUNCTION cairn_record_attachment_reference_flag(
    p_event_id uuid, p_attachment_index int, p_rendition_index int, p_reason text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO attachment_reference_flag (event_id, attachment_index, rendition_index, reason)
    SELECT p_event_id, p_attachment_index, p_rendition_index, p_reason
    WHERE EXISTS (SELECT 1 FROM event_log WHERE event_id = p_event_id)
    ON CONFLICT (event_id, attachment_index, rendition_index) DO NOTHING;
$$;

-- ---------------------------------------------------------------------------
-- One iteration, two policies.
--
-- The traversal itself — `cairn_by_reference_renditions` — lives in **db/027**, beside the
-- accessors and the strict learner that also runs it. It is declared there rather than here
-- for the reason db/027's header now records: its callers are in db/027 and db/050, both of
-- which are in cairn-sync's subset, and putting the shared traversal in the LATER file would
-- have meant the strict learner (earlier) forward-referencing it.
--
-- The two learners differ ONLY in what they do with a fault: the strict one re-raises it, the
-- lenient one records it. That is the whole difference, and `db/tests/050` section 9 reads
-- pg_proc to fail if either ever stops calling the shared traversal — because four files
-- asserted that guarantee while an earlier draft duplicated the loop.
-- ---------------------------------------------------------------------------

-- The text an operator reads: the accessor's message AND its DETAIL.
--
-- SQLERRM returns the primary message only, and every accessor in db/027 puts the
-- discriminating half in `USING DETAIL` — hex_decode_helper.rs states the standard: "its DETAIL
-- says WHICH hex fault, because truncation and wrong-encoding want opposite responses from
-- whoever reads the log." A ledger built on SQLERRM alone records the half that does not
-- discriminate and drops the half that does, while calling the result verbatim.
CREATE OR REPLACE FUNCTION cairn_flag_reason_text(p_message text, p_detail text)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT p_message
        || CASE WHEN COALESCE(btrim(p_detail), '') = '' THEN '' ELSE ' — ' || p_detail END;
$$;

-- The LENIENT learner — db/020's half of the asymmetry.
--
-- Learns every well-formed by-reference rendition and records every malformed one. A defect on
-- one rendition never invalidates its siblings: an attachment whose preview is malformed still
-- yields its original and its extracted text (ADR-0060, applied where it does fit — these ARE
-- independent lines).
--
-- ⚠️ `WHEN raise_exception` IS NARROW ON PURPOSE, AND `WHEN OTHERS` HERE WOULD BE A DISASTER.
-- `WHEN OTHERS` would write a disk error, a serialization failure or a broken constraint into
-- the ledger as "the peer sent a malformed reference" and then admit the event as though
-- nothing had gone wrong — a real fault laundered into a false accusation, and cairn-sync
-- robbed of the non-P0001 SQLSTATE it needs to treat the failure as transient and RETRY.
-- (`OTHERS` also does not catch a statement timeout: 57014 query_canceled is one of the two
-- codes it excludes — the Slice 68 lesson.) Measured on PG 18.1: a 22-class error raised
-- inside this block propagates past the handler untouched, and
-- `the_lenient_learner_does_not_swallow_a_real_fault` pins it with an injected fault.
--
-- ⚠️ BUT P0001 IS NOT A SYNONYM FOR "OUR ACCESSORS", AND AN EARLIER DRAFT SAID IT WAS.
-- The precise reason this narrow catch is safe today is: the only P0001 reachable inside the
-- inner block comes from db/027's accessors, because `blob_note_reference` (db/003) is plain
-- SQL and the one trigger on `blob_store` that raises bare — db/026's `cairn_blob_present_guard`
-- — is declared `FOR EACH ROW WHEN (NEW.present)`, and `blob_note_reference` writes a row with
-- `present` defaulted FALSE. THAT CONDITION LIVES IN ANOTHER FILE. Widen db/026's WHEN clause,
-- or teach blob_note_reference to set `present`, and a wrong-bytes-under-a-content-address
-- integrity refusal — the seam ADR-0013 names as this tier's safety-critical one — is silently
-- rewritten as "the peer sent a malformed reference" and the event admitted. Pinned by
-- `the_blob_present_guard_stays_out_of_reach_of_the_lenient_catch` in the Rust suite, which
-- fails if that WHEN clause moves.
--
-- The OUTER handler catches the ONE fault that has no attachment to name: `attachments` itself
-- not being a list. A malformed `renditions` list is NOT caught here — db/027's traversal
-- returns it as a fault row so its siblings still learn and the flag names its attachment.
-- Nothing has been learned when the outer handler fires, because a set-returning PL/pgSQL
-- function materialises before its first row is returned — and here that costs nothing, since a
-- non-list `attachments` had nothing learnable in it.
--
-- A NEW NAME rather than a second signature: `CREATE OR REPLACE FUNCTION` matches on the
-- argument list, so adding a parameter to cairn_learn_attachment_refs would create an OVERLOAD
-- and leave the old one-argument version resident in every database that has already loaded
-- db/027 — a silently unvalidated door.
CREATE OR REPLACE FUNCTION cairn_learn_attachment_refs_lenient(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    rec RECORD;
    v_door CONSTANT text := 'cairn_learn_attachment_refs_lenient';
    v_event_id uuid := (b ->> 'event_id')::uuid;
    v_detail text;
BEGIN
    BEGIN
        FOR rec IN SELECT * FROM cairn_by_reference_renditions(b, v_door) LOOP
            -- A whole attachment whose renditions were not a list. Its index EXISTS, so the
            -- flag names it; recording NULL here would be a precise untruth (principle 4).
            IF rec.fault IS NOT NULL THEN
                PERFORM cairn_record_attachment_reference_flag(
                    v_event_id, rec.attachment_index, NULL,
                    cairn_flag_reason_text(rec.fault, rec.fault_detail));
                CONTINUE;
            END IF;
            BEGIN
                PERFORM blob_note_reference(
                    cairn_rendition_address(rec.rendition, v_door),
                    cairn_rendition_media_type(rec.rendition, v_door),
                    cairn_rendition_byte_len(rec.rendition, v_door));
            EXCEPTION WHEN raise_exception THEN
                GET STACKED DIAGNOSTICS v_detail = PG_EXCEPTION_DETAIL;
                PERFORM cairn_record_attachment_reference_flag(
                    v_event_id, rec.attachment_index, rec.rendition_index,
                    cairn_flag_reason_text(SQLERRM, v_detail));
            END;
        END LOOP;
    EXCEPTION WHEN raise_exception THEN
        GET STACKED DIAGNOSTICS v_detail = PG_EXCEPTION_DETAIL;
        PERFORM cairn_record_attachment_reference_flag(
            v_event_id, NULL, NULL, cairn_flag_reason_text(SQLERRM, v_detail));
    END;
END;
$$;

-- Neither the recorder nor the lenient learner is a public entry point: both are reached only
-- from `apply_remote_event`, which is SECURITY DEFINER and therefore executes them as the owner.
-- Leaving PUBLIC's default EXECUTE in place made two claims above false for a low-privilege
-- caller — the recorder raises 42501 with only SELECT on the ledger (escaping BOTH handlers),
-- and the lenient learner called with a body carrying no `event_id` absorbs every refusal and
-- records nothing, the silently-unvalidated-door shape this file warns about for overloads.
REVOKE EXECUTE ON FUNCTION cairn_record_attachment_reference_flag(uuid, int, int, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_learn_attachment_refs_lenient(jsonb) FROM PUBLIC;

-- ---------------------------------------------------------------------------
-- The read surface.
--
-- Slice 69's finding was that three slices shipped a §5.9 mechanism with no way to look at it,
-- so its budget stood owed rather than met. A ledger nobody can query is the same mistake.
--
-- A chart-scoped SECURITY DEFINER read granted to BOTH group roles, mirroring db/043's
-- cairn_patient_deferred_sensitivity — whose first draft granted to cairn_agent alone and
-- called that "the runtime role", which it is not (#425). search_path is pinned, and pg_temp
-- is named LAST because `public` alone does not exclude the caller's temp schema (the leak
-- that let a decoy event_log capture an owner-privileged INSERT at both write doors).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_patient_attachment_flags(p_patient uuid)
RETURNS TABLE (
    event_id         uuid,
    event_type       text,
    attachment_index int,
    rendition_index  int,
    reason           text,
    flagged_at       timestamptz)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT f.event_id, e.event_type, f.attachment_index, f.rendition_index,
           f.reason, f.flagged_at
    FROM attachment_reference_flag f
    JOIN event_log e USING (event_id)
    WHERE e.patient_id = p_patient
    ORDER BY f.flagged_at, f.attachment_index, f.rendition_index;
$$;

-- ---------------------------------------------------------------------------
-- The NODE-WIDE read, because the chart-scoped one answers the wrong question first.
--
-- cairn_patient_attachment_flags(uuid) requires you to already know WHICH chart to ask about.
-- But a malformed reference is discovered FROM the ledger, not from a chart — and the question
-- the ledger's own header poses ("chase a peer's encoder bug, or one corrupted import?") is
-- fleet-shaped, not patient-shaped. A report you can only run once you already know the answer
-- is the Slice 69 finding wearing a different hat.
--
-- db/040 is the sibling that got this right: a cairn_agent-only flag TABLE paired with a
-- node-wide cairn_clock_health() granted to both roles. Same shape here.
--
-- WHO CALLS IT (issue #465, added after this file shipped without a caller). This
-- function is the STANDING picture — every flag this node holds, whoever delivered it.
-- The per-CYCLE news is cairn-sync's: both `pull` and `requeue` read the ledger for the
-- events they just admitted and report a `references_unlearnable` count plus one stderr
-- line naming an example, following the `custody_withheld` precedent (a degradation that
-- must be visible without failing the cycle). The two are deliberately different
-- questions — "what did this pull just discover" vs "what does this node stand holding"
-- — and an operator woken by the first comes here for the second.
--
-- NAME, NEVER COUNT still holds: this returns one row per (event_type, reason) with the events
-- that carry it, so "47 flags" can never be all an operator sees.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_attachment_flag_health()
RETURNS TABLE (
    event_type    text,
    reason        text,
    flagged       bigint,
    first_flagged timestamptz,
    last_flagged  timestamptz,
    example_event uuid)
LANGUAGE sql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
    SELECT e.event_type, f.reason, count(*), min(f.flagged_at), max(f.flagged_at),
           -- array_agg, not min(): PostgreSQL has no min() for uuid. Ordered by flagged_at then
           -- flag_id so the example is the EARLIEST occurrence and is stable across calls — an
           -- operator re-running this must land on the same event they were just looking at.
           (array_agg(f.event_id ORDER BY f.flagged_at, f.flag_id))[1]
    FROM attachment_reference_flag f
    JOIN event_log e USING (event_id)
    GROUP BY e.event_type, f.reason
    ORDER BY count(*) DESC, e.event_type, f.reason;
$$;

REVOKE EXECUTE ON FUNCTION cairn_attachment_flag_health() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_attachment_flag_health() TO cairn_agent, cairn_node;

REVOKE EXECUTE ON FUNCTION cairn_patient_attachment_flags(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_patient_attachment_flags(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_patient_attachment_flags(uuid) TO cairn_node;

COMMIT;
