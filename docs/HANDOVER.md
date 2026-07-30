# HANDOVER — Cairn

## ⇒ NEXT

**ADR-0059 is now fully implemented.** Medication slice **6b** shipped 2026-07-28 (branch
`feat/medication-coding-overlay-slice-6b-0059`, ROADMAP Slice 57): the two coding-overlay event types
(`clinical.medication-coding.asserted` + `-correction.asserted`, `db/042`, `SCHEMA_GENERATION` 41→42),
the **strike** back to honest not-yet-coded, and `patient_medication_uncoded` — the coder worklist. The
same branch closed the two 6a hygiene issues **#295** and **#296**. Detail: the session block below and
ROADMAP Slice 57.

**The genuinely next candidates** (nothing blocks a choice between them):

1. **The drugref term→anchor lookup** — the §9 *advisory* tier, and what actually closes the
   **coded↔uncoded** duplicate case ADR-0059 decision 5 deliberately leaves open. Needs a design
   decision first: the cross-service connection model. The slice-6a/6b source guard keeps the trusted
   surface drugref-free and must stay passing.
2. **The §5.9 safety-projection slice** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) —
   the largest unbuilt piece of settled architecture (sequester + the sensitivity stream + the
   de-identified safety projection). Carries [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294):
   the projection must *carry* the coding-derived drug class, never re-derive it.
3. **The med-list UI slice** (Tauri 2 shell) — the first surface that would make a paper-parity *time*
   budget measurable, and it owes [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288): whole-list
   sign-off must collapse to ONE human gesture.
4. **The ADR-0056 code catch-up** (#265→#270) — five filed issues against a settled decision; the door
   still fails closed on an event type it cannot classify.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** The last full pass ran 2026-07-15 (five
passes: in-DB floor, Rust workspace, spec/ADR corpus, matcher, cross-cutting seams —
[report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217) and is
**fully closed**.

---

**Session date:** 2026-07-28 (Slice 57) · **Spec/ADRs:** v0.61 (through ADR-0059) · **Phase:**
architecture complete (every original §11 question closed); **first production clinical surface under
construction** on `cairn-node`.

**Built so far** (full detail in ROADMAP + the ADR log + git):
**demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/
gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet) ·
the **§5.2 advisory Python matcher** (in-DB veto floor · scoring core · veto-gated pipeline/blocking ·
the B3 eval harness, compound blocking keys, synthetic volume generator, supervised Fellegi–Sunter
weight-learning) ·
the **§5.7 identity core C1–C5** (linkage · human-accepted apply seam · auto-apply band · dispute ·
identify · repudiate + the known-alias pool — the confirmed/unconfirmed/under-review contract is
COMPLETE; C5+ `reattribute` waits on a clinical-note surface) ·
the **§5.4 John-Doe subsystem** (slices A–D + finishers + photo/text evidence + the `enroll-human`
ceremony CLI; still open: the §5.12 push-alert and the search-before-create funnel) ·
the **first clinical-content stream `clinical.medication`, slices 1–6b** (assert/cease + the E1
reconciliation flag · bitemporal dose timeline · cross-thread reconciliation ADR-0047 · the attestation
responsibility overlay ADR-0049 · per-field dose correction ADR-0050 · the inline `substance.coding`
drug-identity shape and the two coding-overlay verbs, ADR-0059) + the **twin-check registry** (ADR-0048) ·
the **contributor-role vocabulary floor** (ADR-0051) ·
**born-sealed clinical bodies** (ADR-0052 — an erasability substrate, NOT confidentiality until #231) ·
**per-write human authorship** (ADR-0053 — grading half-live until #245) ·
the **L3 reference-UI shell, slice 1** (framework SETTLED — iced FAILS the accessibility bar, pivot to
**Tauri 2**, an L3 choice below the compatibility boundary; PR #174) ·
**generic reprojection** (ADR-0057 — one registered apply fn per projection + one dispatcher).
Viability proven by spikes (walking skeleton, advisory-actor contract, a first federating node,
Postgres-on-Android).

---

**Session (2026-07-28) — `clinical.medication` slice 6b: the coding-overlay event types.** Full
narrative in **ROADMAP Slice 57**; not restated here. The seven things worth carrying forward:

1. **The slice stayed additive**, exactly as 6a's table-not-columns decision promised: both apply fns
   write the existing `medication_coding` table under the existing winner rule, the dup-key degrades
   with **no change at all** (`'code:' || NULL` is NULL), and only two downstream predicates needed an
   edit.
2. **A strike leaves a row, never deletes one** — deleting would break arrival-order independence,
   because a lower-HLC coding arriving after the strike would have nothing to lose the race against.
   `struck` is **generated** (`coding_code IS NULL`), not written — see 7.
3. **`cairn_medication_thread_patient` gained a `medication_coding` arm.** An overlay may legitimately
   arrive before the assert it codes; its row falls back to the coding event's own patient claim, and
   that claim must be visible to the shared #192 guard or a later assert naming a different patient
   would leave two projections permanently disagreeing about the thread's chart. **Consequence to
   remember:** a coder who codes against the wrong chart now blocks the real assert with a legible
   local-door error (remote apply flags rather than refuses).
4. **`medication_coding` is now written by THREE event types**, so `cairn_reproject` refuses a narrow
   single-type prefix rebuild over it (db/039). Expected, and commented at the registration site.
5. **A test that passes can still be worthless.** The group-display test first asserted only
   `coding_display` and went green against the live defect — the struck member was still winning the
   whole row, dragging its term and dose with it. Asserting the *term* alongside made it discriminate.
   Same lesson in #295: the behavioural collation test cannot catch a future unpinning on a
   deterministic-default cluster, so the actual gate had to be a no-DB source guard.
6. **Only `cargo test --workspace` catches guard-scope gaps.** Slice 6a's drugref guard skips `tests/`
   directories but had never met a `#[cfg(test)]` module inside a `src/` file — 6b's unit tests are the
   first. Per-test-binary runs missed it entirely; the full run did not. **Run the whole
   workspace before claiming a slice is done**, and note that `cargo test | tail` masks cargo's exit
   code (an old lesson, still true).
7. **A redundant column is a convergence hazard, not just clutter** — the PR review's sharpest finding
   (detail in ROADMAP Slice 57's review-round paragraph; workspace now 916/0). `medication_coding.struck`
   duplicated `coding_code IS NULL`, and the table has THREE writers: db/031's INLINE coding upsert wrote
   the anchor columns and not `struck`, so an inline coding that WON the HLC race over an earlier-arriving
   strike left a live anchor beside a stale `struck = TRUE` — two honest nodes reading the same events in
   different arrival orders then disagreed. The fix deletes the writer rather than correcting it
   (`GENERATED ALWAYS AS (coding_code IS NULL) STORED`), so a fourth writer inherits the invariant.
   **Two generalisable rules:** when a projection column is derivable from another, generate it; and a
   CHECK constraint is the *wrong* tool on a projection table — a violation aborts the apply and wedges
   that event forever (the ADR-0058 hazard). The same widening also broke the anchor-conflict view's
   `array_agg`, which — unlike `count(DISTINCT …)` — **keeps NULLs**. Nullable-widening an existing
   column means re-reading every aggregate over it.

**Deliberately NOT done, stated honestly:** no drugref code anywhere in the tree; the coded↔uncoded
duplicate case is still open (needs term→anchor resolution); the §5.9 safety class is still owed
(#294, blocked on #232); no coding UI, so no paper-parity *time* budget was measured — 6b exposes a CLI
ops surface, and the budget is owed by the med-list UI slice (#288 neighbourhood). Left open from the
review round: **[#300](https://github.com/cairn-ehr/cairn-ehr/issues/300)** — the coder worklist lists
every uncoded member of an already-coded reconciled group. Duplicated coder work, but filtering them out
could suppress the mis-reconciliation signal 6a built (an anchor never contradicted), so it wants a
clinical opinion rather than a patch.

**Earlier sessions — condensed.** ROADMAP now carries the per-slice detail (Slices 13–35, 36–56 and 57
are each condensed there, with every still-open issue enumerated in full). The arc: demographics slices
1–5 + gaps A/B/C and the §5.2 matcher pieces (2026-06-25 → 07-08) · the identity/John-Doe/medication
build-out and CI catch-up (07-02 → 07-15) · the five-priority review course P1–P5 and the Priority-6
design queue → ADR-0051 through ADR-0058 (07-16 → 07-24) · the matcher follow-on batches #209/#210,
#211, #290 (07-23 → 07-25) · ADR-0059 and medication slices 6a/6b (07-25 → 07-28).

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
- **Tech-debt loop** (2026-07-29): `/techdebt-loop` triages issues into
  `loop:*` labels and drives `/techdebt-next` one fresh headless session per
  issue until the ready backlog is dry (spec:
  `docs/superpowers/specs/2026-07-29-techdebt-loop-skill-design.md`).
  Launch precondition: enable the repo setting "Allow auto-merge" (verified
  OFF 2026-07-30; gh cannot read it — preflight asks for confirmation, the
  worker's merge step enforces it). First run: `--dry-run`, then
  `--max-issues 1`, then unbounded.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **`clinical.medication` — slices 1–6b are DONE** (the live clinical build front; ADR-0059 fully
  implemented as of 2026-07-28). **Next candidates:** the **drugref term→anchor lookup** (§9 advisory
  tier — the thing that actually closes the coded↔uncoded duplicate case; needs a cross-service
  connection-model decision first, and the source guard keeping the trusted surface drugref-free must
  stay passing); fuzzy/automatic reconciliation + a Tier-A drug dictionary (brand↔generic/DDI beyond the
  exact-anchor case ADR-0059 closes); structured sig/frequency (lands with prescriptions); correcting a
  dose event's *effective date* on the statement-level `started`; the §5.9 safety-projection drug-class
  carry ([#294](https://github.com/cairn-ehr/cairn-ehr/issues/294), blocked on #232).
  **Cross-cutting debt:** [#185](https://github.com/cairn-ehr/cairn-ehr/issues/185) (**cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision**);
  [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory onto the medication/dose/
  reconciliation projections; [#176](https://github.com/cairn-ehr/cairn-ehr/issues/176) (oversize-guard
  remote-apply test). Spine to reuse: `db/031`–`db/033`, `db/041`, `db/042` + `cairn-event::medication`.
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
