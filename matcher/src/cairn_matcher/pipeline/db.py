# matcher/src/cairn_matcher/pipeline/db.py
"""The matcher's one module that imports psycopg at import time (the R4 worker's modules run
SQL too, on a connection they are handed). Thin: it loads a patient's projection rows, calls the
in-DB veto floor, and upserts a proposal. All scoring and
banding logic lives in the pure modules; this module just moves data.

Requires the optional `pipeline` extra (psycopg). The pure core never imports it.
"""

import json
import uuid

from psycopg.rows import dict_row

from cairn_matcher.pipeline.adapter import VALUE_SENTINELS_PARAM, candidate_from_rows
from cairn_matcher.pipeline.banding import ProposalPayload, VetoFinding
from cairn_matcher.pipeline.blocking import (
    ANCHORED_PASSES,
    SYMMETRIC_PASSES,
    pairs_from_anchor,
    require_registered,
    resolve_enabled_passes,
)
from cairn_matcher.pipeline.blocking_sql import _GROUPS_SQL, _RANGE_GROUPS_SQL
from cairn_matcher.pipeline.queue_db import AWAITING_HUMAN
from cairn_matcher.placeholder_uses import PLACEHOLDER_USES_PARAM
from cairn_matcher.records import CandidateRecord

# §5.4 placeholder-name exclusion. A "John Doe" chart carries a system-generated CALLSIGN
# (`Unknown-ED-<site>-<date>-<suffix>`, cairn-event::john_doe) as a real, displayed name so
# the header is never blank — but §5.4 requires the matcher EXCLUDE placeholder names from
# its feature space so two unidentified patients can never match via their callsigns. This is
# an ADVISORY exclusion (the matcher owns its feature space, §5.2/§5.13) — the callsign stays a
# normal name in `patient_name`; it is only withheld from SCORING (load_candidate) and BLOCKING
# (the name_tokens CTE) here.
#
# Which of the two is load-bearing: the SCORING exclusion. The blocking tokenizer splits on
# WHITESPACE and a callsign is hyphen-joined with none, so a whole callsign is a SINGLE
# token; two distinct callsigns thus never share a blocking token to begin with. The blocking
# exclusion earns its keep only in the rare IDENTICAL-callsign collision (same site/day/suffix
# — see cairn-node SUFFIX_HEX_LEN) and as cheap defense-in-depth; it is NOT what stops two
# ordinary John Does from grouping (they already don't). The scoring exclusion is what keeps
# a callsign out of the scorer's name feature.
#
# The reserved set (`PLACEHOLDER_NAME_USES`) and its bound-array form now live in the pure,
# psycopg-free `cairn_matcher.placeholder_uses` module — the single source of truth shared with
# the pure synthetic-eval mirror (`eval/generator.py`), which could not import it from here.
# The Rust↔Python drift guard lives in `tests/test_placeholder_uses_sync.py` (see that module
# for why an omission here is a FALSE-MERGE hazard, not merely lost recall).
_PLACEHOLDER_USES_PARAM = PLACEHOLDER_USES_PARAM


def load_candidate(conn, patient_id) -> CandidateRecord:
    """Read one patient's matching-relevant projection rows and shape a CandidateRecord.

    Reads the winner rows (dob, both sex facets) and the retained sets (names, identifiers).
    Pure shaping is delegated to adapter.candidate_from_rows.
    """
    with conn.cursor(row_factory=dict_row) as cur:
        cur.execute("SELECT value, facets, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field='dob'", (patient_id,))
        dob_row = cur.fetchone()
        # One query for BOTH sex facets (§4.2 sex-at-birth: the birth fact; §5.4
        # administrative-sex: the apparent/phenotypic facet a clinician-observed sex
        # lands on). Split by field name here — 'sex-at-birth'/'administrative-sex'
        # are the projection contract, NOT the scorer's weight key (that is "sex").
        cur.execute("SELECT field, value, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field IN ('sex-at-birth','administrative-sex')",
                    (patient_id,))
        sex_rows = cur.fetchall()
        sex_row = next((r for r in sex_rows if r["field"] == "sex-at-birth"), None)
        admin_sex_row = next((r for r in sex_rows if r["field"] == "administrative-sex"), None)
        # Exclude placeholder-use names (callsigns) from the scoring feature space (§5.4).
        cur.execute("SELECT value, provenance_rank FROM patient_name "
                    "WHERE patient_id=%s AND use_key <> ALL(%s)",
                    (patient_id, _PLACEHOLDER_USES_PARAM))
        name_rows = cur.fetchall()
        cur.execute("SELECT system, match_key FROM patient_identifier WHERE patient_id=%s",
                    (patient_id,))
        identifier_rows = cur.fetchall()
    return candidate_from_rows(
        dob_row=dob_row, sex_row=sex_row, name_rows=name_rows, identifier_rows=identifier_rows,
        admin_sex_row=admin_sex_row
    )


def load_aliases(conn, patient_id) -> frozenset[str]:
    """Read one chart's repudiated known-alias name strings from patient_alias_pool (db/025).

    The view is reason-free (ADR-0006 confidentiality split): only the name `value` is
    exposed, never the forensic `reason`. The matcher recognises a returning fabricated
    persona (§5.5(a)) by these values; how a value got there (the C5 suppressing event
    floor) is not this module's concern — it only reads the projection.

    Single-pair path only. A BATCH driver must not call this per pair: it would re-fetch
    the same chart's aliases once per pair the chart appears in, and issue two empty SELECTs
    for every pair in the (overwhelmingly common) no-repudiation case. `load_aliases_for`
    below reads the whole candidate set once instead.
    """
    with conn.cursor() as cur:
        cur.execute("SELECT value FROM patient_alias_pool WHERE patient_id=%s", (patient_id,))
        return frozenset(row[0] for row in cur.fetchall())


def load_aliases_for(conn, patient_ids) -> dict[str, frozenset[str]]:
    """Read the repudiated known-aliases for a whole set of charts in ONE query.

    The batch counterpart to `load_aliases`. A sweep scores every candidate pair, and a
    chart appears in many pairs; loading its aliases per pair (as `load_aliases` does) is
    redundant I/O, and in the common no-repudiation case it is two empty SELECTs on every
    pair. Instead the sweep pre-loads the aliases for its whole candidate-patient set once
    and hands the lookup to `propose` (which then hits the DB zero extra times per pair).

    Scoped to the given `patient_ids` (never the fleet-wide pool, which grows with every
    repudiation ever synced): the `WHERE patient_id = ANY(...)` probes the base table's
    (subject, value) PK on its leading `subject` column, so this stays an index probe.
    Charts with no repudiated alias are simply absent from the returned dict; the caller
    treats a miss as the empty frozenset. Keys are canonical lowercase uuid text, matching
    `str(patient_id)` at the call site.
    """
    ids = [str(p) for p in patient_ids]
    if not ids:
        return {}
    out: dict[str, set[str]] = {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id, value FROM patient_alias_pool WHERE patient_id = ANY(%s::uuid[])",
            (ids,),
        )
        for pid, value in cur.fetchall():
            out.setdefault(str(pid), set()).add(value)
    return {pid: frozenset(values) for pid, values in out.items()}


def load_trust_for(conn, patient_ids) -> dict[str, str]:
    """§5.7 trust states for a candidate set in ONE query (the load_aliases_for pattern).

    The chart_trust view (db/024) carries rows ONLY for flagged charts (unconfirmed /
    under-review), so an absent key IS the confirmed default — mirrored by
    person_chart_trust's COALESCE. Keys are canonical lowercase uuid text. Scoped to the
    given ids so this stays an index probe, never a scan of every flagged chart in the
    fleet. The single-pair path is the same function over a two-element set — there is
    deliberately no separate singular loader, so this contract lives in exactly one place.
    """
    ids = [str(p) for p in patient_ids]
    if not ids:
        return {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id, trust_state FROM chart_trust WHERE patient_id = ANY(%s::uuid[])",
            (ids,),
        )
        return {str(pid): state for pid, state in cur.fetchall()}


def match_veto(conn, a, b) -> list[VetoFinding]:
    """Call the safety-critical in-DB hard-veto floor (db/016) and return its rows.

    The matcher NEVER re-implements this; it only consults it. A pair with any finding
    cannot be auto-linked (banding enforces that).
    """
    with conn.cursor() as cur:
        cur.execute("SELECT veto_kind, severity, subject, detail FROM cairn_match_veto(%s, %s)",
                    (a, b))
        return [VetoFinding(*row) for row in cur.fetchall()]


# The blocking SQL (the eight passes' statements, _GROUPS_SQL / _RANGE_GROUPS_SQL) lives in the
# pure `pipeline.blocking_sql` module, shared with the per-chart check (`pipeline.targeted`).


def _pairs_from_members(members: list[str]) -> set[tuple[str, str]]:
    """Every canonical within-group pair (uuid value order), as lowercase-uuid-text.

    Pure: the same uuid ordering as runner.canonical_pair, so a pair has one identity no
    matter which group (or pass) surfaces it. Self-pairs are excluded by the strict order.

    Members are first normalized to canonical lowercase-hyphenated uuid text. In that form
    a plain string compare is order-equivalent to the 128-bit value compare (fixed width,
    lowercase hex, hyphens aligned) == runner.canonical_pair's uuid order — so we order by
    string and avoid re-parsing each uuid inside the O(k^2) inner loop.
    """
    ordered = sorted(str(uuid.UUID(str(m))) for m in members)
    out: set[tuple[str, str]] = set()
    for i, a in enumerate(ordered):
        for b in ordered[i + 1:]:
            out.add((a, b))
    return out


def generate_candidate_pairs(
    conn, *, max_block_size: int = 100, enabled_passes=None
) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]:
    """Generate canonical candidate pairs via the blocking passes, capping huge blocks.

    Eight passes (see blocking.ALL_PASSES): six SYMMETRIC group passes (identifier /
    exact-DOB / name-token / name-token+birth-year / dob+first-initial / name+sex,
    _GROUPS_SQL) and two ANCHORED birth-year-range passes (dob-range / dob-range+sex,
    _RANGE_GROUPS_SQL — the §5.4 range-blocking slice).

    `enabled_passes` is the A/B measurement toggle: None runs every pass; a set runs only
    the named ones (unknown names raise — see blocking.resolve_enabled_passes). Filtering
    happens on the returned rows' pass_name — the SQL is never edited per-subset, so the
    toggle can never change what a pass WOULD have produced. A whole STATEMENT is skipped
    only when none of its passes is enabled (its arms are independent UNION ALL branches,
    so skipping it provably cannot affect any enabled pass — and the A/B baseline run
    with the range passes off must not pay for the un-indexable range overlap join).

    Returns (pairs, skipped_blocks). `pairs`: unique canonical (low, high) lowercase-uuid
    tuples from every enabled group with <= max_block_size members. `skipped_blocks`: the
    (pass_name, key, size) of each ENABLED group excluded for exceeding the cap — a block
    shared by hundreds of people is non-discriminating (a group of size k contributes
    C(k,2) pairs; an anchored block of size k contributes k-1), and the §5.13 hub
    duplicate-sweep is the declared backstop for what it drops.

    Read-only — opens a read transaction the CALLER must close (sweep does conn.rollback
    before its write loop, so a long sweep does not pin the xmin horizon).
    """
    enabled = resolve_enabled_passes(enabled_passes)
    pairs: set[tuple[str, str]] = set()
    skipped_blocks: list[tuple[str, str, int]] = []
    # require_registered on every fetched row, against the emitting STATEMENT's declared
    # set: an unregistered (or registered-but-misplaced) SQL arm would otherwise be
    # silently filtered by the `enabled` check on every run (a pass that looks built but
    # contributes zero pairs) — or silently skipped with the wrong statement.
    with conn.cursor() as cur:
        if enabled & SYMMETRIC_PASSES:
            # Two binds now: _PLACEHOLDER_USES_PARAM for name_tokens (first, appears first),
            # VALUE_SENTINELS_PARAM for blocking_sex (second) -- the name+sex arm's sex source.
            cur.execute(_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM))
            for pass_name, key, members in cur.fetchall():
                if require_registered(pass_name, SYMMETRIC_PASSES) not in enabled:
                    continue
                size = len(members)
                if size > max_block_size:
                    skipped_blocks.append((pass_name, key, size))
                else:
                    pairs.update(_pairs_from_members(members))
        if enabled & ANCHORED_PASSES:
            # The anchored range passes: pairs are anchor x member ONLY. The cap counts
            # the WHOLE block (members + the anchor itself) so "block size" means the
            # same thing for both pair-generation shapes, and a skipped block is
            # reported under the anchor's uuid (its natural key). The %s binds the
            # uncertainty-sentinel exclusion in the blocking_sex CTE.
            cur.execute(_RANGE_GROUPS_SQL, (VALUE_SENTINELS_PARAM,))
            for pass_name, anchor, members in cur.fetchall():
                if require_registered(pass_name, ANCHORED_PASSES) not in enabled:
                    continue
                size = len(members) + 1
                if size > max_block_size:
                    skipped_blocks.append((pass_name, str(anchor), size))
                else:
                    pairs.update(pairs_from_anchor(anchor, members))
    return sorted(pairs), skipped_blocks


# The existing row is an auto-application whose link another writer's UN-attested unlink has
# overruled. The same "disputed" test as cairn-node's DISPUTED_SQL (duplicate_review/mod.rs) — the
# standing patient_link row is an unlink nobody attested — written over the ON CONFLICT target.
_AUTO_LINK_OVERRULED = (
    "(match_proposal.status='auto_applied' AND EXISTS (SELECT 1 FROM patient_link pl "
    "WHERE pl.low=match_proposal.patient_low AND pl.high=match_proposal.patient_high "
    "AND pl.state='unlink' AND NOT pl.attested))"
)


def upsert_proposal(conn, low, high, payload: ProposalPayload) -> None:
    """Write (or refresh) the advisory proposal for a canonical-ordered pair.

    Latest-wins on (patient_low, patient_high), but a human's decision (accepted /
    rejected / applied), the matcher's auto_applied, and auto_apply's 'review' kick (a machine
    verdict — a re-run must not send the pair back to the auto band) are PRESERVED — a re-run
    refreshes the score/band/evidence, never a verdict. There are TWO matcher-owned exceptions,
    both moving a row the matcher itself closed back to 'pending' so a live match is never left
    hidden:

    * 'retracted' -> 'pending': a row the matcher withdrew (band dropped below review, see
      retract_awaiting_proposal) but now proposes again must re-surface on the worklist.
    * 'auto_applied' -> 'pending' when another writer's UN-attested unlink now stands for the
      pair (ADR-0078). The matcher's own link lost the overlay, so the two charts are separate
      records again and no human has judged them; db/057 does not hold 'auto_applied' open, so
      without this the pair would reach neither the banner nor the worklist. Reopened, it is
      shown with the dispute, and auto_apply.rs sends it to 'review' rather than linking again.
      applied_event_id is cleared with it, keeping db/019's invariant (set exactly on
      applied/auto_applied); the matcher's signed link event itself stays in event_log.

    Every other status is left untouched. Both CASEs read the row as it was before this UPDATE
    (Postgres evaluates every SET expression against the old row), so they agree.

    Does NOT commit. The caller owns the transaction boundary.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal "
            "(patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) "
            "VALUES (%s,%s,%s,%s,%s,%s,%s) "
            "ON CONFLICT (patient_low, patient_high) DO UPDATE SET "
            "score_total=EXCLUDED.score_total, band=EXCLUDED.band, "
            "veto_findings=EXCLUDED.veto_findings, evidence=EXCLUDED.evidence, "
            "matcher_version=EXCLUDED.matcher_version, updated_at=clock_timestamp(), "
            f"status=CASE WHEN match_proposal.status='retracted' THEN 'pending' "
            f"WHEN {_AUTO_LINK_OVERRULED} THEN 'pending' "
            "ELSE match_proposal.status END, "
            f"applied_event_id=CASE WHEN {_AUTO_LINK_OVERRULED} THEN NULL "
            "ELSE match_proposal.applied_event_id END",
            (low, high, payload.score_total, payload.band.value,
             json.dumps(list(payload.veto_findings)), json.dumps(list(payload.evidence)),
             payload.matcher_version),
        )


def retract_awaiting_proposal(conn, low, high) -> int:
    """Withdraw a proposal no human has decided (AWAITING_HUMAN -> 'retracted'); return rows hit.

    Called when a pair the matcher previously surfaced now bands below the review floor —
    most sharply the §5.4 forcing rule, which persisted a REVIEW row while a chart was
    'unconfirmed' (a transient state) that must not linger once the Doe is identified
    (issue #135). Append-only-friendly: a status move, never a DELETE (db/017 grants none),
    so the advisory row's history is preserved and the worklist and banner (which read db/057's
    match_proposal_open, where 'retracted' is not an open status) stop showing a resolved chart
    against a nonexistent Doe.

    Only rows in AWAITING_HUMAN transition (pending, or auto_apply's
    'review' kick — #743 part 1) — a human's disposition or a matcher auto-application is
    left untouched. A no-op (0 rows) for the common case: a sub-threshold pair that never
    had a proposal. Does NOT commit; the caller owns the transaction boundary.
    """
    with conn.cursor() as cur:
        cur.execute(
            "UPDATE match_proposal SET status='retracted', updated_at=clock_timestamp() "
            "WHERE patient_low=%s AND patient_high=%s AND status = ANY(%s)",
            (low, high, list(AWAITING_HUMAN)),
        )
        return cur.rowcount


def awaiting_proposal_pairs(conn) -> list[tuple[str, str]]:
    """Every advisory proposal still awaiting a human decision (status in AWAITING_HUMAN).

    Returns canonical (patient_low, patient_high) lowercase-uuid TEXT tuples — the same shape
    generate_candidate_pairs and canonical_pair produce, so a caller can compare the two sets
    directly. The sweep's reconciliation pass (issue #210) re-scores any pending pair the
    current blocking passes no longer generate — a pair that has dropped out of the blocking
    universe, e.g. a John Doe whose year-range DOB anchor was replaced by a point date on
    identification — so a stale REVIEW row is not left grouping a resolved chart under a
    nonexistent Doe. Only AWAITING_HUMAN rows are returned: a human disposition or a matcher
    auto-application is never a reconciliation candidate.

    Read-only — opens a read transaction the CALLER must close (the sweep already closes its
    read snapshot before the write loop).
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_low::text, patient_high::text FROM match_proposal "
            "WHERE status = ANY(%s)",
            (list(AWAITING_HUMAN),),
        )
        return [(low, high) for low, high in cur.fetchall()]
