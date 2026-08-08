\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- #345 — the §5.3/§5.8 precedence rule's DB-side half, SQL mirror of
-- crates/cairn-node/tests/patient_precedence.rs.
--
-- The Rust suite exercises the RULE through the real `submit_event` door, which needs a signed
-- event and therefore a signing key this rig does not have. What is checkable here — and what
-- this file pins — is everything the rule stands on: the predicate's own truth table, and the
-- registry state db/047 converges to. Those are the parts a hand-edited database or a
-- half-replayed migration could silently get wrong.
--
-- Runs after the schema is loaded, inside a transaction that ROLLBACKs so it leaves no residue
-- (same idiom as db/tests/034).
BEGIN;

-- 1. `cairn_patient_has_events` is FALSE for a chart the log has never seen. This is the
--    direction that matters: if the predicate ever returned TRUE by default, the rule would be
--    permanently satisfied and the funnel silently bypassable again with nothing failing.
DO $$
DECLARE p uuid := gen_random_uuid();
BEGIN
    IF cairn_patient_has_events(p) THEN
        RAISE EXCEPTION 'FAIL: an unseen patient_id must have no events (the rule would never fire)';
    END IF;
END $$;

-- 2. ... and TRUE as soon as ANY event carries that patient_id — not only a registration. The
--    predicate answers "does this chart exist yet", never "is this chart registered": a chart
--    seeded by a peer's out-of-order event (the lenient remote door, ADR-0061 decision 3) is a
--    chart, and the next LOCAL write to it must not be refused for the wire's lack of ordering.
DO $$
DECLARE p uuid := gen_random_uuid();
BEGIN
    -- A deliberately UNPROJECTED event type. The predicate counts rows in event_log and
    -- knows nothing about types, so any row proves the point — and a type with no
    -- cairn_projection_apply row keeps this case about the predicate instead of dragging in
    -- whatever body shape some projection's apply fn would demand of a hand-written row.
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
        hlc_wall, hlc_counter, node_origin, signed_bytes, content_address, body,
        contributors, signer_key_id, plaintext_twin)
    VALUES (gen_random_uuid(), p, 'test.sql.precedence-probe', 'test/1',
        1, 0, 'peer', '\x4a'::bytea, '\x1220'::bytea || digest('\x4a'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'k', 'peer-seeded chart');
    IF NOT cairn_patient_has_events(p) THEN
        RAISE EXCEPTION 'FAIL: a chart with a peer-seeded event must count as existing';
    END IF;
END $$;

-- 3. The retirement converged: `patient.created` holds NO row in either registry. Checked in
--    both tables because the ORDER of db/047's two DELETEs is what keeps db/005's
--    registered-must-be-classified invariant true — a projection row surviving its class row is
--    precisely the state that invariant exists to make unreachable.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM event_type_class WHERE event_type = 'patient.created';
    IF n <> 0 THEN RAISE EXCEPTION 'FAIL: patient.created is still classified (% row(s))', n; END IF;
    SELECT count(*) INTO n FROM cairn_projection_apply WHERE event_type = 'patient.created';
    IF n <> 0 THEN RAISE EXCEPTION 'FAIL: patient.created still has % projection row(s)', n; END IF;
END $$;

-- 4. Registration carries BOTH apply fns: the retained-set projection it has owned since db/045,
--    and the chart-birth row it took over from the retired type. Losing the second is the silent
--    failure this case exists for — every chart-shaped read would simply stop seeing newly
--    registered charts, with no error anywhere.
DO $$
DECLARE fns text[];
BEGIN
    SELECT array_agg(apply_fn ORDER BY apply_fn) INTO fns
      FROM cairn_projection_apply
     WHERE event_type = 'identity.registration.asserted';
    IF fns IS DISTINCT FROM ARRAY['patient_chart_apply', 'patient_registration_apply'] THEN
        RAISE EXCEPTION 'FAIL: registration must register both apply fns, got %', fns;
    END IF;
END $$;

ROLLBACK;
