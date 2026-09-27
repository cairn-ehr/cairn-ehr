# ROADMAP — Cairn

> **Disposable working scaffolding, not a source of truth.** The canonical *what* is the
> [spec](spec/index.md); the *why* is the [ADR log](spec/decisions/README.md). This file only
> orders the build and logs what each slice built. If it disagrees with the canonical docs, the
> canonical docs win. **Keep it under 500 lines** (#368): condense a slice once it is behind us —
> its *why* belongs in its ADR — but **never drop an open issue number while condensing** (the PR
> #271 review finding).

**Scope:** the **foundation** that must exist before the policy and GUI layers. Ordered bottom-up by
the four-layer model ([ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)):
**wire core → in-DB enforcement floor → sync → identity → security → federation → blobs → native
API**. Policy and UI sit *above* this line and are deliberately out of scope here.

## Cross-cutting (applies to every phase)

- **TDD** — failing test first, then code (load-bearing on the §9 safety-critical surface). **AGPL-3.0**
  for all code; every dependency AGPL-3.0-compatible (checked *before* adding).
- **Language by defect blast radius** ([§9](spec/language-substrate.md)) — safety-critical = Rust or
  in-DB (SQL/PL-pgSQL/pgrx), optimized for reviewer-legibility; advisory/cosmetic = fit-for-purpose
  (Python/ML). The integration boundary is the **PostgreSQL boundary** (≥ 18); avoid FFI coupling.
- Each phase takes the relevant **spike → production-grade**; close honest gaps, don't re-spike.

## Phase 0 — Proven foundations (done, as spikes)

- Event serialization + signatures — COSE_Sign1 + Ed25519 + SHA-256 ([ADR-0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)); `cairn-event`, Bet A ✓. In-DB floor spiked — validated `submit_event` door + recall, holds against a hostile agent (Spike 0002, C1–C5 ✓); `db/001`–`008`, `cairn_pgx` verify.
- First federating node — admission/pairing/mTLS/set-union `node_event` sync ([ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)); `cairn-node`, floor ENFORCED proof. Walking skeleton + WAN sync + replication/failover PoC.

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
  identity algebra: `link` + the linkage projection (C1); the `match_proposal`→apply seam with a human-accepted
  door (C2) and auto-apply of the `auto_candidate` band (C2b); `dispute` + the chart trust-state projection (C3);
  `identify` + *unconfirmed* (C4); `repudiate` + the known-alias pool (C5, the first *suppressing* identity
  event). The confirmed/unconfirmed/under-review contract is COMPLETE.
- **§5.4 John-Doe subsystem** (slices 20, 26–30) — registration front door (A+B); photo evidence carrying the
  day-one §3.14 attachment-reference shape ([ADR-0042](spec/decisions/0042-concrete-attachment-reference-shape.md));
  marks/belongings/EMS-context text evidence; finishers; the `enroll-human` ceremony CLI.
- **§5.2 matcher, advisory tier** (slices 19, 21–25; Python only) — the alias-pool evidence pass; birth-year-range
  blocking + A/B toggle; administrative-sex scoring and the unconfirmed-chart REVIEW rule; the B3 eval mirror;
  supervised Fellegi–Sunter weight-learning (a PoC on small/synthetic data); compound blocking keys.
- **`clinical.medication` slices 1–5** (slices 30b–34, `db/031`–`db/035`) — the first clinical-content stream:
  assert/cease + the E1 reconciliation flag; the bitemporal dose timeline; cross-thread reconciliation as a
  *link* ([ADR-0047](spec/decisions/0047-medication-reconciliation-resolution.md)); the commitment-based
  attestation overlay ([ADR-0049](spec/decisions/0049-commitment-based-sign-off-currency.md)); per-field dose
  correction ([ADR-0050](spec/decisions/0050-dose-correction-per-field-patch.md)); twin-check registry
  ([ADR-0048](spec/decisions/0048-twin-check-registry-dispatch.md)).
- **Slice 35 — the P1 floor-hardening slice** (PR #219; no ADR/spec/SCHEMA change) — the ADR-0030
  hostile-enrolled-writer threat model re-run against the in-DB floor across eight issues
  (#187/#207/#194/#191/#192[+#177]/#190/#193/#195). [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220) remains.

**Still open from these slices** — enumerated in full (see the header rule).

- **Filed and open.** [#141](https://github.com/cairn-ehr/cairn-ehr/issues/141), [#163](https://github.com/cairn-ehr/cairn-ehr/issues/163), [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168), [#184](https://github.com/cairn-ehr/cairn-ehr/issues/184), [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220). Two that carry
  standing consequences: [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (cross-thread dose-correction suppression vector — needs a PK/design
  decision, so it cannot be picked up as routine tech debt) and [#172](https://github.com/cairn-ehr/cairn-ehr/issues/172) (the future actor-write doors —
  rotate-key/`supersede`, actor-event sync apply — must mirror BOTH enroll collision checks; ADR-0054
  makes this live work). #79 (B2 Minors) is matcher-side.
- **Identity C5+.** `reattribute` (§5.5 event-granular strike-through) **waits on a clinical-note surface**; a
  reversal / de-repudiation event; a chart-history VIEW rendering struck names (data already present); an
  accept-at-cap boundary test; the §5.2 coherence feedback loop; contamination cascade on dispute; person-level
  trust aggregation. The §5.12 push-alert is the non-structural John-Doe remainder.
- **Matcher (advisory tier).** A **large hand-crafted gold set** to re-run the learner for authoritative
  magnitudes; **full §7.5 matcher actor registration** (its contributor identity is a provenance string for
  now); **no recovery escrow for the sealed matcher key** (regenerable — a convenience gap); no background
  scheduler; locale comparator packs; the hub-tier duplicate sweep; a veto-aware scorer mode; fuzzy alias
  recognition + an `alias` blocking pass; near-window softening; variable cluster size / hard negatives in the
  generator; a `compare_address` comparator; a CLI sweep entry; the B3 mirror ignores the block cap.
- **Medication (slices 30b–34).** Automated reconciliation **detection** (human-driven *resolution* exists;
  fuzzy detection plus a Tier-A dictionary is the gap); a partially-attested-group read surface; a whole-list
  sign-off summary event; statement-level `started`-date correction and per-field merge across corrections of
  one point; a rendering-suppression overlay for `delete`; structured sig/frequency; a separate `route` field;
  prefer-INN display term.
- **Attachments (slice 26).** Bytes are local only — **cross-node fetch deferred**; the residual DO-UPDATE overwrites a caller-supplied `media_type` (benign).
- **Accepted risk with a named remedy.** The `enroll_actor` dual-mapping guard's TOCTOU window ([#166](https://github.com/cairn-ehr/cairn-ehr/issues/166), closed as *accepted*): the durable fix is a floor-level per-key guard in `db/004`.

**Slices 36–56 — condensed (2026-07-16 → 07-27; the 2026-07-15 whole-project review course, its
Priority-6 design queue, and the first medication-coding slices; full detail in git, the PRs and the
linked ADRs).** The review course is **fully closed**. What exists:

- **P2 sync-convergence integrity** (slices 36–40, PRs #221–#225) — the flagship A→B convergence test driving
  the real binaries over TCP (#199); the cairn-sync SCHEMA subset standing alone (#198); the clinical-plane
  `seq` cursor + periodic full sweep (#196, `db/036`); acked rows freed from the quarantine quota (#197);
  cairn-sync wire hygiene + the `node.superseded` apply arm (#202/#201).
- **P3 — both wire windows shut** (slices 41–43): **ADR-0051** contributor-role vocabulary floor (#203+#96);
  **[ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md)** born-sealed clinical bodies (#189+#92,
  `db/037`) — every clinical JSONB body sealed at write under a per-event DEK the node itself holds, plus a
  custody plane, both doors enforcing sealed⇒clinical, and a rung-3 shred CLI (an *erasability* substrate
  only until Slice 66 pinned custody to admission); **ADR-0053** per-write human authorship (#204) — human
  signs while the node seals, `cairn_authorship_bound` at the strict door.
- **P4/P5 process + tech debt** (slices 44–45, PRs #251/#253/#255) — the #188 schema-version downgrade guard
  in both loaders (repo-wide `SCHEMA_GENERATION` + fs-derived guard tests + the `SCHEMA_LOAD_LOCK` TOCTOU
  close); `scripts/run-db-sql-tests.sh` running the `db/tests/*.sql` mirrors in CI (#212); the registry
  `DO UPDATE` arm (#214); HANDOVER staleness (#215).
- **P6 design queue → five ADRs** (slices 46–50), all design-settled: **ADR-0054** actor-registry federation
  is admit-and-dispute (#205, closes #154 structurally); **ADR-0055** the chained trust-root document (#206);
  **ADR-0056** unknown event types admitted uninterpreted (#200 — the filed premise was *inverted*: the spec
  was right, the code was wrong); **ADR-0057** generic reprojection (#208, PRs #274/#278 — one registered
  apply fn per projection plus one dispatcher replacing ~15 per-type triggers, `cairn_replay_eligible` as the
  #265/#266 seam); **ADR-0058** the grade-gated `t_effective` ceiling (#216, PR #285 — a born `clock_grade`
  bounds the ceiling's rejecting power, closing a latent one-event sync-wedge DoS).
- **Matcher, advisory tier** (slices 51/53/54; Python only) — #209 `derive_thresholds` fails closed on an
  empty non-match set (no impostor ⇒ no safe auto anchor); #210 retracts proposals orphaned when a pair
  leaves the blocking universe; #211 the E3 four-gap batch; #290 eval consumers REPORT the repaired-pair count.
- **Slice 52 — the #217 paper-parity plan-section rule** — every clinical-surface slice plan carries a
  `## Paper-parity benchmark (§1.2)` section or a forced-rationale escape, enforced by a no-DB source guard
  and stated in CONTRIBUTING.md + house rule 7. First live entry: [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288).
- **Slices 55–56 — medication drug coding.** **ADR-0059** (design-only, spec v0.61) anchors drug identity on
  drugref's immortal `moiety_uuid` (INN is display, never key) as `substance.coding {system, code, display}`,
  **advisory + honest-degrading**. Slice 6a (PRs #297/#298, `db/041`, `SCHEMA_GENERATION` 40→41) shipped the
  inline shape: the `medication_coding_system` registry, a two-tier floor, `medication_coding` as its **own**
  projection table, the `(system, code)`-**pair** dup-key, and honest degradation proven **by construction** via
  a source guard that nothing under `db/`, `crates/` or `extensions/` references drugref executably. Sharpest
  review finding: `cairn_execute_shred` did not scrub `medication_coding`, so a shred reporting success left
  the drug's preferred name and immortal anchor readable beside `patient_id` (ADR-0005 rung-3 / #92(b)).

**Still open from slices 36–56** — enumerated in full (see the header rule).

- **Sync/convergence.** [#284](https://github.com/cairn-ehr/cairn-ehr/issues/284) (cairn-node's full SCHEMA list vs cairn-sync's subset staying consistent).
- **Born-sealed / erasure (ADR-0052 follow-ons).** [#230](https://github.com/cairn-ehr/cairn-ehr/issues/230), [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231), [#232](https://github.com/cairn-ehr/cairn-ehr/issues/232), [#233](https://github.com/cairn-ehr/cairn-ehr/issues/233), [#234](https://github.com/cairn-ehr/cairn-ehr/issues/234), [#235](https://github.com/cairn-ehr/cairn-ehr/issues/235),
  [#236](https://github.com/cairn-ehr/cairn-ehr/issues/236), [#237](https://github.com/cairn-ehr/cairn-ehr/issues/237). Two that carry standing
  consequences: **#231 (unwrap-cert kid pinning) landed as Slice 66**, so custody now follows admission
  and born-sealed is confidentiality-capable, not merely an erasability substrate — which also unblocks
  #232 part C (sequester); #232's parts **A/B shipped** (Slices 65/67, discharging #294) and the
  cross-cutting authority floor landed as Slice 68; **parts C and D are now DESIGNED — ADR-0065, spec v0.67 — and remain to BUILD** (#376 answered, #377 merged into it with its dependency direction reversed; rung 2 blocked as #496, chart-wide narrowing as #499, and rung 1's offline glass owed by #498).
- **Authorship (ADR-0053 follow-ons).** [#242](https://github.com/cairn-ehr/cairn-ehr/issues/242), [#243](https://github.com/cairn-ehr/cairn-ehr/issues/243), [#244](https://github.com/cairn-ehr/cairn-ehr/issues/244), [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245), [#247](https://github.com/cairn-ehr/cairn-ehr/issues/247).
  Standing notes: grading is **half-live until #245**; contributor-set authorship is **key-scoped**
  and does not survive key rotation (#247, which constrains #245); a `--author-as` event is *owned*
  under the ADR-0043 suppression gate where a device-signed equivalent was dismissable by anyone.
- **ADR-0054/0055/0056 code work (design-settled, none built).** ADR-0054: #94, the key-loss-ceremony ADR,
  the rotate-key local door. ADR-0055: [#257](https://github.com/cairn-ehr/cairn-ehr/issues/257), [#258](https://github.com/cairn-ehr/cairn-ehr/issues/258), [#259](https://github.com/cairn-ehr/cairn-ehr/issues/259), [#260](https://github.com/cairn-ehr/cairn-ehr/issues/260), [#261](https://github.com/cairn-ehr/cairn-ehr/issues/261).
  ADR-0056: [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (align the node-plane skip) — #265/#266/#267/#269/#270 are closed by Slices 58/60.
  **The posture triad:** the content plane admits-and-disputes (0054) *and* admits-and-defers (0056),
  while the code plane verifies-or-refuses (0055).
- **Reprojection (ADR-0057 follow-ons).** [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (the authoritative Pi5/NVMe same-rig re-run — the
  shipped Bet-B numbers are cross-rig), [#275](https://github.com/cairn-ehr/cairn-ehr/issues/275), [#276](https://github.com/cairn-ehr/cairn-ehr/issues/276), [#277](https://github.com/cairn-ehr/cairn-ehr/issues/277) (heal cannot re-derive `DO NOTHING` projections).
- **Trusted time (ADR-0058 deferred).** [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279), [#280](https://github.com/cairn-ehr/cairn-ehr/issues/280), [#281](https://github.com/cairn-ehr/cairn-ehr/issues/281), [#282](https://github.com/cairn-ehr/cairn-ehr/issues/282), [#283](https://github.com/cairn-ehr/cairn-ehr/issues/283). **Registry hygiene:** [#254](https://github.com/cairn-ehr/cairn-ehr/issues/254) — 8 twin-check registrations still use `DO NOTHING`; unify with the #214 arm or record why not (#276 is its at-scale sibling).
- **Deps.** #252 (`quick-xml` via `wayland-scanner`) — **closed** by retiring iced. Residual duplication: [#317](https://github.com/cairn-ehr/cairn-ehr/issues/317). Advisory gate: [#389](https://github.com/cairn-ehr/cairn-ehr/issues/389).
- **Medication/matcher.** [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (hub-scale sweep re-scoring cost), [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) (med-list sign-off as
  ONE gesture — node tier Slice 61, window Slice 62; what remains is the human **measurement**),
  [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) (the §5.9 safety projection carries the
  coding-derived drug class — **discharged by Slice 67**), [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334) (a reconciled
  group spanning two patients displayed on one chart only — repaired by R1 on PR #688, 2026-09-27;
  the PR body carries the close), [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331) / [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) / [#335](https://github.com/cairn-ehr/cairn-ehr/issues/335) / [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336) / [#337](https://github.com/cairn-ehr/cairn-ehr/issues/337) (Slice 61 follow-ons).

**Operational caveats that outlive these slices.** Pre-ADR-0051 event logs (old `role:"author"`-without-
actor_id, flat-string responsibility) and pre-ADR-0052 plaintext `clinical.*` bodies **REFUSE at db/020** —
**wipe dev/PoC rigs** (the replication-failover demo, the spike rigs), never sync them through. Pre-wire
unsigned actor rows never sync. Test DBs need `cairn_pgx` ≥ 0.3.0.

**Slices 57–69 and the August tech-debt passes — condensed (2026-07-28 → 08-22; condensed again
2026-09-27; full detail in git, the PRs and the linked ADRs — the *why* is in each ADR and is not restated here).**

- **57 — `clinical.medication` 6b: the coding-overlay event types** (completes
  [ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) decision 3; `db/042`). Coding
  is a separately-authored act; a **strike NULLs the anchor** (*"not that, and I don't know"*, principle 4).
  Closed #295, #296. Lessons: a redundant projection column is a convergence hazard; nullable-widening means
  re-reading every aggregate over it (`array_agg` KEEPS NULLs). Open: the coded↔uncoded duplicate case; #294;
  [#300](https://github.com/cairn-ehr/cairn-ehr/issues/300).
- **58 — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power** (PR #302; closes #265, #266).
  An unknown `event_type` used to be refused and never stored; now admitted verbatim, projecting and conferring
  nothing, with `cairn_readjudicate_deferred` (db/043). *Refusal hides, admission cannot; the fix is neutrality,
  not strictness.* Open: [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301) (the node/actor plane still
  fail-closes), [#308](https://github.com/cairn-ehr/cairn-ehr/issues/308), [#309](https://github.com/cairn-ehr/cairn-ehr/issues/309).
- **59 — floor determinism** (PR #311 closes #75). The §3.13 twin blank-test was collation-dependent — the same
  signed event could apply on one node and raise on another; `cairn_twin_is_present` spells the 25 Unicode
  `White_Space` points. *A "merely cosmetic" asymmetry between two implementations of one predicate is worth
  measuring before it is filed as benign.*
- **Interlude — the loop ran unattended (07-31 → 08-01).** Nine PRs; closed #79, #11, #100, #119, #120; loop
  fixes PRs #316, #321, #325. Open: [#312](https://github.com/cairn-ehr/cairn-ehr/issues/312),
  [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314), [#315](https://github.com/cairn-ehr/cairn-ehr/issues/315),
  [#317](https://github.com/cairn-ehr/cairn-ehr/issues/317), [#322](https://github.com/cairn-ehr/cairn-ehr/issues/322),
  [#326](https://github.com/cairn-ehr/cairn-ehr/issues/326), [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327).
- **60 — the residual refusal contract, clinical plane** (closes #267/#270). A deliberate floor refusal on
  verifiable bytes persisted nothing, froze the cursor and exited SUCCESS; now penned by digest and
  auto-released. **`P0001` is a contract with the pull loop** (deliberate → skip and re-offer; anything else →
  freeze); PR #371 fixed the node plane and #370 the clinical one. *Symmetry between two planes is a hypothesis,
  not a goal* — the naive [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) alignment would be a defect.
- **61+62 — the med-list node tier and WINDOW** ([ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md),
  v0.62). Cairn's first clinical READ path; the iced layer retired for `cairn-gui-tauri`; node-tier write cost
  median **222 ms**; sign-off per LINE (the #339 clinician override — *the saline must still be giveable beside
  an unsigned potassium minibag*), reaching the transaction layer (#342). Lessons: **a unit-tested safety
  control can still be defeated by the surface that calls it**; **a compensating control outside CI is not a
  control** (the `gui` job, still not REQUIRED — #444). Owes [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288).
  Open: [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331) · [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332) ·
  [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) · [#335](https://github.com/cairn-ehr/cairn-ehr/issues/335) ·
  [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336) · [#337](https://github.com/cairn-ehr/cairn-ehr/issues/337) ·
  [#340](https://github.com/cairn-ehr/cairn-ehr/issues/340).
- **63 — the search-before-create funnel** ([ADR-0061](spec/decisions/0061-registration-is-an-act-that-carries-its-search.md),
  v0.63, `db/045`/`db/046`). Registration carries its search; **the attestation NAMES the displayed candidates
  rather than counting them**. Open: #346–#357, #359–#362; worth naming **#349**, **#351**, **#352**, and the §1.2
  write-cost half **#360**.
- **64 — closing the funnel's bypass** (closes #345). db/005 step 8b: a chart's first `patient_id`-bearing event
  must be its registration; retiring `patient.created` was the load-bearing half (*an "unless" in a safety floor
  is where the next defect lives*). Unfloored: [#364](https://github.com/cairn-ehr/cairn-ehr/issues/364),
  [#365](https://github.com/cairn-ehr/cairn-ehr/issues/365).
- **65 — the §5.9 sensitivity stream, part A** ([ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
  v0.64, `db/048`). Effective grade = the **max** of event/thread/chart; unknown ranks MAX; the grade is
  node-relative; erratum E6. Follow-ons open: [#374](https://github.com/cairn-ehr/cairn-ehr/issues/374),
  [#378](https://github.com/cairn-ehr/cairn-ehr/issues/378), [#379](https://github.com/cairn-ehr/cairn-ehr/issues/379),
  **#436**; parts C/D **#376**/**#377** (ADR-0065, above).
- **66 — custody follows admission** (closes #231; ADR-0052 E1). The unwrap-cert `kid` is pinned to `trust_peer`;
  **withhold the key, never the bytes**. `unsound = "all"` in both `deny.toml` trees; one advisory ignored with a
  reason, **#389**, whose review date lives in the reason (`advisory_ignore_review_dates.rs`).
- **67 — the §5.9 safety projection, part B** (closes #375; [ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md),
  v0.65). **The seal boundary is the coarsening boundary**; read coarsening is a rendering choice, not a floor;
  `safety_class_map` ships EMPTY (the drugref seam). Fixed #404. Open: [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394),
  [#395](https://github.com/cairn-ehr/cairn-ehr/issues/395), #397 · #398 · #399 · #400 · #401 · [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402),
  [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406), [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407).
- **68 + two interludes** ([ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md), v0.66; closes
  #380, #412, #405, #426). **Claim authority at the apply door**: one predicate `cairn_claim_authority`, one site;
  it gates effect, never admission (*flag what cannot self-heal; view what can*); 7 of 11 production mutations
  had survived a green suite. **One §5.9 leak closed, one narrowed** — each *a guarantee stated in a comment the
  code did not provide*: a column `REVOKE` cannot narrow a table `GRANT` (db/049 §8's 23-column grant —
  cost-raising, not a floor: [#425](https://github.com/cairn-ehr/cairn-ehr/issues/425),
  [#427](https://github.com/cairn-ehr/cairn-ehr/issues/427), [#432](https://github.com/cairn-ehr/cairn-ehr/issues/432));
  `classify_authorship_confidence` graded a forgery `Attested` (now `VerifiedKid`;
  [#428](https://github.com/cairn-ehr/cairn-ehr/issues/428)). **The `search_path` that pinned nothing**: 21 headers
  gained `, pg_temp` after both write doors returned SUCCESS into a decoy temp table. Open:
  [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408), [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409),
  [#413](https://github.com/cairn-ehr/cairn-ehr/issues/413), [#414](https://github.com/cairn-ehr/cairn-ehr/issues/414),
  [#415](https://github.com/cairn-ehr/cairn-ehr/issues/415) (expect it to fire on routine care),
  [#416](https://github.com/cairn-ehr/cairn-ehr/issues/416), [#417](https://github.com/cairn-ehr/cairn-ehr/issues/417),
  [#418](https://github.com/cairn-ehr/cairn-ehr/issues/418), [#419](https://github.com/cairn-ehr/cairn-ehr/issues/419),
  [#420](https://github.com/cairn-ehr/cairn-ehr/issues/420), [#422](https://github.com/cairn-ehr/cairn-ehr/issues/422),
  [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430) (~100 unpinned invoker-rights functions),
  [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431).
- **69 + two follow-on passes — the §5.9 operator surface** (closes #388, #383, #421, #435, #387, #439, #382, #385,
  #381). `patient-sensitivity <chart>` reports the worklist, deferred events, what a custody-thin node cannot
  anchor, and the **measured** count of sealed events held without custody — **NAME, NEVER COUNT**; one header per
  worklist arm; `readback.rs` never merges accountability with effect (`TargetState::OnAnotherChart` must never
  collapse; residual **#436**). The trap-clearing pass made `cargo doc` blocking (#439), asserted the REVOKE
  convention over `pg_proc.proacl` (#382 — a NULL ACL is the PERMISSIVE case), and found §10b's thread-free list
  safety-critical in the DISCLOSURE direction (#385). Residual: [#441](https://github.com/cairn-ehr/cairn-ehr/issues/441).
- **Two tech-debt passes — the silent gates (08-20 → 08-21; closes #446, #442, #443, #449–#453, #386; opens
  [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447)).** Nine gates that could pass without running:
  `cargo_lockfiles_tracked.rs` asks CARGO which manifests own a lockfile (`packaging/crates` had been erroring
  unseen); `db_gate_actually_ran.rs` fails CLOSED over a list derived from the test sources
  (`CAIRN_ALLOW_DB_SKIP=1` opts out); PostgreSQL checks a function inside a VIEW against the INVOKING user.
  Deliberately not done: unifying the 342 bare skip sites (#327).
- **The freeze that hid and the flake that lied (08-21; closes #370, #457; opens [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458)).**
  `cairn_learn_attachment_refs` had **nine** freeze paths and four silent ones on malformed signed bodies — a
  signature proves the bytes are the author's, not that the payload is well formed; the accessors refuse what
  already FAILED or was silently WRONG and accept what already worked. The readiness harness now watches the
  CHILD as well as the port (captures stderr to a file, never a pipe); the flake's cause is named, not fixed.
- **The db-error legibility sweep, four passes (08-22; closes #460, #465, #467, #469, #471, #473, #474, #475;
  `db/050`, SCHEMA 49 → 50; opens [#461](https://github.com/cairn-ehr/cairn-ehr/issues/461),
  [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463), [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464),
  [#468](https://github.com/cairn-ehr/cairn-ehr/issues/468), [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470);
  no ADR — that is the finding).** **An envelope-level field is constrained where it is MINTED and read
  permissively where it ARRIVES** (ADR-0063's rule; blast radius, not category — the #342 trap). Peer text is not
  display text; a flag can be born on a re-apply; a failed read reports `null`, never `0`.
  **`tokio_postgres::Error`'s `Display` IS `"db error"`** and `anyhow!("…: {e}")` discards the source —
  `db_diagnosis`/`legible_db_error` render `message [SQLSTATE] — DETAIL — HINT`; **`LocalDbFault` is not a
  rendering — never "tidy" it into an `anyhow!`**; a frozen cursor looked like a healthy cycle (`PullStats.frozen`).
  **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** (57014). To force a write failure in a SHARED test
  DB, take a LOCK from a second connection under a short `lock_timeout` — never a trigger or `REVOKE`; `Debug`
  must delegate to `Display` on any error that can reach `main`.

**Open-issue index — every open number the narrative above does not name.** The convention is *never drop
an open issue number* (the PR #271 review finding); prose cannot hold ~145 of them, and this index only
guarantees nothing is orphaned here. Live list: `gh issue list --state open`.
#93 #98 #283 — spec/ADR debt (revocation cascade trusts the authoring node's clock; ADR-0016 vs ADR-0001
and the node-compromise threat model; the `clock_grade` still unrendered in the legibility twin). #97 #347 #348 #353 #354 #355 #356 #361 — demographics + Slice 63 follow-ons.
#101 — sync/blob; **item 1 CLOSED by DR slice 2b** — the issue's title says "paginate `EventsAfter`", but it is `EventsAfterSeq` that gained paging: the legacy `EventsAfter` arm is unpaginated **by construction** and stays that way, and it is one of the two paths that can still reach the 64 MiB frame cap. Items 2 (blob `byte_len` wedge) and 3 (BLAKE3 verify in-DB) keep #101 open. #318 #329 #373 #411 #531 #532 #534–#538 — test + tooling hygiene (`cairn-sync/src/main.rs`: #329 filed it at 5.3k, #531 at 11.6k, and it has grown again since — read the file, do not quote a figure that is stale by the end of the slice that wrote it; no frozen wire fixture across crypto-library bumps; `run-db-sql-tests.sh` wrong-cluster + `dropdb` before the #169 marker check; a multi-page pull that fails late reports nothing about the pages that landed).
#458 — a non-object attachment element admitted silently (re-scoped 2026-08-22 to the #460 ledger + a loud
UI, NOT a floor rule). #392 #393 — federation (`peer_pubkey` hex case; custody grants leave no audit trail).
#303 #304 #305 #306 — the tech-debt loop's own tooling, stopped.
#602 — any client can set `cairn.remote_apply` before `submit_event`, turning the strict door's refusals into flags (a decision; found in #584's review, see the ADR-0070 entry under Phase 8/9).
#603 — a late key racing connect-time re-adjudication of a deferred event can leave the promoted record off the chart, and `requeue` then exits 0 (a cross-transaction race; ADR-0070 residual).
#604 — a shred racing a late key can resurrect custody, and since ADR-0070 the projection too; step 9's anti-resurrection check takes no lock (pre-existing, widened; ADR-0070 residual).

**2026-08-23 → 09-12 — the sweep's tail, the misclassification cluster, §5.9 C+D designed, the DR audit,
DR slices 1 → 2d, the restore's §1.2, #511, the closing-keyword guard, #527 and the CodeQL pack —
condensed 2026-09-27.** The *why* of each lives in its ADR (0065–0068) and design doc under
`docs/superpowers/`; HANDOVER's traps 1–8 hold the rules that still bite. Every open number is kept.

- **The sweep's tail (08-23; closes [#481](https://github.com/cairn-ehr/cairn-ehr/issues/481), [#479](https://github.com/cairn-ehr/cairn-ehr/issues/479), [#477](https://github.com/cairn-ehr/cairn-ehr/issues/477)).**
  A guard only runs when its own crate is tested (#450's DB-skip guard moved to a `#[path]`-shared
  `db_gate.rs`); the run loop's `db error` now flows through one pure `operator_chain`; the §5.7 auto-apply
  ceremony guarded as a subsystem. Opened [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485)
  (89 postgres call sites naming no operation) and, in review, [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487)–[#492](https://github.com/cairn-ehr/cairn-ehr/issues/492).
- **The misclassification cluster (08-23; closes [#489](https://github.com/cairn-ehr/cairn-ehr/issues/489), [#482](https://github.com/cairn-ehr/cairn-ehr/issues/482), [#480](https://github.com/cairn-ehr/cairn-ehr/issues/480), #490 items 1–2).**
  One species: *a failure wearing another subsystem's clothes* — `PullIntegrityError`,
  `PullFailureClass::Integrity` (a type outranks a kind), `apply_failure_is_local` (SQLSTATE-class split),
  `LocalDbFault` preserving the SQLSTATE. `pull_peer_integrity.rs` drives all six sites the real way. Still
  open: **#490** item 3 · **#483** · **#484** · **#487** · **#488** · **#491** · **#492** · **#485** · **#476**.
- **§5.9 parts C+D designed — *narrow the custody, never the reach*** ([ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md),
  v0.67; answers [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376), merges [#377](https://github.com/cairn-ehr/cairn-ehr/issues/377);
  design-only). The ladder, the keyring-is-local finding and C1's scope are in HANDOVER's §5.9 paragraph.
  Opened [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494) (ADR-0052's `event_dek` sentence vs the
  built table — an erratum), [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495), [#496](https://github.com/cairn-ehr/cairn-ehr/issues/496),
  **#498**, **#499**. Three generalisations: *a control that a faithful peer defeats by computing correctly is
  incoherent* (registry divergence, not thread resolution, is the real leak); *"conservative" is a property of
  a direction, not a value*; *refuse at a door only what that door can drop whole*.
- **The DR-guarantee audit (08-23; confirms #495, opens [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) and [#502](https://github.com/cairn-ehr/cairn-ehr/issues/502)).**
  ADR-0026 decision 1's three clinical promises were all false — a backup carried zero clinical records and
  the derived unwrap key died with the disk — while **every surface reported honestly and the composite was a
  precise untruth**. `dr_clinical_guarantee_gap.rs` pinned the defect, not the promise (inverted by 2d).
  **A deferral is honest only while its precondition holds** — ADR-0052 made `localstate.rs`'s *"no clinical
  surface yet"* false and nothing reopened it.
- **DR slice 1 — identity dies with the disk; custody must not** ([ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md),
  v0.68; closes #495). The unwrap key is an **independent X25519 keypair** in its own `<key>.unwrap`; adoption
  re-derives once (trap 1); custody and the surviving `event_dek` rows ride `CAIRNL1`, a shredded event's key
  never does; `restore` adopts, never mints; registering is provisioning (trap 2). Opened, still open:
  [#504](https://github.com/cairn-ehr/cairn-ehr/issues/504) (dead `_node_sk` — a decision) ·
  [#505](https://github.com/cairn-ehr/cairn-ehr/issues/505) (the migration mints a second recovery code; the
  open half is an opt-in single-code migration) · [#506](https://github.com/cairn-ehr/cairn-ehr/issues/506) ·
  [#507](https://github.com/cairn-ehr/cairn-ehr/issues/507) · [#508](https://github.com/cairn-ehr/cairn-ehr/issues/508)
  (CBOR leaves unwiped copies of the unwrap secret — a container decision) · [#509](https://github.com/cairn-ehr/cairn-ehr/issues/509) ·
  [#512](https://github.com/cairn-ehr/cairn-ehr/issues/512) (the `M > N` paper-parity defect) ·
  [#513](https://github.com/cairn-ehr/cairn-ehr/issues/513).
- **#503 — `cairn-keystore`: `cairn-sync` loads the node's custody key (08-30; closes [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503)).**
  `CAIRNK1` moved verbatim into its own crate; `cairn-sync` resolves its key once, through a pure decision
  table (trap 3). Opened: [#514](https://github.com/cairn-ehr/cairn-ehr/issues/514) (retire the fallback) ·
  [#515](https://github.com/cairn-ehr/cairn-ehr/issues/515) (the two binaries disagree on the signing-key file
  format) · [#516](https://github.com/cairn-ehr/cairn-ehr/issues/516) · [#517](https://github.com/cairn-ehr/cairn-ehr/issues/517)
  (no test starts the daemon from a PROVISIONED key) · [#518](https://github.com/cairn-ehr/cairn-ehr/issues/518) ·
  #520 · [#521](https://github.com/cairn-ehr/cairn-ehr/issues/521). Lessons: **breakage hid from a gate three
  ways in one slice** (fail-fast, `| tail`, a cross-crate suite) → `scripts/run-db-gated-tests.sh`; **four
  defects were in the task briefs** → run a plan's verbatim code against the project's own gates first;
  **where no test carries a value across the disk, the one link that matters is proven by nothing**
  (`#[serde(default)]`).
- **DR 2a + 2b — the medium format and the transport seam (08-31 / 09-02; `cairn-medium`, `cairn-wire`;
  #101 item 1 only).** `CAIRNB3`: no head block, one global chain across both planes, torn tail ≠ corruption,
  `Plane::Unknown(tag)`; `Transport` as the one seam; paging commits cursor and floor per page. **19 of 19
  single-line mutations survived a round-trip suite** → `wire_pins.rs` golden bytes: *a round-trip test proves
  self-consistency, never correctness*. Open: [#531](https://github.com/cairn-ehr/cairn-ehr/issues/531) ·
  [#532](https://github.com/cairn-ehr/cairn-ehr/issues/532) · [#534](https://github.com/cairn-ehr/cairn-ehr/issues/534)
  (a freeze stops content convergence) · [#535](https://github.com/cairn-ehr/cairn-ehr/issues/535) ·
  [#536](https://github.com/cairn-ehr/cairn-ehr/issues/536) (an unopenable DEK is counted nowhere on sync) ·
  [#537](https://github.com/cairn-ehr/cairn-ehr/issues/537) · [#538](https://github.com/cairn-ehr/cairn-ehr/issues/538) ·
  [#556](https://github.com/cairn-ehr/cairn-ehr/issues/556) (`segment_commitment` does not bind the attestation
  — **free only until a release ships a CAIRNB3 writer**) · [#557](https://github.com/cairn-ehr/cairn-ehr/issues/557) ·
  [#558](https://github.com/cairn-ehr/cairn-ehr/issues/558) · [#559](https://github.com/cairn-ehr/cairn-ehr/issues/559)
  (operator messages naming the wrong remedy) · [#560](https://github.com/cairn-ehr/cairn-ehr/issues/560) ·
  [#561](https://github.com/cairn-ehr/cairn-ehr/issues/561) · [#562](https://github.com/cairn-ehr/cairn-ehr/issues/562) ·
  [#563](https://github.com/cairn-ehr/cairn-ehr/issues/563).
- **#511 — the custody newtypes (09-04; opens [#541](https://github.com/cairn-ehr/cairn-ehr/issues/541), [#543](https://github.com/cairn-ehr/cairn-ehr/issues/543), [#544](https://github.com/cairn-ehr/cairn-ehr/issues/544), [#545](https://github.com/cairn-ehr/cairn-ehr/issues/545)).**
  `Secret32` (zeroizing, redacting, constant-time) and `PublicKey32` make public-for-secret a compile error; they
  do NOT separate secret roles (trap 5). *A prose count with no mechanical pin is a stale count waiting to happen.*
- **The closing-keyword guard (09-04; reopens [#101](https://github.com/cairn-ehr/cairn-ehr/issues/101), [#115](https://github.com/cairn-ehr/cairn-ehr/issues/115), [#434](https://github.com/cairn-ehr/cairn-ehr/issues/434), [#441](https://github.com/cairn-ehr/cairn-ehr/issues/441), [#468](https://github.com/cairn-ehr/cairn-ehr/issues/468), #500, #534).**
  GitHub closes on ADJACENCY: a sentence denying that a PR repaired #500 was what shut #500. `scripts/check_closing_keywords.py` +
  the workflow scan title, body and commits. Residual **#444**, [#547](https://github.com/cairn-ehr/cairn-ehr/issues/547),
  [#548](https://github.com/cairn-ehr/cairn-ehr/issues/548). *The sentence written to prevent an over-claim was
  the instrument of the over-claim.*
- **DR slice 2c — the medium carries the clinical record (09-06; `db/051`, SCHEMA 50 → 51; closes
  [#524](https://github.com/cairn-ehr/cairn-ehr/issues/524), #500 as titled — its READ half became
  [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554) — and [#523](https://github.com/cairn-ehr/cairn-ehr/issues/523),
  whose merge also closed #182, [#404](https://github.com/cairn-ehr/cairn-ehr/issues/404),
  [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430), [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431)
  and #441 — the last three are open on GitHub again, as is [#522](https://github.com/cairn-ehr/cairn-ehr/issues/522);
  #550 opened and closed in-branch).** Five rulings that outlive it: **a backup reproduces the state at CAPTURE
  TIME** (trap 7); custody travels on both paths and **the shred predicate has ONE home** (`db/051`); **Postgres
  burns identity values before conflict arbitration**, so `seq` holes are routine
  ([#549](https://github.com/cairn-ehr/cairn-ehr/issues/549) owes the operator surface); **a torn tail must NOT
  refuse `restore`** (`verify-backup` still refuses); verify-before-write AND read-after-write. §1.2 was measured
  here. Open: [#551](https://github.com/cairn-ehr/cairn-ehr/issues/551) · [#552](https://github.com/cairn-ehr/cairn-ehr/issues/552)
  (a capture is O(whole medium)) · [#553](https://github.com/cairn-ehr/cairn-ehr/issues/553) (an unmarked foreign
  legacy medium can be destroyed by succession).
- **DR slice 2d — the record comes home** ([ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md),
  v0.69; `db/052`, SCHEMA 51 → 52; closes #554; opens [#567](https://github.com/cairn-ehr/cairn-ehr/issues/567)–[#571](https://github.com/cairn-ehr/cairn-ehr/issues/571)).
  **The headline test DECRYPTS a body** — the door wraps what it is handed, so piping a carried key through
  would double-wrap every key while every count agreed. The pen moves into the database (restore pens are
  unbounded), gains custody; `ActorRegistryRow::recorded_at` loses `#[serde(default)]` (a defaulted value would
  silently re-authorise a recalled actor); the no-export rule keys on CUSTODY; the gap pin **inverts** to
  `a_clinical_event_restores_from_a_medium`. **2e is retired as a label.** Still broken: #549, #552, #536, and
  peak read-side memory is unbudgeted.
- **The restore's §1.2, measured** ([ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md),
  v0.70 — provenance warns, never gates; closes #571; opens #572; PR [#573](https://github.com/cairn-ehr/cairn-ehr/pull/573)).
  **100 003 events restore in 116.7 s against 600 s; 85 000 sealed bodies open;** linear 1.17 ms/event, a ceiling
  near 510 000 (M3 Max). #552 confirmed: about two thirds of a nightly capture is independent of what is new.
  `M > N` stands (#512); the excess act is now the recovery code.
- **CodeQL advanced setup + a model pack (09-12; PR [#576](https://github.com/cairn-ehr/cairn-ehr/pull/576); corrects #562).**
  `rust/cleartext-logging` sources are NAME heuristics with no opt-out; one `barrierModel` row per function
  (`ReturnValue`, kind `log-injection`), each with its reason: 44 → 3. Default setup allows no tuning, and the
  organization-level configuration silently overrode the repository's. Tests stay scanned. Not touched:
  [#575](https://github.com/cairn-ehr/cairn-ehr/issues/575) (a real secret on stderr); making it required is #444.
- **#527 — a discriminator is not a salt (09-02; opens [#529](https://github.com/cairn-ehr/cairn-ehr/issues/529), [#530](https://github.com/cairn-ehr/cairn-ehr/issues/530)).**
  18 criticals were one per call site of two fixture helpers whose parameter was named `salt`: CodeQL picks its
  sink by the binding's NAME, and a derivation from literals is constant-folded through. Renamed to `lineage`,
  guarded by `crypto_sink_names_are_genuine.rs` (the inventory of the tree's real cryptography), house rule 6b
  added. Checking the guard's own prose found `PairingBundle.nonce` is signed and never read (#530 — make it
  real, rename it, or remove it: a decision). **A triage tool that drops the message turns a defect into
  noise.** #529: no daemon path prints a patient id — by accident, not by rule.

## Phase 5 — Security & compliance core

- **Erasure = key-custody redistribution / crypto-shred** on the severity ladder ([ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9).
- **Visibility-scope ≠ replication; the safety projection** — sealed bodies emit de-identified, severity-graded safety projection; sensitivity is a graded append-only stream ([ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)).
- **At-rest seal** — ✓ done (ADR-0026 **slice A**): signing key sealed with a dual-recipient envelope (Argon2id
  KEKs from an operational passphrase + a one-time off-node recovery code; XChaCha20-Poly1305), recovery escrow
  minted at `init`, `seal-key` migration.
- **Backup-as-cold-peer** — ✓ **COMPLETE** (ADR-0026 **slice B**): `backup`/`verify-backup` + `last_backup` status.
  Since **DR slice 2c** the medium is a CAIRNB3 image carrying **both planes**, and since **2d** a restore applies the
  clinical plane through the `db/020` door, installs the carried custody, and pens what it cannot key. So ADR-0026
  decision 1's *"the clinical event log survives"* and decision 2's *"clinical events back up as a cold peer"*
  **may now be cited as met** (#500 → [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554); pinned by
  `dr_clinical_guarantee_gap.rs::a_clinical_event_restores_from_a_medium` and `medium_carries_both_planes`).
  Since then, each under Phase 8/9: ADR-0069, #568, #567, #593 (all 23 design tests written), #584 (ADR-0070), #594
  (ADR-0071), #614/#615 (ADR-0072), #619 (ADR-0073), #621 (ADR-0074). **What remains is not slices:** #575 (a
  decision, re-deferred), #611, #602, the races #603/#604, #567's residuals #589 · #590 · #591 · #592, #593's
  residuals #596 · #597 · #598 · #599, PR #601's wave #605–#610, PR #612's round-3 wave #616 · #617, #608's
  late-custody half, and **node-plane completeness accounting** (a general "what did the node plane fail to apply"
  report is a slice of its own).
- **Restore-apply + new-identity `supersede`** — ✓ done at node level (ADR-0026 **slice C**, [issue #50](https://github.com/cairn-ehr/cairn-ehr/issues/50)):
  `cairn-node restore` rehydrates the `node_event` log through a self-trusting `restore_node_event` door
  (empty-genesis fenced), mints a fresh key, records a `supersede`(dead→new); `db/009`. **Cold-medium
  self-identification** ([#53](https://github.com/cairn-ehr/cairn-ehr/issues/53)): a container-level self-marker
  (`CAIRNB2`). **Live residual:** a peer's genuine marker spliced between **byte-identical converged** media is not
  rejectable — multi-enroll restores report `Provenance::SignedFederated` (warn, never gate — ADR-0068).
- **Sealed local-state export** — ✓ **CUSTODY travels** (ADR-0026 **slice D**, completed by DR slice 1 /
  [ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md)): `CAIRNL1` + a `CAIRNX1` `.lsk`
  sidecar carrying the **independent** unwrap secret, the surviving `event_dek` rows (a shredded event's excluded by
  construction) and, since 2c, the enrolled actor registry; verified after write; installed by 2d. ADR-0026 decision
  1's promise 2 ("node-default data-at-rest keys survive") **has no subject in the built system** — neither honoured
  nor violated. **Uniform key-material zeroization** ✓ ([#54](https://github.com/cairn-ehr/cairn-ehr/issues/54)),
  residual **#508**. Optional follow-on: escrow rungs (Shamir M-of-N, QR, TPM).
- **Trusted-time anchoring** — graded-interval `t_recorded` with clock-confidence grade; transparency-log multi-anchor existence proof ([ADR-0027](spec/decisions/0027-trusted-time-anchoring.md)).
- **Audit-log integrity, offline auth, mTLS** ([§7](spec/security.md)).

## Phase 6 — Federation hardening

- **Revocation cascade; anchor-as-power** ([ADR-0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md)).
- **DR / recovery escrow** — ADR-0026 slices A, C, D done at node level; **slice B is COMPLETE — the medium
  carries the clinical plane (2c) and a restore reads it back (2d, #554)**, see Phase 5. Federation-tier
  follow-ons: peer-quorum (social) recovery + escrow rungs
  (Shamir M-of-N, QR, TPM/keyring). Node-tier residual **#505** (the ADR-0066 migration path mints a second
  recovery code).
- **Node-identity `supersede`** — ✓ done (ADR-0026 slice C). **Signing-key rotation** (`rotate-key` actor event) — still reserved, not built.

## Phase 7 — Attachments / byte tier

- **Content-addressed lazy blobs** referenced by the signed event, never inlined; day-one attachment-reference shape ([ADR-0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)). **The concrete shape is FINALIZED** ([ADR-0042](spec/decisions/0042-concrete-attachment-reference-shape.md), 2026-07-08, slice 26): `Attachment{descriptor, renditions:[Rendition{…, inline?, seal?}]}` + `SealRef` in `cairn-event/src/attachment.rs` (all five §3.14 reserves; field order frozen), `EventBody.attachments: Vec<Attachment>`, and reference-eager per-rendition learning in both doors via the shared `cairn_learn_attachment_refs` helper (db/027; db/005 + db/020). Byte tier (db/003 + `cairn-sync` blobd) is chunked/resumable/windowed. First real consumer: §5.4 photo evidence (slice 26). *Deferred: cross-node byte fetch wired into `cairn-node`; per-blob DEK sealing; preview/extracted-text renditions.*
- **Blob self-verification in-DB floor** — ✓ done 2026-07-05 (`db/026_blob_verify_floor.sql` + `cairn_pgx` 0.3.0
  `cairn_blob_verify`/`cairn_blob_verify_error`, thin wrappers over the same `cairn_event::blob_address` L2 uses —
  one hashing implementation, never two): the BLAKE3-vs-address check `cairn-sync` performs before flipping
  `present := TRUE` is restated **in-DB** as a trigger floor on `blob_store`, closing the honest gap db/003 carried
  since the walking skeleton — a raw-SQL client could store arbitrary bytes as any named blob (principle 12
  requires the floor below every client). Stale-`.so` legibility is two-layered: db/026's `to_regprocedure` load
  gate plus `cairn-sync`'s `REQUIRED_PGX_FLOOR` 0.3.0 connect gate. **Honest limits:** `blob_chunk` rows and
  `outboard` are NOT in-DB verified — wrong chunks can only assemble into a whole-blob flip that FAILS the floor
  (space waste, never wrong bytes served), and a wrong outboard yields slices the *fetching* peer's bao decode
  rejects against the signed address root (availability degradation, never an integrity hole).
- **Resource-isolated byte tier** — chunked/preemptible/separately-budgeted; can never starve clinical sync; opt-in byte replication; self-verifying swarm fetch.
- **Rendition set** — the binary's legibility twin (retrievability axis); per-blob DEK crypto-shred inherits.

## Phase 8 — Native API contract (the boundary below the application) · Phase 9 — Terminology

- **Native API: capability-described + conformance-tested, evolves additively** ([ADR-0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md)); the four-layer boundary sits *below* policy/UI ([ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)).
- **Author-scoped export** — the medico-legal copy ([ADR-0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md)). **FHIR interop façade** — distinct from the native API ([§9.7](spec/language-substrate.md)).
- **Phase 9 — ICD-11 canonical interlingua + local-terminology overlay** ([ADR-0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)).

**2026-09-11 → 09-20 — the DR close-out and the node-plane doors, condensed 2026-09-27.** Each ADR (0069–0074)
and plan (`docs/superpowers/plans/2026-09-1*`) carries the full argument and review ledger; HANDOVER's traps
8–14 carry the rules. Every open number is kept.

- **The restore takes its recovery code from a file** ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md),
  v0.71; closes #572 and [#570](https://github.com/cairn-ehr/cairn-ehr/issues/570); PR [#574](https://github.com/cairn-ehr/cairn-ehr/pull/574)).
  `--old-recovery-code-file <PATH>` — a path, never a flag value or env var; read in the step-0 pre-flight; a
  blank file refused. The refusal the design started with was dropped because #527/#562's *"no cron-run command
  reaches `print_recovery_code`"* was already false → **#575**. The restore CLI got its first tests; a fixture
  defect found on the way (the DR wipe left `actor_event`, so every registry-travels assertion proved less).
- **`requeue` releases custody, and something proves it (09-12; closes [#568](https://github.com/cairn-ehr/cairn-ehr/issues/568)).**
  Five DB-gated tests drive the shipped binary and assert a body OPENS. **Arms are indexed by input pair, not by
  outcome**; a mutation that cannot fail is not evidence. `do_requeue` is private to a binary crate (#531/#329).
- **`requeue` never counts a release it did not get (09-12; closes #578–#581; PR [#582](https://github.com/cairn-ehr/cairn-ehr/pull/582)).**
  A pen row carrying a wrapped DEK is released only when custody is SETTLED (trap 8);
  `cairn_custody_landed`/`cairn_custody_state`/`cairn_release_pen_row` went into the EXISTING `db/052` (replay on
  connect reaches every node; a new file would force a whole-tree rebuild); exit 3 = rows still held. Review
  filings: [#585](https://github.com/cairn-ehr/cairn-ehr/issues/585) (nothing reads Postgres notices) ·
  [#586](https://github.com/cairn-ehr/cairn-ehr/issues/586) (two older source guards skip after a file's first
  `#[cfg(test)] mod`) · [#587](https://github.com/cairn-ehr/cairn-ehr/issues/587) (`cairn-sync` does not replay
  schema). The gate failed on a predecessor's fixture: [#583](https://github.com/cairn-ehr/cairn-ehr/issues/583).
- **`verify-backup` asks the clinical-plane question (09-13/14; closes #567; PR [#588](https://github.com/cairn-ehr/cairn-ehr/pull/588)).**
  Fails `backup SHORT` only on evidence (maintainer decision): the node's own sidecar describes the path AND the
  medium's newest clinical seq is behind it or its raw count below it. Three asks not built, each for a pinned
  reason; order in the arm is load-bearing. Residuals [#551](https://github.com/cairn-ehr/cairn-ehr/issues/551) ·
  [#589](https://github.com/cairn-ehr/cairn-ehr/issues/589) · [#590](https://github.com/cairn-ehr/cairn-ehr/issues/590) ·
  [#591](https://github.com/cairn-ehr/cairn-ehr/issues/591) · [#592](https://github.com/cairn-ehr/cairn-ehr/issues/592);
  commented on #559 and #549. **Operators: run it AFTER `backup`.**
- **2d's last six design tests (09-14; closes [#593](https://github.com/cairn-ehr/cairn-ehr/issues/593) and, by a
  lockfile bump, #600; PR [#595](https://github.com/cairn-ehr/cairn-ehr/pull/595)).** Tests 7, 14, 16, 17, 19, 22,
  each shown red under a named mutation (thirteen run, thirteen killed across build and review). #600 was
  RUSTSEC-2026-0285 (`rustls`), bumped in BOTH lockfiles. Opened [#594](https://github.com/cairn-ehr/cairn-ehr/issues/594)
  and [#596](https://github.com/cairn-ehr/cairn-ehr/issues/596) · [#597](https://github.com/cairn-ehr/cairn-ehr/issues/597) ·
  [#598](https://github.com/cairn-ehr/cairn-ehr/issues/598) · [#599](https://github.com/cairn-ehr/cairn-ehr/issues/599).
- **A late key reaches the chart** ([ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md), v0.72;
  builds [#584](https://github.com/cairn-ehr/cairn-ehr/issues/584); PR [#601](https://github.com/cairn-ehr/cairn-ehr/pull/601)).
  Both doors call `cairn_project_late_custody` after the substitution guard; `reproject_owed` and its exit-3 cause
  are retired (traps 9/10). M1–M14 killed. Filed: [#602](https://github.com/cairn-ehr/cairn-ehr/issues/602) (any
  client can set `cairn.remote_apply` before `submit_event`) · the named races [#603](https://github.com/cairn-ehr/cairn-ehr/issues/603),
  [#604](https://github.com/cairn-ehr/cairn-ehr/issues/604) · the review wave [#605](https://github.com/cairn-ehr/cairn-ehr/issues/605)
  (an in-place `db/` edit is unprotected by the #188 downgrade guard) · [#606](https://github.com/cairn-ehr/cairn-ehr/issues/606) ·
  [#607](https://github.com/cairn-ehr/cairn-ehr/issues/607) · [#608](https://github.com/cairn-ehr/cairn-ehr/issues/608)
  (the substitution guards' `<>` fail-open) · [#609](https://github.com/cairn-ehr/cairn-ehr/issues/609) ·
  [#610](https://github.com/cairn-ehr/cairn-ehr/issues/610).
- **A restore that left records behind exits INCOMPLETE** ([ADR-0071](spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md),
  v0.73; builds #594; PR [#612](https://github.com/cairn-ehr/cairn-ehr/pull/612)). Five causes share exit 3; exit
  1 = BLOCKED, checked first (trap 11). Filed [#611](https://github.com/cairn-ehr/cairn-ehr/issues/611) ·
  [#613](https://github.com/cairn-ehr/cairn-ehr/issues/613) · [#614](https://github.com/cairn-ehr/cairn-ehr/issues/614) ·
  [#615](https://github.com/cairn-ehr/cairn-ehr/issues/615) · [#616](https://github.com/cairn-ehr/cairn-ehr/issues/616) ·
  [#617](https://github.com/cairn-ehr/cairn-ehr/issues/617). *A fix written under the pressure of a finding is
  itself unreviewed code; a truthful exit code does not make a false sentence true.*
- **A restore loses no record silently** ([ADR-0072](spec/decisions/0072-a-restore-loses-no-record-silently.md),
  v0.74, errata E1–E2; `db/053`, SCHEMA 52 → **53**; builds #614 + #615; PR [#618](https://github.com/cairn-ehr/cairn-ehr/pull/618)).
  One `cairn_refuse_substitution`, `IS DISTINCT FROM` (trap 12) — porting db/005's guard would have copied #608's
  fail-open a third time. Its census error became [#619](https://github.com/cairn-ehr/cairn-ehr/issues/619).
- **The node plane refuses a substitution and pens it** ([ADR-0073](spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md),
  v0.75; builds #619; PR [#623](https://github.com/cairn-ehr/cairn-ehr/pull/623)). Trap 13; sixteen mutations
  killed; the declared survivor M10 was killable via a `SET ROLE` seam — **"untestable" is a claim.** Filed
  [#620](https://github.com/cairn-ehr/cairn-ehr/issues/620) (the COSE unprotected header lies outside the signature
  — wire core, a decision) · [#621](https://github.com/cairn-ehr/cairn-ehr/issues/621) ·
  [#622](https://github.com/cairn-ehr/cairn-ehr/issues/622) (catalogue guards blind to `BEGIN ATOMIC`) ·
  [#624](https://github.com/cairn-ehr/cairn-ehr/issues/624) · [#625](https://github.com/cairn-ehr/cairn-ehr/issues/625).
  Left open, named: #605 · [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)'s other classes ·
  [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301) · [#569](https://github.com/cairn-ehr/cairn-ehr/issues/569) ·
  node-plane completeness accounting · #608's late-custody half.
- **A deterministic door failure is a refusal, not a fault** ([ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md),
  v0.76; builds #621). The three node doors are total; the puller pens a non-local non-`P0001` failure instead of
  freezing (trap 14). Fifteen mutations killed. Filed [#626](https://github.com/cairn-ehr/cairn-ehr/issues/626)
  (the clinical plane has the identical shape; kept out because db/020 is the 100k-event hot path) ·
  [#628](https://github.com/cairn-ehr/cairn-ehr/issues/628) (`cairn_body`'s `22P05` on a NUL) ·
  [#629](https://github.com/cairn-ehr/cairn-ehr/issues/629) · from the branch review [#631](https://github.com/cairn-ehr/cairn-ehr/issues/631) ·
  [#632](https://github.com/cairn-ehr/cairn-ehr/issues/632) · [#633](https://github.com/cairn-ehr/cairn-ehr/issues/633) ·
  [#634](https://github.com/cairn-ehr/cairn-ehr/issues/634) (#630 closed as a duplicate of #625).

### 2026-09-21 — patient search matches fragments, and then runs in under a second (#636 slice 1, #639)

**Slice 1** widened `db/046` pass 3 in the safe direction: stored punctuated tokens project their alphanumeric
PARTS, and a ≥3-**byte** prefix matches via `starts_with` (never `LIKE`, since an internal `%` survives the
query tokeniser). Monotonicity was **proved** (old `EXCEPT` new: 0 lost, 9 gained). Durable: the minimum gates
PREFIXES, never short NAMES; callsigns are excluded from both new arms; `patient_search_drift.rs` makes the
sweep-paired ⊆ search-found invariant executable. **#639** then found every search cost ~1500 ms on a Pi even
when it found nothing: an `OFFSET 0` fence, `UNION ALL` in the lateral and a skipped parts branch took the floor
**1528.4 → 883.3 ms** and the spread **777.9 → 14.6 ms** at 50,000 real names (traps 15–17; the rig is committed,
`scripts/measure_patient_search.py`). Two lessons: *a zero-row search measures nothing*, and *the neutrality claim
was false when first pushed* (U+0130). Filed [#637](https://github.com/cairn-ehr/cairn-ehr/issues/637) (the
materialised token table — the right fix for the remaining ~860 ms floor, a slice of its own) · #638 (closed:
the gate counts BYTES, culture-neutral) · [#640](https://github.com/cairn-ehr/cairn-ehr/issues/640) ·
[#641](https://github.com/cairn-ehr/cairn-ehr/issues/641) (the parts branch cuts a Devanagari or Thai name at
its marks) · [#643](https://github.com/cairn-ehr/cairn-ehr/issues/643) (the rig times `count(*)`). §1.2: paper
3 → forced 2 → target 2; §5.11's no-spinner limb now met.

### 2026-09-22 → 09-23 — the funnel UI, slices 2a → 2c (PRs #646, #653, #661, #674)

The §5.3/§5.8 search-before-create funnel as the window's front door. Design
`docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` (its dated notes record where each slice
met the code). No ADR, no migration.
- **2a — the pure core** (`cairn-gui-funnel`): the advisory step-3 **trigger** over ONE free name field
  (ADR-0014), the bounded **prompt** (`PROMPT_CAP = 5`), the counted-custody **token** (`AttestedSearch` has no
  public constructor, is not `Clone`, is consumed by `register`), two ports and a mock whose matching rule is NOT
  db/046's. 21/21 mutations killed.
- **2b — the live ports** (`cairn-gui-live`): `DataError::Refused` (#648, closed) — a `P0001` verdict is not an
  outage; both error arms restore the attestation; a derived truncate list that misses identity-stream tables
  (#658).
- **2c's prerequisites** (#661): `TokenStore::settle` (#659), `MockData::fail_next` (#660), `DeliberateRefusal`
  (#651), **one enrolment rule** (#654 — fifteen call sites, not one); `ActorStanding` is four states.
- **2c — the window** (#674). Planning found `search_patients` ordered by chart age over a disjunction, so the
  signed prompt showed the five OLDEST charts; it now ranks (maintainer decision), and truncation on 92% of
  registrations became [#671](https://github.com/cairn-ehr/cairn-ehr/issues/671) (decided 09-26). Built:
  `FunnelSession`, `LiveData::{sharing, require_provisioned, standing, today}`, the `funnel/` module, runbook §8;
  registration form = name + DOB (identifier entry → #672); the header shows age, not DOB (#673). Three review
  rounds: the Critical (a sign-off signed whichever chart was OPEN) → every chart command names the displayed
  chart; a registration never switches charts behind an open one; #675's four items fixed; #677 decided (the
  800 ms guard is soft policy).
- **Still open from the whole run:** [#355](https://github.com/cairn-ehr/cairn-ehr/issues/355) · #645 · #647 ·
  #649 (register's cancellation contract) · #650 · #652 (the P0001 rule's three homes) · #655 (`42501`/`42P01`/
  class-23 land in `Unavailable`) · #656 · #657 (multi-event rollback untested in both trees) · #658 · #662 (seven `init` effects unpinned) · #663 · #664 /
  #666 (what a superseded key classifies as) · #665 · #667 · #668 · #669 · #670 · #672 · #673 · #676. Also cited:
  #442, #450, #583, #636, #638.
- **§1.2:** register paper 5 → forced 4 → target 4 (3 when the prompt is empty); find 3 → 2 → 2. The stopwatch
  half (find ≤ 5 s, register ≤ 20 s) is a **human act**, runbook §8.

### 2026-09-26 — the step-3 prompt is a nudge, not a completeness claim (#671, ADR-0075, PR #678)

[ADR-0075](spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md), spec **v0.77**; no
wire, `db/` or SCHEMA change. **Maintainer's clinical decision:** duplicates are common, the person at the desk
cannot be made to browse, so accept them and make repair by `link` easy — the safety measure is how fast a
duplicate is FOUND.
- **`search.incomplete` = the SEARCH was partial** (ADR-0061's meaning, restored); being cut to `PROMPT_CAP` is
  `PromptList::withheld`, shown, never signed.
- **Seven-key ranking** (reorder only; HANDOVER's funnel rules): passes → identifier → a callsign typed whole →
  name tokens (exact or ≥3-byte prefix, over RETAINED names, #349) → DOB near-miss → exact tokens → chart age.
- **Measured** (`cairn-gui/cairn-gui-tauri/results/2026-09-26-funnel-prompt-ranking.md`, 50,000 real names, 500
  searches per arm): every single slip is now in the five (500/500); a surname typo AND a simply wrong DOB 203/500
  — that residue is the repair path's. ⚠️ Drawn from the pool's namesake-heavy FIRST 50,000 rows; the
  representative re-run is [#685](https://github.com/cairn-ehr/cairn-ehr/issues/685).
- **Filed:** [#679](https://github.com/cairn-ehr/cairn-ehr/issues/679) · [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680) ·
  [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) (the repair path — designed and in build, next entry) ·
  [#682](https://github.com/cairn-ehr/cairn-ehr/issues/682) (an NFD word-final accent is lost before NFC) ·
  [#683](https://github.com/cairn-ehr/cairn-ehr/issues/683) (`CandidateList`'s partiality as one sum type — a wire
  decision) · [#684](https://github.com/cairn-ehr/cairn-ehr/issues/684) (`1980-3-7` misses `1980-03-07` in the DOB
  pass — a SET gap) · #685 · [#686](https://github.com/cairn-ehr/cairn-ehr/issues/686) · #687 (fixed in the PR).
- **§1.2:** paper 1 → forced 1 → target 1; a warning that fired on 92% of registrations is gone.

### 2026-09-27 — the duplicate repair path designed; R1, the combined read, built (ADR-0076, PR #688)

[ADR-0076](spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md),
spec **v0.78**; design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (the
maintainer's decisions table; five slices **R1 → R5**, each with its own plan, PR and §1.2 section); plan
`docs/superpowers/plans/2026-09-27-repair-path-r1-combined-read.md`. `db/054_person_charts.sql`,
`SCHEMA_GENERATION` 53 → **54** (node loader list only; `cairn-sync` lags, #284). PR
[#688](https://github.com/cairn-ehr/cairn-ehr/pull/688), merged 2026-09-27.

The brainstorm surveyed the code before designing and found ADR-0075's *"repair by `link` is easy"* rested on
things that were not there: a `link` repaired nothing a clinician could see, "different people" had no home,
the matcher could not run on its own, and a machine link could override a human `unlink`. ADR-0076 answers
all four; R1 builds the first.
- **What R1 built.** `cairn_person_charts(uuid)` (every chart in the patient's link component, or itself) and
  `cairn_medication_duplicate_groups(uuid[])` (un-reconciled duplicates over the whole SET — the per-patient
  flag view would leave the same drug on two linked charts as two unflagged lines, a double-dose reading
  hazard, caught at planning); `ChartSet` (sorted, dedup'd, never empty) in `cairn-medication-view`;
  `patient/person.rs` (`person_charts`, `chart_identities` — each member's own name/DOB/trust, no winner);
  `medication/read.rs` reads the set and selects groups by MEMBERSHIP (**#334 fixed** — a cross-chart group
  shows on both charts, flagged, withheld from sign-off; `cross_patient` = the group reaches a chart OUTSIDE
  the set); `medication/signoff.rs` attests each thread under its OWN chart and refuses a changed chart set
  (displayed vs first read, first vs second read). The window: `med_list` → `ChartPane { list, members,
  members_error }`, member lines under the identity header, every row of a linked list labelled with its
  source chart, `sign_off`/`cease` send the displayed set (`cairn-gui-tauri/src/chart_set.rs`), a failed
  member read keeps the list with a warning; a pre-existing CSS bug fixed (`[hidden]` now beats
  `#unlock-form {display:flex}`).
- **Tests.** `person_charts.rs`, `medication_duplicate_groups.rs`, `medication_dup_key_drift.rs` (db/054's
  dup_key identical to db/033's once whitespace is normalised), `combined_read.rs` (incl. the golden
  `a_never_linked_chart_reads_exactly_as_before`, captured before the rewrite), `combined_signoff.rs`; two
  `medication_read.rs` #334 tests re-expressed. Full root sweep 2336 passed / 0 failed; webview walked with a
  stubbed bridge over a linked payload.
- **Filed:** [#689](https://github.com/cairn-ehr/cairn-ehr/issues/689) (db/034 admits an attestation naming a
  chart other than its thread's own — a floor gap) · [#690](https://github.com/cairn-ehr/cairn-ehr/issues/690)
  (db/033's local cross-patient guard refuses reconciling two LINKED charts' duplicate threads — the write-side
  mirror, a decision) · [#691](https://github.com/cairn-ehr/cairn-ehr/issues/691) (each linked row names its
  chart by full uuid, on screen and to the screen reader — want a short per-member tag). Commented on
  [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) (the between-reads chart-set refusal has no DB test).
- **PR review round (2026-09-27, five agents, then a second pass on the fixes).** What was fixed:
  - **A doubted link can no longer make a line signable or ceasable.** A doubted link is an un-attested link
    that db/018 flagged, or that trips the hard veto now: db/054 `cairn_chart_set_has_doubted_link`
    re-evaluates the veto at read time, which contains [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220)
    for this read. While a set holds one, a multi-chart group is a wrong-chart hazard; a cease on such a line
    stops only the opened chart's threads and names the rest.
  - **A linked chart whose registration is not held here reads `unknown`**, never the no-row `confirmed`.
  - `MedicationRow::patient_id` is renamed **`display_chart`**; the JSON name is unchanged.
  - **CodeQL** `rust/cleartext-logging` (12 alerts on `parse_uuid_list`, flagged for "uid") is cleared by a
    documented barrier row.
  - **A missing-group report names a reload, not the retired cross-patient cause.**
  - **Index:** `person_member(person_id)`.
  - **The CLI header prints each member's identity state.**

  Filed: [#692](https://github.com/cairn-ehr/cairn-ehr/issues/692) (a failed refresh overwrites the outcome) ·
  [#693](https://github.com/cairn-ehr/cairn-ehr/issues/693) (member line: dob precision, repudiated name) ·
  [#694](https://github.com/cairn-ehr/cairn-ehr/issues/694) (the member lines are absent from the semantic
  model) · [#695](https://github.com/cairn-ehr/cairn-ehr/issues/695) (remaining test gaps) ·
  [#696](https://github.com/cairn-ehr/cairn-ehr/issues/696) (`Option<&ChartSet>` → an enum) ·
  [#697](https://github.com/cairn-ehr/cairn-ehr/issues/697) (the withheld-line wording for a doubted link,
  and whether a doubted set's one-chart lines should be signable — DECIDED (b), 2026-09-27: withhold every
  line not on the opened chart while the set holds a doubted link).
- **Next: R2**, split 2026-09-27 into **R2a** (`patient_link.attested` outranking an un-attested link in
  db/018, the migration re-folding existing winners; `link_charts`/`unlink_charts` + CLI) and **R2b** (the
  window's link/unlink gesture, #681); then #697 (b); then **R3** (the front door collapses by person), **R4** (per-node matcher worker, #679 —
  proposes, never links), **R5** (banner + worklist, #680). Plan each from the design page's section.
- **§1.2:** paper counterpart two folders of one patient clipped together. Reading a linked chart paper 1 →
  forced 1 → target 1; signing off a combined list 1 → 1 → 1 (one gesture covers every line across both
  charts). `M ≤ N`; R1 adds no act. Budget: opening a linked chart ≤ the single-chart open, measurement owed
  by the runbook pass (a human act). The added reads, corrected after review:
  - **Every open:** `cairn_person_charts` and the per-group chart read. The set-wide duplicate query replaces
    the old per-patient flag query rather than adding one.
  - **A linked open only:** the doubted-link check and three identity reads, plus the `patient_chart` read.
  - **A window sign-off or cease:** a full list read before the orchestrator's own reads.
  - The planned "≤ 20 ms" figure was never a measurement.

---

## Above the foundation line (NOT in this roadmap)

- **Policy layer** — hard policy as a signed policy-assertion stream + effective-policy projection ([ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)); soft policy in UI. **GUI / reference UI** — built only on the same public native API everyone else uses (principle 12); paper-parity is the governing law, **no confirmation dialogs as a safety mechanism**. **Active-write thin encounters** and clinical workflow surfaces ([ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)).

## Parallel build-prep (not blocking the critical path)

- **Bet B — Pi compute-cost run** — **PASS twice on Pi 5 / 8 GB**: 2026-06-25 ([PR #57](https://github.com/cairn-ehr/cairn-ehr/pull/57), caveated by a USB-2 dock + PG16) and the clean 2026-07-07 re-run on PG 18.4 + a PCIe NVMe HAT with **both caveats resolved** — B1 p95 3.99 ms @ 2,004,000 events, B2 p95 4.5 ms/374-note chart; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on Cortex-A76). `cairn_pgx` is PG-18-capable (pgrx 0.18.1, [PR #56](https://github.com/cairn-ehr/cairn-ehr/pull/56)). **Only remaining follow-up:** fold the now un-caveated B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
- **Spike 0003 — Postgres on Android** — **Ran 2026-06-25, G0–G3 PASS**: native PG 18.2 + a cross-built pgrx extension (incl. SPI) on a stock Android 16 phone; validates the fractal-topology invariant at the phone tier. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/). Remaining gaps (from-source PG build, APK packaging) are non-load-bearing. **Continued clinical case-mining** stays the highest-signal mode for stress-testing the primitives before product build.
