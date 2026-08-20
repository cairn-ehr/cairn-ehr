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
-- `patient.created` was seeded here until issue #345 RETIRED it: an unfloored walking-skeleton
-- registration act cannot remain a permitted first event once db/047 turns on the §5.3/§5.8
-- precedence rule (step 8b below). Its removal from this seed covers a FRESH database; db/047
-- carries the DELETEs that converge one migrated in place.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
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
-- ---------------------------------------------------------------------------------
-- The cairn_check_* EXECUTE convention (#382), stated once here for the whole floor.
--
-- Postgres grants EXECUTE to PUBLIC by default and every role is a member of PUBLIC, so
-- an un-REVOKEd function is directly callable by anyone holding a connection. For this
-- family the severity is genuinely LOW: NONE of them writes and NONE of them grants, and a
-- caller invoking one learns strictly less than the door already tells it by refusing.
--
-- Say that precisely rather than as "pure jsonb shape validators that read no table", which
-- an earlier draft did and which is false for four of the twenty-two: the two registry
-- triggers (twin_registry_fn, projection_registry_fn) are RETURNS trigger and take no jsonb
-- body at all — the latter reads event_type_class; cairn_check_contributors reads
-- contributor_role; cairn_check_coding_object reads medication_coding_system. All three of
-- those tables are open vocabularies already SELECT-able by PUBLIC, so the conclusion holds —
-- but a rationale a reader cannot verify is worth no more than no rationale.
--
-- It is applied uniformly anyway, because the value is that the convention becomes
-- CHECKABLE. It was previously followed by 5 of 22 functions, and a reader could not tell
-- an oversight from a decision — on the §9 safety-critical surface, "looks inconsistent,
-- probably fine" is the wrong resting state. crates/cairn-node/tests/floor_execute_grants.rs
-- asserts it over the catalogue, so the next omission fails a test instead of going unnoticed.
--
-- Distinct from, and much less load-bearing than, the same REVOKE on the projection
-- appliers: an applier WRITES a projection table, so one callable by the runtime role would
-- let it forge projection state no event in the append-only log supports. Same statement, two
-- different reasons. Do not collapse them into one rule of thumb.
--
-- The two families are also IDENTIFIED differently by the guard, and for a reason worth
-- knowing: appliers are read from the cairn_projection_apply REGISTRY (the authoritative list
-- the dispatcher actually calls), while this family is read from its NAME PREFIX, because it
-- has no registry — plenty of members, like cairn_check_coding_object, are helpers no
-- registration ever mentions. A validator renamed out of the prefix therefore leaves the
-- family silently; an applier cannot leave the registry and stay reachable.
--
-- A THIRD family, of one, was added by #443: cairn_event_twin, the dispatcher that routes an
-- event type to whichever of these validators the registry names for it. (Not to all of them —
-- as the paragraph above says, several prefix members are helpers no registration mentions. The
-- registry names 16 distinct check_fns across 24 rows; the REVOKE convention covers all 22.)
-- Its REVOKE and the reasoning for it sit next to its definition below, not here, because the
-- safety argument is about ITS callers rather than about this convention.
-- ---------------------------------------------------------------------------------
REVOKE EXECUTE ON FUNCTION cairn_check_twin_registry_fn() FROM PUBLIC;

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
-- `SET search_path = public, pg_temp` pinned (same discipline as cairn_event_twin below):
-- the to_regprocedure/to_regclass resolution must never be shadowed by a caller's
-- search_path, regardless of who fires this validation trigger. pg_temp is listed LAST
-- deliberately — see the house-rule note on cairn_node_hlc_merge (db/001, #426).
CREATE OR REPLACE FUNCTION cairn_check_projection_registry_fn()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
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
    -- ADR-0056 decision 4 (issue #266): a projection-registered type MUST be classified.
    --
    -- The remote door admits an UNCLASSIFIED type uninterpreted (db/020, #265) and records
    -- its event_deferred marker AFTER the event_log INSERT — but the AFTER-INSERT
    -- dispatcher fires DURING that INSERT. So a type registered here without an
    -- event_type_class row would be projected at admission, granting exactly the power the
    -- marker exists to withhold. Making that unreachable at migration time is cheaper and
    -- safer than defending against it at runtime.
    --
    -- It is one of THREE things bounding which apply fns run against a deferred row. The
    -- other two are cairn_replay_eligible below (no reprojection path can reach one) and
    -- db/043's gate 4, which DELIBERATELY runs a promoted event's heal-safe apply fns while
    -- its event_deferred marker is still present — that is how promotion proves the event
    -- can project before the marker is deleted (PR #302 finding F1).
    --
    -- So "no apply fn ever sees a deferred row" is NOT true, and must not be assumed. What
    -- IS true is that no apply fn currently READS that state: db/018 (patient_link_apply)
    -- and db/034 (medication_attestation_apply) can keep treating event_log.attester_key as
    -- a vouch because they exclude event_attestation_unvouched (db/001), which is keyed on
    -- the token's verification, not on the deferral. A future apply fn that defensively
    -- skips or asserts on a deferred row would misbehave under gate 4 — raising flags the
    -- event unpromotable forever, and a silent no-op promotes it unprojected, because the
    -- loader no longer runs a targeted reproject after the pass.
    --
    -- Runs LAST of the three independent fail-closed validations. The order carries no
    -- safety meaning (any one of them refusing is enough), but it is not arbitrary either:
    -- the apply_fn and projection_table checks predate this one and their refusals are
    -- pinned by name in projection_registry.rs, so a registration that is wrong in more
    -- than one way keeps reporting the SAME reason it always did.
    --
    -- HONEST RESIDUAL, worth knowing before relying on this: the check runs at REGISTRATION
    -- time, so it cannot see a class row deleted AFTERWARDS. A registered-but-unclassified
    -- type would leave the AFTER-INSERT dispatcher firing for a deferred event, because the
    -- dispatcher reads cairn_projection_apply and never consults event_type_class. The state
    -- is unreachable in practice for two reasons — a type's classification and its
    -- projection registration arrive in the SAME migration, and the one migration that DOES
    -- delete from event_type_class obeys the retirement order below — and event_type_class is
    -- REVOKEd from PUBLIC, so only an owner could create it by hand. Same privilege tier as
    -- cairn_reproject.
    --
    -- RETIREMENT ORDER (issue #345, db/047 — the first and so far only migration to delete a
    -- classification): a migration retiring a type must DELETE its cairn_projection_apply rows
    -- FIRST and its event_type_class row second. Reversed, it would leave precisely the
    -- registered-but-unclassified state this check exists to make unreachable, and no later
    -- validation would notice. db/047 is the worked precedent; copy its order.
    IF NOT EXISTS (SELECT 1 FROM event_type_class WHERE event_type = NEW.event_type) THEN
        RAISE EXCEPTION
            'cairn_projection_apply: event_type "%" is not classified in event_type_class (fail closed) — classify it before registering a projection, or the dispatcher would project an event admitted uninterpreted',
            NEW.event_type;
    END IF;
    RETURN NEW;
END;
$$;
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382,
-- convention stated in db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_projection_registry_fn() FROM PUBLIC;

DROP TRIGGER IF EXISTS cairn_projection_apply_validate ON cairn_projection_apply;
CREATE TRIGGER cairn_projection_apply_validate
    BEFORE INSERT OR UPDATE ON cairn_projection_apply
    FOR EACH ROW EXECUTE FUNCTION cairn_check_projection_registry_fn();

-- Safety surface: a row pointing a type's projection at a no-op would silently
-- stop materialization. Locked down like cairn_event_twin_check.
REVOKE INSERT, UPDATE, DELETE ON cairn_projection_apply FROM PUBLIC;

-- The #266 safety seam (ADR-0056 decision 4): cairn_reproject (db/039) routes every
-- candidate event through this predicate, so NO reprojection path — the loader's heal,
-- the `cairn-node reproject` CLI, or a hand-run mid-upgrade replay — can grant power to
-- an event whose classification-gated floor checks have never been run.
--
-- A deferred event is one the remote door admitted UNINTERPRETED (db/020, issue #265).
-- Its marker is DELETED by cairn_readjudicate_deferred (db/043) only after the deferred
-- gates pass, so "carries no marker" IS "adjudicated". An event that FAILS adjudication
-- keeps its marker with the reason recorded and stays powerless — never silently promoted.
--
-- The live-insert path needs no filter: an event being inserted through a door was
-- adjudicated by that door.
--
-- LANGUAGE sql (not plpgsql) deliberately: it inlines into cairn_reproject's per-type scan
-- as an anti-join rather than costing a function call per replayed event — and the replay
-- scan runs over the WHOLE log (2M events on the Pi5 bench). event_deferred is created in
-- db/001, not in db/043 with the rest of this mechanism, precisely so this body resolves at
-- CREATE time: SQL-language bodies are parsed and name-resolved when the function is
-- created, unlike PL/pgSQL's late binding.
CREATE OR REPLACE FUNCTION cairn_replay_eligible(e event_log)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT NOT EXISTS (SELECT 1 FROM event_deferred d WHERE d.event_id = e.event_id)
$$;
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
-- `SET search_path = public, pg_temp` pinned here exactly like cairn_event_twin's dynamic
-- dispatch further down this file: the %I-quoted apply_fn EXECUTE must never resolve
-- into an attacker-shadowed schema regardless of who/what fired the AFTER INSERT
-- trigger that invokes this function — the dynamic-dispatch safety argument stays
-- self-contained here, not dependent on the firing role's search_path. On why pg_temp
-- is named and named last, see the house-rule note on cairn_node_hlc_merge (db/001).
-- (ADR-0048 quotes this clause in its pre-#426 spelling, `= public`; ADRs are immutable,
-- so treat the spelling there as history and this note as the rule.)
CREATE OR REPLACE FUNCTION cairn_projection_dispatch()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
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
-- `SET search_path = public, pg_temp` is pinned on THIS function (not only on the SECURITY
-- DEFINER doors that call it), so the %I identifier can never be resolved into an
-- attacker-shadowed schema regardless of who invokes the hook — the dynamic-dispatch safety
-- argument is self-contained here, not dependent on the caller's search_path (defense in
-- depth: today the only callers are submit_event/apply_remote_event, which already pin it).
-- On why pg_temp is named and named last, see the house-rule note in db/001 (#426).
--
-- Twin policy (ADR-0039): an authored twin is carried verbatim for EVERY type; if absent,
-- a type with twin_required_msg RAISES (demographics + identity + medication hard-require
-- it), and every other type degrades honestly to a mechanical skeleton.
-- The ONE blank-test for a §3.13 plaintext legibility twin: did the author supply text, or
-- must the floor derive a twin? Declared here (the twin door) and called by the write gate
-- below AND by the db/015 read predicates, so the question is spelled exactly once in SQL.
--
-- WHY AN EXPLICIT CODE-POINT SET AND NOT `\s` (issue #75). Postgres's `\s` is `[[:space:]]`,
-- and membership of that class is decided by the COLLATION's ctype: under a libc UTF-8
-- collation `iswspace(U+00A0)` is true, under `C` / `ucs_basic` it is false. That made the
-- floor's verdict depend on how the database happened to be created — and since
-- cairn_event_twin is also the remote-apply gate (db/020) and RAISEs for a hard-require
-- type, the SAME signed event could apply on one node and raise on another. That is a
-- set-union convergence break, so the test has to be a property of the bytes alone.
--
-- The set below is exactly the 25 code points with Unicode `White_Space=Yes`, which is
-- precisely what Rust's `char::is_whitespace()` — hence `str::trim()`, hence
-- `cairn_event::twin_is_present` — matches. Two nodes therefore agree, and so do the Rust
-- and SQL halves of the floor. Deliberately EXCLUDED, because Unicode and Rust exclude them:
-- U+200B ZERO WIDTH SPACE and U+FEFF BOM are `White_Space=No`, so a twin of those is
-- *present* on both sides.
--
-- `btrim(t, set)` is the literal mirror of Rust's `t.trim().is_empty()`: strip any leading or
-- trailing character in the set and ask whether anything survived. It compares characters
-- directly — no locale, no regex class — which is what makes the answer identical on every
-- node. The code points are written as `U&'\XXXX'` escapes rather than pasted literally so a
-- reviewer can SEE them: a pasted NO-BREAK SPACE is indistinguishable from a normal space in
-- source. (`U&''` requires standard_conforming_strings=on, the default; with it off this
-- migration fails loudly at load rather than silently mis-parsing.) Verified against every
-- BMP code point by crates/cairn-node/tests/twin_blank_parity.rs and
-- db/tests/044_twin_blank_unicode_test.sql.
CREATE OR REPLACE FUNCTION cairn_twin_is_present(p_twin text)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT p_twin IS NOT NULL
       AND btrim(p_twin,
                    -- TAB, LF, VT, FF, CR, SPACE, NEL, NO-BREAK SPACE, OGHAM SPACE MARK
                    U&'\0009\000A\000B\000C\000D\0020\0085\00A0\1680'
                    -- EN QUAD .. HAIR SPACE
                 || U&'\2000\2001\2002\2003\2004\2005\2006\2007\2008\2009\200A'
                    -- LINE SEP, PARAGRAPH SEP, NARROW NBSP, MEDIUM MATH SPACE, IDEOGRAPHIC SPACE
                 || U&'\2028\2029\202F\205F\3000') <> '';
$$;

CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_twin     text    := b ->> 'plaintext_twin';
    v_authored boolean := cairn_twin_is_present(v_twin);
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

-- #443 — the dispatcher joins the REVOKE convention its cairn_check_* siblings follow.
--
-- Until 2026-08-21 cairn_event_twin kept Postgres's default EXECUTE-to-PUBLIC while all 22 of
-- the cairn_check_* functions were revoked. That failed CLOSED (a PUBLIC caller reached the
-- dispatcher and was refused one layer deeper, by "permission denied for function
-- cairn_check_…"), so nothing leaked and nothing was writable. It was still the wrong resting
-- state, for the reason the convention block above gives: a rule followed by every member of a
-- family EXCEPT its entry point is a rule a reader cannot tell from an oversight, and the
-- refusal it produced named which validator an event type maps to — from the wrong layer.
--
-- Safe because no live caller needs a grant, and the three kinds of caller are worth naming so
-- a future one is checked against them rather than assumed: submit_event (below) and
-- apply_remote_event (db/020) are SECURITY DEFINER and run as this schema's owner;
-- cairn_readjudicate_deferred (db/043) is invoker-rights but runs during schema load as the
-- owner, and is itself already REVOKEd FROM PUBLIC. A future INVOKER-rights caller reachable by
-- cairn_agent or cairn_node would need an explicit GRANT here — that is the one change that
-- would turn this line into a breakage, and it should be made deliberately, not by deleting
-- this REVOKE.
--
-- Asserted over the catalogue by crates/cairn-node/tests/floor_execute_grants.rs
-- (public_cannot_execute_the_twin_dispatcher).
REVOKE EXECUTE ON FUNCTION cairn_event_twin(text, jsonb) FROM PUBLIC;

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
        -- ADR-0056 (issue #265, PR #302 review finding F2): count this arm only when the
        -- stored token has actually been VOUCHED.
        --
        -- The remote door stores a deferred event's travelling token without verifying it
        -- — it cannot, because the gate that verifies it is deferred with the
        -- interpretation. Unioning an unverified key here would let a hostile peer put ANY
        -- key it likes inside the target's human-author set simply by attaching a forged
        -- token to an unknown-type event, and its holder could then suppress that event.
        -- That is over-permission on the ADR-0043 floor, which this function's header
        -- forbids in exactly those words.
        --
        -- WHY NOT "is the target deferred?", which is what this originally asked: that is a
        -- PROXY with the wrong lifetime. cairn_readjudicate_deferred (db/043) verifies a
        -- token only when the type's mode DEMANDS one, so an additive event bearing no
        -- responsibility is promoted — event_deferred row deleted — with its token never
        -- checked. The proxy said "vouched" the instant the marker vanished. The marker
        -- below survives promotion and is cleared only by gate 1 actually verifying, so it
        -- answers the question this arm means to ask.
        --
        -- The fix is NEUTRAL, not merely stricter: for a target signed by an AGENT, dropping
        -- this arm empties human_authors and the gate OPENS (the agent-advisory-is-
        -- dismissable rule below). That is correct — an unverified token must not move the
        -- gate in EITHER direction.
        --
        -- Two other readers of event_log.attester_key owe the same exclusion:
        -- patient_link_apply (db/018) and medication_attestation_apply (db/034), both
        -- fixed in the commits that follow this one. A new reader of these columns owes
        -- the same choice.
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
          AND cairn_attestation_vouched(p_target)
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
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382,
-- convention stated in db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_suppression_overlay(text, jsonb) FROM PUBLIC;

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
-- `SET search_path = public, pg_temp` is pinned HERE, not only on the SECURITY DEFINER
-- doors that call it (the cairn_event_twin discipline): the contributor_role lookup must
-- never resolve into a caller-shadowed schema, regardless of who invokes the check. That
-- includes the caller's TEMP schema, which the bare `public` form leaves searched FIRST —
-- see the house-rule note on cairn_node_hlc_merge (db/001, #426).
CREATE OR REPLACE FUNCTION cairn_check_contributors(b jsonb, p_door text, p_strict boolean)
RETURNS void LANGUAGE plpgsql STABLE
SET search_path = public, pg_temp
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
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382,
-- convention stated in db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_contributors(jsonb, text, boolean) FROM PUBLIC;

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
SET search_path = public, pg_temp
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
-- ADR-0064 — what makes a claim AUTHORITATIVE. The read-side twin of
-- cairn_authorship_bound directly above: that one REFUSES at authoring, this one
-- GRADES at read, and the two must stay in lockstep (the same note
-- crates/cairn-event/src/contributor.rs:118 already carries).
--
-- Authority is a HUMAN ACTOR THIS NODE CAN HOLD RESPONSIBLE — never the relaying
-- machine, never the actor's relationship to the chart. Two sufficient routes:
--   R1 'attested' — a VOUCHED attestation whose attester is an enrolled human actor.
--   R2 'self'     — the claim's actor IS the target's actor, both known, and human.
-- Everything else is 'unverified'.
--
-- !! SECURITY DEFINER IS REQUIRED, NOT STYLISTIC !!
-- cairn_attestation_vouched (db/001) is REVOKEd FROM PUBLIC and event_attestation_unvouched
-- carries no SELECT grant, because db/001 reasons that every caller is a SECURITY DEFINER
-- door or a migration-owned trigger. This caller is NEITHER: cairn_sensitivity_standing
-- (db/048) is a plain LANGUAGE sql function granted to cairn_agent, and a non-definer body
-- runs as the CALLING role whether or not it inlines. Without DEFINER the first
-- cairn_effective_sensitivity call by cairn_agent fails with permission denied — the whole
-- sensitivity read path, broken by a privilege, and ONLY under the product's role.
--
-- Pinned by TWO role-switched tests, and the order matters if either is ever simplified:
--   * claim_authority_worklist::the_worklist_is_readable_as_cairn_agent — the STRONGER pin.
--     Its fixture carries a real inert withdrawal, so the predicate's body actually RUNS
--     under cairn_agent against live data.
--   * claim_authority::the_read_path_works_as_cairn_agent — the WEAKER pin. Its chart
--     carries no withdrawal, so the seam's NOT EXISTS matches nothing and the body never
--     executes; what it pins is Postgres's executor-start ACL check alone. Still a real
--     guard, but it would stay green if only SECURITY DEFINER were removed (the `SET
--     search_path` clause independently blocks inlining).
-- This header used to name only the weaker one (#410 review finding A3). Re-anchor to
-- whichever still lands real data through the role switch, never silently drop.
--
-- !! `attester_key IS NOT NULL` IS THE ACTUAL R1 TEST !!
-- cairn_attestation_vouched returns TRUE for an event carrying NO attestation, because
-- "vouched" means "no unvouched MARKER row exists". Delete the NULL guard and every
-- unattested event in the log grades 'attested'. Pinned by
-- an_event_with_no_attestation_at_all_is_unverified.
--
-- STRICTER THAN db/020's SIBLING, DELIBERATELY. db/020's forged-human-author check asks
-- `EXISTS (... AND kind='human')`, which admits a key mapped to BOTH a human and an agent.
-- Here that ambiguity would confer power to strip a protective grade, so the key must
-- resolve to EXACTLY ONE actor and that actor must be human. Principle 4: uncertainty
-- withholds power, it never confers it. Not a fix to db/020 — a different question.
--
-- FIXED ARITY. Never widen this argument list: Postgres OVERLOADS on a changed list rather
-- than replacing, and migration replay never drops what a file stops creating, so a stale
-- definition would survive in every existing database and silently serve any call site
-- missed — including a has_function_privilege pin, which would resolve the STALE signature
-- and pass. A caller with no target passes an explicit NULL. (The hazard's provenance is
-- this file's own `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea)` below, forced
-- when ADR-0052 added p_dek, and db/020:55; db/049:345 states it as verified. It is NOT
-- #404 — that issue is the prospective/effective mirrored-predicate divergence, a different
-- hazard entirely; the attribution came in with this slice's design and is corrected here.)
CREATE OR REPLACE FUNCTION cairn_claim_authority(p_event_id uuid, p_target_event_id uuid)
RETURNS text LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $$
    SELECT CASE
        -- R1 — a vouched attestation by an enrolled human actor.
        WHEN EXISTS (
            SELECT 1 FROM event_log e
             WHERE e.event_id = p_event_id
               AND e.attester_key IS NOT NULL          -- load-bearing; see header
               AND cairn_attestation_vouched(e.event_id)
               AND (SELECT count(*) = 1 AND bool_and(a.kind = 'human')
                      FROM actor_current a
                     WHERE a.signing_key_id = encode(e.attester_key, 'hex')))
        THEN 'attested'
        -- R2 — the human who made the claim is withdrawing their own.
        WHEN p_target_event_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM event_log c
              JOIN event_log t ON t.event_id = p_target_event_id
              JOIN actor_current a ON a.actor_id = c.actor_id
             WHERE c.event_id = p_event_id
               AND c.actor_id IS NOT NULL              -- NULL = a key on several actors,
               AND t.actor_id IS NOT NULL              -- i.e. attribution honestly unknown
               AND c.actor_id = t.actor_id
               AND a.kind = 'human')                   -- ADR-0062 decision 6
        THEN 'self'
        ELSE 'unverified'
    END;
$$;
-- A SECURITY DEFINER function with PUBLIC execute is a privilege-escalation surface.
REVOKE EXECUTE ON FUNCTION cairn_claim_authority(uuid, uuid) FROM PUBLIC;
GRANT  EXECUTE ON FUNCTION cairn_claim_authority(uuid, uuid) TO cairn_agent;

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

-- `SET search_path = public, pg_temp` — pg_temp LAST, and this is one of the two functions
-- where that is load-bearing rather than hygiene. This body runs as the migration OWNER and
-- INSERTs into an unqualified `event_log`; under the bare `public` form a caller holding only
-- EXECUTE here could shadow that name and take the owner-privileged write into their own temp
-- table while this door still returned an event id. See the house-rule note on
-- cairn_node_hlc_merge (db/001, #426) for the mechanism and the demonstration.
CREATE OR REPLACE FUNCTION submit_event(
    p_signed       BYTEA,
    p_attestation  BYTEA DEFAULT NULL,
    p_attester_key BYTEA DEFAULT NULL,
    p_dek          BYTEA DEFAULT NULL
) RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
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
    --      FLAGGED, never rejected (principle 4 — a slow/dead clock must not force fabrication).
    --      The door gates EFFECT not PRESENCE (ADR-0056 / ADR-0058 decision 5): a missing or
    --      unrecognized grade is admitted as/like 'unknown', never refused.
    v_grade := COALESCE(b ->> 'clock_grade', 'unknown');

    --      Mint constraint (ADR-0058 decision 1, floor-enforced — PR #285 review finding 1):
    --      self-asserted is the SOLE grade any node may AUTHOR this slice — no verified clock
    --      source exists yet, so a RATIFIED grade above rank 1 arriving at the LOCAL authoring
    --      door can only be a forged trust brand: a hostile enrolled writer (Spike-0002 threat
    --      model) claiming e.g. 'multi-anchor-corroborated' would mint a falsely TRUSTED
    --      timestamp — the exact fraud the grade exists to brand away ("you cannot forge a
    --      trusted timestamp on an untrusted clock" holds only if this gate does). Strict-submit /
    --      lenient-apply (ADR-0051): the REMOTE door (db/020) admits any grade verbatim, since a
    --      future upgraded peer mints higher grades legitimately. An unrecognized value ranks 0
    --      and passes this gate (decision 5, above). When #279's verified producers land, this
    --      refusal is replaced by anchor-token verification of the claimed grade.
    IF cairn_clock_grade_rank(v_grade) > 1 THEN
        RAISE EXCEPTION 'submit_event: clock_grade "%" is not mintable — no verified clock source exists this slice; only unknown/self-asserted may be authored (ADR-0058 mint constraint, #279)',
            v_grade;
    END IF;

    --      The classifier's 'reject' arm is now unreachable at THIS door (the mint gate above
    --      refuses every grade that could produce it); it is kept live for when #279 makes high
    --      grades mintable, and stays covered by the SQL truth table (db/tests/040).
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

    -- 1d. §5.9 safety-signal shape (ADR-0063). LOCAL DOOR ONLY — db/020 deliberately does
    --     NOT call this. The signal is a FIELD on a clinical event, so a refusal at the
    --     apply door would drop the clinical event it rides on; a defect in a de-identified
    --     advisory field must never cancel clinical content (ADR-0060). See db/049 section 4.
    PERFORM cairn_check_safety_signal(b);

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

    -- 7a. #405 part 2 / ADR-0064: an emitted rung FINER than this chart's grade licenses
    --     is RECORDED, never refused (db/049's `safety_overclaim_flag`/
    --     `cairn_record_safety_overclaim_flag` — see that file for why the block is
    --     LOCAL-DOOR-ONLY, deliberately breaking the `cairn_record_ceiling_flag`
    --     precedent it otherwise copies).
    --
    --     PLACEMENT, AND WHY NOT BESIDE STEP 1d (2026-08-15 review, Critical #1). This
    --     check must reproduce emission's OWN grade lookup exactly —
    --     `crate::safety::prospective_rung`, called from `apply_safety_rung`
    --     (sealed_submit.rs) with the thread read out of `payload.medication_id` BEFORE
    --     the body is sealed. A first version of this block sat beside step 1d and passed
    --     `p_thread = NULL` unconditionally, on the mistaken belief that the clear
    --     payload was unreadable that early. It is not: b_clear, built by step 7 just
    --     above, is exactly that clear view — the block simply ran before step 7 did.
    --     Passing NULL coarsens the LICENSED rung using ANY thread-scoped grade standing
    --     ANYWHERE on the chart (db/049 section 6's catch-all arm), not only a grade on
    --     the thread this event is actually on — so an ordinary, correctly-licensed
    --     `precise` emission on an UNGRADED thread of a chart carrying some OTHER
    --     thread's grade was recorded as an overclaim it never made
    --     (crates/cairn-node/tests/safety_overclaim.rs's
    --     `a_thread_scoped_grade_elsewhere_on_the_chart_does_not_false_flag_this_threads_precise_emission`
    --     pins this — it exercises the real `assert_medication` emission path, not the
    --     raw-safety bypass the rest of that file uses). Coarsening the licensed rung is
    --     the SAFE direction for EMISSION's own rung (over-disclosure is the one
    --     unrecoverable error, db/049 section 6's own "asymmetry that matters"); on a
    --     DETECTOR the identical move is a false positive, and a ledger whose rows are
    --     mostly false accusations against the daemon's own correct output is worse than
    --     no ledger — eventually nobody reads it. So this block runs AFTER step 7,
    --     reading the SAME field from b_clear that apply_safety_rung read pre-seal —
    --     exact parity, not a one-sided "conservative" bound.
    --
    --     pg_input_is_valid, not a bare `::uuid` cast, mirrors sealed_submit.rs's own
    --     `.and_then(|s| s.parse::<uuid::Uuid>().ok())`: an absent OR malformed
    --     medication_id degrades to NULL on both sides, rather than raising here and
    --     having the WHOLE check swallowed by the handler below for a reason unrelated to
    --     the lookup it exists to protect.
    --
    --     MUST NOT FAIL A CLINICAL WRITE (ADR-0063 decision 8, stated categorically — and
    --     the ADR records the real incident this repeats otherwise: an earlier safety
    --     lookup propagated its error with a bare call, so a missing grant or a statement
    --     timeout aborted the MEDICATION ASSERTION over a safety class no clinician
    --     caused). Everything inside — the thread lookup, the grade lookup, both rank
    --     lookups, the flag insert — runs inside its OWN nested DECLARE/BEGIN/END block
    --     with NO exception clause of its own, wrapped by the OUTER block's
    --     `EXCEPTION WHEN query_canceled OR OTHERS`: a raise DURING a DECLARE initializer
    --     is caught only by an ENCLOSING block's handler, never its own (verified — this
    --     is what makes the inner/outer split load-bearing rather than decorative), so any
    --     raise anywhere in here — a missing grant, a NULL where a row was expected — is
    --     swallowed and the write proceeds. An unrecorded overclaim is a bounded loss; a
    --     refused medication assert is not.
    --
    --     A TIMEOUT NEEDS ITS OWN CONDITION NAME and does not ride `OTHERS` — see the
    --     handler's own comment below for why, and do not simplify it back. This sentence
    --     used to claim a bare `WHEN OTHERS` swallowed "a timeout"; it did not, and the
    --     block reproduced the very incident it cites (#410 review finding C2).
    --
    --     Still sits before the event_log INSERT below (unlabeled, between steps 9 and
    --     10): a LATER refusal anywhere else in this function — steps 8/8a/8b/9 before
    --     the INSERT, step 10 after it — rolls this block's flag insert back with
    --     everything else in the transaction (the whole call is one implicit transaction;
    --     an uncaught RAISE aborts all of it, not merely what follows the raise), so a
    --     flag is never recorded for an event that was ultimately refused for an
    --     unrelated reason.
    IF b -> 'safety' ->> 'rung' IS NOT NULL THEN
        BEGIN
            DECLARE
                -- The thread apply_safety_rung read pre-seal, recovered the same way:
                -- payload.medication_id off the CLEAR view, degrading to NULL when absent
                -- or malformed (never raising — see the placement note above).
                v_thread_raw text := b_clear -> 'payload' ->> 'medication_id';
                v_thread     uuid;
                -- The rung THIS chart's grade licenses right now, computed the same
                -- composition emission does (crate::safety::prospective_rung / db/049
                -- section 6): cairn_safety_rung_for_rank(cairn_sensitivity_rank(grade)).
                v_licensed   text;
            BEGIN
                IF v_thread_raw IS NOT NULL AND pg_input_is_valid(v_thread_raw, 'uuid') THEN
                    v_thread := v_thread_raw::uuid;
                END IF;

                SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank(g.grade))
                  INTO v_licensed
                  FROM cairn_prospective_sensitivity((b ->> 'patient_id')::uuid, v_thread) g;

                -- Lower rank = FINER = discloses MORE (db/049 section 2: precise=0 <
                -- kind=10 < existence=20). An emitted rung ranked below what the chart
                -- licenses discloses more than the grade allows — the overclaim direction.
                IF cairn_safety_rung_rank(b -> 'safety' ->> 'rung')
                 < cairn_safety_rung_rank(v_licensed) THEN
                    PERFORM cairn_record_safety_overclaim_flag(
                        v_ca, (b ->> 'patient_id')::uuid,
                        b -> 'safety' ->> 'rung', v_licensed);
                END IF;
            END;
        EXCEPTION WHEN query_canceled OR OTHERS THEN
            -- Advisory ledger entry only — never allowed to fail a clinical write
            -- (ADR-0063 decision 8). Logged so a lookup failing on every write is visible
            -- operationally (mirrors crate::safety::advisory_or_withheld's eprintln), but
            -- the write itself proceeds regardless.
            --
            -- `query_canceled` IS NAMED EXPLICITLY, and dropping it back to a bare
            -- `WHEN OTHERS` reopens #410 finding C2. PostgreSQL matches `OTHERS` against
            -- every error type EXCEPT `query_canceled` (57014) and `assert_failure`, so a
            -- blanket handler does NOT absorb a `statement_timeout` — the propagating
            -- cancel aborts submit_event and REFUSES the medication assert. That is
            -- ADR-0063 decision 8's originating incident exactly ("a missing grant or a
            -- statement timeout aborted the MEDICATION ASSERTION"), and it needs only two
            -- ordinary conditions to co-occur: a deployment that sets statement_timeout,
            -- and a populated safety_class_map. Pinned by safety_overclaim.rs's
            -- `a_stalled_grade_lookup_under_a_statement_timeout_still_admits_the_medication`.
            --
            -- THE TRADE THIS MAKES, stated rather than left implicit: catching 57014 also
            -- absorbs an operator's deliberate cancel for the remainder of this call, and
            -- statement_timeout's timer does not re-arm once it has fired, so the rest of
            -- submit_event (the event_log INSERT and step 10) then runs untimed. That
            -- residue is bounded and accepted: availability over consistency (§1), and a
            -- refused medication assert is the failure this floor exists to prevent.
            -- `assert_failure` is deliberately NOT named — a failed ASSERT is a floor
            -- invariant violation, which must abort, never be swallowed as advisory noise.
            RAISE WARNING 'submit_event: safety-overclaim check failed for %, continuing without recording it (advisory, never fails a clinical write — ADR-0063 decision 8): %',
                v_ca, SQLERRM;
        END;
    END IF;

    -- 8. Plaintext twin (§3.13/§4.5) + any per-type structural floor, via the
    --    cairn_event_twin hook so a new event type adds its branch there, not by
    --    re-declaring this whole door. Runs on the CLEAR view so a sealed body's
    --    structural floor is checked on its real payload, never the ciphertext.
    v_twin := cairn_event_twin(v_type, b_clear);

    -- 8a. The §5.9 sensitivity ceremony (ADR-0062, issue #232 part A/db/048): raising a
    --     grade is frictionless, but a CHART-WIDE raise states why, and a WITHDRAWAL
    --     (lowering) needs a bound human author. Grouped here with step 8, right after the
    --     twin/structural-floor dispatch, because this is one more judgement about the
    --     EVENT ITSELF — same footing as step 8b's own list (signature/clock/contributors/
    --     actor/attestation/seal/twin/structural floor) — so it belongs before step 8b's
    --     chart-history check, not after it (8b's own rule: the event's own defects are the
    --     author's first problem; the chart's history is the second).
    --
    --     v_att_key is the value db/005 already computes for step 4b's
    --     cairn_authorship_bound call: NULL unless a valid attestation token from an
    --     enrolled HUMAN actor was verified for this event (step 4). Reusing it here rather
    --     than re-resolving the attester keeps this call a read of state the door already
    --     established, not a second, possibly-drifting lookup.
    --
    --     STRICT DOOR ONLY — see cairn_sensitivity_ceremony_ok's own header (db/048) for why
    --     apply_remote_event must NEVER call this: a door check at APPLY would let one
    --     peer's honestly rationale-less act be refused by another peer's stricter node,
    --     forking the event set and wedging replication, and for a RAISE specifically it
    --     would be worse than a wedge — refusing a peer's protective assertion would leave
    --     THIS node computing a LOWER grade than the peer already holds, so the refusal
    --     would itself be a disclosure (ADR-0060, the #342 trap).
    --
    --     b_clear, NOT b — DELIBERATE: identical today because step 7 refuses a
    --     sealed non-clinical body, so a sensitivity event's b_clear always equals b — but
    --     reading b here is a latent fail-open, not a no-op. If seal policy ever widens,
    --     `p ->> 'subject_kind'` would read CIPHERTEXT off b, jsonb_typeof would see NULL,
    --     and this gate would silently PASS the chart-wide-raise/withdrawal-authorship
    --     checks — the disclosure direction. Passing b_clear (the CLEAR view every other
    --     event-shape check in this door already reads) keeps this check correct under a
    --     seal-policy change instead of merely correct today.
    PERFORM cairn_sensitivity_ceremony_ok(v_type, b_clear, v_att_key);

    -- 8b. The §5.3/§5.8 PRECEDENCE RULE (ADR-0061 decision 3, issue #345): the first event
    --     carrying a patient_id must be that chart's registration. This is what makes the
    --     search-before-create funnel unbypassable — without it a client mints a chart simply
    --     by asserting a name, and §5.8's obligation to record the search that preceded the
    --     create has nothing to attach to.
    --
    --     ONE RULE, NO "UNLESS". Every registration class rides one event type (§5.3, ADR-0061
    --     decision 2) precisely so this sentence needs no carve-out — not for John Doe, not for
    --     the legacy patient.created (retired in db/047). An "unless" in a safety floor is
    --     where the next defect lives.
    --
    --     PLACEMENT is deliberate: after every check that judges the EVENT itself (signature,
    --     clock, contributors, actor, attestation, seal, twin, structural floor) and before
    --     anything is written. A defect in the event is the author's first problem; the chart's
    --     history is the second.
    --
    --     It is NOT the last refusal in this function, and a check added after it does not
    --     inherit this position. Four refusals still follow, and none of them judges the event:
    --       * step 9 refuses a sealed submit when THIS NODE has no registered unwrap key — a
    --         node-configuration failure, not an event defect. It sits with the custody writes it
    --         guards rather than up here, so the check and the INSERT it protects stay adjacent.
    --       * the post-INSERT arm refuses a substitution: it can only compare against what the
    --         log already holds, so it cannot run before the INSERT at all.
    --       * step 10's two erasure refusals judge the shred TARGET's presence and sealedness —
    --         again the log's state, read after the tombstone itself has been admitted.
    --     A NEW check belongs HERE if it judges the event; if it judges the node's configuration
    --     or the log's contents, it belongs down there, next to the write it guards.
    --
    --     All four are therefore SHADOWED on a chart-less write: a raw-SQL shred whose envelope
    --     names an unregistered patient now reports this rule rather than "targets unknown event".
    --     That is intended. Ordering among refusals is a legibility property, never a safety one
    --     — every path here refuses, nothing is written either way — and an event that is wrong
    --     in two ways keeps reporting the reason it always reported.
    --
    --     STRICT DOOR ONLY. db/020 (apply_remote_event) must NEVER call this: set-union sync
    --     has no ordering, so a peer's clinical event legitimately precedes the registration
    --     that licenses it, and a fail-closed remote door would freeze the puller's watermark
    --     on entirely honest traffic. Same strict-submit/lenient-apply shape as ADR-0051, and
    --     the same lesson as ADR-0056 / ADR-0058 / issue #268. The rule is self-satisfying
    --     afterwards: once a peer's event has landed, a local write to that chart is no longer
    --     a FIRST event, so the lenient admission costs no later refusal.
    --
    --     Scoped to the ENVELOPE's patient_id, never to a patient named in a payload (an
    --     identity.link's target chart may be remote-only and legitimately unregistered here).
    --
    --     WHY THE LENIENT REMOTE DOOR DOES NOT REOPEN THE BYPASS (principle 12): a client role
    --     cannot reach it. cairn_agent has no EXECUTE on apply_remote_event (db/020) and no
    --     INSERT on event_log (db/001), so THIS function is the only way a client writes at all
    --     — which is what makes the rule unbypassable even for one talking raw SQL. The lenient
    --     door is the sync daemon's (cairn_node), carrying events another node already accepted.
    IF v_type <> 'identity.registration.asserted'
       AND NOT cairn_patient_has_events((b ->> 'patient_id')::uuid) THEN
        RAISE EXCEPTION 'submit_event: no chart exists for patient % — the first event on a chart must be its registration (identity.registration.asserted, §5.3/§5.8); register the patient before recording anything about them',
            b ->> 'patient_id';
    END IF;

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
         clock_grade, safety)
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
        v_grade, b -> 'safety')
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
-- `patient.created`'s row is gone with the type (#345, see the event_type_class seed above);
-- `identity.registration.asserted` takes over the chart-birth projection, registered in db/047
-- because the type is not classified until db/045 — later in the replay order than this file.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('patient.amended', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),
    ('note.added',      'patient_chart_apply', ARRAY['patient_chart'], 10, FALSE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
