# ROADMAP — Cairn

> **Disposable working scaffolding, not a source of truth.** The canonical *what* is the
> [spec](spec/index.md); the *why* is the [ADR log](spec/decisions/README.md). This file only
> orders the build. If it disagrees with the canonical docs, the canonical docs win.

**Scope:** the **foundation** that must exist before the policy and GUI layers. Ordered bottom-up by
the four-layer model ([ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)):
**wire core → in-DB enforcement floor → sync → identity → security → federation → blobs → native
API**. Policy and UI sit *above* this line and are deliberately out of scope here.

## Cross-cutting (applies to every phase)

- **TDD** — failing test first, then code (load-bearing on the §9 safety-critical surface).
- **Language by defect blast radius** ([§9](spec/language-substrate.md)) — safety-critical = Rust or
  in-DB (SQL/PL-pgSQL/pgrx), optimized for reviewer-legibility; advisory/cosmetic = fit-for-purpose
  (Python/ML). The integration boundary is the **PostgreSQL boundary** (≥ 18); avoid FFI coupling.
- **AGPL-3.0** for all code; every dependency AGPL-3.0-compatible (checked *before* adding).
- Each phase takes the relevant **spike → production-grade**; close honest gaps, don't re-spike.

## Phase 0 — Proven foundations (done, as spikes)

- Event serialization + signatures — COSE_Sign1 + Ed25519 + SHA-256 ([ADR-0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)); `cairn-event`, Bet A ✓.
- In-DB floor spiked — validated `submit_event` door + recall, holds against a hostile agent (Spike 0002, C1–C5 ✓); `db/001`–`008`, `cairn_pgx` verify.
- First federating node — admission/pairing/mTLS/set-union `node_event` sync ([ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)); `cairn-node`, floor ENFORCED proof.
- Walking skeleton + WAN sync + replication/failover PoC.

## Phase 1 — Event core to production (the wire contract)

- **HLC ordering + incremental sync watermark** — ✓ done at `cairn-node` level ([issue #38](https://github.com/cairn-ehr/cairn-ehr/issues/38), PR #42): real local HLC, per-peer `seq` cursor via advance-only door, full-sweep correctness floor. Promote the same discipline into the production `cairn-event`/`cairn-sync` core.
- **Legibility twin** — mandatory signed mechanically-derived plaintext twin on every event; promote from skeletal ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md), [§3.13](spec/data-model.md)). **Author-materialised twin globalised to every event type** ✓ done ([ADR-0039](spec/decisions/0039-globalise-authored-legibility-twin.md), SCHEMA 13→14, `db/015`): floor prefers authored twin; non-demographic types degrade honestly to a flagged, payload-rendering derived skeleton when absent; demographic types keep ADR-0034's hard requirement; authored-vs-derived is a derivable read-time projection, no stored flag.
- **Canonical identifiers + node-local surrogate keys** ([ADR-0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)).
- **Additive-only schema evolution** discipline baked into the event format ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).

## Phase 2 — In-DB enforcement floor (unbypassable safety floor)

- **`submit_event` validated write surface** hardened to production ([ADR-0022](spec/decisions/0022-validated-submit-surface-the-write-path.md)); RLS + constraints + append-only envelope; raw-SQL clients still cannot break the floor (principle 12).
- **Actor registry + version-pinning + key custody** ([ADR-0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)); skill-epoch + served-model digest as pinned actor determinants ([ADR-0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md)).
- **Authorship + attestation** — compositional author set, separable responsibility; closed contributor-role enum ([ADR-0007](spec/decisions/0007-authorship-and-accountability.md), [ADR-0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md)); additive-vs-suppressing derived, not declared ([ADR-0010](spec/decisions/0010-additive-vs-suppressing-classification.md)).
- **Advisory-actor integration contract** — L2/L3 attachment through the floor ([ADR-0030](spec/decisions/0030-advisory-actor-integration-contract.md)).
- **Bitemporal time** — `t_recorded` (HLC ceiling) vs freely-backdatable `t_effective`; clashes flagged, never auto-resolved ([ADR-0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)). *Tier-1 ceiling (`t_effective ≤ t_recorded`) now enforced at the `submit_event` door (2026-07-02 review); the graded-interval / RTC-less-Pi refinement + the tier-2 clash flag are [#103](https://github.com/cairn-ehr/cairn-ehr/issues/103) / [#91](https://github.com/cairn-ehr/cairn-ehr/issues/91).*
- **Acknowledged-uncertainty value types** — first-class unknown / not-yet-asked / refused / ranges ([§3.7](spec/data-model.md)).

## Phase 3 — Sync engine (set-union + the two planes)

- **Set-union sync with scope as prefetch hint, not authority** ([ADR-0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)).
- **Two-plane schema/code evolution** — events sync forward-compatibly; code/DDL/pgrx travel a separate signed, per-architecture, sneakernet-capable distribution plane; version is a local node property ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md), [§6.5](spec/sync.md)).
- **Record discovery + replicated essential tier** ([ADR-0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md)).
- **Signing-context domain separation + honest-degradation seams** ([ADR-0040](spec/decisions/0040-signing-context-domain-separation.md), issues #95/#108/#109): one signature per event, domain-separated by a registered signing context (content-type + `external_aad`); durable clinical-plane pull quarantine with a re-offer floor (#108); the verify primitives wired into the doors — every signature door surfaces `cairn_verify_error` as exception DETAIL, cairn-sync fails fast on a stale `cairn_pgx` (`cairn_pgx_version() >= 0.2.0`) at startup, and `event_twin_provenance` exposes a `verifiable` column (#109). Node-event-plane quarantine sibling: #111.
- **Clinical-plane in-DB apply door** — ✓ done ([issue #91](https://github.com/cairn-ehr/cairn-ehr/issues/91), review A2/A5b/M8/H4): `apply_remote_event` (`db/020`), the sibling of `apply_remote_node_event`, so a replicated clinical event faces the SAME floor as a locally-authored one (signature, enrollment, fail-closed classification, attestation gate, twin floor, substitution guard); `cairn-sync` now does zero checks and zero raw DML on apply. Attestation tokens are stored (`db/001` additive columns) and travel on the sync wire so the suppress gate is re-runnable at every hop; `t_effective` wire-pinned to an explicit UTC offset (`cairn_t_effective`, both doors); node-local projection guards clamp-and-flag at apply instead of vetoing (`identity_projection_flag`, db/018). Known residual: the actor registry does not replicate yet, so cross-node apply needs the operator enrollment ceremony (`cairn-sync enroll`) until ADR-0011 registry sync exists.
- **Durable pull-plane quarantine** — ✓ done on both planes: clinical (`cairn-sync`, [#108](https://github.com/cairn-ehr/cairn-ehr/issues/108)/`db/021`) and node-event (`cairn-node` `sync.rs`, [#111](https://github.com/cairn-ehr/cairn-ehr/issues/111)/`db/022`). An UNVERIFIABLE pulled event is penned durably with a re-offer floor (never a silent skip-past), auto-releases when its cause is fixed, and fails the pull loudly until resolved or human-acked; a verifiable-but-refused event stays skip-and-swept (self-healing). No manual requeue on the node plane — the derived floor + full sweep re-offer, and success auto-releases.

## Phase 4 — Identity & demographics subsystem

- **Identity event algebra** — closed link/unlink/reattribute/repudiate/identify/dispute set; immortal UUIDs; never merge/erase ([§5.7](spec/identity.md), principle 2).
- **Demographics assertion stream** — per-field projection policy ([§4](spec/demographics.md)). **Address model specified** ([ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md), [§4.3](spec/demographics.md)): culture-neutral three-facet value (display legibility twin + optional geolocation + culture-tagged structured parts via a content-addressed locale profile reusing ADR-0014). **Patient-identifier representation specified** ([ADR-0033](spec/decisions/0033-patient-identifier-representation.md), [§4.4](spec/demographics.md)): namespace/profile split (stable veto key + versioned validator) + a normalized form materialised so the hard veto survives a profile-less node; advisory validation; professional **licensure/registration** IDs fixed in the §7.5 actor registry (billing/relational provider numbers split out to §4.6, below). **Demographic legibility twin specified** ([ADR-0034](spec/decisions/0034-demographic-legibility-twin.md), [§4.5](spec/demographics.md)): every demographic assertion carries the §3.13 principle-11 twin, materialised profile-independently, with `display`/`value` reconciled as its value-core and a forward guarantee for future field shapes. **Provider-number relational model specified** ([ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md), [§4.6](spec/demographics.md)): abstract entity (open `kind`) + reified relationships carrying their own identifier sets + subject-kind partitioning `{patient, entity, relationship}` as structural non-conflation. **All demographics gaps now closed.** **Demographics IMPLEMENTATION underway** (first production clinical surface, on `cairn-node`). **Slice 1 — §4.4 patient identifiers** (`db/010_demographics.sql`): culture-neutral structural floor + authored §4.5 twin carried through the reused `submit_event` + set-union `patient_identifier` projection; pure `cairn-event::demographics` builders + `EventBody.plaintext_twin`. **Slice 2 — §4.2 DOB + sex-at-birth** (`db/011_demographics_fields.sql`): the *provenance-precedence* mechanic — generic `demographic.field.asserted` event + `cairn_provenance_rank` ladder (incl. new `fact-proven` top tier; unrecognized→0) + winner-by-`(rank,HLC,origin)` `patient_demographic` projection ("verified value locks"); **floor stays open / projection gated** (unknown field stored + legible but not projected — federation-forward per ADR-0012); §4.1 ladder prose extended. **Slice 3 — §4.2 names** (`patient_name` retained-set projection + `patient_name_current` display-winner VIEW): recency-first within the legal-use tier (HLC wins; provenance/origin break ties); falls back to most-recent any-`use` when no legal name exists; all names retained as evidence; deliberately diverges from DOB's provenance-lock ([ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md)). **Slice 4 — §4.2 administrative-sex + gender-identity** (`db/013_demographics_sex_gender.sql`): per-field winner policy via an IMMUTABLE `cairn_demographic_field_policy(field)` classifier; administrative-sex provenance-first (document-anchored; recency breaks equal-provenance ties); gender-identity recency-first (patient's current stated identity always wins regardless of provenance — the inverse of DOB's ordering; provenance still feeds the §5.2 matcher). Karyotype resolved ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) as a distinct field — no karyotype code yet; spec/ADR only. Additive: no new event type, no floor change, no `patient_demographic` schema change; db/013 supersedes db/011's trigger. **Slice 5 — §4.3 address** (`db/014_demographics_address.sql`): retained-set `patient_address` + per-use `patient_address_current` recency-first VIEW (one current address per `use`); additive floor branch; per-use recency-first winner — addresses are volatile, a fresh patient-stated move must displace a stale document-verified address ([ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)). **Slice 6 — §4.4/§5.2 in-DB hard-veto floor (piece A)** (`db/016_match_veto.sql`, SCHEMA 14→15):
`cairn_match_veto(patient_a, patient_b) RETURNS TABLE(veto_kind, severity, subject, detail)` + scalar
`cairn_has_hard_veto`. Implements the closed hard-veto set (§5.13): same-system identifier mismatch ·
verified-DOB clash · verified-sex-at-birth clash. Two verdict levels: `hard_veto` (trustworthy clash —
`normalized` present & disjoint, or both verified + same-precision + differ) vs `degrade_hold` (profile-less
node — holds for human, never auto-demotes). Precision-gated DOB, no date parsing (culture-neutral floor);
set-based per-system identifier comparison (sharing any value = positive evidence); `system: unknown` never
vetoes. Pure SQL helpers over existing projections; no event-format change, no `submit_event` change, no new
table. 12 integration tests, all green. Deceased-status veto deferred (no projection yet; stub in db/016).
**Slice 7 — §5.2/§5.13 advisory matcher scoring core (piece B1)** (`matcher/`, `cairn-matcher` — the first **Python**
component; AGPL-3.0, zero runtime deps, **pure functions only**, fit-for-purpose §9 tier): the comparator API contract
(`agreement.py`; ordinal `AgreementLevel`, `PHONETIC`/`NICKNAME` reserved but never emitted by core — anti-cultural-capture)
+ in-house Jaro–Winkler + 4 culture-neutral comparators (`compare_exact`/`compare_edit_distance`/precision-aware
`compare_dob` (parses no date strings)/history-set `compare_name_set`) + positive-only `compare_identifier_sets` (never
DISAGREE — mismatch stays db/016's job) + the field→comparator registry seam (`orchestrator.py`) + the **Fellegi–Sunter**
combiner (`scoring.py`; `provenance_factor` scaling, `INSUFFICIENT_DATA`→0) producing an explainable `MatchScore`. The three
principle-bearing invariants hold end-to-end (no-data-never-disagreement §3.7; provenance-aware §4.2; name-history-set). 55
pure tests (`uv run pytest`); brainstorm→spec→plan→subagent-SDD; final opus review caught + fixed one Critical (score
symmetry under greedy name-pairing). No new ADR, no spec bump.
**Slice 8 — §5.2 advisory matcher pipeline (piece B2)** (`cairn_matcher/pipeline/`, new `db/017_match_proposal.sql`,
SCHEMA 15→16): the veto-gated **pairwise** pipeline. Pure `adapter.py` (`patient_*` projection rows → B1 `CandidateRecord`;
precision-gated **ISO** DOB extraction — parses no locale date strings, non-ISO→`None`; untagged `sorted()` token-bag names;
identifier sets on `match_key`) + pure `banding.py` (`MatchScore` + db/016 veto findings → `auto_candidate` iff `≥ T_auto`
**and no veto**, else `review`, else `None` — any veto caps at `review`, never auto-link/auto-reject; below `T_review`
persists nothing; `matcher_version` = pkg+weights digest, ADR-0014) + `db.py`/`runner.py` (the only psycopg modules —
`propose` = load→score→call in-DB `cairn_match_veto`→band→[if not None] upsert→commit, one txn, commit owned by the
runner). `db/017` is an **advisory** worklist (PK `(low,high)`, `CHECK(low<high)`, JSONB veto/evidence, human `status`
preserved on re-run) — *not a safety gate*. **psycopg** optional (`pipeline` extra; LGPL→AGPL-ok), B1's pure core
unchanged. 92 tests with DB (5 gated integration) / 87 + 5 skipped without; opus whole-branch review MERGE-READY (0
Critical/0 Important; one Important — commit moved to runner — fixed in-branch).
**Slice 9 — §5.2 advisory matcher blocking + batch sweep (piece B2b)** (`cairn_matcher/pipeline/`, **no `db/` file, no
SCHEMA bump** — advisory): B2 scored a *given* pair; B2b decides **which** pairs to score across the whole patient set
(no O(n²)). Pure read-only `db.generate_candidate_pairs(conn, *, max_block_size=100)` — a **3-pass blocking disjunction**
(shared identifier excl. `unknown` · exact DOB · shared name token), group-based CTEs, deduped to one **canonical**
`(low,high)` per pair by **uuid VALUE** order, self-pairs structurally excluded; an **oversized-block guard** skips +
**reports** (`skipped_blocks`) any group `> max_block_size` (never a silent cap; *C(k,2)* reasoning; hub sweep is the
backstop). New `pipeline/sweep.py` — `SkippedBlock`/`SweepError`/`SweepResult` frozen dataclasses + `sweep()`: phase 1
generate→`rollback` (close read snapshot, xmin guard), phase 2 loop the existing `runner.propose()` per pair (one txn each,
idempotent, human `status` preserved) with **skip-and-report** errors (never aborts the batch). Recall-oriented blocking;
the pure scorer stays the source of truth. No new dep. 113 tests with DB (9 candidate-gen + 5 sweep, incl. a real-monkeypatch
failing-pair) / 93 + 20 skipped without; opus whole-branch review READY-TO-MERGE (0 Critical/0 Important).
**Slice 10 — §5.2 matcher eval harness (piece B3 keystone)** (`cairn_matcher/eval/`, **no `db/` file, no SCHEMA bump**
— advisory measurement substrate): unblocks the measurement-driven B3 items (compound blocking keys, weight-learning).
A new pure-by-default sub-package mirroring `pipeline/`'s pure-core + optional-DB split: `dataset.py` (entity-cluster
JSON format + loader; `record_to_candidate` **reuses the real `candidate_from_rows`** — no drift; `truth_pairs`/`all_pairs`
ground truth), `metrics.py` (confusion + precision/recall/F1 at strict+lenient operating points + auto-false-link-rate +
missed-match-rate + score separation; zero-denominator→0.0, never NaN), `scorer_eval.py` (`evaluate_scorer` runs the
**real** `field_comparisons→score→band`; `weights`/`thresholds`/`config` are params — the weight-learning lever),
`report.py` (+ honest "regression/tuning instrument, not a statistical accuracy claim" caveat), `__main__.py`
(`python -m cairn_matcher.eval`; psycopg lazy so the pure path never imports it), `blocking_eval.py` (DB-gated, `pipeline`
extra: seeds `patient_*` label→uuid5, calls the **real** `generate_candidate_pairs`, `rollback` xmin-guard → pair-completeness
/ reduction-ratio / dropped-true-matches / Σ`C(size,2)` estimate) + a culture-plural `gold_v1.json` fixture. No new dep
(pure core stdlib-only). 146 with DB / 123 + 23 skipped without; opus whole-branch review READY-TO-MERGE (0 Critical/0
Important) + post-review fixes in PR #83 (ephemeral/idempotent blocking seed — no `conn.commit()`; dataset loader
validates name/identifier keys).
**Slice 11 — §5.2 compound blocking key (name-token + birth-year)** (`pipeline/db.py`, **no `db/` file, no SCHEMA bump**
— advisory): one **additive** `UNION ALL` branch in `_GROUPS_SQL` (a `birth_year` CTE + a `name+year` pass) partitions an
over-broad single-name-token block by birth-year so the sub-blocks survive the oversized-block cap, recovering true-match
pairs the cap drops wholesale. Additive ⇒ **recall non-decreasing** (pairs deduped by canonical uuid pair across passes);
also rescues precision-mismatched DOBs (first 4-digit run groups `"1990"`/`"1990-05-12"`, exact-DOB does not). Honest,
culture-neutral degrade (principle 4): birth-year is the **first 4-consecutive-digit run** (`substring(value FROM
'[0-9]{4}')`) — no date parsing, so an ISO value and a day-first import (`"12/05/1990"`) of the same person both group;
a DOB with no 4-digit run stays covered by the single-token pass. 5 new DB-gated tests (rescue / honest-degrade /
precision-mismatch / cross-format / cross-pass dedup); 151 with DB / 123 + 28 skipped without; clean per-task reviews.
Known limitation (user-flagged): year extraction still degrades on 2-digit years and non-Gregorian calendars, to revisit
on real data (advisory — a wrong year only feeds the scorer extra pairs, never a false link). Discovered + filed
[issue #84](https://github.com/cairn-ehr/cairn-ehr/issues/84) (pre-existing test-leak + harness `KeyError`).
**Slice 12 — §5.2 matcher eval synthetic volume generator (piece B3)** (`cairn_matcher/eval/generator.py` +
`eval/generate.py`, **no `db/` file, no SCHEMA bump** — advisory tooling): unblocks measuring blocking at volume without
hand-authoring a large gold set. Pure, stdlib-only `generator.py` (no psycopg): `shares_blocking_key` mirrors the three
base blocking passes; four pure corruption operators (DOB reformat, DOB typo, name edit, identifier mangle); culture-plural
curated name pools; `GenSpec` + `generate_dataset` build seed+one-corrupted-clone entity clusters (cluster size fixed at 2)
with a `_repair` step that **guarantees** every seed↔clone pair stays recoverable by >=1 base blocking key — a regression/
volume instrument, not a statistical accuracy claim (recoverable by construction, not by real-world resemblance). `generate.py`
is the disk/CLI edge (`python -m cairn_matcher.eval.generate --entities N --seed S --out path`), byte-deterministic JSON,
feeding the existing `python -m cairn_matcher.eval` CLI unchanged. No new dep. A drift canary
(`test_eval_generator_sync.py`) pins `shares_blocking_key` to `_GROUPS_SQL` so narrowing a base pass fails the fast
suite. 147 + 29 skipped without DB (pure suite; DB suite 173). DB-gated volume test on a generated 200-entity set at `max_block_size=10_000`:
`pair_completeness==1.0`, 0 dropped true matches, `reduction_ratio≈0.919` (6,467/79,800 pairs) — confirms the recoverability
invariant end-to-end through the real blocking SQL.
**Slice 13 — §5.1/§5.7 identity linkage core (piece C1)** (`crates/cairn-event/src/identity.rs`,
`db/018_identity_linkage.sql` wired into `cairn-node`'s `db.rs` SCHEMA array, length 16→17): the first slice of the
closed §5.7 identity-event algebra. Pure Rust `LinkAssertion` builder (`link_assertion_body`/`unlink_assertion_body`
+ `render_link_twin`/`render_unlink_twin`; confidence omit-when-absent). Two additive event types
`identity.link.asserted`/`identity.unlink.asserted` through the **reused** `submit_event` door (never re-declared);
`cairn_check_link_assertion` culture-neutral structural floor (distinct valid UUID subjects + non-empty provenance;
self-link rejected); extends the `cairn_event_twin` hook (preserves demographic + honest-degrade branches, adds an
identity branch with a HARD authored-twin requirement). `patient_link` HLC-overlay edge table (canonical `(low,high)`,
latest-HLC-wins via `ON CONFLICT … WHERE` strict-greater — out-of-order convergent). `person_member` golden-identity
projection: `person_id` = min-UUID of the connected component, maintained by `cairn_recompute_component` (bounded
recursive-CTE walk from both touched endpoints — correct on merge **and** unmerge/split) with a fail-loud oversize
guard (`cairn_max_component_size()` GUC, default 10000; rejects the offending event, never a silent cap).
`person_chart` thin demonstrated union VIEW (`COALESCE` to self for UUIDs unknown to the link graph). 15 DB-gated
integration tests (floor accept/reject; edge overlay + out-of-order convergence; pair/transitive/
diamond-unlink-stays-merged/chain-unlink-splits/idempotent/oversize-guard component cases; VIEW union +
unlinked-defaults-to-self); full cairn-node suite green, clippy `--tests` clean. **Additive, no `db/` floor bypass, no
SCHEMA/ADR/spec change** (implements settled §5.1/§5.7/ADR-0014). Deferred: C2 (below), C3+ (below), an
accept-at-cap boundary test for the oversize guard.

**Slice 14 — §5.2/§5.7 match_proposal→apply seam (piece C2)** (`db/019_apply_proposal.sql` wired into `db.rs`
SCHEMA array 17→18; `crates/cairn-node/src/apply_proposal.rs`): a **human-accepted** advisory proposal becomes a
real **human-attested** `identity.link.asserted` event through the C1 door. **Human-accepted only** (auto-apply of
the `auto_candidate` band deferred to C2b) and the accepting reviewer is a **responsibility-bearing (attested)
contributor** — a human vouching for a patient merge bears responsibility. **Key property: no floor change** — the
link is additive, but placing a responsibility-bearing contributor trips the existing db/005 attestation gate
(valid human token, bound to the event), so C2 composes settled §5.7 (C1) + ADR-0030 (attestation) + ADR-0014
verbatim; `submit_event` untouched, no new event type. Additive `applied_event_id UUID` column on `match_proposal`.
Pure `compose_provenance` + `build_attested_link_body` (event_id caller-supplied → deterministic) + IO
`apply_accepted_proposal` (read accepted proposal → sign+attest with the human key → 3-arg `submit_event` →
mark `status='applied'`+`applied_event_id`, all in **one transaction** ⇒ atomicity is the idempotency guarantee).
The accepted-proposal read is `SELECT … FOR UPDATE` (concurrent applies of one pair serialize; the loser bails on
the `'applied'` status rather than both appending a link event) and the `(low, high)` args are canonicalized (a
reverse-order pair still finds the proposal) — both PR-review hardening applied in-branch.
6 tests (3 pure unit + happy-path projection + idempotency/non-human-attester-refused/pending-not-applied/reverse-order-pair);
full cairn-node suite green, clippy `--tests` clean. **Additive, no ADR/spec change** (implements settled
§5.2/§5.7/ADR-0030/ADR-0014). Deferred: **C2b** auto-apply of `auto_candidate`; matcher as a compositional
contributor (needs §7.5 matcher-actor registration — lives in provenance string for now); a CLI subcommand +
production human-key custody (ADR-0011).

**Slice 15 — §5.2/§5.7 auto-apply of the `auto_candidate` band (piece C2b)** (`crates/cairn-node/src/matcher_actor.rs`
+ `auto_apply.rs`; `apply-auto-candidates` CLI): a matcher proposal banded `auto_candidate` (score ≥ auto AND zero
vetoes at propose time) becomes a **matcher-authored, un-attested, recallable** `identity.link.asserted` event —
**no human in the loop** — through the *same* `submit_event` door. **No `db/` migration, no floor change,
no SCHEMA/ADR/spec bump** (the db/018 floor already made an identity link additive + `targets_other_author=FALSE`, so
an un-attested matcher link needs no attestation) — the change is Rust plus two comment-only clarifications in
`db/017`/`db/019` (the new `auto_applied` status makes db/019's documented `applied` invariant honest; no DDL).
Realises the deferred **§7.5 matcher-actor** piece: each distinct
`matcher_version` is its OWN `agent` actor with its OWN key (auto-enrolled, owner ceremony), pinned under `skill_epoch`
so the db/006 `events_by_actor_epoch` recall selects a bad config's auto-links **precisely** (contamination cascade).
Contributor role `suggested` (ADR-0028 contributory, no `responsibility`) ⇒ authorship present, accountability absent
(principle 10). **Apply-time veto re-check** (the no-human-backstop safety add): a since-vetoed pair is kicked to human
`review`, never auto-linked over. Status `pending → auto_applied` (distinct from C2's human `applied`) or `→ review`;
idempotent (only `pending` picked up), respects a human `rejected`. 6 pure + 7 DB-gated tests (enroll-once/reuse,
distinct-epoch actors, un-attested link + person projection, veto→review, human-rejected skipped, batch+idempotent,
recall precision) + end-to-end CLI smoke; full cairn-node suite + workspace clippy green. Deferred: no background
scheduler (operator-invoked CLI only); matcher key sealed but no recovery escrow (regenerable); ADR-0028 role enum
still not DB-enforced ([#96](https://github.com/cairn-ehr/cairn-ehr/issues/96)).

**Slice 16 — §5.7 `dispute` + the chart trust-state projection (piece C3)** (`db/023_identity_dispute.sql` wired into
`db.rs`; `crates/cairn-event/src/identity.rs` dispute builders): the patient-initiated "I was never there" front door
(§5.5(b) identity theft) **and** the §5.7 projection-side contract — the chart **trust state** (*confirmed /
under-review*) — the keystone that C1 explicitly deferred and that the rest of the algebra composes into. Two
**additive** dispute event types (`identity.dispute.asserted` / `.resolved`) through the reused `submit_event` door
(low-ceremony like the C1 link; a dispute annotates trust, never erases/moves/blocks — attestation only if a
responsibility-bearing contributor is named); a culture-neutral `cairn_check_dispute_assertion` structural floor +
HARD-required legibility twin; a `chart_dispute` standing overlay keyed by the dispute's own id (HLC-latest wins,
converges out-of-order — the C1 `patient_link` shape, but a single-row fact so no BFS/oversize guard); a `chart_trust`
effective-state **VIEW** shaped so `identify`/`reattribute`/the §5.2 coherence check ADD source branches later (never a
rewrite); surfaced via a `person_chart_trust` view **composing on top of** C1's `person_chart` (reusing its
`person_member` join). **No SCHEMA/ADR/spec bump; `db/018` untouched** (implements settled §5.7). A review finding
steered the composition: extending `person_chart` in place would need `DROP+CREATE` (since `CREATE OR REPLACE VIEW`
cannot shrink an already-extended view across the `connect_and_load_schema` reload), and a bare `DROP` would abort
node boot once any dependent view sits on `person_chart` — so a separate composing view keeps `person_chart`
droppable-free. 3 pure builder unit tests + 14
DB-gated integration tests (accept, HLC overlay, out-of-order convergence, multi-dispute resolve, idempotent
re-assert, dispute-before-chart safety signal, five floor rejections); full workspace suite + clippy green on PG18 /
cairn_pgx 0.2.0. Deferred: the *unconfirmed* (identity-pending) state + registration classes / John Doe (C4/C5 with
`identify`); `reattribute` (§5.5 strike-through + tiered adjudication) and `repudiate` (alias pool); the §5.2 coherence
feedback loop; notification/contamination cascade on dispute; person-level trust aggregation (read-surface tier).

**Slice 17 — §5.4/§5.7 `identify` + the *unconfirmed* trust state (piece C4)** (`db/024_identity_identify.sql` wired
into `db.rs`; `crates/cairn-event/src/identity.rs` pending/identify builders): the third and final state of the §5.7
trust-state contract C3 opened. Two **additive** event types — `identity.pending.asserted` (the §5.4 John-Doe front
door: marks a chart identity-pending → *unconfirmed*) and `identity.identify.asserted` (§5.7 "who, method": establishes
identity → *confirmed*) — through the reused `submit_event` door (low-ceremony like C1/C3; `method` structurally
required = "method recorded", the "Human" vouching composing via the existing attestation gate when a
responsibility-bearing contributor is named). Keyed by the **subject itself** (a per-chart lifecycle state, unlike a
dispute's own id ⇒ *no* subject-consistency guard is possible or needed); `chart_identity_state` HLC-overlay table
(latest-HLC wins, full pending⇄identified lifecycle, out-of-order convergent, no BFS/oversize guard). `chart_trust`
reworked into a **severity-max UNION** composing **under-review (open dispute, 2) over unconfirmed (pending, 1)** — the
"highest standing assertion" discipline (§5.9) — with the column contract unchanged so `CREATE OR REPLACE VIEW` stays
reload-idempotent and C3's `person_chart_trust` is untouched (it now surfaces `unconfirmed` for free). Precedence
documented: under-review (attribution actively challenged, data present possibly wrong-patient) outranks unconfirmed
(who-is-this unknown, absent history). **No SCHEMA/ADR/spec bump; `db/023` untouched** (implements settled
§5.4/§5.7; CREATE-OR-REPLACEs the shared twin hook + `chart_trust`). 3 pure builder unit tests + 15 DB-gated
integration tests (accept, HLC overlay both directions, re-pending-reopens lifecycle, idempotent re-assert, pending→
unconfirmed on `chart_trust`+`person_chart_trust`, identify→confirmed, pending-before-chart safety signal, the **C3⊔C4
compose/precedence proof** — dispute outranks pending → resolve → identify, five floor rejections); full workspace
suite + clippy green on PG16 / cairn_pgx 0.2.0. Deferred: the full §5.4 John-Doe registration subsystem (callsign,
clinician-observed evidence assertions, matcher re-run), the "prior history now available" push alert on link,
registration-class funnel partitioning (§5.3/§5.8); `reattribute` (§5.5 strike-through + tiered adjudication) and
`repudiate` (alias pool); the §5.2 coherence feedback loop; person-level trust aggregation (read-surface tier).

**Slice 18 — §5.5(a)/§5.7 `repudiate` + the known-alias pool (piece C5)** (`db/025_identity_repudiate.sql` wired into
`db.rs`; `crates/cairn-event/src/identity.rs` repudiate builder): the **first *suppressing*** identity event (C1–C4 were
all additive/annotative). The §5.5(a) fabricated-persona case — a patient presented under a deliberately false name;
once established false, the name is struck from the display header but stays in the record (fact of presentation
preserved, principle 1) and enters a matcher-visible **known-alias pool** (aliases are reused). One event type
`identity.repudiate.asserted` registered **`mode='suppressing'`**, so the db/005 attestation gate structurally forces a
valid **human** attestation token — §5.7's "Human" made unbypassable in the DB (no floor special-case; reuses the
`salience.downgrade` gate). This is the deliberate contrast with the additive C1/C3/C4 (whose "human vouches" bit only
when a responsibility contributor was named). **Digital strike-through** (principle 1+2): the assertion event and
db/012's `patient_name` retained set are **untouched**; a **value-grained** `name_repudiation` overlay (keyed by
`(subject, value)` — a false name is false however labelled, and value-keying avoids replicating db/012's `use`-fold →
no drift; HLC-latest-wins so a future reversal composes) records the struck value, and `patient_name_current` is
`CREATE-OR-REPLACE`d to **anti-join** it (same column contract ⇒ reload-idempotent). New `patient_alias_pool` VIEW
surfaces struck names to the §5.2 matcher. `cairn_check_repudiation_assertion` structural floor (valid subject uuid,
non-empty value + reason) + **HARD-required legibility twin**. **Design call** (documented): striking a chart's *only*
name → `patient_name_current` has *no* row for it — honest (name genuinely unknown-now; showing the known-false one is
a precise untruth, principle 4); "header shows something" is satisfied one layer up by the §5.4 callsign / *unconfirmed*
rendering (C4). **Honest limit:** value match is exact-string on an opaque value (culture-neutral, deterministic — the
floor must be precise); fuzzy recognition of a returning alias is the advisory matcher's job over `patient_alias_pool`.
**No SCHEMA/ADR/spec bump; db/010–024 untouched** (implements settled §5.5/§5.7; CREATE-OR-REPLACEs the shared twin
hook + `patient_name_current`). 2 pure builder unit tests + 10 DB-gated integration tests (struck name leaves winner +
surviving name takes over + alias-pool entry + retained-set evidence preserved; only-name → no winner; idempotent
re-assert + HLC-latest reason; newer re-assertion does NOT un-strike [HLC-blind anti-join pinned]; **un-attested AND
agent-attested repudiation refused** — the suppressing "Human" floor; four floor rejections). Full workspace suite
(364 passed / 0 failed) + clippy green on a from-scratch PG16 / cairn_pgx 0.2.0 in-container rig. **Review hardening**
(3-agent adversarial pass, 0 hard bugs): `patient_alias_pool` made reason-free + base overlay not agent-granted (no
cross-patient forensic-`reason` leak, ADR-0006); `reason` NOT NULL; HLC-blind + agent-attested-refused tests added.
**Deferred:** a reversal / de-repudiation event (the overlay is HLC-versioned so it composes without a
rewrite); a chart-history VIEW rendering struck names (data already present); ~~matcher wiring that *consumes*
`patient_alias_pool`~~ **(done — slice 19, below)**; `reattribute` (needs a clinical-note surface that does not yet
exist — premature).

**Slice 19 — the §5.2 matcher consumes `patient_alias_pool` (known-alias evidence)** (advisory Python;
`matcher/src/cairn_matcher/pipeline/{alias,db,runner,banding}.py`; **no `db/` floor, no SCHEMA/ADR/spec bump**).
Closes C5's deferred matcher wiring. **Key finding:** because C5 left db/012's `patient_name` retained set
physically untouched, a struck name is still a blocking token *and* still scored, so the returning-persona pair
**already** gets proposed — consuming the alias pool does **not** enable a missing match. Its genuine value is
**explainability / paper-parity**: the proposal now carries a `known_alias` evidence entry restoring the registry's
"known alias" flag to the worklist. New pure `known_alias_evidence` (`pipeline/alias.py`) recognises a repudiated
alias corroborated by the other chart in **normalized space** (NFC + casefold token-bag, reusing the adapter's
`_name_bag` so "same name" is byte-identical to the scorer — no drift); `band()` gains `has_known_alias` → always
**REVIEW** (never dropped below threshold, never auto-linked on a name a chart declared false — §5.7 "Human");
`build_payload` appends the entries; `runner.propose` reads the reason-free `patient_alias_pool` (ADR-0006
confidentiality preserved) for both charts and threads it through. **Flag, never suppress** — the deliberate call:
the matcher cannot distinguish a returning fabricated persona from a real, different bearer of that false name, so
suppression would kill the very §5.5(a) recognition it exists to serve; only a human can adjudicate. 6 pure alias
tests + 4 banding tests + 3 DB-gated e2e (`test_alias_pipeline.py`); conftest extended to apply db/018–025.
**Deferred (recorded):** fuzzy/edit-distance alias recognition (this cut is normalized-exact); a dedicated `alias`
blocking pass (zero recall today — the name-token pass already generates the identical pair; pure future-proofing);
any scoring-weight treatment of known-false names (declined by design — needs B3 weight-learning + a spec call).

**Remaining matcher pieces:** **B3** — weight-learning (measurable via the harness) + further compound keys
(`dob+first-initial`, `name+sex`) + locale comparator packs (phonetic/nickname + content-addressed profiles) + hub-tier
aggressive duplicate-sweep + proposal retraction + full §7.5 matcher actor registration; an A/B pass-toggle in
`generate_candidate_pairs` for one-command compound-key before/after (today it's git-revert). **Identity: pieces C1
(the §5.1/§5.7 linkage core — `db/018`) and C2 (the `match_proposal`→apply seam — `db/019`, `apply_proposal.rs`)
are now BUILT** (slices 13–14, above), as is **C2b** — auto-apply of the `auto_candidate` band (slice 15, above),
**C3** — `dispute` + the chart trust-state projection (slice 16, above) — and **C4** — `identify` + the *unconfirmed*
trust state (slice 17, above), which completes the §5.7 confirmed/unconfirmed/under-review contract, and **C5** —
`repudiate` + the known-alias pool (slice 18, above), the first *suppressing* identity event. Remaining:
**C5+** — the rest of the §5.7 algebra (`reattribute` §5.5 event-granular strike-through + tiered adjudication — waits on
a clinical-note surface) + the full §5.4 John-Doe registration subsystem. **Next:**
weight-learning, C5+, or the matcher-actor's fuller §7.5 registration (served-model digest etc.);
the A/B pass-toggle (would unblock quantitative compound-key before/after) + veto-aware
scorer mode; variable cluster size / an unrecoverable fraction / hard negatives in the volume generator; a
`compare_address` comparator; a CLI sweep entry; B2 follow-up Minors → [issue #79](https://github.com/cairn-ehr/cairn-ehr/issues/79).
([Issue #69](https://github.com/cairn-ehr/cairn-ehr/issues/69): codebase-wide projection-tiebreak collation canonicalization, deferred.)
- **Point-of-care identity, possession semantics, `sign-as` salvage** ([ADR-0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)).
- **Locale-pluggable matcher comparators** — *advisory only* (Python/ML); comparator-profile tag travels with each demographic assertion, degrades honestly to human review ([ADR-0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md)).

## Phase 5 — Security & compliance core

- **Erasure = key-custody redistribution / crypto-shred** on the severity ladder ([ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9).
- **Visibility-scope ≠ replication; the safety projection** — sealed bodies emit de-identified, severity-graded safety projection; sensitivity is a graded append-only stream ([ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)).
- **At-rest seal** — ✓ done at node level (ADR-0026 **slice A**): signing key sealed with a dual-recipient
  envelope (Argon2id KEKs from an operational passphrase + a one-time off-node recovery code; XChaCha20-Poly1305),
  recovery escrow minted at `init`, `seal-key` migration.
- **Backup-as-cold-peer (export + health)** — ✓ done at node level (ADR-0026 **slice B**): `backup`/`verify-backup`
  CLI + `last_backup` status; signed-event medium, self-verifying via the existing signature invariant; fail-safe
  node-local health sidecar; shared `fsio` atomic-write.
- **Restore-apply + new-identity `supersede`** — ✓ done at node level (ADR-0026 **slice C**, [issue #50](https://github.com/cairn-ehr/cairn-ehr/issues/50)):
  `cairn-node restore` rehydrates the `node_event` log into a fresh DB via a self-trusting `restore_node_event` door
  (empty-genesis fenced — a no-op on a live node), mints a fresh key, records a `supersede`(dead→new); `db/009` op
  `supersede` + `node_lineage`; `status` `supersedes` line. **Cold-medium self-identification** ([#53](https://github.com/cairn-ehr/cairn-ehr/issues/53),
  2026-06-26): a federated medium can't be self-identified from its (convergent) events, so the backup writes a
  **container-level self-marker** — `crates/cairn-node/src/medium.rs`, `CAIRNB2` format; a **signed** `node.self_attested`
  (unforgeable + event-set-bound via `event_set_commitment`, rejecting a different-set splice) or **unsigned** (operator-error-safe).
  `restore::resolve_dead_node` rejects a peer/off-medium `--superseded-node` fail-closed. Known residual (code review): the
  commitment binds to set *content*, so it can't reject a peer's genuine marker spliced between **byte-identical converged**
  media; impossible on a sole-enroll medium, so multi-enroll restores report `Provenance::SignedFederated` → confirm-on-restore.
  Net: forgery-proof always; misdirect-proof for sole-enroll + different-set splices; converged-peer splice is confirm-on-restore.
- **Sealed local-state export** — ✓ done at node level (ADR-0026 **slice D**): a long-lived local-state DEK dual-wrapped
  once at provisioning (op-pass + recovery code, point-5 compliant); `CAIRNL1` export co-located with the backup medium +
  `CAIRNX1` `.lsk` sidecar; additive-CBOR `LocalState` with typed-empty slots + DB read/apply **seams** the clinical tier
  extends; signing key never in the bundle (point 4); `establish-local-state-key` + `status` line; honest-degrades on
  absent/corrupt export. `localstate.rs` (no schema change). **All ADR-0026 slices (A–D) complete.**
- **Uniform key-material zeroization** — ✓ done ([#54](https://github.com/cairn-ehr/cairn-ehr/issues/54), 2026-06-26):
  every transient KEK/DEK/seed/LSK held in `Zeroizing` (wiped on drop) across `seal.rs` + `localstate.rs`; key-yielding
  functions return `Zeroizing<[u8;32]>`. Remaining optional follow-on: escrow rungs (Shamir M-of-N, QR, TPM/keyring)
  ([ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md)).
- **Trusted-time anchoring** — graded-interval `t_recorded` with clock-confidence grade; transparency-log multi-anchor existence proof ([ADR-0027](spec/decisions/0027-trusted-time-anchoring.md)).
- **Audit-log integrity, offline auth, mTLS** ([§7](spec/security.md)).

## Phase 6 — Federation hardening

- **Revocation cascade; anchor-as-power** ([ADR-0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md)).
- **DR / recovery escrow** — ✓ done at node level (ADR-0026 slices A–D, see Phase 5); uniform key zeroization
  ([#54](https://github.com/cairn-ehr/cairn-ehr/issues/54)) ✓ done. Federation-tier follow-ons: peer-quorum (social)
  recovery + escrow rungs (Shamir M-of-N, QR, TPM/keyring).
- **Node-identity `supersede`** — ✓ done (ADR-0026 slice C). **Signing-key rotation** (`rotate-key` actor event) — still reserved, not built.

## Phase 7 — Attachments / byte tier

- **Content-addressed lazy blobs** referenced by the signed event, never inlined; day-one attachment-reference shape ([ADR-0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)).
- **Resource-isolated byte tier** — chunked/preemptible/separately-budgeted; can never starve clinical sync; opt-in byte replication; self-verifying swarm fetch.
- **Rendition set** — the binary's legibility twin (retrievability axis); per-blob DEK crypto-shred inherits.

## Phase 8 — Native API contract (the boundary below the application)

- **Native API: capability-described + conformance-tested, evolves additively** ([ADR-0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md)); the four-layer boundary sits *below* policy/UI ([ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)).
- **Author-scoped export** — the medico-legal copy ([ADR-0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md)).
- **FHIR interop façade** — distinct from the native API ([§9.7](spec/language-substrate.md)).

## Phase 9 — Terminology services

- **ICD-11 canonical interlingua + local-terminology overlay** ([ADR-0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)).

---

## Above the foundation line (NOT in this roadmap)

- **Policy layer** — hard policy as a signed policy-assertion stream + effective-policy projection ([ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)); soft policy in UI.
- **GUI / reference UI** — built only on the same public native API everyone else uses (principle 12); paper-parity is the governing law, **no confirmation dialogs as a safety mechanism**.
- **Active-write thin encounters** and clinical workflow surfaces ([ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)).

## Parallel build-prep (not blocking the critical path)

- **Bet B — Pi compute-cost run** — **Ran 2026-06-25 on Pi 5 / 8 GB → PASS** ([PR #57](https://github.com/cairn-ehr/cairn-ehr/pull/57)): all §6 gates green with headroom; B4 confirms ADR-0015's BLAKE3 blob-digest default (BLAKE3 ~4× SHA-256 on Cortex-A76). `cairn_pgx` now PG-18-capable (pgrx 0.18.1, [PR #56](https://github.com/cairn-ehr/cairn-ehr/pull/56)). Open follow-ups: clean re-run on PG 18 + USB-3 SSD + 27 W PSU for authoritative precision numbers; drop "provisional" from the ADR-0015 blob-digest line.
- **Spike 0003 — Postgres on Android** — **Ran 2026-06-25, G0–G3 PASS**: native PG 18.2 + a cross-built pgrx extension (incl. SPI) on a stock Android 16 phone; validates the fractal-topology invariant at the phone tier. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/). Remaining gaps (from-source PG build, APK packaging) are non-load-bearing.
- **Continued clinical case-mining** — the highest-signal mode for stress-testing the primitives before product build.
