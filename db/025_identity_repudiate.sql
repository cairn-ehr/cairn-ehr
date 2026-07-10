-- db/025_identity_repudiate.sql
-- Cairn — §5.5(a)/§5.7 `repudiate` + the known-alias pool (identity piece C5).
--
-- WHAT: the FIRST *suppressing* identity event. C1–C4 were all additive/annotative
-- (link adds an edge; dispute/identify annotate the trust state) — none removes anything
-- from a projection. `repudiate` strikes a known-false name from the display winner: the
-- §5.5(a) fabricated-persona case (a patient presented under a deliberately false name;
-- once established false, the name leaves the header but stays in the record — the fact of
-- presentation under it is medico-legally required — and enters a matcher-visible alias pool).
--
-- MECHANISM (digital strike-through, principle 1 + 2): the name assertion event and
-- db/012's retained-set row (patient_name) are LEFT UNTOUCHED. A separate overlay
-- (name_repudiation) records the struck value; the display winner (patient_name_current)
-- is CREATE-OR-REPLACEd to anti-join it; the matcher reads the struck names via a new
-- patient_alias_pool view. Nothing is erased — the winner just excludes it.
--
-- Additive DDL only; no submit_event re-declaration, no floor change to the gate, no
-- SCHEMA-version bump. db/010–024 are left UNTOUCHED — this migration CREATE-OR-REPLACEs
-- the shared cairn_event_twin hook (adding one branch) and patient_name_current (adding an
-- anti-join, same column contract), the established later-migration pattern.

BEGIN;

-- 1. Register the repudiate event type as *suppressing* (ADR-0010; fail-closed registry).
--    suppressing is load-bearing here: the db/005 attestation gate (step 4) forces a valid
--    attestation token from an enrolled HUMAN on any suppressing event — so every
--    repudiation structurally requires a responsibility-bearing human to vouch, with NO
--    floor special-case. This is the deliberate contrast with the additive C1/C3/C4 events
--    (whose "human vouches" bit only when a responsibility contributor was named): a
--    repudiation removes clinical display content, §5.7 marks it "Human", and
--    suppressing-mode makes that unbypassable in the DB (principle 12). It reuses the exact
--    gate that already guards salience.downgrade / visibility.suppress.
--
--    targets_other_author=FALSE: a repudiation is VALUE-grained — it names the known-false
--    `value`, not a `target_event_id` — so the step-5 target-existence gate (which only
--    fires when the payload carries target_event_id) is a no-op and correctly stays off.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.repudiate.asserted', 'suppressing', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The §5.7 structural floor. Culture-neutral: validates STRUCTURE only — a valid subject
--    uuid, a non-empty known-false `value`, and a non-empty `reason` (why known-false,
--    §4.1 value-open — "unknown" is honest, "" is fabrication-only). Each violation is a
--    distinct legible exception (the cairn_check_dispute/identity_state pattern).
CREATE OR REPLACE FUNCTION cairn_check_repudiation_assertion(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'repudiation assertion: missing payload';
    END IF;
    -- subject: present, string, valid uuid (the chart that carried the false name). No
    -- cross-existence check — a repudiation may arrive before or independently of the chart
    -- it names (offline-first, set-union), like the C3/C4 identity overlays.
    IF jsonb_typeof(p -> 'subject') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'repudiation assertion: subject must be a uuid string (§5.7)';
    END IF;
    BEGIN
        PERFORM (p ->> 'subject')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'repudiation assertion: subject must be a valid uuid (§5.7)';
    END;
    -- value: the exact known-false name string. Present + non-empty — a repudiation that
    -- names no value would strike nothing (or, worse, be ambiguous). Opaque to the core
    -- (any script/culture); it is matched to the retained set by exact equality (§5.5(a)).
    --
    -- DELIBERATELY NO existence check that `value` matches a live patient_name member: a
    -- repudiation may legitimately arrive BEFORE the name assertion it strikes (offline-first,
    -- out-of-order set-union — the same reason the C3/C4 floors accept dispute/pending before
    -- the chart exists). A hard reject would break that convergence; the overlay simply lands
    -- and suppresses the member if/when it arrives. The residual footgun — a mistyped /
    -- trailing-spaced value that matches nothing, so the false name keeps displaying with no
    -- error — is a UI-layer responsibility (pre-fill the exact value from the chart's name
    -- list; confirm the header actually changed), NOT the floor's: the floor cannot enforce
    -- existence without sacrificing offline-first, and must stay precise (a fuzzy match could
    -- strike the WRONG, possibly true, name).
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'repudiation assertion: value must be a non-empty string (§5.5a)';
    END IF;
    -- reason: present + non-empty (§4.1 ladder; value-open).
    IF jsonb_typeof(p -> 'reason') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'reason')) = 0 THEN
        RAISE EXCEPTION 'repudiation assertion: reason must be a non-empty string (§4.1)';
    END IF;
END;
$$;

-- 3. Extend the per-type twin hook. Repudiation: run the floor + HARD-require an authored
--    twin (identity events are legible-critical, like link/dispute/identify/demographics).
--    This CREATE OR REPLACE PRESERVES every existing branch (db/010 demographics, db/018
--    link, db/023 dispute, db/024 identity-state, db/015 honest-degrade fallback) and only
--    ADDS the repudiate branch — submit_event itself is NEVER re-declared. Floor call +
--    require-twin flag are set together in the one branch (the C4 desync-proof ladder).
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin          text := b ->> 'plaintext_twin';
    v_authored      boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_twin_required text := NULL;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type IN ('identity.link.asserted', 'identity.unlink.asserted') THEN
        PERFORM cairn_check_link_assertion(b);
        v_twin_required := 'identity linkage assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.dispute.asserted', 'identity.dispute.resolved') THEN
        PERFORM cairn_check_dispute_assertion(p_type, b);
        v_twin_required := 'identity dispute assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.pending.asserted', 'identity.identify.asserted') THEN
        PERFORM cairn_check_identity_state_assertion(p_type, b);
        v_twin_required := 'identity-state assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type = 'identity.repudiate.asserted' THEN
        PERFORM cairn_check_repudiation_assertion(b);
        v_twin_required := 'identity repudiation assertion requires a non-empty authored twin (§5.7)';
    END IF;

    IF v_authored THEN
        RETURN v_twin;
    END IF;
    IF v_twin_required IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_twin_required;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

-- 4. name_repudiation: the standing strike-through overlay. Keyed by (subject, value) —
--    VALUE-grained, not (subject, use_key, value) and not a target event id:
--      * a fabricated name is false HOWEVER it was labelled (one false value, not one per
--        `use`), AND
--      * keying on the raw opaque `value` (db/012 stores it verbatim as p->>'value'; this
--        overlay stores it verbatim too) makes the display anti-join a plain exact-string
--        equality with NO `use` fold on either side — nothing to drift out of sync with
--        db/012's lower(… COLLATE "C") key-fold. (A use-grained key would have to replicate
--        that fold bug-for-bug forever or silently fail to suppress.)
--    HLC-latest-wins so a re-assert is idempotent and a FUTURE reversal event composes in by
--    HLC with no rewrite (not built this slice; the append-only correction path is separate).
CREATE TABLE IF NOT EXISTS name_repudiation (
    subject     UUID    NOT NULL,
    value       TEXT    NOT NULL,   -- the exact known-false name string (opaque; struck from display, kept as alias)
    reason      TEXT    NOT NULL,   -- why known-false (the floor guarantees non-empty; NOT NULL self-documents that)
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    content_address BYTEA NOT NULL,   -- winning event's content address; the #115 tiebreak
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (subject, value)
);
-- NB: no broad GRANT on this base table. `reason` is free-text forensic context ("confessed
-- fabricated persona to evade a warrant") that must NOT be exposed cross-patient on a
-- name-searchable surface (ADR-0006: confidentiality lives in visibility/key-custody, not on
-- a widely-readable view). The agent role reads the *reason-free* patient_alias_pool view
-- below (a PG view runs with its owner's table privileges, so cairn_agent needs no direct
-- grant here); `reason` stays confined to privileged/audit + a future per-chart chart-history
-- read surface.
--
-- The PK (subject, value) serves the display anti-join (patient_name_current probes by the
-- full (subject, value) pair — leading-column `subject` is bound). But the §5.2 matcher's
-- own access pattern over patient_alias_pool is a CROSS-patient lookup keyed on `value`
-- alone ("who has ever presented under this alias?" — SELECT patient_id … WHERE value = ?),
-- which the subject-leading PK cannot serve, so it would seq-scan a fleet-wide alias pool
-- that grows with every repudiation ever synced. Index `value` so that lookup stays a
-- probe, mirroring db/024's proactive worklist index. (The matcher wiring that runs this
-- query is deferred to a later slice; the index is cheap and pre-empts the cliff.)
CREATE INDEX IF NOT EXISTS name_repudiation_value_idx ON name_repudiation (value);

-- Incremental maintenance: fold exactly the one new repudiation into the overlay. The row
-- overlays atomically only when the incoming HLC is strictly greater (ON CONFLICT … WHERE),
-- so out-of-order arrival / re-assert converges to the highest-HLC assertion,
-- arrival-order-independent (the db/024 chart_identity_state shape).
CREATE OR REPLACE FUNCTION name_repudiation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
    v_cur record;
BEGIN
    -- #157: detect a Byzantine HLC-triple collision against the current repudiation of this exact
    -- (subject, value) and record an advisory signal before overlaying the winning `reason`. Note
    -- the SUPPRESSION decision itself is value-keyed + idempotent (see the #69 note below), so this
    -- only ever surfaces which advisory `reason` the collision resolved to — never un-suppresses.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM name_repudiation
      WHERE subject = (p ->> 'subject')::uuid AND value = p ->> 'value';
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'name_repudiation', (p ->> 'subject') || '|' || (p ->> 'value'),
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;

    INSERT INTO name_repudiation
        (subject, value, reason, hlc_wall, hlc_counter, origin, content_address)
    VALUES
        ((p ->> 'subject')::uuid, p ->> 'value', p ->> 'reason',
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (subject, value) DO UPDATE SET
        reason      = EXCLUDED.reason,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    -- content_address is the deterministic final tiebreaker (#115): a Byzantine (wall,counter,
    -- origin) collision now converges to the same winner on every node. The remaining TEXT
    -- collation-sensitivity of the intermediate `origin` comparison stays tracked by #69 (a
    -- cross-collation tie can only pick a different advisory `reason`, never un-suppress a
    -- name, since the SUPPRESSION decision itself is value-keyed + idempotent, independent of
    -- this tuple).
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        name_repudiation.hlc_wall, name_repudiation.hlc_counter, name_repudiation.origin,
        name_repudiation.content_address);
    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS name_repudiation_apply_trg ON event_log;
CREATE TRIGGER name_repudiation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'identity.repudiate.asserted')
    EXECUTE FUNCTION name_repudiation_apply();

-- 5. patient_name_current: rework db/012's display winner to ANTI-JOIN the overlay. Column
--    list + ORDER BY are copied VERBATIM from db/012 (including its ADR-0045 (#69) COLLATE
--    "C" tiebreak fix — this CREATE OR REPLACE runs AFTER db/012's in schema load order, so
--    it must carry the same collation pins or it would silently re-introduce the collation-
--    dependent divergence db/012 just closed) so the CREATE OR REPLACE keeps the exact
--    column contract (reload-idempotent across connect_and_load_schema; any dependent
--    stays valid); the ONLY change is the NOT EXISTS filter excluding repudiated members.
--    DRIFT GUARD (#159): because this copy is live (loads after db/012) and its ORDER BY is a
--    verbatim duplicate, the no-DB test crates/cairn-node/tests/name_winner_order_drift.rs asserts
--    the two clauses are byte-identical — edit BOTH together or the build fails. Nothing in SQL can
--    enforce it (DISTINCT ON forces a per-view ORDER BY, and the anti-join must run BEFORE the
--    winner is picked, so the ordering can't be factored into a shared base view).
--    The anti-join is deliberately HLC-BLIND: a standing repudiation strikes its (subject,
--    value) regardless of any name assertion's HLC — INCLUDING a strictly-newer re-assertion
--    of the same string. This is the safety-preserving choice: re-typing a known-false name
--    (e.g. a clerk re-registering from the same old insurance card that bore the fabrication)
--    must NOT silently resurrect it in the header. A repudiation made in error is undone by an
--    explicit reversal event (append-only correction path — deferred this slice; the overlay
--    is HLC-versioned so it composes in with no rewrite), never by re-asserting the value.
--    A struck name is removed from the winner, so DISTINCT ON picks the next surviving name.
--    If a chart's ONLY name is repudiated, it has NO winner row — the honest outcome: the
--    name is genuinely unknown-now, and showing the known-false one would be a precise
--    untruth (principle 4). "Header shows something" is then satisfied one layer up by the
--    §5.4 callsign / *unconfirmed* rendering (C4), not by lying in the header.
CREATE OR REPLACE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, use_key, value, use_raw, provenance, provenance_rank,
    last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_name
WHERE NOT EXISTS (
    SELECT 1 FROM name_repudiation r
    WHERE r.subject = patient_name.patient_id
      AND r.value   = patient_name.value)
ORDER BY patient_id,
         (use_key = 'legal') DESC,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin COLLATE "C" DESC,
         use_key COLLATE "C" DESC, value COLLATE "C" DESC;

-- 6. patient_alias_pool: the §5.2 matcher's known-alias pool — the struck names, retained
--    and reusable (a fabricated persona returns under the same false name). Grain is
--    (patient_id, value); the matcher looks up "who has used this presenting name as a
--    known alias?" (SELECT patient_id … WHERE value = ?). Fuzzy recognition of a returning
--    alias is the ADVISORY matcher's job over this view — never the suppression floor's.
--    DELIBERATELY REASON-FREE: the matcher only needs (patient_id, value) to reuse an alias,
--    and this view is name-searchable ACROSS patients — surfacing another chart's forensic
--    `reason` here would leak sensitive free-text to the agent role (ADR-0006). `reason`
--    stays in the base overlay for privileged/chart-history reads only.
CREATE OR REPLACE VIEW patient_alias_pool AS
SELECT subject AS patient_id, value, hlc_wall, hlc_counter, origin, updated_at
FROM name_repudiation;

GRANT SELECT ON patient_name_current, patient_alias_pool TO cairn_agent;

COMMIT;
