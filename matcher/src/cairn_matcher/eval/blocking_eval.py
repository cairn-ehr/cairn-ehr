"""DB-gated blocking-recall eval: how well candidate generation covers true matches.

The one DB-touching eval module (needs the optional `pipeline` extra, psycopg). It seeds
a dataset into the patient_* projections, calls the REAL generate_candidate_pairs, and
measures pair-completeness (blocking recall) and reduction-ratio against ground truth.
No parallel blocking implementation — the SQL stays the source of truth.

Dataset record_ids are readable labels; the projection key is a uuid. We derive a stable
uuid5 per label (deterministic, so a re-run is reproducible) and reverse-map the
generated uuid pairs back to labels to compare against the label-space ground truth.
"""

import json
import uuid
from dataclasses import dataclass

from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    canonical_label_pair,
    truth_pairs,
)
from cairn_matcher.pipeline.blocking import dropped_pair_estimate

# A fixed namespace so label -> uuid is stable across runs (reproducible eval seeding).
_LABEL_NS = uuid.UUID("6f9b4c2e-1d3a-4e5f-8a7b-0c1d2e3f4a5b")


@dataclass(frozen=True)
class BlockingMetrics:
    """Blocking-recall metrics for one dataset under one blocking cap."""

    pair_completeness: float          # |generated & true| / |true|  (the recall ceiling)
    reduction_ratio: float            # 1 - |generated| / |all pairs|
    generated_pairs: int
    total_pairs: int
    skipped_blocks: tuple[tuple[str, str, int], ...]  # (pass_name, key, size) over cap
    dropped_pair_estimate: int        # pairs the skipped blocks would have contributed
                                      # (C(s,2) symmetric / s-1 anchored — see
                                      # blocking.dropped_pair_estimate)
    dropped_true_matches: tuple[tuple[str, str], ...]  # true matches blocking missed


def record_uuid(label: str) -> str:
    """Deterministic uuid (text) for a record label — stable across runs."""
    return str(uuid.uuid5(_LABEL_NS, label))


def seed_dataset(conn, ds: LabelledDataset) -> dict[str, str]:
    """Insert every dataset record into the patient_* projections (no commit).

    Mirrors tests/conftest.seed_patient but reads the dataset's dict fields. Returns the
    uuid->label reverse map the caller uses to translate generated pairs back to labels.

    Deliberately does NOT commit: the rows live in the caller's open read transaction,
    which generate_candidate_pairs reads (read-your-own-writes on one connection) and
    evaluate_blocking's conn.rollback() then discards. Committing here would persist
    synthetic 'seed' patients permanently — and because patient_demographic is
    PRIMARY KEY (patient_id, field) with no ON CONFLICT below, the deterministic uuid5
    labels would make a second run raise a unique violation. Eval seeding stays ephemeral.
    """
    reverse: dict[str, str] = {}
    with conn.cursor() as cur:
        for rec in ds.all_records():
            pid = record_uuid(rec.record_id)
            reverse[pid] = rec.record_id
            if rec.dob is not None:
                cur.execute(
                    "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                    "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, "
                    "asserted_origin) VALUES (%s,'dob',%s,%s,'seed',%s,0,0,'seed')",
                    (pid, rec.dob.get("value"),
                     json.dumps({"precision": rec.dob.get("precision")}),
                     rec.dob.get("provenance_rank", 0)),
                )
            # Both sex facets feed blocking_sex's UNION (db.py), so a range-DOB Doe
            # carrying only administrative-sex can still be rescued by the dob-range+sex
            # pass. One shared INSERT (blocking_sex reads value only, so no facets):
            # the two rows differ ONLY in the field literal, and keeping one SQL string
            # means the seeding shape cannot drift between the facets.
            for field, row in (("sex-at-birth", rec.sex_at_birth),
                               ("administrative-sex", rec.administrative_sex)):
                if row is not None:
                    cur.execute(
                        "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                        "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, "
                        "asserted_origin) VALUES (%s,%s,%s,NULL,'seed',%s,0,0,'seed')",
                        (pid, field, row.get("value"), row.get("provenance_rank", 0)),
                    )
            for n in rec.names:
                cur.execute(
                    "INSERT INTO patient_name (patient_id, use_key, value, use_raw, "
                    "provenance, provenance_rank, last_hlc_wall, last_hlc_count, "
                    "asserted_origin) VALUES (%s,'legal',%s,'legal','seed',%s,0,0,'seed') "
                    "ON CONFLICT DO NOTHING",
                    (pid, n["value"], n.get("provenance_rank", 0)),
                )
            for i in rec.identifiers:
                cur.execute(
                    "INSERT INTO patient_identifier (patient_id, system, match_key, value, "
                    "normalized, profile, use_type, provenance, asserted_hlc_wall, "
                    "asserted_hlc_count, asserted_origin) VALUES "
                    "(%s,%s,%s,%s,%s,NULL,NULL,'seed',0,0,'seed') ON CONFLICT DO NOTHING",
                    (pid, i["system"], i["match_key"], i["value"], i["match_key"]),
                )
    return reverse


def evaluate_blocking(conn, ds: LabelledDataset, *, max_block_size: int = 100) -> BlockingMetrics:
    """Seed the dataset, run the real blocking, and measure recall/reduction.

    Calls generate_candidate_pairs (lazy import: keeps the module importable without the
    function name leaking into the pure path) then rolls back — discarding the uncommitted
    seed (so the eval leaves no synthetic patients behind) and releasing the read snapshot,
    mirroring the sweep's xmin-horizon discipline.
    """
    from cairn_matcher.pipeline.db import generate_candidate_pairs

    reverse = seed_dataset(conn, ds)
    uuid_pairs, skipped = generate_candidate_pairs(conn, max_block_size=max_block_size)
    conn.rollback()

    # Blocking scans the WHOLE connected DB, not just the seed: on a live target a
    # RESIDENT chart can join a block (e.g. a real John Doe's year-range window anchors
    # every seeded record born inside it). Those resident<->seeded pairs are outside the
    # labelled ground truth — reverse knows only seeded uuids — so they are excluded
    # from the metrics rather than crashing the translation with a KeyError.
    generated = {
        canonical_label_pair(reverse[low], reverse[high])
        for low, high in uuid_pairs
        if low in reverse and high in reverse
    }
    truth = truth_pairs(ds)
    total = len(all_pairs(ds))

    dropped_true = tuple(sorted(truth - generated))
    return BlockingMetrics(
        pair_completeness=(len(generated & truth) / len(truth)) if truth else 0.0,
        reduction_ratio=(1.0 - len(generated) / total) if total else 0.0,
        generated_pairs=len(generated),
        total_pairs=total,
        skipped_blocks=tuple(skipped),
        # Shape-aware: C(s,2) for symmetric blocks, s-1 for anchored ones (the pure
        # helper branches on blocking.ANCHORED_PASSES). Not hypothetical even before
        # the generator learns range dobs: a resident year-range chart on a live
        # CAIRN_TEST_PG target can put an anchored block into `skipped` today.
        dropped_pair_estimate=dropped_pair_estimate(skipped),
        dropped_true_matches=dropped_true,
    )
