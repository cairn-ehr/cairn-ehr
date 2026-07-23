-- Cairn walking skeleton — the validated submit surface (Spike 0002 §4.4 / ADR-0022).
--
-- submit_event is the ONE generic write door. It runs the write-time seams in-DB,
-- atomically: verify (cairn_pgx) -> resolve actor -> classify additive/suppressing
-- -> gate attestation -> owner-gate cross-author overlays -> bind provenance ->
-- append. The grant floor (REVOKE INSERT on event_log; GRANT EXECUTE here) makes
-- direct DB access safe by construction (ADR-0021). Every rejection is legible.

BEGIN;

-- Additive vs suppressing classification (ADR-0010). A new event type adds a row
-- here (additive-only registry); unknown types are rejected (fail closed).
CREATE TABLE IF NOT EXISTS event_type_class (
    event_type            TEXT PRIMARY KEY,
    mode                  TEXT NOT NULL CHECK (mode IN ('additive','suppressing')),
    targets_other_author  BOOLEAN NOT NULL DEFAULT FALSE
);
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('patient.created', 'additive',    FALSE),
    ('patient.amended', 'additive',    FALSE),
    ('note.added',      'additive',    FALSE),
    ('advisory.added',  'additive',    FALSE),
    ('salience.downgrade','suppressing', TRUE),
    ('visibility.suppress','suppressing', TRUE)
ON CONFLICT (event_type) DO NOTHING;

-- Per-type twin/floor-check registry (#173, ADR-0048). Sibling of event_type_class:
-- a new event type registers its structural check + twin requirement by INSERTing ONE
-- row here (additive), instead of copying the whole cairn_event_twin dispatch chain into
-- a new migration. The single stable dispatcher (below) reads this table. Columns are
-- independent: check_fn NULL ⇒ no structural floor for this type; twin_required_msg NULL
-- ⇒ an absent authored twin degrades honestly to a skeleton (ADR-0039) rather than raising.
CREATE TABLE IF NOT EXISTS cairn_event_twin_check (
    event_type         TEXT PRIMARY KEY,
    check_fn           TEXT,
    twin_required_msg  TEXT
);

-- Fail-closed at REGISTRATION time (not first-call): a registered check_fn must exist with
-- the unified (text, jsonb) signature. A slice that registers a typo'd or not-yet-created
-- check fn fails loudly on schema load, for this migration and every future one, with
-- nothing to remember. (to_regprocedure returns NULL for an absent function; valid type
-- names never raise.) Residual: this validates registration, not later function mutation —
-- a migration that broke a check fn's signature afterwards would surface at runtime
-- (the dispatcher's EXECUTE raises, still fail-closed).
CREATE OR REPLACE FUNCTION cairn_check_twin_registry_fn()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.check_fn IS NOT NULL
       AND to_regprocedure(NEW.check_fn || '(text, jsonb)') IS NULL THEN
        RAISE EXCEPTION 'cairn_event_twin_check: check_fn %(text, jsonb) does not exist (fail closed)', NEW.check_fn;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS cairn_event_twin_check_validate ON cairn_event_twin_check;
CREATE TRIGGER cairn_event_twin_check_validate
    BEFORE INSERT OR UPDATE ON cairn_event_twin_check
    FOR EACH ROW EXECUTE FUNCTION cairn_check_twin_registry_fn();

-- Safety surface (like event_type_class): a row pointing a type's check at a no-op would
-- drop its floor. Lock it down; submit_event reads it as its SECURITY DEFINER owner, so
-- cairn_agent needs no grant.
REVOKE INSERT, UPDATE, DELETE ON cairn_event_twin_check FROM PUBLIC;

-- ---------------------------------------------------------------------------
-- The projection registry (#208 / ADR-0057): registration IS the wiring.
-- A projection lives only in its registered apply function; ONE dispatcher
-- trigger (below) replaces every per-type projection trigger, and
-- cairn_reproject (db/039) heals/rebuilds by replaying the IDENTICAL dispatch.
-- Same discipline as cairn_event_twin_check above (ADR-0048): register-by-row
-- in the migration that defines the fn, fail closed at registration time.
--
-- heal_safe: TRUE iff replaying an event through this fn over an EXISTING
-- projection converges (insert-or-better winner logic). A counter-shaped
-- projection (patient_chart.note_count) is NOT: replay would increment again.
-- Heal-mode reproject skips heal_safe=false rows with a notice; rebuild mode
-- (truncate-then-replay) handles them. New projections should be idempotent;
-- heal_safe=false needs a comment justifying the shape.
--
-- CAVEAT (#277): heal_safe=TRUE means "replay won't corrupt", NOT "replay
-- re-derives". An append-only fn keyed on event identity with ON CONFLICT
-- DO NOTHING (medication_dose_*, medication_attestation) is heal_safe=TRUE
-- because replay is idempotent — but heal leaves an already-materialised row
-- UNTOUCHED, so a fix to how that fn EXTRACTS a value from the body is NOT
-- healed by the loader's generation-change heal; only `reproject --rebuild`
-- re-extracts. Weigh this when shipping such a fix (see #277 for the options).
CREATE TABLE IF NOT EXISTS cairn_projection_apply (
    event_type        TEXT    NOT NULL,
    apply_fn          TEXT    NOT NULL,
    projection_tables TEXT[]  NOT NULL,
    run_order         INTEGER NOT NULL DEFAULT 100,
    heal_safe         BOOLEAN NOT NULL DEFAULT TRUE,
    PRIMARY KEY (event_type, apply_fn)
);

-- Fail closed at REGISTRATION time, exactly like cairn_check_twin_registry_fn:
-- the apply fn must exist with the unified (event_log) signature, and every
-- projection_tables entry must be a real relation (it is rebuild-scope metadata
-- — a typo would silently exempt the real table from rebuild's refusal check).
-- `SET search_path = public` pinned (same discipline as cairn_event_twin below):
-- the to_regprocedure/to_regclass resolution must never be shadowed by a caller's
-- search_path, regardless of who fires this validation trigger.
CREATE OR REPLACE FUNCTION cairn_check_projection_registry_fn()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE v_tbl text;
BEGIN
    IF to_regprocedure(NEW.apply_fn || '(event_log)') IS NULL THEN
        RAISE EXCEPTION
            'cairn_projection_apply: apply_fn %(event_log) does not exist (fail closed)',
            NEW.apply_fn;
    END IF;
    FOREACH v_tbl IN ARRAY NEW.projection_tables LOOP
        IF to_regclass(v_tbl) IS NULL THEN
            RAISE EXCEPTION
                'cairn_projection_apply: projection table "%" does not exist (fail closed)',
                v_tbl;
        END IF;
    END LOOP;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS cairn_projection_apply_validate ON cairn_projection_apply;
CREATE TRIGGER cairn_projection_apply_validate
    BEFORE INSERT OR UPDATE ON cairn_projection_apply
    FOR EACH ROW EXECUTE FUNCTION cairn_check_projection_registry_fn();

-- Safety surface: a row pointing a type's projection at a no-op would silently
-- stop materialization. Locked down like cairn_event_twin_check.
REVOKE INSERT, UPDATE, DELETE ON cairn_projection_apply FROM PUBLIC;

-- The #266 safety seam (ADR-0056 decision 4): cairn_reproject routes every
-- candidate event through this predicate. Constantly TRUE today — no deferred
-- events can exist while the remote door still fail-closes on unknown types.
-- #265's explicit deferred marker hooks in HERE and only here, so a manual
-- mid-upgrade reproject can never grant power to an unadjudicated deferred
-- event. The live-insert path needs no filter: an event being inserted through
-- a door was adjudicated by that door.
CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log)
RETURNS boolean LANGUAGE sql STABLE AS $$ SELECT TRUE $$;
-- Locked down like every predicate in this file: cairn_reproject (db/039) calls it as
-- the migration-defining owner, so no runtime role needs a grant, and PUBLIC's default
-- EXECUTE would let any connected role probe/depend on a predicate that becomes a real
-- safety-relevant filter under #265.
REVOKE EXECUTE ON FUNCTION cairn_replay_eligible(event_log) FROM PUBLIC;

-- The ONE projection trigger: look up the registered apply fns for this event's
-- type and run each. Deterministic order (run_order, then name — mirrors the
-- old alphabetical trigger-name firing order). Types with no registered rows
-- (e.g. carried-not-projected federation types, ADR-0012) dispatch nothing —
-- the same behavior the old WHEN-filtered triggers gave them.
--
-- `SET search_path = public` pinned here exactly like cairn_event_twin's dynamic
-- dispatch further down this file: the %I-quoted apply_fn EXECUTE must never resolve
-- into an attacker-shadowed schema regardless of who/what fired the AFTER INSERT
-- trigger that invokes this function — the dynamic-dispatch safety argument stays
-- self-contained here, not dependent on the firing role's search_path.
CREATE OR REPLACE FUNCTION cairn_projection_dispatch()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE r record;
BEGIN
    FOR r IN
        SELECT apply_fn FROM cairn_projection_apply
        WHERE event_type = NEW.event_type
        ORDER BY run_order, apply_fn
    LOOP
        EXECUTE format('SELECT %I($1)', r.apply_fn) USING NEW;
    END LOOP;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS cairn_projection_dispatch_trg ON event_log;
CREATE TRIGGER cairn_projection_dispatch_trg
    AFTER INSERT ON event_log
    FOR EACH ROW EXECUTE FUNCTION cairn_projection_dispatch();

-- Skeleton plaintext twin: the mechanical §3.13 fallback rendering. Kept as its own
-- helper so the per-type twin hook below can fall back to it without duplicating the
-- format. TODO: spec §3.13/ADR-0012 want the clinical payload rendered too.
CREATE OR REPLACE FUNCTION cairn_twin_skeleton(p_type text, b jsonb)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT format('[%s] %s for patient %s', p_type, b ->> 'schema_version', b ->> 'patient_id');
$$;

-- The single, stable per-event-type twin hook (§3.13/§4.5, #173/ADR-0048). Declared ONCE
-- here and never re-declared — a new event type registers a cairn_event_twin_check row in
-- its own migration (additive), so no slice ever copies this dispatch body (the prior
-- copy-a-stale-chain floor-regression hazard is designed out). Returns the plaintext twin
-- and, for a registered type, runs its structural floor (raising on violation).
--
-- Dispatch is dynamic: the check_fn name comes from the LOCKED, migration-only registry
-- table (never user input) and %I quotes it; a missing/mis-signed fn RAISES (fail-closed),
-- though the registry trigger already refused an unregistered fn at load time. The
-- EXECUTE 'SELECT fn($1,$2)' form is the dynamic equivalent of PERFORM fn(...) (every
-- check fn RETURNS void and works by RAISE-on-violation).
--
-- `SET search_path = public` is pinned on THIS function (not only on the SECURITY DEFINER
-- doors that call it), so the %I identifier can never be resolved into an attacker-shadowed
-- schema regardless of who invokes the hook — the dynamic-dispatch safety argument is
-- self-contained here, not dependent on the caller's search_path (defense in depth: today
-- the only callers are submit_event/apply_remote_event, which already pin it).
--
-- Twin policy (ADR-0039): an authored twin is carried verbatim for EVERY type; if absent,
-- a type with twin_required_msg RAISES (demographics + identity + medication hard-require
-- it), and every other type degrades honestly to a mechanical skeleton.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql
SET search_path = public
AS $$
DECLARE
    v_twin     text    := b ->> 'plaintext_twin';
    v_authored boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_fn       text;
    v_msg      text;
BEGIN
    SELECT check_fn, twin_required_msg INTO v_fn, v_msg
        FROM cairn_event_twin_check WHERE event_type = p_type;

    IF v_fn IS NOT NULL THEN
        EXECUTE format('SELECT %I($1, $2)', v_fn) USING p_type, b;
    END IF;

    IF v_authored THEN
        RETURN v_twin;
    END IF;
    IF v_msg IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_msg;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

-- Suppression owner-gate (ADR-0043 / issue #99). A suppressing overlay
-- (salience.downgrade / visibility.suppress) that forecloses on a HUMAN author's
-- event is self-only: only that human may suppress it. Cross-human suppression is
-- refused — disagreement is expressed additively (a note referencing the target),
-- never by touching another author's content (principle 1/2, paper-parity).
-- An agent-authored / un-owned advisory (no responsible human) stays dismissable by
-- any enrolled human — the clinician-overrides-the-machine path (principle 10), NOT
-- the burying of a colleague.
--
-- The target's human authors = {signer_key_id if it EVER resolved to a kind='human'
-- actor} ∪ {hex(attester_key) if a human attestation is stored}. Empty set ⇒
-- agent/un-owned ⇒ permitted. Non-empty ⇒ permitted only if the attester is in it.
-- STABLE (reads event_log + actor_event). Shared by BOTH doors so a replicated
-- cross-human suppress faces the identical refusal (principle 12). Safe direction:
-- an unknown/ambiguous attester on human-authored content refuses, never permits.
--
-- Signer human-ness is resolved from the append-only actor_event HISTORY, not
-- actor_current — mirroring the discipline db/020 step 2 already uses for stamping.
-- Authorship is an immutable historical fact: a plain note.added stores no
-- attester_key, so its ONLY human-author signal is the signer's registry kind AT
-- AUTHORING TIME. If a departed/rotated author's key later drops out of
-- actor_current (revoke, or supersede onto a new key), querying actor_current here
-- would silently empty the human-author set and flip the gate open — over-permission
-- on the safety floor (any enrolled human could then suppress a departed colleague's
-- notes), which contradicts ADR-0043's never-over-permission invariant. actor_event
-- is append-only, so this branch is monotonic: a key that was ever human stays human
-- for this check forever. Wrong direction is over-refusal, never over-permission.
CREATE OR REPLACE FUNCTION cairn_suppression_author_ok(p_target UUID, p_attester_key BYTEA)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH tgt AS (
        SELECT el.signer_key_id, el.attester_key
        FROM event_log el WHERE el.event_id = p_target
    ),
    human_authors AS (
        SELECT t.signer_key_id AS kid FROM tgt t
        WHERE EXISTS (SELECT 1 FROM actor_event ae
                      WHERE ae.signing_key_id = t.signer_key_id
                        AND ae.op IN ('enroll','supersede')
                        AND ae.kind = 'human')
        UNION
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
    )
    SELECT NOT EXISTS (SELECT 1 FROM human_authors)
        OR EXISTS (SELECT 1 FROM human_authors h WHERE h.kid = encode(p_attester_key, 'hex'));
$$;

-- Fail-closed suppression target resolution (issue #191, finding A3). The floor contract
-- for every targets_other_author type: the payload MUST name its target under
-- `target_event_id`, as a valid UUID. The old gates were key-presence-conditional
-- (`IF v_targets_other AND payload ? 'target_event_id'`), so ABSENCE — or a target
-- smuggled under any other key — skipped target validation AND the ADR-0043 owner-gate
-- entirely: an unowner-gated cross-human suppression path for the first consumer that
-- resolves targets leniently. ONE shared helper, called by BOTH doors and by the
-- registry check below, so the refusal is identical everywhere (principle 12).
-- pg_input_is_valid keeps a malformed UUID legible (names the field) instead of a bare
-- 22P02 cast error.
CREATE OR REPLACE FUNCTION cairn_suppression_target_id(b jsonb)
RETURNS uuid LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_raw text := b -> 'payload' ->> 'target_event_id';
BEGIN
    IF v_raw IS NULL THEN
        RAISE EXCEPTION 'suppression overlay: payload.target_event_id is required — a targeting overlay without a valid target fails closed (issue #191)';
    END IF;
    IF NOT pg_input_is_valid(v_raw, 'uuid') THEN
        RAISE EXCEPTION 'suppression overlay: payload.target_event_id (%) is not a valid UUID (issue #191)', v_raw;
    END IF;
    RETURN v_raw::uuid;
END;
$$;

-- ADR-0048 structural floor for the suppression types: registered so the requirement is
-- carried by the locked registry (both doors run it via the cairn_event_twin dispatcher),
-- not only by the door-gate branch above it — a future targets_other type that forgets its
-- registry row still fails closed at the door gate, and vice versa (defense in depth).
CREATE OR REPLACE FUNCTION cairn_check_suppression_overlay(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_suppression_target_id(b);
END;
$$;

-- twin_required_msg stays NULL: a suppression overlay keeps the honest mechanical
-- skeleton fallback (ADR-0039) — the structural requirement here is the target, not
-- an authored twin.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('salience.downgrade',   'cairn_check_suppression_overlay', NULL),
    ('visibility.suppress',  'cairn_check_suppression_overlay', NULL)
ON CONFLICT (event_type) DO NOTHING;

-- Responsibility↔attester binding (issue #195, finding A7). The attestation token
-- proves SOME enrolled human vouched for these bytes; without this check the signed,
-- immutable body could claim `responsibility` for a DIFFERENT actor — permanently
-- recording an unverified responsibility claim about a person who never touched the
-- event (projections key on the verified attester_key, so display was safe; the
-- RECORD was not). Contract: a contributor claiming `responsibility` must name the
-- verified attester's key. This also (deliberately) limits one event to ONE
-- responsibility-holder — the door verifies one token; plural/proxy responsibility
-- shapes are the #203/#96 wire-shape decision and would extend this predicate, not
-- bypass it. Shared by BOTH doors (principle 12).
CREATE OR REPLACE FUNCTION cairn_responsibility_bound(b jsonb, p_attester_key bytea)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE e ? 'responsibility'
          AND e ->> 'actor_id' IS DISTINCT FROM encode(p_attester_key, 'hex'));
$$;

-- ---------------------------------------------------------------------------
-- The ratified contributor-role vocabulary (ADR-0028 membership + ADR-0051
-- ratification of `recorded` and the floor check itself; issues #203/#96).
--
-- The role enum is a safety primitive: the structural "AI-generated" reading and
-- the ADR-0010 suppression owner-gate branch on whether a role BEARS
-- responsibility, so "closed" must be floor, not convention. This table is the
-- floor-queryable form; `cairn-event::contributor::ROLE_VOCABULARY` is the Rust
-- mirror (drift guard: crates/cairn-node/tests/contributor_roles.rs). Additive-only:
-- a new member is an ADR-recorded act appending ONE row here + one tuple there,
-- and its canonical WIRE value must carry the partition prefix (`bearing:x` /
-- `contrib:x`) so a node that predates it can still classify it (#96).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS contributor_role (
    role  TEXT PRIMARY KEY,
    bears BOOLEAN NOT NULL   -- responsibility-bearing vs contributory (ADR-0007/0028)
);

INSERT INTO contributor_role (role, bears) VALUES
    -- Responsibility-bearing (6) — ADR-0028.
    ('authored',    true),
    ('ordered',     true),
    ('attested',    true),
    ('co-signed',   true),
    ('witnessed',   true),
    ('dictated',    true),
    -- Contributory (6) — ADR-0028's five + `recorded` (ADR-0051): the recording
    -- device/system that captured and persisted the event; asserts capture
    -- fidelity, adds no clinical content, bears no clinical responsibility.
    ('drafted',     false),
    ('transcribed', false),
    ('graded',      false),
    ('triaged',     false),
    ('suggested',   false),
    ('recorded',    false)
ON CONFLICT (role) DO NOTHING;

-- Safety surface (like event_type_class): a stray write here MOVES the floor itself —
-- an inserted 'bearing' row would let its author mint arbitrary responsibility-bearing
-- roles through the strict door; flipping a member's `bears` breaks partition coherence
-- for every consumer. Lock it down; both doors read it as their SECURITY DEFINER owner,
-- so no runtime role needs a grant. Growth is a migration-only, ADR-recorded act.
REVOKE INSERT, UPDATE, DELETE ON contributor_role FROM PUBLIC;

-- The contributor-set floor (ADR-0051), shared by BOTH doors (principle 12) with
-- one strictness switch that encodes the doors' different obligations:
--
--   * submit (p_strict = true, the AUTHORING door): fail closed on any role this
--     node's vocabulary has not ratified — a door only authors what it can stand
--     behind — and refuse `on_behalf_of` until a proxy-grant ADR defines how the
--     principal's consent is verified.
--   * apply (p_strict = false, the SYNC door): NEVER reject on role membership —
--     set-union losslessness (#96): a future member arrives partition-prefixed and
--     classifies by its prefix; a wholly-unknown role claiming nothing degrades to
--     the vouching-unknown reading at projection time. `on_behalf_of` is admitted
--     as a signed, display-gated claim (spec §3.9 promises the proxy transition
--     "with no schema migration" — refusing it here would wedge every future proxy
--     event out of the set-union, the #201 lesson).
--
-- Checks that hold at BOTH doors are the never-lawful shapes no conformant door of
-- ANY schema version could mint (same refusal class as an invalid attestation
-- token): a contributor without actor_id/role (illegible authorship), a
-- responsibility value that is not an object naming held_by (the retired flat
-- string), held_by naming anyone but the entry's own actor (the #195 binding —
-- combined with cairn_responsibility_bound's actor=attester check this chains
-- held_by = actor_id = verified attester), and responsibility claimed on a
-- non-bearing role (partitions are additive-only and never flip, so this
-- incoherence can never become valid).
-- `SET search_path = public` is pinned HERE, not only on the SECURITY DEFINER doors
-- that call it (the cairn_event_twin discipline): the contributor_role lookup must
-- never resolve into a caller-shadowed schema, regardless of who invokes the check.
CREATE OR REPLACE FUNCTION cairn_check_contributors(b jsonb, p_door text, p_strict boolean)
RETURNS void LANGUAGE plpgsql STABLE
SET search_path = public
AS $$
DECLARE
    e       jsonb;
    v_role  text;
    v_resp  jsonb;
    v_bears boolean;
BEGIN
    IF p_strict AND (jsonb_typeof(b -> 'contributors') IS DISTINCT FROM 'array'
                     OR jsonb_array_length(b -> 'contributors') = 0) THEN
        RAISE EXCEPTION '%: contributors must be a non-empty array — an event must declare its authorship (ADR-0051)', p_door;
    END IF;
    FOR e IN SELECT * FROM jsonb_array_elements(
                 CASE WHEN jsonb_typeof(b -> 'contributors') = 'array'
                      THEN b -> 'contributors' ELSE '[]'::jsonb END) LOOP
        v_role := e ->> 'role';
        IF e ->> 'actor_id' IS NULL OR v_role IS NULL THEN
            RAISE EXCEPTION '%: a contributor entry lacks actor_id/role — illegible authorship refused (ADR-0051)', p_door;
        END IF;
        IF p_strict AND NOT EXISTS (SELECT 1 FROM contributor_role r WHERE r.role = v_role) THEN
            RAISE EXCEPTION '%: contributor role "%" is not in the ratified role vocabulary — this door only authors roles it can stand behind (ADR-0028/ADR-0051)', p_door, v_role;
        END IF;
        IF e ? 'responsibility' THEN
            v_resp := e -> 'responsibility';
            IF jsonb_typeof(v_resp) IS DISTINCT FROM 'object' OR v_resp ->> 'held_by' IS NULL THEN
                RAISE EXCEPTION '%: responsibility must be an object naming held_by — the flat-string shape is retired (ADR-0051, spec §3.9)', p_door;
            END IF;
            IF v_resp ->> 'held_by' IS DISTINCT FROM e ->> 'actor_id' THEN
                RAISE EXCEPTION '%: responsibility.held_by must name the contributor entry''s own actor (issue #195 binding, ADR-0051)', p_door;
            END IF;
            IF p_strict AND v_resp ? 'on_behalf_of' THEN
                RAISE EXCEPTION '%: on_behalf_of is not yet admissible at the authoring door — proxy responsibility awaits its verification mechanism (ADR-0051)', p_door;
            END IF;
            -- Partition coherence: known members classify from the table, future
            -- members from their mandatory prefix; anything else claiming
            -- responsibility is unmintable by a conformant door of any version.
            v_bears := coalesce(
                (SELECT r.bears FROM contributor_role r WHERE r.role = v_role),
                v_role LIKE 'bearing:%');
            IF NOT v_bears THEN
                RAISE EXCEPTION '%: responsibility claimed on non-responsibility-bearing role "%" — incoherent authorship refused (ADR-0051)', p_door, v_role;
            END IF;
        END IF;
    END LOOP;
END;
$$;

-- Authorship binding (ADR-0053, issue #204). The authorship analog of
-- cairn_responsibility_bound (#195): a responsibility-BEARING contributor may only
-- name an actor who AUTHENTICATED to the event — the signer, or the verified
-- attester. So an `authored`/`ordered`/`attested` claim about a human is unforgeable:
-- that human either signed the bytes or attested them. Contributory roles
-- (`recorded`/`drafted`/...) are EXEMPT — a device/auxiliary contributor need not
-- sign or attest (the node stays `recorded` while the human signs). Bearing-ness
-- classifies from the ratified table, else the mandatory `bearing:` prefix (the same
-- idiom as cairn_check_contributors). STABLE (reads contributor_role) with a pinned
-- search_path (the contributor_role lookup must never resolve into a shadowed schema).
--
-- NOTE on the `bearing:` prefix arm: it is UNREACHABLE from this function's only call
-- site. Step 1c already ran cairn_check_contributors(..., p_strict => true), which
-- refuses any role outside the ratified table, so a future `bearing:x` role never
-- reaches step 4b at THIS door. The arm is kept deliberately — it costs nothing, it
-- keeps the idiom identical to its siblings, and it is what makes the predicate safe to
-- reuse from a lenient caller (e.g. the #245 read-side grader) without re-deriving the
-- partition rule. Do not read it as live coverage of future roles at the strict door.
--
-- STRUCTURAL, not semantic — exactly like its sibling cairn_responsibility_bound, this
-- predicate is intentionally structural over ALL responsibility-bearing contributors: it
-- checks only that the named actor authenticated (signed or attested), never who/what that
-- actor is (it does NOT resolve actor-kind). It therefore FAILS CLOSED stricter than the
-- §3.9 prose ("resolves to a human actor"): the deferred token-backed-author / AI-scribe
-- path (an author who did not sign) authenticates through the verified-attester arm
-- (actor_id = attester), so no lawful future authorship shape is wrongly refused here.
--
-- STRICT DOOR ONLY. The apply door (db/020) must NOT call this: an unverifiable
-- authorship claim there is a forgery OR an author authenticated by a scheme this
-- older node cannot parse (ADR-0012 guarantees such events arrive), and the two are
-- indistinguishable — so apply admits and GRADES (classify_authorship_confidence),
-- never refuses. Do not "simplify" this into a both-doors symmetry.
CREATE OR REPLACE FUNCTION cairn_authorship_bound(b jsonb, p_signer text, p_attester_key bytea)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public
AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE coalesce((SELECT r.bears FROM contributor_role r WHERE r.role = e ->> 'role'),
                       (e ->> 'role') LIKE 'bearing:%')
          AND (e ->> 'actor_id') IS DISTINCT FROM p_signer
          AND (p_attester_key IS NULL
               OR (e ->> 'actor_id') IS DISTINCT FROM encode(p_attester_key, 'hex')));
$$;

-- ---------------------------------------------------------------------------
-- ADR-0052 custody plane, part 1 — the CLEAR-view table and its read helper.
--
-- These two definitions live HERE, in db/005, rather than in db/037 (the rest of
-- the custody plane), for a hard migration-ordering reason: db/034's two
-- `LANGUAGE sql` functions (cairn_medication_thread_commitment,
-- cairn_medication_thread_readable_count) call cairn_clear_payload, and a
-- `LANGUAGE sql` function resolves its references EAGERLY at CREATE time. If the
-- helper were still defined in db/037 (which loads AFTER db/034), a genuinely
-- FRESH database would fail at db/034 with "function cairn_clear_payload(event_log)
-- does not exist". db/005 (the submit door) is the earliest migration present in
-- BOTH the cairn-node main schema (crates/cairn-node/src/db.rs) AND the cairn-sync
-- subset (crates/cairn-sync/src/main.rs), it loads before db/034, event_log already
-- exists (db/001, with its `body` and `sealed` columns), and the door below is the
-- first user of event_clear — so this is the correct common home. The rest of the
-- custody plane (event_dek, node_unwrap_key, erasure_shred_log, shred execution,
-- the erasure.shred.asserted verb) stays in db/037. Idempotent: CREATE TABLE IF NOT
-- EXISTS / CREATE OR REPLACE, so replay is safe on a DB that loaded the pre-move
-- layout.
--
-- The operational clear view of sealed bodies: THE single derived-plaintext surface
-- (clear payload + clear twin), populated by the doors, deleted by a shred. No FK to
-- event_log: the door inserts this row BEFORE the event_log row so the AFTER INSERT
-- projection triggers can already read it (same transaction — atomicity keeps them
-- consistent). Future FTS/RAG indexes MUST build on this table and nothing else (#92 (b)).
CREATE TABLE IF NOT EXISTS event_clear (
    event_id UUID  PRIMARY KEY,
    body     JSONB NOT NULL,   -- the CLEAR payload (matches event_log.body semantics)
    twin     TEXT  NOT NULL    -- the CLEAR legibility twin
);

-- Projection read helper: the ONE way a projection trigger reads a clinical payload.
-- Unsealed → the derived body column; sealed → the clear shadow (NULL when this node
-- holds no custody: the caller skips projection). LANGUAGE sql, so its callers in
-- db/034 bind eagerly against it (see the ordering note above).
CREATE OR REPLACE FUNCTION cairn_clear_payload(ev event_log) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN NOT ev.sealed THEN ev.body
                ELSE (SELECT body FROM event_clear WHERE event_id = ev.event_id)
           END
$$;

-- Grant floor for event_clear (door-managed writes only; SELECT is the clear READ
-- surface for chart/FTS). cairn_agent is created in db/004, before this migration.
-- Moved here from db/037 alongside the table definition above.
REVOKE ALL ON event_clear FROM PUBLIC;
REVOKE ALL ON event_clear FROM cairn_agent;
GRANT SELECT ON event_clear TO cairn_agent;  -- the clear READ surface (chart/FTS)
-- ---------------------------------------------------------------------------

-- ADR-0052: the door gained p_dek. A CREATE OR REPLACE with a different arg
-- list would OVERLOAD (3-arg + 4-arg → ambiguous 1-arg calls), so drop the old
-- signature first. Idempotent across replays.
DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);

CREATE OR REPLACE FUNCTION submit_event(
    p_signed       BYTEA,
    p_attestation  BYTEA DEFAULT NULL,
    p_attester_key BYTEA DEFAULT NULL,
    p_dek          BYTEA DEFAULT NULL
) RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    b              JSONB;
    v_event_id     UUID;
    v_ca           BYTEA;
    v_type         TEXT;
    v_mode         TEXT;
    v_targets_other BOOLEAN;
    v_bears        BOOLEAN;
    v_target_id    UUID;
    v_twin         TEXT;
    v_t_eff        TIMESTAMPTZ;
    v_grade        text;              -- ADR-0058 born clock-confidence grade (issue #216)
    v_verdict      text;              -- cairn_ceiling_classify result: ok | flag | reject
    v_att          BYTEA;
    v_att_key      BYTEA;
    v_actor_ids    BYTEA[];
    v_actor_id     BYTEA;
    v_target_sealed BOOLEAN;          -- erasure arm: is the shred TARGET born-sealed? (finding #5)
    -- ADR-0052 born-sealed arm.
    v_sealed       BOOLEAN := false;  -- did the body arrive as the sealed container?
    b_clear        JSONB;             -- the CLEAR view floor checks + projections run on
    v_inner        JSONB;             -- {payload, plaintext_twin} recovered by cairn_unseal_body
    v_pub          BYTEA;             -- this node's X25519 unwrap-key public half
    v_twin_stub    TEXT;              -- the outer, signed mechanical stub twin (principle 11)
BEGIN
    -- 0. Size ceiling (review fix A7a): refuse an oversized event BEFORE the crypto work,
    --    so an event too large to replicate or back up can never be admitted (it would
    --    otherwise wedge sync at its seq forever). See cairn_max_event_bytes() (db/001).
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'submit_event: event is % bytes, over the % -byte admission ceiling (would wedge sync/backup)',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;

    -- 1. Signature floor (C5.1). cairn_verify is the in-DB pgrx gate.
    IF NOT cairn_verify(p_signed) THEN
        -- Keep the boolean floor; attach the legible reason as DETAIL so an operator can
        -- tell a wire-format skew / pre-ADR-0040 context mismatch from actual tampering
        -- (issue #109). cairn_verify already returned false, so cairn_verify_error is
        -- non-NULL here; the coalesce guards only the impossible NULL case.
        RAISE EXCEPTION 'submit_event: signature verification failed (unsigned or malformed event)'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'submit_event: event body could not be parsed after verify';
    END IF;

    v_event_id := (b ->> 'event_id')::uuid;
    v_type     := b ->> 'event_type';
    -- content_address = sha256 of the signed wire bytes (the COSE envelope), identical to event_address() in cairn-event and the db/001 CHECK. (Distinct from canonical_json_address, which hashes the actor pinned-set body for actor_id.) Attestation tokens bind to THIS value.
    v_ca       := '\x1220'::bytea || digest(p_signed, 'sha256');

    -- 1a. Clock-drift ceiling at the LOCAL door (issue #187, finding A1): refuse an event
    --     whose asserted HLC wall is implausibly far in OUR future. Every standing-state
    --     overlay ranks winners by `ORDER BY hlc_wall DESC`, so one admitted event with a
    --     wall of ~2^62 would win every projection on every node FOREVER — no honest later
    --     event could ever outrank it, and in an append-only system the only recovery would
    --     be operator recall + projection rebuild (a floor violation, not a display concern).
    --     REJECTION is safe here for the same reason db/007 rejects on the node plane:
    --     nothing has accepted this event yet (it is being authored, not replicated), so a
    --     refusal cannot fork the fleet or wedge a sync watermark. The bound is the shared
    --     cairn_max_hlc_drift_ms() (db/001, 24h) — generous to honest clock skew (an offline
    --     node's drifted RTC), measured against clock_timestamp() (our own wall clock), never
    --     the possibly-already-advanced hlc_state, so the bound cannot itself be ratcheted.
    --     (The clinical REMOTE door, db/020, deliberately clamps-and-admits instead — a
    --     refused verifiable event would freeze the puller's watermark; see hlc_drift.rs.)
    IF (b -> 'hlc' ->> 'wall')::bigint
           > (extract(epoch FROM clock_timestamp()) * 1000)::bigint + cairn_max_hlc_drift_ms() THEN
        RAISE EXCEPTION 'submit_event: HLC wall % ms is more than % ms ahead of local time — clock-drift ceiling (issue #187)',
            (b -> 'hlc' ->> 'wall')::bigint, cairn_max_hlc_drift_ms();
    END IF;

    -- 1b. t_effective wire pin (issue #91/H4): parse the asserted claim through the ONE
    --     explicit-offset validator (db/001 cairn_t_effective), so the stored instant is
    --     identical on every node regardless of session TimeZone/DateStyle.
    v_t_eff := cairn_t_effective(b ->> 't_effective');

    -- 1b'. Grade-gated bitemporal ceiling (ADR-0058 refines ADR-0003 §3.6). The born clock_grade
    --      (a mandatory EventBody field — compile-time guaranteed for conforming clients; an absent
    --      or unrecognized value reads as the safe 'unknown', rank 0) gates the ceiling's rejecting
    --      power: at unknown/self-asserted the upper bound is OPEN, so a forward t_effective is
    --      FLAGGED, never rejected (principle 4 — a slow/dead clock must not force fabrication). The
    --      'reject' arm fires only for a credible high grade — production-unreachable this slice (no
    --      node mints above self-asserted; exercised by synthesis in the tests). The door gates
    --      EFFECT not PRESENCE (ADR-0056): a missing grade is admitted as 'unknown', never refused.
    v_grade := COALESCE(b ->> 'clock_grade', 'unknown');
    v_verdict := cairn_ceiling_classify((b -> 'hlc' ->> 'wall')::bigint, v_grade, v_t_eff);
    IF v_verdict = 'reject' THEN
        RAISE EXCEPTION 'submit_event: t_effective (%) exceeds the ceiling for a "%" clock (ADR-0058 grade-gated)',
            b ->> 't_effective', v_grade;
    ELSIF v_verdict = 'flag' THEN
        PERFORM cairn_record_ceiling_flag(v_ca, (b -> 'hlc' ->> 'wall')::bigint, v_t_eff, v_grade, 'flag');
    END IF;

    -- 1c. Contributor-set floor (ADR-0051, issues #203/#96): the STRICT door — every
    --     role must be in the ratified vocabulary, and a responsibility claim must be
    --     a well-formed {held_by} object on a bearing role (see cairn_check_contributors).
    PERFORM cairn_check_contributors(b, 'submit_event', true);

    -- 2. Resolve the signer against the actor registry (must be enrolled, non-revoked)
    --    and RECORD the resolution (issue #99): a unique key->actor mapping stamps the
    --    admitting actor_id on the row, so a later contamination-cascade recall selects
    --    this event exactly even after the key is re-enrolled under a new skill_epoch.
    --    A key concurrently registered to several actors stamps NULL — attribution
    --    honestly unknown (principle 4) — and the recall query (db/006) over-selects
    --    NULL rows rather than ever missing one.
    SELECT array_agg(DISTINCT actor_id) INTO v_actor_ids
        FROM actor_current WHERE signing_key_id = b ->> 'signer_key_id';
    IF v_actor_ids IS NULL THEN
        RAISE EXCEPTION 'submit_event: signer % is not an enrolled, non-revoked actor', b ->> 'signer_key_id';
    END IF;
    v_actor_id := CASE WHEN array_length(v_actor_ids, 1) = 1 THEN v_actor_ids[1] END;

    -- 3. Classify (fail closed on unknown type).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    IF v_mode IS NULL THEN
        RAISE EXCEPTION 'submit_event: unknown event_type % (no classification — fail closed)', v_type;
    END IF;

    -- Does any contributor claim a responsibility (bearing role with attestation)?
    v_bears := EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE e ? 'responsibility');

    -- 4. Attestation gate. A suppressing event, OR any asserted responsibility,
    --    requires a valid attestation token bound to THIS event (C2, C5.2, C5.3).
    IF v_mode = 'suppressing' OR v_bears THEN
        IF p_attestation IS NULL OR p_attester_key IS NULL THEN
            RAISE EXCEPTION 'submit_event: % requires attestation (no token presented) — un-vouched suppress/responsibility refused', v_type;
        END IF;
        IF NOT cairn_attestation_ok(p_attestation, v_ca, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: attestation token invalid or not bound to this event';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM actor_current
                       WHERE signing_key_id = encode(p_attester_key,'hex') AND kind = 'human') THEN
            RAISE EXCEPTION 'submit_event: attester is not an enrolled human actor (forged human author refused)';
        END IF;
        -- #195: the body's responsibility claim must name the human whose token we
        -- just verified — never a third party (see cairn_responsibility_bound).
        IF NOT cairn_responsibility_bound(b, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: a contributor claims responsibility for an actor other than the verified attester — unverified responsibility claim refused (issue #195)';
        END IF;
        -- Store the VERIFIED responsibility proof beside the event (issue #91/M7):
        -- it must keep travelling with the event on the sync wire, or a downstream
        -- node could never re-run this gate at its own apply door.
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    -- 4b. Authorship binding (ADR-0053, issue #204): every responsibility-bearing
    --     contributor must be AUTHENTICATED — its actor_id is the event's signer or
    --     the verified attester (v_att_key, set by step 4, else NULL). Extends the
    --     #195 responsibility<->attester binding to AUTHORSHIP so an authored/ordered
    --     claim about a human is unforgeable. Contributory roles are exempt. STRICT
    --     door only; the apply door admits + grades (see cairn_authorship_bound).
    IF NOT cairn_authorship_bound(b, b ->> 'signer_key_id', v_att_key) THEN
        RAISE EXCEPTION 'submit_event: a responsibility-bearing contributor names an actor that is neither the event signer nor the verified attester — forged authorship refused (ADR-0053; the author must sign or attest)';
    END IF;

    -- 5. Target gate for an overlay on another author's event — UNCONDITIONAL for every
    --    targets_other type (issue #191): the old `AND (payload ? 'target_event_id')`
    --    guard made the whole gate key-presence-conditional, so absence failed OPEN past
    --    both the existence check and the ADR-0043 owner-gate. cairn_suppression_target_id
    --    RAISEs legibly on a missing or malformed target (fail closed).
    IF v_targets_other THEN
        v_target_id := cairn_suppression_target_id(b);
        IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target_id) THEN
            RAISE EXCEPTION 'submit_event: overlay targets unknown event %', v_target_id;
        END IF;

        -- ADR-0043 owner-gate: a suppressing overlay of a HUMAN author's event is
        -- self-only. Cross-human suppression is refused; express disagreement
        -- additively. (Agent advisories are un-owned ⇒ cairn_suppression_author_ok
        -- returns TRUE ⇒ dismissable.) p_attester_key is non-NULL here: step 4
        -- already refused a suppressing event without a valid human token.
        IF v_mode = 'suppressing'
           AND NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: cross-author suppression refused — you may only suppress your own events; express disagreement additively (a note referencing the target). (ADR-0043)';
        END IF;
    END IF;

    -- 6. Provenance binding (C3): an advisory must cite its source blob's address.
    IF v_type = 'advisory.added' THEN
        IF jsonb_array_length(COALESCE(b -> 'attachments', '[]'::jsonb)) = 0 THEN
            RAISE EXCEPTION 'submit_event: advisory.added must carry a provenance attachment reference';
        END IF;
    END IF;

    -- 7. ADR-0052 born-sealed arm. A clinical body arrives EITHER as the sealed
    --     container (payload.sealed = true) — the shipped default — or as legacy
    --     plaintext, which the STRICT door refuses: an unsealed clinical body is
    --     permanently un-shreddable, and this floor is what makes the posture
    --     unbypassable (principle 12). The apply door stays lenient (set-union).
    v_sealed := COALESCE((b -> 'payload' ->> 'sealed')::boolean, false);
    b_clear  := b;
    IF v_sealed THEN
        -- ADR-0052 §2 (the INVERSE of the born-sealed floor below): ONLY clinical.* bodies
        -- are born-sealed. Demographic/identity/patient/node/erasure bodies are plaintext BY
        -- NECESSITY — their projections/matchers bind on NEW.body DIRECTLY, so a sealed
        -- (ciphertext) body of one of those types can never project, and its ciphertext would
        -- detonate a NEW.body-reading projection (a NULL field driven into a NOT NULL column).
        -- This is a never-lawful shape; refuse it CLEANLY here, before anything is stored
        -- (submit refusals are safe — nothing has accepted the event). The apply door cannot
        -- mirror this RAISE — a refusal there would freeze the seq watermark on a verifiable
        -- event — so it stays lenient and the non-clinical projection triggers are made
        -- seal-robust instead (they RETURN NULL on a sealed row; db/002/010-014/018/023-025).
        IF v_type NOT LIKE 'clinical.%' THEN
            RAISE EXCEPTION 'submit_event: % is not a clinical body — only clinical.* bodies are born-sealed; demographic/identity/patient/node/erasure bodies are plaintext by necessity and must never be sealed (ADR-0052 §2)', v_type;
        END IF;
        IF p_dek IS NULL THEN
            RAISE EXCEPTION 'submit_event: sealed event requires its DEK at the strict door (ADR-0052)';
        END IF;
        v_inner := cairn_unseal_body(b -> 'payload', p_dek, v_event_id::text);
        IF v_inner IS NULL THEN
            RAISE EXCEPTION 'submit_event: sealed body failed to open with the presented DEK (wrong key, tampered container, or event-id mismatch) — refused (ADR-0052)';
        END IF;
        v_twin_stub := b ->> 'plaintext_twin';
        IF COALESCE(v_twin_stub, '') = '' THEN
            RAISE EXCEPTION 'submit_event: sealed event must carry a signed plaintext twin STUB (principle 11 — the row must stay self-describing) (ADR-0052)';
        END IF;
        -- The floor checks below run on the CLEAR view; the log stores ciphertext.
        b_clear := jsonb_set(jsonb_set(b, '{payload}', v_inner -> 'payload'),
                             '{plaintext_twin}', v_inner -> 'plaintext_twin');
    ELSIF v_type LIKE 'clinical.%' THEN
        RAISE EXCEPTION 'submit_event: % is a clinical body and must be born-sealed — plaintext clinical submissions are refused at the strict door (ADR-0052; wipe pre-ADR-0052 dev rigs, never sync them through)', v_type;
    END IF;

    -- 8. Plaintext twin (§3.13/§4.5) + any per-type structural floor, via the
    --    cairn_event_twin hook so a new event type adds its branch there, not by
    --    re-declaring this whole door. Runs on the CLEAR view so a sealed body's
    --    structural floor is checked on its real payload, never the ciphertext.
    v_twin := cairn_event_twin(v_type, b_clear);

    -- 9. Custody + operational clear view — BEFORE the log INSERT so the AFTER
    --     INSERT projection triggers can already read the shadow (same txn).
    --     An already-shredded target gets NEITHER: set-union may re-deliver the
    --     row forever, but custody never resurrects (arrival-order independence).
    IF v_sealed AND NOT EXISTS (SELECT 1 FROM erasure_shred_log WHERE target_event_id = v_event_id) THEN
        SELECT unwrap_pub INTO v_pub FROM node_unwrap_key;
        IF v_pub IS NULL THEN
            RAISE EXCEPTION 'submit_event: node unwrap key not registered — the authoring daemon must call cairn_register_unwrap_key first (ADR-0052)';
        END IF;
        INSERT INTO event_dek (event_id, dek_wrapped)
        VALUES (v_event_id, cairn_wrap_dek(p_dek, v_pub))
        ON CONFLICT (event_id) DO NOTHING;
        INSERT INTO event_clear (event_id, body, twin)
        VALUES (v_event_id, b_clear -> 'payload', v_twin)
        ON CONFLICT (event_id) DO NOTHING;
    END IF;

    INSERT INTO event_log
        (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
         node_origin, t_effective, signed_bytes, content_address, body, contributors,
         signer_key_id, plaintext_twin, attachments, attestation, attester_key, actor_id, sealed,
         clock_grade)
    VALUES (
        v_event_id, (b ->> 'patient_id')::uuid, v_type, b ->> 'schema_version',
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin',
        v_t_eff,
        -- body stays the honest derived view: the ciphertext container for a
        -- sealed row (event_log is append-only + never holds cleartext); the
        -- CLEAR payload lives in the event_clear shadow above.
        p_signed, v_ca, b -> 'payload', b -> 'contributors',
        b ->> 'signer_key_id',
        CASE WHEN v_sealed THEN v_twin_stub ELSE v_twin END,
        COALESCE(b -> 'attachments','[]'::jsonb),
        v_att, v_att_key, v_actor_id, v_sealed,
        v_grade)
    ON CONFLICT (event_id) DO NOTHING;

    -- Idempotent re-submit of the SAME event is a silent no-op (set-union).
    -- But a DIFFERENT event reusing this event_id (substitution) must not pass
    -- silently: compare the stored content-address to what we just verified.
    IF NOT FOUND THEN
        IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
            RAISE EXCEPTION 'submit_event: event_id % already exists with different content (substitution refused)', v_event_id;
        END IF;
    END IF;

    -- Learn any attachment references, per rendition (reference-eager, byte-lazy).
    -- Shared with the remote-apply door via cairn_learn_attachment_refs (db/027) so the
    -- two doors never drift.
    PERFORM cairn_learn_attachment_refs(b);

    -- 10. The erasure plane: an admitted shred tombstone EXECUTES here (ADR-0052).
    --     Strict door: the target must exist locally AND be born-sealed. Shredding the
    --     unknown is a user error at authoring time; shredding a NON-sealed (plaintext)
    --     target is a FALSE erasure — crypto-shred destroys a per-event DEK, but a plaintext
    --     body has none and stays readable in the append-only log forever, so reporting an
    --     erasure that cannot happen is refused here (code-review finding #5, ADR-0052 §6).
    --     The APPLY door is lenient on BOTH (a shred may arrive before its target on the wire,
    --     and a non-conformant peer's shred of a plaintext event must not freeze the watermark)
    --     — it degrades honestly instead. The tombstone itself is plaintext by design (v_sealed
    --     is false for erasure.*), so b_clear = b here.
    IF v_type = 'erasure.shred.asserted' THEN
        SELECT sealed INTO v_target_sealed FROM event_log
            WHERE event_id = (b_clear -> 'payload' ->> 'target_event_id')::uuid;
        IF NOT FOUND THEN
            RAISE EXCEPTION 'submit_event: erasure.shred targets unknown event % — nothing to shred here', b_clear -> 'payload' ->> 'target_event_id';
        END IF;
        IF NOT v_target_sealed THEN
            RAISE EXCEPTION 'submit_event: erasure.shred targets a non-sealed (plaintext) event % — crypto-shred can only erase a born-sealed body (no DEK to destroy; the body is in the append-only log). Refusing a false erasure (ADR-0052 §6)', b_clear -> 'payload' ->> 'target_event_id';
        END IF;
        PERFORM cairn_execute_shred(
            (b_clear -> 'payload' ->> 'target_event_id')::uuid,
            v_event_id, b_clear -> 'payload' ->> 'basis');
    END IF;

    RETURN v_event_id;
END;
$$;

-- The grant floor (C5.4 / ADR-0021): no direct event_log writes; the only door is
-- submit_event. The agent reads projections + the log, executes the door, nothing else.
REVOKE INSERT, UPDATE, DELETE ON event_log FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON event_log FROM cairn_agent;
-- The classification table is itself a safety surface: reclassifying a
-- suppressing op as additive would dodge the attestation gate. Lock it down;
-- submit_event reads it as its SECURITY DEFINER owner, so cairn_agent needs nothing.
REVOKE INSERT, UPDATE, DELETE ON event_type_class FROM PUBLIC;
-- submit_event is SECURITY DEFINER, so PUBLIC's default EXECUTE on a new function
-- would let *any* connected role drive the privileged write door (bypassing the
-- table REVOKEs above). Close that: only cairn_agent may knock.
REVOKE EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea, bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea, bytea) TO cairn_agent;
GRANT SELECT ON event_log, patient_chart, actor_current TO cairn_agent;

-- db/002's patient_chart projection rows (registered here: db/002 loads before
-- this registry exists). note.added is heal_safe=false BY SHAPE: note_count is
-- a counter — replaying an already-counted event would increment again. It
-- heals only via rebuild (truncate-then-replay). See ADR-0057.
--
-- DO UPDATE, not DO NOTHING (#214 idiom, see db/031's medication registrations):
-- the loader replays this file on every connect, so a stale/tampered row heals to the
-- migration text. The IS DISTINCT FROM guard keeps the steady-state replay write-free —
-- without it every connect rewrites all three rows (dead tuple + validate-trigger fire)
-- even when nothing changed.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('patient.created', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),
    ('patient.amended', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),
    ('note.added',      'patient_chart_apply', ARRAY['patient_chart'], 10, FALSE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
