# HANDOVER — Cairn

## ⇒ NEXT: the 2026-07-15 review course is ✅ FULLY CLOSED (P1–P5). Priority-6 queue all done: #205 → ADR-0054; #206 → ADR-0055; #200 → ADR-0056; #208 → ADR-0057 (generic reprojection, merged PR #274); **#216 ✅ → [ADR-0058](spec/decisions/0058-grade-gated-teffective-ceiling.md)** (grade-gated `t_effective` ceiling, spec v0.60 — a born `clock_grade` gates the ceiling's rejecting power: at `self-asserted`/`unknown` (every node today) the ceiling **flags-never-rejects** a forward `t_effective` (principle-4 fix for slow/dead/absent-RTC clocks), the remote-apply door **admits-and-flags, never rejects** (closes a latent one-event sync-wedge DoS reachable by the Spike-0002 threat model), plus `cairn_clock_health()` the "clock-behind-its-own-HLC" honesty read; anchor-plane follow-ons [#279](https://github.com/cairn-ehr/cairn-ehr/issues/279)–[#283](https://github.com/cairn-ehr/cairn-ehr/issues/283) + [#284](https://github.com/cairn-ehr/cairn-ehr/issues/284)). **Only #217 remains** from the review course (paper-parity benchmark as a required slice-plan section). Matcher review-follow-ons **#209 + #210 ✅** (2026-07-23; advisory-tier ADR-free TDD bugfix, Slice 51: `derive_thresholds` now fails closed on an empty non-match set + `kfold_lift` skips such folds — no impostor ⇒ no safe auto anchor, #209; a sweep-level reconciliation pass retracts pending proposals orphaned when a pair leaves the blocking universe, e.g. a fully-identified Doe, #210). Remaining feature work now unblocked: matcher **[#211](https://github.com/cairn-ehr/cairn-ehr/issues/211)** (minor batch of 4 logic gaps), medication slices 6+.

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
- **#208 (D3)** — a generic reprojection mechanism + the written "a projection fix ships with its
  backfill" rule + one measured full-replay number at Bet-B volume. **Now load-bearing for ADR-0056**
  (#266's reclassify-then-reproject path is exactly this mechanism).
- **#216** — decide the `t_effective` ceiling semantics against ADR-0027's graded interval
  (write-door bound + remote-door quarantine-vs-reject).
- **#217** — make the §1.2 paper-parity benchmark a required section of every clinical-surface
  slice plan, starting with the Tauri client.

**Explicitly deprioritized behind P6, now UNBLOCKED (P1–P5 closed; the "before slice 5" guards —
#214's label fix and the verb-then-vouch factoring — are in):** matcher #209/#210/#211,
further medication slices (6+), matcher B3 measurement work. These are additive, advisory-tier or
well-drilled; nothing above is blocked on them and they get no more expensive by waiting.

---

**Session date:** 2026-07-23, latest (matcher review-follow-ons **#209 + #210** — an advisory-tier,
ADR-free TDD bugfix wholly inside `matcher/`; no spec/SCHEMA/wire/ADR change; Slice 51 below; full
matcher suite 386/0 + ruff clean + independent code-review pass. 2026-07-22→23 was the #216 grade-gated
`t_effective` ceiling build → ADR-0058,
spec v0.60, a brainstorm→spec→plan→subagent-driven-TDD build, Tasks 1–8; born `clock_grade` wire field
+ `db/040` grade-gated classifier + both doors reworked + `cairn_clock_health` + `t_effective_ceiling_flag`;
2026-07-21 had the #208 generic-reprojection build → ADR-0057, spec v0.59, a
brainstorm→spec→plan→TDD subagent-driven build; 2026-07-20 had the #200 design session → ADR-0056
admit-and-defer and, earlier, the #206 design session → ADR-0055 distribution trust root plus PR #264
moving the `clinical_pull` listen ports below the ephemeral floor, issue #263;
2026-07-19 had the #205 design session → ADR-0054, the P5 process-mechanization session
#212/#213/#214/#215 closing the review course, and the P4 tech-debt slice PR #251; 2026-07-18 had
#204 ADR-0053 + the PR #246 fix pass + the GUI/L3 easyGP editing-area mining; 2026-07-17 #189+#92
ADR-0052; 2026-07-16 ADR-0051 + the full P2 arc + the P1 floor-hardening slice; last full
regeneration 2026-07-14) · **Spec/ADRs:** v0.60 (through
ADR-0058) · **Phase:** architecture complete (every original §11 question closed);
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
the **first clinical-content stream `clinical.medication`, slices 1–5** (assert/cease + the E1
reconciliation flag · bitemporal dose timeline · cross-thread reconciliation links, ADR-0047 ·
the attestation responsibility overlay, ADR-0049 · per-field dose effective/reason correction,
ADR-0050) + the **twin-check registry** (ADR-0048) ·
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

**Session (2026-07-23) — matcher review-follow-ons #209 + #210 (advisory Python tier, ROADMAP Slice
51; no spec/SCHEMA/wire/ADR change — TDD bugfix wholly inside `matcher/`).** **#209:**
`eval/learner.derive_thresholds` silently fell back to `review = min(match)` when the labelled scores
held NO non-match pair — anchoring the auto threshold to the *weakest true match*, so ordinary
non-matches band AUTO_CANDIDATE on held-out data (a false auto-link, the matcher's stated dangerous
rate), with `collided` trivially False so nothing flagged it. Now **fails closed** (raises: no impostor ⇒
no safe anchor), and `eval/crossval.kfold_lift` **skips-and-counts** a fold whose training partition has
no non-match pairs (new `_has_nonmatch_pairs`, symmetric with the existing no-match skip), so one
degenerate fold never aborts a cross-validation run — the CLI already turned the raise into a clean exit
2. **#210:** a PENDING proposal whose pair **left the blocking universe** (a Doe forced to REVIEW while
unconfirmed, then fully identified — year-range DOB → point date — so no pass regenerates the pair) was
never re-scored, so `retract_pending_proposal` (only called inside `propose()` for currently-generated
pairs) never fired: a stale REVIEW row grouped a resolved chart under a nonexistent Doe forever. Now
`pipeline/sweep.sweep` runs a **reconciliation pass** — re-`propose()`s every currently-PENDING pair the
sweep did NOT regenerate (new `db.pending_proposal_pairs` reader; new `SweepResult.reconciled` +
`reconciled_retracted` counters — total re-scored vs. the withdrawn subset, the pass's health signal),
reusing propose()'s existing band-None retract path. Re-scoring (not blindly deleting) is deliberate: a
pair withheld only by a block-size cap re-bands and is re-persisted, never wrongly withdrawn; human/auto
dispositions are doubly protected (`WHERE status='pending'` + the retracted→pending upsert arm). TDD
RED-first for both; the #210 test guards that the pair genuinely left the blocking universe (the inverse
of the #135 test's guard) so only the new pass can retract it. Full matcher suite **386/0** + ruff clean +
independent code-review passes (self-review lodged the `reconciled_retracted` split + follow-on #287 — a
hub-scale re-scoring-cost note, correct-behavior not a defect). **Remaining:** #211 (minor batch), #287
(future opt), #217, medication 6+.

**Session (2026-07-20→21) — the #208 generic-reprojection build: ADR-0057 (spec v0.59; ROADMAP
Slice 49; the fourth Priority-6 item, but taken all the way to product code —
brainstorm→spec→plan→subagent-driven TDD build, Tasks 0–10).** The ~15 per-type projection triggers
each healed only *future* inserts (a `CREATE OR REPLACE` left every already-materialised row wrong —
ADR-0045's read-side-only winner fix was the worked example), and the tree's one bespoke
`cairn_demographic_backfill()` re-expressed its trigger's winner logic twice more and scanned the whole
log on **every connect**. Replaced by ONE code path: a locked `cairn_projection_apply` registry + a
single `cairn_projection_dispatch` trigger (each former trigger body refactored to `fn(event_log)`; an
unregistered projection cannot fire) + `cairn_reproject(prefix, rebuild, source)` feeding `event_log`
through the *identical* dispatch (heal = no-delete wrong-winner convergence; rebuild = TRUNCATE+replay
for the wrote-garbage class, refusing a narrow prefix over a multi-type table). The every-connect
backfill is deleted; both loaders run a gen-gated `cairn_reproject('', false, 'loader')` only on a
schema-generation change. `cairn_replay_eligible(e)` is the #265/#266 seam (constantly `true` until
#265). **Three planning amendments mid-build, all ratified:** the Task-1 registration recipe (per-fn
REVOKE + IS-DISTINCT-FROM steady-state guard + `search_path` pin), the Task-4 premise correction
(`cairn_reproject` is registry-driven, so swapped test calls stay RED until the demographic rows
register — a stronger pin than the plan expected), and the Task-8 stop-gate (rebuild tripped the
~30-min gate → optimise the per-event loop to set-based invocation, re-measure, *then* ADR).
**Review-caught fixes:** heal-**before**-stamp in both loaders (a failed heal withholds the stamp → the
next connect retries → the silent-stale window a stamp-then-heal would open is closed); the three
append-only alarm tables (`identity_projection_flag`, `medication_projection_flag`,
`medication_patient_conflict_flag`) dedup replay by **event identity** (`content_address` +
`NULLS DISTINCT`), never by observation shape; heal mode **skips** the `heal_safe=false` counter-shaped
`patient_chart_apply` note-count increment and reports it in `reproject_log.skipped_fns`. **Measured at
Bet-B volume** (Mac dev box, 2,006,000 events / 200 patients): write-path through the live dispatcher
p50 0.076 / p95 0.236 ms (~17× under the Pi B1 budget); heal of the 200,580 applicable events in
2.098 s (~95.6k ev/s set-based, a 5.8× speedup over the 12.2 s per-event PL/pgSQL loop); rebuild of all
2,006,000 in 54:32 — **loop-invariant by construction** (rebuild ≈ re-ingest cost, the live per-row
ingest ran ~1.2 ms/ev on the same corpus; honestly a rare human-supervised recovery op, **not** a
low-latency SLA). The Mac numbers are cross-rig — the authoritative same-rig A/B is the Pi5/NVMe re-run,
[#272](https://github.com/cairn-ehr/cairn-ehr/issues/272). Structural guards: a sole-dispatcher catalog
test (the only `AFTER INSERT` trigger on `event_log` is the dispatcher) + 22/25 registry row-count pins
mirrored in Rust **and** SQL (the #212 two-place pattern). **Follow-ons:** #272 (Pi re-run) +
**[#273](https://github.com/cairn-ehr/cairn-ehr/issues/273)** ✅ RESOLVED (PR #278) — the pre-existing
db/035 gap the conversion surfaced: the dose-correction apply fn's live body had lost the #192
patient-consistency guard call db/032's original carried. Guard restored in db/035's body; and the fn's
`medication_patient_conflict_flag` write (via that guard, on remote-apply) is now declared in its
`cairn_projection_apply` inventory (db/032) — pinned by a registry-completeness test + two behavioral
(local-refuse / remote-converge-and-flag) tests + **[#277](https://github.com/cairn-ehr/cairn-ehr/issues/277)**
— the loader's gen-change heal cannot re-derive `ON CONFLICT DO NOTHING` projections (`medication_dose_*`,
`medication_attestation`) after an extraction-logic fix (`heal_safe=TRUE` is replay-safe, not
auto-healable; caveat now documented at db/005's `heal_safe` definition, surfaced in this PR's review) —
and #266's reclassify-then-reproject path **consumes** this mechanism through the `cairn_replay_eligible`
seam. **Next:** #216/#217, or the unblocked feature work.

**P6 design sessions (2026-07-19 → 07-20), condensed — full detail in ROADMAP Slices 46–48 + the
ADRs + git; the open follow-ons are also in the ⇒ NEXT block above.** Three consecutive docs-only
sessions cleared the review-course design queue ahead of #208. **#205 → ADR-0054** actor-registry
federation (spec v0.56, Slice 46): admit-and-dispute — a signed actor-event wire shape on the node
plane, a **derived disputed state over live bindings** under which registry uncertainty withholds
*permissions* never *content*; adjudication = `supersede` by human ceremony, never auto-resolved;
closes #154 structurally, discharges the #172 sync-door half. Open follow-ons: #94 + the key-loss
ceremony ADR + the rotate-key local door. **Operational caveat:** pre-wire unsigned actor rows never
sync — **wipe dev rigs**. **#206 → ADR-0055** distribution-plane trust-root governance (spec v0.57,
Slice 47): no privileged root — a **channel** `{trust-root chain, transparency log, release stream}`
is the trust unit, the steward only the official channel's default anchor; a chained content-addressed
root document (N=1/M=1 first-class, monotonic pin, no expiry), a root/release role split, a fork-freeze
rule, a verify-or-refuse newest-root load gate, and an honest N=1 posture with a ≥2-of-3 ratchet
tripwire before the first outside-steward deployment. Follow-ons: #257/#258/#259/#260 + #261 (sync-auth
onboarding UX). **#200 → ADR-0056** unknown event types admitted uninterpreted (spec v0.58, Slice 48):
the filed premise was **inverted** — the spec was right, the code fail-closed at `db/020`.
Admit-and-defer — an unknown type is stored verbatim, re-propagated, skeleton-twin rendered, **no
projection rows / no power**; the strict door keeps failing closed (carry what you cannot author);
power is granted only at reclassification, which **re-adjudicates the deferred gates before
reprojecting** — so *no unattested suppression* holds at every instant (this couples to #208/ADR-0057).
PR #271's review folded three corrections into ADR-0056 (sealed-scope is **not** a remote-door refusal
— a refusal there would freeze the seq watermark on a verifiable event; reclassification must
re-adjudicate the attestation/target-exists/cross-author-suppression gates, not merely reproject; the
ADR converted from `file:line` to symbol-level since #265 deletes the cited line) and restored open
#141/#184 the first ROADMAP prune had dropped. Follow-ons: #265–#270. **The posture triad:** content
plane admits-and-disputes (0054) *and* admits-and-defers (0056); code plane verifies-or-refuses (0055).

**Session (2026-07-19, earlier) — the P5 process-mechanization session: #212 + #213 + #214 + #215,
closing the whole review course (PRs #253 + #255, merged; full detail in ROADMAP Slice 45).** Highlights: **#214** — the medication §3.15/§3.16→§3.3 mislabel fixed across
registry rows + Rust mirror + headers; the medication twin-check registrations flip to `ON CONFLICT
DO UPDATE` so **replay converges registry rows to the migration text** (under `DO NOTHING` the fixed
strings could never reach an existing DB — pinned by a tamper-then-replay heal test). **#212** —
the framing drift pair deleted (shared pure `cairn_event::framing` core; caps stay per-plane policy:
node 8 MiB / clinical 64 MiB), the `reviewed_count` pair deleted (derived from
`cairn_medication_thread_readable_count`, the same fn the ADR-0049 false-fresh gate compares
against), the matcher conftest `_SCHEMA_FILES` hand-list (stalled at 025) now fs-derived + guarded,
the verb-then-vouch six-fold copy verified **already factored** by ADR-0052's `seal_sign_submit`,
and the **first property-test suite** (proptest, licence-checked) landed — which immediately
**caught a real defect**: `classify_authorship_confidence` graded an anonymous bearing claim
(missing `actor_id`) as `Device` instead of `Unverified`; fixed pre-#245, before any read path
consumes the grade. A DB-gated hostile-JSONB property pins the twin floor (twin-less ⇒ legible
raise, backend survives). **#213** — keystore zeroization edges closed, house-rule-6 bench literals
derived, the auto_apply advisory-lock leak closed by construction (`ceremony_locked` + one
unconditional unlock), recovery-code normalization now MAPS Crockford-ambiguous glyphs (I/L→1, O→0)
instead of deleting them, and the cairn-gui merge fallback is validated against `offered`. **#215**
— spec v0.55 prose-honesty batch (index.md/CLAUDE.md status currency + a "HANDOVER wins" pointer;
ROADMAP duplicate Slice 30 → 30b; sync.md §6.3 gains the quarantine/re-offer floor rows incl. the
`acked` exception + the unbounded-pen honesty; §6.1 the ADR-0045 same-code caveat + winner-rules-
are-ADR-gated; security.md the human key-loss-ceremony gap + SMS caveat; identity.md the deceased-
veto stub + alias-pool rung-2 note). Also: cairn-gui rustfmt/clippy drift cleaned (tree sits outside
the CI gates), and [#252](https://github.com/cairn-ehr/cairn-ehr/issues/252) filed — quick-xml
RUSTSEC-2026-0194/0195 via wayland-scanner in the gui lock, upstream-blocked (the #11 shape).

**Sessions (2026-07-17→18, condensed — full detail in ROADMAP Slices 42–43 + the PRs):**
**#204/ADR-0053 authoring-human** (Slice 43, PR #246 incl. its fix pass): authenticated human author
`{human,authored}`+`{node,recorded}`, human signs / node keeps custody; strict door refuses forged
authorship, apply admits + grades. Standing notes that outlive the slice: grading is **half-live
until #245** (no read path consumes `classify_authorship_confidence` yet); [#247](https://github.com/cairn-ehr/cairn-ehr/issues/247)
— authorship in contributor sets is **key-scoped** (doesn't survive key rotation; constrains #245);
UI-layer behaviour note — a `--author-as` event is *owned* under the ADR-0043 suppression gate where
the device-signed equivalent was dismissable by anyone.
**#189+#92/ADR-0052 born-sealed** (Slice 42, PR #239 incl. its fix pass): every clinical body sealed
at write, node holds the DEK — erasability substrate, NOT confidentiality until
[#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) (unwrap-cert kid pinning) lands.
**Operational caveats:** pre-ADR-0052 plaintext-clinical rigs must be WIPED; test DBs need
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

**Sessions (2026-07-16, condensed — full detail in ROADMAP Slices 36–41 + the PRs + git; the P1 slice
itself now sits in ROADMAP's condensed "Slices 13–35" block):** the P1
floor-hardening slice (PR #219), the full P2 arc (PRs #221–#225), and **#203+#96/ADR-0051** — the
contributor-role vocabulary floor (Slice 41, PR #229): 12 ratified members (6 bearing + 6
contributory incl. `recorded`), `{held_by, on_behalf_of?}` responsibility objects, future members
partition-prefixed, strict-submit/lenient-apply at both doors. **Operational caveat, by design:**
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
**suppression owner-gate** (ADR-0043; open [#154](https://github.com/cairn-ehr/cairn-ehr/issues/154)).

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
- **`clinical.medication` — next slice** (the live clinical build front). Slices 1 (assert/cease) + 2 (dose
  change/correction overlay + bitemporal dose timeline) + 3 (cross-thread reconciliation — ADR-0047, `db/033`;
  PR #178) + 4 (attestation — ADR-0049, `db/034`; PR #182) + 5 (dose effective/reason per-field correction —
  ADR-0050, `db/035`; corrected effective drives winner selection) are DONE. **Next candidates:**
  a **prefer-INN display term** for reconciled groups; **fuzzy/automatic reconciliation** + the Tier-A drug
  dictionary (brand↔generic/DDI) — human-driven resolution exists, automated *detection* is the gap; structured
  sig/frequency (lands with prescriptions); correcting a dose event's *effective date* on the statement-level
  `started` (slice 5 covers the dose-timeline effective; the assert's `started` is a separate concern).
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread correction
  *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision**);
  [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the medication/dose/
  reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize-guard
  remote-apply test); ~~#177 (cross-patient reconciliation)~~ — **RESOLVED 2026-07-16** with the #192
  patient-consistency slice. Spine to reuse: `db/031`–`db/035` + `cairn-event::medication`.
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

**Ecosystem evals** (`docs/ecosystem/`, neither spec nor ADR): 0001 (kastellan/localmail plugins), 0003
(reference-data sourcing — medicines/terminologies, fed ADR-0025).

**Spikes:** 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice, see above); 0002 (advisory-actor —
C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓; PR #47/#48); 0004 (iced reference-UI
viability — FAIL on a11y → Tauri 2).
