"""DB-gated: a generated volume set is fully recoverable by blocking under a large cap.

Confirms the recoverability invariant end-to-end through the REAL generate_candidate_pairs:
with no block over the cap, blocking recall is total and no true match is dropped. Reuses
evaluate_blocking's rollback discipline, so it leaves no synthetic patients behind.
"""

from cairn_matcher.eval.blocking_eval import evaluate_blocking
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset, shares_blocking_key


def test_generated_volume_set_is_fully_recoverable(pg_conn):
    ds = load_dataset(generate_dataset(GenSpec(seed=1, n_entities=200)))
    metrics = evaluate_blocking(pg_conn, ds, max_block_size=10_000)
    assert metrics.pair_completeness == 1.0
    assert metrics.dropped_true_matches == ()
    assert metrics.total_pairs > metrics.generated_pairs   # reduction happened
    assert 0.0 < metrics.reduction_ratio <= 1.0


def test_estimate_heavy_volume_set_is_fully_recoverable(pg_conn):
    """Names corrupted AND dobs replaced by estimated-age windows: with _repair
    standing down on window-overlap pairs (the Task-2 mirror), the range-ONLY
    sentinel pairs asserted below can be carried by the REAL dob-range pass alone —
    the end-to-end proof the mirror never over-claims what the SQL recovers.

    Honest sensitivity note: corrupt_name touches ONE token in one mode, so even at
    p_name=0.9 most estimate clones still share a name token with their seed and stay
    name-recoverable. The proof's teeth are the pairs with NO key besides the window,
    so their presence is asserted directly instead of assumed from the knobs.
    """
    spec = GenSpec(seed=2, n_entities=150, p_dob_estimate=0.9, p_name=0.9)
    ds_dict = generate_dataset(spec)
    ds = load_dataset(ds_dict)
    ranged = [
        r for e in ds.entities for r in e.records
        if (r.dob or {}).get("precision") == "year-range"
    ]
    assert len(ranged) > 100  # non-vacuous: the knob really produced range clones
    # The sentinel subset: range clones whose pair shares NOTHING once the dob is
    # stripped (no identifier, no name token) — recoverable ONLY via the anchored
    # window overlap, so an over-claiming mirror fails pair_completeness below.
    # Same-version determinism (GenSpec docstring) keeps the count stable for seed=2
    # (currently 11); a generator change that shifts RNG consumption must re-clear
    # this floor or the proof has silently lost its teeth.
    range_only = [
        e for e in ds_dict["entities"]
        if e["records"][1].get("dob", {}).get("precision") == "year-range"
        and not shares_blocking_key({**e["records"][0], "dob": None},
                                    {**e["records"][1], "dob": None})
    ]
    assert len(range_only) >= 10, "the proof lost its range-only sentinel pairs"
    metrics = evaluate_blocking(pg_conn, ds, max_block_size=10_000)
    assert metrics.pair_completeness == 1.0
    assert metrics.dropped_true_matches == ()
