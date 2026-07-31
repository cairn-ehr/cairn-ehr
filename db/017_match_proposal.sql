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
-- Additive: no event-format change, no submit_event change. Reads nothing; only the
-- Python pipeline writes here (as a role granted cairn_agent).

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
    -- NO UPDATE TRIGGER, DELIBERATELY (#79). `runner.upsert_proposal` sets updated_at =
    -- clock_timestamp() explicitly in its ON CONFLICT DO UPDATE arm, and it is the only
    -- writer. A BEFORE UPDATE trigger would be more robust against a future writer that
    -- forgets — but this is the advisory tier, where a stale updated_at costs a worklist
    -- a wrong sort order, never record integrity, and the project keeps in-DB machinery
    -- for the safety-critical floor (ADR-0001's "fat Postgres" is about the floor, not
    -- about advisory bookkeeping). Recorded here so a future reader does not add one as a
    -- "bug fix": if a SECOND writer to this table ever appears, revisit the trade — that
    -- is the condition that flips it, not the mere absence of the trigger.
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_low, patient_high),
    CHECK (patient_low < patient_high)
);

-- #207 paired-ALTER discipline. The CREATE TABLE above is IF NOT EXISTS, so on a database
-- that already has match_proposal it is a no-op and the inline CONSTRAINT never lands —
-- the table would stay unconstrained forever while the file *looks* like it constrains it.
-- Every schema file is replayed on each connect, so the ALTER must be idempotent: guard on
-- pg_constraint rather than catching the duplicate_object error, matching db/041.
--
-- Safe to add to a populated table: the only writer is the matcher pipeline, which sources
-- the value from the Band enum, so no pre-existing row can violate it. If one somehow does,
-- the ALTER fails LOUDLY on the next connect — the correct outcome for a row nothing can
-- interpret, and far better than discovering it in a worklist.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_band_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        ALTER TABLE match_proposal
            ADD CONSTRAINT match_proposal_band_check
            CHECK (band IN ('auto_candidate','review'));
    END IF;
END $$;

-- Advisory writer. cairn_agent is the NOLOGIN role (db/004) the matcher's login role is
-- granted into. Retraction is a status UPDATE ('pending' -> 'retracted'), never a DELETE:
-- the advisory row's history is preserved (append-only-friendly), so UPDATE suffices and
-- no DELETE is granted.
GRANT SELECT, INSERT, UPDATE ON match_proposal TO cairn_agent;
