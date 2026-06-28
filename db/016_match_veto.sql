-- db/016_match_veto.sql
-- §4.4/§5.2 in-DB hard-veto + coherence-check (the matching pipeline's safety floor).
--
-- WHAT: given two patient candidates, return the closed set of HARD VETOES between
-- them — strong evidence AGAINST a link. A veto FORCES A HUMAN DECISION: it never
-- auto-links and never auto-rejects (an auto-reject is itself a silent false split,
-- identity §5.2/§5.13). This function only COMPUTES a verdict; it never writes,
-- links, demotes, or queues anything.
--
-- WHY HERE (not in the Python matcher): the matcher is advisory and only *proposes*
-- (identity §5.2 NOTE). The hard-veto floor is safety-critical (§9) — it must be
-- deterministic, in-database, and parse nothing culture-specific. This is the floor
-- every future matcher proposal must pass.
--
-- Reads only the existing projections patient_identifier (db/010) and
-- patient_demographic (db/011). Additive: no event-format change, no submit_event
-- change, no new table. Reuses cairn_provenance_rank's output (the cached
-- patient_demographic.provenance_rank column).
--
-- Two verdict levels (the §4.4 honest-degradation nuance):
--   hard_veto    — a TRUSTWORTHY clash; blocks auto-link AND (once linking exists)
--                  may demote an existing link to under-review.
--   degrade_hold — an UNTRUSTWORTHY basis (a profile-less node can't tell a real
--                  identifier mismatch from formatting noise); blocks auto-link and
--                  surfaces to a human, but must NOT demote an existing link.
-- Both stay on the safe side of false-merge >> false-split: neither auto-rejects.

-- ---------------------------------------------------------------------------
-- Helper: the §4.4 identifier veto over patient_identifier.
--
-- A patient may legitimately hold MULTIPLE identifiers in one `system` (the
-- projection PK is (patient_id, system, match_key)), so the comparison is
-- SET-BASED per system, not value-to-value. A clash exists for a shared system
-- only when the two patients share NO common identifier — sharing even one value
-- is positive evidence (a match signal), never a veto.
--   * `system = 'unknown'` (the §4.4 sentinel) NEVER participates in a veto.
--   * Trustworthy comparison is possible only over the materialised `normalized`
--     form. If the two sides share a normalized value -> no finding. Else if both
--     sides carry at least one non-null normalized -> the trustworthy sets are
--     disjoint -> hard_veto. Else (>=1 side is profile-less, normalized absent) ->
--     fall back to the raw `value`: shared value -> no finding; disjoint -> the
--     difference may be pure formatting noise -> degrade_hold.
--   * `9434765919` vs `943 476 5919` share one `normalized` -> no finding.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_identifier_veto(p_a uuid, p_b uuid)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    WITH a AS (
        SELECT system, value, normalized FROM patient_identifier
        WHERE patient_id = p_a AND system <> 'unknown'
    ),
    b AS (
        SELECT system, value, normalized FROM patient_identifier
        WHERE patient_id = p_b AND system <> 'unknown'
    ),
    shared_system AS (
        SELECT system FROM a INTERSECT SELECT system FROM b
    ),
    per_sys AS (
        SELECT
            s.system,
            -- the two sides share at least one non-null normalized value
            EXISTS (
                SELECT 1 FROM a JOIN b ON a.system = b.system
                WHERE a.system = s.system
                  AND a.normalized IS NOT NULL
                  AND a.normalized = b.normalized
            ) AS shared_norm,
            -- both sides carry at least one non-null normalized for this system
            EXISTS (SELECT 1 FROM a WHERE a.system = s.system AND a.normalized IS NOT NULL)
            AND
            EXISTS (SELECT 1 FROM b WHERE b.system = s.system AND b.normalized IS NOT NULL)
                AS both_have_norm,
            -- the two sides share at least one raw value string
            EXISTS (
                SELECT 1 FROM a JOIN b ON a.system = b.system
                WHERE a.system = s.system AND a.value = b.value
            ) AS shared_val
        FROM shared_system s
    )
    SELECT
        'identifier'::text,
        CASE WHEN both_have_norm THEN 'hard_veto'::text ELSE 'degrade_hold'::text END,
        system,
        CASE WHEN both_have_norm
             THEN format('same system %L, no shared normalized identifier (trustworthy mismatch)', system)
             ELSE format('same system %L, values differ but a profile is absent — held for human review', system)
        END
    FROM per_sys
    WHERE NOT shared_norm
      AND NOT shared_val;
$$;

-- ---------------------------------------------------------------------------
-- Helper: the verified DOB / sex-at-birth coherence clash over
-- patient_demographic (one winner row per (patient_id, field)).
--
-- Fires hard_veto IFF: both patients have a winner for `p_field`, BOTH winners
-- are VERIFIED (provenance_rank >= 60: document-verified | fact-proven — the
-- "verified value locks" property of the db/011 projection means a node's winner
-- already reflects its verified value when one exists), the winners carry the SAME
-- precision facet, and the `value` strings differ.
--
-- PARSES NO DATES. The floor never parses the open `value` string (db/011) — date
-- parsing is locale-specific, profile-dependent logic that belongs in the advisory
-- Python matcher, not the safety floor.
--   * Different precision -> NO finding. `1980` (year) vs `1980-03-15` (day) are a
--     consistent coarsening; principle 4: imprecision is partial agreement, never
--     disagreement. (IS NOT DISTINCT FROM treats both-null precision as equal, so
--     sex-at-birth — which carries no precision facet — reduces to "both verified +
--     values differ".)
--   * Known conservative residual: same precision, different format/coding
--     (`15/03/1980` vs `1980-03-15`, or `M` vs `male`) -> a false hard_veto. Safe
--     side (routes to human review, never auto-rejects/merges); rare within one
--     node's own data; resolved by the advisory matcher's locale comparators.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_field_clash(p_a uuid, p_b uuid, p_field text)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    SELECT
        -- veto_kind = p_field is intentional: the closed vocabulary
        -- {'identifier', 'dob', 'sex-at-birth'} is owned by cairn_match_veto,
        -- the only caller, which passes only valid literals from that set.
        p_field,
        'hard_veto'::text,
        p_field,
        format('verified %s clash (precision %s): %L vs %L',
               p_field,
               coalesce(x.facets ->> 'precision', 'none'),
               x.value, y.value)
    FROM patient_demographic x
    JOIN patient_demographic y ON y.field = x.field
    WHERE x.patient_id = p_a
      AND y.patient_id = p_b
      AND x.field = p_field
      AND x.provenance_rank >= 60
      AND y.provenance_rank >= 60
      AND x.value IS DISTINCT FROM y.value
      AND (x.facets ->> 'precision') IS NOT DISTINCT FROM (y.facets ->> 'precision');
$$;

-- ---------------------------------------------------------------------------
-- The public entry point: the union of the closed hard-veto set between two
-- patient candidates. Empty set = no veto (clear to auto-link, subject to the
-- matcher's own conservative threshold — not this function's concern). Symmetric,
-- deterministic; a = b yields empty naturally (identical identifier sets share a
-- normalized value, or when profile-less, share their raw value; identical
-- demographic winners are value-equal).
--
-- DECEASED-STATUS CONFLICT (§5.13 closed set) IS DEFERRED — no deceased field is
-- projected yet (patient_demographic projects only dob + sex-at-birth). When a
-- deceased projection lands, add a fourth branch:
--     UNION ALL SELECT * FROM cairn_field_clash(p_a, p_b, 'deceased')
-- (or a bespoke helper if deceased needs different clash semantics). See the
-- design doc §6 and HANDOVER. Do NOT silently drop it.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_match_veto(p_a uuid, p_b uuid)
RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
LANGUAGE sql STABLE AS $$
    SELECT * FROM cairn_identifier_veto(p_a, p_b)
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'dob')
    UNION ALL
    SELECT * FROM cairn_field_clash(p_a, p_b, 'sex-at-birth');
$$;

-- ---------------------------------------------------------------------------
-- Scalar convenience: the matcher's auto-link gate. True iff any HARD_VETO-severity
-- finding exists. A lone degrade_hold does NOT trip this gate (the caller still
-- surfaces such a pair to a human, but it is not a trustworthy veto).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_has_hard_veto(p_a uuid, p_b uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM cairn_match_veto(p_a, p_b) WHERE severity = 'hard_veto'
    );
$$;

GRANT EXECUTE ON FUNCTION cairn_identifier_veto(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_field_clash(uuid, uuid, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_match_veto(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_has_hard_veto(uuid, uuid) TO cairn_agent;
