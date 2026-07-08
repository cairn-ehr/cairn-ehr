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
    band               TEXT    NOT NULL,   -- 'auto_candidate' | 'review' (matcher's assessment)
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
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_low, patient_high),
    CHECK (patient_low < patient_high)
);

-- Advisory writer. cairn_agent is the NOLOGIN role (db/004) the matcher's login role is
-- granted into. Retraction is a status UPDATE ('pending' -> 'retracted'), never a DELETE:
-- the advisory row's history is preserved (append-only-friendly), so UPDATE suffices and
-- no DELETE is granted.
GRANT SELECT, INSERT, UPDATE ON match_proposal TO cairn_agent;
