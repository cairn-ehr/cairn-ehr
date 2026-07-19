-- 034_medication_attestation.sql — slice 4 of the clinical.medication surface.
--
-- One additive verb: clinical.medication-attestation.asserted. A human takes
-- clinical responsibility (principle 10, ADR-0007) for one medication_id thread,
-- pinning a convergent commitment of the thread's content-event SET it reviewed.
-- Responsibility is enforced entirely by the db/005 attestation gate (the payload
-- carries a responsibility-bearing contributor -> the 3-arg door demands a valid
-- human token). This migration is purely structural floor + a set-commitment helper
-- + an overlay/projection (part 2). db/031, db/032, db/033 are UNTOUCHED, and the
-- current-list views are NOT widened (replay rule). See ADR-0049.
BEGIN;

-- 1. Register the verb (fail-closed registry, ADR-0010). Additive: an attestation
--    adds accountability and forecloses on nothing, so ADR-0043's owner-gate does
--    NOT apply and a clinician may vouch for a thread another author recorded.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-attestation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor. Culture-neutral, OFFLINE-FIRST (no check the thread exists
--    locally — set-union sync may deliver the attestation before the thread). The
--    twin requirement is enforced by the db/005 dispatcher via twin_required_msg
--    (step 3), NOT here. Mirrors cairn_check_medication_reconciliation.
CREATE OR REPLACE FUNCTION cairn_check_medication_attestation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication attestation: missing payload';
    END IF;
    -- M1 (issue #181): a responsibility-bearing contributor is what this event type
    -- EXISTS to carry (principle 10 — an attestation confers responsibility). Its
    -- absence is only reachable by a hostile/buggy raw-SQL client: such a body skips
    -- the db/005 attestation gate (v_bears is false, no token demanded, attester_key
    -- stays NULL) and, without this check, would fail only later at the part-2 apply
    -- trigger's `attester_kid TEXT NOT NULL` with a cryptic "null value violates
    -- not-null constraint". Reject it HERE, legibly, at the floor — the clean
    -- hostile-client rejection point (principle 12, mirroring db/026's blob-verify
    -- errors). The predicate mirrors the db/005 gate's own `e ? 'responsibility'`, so
    -- the floor and the gate agree on what "carries responsibility" means.
    --
    -- Be precise about what the type guard buys (door path vs. direct call): BOTH
    -- submit doors compute v_bears — EXISTS(SELECT 1 FROM
    -- jsonb_array_elements(b->'contributors') ...) — BEFORE this floor runs (db/005
    -- submit_event, db/020 apply_remote_event). So through a door, an array WITHOUT a
    -- responsibility contributor (and the absent/empty-array cases) make v_bears false
    -- and reach HERE for the legible rejection — but a *non-array* `contributors` is
    -- already rejected upstream at the v_bears line with a cryptic "cannot extract
    -- elements from a scalar" (a pre-existing, all-types legibility gap tracked in
    -- issue #184, NOT closed by this check). The `jsonb_typeof(...) IS DISTINCT FROM
    -- 'array'` guard is therefore defense-in-depth for a DIRECT caller of this check fn
    -- (a future door, or the floor_check_fn_directly_rejects_non_array_contributors
    -- test): the OR short-circuits so jsonb_array_elements never runs on a non-array,
    -- yielding this legible message instead of the scalar-extract error. The production
    -- Rust builder always includes the contributor, so no well-formed event is affected.
    IF jsonb_typeof(b -> 'contributors') IS DISTINCT FROM 'array'
       OR NOT EXISTS (
            SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
            WHERE e ? 'responsibility') THEN
        RAISE EXCEPTION 'medication attestation: requires a responsibility-bearing contributor (an attestation confers responsibility; ADR-0049/principle 10)';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: medication_id must be a valid uuid';
    END;
    IF jsonb_typeof(b -> 'patient_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (b ->> 'patient_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication attestation: patient_id must be a valid uuid';
    END;
    -- reviewed_commitment: a non-empty, EVEN-length hex string (the pinned set
    -- commitment). Even length is required because the part-2 apply trigger
    -- decode()s this value as bytea ('hex') — an odd-length string makes decode()
    -- raise a cryptic low-level error instead of a legible floor rejection. Reject
    -- it HERE, at the floor (the clean hostile-client rejection point, principle 12),
    -- not in the trigger (Task-3 review Minor).
    IF jsonb_typeof(p -> 'reviewed_commitment') IS DISTINCT FROM 'string'
       OR (p ->> 'reviewed_commitment') !~ '^[0-9a-fA-F]+$'
       OR length(p ->> 'reviewed_commitment') % 2 <> 0 THEN
        RAISE EXCEPTION 'medication attestation: reviewed_commitment must be a non-empty even-length hex string';
    END IF;
    -- reviewed_count: a non-negative integer legibility hint.
    IF jsonb_typeof(p -> 'reviewed_count') IS DISTINCT FROM 'number'
       OR (p ->> 'reviewed_count')::numeric < 0
       OR (p ->> 'reviewed_count')::numeric <> floor((p ->> 'reviewed_count')::numeric) THEN
        RAISE EXCEPTION 'medication attestation: reviewed_count must be a non-negative integer';
    END IF;
END;
$$;

-- 3. Register the verb's floor + HARD twin requirement in the #173/ADR-0048 registry
--    (the single db/005 dispatcher reads these rows). Placed AFTER the check fn above
--    so the fail-closed registry trigger sees cairn_check_medication_attestation(text,
--    jsonb) declared at load time (an implementer catch from #173: registry INSERT must
--    follow the CREATE, or a fresh load rolls back).
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-attestation.asserted', 'cairn_check_medication_attestation',
     'medication attestation requires a non-empty authored twin (§3.13/§3.3)')
-- DO UPDATE, not DO NOTHING (#214): replay must converge the row to the migration text
-- (see db/031's medication registration for the rationale).
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg;

-- 4. The set-commitment SINGLE SOURCE. Sorted-concat-hash of the thread's content-event
--    content_addresses (byte order -> order-independent, collation-free; mirrors
--    event_set_commitment in medium.rs). Called at BOTH author time (the orchestrator
--    pins this value) and read time (the staleness view recomputes it) -> byte-identity
--    guaranteed, no Rust<->SQL drift. NULL when the thread has no local content events
--    (orphan): the orchestrator bails, the projection reads NULL -> stale. Content
--    events EXCLUDE reconciliation/separation/attestation (not thread content).
CREATE OR REPLACE FUNCTION cairn_medication_thread_commitment(p_medication_id uuid)
RETURNS bytea LANGUAGE sql STABLE AS $$
    -- ADR-0052: a sealed content event carries CIPHERTEXT in event_log.body, so its
    -- medication_id lives in the event_clear shadow — the thread lookup must read the
    -- CLEAR payload via cairn_clear_payload (unsealed rows resolve to body unchanged,
    -- the same sealed-aware read the projection triggers use). content_address is on
    -- event_log for EVERY row (it hashes the signed bytes, sealed or not).
    --
    -- CAUTION — ADR-0049 FALSE-FRESH hazard (Tasks 8/9 MUST close this): routing the
    -- thread filter through cairn_clear_payload makes this commitment a function of
    -- CUSTODY, not of the event SET. A node that holds an attestation's custody but is
    -- MISSING a later content event's custody recomputes H({ca1}) for a thread that has
    -- actually grown to {ca1, ca2} -> the staleness view reads stale = FALSE and shows the
    -- thread FRESH when it in fact grew. That is a FALSE-FRESH — the exact unsafe direction
    -- ADR-0049 exists to forbid: staleness-signal CORRUPTION, NOT honest degradation.
    -- UNREACHABLE today (the apply door does not populate custody yet), but Tasks 8/9 MUST
    -- gate the staleness view to force stale/unknown whenever the readable content-event
    -- count is LESS THAN the attestation's reviewed_count. See ADR-0052 (+ the issue to be
    -- filed for the sealed-aware staleness gate).
    SELECT CASE WHEN count(*) = 0 THEN NULL
                ELSE digest(string_agg(el.content_address, ''::bytea ORDER BY el.content_address), 'sha256')
           END
    FROM event_log el
    WHERE el.event_type IN (
            'clinical.medication.asserted',
            'clinical.medication-cessation.asserted',
            'clinical.medication-dose-change.asserted',
            'clinical.medication-dose-correction.asserted')
      AND (cairn_clear_payload(el) ->> 'medication_id')::uuid = p_medication_id;
$$;

-- 4a. The READABLE content-event count for a thread — the ADR-0049 FALSE-FRESH gate's
--     tripwire (Task 8, issues #189/#92). Deliberately the SAME filter as the commitment
--     fn above (four content types, read through cairn_clear_payload) and byte-identical
--     to how the author-time orchestrator sizes reviewed_count
--     (crates/cairn-node/src/medication/attestation.rs::thread_commitment_on) — so on a
--     FULL-custody node readable_count == reviewed_count and the gate below never fires
--     spuriously; on a PARTIAL-custody node (a sealed content event synced in WITHOUT its
--     DEK is invisible to cairn_clear_payload) it reads SHORT.
--
--     Task 8 is the task that first populates custody on the sync path (db/020's sealed
--     arm), so the false-fresh hazard cairn_medication_thread_commitment warns about goes
--     LIVE here: a node that cannot reproduce the reviewed set must never read a grown
--     thread as "fresh". The staleness view (part 2) forces stale whenever this count is
--     LESS than the attestation's reviewed_count.
CREATE OR REPLACE FUNCTION cairn_medication_thread_readable_count(p_medication_id uuid)
RETURNS bigint LANGUAGE sql STABLE AS $$
    SELECT count(*)
    FROM event_log el
    WHERE el.event_type IN (
            'clinical.medication.asserted',
            'clinical.medication-cessation.asserted',
            'clinical.medication-dose-change.asserted',
            'clinical.medication-dose-correction.asserted')
      AND (cairn_clear_payload(el) ->> 'medication_id')::uuid = p_medication_id;
$$;

-- 4b. Read-time support for the set-commitment fn. Unlike the other medication read
--     views (which read trigger-maintained projection TABLES), the commitment is
--     re-derived straight from event_log at BOTH author and read time — that direct
--     read is deliberate (one source, no Rust<->SQL drift), but the thread filter
--     `(body ->> 'medication_id')::uuid = $1` lands on a jsonb expression no existing
--     event_log index covers. The staleness view calls the fn once per attested
--     thread, so without this the projection scans event_log per thread. A PARTIAL
--     functional index (only the four content-event types the fn sums) makes each
--     commitment a bounded index lookup. `(body ->> 'medication_id')` is immutable and
--     the ::uuid cast is immutable, so the expression is indexable.
--     ADR-0052 caveat: the commitment fn now reads the thread key through
--     cairn_clear_payload (sealed rows carry ciphertext in body), so this index only
--     accelerates the UNSEALED path; sealed-thread commitment is a small scan for now
--     (pre-clinical, acceptable — a sealed-aware index on the event_clear shadow is the
--     follow-up if the staleness view ever becomes a hot path).
CREATE INDEX IF NOT EXISTS event_log_medication_thread_idx
    ON event_log (((body ->> 'medication_id')::uuid))
    WHERE event_type IN (
        'clinical.medication.asserted',
        'clinical.medication-cessation.asserted',
        'clinical.medication-dose-change.asserted',
        'clinical.medication-dose-correction.asserted');

COMMIT;

BEGIN;

-- 5. The attestation overlay: one row per attestation event (append-only; every
--    vouch retained for audit). attester_kid is the VERIFIED responsible human,
--    read from event_log.attester_key (the db/005 gate stored it after checking the
--    token + kind='human'). reviewed_commitment stored as bytea for a direct compare.
CREATE TABLE IF NOT EXISTS medication_attestation (
    event_id            UUID PRIMARY KEY,       -- the attestation event's own id
    medication_id       UUID NOT NULL,
    patient_id          UUID NOT NULL,
    attester_kid        TEXT NOT NULL,          -- hex of the verified human attester key
    reviewed_commitment BYTEA NOT NULL,
    reviewed_count      INTEGER NOT NULL,
    hlc_wall            BIGINT NOT NULL,
    hlc_counter         INTEGER NOT NULL,
    origin              TEXT NOT NULL,
    content_address     BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_attestation TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_attestation_thread_idx
    ON medication_attestation (medication_id);

-- 6. Apply trigger: fold each attestation event into the overlay (door-agnostic —
--    fires for both the local submit door and the db/020 remote-apply door). Append
--    a row keyed by the event's own id; a re-delivered event is deduped by the PK.
CREATE OR REPLACE FUNCTION medication_attestation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p jsonb := cairn_clear_payload(NEW);
BEGIN
    IF p IS NULL THEN RETURN NULL; END IF;
    INSERT INTO medication_attestation
        (event_id, medication_id, patient_id, attester_kid, reviewed_commitment,
         reviewed_count, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id,
        (p ->> 'medication_id')::uuid,
        NEW.patient_id,
        -- attester_kid is read from attester_key (the VERIFIED responsible human the
        -- db/005 gate stored), NOT signer_key_id. This is deliberate: responsibility is
        -- SEPARABLE from authorship (principle 10 / ADR-0007) — signature proves origin,
        -- attestation confers responsibility. Today the two coincide because the
        -- attestation orchestrator (attestation.rs) has the human key both sign the event
        -- AND mint the token, so every test uses signer == attester; but a future flow
        -- could sign with one key and vouch with another, and this projection must key on
        -- WHO took responsibility (attester_key), never who happened to sign. INVARIANT:
        -- attester_key is guaranteed non-NULL here — a `-attestation.asserted` event
        -- always carries a responsibility contributor (enforced by the M1 floor check
        -- above), which trips the db/005 gate that populates attester_key; the M1 check
        -- turns a would-be NULL into a legible floor rejection long before this trigger.
        encode(NEW.attester_key, 'hex'),
        decode(p ->> 'reviewed_commitment', 'hex'),
        (p ->> 'reviewed_count')::integer,
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (event_id) DO NOTHING;                    -- append-only, idempotent
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_attestation_apply_trg ON event_log;
CREATE TRIGGER medication_attestation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-attestation.asserted')
    EXECUTE FUNCTION medication_attestation_apply();

-- 7. Per-thread standing attestation: the LATEST vouch per thread (by its own
--    convergent position), with the staleness verdict = "the thread's current
--    content-set commitment differs from what this vouch reviewed". Because thread
--    content is append-only (grow-only), ANY later content event (higher OR lower
--    HLC) changes the commitment -> stale. content_address is bytea -> byte-order
--    tiebreak needs no COLLATE.
--
--    ADR-0049 FALSE-FRESH gate (Task 8, issues #189/#92): the commitment above is
--    computed through cairn_clear_payload, so it is CUSTODY-dependent, not event-set-
--    dependent. On a PARTIAL-custody node (a sealed content event synced WITHOUT its DEK
--    is unreadable, hence uncountable and un-hashable for its thread) the recomputed
--    commitment can coincidentally match what the vouch pinned even though the attester
--    reviewed MORE — reading a grown thread as FALSE-FRESH, the exact unsafe direction
--    ADR-0049 forbids. The FIRST disjunct closes that: when this node cannot even
--    reproduce the reviewed set (its readable content count < reviewed_count), the vouch
--    is forced stale/unknown — never "current" — regardless of whether the partial
--    commitment happens to match. Safe direction: uncertainty can only WITHHOLD "fresh".
--    (Residual, tracked as follow-up: when readable_count == reviewed_count but the thread
--    grew via a content event this node cannot attribute to the thread at all — a sealed-
--    no-custody row — the tripwire cannot see it; closing that needs thread-membership
--    metadata that survives custody loss, a separate design decision. See
--    cairn_medication_thread_commitment's CAUTION note.)
CREATE OR REPLACE VIEW medication_thread_attestation AS
SELECT DISTINCT ON (a.medication_id)
       a.medication_id,
       a.patient_id,
       a.attester_kid,
       a.hlc_wall     AS attested_wall,
       a.hlc_counter  AS attested_counter,
       a.reviewed_count,
       (cairn_medication_thread_readable_count(a.medication_id) < a.reviewed_count
        OR cairn_medication_thread_commitment(a.medication_id) IS DISTINCT FROM a.reviewed_commitment)
           AS stale
FROM medication_attestation a
ORDER BY a.medication_id, a.hlc_wall DESC, a.hlc_counter DESC, a.content_address DESC;
GRANT SELECT ON medication_thread_attestation TO cairn_agent;

-- 8. Group rollup (conservative): a reconciled group is "attested & current" iff
--    EVERY member thread has a non-stale attestation. Singletons (group_id =
--    medication_id) reduce trivially to their thread. medication_thread_group (db/033)
--    lists every locally-asserted thread with its group_id, so an orphan attestation
--    (no local assert) is simply not a member -> renders nothing until it arrives.
CREATE OR REPLACE VIEW medication_group_attestation AS
SELECT g.group_id,
       g.patient_id,
       bool_and(ta.medication_id IS NOT NULL AND NOT ta.stale)      AS attested_current,
       count(*) FILTER (WHERE ta.medication_id IS NULL)             AS unattested_members,
       count(*) FILTER (WHERE ta.stale)                             AS stale_members
FROM medication_thread_group g
LEFT JOIN medication_thread_attestation ta ON ta.medication_id = g.medication_id
GROUP BY g.group_id, g.patient_id;
GRANT SELECT ON medication_group_attestation TO cairn_agent;

COMMIT;
