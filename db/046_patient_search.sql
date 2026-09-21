-- Cairn — §5.8 search-before-create: advisory candidate generation.
--
-- ADVISORY, NOT A FLOOR (ADR-0061, ADR-0014). A missed candidate produces a false
-- SPLIT — §5.2's explicitly safe direction — and ADR-0014 already names the standing
-- backstop: the hub-tier background duplicate sweep. So this function never blocks,
-- never vetoes and never decides; it offers rows to a human.
--
-- SQL rather than a call into the advisory Python tier because a registration path must
-- beat paper (§1.2) and §5.11's latency limb is explicit ("type a few chars and enter, no
-- spinner"). Coupling two services on that path buys no safety.
--
-- DRIFT NOTE: the three blocking keys below mirror matcher/pipeline/db.py's three-pass
-- disjunction. They are NOT the same query — the sweep blocks all-by-all, this maps
-- query -> set — so only the KEY EXTRACTION is shared. Convergence is tracked as issue #353;
-- if you change a key here, check the matcher.
--
-- UPDATE (#636): search is now DELIBERATELY WIDER than the matcher on the name key — it
-- matches stored token PARTS and 3+ character PREFIXES; the matcher's blocking keys are
-- unchanged. This is safe in exactly one direction and only that one: the invariant this
-- note protects is "a chart the sweep would pair is a chart this search finds"
-- (sweep-paired ⊆ search-found), and widening search preserves it. NARROWING search, or
-- widening the matcher without widening search, would break it. Made executable by
-- crates/cairn-node/tests/patient_search_drift.rs. Widening the matcher to match is a
-- separate question with its own recall/precision and sweep-cost evaluation (#353).
--
-- DELIBERATELY REDUNDANT DEDUPLICATION — read this before "cleaning it up". Each branch
-- carries its own `SELECT DISTINCT` *and* the branches are combined with plain `UNION`
-- (not `UNION ALL`), so every row is de-duplicated twice. That is on purpose and both
-- halves stay:
--
--   * `UNION` alone would suffice — it re-dedups the whole input bag, and because
--     `matched_pass` is a per-branch LITERAL ('identifier'/'dob'/'name'), two rows can only
--     ever collide when they came from the SAME branch. So every possible duplicate is a
--     within-branch duplicate, and the outer UNION already removes it.
--   * The per-branch `DISTINCT` alone would ALSO suffice, for the same reason.
--
-- Keeping both is cheap (the planner sees one dedup opportunity per branch either way) and
-- buys local legibility: each branch reads as "the set of patients this key matches",
-- which is what a reviewer must check it against, without having to hold the combinator
-- three branches below in their head. Dropping either one is safe TODAY and stops being
-- safe the moment a branch gains a non-literal `matched_pass` or a fourth pass is added
-- with an overlapping label — which is exactly the kind of change that would not think to
-- re-derive this argument. The belt and the braces are both one word long.
-- (Recorded here rather than only in the Rust tests, because a "drop the redundant
-- DISTINCT" cleanup would happen in THIS file.)
--
-- UPDATE (#636): the expiry named above has arrived, in the mild form. Pass 3's lateral now
-- UNIONs two token sources, so one patient_name row can yield the same token twice (a
-- single unpunctuated word is both a whole token and its own alphanumeric part). The
-- lateral's own UNION removes that, and the outer UNION removes anything it misses. What
-- is no longer true is the claim that "every possible duplicate is a within-branch
-- duplicate" holds for pass 3 INTERNALLY — so the per-branch DISTINCT is now doing real
-- work rather than being redundant belt. Keep all three dedups.
BEGIN;

CREATE OR REPLACE FUNCTION cairn_search_candidates(
    p_name_tokens text[],
    p_birth_date  text,
    p_identifiers jsonb          -- [{"system": "...", "value": "..."}]
) RETURNS TABLE (patient_id uuid, matched_pass text)
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    -- Pass 1: shared identifier. Highest precision — the same system and the same
    -- match_key is near-conclusive, which is why it is also a db/016 hard-veto axis.
    --
    -- Matches EITHER match_key (= coalesce(normalized, value), db/010) OR the raw value,
    -- not match_key alone (review-round fix, #344 Important 1). match_key is the
    -- MATERIALISED canonical form when a §4.4 profile produced one (e.g. an NHS number's
    -- digits-only "9434765919"), but a clerk searching types what is PRINTED on the card
    -- ("943 476 5919") — the raw `value`, not its normalisation, which is profile-derived
    -- and this query has no profile to re-derive it with (ADR-0033). Without the OR, a
    -- chart registered with a materialised key is unfindable by anyone who types the
    -- identifier exactly as the original registrar was handed it.
    SELECT DISTINCT pi.patient_id, 'identifier'::text
      FROM patient_identifier pi
      JOIN jsonb_array_elements(COALESCE(p_identifiers, '[]'::jsonb)) q
        ON pi.system = (q ->> 'system')
       AND (pi.match_key = (q ->> 'value') OR pi.value = (q ->> 'value'))
    UNION
    -- Pass 2: exact DOB. No date parsing, no range logic — an exact string compare on the
    -- projected value, matching the deliberately parse-free db/016 discipline.
    SELECT DISTINCT pd.patient_id, 'dob'::text
      FROM patient_demographic pd
     WHERE p_birth_date IS NOT NULL
       AND pd.field = 'dob'
       AND pd.value = p_birth_date
    UNION
    -- Pass 3: shared name token. Culture-neutral: EXACT token equality in ANY position, so
    -- a name typed in a different order still finds the chart, with no name-order model.
    --
    -- KNOWN ASYMMETRY, issue #348 — PARTIALLY CLOSED by #636 slice 1a; kept here (not
    -- deleted) so a contributor triaging #348 lands on an accurate account instead of a
    -- stale one. The structural asymmetry #348 named is still literally true: this side's
    -- WHOLE-token source keeps a token's edge punctuation verbatim (deliberately — it is
    -- copied verbatim from the matcher, see the DRIFT NOTE above, and a callsign needs its
    -- punctuation intact, see the callsign comment below), while `SearchQuery::new` (query
    -- side) TRIMS edge punctuation from its whole token. What #348 originally reported —
    -- that a chart registered as "Smith, John" (the registration-desk convention;
    -- `register_patient` stores it raw and unparsed, by design) holds the whole token
    -- "smith," while a clerk typing the surname alone produces "smith", and the two do not
    -- match — is NO LONGER TRUE. The parts source added by slice 1a splits "smith," on its
    -- trailing punctuation into the alphanumeric part "smith", exactly equal to the
    -- clerk's query token; `SearchQuery::new` independently emits the same whole+parts
    -- split (its own #344 fix), so both sides now converge through the PARTS channel even
    -- though the WHOLE-token channel stays asymmetric by design. What remains open under
    -- #348: a query token that is neither an exact stored whole token NOR an alphanumeric
    -- part of one — e.g. internal punctuation typed differently than it was stored — can
    -- still miss. Narrower than the original report, not eliminated. Still the safe
    -- direction when it happens (a false split, never a false merge). Joint widening of
    -- the WHOLE-token channel, if ever wanted, remains a #353 decision.
    --
    -- READS `patient_name`, NOT `patient_name_current` — DELIBERATE, issue #349. This is
    -- the RETAINED name set, which INCLUDES values later struck by
    -- `identity.repudiate.asserted` (db/025); the `_current` view anti-joins those out for
    -- DISPLAY. That is exactly the right split: a struck name must not head a chart, but a
    -- clerk must still be able to FIND the chart by the alias a fabricated persona presented
    -- under (§5.5(a)) — otherwise repudiation itself manufactures a duplicate for precisely
    -- the patients that subsystem exists for. `search.rs`'s `read_names_ever_asserted` also
    -- depends on this table holding the row. A "surely this should read the _current view"
    -- cleanup would silently reverse both, with no test failing to say so.
    --
    -- The tokenising expression is COPIED VERBATIM from matcher/src/cairn_matcher/pipeline/
    -- db.py's _GROUPS_SQL: `regexp_split_to_table(lower(normalize(value, NFC)), '\s+')`,
    -- including its `token <> ''` guard (see below). Same key extraction, so a chart the
    -- sweep would pair is a chart this search finds. NFC normalisation is load-bearing, not
    -- decoration: without it a composed and a decomposed "José" are different tokens and the
    -- chart is silently unfindable.
    --
    -- Exact equality is no longer the whole story (#636 slice 1b added the PREFIX arm
    -- below, via `starts_with`), but the reasoning against `LIKE '%token%'` is unchanged: a
    -- LEADING wildcard cannot use an index at all, and the §1.2 paper-parity budget is 5 s
    -- to find an existing chart. `starts_with(tok, prefix)` is NOT a leading-wildcard match
    -- — it is the same shape as `LIKE 'x%'`, which CAN use an index — so this pass is still,
    -- in principle, indexable.
    --
    -- In practice there is still no index: `patient_name` carries none on `value` at all,
    -- and the tokens this pass matches against are the OUTPUT of a set-returning function
    -- (`regexp_split_to_table`, inside the lateral), not a column — Postgres cannot index a
    -- set-returning function's output without first materialising it into a real token
    -- table (one row per patient per token) and indexing THAT. This pass has always been a
    -- full scan; nothing here changes that. The paragraph above says the door remains open,
    -- not that anyone has walked through it.
    --
    -- Callsigns ARE included here — in the WHOLE-token source only, unlike in the matcher
    -- (which excludes them via `use_key <> ALL(...)`). The parts source and the prefix arm
    -- below each carry their own `pn.use_key <> 'callsign'` guard and exclude them; see
    -- those guards for why. Both stances are right: a callsign is not evidence of identity,
    -- so it must not feed the scorer, and it must not let one typed fragment surface every
    -- John Doe on the node — but a clerk must still be able to find the John Doe in front
    -- of them by typing the callsign back in full, which only the whole-token source needs
    -- to serve.
    --
    -- `tok <> ''` mirrors the matcher's own guard: the §4.2/§4.4 structural floor only
    -- requires a non-BLANK (trimmed) name, so a value with leading/trailing whitespace
    -- (" Smith", "Smith  ") is legitimately admitted and `regexp_split_to_table` on that
    -- value emits an EMPTY string as one of its tokens. Without this guard, a stray empty
    -- element in p_name_tokens (e.g. from a caller's own naive split producing a leading or
    -- trailing blank) would equal that empty projected token and surface a chart with no
    -- typed evidence behind the match at all.
    SELECT DISTINCT pn.patient_id, 'name'::text
      FROM patient_name pn
      CROSS JOIN LATERAL (
            -- The whole whitespace-delimited token, as before: this is what matches a
            -- punctuated name typed back exactly as printed, and an intact callsign.
            SELECT w AS tok
              FROM regexp_split_to_table(lower(normalize(pn.value, NFC)), '\s+') AS w
             WHERE w <> ''
            UNION
            -- Its alphanumeric PARTS (#636, slice 1a) — the mirror of what
            -- SearchQuery::new already emits on the query side, so a clerk typing one half
            -- of "Fyodorowksi-Eschenbacher" finds the chart. Single characters are dropped
            -- for the query side's reason: they cannot narrow a search and only inflate the
            -- advisory set.
            --
            -- CALLSIGNS ARE EXCLUDED, and this is not optional. A callsign is
            -- "Unknown-<class>-<site>-<date>-<tail>"; fragmenting it would project parts
            -- like 'unknown' and 'ed', so one typed word would surface every John Doe on
            -- the node. The query side keeps whole words for exactly this reason; this is
            -- the same guard on the other side. Pinned by
            -- `a_stored_callsign_is_not_fragmented_into_common_parts`.
            SELECT p
              FROM regexp_split_to_table(lower(normalize(pn.value, NFC)),
                                         '[^[:alnum:]]+') AS p
             WHERE length(p) > 1
               AND pn.use_key <> 'callsign'
      ) AS toks
      JOIN unnest(COALESCE(p_name_tokens, ARRAY[]::text[])) t
        ON toks.tok = lower(normalize(t, NFC))
        -- PREFIX matching (#636, slice 1b), because a clerk types a fragment and picks from
        -- a list rather than typing a compound surname in full.
        --
        -- `starts_with`, NOT `LIKE lower(...) || '%'`: SearchQuery::new trims only the EDGE
        -- punctuation of a word, so an internal '%' or '_' survives into a query token and
        -- would be read by LIKE as a wildcard. starts_with has no escaping surface and is
        -- what LIKE 'x%' optimises to anyway.
        --
        -- MINIMUM 3 CHARACTERS, and note WHAT it gates: the PREFIX arm only. Exact matching
        -- above has no length rule, so a two-character surname ("Wu", "Ng") stays findable —
        -- only a two-character prefix OF A LONGER token is refused, which is the unselective
        -- case this exists for. A one- or two-character prefix matches a large fraction of
        -- any population; worse, if such a search preceded a registration, that whole
        -- candidate list would be written into a permanent signed attestation (ADR-0061).
        -- Pinned by `a_two_character_surname_is_still_found_by_exact_match`.
        --
        -- `pn.use_key <> 'callsign'` here too, and this half is NOT in the slice-1b design
        -- doc — it surfaced only when this arm was run against the existing test suite.
        -- Without it, the guard above (line ~185) stops a callsign's PARTS from being
        -- projected, but the WHOLE-token branch (line ~166) still projects the intact
        -- callsign deliberately, and `starts_with('unknown-ed-site1-...', 'unknown')` is
        -- true: a clerk typing the leading word of any John Doe callsign would prefix-match
        -- every John Doe on the node, exactly the hazard the parts-branch guard exists to
        -- prevent, reopened from the whole-token side. Exact matching is UNAFFECTED — typing
        -- the callsign in full still finds it, because the equality arm above carries no
        -- such restriction. Pinned by `a_stored_callsign_is_not_fragmented_into_common_parts`
        -- (which predates this arm but caught the regression the moment this arm landed).
        OR (pn.use_key <> 'callsign'
            AND length(lower(normalize(t, NFC))) >= 3
            AND starts_with(toks.tok, lower(normalize(t, NFC))))
     WHERE toks.tok <> ''
$$;

REVOKE EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_search_candidates(text[], text, jsonb) TO cairn_agent;

COMMIT;
