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

**Slices 36–56 — condensed (2026-07-16 → 07-27: the 2026-07-15 whole-project review course — **fully closed** — its
Priority-6 design queue, and the first medication-coding slices; detail in git, the PRs and the linked ADRs).**

- **P2 sync-convergence integrity** (36–40, PRs #221–#225): the A→B convergence test over the real binaries and TCP
  (#199); the cairn-sync SCHEMA subset standing alone (#198); the clinical-plane `seq` cursor + periodic full sweep
  (#196, `db/036`); acked rows freed from the quarantine quota (#197); wire hygiene + `node.superseded` (#202/#201).
- **P3 — both wire windows shut** (41–43): **ADR-0051** contributor-role vocabulary floor (#203+#96);
  **[ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md)** born-sealed clinical bodies (#189+#92, `db/037`) —
  every clinical body sealed at write under a per-event DEK the node holds, a custody plane, sealed⇒clinical at both
  doors, a rung-3 shred CLI; **ADR-0053** per-write human authorship (#204, `cairn_authorship_bound`).
- **P4/P5** (44–45, PRs #251/#253/#255): the #188 schema-version downgrade guard in both loaders
  (`SCHEMA_GENERATION`, `SCHEMA_LOAD_LOCK`); `scripts/run-db-sql-tests.sh` in CI (#212); the registry `DO UPDATE` arm
  (#214); HANDOVER staleness (#215).
- **P6 design queue → five ADRs** (46–50): **ADR-0054** actor-registry federation is admit-and-dispute (#205, closes
  #154 structurally); **ADR-0055** the chained trust-root document (#206); **ADR-0056** unknown event types admitted
  uninterpreted (#200 — the filed premise was *inverted*); **ADR-0057** generic reprojection (#208, PRs #274/#278; one
  apply fn per projection, `cairn_replay_eligible` the #265/#266 seam); **ADR-0058** the grade-gated `t_effective`
  ceiling (#216, PR #285 — closing a latent one-event sync-wedge DoS).
- **Matcher** (51/53/54, Python): #209 fail closed on an empty non-match set; #210 retract orphaned proposals; #211;
  #290. **Slice 52 — the #217 paper-parity plan-section rule** (house rule 7; first live entry
  [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)).
- **55–56 — drug coding.** **ADR-0059** anchors drug identity on drugref's immortal `moiety_uuid` (INN is display,
  never key) as `substance.coding`, advisory + honest-degrading. Slice 6a (PRs #297/#298, `db/041`, generation 40→41):
  the `medication_coding_system` registry, `medication_coding` as its own projection, the `(system, code)`-pair
  dup-key, and a source guard that nothing executable references drugref. Sharpest review finding: a shred left
  `medication_coding` readable beside `patient_id` (ADR-0005 rung-3 / #92(b)).

**Still open from slices 36–56** — enumerated in full (see the header rule).

- **Sync:** #284. **Born-sealed / erasure (ADR-0052):** #230 · #231 · #232 · #233 · #234 · #235 · #236 · #237. #231
  (unwrap-cert kid pinning) landed as Slice 66, so custody follows admission; #232's parts **A/B shipped** (Slices
  65/67, discharging #294), the authority floor as Slice 68, and **parts C/D are DESIGNED (ADR-0065, v0.67), not
  built** (#376 answered, #377 merged into it; rung 2 #496, chart-wide narrowing #499, rung-1 offline glass #498).
- **Authorship (ADR-0053):** #242 · #243 · #244 · #245 · #247. Grading is half-live until #245; contributor-set
  authorship is key-scoped and does not survive rotation (#247 constrains #245); a `--author-as` event is *owned*
  under the ADR-0043 suppression gate.
- **ADR-0054/0055/0056 code (design-settled, none built):** ADR-0054 — #94, the key-loss-ceremony ADR, the rotate-key
  door; ADR-0055 — #257 · #258 · #259 · #260 · #261; ADR-0056 — #268 (#265/#266/#267/#269/#270 closed by Slices
  58/60). **The posture triad:** the content plane admits-and-disputes (0054) and admits-and-defers (0056); the code
  plane verifies-or-refuses (0055).
- **Reprojection (ADR-0057):** #272 (the authoritative Pi5/NVMe same-rig re-run), #275, #276, #277 (heal cannot
  re-derive `DO NOTHING` projections). **Trusted time (ADR-0058):** #279 · #280 · #281 · #282 · #283. **Registry
  hygiene:** #254 (8 twin-check registrations still `DO NOTHING`). **Deps:** #252 closed by retiring iced; #317; #389.
- **Medication/matcher:** #287 (hub-scale sweep cost), #288 (med-list sign-off as ONE gesture — the human
  **measurement** remains), #294 (discharged by Slice 67), [#334](https://github.com/cairn-ehr/cairn-ehr/issues/334)
  (repaired by R1, PR #688), #331 · #333 · #335 · #336 · #337 (Slice 61 follow-ons).

**Operational caveats that outlive these slices.** Pre-ADR-0051 event logs and pre-ADR-0052 plaintext `clinical.*`
bodies **REFUSE at db/020** — **wipe dev/PoC rigs**, never sync them through. Pre-wire unsigned actor rows never sync.
Test DBs need `cairn_pgx` ≥ 0.3.0.

**Slices 57–69 and the August tech-debt passes — condensed (2026-07-28 → 08-22; full detail in git, the PRs and the
linked ADRs — the *why* is in each ADR and is not restated here).**

- **57 — medication 6b: the coding-overlay event types** ([ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)
  decision 3; `db/042`; closed #295, #296). A **strike NULLs the anchor** (*"not that, and I don't know"*). Lessons: a
  redundant projection column is a convergence hazard; `array_agg` KEEPS NULLs. Open: the coded↔uncoded case; #294;
  [#300](https://github.com/cairn-ehr/cairn-ehr/issues/300).
- **58 — the ADR-0056 floor: admit uninterpreted, re-adjudicate before power** (PR #302; closes #265, #266;
  `cairn_readjudicate_deferred`, db/043). *Refusal hides, admission cannot.* Open: [#301](https://github.com/cairn-ehr/cairn-ehr/issues/301)
  (the node/actor plane still fail-closes), [#308](https://github.com/cairn-ehr/cairn-ehr/issues/308),
  [#309](https://github.com/cairn-ehr/cairn-ehr/issues/309).
- **59 — floor determinism** (PR #311 closes #75): the twin blank-test was collation-dependent;
  `cairn_twin_is_present` spells the 25 Unicode `White_Space` points.
- **Interlude — the loop ran unattended (07-31 → 08-01).** Nine PRs; closed #79, #11, #100, #119, #120; loop fixes PRs
  #316, #321, #325. Open: #312 · #314 · #315 · #317 · #322 · #326 · [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327).
- **60 — the residual refusal contract, clinical plane** (closes #267/#270): a deliberate refusal on verifiable bytes is
  penned by digest and auto-released. **`P0001` is a contract with the pull loop** (PR #371 node plane, #370 clinical).
  *Symmetry between two planes is a hypothesis, not a goal* — the naive [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)
  alignment would be a defect.
- **61+62 — the med-list node tier and WINDOW** ([ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md),
  v0.62): the first clinical READ path; iced retired for `cairn-gui-tauri`; write cost median **222 ms**; sign-off per
  LINE (#339), reaching the transaction layer (#342). *A unit-tested safety control can still be defeated by the
  surface that calls it; a compensating control outside CI is not a control* (#444). Owes #288. Open: #331 · #332 ·
  #333 · #335 · #336 · #337 · #340.
- **63 — the search-before-create funnel** ([ADR-0061](spec/decisions/0061-registration-is-an-act-that-carries-its-search.md),
  v0.63, `db/045`/`db/046`): the attestation NAMES the displayed candidates. Open: #346–#357, #359–#362; worth naming
  **#349**, **#351**, **#352**, and the §1.2 write-cost half **#360**.
- **64 — closing the funnel's bypass** (closes #345): db/005 step 8b, a chart's first event is its registration;
  retiring `patient.created` was the load-bearing half. Unfloored: #364, #365.
- **65 — the §5.9 sensitivity stream, part A** ([ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
  v0.64, `db/048`): effective grade = the **max** of event/thread/chart; unknown ranks MAX; erratum E6. Open: #374 ·
  #378 · #379 · **#436**; parts C/D **#376**/**#377** (ADR-0065).
- **66 — custody follows admission** (closes #231): the unwrap-cert `kid` pinned to `trust_peer`; **withhold the key,
  never the bytes**. `unsound = "all"` in both `deny.toml` trees; **#389**'s review date lives in its reason.
- **67 — the §5.9 safety projection, part B** (closes #375; [ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md),
  v0.65): **the seal boundary is the coarsening boundary**; `safety_class_map` ships EMPTY. Fixed #404. Open:
  [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394) · #395 · #397 · #398 · #399 · #400 · #401 ·
  [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402) · [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) ·
  [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407).
- **68 + two interludes** ([ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md), v0.66; closes #380,
  #412, #405, #426): one `cairn_claim_authority` predicate gates effect, never admission; 7 of 11 production mutations
  had survived a green suite. A column `REVOKE` cannot narrow a table `GRANT` (db/049 §8 — #425, #427, #432);
  `classify_authorship_confidence` graded a forgery `Attested` (#428); 21 headers gained `, pg_temp`. Open: #408 · #409
  · #413 · #414 · #415 (expect it to fire on routine care) · #416 · #417 · #418 · #419 · #420 · #422 · #430 (~100
  unpinned invoker-rights functions) · #431.
- **69 + two passes — the §5.9 operator surface** (closes #388, #383, #421, #435, #387, #439, #382, #385, #381):
  `patient-sensitivity` — **NAME, NEVER COUNT**; `TargetState::OnAnotherChart` must never collapse (residual **#436**);
  `cargo doc` blocking (#439); a NULL `proacl` is the PERMISSIVE case (#382). Residual: #441.
- **The silent gates (08-20 → 08-21; closes #446, #442, #443, #449–#453, #386; opens [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447)).**
  Nine gates that could pass without running (`cargo_lockfiles_tracked.rs`, `db_gate_actually_ran.rs`). Not done:
  unifying the 342 bare skip sites (#327).
- **The freeze that hid and the flake that lied (08-21; closes #370, #457; opens [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458)).**
  `cairn_learn_attachment_refs` had nine freeze paths on malformed signed bodies — a signature proves authorship, not
  well-formedness.
- **The db-error legibility sweep (08-22; closes #460, #465, #467, #469, #471, #473, #474, #475; `db/050`; opens #461 ·
  [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463) · [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464) ·
  #468 · [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470)).** Constrain an envelope field where it is MINTED,
  read it permissively where it ARRIVES. **`tokio_postgres::Error`'s `Display` IS `"db error"`** — `legible_db_error`
  renders `message [SQLSTATE] — DETAIL — HINT`; **never "tidy" `LocalDbFault` into an `anyhow!`**; `EXCEPTION WHEN
  OTHERS` does not catch 57014; force a write failure in a shared test DB with a LOCK under a short `lock_timeout`.

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

**2026-08-23 → 09-12 — the sweep's tail, the misclassification cluster, §5.9 C+D designed, the DR audit, DR slices
1 → 2d, the restore's §1.2, #511, the closing-keyword guard, #527 and the CodeQL pack — condensed.** The *why* of each
lives in its ADR (0065–0068) and design doc under `docs/superpowers/`; HANDOVER's traps hold the rules that still bite.

- **The sweep's tail (08-23; closes [#481](https://github.com/cairn-ehr/cairn-ehr/issues/481), [#479](https://github.com/cairn-ehr/cairn-ehr/issues/479), [#477](https://github.com/cairn-ehr/cairn-ehr/issues/477)).**
  A guard runs only when its crate is tested (`db_gate.rs`); one pure `operator_chain`. Opened #485 (89 postgres call
  sites naming no operation) and #487–#492.
- **The misclassification cluster (08-23; closes [#489](https://github.com/cairn-ehr/cairn-ehr/issues/489), [#482](https://github.com/cairn-ehr/cairn-ehr/issues/482), [#480](https://github.com/cairn-ehr/cairn-ehr/issues/480), #490 items 1–2)** —
  *a failure wearing another subsystem's clothes* (`PullIntegrityError`; a type outranks a kind). Open: **#490** item
  3 · **#483** · **#484** · **#487** · **#488** · **#491** · **#492** · **#485** · **#476**.
- **§5.9 parts C+D designed — *narrow the custody, never the reach*** ([ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md),
  v0.67; answers #376, merges #377). Opened #494 (ADR-0052's `event_dek` erratum), #495, #496, **#498**, **#499**.
  *A control a faithful peer defeats by computing correctly is incoherent; "conservative" is a property of a
  direction; refuse at a door only what that door can drop whole.*
- **The DR-guarantee audit (08-23; confirms #495, opens [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500), [#502](https://github.com/cairn-ehr/cairn-ehr/issues/502)).**
  ADR-0026 decision 1's three clinical promises were all false while every surface reported honestly — *the composite
  was a precise untruth*. *A deferral is honest only while its precondition holds.*
- **DR slice 1 — identity dies with the disk; custody must not** ([ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md),
  v0.68; closes #495): an independent X25519 unwrap keypair in `<key>.unwrap`; custody rides `CAIRNL1`; `restore`
  adopts, never mints (traps 1–2). Open: #504 (dead `_node_sk` — a decision) · #505 (a second recovery code) · #506 ·
  #507 · #508 (CBOR leaves unwiped copies of the unwrap secret) · #509 · #512 (the `M > N` paper-parity defect) · #513.
- **#503 — `cairn-keystore`: `cairn-sync` loads the node's custody key (08-30; closes [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503))**
  through a pure decision table (trap 3). Opened #514 (retire the fallback) · #515 (the binaries disagree on the
  signing-key file format) · #516 · #517 (no test starts the daemon from a PROVISIONED key) · #518 · #520 · #521.
  *Breakage hid from a gate three ways* → `scripts/run-db-gated-tests.sh`; *run a plan's verbatim code against the
  gates first*.
- **DR 2a + 2b — the medium format and the transport seam (08-31 / 09-02; `cairn-medium`, `cairn-wire`; #101 item 1
  only).** `CAIRNB3`; `Transport` as the one seam. *19 of 19 mutations survived a round-trip suite* → `wire_pins.rs`
  golden bytes. Open: #531 · #532 · #534 (a freeze stops content convergence) · #535 · #536 (an unopenable DEK is
  counted nowhere on sync) · #537 · #538 · #556 (`segment_commitment` does not bind the attestation — **free only until
  a release ships a CAIRNB3 writer**) · #557 · #558 · #559 (operator messages naming the wrong remedy) · #560 · #561 ·
  #562 · #563.
- **#511 — the custody newtypes (09-04; opens #541, #543, #544, #545).** `Secret32`/`PublicKey32` make
  public-for-secret a compile error; they do NOT separate secret roles (trap 5).
- **The closing-keyword guard (09-04; reopens #101, #115, #434, #441, #468, #500, #534).** GitHub closes on ADJACENCY;
  `scripts/check_closing_keywords.py` + the workflow scan title, body and commits. Residual **#444**, #547, #548.
- **DR slice 2c — the medium carries the clinical record (09-06; `db/051`; closes [#524](https://github.com/cairn-ehr/cairn-ehr/issues/524),
  #500 as titled — its READ half became #554 — and [#523](https://github.com/cairn-ehr/cairn-ehr/issues/523), whose
  merge also closed #182, #404, #430, #431 and #441 — the last three open again, as is #522; #550 opened and closed
  in-branch).** A backup reproduces the state at CAPTURE TIME (trap 7); the shred predicate has ONE home; `seq` holes
  are routine (#549 owes the operator surface); a torn tail must NOT refuse `restore`. Open: #551 · #552 (a capture is
  O(whole medium)) · #553 (an unmarked foreign legacy medium can be destroyed by succession).
- **DR slice 2d — the record comes home** ([ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), v0.69;
  `db/052`; closes #554; opens #567–#571). **The headline test DECRYPTS a body**; the pen moves into the database and
  gains custody; the gap pin inverts to `a_clinical_event_restores_from_a_medium`. **2e is retired as a label.**
- **The restore's §1.2, measured** ([ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md),
  v0.70 — provenance warns, never gates; closes #571; opens #572; PR [#573](https://github.com/cairn-ehr/cairn-ehr/pull/573)):
  **100 003 events in 116.7 s against 600 s**; #552 confirmed; `M > N` stands (#512).
- **CodeQL advanced setup + a model pack (09-12; PR [#576](https://github.com/cairn-ehr/cairn-ehr/pull/576); corrects #562):**
  one `barrierModel` row per NAME-heuristic source, 44 → 3. Not touched: #575 (a real secret on stderr); required is #444.
- **#527 — a discriminator is not a salt (09-02; opens #529, #530).** CodeQL picks its sink by the binding's NAME;
  renamed to `lineage`, `crypto_sink_names_are_genuine.rs`, house rule 6b. #530: `PairingBundle.nonce` is signed and
  never read (a decision). #529: no daemon path prints a patient id — by accident, not by rule.

## Phase 5 — Security & compliance core

- **Erasure = key-custody redistribution / crypto-shred** on the severity ladder ([ADR-0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md), principle 9).
- **Visibility-scope ≠ replication; the safety projection** — sealed bodies emit de-identified, severity-graded safety projection; sensitivity is a graded append-only stream ([ADR-0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md)).
- **At-rest seal** — ✓ done (ADR-0026 **slice A**): signing key sealed with a dual-recipient envelope (Argon2id
  KEKs from an operational passphrase + a one-time off-node recovery code; XChaCha20-Poly1305), recovery escrow
  minted at `init`, `seal-key` migration.
- **Backup-as-cold-peer** — ✓ **COMPLETE** (ADR-0026 **slice B**): `backup`/`verify-backup` + `last_backup` status.
  Since **DR slice 2c** the medium is a CAIRNB3 image carrying **both planes**; since **2d** a restore applies the
  clinical plane through the `db/020` door, installs the carried custody and pens what it cannot key, so ADR-0026
  decision 1's *"the clinical event log survives"* and decision 2's *"clinical events back up as a cold peer"* **may
  be cited as met** (#500 → [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554);
  `dr_clinical_guarantee_gap.rs::a_clinical_event_restores_from_a_medium`, `medium_carries_both_planes`). Since then
  (Phase 8/9 below): ADR-0069, #568, #567, #593, #584 (ADR-0070), #594 (ADR-0071), #614/#615 (ADR-0072), #619
  (ADR-0073), #621 (ADR-0074). **What remains is not slices:** #575 (re-deferred), #611, #602, the races #603/#604,
  #589 · #590 · #591 · #592, #596 · #597 · #598 · #599, #605–#610, #616 · #617, #608's late-custody half, and
  **node-plane completeness accounting** (a slice of its own).
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
  `cairn_blob_verify`, thin wrappers over the same `cairn_event::blob_address` L2 uses — one hashing implementation):
  the BLAKE3-vs-address check is restated **in-DB** as a trigger floor on `blob_store`, so a raw-SQL client can no
  longer store arbitrary bytes as a named blob (principle 12). **Honest limits:** `blob_chunk` rows and `outboard` are
  NOT in-DB verified — wrong chunks can only assemble into a whole-blob flip that FAILS the floor, and a wrong
  outboard is rejected by the fetching peer's bao decode (availability degradation, never an integrity hole).
- **Resource-isolated byte tier** — chunked/preemptible/separately-budgeted; can never starve clinical sync; opt-in byte replication; self-verifying swarm fetch.
- **Rendition set** — the binary's legibility twin (retrievability axis); per-blob DEK crypto-shred inherits.

## Phase 8 — Native API contract (the boundary below the application) · Phase 9 — Terminology

- **Native API: capability-described + conformance-tested, evolves additively** ([ADR-0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md)); the four-layer boundary sits *below* policy/UI ([ADR-0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)).
- **Author-scoped export** — the medico-legal copy ([ADR-0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md)). **FHIR interop façade** — distinct from the native API ([§9.7](spec/language-substrate.md)).
- **Phase 9 — ICD-11 canonical interlingua + local-terminology overlay** ([ADR-0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md)).

**2026-09-11 → 09-20 — the DR close-out and the node-plane doors, condensed.** Each ADR (0069–0074) and plan
(`docs/superpowers/plans/2026-09-1*`) carries the argument and review ledger; HANDOVER's traps 8–14 carry the rules.

- **The restore takes its recovery code from a file** ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md),
  v0.71; closes #572 and #570; PR [#574](https://github.com/cairn-ehr/cairn-ehr/pull/574)): `--old-recovery-code-file
  <PATH>`, read in the step-0 pre-flight. The refusal the design started with was dropped because *"no cron-run
  command reaches `print_recovery_code`"* was already false → **#575**.
- **`requeue` releases custody (09-12; closes #568)** — five DB tests drive the binary and assert a body OPENS; *arms
  are indexed by input pair, not by outcome*. **`requeue` never counts a release it did not get** (closes #578–#581;
  PR [#582](https://github.com/cairn-ehr/cairn-ehr/pull/582)): released only when custody is SETTLED (trap 8),
  in the EXISTING `db/052`. Filed #585 (nothing reads Postgres notices) · #586 (two older source guards skip after a
  file's first `#[cfg(test)] mod`) · #587 (`cairn-sync` does not replay schema); a predecessor's fixture failed the
  gate (#583).
- **`verify-backup` asks the clinical-plane question (09-13/14; closes #567; PR [#588](https://github.com/cairn-ehr/cairn-ehr/pull/588))**
  — `backup SHORT` only on evidence. Residuals #551 · #589 · #590 · #591 · #592; commented on #559 and #549.
  **Operators: run it AFTER `backup`.**
- **2d's last six design tests (09-14; closes #593 and, by a lockfile bump, #600 — RUSTSEC-2026-0285, both lockfiles;
  PR [#595](https://github.com/cairn-ehr/cairn-ehr/pull/595)).** Thirteen mutations run, thirteen killed. Opened #594 and
  #596 · #597 · #598 · #599.
- **A late key reaches the chart** ([ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md), v0.72; builds #584;
  PR [#601](https://github.com/cairn-ehr/cairn-ehr/pull/601)): both doors call `cairn_project_late_custody` after the
  substitution guard (traps 9/10). Filed #602 (any client can set `cairn.remote_apply` before `submit_event`) · the
  races #603, #604 · #605 (an in-place `db/` edit is unprotected by the #188 guard) · #606 · #607 · #608 (the
  substitution guards' `<>` fail-open) · #609 · #610.
- **A restore that left records behind exits INCOMPLETE** ([ADR-0071](spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md),
  v0.73; builds #594; PR [#612](https://github.com/cairn-ehr/cairn-ehr/pull/612)): five causes share exit 3; exit 1 =
  BLOCKED (trap 11). Filed #611 · #613 · #614 · #615 · #616 · #617. *A fix written under the pressure of a finding is
  itself unreviewed code.*
- **A restore loses no record silently** ([ADR-0072](spec/decisions/0072-a-restore-loses-no-record-silently.md), v0.74,
  errata E1–E2; `db/053`; builds #614 + #615; PR [#618](https://github.com/cairn-ehr/cairn-ehr/pull/618)): one
  `cairn_refuse_substitution`, `IS DISTINCT FROM` (trap 12). Its census error became #619.
- **The node plane refuses a substitution and pens it** ([ADR-0073](spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md),
  v0.75; builds #619; PR [#623](https://github.com/cairn-ehr/cairn-ehr/pull/623)): trap 13; the declared survivor M10
  was killable via a `SET ROLE` seam — **"untestable" is a claim.** Filed #620 (the COSE unprotected header lies outside
  the signature — wire core, a decision) · #621 · #622 (catalogue guards blind to `BEGIN ATOMIC`) · #624 · #625. Left
  open: #605 · #268's other classes · #301 · #569 · node-plane completeness accounting · #608's late-custody half.
- **A deterministic door failure is a refusal, not a fault** ([ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md),
  v0.76; builds #621): the three node doors are total; the puller pens a non-local non-`P0001` failure (trap 14). Filed
  #626 (the clinical plane has the identical shape; db/020 is the 100k-event hot path) · #628 (`cairn_body`'s `22P05` on
  a NUL) · #629 · #631 · #632 · #633 · #634 (#630 closed as a duplicate of #625).

### 2026-09-21 → 09-26 — search fragments and speed (#636 slice 1, #639); the funnel UI 2a → 2c (PRs #646, #653, #661, #674); #671 (ADR-0075, PR #678)

- **#636 slice 1** widened `db/046` pass 3 in the safe direction (punctuated tokens project their PARTS; a ≥3-**byte**
  prefix via `starts_with`, never `LIKE`); monotonicity **proved** (0 lost, 9 gained); `patient_search_drift.rs`
  makes sweep-paired ⊆ search-found executable. **#639** took the Pi floor **1528.4 → 883.3 ms** and the spread
  **777.9 → 14.6 ms** at 50,000 real names (traps 15–17; `scripts/measure_patient_search.py`). *A zero-row search
  measures nothing.* Filed #637 (the materialised token table — the fix for the ~860 ms floor, its own slice) · #638
  (closed) · #640 · #641 (the parts branch cuts a Devanagari or Thai name at its marks) · #643 (the rig times
  `count(*)`). §1.2: paper 3 → forced 2 → target 2.
- **The funnel UI** (design `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md`; no ADR, no
  migration). **2a** the pure core (`cairn-gui-funnel`: the advisory trigger, `PROMPT_CAP = 5`, the counted-custody
  token `AttestedSearch`; 21/21 mutations killed). **2b** the live ports (`DataError::Refused`, #648; #658). **2c's
  prerequisites** (#661): `TokenStore::settle` (#659), `MockData::fail_next` (#660), `DeliberateRefusal` (#651), one
  enrolment rule (#654). **2c — the window** (#674): the search now RANKS (it showed the five OLDEST charts);
  `FunnelSession`, the `funnel/` module, runbook §8; identifier entry → #672; the header shows age (#673). Review: a
  sign-off signed whichever chart was OPEN → every chart command names the displayed chart; #675 fixed; #677 decided.
  Still open: #355 · #645 · #647 · #649 · #650 · #652 · #655 (`42501`/`42P01`/class-23 land in `Unavailable`) · #656 ·
  #657 · #658 · #662 (seven `init` effects unpinned) · #663 · #664 / #666 · #665 · #667 · #668 · #669 · #670 · #672 ·
  #673 · #676. Also cited: #442, #450, #583, #636, #638. §1.2: register 5 → 4 → 4; find 3 → 2 → 2.
- **#671 — the step-3 prompt is a nudge, not a completeness claim** ([ADR-0075](spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md),
  v0.77). **Maintainer's decision:** duplicates are common and the desk cannot be made to browse, so make repair by
  `link` easy — the safety measure is how fast a duplicate is FOUND. `search.incomplete` = the SEARCH was partial;
  the cut is `PromptList::withheld`, never signed; seven-key ranking (#349). Measured: every single slip in the five
  (500/500); a surname typo AND a wrong DOB 203/500 — the repair path's residue (representative re-run #685). Filed
  [#679](https://github.com/cairn-ehr/cairn-ehr/issues/679) · [#680](https://github.com/cairn-ehr/cairn-ehr/issues/680) ·
  [#681](https://github.com/cairn-ehr/cairn-ehr/issues/681) (the repair path, below) · #682 (an NFD word-final accent
  is lost) · #683 (`CandidateList`'s partiality as one sum type) · #684 (`1980-3-7` misses `1980-03-07` — a SET gap) ·
  #685 · #686 · #687 (fixed). §1.2: 1 → 1 → 1; a warning that fired on 92% of registrations is gone.

### 2026-09-27 — the duplicate repair path designed; R1, the combined read, built (ADR-0076, PR #688)

Merged 2026-09-27. [ADR-0076](spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md),
spec **v0.78**; design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (slices **R1 →
R5**, each with its own plan, PR and §1.2 section); plan `docs/superpowers/plans/2026-09-27-repair-path-r1-combined-read.md`;
`db/054_person_charts.sql`, `SCHEMA_GENERATION` 53 → **54** (#284). ADR-0075's *"repair by `link` is easy"* rested on
four missing things; ADR-0076 answers all four, R1 builds the first.
- **Built:** `cairn_person_charts(uuid)`, the set-wide `cairn_medication_duplicate_groups(uuid[])`, `ChartSet`,
  `patient/person.rs`; the read selects groups by MEMBERSHIP (**#334 fixed**); sign-off attests each thread under its
  OWN chart and refuses a changed chart set; `ChartPane` with member lines and per-row source labels; the doubted-link
  withholding (db/054 `cairn_chart_set_has_doubted_link`, containing [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220)
  for this read; widened by R1b); an unheld linked chart reads `unknown`. Tests: `person_charts.rs`,
  `medication_duplicate_groups.rs`, `medication_dup_key_drift.rs`, `combined_read.rs` (incl. the golden
  `a_never_linked_chart_reads_exactly_as_before`), `combined_signoff.rs`.
- **Filed:** #689 (db/034 admits an attestation naming another chart) · #690 (reconciling across LINKED charts — a
  decision) · #691 (a short per-member tag) · #692 · #693 · #694 · #695 · #696 · #697 (decided (b); built in R1b).
  Commented on [#333](https://github.com/cairn-ehr/cairn-ehr/issues/333).
- **Next:** R2a, R2b-1, R2b-2, **R1b** (#697 (b) + #701, PR #717) — entries below, all merged; **R3** (the front door
  collapses by person, PR #721 — entry below, awaiting merge). Then **R4** (per-node matcher worker, #679 — proposes,
  never links) and **R5** (banner + worklist, #680; #700, #723). Plan each from the design page's section.
- **§1.2:** two folders of one patient clipped together. Reading a linked chart 1 → 1 → 1; signing off a combined list
  1 → 1 → 1. Budget: opening a linked chart ≤ the single-chart open (runbook pass, a human act).

### 2026-09-27 — repair path R2a: a human's link judgement outranks a machine's (PR #698)

Merged 2026-09-28; plan `docs/superpowers/plans/2026-09-27-repair-path-r2a-link-precedence-floor.md`; ADR-0076
decisions 4–5; `db/055`, `SCHEMA_GENERATION` 54 → **55** (#284).
- **Built:** `patient_link.attested` (db/018) and `cairn_link_overlay_wins` — attested first, then HLC; db/055's
  backfill, generation heal and own re-fold (#703); `chart_link` `link_charts`/`unlink_charts` with a human `Reviewer`,
  reading back `LinkOutcome::effect` (`TookEffect`/`Outranked`/`StillJoined`) in its own transaction, one lock order
  (proposal row, then CARNLK); `auto_apply` answers `AlreadyJudged`; CLI `link-charts` / `unlink-charts`. Tests:
  `link_precedence.rs`, `chart_link.rs` (incl. a `pg_stat_activity` lock-order test), `auto_apply.rs`.
- **Filed:** #699 (neither-held unlink; decided (a) in R2b-2) · #700 (auto-apply's judged-pair skip races; a skipped
  proposal stays `pending` — R5's worklist must filter it) · #701 (db/054 should read `pl.attested`) · #702 (a floor
  refusal reads as a bare `db error` — addressed) · #703 · #704 · #705 · #706.
- **§1.2:** paper 3 → forced 3 → target 3 (2 from R5's banner); review-and-link ≤ 20 s owed by the runbook pass.

### 2026-09-29 — repair path R2b-1: "Same person as…" (link) built (PR #707)

Merged 2026-09-29; plan `docs/superpowers/plans/2026-09-29-repair-path-r2b1-same-person-as.md`; the design page's
as-built note lists every deviation. No ADR, no SQL object.
- **Built:** `cairn_node::patient::compare` (`chart_facts`; `cross_vetoes` over BOTH displayed sets); `chart_link`'s
  pre-check refusals as marked verdicts (#702); `cairn-gui-tauri/src/link/{view,mod,search}.rs` and `src-ui/link.js`
  (findings first, "This record"/"Other record", **Link — same person**). Link names BOTH compared sets; every panel
  message goes through `setMessage`; a `/review-pr` round fixed 7 Important defects. Tests: `patient/compare.rs`,
  `tests/chart_compare.rs`, `link/view_tests.rs`, `link/mod.rs`; the panel walked headless (no JS harness, #332; the
  view tests split at 500 lines, #467 guard).
- **Filed:** #708 (`link_charts` should re-check both compared sets in its transaction) · #709 (a link outcome can go
  unseen when it lands after the chart changed) · #710 (review-round residuals).
- **§1.2:** paper 3 (fetch, lay side by side, clip) → forced 3 (find → Compare → Link) → target 3 (2 from R5's banner);
  review-and-link ≤ 20 s, runbook §9 — a human act.

### 2026-09-30 — repair path R2b-2: "Not the same person…" (unlink) and #699 (a) built (PR #711)

Merged 2026-09-30; plan `docs/superpowers/plans/2026-09-30-repair-path-r2b2-not-the-same-person.md`;
**[ADR-0077](spec/decisions/0077-an-unlink-may-be-filed-under-the-record-it-was-judged-from.md)** (spec **v0.79**)
records #699 (a). No SQL object.
- **Built:** `chart_link/admit.rs` (pure `FiledUnder::{Subject, RecordOf}`) and `judge.rs` ("still joined?" asked of
  the SUBJECTS; a `RecordOf` re-check under CARNLK); CLI `unlink-charts --from`; `patient::edges::record_edges`; the
  pane's per-LINK "How these charts are linked" list with `compare_linked` / `unlink_records`; `src-ui/unlink.js`;
  runbook §10. Tests: `admit.rs`; `tests/unlink_from_record.rs`, `tests/record_edges.rs`, `tests/chart_link.rs`; a
  deterministic CARNLK race test.
- **Filed:** #713 (a retry after a committed Outranked can silently overrule a colleague) · #714 (`CanonicalPair` /
  `JudgedFrom` newtypes) · #715 (`link.js` drift guard) · #712 (`chart_set.rs` over 500, wiring tests).
- **§1.2:** paper 2 (unclip, annotate) → forced 1 → target 2; review-and-unlink ≤ 15 s, runbook §10 — a human act.

### 2026-10-03 — repair path R1b: a doubted set withholds every line not on the opened chart (#697 (b), #701; PR #717)

Merged 2026-10-03; plan `docs/superpowers/plans/2026-10-03-repair-path-r1b-doubted-link-withholds.md`; design page
section R1b + its as-built note. db/054's `cairn_chart_set_has_doubted_link` body changed; generation stays **55**.
- **Built:** `WrongChartReasons { outside_set, doubted_link }` beside the kept, fail-safe `cross_patient`;
  `is_wrong_chart_hazard()` / `hazard_reasons()` (status-blind) / `withheld_reasons()`; `WithheldLine`;
  `DOUBTED_LINK_INSTRUCTION` (`cairn-medication-view`). The pure rule `medication/hazard.rs::wrong_chart_reasons`:
  in a doubted set every line not recorded ONLY on the opened chart is withheld. db/054 reads the stored
  `pl.attested` (#701) and counts an ATTESTED unlink between two charts still in the set as a doubt (the A–C–X
  bridge). Per-reason wording in the window and the CLI (`list_text`, one `doubted_link_note` below the list).
- **Tests:** `hazard.rs`/`read.rs` units (mutation-checked); `tests/doubted_link_withholds.rs` (10 DB tests incl. the
  mirror, #220's path, #701, the bridge with a line on the bridge chart, an un-attested unlink is no doubt); a golden
  for the unchanged outside-set CLI strings.
- **Filed:** #716 (the window cannot confirm a standing link, nor show which link is in doubt) · #718 (db/054's
  SECURITY DEFINER no longer has a reason) · #719 (residuals) · #720 (`wrong_chart` the only Rust source of truth).
  Commented #335 (the doubt state can change between display and sign-off).
- **§1.2:** two clipped folders, one page in doubt. Reading 1 → 1 → 1; sign-off 1 → 1 → 1; lifting the hold 1 → 1 → 1
  **per doubted link** (confirm CLI-only until #716); in the A–C–X bridge CONFIRMING A–C or C–X never lifts it.

### 2026-10-03 — repair path R3: the front door collapses by person (ADR-0076 decision 6; PR #721)

Built on PR [#721](https://github.com/cairn-ehr/cairn-ehr/pull/721), awaiting merge. Plan
`docs/superpowers/plans/2026-10-03-repair-path-r3-front-door-by-person.md`; design page section R3 + its as-built note
(and the SDD ledger's rulings R1–R11). No SQL object, no wire change; `SCHEMA_GENERATION` stays **55**. Maintainer's
decisions in the brainstorm: each member line is its own open target; browse collapses too, not only the prompt.
- **Built:** `cairn-patient-search` — `PersonRow` (never empty; a JSON array of candidates), the pure
  `group_by_person` (a row sits at its best-ranked member; unmatched members follow, oldest first; a missing component
  is `MissingComponent`, never a row of one), `CandidateList.people`, `charts()` and `displayed_charts()` (the ONE
  list `SearchAttestation::from_displayed` signs), `TrustState::Unknown`. `cairn-node` — `patient/search_person.rs`
  (`read_components`, one `unnest` × `cairn_person_charts` statement that refuses a component without its own chart;
  `display_name_for`; `trust_state_for`), and `patient/candidate_text.rs` (the CLI's lines out of `main.rs`; a linked
  member prints as "↳ linked: …"). A chart not held here reads `Unknown` and "(registration not yet received here)"
  and no longer sets the signed `incomplete` flag; a HELD nameless member still does. `cairn-gui-funnel` — the prompt
  cap counts ROWS and never splits one (`PromptCounts::shown_charts`). `cairn-gui-tauri` — `funnel/rows.rs`
  (`PersonRowView`, "One person — N linked charts", `people_phrase`, `charts_suffix`), an open button per member,
  `AppState::shown` holds every member, the label announced via `aria-describedby`; the link panel's search collapses
  too ("This record (N charts) also matched and is not listed"), and `link.js`'s search read is guarded (`found`).
- **Tests:** `person.rs` units; `tests/search_by_person.rs` (8 DB tests: a linked pair is one row, the matched member
  leads its row, best-member placement, an unmatched linked chart shown and signed, a member not held here, a
  never-linked golden, a matched chart not held here, and the end-to-end registration that reads `search.displayed`
  back); `prompt.rs`'s row-cap test; goldens for every single-chart sentence (`view.rs`, `candidate_text.rs`,
  `link/search.rs`); `commands.rs`'s open tests; the webview-fields guards.
- **Filed:** #722 (the `--mock` window has no linked pair, so a person row cannot be walked in `--mock`) · #723 (the
  front door calls a doubted set "One person" — belongs with R5's doubt work).
- **§1.2:** paper counterpart: the card index, where two folders found to be one patient are clipped into ONE slot.
  Finding 1 → 1 → 1; opening 1 → 1 → 1; registering after the prompt 1 → 1 → 1. `M ≤ N`; no new act. One added query
  per search (the component read); budget unchanged (find ≤ 5 s, register ≤ 20 s, runbook §8 — a human act). The load
  falls: one person takes one of the prompt's five places instead of two.

## Above the foundation line (NOT in this roadmap)

- **Policy layer** — hard policy as a signed policy-assertion stream + effective-policy projection ([ADR-0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)); soft policy in UI. **GUI / reference UI** — built only on the same public native API everyone else uses (principle 12); paper-parity is the governing law, **no confirmation dialogs as a safety mechanism**. **Active-write thin encounters** and clinical workflow surfaces ([ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)).

## Parallel build-prep (not blocking the critical path)

- **Bet B — Pi compute-cost run** — **PASS twice on Pi 5 / 8 GB**: 2026-06-25 ([PR #57](https://github.com/cairn-ehr/cairn-ehr/pull/57), caveated by a USB-2 dock + PG16) and the clean 2026-07-07 re-run on PG 18.4 + a PCIe NVMe HAT with **both caveats resolved** — B1 p95 3.99 ms @ 2,004,000 events, B2 p95 4.5 ms/374-note chart; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on Cortex-A76). `cairn_pgx` is PG-18-capable (pgrx 0.18.1, [PR #56](https://github.com/cairn-ehr/cairn-ehr/pull/56)). **Only remaining follow-up:** fold the now un-caveated B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
- **Spike 0003 — Postgres on Android** — **Ran 2026-06-25, G0–G3 PASS**: native PG 18.2 + a cross-built pgrx extension (incl. SPI) on a stock Android 16 phone; validates the fractal-topology invariant at the phone tier. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/). Remaining gaps (from-source PG build, APK packaging) are non-load-bearing. **Continued clinical case-mining** stays the highest-signal mode for stress-testing the primitives before product build.
