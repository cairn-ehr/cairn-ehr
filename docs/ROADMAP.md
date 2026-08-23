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
  group spanning two patients displays on one chart only — wrong-chart hazard; read-path defence
  shipped, view unfixed), [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331) / [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333) / [#335](https://github.com/cairn-ehr/cairn-ehr/issues/335) / [#336](https://github.com/cairn-ehr/cairn-ehr/issues/336) / [#337](https://github.com/cairn-ehr/cairn-ehr/issues/337) (Slice 61 follow-ons).

**Operational caveats that outlive these slices.** Pre-ADR-0051 event logs (old `role:"author"`-without-
actor_id, flat-string responsibility) and pre-ADR-0052 plaintext `clinical.*` bodies **REFUSE at db/020** —
**wipe dev/PoC rigs** (the replication-failover demo, the spike rigs), never sync them through. Pre-wire
unsigned actor rows never sync. Test DBs need `cairn_pgx` ≥ 0.3.0.

**Slices 57–60 + the unattended interlude — condensed (2026-07-28 → 08-01; full detail in git and the
PRs; the *why* is in each ADR and must not be restated here).**

- **57 — `clinical.medication` 6b: the coding-overlay event types** (completes
  [ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) decision 3; `db/042`,
  SCHEMA 41→42). Coding is a **separately-authored act**, both types `('additive', FALSE)` so a pharmacist
  is not routed through the ADR-0043 owner gate. The decision it turns on: a reviewer who establishes a
  drug is NOT metformin but cannot say what it is must record *"not that, and I don't know"* (principle 4),
  so a **strike NULLs the anchor**. CLI `medication-code` / `medication-code-correct`, deliberately **no**
  `--attest-as`. Closed #295, **#296**. Lessons: a redundant projection column is a convergence hazard ·
  nullable-widening means re-reading every aggregate over it (`array_agg` KEEPS NULLs). Open: the
  **coded↔uncoded** duplicate case (ADR-0059 decision 5 is explicit its key does not close it); #294;
  [#300](https://github.com/cairn-ehr/cairn-ehr/issues/300).
- **58 — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power** (PR #302; closes
  [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) + [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266);
  SCHEMA 42→43). `apply_remote_event` used to RAISE on an `event_type` absent from `event_type_class`, so
  the event was **never stored at all** — a phone-tier node carrying a chart between two upgraded
  facilities acquired nothing past the first unknown-type event. The door now admits verbatim, projects
  nothing, confers nothing, records `event_deferred` (node-local, never on the wire);
  `cairn_readjudicate_deferred` (db/043) re-runs the classification-gated checks **before** anything
  reprojects. Lessons: refusal hides, admission cannot · an unverified value stored "for later" leaks into
  a live gate, and the fix is **neutrality, not strictness**. Open:
  [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301) (the node/actor plane still fail-closes, so
  §6.5's invariant holds **for clinical events only**),
  [#308](https://github.com/cairn-ehr/cairn-ehr/issues/308), [#309](https://github.com/cairn-ehr/cairn-ehr/issues/309).
- **59 — floor determinism** (PR #311 closes [#75](https://github.com/cairn-ehr/cairn-ehr/issues/75)).
  The §3.13 twin blank-test was **collation-dependent — a convergence break, not the cosmetic asymmetry
  #75 described**: Postgres's `\s` is `[[:space:]]`, whose ctype membership the collation decides, and
  `cairn_event_twin` is also the remote-apply gate, so **the same signed event could apply on one node and
  raise on another** (principle 1). Fixed by `cairn_twin_is_present(text)` in db/005, spelling the 25
  Unicode `White_Space=Yes` points as visible `U&'\XXXX'` escapes. **Generalisable: a "merely cosmetic"
  asymmetry between two implementations of one predicate is worth measuring before it is filed as
  benign.** Same PR readied the tech-debt loop ([#312](https://github.com/cairn-ehr/cairn-ehr/issues/312)).
- **Interlude — the loop ran unattended (07-31 → 08-01).** Nine PRs, no slice of their own. Closed **#79**,
  **#11** (residue [#317](https://github.com/cairn-ehr/cairn-ehr/issues/317)), **#100**, **#119**, **#120**.
  Loop fixes: PRs #316, #321 (a headless worker dies at turn end, so a successful cycle counted as a
  failure), #325. Open: #312, #314, #315, #322, #326, #327.
- **60 — ADR-0056 decision 5: the residual refusal contract, clinical plane** (closes
  [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267)/[#270](https://github.com/cairn-ehr/cairn-ehr/issues/270)).
  A *deliberate* floor refusal on **verifiable** bytes persisted nothing, froze the cursor and **exited
  SUCCESS** — a wedged peer link indistinguishable from a healthy one. Now penned by digest, deduped,
  auto-released when the refusal later applies, and a frozen watermark fails loudly. Three lessons: a
  refusal that persists nothing cannot be audited · when a call site cannot make a distinction, check
  whether an intermediate layer threw it away (`apply_signed` flattened `postgres::Error` to `String`,
  discarding the SQLSTATE — **`P0001` is a contract with the pull loop**: `cairn-sync` treats it as
  deliberate (skip, re-offer) and anything else as transient (freeze the cursor), so a bare `decode()`
  inside a door stalls sync from that peer permanently; PR #371 fixed the node plane and
  [#370](https://github.com/cairn-ehr/cairn-ehr/issues/370) the CLINICAL one, 2026-08-21 — see that entry) · symmetry between two planes is a hypothesis, not a goal — the naive
  [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) alignment would be a defect, since the node
  plane's deny-all is routine *scoping*, not refused history. Open: #268; the pen is fillable by a
  hostile-but-enrolled peer, bounded by the per-peer quota (§6.3).

**Slices 61–67 — condensed (2026-08-02 → 08-14; the first runnable clinical surface, then the §5.9
stream's first two parts; full detail in git, the PRs and the linked ADRs).**

- **61+62 — the med-list node tier and WINDOW** ([ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md),
  spec **v0.62**, SCHEMA 43→44). Cairn's first clinical READ path: pure `cairn-medication-view`,
  `medication/read.rs`, `medication/signoff.rs`, two CLI verbs; the **iced layer retired** for
  `cairn-gui-tauri` (semantic-HTML plain JS), `db/044` aggregate-only gesture timing. Node-tier write cost
  median **222 ms**. Sign-off is per LINE, like a paper drug chart. ADR-0060 came from a clinician override
  (#339) — *the saline must still be giveable beside an unsigned potassium minibag* — and reaches the
  transaction layer (decision 7), finding **#342**; underlying view defect **#334**. Two lessons repeated
  in HANDOVER: **a unit-tested safety control can still be defeated by the surface that calls it** (the
  idle re-lock never fired — a shared accessor counted every poll as activity, every `SessionKey` unit
  test passing), and **a compensating control outside CI is not a control** (`cairn-gui` is a separate
  workspace; the `gui` job now covers it, ⚠️ still not REQUIRED — **#444**). MPL-2.0 allowed for the GUI
  tree only. Owes **#288**. Open: #331 · #332 · #333 · #335 · #336 · #337 · #340.
- **63 — the §5.3/§5.8 search-before-create funnel** ([ADR-0061](spec/decisions/0061-registration-is-an-act-that-carries-its-search.md),
  spec v0.63, SCHEMA 44→46, `db/045`/`db/046`). Registration is an act that CARRIES the search that
  preceded it, and **the attestation NAMES the displayed candidates rather than counting them** — *was the
  duplicate on screen when the clerk clicked create?* has opposite fixes for yes and no, and `N = 3` cannot
  separate them. Ships `cairn-patient-search`, two CLI verbs, John Doe re-expressed onto the same act.
  Open: #346–#357, #359–#362; live ones worth naming are **#349**, **#351**, **#352**, and the §1.2
  write-cost half **#360**.
- **64 — closing the funnel's bypass** (closes **#345**; SCHEMA 46→47). db/005 step 8b: a chart's first
  `patient_id`-bearing event must be its registration. The remote door stays lenient **by design** —
  set-union sync has no ordering. **Retiring `patient.created` was the load-bearing half**: otherwise the
  rule reads *"…unless…"*, and an "unless" in a safety floor is where the next defect lives. The rule never
  reaches a patient named in a *payload*, so `patient.amended`/`note.added` survive unfloored (**#364**,
  **#365**).
- **65 — the §5.9 sensitivity stream, part A** ([ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
  spec v0.64, SCHEMA 47→48, `db/048`). Graded append-only assertions over event / thread / chart; effective
  grade is the **max** of the three. Computes and reports only. Three traps, all repeated in HANDOVER
  because each is a cleanup a later reader would attempt in good faith: **unknown ranks MAX** (inverting
  db/040's `ELSE 0`); **the grade is node-relative** (less custody deliberately computes *higher*, so any
  cross-node equality test needs *given equal custody*, and per ADR-0064 §4 equal actor-registry state
  too); and erratum E6, `content_address IS NOT NULL` is the "did anything win" test, never
  `subject_kind <> 'none'`. Parts: B #375 (Slice 67), C **#376**, D **#377** (both designed 2026-08-23 —
  see the ADR-0065 entry below). Follow-ons open: #374, #378, #379, **#436**.
- **66 — custody follows admission** (closes **#231**; ADR-0052 erratum E1). The unwrap-cert `kid` is
  pinned to `trust_peer` (db/007); before it, any self-signed cert reaching the serve port obtained
  read-custody of every non-shredded sealed body. **Withhold the key, never the bytes** — refusing the
  bytes would fork the event set. Repair is TWO steps (`pull --full`, then `cairn_reproject()`). Same day
  (PR #390): `unsound = "all"` in both `deny.toml` trees, cargo-deny v2's `"none"` default having let an
  advisory pass in silence; one finding ignored with a reason (**#389**) but **no** expiry — cargo-deny
  0.19.9 accepts only `id` and `reason`, so a review date lives in the `reason` and
  `advisory_ignore_review_dates.rs` fails when it passes.
- **67 — the §5.9 safety projection, part B** (closes **#375**, discharges #294;
  [ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md), spec v0.65,
  SCHEMA 48→49). **The seal boundary is the coarsening boundary:** precise `{class, severity}` travels
  sealed with the body, a grade-chosen **rung** rides the envelope in the clear, so *coarsen-but-survive*
  after a crypto-shred is structural. Two coarsenings, load-bearing for DIFFERENT reasons: emission binds a
  peer's raw-SQL client; read answers a peer that legitimately emitted a finer rung, and **read coarsening
  is a rendering choice, not a floor**. `safety_class_map` ships **EMPTY** — the seam drugref plugs into.
  The PR #403 review also fixed **#404**. Open: #394, #395, #397–#402, #406, #407.


**Slice 68 + two interludes — condensed (2026-08-15/16;
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md), spec v0.66; closes
[#380](https://github.com/cairn-ehr/cairn-ehr/issues/380), #412,
[#405](https://github.com/cairn-ehr/cairn-ehr/issues/405),
[#426](https://github.com/cairn-ehr/cairn-ehr/issues/426); ADR-0063 gains erratum E1; no new migration).**

- **Claim authority at the apply door.** A protection-removing claim takes effect only when a human this
  node can hold responsible stands behind it. **One predicate, one site:** `cairn_claim_authority(claim,
  target) → 'attested' | 'self' | 'unverified'` (db/005), consulted at exactly one clause in
  `cairn_sensitivity_standing` (db/048), so display coarsening, safety-rung emission and part C's dial all
  inherit it — the anti-drift answer to #404. It **gates effect, never admission**, and only in the
  withholding direction, so no door refusal and no fork (the #342 trap). *Flag what cannot self-heal; view
  what can* became a stated rule: the withdrawal worklist is a VIEW, `safety_overclaim_flag` a LEDGER.
  Gives [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) its first SQL counterpart — NOT its
  mirror. The PR #410 review found **7 of 11 production-code mutations survived a green suite**: an
  unpinned R2 self-identity equality, `EXCEPTION WHEN OTHERS` not catching a statement timeout (57014 is
  one of the two codes it excludes), a fail-OPEN protection-stripping comparison, and comments asserting
  guarantees the fixtures never delivered. Open: #408, #409, #413, #414, #415, #416, #417, #418, #419,
  #420, #422.
- **One §5.9 leak closed, one narrowed.** Both were **a guarantee stated in a comment the code did not
  provide**, one per plane — and the review found the first fix had reproduced that shape in its own
  prose. *In SQL:* a column-level `REVOKE` cannot narrow a table-level `GRANT`, so db/049 §8 drops
  `cairn_agent` to an explicit 23-column grant omitting `safety` and both read functions became `SECURITY
  DEFINER` — a pair where either half alone is broken. **Cost-raising, not a floor:** `event_log.safety`
  copies a *clear* field of the signed body and `signed_bytes` stays granted (#424 closed; the open design
  question is **#432**), and the runtime role is a `cairn_node` member db/049 never narrows (**#425**).
  ADR-0063 decision 2 — emission-time coarsening — is what binds. Replay-window residual **#427**. *In
  Rust:* a parameter name is not a security property; `classify_authorship_confidence` graded a forgery
  `Attested`. Both key arguments are now a `VerifiedKid` newtype; mint-site allowlist unpinned **#428**.
- **The `search_path` that pinned nothing** (21 headers gained `, pg_temp`). Live data loss at both
  owner-rights write doors, not hygiene: `SET search_path = public` does not exclude the session temp
  schema, so with a decoy `event_log` in place `submit_event` and `apply_remote_event` each **returned
  SUCCESS while the owner-privileged INSERT landed in the caller's temp table** — demonstrated as
  `cairn_agent`, a role with no write privilege on `event_log` at all. A pinned path must deny the temp
  schema the *first look*, and the guard is over `pg_proc`, not a name list. Open: **#430** (~100 unpinned
  invoker-rights functions; `cairn_patient_has_events` is safe only by inheriting `submit_event`'s path),
  **#431**, **#420**.


**Slice 69 + its two follow-on passes — condensed (2026-08-18 → 08-20; closes #388, #383, #421, #435,
#387, #439, #382, #385, #381; opens **#436**, **#441**, **#444**; ADR-0064 gains errata E1/E2; no new
ADR, no new migration, SCHEMA stays 49).**

- **The §5.9 operator surface.** Three slices had shipped a §5.9 mechanism and no way to look at it, so
  ADR-0064's §1.2 budget stood *owed*. `patient-sensitivity <chart>` reports the withdrawal worklist
  (reason + rationale + accountable actor), deferred `sensitivity.%` events, the standing assertions a
  custody-thin node cannot anchor, safety overclaims, and the **measured** count of sealed medication
  events held without custody; `sensitivity-assert`/`-withdraw` read back what took effect. **NAME, NEVER
  COUNT** — #388 part 3 and #383 both asked for a *count*, and a count cannot separate *custody-blind*
  from *genuinely empty*, the one question the line exists to answer. Mechanics: a chart-scoped definer
  (`cairn_patient_deferred_sensitivity(uuid)`, db/043) because `event_deferred` is granted to `cairn_node`,
  not `cairn_agent`; db/048 projects `responsible_actor_id`, which its `judged` CTE always computed and the
  outer SELECT dropped (#421); custody-blindness is **measured** in db/048 §11b, not inferred from
  `standing.is_empty()`; peer-copied fields are Debug-escaped (a newline forged a report line); **one
  header per worklist arm**, because the `stranger-attested` arm DID take effect and a shared *"did NOT
  take effect"* sentence told the operator a completed, unaccountable removal of protection had not
  happened; and the report declares what it cannot contain, asserted over *empty* lists.
- **The read-back and its known gap.** `readback.rs` reports two independently observed facts **never
  merged** — which worklist arm (accountability) and what this node can say about the target (effect) —
  because db/048's `inert` arm merges *"nobody accountable"* with *"not replicated here yet"*. It gives
  ADR-0064's KNOWN GAP its first test: `TargetState::OnAnotherChart` must never collapse into
  `Held { still_standing: false }`, since `cairn_sensitivity_standing` is patient-scoped on both sides
  (load-bearing — else chart B strips chart A) and a naive membership test reports a mis-chart withdrawal
  **effective**, a precise untruth in the reassuring direction on a confidentiality surface. Residual
  **#436**: the same shape arriving by REPLICATION is invisible, and the fix is visibility, not a door.
  Type design: `source` → a `Provenance` enum; `GRADE_*` stay **consts** (ADR-0062 decision 2: `grade` is
  OPEN and db/048 mints a third `source` itself); **`WinningSubject`** fuses `chart_source` +
  `chart_content_address` on the ADDRESS, making erratum E6 structurally unrepeatable.
- **The trap-clearing pass (2026-08-20).** **#439:** `cargo doc` was red on `main` and completed only
  under two `-A` flags, hiding every later rustdoc error; the BLOCKING copy now runs as the last steps of
  the required `test` job (root workspace + `fixtures` + `cairn_pgx`) under an explicit `RUSTDOCFLAGS=-D
  warnings` — **#444** owns promoting the still-unrequired jobs. **#382:** the `REVOKE EXECUTE … FROM
  PUBLIC` convention was followed by 5 of 22 `cairn_check_*` validators; the deliverable is
  CHECKABILITY — `floor_execute_grants.rs` asserts it over `pg_proc.proacl` (**a NULL ACL is the
  PERMISSIVE case**), its applier half reading the `cairn_projection_apply` REGISTRY, not the `_apply`
  suffix (1 of 21 carries none). **#385:** the five thread projections index `content_address` and
  `cairn_event_thread` returns early for §10b's thread-free types — **which made that list safety-critical
  in the DISCLOSURE direction**, since §11's conservative bound is gated on the negation of the same
  predicate, so widening it wrongly silences all three thread arms at once and a standing `sequestered`
  grade reads back `('routine','none')`. The first draft's comment claimed the opposite; correcting it was
  the most consequential finding. **#381:** three pins on the db/048 SQL mirror. All guards
  mutation-tested. Residual: #385's index win is unmeasured on volume data; two hand-maintained mirror
  lists (**#441**). Still open on this surface: **#414**, **#415** (expect it to fire on routine care now
  that it is visible), **#416**.

**Two tech-debt passes — the silent gates and the trap-clearing they left behind (2026-08-20 → 08-21; closes [#446](https://github.com/cairn-ehr/cairn-ehr/issues/446), **#442**, **#443**, [#449](https://github.com/cairn-ehr/cairn-ehr/issues/449)–[#453](https://github.com/cairn-ehr/cairn-ehr/issues/453), [#386](https://github.com/cairn-ehr/cairn-ehr/issues/386); opens **#447**; no ADR, no migration, SCHEMA stays 49).** Nine gates that could pass **without running** — built nothing, so the lessons live in HANDOVER and only the mechanics are logged here.

- **Lockfiles + CI (#446).** `cargo_lockfiles_tracked.rs` asks **cargo** — not a hand-rolled `members`/`exclude` parser — which manifests own a lockfile, then asserts each is tracked AND matched by no ignore rule. Two trees failed, and asking cargo at all surfaced a third defect: **`packaging/crates` was in no workspace and not excluded**, so every cargo command there had been erroring since the crates graduated. Zero exemptions now, and every repo cargo invocation in `rust.yml` passes `--locked` (fronted by `cargo fetch --locked` for `cargo pgrx install`, which has no such flag). `git check-ignore` needs `--no-index` (git otherwise *skips tracked paths*) and has **three** exit codes (0/1/**128**) — reading 128 as "clean" was a second route to vacuity.
- **The DB gate (#442 → #449/#450, #451).** `db_gate_actually_ran.rs` fails when a gate variable the suite reads is unset, over a list **derived from the test sources** — a `CAIRN_TEST_*` name counts only inside an `env::var("…")` argument, so header prose can no longer invent a requirement. It **fails CLOSED** (`CAIRN_ALLOW_DB_SKIP=1` opts out), because `CI` is set in zero places here and a scrubbed environment silently disabled it. #451 gives the matcher the same guard in the other language.
- **The REVOKE convention (#443 → #453) and two unifications (#452, #386).** `cairn_event_twin` plus the whole `cairn_twin_%` family — **four** functions, not the two the issue named — paired with two GRANTs, because **PostgreSQL checks a function called inside a VIEW against the INVOKING user** and the INNER call too, so a bare REVOKE broke `event_twin_provenance` for `cairn_agent`. Correcting the record: the registry names **16 distinct** `check_fn`s across **24 rows**; **22** is the `cairn_check_%` family. Three hand-rolled source walks became one leaf module (`tests/common/sources.rs`, `#[path]`-included, deliberately not `common/mod.rs`): one followed symlinks unbounded, another swallowed unreadable dirs, and both fixes were already in the third. #386 drives db/048's two `to_regclass` probes on the cairn-sync subset.
- **Deliberately not done:** unifying the 342 bare `else { return }` skip sites would add ~1000 lines of boilerplate and enlarge [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327)'s job. **Residual:** cargo-deny still covers three of six trees (**#447**).

**Two self-contained defects — the freeze that hid and the flake that lied (2026-08-21; closes [#370](https://github.com/cairn-ehr/cairn-ehr/issues/370), [#457](https://github.com/cairn-ehr/cairn-ehr/issues/457); opens [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458); no ADR, no migration file, SCHEMA stays 49).** Workspace sweep **1434 passed / 0 failed**, `clinical_pull` green in the parallel run.

- **#370 — the clinical plane's copy of #228, and the family was nine.** `cairn_learn_attachment_refs` (db/027) read `digest_hex`, `media_type` and `byte_len` out of a signed body with no shape check; a signature proves the bytes are what the author signed, not that the payload is well formed. Measured on PG 18.1, that one function had **nine** freeze paths across four SQLSTATE classes — 22023 (`attachments`/`renditions` non-array *including JSON null*, non-hex, odd-length) · 23502 (absent `digest_hex`, a scalar rendition, absent `media_type`) · 22P02 (fractional `byte_len`) · 22003 (`byte_len` past bigint) — every one of which `cairn-sync` reads as transient and **freezes the clinical pull cursor on**, permanently, reported to the operator as "transient?". Plus **four SILENT** paths that wrote something wrong: an empty `digest_hex` (the address is `blob_store`'s PRIMARY KEY, so every empty reference from every peer collides into ONE row), a negative `byte_len`, a blank `media_type`, and a scalar attachment (**#458**). Fixed by three validating accessors plus a total list-coercion in db/027, with `digest_hex` routed through db/001's `cairn_decode_hex_or_raise` — the first CLINICAL-plane call site of the #228 helper, and one that must be registered in `hex_decode_helper.rs`'s per-file count list. **The rule the accessors follow:** refuse what already FAILED plus what was silently WRONG; accept everything that already worked (uppercase hex, an absent `byte_len`, a `byte_len` encoded as a digit STRING are accepted *deliberately*, and pinned), because every refusal added at a remote door is a new way for a peer's clinical event to be penned. **Granularity was left at "both doors refuse" and that was wrong — see the #460 entry below**, which builds the asymmetry (refuse where the field is MINTED, admit-and-flag where it ARRIVES) and corrects the argument first written here: the pen never releases, because the malformed field sits inside the signature.
- **#457 — the readiness harness polled a PORT and never the CHILD.** So EADDRINUSE, a missing `--key` and a panic during schema load all produced one 60-second message blaming startup latency — which is why #238's ceiling and #263's port floor both aimed at the wrong thing and the flake outlived both. `crates/cairn-sync/tests/common/serve.rs` (a `#[path]` leaf module, self-tests in `serve_readiness_shared.rs` per the #452 precedent) now watches child and port together, captures the child's stderr **to a file rather than a pipe** — an unread pipe fills and blocks the child, a readiness harness causing the failure it reports — and makes spawn-and-wait ONE call, so a thirteenth test cannot serve on one port and wait on another. **The cause is named, not fixed:** always exactly three of twelve, always the full ceiling, macOS, only under a loaded parallel sweep — a child ALIVE but not yet at `main` fits, and this repo has hit macOS `_dyld_start` loader stalls before; a stall now reports `TimedOut` with a live pid. The pass also **corrected a false comment**: the port header claimed `std::net::TcpListener` does not set `SO_REUSEADDR`. It does, on every non-Windows target — verified in the pinned 1.96.0 toolchain's own source at `library/std/src/sys/net/connection/socket/mod.rs:550-553`, and `cmd_serve` binds through exactly that call. **Both fixes mutation-tested, and mutation found the same species in this pass's own work:** a comment claimed dropping a `COALESCE` would make a guard fail OPEN; removing it left every test green (`jsonb_array_elements(NULL)` yields zero rows rather than raising) — the comment now states the property that IS true (the coercion is TOTAL) and a test asserts it.

**The db-error legibility sweep, four passes (2026-08-22; closes [#460](https://github.com/cairn-ehr/cairn-ehr/issues/460), [#465](https://github.com/cairn-ehr/cairn-ehr/issues/465), [#467](https://github.com/cairn-ehr/cairn-ehr/issues/467), [#469](https://github.com/cairn-ehr/cairn-ehr/issues/469), [#471](https://github.com/cairn-ehr/cairn-ehr/issues/471), [#473](https://github.com/cairn-ehr/cairn-ehr/issues/473), [#474](https://github.com/cairn-ehr/cairn-ehr/issues/474), [#475](https://github.com/cairn-ehr/cairn-ehr/issues/475); opens **#461**, **#463**, **#464**, **#468**, **#470**; `db/050`, SCHEMA 49 → 50; **no new ADR in any of them — that is the finding**).**

- **#460 — admit and flag: a malformed attachment reference must not sink a replicated clinical event.** #370's fix refused at BOTH doors, contradicting **ADR-0063**, written eight days earlier, which decides this exact shape for the §5.9 `safety` field and states the rule generally — **an envelope-level field is constrained where it is MINTED and read permissively where it ARRIVES**. **The category test:** a sensitivity assertion IS an event (refusing one drops that assertion alone); `safety`, `clock_grade` and an attachment rendition reference are FIELDS ON a clinical event (refusing one at apply drops the clinical act it rode on — the **#342** trap, hit five times). The deciding argument is **blast radius, not category**; ADR-0065 later NAMES the rule. Residuals: **#463** (flag resolution — a DECISION, overlay vs delete), **#464** (unbounded per-rendition subtransactions), **#458** (a non-object element — a loud UI, NOT a floor rule).
- **#465 — the signal the flag owed, and peer text is not display text.** A flag nothing reads is not a control. **⇒ `custody_withheld` is unbounded prose from an UNADMITTED peer, printed raw** — enough to forge a `0 attachment reference(s)` all-clear on the very alarm being installed. **⇒ A flag can be born on a re-apply**: db/020 calls the lenient learner unconditionally, so a node upgrading onto db/050 flags its whole pre-#460 back-catalogue; the report is keyed on the admitted addresses **and** a `flag_id` watermark, and dropping either turns five tests red. **⇒ A failed read reports `null`, never `0`** — zero is a claim, and after a failed read it would mute a monitor exactly when this node stopped being able to see. Open: **#468** (the alert fires ONCE EVER while its stated precedent re-fires every cycle), **#470** (the per-cycle ledger read is owner-privileged).
- **#467/#469 — the two places a database failure said nothing, and the freeze that hid.** **⇒ `tokio_postgres::Error`'s `Display` IS the string `"db error"`** — a bare kind match never chains to the source holding the message, the DETAIL and the SQLSTATE — **and `anyhow!("…: {e}")` discards the source too**, so the wrapper meant to add context subtracts the diagnosis. `db_diagnosis` renders `message [SQLSTATE] — DETAIL — HINT`, byte-identical to `cairn-sync`'s `legible_db_error`, so an operator learns one format. **⇒ `LocalDbFault` IS NOT A RENDERING and must not be "tidied" into an `anyhow!`** (`Display` is the text, `source()` is what a classifier walks) — the trap in this sweep most likely to be sprung in good faith. **⇒ A frozen cursor looked exactly like a healthy cycle:** all three of `pull_into`'s freeze paths `break` and return `Ok` (correct — freezing is the deliberate availability choice), so a `53100` disk-full emitted **neither** `LOCAL FAULT` nor `PARTITION` and a monitor keyed on those two tokens would watch a stuck node forever (`PullStats.frozen` + `frozen_cursor_line`). Also: **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** (57014), and db/027 raises where db/050 records — **`WHEN OTHERS` there would be a disaster**, writing a disk error into the ledger as "the peer sent garbage".
- **#471/#473/#474/#475 — finishing what the failure started.** Partial-completion reporting (`RequeueInterruptedError`), the guard widening that **failed on exactly three lines, all comments quoting the shape that caused the defect** — *a guard that punishes the precise description of its own bug pushes every future writer toward vaguer prose*, so whole-line comments are skipped while a trailing comment after code is still scanned — **and a rename is not proof**: two renamed bindings genuinely held database errors and the widened guard reported green over both. **Two test mechanics worth reusing:** to force a write failure in a SHARED test database take a LOCK from a second connection under a short `lock_timeout` (never a trigger or a `REVOKE`, which persist if the test panics and poison every later suite) — `FOR UPDATE` for a write, `ACCESS EXCLUSIVE` when the target is a read, since MVCC readers do not block; and **`Debug` must delegate to `Display`** on any error that can reach `main`, because `fn main() -> R<()>` has no error printer and `Termination` prints `{err:?}`.

**Open-issue index — every open number the narrative above does not name.** The convention is *never drop
an open issue number* (the PR #271 review finding); prose cannot hold ~145 of them, and this index only
guarantees nothing is orphaned here. Live list: `gh issue list --state open`.
#93 #98 #283 — spec/ADR debt (revocation cascade trusts the authoring node's clock; ADR-0016 vs ADR-0001
and the node-compromise threat model; the `clock_grade` still unrendered in the legibility twin). #97 #347 #348 #353 #354 #355 #356 #361 — demographics + Slice 63 follow-ons.
#101 — sync/blob (paginate `EventsAfter`, blob `byte_len` wedge, BLAKE3 verify in-DB). #318 #329 #373 #411 —
test + tooling hygiene (`cairn-sync/src/main.rs` at **10.1k** lines; no frozen wire fixture across
crypto-library bumps; `run-db-sql-tests.sh` wrong-cluster + `dropdb` before the #169 marker check).
#458 — a non-object attachment element admitted silently (re-scoped 2026-08-22 to the #460 ledger + a loud
UI, NOT a floor rule). #392 #393 — federation (`peer_pubkey` hex case; custody grants leave no audit trail).
#303 #304 #305 #306 — the tech-debt loop's own tooling, stopped.

**The sweep's tail (2026-08-23; closes [#481](https://github.com/cairn-ehr/cairn-ehr/issues/481), [#479](https://github.com/cairn-ehr/cairn-ehr/issues/479), [#477](https://github.com/cairn-ehr/cairn-ehr/issues/477); opens [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485) and, from its review round, [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487)–[#492](https://github.com/cairn-ehr/cairn-ehr/issues/492); `crates/cairn-node` + `crates/cairn-sync` — no migration, no ADR, SCHEMA stays 50).** The three the entry above raised and could not absorb. Every item mutation-checked.

- **#481 — a guard only runs when its own crate is tested.** #450's fail-closed DB-skip guard lived in a `cairn-node` integration test, so `cargo test -p cairn-sync` printed `101 passed` with no database — over the crate holding the ONLY test of a real mid-loop requeue interruption (#471) and the whole of #475's acceptance criterion. It moved to `crates/cairn-node/tests/common/db_gate.rs`, pulled in by BOTH crates with `#[path]` (one implementation, two binaries — #452's lesson). Two findings the move produced: **`file!()` expands to the path the INCLUDING file wrote**, so the shared module has two spellings and the self-exclusion stopped firing — caught because the assertion checks it fired **exactly once** rather than assuming it did (new pure `lexically_normalized` folds `..` without touching the filesystem; `canonicalize` would need a path relative to the process cwd, which under `cargo test -p X` is the crate dir); and the durable half, `every_crate_with_db_gated_tests_runs_this_guard`, DERIVES the obligation from which crates read a gate variable.
- **⇒ The fix that landed the day before printed the whole diagnosis twice.** `db_diagnosis::operator_chain` walked past the `tokio_postgres::Error` it had just rendered into that error's own `DbError`. Its suffix rule cannot catch this — `compose_db_diagnosis` and `DbError::Display` format the same three fields differently, so neither ends with the other. **It survived because every fixture built its error from an unparseable connection string** (`Kind::ConfigParse`, whose rendering ends with its cause, so the suffix rule fired and the dedupe LOOKED correct); the arm every in-DB refusal takes had no coverage and could not have — a `DbError` cannot be constructed by hand. Fix is one `break`, plus a DB-gated test in **both** crates.
- **#479 — the run loop's own species, on the surface `bet_a.py` reads.** `cycle 118: PULL FAILED: db error`, every cycle for the life of the process, because `cmd_run` builds its client ONCE outside the loop. New pure `operator_chain` (the `Box<dyn Error>` twin of `cairn-node`'s) feeds both the terminal line and the JSONL `pull_error` key. Eight sites, including the serve trust-set lookup — the AUTHORIZATION path for an inbound peer, answered with eight characters — and `do_fingerprint`'s call site, which had **no `else` arm at all**.
- **⇒ The landmine the issue warned about, disarmed rather than avoided.** Naming the failing operation means wrapping, and `downcast_ref` inspects the OUTERMOST type only — so a wrapper would have pushed the `postgres::Error` out of `classify_pull_failure`'s reach and logged a dead local database as link downtime: #469's defect, reinstated by its own fix. `chain_reaches_a_postgres_error` walks instead, checked **last**, after the two arms that match on the outermost type deliberately.
- **#477 — the §5.7 auto-apply ceremony, converted as a SUBSYSTEM.** `auto_apply.rs` alone would have left `resolve_failure_line` rendering `matcher_actor.rs`'s three unwrapped registry reads. Both files added to `GUARDED` (five files), each with its own count pin. **The two count pins deliberately count different shapes:** `sync.rs` counts bare `LocalDbFault::new(`; `auto_apply.rs` counts the `.map_err(|e| …` shape, because its TEST module builds one too.
- **Where an interpolation scan cannot reach, pin the SITES.** `cairn-sync/src/main.rs` stays out of `GUARDED` (10.1k lines, most `{e}` sites hold non-database errors); its fixed sites are pinned by exact shape plus a count, inside `cairn-node`'s guard file — which reads by repo path, so a fourth copy of the machinery was not needed. Honest about being narrower than a scan.
- **⇒ The review round, same PR.** It found the sweep's own guard green over two of the eight sites it pinned: **two shapes stopped at the format string and its comma**, so reverting the byte tier's chunk insert and the serve trust-set lookup to a bare `e` left it reporting `5 passed` (measured). Both now pin the rendering call. **All three presence guards also read RAW text**, so a wrapper deleted and left behind in the `//` comment explaining its deletion still satisfied them — new pure `flattened_code` strips comments and flattens whitespace, which also makes every shape reflow-proof. Plus: a count pin's doc claiming the opposite of what the count does; a silently-reverting `pull_error` test; the fingerprint fix reaching stderr but not the JSONL artifact; and four comments the code contradicted. Residual: **23** further `cairn-node/src/` files hold **89** postgres call sites naming no operation (**#485**, per-file table there) — ugly-but-not-silent, since a bare `?` preserves `source()`.

**The misclassification cluster (2026-08-23, second pass; closes [#489](https://github.com/cairn-ehr/cairn-ehr/issues/489), [#482](https://github.com/cairn-ehr/cairn-ehr/issues/482), [#480](https://github.com/cairn-ehr/cairn-ehr/issues/480), [#490](https://github.com/cairn-ehr/cairn-ehr/issues/490) items 1–2; `crates/cairn-sync` + `crates/cairn-node` — no migration, no ADR, SCHEMA stays 50).** Three issues the round above raised, all one species: **a failure wearing another subsystem's clothes.** Every item mutation-checked; full local gate 1568 passed / 0 failed over 139 binaries.

- **#489 — `classify_pull_failure`'s default arm claimed safety BY ELIMINATION, and three sites falsified it.** A corrupt response body, an events-vs-seqs count mismatch and non-ascending seqs all returned a bare `String`/`serde_json::Error`, fell to `partition`, and were counted by `bet_a.py` as **link downtime** — for a peer that had answered in full. Sixteen lines above the second of them, the structurally identical signing-context skew had returned a `PullIntegrityError` since #108; the two classes disagreed about the same kind of fault. All three now return `PullIntegrityError` and are driven end to end against `serve_canned`.
- **#489 part 1 / #490 item 2 — a pen refusal has two causes that call for opposite actions.** `pen_refused` fed `cycle_is_loud` → the single class `integrity` (*the peer answered and its DATA is the problem*), so a local disk-full, a revoked grant or a lock timeout sent an operator to audit a peer's **signatures**. It could not have been fixed without #490 item 2 first: `quarantine_event`'s private `legible()` rendered a server error into `db.message().to_string()` — a `String`, which has no `source()` — destroying the SQLSTATE and the chain a classifier reads, which is exactly the trap `LocalDbFault`'s own doc warns about, one file over. With `LocalDbFault` in its place (3 new sites) the two causes separate: new pure `pen_refusal()` + `PenRefusal{message, local_fault}` sets `PullIntegrityError::also_local_fault`, so a **database** refusal reports `["integrity","local_fault"]` (the mirror of `CursorCommitError::also_loud`) while the **per-peer quota** — a budget exhausted by the peer's own garbage — stays `["integrity"]` alone. **BOTH directions are pinned**; the second needed an assertion added to the existing quota test, because the mutation `local_fault: true` passed the whole suite until it was.
- **#482 — `cairn-node` was one class behind its sibling, and the missing case was a security event.** `PullFailureClass` had `LocalFault` and `Partition`, where `Partition` was simultaneously the default-by-elimination and the instruction *go and look at the link*. An **mTLS pin mismatch** — a rotated or REVOKED peer key — was therefore indistinguishable from a satellite outage, and charged against the availability figure; a short frame and an over-cap frame prefix likewise. New `PullFailureClass::Integrity`, deliberately the sibling crate's word widened by one clause to cover identity (**a pinned key IS a claim about the bytes that follow it**); the enum is not `#[non_exhaustive]`, so it broke `run`'s one `match` — the forcing function. **The recogniser is a type or an `io::ErrorKind`, never message text:** `io::ErrorKind::InvalidData` anywhere in the chain covers both `tokio-rustls` (which maps a `rustls::Error` surfacing out of the HANDSHAKE to that kind — not every one: a failure constructing the connection is `ErrorKind::Other`) and `read_frame`'s over-cap prefix, while the short-frame check — which reads intact bytes and finds them self-inconsistent, so has no io error at all — carries a typed `PeerIntegrityError`. `LocalFault` stays FIRST because **a TYPE outranks a KIND**: a broad `ErrorKind` net must not re-label what a concrete type already claimed. (The first draft said *this node's database is reached over TLS too*. Both crates connect `NoTls` and `db.rs` already said so as a PR #478 finding; the ordering is still load-bearing because an `io::Error` wrapping a `LocalDbFault` carries both signals, and it is the standing guard for a remote `db_conn`.) Accepted blur, documented: a badly lossy link reads as a peer problem — the safe direction.
- **#480 — `ApplyError` is not "legible by construction", and `do_requeue` believed it was.** Its no-`DbError` arm was `postgres::Error`'s bare kind (`db error`), and `apply_signed`'s FIRST statement is a newness probe on `event_log` that never reaches the door — so a lock storm or a half-finished `pg_restore` was written into `sync_quarantine.last_requeue_error` as *the in-DB door adjudicated these bytes and rejected them*, which is what an operator reads while deciding whether an event is corrupt. Now the failure is routed by the new pure **`apply_failure_is_local`** (a SQLSTATE-class split) to `RequeueInterruptedError` (#471's partial-completion report); the type keeps its `postgres::Error` as `source()`, and `operator_text()` renders the ONE shared format. **The accepted cost is the PULL path's cost too** and is documented at the site: a LOCAL fault stops every run at the same row rather than annotating it — delayed, never lost, loud, and free, because the row behind it would meet the same broken database anyway.
- **⇒ Where a pin's fixture is built by the test, the production site is unpinned.** The classifiers' unit tests would have stayed green through a revert of any of the six sites, so each is now driven the real way: a **deny-all `TrustStore`** IS the revoked-peer case against the node's own `serve`; a new **hostile stub peer** (real pinned mTLS handshake, then one deliberately malformed frame) covers the two protocol checks; `serve_canned` covers the three clinical-plane ones; and the two local-fault paths are forced by a row lock and an `ACCESS EXCLUSIVE` table lock under a short `lock_timeout` — never a `REVOKE` or a trigger, which persist in the shared test database if a test panics. New `crates/cairn-node/tests/pull_peer_integrity.rs`.
- **⇒ The review round, same PR: the fix carried the defect it was fixing, one match arm above itself.** `also_local_fault` was computed from the pen write alone, so an APPLY that failed on this node's database still published `integrity` alone — #489 part 1's own sentence, in the arm above the fixed one — and `bet_a.py` folded the blocked write's `elapsed_ms` into the A4 latency percentiles because its filter is `not r.get("local_fault")`. The existing `40001` test was ONE assertion short of proving it. Separately, #480's guard `!is_deliberate_refusal()` was wider than *our machine broke*: it halted `do_requeue` on every non-`P0001`, including deterministic byte-attributable ones (`22P02`, a constraint violation, an `XX000` from a function fed adversarial bytes), and since the listing is `ORDER BY first_seen` and `cairn-sync quarantine` is read-only, one such row wedged every row behind it forever — the outcome `db/001_envelope.sql`'s header already records one plane over. New pure **`apply_failure_is_local`** serves both callers; a byte-attributable failure annotates *"NOT a deliberate floor refusal"* in the database's vocabulary and the sweep continues.
- **⇒ A guard for an ordering property that could not observe the ordering.** The `LocalFault`-first test used `anyhow::Error::from(pg).context(io_error)` on the stated belief that a `.context()` value is reachable from `chain()`. It is not — `ContextError::source()` returns only the inner error — so the `Integrity` arm never matched the fixture and **swapping the two arms left the test green** (measured). Rebuilt on `io::Error::new(InvalidData, LocalDbFault::new(…, pg))`, whose chain genuinely carries both, and it now asserts both signals are present before asserting which wins. Also: `cairn-sync`'s own over-cap frame prefix was still `partition` because the request site flattened its `io::Error` with `format!` — the same `source()`-destroying trap this PR documents elsewhere — so the two planes gave one failure two operator words; new typed `PeerRequestError` + `chain_reaches_a_peer_frame_error`, checked LAST. Plus `merge_pen_refusal` (first-wins message, OR-ed flag), `operator_text()` on the freeze line, `run: PEER INTEGRITY` to stop colliding with the `Ok` arm's `run: INTEGRITY:`, both revocation directions named, an end-to-end `partition` guard on the clinical plane, `lock_timeout` on the blocker connections, and six comment claims the code contradicted.
- **Two more `db error` sites and two comments the fix contradicted.** The pull loop's pen-release `DELETE` rendered `{de}` — a binding `RAW_ERROR_BINDINGS` does not name (#490 item 1) — and `db_diagnosis`'s header still listed `quarantine_event`'s now-deleted renderer while the guard's own header still called `ApplyError` legible by construction. `SYNC_DAEMON_LOCAL_DB_FAULT_SITES` 3 → **6**, four new shape pins, **each verified to fail under the revert it names**.
- **Still open:** **#490** item 3 (two stderr-only signals — `blobd_error_line`, the serve thread's death — never reach the JSONL) · **#483** · **#484** · **#487** · **#488** · **#491** · **#492** · **#485** · **#476**.

**§5.9 parts C+D designed — *narrow the custody, never the reach* (2026-08-23; [ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md), spec v0.66 → v0.67; answers [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376), merges [#377](https://github.com/cairn-ehr/cairn-ehr/issues/377); opens [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494), [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495), [#496](https://github.com/cairn-ehr/cairn-ehr/issues/496); opens **#498** and **#499** from its own PR review; design-only — no code, no migration, SCHEMA stays 50).** #231's close (Slice 66) lifted the hard block. Full reasoning in the ADR and in `docs/superpowers/specs/2026-08-23-custody-narrowing-design.md`; the build is C1.

- **The ladder and its invariant.** Custody follows admission (default) → named **nodes** → named **actors**, with audited **break-glass at every rung**, under one rule: **narrowing changes the cost and the noise of reading, never whether content can be REACHED — at a node that holds the key or can reach one**. The bound is not decoration: a rung-1 break-glass is a NETWORK act, so a partitioned non-holder falls to honest disclosure and cannot reach it (**#498**, the one paper-parity row where this ladder loses to the envelope, which travels inside the file). Composition is **INTERSECTION** — forced, since ADR-0064's free inheritance holds only if adding an assertion can never widen — **and it can EMPTY** (**#499**: two honest chart-wide narrowings collapse custody to nobody and make every read loud), so **chart-wide narrowing is not in C1**. Node custody is the **norm** and per-clinician the **exception** — a blanket per-clinician policy makes work inside a location impossible (in an ED the team reads the chart) — which is also what keeps break-glass rare enough to mean anything. Making break-glass the route for the *normal* case would be §5.11's confirmation-dialog disease and §5.12's alert fatigue in one gesture.
- **The structural finding: no new key material.** `event_dek` already holds this node's wrapped copy and is REVOKEd from PUBLIC/`cairn_agent`, so a rung-2 break-glass is the in-DB floor admitting an actor and writing its audit row in the same transaction, and a rung-1 break-glass is ADR-0004's acquisition trichotomy. **The node's own DEK is the keyring and the floor is the glass** — no escrow tier, and the keyring is **LOCAL** (one reached over the network fails at 3am under partition). Hence **parts C and D are not separable and #377's stated dependency is backwards: the glass must exist before anything is sealed behind it.**
- **Custody is an additive field on the sensitivity assertion, not a new event type** — one gesture sets both dials (two would be independently *forgettable*, `M > N` against paper), ADR-0064's authority floor is inherited free (widening is protection-removing, expressed as withdraw-by-reference), and none of the four pinned registry row-counts move.
- **⇒ ADR-0064's handoff argument is CORRECTED while its conclusion is adopted.** The thread-resolution case has the well-custodied node computing the TRUE grade and the custody-less one over-protecting — decision 9 working. The real quiet leaks are **registry divergence** (A revoked actor Z, B has not → the same withdrawal is inert on A and authorised on B, so **B serves the DEK**; both HONEST) and replication lag. Recorded because the original argument would have justified hardening thread resolution, which is not where the leak is. Generalised: **a control that a faithful peer defeats *by computing correctly* is not weak — it is incoherent.**
- **⇒ Custody narrows on `event`/`patient`, never `thread`** — a custody-less node cannot resolve membership, and inheriting decision 9's bound would make break-glass routine on the nodes that see the patient least. **The bound is right for disclosure and wrong for custody**, the second ADR to hit that asymmetry (ADR-0064 decision 8 was the first): *"conservative" is a property of a direction, not of a value.*
- **⇒ The three-implementations-no-name rule is finally named: *refuse at a door only what that door can drop whole*.** Unparseable custody **holds nobody and the grade still STANDS** — fail-closed is affordable ONLY because the keyring guarantees reachability, and refusing the assertion would destroy protection with a malformed protection field. Unknown ranks MAX as in db/048/049 but for a **different** reason (it withholds quiet access, not protection), flagged so nobody carries the wrong justification into a fourth site.
- **⇒ Rung 2 is floor-enforced, not cryptographic.** Per-actor wrapping is available (`--author-as` is a passphrase-sealed file the node never holds, so ADR-0052's HKDF works one level down) but against node-level DB access it buys **noise, not protection** — that access can break glass anyway — while creating permanent unreadability with **no escrow** (ADR-0026 decision 4) and **no `erasure_shred_log` row to say so**. *An EHR may lose a record deliberately, audibly and by ceremony; never by a forgotten passphrase.* Deferred with its threat named; blocked meanwhile on a **reader identity** that does not exist (#496, §5.11 — today's surfaces attribute writes only).
- **Two ADR divergences found by checking rather than assuming.** **#494** — ADR-0052 decision 4 describes `event_dek` as `(event_id, holder, dek_wrapped)`; the built table has no `holder` column and `PRIMARY KEY (event_id)` structurally forbids the multi-holder custody that sentence says the design needed (erratum, not a migration). **#495** — ADR-0052 derives the node unwrap secret from the signing seed and says ADR-0026's escrow covers it, while ADR-0026 decision 4 says the signing key is **never backed up**; if both hold literally, every born-sealed body on a restored node goes dark. **CONFIRMED IN CODE the same day (fourth pass) — both do hold, and #500 was split out of it. See the DR-guarantee entry below.**
- **C1 (buildable now):** rung 1 (`custody.nodes`, both doors, serve-door withholding), the audited break-glass path, and the **in-chart location signal** — of the three notification directions (location / custodian / patient) it is the only one that actually restrains, and it needs no channel. Patient and custodian are §5.12 discharging obligations in part D, where the ADR records the DV hazard: *"sealed content on your record was opened at Clinic A"* delivered to a household phone reaches the person the record was sequestered against, with a pointer.


**The DR-guarantee audit — three promises, none of them true (2026-08-23, fourth pass; confirms [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495), opens [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500); adds `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` — 4 tests, all four mutation-checked — and corrects the expired comments at their source (`localstate.rs` header + `read_local_state`, `backup.rs::read_event_set`, and the stale justification on `tests/localstate.rs`'s emptiness assertion); no behaviour change, no migration, no ADR, SCHEMA stays 50).** #495 was filed from the ADR-0065 design pass with an honest caveat — *"I have not read the sealed local-state export design, so this may already be resolved there."* Reading it turned the suspicion into two confirmed defects.

- **ADR-0026 decision 1's three clinical promises are all false.** It guarantees, for total hardware loss of a solo node restored from the sealed medium: *"the **clinical event log survives**"*, *"**node-default data-at-rest keys survive**"*, *"**sealed-episode DEKs survive minus any erased ones**"*. Built: `backup.rs:138` exports `SELECT signed_bytes FROM node_event` (the federation plane), and `LocalState`'s two DEK slots are `Vec::new()` with `read_local_state`'s `_db` parameter **unused**.
- **#500 — the bytes.** The medium carries no clinical event at all. A solo clinic — the deployment ADR-0026 opens by naming as first-class, *"replication provides **zero** durability"* — backs up nightly, `verify-backup` passes, health is reported honestly, and restore recovers its peering history and **zero clinical records**.
- **#495 — the key.** `restore.rs` mints a fresh seed by design (decision 4); the X25519 unwrap secret is HKDF-derived from it (ADR-0052 decision 4), so every inherited `event_dek` row is unopenable. **Fixing either alone is useless**: one leaves a working key with nothing to open, the other sealed bodies with no key. #495 carries three fix options — escrow the secret / break the derivation / declare the loss — none symmetric, each superseding an ADR.
- **⇒ A DEFERRAL IS ONLY HONEST WHILE ITS STATED PRECONDITION HOLDS, AND NOTHING WATCHES FOR ONE EXPIRING.** `localstate.rs:10` declared its empty seam truthfully — *"the federation-node tier has no clinical surface yet"*. ADR-0052 made that sentence false and nothing reopened the seam, while this file went on recording slices A–D as ✓ done. The first defect here whose cause is a **true comment going stale**, and it is whole-record loss. **Every ✓ in this file rests on a sentence; the sentence is what to re-check.**
- **⇒ THE CEREMONY SUCCEEDING IS THE WORST SHAPE OF THIS BUG.** `main.rs:349` runs the local-state export on the live backup path, seals an empty bundle, writes the `.lsk` sidecar and reports success; `backup-status.json` records a true count of what the medium actually holds. **Every surface is honest and the composite is a precise untruth** — principle 4 violated by a system in which no single component lies.
- **⇒ WHERE A GUARANTEE IS ALREADY FALSE, PIN THE DEFECT, NOT THE PROMISE.** No `#[ignore]` exists in this crate and a permanently-red test would block the gate for every unrelated change, so the suite asserts what is true **today**, each assertion naming what it must be INVERTED to — the pinned-count idiom, where the guard failing IS the guard working. Anti-vacuity is explicit: the node is provisioned so the medium is genuinely non-empty, the `event_dek` row is written by the **production door** rather than the test, and the pure test asserts the happy-path unwrap *first* so the refusal cannot pass for the wrong reason. All four mutations verified red.
- **A design-level coupling worth remembering:** deriving the unwrap secret from the signing seed bought *"no new key-management mechanism"* (ADR-0052 decision 4) and paid for it with a contradiction against ADR-0026 decision 4 that **neither ADR could see from inside itself**. Cross-ADR claims about *the same key material* need checking where they meet, which is code.

## Phase 5 — Security & compliance core

- **Erasure = key-custody redistribution / crypto-shred** on the severity ladder ([ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9).
- **Visibility-scope ≠ replication; the safety projection** — sealed bodies emit de-identified, severity-graded safety projection; sensitivity is a graded append-only stream ([ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)).
- **At-rest seal** — ✓ done (ADR-0026 **slice A**): signing key sealed with a dual-recipient envelope (Argon2id
  KEKs from an operational passphrase + a one-time off-node recovery code; XChaCha20-Poly1305), recovery escrow
  minted at `init`, `seal-key` migration.
- **Backup-as-cold-peer** — ⚠️ **PARTIAL, not done** (ADR-0026 **slice B**): `backup`/`verify-backup` CLI +
  `last_backup` status; medium self-verifying via the existing signature invariant; fail-safe health sidecar.
  **But the medium carries `node_event` ONLY** (`backup.rs:138`) — the clinical event log is absent, so a
  solo node's restore recovers its peering history and zero clinical records, against ADR-0026 decision 1's
  *"the clinical event log survives"* and decision 2's *"clinical events back up as a cold peer"*
  (**[#500](https://github.com/cairn-ehr/cairn-ehr/issues/500)**, found 2026-08-23; pinned by
  `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`).
- **Restore-apply + new-identity `supersede`** — ✓ done at node level (ADR-0026 **slice C**, [issue #50](https://github.com/cairn-ehr/cairn-ehr/issues/50)):
  `cairn-node restore` rehydrates the `node_event` log into a fresh DB via a self-trusting `restore_node_event` door
  (empty-genesis fenced), mints a fresh key, records a `supersede`(dead→new); `db/009` op `supersede` + `node_lineage`.
  **Cold-medium self-identification** ([#53](https://github.com/cairn-ehr/cairn-ehr/issues/53)): a federated medium
  can't be self-identified from its convergent events, so the backup writes a container-level self-marker
  (`medium.rs`, `CAIRNB2`); `restore::resolve_dead_node` fail-closes on a peer/off-medium `--superseded-node`.
  **Live residual:** the commitment binds set *content*, so a peer's genuine marker spliced between
  **byte-identical converged** media is not rejectable — impossible on a sole-enroll medium, so multi-enroll
  restores report `Provenance::SignedFederated` → confirm-on-restore.
- **Sealed local-state export** — ⚠️ **container done, contents empty** (ADR-0026 **slice D**): a long-lived
  local-state DEK dual-wrapped once at provisioning; `CAIRNL1` export + a `CAIRNX1` `.lsk` sidecar;
  additive-CBOR `LocalState` with typed-empty slots + DB read/apply **seams** the clinical tier extends;
  signing key never in the bundle. **The seam's stated precondition — *"the federation-node tier has no
  clinical surface yet"* (`localstate.rs:10`) — EXPIRED when ADR-0052 made every clinical body born-sealed,
  and nothing reopened it.** `read_local_state`'s `_db` parameter is unused, so the export cannot see custody
  even in principle, while `main.rs:349` runs the ceremony on the live backup path and reports success over
  an empty bundle. With restore minting a fresh seed (decision 4) and the X25519 unwrap secret derived from
  it (ADR-0052 decision 4), **every born-sealed body on a restored node is unopenable**
  (**[#495](https://github.com/cairn-ehr/cairn-ehr/issues/495)**, confirmed in code 2026-08-23 — it carries
  the three fix options, none symmetric; each supersedes an ADR).
  **ADR-0026 slices A and C complete; B and D are partial — see #495/#500.** **Uniform key-material zeroization** ✓ ([#54](https://github.com/cairn-ehr/cairn-ehr/issues/54)):
  every transient KEK/DEK/seed/LSK in `Zeroizing`. Optional follow-on: escrow rungs (Shamir M-of-N, QR, TPM).
- **Trusted-time anchoring** — graded-interval `t_recorded` with clock-confidence grade; transparency-log multi-anchor existence proof ([ADR-0027](spec/decisions/0027-trusted-time-anchoring.md)).
- **Audit-log integrity, offline auth, mTLS** ([§7](spec/security.md)).

## Phase 6 — Federation hardening

- **Revocation cascade; anchor-as-power** ([ADR-0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md)).
- **DR / recovery escrow** — ✓ done at node level (ADR-0026 slices A–D, see Phase 5). Federation-tier
  follow-ons: peer-quorum (social) recovery + escrow rungs (Shamir M-of-N, QR, TPM/keyring).
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

---

## Above the foundation line (NOT in this roadmap)

- **Policy layer** — hard policy as a signed policy-assertion stream + effective-policy projection ([ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)); soft policy in UI. **GUI / reference UI** — built only on the same public native API everyone else uses (principle 12); paper-parity is the governing law, **no confirmation dialogs as a safety mechanism**. **Active-write thin encounters** and clinical workflow surfaces ([ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)).

## Parallel build-prep (not blocking the critical path)

- **Bet B — Pi compute-cost run** — **PASS twice on Pi 5 / 8 GB**: 2026-06-25 ([PR #57](https://github.com/cairn-ehr/cairn-ehr/pull/57), caveated by a USB-2 dock + PG16) and the clean 2026-07-07 re-run on PG 18.4 + a PCIe NVMe HAT with **both caveats resolved** — B1 p95 3.99 ms @ 2,004,000 events, B2 p95 4.5 ms/374-note chart; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on Cortex-A76). `cairn_pgx` is PG-18-capable (pgrx 0.18.1, [PR #56](https://github.com/cairn-ehr/cairn-ehr/pull/56)). **Only remaining follow-up:** fold the now un-caveated B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
- **Spike 0003 — Postgres on Android** — **Ran 2026-06-25, G0–G3 PASS**: native PG 18.2 + a cross-built pgrx extension (incl. SPI) on a stock Android 16 phone; validates the fractal-topology invariant at the phone tier. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/). Remaining gaps (from-source PG build, APK packaging) are non-load-bearing. **Continued clinical case-mining** stays the highest-signal mode for stress-testing the primitives before product build.
