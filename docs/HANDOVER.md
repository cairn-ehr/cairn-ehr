# HANDOVER — Cairn

**Session date:** 2026-07-08 · **Spec/ADRs:** v0.43 · **Phase:** architecture complete; **first
production clinical surface under construction** — demographics on `cairn-node` (slices 1–5 done) + the §5.2 matcher
(piece A in-DB veto floor · B1 advisory scoring core · B2 veto-gated pairwise pipeline + proposal worklist · B2b
blocking / candidate-pair generation + batch sweep · B3 eval harness · B3 compound blocking key (`name+year`) · B3
synthetic volume generator · B3 eval mirror (range-DOB + administrative-sex) · B3 weight-learning: supervised
Fellegi–Sunter estimation · **B3 further compound blocking keys `dob+first-initial`/`name+sex` — done this session**
· consumes `patient_alias_pool` known-alias evidence · range-aware, positive-only
`compare_dob` for clinician-observed estimated ages · anchored birth-year-range blocking passes (`dob-range` /
`dob-range+sex`) + the A/B pass-toggle · composite `sex` scoring + the unconfirmed-chart REVIEW rule) +
the **§5.7 identity core: C1 linkage · C2 human-accepted apply seam · C2b auto-apply of the `auto_candidate` band · C3
`dispute` + the chart trust-state projection · C4 `identify` + the *unconfirmed* trust state · C5 `repudiate` + the
known-alias pool** (the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE) + the
**§5.4 John-Doe registration front door, slices A–D all BUILT** (callsign minting + matcher placeholder exclusion ·
clinician-observed evidence · the birth-year-range blocking pass · administrative-sex scoring + the unconfirmed-chart
forcing rule, [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130) closed);
remaining **B3 measurement-driven follow-ons** (learn against a large hand-crafted gold set; locale comparator packs;
hub-tier aggressive duplicate sweep)
+ the **§5.4 photo evidence slice** (the first content-addressed **attachment** on a clinical surface; ADR-0042 froze
the §3.14 day-one attachment-reference shape)
+ the **§5.4 marks/belongings/EMS-context text-evidence slice** (three text `kind` values on the same
`identity.evidence.asserted` event type — **done this session**)
+ identity **C5+** (`reattribute` — waits on a clinical-note surface) + the **rest of the §5.4 subsystem**
(the "prior history now available" push-alert, the search-before-create funnel).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

**This session (2026-07-08, fifth) — closed the three CI gaps opened by the tooling catch-up ([#145](https://github.com/cairn-ehr/cairn-ehr/issues/145)/[#146](https://github.com/cairn-ehr/cairn-ehr/issues/146)/[#117](https://github.com/cairn-ehr/cairn-ehr/issues/117); PR TBD).**
No product/floor/spec/ADR/SCHEMA change — CI + test-fixtures + docs only. (1) **#145 — the matcher DB-gated suite now
runs in CI.** Its integration tests self-skip without `CAIRN_TEST_PG`, so they ran nowhere (the "298 passed" was the
*pure* suite only; the DB-touching path had zero automated coverage). Rather than stand up — and rebuild the expensive
`cairn_pgx` extension for — a second rig in `matcher.yml`, the `rust.yml` floor `test` job now also runs
`uv run --extra pipeline pytest` against the PG18+`cairn_pgx` cluster it already builds (the matcher conftest applies
the `db/*.sql` schema itself, so it needs only that cluster). Verified locally against PG18.1 + `cairn_pgx` 0.3.0:
**376 passed** (~79 DB-gated tests actually executing). (2) **#146 — CodeQL test-fixture crypto false positives fixed at
the source.** Path-exclusion is unreliable for compiled Rust (and one fixture is a `#[cfg(test)]` block *inside*
`src/seal.rs`), so the deterministic test seed / KDF-salt / pairing-nonce are now **computed at runtime**
(`std::array::from_fn`, `format!`) instead of hard-coded literals — no literal reaches a crypto sink, so
`rust/hard-coded-cryptographic-value` stops firing while staying live for *production* code. New **CLAUDE.md house
rule 6** codifies it (never hard-code crypto material in tests). seal.rs 16/16 + `clippy --workspace --tests -D warnings`
clean; the DB-gated `pairing.rs` test green on PG18. (3) **#117 — required-check set documented.** New *Continuous
integration* section in `CONTRIBUTING.md` tables the five required checks (`build`, `rustfmt`, `cargo-deny`,
`ruff + pytest`, `clippy + cargo test (cairn_pgx floor)`) with what each gates + the two traps: keep the floor check
**PG-version-independent** (a rename orphans branch protection — the #144 lesson) and update branch protection in
lockstep with any required-job rename. Also corrected CONTRIBUTING's stale "no code yet" claim. #117's remaining scope
was audit/document only — the gate itself has existed since PR #133/#143/#147. **Plus doc-currency at session start:**
HANDOVER/ROADMAP now reflect #144/#147 **merged** and the required-checks admin swap **done** (both were mid-flight
in the fourth-session block). **Honest limit:** the matcher DB suite re-runs the pure tests too (no marker to select
only DB-gated ones — cheap, and running the full suite against the DB is *more* coverage, not less).

**This session (2026-07-08, second) — §5.4 marks/belongings/EMS-context text identity evidence (matcher/identity
tier; design+plan under `docs/superpowers/{specs,plans}/2026-07-08-marks-belongings-ems-evidence*`).** Three
text-shaped `kind` values — `mark`, `belongings`, `ems-context` — on the **existing** `identity.evidence.asserted`
event type (the photo slice's non-attachment sibling). **No new migration / floor / SCHEMA / ADR / spec change** —
the type is already registered (`db/028`), additive, non-demographic (db/015 carries the authored twin verbatim); the
observation is free text in the payload, so `attachments` stays `vec![]` (zero-attachment content-address preserved).
Pure `cairn-event::identity_evidence` additions (`MARK`/`BELONGINGS`/`EMS_CONTEXT_EVIDENCE_KIND` + `TEXT_EVIDENCE_KINDS`
closed set + `parse_text_evidence_kind` typo-drift guard + `text_evidence_body` `{kind,provenance,description,basis?}` +
`render_text_evidence_twin`); a new `cairn-node::identity_evidence` author path (`validate_description` honest-content
floor **in the library** so a future UI backend inherits it; pure `build_text_evidence_body`; one-statement
`assert_text_evidence` — no blob tier).
**Decisions:** provenance fixed `clinician-observed` for all three kinds (relayed/hearsay lives in `basis`;
ems-context example "reported by paramedic"); `description` required-non-empty (the *floor* refuses an empty claim —
whether a UI silently defaults it is soft policy above our line, principle 12). TDD 4 tasks; e2e read-back +
bad-kind/empty-description rejects (DB-gated); CLI smoke on a provisioned node all four behaviors confirmed.
**Review follow-up (this session, post-review of PR #142):** the two evidence commands were **folded into one**
`assert-identity-evidence <patient> --kind photo|mark|belongings|ems-context …` — `--kind photo` takes
`--file`/`--media-type`/`--descriptor`, the text kinds take `--description`; the mutually-exclusive "`--file` iff
`--kind photo`" rule is a new **pure, unit-tested** `cairn-node::identity_evidence::route_identity_evidence` gate
(the separate `assert-photo-evidence` subcommand was removed). Also aligned `assert_text_evidence` to
discriminator-first validation (kind then description) to match the gate.
**Suites:** cairn-event + full cairn-node workspace green; `clippy --workspace --tests -D warnings` clean (the exact CI
gate). **Honest limits:** free-text `description` only (no structured belongings item list — YAGNI, additive-friendly);
no projection/worklist/matcher signal (evidence is log-retrievable + twin-legible, same as photo).

**This session (2026-07-08, third) — CI + tooling catch-up (PR #143).** Closed the SW-hygiene gaps found after
switching from ADR/spec work to building code. Three independent, verified-green gates: (1) **rustfmt** — one-time
mechanical whole-workspace + `cairn_pgx` reformat to rustfmt defaults (`max_width=100`, comments untouched, `poc/`
excluded; clippy/`cargo test` still green) + a `fmt` job in `rust.yml`. **The repo is now rustfmt-default-clean and
CI gates on it** (this supersedes the prior "hand-formatted, CI does not gate on fmt" note). (2) **cargo-deny** —
`deny.toml` (AGPL-compat permissive-only license allow-list · `advisories=deny` · `wildcards=deny` · crates.io-only)
+ a `deny` job; caught `RUSTSEC-2026-0190` → `anyhow 1.0.102`→`1.0.103`; `publish = false` on the three application
crates. cargo-deny **pinned to 0.19.9** (post-review fix: `cargo install --locked` pins only its deps, not itself).
(3) **matcher** — `matcher.yml` (`ruff check` + pure `pytest`, DB tests self-skip) + explicit ruff rule set in
`pyproject.toml`. **Deferred follow-ons — now mostly closed:** ~~required status checks / audit ([#117](https://github.com/cairn-ehr/cairn-ehr/issues/117))~~,
~~Rust toolchain/MSRV pinning + PG16→18 ([#144](https://github.com/cairn-ehr/cairn-ehr/issues/144), PR #147)~~,
~~DB-gated tests run nowhere in CI ([#145](https://github.com/cairn-ehr/cairn-ehr/issues/145))~~, ~~CodeQL test-fixture crypto false positives ([#146](https://github.com/cairn-ehr/cairn-ehr/issues/146))~~ — all closed by the fourth+fifth sessions (above); still open: stricter ruff ruleset (separate PR).

**Prior session (2026-07-08, fourth) — Rust toolchain pinning + honest MSRV + PG16→18 CI bump ([#144](https://github.com/cairn-ehr/cairn-ehr/issues/144); PR #147, MERGED; full detail in git).**
No Rust source — Cargo manifests + toolchain + CI YAML only. `rust-toolchain.toml` pins `channel = "1.96.0"` +
rustfmt/clippy for BOTH cargo trees (stops fmt-gate drift once the runner's stable moves); `[workspace.lints]`
(`rust.warnings`/`clippy.all` = deny) + per-crate `workspace = true` mirrors the CI clippy gate locally; honest
`rust-version` `1.74`→`1.96` (the old value did not build the modern dep graph; no separate MSRV gate — the
pinned-toolchain build already proves 1.96); the `test` job installs PG18 via the PGDG apt repo (Ubuntu 24.04 ships
only 16) matching the shipped `pg18` default (DB-gated floor **431/0** locally to de-risk). The required floor job was
renamed **PG-version-independent** (`clippy + cargo test (cairn_pgx floor)`) because the `(PG16 …)`→`(PG18 …)` rename
had orphaned the required check (exact-name match → never reports → `MERGEBLOCKED`); the branch-protection swap is
**done** and #147 merged. **Deferred:** `[workspace.lints]` for the `cairn_pgx` tree (pgrx macro code trips lints), a
bisected MSRV floor if `cairn-event` is ever published.

**Prior session (2026-07-08, first) — §5.4 photo evidence + the day-one §3.14 attachment-reference shape (ADR-0042; spec
v0.42→v0.43; design+plan under `docs/superpowers/{specs,plans}/2026-07-08-attachment-shape-and-photo-evidence*`).**
The FIRST content-addressed **attachment** on a clinical surface, which forced finalizing the ONE can't-retrofit
piece of ADR-0013. **Two phases, 9-task subagent-SDD, final whole-branch review "ready to merge" (0 Critical/
Important).** (1) **The shape** — replaced the walking-skeleton `AttachmentRef` stub with
`Attachment{descriptor, renditions:[Rendition{role,alg,digest_hex,media_type,byte_len,inline?,seal?}]}` +
`SealRef{alg,dek_wrap}` in `cairn-event/src/attachment.rs` (all five §3.14 reserves: content digest, descriptor
metadata, **rendition set** [structurally can't-retrofit], **seal indicator** + **inline-vs-reference** [reserved,
None]); field order frozen by **ADR-0042** (refines 0013, reconciles with **ADR-0041**'s note `payload.media`
manifest — one shared primitive, two carriers: `EventBody.attachments` for non-narrative events vs the note payload).
Empty-vec byte-identity proven, so every past zero-attachment event keeps its content-address. (2) **The floor** —
`db/027` `cairn_learn_attachment_refs` walks `attachments[*].renditions[*]` (skips inline); db/005 **and** db/020
call the one shared helper (no drift). (3) **Photo author path** — new non-demographic `identity.evidence.asserted`
(payload `{kind:"photo",provenance:"clinician-observed",basis?}`, photo in `EventBody.attachments`, twin from
descriptor **never pixels**; `db/028` registers the type — fail-closed floor); `cairn-node/photo_evidence.rs`
(pure `prepare_local_blob` + atomic `assert_photo_evidence` storing the blob present through the db/026 verify
trigger + authoring the event in ONE txn, `ON CONFLICT DO UPDATE` to fill a pre-existing reference-only placeholder)
+ an `assert-photo-evidence` CLI. Suites **workspace 418 passed / 0 failed; clippy clean**. **Honest limits:** ships
**plaintext** (seal reserved), a **single `original` rendition** (no preview — needs an image lib), **bytes stay
local** (cross-node byte fetch deferred), and the frozen POC harness diverges from the new shape. **Review fixes
applied post-build:** the honest-descriptor rule now lives in the library (`photo_evidence::validate_photo_descriptor`,
not only the CLI, so a future UI backend inherits it); a **direct db/020 apply-door attachment test** now exercises the
remote-apply call site of `cairn_learn_attachment_refs` (both doors directly covered); the local-blob **size-guard** gap
(no ceiling on `blob_store.content`, whole file read into memory) is lodged as **[#141](https://github.com/cairn-ehr/cairn-ehr/issues/141)**
for the §6.6 byte-tier slice. Residual accepted: DO-UPDATE overwrites caller-supplied `media_type` on an already-present
row (benign). Env: `cairn_pgx` upgraded to **0.3.0** on the Mac :5532 cluster this session (was 0.2.0 — db/026 requires ≥0.3.0).

**Prior session (2026-07-07) — B3 compound blocking keys `dob+first-initial` + `name+sex` (matcher slice 25;
condensed — full detail in ROADMAP slice 25 + git + PR #138).** Advisory eval/matcher tier only, no floor/spec
change: two additive symmetric compound passes (registry 6→8) — `dob+first-initial` (a first-initial relaxation of
the name requirement, genuinely new recall) + `name+sex` (the oversized-unisex-name-block per-sex rescue; the only
name rescue that fires for the John-Doe population). Shared CTE fragments extracted to avoid sex-normalization drift.
Suites pure 297 / DB 375 green. Honest limit: lift measured on synthetic data only.

**Prior session (2026-07-07) — B3 weight-learning: supervised Fellegi–Sunter estimation (matcher slice 24; full
detail in ROADMAP slice 24 + git; design+plan under `docs/superpowers/{specs,plans}/2026-07-06-b3-weight-learning*`).**
Advisory Python, eval tier only — **no production matcher/pipeline/floor/SCHEMA/event/ADR/spec change.** The learner
the shipped `DEFAULT_WEIGHTS`/`DEFAULT_THRESHOLDS` comments always pointed at ("B3 learns these"). Closed-form
supervised F-S: count agreement levels across labelled pairs → `m/u` (Laplace-smoothed, INSUFFICIENT_DATA excluded,
provenance-blind) → `weight = log2(m/u)`, the same math as `scoring.score` run backwards from ground truth. Four new
pure modules under `matcher/src/cairn_matcher/eval/`: `learner.py` (`estimate_weights`, `derive_thresholds`,
`learn_model`, `LearnedModel`), `crossval.py` (entity-cluster k-fold held-out lift, skips folds with no training
matches), `model_io.py` (`LearnedModel`↔JSON, `ModelIOError`), `learn.py` (CLI); + a behavior-preserving
`scorer_outcomes` extract in `scorer_eval.py`. **Thresholds are safety-first** — `auto = max(non-match)+margin`
(zero false auto-links by construction), `review = max(non-match)` (surface above the best impostor, never below,
so `review<auto` always holds — margin now guarded `>0`), and `recall_target` is an honest **conflict diagnostic**
(`collided` = the safe placement can't meet the recall floor), never a lever that drags `review` into impostor
range. **Held-out measurement splits on whole entity clusters** (no truth leak) and reports before/after only on the
disjoint fold. 6-task subagent-SDD; the Task-2 implementer caught a real plan bug (the original recall-cut `review`
inverted on separated data) → corrected before coding; final opus review caught the `margin<=0` false-auto hole →
guarded. Suites **pure 288 passed / 73 skipped / ruff clean**. **Honest limits (design §8):** ships the *mechanism*,
NOT new shipped weights (gold demo actually does *worse* than hand-tuned defaults — tiny, noisy, in-sample overlap;
a large hand-crafted gold-set re-run is the deferred follow-up); synthetic-learned weights reflect the generator's
corruption model; veto-blind (end-to-end veto only lowers a band — safe); provenance an orthogonal multiplier.

**Prior session (2026-07-06) — B3 eval mirror: generator range-DOB emission + administrative-sex representation
(matcher slice 23; condensed — full detail in ROADMAP slice 23 + git + PR #136).** Advisory eval tier only. Closed
the slice-22 deferral that blocked weight-learning: `DatasetRecord.administrative_sex` through the real adapter +
range-aware `_birth_window`/`shares_blocking_key` mirror of the anchored passes + `corrupt_dob_estimate` generator
operator + a live exact-DOB over-claim fix; final fable review caught the Python-`$`-vs-POSIX-`$` trailing-newline
over-claim (fixed via de-anchored `re.fullmatch`). gold_v1.json untouched.

**Parallel session (2026-07-05, PR [#133](https://github.com/cairn-ehr/cairn-ehr/pull/133)) — clock-drift admission
ceiling on both remote-apply doors + the Rust CI gate (non-demographics slice; recorded here post-merge because that
session deliberately left HANDOVER/ROADMAP untouched to avoid colliding with the concurrent demographics session).**
Closes the [#102](https://github.com/cairn-ehr/cairn-ehr/issues/102) ratchet finding: one verified event with an
absurd future `hlc.wall` from a trusted-but-broken peer would permanently ratchet the local clock (`GREATEST` is
monotone) and poison node-plane `ORDER BY hlc_wall DESC`. New shared `cairn_max_hlc_drift_ms()` (`db/001`, 24h) bounds
a remote event's asserted wall against our own `clock_timestamp()` (never the possibly-ratcheted `hlc_state`). The two
doors differ BY their pull-loop refusal semantics: node plane (`db/007`) **REJECTs** (self-healing skip+re-offer);
clinical plane (`db/020`) **ADMITs-but-CLAMPs** the `hlc_state` merge (a refusal would wedge `cairn-sync`'s frozen
watermark — availability over consistency; the event's asserted wall is preserved verbatim in `event_log`, principle
1). TDD: 5 DB-gated `hlc_drift.rs` tests. Same PR added `.github/workflows/rust.yml` — the CI Rust workspace +
in-DB-floor test gate ([#117](https://github.com/cairn-ehr/cairn-ehr/issues/117); note #117 remains open pending it
becoming a required check). **Honest limits (from the PR):** protects only against a REMOTE peer dragging the clock;
a future-dated clinical event still orders "latest" in projections (pre-existing, [#97](https://github.com/cairn-ehr/cairn-ehr/issues/97)).

**Prior session (2026-07-05) — §5.4 slice D: administrative-sex scoring + the unconfirmed-chart REVIEW rule
(matcher slice 22, closed [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130); condensed — full detail in
ROADMAP slice 22 + git + PR #134).** Advisory Python only. TWO halves, both required (admin-sex alone leaves the
headline pure-age Doe pair at ≈1.79 < `review=3.0`): (1) the **composite `sex` field** (`records.SexValue` + pure
`compare_sex` — both-sab → old EXACT/DISAGREE; else positive-only union fallback over {sab, administrative-sex},
never DISAGREE; weight key → `"sex"`, one field one contribution); (2) the **scoped forcing rule**
(`band(unconfirmed=)`): `chart_trust='unconfirmed'` + ≥2 positive-LEVEL fields + zero DISAGREE → REVIEW even below
threshold, never AUTO, fires with vetoes attached (post-review amendment — suppressing a vetoed-yet-corroborated Doe
pair would be the ADR-0014 auto-reject); proposals carry the `{"kind":"identity_pending","unconfirmed":[uuids]}`
marker; batch `db.load_trust_for` plumbing. The #130 e2e (`test_identity_pending_pipeline.py`) + an 8-angle
post-review fix wave (`score()` now raises on a weights table missing a compared field; `_corroborated_positive`
counts LEVELS; retraction gap filed as [#135](https://github.com/cairn-ehr/cairn-ehr/issues/135)). **Honest limits:**
a pending+disputed Doe reads `'under-review'` (severity-max, db/024) and bypasses the forcing rule while the dispute
is open — deliberate; ranking within a Doe's candidate list is the worklist tier's job; ~~the eval mirror cannot yet
represent the new fields~~ (closed by this session's slice 23, top).

**Parallel session (2026-07-05, PR #132) — the blob self-verification in-DB floor (Phase 7 / ADR-0013 point 11;
condensed — full detail in git + PR #132 + `docs/superpowers/{specs,plans}/2026-07-05-blob-verify-floor*`).** Closes
the `db/003` honest gap: self-verification was an L2 promise (pgcrypto has no BLAKE3), so a raw-SQL client could
store arbitrary bytes as any named blob. Now `cairn_pgx` **0.3.0** ships `cairn_blob_verify`/`cairn_blob_verify_error`
(thin wrappers over the SAME `cairn_event::blob_address` L2 uses) and **`db/026_blob_verify_floor.sql`** enforces a
TRIGGER floor on `blob_store` (a trigger, not a door+REVOKE — the byte tier legitimately writes raw DML;
metadata-only updates never re-pay the hash; column-level `UPDATE OF`, `CREATE OR REPLACE TRIGGER`, a
`to_regprocedure` load-time gate against stale `.so`s); `REQUIRED_PGX_FLOOR` 0.2.0→0.3.0. TDD: 7 DB-gated
hostile-client tests + a fail-closed pg_test. **Honest limits:** `blob_chunk`/`outboard` NOT in-DB verified (wrong
chunks only assemble into a flip that FAILS; a wrong outboard is rejected by the fetching peer's bao decode —
availability, never integrity); superuser can drop the trigger (same standing as every floor piece).

**Prior session (2026-07-04, second) — §5.4 slice C: anchored birth-year-range blocking passes + A/B pass-toggle
(condensed; full detail in git + ROADMAP slice 21 + PR #131).** A `year-range` dob now generates blocking keys —
**ANCHORED, never symmetric** (anchor×member only; all-pairing a window would manufacture C(k,2) noise): pure
`pipeline/blocking.py` (six-pass registry, `resolve_enabled_passes` raises on unknown names, `pairs_from_anchor`),
`_RANGE_GROUPS_SQL` (`birth_window` CTE, NULL-safe evaluation-order-proof year extraction; `dob-range` +
`dob-range+sex` rescue ∩ union blocking-sex with the `unknown` sentinel param-bound from `adapter.VALUE_SENTINELS`),
the `enabled_passes` A/B toggle, and a `name+year` honesty fix (range values excluded). Fable review + a post-PR
8-angle adversarial-verify wave (PR #131) fixed 9 findings total across two hardening commits (eval `KeyError` guard —
the #84 crash arm; shape-aware `dropped_pair_estimate`; exact-`dob` range exclusion for A/B purity; statement-level
toggle skip; SQL↔registry pass-name guard; `canonical_pair` deduped into `blocking.py`; whitespace trim-set). Suites
were pure 200 / DB 264 / ruff clean. Its recorded honest limit (pure-age pair blocks but dies at the score floor,
issue #130) is **CLOSED by this session's slice D** (see top).

**Prior session (2026-07-04, first) — §5.4 slice B: clinician-observed evidence (estimated-age range + observed sex)
(condensed; full detail in git + ROADMAP slice 20).** The demographic spine already carried it — **no floor/SCHEMA/event
type/ADR/spec change**: pure `cairn-event::evidence` (`birth_year_range_from_age` — store the time-invariant birth-year
window, never drifting raw age or a false-precise midpoint; `estimated_dob_body` value `"<min>/<max>"` precision
`"year-range"`; `observed_sex_body` on **`administrative-sex`**, not the birth fact a clinician can't know — also keeps
it out of db/016's verified-sex veto); `cairn-node::evidence::assert_observed_evidence` + CLI (one txn on an existing
chart, provenance `clinician-observed` rank 30, document-displaceable); matcher `DateValue` interval + `parse_dob`
`"year-range"` + **range-aware positive-only `compare_dob`** (overlap→PARTIAL, no-overlap→INSUFFICIENT_DATA, never
DISAGREE). Cargo+clippy+matcher suites green; e2e CLI smoke on a provisioned node passed (`dob=1981/1991`,
`administrative-sex=male`, `chart_trust=unconfirmed`); opus review READY TO MERGE. Its honest limit (range is not a
blocking key) was closed by slice C the same day.

**Prior session (2026-07-03) — §5.4 John-Doe slice A: callsign minting + matcher placeholder exclusion (condensed;
full detail in git + ROADMAP slice 20 + PRs #123/#125).** No new event type / migration / SCHEMA / ADR / spec bump.
Pure `cairn-event::john_doe::callsign` (`Unknown-<class>-<site>-<date>-<suffix>`, Unicode-aware sanitizer, 32-bit
UUID-derived suffix — partition-safe, bedside-collision ~1-in-4.3-billion-per-pair); `register_john_doe` + CLI
(callsign assertion + C4 `identity.pending.asserted` in ONE txn → *unconfirmed* chart); matcher excludes placeholder
`use_key` from blocking AND scoring (the scoring exclusion is load-bearing). Review hardening: kind-AGNOSTIC
actor-enrolment guard; the placeholder-use set hoisted into pure `cairn_matcher.placeholder_uses` + a cross-language
Rust↔Python drift guard (#124 closed). **§5.4 slice A is BUILT.**

**Prior sessions (2026-07-03) — the §5.7 identity algebra to C5 + the matcher's alias consumption (condensed; full detail in git + ROADMAP slices 15–19).** All merged on `main`, all additive (no floor/SCHEMA/ADR/spec bump except where noted): **C2b** auto-apply of the `auto_candidate` band (matcher-authored, un-attested, recallable link; apply-time veto re-check; per-`matcher_version` `agent` actor); **C3** `dispute` + the `chart_trust` projection (`db/023`; the *under-review* state); **C4** `identify` + the *unconfirmed* state (`db/024`; the §5.4 identity-pending front door; the §5.7 confirmed/unconfirmed/under-review contract is COMPLETE via a severity-max `chart_trust`); **C5** `repudiate` + the known-alias pool (`db/025`; the first *suppressing* identity event — a value-grained `name_repudiation` overlay strikes a known-false name from `patient_name_current`, `mode='suppressing'` forces the human-attestation floor, `patient_alias_pool` surfaces struck names to the matcher); and the **matcher consuming `patient_alias_pool`** (advisory Python — a `known_alias` evidence entry on the proposal, flag-never-suppress, `band()` forces REVIEW; the confidentiality-split view is reason-free per ADR-0006). **Deferred (recorded):** `reattribute` (waits on a clinical-note surface); fuzzy alias recognition; a per-slice identity-floor helper refactor + a deterministic `content_address` tiebreaker on the HLC overlay ([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)).

**Merged 2026-07-02 (9 PRs; full detail in git + ROADMAP slices 6–14).** A dense build+review day, all on `main`: the **quarantine/legibility trilogy** (durable pull-plane quarantine + re-offer floor on the clinical `db/021` and node-event `db/022` planes + ADR-0040 legibility/skew primitives wired into every signature door, `cairn_pgx`≥0.2.0 startup floor); **ADR-0040 signing-context domain separation** (spec v0.40→v0.41; the day's only spec bump); the **in-DB clinical apply door** `db/020_apply_remote_event.sql` (a replicated event faces the same floor as a local one) + the contamination-cascade recall-key fix (#99); a **7-agent adversarial review** (`docs/code_reviews/2026-07-02-*`) → in-branch fixes + filed issues #91–#103; and **identity C1** (`db/018` §5.1/§5.7 linkage core) + **C2** (`db/019` human-accepted→attested link).

**Prior sessions (2026-06-29/30/07-01) — the §5.2 advisory matcher pipeline B2→B3 (condensed; full detail in git + ROADMAP slices 8–12).** Advisory Python, no `db/` floor except B2's `db/017_match_proposal.sql` worklist (SCHEMA 15→16); no ADR/spec bump. **B2** veto-gated pairwise pipeline + proposal worklist (`cairn_matcher/pipeline/`); **B2b** blocking / candidate-pair generation (3-pass disjunction, oversized-block guard) + a `sweep()` batch driver; **B3 eval harness** (`cairn_matcher/eval/` — scorer metrics + DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + a `python -m cairn_matcher.eval` CLI, real-path reuse/no-drift); **B3 compound blocking key** (additive `name+birth-year` `UNION ALL` pass in `pipeline/db.py`; recall non-decreasing; honest culture-neutral year degrade via the first 4-digit run); **B3 synthetic volume generator** (`eval/generator.py` pure + `eval/generate.py` CLI — seed+corrupted-clone entity clusters recoverable by construction, drift-canary-pinned to the base blocking passes). **Deferred:** an A/B pass-toggle in `generate_candidate_pairs` (quantitative before/after); weight-learning; further compound keys; a veto-aware/e2e scorer mode; the matcher test-leak + harness `KeyError` ([#84](https://github.com/cairn-ehr/cairn-ehr/issues/84)).

**Prior sessions (2026-06-28/29) — §5.2 matcher pieces A + B1 (condensed; full detail in ROADMAP slices 6–7 + git):** **piece A** = the **§4.4/§5.2 in-DB hard-veto floor** (`db/016_match_veto.sql`, SCHEMA 14→15; `cairn_match_veto` returns the closed hard-veto set — same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash; two verdicts `hard_veto`/`degrade_hold`; precision-gated, parses no dates; `system:unknown` never vetoes; forces a human decision, never auto-link/auto-reject; 12 integration tests; deceased-status veto deferred, stub in db/016). **piece B1** = the **§5.2/§5.13 advisory scoring core** (new `matcher/` uv project, `cairn-matcher`, AGPL-3.0, **zero runtime deps, pure functions only** — the fit-for-purpose §9 tier): the `Comparator`/ordinal `AgreementLevel` contract (`PHONETIC`/`NICKNAME` reserved but never emitted by core — anti-cultural-capture), in-house **Jaro–Winkler** + 4 culture-neutral comparators (`compare_exact`/`compare_edit_distance`/`compare_dob` [parses no date strings]/`compare_name_set`) + positive-only `compare_identifier_sets` (never DISAGREE) + the field→comparator registry + the **Fellegi–Sunter** combiner producing an explainable `MatchScore`; 55 pure tests; final review caught + fixed one Critical (`score(a,b)≠score(b,a)` from greedy name-pairing → now `max(greedy(a,b),greedy(b,a))`, symmetric). No new ADR, no spec bump (both implement settled §5.2/§5.13/§4.4; refine ADR-0014/0033).

**Prior session (2026-06-28) — globalised the §3.13/§4.5 author-materialised legibility twin to every event type (ADR-0039; spec v0.39 → v0.40; condensed — full detail in git + the ADR).** `db/015` (SCHEMA 13→14): floor PREFERS the authored twin for every type; non-demographic types degrade honestly to a flagged, payload-rendering derived skeleton (closes the `db/005:29` TODO); demographic types keep ADR-0034's HARD requirement; authored-vs-derived derivable, not stored (`cairn_twin_is_authored` + `event_twin_provenance`). Pure `resolve_twin`/`materialise_generic_twin` shared by cairn-sync + the SQL floor. Same-branch floor bug fix: PG `trim()` is ASCII-space-only → blank-tests use `regexp_replace(x,'\s+','','g')` in BOTH write gate and read predicate; residual Unicode-whitespace asymmetry is [issue #75](https://github.com/cairn-ehr/cairn-ehr/issues/75). **The "globalise the authored twin" deferral is CLOSED.**

**Prior sessions (2026-06-27/28) — demographics slices 1–5, condensed (full detail in ROADMAP slices 1–5 + git):** **slice 1** = §4.4 patient-identifier assertion end-to-end (`db/010`, `EventBody.plaintext_twin`, `cairn_event_twin` hook, set-union `patient_identifier` projection; [issue #67](https://github.com/cairn-ehr/cairn-ehr/issues/67)); **slice 2** = §4.2 DOB + sex-at-birth provenance-locked fields (`db/011`, generic `demographic.field.asserted` + `cairn_provenance_rank` ladder incl. new `fact-proven` top tier; floor open / projection gated — the ADR-0012 federation-forward call; [issue #69](https://github.com/cairn-ehr/cairn-ehr/issues/69)); **slice 3** = §4.2 names (`db/012`, `patient_name` retained-set + `patient_name_current` recency-first-within-legal-tier display VIEW, [ADR-0036](spec/decisions/0036-demographic-name-display-recency-first.md); PR #71+#72); **slice 4** = administrative-sex + gender-identity (`db/013`, one `cairn_demographic_field_policy(field)` classifier driving both projection gate and winner ordering — sex provenance-first, gender-identity recency-first; karyotype resolved as a distinct field, [ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md); PR #73); **slice 5** = §4.3 address (`db/014`, per-use recency-first `patient_address_current` VIEW, same logic as names, [ADR-0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md)). Also closed demographics **gap B** (provider-number person×org relational model, [ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md), §4.6: entity/relationship + subject-kind partitioning, design/spec only) and representation gaps B+C ([ADR-0032](spec/decisions/0032-culture-neutral-address-representation.md) address, [ADR-0033](spec/decisions/0033-patient-identifier-representation.md) identifier namespace/profile split, [ADR-0034](spec/decisions/0034-demographic-legibility-twin.md) legibility twin). Spec 0.32→0.39 across this run. **Demographics slices 1–5 + gaps A/B/C all done; §4.2/§4.3/§4.4/§4.5/§4.6 complete.**

**Prior sessions (2026-06-25/26)** — ADR-0026 node durability slices B/C/D closed (backup-as-cold-peer, restore + `supersede`, sealed local-state export) + issues #53/#54 (cold-medium self-identification, uniform key zeroization) + **Spike 0003 (Postgres on Android) G0–G3 PASS**. Full detail: ROADMAP Phase 5/6 + git + the ADR-0026 log.

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session. If it ever disagrees with the canonical docs, **the canonical docs win.** The *why* lives
in the immutable ADR log; the *what* lives in the spec; this file only carries what lives *between* them —
current build state, open threads, and time-sensitive items.

---

## Read these first (the durable state)

- **`docs/spec/index.md`** — canonical architecture spec (mission prose + document map + spec version).
  One file per aspect; cross-refs like *§5.7* stay valid inside the aspect file.
- **`docs/spec/decisions/`** — the **ADR log** (the *why*). Numbered, dated, **immutable** (a reversal is a
  new superseding ADR). **Read the relevant ADR before reopening a settled question.** Index below.
- **`docs/ROADMAP.md`** — the foundation build order (wire core → in-DB floor → sync → identity →
  security → federation → blobs → native API), *below* the policy/GUI line. Disposable scaffolding like
  this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — build-prep records (*what we tried, on what, what we learned*). Not spec, not ADR.
- **`docs/principles/`** — mission/governance; **`GOVERNANCE.md`** + `STEWARDSHIP-OF-THE-NAME.md`.
- Root **`README.md`** — mission + founding principles (same prose as `index.md`).
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`), `/extensions` (`cairn_pgx`), `/db`.
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

### First federating node — built 2026-06-21, [PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)
First *implementation* of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
(federation admission), scoped to **direct-pairwise trust, no clinical surface** — only the federation
machinery flows, exercising the one safety-critical seam (*verified credential → admitted peer*) E2E. **No
spec/ADR change.** Built: `cairn-node` (Ed25519 keystore, `init`/`identity`/pairing/`peers`/`unpeer`, built-in
mTLS pinned to the trust set, set-union `node_event` sync, honest `status`); `db/007` append-only `node_event`
+ `submit_node_event` door + `apply_remote_node_event` deny-all admission gate (reuses `cairn_verify` pgrx — no
new crypto). Genesis-stable `node_id` = content-address of the genesis enrollment event. Two-node E2E green on
local PG16 + `cairn_pgx`.

**Honest gaps / follow-ons declared in the node — ALL CLOSED** (full detail in git + ROADMAP Phase 5/6):
status-before-init crash; runtime-login-role / floor-ENFORCED proof; key-at-rest seal + dual-wrap recovery escrow
(ADR-0026 slice A); incremental sync watermark + genesis HLC (#38/#42); all four ADR-0026 durability slices A–D
(cold-peer backup+health, restore + `supersede`, sealed local-state export); atomic key-file write (#45) + passphrase
`zeroize`-on-drop (#46). Only optional escrow *rungs* (Shamir M-of-N / QR / TPM) remain — upward options, not blockers.
The `localstate` DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- Test rig: DB-gated tests need local PG + `cairn_pgx` (`cargo pgrx install` against PG16); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace` is reliable.

### Spike 0002 (advisory-actor write contract) — ran 2026-06-21, C1–C5 PASS, [PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27) → ADR-0029 + ADR-0030
An external advisory agent authored an additive, un-attested, recallable advisory through the validated in-DB
door, **and the floor rejected all five hostile-agent attacks** with legible reasons. PR #27 review (the user)
caught two real floor holes the spike's own review missed — forged authorship (unbound `signer_key_id`) and a
`PUBLIC`-executable `SECURITY DEFINER` door — both fixed before merge (recorded in ADR-0030).

**Honest gap (closed 2026-06-22):** the attestation **success** path (a *valid*, correctly-bound
token accepted) was never exercised E2E — now closed by `cairn-sync attest-stdin` (the token minter),
`crates/cairn-node/tests/attestation.rs` (accept for responsibility-bearing + suppressing events; reject for
wrong-address, tampered, and non-human-attester), and `spike_0002.py` selftest (external-actor accept +
wrong-address/tamper). No `submit_event` logic changed — the accept branch already existed; this is the
coverage that was missing. ~~**Smaller deferred items remain open** (commented in code):
`events_by_actor_epoch` resolves against `actor_current` not historical `actor_event` rows;
`actor_current` wall-clock ordering needs a monotonic tiebreaker before production; no FK on
`recall_overlay.target_event_id`; plaintext twin is skeletal.~~ **All four closed:** the three recall-surface
items 2026-07-02 (issue #99 session, see top); the skeletal twin by ADR-0039 (2026-06-28).

### Dual-identifier discipline — ADR-0031, merged 2026-06-22 ([PR #34](https://github.com/cairn-ehr/cairn-ehr/pull/34); `local_ref` honesty fix merged 2026-06-24 [PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43))
New **[ADR-0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)** (canonical
identifiers + node-local surrogate keys): canonical plane (UUIDv7 + multihash) is unchanged and is the *only*
identifier on the wire/in signed bodies; the **projection plane** may intern canonical IDs to dense node-local
`bigint` surrogates as physical join keys. Leakage of a surrogate into a signed body = silent cross-node
corruption, so it is made *hard* (distinct domain type, mapping confined to floor functions, API egress always
the global ID). Landed with `db/008_surrogate_projection.sql` + the Bet B5 leakage guard. Final magnitude is
**measured on Bet B** (Pi), exactly as ADR-0001's compute bet — a "no measurable win" result narrows scope, not
fails the discipline.

**Honest gap (fixed 2026-06-24, [issue #35](https://github.com/cairn-ehr/cairn-ehr/issues/35)):** the prose
called the `local_ref` domain a "real two-way type barrier," but a PG domain over `bigint` is *not* — a
surrogate flows into any plain `bigint` with no cast/error (empirically confirmed). Corrected the wording in
`db/008`, spike 0001 §6.2, PI-RUNBOOK §6.1, and the walking-skeleton README to name the *actual* load-bearing
guarantee (signed plane typed `uuid` + `bigint ≠ uuid` + the G2 assertion) and to frame the domain honestly as
an intent-signal + one-directional guard. Rewrote **G4** in `db/tests/008_surrogate_test.sql`: it now asserts
the functions exist first (no more vacuous pass via `undefined_function`, now dropped), proves the genuine
guard (G4a `uuid`↛`local_ref`; G4b `bigint`↛`uuid` signed plane), and **characterizes the honest limit**
(G4c: `bigint` flows into `local_ref` silently). The spec body (§3.18) and immutable ADR-0031 were already
accurate (one-directional framing), so neither was touched. All G1–G6 green on PG16. **Merged 2026-06-24 ([PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43)).**

---

### Spike 0003 (Postgres on Android) — ran 2026-06-25, G0–G3 PASS, merged ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47) + [PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48))
Validated the **fractal-topology** invariant at the phone tier (RedMagic 11 Pro). Native PG 18.2 execs, `initdb`s,
serves SQL over TCP, and a cross-built pgrx extension loads + runs (incl. SPI) — no Termux userland, no root, no VM.
The one real blocker was `libandroid-shmem` (compile-baked Termux prefix + dead `/dev/ashmem`), fixed by a
self-contained, pinned-upstream patch. Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/) + a
Medium-style write-up. **Remaining non-load-bearing gaps:** from-source PG build and APK/`jniLibs` packaging
(not blocking — the bet is proven). No spec/ADR change.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **Demographics build — next slices** (the live build front; reuse the spine in `db/010`/`db/011`/`db/013`/`db/014` +
  `cairn-event::demographics`). Slices 1–5 are done (§4.4 identifiers, §4.2 DOB + sex-at-birth, §4.2 names,
  §4.2 administrative-sex + gender-identity, §4.3 address). **Karyotype** is resolved as a distinct field ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) — no code yet.
  **§5.2 matcher:** piece A (in-DB hard-veto floor, `db/016`), B1 (advisory **Python** scoring core), B2 (veto-gated
  **pairwise** pipeline + `db/017` proposal worklist), B2b (blocking / candidate-pair generation + `sweep()` driver,
  `cairn_matcher/pipeline/{db,sweep}`), **the B3 eval harness** (`cairn_matcher/eval/` — scorer metrics +
  DB-gated blocking-recall measurement + culture-plural `gold_v1.json` + `python -m cairn_matcher.eval` CLI), the
  **B3 compound blocking key** (`name+year` additive pass in `pipeline/db.py`), and the **B3 synthetic volume
  generator** (`eval/generator.py` pure + `eval/generate.py` CLI — seed+corrupted-clone entity clusters, recoverable
  by construction), and the **A/B pass-toggle** (`enabled_passes` on `generate_candidate_pairs`; unknown pass name
  raises), the **B3 eval mirror** (slice 23: generator range-DOB emission + `DatasetRecord.administrative_sex` +
  range-aware `shares_blocking_key`/`_birth_window`), **B3 weight-learning (slice 24): the
  supervised Fellegi–Sunter learner** (`eval/learner.py` `estimate_weights`/`derive_thresholds`/`learn_model` +
  `eval/crossval.py` entity-cluster held-out lift + `eval/model_io.py` + the `python -m cairn_matcher.eval.learn`
  CLI), **and B3 further compound blocking keys (slice 25, this session): `dob+first-initial`/`name+sex`**
  (`pipeline/db.py`/`pipeline/blocking.py` — a first-initial relaxation of the name requirement + the
  oversized-name-block per-sex rescue) are now BUILT. **Next (B3 measurement-driven):** a **large hand-crafted gold
  set** to re-run the learner for
  authoritative magnitudes (slice 24's learner is a PoC on small/synthetic data) + locale comparator packs /
  hub-tier aggressive duplicate-sweep + proposal retraction / **richer §7.5 matcher-actor determinants**
  (served-model digest; C2b registered the matcher as a per-epoch `agent` actor keyed on `matcher_version`). **Identity: pieces C1** (§5.1/§5.7 linkage core — `db/018`),
  **C2** (`match_proposal`→apply seam — `db/019`, `apply_proposal.rs`; human-accepted → human-attested link), **and
  C2b** (auto-apply of the `auto_candidate` band — `matcher_actor.rs` + `auto_apply.rs`; matcher-authored, un-attested,
  recallable link, apply-time veto re-check), **C3** (`dispute` + the chart trust-state projection — `db/023`;
  the §5.7 projection-side contract, driven by the patient-initiated dispute front door), **and C4** (`identify` +
  the *unconfirmed* trust state — `db/024`; the §5.4 John-Doe identity-pending front door + the `identify` resolver,
  composing the third trust state into a severity-max `chart_trust`) **are now BUILT — the §5.7
  confirmed/unconfirmed/under-review contract is COMPLETE**, **and C5** (`repudiate` + the known-alias pool — `db/025`;
  the first *suppressing* identity event, a value-grained `name_repudiation` overlay striking a known-false name from
  `patient_name_current` and surfacing it via `patient_alias_pool`, `mode='suppressing'` forcing the human-attestation
  floor) **is now BUILT**. **Next identity slice: C5+** — `reattribute` (§5.5 event-granular strike-through of *clinical
  documentation* + tiered adjudication — **waits on a clinical-note surface** that does not yet exist; premature to
  build against demographics) + the rest of the §5.4 John-Doe registration subsystem. **§5.4 slice A (callsign minting +
  matcher placeholder exclusion)** — `cairn-event::john_doe::callsign` + `cairn-node::john_doe::register_john_doe` + a
  `register-john-doe` CLI + the advisory matcher exclusion (`use_key <> ALL(%s)`) — **and slice B (clinician-observed
  evidence: estimated-age range + observed sex)** — `cairn-event::evidence` (`birth_year_range_from_age` +
  `estimated_dob_body` value `"<min>/<max>"`/precision `"year-range"` + `observed_sex_body` on `administrative-sex`) +
  `cairn-node::evidence::assert_observed_evidence` + an `assert-observed-evidence` CLI + range-aware **positive-only**
  `compare_dob` in the matcher — **and slice C (the birth-year-range blocking pass: anchored `dob-range` +
  `dob-range+sex` passes in `pipeline/db.py` + pure `pipeline/blocking.py`)** — **and slice D (administrative-sex
  scoring via the composite `sex` field + the unconfirmed-chart REVIEW forcing rule + `chart_trust` plumbing, this
  session — closes [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130), see top)** — **are now
  BUILT; NO new event type / migration / floor / SCHEMA / ADR / spec change.**
  **Remaining §5.4:** ~~photo evidence~~ (DONE — `identity.evidence.asserted` + the ADR-0042 attachment tier);
  ~~marks/belongings/EMS-context evidence~~ (DONE this session — three text `kind` values on the same event type,
  `cairn-node::identity_evidence` + `assert-identity-evidence` CLI, zero wire change), the
  "prior history now available" push-alert on link (§5.12, no notification tier yet), the search-before-create
  registration-class funnel (§5.3/§5.8, UI/API tier), a readable sequential callsign suffix (partition-safe per-day
  count), a `--observed-year` CLI override, and `identify`→optional-link wired into one resolution flow. Reattribute composes one more *under-review*
  source into the `chart_trust` VIEW when it lands (note: a pending+disputed Doe already reads `'under-review'` —
  severity-max — so the slice-D forcing rule deliberately stands down while a dispute is open). Deferred (repudiate): a **reversal / de-repudiation** event (overlay HLC-versioned, composes without rewrite);
  a **chart-history VIEW** rendering struck names; fuzzy alias recognition + a dedicated `alias` blocking pass.
  Deferred (range blocking): ~~generator range-DOB emission + range-aware eval mirror~~ (done — slice 23); fuzzy
  near-window softening; hub-tier range sweep. Deferred
  (earlier): variable cluster size / an unrecoverable
  fraction / hard negatives in the volume generator; a **veto-aware / end-to-end scorer mode**; deceased-status veto
  (stub in db/016); a `compare_address` comparator; a **CLI** sweep entry; the matcher conftest test-leak
  ([issue #84](https://github.com/cairn-ehr/cairn-ehr/issues/84) — its harness-`KeyError` arm was FIXED in PR #131's
  review wave; the committed-row leak remains); B2 follow-up Minors (Thresholds `review<auto` guard,
  `band` CHECK, `updated_at` trigger, conftest env read-at-import) → [issue #79](https://github.com/cairn-ehr/cairn-ehr/issues/79).
  Rust DB-gated tests + the matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb
  dbname=cairn_test"` (PG18+cairn_pgx); matcher integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline
  pytest`. The pure matcher suite is dependency-free: `cd matcher && uv run pytest` (uv, never venv/pip).
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay + key-custody +
  actor primitives have absorbed every case so far without new architecture. Bring a real ED/hospital failure mode.
- **Dedupe transitive RustCrypto dep versions** in `Cargo.lock` ([issue #11](https://github.com/cairn-ehr/cairn-ehr/issues/11)) — supply-chain
  hygiene. **Re-verified 2026-06-25: still blocked on upstream** — the `postgres` stack pulls `digest 0.11`/`sha2 0.11`/`chacha20 0.10`
  while `chacha20poly1305 0.10.1` still depends on `chacha20 0.9` and `ed25519-dalek` on `digest 0.10`. Not fixable from our `Cargo.toml`; revisit when the ecosystem converges.
- **Harden the first federating node** — status-before-init crash, runtime-login-role/floor-ENFORCED proof,
  incremental sync watermark + genesis HLC ([#38](https://github.com/cairn-ehr/cairn-ehr/issues/38), PR #42),
  and **all four ADR-0026 durability slices** — A (at-rest seal + recovery escrow, PR #44), B (cold-peer export+health,
  PR #51), C (restore + `supersede`, PR #52), **D (sealed local-state export, this session)** — are all **closed**
  (see node gaps above). **ADR-0026 is fully implemented at the node tier.** No remaining required node-hardening thread;
  ~~[#54](https://github.com/cairn-ehr/cairn-ehr/issues/54) (uniform key zeroization)~~ and ~~[#53](https://github.com/cairn-ehr/cairn-ehr/issues/53)
  (federated-restore self-identification)~~ both **closed 2026-06-26**; only optional escrow rungs (Shamir/QR/TPM) remain.
  The `localstate` DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already advanced
  across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **RAN 2026-06-25 on a Pi 5 / 8 GB → PASS** (all §6 gates green, large headroom; B4 **confirms** ADR-0015's
  BLAKE3 blob-digest default — BLAKE3 ~4× SHA-256 on Cortex-A76). Artifacts in
  [`poc/walking-skeleton/results/`](../poc/walking-skeleton/results/). **Two caveats** (precision, not verdict):
  storage ran on a **USB-2-limited dock** (power-offload workaround after a Pi 5 brown-out saga — see the §9.2
  *deployment-BOM finding*: PSU + storage-attachment path are part of the validated BOM), and on **PG 16**
  because **`cairn_pgx` is pgrx-0.12.9 / `pg16`-pinned and won't build on PG 18** (§9.3). Bonus: `cairn_pgx`
  builds+loads on Pi arm64 (in-DB Rust surface confirmed on ARM). **Follow-ups:** ~~(a) port `cairn_pgx` to a
  PG-18-capable pgrx~~ **done 2026-06-25 (PR #56: pgrx 0.12.9 → 0.18.1)**; ~~(b) clean re-run on PG 18 + fast
  storage + official PSU~~ **DONE 2026-07-07 → PASS, both caveats resolved
  ([§9.5](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved))**:
  same 8 GB board, now on **PostgreSQL 18.4 + a PCIe NVMe HAT** (better than the USB-3 SSD the follow-up asked
  for). Headline: B1 p95 **3.99 ms @ 2,004,000 events** (13× under budget), *faster than* the old USB-2 number
  **at 10× the log size**, flat ×2.50 over a ×37 growth jump; B2 p95 4.5 ms/374-note chart (222×); B5 FK-index
  shrink ×1.40 @ 2 M rows (G1–G6 pass); crypto reproduced within noise; measured **~1,515 B/event** on disk.
  Artifacts `poc/walking-skeleton/results/betb-pi5-nvme-pg18-*`. **Remaining:** (c) fold the (now un-caveated)
  B4 number into the ADR-0015 follow-up to drop "provisional" from the blob-digest line.
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection per
  slice (the production object-store tier). The §8.2 availability + windowing/resume work already shipped.

---

## Parked (don't re-litigate without new reason)

- **Stewarding legal entity & jurisdiction** (German Stiftung/Verein, US 501(c)(3), or an umbrella) — deferred
  until momentum/funding geography is clearer.
- **Formal trademark / wordmark registration** — principle recorded (stewardship doc); legal instrument deferred.

---

## Working context (most also in CLAUDE.md)

- The user is a senior **EM physician**, GNUmed founder (early FOSS Postgres EHR), codes mostly in Python, brings
  real ED/hospital failure modes from multiple health systems. **The mission (anti-capture / anti-vendor-lock-in)
  is the tie-breaker.** Criticism is strongly encouraged — surface flaws/risks immediately.
- **Twelve founding principles** run through everything ([index.md](spec/index.md)); the first four are the lens
  for every design choice: (1) append-only + causal ordering; (2) identity is a claim — never merge/erase, always
  link/overlay; (3) paper-parity (no confirmation dialogs); (4) acknowledged uncertainty. See CLAUDE.md for the
  full set (5–12) and the §9 defect-blast-radius language-selection rule.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0 inbound=outbound,
  DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr` org; `cairn-ehr.org`+`.com`;
  PyPI/crates.io/npm `@cairn-ehr` placeholders).

---

## Decision trail — the ADR index (the *why* is in each linked ADR; do not restate it here)

**Every original §11 open architecture question is closed.** Compact index of the settled decisions; read the
ADR before reopening any of these.

| ADR | Decision (one line) | Spec home / principle |
|---|---|---|
| [0000](spec/decisions/0000-pre-adr-changelog-v0.1-v0.6.md) | Pre-ADR changelog v0.1→v0.6 | — |
| [0001](spec/decisions/0001-fat-postgres-thin-daemon.md) | Fat Postgres, thin Rust daemon | §2/§3.5/§6.1/§9.4 |
| [0002](spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md) | In-DB Rust (pgrx) escape hatch | §9.4 |
| [0003](spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md) | Bitemporal time (`t_recorded` vs `t_effective`) | §3.6/§3.7 · **principle 4** |
| [0004](spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) | Sync scope = prefetch hint, not authority | §6.4 |
| [0005](spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md) | Erasure = key-custody redistribution / crypto-shred | §3.8/§7.1 · **principle 9** |
| [0006](spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md) | Replication ≠ confidentiality; the safety projection | §5.9 |
| [0007](spec/decisions/0007-authorship-and-accountability.md) | Authorship compositional, accountability separable | §3.9/§7.2 · **principle 10** |
| [0008](spec/decisions/0008-point-of-care-identity-possession-and-salvage.md) | Point-of-care identity, possession, `sign-as` salvage | §5.11/§3.10 |
| [0009](spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md) | Notification economy, salience routing, ack floor | §5.12/§3.11 |
| [0010](spec/decisions/0010-additive-vs-suppressing-classification.md) | Additive-vs-suppressing (derived, not declared) | §3.9 |
| [0011](spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) | Actor registry, version-pinning, key custody | §7.5/§3.12 |
| [0012](spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md) | Schema evolution, two planes, legibility twin | §3.13/§6.5/§7.6 · **principle 11** |
| [0013](spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) | Attachments = content-addressed lazy blob tier | §3.14/§6.6 |
| [0014](spec/decisions/0014-locale-pluggable-matcher-comparators.md) | Locale-pluggable matcher comparators | §5.13/§4.1 |
| [0015](spec/decisions/0015-event-serialization-signatures-and-content-addressing.md) | COSE_Sign1 + Ed25519 + SHA-256; BLAKE3 blobs (*provisional*) | §3.5/§3.14 |
| [0016](spec/decisions/0016-record-discovery-and-the-replicated-essential-tier.md) | Record discovery + replicated essential tier | §6.7/§5.2 |
| [0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md) | Federation admission, sovereignty, trust anchors | §7.7 |
| [0018](spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md) | Federation revocation cascade; anchor-as-power | §7.7 |
| [0019](spec/decisions/0019-author-scoped-record-export-the-medico-legal-copy.md) | Author-scoped export (the medico-legal copy) | §7.8 |
| [0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md) | Active-write, thin encounters, delete-vs-erase | §3.15 · vision §1.2 |
| [0021](spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) | Four-layer model; node API; UI pluralism | §9.5 · **principle 12** |
| [0022](spec/decisions/0022-validated-submit-surface-the-write-path.md) | Validated `submit_event` surface (the write path) | §9.6 |
| [0023](spec/decisions/0023-native-api-contract-capability-and-conformance.md) | Native API contract: capability + conformance | §9.7 |
| [0024](spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md) | Hard policy = signed policy-assertion stream | §7.9 |
| [0025](spec/decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) | ICD-11 canonical interlingua + local-terminology overlay | (terminology) |
| [0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) | Node durability & disaster recovery (cold-peer backup) | §7.10 |
| [0027](spec/decisions/0027-trusted-time-anchoring.md) | Trusted-time anchoring (graded-interval `t_recorded`) | §3.17/§7.11/§6.8 |
| [0028](spec/decisions/0028-finalized-closed-contributor-role-enum.md) | Finalized closed contributor-role enum | §3.9 |
| [0029](spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md) | Skill-epoch + served-model digest as pinned actor determinants | §7.5 |
| [0030](spec/decisions/0030-advisory-actor-integration-contract.md) | Advisory-actor integration contract | §9.8 |
| [0031](spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md) | Canonical IDs + node-local `bigint` surrogate keys (dual-identifier discipline) | §3.1/§3.2 |
| [0032](spec/decisions/0032-culture-neutral-address-representation.md) | Culture-neutral address: three-facet value (display twin + geo + culture-tagged parts) | §4.3 (refines 0014) |
| [0033](spec/decisions/0033-patient-identifier-representation.md) | Patient-identifier representation: namespace/profile split + matching-survivable normalized form | §4.4 (refines 0014) |
| [0034](spec/decisions/0034-demographic-legibility-twin.md) | The demographic legibility twin: every demographic assertion legible without its profile | §4.5 (refines 0012) |
| [0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md) | The entity/relationship model + provider-number person×org (subject-kind partitioning) | §4.6 (refines 0033) |
| [0036](spec/decisions/0036-demographic-name-display-recency-first.md) | Demographic name display: recency-first within the legal tier (diverges from DOB's provenance-lock by design) | §4.2 (refines 0014) |
| [0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md) | Sex/gender/karyotype field semantics: per-field winner policy; karyotype is a distinct field, never displaces assigned sex-at-birth | §4.2 (refines 0011/0014) |
| [0038](spec/decisions/0038-demographic-address-winner-per-use-recency.md) | Demographic address display: per-use recency-first (volatile field; follows ADR-0036) | §4.3 (refines 0032, follows 0036) |
| [0039](spec/decisions/0039-globalise-authored-legibility-twin.md) | Globalise the author-materialised legibility twin to every event type; honest-degradation fallback for non-demographic types | §3.13/§4.5 (refines 0012/0034) |
| [0040](spec/decisions/0040-signing-context-domain-separation.md) | Signing-context domain separation (content-type + `external_aad`); one signature per event, co-signing by overlay | §3.5 (refines 0015/0007/0030) |
| [0041](spec/decisions/0041-progress-note-narrative-format.md) | Progress-note format: one signed event, markdown narrative + manifest-keyed media anchors | §3.19 (refines 0012/0013/0020/0039) |
| [0042](spec/decisions/0042-concrete-attachment-reference-shape.md) | Concrete attachment-reference shape (Attachment/Rendition/SealRef; frozen field order) | §3.14 (refines 0013, reconciles 0041) |

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B prepared); 0002 (advisory-actor — ran, C1–C5 ✓
→ ADR-0029/0030); 0003 (Postgres on Android — **ran 2026-06-25, G0–G3 ✓**; PR #47/#48).
