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
  [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) (the #190 veto is link-arrival-only).
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

**Slice 36 — sync-convergence CI (2026-07-16; the review course, Priority 2 opener; issue #199 [B4], covers the
#176 deferred branch; branch `feat/sync-convergence-ci-199`, PR #221; no ADR/spec/SCHEMA/event-type change —
tests + CI wiring only).** The flagship set-union guarantee was verified only by hand: CI never set `CAIRN_TEST_PG2`/`PG3`,
so `federation.rs`/`sync_watermark.rs` self-skipped on every run, and no `clinical.medication.*` event had ever
been driven through the db/020 apply door. Now: (a) `rust.yml` provisions `cairn_test2`/`cairn_test3` on the same
CI cluster and exports all three conn strings — no Rust test self-skips in CI anymore; (b) new
`medication_remote_apply.rs` (10 tests): every medication verb —
assert/cessation/dose-change/dose-correction/reconciliation/separation — through `apply_remote_event`
(projection + set-union idempotence + arrival-order independence, incl. correction-before-target and a
later-HLC separation arriving before the reconcile it repairs), the slice-4 **attestation-token round trip**
(valid token applies + projects the VERIFIED attester; missing token, non-human attester, and a #195
unbound-responsibility claim each refused legibly), and the **#176 oversize-group remote clamp-and-flag branch**
(admitted, recompute skipped, `medication_projection_flag` row — never a veto); (c) new cairn-sync
`clinical_pull.rs`: the **A→B clinical-plane pull through the real binary** (`serve` on A, `pull` on B, real TCP)
authoring via the production orchestrators, asserting **byte-identical medication read-state** on both nodes and
that the human vouch travelled the wire and re-verified (stale=false on B), plus a standing
refuse-and-recover test (unenrolled author: pull completes, nothing applies or pens, watermark freezes at the
A1 contiguous-applied prefix; converges on the first pull after enrollment). PR #221 review findings fixed
in-branch. Workspace 655/0 failed.

**Slice 37 — the cairn-sync SCHEMA subset stands alone (2026-07-16; the review course, Priority 2; issue #198 [B3];
branch `fix/sync-schema-subset-198`; no ADR/spec/SCHEMA/event-type change — a loader-list fix + its drift guard).**
cairn-sync's embedded migration subset omitted `db/027` (+`db/029`), but both write doors PERFORM
`cairn_learn_attachment_refs` unconditionally and the db/002 `patient_chart` trigger calls the #157 collision
predicate/recorder on every `patient.created` — PL/pgSQL late binding loads the subset cleanly and fails only at
the **first write** (a total write outage on a fresh `cairn-sync init` DB; invisible in CI because every suite ran
against a database cairn-node's full 35-file loader had already visited). Fix: 027+029 added to `SCHEMA`, plus the
standing drift guard the review demanded — `schema_subset_tests` (in `main.rs`, where the private `SCHEMA` const
lives) wipes `cairn_test2`, loads ONLY the subset, honesty-checks that no full-schema residue survived, and drives
**both doors**: `submit_event` with a by-reference attachment (the lazy blob reference must land in `blob_store`),
`apply_remote_event` overlaying the same patient (executes the db/029 predicate with a standing winner), and a
genuine Byzantine HLC-triple pair (same triple, different bodies) whose advisory `hlc_collision_log` row must land.
A future door→function edge into an unlisted migration now fails this test with the exact production error instead
of shipping a first-write outage. PR #222 review findings fixed in-branch: the db/006 recall-ceremony doors
(`recall_event`, `events_by_actor_epoch`) are driven too — with them every caller-facing entry point the subset
ships is executed (db/021 ships only a table, no function) — and the honesty guard grew to three canaries across
three non-subset migrations so one renamed canary can't leave it vacuously green. Workspace 656/0 failed.

**Slice 38 — the clinical-plane seq cursor + periodic full sweep (2026-07-16; the review course, Priority 2; issue
#196 [B1]; branch `fix/clinical-sync-seq-cursor-196`; `db/036` additive columns only — no ADR/spec/event-type
change).** `cairn-sync do_pull` cursored on the HLC watermark and never swept, so an event landing in a peer's
`event_log` with an HLC BELOW an already-advanced watermark — a multi-hop arrival from a third node, or an L2 agent
self-stamping an older `hlc_wall` — was never re-fetched: a silent set-union / convergence violation (the flagship
guarantee). Ports the #38 node-plane treatment. `db/036` (idempotent additive ALTERs, no CREATE-TABLE widening → no
migration-replay-widening guard entry; registered in BOTH the cairn-sync and cairn-node SCHEMA lists):
`event_log.seq` (BIGINT IDENTITY, node-LOCAL insertion order — a newly-learned low-HLC event still gets a fresh high
seq, so it always sorts above the cursor and can't be skipped), `sync_state.last_seq` (per-peer cursor, advance-only
GREATEST), `sync_state.quarantine_floor_seq` (the seq re-offer floor — a SEPARATE persisted column, NOT derived from
pen rows, so it self-clears on a clean cycle while the pen row survives as an audit trace; a derive-from-rows floor
would re-ship from the low seq forever after a transient corruption heals — a discovered regression the user chose
to avoid), and `sync_quarantine.refused_seq` (forensics). The vestigial HLC watermark/floor columns are kept,
deprecated-in-place (a DROP is the non-additive move ADR-0012 forbids — an older binary still reads them). Wire
(additive, principle 12): a new `EventsAfterSeq { after_seq }` request + a parallel `seqs[]` on `EventsResponse`;
serve `WHERE seq > $1 ORDER BY seq`; the legacy `EventsAfter` stays served. `cmd_run` does a full sweep
(after_seq=0) every `FULL_SWEEP_EVERY` (10) cycles as the correctness floor for the residual BIGSERIAL
out-of-order-commit gap; `cmd_pull` gains `--full`. Penned events advance the cursor (handled) while the floor
re-offers them. TDD: every in-file quarantine test migrated HLC→seq (value-only, behaviour unchanged) + a direct
seq-bookkeeping test; three real-binary A→B acceptance tests (`clinical_pull.rs`) — the headline
low-HLC-below-cursor convergence (fails on the old HLC-fetch code), a `--full`-sweep-reconciles-a-forced-skip, and
re-pull-from-zero idempotence (ADR-0004); `reset()` gained `RESTART IDENTITY` for deterministic seqs. PR #223
review fixes (same branch, TDD): `seqs[]` validated before any use (strictly ascending + positive — untrusted wire
values must not poison the persistent cursor/floor; `saturating_sub` on the floor fetch), a no-response transport
failure on `EventsAfterSeq` now names the likely pre-#196 peer + the remedy (was a bare EOF), the stale
derive-from-`min(refused_seq)` comment corrected + a superseded-mid-build addendum on the design doc (the floor is
the separate self-clearing `quarantine_floor_seq` column, NOT derived from pen rows), and #101 pointers restored at
`FULL_SWEEP_EVERY`/the serve arm (#101 updated: the sweep re-ships the whole log in one frame, so its wedge fires
periodically by design once history outgrows the read window — pagination's priority raised). Workspace
665/0 failed.

**Slice 39 — acked rows freed from the clinical quarantine quota (2026-07-16; the review course, Priority 2;
issue #197 [B2]; branch `fix/quarantine-quota-acked-197`; no ADR/spec/SCHEMA/event-type change).**
`quarantine_event`'s per-peer quota subqueries counted ALL pen rows, acked included, so the quota error's own
documented remedy ("fix or ack the held rows") could never unfreeze the cursor — after acking a flood, every new
refused frame still hit `Err(quota)`; the only real way out was an undocumented manual `DELETE`. Fix mirrors the
node plane (`cairn-node/src/sync.rs` — its comment records exactly this lesson): `AND NOT acked` on both the
row-count and byte-sum subqueries; an acked row is a resolved human decision, retained as the record of it, never
a consumer of the budget. Quota error text made honest ("quota of unacked rows … acked rows stop counting"). TDD:
two RED-first DB-gated tests (row + byte halves: pen filled to quota with ACKED rows → a fresh corrupt frame must
be penned as a normal loud unacked refusal, never a pen-quota freeze). Workspace 667/0 failed.

**Slice 40 — the P2 closers: cairn-sync wire hygiene + the node.superseded apply arm (2026-07-16; the review
course, Priority 2; issues #202 [B7] + #201 [B6]; branch `fix/sync-hygiene-202-node-supersede-201`; no
ADR/spec/SCHEMA/event-type change — an in-place apply-door arm + three cairn-sync hardenings).**
**#202:** (1) `read_frame` refuses a length prefix over the new `MAX_FRAME_BYTES` (64 MiB) BEFORE allocating —
a hostile/corrupt u32 prefix could previously demand a 4 GiB allocation on both the puller (peer response) and
the server (any client reaching the port; WireGuard is the perimeter, not authentication). The cap is
batch-scale, NOT the node plane's per-event 8 MiB, because the events response is deliberately unpaginated
(#101): a log outgrowing it fails the sweep loudly with the cap named in the error; pagination (#101) stays the
real fix. (2) `do_fingerprint`'s TEXT sort keys pinned with `COLLATE "C"` (the ADR-0045/#69 discipline) — BOTH
of them: the review's `node_origin` (event_hash) and the same-failure-mode `patient_id::text`
(projection_hash); two honest nodes with different cluster collations no longer raise a false divergence alarm
from the very tool meant to prove convergence; the SQL is extracted to consts under a standing drift guard (the
#159 pattern) and validated against PG18. (3) The byte-tier thread's silent `Err(_) => 0` arm now logs a
unit-tested line — a permanently failing blobd pass (bad conn string after a DB restart, schema skew) was
indistinguishable from "no blobs to fetch" for the life of the process. **#201:** `apply_remote_node_event`'s
op map omitted `node.superseded` while the submit (db/007) and restore (db/009) doors both emit/apply it — a
peer pulling a restored node's history refused the lineage event on every full sweep FOREVER (busy-loop noise +
a permanent set-union exclusion on the node plane). Resolved as **REPLICATE**, not lineage-stays-local:
admission is trust-bounded exactly like peer/revoke (the author must resolve to an active peer), and the claim
feeds ONLY the advisory `node_lineage` view — `node_current` resolves keys from `enroll` rows alone and
`trust_peer` reads only `peer`/`revoke`, so a false supersede from a hostile-but-trusted peer hijacks neither
key resolution nor peer trust (principle 2: an attributable, signed claim). A stays-local comment could not
have fixed the wedge anyway (the serve stream ships the whole `node_event` set), and ADR-0026's durability
model ("a backup is just another replication peer") wants peers holding the COMPLETE set. The new arm mirrors
the submit door: legible missing-field guard, ON CONFLICT idempotence, the A3 HLC merge; a cross-reference now
sits at the submit site. TDD RED-first throughout (the frame test failed UnexpectedEof-not-InvalidData on the
doomed-allocation path; the admission test failed with the production "unknown node event_type node.superseded"
verbatim); the admission test covers admit + lineage row + set-union idempotent re-apply + deny-all stranger +
legible malformed refusal. A **PR #225 review round** landed on the same branch (TDD RED-first, 3 new tests):
`write_frame` gained the mirror-image SOURCE-side cap — an over-cap events response previously serialized and
shipped in full only to die at the peer's read cap, with nothing in the serving node's own log to say why its
peer stopped converging (the refusal now surfaces there via the serve loop's connection-error line, and the
>4 GiB u32-prefix truncation becomes unreachable); the projection fingerprint gained `'|'` field separators —
the RED test proved (name `X`, dob `1980`) vs (name `X1`, dob `980`) hashed EQUAL, a false CONVERGENCE (missed
divergence), the exact inverse of the collation false alarm; and both fingerprint consts are now EXECUTED
against the real schema in CI (the drift guard only string-matched them, so a quoting slip would have shipped).
Follow-ups filed: [#227](https://github.com/cairn-ehr/cairn-ehr/issues/227) (extract db/007's thrice-copied A3
HLC-merge block into one guarded helper; the helper must not become a grantable clock-ratchet door) +
[#228](https://github.com/cairn-ehr/cairn-ehr/issues/228) (non-NULL malformed hex in node-event payloads fails
with an illegible generic decode error across all three doors). Workspace 677/0 failed. **P2 (sync-convergence
integrity) is COMPLETE — the review course continues at Priority 3 (#203/#96 + #189/#92 + #204, the two closing
wire windows).**

**Slice 41 — the contributor-role vocabulary floor + responsibility wire shape (2026-07-16; the review course,
Priority 3 opener; issues #203 [C2] + #96 [B5]; branch `feat/adr-0051-role-vocabulary-203-96`;
[ADR-0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md), spec §3.9,
v0.52; no new event type, no SCHEMA change — a db/005-owned vocabulary table + one shared check fn on both
clinical doors).** Ratified **`recorded` as the 12th role member (contributory** — the recording device/system:
capture fidelity, no content, no clinical responsibility; 6 bearing + 6 contributory), retroactively legalising
what every orchestrator already minted. Retired the flat-string `responsibility: "attested"` for the spec-§3.9
**object `{held_by, on_behalf_of?}`** — the proxy case is now wire-expressible (the can't-retrofit piece);
`held_by = actor_id = verified attester` extends the #195 binding chain; `on_behalf_of` is **refused at the
submit door** until a proxy-grant ADR defines verification, **admitted at the apply door** as a signed
display-gated claim (§3.9 promises the proxy transition without schema migration — an apply-door refusal would
be the #201 wedge again). **#96's unknown-member encoding:** future members travel **partition-prefixed**
(`bearing:x`/`contrib:x`, a permanent part of the signed value) so an old node classifies them without an
upgrade; a role neither ratified nor prefixed reads as first-class **vouching-unknown**, never collapsed to
un-vouched. The floor: `cairn_check_contributors` (db/005, shared by both doors, principle 12) — **strict at
submit** (a door only authors its ratified vocabulary; non-empty contributor set; actor_id+role mandatory),
**lenient at apply** (role membership NEVER rejects — set-union losslessness; refusals reserved for
never-lawful shapes: responsibility on a non-bearing role, non-object responsibility, held_by naming another
actor). `contributor_role(role, bears)` is the floor-queryable vocabulary; `cairn-event::contributor` is the
Rust mirror (`classify_role` for future §5.10 consumers) under a standing drift-guard test. The strict door
immediately caught three out-of-vocabulary PRODUCTION mint sites beyond the review's list: cairn-sync's
authoring path minted `role:"author"` with **no actor_id at all** (its events cross db/020 on every pulling
peer — would now be refused), and `identity.rs`/`medium.rs` minted `"device"` (an actor KIND, not a role); all
now mint ratified vocabulary. TDD RED-first (10 RED refusal/drift tests + 5 lossless-admission pins that must
stay green forever); workspace **696/0** + fmt + clippy clean; docs build green. **This slice also SCHEDULES
#204 [C3]:** the attribution-token / authoring-human slice (per-write attribution, `session.user ≠
event.author`, `sign-as`; §3.10/ADR-0008) is committed as the NEXT clinical-plane slice before any new
clinical stream — `recorded` makes the device-only interim honest, #204 ends it. A **PR #229 review round**
then landed on the same branch (4 new tests, 700/0): the `contributor_role` table gained the explicit house
REVOKE — the vocabulary table IS floor, so a stray write moves the floor itself (an inserted 'bearing' row
mints arbitrary responsibility-bearing roles through the strict door; flipping a member's `bears` breaks
partition coherence) — with a `floor_enforced.rs` pin proving INSERT/UPDATE/DELETE all deny 42501 for the
unprivileged runtime role; `cairn_check_contributors` pins `SET search_path = public` on itself (the
cairn_event_twin defense-in-depth discipline, not only on the SECURITY DEFINER doors); the SQL↔Rust drift
guard sorts `COLLATE "C"` (the ADR-0045/#69 discipline — `co-signed`'s hyphen made the comparison
collation-dependent under ICU); and 3 more never-lawful apply-door refusal pins (missing actor_id,
flat-string responsibility, responsibility on an unprefixed unknown role). **Operational caveat, pinned by
design:** event logs minted by pre-ADR-0051 binaries (cairn-sync's `role:"author"` with no actor_id,
flat-string responsibility) now refuse at db/020 on every full sweep — dev/PoC rigs holding them
(replication-failover demo, spike rigs) must be wiped, not synced through.

**Slice 42 — born-sealed clinical bodies (2026-07-17; the review course, Priority 3 item #189 [C1] +
#92; branch `feat/adr-0052-born-sealed-189-92`;
[ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md), spec §3.5/§3.8/§5.9, v0.53).** The
posture decision: **every clinical JSONB body is born sealed** under a per-event DEK wrapped for the
node's *own* key — an **erasability substrate, NOT confidentiality** (the node reads its own data
freely, projections and FTS behave exactly as before, nothing is hidden from anyone). Its sole effect
is that **every ADR-0005 erasure rung stays reachable for every clinical event, forever** — a plaintext
default silently forecloses rungs 2–4 for the whole record and *that* is the system taking a policy
stance against erasability (principle 9). The word "sealed" is split into two properties: **erasable**
(shipped default, node keeps custody) vs **sequestered** (custody narrowed, the ADR-0006 confidentiality
half — deferred, posture ratified). **Crypto core** (`cairn-event::seal`): per-event DEK
XChaCha20-Poly1305, **seal-then-sign** (the signature covers the *ciphertext*, so it verifies on every
node and after shred; AAD binds `event_id` so a container can't be lifted between events); the
**legibility twin travels inside the sealed region under the same DEK** (the sealed row's outer
`plaintext_twin` is a signed mechanical *stub* naming only type + seal state — there is no plaintext
twin column to leak from); an **X25519/HKDF wrap plane** derived from the node's Ed25519 signing seed
(`info = "cairn-node-unwrap-x25519-v1"` — the DB holds only the *public* half, so an ordinary DB backup
can never reconstruct a DEK, and ADR-0026's op-pass + recovery-code escrow already covers it, KEK escrow
now **mandatory**); the public half published as a signed **unwrap-key certificate** under its own
ADR-0040 context `CTX_UNWRAP_KEY`. **`db/037` custody plane**: `event_dek(event_id, holder, dek_wrapped)`
(the mutable keystore beside the log — never inside the signed bytes; the reserved `db/001`
`event_log.dek_wrapped` column retired-unused, superseded), `event_clear` (the mutable derived-plaintext
operational twin — the FTS/RAG substrate, never a column on the append-only row) + `cairn_clear_payload`,
**both homed in `db/005`** for `LANGUAGE sql` eager-bind ordering (they must exist before the projection
fns that call them), and `erasure_shred_log`; `erasure.shred.asserted` twin-registered (twin registry
**18→19**), plaintext-by-necessity so the tombstone outlives all keys. **Two doors** (the ADR-0051
strict-submit / lenient-apply asymmetry): the **strict submit door refuses an UNSEALED `clinical.*`
body** (this is what makes born-sealed unbypassable, not a convention) — caller passes signed sealed
bytes **plus the DEK**, the door verifies signature-over-ciphertext, **decrypts in-DB** (via `cairn_pgx`
`cairn_unseal_body`), runs the *full* existing ADR-0048 twin/floor checks on the plaintext, builds
projections + the `event_clear` row, wraps the DEK for the node into `event_dek`; a sealed event
*without* its DEK is refused. The **lenient apply door** admits a foreign plaintext body (set-union
losslessness) and admits a **sealed event arriving without custody on structural checks only** (*can't
read → never reject*). The final-review round closed the one gating cross-cutting hole — **sealed⇒clinical
scope enforced at BOTH doors** (a sealed *non*-clinical body is refused; the sealed flag can't smuggle
a body past its type's floor), non-clinical projection triggers made seal-robust, and subset-node shred
wedged. **Seal-at-write for all 7 medication verbs** (assert / cease / dose-change / dose-correction /
attestation / reconciliation-link / separation) routed through the one `medication::sealed_submit`
path — the pure verb builders are unchanged, clinical semantics identical, now sealed. **Custody sidecar
on the clinical wire** (additive ADR-0012 field): `cairn-sync` serve/pull unwrap-then-rewrap the DEK
per-peer (custody follows admission trust), **shred-aware DEK exclusion** — custody is *never* granted to
an already-shredded event (arrival-order independent: a DEK sidecar landing after the shred tombstone is
refused, the shred wins regardless of message order). **Shred CLI** (`cairn-node shred`): the rung-3
audited crypto-shred ceremony — destroy the `event_dek` rows, scrub all derived plaintext (`event_clear`
+ projections + the mandatory future FTS invalidation), append the signed plaintext
`erasure.shred.asserted` tombstone (*existed → destroyed, basis Z*); **the log row is never touched** (its
signature still verifies, a resurrected opaque row is keyless noise). **E2E proof** (`cairn-sync`,
real binaries): sealed sync **with custody** converges A→B readable, **shred propagates**, and a
**cold-peer restore replays the shred log before projecting so it resurrects nothing** — a restored
backup can no more resurrect an erased body than a sibling can. **Bench** (`cairn-sync bench-seal`,
~1.5 KB body, N=10 000, release): whole seal→wrap→unwrap→unseal ≈ **0.11 ms/event**, ~**37× under** the
Bet-B ~4 ms budget (the X25519 wrap dominates; per-event DEKs comfortably affordable on a dev-class node,
the Pi-class re-run + per-episode-hierarchy question deferred). **ADR-0049 false-fresh gate** (§9): a
sealed thread's commitment is now a function of local *custody*, not the pure append-only content-event
set, so a partial-custody node could read false-fresh; `reviewed_count` is promoted (for sealed threads
only) to a **safe-direction withholding tripwire** — `readable_count < reviewed_count` forces
stale/unknown, never asserting fresh it would otherwise have missed (inert on unsealed/full-custody
threads). **Operational caveat, by design:** the born-sealed floor **refuses plaintext `clinical.*` at
submit**, so pre-ADR-0052 plaintext clinical dev/PoC rigs must be **WIPED** — old logs won't cross
(honest degradation per ADR-0012, moot pre-production). TDD RED-first throughout; workspace **761/0** +
fmt + clippy clean; docs build green; **final whole-branch review = READY TO MERGE**. Nine follow-ups
filed (all deferred/hardening, none gating): [#230](https://github.com/cairn-ehr/cairn-ehr/issues/230)
(sealed-no-custody residual false-fresh, ADR-0052 §9), [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231)
(pin the unwrap-cert kid to the trust set — transport is currently the sole custody-read gate),
[#232](https://github.com/cairn-ehr/cairn-ehr/issues/232) (sequester + sensitivity-stream +
safety-projection — the ADR-0006 confidentiality half),
[#233](https://github.com/cairn-ehr/cairn-ehr/issues/233) (unwrap-key rotation ceremony),
[#234](https://github.com/cairn-ehr/cairn-ehr/issues/234) (blob-byte born-sealing),
[#235](https://github.com/cairn-ehr/cairn-ehr/issues/235) (shred authorization policy hooks),
[#236](https://github.com/cairn-ehr/cairn-ehr/issues/236) (FTS/RAG on the `event_clear` shadow only +
shred invalidation), [#237](https://github.com/cairn-ehr/cairn-ehr/issues/237) (code-hygiene bundle),
[#238](https://github.com/cairn-ehr/cairn-ehr/issues/238) (clinical_pull test readiness-timeout flake).

**Post-review `/fixall` pass (2026-07-18).** A whole-diff `/review` of PR #239 caught **five issues the "READY TO MERGE / none gating / fmt clean" verdict had missed**, all fixed on-branch (RED-first): (1) *[gating]* a wrongly-sealed `identity.link` **wedged clinical sync** — `patient_link_apply()` cast `subject_a`/`subject_b` to `uuid` in its DECLARE block, before the seal guard could return, so a non-UUID top-level field raised at apply and froze the watermark (casts moved below the guard, db/018); (2) *[gating]* `cairn_execute_shred` scrubbed only 3 of 7 medication projections, leaving **dose-correction / reconciliation / attestation plaintext readable after a shred** (rung-3 defeated) — now scrubs all by `content_address` + recomputes the derived `medication_group_member` (db/037); (3) *[CI-red]* rustfmt failed on the workspace-**excluded** `cairn_pgx` extension; (4) crypto-shred of a **non-sealed target** was a false erasure — now refused at the CLI + the db/005 floor; (5) silent serve-side DEK re-wrap failure — now logged. Verified cairn-node 298/0, cairn-sync 51/0, cairn-event 140/0 (+3 regression tests), fmt+clippy clean on both trees. The subset/E2E tests require **cairn_pgx ≥ 0.3.0 on both test databases** (a stale 0.1.0 on the 2nd DB trips db/026's version gate). #231 (unwrap-cert kid pinning) reaffirmed as the load-bearing gap: born-sealed ships **erasability, not confidentiality**, until it lands.

**Slice 43 — the authoring-human attribution slice (2026-07-18; the review course, Priority 3 CLOSER; issue
#204 [C3]; branch `feat/adr-0053-authoring-human-204`;
[ADR-0053](spec/decisions/0053-per-write-human-authorship.md), spec §3.9/§3.10, v0.54; no new event type, no
SCHEMA change).** Closes the mirror-image gap the review found — the build had *responsibility* (ADR-0049
attestation) but no *authorship*, so by §3.9's own rule every un-attested medication row read as
machine-generated content. A clinical event can now carry an **authenticated human author**: contributors
`[{human,"authored"},{node,"recorded"}]`, **signed by the human**, while the node seals the body and holds the
DEK — realizing ADR-0008's `session.user ≠ event.author` at the data/floor/CLI layer (the durable-draft +
`sign-as` UX defers to the Tauri surface, #243). `authored` is carried **without** a `responsibility` object —
the legitimate "authored, not-yet-vouched" state (§3.9). **Floor (db/005):** `cairn_authorship_bound` — a
responsibility-*bearing* contributor's `actor_id` must be the event's **signer or the verified attester** (the
#195 responsibility↔attester binding extended to authorship; forged authorship refused at the strict submit
door, step 4b); contributory roles (`recorded`) exempt so the device default is untouched; structural /
fails-closed like its sibling `cairn_responsibility_bound`. **Strict-enforce / apply-grade asymmetry (the
load-bearing decision, design §4):** db/020 is **unchanged** — the sync door never refuses an unverifiable
authorship claim, because at apply a forgery is indistinguishable from an author authenticated by a scheme an
older node cannot parse (ADR-0012), and refusing would make a lawful future medication *invisible* — inferior
to paper. It **admits + grades** instead: `cairn-event::classify_authorship_confidence` → `attested` (human
signed / verified attester) · `unverified` (unverifiable claim, upgradable) · `device` (recorded-only).
**Node:** `AuthorParams` + an `author` param threaded through `seal_sign_submit` and all six medication
orchestrators (the human's key signs; `ensure_unwrap_key` keeps the NODE as custodian — born-sealed
erasability preserved under human signature); pure `with_human_author` does the body rewrite. **CLI:**
`--author-as` / `--author-passphrase` on the six verbs (composes with `--attest-as`). **Tests:** forged
authorship refused through the real door (raises at step 4b before custody), device path unchanged, the
suppression owner-gate now recognises the human author. TDD RED-first; workspace **775/0** (all 3 DBs) + fmt
(both trees) + clippy + cargo-deny + mkdocs clean; **final whole-branch review (opus) = READY TO MERGE** (0
gating; custody-stays-node, floor soundness, eager-bind ordering, and the strict/apply asymmetry all
independently verified). A cross-crate build gap the per-crate task runs missed — `cairn-sync/tests/clinical_pull.rs`
calling the medication orchestrators — was caught by the coordinator's full-workspace build and fixed
(author=None). Four follow-ups filed: [#242](https://github.com/cairn-ehr/cairn-ehr/issues/242) (the
`asserted` grade + token-backed author — verbal orders / AI-scribe; the floor's verified-attester arm already
reserves room), [#243](https://github.com/cairn-ehr/cairn-ehr/issues/243) (durable drafts + `sign-as` salvage —
the ADR-0008 UI half), [#244](https://github.com/cairn-ehr/cairn-ehr/issues/244) (author+responsibility on one
event), [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) (SQL mirror + §5.10 authorship-confidence
projection). **This closes Priority 3 — both wire windows are shut; the review course continues at Priority 4
(#188 [D1], the schema-version guard).**

**Post-review `/fixall` pass on PR #246 (2026-07-18).** A whole-diff `/review` found **0 correctness
defects** — the floor predicate, the strict-enforce/apply-grade asymmetry, the kid-derived-from-key CLI path,
and the no-regression claim over all four pre-existing bearing-contributor call sites (`apply_proposal`,
`shred`, `medication::attestation`, `auto_apply` — each already sets `signer_key_id` to the human it names,
so none trips step 4b) were each independently checked and held. It did find **one real coverage hole and
four polish items**, all fixed on-branch: (1) *[the substantive one]* every author test passed
`attest: None`, so the advertised `--author-as` + `--attest-as` **composition was untested** — and the only
path reaching it, `submit_reconcile_like`'s attested arm, **hand-duplicated** the author rewrite from
`seal_sign_submit`, giving that duplicate zero coverage; the duplication is now the shared
`sealed_submit::apply_author` (which is also what guarantees the **non-idempotent** `with_human_author` is
applied exactly once per body — calling it twice prepends a second `authored` contributor), covered by
`author_and_attest_compose_with_different_humans_on_reconcile` using **two different humans** (registrar
authors, supervisor vouches) so a regression collapsing author into attester cannot pass; **proven RED by
sabotaging the rewrite before accepting it green**. (2) `authorship_binding.rs` tested only the NULL-attester
arm — the **verified-attester arm** (the one the deferred #242 token-author path authenticates through) now
has 4 cases, including one-good-one-forged. (3) `classify_authorship_confidence` is doc-flagged **NOT YET
WIRED TO A READ PATH**: it has no production consumer, so the ADR's "apply admits and **grades**" is today
only half-live (apply admits; #245 brings the grading). Not a live hole — no read path surfaces the
contributor set to a clinician yet — but the type must not be mistaken for enforcement. (4) db/005 now
records that the `bearing:` prefix arm of `cairn_authorship_bound` is **unreachable at its only call site**
(step 1c's strict `cairn_check_contributors` already refuses unratified roles) and is kept for safe reuse by
a lenient caller, not as live future-role coverage. Workspace **777/0** (all 3 DBs, **0 self-skips**) +
fmt + clippy `-D warnings` + cargo-deny + mkdocs clean. One issue filed:
[#247](https://github.com/cairn-ehr/cairn-ehr/issues/247) — `contributors[].actor_id` holds a signing-**key**
id, not an `actor_current.actor_id`, so authorship is **key-scoped and does not survive key rotation** or
re-enrolment under a new skill_epoch (pre-existing, shared with the #195 binding; #99 already solved the
same problem at row level by stamping the resolved actor, and the contributor set is now the one place that
does not). It constrains #245: the projection must either join through `actor_event` at read time or we
record an ADR saying key-id-in-body is the permanent shape. **Behaviour note for the UI layer:** making the
human the signer also *tightens* the ADR-0043 suppression owner-gate — a `--author-as` event is now **owned**,
where the same event device-signed and un-attested was un-owned and dismissable by anyone. Intended and
covered by `human_author_owns_suppression_rights`, but it is a change in *who may dismiss*, not purely a gain
as the ADR's "for free" phrasing suggests.

**Slice 44 — the P4 tech-debt slice: the schema-version guard + db/tests in CI (2026-07-19; the review
course, Priority 4; issues #188 [D1] + #238 + the #212 CI half; branch `claude/tech-debt-cleanup-8513ce`,
PR #251; no ADR/spec change — first brick of the settled ADR-0012 code plane).** A triage of all 50 open
issues for "blocks other development" put three items in tier 1; all landed in one branch. **#188 (the
Critical latent hazard):** `db/038_node_schema.sql` — a singleton `node_schema(version, loaded_at,
loader_build)` — plus a downgrade-refusal guard in **BOTH** loaders (cairn-node `connect_and_load_schema`,
the every-connect silent replay path, and cairn-sync `init`, which now replays its subset through a guarded
`load_schema`): a recorded generation ABOVE the binary's embedded one refuses with a legible error before
any `CREATE OR REPLACE` runs; an absent table/row means "generation unknown, proceed" (hand-loaded rigs
stay usable — explicitly tested); the stamp lands only after a full successful replay. The generation is
the **repo-wide constant** `cairn_event::schema_generation::SCHEMA_GENERATION`, shared by both doors
because cairn-sync's subset legitimately LAGS db/'s newest file — the PR-#251 review round caught that the
original per-list derivation would split the two doors' generations the moment a node-only migration lands
(cairn-sync `init` would then refuse every healthy node); kept honest by a fs-derived cairn-event guard
test (constant == newest `db/*.sql` on disk) + a cairn-node completeness unit test + cairn-sync
subset-shape tests. The same review round caught a check-then-act (TOCTOU) hole — an old + new binary
connecting together could interleave into the very silent downgrade #188 targets — so both loaders hold
the session-level `SCHEMA_LOAD_LOCK` ("CARNLOAD") advisory lock across check→replay→stamp, pinned by
deterministic interleaving tests on both doors (RED first). **#238:** the `wait_listening` readiness
ceiling 5 s → 60 s (the poll returns on first accept, so the ceiling is pure headroom under
parallel-workspace CPU load). **#212 (CI half), DECIDED wire-not-delete:** `scripts/run-db-sql-tests.sh`
builds a throwaway `cairn_sqltest` database (refuses `cairn_test*` dbnames, so the spike-only db/008 its
own test needs never touches the shared test DBs), loads every migration, runs all 10 `db/tests/*.sql`
mirrors under `ON_ERROR_STOP`; wired into `rust.yml` after the cargo-test step — a missed twin-registry
mirror bump now fails CI instead of drifting (the #183 luck-catch, mechanized). Noticed in passing and
added to the #212 remainder: a FOURTH hand-mirror of the loader list at
`matcher/tests/conftest.py::_SCHEMA_FILES` (ends at 025), and a FIFTH — the guard/replay/stamp logic
itself is hand-mirrored between the two loaders (async/sync split, low churn). **The review course
continues at the Priority 5 remainder (#212 drift guards + verb-then-vouch, #214, #215, #213).**

**Slice 45 — the P5 process-mechanization session: #212 remainder + #213 + #214 + #215 (2026-07-19;
the review course, Priority 5 — CLOSES the whole 2026-07-15 review course; branch
`chore/p5-process-mechanization`; spec v0.55, prose honesty only, no decision change).**
**#214 (the self-propagating mislabel):** medication prose lives at data-model **§3.3** (§3.15 is
active-write, §3.16 is ICD-11); the wrong cite was baked into the ADR-0048 locked registry rows'
error strings (db/031–034), the byte-for-byte Rust mirror, every migration header (each new file
copied it from the previous — four times), module docs/test headers, and ROADMAP Slice 30b. The
medication twin-check registrations flip `ON CONFLICT DO NOTHING → DO UPDATE` so **replay CONVERGES
existing registry rows to the migration text** (under DO NOTHING the fixed strings could never reach
an existing DB); pinned by a tamper-then-replay heal test. **#212 (the drift pairs):** the two
hand-rolled framing implementations are now thin I/O wrappers over ONE pure core
(`cairn_event::framing` — cap-before-alloc, refuse-at-source, u32-truncation-unreachable; the cap
VALUE stays per-plane policy, node 8 MiB / clinical 64 MiB, deliberately different); the
`reviewed_count` hand copy of the four content-event types is deleted — `thread_commitment_on` calls
`cairn_medication_thread_readable_count`, the same db/034 fn the ADR-0049 false-fresh gate compares
`reviewed_count` against, so the two are definitionally one measure; the matcher conftest
`_SCHEMA_FILES` hand-list (silently stalled at 025 while the loader grew to 038) is now fs-derived
(minus the spike-only 008) under a newest-file-included guard test; the six-fold verb-then-vouch
copy was verified **already factored** by ADR-0052's `seal_sign_submit` (the two residual
`submit_event` sites — the two-thread reconcile path and the attestation token door — are
legitimately distinct); and the **first property-test suite** landed (proptest, MIT/Apache-2.0,
cargo-deny-verified): pure laws for `classify_role` (total; ratified/partition-prefixed/honestly-
Unknown over arbitrary strings — the #96 wire case) and `classify_authorship_confidence`, plus a
DB-gated hostile-JSONB property through the live twin floor (48 arbitrary bodies: twin-less ⇒
legible raise, backend survives; proven to bite by sabotage). **The property run immediately caught
a real defect:** a bearing-role contributor with a missing `actor_id` (an anonymous authorship
claim) was dropped by `filter_map` and graded `Device` instead of `Unverified` — the exact
"claimed-but-not-authenticated collapses to device-generated" reading the enum doc forbids; fixed
(anonymous claims count as bearing, never authenticate) before any read path ships (#245).
**#213 (hygiene):** keystore plaintext-seed temporaries all `Zeroizing` (the #54 discipline extended
one layer up); house-rule-6 bench/test crypto literals derived via `std::array::from_fn` (incl. the
non-test `bench_sign_verify`); the auto_apply advisory-lock leak closed **by construction**
(`ceremony_locked` body + one unconditional unlock in the wrapper); `normalize_recovery_code` now
MAPS Crockford-ambiguous glyphs (I/L→1, O→0, TDD RED-first) instead of deleting them — a
transcription slip on the disaster-recovery path unseals instead of "wrong passphrase"; the
cairn-gui merge fallback is validated against `offered` (an unoffered site default no longer
surfaces through the very filter meant to stop it, TDD RED-first). **#215 (prose honesty, spec
v0.55):** index.md/CLAUDE.md status currency (+ a "HANDOVER wins on build state" pointer at the
source of the staleness); ROADMAP's duplicate Slice 30 disambiguated as 30b (renumbering would break
Slice-N cross-references); sync.md §6.3 gains the quarantine/re-offer floor's spec home (refusal +
durable re-offer row; the `acked=TRUE` recorded-human-decision exception named as the ONE deliberate
exception to never-drop; the honest unbounded-pen row — no cap/expiry, a denial-of-storage exposure
bounded by peer admission); sync.md §6.1 states the ADR-0045 convergence claim's honest limit
(same-projection-code only) + the winner-rules-are-ADR-gated convention; security.md records the
**human key-loss recovery ceremony as an unspecified gap needing its own ADR before any pilot
enrolls real humans** (ADR-0044/0046 anti-reuse guards + #247 key-scoped authorship make ad-hoc
answers dangerous) and the SMS rung's carrier-dependent/phishable caveat; identity.md flags the
deceased-status veto as a db/016 stub and the alias pool's rung-2 erasure-reach note. **Also:**
cairn-gui rustfmt + clippy drift cleaned (the tree sits outside the CI gates);
[#252](https://github.com/cairn-ehr/cairn-ehr/issues/252) filed — quick-xml RUSTSEC-2026-0194/0195
(DoS-class) via wayland-scanner in the gui lock, upstream-blocked (the #11 shape). Verification:
workspace **800/0** (all 3 DBs), matcher 383 + ruff, cairn-gui green, all 10 SQL mirrors on a fresh
throwaway DB, fmt ×3 trees, clippy `-D warnings` ×2, cargo-deny, mkdocs. **Post-review fix pass
(same day, on the PR):** the `DO UPDATE` arms gained an `IS DISTINCT FROM` guard so the
steady-state replay is write-free again (RED-first via an xmin-stability test — the unguarded arm
rewrote all 7 medication rows on every connect); the heal test now also tampers+heals `check_fn`
(to an *existing* wrong fn — the db/005 validate trigger fail-closes on nonexistent ones); the
matcher-conftest exclusion-direction coupling is documented at the site; and
[#254](https://github.com/cairn-ehr/cairn-ehr/issues/254) filed — the other 8 registry files still
`DO NOTHING`, so a future string fix there would pass fresh CI but never reach a standing rig
(unify-or-decide, house rule 5). **With this the 2026-07-15
review course is fully closed; next is the Priority-6 design queue (#205 first) or the now-unblocked
feature work.**

**Slice 46 — the #205 design session: ADR-0054 actor-registry federation (2026-07-19; P6 design
queue, first item; spec v0.56; docs-only — code slices are future feature work).** The C4
contradiction (fail-closed enrolment vs never-reject set-union custody) is resolved as
**admit-and-dispute**: the actor event becomes a first-class signed wire event (node-signed COSE
under a dedicated context, content-addressed, HLC+origin; `(HLC, content_address)` winner order;
pre-wire rows never sync) riding the **node plane** (deny-all peers, full replication in the trust
neighborhood, db/022 pen); the apply door **admits unconditionally and detects** — the conflict
becomes a **derived disputed state over live bindings** (log convergence ⇒ state convergence, no
dispute events), under which `actor_current` picks no winner, implicated events attribute to the
honest **candidate set**, and **registry uncertainty withholds permissions, never content** (closes
#154 structurally; the #172 sync-door half discharged by specification). Adjudication = the
existing `supersede` by audited human ceremony (join / per-key fork / revoke+cascade), with
conflicting adjudications honestly re-deriving as disputed. Considered-and-rejected: pen-outside
(registries diverge), deterministic tiebreak (automated identity resolution — principle 2's
forbidden move). Spec homes: security §7.5, sync §6.3+§6.9, data-model §3.12, identity §5.10.
Design/plan docs under `docs/superpowers/`.

**Slice 47 — the #206 design session: ADR-0055 distribution-plane trust-root governance
(2026-07-20; P6 design queue, second item; spec v0.57; docs-only — code slices filed as
follow-ons).** Review finding C5 (single steward key signing the highest-blast-radius artifact)
resolves by applying the corpus's own anchor doctrine to the steward: **no privileged root on the
distribution plane** — a **channel** `{trust-root chain, transparency log, release stream}` is the
trust unit, the root is provisioning config on the ADR-0017 spectrum, the steward is only the
official channel's default anchor. Mechanism: a **chained, content-addressed trust-root document**
(version N+1 signed by ≥ M_root of version N; explicit TUF-shape multi-sig, N=1/M=1 first-class;
monotonic pin; **no expiry** — availability floor, compensated by retirement key-destruction +
log/gossip fork detection), a **root/release role split** (constitution key vs daily pen — a
release-key compromise is one rotation, not a fleet re-provisioning), a **fork-freeze rule** (two
verifiable successors ⇒ security incident + ceremony, never arrival-order), the **verify-or-refuse
load gate** (newest-root rule: `root_version_ref` is diagnostic, never authoritative; legible
refusals; co-signer floor), the **transparency log** (ADR-0027 shape as a self-hostable node role;
rebuilder attestations = co-signatures; limits stated: genesis TOFU procedural, young-log monitor
scarcity, reproducible builds carry near-term weight), an **honest N=1 posture with a ratchet
tripwire** (≥ 2-of-3 before the first production deployment outside the steward's control;
ADR-0026 escrow custody floor), and **one root shape for all three provisioning-time roots**
(§7.6/§7.9/§7.7). Code plane vs content plane now carry opposite postures (verifies-or-refuses vs
admits-and-disputes — the ADR-0054 contrast completed). Follow-ons filed: #257 (verifier/load-gate
code), #258 (transparency-log role), #259 (reproducibility CI), #260 (freshness rung), #261
(sync-auth onboarding UX design session). Design/plan docs under `docs/superpowers/`.

**Slice 48 — the #200 design session: ADR-0056 unknown event types admitted uninterpreted
(2026-07-20; P6 design queue, third item; spec v0.58; docs-only — code slices filed as
follow-ons).** Review finding B5 filed #200 as a spec *over-promise* ("refusal + durable re-offer
*is* the contract, it just needs stating"). Investigation found the opposite: the **spec was right
and the code was wrong**. Three findings, all verified — (F1) `sync.md` §6.5's lossless-forwarding
invariant ("stores, re-propagates and exports byte-for-byte — never rejecting") is **contradicted**,
not merely imprecise, by `db/020:163-167`'s fail-closed on an unclassifiable `event_type`; it holds
for unknown *fields*, never for unknown *types*. (F2) §6.3's claim that refused bytes "are
quarantined *verbatim* by digest" is **false for verifiable bytes** — both pens hold *unverifiable*
bytes only (`clinical_pull.rs:766-769` asserts `penned == 0`). (F3) the two planes diverge and the
clinical one fails **quietly**: node plane skips-and-advances (10-cycle sweep re-offer,
`sync.rs:680-685`), clinical plane freezes its cursor and **still exits success**
(`main.rs:1697-1716`, `PullIntegrityError` at `:1834` never fires on a freeze) — so one
unclassifiable event from an upgraded peer silently wedges a whole peer's pull.
**The deciding failure:** under fail-closed, a phone-tier node carrying a chart between two upgraded
facilities (§6.1 sneakernet) acquires *nothing* past the first unknown type — a future
`clinical.medication.recall` would be **absent**, not merely unrendered (`cairn_twin_skeleton`
already renders any type). Resolution — **admit-and-defer**: an unknown type is *not a refusal*
(stored verbatim, re-propagated, skeleton-twin rendered, **no projection rows, no power**); the
**strict door keeps failing closed** (carry a type you cannot author — ADR-0051's
strict-submit/lenient-apply applied to types); the **floor gates effect, not presence** (enumerated:
signature/enrollment/envelope/oversize/`t_effective` ceiling/never-lawful contributor shapes still
refuse — each decidable from the envelope alone; the suppressing⇒attestation gate is moot since
suppressing power is withheld); **power is granted at reclassification, never retroactively
assumed** — classification arrival **re-runs the deferred classification-gated checks and only then
reprojects**, so *no unattested suppression* holds at every instant rather than
violated-then-repaired (couples to #208); and **refusal + durable re-offer survives as the residual
contract** for genuine refusals. **No wire change** — ADR-0010's derived-not-declared stands, so no
self-declared mode field (a declaration can lie).
**The posture triad completes:** content plane admits-and-disputes (0054) *and* admits-and-defers
(0056); code plane verifies-or-refuses (0055) — content withheld is a safety failure, code admitted
is a compromise. Cost is small: `serve` already reads `event_log` unconditionally, sealed-scope is
**not enforced at the remote door at all** (strict-door-only by design — `db/020:229-234`), and
`cairn_event_twin` already degrades — **one fail-closed line** stands between the tree and the
contract. Follow-ons filed: #265 (door admits uninterpreted), #266 (re-adjudicate + reproject on
classification), #267 (pen door refusals verbatim), #268 (align node-plane skip), #269 (node-plane
heal test gap), #270 (frozen watermark must fail loud). Design doc under `docs/superpowers/specs/`.

**PR #271 review corrections (2026-07-20; folded in before merge).** Three findings against the
first draft, all fixed here and in the ADR: (i) the enumerated floor list wrongly named **sealed-scope**
as a remote-door refusal — `apply_remote_event` deliberately mirrors neither the born-sealed scope
rule nor the unopenable-body refusal (`db/020:229-234`, reason at `db/005:658-661`: *"a refusal there
would freeze the seq watermark on a verifiable event"* — ADR-0056's own argument), and leaving it
would have pointed #265's implementer at exactly that failure; (ii) decision 4 said only
"reprojects", but admitting uninterpreted **skips** the attestation gate, the overlay-target-exists
check and the ADR-0043 cross-author-suppression refusal (all downstream of the NULL `v_mode` /
`v_targets_other`), so reclassification must **re-adjudicate before backfilling** or power is granted
that never passed a gate — #266 retitled and rescoped accordingly, and the deferred state must be
recorded *explicitly* rather than inferred from NULL fall-through (noted on #265); (iii) ADR-0056 was
the only ADR in the corpus citing `file:line` — converted to symbol-level references, since ADRs are
immutable and #265 deletes the very line the central claim cited. Line-level evidence stays in the
mutable design note and here.

**Slice 49 — the #208 generic-reprojection slice: ADR-0057 (2026-07-20→21; P6 design queue, fourth
item — but taken brainstorm→spec→plan→TDD build end-to-end, subagent-driven Tasks 0–10; branch
`design/208-generic-reprojection`;
[ADR-0057](spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md), spec v0.59).** The
~15 per-type `AFTER INSERT … FOR EACH ROW WHEN (event_type = …)` projection triggers each healed only
*future* inserts — a `CREATE OR REPLACE` left every already-materialised row wrong (ADR-0045's
read-side-only winner fix was the worked example; the tree's one bespoke `cairn_demographic_backfill()`
re-expressed its trigger's winner logic twice more and ran a full `event_log` scan on **every
connect**). Replaced by **one code path**: a locked registry `cairn_projection_apply(event_type,
apply_fn, projection_tables, run_order, heal_safe, PK(event_type,apply_fn))` (fail-closed load-time
validation, `REVOKE`d, ADR-0048 discipline) + **one dispatcher** `cairn_projection_dispatch`
(`AFTER INSERT ON event_log`) that calls each registered `apply_fn(NEW)` in `run_order` — every former
trigger body mechanically refactored to `fn(event_log) RETURNS void`, the `WHEN` clauses dropped; an
unregistered projection cannot fire. **`cairn_reproject(prefix, rebuild, source)`** feeds `event_log`
through the *identical* dispatch (single logic path for replay too), recording per-type counts in a
node-local `reproject_log`: **heal** (default, no deletes — converges the wrong-winner class by
arrival-order-independence) vs **rebuild** (`TRUNCATE`+replay, wrote-garbage class, refuses a narrow
prefix over a multi-type table). The generic mechanism **subsumes** `cairn_demographic_backfill()` and
its every-connect call (both deleted); both loaders instead run `cairn_reproject('', false, 'loader')`
**only on a schema-generation change**. `cairn_replay_eligible(e)` is the #265/#266 seam (constantly
`true` today — no deferred events can exist until #265). **Review-caught corrections:** (i) the loader
heal runs **before** the `node_schema` generation stamp in both loaders — a failed heal withholds the
stamp and the next connect retries, closing the silent-stale window a stamp-then-heal would open
(load-bearing, not cosmetic); (ii) the three append-only alarm tables (`identity_projection_flag`,
`medication_projection_flag`, `medication_patient_conflict_flag`) dedup replay by **event identity** —
`content_address` added to their unique key with `NULLS DISTINCT` — never by observation shape (a
blanket `ON CONFLICT DO NOTHING` on the natural key would have collapsed two genuinely distinct events;
`NULLS DISTINCT` leaves pre-fix legacy rows untouched, *never erase*); (iii) heal mode **skips**
`heal_safe = false` rows (the counter-shaped `patient_chart_apply` note-count increment, which would
double-count) and reports them in `reproject_log.skipped_fns` — rebuild heals them. **Structural
guards:** a catalog test asserting the *only* `AFTER INSERT` trigger on `event_log` is the dispatcher;
22/25 registry row-count pins mirrored in Rust **and** SQL (the #212 two-place pattern). **Measured at
Bet-B volume** (Mac dev box, 2,006,000 events / 200 patients): write-path through the live dispatcher
**p50 0.076 / p95 0.236 ms** (~17× under the Pi B1 budget); heal replay of the 200,580 applicable
events in **2.098 s** (~95.6k ev/s, set-based — a **5.8×** speedup over the 12.2 s per-event PL/pgSQL
loop it replaced, the Task-8 stop-gate resolution); rebuild of all 2,006,000 in **54:32**,
**loop-invariant by construction** — its cost is the apply fns' own write work, so rebuild ≈ re-ingest
cost (the live per-row ingest path ran ~1.2 ms/ev on the same corpus) and rebuild is explicitly **not**
under a low-latency SLA. **Follow-ons:** [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (the
authoritative Pi5/NVMe same-rig A/B re-run — the Mac numbers establish shape and clear budget but are
cross-rig), [#273](https://github.com/cairn-ehr/cairn-ehr/issues/273) (a pre-existing db/035 gap
surfaced during the conversion: the dose-correction apply fn's live body had lost the #192
patient-consistency guard call — house rule 5; **fixed in PR #278**: guard restored in db/035 and the
fn's `medication_patient_conflict_flag` write declared in its `cairn_projection_apply` inventory, db/032),
and [#277](https://github.com/cairn-ehr/cairn-ehr/issues/277)
(the loader's gen-change heal cannot re-derive `ON CONFLICT DO NOTHING` projections — `medication_dose_*`,
`medication_attestation` — after an extraction-logic fix: `heal_safe=TRUE` means replay-safe, not
auto-healable; caveat documented at db/005's `heal_safe` definition, surfaced in the PR #274 review).
#266's reclassify-then-reproject path **consumes** this mechanism through the `cairn_replay_eligible` seam.

**Slice 50 — the #216 grade-gated `t_effective` ceiling: ADR-0058 (2026-07-22→23; P6 queue, fifth item,
brainstorm→spec→plan→subagent-driven-TDD Tasks 1–8; branch `design/216-grade-gated-teffective-ceiling`;
[ADR-0058](spec/decisions/0058-grade-gated-teffective-ceiling.md), spec v0.60).** The bitemporal ceiling
`t_effective ≤ t_recorded` was enforced as a **binary check against the point HLC wall** at both doors —
two coupled defects (2026-07-15 review I3/G): (1) a **principle-4 violation** — a node whose clock reads
behind true time (a slow RTC, a misconfigured TZ, or a **dead/absent RTC** — the freshly-booted offline
Pi, months-to-decades off) rejected a *truthful* clinician's forward `t_effective`, manufacturing a
falsification finding; (2) a **live sync-wedge DoS** — the remote door's hard-`RAISE` on a *verifiable*
forward-dated event set `do_pull`'s `frozen=true` and halted the seq cursor, so one signed event (Spike-0002
threat model) wedged all clinical replication from that peer. Fix: a **born `clock_grade`** (ADR-0027
ladder, mandatory `EventBody` field, mint constrained to `self-asserted`) **gates the ceiling's rejecting
power** via one pure classifier `cairn_ceiling_classify(hlc_wall, grade, t_eff) → ok|flag|reject` (db/040):
at `unknown`/`self-asserted` (every node today) the upper bound is **open**, so a forward `t_effective` is
**flagged, never rejected**; the **write door** (strict) additionally **floor-enforces the mint
constraint** (PR #285 review finding 1: any ratified grade above `self-asserted` is refused outright —
a self-declared high grade at the authoring door can only be a forged trust brand; the classifier's
dormant `reject` arm stays covered by the SQL truth table until #279 makes high grades mintable), the
**remote door** (lenient) **never rejects on
the ceiling** — it admits unchanged and records an advisory `t_effective_ceiling_flag` row (cross-type
door-side write, not an ADR-0057 projection), closing the DoS by mirroring the door's own HLC-drift
clamp-and-admit rule; `emit_event`'s direct author-side INSERT runs the same classify+flag (PR #285
finding 2), so the author's flag ledger matches its peers'. Corrects ADR-0027 §6 `upper=RTC`→`RTC+W(grade)`. Adds `cairn_clock_health()`
(SECURITY DEFINER, ADR-0027 §7 honest-assembly read: RTC-vs-HLC-floor, `is_behind`, `effective_lower_bound`)
surfaced in `status`. `SCHEMA_GENERATION` 39→40 (db/040 in both loader lists). The headline test is the
`do_pull` wedge regression (a forward-dated event no longer freezes the pull). Mint constrained to
self-asserted — floor-enforced at the strict door, so no config-declared or hostile-signed grade can
either re-arm the reject or mint a falsely trusted timestamp. Deferred (filed): [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279)
anchor/notary planes + overlay grade-upgrade, [#280](https://github.com/cairn-ehr/cairn-ehr/issues/280)
causal lower-bound tightening, [#281](https://github.com/cairn-ehr/cairn-ehr/issues/281) UI clock alert,
[#282](https://github.com/cairn-ehr/cairn-ehr/issues/282) auto-downgrade, [#283](https://github.com/cairn-ehr/cairn-ehr/issues/283)
twin grade-line, [#284](https://github.com/cairn-ehr/cairn-ehr/issues/284) SCHEMA-list cross-check. **Operational
caveat:** the mandatory born field means pre-slice dev/PoC rigs read `unknown` — **wipe rigs**; and a
cairn_pgx built before the `clock_grade` field silently drops it (rebuild required).

**Slice 51 — matcher review-follow-ons #209 + #210 (2026-07-23; advisory Python tier; branch
`fix/matcher-threshold-anchor-209-stale-proposal-210`; no ADR/spec/SCHEMA/event-type change — a TDD
bugfix wholly inside `matcher/`).** **#209 (learner safety anchor):**
`eval/learner.derive_thresholds` fell back to `review = min(match)` when the labelled scores contained
no non-match pair — anchoring `auto` to the WEAKEST true match, so ordinary shared-name-token
non-matches band AUTO_CANDIDATE on held-out data (a false auto-link) while `collided` stayed trivially
False, vacating the documented "auto = max(non-match)+margin ⇒ zero false auto-links" invariant with no
warning. Now **fails closed** (raises: with zero impostors the anchor-to-the-strongest-impostor
contract is unsatisfiable), and `eval/crossval.kfold_lift` gains a `_has_nonmatch_pairs` guard that
**skips-and-counts** a fold whose training partition has no non-match pairs (symmetric with the existing
no-match-pairs skip), so a single degenerate fold never aborts a run; the `learn.py` CLI already wraps
the raise into a clean exit 2. **#210 (stale-proposal leak, the #135 hazard one layer deeper):** a
PENDING proposal whose pair dropped OUT of the blocking universe (a John Doe forced to REVIEW while
unconfirmed, then fully identified — its year-range DOB replaced by a point date, so no pass regenerates
the pair) was never re-scored, so `retract_pending_proposal` (only called inside `propose()` for
currently-generated pairs) never fired: a stale REVIEW row grouped a resolved chart under a nonexistent
Doe indefinitely. `pipeline/sweep.sweep` now runs a **reconciliation pass** — re-`propose()`s every
currently-PENDING pair the sweep did NOT regenerate (new `db.pending_proposal_pairs` reader, read in the
same closed-before-write snapshot; new `SweepResult.reconciled` counter), reusing propose()'s existing
band-None retract path. Re-scoring rather than blindly deleting is deliberate: a pair withheld only by a
block-size cap re-bands and is re-persisted, never wrongly withdrawn; a human/auto disposition is doubly
protected (`retract_pending_proposal`'s `WHERE status='pending'` + `upsert_proposal`'s
retracted→pending arm). TDD RED-first for both fixes; the #210 test guards that the pair genuinely left
the blocking universe (the INVERSE of the #135 end-to-end test's still-blocks guard) so only the new
pass can retract it. Full matcher suite **386/0** + ruff clean + an independent code-review pass (no
defects). Open follow-on: **[#211](https://github.com/cairn-ehr/cairn-ehr/issues/211)** (four minor
matcher logic gaps — the E3 batch) remains.

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
