-- db/017_match_proposal.sql
-- §5.2 advisory match-proposal worklist (matcher piece B2 output).
--
-- WHAT: the durable, advisory output of the probabilistic matcher — one row per scored
-- patient pair the matcher thinks MIGHT be the same person. A review UI reads it; the
-- (future, §5.7) link-apply seam (piece C) consumes it.
--
-- ADVISORY, NOT A SAFETY GATE. There is no validated submit door here and no
-- submit_event involvement: a bad row is a bad PROPOSAL a human reviews, never record
-- corruption. The safety-critical floor is db/016 (cairn_match_veto), which the matcher
-- CALLS before writing; this table only records the advisory verdict.
--
-- Additive: no event-format change, no submit_event change. Reads nothing.
--
-- WRITERS (all as a role granted cairn_agent). The Python matcher INSERTs the proposal and
-- retracts it (pipeline/db.py upsert_proposal / retract_pending_proposal); the Rust apply
-- seams then move `status` — apply_proposal.rs (C2, human-driven) and auto_apply.rs (C2b,
-- ×2). Five sites, two languages. This list used to read "only the Python pipeline writes
-- here", which was already false when the C2/C2b seams landed and is corrected here (#79);
-- keep it current, because two comments below reason about who writes.

CREATE TABLE IF NOT EXISTS match_proposal (
    -- The pair is stored in canonical (least, greatest) order so it is a natural unique
    -- key and the whole table is symmetric: propose(a,b) and propose(b,a) touch one row,
    -- mirroring cairn_match_veto's symmetry. The CHECK enforces the ordering invariant.
    patient_low        UUID    NOT NULL,
    patient_high       UUID    NOT NULL,
    score_total        DOUBLE PRECISION NOT NULL,
    -- The matcher's IMMUTABLE propose-time assessment ('auto_candidate' | 'review'). It is
    -- NOT the disposition axis: when C2b (cairn-node::auto_apply) re-checks the veto and
    -- finds a pair vetoed since propose, it moves `status` to 'review' but leaves `band`
    -- unchanged (the matcher still assessed it auto_candidate). A human-review worklist must
    -- therefore filter on `status`, not `band`.
    -- CONSTRAINED, unlike `status` below: the two band values are owned by the Python
    -- `cairn_matcher.pipeline.banding.Band` enum and that set is CLOSED, so a CHECK costs
    -- nothing and stops a writer that is NOT the matcher pipeline (a psql session, a
    -- migration script, a future service) storing a band no reader can interpret (#79).
    -- Named, not auto-named, so the paired ALTER below can find it idempotently.
    band               TEXT    NOT NULL
        CONSTRAINT match_proposal_band_check CHECK (band IN ('auto_candidate','review')),
    veto_findings      JSONB   NOT NULL,   -- cairn_match_veto rows, verbatim (explainability)
    evidence           JSONB   NOT NULL,   -- per-field MatchScore breakdown (explainability)
    matcher_version    TEXT    NOT NULL,   -- cairn_matcher version + config digest (ADR-0014)
    -- The disposition axis: 'pending' -> human 'accepted'/'rejected'/'applied' (C2) or
    -- matcher 'auto_applied'/'review' (C2b) or matcher 'retracted' (the pair dropped below
    -- the review floor after being surfaced — e.g. a §5.4 forced-REVIEW Doe was identified,
    -- issue #135; matcher-owned and reversible: a genuine re-proposal reverts it to
    -- 'pending'). No CHECK — deliberately open (advisory table).
    status             TEXT    NOT NULL DEFAULT 'pending',  -- disposition (see band note above)
    created_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    -- NO UPDATE TRIGGER, DELIBERATELY (#79). EVERY writer listed in the header sets
    -- updated_at = clock_timestamp() explicitly today: db.py's upsert_proposal (in its
    -- ON CONFLICT DO UPDATE arm) and retract_pending_proposal, plus apply_proposal.rs and
    -- auto_apply.rs (×2). A BEFORE UPDATE trigger would be more robust against a future
    -- writer that forgets — but this is the advisory tier, where a stale updated_at costs a
    -- worklist a wrong sort order, never record integrity, and the project keeps in-DB
    -- machinery for the safety-critical floor (ADR-0001's "fat Postgres" is about the
    -- floor, not about advisory bookkeeping). Recorded here so a future reader does not add
    -- one as a "bug fix". The condition that flips the trade is a writer that FORGETS the
    -- column — not merely an additional writer, of which there are already five.
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_low, patient_high),
    CHECK (patient_low < patient_high)
);

-- #207 paired-ALTER discipline. The CREATE TABLE above is IF NOT EXISTS, so on a database
-- that already has match_proposal it is a no-op and the inline CONSTRAINT never lands —
-- the table would stay unconstrained forever while the file *looks* like it constrains it.
-- Every schema file is replayed on each connect, so this block must be idempotent.
--
-- GUARDED ON THE DEFINITION, NOT THE NAME. The obvious `IF NOT EXISTS (… conname = …)`
-- form (as db/041 uses) is subtly wrong for a set that can WIDEN: once the constraint
-- exists, a later edit to the value list is silently skipped on every database that already
-- has it, so a new band would be storable only on freshly-created databases. Both test
-- suites build their databases fresh, so neither could ever catch that — it would surface
-- only on long-lived rigs and real nodes. Comparing the deparsed definition instead makes a
-- STALE constraint converge like an absent one. Steady state stays free: one catalog read,
-- no lock, no table scan. db/tests/017 pins both halves (convergence AND the write-free
-- steady state, via constraint oid stability).
--
-- ADDED `NOT VALID`, DELIBERATELY. A plain ADD validates every existing row, so ONE
-- uninterpretable band on this ADVISORY table would abort the schema load — and
-- connect_and_load_schema treats that as fatal, so the node would not start. Bricking a
-- node over an advisory worklist row inverts availability-over-consistency. NOT VALID
-- enforces the CHECK on every future INSERT/UPDATE (which is the entire point — it guards
-- against a writer that is not the matcher) while leaving any pre-existing junk row alone
-- to be found by a query rather than by an outage. It also skips the validation scan.
DO $$
DECLARE
    -- What Postgres deparses `CHECK (band IN ('auto_candidate','review'))` back to. If a
    -- future PG version changes that rendering, the oid-stability test in db/tests/017
    -- fails loudly rather than letting this block re-add the constraint on every connect.
    want CONSTANT text :=
        'CHECK ((band = ANY (ARRAY[''auto_candidate''::text, ''review''::text])))';
    have text;
BEGIN
    -- Strip the NOT VALID suffix before comparing: the constraint this block adds carries
    -- it while the one the CREATE TABLE above adds does not, and both are up to date.
    SELECT regexp_replace(pg_get_constraintdef(oid), '\s+NOT VALID$', '') INTO have
      FROM pg_constraint
     WHERE conname = 'match_proposal_band_check'
       AND conrelid = 'match_proposal'::regclass;

    -- NULL (absent) or a different expression (stale) both converge to the current set.
    IF have IS DISTINCT FROM want THEN
        ALTER TABLE match_proposal DROP CONSTRAINT IF EXISTS match_proposal_band_check;
        ALTER TABLE match_proposal
            ADD CONSTRAINT match_proposal_band_check
            CHECK (band IN ('auto_candidate','review')) NOT VALID;
    END IF;
END $$;

-- Advisory writer. cairn_agent is the NOLOGIN role (db/004) the matcher's login role is
-- granted into. Retraction is a status UPDATE ('pending' -> 'retracted'), never a DELETE:
-- the advisory row's history is preserved (append-only-friendly), so UPDATE suffices and
-- no DELETE is granted.
GRANT SELECT, INSERT, UPDATE ON match_proposal TO cairn_agent;
