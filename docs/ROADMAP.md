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

- **HLC ordering + incremental sync watermark** — ✓ done at `cairn-node` level ([issue #38](https://github.com/cairn-ehr/cairn-ehr/issues/38), PR #42): real local HLC, per-peer `seq` cursor via advance-only door, full-sweep correctness floor. Promote the same discipline into the production `cairn-event`/`cairn-sync` core. **Clock-drift admission ceiling** ✓ done (PR #133, closes the [#102](https://github.com/cairn-ehr/cairn-ehr/issues/102) ratchet finding): shared `cairn_max_hlc_drift_ms()` (24h) bounds a remote event's asserted wall against our own `clock_timestamp()` on BOTH remote-apply doors — node plane REJECTs (self-healing skip+re-offer), clinical plane ADMITs-but-CLAMPs the `hlc_state` merge (a refusal would wedge `cairn-sync`'s frozen watermark; the event's asserted wall is preserved verbatim, principle 1). Same PR added the CI **Rust workspace + in-DB floor test gate** (`.github/workflows/rust.yml`, [#117](https://github.com/cairn-ehr/cairn-ehr/issues/117)). **CI hygiene gates extended** ✓ (PR #143): `fmt` (rustfmt-defaults, whole-workspace reformat + check on both cargo trees), `deny` (cargo-deny 0.19.9 — AGPL-compat license allow-list + RUSTSEC advisories + wildcard/source bans, `deny.toml`), and `matcher.yml` (ruff + pytest for the advisory Python tier). **Toolchain pinned** ✓ (PR #147, merged; closes [#144](https://github.com/cairn-ehr/cairn-ehr/issues/144)): `rust-toolchain.toml` pins the exact channel (`1.96.0`) + rustfmt/clippy components for both cargo trees (stops fmt-gate drift), `[workspace.lints]` mirrors the CI `-D warnings` gate locally, honest `rust-version` `1.74`→`1.96`, and the `test` job now gates on **PG18** (PGDG apt repo) matching the shipped `pg18` default. **CI gaps closed** ✓ (PR #149): the matcher DB-gated suite now runs in the floor `test` job against the same PG18+`cairn_pgx` cluster ([#145](https://github.com/cairn-ehr/cairn-ehr/issues/145)); CodeQL test-fixture crypto false positives fixed at the source — runtime-derived test seed/salt/nonce + a CLAUDE.md house rule ([#146](https://github.com/cairn-ehr/cairn-ehr/issues/146)); the required-check set is documented in `CONTRIBUTING.md` ([#117](https://github.com/cairn-ehr/cairn-ehr/issues/117)); and the **stricter ruff ruleset** (I/UP/B/E5 at `line-length=100`, Rust-parity) is now enforced in `matcher.yml` — closing the last PR #143 deferral.
- **Legibility twin** — mandatory signed mechanically-derived plaintext twin on every event; promote from skeletal ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md), [§3.13](spec/data-model.md)). **Author-materialised twin globalised to every event type** ✓ done ([ADR-0039](spec/decisions/0039-globalise-authored-legibility-twin.md), SCHEMA 13→14, `db/015`): floor prefers authored twin; non-demographic types degrade honestly to a flagged, payload-rendering derived skeleton when absent; demographic types keep ADR-0034's hard requirement; authored-vs-derived is a derivable read-time projection, no stored flag.
- **Canonical identifiers + node-local surrogate keys** ([ADR-0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)).
- **Additive-only schema evolution** discipline baked into the event format ([ADR-0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).

## Phase 2 — In-DB enforcement floor (unbypassable safety floor)

- **`submit_event` validated write surface** hardened to production ([ADR-0022](spec/decisions/0022-validated-submit-surface-the-write-path.md)); RLS + constraints + append-only envelope; raw-SQL clients still cannot break the floor (principle 12).
- **Actor registry + version-pinning + key custody** ([ADR-0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)); skill-epoch + served-model digest as pinned actor determinants ([ADR-0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md)). **Enroll collision floor now ENFORCED** ✓ ([ADR-0044](spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md), closes [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)): since `actor_id = content-address(pinned set)` alone (the key stays mutable across `rotate-key`), two distinct keys with an identical pinned set collided into one `actor_id` and `actor_current` silently dropped the earlier — a silent identity-merge (principle 2). `enroll_actor` now fails closed on a distinct-key collision across the whole `actor_event` history (immortal even after `revoke`); idempotent same-key re-enroll passes. Single door (no actor-sync apply door yet); humans carry a person-distinguishing determinant (guidance). **Now bidirectional** ✓ ([ADR-0046](spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md), closes [#166](https://github.com/cairn-ehr/cairn-ehr/issues/166)): the A-direction (one `actor_id` ← two keys) is joined by the **B-direction** (one key → two `actor_id`s), which `submit_event` (db/005) would otherwise punish by NULLing that key's authorship node-wide. A new pure whole-history predicate `cairn_key_actor_id_conflict` + a per-key advisory lock (key-lock-first → deadlock-free) refuse it; idempotent/distinct-key/matcher-per-epoch enrolls are unaffected. Both future doors that bind a key to an actor (rotate-key/`supersede`, actor-sync apply) must mirror both checks.
- **Deterministic overlay convergence now ENFORCED** ✓ (closes [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115) part 1): every standing-state overlay folds a new event in via one shared pure `cairn_hlc_overlay_wins()` predicate that appends the event `content_address` (BYTEA multihash — canonical, UNIQUE, collation-free) as the deterministic final tiebreaker after `(hlc_wall, hlc_counter, origin)`. Before, two distinct events sharing an identical HLC triple (a Byzantine/broken signer reusing its own triple) settled by arrival order → silent cross-node divergence in the safety-critical projection layer (clinician-visible for `chart_dispute`). Applied to the five uniform state overlays — `patient_chart` (db/002), `patient_link` (db/018), `chart_dispute` (db/023), `chart_identity_state` (db/024), `name_repudiation` (db/025). Projection-read-side only (no wire/event-format/ADR/spec change). Demographic overlays (db/010–014) then closed their residual TEXT-collation gap — see the collation bullet below (#69). #115 part 2 (twin-ladder registry, `cairn_require_uuid`) still open. **Byzantine collision now also SURFACED** ✓ (closes [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157)): the tiebreaker resolved a genuine HLC-triple collision (proof of a broken/hostile signer) silently; `db/029_hlc_collision_log.sql` adds a shared pure `cairn_hlc_triple_collision()` predicate + a **convergent** append-only `hlc_collision_log` (canonical unordered `content_address` pair as PK → one row per 2-way collision per node) + a **structurally** non-gating recorder (`INSERT ... SELECT` with a null-guard `WHERE` + `ON CONFLICT DO NOTHING` → can never raise, so it cannot gate the apply path by construction), and each of the five overlay triggers records the signal before its unchanged upsert. Advisory/observability only (accepted limits: a concurrent apply may miss the signal; a ≥3-way collision records a non-convergent pairwise chain — the §5.13 sweep is the backstop, the resolution stays correct regardless); the Python §5.13-sweep / human-worklist consumer is a documented future seam.
- **Collation-independent projection tiebreaks now ENFORCED** ✓ (closes [#69](https://github.com/cairn-ehr/cairn-ehr/issues/69); [ADR-0045](spec/decisions/0045-collation-independent-projection-tiebreaks.md), spec v0.46): every projection winner tiebreak over a TEXT key (`node_origin`/`asserted_origin` + the final `value`/`display`/`use_key`) now compares under **`COLLATE "C"`** (byte order of the identical-on-every-node UTF-8 bytes), so a `(rank,wall,counter)` tie converges to the same display winner across a federation of mixed default collations — before, the default (possibly locale/ICU) collation was a node-local property, so honest nodes could pick different winners (the cross-origin `(wall,counter)` tie needs no misbehavior; it was decided before #115's collation-free `content_address`). One shared `cairn_hlc_overlay_wins` fix (db/002) covers the five overlays; inline `COLLATE "C"` on `patient_identifier` (db/010), `patient_demographic` (db/013 both branches + `cairn_demographic_backfill`; db/011 superseded), `patient_name` (db/012 trigger + `patient_name_current` VIEW **and its db/025 re-definition**), `patient_address` (db/014 trigger + VIEW). Projection-read-side only (no wire/floor/SCHEMA change). ADR-0045 makes the invariant binding on future projection slices. Drift follow-up ✓ (closes [#159](https://github.com/cairn-ehr/cairn-ehr/issues/159)): the `patient_name_current` winner ORDER BY is duplicated across db/012 + db/025 (db/025's copy is live), with nothing in SQL keeping them in lockstep (DISTINCT ON + the pre-winner anti-join preclude a shared base view). Guarded now by a no-DB source-level test (`crates/cairn-node/tests/name_winner_order_drift.rs`) asserting the two clauses stay byte-identical, catching drift in either direction; cross-reference DRIFT comments added to both migrations.
- **Authorship + attestation** — compositional author set, separable responsibility; closed contributor-role enum ([ADR-0007](spec/decisions/0007-authorship-and-accountability.md), [ADR-0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md)); additive-vs-suppressing derived, not declared ([ADR-0010](spec/decisions/0010-additive-vs-suppressing-classification.md)). **Suppression owner-gate now ENFORCED** ✓ (ADR-0043, closes the last open sub-item of [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99)): a suppressing overlay of a **human author's** event is self-only (cross-human suppression refused — disagreement is additive; agent/un-owned advisories stay dismissable, principle 10), enforced identically at both write doors via one shared `cairn_suppression_author_ok` helper (`db/005` + `db/020`, principle 12). §5.9 sensitivity-sealing + `repudiate` carved out.
- **Twin-check dispatch de-risked** ✓ ([#173](https://github.com/cairn-ehr/cairn-ehr/issues/173); [ADR-0048](spec/decisions/0048-twin-check-registry-dispatch.md), spec v0.49): the per-type structural-floor + legibility-twin dispatcher `cairn_event_twin` was re-declared in 11 migrations, each copying the whole growing IF/ELSIF chain — a stale copy could silently DROP a floor check (a safety-floor regression with no error). Replaced with a locked **registry table** `cairn_event_twin_check(event_type, check_fn, twin_required_msg)` + a fail-closed load-time validation trigger, a **single stable dispatcher** (db/005 only, dynamic `EXECUTE %I` over the table), and all per-type check fns unified to `(p_type text, b jsonb) RETURNS void`. A new event type registers ONE additive row and never touches the dispatcher; the single-source invariant is enforced by the no-DB guard `twin_dispatch_single_source.rs`. First dynamic SQL in the floor (bounded: migration-only locked table, `%I` quoting, fail-closed, load-time validated, `search_path`-pinned definers). ZERO behaviour change (15 seed rows verbatim from db/033's chain; full suite green). `event_type_class` deliberately not merged (future convergence).
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
- **Demographics assertion stream** — per-field projection policy ([§4](spec/demographics.md)). **Address model specified** ([ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md), [§4.3](spec/demographics.md)): culture-neutral three-facet value (display legibility twin + optional geolocation + culture-tagged structured parts via a content-addressed locale profile reusing ADR-0014). **Patient-identifier representation specified** ([ADR-0033](spec/decisions/0033-patient-identifier-representation.md), [§4.4](spec/demographics.md)): namespace/profile split (stable veto key + versioned validator) + a normalized form materialised so the hard veto survives a profile-less node; advisory validation; professional **licensure/registration** IDs fixed in the §7.5 actor registry (billing/relational provider numbers split out to §4.6, below). **Demographic legibility twin specified** ([ADR-0034](spec/decisions/0034-demographic-legibility-twin.md), [§4.5](spec/demographics.md)): every demographic assertion carries the §3.13 principle-11 twin, materialised profile-independently, with `display`/`value` reconciled as its value-core and a forward guarantee for future field shapes. **Provider-number relational model specified** ([ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md), [§4.6](spec/demographics.md)): abstract entity (open `kind`) + reified relationships carrying their own identifier sets + subject-kind partitioning `{patient, entity, relationship}` as structural non-conflation. **All demographics gaps now closed.** **Demographics IMPLEMENTATION underway** (first production clinical surface, on `cairn-node`). **Slice 1 — §4.4 patient identifiers** (`db/010_demographics.sql`): culture-neutral structural floor + authored §4.5 twin carried through the reused `submit_event` + set-union `patient_identifier` projection; pure `cairn-event::demographics` builders + `EventBody.plaintext_twin`. **Slice 2 — §4.2 DOB + sex-at-birth** (`db/011_demographics_fields.sql`): the *provenance-precedence* mechanic — generic `demographic.field.asserted` event + `cairn_provenance_rank` ladder (incl. new `fact-proven` top tier; unrecognized→0) + winner-by-`(rank,HLC,origin)` `patient_demographic` projection ("verified value locks"); **floor stays open / projection gated** (unknown field stored + legible but not projected — federation-forward per ADR-0012); §4.1 ladder prose extended. **Slice 3 — §4.2 names** (`patient_name` retained-set projection + `patient_name_current` display-winner VIEW): recency-first within the legal-use tier (HLC wins; provenance/origin break ties); falls back to most-recent any-`use` when no legal name exists; all names retained as evidence; deliberately diverges from DOB's provenance-lock ([ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md)). **Slice 4 — §4.2 administrative-sex + gender-identity** (`db/013_demographics_sex_gender.sql`): per-field winner policy via an IMMUTABLE `cairn_demographic_field_policy(field)` classifier; administrative-sex provenance-first (document-anchored; recency breaks equal-provenance ties); gender-identity recency-first (patient's current stated identity always wins regardless of provenance — the inverse of DOB's ordering; provenance still feeds the §5.2 matcher). Karyotype resolved ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) as a distinct field — no karyotype code yet; spec/ADR only. Additive: no new event type, no floor change, no `patient_demographic` schema change; db/013 supersedes db/011's trigger. **Slice 5 — §4.3 address** (`db/014_demographics_address.sql`): retained-set `patient_address` + per-use `patient_address_current` recency-first VIEW (one current address per `use`); additive floor branch; per-use recency-first winner — addresses are volatile, a fresh patient-stated move must displace a stale document-verified address ([ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)). **Slices 6–12 — §5.2 matcher pieces A/B1/B2/B2b/B3 harness + compound key + generator** (2026-06-28→07-01; condensed, full detail in git). Advisory Python `matcher/` (`cairn-matcher`, AGPL-3.0, zero runtime deps, pure functions — fit-for-purpose §9 tier); no ADR/spec bump throughout (implements settled §5.2/§5.13/§4.1). **Slice 6 — piece A** (`db/016_match_veto.sql`, SCHEMA 14→15): the in-DB hard-veto floor — `cairn_match_veto`/`cairn_has_hard_veto` implement the closed hard-veto set (same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash); `hard_veto`/`degrade_hold` verdicts, precision-gated DOB (no date parsing), `system:unknown` never vetoes; 12 tests; deceased-status veto deferred (stub). **Slice 7 — piece B1**: the scoring core — comparator contract (`PHONETIC`/`NICKNAME` reserved, never emitted — anti-cultural-capture) + in-house Jaro–Winkler + 4 culture-neutral comparators + positive-only `compare_identifier_sets` + Fellegi–Sunter combiner (`MatchScore`); 55 pure tests; final review fixed one Critical (score symmetry, greedy name-pairing `max(a,b / b,a)`). **Slice 8 — piece B2** (`db/017_match_proposal.sql`, SCHEMA 15→16): the veto-gated pairwise pipeline — ISO-only DOB extraction, token-bag names, `auto_candidate`/`review`/`None` banding (any veto caps at review, never auto-link/auto-reject); `db/017` an advisory worklist, not a safety gate; 92 tests with DB. **Slice 9 — piece B2b** (no `db/` file): blocking/candidate-pair generation — 3-pass disjunction (shared identifier · exact DOB · shared name token), canonical-pair dedup, oversized-block guard skips+reports (never silently caps) + `sweep()` batch driver; 113 tests with DB. **Slice 10 — B3 harness** (`cairn_matcher/eval/`, no `db/` file): scorer metrics (precision/recall/F1, zero-denominator→0.0) + DB-gated blocking-recall measurement (pair-completeness/reduction-ratio/dropped-true-matches) + culture-plural `gold_v1.json` + CLI; 146 with DB. **Slice 11 — B3 compound key** (`pipeline/db.py`): additive `name+year` `UNION ALL` pass (birth-year CTE, first-4-digit-run culture-neutral degrade) partitions oversized name-token blocks — recall non-decreasing; 151 with DB; filed [issue #84](https://github.com/cairn-ehr/cairn-ehr/issues/84) (test-leak + harness `KeyError`, the `KeyError` arm later fixed in slice 21). **Slice 12 — B3 generator** (`eval/generator.py` + `generate.py`, pure/stdlib): seed+corrupted-clone entity clusters recoverable by construction (a `_repair` step guarantees ≥1 shared blocking key), drift-canary-pinned to `_GROUPS_SQL`; 200-entity volume test: `pair_completeness == 1.0`, `reduction_ratio≈0.919`. All pieces' whole-branch reviews READY-TO-MERGE/MERGE-READY (0 Critical outstanding per slice; findings fixed in-branch or in PR #83's post-review wave).
- **Point-of-care identity, possession semantics, `sign-as` salvage** ([ADR-0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)).
- **Locale-pluggable matcher comparators** — *advisory only* (Python/ML); comparator-profile tag travels with each demographic assertion, degrades honestly to human review ([ADR-0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md)).

**Slices 13–35 — condensed (2026-07-02 → 07-16; full detail in git, the PRs and the linked ADRs).**
The identity/John-Doe/medication build-out and the review course's Priority-1 slice. What exists:

- **§5.7 identity core C1–C5** (slices 13–18, `db/018`/`019`/`023`/`024`/`025`, SCHEMA 16→18) — the closed
  identity algebra: `link` assertions and the linkage projection (C1); the `match_proposal`→apply seam with a
  human-accepted door (C2); auto-apply of the `auto_candidate` band via `matcher_actor.rs` (C2b); `dispute` +
  the chart trust-state projection (C3); `identify` + the *unconfirmed* trust state (C4); `repudiate` + the
  known-alias pool (C5, the first *suppressing* identity event). The
  confirmed/unconfirmed/under-review contract is COMPLETE.
- **§5.4 John-Doe subsystem** (slices 20, 26–30) — registration front door (A+B, no new event type); photo
  evidence carrying the day-one §3.14 attachment-reference shape ([ADR-0042](spec/decisions/0042-concrete-attachment-reference-shape.md));
  marks/belongings/EMS-context text evidence (three `kind` values on the existing evidence type); finishers
  (node-local ordinal + `--observed-year`; `identify` → optional link); the `enroll-human` ceremony CLI.
- **§5.2 matcher, advisory tier** (slices 19, 21–25; Python `matcher/`, no `db/` change) — the alias-pool
  evidence pass; birth-year-range blocking + A/B pass toggle; administrative-sex scoring and the
  unconfirmed-chart REVIEW rule; the B3 eval mirror (generator range-DOB + sex representation); supervised
  Fellegi–Sunter weight-learning (a PoC on small/synthetic data — see the gold-set item below); compound
  blocking keys (`dob+first-initial`, `name+sex`).
- **`clinical.medication` slices 1–5** (slices 30b–34, `db/031`–`db/035`) — the first clinical-content stream:
  assert/cease + the E1 reconciliation flag; the bitemporal dose overlay/timeline; cross-thread reconciliation
  as a *link* ([ADR-0047](spec/decisions/0047-medication-reconciliation-resolution.md)); the commitment-based
  attestation responsibility overlay ([ADR-0049](spec/decisions/0049-commitment-based-sign-off-currency.md), plus a
  hardening/coverage follow-up); per-field dose effective-date/reason correction
  ([ADR-0050](spec/decisions/0050-dose-correction-per-field-patch.md)). Twin-check registry:
  [ADR-0048](spec/decisions/0048-twin-check-registry-dispatch.md).
- **Slice 35 — the P1 floor-hardening slice** (2026-07-16, PR #219; no ADR/spec/SCHEMA change) — the ADR-0030
  hostile-enrolled-writer threat model re-run against the in-DB floor across eight issues
  (#187/#207/#194/#191/#192[+#177]/#190/#193/#195), closing the local-door HLC drift ceiling, the widened-column
  replay guard, `content_address` final tiebreaks, the fail-closed suppression-target gate, medication
  patient-consistency (resolving #177), the un-attested `identity.link` veto, the restore-door drift ceiling and
  the responsibility↔attester binding. Follow-up [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) remains
  (the #190 veto is link-arrival-only).

**Still open from these slices.** Condensing 13–35 must not lose the open remainder, so it is enumerated in full
here (a PR #271 review finding: the first pass dropped two *open* issues out of every tracked file).

- **Filed and open.** [#141](https://github.com/cairn-ehr/cairn-ehr/issues/141) — photo evidence has no size guard
  on the local blob-store path (§6.6 byte-tier slice). [#184](https://github.com/cairn-ehr/cairn-ehr/issues/184) —
  a non-array `contributors` yields a cryptic scalar-extract error at **both** submit doors, all event types.
  [#163](https://github.com/cairn-ehr/cairn-ehr/issues/163) (demographics currency),
  [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many),
  [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (cross-thread dose-correction suppression vector —
  needs a PK/design decision), [#79](https://github.com/cairn-ehr/cairn-ehr/issues/79) (B2 Minors),
  [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) (the #190 veto is link-arrival-only),
  [#172](https://github.com/cairn-ehr/cairn-ehr/issues/172) (the future actor-write doors —
  rotate-key/`supersede`, actor-event sync apply — must mirror BOTH enroll collision checks; ADR-0054
  makes this live work).
- **Identity C5+.** `reattribute` (§5.5 event-granular strike-through of clinical documentation) **waits on a
  clinical-note surface**; a reversal / de-repudiation event; a chart-history VIEW rendering struck names (the data
  is already present); an accept-at-cap boundary test for the oversize guard; the §5.2 coherence feedback loop;
  notification / contamination cascade on dispute; person-level trust aggregation. The §5.12 push-alert and the
  §5.3/§5.8 search-before-create funnel are the non-structural John-Doe remainder.
- **Matcher (advisory tier).** A **large hand-crafted gold set** to re-run the learner for authoritative magnitudes;
  **full §7.5 matcher actor registration** (the matcher's contributor identity lives in a provenance string for
  now); **no recovery escrow for the sealed matcher key** (regenerable today, so this is a convenience gap, not a
  data-loss one); no background scheduler (operator-invoked CLI only); locale comparator packs; the hub-tier
  aggressive duplicate sweep; a veto-aware scorer mode; fuzzy/edit-distance alias recognition and a dedicated
  `alias` blocking pass; fuzzy near-window softening; variable cluster size / hard negatives in the volume
  generator; a `compare_address` comparator; a CLI sweep entry; the B3 mirror still ignores the block-size cap.
- **Medication (slices 30b–34).** Automated reconciliation **detection** — the human-driven *resolution* exists,
  fuzzy/automatic detection plus a Tier-A dictionary is the gap; a partially-attested-group read surface (which
  member is stale); a whole-list sign-off summary event; statement-level `started`-date correction and per-field
  merge across corrections of the same point; a rendering-suppression visibility overlay for `delete`; structured
  sig/frequency; a separate `route` field; prefer-INN display term.
- **Attachments (slice 26).** Bytes are local only — **cross-node fetch deferred**; the residual DO-UPDATE
  overwrites a caller-supplied `media_type` (benign).
- **Accepted risk with a named remedy.** The `enroll_actor` dual-mapping guard's TOCTOU window
  ([#166](https://github.com/cairn-ehr/cairn-ehr/issues/166), closed as *accepted*): the durable fix is a
  floor-level per-key guard in `db/004`. Recorded here so the accepted risk keeps its remedy attached.

*Done, not open* (called out because an earlier condensation listed it as outstanding): stale forced-REVIEW
proposal **retraction** — [#135](https://github.com/cairn-ehr/cairn-ehr/issues/135), closed by PR #151.

**Slices 36–56 — condensed (2026-07-16 → 07-27; the 2026-07-15 whole-project review course, its
Priority-6 design queue, and the first medication-coding slices. Full detail in git, the PRs and the
linked ADRs; the *why* is in each ADR and must not be restated here).** The review course is **fully
closed**. What exists:

- **P2 sync-convergence integrity** (slices 36–40, PRs #221–#225) — the flagship A→B convergence test
  driving the real binaries over TCP (#199); the cairn-sync SCHEMA subset standing alone (#198); the
  clinical-plane `seq` cursor + periodic full sweep (#196, `db/036`); acked rows freed from the
  quarantine quota (#197); cairn-sync wire hygiene + the `node.superseded` apply arm (#202/#201).
  Open: [#227](https://github.com/cairn-ehr/cairn-ehr/issues/227),
  [#228](https://github.com/cairn-ehr/cairn-ehr/issues/228).
- **P3 — both wire windows shut** (slices 41–43). **[ADR-0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)**
  contributor-role vocabulary floor (#203+#96): `recorded` ratified, `{held_by, on_behalf_of?}`
  responsibility objects, partition-prefixed future members, strict-submit/lenient-apply.
  **[ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md)** born-sealed clinical bodies
  (#189+#92, `db/037`): every clinical JSONB body sealed at write under a per-event DEK the node itself
  holds — an *erasability* substrate, not confidentiality — with a custody plane, both doors enforcing
  sealed⇒clinical scope, and a rung-3 shred CLI. **[ADR-0053](spec/decisions/0053-per-write-human-authorship.md)**
  per-write human authorship (#204): `{human,authored}`+`{node,recorded}`, human signs while the node
  seals; `cairn_authorship_bound` strict-door binding; `--author-as`.
- **P4/P5 process + tech debt** (slices 44–45, PRs #251/#253/#255) — the #188 schema-version downgrade
  guard in both loaders (repo-wide `SCHEMA_GENERATION` + fs-derived guard tests + the `SCHEMA_LOAD_LOCK`
  TOCTOU close); `scripts/run-db-sql-tests.sh` running the `db/tests/*.sql` mirrors in CI (#212); the
  registry `DO UPDATE` convergence arm (#214); HANDOVER staleness (#215). #212's first property suite
  (proptest) immediately caught a real grading defect before any read path shipped.
- **P6 design queue → four ADRs** (slices 46–50). **[ADR-0054](spec/decisions/0054-actor-registry-federation-admit-and-dispute.md)**
  actor-registry federation is admit-and-dispute (#205; closes #154 structurally).
  **[ADR-0055](spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md)**
  chained trust-root document (#206). **[ADR-0056](spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)**
  unknown event types admitted uninterpreted (#200 — the filed premise was *inverted*: the spec was
  right, the code was wrong). **[ADR-0057](spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md)**
  generic reprojection (#208, PRs #274/#278): one registered `cairn_projection_apply` fn per projection
  + one dispatcher replacing ~15 per-type triggers; `cairn_reproject` is the generic heal/rebuild replay
  both loaders run on a schema-generation change; `cairn_replay_eligible` is the #265/#266 seam.
  **[ADR-0058](spec/decisions/0058-grade-gated-teffective-ceiling.md)** grade-gated `t_effective`
  ceiling (#216, PR #285): a born `clock_grade` bounds the ceiling's rejecting power — at
  `self-asserted`/`unknown` (every node today) it flags-never-rejects, and the remote door
  admits-and-flags, closing a latent one-event sync-wedge DoS.
- **Matcher, advisory tier** (slices 51/53/54; Python `matcher/` only, no spec/SCHEMA/wire/ADR change) —
  #209 `derive_thresholds` fails closed on an empty non-match set (no impostor ⇒ no safe auto anchor);
  #210 a sweep-level pass retracts proposals orphaned when a pair leaves the blocking universe; #211 the
  E3 four-gap batch (alias-map canonicalization, inverted-threshold refusal, a `lower()`-vs-`casefold()`
  doc-honesty fix, `repaired: True` marking); #290 eval consumers REPORT the repaired-pair count so a
  reader discounts the optimistic recall/F1. Open: [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287).
- **Slice 52 — the #217 paper-parity plan-section rule** — every clinical-surface slice plan now carries a
  `## Paper-parity benchmark (§1.2)` section or a forced-rationale escape, enforced by a no-DB source
  guard and stated in CONTRIBUTING.md + CLAUDE.md house rule 7. First live entry:
  [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288).
- **Slices 55–56 — medication drug coding** (ADR-0059 + its first code slice). **[ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)**
  (design-only, spec v0.61) anchors drug identity on drugref's immortal `moiety_uuid` (INN is display,
  never key), as `substance.coding {system, code, display}`, **advisory + honest-degrading**. Slice 6a
  (PRs #297/#298, `db/041`, `SCHEMA_GENERATION` 40→41) shipped the inline shape: the
  `medication_coding_system` registry + a two-tier floor (structural refuses at both doors,
  registry-derived is strict-submit/lenient-apply), `medication_coding` as its **own** projection table,
  the `(system, code)`-**pair** dup-key, a prefer-coded group display, an advisory anchor-conflict view,
  and honest degradation proven **by construction** via a source guard that no `.sql`/`.rs` under `db/`,
  `crates/` or `extensions/` references drugref executably. Three findings changed shipped behaviour:
  db/020's `cairn.remote_apply` marker moved to precede the floor dispatch; the canonical UUID spelling
  is pinned at the strict door; and `cairn_execute_shred` did not scrub `medication_coding` — a shred
  reporting success left the drug's preferred name and its immortal anchor readable beside `patient_id`
  in a `cairn_agent`-readable table (the ADR-0005 rung-3 / #92(b) failure). Open:
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294).

**Still open from slices 36–56.** Enumerated in full, because condensing must not lose the open
remainder (the PR #271 review finding).

- **Sync/convergence.** [#227](https://github.com/cairn-ehr/cairn-ehr/issues/227),
  [#228](https://github.com/cairn-ehr/cairn-ehr/issues/228),
  [#284](https://github.com/cairn-ehr/cairn-ehr/issues/284) (cairn-node's full SCHEMA list vs cairn-sync's
  subset staying consistent).
- **Born-sealed / erasure (ADR-0052 follow-ons).** [#230](https://github.com/cairn-ehr/cairn-ehr/issues/230)–[#237](https://github.com/cairn-ehr/cairn-ehr/issues/237):
  notably [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) (unwrap-cert kid pinning — until it
  lands, born-sealed is an *erasability* substrate, NOT confidentiality),
  [#232](https://github.com/cairn-ehr/cairn-ehr/issues/232) (sequester + the sensitivity stream + §5.9
  safety-projection emission), [#233](https://github.com/cairn-ehr/cairn-ehr/issues/233) (unwrap-key
  rotation ceremony), [#234](https://github.com/cairn-ehr/cairn-ehr/issues/234) (blob-byte born-sealing),
  [#235](https://github.com/cairn-ehr/cairn-ehr/issues/235) (shred authorization policy hooks),
  [#236](https://github.com/cairn-ehr/cairn-ehr/issues/236) (FTS/RAG must build on the `event_clear`
  shadow with shred-triggered invalidation), [#237](https://github.com/cairn-ehr/cairn-ehr/issues/237)
  (code hygiene).
- **Authorship (ADR-0053 follow-ons).** [#242](https://github.com/cairn-ehr/cairn-ehr/issues/242) (the
  `asserted` grade + token-backed author — verbal orders, AI-scribe, dictation),
  [#243](https://github.com/cairn-ehr/cairn-ehr/issues/243) (point-of-care durable session-decoupled
  drafts + `sign-as` salvage — the ADR-0008 UI half),
  [#244](https://github.com/cairn-ehr/cairn-ehr/issues/244) (authorship + responsibility on one clinical
  event — collapse the self-vouch case), [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) (the
  SQL mirror of `classify_authorship_confidence` + the §5.10 authorship-confidence projection),
  [#247](https://github.com/cairn-ehr/cairn-ehr/issues/247). Standing notes: grading stays **half-live
  until #245** wires a read path for `classify_authorship_confidence`; authorship in a contributor set is
  **key-scoped** and does not survive key rotation (#247, which constrains #245); a `--author-as` event is
  *owned* under the ADR-0043 suppression gate where a device-signed equivalent was dismissable by anyone.
- **ADR-0054/0055/0056 code work (all design-settled, none built).** ADR-0054: #94, the key-loss-ceremony
  ADR, the rotate-key local door. ADR-0055: [#257](https://github.com/cairn-ehr/cairn-ehr/issues/257)
  (root-chain verifier + load gate), [#258](https://github.com/cairn-ehr/cairn-ehr/issues/258)
  (transparency-log role), [#259](https://github.com/cairn-ehr/cairn-ehr/issues/259) (reproducibility CI),
  [#260](https://github.com/cairn-ehr/cairn-ehr/issues/260) (freshness rung),
  [#261](https://github.com/cairn-ehr/cairn-ehr/issues/261) (sync-auth onboarding UX design session).
  ADR-0056: [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) (door admits uninterpreted),
  [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) (re-adjudicate the deferred gates, *then*
  reproject — reprojection alone would grant power that never passed the attestation / target-exists /
  cross-author-suppression gates), [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267) (pen door
  refusals verbatim), [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (align the node-plane
  skip), [#269](https://github.com/cairn-ehr/cairn-ehr/issues/269) (node-plane heal test gap),
  [#270](https://github.com/cairn-ehr/cairn-ehr/issues/270) (a frozen watermark must fail loud).
  **The posture triad:** the content plane admits-and-disputes (0054) *and* admits-and-defers (0056),
  while the code plane verifies-or-refuses (0055).
- **Reprojection (ADR-0057 follow-ons).** [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (the
  authoritative Pi5/NVMe same-rig re-run — the shipped Bet-B numbers are cross-rig),
  [#275](https://github.com/cairn-ehr/cairn-ehr/issues/275) (per-row logic-generation watermark),
  [#276](https://github.com/cairn-ehr/cairn-ehr/issues/276) (registry governance at scale),
  [#277](https://github.com/cairn-ehr/cairn-ehr/issues/277) (the loader's heal cannot re-derive
  `ON CONFLICT DO NOTHING` projections after an extraction-logic fix).
- **Trusted time (ADR-0058 deferred).** [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279)
  (anchor/notary planes + grade-upgrade tokens), [#280](https://github.com/cairn-ehr/cairn-ehr/issues/280)
  (causal lower-bound tightening), [#281](https://github.com/cairn-ehr/cairn-ehr/issues/281) (clock-sanity
  UI alert), [#282](https://github.com/cairn-ehr/cairn-ehr/issues/282) (auto-downgrade a failed clock),
  [#283](https://github.com/cairn-ehr/cairn-ehr/issues/283) (render `clock_grade` in the twin).
- **Registry hygiene.** [#254](https://github.com/cairn-ehr/cairn-ehr/issues/254) — 8 twin-check
  registrations still use `ON CONFLICT DO NOTHING`; unify with the #214 `DO UPDATE` arm or record why not.
- **Deps.** [#252](https://github.com/cairn-ehr/cairn-ehr/issues/252) — `quick-xml` RUSTSEC-2026-0194/0195
  via `wayland-scanner` (cairn-gui), upstream-blocked.
- **Medication/matcher.** [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (hub-scale sweep
  re-scoring cost), [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) (med-list whole-list sign-off
  must collapse to ONE human gesture — owed by the future Tauri med-list slice),
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) (the §5.9 safety projection must *carry* the
  coding-derived drug class rather than re-derive it; blocked on #232).

**Operational caveats that outlive these slices.** Pre-ADR-0051 event logs (old `role:"author"`-without-
actor_id, flat-string responsibility) and pre-ADR-0052 plaintext `clinical.*` bodies **REFUSE at db/020** —
**wipe dev/PoC rigs** (the replication-failover demo, the spike rigs), never sync them through. Pre-wire
unsigned actor rows never sync. Test DBs need `cairn_pgx` ≥ 0.3.0.

**Slice 57 — `clinical.medication` slice 6b: the coding-overlay event types (2026-07-28; branch
`feat/medication-coding-overlay-slice-6b-0059`; completes
[ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) decision 3; no spec/ADR
change; `SCHEMA_GENERATION` 41→42).** Coding becomes a **separately-authored act**: `db/042` adds
`clinical.medication-coding.asserted` and `-correction.asserted`, both `('additive', FALSE)` — a
correction ADDS a claim, and `targets_other_author = TRUE` would route it through the ADR-0043 owner gate
and refuse a pharmacist correcting someone else's coding. **Why the strike exists** (the decision the
slice turns on): a reviewer who establishes a medication is NOT metformin but cannot say what it is would
otherwise have to leave a known-wrong anchor standing or invent an identity they cannot vouch for — the
fabrication principle 4 forbids. The correction event must be able to say *"not that, and I don't know."*

The slice stayed **additive** — 6a's table-not-columns payoff: both apply fns write the existing
`medication_coding` table under the existing overlay-winner rule. A strike NULLs the anchor rather than
deleting the row (deleting breaks arrival-order independence — a lower-HLC coding arriving later would
have nothing to lose the race against). `patient_medication_uncoded` is the coder worklist, with
`previously_struck` separating "nobody has coded this" from "a reviewer established this is NOT what it
was coded as". CLI: `medication-code` / `medication-code-correct`, both `--author-as` (ADR-0053) but
deliberately **no** `--attest-as` — coding a drug identity is not a sign-off of the medication list.
Also closed **#295** (anchor-conflict collation pin — the behavioural test *cannot* catch a future
unpinning on a deterministic-default cluster, so the real gate is a no-DB source guard) and **#296** (a
cairn-sync test dropped `event_log.seq`, letting the migration re-add it at the END and permanently
reorder a SHARED test database — the root cause of the long-carried "recreate the test DBs" gotcha).

**Four lessons worth keeping** (full detail in git): **(1) a redundant projection column is a convergence
hazard** — `struck` duplicated `coding_code IS NULL` and only two of three writers set it, so arrival
order decided what a node read; it is now `GENERATED ALWAYS AS … STORED`, deleting the writer rather than
correcting it. Deliberately **not** a CHECK: a violated CHECK aborts the apply and wedges that event
forever. **(2) Nullable-widening a column means re-reading every aggregate over it** — unlike
`count(DISTINCT …)`, `array_agg` KEEPS NULLs, so the anchor-conflict view emitted a blank entry.
**(3) A passing test can be worthless** — the group-display test asserted only `coding_display` and went
green against a live defect; asserting the *term* alongside made it discriminate. **(4) Only
`cargo test --workspace` catches guard-scope gaps** — 6a's drugref guard had never met a `#[cfg(test)]`
module inside a `src/` file. Workspace 916/0. Filed
[#300](https://github.com/cairn-ehr/cairn-ehr/issues/300): the worklist lists every uncoded member of an
already-coded reconciled group — a design question (hiding them could suppress the mis-reconciliation
signal 6a built), not a defect.

**Deliberately NOT done:** no drugref code anywhere in the tree; the **coded↔uncoded** duplicate case
remains open (needs term→anchor resolution — ADR-0059 decision 5 is explicit the key does not close it);
the §5.9 safety class is still owed ([#294](https://github.com/cairn-ehr/cairn-ehr/issues/294), blocked on
#232); the coding UI and its §1.2 time budget are owed by the med-list UI slice
([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) neighbourhood).

**Slice 58 — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power (2026-07-29; branch
`feat/adr-0056-admit-uninterpreted-floor-265-266`; closes
[#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) +
[#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) — decisions 1 and 4 of
[ADR-0056](spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md), ratified 2026-07-20 with no
code; six tasks, TDD throughout; `SCHEMA_GENERATION` 42→43).** `apply_remote_event` used to RAISE on an
`event_type` absent from `event_type_class`, so the event was **never stored at all**. A phone-tier node
carrying a chart between two upgraded facilities — the §6.1 sneakernet path, the case Cairn exists for —
acquired nothing past the first unknown-type event: not unrendered, *absent*. `sync.md` §6.5's
lossless-forwarding invariant was therefore false for unknown *types*, and the spec was right while the code
was wrong. The door now admits verbatim, projects nothing, confers nothing, and records an explicit marker.

`event_deferred` (in **db/001**, next to `event_log`) is that marker — node-local, never on the wire. It
lives there rather than in this slice's own db/043 because db/005's `cairn_replay_eligible` and
`cairn_suppression_author_ok` read it and both are `LANGUAGE sql`, whose bodies resolve table names at
**CREATE** time, unlike PL/pgSQL's late binding. Its presence IS the invariant ("powerless; the
classification-gated checks have not been passed"); promotion **deletes** the row rather than marking it
resolved, so there is one source of truth. `adjudication_error` is decision 4's *flagged legibly*, surfaced
by a new `cairn-node deferred` listing.

**Why re-adjudication is load-bearing rather than bookkeeping.** Admitting uninterpreted necessarily SKIPS
every floor check derived from the type's mode or its target relationship — in db/020 the
suppressing⇒attestation gate, the overlay-target-exists refusal and the ADR-0043 cross-author refusal all sit
downstream of the classification lookup. Those are *deferred with* the interpretation, not waived by it, so
`cairn_readjudicate_deferred` (db/043) re-runs all three **before** anything reprojects; that ordering is what
makes "no unattested suppression" hold at every instant rather than being violated-then-repaired. The
envelope is re-derived with `cairn_body(signed_bytes)`, never reconstructed from projection columns, so the
predicates see exactly what the door saw. Failures are captured **per row, never raised** — the pass runs
inside `connect_and_load_schema`, and a raise would wedge the node on one bad event, the very failure mode
the ADR removes. Candidates are ordered by HLC so a deferred overlay is adjudicated after the deferred target
it points at. `cairn_replay_eligible` — the constantly-TRUE stub ADR-0057 built *for* this slice — becomes
"carries no marker", so no reprojection path can grant unadjudicated power.

**Two decisions the ADR did not force.** (1) The pass runs on **every connect**, not only on a
schema-generation change. Classification arrives only with a code-plane update, but re-adjudication can FAIL
for a reason that resolves without one — `overlay targets unknown event`, where the target is still in flight
from another peer. Generation-gated, that event would stay powerless until the next code update, potentially
months; `event_deferred` is empty on a healthy node, so the pass costs one indexed probe. A generation change
still reprojects everything; otherwise only the promoted types are healed, in **heal** mode (a narrow rebuild
would hit db/039's shared-table refusal, which since slice 6b is the normal case). (2) The projection
registry gained a **classified-before-projected** guard: the marker is written *after* the `event_log`
INSERT while the AFTER-INSERT dispatcher fires *during* it, so an unclassified-but-registered type would be
projected at admission. Honest residual, found by a failing test and recorded at the guard site: the check
runs at *registration* time, so unreachability rests on two premises — the guard, plus the fact that
classification and registration arrive in the same migration and no migration ever DELETEs a class row.

**The security finding, and the trap under it.** A suppressing event's attestation token travels on the sync
wire and was stored only where the gate passed — so skipping the gate naively **drops** it, and
re-adjudication would then have nothing to verify, silently turning admit-and-defer into a slower
fail-closed. The deferred arm therefore stores it unverified: *carried, not vouched*. Auditing every reader
of `event_log.attester_key` against that state found one that breaks.
`cairn_suppression_author_ok` reads the **target's** `attester_key` into the ADR-0043 owner-gate's
human-author set, and unlike the two projection apply fns that read the same column (db/018, db/034 — kept
unreachable by the registration guard and `cairn_replay_eligible`) it **is** reachable for a deferred row. A
hostile peer attaching a forged token to an unknown-type event would put any key it liked inside that
event's permitted-suppressor set — over-permission on a floor whose own header says *"wrong direction is
over-refusal, never over-permission."* The gate now ignores a deferred target's token entirely. The fix is
deliberately **neutral, not merely stricter**: for an agent-signed deferred target it empties the author set
and the gate OPENS (the agent-advisory-is-dismissable rule), because an unverified token must not move the
gate in *either* direction. Pinned by the slice's security test, which asserts the hazard's precondition
before asserting the fix.

**Deliberately NOT done, stated honestly.** The **node/actor plane still fail-closes** on an unmappable type
(`db/007`) — filed as [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301) rather than left silent: the
carrier-forwarding argument transfers, but `node_event` is type-shaped (four hardcoded ops, bespoke INSERTs,
per-type trust logic), so it needs a carried-not-interpreted row shape and a reader audit of its own. So
§6.5's invariant is now true as written **for clinical events only**, and that asymmetry is a known gap, not
a design. ADR-0056 **decision 5** — the residual refusal contract — is untouched and is the next slice:
[#267](https://github.com/cairn-ehr/cairn-ehr/issues/267) (door refusals on verifiable bytes pen nothing),
[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (node-plane skip-and-advance vs clinical freeze),
[#269](https://github.com/cairn-ehr/cairn-ehr/issues/269) (no test of a skipped event healing via full
sweep), [#270](https://github.com/cairn-ehr/cairn-ehr/issues/270) (a frozen clinical watermark exits
**success**). This slice shrinks their blast radius — with unknown types no longer refusing, those paths are
now exercised only by genuine refusals. No paper-parity time budget: the slice takes the §1.2
forced-rationale escape (no human act changes at any layer; it changes only what a node retains and when
power is granted).

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

- **Content-addressed lazy blobs** referenced by the signed event, never inlined; day-one attachment-reference shape ([ADR-0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)). **The concrete shape is FINALIZED** ([ADR-0042](spec/decisions/0042-concrete-attachment-reference-shape.md), 2026-07-08, slice 26): `Attachment{descriptor, renditions:[Rendition{…, inline?, seal?}]}` + `SealRef` in `cairn-event/src/attachment.rs` (all five §3.14 reserves; field order frozen), `EventBody.attachments: Vec<Attachment>`, and reference-eager per-rendition learning in both doors via the shared `cairn_learn_attachment_refs` helper (db/027; db/005 + db/020). Byte tier (db/003 + `cairn-sync` blobd) is chunked/resumable/windowed. First real consumer: §5.4 photo evidence (slice 26). *Deferred: cross-node byte fetch wired into `cairn-node`; per-blob DEK sealing; preview/extracted-text renditions.*
- **Blob self-verification in-DB floor** — ✓ done 2026-07-05 (`db/026_blob_verify_floor.sql` + `cairn_pgx` 0.3.0
  `cairn_blob_verify`/`cairn_blob_verify_error`, thin wrappers over the same `cairn_event::blob_address` L2 uses —
  one hashing implementation, never two): the BLAKE3-vs-address check `cairn-sync` performs before flipping
  `present := TRUE` is restated **in-DB** as a trigger floor on `blob_store` (INSERT arriving present; column-level
  UPDATE OF content/address/present that flips into present, swaps content under a present row, or re-keys it —
  metadata-only updates neither re-pay the hash nor detoast the content for the WHEN comparison), closing the honest
  gap db/003 recorded since the walking skeleton: a raw-SQL client could store arbitrary
  bytes as any named blob (the exact "wrong-hash blob served as the named one" failure ADR-0013 point 11 designates
  as this tier's safety-critical seam; principle 12 requires the floor below every client). Stale-`.so` legibility is
  two-layered: db/026 itself refuses to load when `cairn_blob_verify` is absent (a `to_regprocedure` gate binding
  every loader, cairn-node included — the guard is late-bound PL/pgSQL, so without this the load would succeed and
  the illegible `undefined function` would surface only at the first present-flip), and `cairn-sync`'s
  `REQUIRED_PGX_FLOOR` 0.2.0 → 0.3.0 connect gate (now also on `put-blob`/`gen-blob`/`blobd`, the commands whose
  writes fire the trigger) catches `.so` skew after init. TDD: 7 DB-gated hostile-client tests
  (`crates/cairn-node/tests/blob_floor.rs`) + a `cairn_pgx`
  pg_test (fail-closed on tampered bytes / truncated / wrong-prefix / empty addresses). **Honest limits (recorded
  in the design doc):** `blob_chunk` rows and `outboard` are NOT in-DB verified — wrong chunks can only assemble
  into a whole-blob flip that FAILS the floor (space waste, never wrong bytes served), and a wrong outboard yields
  slices the *fetching* peer's bao decode rejects against the signed address root (availability degradation, never
  an integrity hole). No event-format change, no ADR/spec change (implements settled ADR-0013).
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
