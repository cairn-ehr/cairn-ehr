-- Cairn — the one substitution refusal all five event-log write doors share: db/005, db/020,
-- db/009, and db/007's two (#615, #608, #619).
--
-- WHY THIS FILE EXISTS. A substitution is a SECOND, DIFFERENT event filed under an event_id the
-- log already holds. Every door inserts `ON CONFLICT (…) DO NOTHING`, because an idempotent
-- re-write of the SAME event must stay a silent no-op — that is set-union, and it is what makes
-- sync safe (principle 1). But the identical no-op is what a substitution looks like from the
-- INSERT's point of view, so without a comparison the two are indistinguishable and the rival is
-- DISCARDED in silence: two nodes then hold different bytes under one event_id, forever, with no
-- alarm.
--
-- Two doors — db/005 and db/020, the `event_log` pair — already refused it, each with its own
-- inline copy of the same four lines. `restore_node_event` (db/009) did not, which is #615: an
-- attacker who can append to a sneakernet medium reuses the event_id of the clinic's
-- `peer.revoked`, the genuine revocation is dropped, and the node comes back TRUSTING A PEER THE
-- CLINIC REVOKED, at exit 0. That door is self-trusting by design (db/009's own drift-ceiling
-- comment says the medium "can contain OTHER signers' events and is attacker-appendable"), so it
-- is the door where the guard matters most. It was not the only one without it: db/007's two
-- node_event doors had none either — a census #615 missed; #619 (ADR-0073) gave them the call.
--
-- WHY A HELPER RATHER THAN A THIRD COPY. Both existing copies compare with `<>`. The branch is
-- reached only when a row with that id exists, so the read-back should always find one — but if
-- it ever does not, `<>` yields NULL, the IF does not fire, and THE GUARD PASSES SILENTLY
-- (#608). Copying that into the floor a third time is not a defensible way to fix a door, and
-- three doors spelling one invariant two ways is exactly the drift #159 needed a byte-identical
-- source guard to catch. One function, compared with IS DISTINCT FROM, fixes it once — and
-- `crates/cairn-node/tests/substitution_guard_is_single_source.rs` keeps it one.
--
-- WHY IT IS PURE, AND WHY THAT MATTERS. It reads no table: both content-addresses arrive as
-- arguments. That is what lets the same function serve `event_log` (db/005, db/020) and
-- `node_event` (db/007, db/009) without knowing about either, and it is why each door keeps
-- its OWN read. db/005 and db/020 are on the 100k-event clinical path and read only when
-- their INSERT was a no-op; db/009 (tens of node events per medium) and db/007's two doors (the
-- node plane is tens of events) read unconditionally, and are thereby robust to a later edit
-- disarming a ROW_COUNT they no longer set.
--
-- ⚠️ NO `REVOKE EXECUTE … FROM PUBLIC`, AND THAT IS DELIBERATE — NOT AN OVERSIGHT OF #382.
-- The convention `crates/cairn-node/tests/floor_execute_grants.rs` checks covers four families:
-- the per-event-type structural validators, the registered projection appliers, and two registry
-- triggers. This function belongs to none of them. It reads nothing, writes nothing and grants
-- nothing, so a PUBLIC caller invoking it learns strictly less than the door already tells it by
-- refusing — the same reasoning that leaves `cairn_decode_hex_or_raise` (db/001), its closest
-- sibling, unrevoked. #382's point is that a missing REVOKE a reader cannot classify as
-- deliberate is worse than either extreme; this paragraph is that classification.
-- **If this function ever starts reading a table, revisit it.**
--
-- ⚠️ REPLAY ORDER LEAVES A WINDOW, AND IT IS ACCEPTED RATHER THAN UNNOTICED. Each db/*.sql is its
-- own transaction, replayed in numeric order on every connect, so db/005 and db/020 are REPLACED
-- WITH BODIES THAT CALL THIS FUNCTION BEFORE THIS FILE CREATES IT. `SCHEMA_LOAD_LOCK` serialises
-- loaders against each other but not against ordinary writers, so a `cairn-sync` pull or an agent
-- submit landing inside that window fails with `42883 function cairn_refuse_substitution(...) does
-- not exist` — the #198 shape, displaced from "a loader omitted the file" to "a loader has not
-- reached it yet". The window is one file wide and one replay long, the failure is loud and
-- transient (the next attempt succeeds), and no write is lost: a door that raises writes nothing.
-- Moving this function into an early file would close it, at the cost of the #188 generation bump
-- that is the whole reason it is a new file (#605). If that trade is ever revisited, this is the
-- paragraph to revisit with it.

BEGIN;

CREATE OR REPLACE FUNCTION cairn_refuse_substitution(
    p_found_ca  BYTEA,
    p_new_ca    BYTEA,
    p_event_id  UUID,
    p_door      TEXT
) RETURNS VOID
-- NOT `IMMUTABLE`, though it reads nothing and would qualify on that test. A function whose only
-- effect is a side-effecting RAISE is not a value-returning pure function, and IMMUTABLE licenses
-- the planner to fold it: with constant arguments the refusal then fires at PLAN time rather than
-- execution. Harmless at every current call site (each passes variables or a sub-select), but
-- it is a genuine surprise waiting for the first caller that passes literals, and the default
-- volatility costs nothing here.
LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    -- IS DISTINCT FROM, never `<>` (#608). Three cases, not two:
    --   * same address      -> an idempotent re-write. Allow: this is set-union.
    --   * different address -> a substitution. Refuse.
    --   * p_found_ca IS NULL -> the caller could not establish what is stored under this id.
    --     `<>` yields NULL here and the write passes SILENTLY; IS DISTINCT FROM refuses. On the
    --     §9 safety-critical surface, "cannot tell" is a refusal, not a pass.
    IF p_found_ca IS DISTINCT FROM p_new_ca THEN
        -- The door name is interpolated so this reproduces both pre-existing messages
        -- byte-for-byte — which is why replacing the inline copies moved no test's expected
        -- text. Each door's own suite pins the text it raises; this file pins the comparison.
        RAISE EXCEPTION '%: event_id % already exists with different content (substitution refused)',
            p_door, p_event_id;
    END IF;
END;
$$;

COMMENT ON FUNCTION cairn_refuse_substitution(BYTEA, BYTEA, UUID, TEXT) IS
    'Refuse a second, different event filed under an event_id the log already holds. Called by '
    'submit_event (db/005), apply_remote_event (db/020), restore_node_event (db/009), and '
    'submit_node_event and apply_remote_node_event (db/007). Pure: reads no table, so it serves '
    'event_log and node_event alike. The derived inventory of every event-log writer lives in '
    'crates/cairn-node/tests/substitution_guard_covers_every_writer.rs.';

COMMIT;
