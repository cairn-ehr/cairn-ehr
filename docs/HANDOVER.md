# HANDOVER — Cairn

## ⇒ NEXT: the 2026-07-15 review course is ✅ FULLY CLOSED (P1–P5 + the whole Priority-6 queue + #217). Priority-6 queue all done: #205 → ADR-0054; #206 → ADR-0055; #200 → ADR-0056; #208 → ADR-0057 (generic reprojection, merged PR #274); **#216 ✅ → [ADR-0058](spec/decisions/0058-grade-gated-teffective-ceiling.md)** (grade-gated `t_effective` ceiling, spec v0.60 — a born `clock_grade` gates the ceiling's rejecting power: at `self-asserted`/`unknown` (every node today) the ceiling **flags-never-rejects** a forward `t_effective` (principle-4 fix for slow/dead/absent-RTC clocks), the remote-apply door **admits-and-flags, never rejects** (closes a latent one-event sync-wedge DoS reachable by the Spike-0002 threat model), plus `cairn_clock_health()` the "clock-behind-its-own-HLC" honesty read; anchor-plane follow-ons [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279)–[#283](https://github.com/cairn-ehr/cairn-ehr/issues/283) + [#284](https://github.com/cairn-ehr/cairn-ehr/issues/284)). **#217 ✅** (paper-parity benchmark now a required slice-plan section, ROADMAP Slice 52) — **the review course is FULLY closed; nothing remains open from 2026-07-15.** Matcher review-follow-ons **#209 + #210 ✅** (2026-07-23; advisory-tier ADR-free TDD bugfix, Slice 51: `derive_thresholds` now fails closed on an empty non-match set + `kfold_lift` skips such folds — no impostor ⇒ no safe auto anchor, #209; a sweep-level reconciliation pass retracts pending proposals orphaned when a pair leaves the blocking universe, e.g. a fully-identified Doe, #210). Matcher **[#211](https://github.com/cairn-ehr/cairn-ehr/issues/211) ✅ 2026-07-25** (the E3 four-gap batch — advisory-tier TDD bugfix + doc-honesty, Slice 53). Matcher **[#290](https://github.com/cairn-ehr/cairn-ehr/issues/290) ✅ 2026-07-25** (the #211.4 seam's consumer — eval consumers now REPORT the repaired-pair count, advisory-tier additive TDD, Slice 54). Medication slice 6a shipped 2026-07-27 (**merged: PR #297 + the shred-scrub fix PR #298**, ROADMAP Slice 56 —
the `clinical.medication` code slice implementing [ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)):
the inline `substance.coding {system, code, display}` shape replacing the reserved `inn_code` slot, the
`medication_coding_system` registry + two-tier floor (`db/041`, `SCHEMA_GENERATION` 40→41), its own
`medication_coding` projection table, the `(system, code)`-pair dup-key, and honest degradation proven by
a no-drugref-reference source guard; detail in the session block. Filed
**[#294](https://github.com/cairn-ehr/cairn-ehr/issues/294)** — the §5.9 safety projection must *carry*
the coding-derived drug class rather than re-derive it (ADR-0059 decision 4; blocked on #232). **The
unblocked next step is slice 6b** — the two coding-overlay event types
(`clinical.medication-coding.asserted` + `-correction.asserted`) in a new `db/` file. **In design
(2026-07-27) the "re-route the read views through an effective-coding view" guess was retired:** because
6a gave the coding its own table, both overlays write that same table under the same winner rule, so the
slice is additive — no view is re-routed and no view's column set changes (one view *body* changes, the
prefer-coded predicate). Ledger notes still bind it (full list in the session block): the overlay
apply-write needs the same `jsonb_typeof(...) IS DISTINCT FROM 'null'` guard
the assert path needed; `medication_coding` becomes a **multi-type** table once the overlays write it
(`db/039` already refuses a narrow `cairn_reproject` prefix over one); and 6a's
`patient_id = cairn_medication_thread_patient(...)` is NON-NULL only because the assert upserts
`medication_statement` first — an overlay arriving BEFORE its assert gets NULL there and needs its own
answer. Other unblocked feature work: further medication slices,
**[#287](https://github.com/cairn-ehr/cairn-ehr/issues/287)** (hub-scale reconciliation re-scoring-cost
note), plus the UI-slice obligation **[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)**
(med-list whole-list sign-off must collapse to one human gesture, owed by the future Tauri med-list slice).
Two hygiene items filed out of the 6a arc, both still open:
**[#295](https://github.com/cairn-ehr/cairn-ehr/issues/295)** (the shipped anchor-conflict collation pin
carries no regression test; one `Copy`-borrow simplification left half-done) and
**[#296](https://github.com/cairn-ehr/cairn-ehr/issues/296)** (test pollution — a `cairn-sync` test drops
`event_log.seq` and never restores it, so the re-added column lands last and a later *positional*
`ROW(...)::event_log` literal in `born_sealed_schema` binds `clock_grade` into `seq`; this is the root
cause of the long-carried "recreate the test DBs" gotcha).

A five-pass whole-project review ran 2026-07-15 (in-DB floor, Rust workspace, spec/ADR corpus,
matcher, cross-cutting seams). Full report: [`docs/code_reviews/2026-07-15-whole-project-architecture-review.md`](code_reviews/2026-07-15-whole-project-architecture-review.md);
every finding is filed as a GitHub issue (#187–#217) with a finding→issue map at the foot of the report.

**Standing gate:** whole-project review cycles like this one repeat periodically, and there will be
**no release for clinical use before repeated review cycles pass cleanly.**

**The five priorities, all closed (full detail: ROADMAP Slices 36–45 + the PRs + git):**
- **P1 ✅ 2026-07-16** — floor hardening vs the Spike-0002 hostile enrolled writer (#187/#207/#194/
  #191/#192[+#177]/#190/#193/#195; PR #219). Open follow-up: [#220](https://github.com/cairn-ehr/cairn-ehr/issues/220)
  (the #190 hard veto is link-arrival-only; needs a re-check hook or background sweep).
- **P2 ✅ 2026-07-16** — sync-convergence integrity, five slices (#199/#198/#196/#197/#202+#201;
  PRs #221–#225; ROADMAP Slices 37–40). Follow-ups [#227](https://github.com/cairn-ehr/cairn-ehr/issues/227)/[#228](https://github.com/cairn-ehr/cairn-ehr/issues/228).
- **P3 ✅ 2026-07-16→18** — both wire windows shut: [ADR-0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  role-vocabulary floor (#203+#96, Slice 41) · [ADR-0052](spec/decisions/0052-born-sealed-clinical-bodies.md)
  born-sealed clinical bodies (#189+#92, Slice 42; follow-ups #230–#238; **wipe pre-ADR-0052
  plaintext-clinical dev/PoC rigs** — the floor refuses plaintext `clinical.*`) ·
  [ADR-0053](spec/decisions/0053-per-write-human-authorship.md) per-write human authorship (#204,
  Slice 43; follow-ups #242–#245; grading half-live until #245 wires a read path).
- **P4 ✅ 2026-07-19** — the #188 schema-version downgrade guard in BOTH loaders (repo-wide
  `SCHEMA_GENERATION` constant + fs-derived guard tests + the `SCHEMA_LOAD_LOCK` TOCTOU close;
  PR #251, Slice 44) + #238 flake fix + the #212 CI half (`scripts/run-db-sql-tests.sh` in `rust.yml`).
- **P5 ✅ 2026-07-19** — the process-mechanization session (#212/#213/#214/#215; PRs #253 + #255,
  merged; Slice 45 below). #212's property suite **caught a real grading defect** before any read
  path shipped. Post-review follow-up: [#254](https://github.com/cairn-ehr/cairn-ehr/issues/254)
  (the 8 remaining `DO NOTHING` twin-check registry files — unify with the #214 `DO UPDATE` arm
  or record why not).

**Priority 6 — design sessions (no rush, but settle before the dependent feature work).**
- **#205 (C4) ✅ 2026-07-19** — resolved by [ADR-0054](spec/decisions/0054-actor-registry-federation-admit-and-dispute.md)
  (admit-and-dispute; spec v0.56; closes #154 structurally, discharges the #172 sync-door half);
  code slices are future feature work.
- **#206 (C5) ✅ 2026-07-20** — resolved by [ADR-0055](spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md)
  (chained trust-root document; spec v0.57); follow-ons filed: [#257](https://github.com/cairn-ehr/cairn-ehr/issues/257)
  (verifier/load-gate code), [#258](https://github.com/cairn-ehr/cairn-ehr/issues/258) (transparency-log
  role), [#259](https://github.com/cairn-ehr/cairn-ehr/issues/259) (reproducibility CI),
  [#260](https://github.com/cairn-ehr/cairn-ehr/issues/260) (freshness rung),
  [#261](https://github.com/cairn-ehr/cairn-ehr/issues/261) (sync-auth onboarding UX design session).
- **#200 (B5) ✅ 2026-07-20** — resolved by [ADR-0056](spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)
  (admit-and-defer; spec v0.58). The filed premise was **inverted**: the spec was right, the code
  was wrong, so the fix is code catching up rather than the promise shrinking. Follow-ons filed:
  [#265](https://github.com/cairn-ehr/cairn-ehr/issues/265) (door admits uninterpreted),
  [#266](https://github.com/cairn-ehr/cairn-ehr/issues/266) (re-adjudicate the deferred gates, *then*
  reproject — retitled in the PR #271 review; reprojection alone would grant power that never passed
  the attestation / target-exists / cross-author-suppression gates),
  [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267) (pen door refusals verbatim),
  [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (align node-plane skip),
  [#269](https://github.com/cairn-ehr/cairn-ehr/issues/269) (node-plane heal test gap),
  [#270](https://github.com/cairn-ehr/cairn-ehr/issues/270) (frozen watermark must fail loud).
- **#208 (D3) ✅ 2026-07-21** — resolved by [ADR-0057](spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md)
  (generic reprojection; spec v0.59; ROADMAP Slice 49; PRs #274/#278); the #266 reclassify-then-reproject
  path consumes this mechanism.
- **#216 ✅ 2026-07-23** — resolved by [ADR-0058](spec/decisions/0058-grade-gated-teffective-ceiling.md)
  (grade-gated `t_effective` ceiling; spec v0.60; ROADMAP Slice 50; PR #285).
- **#217 ✅ 2026-07-24** — the §1.2 paper-parity benchmark is now a required slice-plan section
  (ROADMAP Slice 52; CONTRIBUTING.md + CLAUDE.md house rule 7 + a no-DB source guard); filed the first
  live entry [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288). **The 2026-07-15 review course is fully closed.**

---

**Session date:** 2026-07-27, latest (**Slice 56** — the `clinical.medication` slice 6a code build: the
inline `substance.coding` shape from ADR-0059; code-only, spec v0.61 unchanged; detail in the session
block below. Earlier 2026-07-25 was **ADR-0059** — the Cairn↔drugref medication drug-coding seam, a
**design-only** ADR fixing the coding wire-shape before code, spec v0.61, ROADMAP Slice 55. Earlier still
2026-07-25 was matcher **#290** — the #211.4 seam's consumer: the eval consumers
now REPORT how many measured/trained true-match pairs a synthetic verbatim-name repair made artificially
easy, so a reader discounts the optimistic recall/F1. Advisory-tier, ADR-free, additive TDD wholly inside
`matcher/`; no spec/SCHEMA/wire/ADR change; Slice 54 below; suite 395→409/0 + ruff clean + independent
code-review pass (clean). Earlier 2026-07-25 was matcher four-gap batch **#211** — an advisory-tier, ADR-free TDD
bugfix + doc-honesty wholly inside `matcher/`; no spec/SCHEMA/wire/ADR change; Slice 53 below; suite
386→395/0 + ruff clean + independent code-review pass. 2026-07-24 was the #217 paper-parity plan-section
rule (Slice 52). 2026-07-23 was matcher review-follow-ons **#209 + #210** — an advisory-tier,
ADR-free TDD bugfix wholly inside `matcher/`; no spec/SCHEMA/wire/ADR change; Slice 51 below; full
matcher suite 386/0 + ruff clean + independent code-review pass. Earlier still: the #216/ADR-0058
grade-gated `t_effective` ceiling (2026-07-22→23), the #208/ADR-0057 generic-reprojection build
(2026-07-21), the #200/#206/#205 P6 design-session trio → ADR-0056/0055/0054 (2026-07-19→20), the P5
process-mechanization session (2026-07-19), #204/ADR-0053 + #189+#92/ADR-0052 (2026-07-17→18), and the
2026-07-16 P1/P2/ADR-0051 arc — full detail in each's own condensed block below + the NEXT block +
ROADMAP. Last full regeneration 2026-07-14) · **Spec/ADRs:** v0.61 (through
ADR-0059) · **Phase:** architecture complete (every original §11 question closed);
**first production clinical surface under construction** on `cairn-node`. Built so far
(full detail in ROADMAP + the ADR log + git):
**demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names ·
administrative-sex/gender-identity · §4.3 address; karyotype resolved as a distinct field,
ADR-0037, no code yet) ·
the **§5.2 advisory Python matcher** (piece A in-DB veto floor · B1 scoring core · B2/B2b
veto-gated pipeline/blocking · the B3 eval harness, compound blocking keys, synthetic volume
generator, supervised Fellegi–Sunter weight-learning · range-DOB/composite-sex evidence scoring) ·
the **§5.7 identity core C1–C5** (linkage · human-accepted apply seam · auto-apply band · dispute ·
identify · repudiate + the known-alias pool — the confirmed/unconfirmed/under-review contract is
COMPLETE; C5+ `reattribute` waits on a clinical-note surface) ·
the **§5.4 John-Doe subsystem** (slices A–D + finishers 1–3 + photo/text evidence + the
`enroll-human` ceremony CLI; still open: the §5.12 push-alert + the search-before-create funnel) ·
the **first clinical-content stream `clinical.medication`, slices 1–6a** (assert/cease + the E1
reconciliation flag · bitemporal dose timeline · cross-thread reconciliation links, ADR-0047 ·
the attestation responsibility overlay, ADR-0049 · per-field dose effective/reason correction,
ADR-0050 · the inline `substance.coding` drug-identity shape, ADR-0059) + the **twin-check registry**
(ADR-0048) ·
the **contributor-role vocabulary floor** (ADR-0051 — `recorded` ratified, `{held_by}` responsibility
objects, partition-prefixed future members, strict-submit/lenient-apply) ·
**born-sealed clinical bodies** (ADR-0052 — every clinical JSONB body sealed at write under a per-event
DEK held by the node itself, an erasability substrate not confidentiality; `db/037` custody plane
`event_dek`/`event_clear`/`erasure_shred_log`, both doors enforce sealed⇒clinical scope, all 7
medication verbs seal-at-write, custody sidecar + rung-3 shred CLI; twin registry 18→19) ·
**per-write human authorship** (ADR-0053 — a clinical event carries an authenticated human author
`{human,authored}`+`{node,recorded}`, human signs / node holds custody; `cairn_authorship_bound` strict-door
binding; db/020 admits+grades; `--author-as`) ·
the **L3 reference-UI shell, slice 1** (framework SETTLED — iced FAILS the accessibility bar,
pivot to **Tauri 2**, an L3 choice below the compatibility boundary; PR #174) ·
**generic reprojection** (ADR-0057, spec v0.59 — one registered `cairn_projection_apply` fn per
projection + a single `cairn_projection_dispatch` trigger replacing the ~15 per-type projection
triggers; `cairn_reproject` heal/rebuild run gen-gated by both loaders; the every-connect
`cairn_demographic_backfill` retired; measured at Bet-B volume).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

**Session (2026-07-27, latest) — `clinical.medication` slice 6a: the inline `substance.coding` shape
(implements [ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md); code-only,
no spec/ADR change; ROADMAP Slice 56; branch `feat/medication-coding-slice-6a-0059`; six tasks, each
independently reviewed clean).** `SubstanceCoding {system, code, display}` replaces the reserved
`inn_code` slot (`b44d56b`; the legibility twin appends the captured INN label only when it differs from
the clinician's own term); the node surface gains a pure all-or-nothing `coding_from_parts` +
`--coding-system/--coding-code/--coding-display` CLI flags swept across ~25 call sites (`ace041b`).
`db/041_medication_coding.sql` adds the `medication_coding_system` vocabulary registry (register-by-row,
so substituting a drug-identity authority is a row, not a patch) and a **two-tier** floor: structural
gaps (system/code/display non-empty) refuse at BOTH doors like `substance.term`; registry-derived checks
(unknown system, non-canonical uuid) are strict-submit/lenient-apply (`ca7d5ea`/`e2d8ced`;
`SCHEMA_GENERATION` 40→41). `medication_coding` lands as its **own projection table** — not columns on
`medication_statement`, so slice 6b's coding overlays add rows instead of rewriting view bodies — with
the coding triple widened coherently across the db/031/032/033 read views and retraction-safety pinned:
an uncoded re-assert, or an explicit JSON `"coding": null`, can never silently clear an existing coding
(`f7b8d76`/`5594ab2`). The E1 dup-key becomes `coalesce('code:'||system||'|'||code,
'term:'||normalized-term)` — the **pair**, never a bare code, else the reserved finer drugref tree levels
re-split the same substance cross-node; `medication_group_display` now prefers a coded member; a new
advisory `medication_group_coding_conflict` view flags two different anchors inside one reconciled group
(`6e777c4`). Honest degradation is proven **by construction**, not by mocking absence: a source guard
(three review rounds narrowing string/macro exemptions down to a structural residue check) pins that no
`.sql`/`.rs` file under `db/`, `crates/` or `extensions/` (`target/` and `tests/` skipped) references
drugref executably, plus a `clinical_pull`
cross-node coding-convergence assertion (`fb30ce9`/`d92ad8a`/`93ee103`/`c44b311`). **Three findings changed
shipped behaviour:** db/020's `cairn.remote_apply` marker was raised AFTER the twin/floor dispatch, so no
per-type check_fn could ever see it at the remote door — moved to precede the floor call, verified
against all 4 existing readers (all projection-layer, unaffected); the strict door now pins the
**canonical** UUID spelling (Postgres accepts braced/uppercase/unhyphenated forms, which the
TEXT-compared dup-key would otherwise split permanently once frozen into a signed body); and
**PR-review finding (critical): `cairn_execute_shred` did not scrub `medication_coding`** — a shred that
reported success left `coding_display` (the drug's preferred name) and `coding_code` (the immortal moiety
anchor) readable next to `patient_id` in a `cairn_agent`-readable table, the ADR-0005 rung-3 / #92(b)
failure verbatim and a recurrence of db/037's own earlier "finding #2" (which added the other four verbs'
projections to the scrub for exactly this reason). db/037 now scrubs it by
the same provenance-precise `content_address = v_ca` key as its five siblings, pinned by
`shred_scrubs_the_drug_coding_projection` (the pre-existing sibling test asserts an UNCODED input, so it
never wrote a coding row and could not have caught this). Two review minors also changed behaviour:
`medication_coding.patient_id` is now sourced from the thread's STANDING chart
(`cairn_medication_thread_patient`) rather than `e.patient_id`, so a stale cross-patient re-assert that
LOSES the statement's overlay race can no longer file the coding under the losing event's patient (#192);
and `medication_coding_system.system` gained a shape CHECK (non-blank, no `|`) via a paired ALTER (#207),
because `|` is the load-bearing separator in the flattened `<system>|<code>` dup-key and a system named
`a|b` would silently collide two different substances into one duplicate group. Filed
**[#294](https://github.com/cairn-ehr/cairn-ehr/issues/294)** — the §5.9 safety projection must *carry*
the coding-derived drug class (ADR-0059 decision 4) rather than re-derive it, owed by the future
safety-projection slice (blocked on #232). **Deliberately NOT done, stated honestly:** the two
coding-overlay event types (`clinical.medication-coding.asserted` + its correction) are **slice 6b**; the
coded↔uncoded duplicate case is not closed (only coded↔coded is); no drugref code exists anywhere in the
tree; the §5.9 safety class is not captured. **Unblocked follow-on (next session): slice 6b** — the two
coding-overlay event types in a new `db/` file, re-routing the same-column-set views through an
effective-coding view. Four notes bind it: the overlay apply-write needs the same
`jsonb_typeof(...) IS DISTINCT FROM 'null'` guard the assert path needed; `medication_coding` becomes
a **multi-type** table once the overlays write it — `db/039`'s `cairn_reproject` already refuses a narrow
prefix over a multi-type table; the overlay's own scrub is already covered (db/037 keys on
`content_address`, so whichever event produced the surviving row is the one a shred erases) but any
FURTHER coding table 6b adds must be added to `cairn_execute_shred` in the same commit; and — the sharp
one — 6a's `patient_id = cairn_medication_thread_patient(...)` is only NON-NULL because the assert path
upserts `medication_statement` immediately before it. A coding OVERLAY may legitimately arrive BEFORE the
assert it codes (the table has no FK precisely for that arrival-order independence), where that lookup
returns NULL and would violate the NOT NULL column, so 6b's apply fn needs its own answer — carry the
overlay event's `patient_id`, or defer the row until the thread exists.

**Session (2026-07-25) — the Cairn↔drugref medication drug-coding seam → [ADR-0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)**
(design-only, spec v0.61, ROADMAP Slice 55; full narrative there — the wire-shape decisions, the ICD-11
divergence rationale, and the five pre-merge review corrections; not restated here). **Implemented as
slice 6a, 2026-07-27** (see the session block above).

**2026-07-25, earlier — matcher #290: wire `repaired_record_ids` into eval reporting** (advisory Python
tier; no spec/SCHEMA/wire/ADR change; full detail: ROADMAP Slice 54). The generator's `_repair` marks a
clone `repaired: True`; eval consumers now REPORT the count everywhere (never exclude, never change the
learned model) via a shared `dataset.repaired_truth_pairs` primitive threaded through
`ScorerMetrics`/`LiftReport`/`LearnMetadata`. Suite 395→409/0 + independent code-review pass clean.
Remaining: [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (hub-scale re-scoring cost).

**2026-07-25 — matcher four-gap batch #211** (advisory Python tier; the E3 batch from the 2026-07-15
review; full detail: ROADMAP Slice 53). Alias-map canonicalization now matches the trust-map (closes a
latent known-alias REVIEW-forcing bypass); `Thresholds` refuses an inverted `review > auto`; a
SQL-`lower()`-vs-scorer-`casefold()` doc-honesty fix; `_repair` clones marked `repaired: True` (the seam
#290 consumes). Suite 386→395/0. Closed #211; remaining [#287](https://github.com/cairn-ehr/cairn-ehr/issues/287).

**2026-07-24 — the #217 paper-parity plan-section rule** (process + tooling; no spec/ADR/wire/SCHEMA
change; full detail: ROADMAP Slice 52). Every clinical-surface slice plan now carries a
`## Paper-parity benchmark (§1.2)` section or a forced-rationale escape, enforced by
`crates/cairn-node/tests/paper_parity_plan_section.rs`. Filed
**[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)** — med-list whole-list sign-off must
collapse to one human gesture, owed by the future Tauri med-list slice. **The 2026-07-15 review course is
now FULLY closed.**

**2026-07-23 — matcher review-follow-ons #209 + #210** (advisory Python tier; no spec/SCHEMA/wire/ADR
change; full detail: ROADMAP Slice 51). **#209:** `derive_thresholds` now fails closed on an empty
non-match set instead of anchoring `auto` to the weakest true match (a latent false-auto-link risk);
`kfold_lift` skips such folds. **#210:** a sweep-level reconciliation pass now re-scores every PENDING
proposal the sweep didn't regenerate, so a pair that left the blocking universe (e.g. a Doe fully
identified) can no longer be stuck under a stale REVIEW row forever. Suite 386/0. Remaining:
[#287](https://github.com/cairn-ehr/cairn-ehr/issues/287) (future opt).

**2026-07-20→21 — the #208 generic-reprojection build → ADR-0057, spec v0.59** (brainstorm→spec→plan→
subagent-driven TDD, Tasks 0–10; full narrative incl. the Bet-B performance measurements: ROADMAP Slice
49). One registered `cairn_projection_apply` fn per projection + one `cairn_projection_dispatch` trigger
replaces the ~15 per-type triggers; `cairn_reproject` is the generic heal/rebuild replay both loaders run
on a schema-generation change; `cairn_replay_eligible` is the #265/#266 seam. Open follow-ons:
[#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (the authoritative Pi5/NVMe same-rig re-run —
the Mac Bet-B numbers are cross-rig) and [#277](https://github.com/cairn-ehr/cairn-ehr/issues/277) (the
loader's gen-change heal cannot re-derive `ON CONFLICT DO NOTHING` projections after an extraction-logic
fix). [#273](https://github.com/cairn-ehr/cairn-ehr/issues/273) ✅ resolved in the same PR (#278).

**2026-07-19 → 07-20 — the P6 design-session trio → ADR-0054/0055/0056** (mechanism + all follow-on
issue numbers: the Priority-6 bullets above + ROADMAP Slices 46–48; not restated here). Not captured
elsewhere: open follow-ons **#94** + the key-loss-ceremony ADR + the rotate-key local door (ADR-0054);
**operational caveat** — pre-wire unsigned actor rows never sync, wipe dev rigs; **the posture triad** —
content plane admits-and-disputes (0054) *and* admits-and-defers (0056), code plane verifies-or-refuses
(0055).

**2026-07-19, earlier — the P5 process-mechanization session** #212+#213+#214+#215, closing the whole
review course (PRs #253+#255, merged; full detail: ROADMAP Slice 45; headline outcomes in the P5 bullet
above). Notable: the session's first property-test suite (proptest) immediately caught a real grading
defect (`classify_authorship_confidence` mis-graded an anonymous bearing claim `Device` not
`Unverified`), fixed pre-#245.

**2026-07-17→18 — ADR-0052 born-sealed + ADR-0053 authoring-human** (full detail: ROADMAP Slices 42–43).
Standing notes that outlive the slices: authorship grading stays **half-live until #245** wires a read
path for `classify_authorship_confidence`; [#247](https://github.com/cairn-ehr/cairn-ehr/issues/247) —
authorship in a contributor set is **key-scoped** (doesn't survive key rotation; constrains #245); a
`--author-as` event is *owned* under the ADR-0043 suppression gate where a device-signed equivalent was
dismissable by anyone; born-sealed is an erasability substrate, NOT confidentiality, until
[#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) (unwrap-cert kid pinning) lands; test DBs need
cairn_pgx ≥ 0.3.0.

**GUI/L3 design threads (2026-07-16 + 07-18, design-only; full detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and [`easygp-editing-area-inventory.md`](../scratch/ui-sketches/easygp-editing-area-inventory.md)).**
easyGP mining (screenshots + developer-guide chapters; source material git-ignored under
`docs/untracked_for_brainstorming/` — real photos, **never commit or publish**). Headline: easyGP's six
editing-area invariants ≅ Cairn's event envelope near line-for-line — external validation that the
envelope is the right user-facing grammar. Live outputs awaiting graduation: **ten GUI principles**
queued for the shell spec (one entry grammar; type-ahead primary; auto-fill to the fork; state ambient
never modal; vocabulary never blocks; session folds; documents = previewed projections; record-as-book
incl. the audit overlay; the drawing hand; per-user geometry) + a **GP-manifest seed** + eleven
principle-4 prior-art exhibits. **Open:** co-author questions in the editing-area note §7;
results-inbox screenshots pending (the three-zone vs two-pane question rides on them — don't
improvise it). **Team/scope:** the easyGP co-author may return to lead **GP-facing GUI design**; HH
designs **ED & ward** once core infra is nailed down; the shell's role-manifest layer is the seam
(uniform core, plural edges — ADR-0021 working as intended).

**2026-07-16 — the P1 floor-hardening slice + the full P2 arc + ADR-0051** contributor-role vocabulary
floor (full detail: ROADMAP Slices 36–41 + the PRs + git). **Operational caveat, by design:**
pre-ADR-0051 event logs (old `role:"author"`-without-actor_id, flat-string responsibility) REFUSE at
db/020 — **wipe dev/PoC rigs** (replication-failover demo, spike rigs), never sync them through.

**Earlier sessions (2026-07-09 → 07-15), condensed — full detail in git + the PRs + the linked ADRs +
ROADMAP's condensed "Slices 13–35" block:** **medication slices 1–5** (assert/cease + E1 flag · bitemporal dose timeline `db/032`
· cross-thread reconciliation ADR-0047 `db/033` · attestation overlay ADR-0049 `db/034` · per-field dose
correction ADR-0050 `db/035`; open [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) db/032
suppression PK-eviction) · the **twin-check registry refactor** (ADR-0048) · the **reference-UI verdict**
(iced FAILS a11y → Tauri 2; PR #174) · the **enroll dual-mapping guard** (ADR-0046, closes #166; open
[#172](https://github.com/cairn-ehr/cairn-ehr/issues/172)) · the **`enroll-human` CLI** + §5.4 finishers 1–3
(open [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168)) · **collation-independent tiebreaks**
(ADR-0045, closes #69) + #159 drift guard · the **HLC-collision advisory log** (`db/029`) + `content_address`
tiebreaker (#115 pt 1) · the **enroll `actor_id` collision floor** (ADR-0044, closes #152) · the
**suppression owner-gate** (ADR-0043; #154 later closed structurally by ADR-0054).

**Merged 2026-07-08 (condensed — full detail in git + the PRs + ROADMAP Phase 1).** §5.4 marks/belongings/EMS-context text identity evidence (PR #142, three text `kind` values on the existing `identity.evidence.asserted` type, no floor/SCHEMA/ADR/spec change) + a CI/tooling catch-up day (PRs #143/#147/#149/#150/#151: fmt gate, cargo-deny, `matcher.yml`, toolchain pin, PG16→18 CI, CodeQL crypto FP fix → house rule 6, matcher test-leak/retraction fixes). Closed [#144]/[#145]/[#146]/[#117]/[#135]/[#84 pt1].

**Earlier sessions (2026-06-25 → 07-08), condensed** — demographics slices 1–5 + gaps A/B/C (§4.2–4.6, ADR-0032→0038); the §5.2 matcher pieces A/B1 + the B2→B3 pipeline; the globalised author twin (ADR-0039); the identity C1/C2 apply doors + the quarantine/legibility trilogy (ADR-0040); §5.4 John-Doe slice A + photo evidence (ADR-0042); ADR-0026 node durability B/C/D + Spike 0003 (Postgres-on-Android). **Full detail: ROADMAP + the ADR log + git.**

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

- **First federating node** — built 2026-06-21 ([PR #28](https://github.com/cairn-ehr/cairn-ehr/pull/28)),
  the first implementation of [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md),
  scoped to direct-pairwise trust, no clinical surface: `cairn-node` (Ed25519 keystore,
  `init`/`identity`/pairing/`peers`/`unpeer`, built-in mTLS pinned to the trust set, set-union `node_event`
  sync, honest `status`) + the `db/007` submit/apply doors with a deny-all admission gate. Genesis-stable
  `node_id` = content-address of the genesis enrollment event. **Every honest gap declared at build time is
  CLOSED** (full detail in git + ROADMAP Phases 5/6), including all four
  [ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) durability slices A–D — only
  optional escrow *rungs* (Shamir M-of-N / QR / TPM) remain, upward options, not blockers. The `localstate`
  DB read/apply **seams** are where the future clinical tier plugs DEKs/drafts/config.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`), so plain `cargo test --workspace`
  is reliable.
- **Spike 0002 (advisory-actor write contract)** — ran 2026-06-21, **C1–C5 PASS**
  ([PR #27](https://github.com/cairn-ehr/cairn-ehr/pull/27)) → ADR-0029 + ADR-0030: the in-DB floor held
  against a hostile agent with direct DB access, all rejections legible. Every deferred item since closed
  (the attestation success path E2E, the recall-surface trio, the skeletal twin → ADR-0039).
- **Dual-identifier discipline** — ADR-0031 ([PR #34](https://github.com/cairn-ehr/cairn-ehr/pull/34)):
  the canonical plane (UUIDv7 + multihash) is the *only* identifier on the wire/in signed bodies; the
  projection plane may intern to node-local `bigint` surrogates (`db/008` + the leakage guard). The
  `local_ref` "type barrier" honesty fix merged 2026-06-24
  ([PR #43](https://github.com/cairn-ehr/cairn-ehr/pull/43), issue #35 — the domain is an intent-signal +
  one-directional guard; the load-bearing guarantee is the typed signed plane). Final magnitude measured
  on Bet B.
- **Spike 0003 (Postgres on Android)** — ran 2026-06-25, **G0–G3 PASS**
  ([PR #47](https://github.com/cairn-ehr/cairn-ehr/pull/47) + [PR #48](https://github.com/cairn-ehr/cairn-ehr/pull/48)):
  native PG 18.2 + a cross-built pgrx extension on a RedMagic 11 Pro — no Termux userland, no root, no VM
  (fractal topology at the phone tier). Runnable kit at [`poc/pg-android-kit/`](../poc/pg-android-kit/).
  Remaining non-load-bearing gaps: from-source PG build, APK/`jniLibs` packaging.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **`clinical.medication` — slice 6b next** (the live clinical build front). Slices 1–5 (assert/cease ·
  dose correction/timeline · reconciliation ADR-0047 · attestation ADR-0049 · dose correction ADR-0050)
  and **6a** (the inline `substance.coding` shape, ADR-0059, ROADMAP Slice 56) are DONE. **Slice 6b**
  (unblocked): the two coding-overlay event types (`clinical.medication-coding.asserted` +
  `-correction.asserted`) + an effective-coding view — see the ⇒ NEXT block for the two ledger notes that
  bind it. **Other next candidates:** fuzzy/automatic reconciliation + a Tier-A drug dictionary
  (brand↔generic/DDI beyond the exact-anchor case ADR-0059 closes); structured sig/frequency (lands with
  prescriptions); correcting a dose event's *effective date* on the statement-level `started`; the §5.9
  safety-projection drug-class carry ([#294](https://github.com/cairn-ehr/cairn-ehr/issues/294), blocked
  on #232). **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision**);
  [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the medication/dose/
  reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize-guard
  remote-apply test). Spine to reuse: `db/031`–`db/033`, `db/041` + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine to reuse: `db/010`–`db/030` +
  `cairn-event::demographics`; everything listed in the Phase paragraph above is BUILT — demographics
  slices 1–5, matcher A/B1/B2/B2b/B3, identity C1–C5, the §5.4 John-Doe subsystem).
  **Next (B3 measurement-driven):** a **large hand-crafted gold set** to re-run the learner for
  authoritative magnitudes (slice 24's learner is a PoC on small/synthetic data); locale comparator packs;
  the hub-tier aggressive duplicate sweep; proposal retraction; richer §7.5 matcher-actor determinants
  (served-model digest). **Next identity:** C5+ `reattribute` (§5.5 event-granular strike-through of
  *clinical documentation* — **waits on a clinical-note surface**; note a pending+disputed Doe already reads
  `'under-review'`, severity-max, so the slice-D forcing rule stands down while a dispute is open); the
  §5.12 "prior history now available" push-alert; the §5.3/§5.8 search-before-create funnel.
  Karyotype is resolved as a distinct field ([ADR-0037](spec/decisions/0037-demographic-administrative-sex-and-per-field-winner-policy.md)) —
  no code yet. Smaller deferred items live in the issues:
  [#79](https://github.com/cairn-ehr/cairn-ehr/issues/79) (B2 minors),
  [#168](https://github.com/cairn-ehr/cairn-ehr/issues/168) (entity→role-actor 1:many); plus (unfiled, in
  code comments): repudiation reversal event + a chart-history VIEW of struck names; fuzzy alias
  recognition + an `alias` blocking pass; fuzzy near-window range softening; volume-generator hard
  negatives / variable cluster size; a veto-aware end-to-end scorer mode; deceased-status veto (stub in
  db/016); a `compare_address` comparator; a CLI sweep entry.
  **Test env:** Rust DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx); the multi-node convergence suites additionally need
  `CAIRN_TEST_PG2`/`PG3` pointing at `cairn_test2`/`cairn_test3` on the same cluster (without them those
  tests self-skip locally — CI sets all three since #199). Matcher integration: `cd matcher &&
  CAIRN_TEST_PG=… uv run --extra pipeline pytest`. The pure matcher suite is dependency-free:
  `cd matcher && uv run pytest` (uv, never venv/pip).
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay + key-custody +
  actor primitives have absorbed every case so far without new architecture. Bring a real ED/hospital failure mode.
  The record now lives in [`docs/case-studies/`](case-studies/README.md). First entry
  ([Case 0001](case-studies/0001-improving-practice-software-column.md), 2026-07-11): 16 Australian GP-software
  failure modes from Dr Oliver Frank's magazine column — all absorbed, **0 new architecture**, but three action
  items surfaced: **① re-affirmation-without-change currency** (two timestamps on one fact —
  `asserted-since` vs `confirmed-current-as-of`) — **checked against code → [issue #163](https://github.com/cairn-ehr/cairn-ehr/issues/163)**:
  the envelope already records a re-affirmation (append-only, distinct `content_address`), so no can't-retrofit
  gap; the gap is that every `patient_*` projection (`db/010`–`db/014`) collapses both timestamps into one
  overwrite-on-reaffirm winner-HLC triple, and `first_seen`/`updated_at` are local non-convergent
  `clock_timestamp()` stamps; **② open-loop/obligation** (order/recall/referral with no closing ack) may warrant a named
  projection, and must be surfaced by salience not a modal (paper-parity); **③ impossible-vs-uncertain** constraint
  rule for the in-DB floor (reject only the physically/type-impossible, advisorily flag the merely improbable).
- **Dedupe transitive RustCrypto dep versions** in `Cargo.lock` ([issue #11](https://github.com/cairn-ehr/cairn-ehr/issues/11)) — supply-chain
  hygiene. **Re-verified 2026-06-25: still blocked on upstream** — the `postgres` stack pulls `digest 0.11`/`sha2 0.11`/`chacha20 0.10`
  while `chacha20poly1305 0.10.1` still depends on `chacha20 0.9` and `ed25519-dalek` on `digest 0.10`. Not fixable from our `Cargo.toml`; revisit when the ecosystem converges.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already advanced
  across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **PASS twice** — 2026-06-25 (caveated: USB-2 dock, PG16) and the clean 2026-07-07 re-run on PG 18.4 + a
  PCIe NVMe HAT, both caveats resolved
  ([§9.5](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved)):
  B1 p95 **3.99 ms @ 2,004,000 events** (13× under budget), B2 p95 4.5 ms/374-note chart, ~1,515 B/event on
  disk; B4 confirms ADR-0015's BLAKE3 blob-digest default (~4× SHA-256 on Cortex-A76); `cairn_pgx`
  builds+loads on Pi arm64. Artifacts in [`poc/walking-skeleton/results/`](../poc/walking-skeleton/results/).
  **Remaining:** (c) fold the (now un-caveated) B4 number into the ADR-0015 follow-up to drop "provisional"
  from the blob-digest line.
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` (Todo/BMI strip, pure fossils, Research-module
  ranking logic) and open the **results/inbox design session** (the three-zone-layout vs two-pane-shell
  question is parked there — don't improvise it).
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
| [0043](spec/decisions/0043-suppression-self-only-disagreement-is-additive.md) | Suppression is self-only (human-authored content); disagreement is additive; agent advisories dismissable | §9.6/§3.9 (refines 0010/0022) |
| [0044](spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md) | Enroll fails closed on `actor_id` collision with a distinct key; humans carry a person-distinguishing determinant | §7.5 (refines 0011/0029) |
| [0045](spec/decisions/0045-collation-independent-projection-tiebreaks.md) | Collation-independent projection winner tiebreaks (`COLLATE "C"`) | §5.7/§4 (refines principle 1) |
| [0046](spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md) | Enroll fails closed on key→actor dual mapping (B-direction whole-history guard) | §7.5 (refines 0044/0011) |
| [0047](spec/decisions/0047-medication-reconciliation-resolution.md) | Medication reconciliation is a link, not a cessation; symmetric min-UUID collapse; latest-effective group status | §3.15/§3.16 (principle 2; reuses identity linkage) |
| [0048](spec/decisions/0048-twin-check-registry-dispatch.md) | The per-type twin/floor-check registry: one stable dispatcher, register-by-row, unified check-fn signature | §9.6 (refines 0022/0039) |
| [0049](spec/decisions/0049-commitment-based-sign-off-currency.md) | Commitment-based sign-off currency: separable per-thread attestation overlay; staleness by set-commitment compare, not a position pin; supersede, never retract | §3.15/§3.16 (refines 0007, principle 10) |
| [0050](spec/decisions/0050-dose-correction-per-field-patch.md) | Dose correction is a per-field patch: explicit strike sentinel; corrected effective drives current-dose winner selection; correction-note separate from clinical reason | §3.3/§3.6 (refines principle 4) |
| [0051](spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) | Contributor-role vocabulary floor: `recorded` ratified (12th, contributory); responsibility = `{held_by, on_behalf_of?}`; future members partition-prefixed; strict-submit/lenient-apply | §3.9 (refines 0028/0007/0049/0012) |
| [0052](spec/decisions/0052-born-sealed-clinical-bodies.md) | Born-sealed clinical bodies: every clinical JSONB body sealed at write under a per-event DEK held by the node (erasability substrate, not confidentiality); erase ladder always reachable; two doors enforce sealed⇒clinical scope; custody plane + custody sidecar + rung-3 shred | §3.5/§3.8/§5.9 (refines 0005/0006/0026/0048/0051) |
| [0053](spec/decisions/0053-per-write-human-authorship.md) | Per-write human authorship: `{human,authored}`+`{node,recorded}`, human signs while the node seals + holds the DEK; `cairn_authorship_bound` strict-door binding; apply admits + grades | §3.9/§3.10 (refines 0007/0008/0028/0051/0052) |
| [0054](spec/decisions/0054-actor-registry-federation-admit-and-dispute.md) | Actor-registry federation is admit-and-dispute: signed actor-event wire shape on the node plane; derived live-bindings disputed state; content never waits, permissions always wait; adjudication = supersede by human ceremony, never auto-resolved | §7.5/§6.9/§3.12/§5.10 (refines 0011/0044/0046) |
| [0055](spec/decisions/0055-distribution-trust-root-governance-chained-root-document.md) | Distribution trust root: no privileged root — channels with the steward as default anchor; chained threshold-capable root document (N=1 first-class, no expiry); root/release role split; fork-freeze never-silently-pick; transparency log by ADR-0027 reuse; one root shape for §7.6/§7.9/§7.7 | §7.6/§7.9/§7.7/§6.5 (refines 0012/0024; applies 0017/0018) |
| [0056](spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) | Unknown event types are admitted uninterpreted: custody total, interpretation deferred, power earned; strict door still fail-closes (carry what you cannot author); the floor gates effect not presence; refusal + durable re-offer kept as the residual contract | §6.5/§6.3/§3.13 (refines 0012/0022; extends 0054; upholds 0010/0051) |
| [0057](spec/decisions/0057-generic-reprojection-registered-apply-dispatch.md) | Generic reprojection: a projection lives only in its registered apply fn; one dispatcher replaces the ~15 per-type triggers; `cairn_reproject` heal/rebuild is generic replay, run by the loader on a schema-generation change (every-connect backfill retired); `cairn_replay_eligible` is the #266 seam | §9.4/§9.1 (refines 0048/0045; upholds 0056; load-bearing for #266) |
| [0058](spec/decisions/0058-grade-gated-teffective-ceiling.md) | Grade-gated `t_effective` ceiling: a born `clock_grade` bounds the ceiling's rejecting power — `self-asserted`/`unknown` flag-never-reject (principle-4 fix for slow/dead clocks), remote door admits-and-flags never rejects (closes a sync-wedge DoS), interval derived not stored, mint constrained to self-asserted, gate-effect-not-presence; `cairn_clock_health` honest-assembly read; corrects ADR-0027 §6 `upper=RTC`→`RTC+W` | §3.6/§3.17 (refines 0003/0027; upholds 0051/0056) |
| [0059](spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) | Medication drug-identity coding (drug-axis companion to 0025): anchor on `drugref`'s immortal `moiety_uuid` (INN is display, never key); structured `substance.coding {system, code, display}` **replacing** the reserved `inn_code` slot; separately-authored inline-or-overlay coding act (`clinical.medication-coding.asserted` + `-correction.asserted`); **advisory + honest-degrading** — drugref-absent nodes still read/sync/reconcile, and the §5.9 safety class is **captured pre-seal on the coding node and carried**, never re-derived by the reader; dup-key on `(system, code)`, closing coded↔coded only; inline shape shipped as code slice 6a 2026-07-27 (ROADMAP Slice 56), the coding-overlay event types are slice 6b | §3.16/§3.3 (refines 0025/0047; applies 0007/0014/0052/0057) |

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice, see above); 0002 (advisory-actor —
C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓; PR #47/#48); 0004 (iced reference-UI
viability — FAIL on a11y → Tauri 2).
