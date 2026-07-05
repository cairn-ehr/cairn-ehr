# Design — administrative-sex scoring + the unconfirmed-chart REVIEW rule

**Date:** 2026-07-05 · **Issue:** [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130) ·
**Tier:** advisory Python only (fit-for-purpose, §9) — **no `db/` migration, no SCHEMA bump, no ADR,
no spec edit.** Implements settled §5.2/§5.13/§5.4 behaviour. Blocking SQL untouched.

## 1. Problem

The headline §5.4 pair — a callsign-only John Doe carrying clinician-observed evidence (estimated-age
birth-year range + observed sex) vs their prior chart — now *blocks* (the 2026-07-04 anchored
`dob-range`/`dob-range+sex` passes) but then dies at the scoring floor:

- the callsign is excluded from name scoring (correct per §5.4);
- the observed sex lands on `administrative-sex`, which `load_candidate` never reads
  (it queries `sex-at-birth` only);
- so the only scoreable field is dob → PARTIAL 1.5 × provenance-factor(30) ≈ **1.07** < `review=3.0`
  → `propose()` persists nothing. Blocked, scored, silently dropped.

**The design-critical arithmetic:** scoring administrative-sex alone does NOT fix this. With sex
scored, the pair reaches ≈ 1.07 + 1.0 × 0.714 ≈ **1.79 — still below 3.0**. And honest
Fellegi–Sunter weights cannot be inflated past it: an 11-year birth window plus a two-valued field
genuinely is weak evidence. So the slice has two halves, and both are required for the headline pair:

1. **Score sex using administrative-sex** (also benefits evidence-richer pairs); and
2. **a scoped banding rule** that forces an identity-pending chart's corroborated pairs to REVIEW —
   the §5.4 point is that an *unconfirmed* chart needs human identification effort, and the paper
   counterpart ("male, about 40 — search the registry") returns a candidate list, not silence. The
   precedent is `banding.py`'s known-alias forcing: a signal that must reach a human is never dropped
   below threshold.

## 2. Pure core — the composite `sex` field

Chosen over (rejected) alternatives: a separate `administrative-sex` FieldSpec double-counts a
heavily-correlated field (breaking F–S independence) and would emit DISAGREE on a trans patient's
true match; a pure union compare replacing sex-at-birth either loses the honest birth-fact clash
signal or turns observed evidence into a suppressor.

- `records.py`: `CandidateRecord` gains `administrative_sex: FieldValue | None = None` (additive).
  New frozen `SexValue(sex_at_birth: str | None, administrative: str | None)` — the composite value
  the comparator sees.
- `comparators.py`: new pure `compare_sex(a, b, ctx)`:
  1. Either side `None` (no sex evidence at all) → `INSUFFICIENT_DATA`.
  2. **Both** sides carry `sex_at_birth` → exact-compare those two values (trim-only, no casefold —
     same discipline as `compare_exact`): `EXACT` / `DISAGREE`. A birth-fact clash stays honest
     negative evidence, aligned with the db/016 veto's subject.
  3. Otherwise the **positive-only union fallback**: each side's set of present values
     (`{sex_at_birth, administrative}` minus `None`/empty-after-trim); either set empty →
     `INSUFFICIENT_DATA`; intersection non-empty → `EXACT`; disjoint → `INSUFFICIENT_DATA`,
     **never DISAGREE**. Clinician-observed evidence may *support* but never *suppress* a match
     (slice B's rule; `compare_identifier_sets` precedent; an apparent-sex misjudgement of an
     androgynous patient must not penalise the true pair). Mirrors the blocking pass's union-sex.
- `orchestrator.py`: the `sex-at-birth` FieldSpec becomes `FieldSpec("sex", compare_sex, …)`.
  Per-side provenance rank: `sex_at_birth`'s rank when present, else `administrative_sex`'s; the
  orchestrator's existing `min(rank_a, rank_b)` then takes the weaker side. This is a documented
  second-order approximation (the branch that fires may in an edge case use the other value's rank);
  the swing is bounded by the [0.5, 1.0] provenance factor on a weight of 1.0.
- `scoring.py`: `DEFAULT_WEIGHTS` key `"sex-at-birth"` → `"sex"`, same weights
  (`EXACT: 1.0, DISAGREE: -2.0`) — one field, one contribution, no double-count.
  `matcher_version()` digests the weight table, so the version pin changes automatically and
  proposals from the new config are distinguishable (per-epoch matcher actor, C2b).
- `adapter.py`: `candidate_from_rows` gains keyword-only `admin_sex_row=None`, shaped through the
  existing `single_field` — the `unknown` uncertainty sentinel already degrades to absence
  (no-data-is-never-agreement, principle 4).

## 3. IO layer

- `db.load_candidate`: the sex SELECT widens to
  `field IN ('sex-at-birth','administrative-sex')` (still one query; rows split by field in Python).
- New `db.load_trust_for(conn, patient_ids) -> dict[str, str]`: batch read of the `chart_trust`
  view for the sweep's whole candidate set (mirrors `load_aliases_for`; the view has rows only for
  flagged charts, so an absent key means *confirmed*). *(Post-review amendment: the planned
  single-pair `load_trust(conn, pid)` was dropped — the per-pair fallback in `propose()` calls the
  same batch loader over the two-element pair, so the view contract lives in one function.)*
  `cairn_agent` already holds SELECT on the view (db/024).

## 4. Banding — the scoped forcing rule

`band()` gains `unconfirmed: bool = False` (mirroring `has_known_alias`). New rule, slotted in the
`score < review` branch beside the shared-identifier rescue:

> If the pair involves at least one chart whose `chart_trust` state is `'unconfirmed'`, and the
> score carries **≥ 2 fields at a positive agreement level** and **zero DISAGREE-level fields**
> → `REVIEW`. Never `AUTO_CANDIDATE` — the below-review forcing only ever yields REVIEW (§5.7
> reserves the *identify* step for a human; an above-threshold link follows the normal
> auto-above-threshold path).

Gates, and why each *(amended by the post-review fix wave on PR #134 — the veto gate was removed
and the positive-field count made level-based; the bullets below describe the shipped rule)*:

- **≥ 2 positive fields** — structural flood control. Age-window overlap + shared sex (or overlap +
  shared identifier off belongings) surfaces; a bare 11-year window overlap — which any sizeable DB
  satisfies for ~a double-digit percentage of charts — does not. Structural (**agreement levels**,
  never weight contributions) so it genuinely does not drift as provenance factors or weights
  change — a B3-learned 0.0 weight on a positive level cannot stand the rule down. The
  paper counterpart searches on age AND sex, not age alone. Per-Doe volume stays bounded by the
  blocking cap (an anchored block contributes at most `cap − 1` pairs).
- **Zero DISAGREE fields** — disagreeing evidence means the normal scoring path should decide;
  forcing is only for pairs whose evidence is thin but uncontradicted.
- **Fires with veto findings present** (post-review amendment; the design originally gated on
  "no vetoes" with a near-vacuous rationale that was factually wrong: `cairn_identifier_veto`
  (db/016) needs NO verified values — disjoint same-system identifiers fire it, e.g. off a Doe's
  belongings — and the identifier comparator is positive-only, so a vetoed-yet-corroborated Doe
  pair is reachable). ADR-0014 §6: a veto forces a human decision, never an auto-reject; the pair
  surfaces as REVIEW with the veto findings attached. A verified birth-fact clash still routes
  through the normal path via the zero-DISAGREE gate.
- **`'under-review'` does NOT trigger it** — that is a dispute state (C3), not identity-pending;
  a disputed chart's pairs go through normal scoring.
- Priority order in `band()`: known-alias forcing (unchanged, first) → then within the
  below-review branch: shared-identifier rescue, then this rule.

The proposal carries an evidence entry appended after the field breakdown (the alias-marker
pattern): `{"kind": "identity_pending", "unconfirmed": [<chart uuid>, …]}` — `"kind"` is the one
discriminator key for every non-field evidence entry (the `known_alias` convention) — so a hub
worklist can group/filter a Doe's candidate list without any suppression here.

`runner.propose()` gains an optional preloaded `trust` map (batch callers supply it; single-pair
calls load on demand — the `aliases` pattern) and passes the flag to `band()`; `sweep` pre-loads
trust states for its candidate set once per run.

## 5. Arithmetic check (the headline pair)

Doe (age 40 ± 5 + observed male, all clinician-observed rank 30) vs prior chart (point DOB inside
the window, sex male):

- dob: range-overlap PARTIAL → 1.5 × 0.714 ≈ 1.07
- sex: union fallback EXACT → 1.0 × 0.714 ≈ 0.71
- total ≈ **1.79** < review 3.0 — but 2 positive fields + 0 disagreements + unconfirmed chart
  → **REVIEW proposal persisted**, carrying the `identity_pending` marker. Closes #130's headline
  gap end-to-end: blocked (slice C) → scored (this slice) → surfaced (this slice).

Evidence-richer pairs additionally gain the sex contribution on the normal path (e.g. the
edit-distance-name + dob-PARTIAL pair that already crossed review now scores higher).

## 6. Tests (TDD, red first)

Pure (no DB):

- `compare_sex` branch matrix: both-sab EXACT/DISAGREE; trans true-match (sab male vs admin female
  present on the same side) never DISAGREEs via fallback; admin-only vs admin-only intersect →
  EXACT; disjoint fallback → INSUFFICIENT_DATA (never DISAGREE); absent sides; empty-after-trim.
- Orchestrator: composite extraction + the rank rule (sab rank preferred, admin rank as fallback);
  weaker-side reduction unchanged.
- Adapter: `admin_sex_row` shaping; `unknown` sentinel on administrative-sex degrades to absence.
- Scoring: `"sex"` weight key wired; `matcher_version` digest changes.
- Banding forcing matrix: fires on the headline shape; blocked by only-1-positive-field; blocked by
  any DISAGREE; blocked by a veto finding; fires when either or both charts unconfirmed; never
  yields AUTO_CANDIDATE; inert when `unconfirmed=False`; evidence marker present in the payload.
- Rename ripple, precisely scoped: `'sex-at-birth'` as the **weight/FieldSpec key** (scoring,
  orchestrator, their tests) renames to `"sex"`; `'sex-at-birth'` as the **projection field name**
  (SQL in `pipeline/db.py`, `eval/blocking_eval.py`, conftest seeding) is the DB contract and MUST
  NOT change. `gold_v1.json` carries no reference to either.

DB-gated (PG18 + cairn_pgx rig, `CAIRN_TEST_PG` :5532):

- `load_candidate` returns `administrative_sex` populated from the projection.
- `load_trust_for`: flagged chart → `'unconfirmed'`; absent → missing key (confirmed default).
- **The #130 end-to-end** (extends `test_observed_age_pipeline.py`): `register-john-doe` +
  `assert-observed-evidence`–shaped chart vs a prior chart with in-window DOB + matching sex →
  `sweep()` produces a REVIEW proposal carrying the `identity_pending` evidence entry.

## 7. Honest limits (recorded)

- Weights and thresholds remain shipped defaults; B3 weight-learning is unchanged and unblocked.
- The forcing rule surfaces a *bounded candidate list*; ranking within it (and per-Doe grouping UI)
  is the worklist tier's job — deliberately not built here.
- Generator range-DOB emission + range-aware eval mirror (the quantitative recall number) remain
  deferred from the 2026-07-04 slice.
- The per-side rank rule's edge-case approximation (§2) is documented in code; revisit only if
  provenance-sensitive tuning (B3) makes it observable.
