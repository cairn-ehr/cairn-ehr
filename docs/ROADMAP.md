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
**Slice 13 — §5.1/§5.7 identity linkage core (piece C1)** (`cairn-event/src/identity.rs`, `db/018_identity_linkage.sql`,
SCHEMA 16→17): first slice of the closed §5.7 algebra. Pure `LinkAssertion` builder; two additive event types
`identity.link.asserted`/`identity.unlink.asserted` through the reused `submit_event` door; `cairn_check_link_assertion`
culture-neutral floor (distinct valid UUID subjects + non-empty provenance; self-link rejected); HARD authored-twin in
the `cairn_event_twin` hook. `patient_link` HLC-overlay edge table (canonical `(low,high)`, latest-HLC-wins, out-of-order
convergent — now with the #115 content_address tiebreaker); `person_member` golden-identity projection (`person_id` =
min-UUID of the component via `cairn_recompute_component`, correct on merge **and** unmerge/split, fail-loud oversize
guard `cairn_max_component_size()` GUC). 15 DB-gated tests. **Additive, no floor bypass, no SCHEMA/ADR/spec change**
(settled §5.1/§5.7/ADR-0014). Deferred: an accept-at-cap boundary test for the oversize guard.

**Slice 14 — §5.2/§5.7 match_proposal→apply seam (piece C2)** (`db/019_apply_proposal.sql`, SCHEMA 17→18;
`cairn-node/src/apply_proposal.rs`): a **human-accepted** advisory proposal becomes a real **human-attested**
`identity.link.asserted` event through the C1 door. **Key property: no floor change** — the link is additive, but
placing a responsibility-bearing contributor trips the existing db/005 attestation gate, so C2 composes settled §5.7
(C1) + ADR-0030 + ADR-0014 verbatim; `submit_event` untouched, no new event type. Additive `applied_event_id UUID`
column; pure `compose_provenance`/`build_attested_link_body` + IO `apply_accepted_proposal` (read `FOR UPDATE` →
sign+attest → 3-arg `submit_event` → mark applied, one txn = idempotency; canonical `(low,high)`). 6 tests, green.
**Additive, no ADR/spec change.** Deferred: **C2b** auto-apply of `auto_candidate`; matcher as a compositional
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

**Slice 20 — §5.4 John-Doe registration front door (slices A + B)** (composes built primitives; **no new event type,
no `db/` floor change, no SCHEMA/ADR/spec bump**). **A — callsign + matcher placeholder exclusion** (prior session,
PR #123/#125): `cairn-event::john_doe::callsign` mints `Unknown-<class>-<site>-<date>-<suffix>` (UUID-derived suffix,
partition-safe), `cairn-node::john_doe::register_john_doe` composes a `use_key='callsign'` name assertion + C4's
`identity.pending.asserted` in one txn (chart renders *unconfirmed*), a `register-john-doe` CLI, and the advisory
matcher excludes placeholder `use_key` from blocking + scoring (`use_key <> ALL(%s)`), with a cross-language
`CALLSIGN_USE`↔placeholder-set drift guard (issue #124). **B — clinician-observed evidence (estimated-age range +
observed sex), full loop** (this session): the demographic spine already carries it (db/011 dob field requires
`facets.precision` + accepts `facets.basis`, `clinician-observed`=rank 30). Pure `cairn-event::evidence`
(`birth_year_range_from_age` → a time-invariant birth-year *range*, never raw age nor a false-precise midpoint —
principle 4; `estimated_dob_body` value `"<min>/<max>"`/precision `"year-range"`; `observed_sex_body` on
`administrative-sex`, not the `sex-at-birth` birth fact a clinician can't know); `cairn-node::evidence::assert_observed_evidence`
+ an `assert-observed-evidence <patient-uuid>` CLI (authors on an EXISTING chart, one txn, provenance
`clinician-observed`; `register-john-doe` unchanged; `db::next_hlc` promoted `pub`); advisory matcher made
**range-aware + POSITIVE-ONLY** — `DateValue` interval, `parse_dob` reads `"year-range"`, `compare_dob` overlap→PARTIAL /
no-overlap→INSUFFICIENT_DATA / **never DISAGREE** (a soft estimate supports but never suppresses a match). 8 pure
`cairn-event` + 4 DB-gated `cairn-node` integration (`observed_evidence.rs`) + matcher pure (`DateValue`/`parse_dob`/
`compare_dob` incl. inclusive touching-boundary + symmetry) + a DB-gated matcher e2e (`test_observed_age_pipeline.py`).
Full workspace + matcher suites green (cargo 0 failed / clippy clean; matcher 226 passed / ruff clean, PG18 + cairn_pgx
0.2.0); end-to-end CLI smoke on a provisioned node passed (`dob=1981/1991` year-range clinician-observed +
`administrative-sex=male`, `chart_trust=unconfirmed`); opus whole-branch review READY-TO-MERGE. **Honest limit
(recorded):** slice B's range is a *scoring* signal, **not a blocking key** (`"1981/1991"`→first-4-digit `1981` won't
block a 1985-born candidate; a John Doe's only name is an excluded callsign) — the estimate helps once a pair is blocked
by another key (belongings identifier, refined name, hub sweep). **Remaining §5.4:** ~~photo/marks/belongings/EMS-context
evidence~~ **(done — slices 26/27, below)**, the "prior history now available" push-alert (§5.12, no notification tier),
the search-before-create funnel (UI/API tier), ~~a birth-year-*range* blocking pass~~ **(done — slice 21, below)**, a
readable sequential callsign suffix, a `--observed-year` override, `identify`→optional-link resolution flow.

**Slice 21 — §5.4 birth-year-range blocking pass + A/B pass-toggle** (this session; advisory Python only — **no `db/`
floor change, no SCHEMA/ADR/spec bump**; design+plan under `docs/superpowers/{specs,plans}/2026-07-04-dob-range-blocking-pass*`).
Closes slice 20's recorded honest limit: a `year-range` dob now generates blocking keys. Two **additive, ANCHORED**
passes in `pipeline/db.py` (`_RANGE_GROUPS_SQL`): **`dob-range`** — a `birth_window` CTE gives every chart an inclusive
birth-year interval (range rows via `facets.precision='year-range'` + NULL-safe `substring` year extraction,
**evaluation-order-proof** — a malformed value can never crash the sweep; point rows via the first-4-digit run,
year-range excluded so a range never double-enters as a false point); anchors = range charts; members = window-overlap
(range↔point AND range↔range — two John Does at two sites, the only key that pair can share); pairs are **anchor×member
ONLY** (all-pairing a window would manufacture C(k,2) noise — new pure `pipeline/blocking.py::pairs_from_anchor`);
**`dob-range+sex`** — the same join ∩ a shared blocking-sex value (**UNION** of `sex-at-birth` + `administrative-sex`,
so the trans case still groups; `unknown` sentinel excluded — no-data-is-never-agreement), the additive **rescue** when
the plain window block exceeds the cap (skipped+reported, hub sweep the backstop). Plus the **A/B pass-toggle**
(`enabled_passes` on `generate_candidate_pairs`; unknown pass name raises — a silent typo would fake a measurement) and
an honesty fix (`birth_year` CTE excludes `year-range`, so `"1981/1991"` no longer leaks `1981` into `name+year` as a
fake birth year). TDD: 9 pure (`test_blocking_passes.py`) + 14 DB-gated (`test_dob_range_blocking.py`) + 3 toggle tests.
Fable whole-branch review → fix wave (order-proof guard, unknown-sentinel exclusion) → **READY TO MERGE**; a post-PR
8-angle review→adversarial-verify wave (PR #131) then fixed 7 more: eval-harness `KeyError` guard vs resident charts
(the #84 crash arm), shape-aware `dropped_pair_estimate` (s−1 for anchored skips, not C(s,2)), `blocking_sex` sentinel
param-bound from `adapter.VALUE_SENTINELS` + explicit whitespace trim-set, exact-`dob` arm excludes `year-range` (A/B
purity), statement-level toggle skip, SQL↔registry pass-name guard, `canonical_pair` deduped into pure `blocking.py`.
Suites pure 200 / DB 264 / ruff clean. ~~**Honest limit (recorded, [issue #130](https://github.com/cairn-ehr/cairn-ehr/issues/130)):**
the pure-age John Doe pair now *blocks* but still scores below `review=3.0`~~ **(closed — slice 22, below)**.
**Deferred:** ~~generator range-DOB emission + range-aware
eval mirror (the quantitative recall number the toggle now enables; must also mirror `administrative_sex` — slice 22's
composite-sex fallback is unrepresentable in the eval `DatasetRecord` until it does)~~ **(done — slice 23, below)**;
fuzzy near-window softening; hub-tier range sweep.

**Slice 22 — §5.4 administrative-sex scoring + the unconfirmed-chart REVIEW rule** (2026-07-05; closes
[issue #130](https://github.com/cairn-ehr/cairn-ehr/issues/130); advisory Python only — **no `db/` floor change, no
SCHEMA/ADR/spec bump**; design+plan under `docs/superpowers/{specs,plans}/2026-07-05-admin-sex-scoring*`). The
design-critical arithmetic: scoring administrative-sex alone leaves the headline pure-age Doe pair at ≈1.79 < `review=3.0`
(honest F–S weights can't be inflated past an 11-year window + a two-valued field), so the slice has TWO halves. (1) **The
composite `sex` field**: `records.SexValue` + pure `compare_sex` — both charts carry `sex-at-birth` → old EXACT/DISAGREE
semantics (a birth-fact clash stays negative evidence, aligned with db/016); otherwise a **positive-only union fallback**
over {`sex-at-birth`, `administrative-sex`} (intersect→EXACT, disjoint→INSUFFICIENT_DATA, **never DISAGREE** — observed
evidence supports but never suppresses; mirrors blocking's union-sex); one field one contribution (no correlated
double-count); weight key `sex-at-birth`→`"sex"` (projection field names untouched); `load_candidate` reads both facets
in ONE query; side rank = sab's if present else admin's (documented second-order approximation). (2) **The scoped forcing
rule** (`banding.band(unconfirmed=)`, the known-alias-forcing precedent): a pair with a `chart_trust='unconfirmed'` chart,
**≥2 positive-LEVEL fields, zero DISAGREE** → REVIEW even below threshold — never AUTO; fires **with vetoes attached**
(post-review amendment: the original no-vetoes gate rested on a false "near-vacuous" premise — an identifier veto needs
no verified values, so a vetoed-yet-corroborated Doe pair is reachable, and suppressing it would be the ADR-0014
auto-reject); `'under-review'` (a dispute) deliberately does NOT trigger it; per-Doe volume bounded by the blocking cap;
every persisted proposal involving an unconfirmed chart carries an `{"kind":"identity_pending","unconfirmed":[uuids]}`
evidence marker (`"kind"` = the one non-field-evidence discriminator, the alias-marker convention; worklist grouping).
Trust plumbing mirrors aliases: one batch loader `db.load_trust_for` (sweep preloads; propose's per-pair fallback is the
same one-query loader), canonical lowercase-uuid keys for map + marker. TDD: pure + DB-gated incl. the #130 e2e
(`test_identity_pending_pipeline.py` — the headline pair surfaces as REVIEW; a no-pending control (shared seed helper)
proves the RULE, not sex scoring, surfaces it, hardened non-vacuous; a direct-`propose()` test covers the on-demand
trust seam; the two-Does pair carries both uuids in the marker).
6-task subagent-SDD each reviewed clean; **final whole-branch review (fable): 0 Critical/Important**, 2 test-only
must-fixes fixed → re-review READY TO MERGE; then an **8-angle post-review fix wave** (veto-gate removal;
`_corroborated_positive` counts agreement LEVELS so learned weights can't stand the rule down; `score()` raises on a
weights table missing a compared field — the stale-table/key-rename hazard — instead of silently zeroing; marker key
`"rule"`→`"kind"`; singular `load_trust` deleted; the stale-forced-REVIEW retraction gap filed as
[#135](https://github.com/cairn-ehr/cairn-ehr/issues/135)). Suites pure 227 / DB 298 (full) /
ruff clean. **Honest limits (recorded):** a pending+disputed Doe reads `'under-review'` (severity-max view) and bypasses
the forcing rule while the dispute is open — deliberate, per db/024 semantics; ranking within a Doe's surfaced candidate
list is the worklist tier's job; weights/thresholds remain shipped defaults (B3 learning unblocked); forced-REVIEW rows
persist after the Doe is identified ([#135](https://github.com/cairn-ehr/cairn-ehr/issues/135)); ~~the eval mirror cannot
yet represent `administrative_sex` (folded into the deferred range-aware eval-mirror work, slice 21 above — B3
weight-learning needs it first)~~ **(closed — slice 23, below)**.

**Slice 23 — B3 eval mirror: generator range-DOB emission + administrative-sex representation** (2026-07-05/06;
advisory Python only, eval-harness tier — **no production matcher/pipeline/floor change, no SCHEMA/ADR/spec bump**;
design+plan under `docs/superpowers/{specs,plans}/2026-07-05-eval-range-adminsex-mirror*`). Unblocks B3
weight-learning: the harness now carries the field set the shipped matcher scores and blocks on. Four additive parts:
(1) `DatasetRecord.administrative_sex` plumbed through the REAL adapter (`candidate_from_rows(admin_sex_row=)`) so the
pure scorer eval exercises slice 22's composite-sex fallback (pinned by an sab-vs-admin `field_comparisons` EXACT
test); (2) pure `_birth_window` mirror of `_RANGE_GROUPS_SQL`'s birth_window CTE + an anchored range-overlap branch in
`shares_blocking_key` (overlap ∧ ≥1 side is_range; `dob-range+sex` needs no separate branch — same overlap join,
subset), plus a fix for a live over-claim found in design: the exact-DOB branch compared raw values with no precision
guard, so two identical `year-range` strings faked an exact key the SQL excludes (`IS DISTINCT FROM 'year-range'`);
(3) `corrupt_dob_estimate` generator operator — dob → inclusive birth-year window CONTAINING the current value's first
4-digit run (tol 2–5 ⇒ the 5–11-year §5.4 widths, provenance 30), sex moved sab→`administrative_sex` (observed facet;
random draw when the seed recorded none), knob `p_dob_estimate=0.15`, LAST in `_OPERATORS`; `_repair` now stands down
on window-overlap pairs (pinned by identity: `repaired is clone`); (4) drift canary extended to `_RANGE_GROUPS_SQL`
(3-tuple table; the exact-dob exclusion pinned by a dob-arm-unique two-clause fragment after review found the one-line
literal occurs twice in `_GROUPS_SQL`; containment check whitespace-normalized so a cosmetic reindent of the SQL can't
trip it) + `seed_dataset` writes `administrative-sex` rows + two DB-gated proofs: the
`dob-range+sex` rescue sees seeded admin-sex under `enabled_passes` isolation, and an estimate-heavy volume set
(`p_dob_estimate=0.9, p_name=0.9`, n=150, >100 range clones asserted non-vacuous) measures `pair_completeness == 1.0`
— the end-to-end proof the mirror never over-claims what the SQL recovers. 5-task subagent-SDD, each reviewed;
**final whole-branch review (fable) found 1 Critical**: Python's `$` matches before a trailing newline, POSIX's does
not, so `"1980/1990\n"` got a window the SQL rejects — the exact over-claim class the slice exists to close; fixed by
de-anchored `re.fullmatch` (+ the canary now pins BOTH overlap-join bounds); re-review READY TO MERGE. Suites pure
253 / DB-gated 326 (full) / ruff clean. **Honest limits (recorded):** the mirror still ignores the block-size cap
(volume proofs run under a large cap; `evaluate_blocking` reports skips honestly); a `p_dob_typo`-shifted year windows
around the typo (honestly unrecoverable by the range key alone; `_repair` restores a name token — safe direction);
same-seed generator output differs from pre-slice output (reproducibility within a version, not cross-version
stability); gold_v1.json deliberately untouched (synthetic data carries the new fields).

**Slice 24 — B3 weight-learning: supervised Fellegi–Sunter estimation** (2026-07-07; advisory Python, eval tier only —
**no production matcher/pipeline/floor change, no SCHEMA/event/ADR/spec bump**; design+plan under
`docs/superpowers/{specs,plans}/2026-07-06-b3-weight-learning*`). The learner the shipped
`DEFAULT_WEIGHTS`/`DEFAULT_THRESHOLDS` comments always pointed at. Closed-form supervised F-S: count agreement levels
across labelled pairs → `m/u` (additive-Laplace-smoothed; INSUFFICIENT_DATA excluded → 0, principle 4;
provenance-blind — the `provenance_factor` stays an orthogonal score-time multiplier) → `weight = log2(m/u)`, the same
math as `scoring.score` run backwards from ground truth. Four new pure modules under `matcher/src/cairn_matcher/eval/`:
`learner.py` (`estimate_weights`, `derive_thresholds`, `learn_model`, `LearnedModel`/`LearnMetadata`), `crossval.py`
(deterministic entity-cluster k-fold — split on WHOLE clusters so no match pair straddles train/test — + held-out
before/after lift, skips folds whose training partition has no match pairs), `model_io.py` (`LearnedModel`↔JSON,
`ModelIOError` on malformed input), `learn.py` (the `python -m cairn_matcher.eval.learn` CLI); + a behavior-preserving
`scorer_outcomes` extract in `scorer_eval.py`. **Thresholds are safety-first, anchored to the best impostor:**
`auto = max(non-match)+margin` (zero false auto-links by construction; `margin` guarded `>0` — a non-positive margin
collapses/inverts the gap), `review = max(non-match)` (surface above the top impostor, never below → `review<auto`
always), and `recall_target` is an honest **conflict diagnostic** (`collided` = the safe placement can't meet the
recall floor), never a lever that drags `review` into impostor range. 6-task subagent-SDD; the Task-2 implementer
caught a real plan bug (the original recall-cut `review` inverted on cleanly-separated data) → corrected before
coding; **final whole-branch review (opus) found 1 Important** (`margin<=0` false-auto hole) → guarded; re-review READY
TO MERGE. Suites pure 288 passed / 73 skipped / ruff clean. **Honest limits (design §8):** ships the *mechanism*, NOT
new shipped weights — the gold demo actually scores *worse* than the hand-tuned defaults (tiny set, noisy k-fold,
in-sample overlap; `collided=True`), so a **large hand-crafted gold-set re-run** is the deferred authoritative
follow-up; synthetic-learned weights reflect the generator's corruption model; veto-blind (end-to-end the veto only
lowers a band — safe); no pipeline consumer loads a learned model yet (advisory desk artifact).

**Slice 25 — B3 compound blocking keys: `dob+first-initial` + `name+sex`** (2026-07-07; advisory Python, eval/matcher
tier only, no floor/spec bump; condensed — full detail in git + PR #138). Two additive symmetric compound passes
(registry 6→8): `dob+first-initial` (birth-year + first char of each name token — a first-initial relaxation of the
name requirement, genuinely new recall for transpose/diacritic/misspelling variants) + `name+sex` (name token +
normalized sex — the oversized-unisex-name-block per-sex rescue; the only name rescue that fires for the John-Doe
population). Shared CTE fragments extracted to avoid sex-normalization drift. Suites pure 297 / DB 375 green. Honest
limit: `name+sex` gain invisible to the uncapped metric (proven by a targeted over-cap DB test); lift on SYNTHETIC
data only, real magnitudes await the large hand-crafted gold set (deferred slice-24 follow-on).

**Slice 26 — §5.4 photo evidence + the day-one §3.14 attachment-reference shape** (2026-07-08; **ADR-0042**, spec
v0.42→v0.43; design+plan under `docs/superpowers/{specs,plans}/2026-07-08-attachment-shape-and-photo-evidence*`).
The FIRST content-addressed **attachment** on a clinical surface — forced finalizing the ONE can't-retrofit piece of
ADR-0013 (also lands the Phase-7 attachment-reference shape). 9-task subagent-SDD, final whole-branch review "ready to
merge" (0 Critical/Important); workspace 418 passed / 0 failed, clippy clean. **(1) Shape:** `AttachmentRef` stub →
`Attachment{descriptor, renditions:[Rendition{role,alg,digest_hex,media_type,byte_len,inline?,seal?}]}` +
`SealRef{alg,dek_wrap}` (`cairn-event/src/attachment.rs`) — all five §3.14 reserves; rendition set nested
(structurally can't-retrofit), seal + inline reserved-None; field order frozen by ADR-0042 (reconciles ADR-0041's
note `payload.media` — one shared primitive, two carriers). `EventBody.attachments: Vec<Attachment>`, empty-vec
byte-identity proven. **(2) Floor:** `db/027` `cairn_learn_attachment_refs` walks `attachments[*].renditions[*]`
(skips inline); db/005 + db/020 call the one shared helper (no drift). **(3) Author path:** non-demographic
`identity.evidence.asserted` (`db/028` registers it — fail-closed floor; twin from descriptor never pixels);
`cairn-node/photo_evidence.rs` (pure `prepare_local_blob` + atomic `assert_photo_evidence` — blob stored present
through the db/026 verify trigger + event authored in ONE txn, `ON CONFLICT DO UPDATE` fills a placeholder) + an
`assert-photo-evidence` CLI. **Honest limits:** plaintext (seal reserved), single `original` rendition (no preview),
bytes local (cross-node fetch deferred), POC harness diverges. **Review fixes applied:** honest-descriptor rule
moved into the library (`validate_photo_descriptor`, not only the CLI); a direct db/020 apply-door attachment test
added (both doors now directly cover `cairn_learn_attachment_refs`); local-blob size-guard gap lodged as
[#141](https://github.com/cairn-ehr/cairn-ehr/issues/141) (§6.6 byte-tier slice). Residual (benign): DO-UPDATE
overwrites caller `media_type` on an already-present row.

**Slice 27 — §5.4 marks/belongings/EMS-context text identity evidence** (2026-07-08; design+plan under
`docs/superpowers/{specs,plans}/2026-07-08-marks-belongings-ems-evidence*`). Three text-shaped `kind` values —
`mark`, `belongings`, `ems-context` — on the **existing** `identity.evidence.asserted` event type (the photo slice's
non-attachment sibling). **No new migration / floor / SCHEMA / ADR / spec change** — the type is already registered
(`db/028`), additive, non-demographic (db/015 carries the authored twin verbatim); the observation is free text in the
payload, so `attachments` stays empty (zero-attachment content-address preserved). Pure `cairn-event::identity_evidence`
additions (`MARK`/`BELONGINGS`/`EMS_CONTEXT_EVIDENCE_KIND` + `TEXT_EVIDENCE_KINDS` closed set + `parse_text_evidence_kind`
typo-drift guard + `text_evidence_body` `{kind,provenance,description,basis?}` + `render_text_evidence_twin`); new
`cairn-node::identity_evidence` author path (`validate_description` honest-content floor **in the library**; pure
`build_text_evidence_body`; one-statement `assert_text_evidence` — no blob tier).
Provenance fixed `clinician-observed` (relayed/hearsay in `basis`); `description` required-non-empty (floor refuses an
empty claim; UI defaults are soft policy, principle 12). 4-task TDD; e2e + CLI smoke green; clippy clean.
**Review follow-up (PR #142):** the slice-26 photo command and this slice's text command were **folded into one**
`assert-identity-evidence --kind photo|mark|belongings|ems-context …` behind a new pure, unit-tested
`route_identity_evidence` flag gate. **Honest limits:** free-text description only; no projection/worklist/matcher signal.
**Remaining §5.4:** the "prior history now available" push-alert (§5.12, no notification tier), the search-before-create
funnel (§5.3/§5.8, UI/API tier); ~~an `enroll-human` ceremony CLI~~ **done — slice 30 below**;
~~a readable callsign suffix~~ + ~~`--observed-year`~~ done in slice 28 below; ~~`identify`→optional-link~~ **done —
slice 29 below**.

**Slice 28 — §5.4 finishers PR#1: node-local John-Doe ordinal + `--observed-year`** (2026-07-11; design+plan under
`docs/superpowers/{specs,plans}/2026-07-11-john-doe-ordinal-and-observed-year*`; **no new event type / migration-floor /
SCHEMA / ADR / spec change** beyond one read-only VIEW). **Finisher 1 — node-local friendly ordinal:** the callsign
identity string stays UUID-suffixed (partition-safe — a per-day counter was deliberately NOT used, it races on a
partition), so a new read-only `db/030_john_doe_local_ordinal.sql` VIEW derives "this node's John Doe #N" from
`event_log` — `row_number()` **PARTITION BY `node_origin`** over each node's own callsign registrations
(`demographic.field.asserted` + `field=name` + `facets.use=callsign` + `provenance=system:john-doe-registration`),
ordered by the collation-free `(hlc_wall,hlc_counter,content_address)` spine. Node-local by construction (foreign
registrations land in their own partition); never signed/on-the-wire/an-identity. `register_john_doe → (Uuid,String,i64)`;
CLI prints `local ref: John Doe #N (this node)`. All-time (no daily reset → no TZ semantics). **Finisher 2 —
`--observed-year`:** pure `resolve_observed_year(Option<i32>,current_year)` bounds a supplied year to `1900..=current`
(reject future/absurdly-historical; principle 4), defaults to today; feeds the already-parameterized
`assert_observed_evidence(...)` — computed DOB range only, **not** `t_effective` (deliberate scope boundary).
Subagent-driven TDD (new DB-gated partition/callsign-only test + 5 pure tests); full workspace green; fmt+clippy clean.
**Finisher 3 (`identify`→optional-link) done in slice 29 below.**

**Slice 29 — §5.4 finisher 3: `identify` → optional link** (2026-07-11, PR #165; design+plan under
`docs/superpowers/{specs,plans}/2026-07-11-john-doe-identify-optional-link*`; **no new event type / migration / floor /
SCHEMA / ADR / spec change**). The last structural finisher: the node surface that RESOLVES a John-Doe chart (type/
floor/overlay/builders already existed — this added the Rust path). New `cairn-node::identify`: device-additive
`build_identify_body` (flips chart *unconfirmed*→*confirmed*) + the async `identify_patient` orchestrator authoring the
identify and, on `--link <prior>`, an OPTIONAL human-attested `identity.link.asserted` (reusing
`apply_proposal::build_attested_link_body`) — **both in ONE atomic transaction** (a link rejection rolls the identify
back). `attester_is_enrolled_human` advisory pre-check on `actor_current` (mirrors the db/005 floor). New
`identify-patient` CLI. Subagent-driven TDD (5 DB-gated + 2 pure); whole-branch review clean. **§5.4 structural
finishers 1–3 all done.**

**Slice 30 — §5.4 `enroll-human` ceremony CLI** (2026-07-11; design+plan under
`docs/superpowers/{specs,plans}/2026-07-11-enroll-human-ceremony-cli*`; **no migration / floor / SCHEMA / ADR / spec
change** — additive Rust reusing the `enroll_actor` db/004 floor). The `identify --link` prerequisite: enrols a
clinician's key as a `kind='human'` actor with an ADR-0044 person-distinguishing determinant, and drops the raw-SQL
human enrollment from `identify.rs` tests (house rule 5). New `cairn-node::enroll`: pure
`build_human_pinned(role, registration_id?, handle?)` — requires ≥1 determinant (principle 4), **never pins the key**
(rotate-key stability, ADR-0011 §5); async `enroll_human_actor` — a **dual-mapping guard** (one key→>1 `actor_current`
row makes db/005 NULL that key's authorship node-wide) + an advisory ADR-0044 collision pre-check over the floor. New
`enroll-human` CLI: pre-I/O + pre-mint validation, mint-if-absent personal key (sealed + recovery code, or
`--insecure-plaintext`; no `.lsk` node-escrow). Subagent-driven TDD (6 pure + 7 DB-gated `tests/enroll_human.rs`);
whole-branch review (opus) **Ready = YES**, 0 Critical/Important, 3 Minor fixed; a post-PR `/review` pass then fixed 4
further Minors (documented the (entity, role) actor model so `--role` splits are understood as intended not a bug;
extracted the pre-mint collision check to a DB-gated library fn; softened the "no stray key" claim to best-effort;
documented the load-branch unseal). Full workspace green (cairn-node lib 117 + all DB-gated incl. enroll_human 7/7 &
identify 5/5 · cairn-event 86 · cairn-sync 18); fmt + clippy --workspace + mkdocs clean. **Follow-ups:
[#166](https://github.com/cairn-ehr/cairn-ehr/issues/166):** the dual-mapping guard's
accepted TOCTOU (concurrent enroll of the SAME key under DIFFERENT actor_ids — floor lock is actor_id-keyed, not
key-keyed); documented as accepted, durable fix is a floor-level per-key guard in db/004. **[#168](https://github.com/cairn-ehr/cairn-ehr/issues/168):**
make the entity→role-actor (1:many) relationship first-class (today implicit via a shared `registration_id` pinned
into each role-actor).

**Slice 30 — `clinical.medication`: the first clinical-content event stream** (2026-07-12; branch
`feat/medication-recording-slice-1`; **no ADR/spec/SCHEMA/floor-contract/wire change** — graduates
data-model §3.15/§3.16 + the "union + flagged for reconciliation" line into product code; design+plan under
`docs/superpowers/{specs,plans}/2026-07-11-medication-recording-*`). Distinct from slices 1–29 above: those
are all *administrative/identity* data about the patient (demographics, matcher, identity algebra,
John-Doe); this is the first event stream carrying actual *clinical content* — what medication the patient
is on. Two append-only verbs over an immortal `medication_id` thread: `clinical.medication.asserted`
(schema `clinical.medication/1`) + `clinical.medication-cessation.asserted`
(`clinical.medication-cessation/1`). `cairn-event::medication` pure builders — substance ref is mandatory
`term` + nullable `inn_code` + formulation (principle-4 uncertainty floor: only `term` mandatory, all else
honest-unknown); free-text `DoseUnit` with a recommended vocab; `info_source` provenance-of-claim. New
`db/031_medication.sql`: the structural floor (`cairn_check_medication_assertion` + the shared
`cairn_event_twin` hook — non-empty `term` + `info_source`, valid `medication_id`); `medication_statement` /
`medication_cessation` kept as **separate projections** so they're arrival-order-independent (an orphan
cessation renders nothing until its assert arrives, then surfaces in `patient_medication_past`); the
`patient_medication{,_current,_past}` views union across sources with staleness visible via assert date;
the **E1 deterministic advisory reconciliation flag** (view `patient_medication_reconciliation_flag`;
`coalesce(inn_code, normalized term)` — advisory only, cleared by ceasing a duplicate; fuzzy brand↔generic
deferred). `cairn-node::medication` orchestrators
(`assert_medication` / `cease_medication`, both device-additive) + `medication-assert` / `medication-cease`
CLI verbs; end-to-end CLI smoke passed live. Cessation is offline-first (no requirement that the local node
has already seen the corresponding assert). Subagent-driven TDD (6 tasks); full workspace green — fmt +
clippy `--workspace -D warnings` clean, all tests pass incl. **DB-gated `tests/medication.rs` 9/9**
alongside the existing cairn-node/cairn-event/cairn-sync suite. **Post-review fix:** `asserted_at` derives
from the convergent `hlc_wall` (t_recorded), not the local `updated_at` fold clock, keeping the staleness
signal honest and node-independent (regression-tested). **Deferred:** dose-correction/change
overlay; fuzzy reconciliation (brand↔generic, typos, salts); reconciliation *resolution* as a first-class
event; a `delete` rendering-suppression visibility overlay; structured sig/frequency (lands with
prescriptions); the Tier-A dictionary + autocomplete + DDI; a separate `route` field; active
review/last-confirmed staleness; the [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-triple
collision advisory extended onto the medication projections; human-attested clinical responsibility on a
medication statement (slice 1 is device-additive throughout).

**Slice 31 — medication dose overlay (slice 2 of `clinical.medication`)** (2026-07-12; branch
`feat/medication-dose-overlay-slice-2`; **no ADR/spec/SCHEMA/floor-contract/wire change** — graduates the
slice-30 §8 deferral into product code; design+plan under
`docs/superpowers/{specs,plans}/2026-07-12-medication-dose-overlay-*`). Two new **additive** verbs over the
existing `medication_id` thread: `clinical.medication-dose-change.asserted` (titration — both doses true over
effective time) + `clinical.medication-dose-correction.asserted` (a recorded dose was wrong; references the dose
event it fixes via a plain `corrects` UUID — **not** the existence-forcing `target_event_id`, so it stays
offline-first). New `db/032_medication_dose.sql` (**db/031 UNTOUCHED**): the structural floor
(`cairn_check_medication_dose` + two `cairn_event_twin` branches reproduced verbatim; both types
`additive`/`targets_other_author=FALSE` — a correction is additive, **not** suppressing, so the ADR-0043
owner-gate does not apply and cross-author correction is ungated with the original preserved). A **dose timeline**:
`medication_dose_event` (point-0 seeded from the assert by a 2nd additive trigger + one row per change,
`ON CONFLICT DO NOTHING` idempotent) + `medication_dose_correction` (HLC-wins overlay keyed by the **target** dose
event, offline-first orphan convergence). `medication_current_dose` picks the **latest-EFFECTIVE** point (bitemporal
§5.1: `cairn_dose_effective_sort_key` — ISO-lexical string, null→recording-time; then HLC/`content_address`, all
`COLLATE "C"` → fully node-convergent, a backdated change never overrides a real later one).
`patient_medication_dose_history` = the titration trail; `patient_medication_current`/`_past` reworked to source the
dose from the timeline **without widening** (same column set as db/031 — a widening `CREATE OR REPLACE` breaks
`connect_and_load_schema`'s every-connect db/031 replay: "cannot drop columns from view"). **correct-to-unknown shows
unknown, not the stale original** (views key on correction-row presence, not `COALESCE`). `cairn-event::medication`
split into `assert`/`cessation`/`dose`; device-additive `change_dose`/`correct_dose` orchestrators +
`resolve_correction_target` (defaults to the current dose point) + `medication-change-dose`/`medication-correct-dose`
CLI. Subagent-driven TDD (8 tasks); full workspace green — fmt + clippy `--workspace -D warnings`,
`cargo test --workspace` 0 failures / 31 binaries (DB-gated `medication_dose` **12/12** + slice-1 `medication` 10/10
across many reconnects), mkdocs. Whole-branch review (opus): **Ready to merge, 0 Critical/Important**; 2 floor
findings caught + fixed in-build (a 3VL NULL hole in the no-op guard → content-check + `COALESCE(...,FALSE)`; an
empty-`{"dose":{}}` raw-SQL bypass → the guard checks dose/effective CONTENT not key-presence, proven by a hostile
hand-injected test). **Deferred (slice 3+):** cross-thread **reconciliation resolution** (link two threads as the
same real med — never-merge); correcting a dose event's *effective date*/*reason* (slice 2 corrects the value only);
the #173 twin-dispatch registry refactor; the #157 collision advisory onto the dose projections; human-attested
clinical responsibility on a dose event.

**Matcher cleanup (2026-07-08, sixth session — advisory/test-infra only, no product/floor/spec bump):**
~~stale forced-REVIEW proposal retraction ([#135](https://github.com/cairn-ehr/cairn-ehr/issues/135))~~ **done**
(PR #151): `propose()`'s band-None branch now retracts a still-`pending` row (`status='retracted'`, append-only, no
DELETE) once a Doe is identified, `upsert_proposal` reverts `retracted→pending` on a genuine re-proposal, human
dispositions preserved. ~~matcher integration-test committed-row leak ([#84](https://github.com/cairn-ehr/cairn-ehr/issues/84) pt1)~~
**done** (PR #150): `managed_pg_conn` truncates projections on teardown (pt2 `KeyError` already fixed in PR #131).

**Remaining matcher pieces:** **B3** — a large hand-crafted gold set to re-run the slice-24 learner + locale comparator packs (phonetic/nickname + content-addressed profiles) + hub-tier
aggressive duplicate-sweep + full §7.5 matcher actor registration; ~~an A/B pass-toggle in
`generate_candidate_pairs`~~ **(done — slice 21)**; ~~scoring `administrative-sex` / the evidence-sparse score floor
([issue #130](https://github.com/cairn-ehr/cairn-ehr/issues/130))~~ **(done — slice 22)**. **Identity: pieces C1
(the §5.1/§5.7 linkage core — `db/018`) and C2 (the `match_proposal`→apply seam — `db/019`, `apply_proposal.rs`)
are now BUILT** (slices 13–14, above), as is **C2b** — auto-apply of the `auto_candidate` band (slice 15, above),
**C3** — `dispute` + the chart trust-state projection (slice 16, above) — and **C4** — `identify` + the *unconfirmed*
trust state (slice 17, above), which completes the §5.7 confirmed/unconfirmed/under-review contract, and **C5** —
`repudiate` + the known-alias pool (slice 18, above), the first *suppressing* identity event. Remaining:
**C5+** — the rest of the §5.7 algebra (`reattribute` §5.5 event-granular strike-through + tiered adjudication — waits on
a clinical-note surface). The §5.4 John-Doe subsystem's structural finishers are all built (slices 20–29) and the
`enroll-human` ceremony CLI (slice 30); the non-structural remainder is the §5.12 push-alert and the §5.3/§5.8
search-before-create funnel.
**Other deferred:** a veto-aware
scorer mode; variable cluster size / an unrecoverable fraction / hard negatives in the volume generator; a
`compare_address` comparator; a CLI sweep entry; B2 follow-up Minors → [issue #79](https://github.com/cairn-ehr/cairn-ehr/issues/79).
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
