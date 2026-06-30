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
- **Bitemporal time** — `t_recorded` (HLC ceiling) vs freely-backdatable `t_effective`; clashes flagged, never auto-resolved ([ADR-0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)).
- **Acknowledged-uncertainty value types** — first-class unknown / not-yet-asked / refused / ranges ([§3.7](spec/data-model.md)).

## Phase 3 — Sync engine (set-union + the two planes)

- **Set-union sync with scope as prefetch hint, not authority** ([ADR-0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)).
- **Two-plane schema/code evolution** — events sync forward-compatibly; code/DDL/pgrx travel a separate signed, per-architecture, sneakernet-capable distribution plane; version is a local node property ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md), [§6.5](spec/sync.md)).
- **Record discovery + replicated essential tier** ([ADR-0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md)).

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
**Remaining matcher pieces:** **B3** — weight-learning (measurable via the harness) + further compound keys
(`dob+first-initial`, `name+sex`) + locale comparator packs (phonetic/nickname + content-addressed profiles) + hub-tier
aggressive duplicate-sweep + proposal retraction + full §7.5 matcher actor registration; **piece C** — the **§5.7
link-apply seam** (needs the identity event algebra). **Next:** weight-learning, or piece C; a synthetic corruption /
volume generator (same dataset format; unblocks quantitative compound-key before/after) + veto-aware scorer mode; a
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
