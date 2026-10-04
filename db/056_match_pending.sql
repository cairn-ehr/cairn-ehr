-- db/056_match_pending.sql
-- Repair path R4 (#679, ADR-0076 decision 7): the commit-time duplicate check's queue.
--
-- WHAT: whenever one of the advisory §5.2 matcher's INPUTS changes for a chart (a name, a DOB or
-- sex, an identifier, the §5.4 identity state, a repudiated name, or a new chart), a NOTICE row
-- is appended here. The node's matcher worker (`cairn-matcher watch`, matcher/) drains the
-- notices: it checks each chart against the population and writes match_proposal rows (db/017).
-- It NEVER links — a hit is a proposal a human resolves.
--
-- WHY APPEND-ONLY NOTICES, NOT ONE ROW PER PATIENT (design page, "R4 — designed 2026-10-04"):
--   * the hook below is a plain INSERT of a fresh bigserial key. It cannot conflict with
--     anything, so it never waits on the worker's open transaction and never raises a
--     serialization error, whatever isolation level a future writer picks. A keyed upsert
--     (ON CONFLICT) would make a clinical write wait behind the worker.
--   * the worker deletes exactly the notice ids it READ (captured before it reads the
--     projections, deleted by id, never by "id <=": a bigserial is assigned at INSERT, not
--     commit, so a slow transaction's lower id can commit mid-check), so a change landing
--     while the chart is being checked survives and is checked again. Deleting "the patient's row" would lose it.
--
-- WHY IT CAN NEVER FAIL A CLINICAL WRITE: structurally, like db/029's collision recorder — no
-- RAISE, no EXCEPTION block, a null guard, an insert that cannot conflict. It is deliberately
-- NOT wrapped in EXCEPTION WHEN OTHERS: a swallowed failure is a silently skipped check. What
-- can still raise (a full disk, a dropped table) would fail the clinical write anyway.
-- Guarded by crates/cairn-node/tests/match_pending.rs::the_hook_has_no_raising_path.
--
-- WHICH WRITES QUEUE A CHECK: the projection upserts are conditional (DO UPDATE ... WHERE new >
-- old), and a row trigger fires only on a row that actually changes. So normal use queues one
-- notice per real change, an upgrade heal queues only charts whose winner changed, and a
-- `reproject --rebuild`, a restore or a new node's first pull queues every chart (correct: the
-- inputs were rewritten; the worker switches to one sweep for a large backlog).

CREATE TABLE IF NOT EXISTS match_pending (
    id         BIGSERIAL   PRIMARY KEY,
    patient_id UUID        NOT NULL,
    -- 'change': an input changed. 'config': the worker's matcher_version changed (or it ran for
    -- the first time), so every chart is re-checked. Labels the status; does not order the drain.
    reason     TEXT        NOT NULL CHECK (reason IN ('change', 'config')),
    queued_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- The worker's two reads: "the newest notices, grouped by chart" and "the ids of this chart's
-- notices" (it deletes exactly those ids afterwards).
CREATE INDEX IF NOT EXISTS match_pending_patient_idx ON match_pending (patient_id, id);

-- One row, written only by the worker: the matcher_version it last ran (a change re-queues every
-- chart) and last_drained_at, the worker's PROGRESS stamp — written after each chart it checks,
-- at the start of a sweep and (throttled) during one, and after an empty round. It is both the
-- status line's "last ran HH:MM" (last active) and the "no progress" half of the stalled rule
-- (cairn_duplicate_check_status). Its ABSENCE means no worker has ever run on this node — see
-- cairn_chart_check_pending.
CREATE TABLE IF NOT EXISTS match_worker_state (
    singleton       BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    matcher_version TEXT        NOT NULL,
    last_drained_at TIMESTAMPTZ
);

-- The hook. TG_ARGV[0] names the chart-id column (patient_id, or subject on the two identity
-- tables). SECURITY DEFINER so every writer — the submit door, the sync door, a restore, a
-- rebuild, a test seeding rows directly — can append a notice without its own grant.
CREATE OR REPLACE FUNCTION cairn_match_enqueue()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    INSERT INTO match_pending (patient_id, reason)
    SELECT (to_jsonb(NEW) ->> TG_ARGV[0])::uuid, 'change'
    WHERE to_jsonb(NEW) ->> TG_ARGV[0] IS NOT NULL;
    -- One wake-up per transaction: NOTIFY collapses identical payloads within a transaction, so
    -- a rebuild that writes 500k rows wakes the worker once, at commit.
    PERFORM pg_notify('cairn_match_pending', '');
    RETURN NULL;
END $$;
REVOKE EXECUTE ON FUNCTION cairn_match_enqueue() FROM PUBLIC;

-- The matcher's inputs. patient_chart is INSERT-ONLY: db/002 updates that row on EVERY clinical
-- event (last_activity, note_count), so an UPDATE hook there would queue a duplicate check on
-- every medication write. A new chart is the only patient_chart event the matcher cares about.
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT ON patient_chart
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_name
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_demographic
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_identifier
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON chart_identity_state
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('subject');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON name_repudiation
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('subject');

-- Has THIS chart been checked since its identity evidence last changed? FALSE only when the
-- worker has run at least once AND no notice for the chart is waiting. Before the first worker
-- run every chart is pending: charts created before db/056 have no notices, and saying
-- "checked" about them would be a false claim (principle 4). R5's banner reads this.
CREATE OR REPLACE FUNCTION cairn_chart_check_pending(p_patient uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (SELECT 1 FROM match_pending WHERE patient_id = p_patient)
        OR NOT EXISTS (SELECT 1 FROM match_worker_state)
$$;

-- The node-wide status (cairn-node `duplicate-check`; R5's front door later).
--
-- quiet_age_s: seconds since ANYTHING last happened while charts wait — the later of the newest
-- waiting notice (a change arrived) and the worker's last progress stamp (last_drained_at, written
-- after each chart it checks and, throttled, during a sweep). NULL when nothing waits.
-- GREATEST ignores a NULL argument, so a worker that has never stamped progress falls back to the
-- newest notice's age. The node calls the check "behind" only when charts wait AND this quiet time
-- passes its threshold: no new change AND no progress means the worker is stopped or stuck.
--
-- WHY NOT "THE NEWEST NOTICE IS OLD" (the earlier rule, ruling R13): a restore, a `reproject
-- --rebuild` or a matcher-version re-check queues every chart ALL AT ONCE, so every notice has
-- about the same queued_at. Five minutes later even the newest notice is old while a healthy worker
-- is still working through the backlog (~9 s a chart at 10 000 records: 500 charts is over an
-- hour), and the line would read "behind" the whole time — training staff to ignore it.
-- VOLATILE (the default): it reads clock_timestamp().
--
-- The OUT columns changed (newest_age_s -> quiet_age_s), and CREATE OR REPLACE cannot change a
-- function's return type. This heals a database that loaded this file's pre-merge shape; it is a
-- no-op everywhere else (the old column name is the guard).
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_proc WHERE proname = 'cairn_duplicate_check_status'
               AND 'newest_age_s' = ANY (proargnames)) THEN
        DROP FUNCTION cairn_duplicate_check_status();
    END IF;
END $$;
CREATE OR REPLACE FUNCTION cairn_duplicate_check_status(
    OUT charts_waiting    bigint,
    OUT quiet_age_s       bigint,
    OUT config_recheck    boolean,
    OUT worker_seen       boolean,
    OUT last_drained_hhmm text)
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT
        (SELECT count(DISTINCT patient_id) FROM match_pending),
        (SELECT floor(extract(epoch FROM clock_timestamp() - GREATEST(
                    max(p.queued_at),
                    (SELECT last_drained_at FROM match_worker_state))))::bigint
           FROM match_pending p
          HAVING count(*) > 0),
        EXISTS (SELECT 1 FROM match_pending WHERE reason = 'config'),
        EXISTS (SELECT 1 FROM match_worker_state),
        (SELECT to_char(last_drained_at, 'HH24:MI') FROM match_worker_state)
$$;

-- The worker (cairn_agent) reads and clears notices, queues a 'config' re-check, and keeps its
-- state row. The node's runtime role only reads.
GRANT SELECT, INSERT, DELETE ON match_pending TO cairn_agent;
GRANT USAGE, SELECT ON SEQUENCE match_pending_id_seq TO cairn_agent;
GRANT SELECT, INSERT, UPDATE ON match_worker_state TO cairn_agent;
GRANT SELECT ON match_pending, match_worker_state TO cairn_node;
GRANT EXECUTE ON FUNCTION cairn_chart_check_pending(uuid) TO cairn_agent, cairn_node;
GRANT EXECUTE ON FUNCTION cairn_duplicate_check_status() TO cairn_agent, cairn_node;
