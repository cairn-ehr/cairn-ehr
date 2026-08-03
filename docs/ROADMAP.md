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
  must collapse to ONE human gesture — the node-tier gesture landed in Slice 61; the *measurable* one waits
  on the UI), [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) (the §5.9 safety projection must
  *carry* the coding-derived drug class rather than re-derive it; blocked on #232),
  [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334) (a reconciled group spanning two patients
  displays on one chart only — wrong-chart hazard; read-path defence shipped, view unfixed),
  [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331) /
  [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) /
  [#335](https://github.com/cairn-ehr/cairn-ehr/issues/335) /
  [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336) /
  [#337](https://github.com/cairn-ehr/cairn-ehr/issues/337) (Slice 61 follow-ons).

**Operational caveats that outlive these slices.** Pre-ADR-0051 event logs (old `role:"author"`-without-
actor_id, flat-string responsibility) and pre-ADR-0052 plaintext `clinical.*` bodies **REFUSE at db/020** —
**wipe dev/PoC rigs** (the replication-failover demo, the spike rigs), never sync them through. Pre-wire
unsigned actor rows never sync. Test DBs need `cairn_pgx` ≥ 0.3.0.

**Slice 57 — `clinical.medication` slice 6b: the coding-overlay event types (2026-07-28; branch
`feat/medication-coding-overlay-slice-6b-0059`; completes
[ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) decision 3; no spec/ADR
change; `SCHEMA_GENERATION` 41→42).** Coding becomes a **separately-authored act** (`db/042`, both types
`('additive', FALSE)`; a correction ADDS a claim, so a pharmacist is not routed through the ADR-0043 owner
gate). **Why the strike exists** — the decision the slice turns on: a reviewer who establishes a medication
is NOT metformin but cannot say what it is must be able to record *"not that, and I don't know"*, rather
than leave a known-wrong anchor standing or invent an identity they cannot vouch for (principle 4). A
strike NULLs the anchor rather than deleting the row, which would break arrival-order independence.
`patient_medication_uncoded` is the coder worklist; CLI `medication-code` / `medication-code-correct`,
deliberately **no** `--attest-as` — coding a drug identity is not a sign-off of the medication list. Also
closed #295 and **#296** (a cairn-sync test dropped `event_log.seq` and permanently reordered a SHARED test
database — root cause of the long-carried "recreate the test DBs" gotcha).

**Lessons kept:** a **redundant projection column is a convergence hazard** (`struck` duplicated
`coding_code IS NULL` with only two of three writers setting it, so arrival order decided what a node read
— now `GENERATED ALWAYS AS … STORED`; deliberately not a CHECK, since a violated CHECK aborts the apply and
wedges that event forever) · **nullable-widening a column means re-reading every aggregate over it**
(`array_agg` KEEPS NULLs) · **a passing test can be worthless** (asserting only `coding_display` went green
against a live defect) · **only `cargo test --workspace` catches guard-scope gaps**.

**Still open from this slice:** no drugref code anywhere in the tree, so the **coded↔uncoded** duplicate
case stays open (ADR-0059 decision 5 is explicit the key does not close it);
[#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) (§5.9 safety class, blocked on #232);
[#300](https://github.com/cairn-ehr/cairn-ehr/issues/300) (worklist lists every uncoded member of an
already-coded group — a design question, not a defect); the coding UI and its §1.2 budget.

**Slice 58 — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power (2026-07-29 + review
round 07-30; branch `feat/adr-0056-admit-uninterpreted-floor-265-266`, PR #302; closes
[#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) +
[#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) — decisions 1 and 4 of
[ADR-0056](spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md); `SCHEMA_GENERATION` 42→43).**
`apply_remote_event` used to RAISE on an `event_type` absent from `event_type_class`, so the event was
**never stored at all** — a phone-tier node carrying a chart between two upgraded facilities (the §6.1
sneakernet path) acquired nothing past the first unknown-type event: not unrendered, *absent*. §6.5's
lossless-forwarding invariant was false for unknown types; the spec was right, the code was wrong. The door
now admits verbatim, projects nothing, confers nothing, and records `event_deferred` (node-local, never on
the wire — its presence IS the invariant). `cairn_readjudicate_deferred` (db/043) re-runs the
classification-gated checks **before** anything reprojects, which is what makes "no unattested suppression"
hold at every instant rather than being violated-then-repaired; it runs on every connect, while
reprojection stays generation-gated. Full design + the reasoning-failure record in
[the design doc](superpowers/specs/2026-07-29-adr-0056-promotion-must-be-proven-design.md).

**The four lessons worth carrying** (detail in git, the ADR, and the design doc):

1. **Refusal hides; admission cannot.** Choosing between refusing and admitting-without-power, admitting is
   the recoverable direction.
2. **An unverified value stored "for later" leaks into a live gate.** A forged travelling attestation token
   would have entered the ADR-0043 owner gate's author set. Two rules: when you store a value you have not
   verified, **name the state** and **audit every reader**; and the fix for an unverified input is
   **neutrality, not strictness** — compute as if nothing travelled.
3. **A promotion must PROVE the event takes effect, never assume it.** Promotion once deleted the marker
   without checking the event could project; the loader's heal then raised, and `event_log` being
   append-only, nothing could undo it — **the node bricked**. Fixed by a structural gate plus running the
   type's heal-safe apply fns inside the promotion subtransaction.
4. **Test pollution is designed against, not cleaned up after** — de-classify in `setup()`, not at test end,
   which does not survive a panic.

Also: **five of the review round's seven tasks had plan-mandated defects**, each a flaw in the plan's text
rather than implementer error — mandated test code is not exempt from review. **Measurement caveat that
outlives the slice:** without `CAIRN_TEST_PG2`/`PG3` the multi-node convergence suites self-skip and cargo
counts them as **passed**, so a workspace count alone cannot distinguish a skip from a pass.

**Still open from this slice:** [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301) (the node/actor
plane still fail-closes on an unmappable type, so §6.5's invariant is true **for clinical events only** — a
known gap, not a design), [#308](https://github.com/cairn-ehr/cairn-ehr/issues/308),
[#309](https://github.com/cairn-ehr/cairn-ehr/issues/309).

**Slice 59 — floor determinism + tech-debt-loop launch readiness (2026-07-31; PR
[#311](https://github.com/cairn-ehr/cairn-ehr/pull/311) closes
[#75](https://github.com/cairn-ehr/cairn-ehr/issues/75); no ADR/spec change, no `SCHEMA_GENERATION` bump).**
Two threads, both below the clinical surface.

**(1) The twin blank-test was collation-dependent — a convergence break, not the cosmetic asymmetry #75
described.** The §3.13 question *"did the author supply a twin, or must the floor derive one?"* was spelled
out three times in SQL (`regexp_replace(t, '\s+', …)`) and once in Rust (`!t.trim().is_empty()`). #75 filed
the Unicode gap as benign. Measuring it found worse: Postgres's `\s` is `[[:space:]]`, whose membership is
decided by the **collation's ctype** — `iswspace(U+00A0)` is true under a libc UTF-8 collation, false under
`C`/`ucs_basic`. Since `cairn_event_twin` is also the remote-apply gate and RAISEs for a hard-require type,
**the same signed event could apply on one node and raise on another** — a set-union convergence break
(principle 1). Fixed by one definition per language: `cairn_twin_is_present(text)` in db/005 (`btrim` over
the 25 Unicode `White_Space=Yes` code points, written as `U&'\XXXX'` escapes so a reviewer can *see* them —
a pasted NO-BREAK SPACE is invisible in source), db/015's predicates delegating to it, and Rust's
`twin_is_present` made `pub` as the cross-boundary contract. Three tests written first, all red — including
an exhaustive proof that the 25-point list IS the complete `White_Space` set, and a parity test classifying
every BMP code point on **both** sides. **The generalisable catch: a "merely cosmetic" asymmetry between two
implementations of one predicate is worth measuring before it is filed as benign.**

**(2) The tech-debt loop's first real run.** Halted at `failed-permission` (fixed durably by PR #310 —
`scripts/run-db-gated-tests.sh` bakes the DB env in, since permission rules are prefix matches and a leading
`VAR=value` can never match an allowlist entry). Three preflight corrections, recorded in
`.claude/skills/techdebt-loop/SKILL.md`: repo auto-merge **is** probeable (a recent `autoMergeRequest` on a
merged PR proves the setting was on); the worker takes the **lowest-numbered** `loop:ready` issue and triage
never re-checks the label, so a mistaken label parks at the front forever — the head was #11, which would
have made the first unattended cycle attempt a **major-version crypto bump on the §9 signing surface**, so
the skill now mandates a head-of-queue inspection before an unpinned launch; the mechanism gap is
[#312](https://github.com/cairn-ehr/cairn-ehr/issues/312). Paper-parity: not clinical-surface — an in-DB
determinism fix plus development tooling.

**Interlude — the tech-debt loop ran unattended (2026-07-31 → 08-01).** Nine PRs merged with no slice of
their own; recorded so the build state is not a mystery. Issue work: **#79** (matcher B2 minors), **#11**
(the RustCrypto stacks converged once the unifying majors landed — the earlier "still blocked" reading had
probed our own `Cargo.lock`, which can never show a new upstream major; residue
[#317](https://github.com/cairn-ehr/cairn-ehr/issues/317)), **#100** (`matcher_version` pins the full
effective config, not only weights), **#119** (`chart_trust` severity→label mapping lives once, at
emission), **#120** (shared `cairn-node` integration-test scaffolding). Loop-mechanism fixes: PRs #316,
#321 (a headless worker dies at turn end, so a successful cycle was being counted as a failure), #325.
**Still open:** [#312](https://github.com/cairn-ehr/cairn-ehr/issues/312) (triage never re-checks
`loop:ready`), [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314),
[#315](https://github.com/cairn-ehr/cairn-ehr/issues/315),
[#322](https://github.com/cairn-ehr/cairn-ehr/issues/322),
[#326](https://github.com/cairn-ehr/cairn-ehr/issues/326) (the worker's CI-wait idiom is dead — cycles
complete by tight polling), [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327).

**Slice 60 — ADR-0056 decision 5: the residual refusal contract, clinical plane (2026-08-01; branch
`fix/adr-0056-residual-refusal-contract`; closes [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267)
and [#270](https://github.com/cairn-ehr/cairn-ehr/issues/270), tests
[#269](https://github.com/cairn-ehr/cairn-ehr/issues/269); no spec/ADR change, no schema change, no
`SCHEMA_GENERATION` bump).** Slice 58 removed the *unknown-type* refusal at the door; what remained in the
puller's error arm was the genuine residual class — unenrolled/revoked signer, malformed envelope, oversize,
`t_effective` past the ceiling, unlawful contributor shapes. For those, §6.3's promise ("quarantined
*verbatim* by digest … the refusal is answered legibly") was false: the puller **persisted nothing**, froze
its cursor, and **exited SUCCESS**, so a peer link wedged behind one bad author's event was indistinguishable
from a healthy one — and the backlog grew silently every cycle.

The enabling fact was one line of lost information: `apply_signed` flattened `postgres::Error` into a
`String`, discarding the SQLSTATE, so the puller could not tell a deliberate `RAISE EXCEPTION` (`P0001`,
every db/020 refusal) from a transient fault. `ApplyError` keeps both the legible message and the code; two
pure predicates carry the decisions and are unit-tested with no database. A penned event that later applies
**auto-releases**, so the pen never duplicates `event_log`.

**Lessons worth carrying:**

1. **A refusal that persists nothing is a refusal you cannot audit.** The evidence lived only in a stderr
   line on a machine nobody was watching. The fix is not more logging — it is the *same* durable mechanism
   the unverifiable class already had (pen verbatim by digest, pin the re-offer floor, dedupe on re-offer).
2. **One contract per door, not one per refusal class.** A deliberate refusal now takes the unverifiable
   path exactly: penned, pinned, cursor still advancing so *other* authors' events keep flowing (principle 5).
   The freeze arm survives only where retrying the same bytes is correct: a transient fault. Both are loud.
3. **Symmetry between planes is a hypothesis, not a goal.** #268 asks the node plane to match, and the
   naive alignment would be a defect: `stream_node_events` serves every row, so refusing non-peers' events
   is that plane's routine *scoping*, not a refusal of history. Penning it would hold the loud signal on
   permanently (alarm fatigue — what ADR-0009 forbids). Prerequisite: a refusal-class partition in `db/007`;
   analysis on the issue, still `loop:blocked`.
4. **A message assembled from unconditional clauses will eventually lie.** The loud text always rendered its
   counts and its "preserved verbatim in sync_quarantine" tail — so a freeze-only cycle announced "0
   unverifiable and 0 floor-refused event(s)" and pointed the operator at an empty pen, and a quota freeze
   claimed "it clears by itself" two sentences after saying it needed a human ack. Both were reachable from
   an existing green test that asserted only the word `"quota"`. Every clause is now conditional on the
   state that makes it true. **On the operator path the message IS the product — assert what a message must
   NOT say, not only what it must.**
5. **`P0001` is a verdict-vs-fault test, not permanent-vs-transient.** One member of the deliberate class is
   ordering-transient: an overlay whose target is still in flight from another link. It is now penned and
   keeps the cycle loud until the target lands, then auto-releases — noisier than the freeze it replaced,
   accepted because the alternative wedges the link for every *other* author; cost named in §6.3.

**Review round (same day, PR #330).** Five findings, all fixed in place. It also added the end-to-end
freeze-arm test the first draft had declined (swap the apply door for one raising `40001` — deterministic,
and `locked_client` re-applies the schema, so it cannot leak), scoped the auto-release comment's over-claim
about `acked` rows, and stopped a failed pen-row release from aborting the cycle as a `partition`.

Paper-parity: not clinical-surface — this changes only what a node does with bytes its own floor refused;
no human act changes at any layer and no runnable clinical surface is exposed.

**Slice 61 — the med-list node tier: Cairn's first clinical READ path + whole-list sign-off (2026-08-02;
branch `feat/med-list-ui-slice-288`; Tasks 1–4 of a 12-task plan; owes
[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288); no spec/ADR change, no schema change, no
`SCHEMA_GENERATION` bump).** Every slice before this one *authored* events; nothing read clinical content
back out in Rust. Four pieces: `crates/cairn-medication-view` (pure — no DB driver, no GUI toolkit — holding
the read model and the single definition of what a sign-off gesture attests), `cairn-node`'s
`medication/read.rs` (seven small statements over the db/031–035 projections, assembled in Rust rather than
one two-level-aggregate join, so each is checkable against its view), `medication/signoff.rs` (N per-thread
attestations behind ONE unseal and ONE transaction — the #288 gesture), and the `medication-list` /
`medication-sign-off` CLI verbs.

**The design turn, and it came from the clinician.** The first draft had sign-off attest the whole list.
The correction: the paper counterpart is the **drug chart**, where every line carries the signature of
whoever is responsible for *that* drug — not a med-rec form signed once at the bottom. So a gesture signs
only threads whose vouch is **absent or stale**, and another clinician's current signature is never
silently reassigned to you. Ceased lines stay visible (a struck line stays on the paper chart) and are
never re-signed.

**Three lessons worth carrying:**

1. **Group/thread asymmetry is the defect-prone seam.** ADR-0047 collapses reconciled duplicates into one
   displayed row; ADR-0049 attests per thread. Nearly every defect this build surfaced lived there. The
   shared crate exists so the node and the future UI cannot answer *"what is about to be signed?"*
   differently — a divergence puts a green "signed" badge over a thread nobody signed.
2. **Refuse the chart when a line is missing; withhold the line when it is present but untrustworthy.** A
   reconciled group whose members span two patients displays on one chart only (db/033 joins a
   `DISTINCT ON (group_id)` display view against a per-`(group, patient)` status view). The losing patient's
   chart silently dropped a real drug and sign-off answered "nothing to sign off"; the winner's chart showed
   it twice, and its dose comes from a whole-group `DISTINCT ON` that ignores patient, so it can display the
   *other* patient's dose. The read path now dedupes by group, names what it cannot show
   (`groups_missing_from_chart`), **refuses** to sign an incomplete chart, and **withholds and reports** a
   cross-patient line rather than signing it. Refusing the whole chart in the second case was rejected
   deliberately: blocking eleven sound drugs over a twelfth suspect one is slower than paper, which §1.2
   forbids. The underlying view defect is [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334).
   Whether the *first* half of that asymmetry survives is now itself an open question —
   [#339](https://github.com/cairn-ehr/cairn-ehr/issues/339), below.
3. **A named remedy must name its arguments** (PR-review round 3). All three cross-patient warnings told the
   operator to run `medication-separate` — which takes two THREAD ids — while printing only a GROUP id, and
   the losing patient's own thread appeared on no surface at all (their chart is empty, and the vouch read is
   patient-scoped). The one exit from a hard refusal was therefore reachable only by raw SQL. The read model
   now carries `separation_targets` (each hazardous group's FULL membership, deliberately including the other
   patient's thread — a bare id with no clinical content, the minimum needed to repair a wrong-chart link the
   node is itself complaining about), and one `SEPARATION_INSTRUCTION` const + one renderer serve all three
   call sites so the repair advice cannot drift between them. **Generalised: a safety refusal is only as good
   as the escape hatch it names.**

**Deliberately NOT done.** No UI, so the §1.2 *time* budget stays unmeasured — the plan's benchmark (paper
N=3 human acts → architecture-forced M=1 → UI-bundled K=1 for review-and-sign) is owed by Task 10. Open:
[#331](https://github.com/cairn-ehr/cairn-ehr/issues/331) (a "nil medications, reviewed" act has no home),
[#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) (two safety branches need a test-only injection
seam), [#335](https://github.com/cairn-ehr/cairn-ehr/issues/335) (the two-read compare is best-effort at
READ COMMITTED, not an isolation guarantee), [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336)
(O(all medications) per chart open), [#337](https://github.com/cairn-ehr/cairn-ehr/issues/337) (byte-order
sort clusters capitalised brands above lowercase generics),
[#339](https://github.com/cairn-ehr/cairn-ehr/issues/339) (refusing the WHOLE chart over one invisible line
argues from whole-list semantics the per-line design abandoned — needs a clinician call; the cost is pinned
by a test), [#340](https://github.com/cairn-ehr/cairn-ehr/issues/340) (three near-identical medication
TRUNCATE lists in the test scaffolding, not interchangeable, no guard). **Tasks 5–12** (retire the superseded `cairn-gui`
iced workspace · data port · view model · db/044 gesture timing · Tauri backend · webview · measurement ·
docs) are handed to a fresh session with the plan file, not abandoned.

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
