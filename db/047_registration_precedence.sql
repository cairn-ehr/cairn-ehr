-- Cairn — retire `patient.created`; registration takes over the chart-birth projection
-- (spec §5.3/§5.8, ADR-0061; issue #345).
--
-- The precedence rule itself lives at the door that enforces it — `submit_event` step 8b in
-- db/005, over the `cairn_patient_has_events` predicate in db/001. This file is the other half
-- of the same change: making the rule's single sentence TRUE by removing the one event type
-- that would otherwise need an exception carved out of it.
--
-- # Why a type is being retired at all
--
-- `patient.created` is a walking-skeleton event type: classified `additive` in db/005,
-- projecting to `patient_chart` at run_order 10, carrying a `{name, dob, sex}` payload
-- superseded by demographics slices 1-5, with NO structural floor and no twin-check row. It is
-- an unfloored registration act. Leaving it classified once db/005 step 8b requires the first
-- event on a chart to be a registration would mean either
--
--   (a) it stays a PERMITTED first event — reintroducing exactly the "unless" that ADR-0061
--       decision 2 removed by giving all three §5.3 classes one event type, and an "unless" in
--       a safety floor is where the next defect lives; or
--   (b) it becomes a type that may only ever be written SECOND, which is no type at all — its
--       whole payload is superseded, and nothing authors it but test fixtures.
--
-- # Order matters, and db/005 says so
--
-- db/005's projection-registry validation trigger (cairn_check_projection_registry_fn) refuses
-- to register a projection for an unclassified type, and records the residual: the check runs
-- at REGISTRATION time, so it cannot see a class row deleted AFTERWARDS. A
-- registered-but-unclassified type would leave the AFTER-INSERT dispatcher firing for an event
-- the floor admitted uninterpreted — granting exactly the power ADR-0056's deferral withholds.
-- So the projection rows go FIRST and the class row second. Any future retirement must do the
-- same; this file is the precedent.
--
-- # What happens to `patient.created` events that already exist
--
-- They stay in the log, exactly as principle 1 requires — nothing is rewritten and nothing is
-- erased. Their existing `patient_chart` rows stay too, because a heal-mode reproject never
-- truncates. A `reproject --rebuild` would NOT re-derive them, since the type no longer
-- resolves to an apply fn: the chart row would come back only from the events that still
-- project. That is acceptable and deliberate on a pre-clinical project, and it is stated here
-- so it is discovered by reading rather than by surprise.
--
-- A PEER still running older code may keep sending `patient.created`. The remote door admits
-- it UNINTERPRETED (ADR-0056: custody total, interpretation deferred, power earned) — no
-- projection, no refusal, no wedged watermark. Retiring a type locally is not removing it from
-- the wire, and this migration must never be read as a wire break.

BEGIN;

-- 1. Registration takes over the chart-birth projection.
--
--    The apply fn is db/002's `patient_chart_apply`, which gained an
--    `identity.registration.asserted` branch in the same commit as this file. Registered HERE
--    rather than in db/045 (which registers the retained-set `patient_registration_apply` for
--    the same type) because both preconditions the validation trigger checks — the apply fn
--    exists, and the event type is classified — are first simultaneously true at this point in
--    the replay order, and because everything #345 changes should be readable in one file.
--
--    A type with TWO registered apply fns is the dispatcher's normal case, not a special one:
--    it orders by (run_order, apply_fn) within a type, so `patient_chart_apply` runs before
--    `patient_registration_apply`. The two write disjoint tables, so the order carries no
--    meaning beyond determinism.
--
--    heal_safe = TRUE: the branch is an idempotent upsert keyed on patient_id whose only
--    non-constant write is a GREATEST() on last_activity, so replaying an already-projected
--    registration converges rather than accumulating (unlike note.added's counter).
--
--    #214 DO UPDATE arm so a tampered or stale row heals to the migration text on replay; the
--    IS DISTINCT FROM guard keeps a converged replay write-free (no dead tuple, no validation
--    trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('identity.registration.asserted', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- 2. Retire the type: projection registrations FIRST, classification second (see the header).
--
--    Both DELETEs are needed even though db/005 no longer SEEDS either row, and this is the
--    whole reason this file exists rather than the seed edit alone: the loader replays every
--    migration on every connect, but an `INSERT ... ON CONFLICT DO NOTHING` can never REMOVE a
--    row an older build already wrote. A database migrated in place converges only because of
--    these two statements. Both are idempotent, so a fresh database runs them as no-ops.
DELETE FROM cairn_projection_apply WHERE event_type = 'patient.created';
DELETE FROM event_type_class       WHERE event_type = 'patient.created';

COMMIT;
