# Design — B3 weight-learning: supervised Fellegi–Sunter estimation

**Date:** 2026-07-06 · **Scope:** advisory Python only (`matcher/`) — no `db/` floor change, no SCHEMA
bump, no event-type change, no ADR/spec edit. Fit-for-purpose §9 tier (the matcher is advisory). Builds
the measurement-driven B3 slice that slice 23 (the eval mirror) unblocked.

**Status: proof-of-concept.** This slice ships the *learner mechanism* plus an honest demonstration on
the data we have (small curated `gold_v1.json` + the synthetic volume generator). Per §5.13 / ADR-0014
production weights come from each deployment's own real local adjudication data — so the numbers here are
a PoC, not "the weights." The intended follow-up (deferred, waits on the user's time) is to re-run the
same mechanism against a **large hand-crafted gold set** for authoritative magnitudes.

## 1. Problem

`scoring.DEFAULT_WEIGHTS` (the per-field `log2(m/u)` table) and `banding.DEFAULT_THRESHOLDS`
(`review=3.0, auto=8.0`) are hand-picked *illustrative* magnitudes — every one of them carries a code
comment saying "B3 learns real ones from local data." The eval harness was deliberately built so the
learner could plug straight in: `eval/scorer_eval.evaluate_scorer(ds, *, weights, thresholds, config)`
already runs the **real** production scoring→banding path over a labelled dataset and returns
`ScorerMetrics`, with `weights`/`thresholds` as parameters — "sweeping them is exactly how
weight-learning will use this harness" (its own docstring).

Slice 23 closed the last precondition: the eval harness now carries the exact field set the shipped
matcher scores (composite `sex`) and blocks on (anchored range passes), so learning no longer trains on
a stale field set.

What is missing is the learner itself: a pure function that turns a labelled dataset into a
`(Weights, Thresholds)` model, plus an honest before/after measurement of the lift.

## 2. Non-goals

- **Black-box search / optimizer.** Rejected in brainstorming: the scorer *is* Fellegi–Sunter, so the
  canonical closed-form m/u estimator is both more legible (§9) and on-model. No grid/random search.
- **Committing learned weights as the new `DEFAULT_WEIGHTS`.** This slice ships the mechanism + a PoC
  demonstration; it does **not** replace the shipped defaults. A large-gold-set re-run does that later.
- **A production consumer that loads the artifact.** The `LearnedModel` JSON round-trips (so a future
  deployment *could* load it into `Weights`/`Thresholds`), but nothing in the pipeline reads it yet.
- **Per-provenance-band weight learning.** Provenance stays an orthogonal multiplier (see §4). A future
  refinement, out of scope.
- **A veto-aware / end-to-end learned mode.** The pure eval is veto-blind, same documented caveat as
  `evaluate_scorer`. Learned thresholds are for the veto-free scorer; end-to-end veto capping only ever
  *lowers* a band (the safe direction).
- Any change to `pipeline/db.py`, comparators, the orchestrator, `scoring.py`, or `banding.py` beyond
  what the learner imports read-only. The production path is untouched.

## 3. Method — supervised closed-form Fellegi–Sunter estimation

The scorer combines per-field agreement levels by summing `log2(m/u)` log-weights (`scoring.score`).
Fellegi–Sunter weights have a canonical supervised estimator that is **pure counting** over labelled
pairs — no optimizer, no hyperparameters beyond a documented smoothing constant, deterministic, and
maximally reviewer-legible: you can read exactly why each weight is the value it is.

### 3.1 Weight estimation — `estimate_weights`

Input: for every record pair, the `list[FieldComparison]` (field, level, provenance_rank) produced by
the **production** `orchestrator.field_comparisons()` path — the identical call `evaluate_scorer` makes,
so the learner can never drift from what ships — paired with its ground-truth label (match /
non-match, from the dataset's entity clusters via `dataset.truth_pairs`).

Counting is **conditioned on the field being comparable** (level ≠ `INSUFFICIENT_DATA`), matching
scoring's rule that a missing field contributes *exactly* 0 (§3.7, no-data-is-never-disagreement).
`INSUFFICIENT_DATA` is never assigned a learned weight.

For each field `f` and each agreement level `L` the field prices:

- `m[f][L] = P(level = L | true match, f comparable)`
- `u[f][L] = P(level = L | true non-match, f comparable)`
- `weight[f][L] = log2( m[f][L] / u[f][L] )`

**Zero-count smoothing (mandatory).** With finite data some `(f, L)` cells have zero matches or zero
non-matches, and `log2` of `0/u` or `m/0` is `±inf`. We apply additive (Laplace) smoothing with a small
pseudo-count `α` (default documented, e.g. 0.5) spread over the level set the field actually prices, so
`m, u ∈ (0, 1)` strictly. A level seen only in matches then earns a large but **bounded** positive
weight (never `+inf`); symmetric on the negative side. Deterministic.

**Level set per field.** We price exactly the levels that appear for that field in the training data
(union of levels observed in either class), so the learner never invents a weight for a level no data
supports. `identifier` (positive-only, EXACT alone) therefore learns only `EXACT`, exactly as the
hand-authored table has it.

**Output:** a `Weights` (the existing immutable `scoring.Weights` / `FieldWeights` structure), directly
usable by `score()`.

### 3.2 Provenance is an orthogonal multiplier (modeling assumption)

`score()` computes `weight[f][L] * provenance_factor(rank)`. Estimation counts levels **provenance-blind**
and learns the base `log2(m/u)`; the provenance factor is applied at score time exactly as in production.
This keeps the learned weight consistent with how scoring consumes it. (Learning a separate weight per
(level, provenance-band) is a documented future refinement, not this slice.)

### 3.3 Threshold derivation — `derive_thresholds` (safety-first, coupled to the weights)

Learned weights rescale the total score, so the shipped `(3.0, 8.0)` are meaningless afterward. The
learner **re-derives both thresholds in the same pass** by scoring the training partition with the
learned weights (provenance applied, via the real `score()`), then:

- **`auto` = `max(non-match score) + margin`** → **zero false auto-links** on the training partition by
  construction (an auto-link is an un-attested, if recallable, link — false-auto must stay ~0, the
  matcher's stated dangerous rate). Class overlap (a non-match outscoring some matches) pushes `auto` up,
  moving those true matches from AUTO down to REVIEW — the *safe* direction. Reported honestly; the
  degenerate "nothing auto-links" case is surfaced, not hidden.
- **`review` = the score meeting a recall floor** (default `recall_target = 0.99`): the highest cut-off
  such that ≥ target fraction of true matches score ≥ it, i.e. surface ≥99% of true matches to a human.
- **Invariant `review < auto`** (required by `band()`): enforced. If the recall floor would push `review`
  ≥ `auto`, that is flagged in the model metadata rather than silently clamped — the honest signal that
  the two objectives collided on this data.

**Output:** the existing `banding.Thresholds`, directly usable by `band()`.

### 3.4 Composition — `learn_model`

`learn_model(training_ds, *, config, alpha, recall_target, margin) -> LearnedModel` composes 3.1 + 3.3:
build candidates once per record (reusing `dataset.record_to_candidate`), enumerate `all_pairs`, get
comparisons + labels via the production path, estimate weights, then derive thresholds. Pure and
deterministic. `LearnedModel` bundles `weights`, `thresholds`, and metadata (α, recall_target, margin,
training pair/match counts, the `review<auto` collision flag).

## 4. Honest measurement — `eval/crossval.py`

The learner is a pure function of *any* labelled dataset. Measurement is where honesty is at risk:
reporting train-set lift as if it generalizes would be a precise untruth.

- **Split on entity clusters, never on pairs.** A cluster-mate straddling the train/test boundary would
  leak truth (its within-cluster match pairs would span both partitions). Folds partition **whole
  `EntityCluster`s**; every pair evaluated is within a single fold.
- **Deterministic folds.** Clusters are striped into k folds by **sorted `entity_id`** (no RNG, no seed)
  — reproducible by construction, matching the generator's reproducibility contract.
- **Held-out lift.** For each fold: learn on the other k−1 folds, then evaluate the held-out fold twice
  with `evaluate_scorer` — once with `DEFAULT_WEIGHTS`/`DEFAULT_THRESHOLDS` (before) and once with the
  learned model (after). Aggregate `ScorerMetrics` across held-out folds and report before vs after,
  reusing `metrics.py` + `report.py`. Train-set metrics are never reported.
- Small sets (gold) use k-fold; that its per-fold weights are noisy is a stated honest limit, not a bug.

## 5. Model artifact + CLI — `eval/model_io.py` + a thin CLI

- **`model_io.py`** — pure JSON serialize/deserialize for `LearnedModel` (and its `Weights`/`Thresholds`).
  Round-trips exactly (a reconstructed model scores identically). Rejects malformed input loudly
  (`DatasetError`-style), never silently defaulting. Lets a future deployment load a learned model; no
  pipeline code reads it yet.
- **CLI** (mirrors `eval/generate.py`): `python -m cairn_matcher.eval.learn [dataset.json] [--folds K]
  [--recall-target R] [--margin M] [--alpha A] [--out model.json]`. Defaults to the bundled gold set.
  Prints the before/after held-out report + the learned weight table; writes the artifact when `--out`
  is given. Pure-Python, dependency-free (no DB — this is the scorer tier, not the blocking tier).

## 6. Files (all pure, each < 500 lines, additive under `eval/`)

| File | Purpose |
|---|---|
| `eval/learn.py` | `estimate_weights`, `derive_thresholds`, `learn_model`, `LearnedModel` (core, pure) |
| `eval/crossval.py` | entity-cluster k-fold split + held-out before/after lift report (pure) |
| `eval/model_io.py` | `LearnedModel` ↔ JSON (pure) |
| `eval/learn.py` `main()` (or sibling CLI) | the `python -m` entry mirroring `generate.py` |

No change to `scoring.py`, `banding.py`, `orchestrator.py`, comparators, `pipeline/`, or any `db/` file.

## 7. Testing (TDD — failing test first)

- **`test_eval_learn.py`** — estimation correctness on hand-built comparison sets (known m/u → known
  `log2` weight); monotonicity (a level more concentrated in matches earns a higher weight); smoothing
  keeps a zero-count cell finite and bounded; `INSUFFICIENT_DATA` never learns a weight; `identifier`
  learns EXACT only; empty/degenerate input raises loudly.
- **`test_eval_thresholds.py`** — zero-false-auto property holds on the training scores; the recall floor
  is met; `review < auto` enforced; the collision case flags rather than clamps; the margin is applied.
- **`test_eval_crossval.py`** — a cluster is never split across folds; folds cover every cluster exactly
  once; folds are deterministic across runs; held-out before/after both computed on disjoint data.
- **`test_eval_model_io.py`** — JSON round-trip reconstructs a model that scores identically; malformed
  input rejected.
- **`test_eval_learn_cli.py`** — smoke: runs on bundled gold, prints a before/after report, `--out`
  writes a loadable artifact.

## 8. Honest limits (documented in spec + code)

1. **PoC, not the shipped weights.** Demonstration on the data we have; `DEFAULT_WEIGHTS`/
   `DEFAULT_THRESHOLDS` are unchanged. Authoritative magnitudes await a large hand-crafted gold set
   (deferred, user's time) or real deployment adjudication (§5.13 / ADR-0014).
2. **Synthetic reflects the generator.** Weights learned from `eval/generator.py` output partly encode
   the generator's corruption model, not real-world epidemiology of names/DOBs/typos.
3. **Gold is small.** k-fold weights from a handful of clusters are noisy; the lift number carries that
   variance.
4. **Veto-blind.** Same caveat as `evaluate_scorer`: the pure eval scores without the in-DB veto, so
   learned thresholds are for the veto-free scorer. End-to-end the veto only ever caps a band *down*
   (AUTO→REVIEW), never up — the safe direction — so a veto-blind threshold cannot manufacture a false
   auto-link at run time.
5. **Provenance is a multiplier, not a learned dimension** (§3.2) — a future refinement.
6. **`gold_v1.json` untouched** — the curated culture-plural set is read-only input, not modified.
