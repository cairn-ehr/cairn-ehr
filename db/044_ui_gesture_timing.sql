-- db/044 — node-local UI gesture timing, AGGREGATES ONLY (#288 / §1.2).
--
-- WHY THE COLUMNS THAT ARE NOT HERE MATTER MOST. Per-clinician gesture timings are a
-- productivity-surveillance dataset. There is deliberately NO user id, NO patient id, NO
-- per-sample row and NO timestamp: there is nothing to re-identify because the identifying
-- columns never exist. An anti-capture project must not ship a ready-made monitoring
-- substrate as a side effect of measuring a paper-parity benchmark. It is also a safety
-- hazard in its own right — clinicians who know they are timed rush the review step the
-- sign-off gesture exists to force.
--
-- These rows are NODE-LOCAL. They never sync, and they never touch the append-only signed
-- clinical event stream — mixing UI-tier data into the wire core is a category error
-- (principle 12), and here it would additionally turn a site metric into a person-level
-- record.
--
-- Replay-safe: CREATE TABLE IF NOT EXISTS, and the loader re-runs every db/*.sql on every
-- connect. Nothing here is widened later without a paired ALTER (see #207).
BEGIN;

CREATE TABLE IF NOT EXISTS ui_gesture_timing (
    gesture_kind TEXT   NOT NULL,
    size_bucket  TEXT   NOT NULL,
    n            BIGINT NOT NULL DEFAULT 0,
    p50_ms       INTEGER,
    p95_ms       INTEGER,
    PRIMARY KEY (gesture_kind, size_bucket),
    -- Closed vocabularies: an unrecognised kind or bucket is a bug in the caller, and a
    -- free-text column here would be an invitation to smuggle an identifier in.
    CONSTRAINT ui_gesture_timing_kind_ck   CHECK (gesture_kind IN ('signoff', 'cease')),
    CONSTRAINT ui_gesture_timing_bucket_ck CHECK (size_bucket IN ('1-3', '4-8', '9+')),
    CONSTRAINT ui_gesture_timing_n_ck      CHECK (n >= 0),
    CONSTRAINT ui_gesture_timing_p50_ck    CHECK (p50_ms IS NULL OR p50_ms >= 0),
    CONSTRAINT ui_gesture_timing_p95_ck    CHECK (p95_ms IS NULL OR p95_ms >= 0)
);

GRANT SELECT, INSERT, UPDATE ON ui_gesture_timing TO cairn_agent;

COMMIT;
