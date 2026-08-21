# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the cross-cutting authority floor and the operator surface over all three are built; ⇒ C is next.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; **do not
re-derive their decisions.**

- **Part A** (Slice 65, ADR-0062) — graded append-only assertions over an event / a thread / a whole
  chart; the effective grade is the **max** over all three. Computes and reports only.
- **Part B** (Slice 67, ADR-0063) — the precise `{class, severity}` is captured **pre-seal** and sealed
  with the body; a **rung** chosen by the standing grade rides the envelope in the clear. Emits a
  *signal*; enforces nothing.
- **The authority floor** (Slice 68, ADR-0064, spec v0.66) — a protection-removing claim takes effect
  only when a human this node can hold responsible stands behind it: one predicate
  (`cairn_claim_authority`, db/005) at exactly one site (the `NOT EXISTS` in
  `cairn_sensitivity_standing`, db/048), so display coarsening, safety-rung emission and part C's dial
  all inherit it structurally. It gives **#245** its first SQL counterpart — NOT its "mirror" (a word
  both `contributor.rs` and ADR-0064 explicitly retract), and NOT its display half, which stays open.
  **This is the floor part C keys on** — read it before touching sequester.
- **The operator surface** (Slice 69, 2026-08-18; closes #388, #383, #421) — `patient-sensitivity
  <chart>`, the one query that tells the whole truth: the withdrawal worklist (each arm stating whether
  it took effect), deferred `sensitivity.%` events, the standing assertions a custody-thin node cannot
  anchor, safety overclaims, and the **measured** count of sealed medication events held without custody.
  **ADR-0064's §1.2 budget is MET** (errata E1/E2), pinned by a test. Its review follow-ons #434/#435 are
  CLOSED, which opened [#436](https://github.com/cairn-ehr/cairn-ehr/issues/436).
- **⇒ Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)):
  Slice 66 (#231) pinned custody to admission and Slice 68 closed the un-attested-strip hole a
  grade-keyed dial would otherwise have inherited. **What remains is the dial question, sharpened by
  ADR-0064 §8**: a custody dial *derived from* the effective grade is only as strong as its
  most-custodial holder — the grade is node-relative (ADR-0062 decision 9), so a well-custodied peer
  legitimately computes a *lower* grade and hands out the DEK on it, and no amount of authority hardening
  changes that. An **explicit custody act** (a signed `custody.narrowed`-shaped event, not a value
  derived from the sensitivity stream) has no such property. **This is an input to #376, not a decision
  taken — do not treat it as settled.**
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

**Two §5.9 leaks were closed 2026-08-16** (#412, #405). Two facts. **`REVOKE SELECT (column)` is inert
while a table-level grant stands**, so `cairn_agent` holds an explicit 23-column grant on `event_log`
omitting `safety` — and **adding a column to `event_log` now requires granting it in db/049 section 8**
(fail-closed; `safety_read_grants.rs` names the missing one). And the correction that matters most:
**that grant is cost-raising, not a floor** — the column copies a *clear* field of the signed body, so
`cairn_body(signed_bytes) -> 'safety'` still returns it uncoarsened, and the runtime role is a
`cairn_node` member which keeps the table grant (**#425**, **#427**). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 (emission-time coarsening) binds. Whether a node should
attempt one below the envelope AT ALL is **[#432](https://github.com/cairn-ehr/cairn-ehr/issues/432)**.

Slice 65's own follow-ons still open: **#374** (thread resolution resolves only a thread's *current head*
— erratum E4 narrows it), **#378** (the withdrawal rationale is clear text forever and replicates — the UI
must warn at entry today), **#379** (the grade in the twin) and **#436** (the mis-chart withdrawal, when it
arrives by replication). **#374 and #379 each need a DECISION, not a patch** — #374 puts a body read on the
safety-critical grade path, and #379 must choose *which* grade an immutable artefact states and land
together with #283 or the demographic twin-match floor refuses a one-sided widening. Closed:
**#383/#388** (2026-08-18) · **#434/#435/#387** (08-19) · **#381/#382/#385/#439**, **#446/#442/#443** and
**#449/#450/#451/#452/#453/#386**, and **#370/#457** (08-21).

> [!NOTE]
> **CLOSED — the `arrayref` supply-chain incident ([#445](https://github.com/cairn-ehr/cairn-ehr/issues/445), 2026-08-20).**
> A 0.3.10 whose published artifact did not match its own source added a *normal* dependency on
> **`proc-macro1`**, a `proc-macro2` typosquat, reaching `cairn-event` via `blake3 → bao`; fixed by
> `blake3` 1.8.7 the same day, no code change. `bao` was **not** the cause but is stale — `bao-tree` as
> successor is [#454](https://github.com/cairn-ehr/cairn-ehr/issues/454). **The finding that outlived
> it** — `cairn_pgx`'s lockfile was gitignored, so the extension enforcing the in-DB floor re-resolved on
> every CI run — is closed in both halves (#446 tracks every lock, PR #448 passes `--locked`); the full
> narrative lives in `crates/cairn-node/tests/cargo_lockfiles_tracked.rs`.

> [!IMPORTANT]
> **Two code traps that outlive their slices, repeated here because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`.**
>    The catch-all arm reports `'coarsened'`, and `none` is a legal open-vocabulary value that collided
>    with the sentinel (ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds
>    *reject power* (safe); in the sensitivity and safety ladders it would withhold *protection* or mute a
>    warning. Aligning the three is the cleanup most likely to be attempted in good faith, and it reopens
>    a leak. Each `ELSE` carries a shouting comment for that reason — and each is pinned by a test.

**Three things still owed are HUMAN acts an agent cannot do:**

1. **The §1.2 time budget is a seeded figure, not a measured one.** Follow
   [`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md)
   (commands verified working) and record into a dated copy of `TEMPLATE.md`. Only the *write* half is
   measured (median 222 ms — `results/2026-08-03-node-tier-write-cost.md`, **PARTIAL** in its title for
   this reason). Slice 63 owes BOTH halves for registration (≤ 5 s to find an existing chart, ≤ 20 s to
   register a new one); the node-tier write-cost half is
   [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) (nothing is wired; db/044's `gesture_kind`
   CHECK refuses a registration row until widened).
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** ([#444](https://github.com/cairn-ehr/cairn-ehr/issues/444),
   admin-only) — "clippy + cargo test (cairn-gui)" (PR #343: the reference-UI workspace and its JS/Rust
   drift guard) and "cargo doc (API surface)" (#439). Both run on every PR; neither is in main's branch
   protection, so both can go red without blocking a merge. Match the job names exactly — a mismatch
   orphans the required check and blocks every PR silently. `CONTRIBUTING.md` carries the current state
   in a "jobs that run but do not yet block" table, **dated, because branch protection lives on GitHub
   and no gate can keep that table honest**. Note the doc gate is no longer *only* advisory: the
   root-workspace, `--features fixtures` and `cairn_pgx` doc builds all run as the last steps of the
   REQUIRED `test` job, so promoting `doc` now buys speed of signal, not coverage. Only cairn-gui's half
   still depends on an unrequired job.

**If a measurement falls outside its budget, that is the finding — file an issue; do not adjust the
budget to match.**

**The other build candidates** (any can be picked up next; nothing blocks a choice):

1. **The registration/search UI slice** — the picker is the wrong-chart affordance paper has and the
   med-list window does not. **Constraint from Slice 63:** it must **open** a chart, never *retarget* an
   open window — retargeting re-creates the §5.8 item 4 / §5.11 windowing misfile possession semantics
   exist to prevent. Also wires the kept-but-unwired pane/routing/freshness machine.
2. **The drugref term→anchor lookup** — the §9 *advisory* tier, and what actually closes the
   **coded↔uncoded** duplicate case ADR-0059 decision 5 leaves open. Needs a design decision first: the
   cross-service connection model. The slice-6a/6b source guard keeps the trusted surface drugref-free
   and must stay passing. **Slice 67 gave it a second consumer:** `safety_class_map` is the empty seam
   drugref would populate.
3. **The node/actor plane's two divergences** — `db/007` fail-closes on an unmappable type (**#301**) and
   skips-and-advances a verifiable refusal where the clinical plane now pens (**#268**). **Neither is a
   symmetric fix**, and both are `loop:blocked`.

**Standing gate:** whole-project review cycles repeat periodically, and there will be **no release for
clinical use before repeated review cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), findings #187–#217), **fully
closed**. A runnable clinical surface exists that has never been through one — include it next.

**The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
session holds the main repo. **Never start it alongside a human session**: they contend on one cargo lock
and one `test_serial_guard` advisory lock (a stray loop once stretched a session's suites from ~3 min to
~90 min).

> [!TIP]
> **A live IDE contends the same way, and it is not obvious.** rust-analyzer's `cargo check
> --workspace --all-targets` holds the shared `target/` lock, so a narrow `cargo test` blocks before it
> compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE. **The old
> "recreate cairn_test/2/3 after an `event_log` column add" note is OBSOLETE** — since #296 the suites
> build `event_log` rows by name via `jsonb_populate_record`, so the stale-column-order failure is
> structurally closed.

---

**Session date:** 2026-08-21 (four passes — the #439/#382/#385/#381 trap-clearing, the #446/#442/#443 silent-gate pass, the #449–#453/#386 second trap-clearing, then **#370 + #457**, two self-contained defects; plus a doc-currency pass in which ROADMAP gained an **open-issue index**, because condensing had orphaned 22 numbers) · **Spec/ADRs:** v0.66 (through **ADR-0064**, *admit the claim, withhold the power*; ADR-0063 gained erratum E1, **ADR-0064 gained erratum E1**) · **`SCHEMA_GENERATION`:** 49 (`db/049`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** (full detail in ROADMAP + the ADR log + git):

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address; karyotype resolved as a distinct field, ADR-0037, no code yet) · **the
  §5.2 advisory Python matcher** (in-DB veto floor, scoring core, veto-gated pipeline/blocking, the B3
  eval harness, compound blocking keys, volume generator, Fellegi–Sunter weight-learning).
- **The §5.7 identity core C1–C5** — link · human-accepted apply seam · auto-apply band · dispute ·
  identify · repudiate + the known-alias pool. The confirmed/unconfirmed/under-review contract is
  COMPLETE; C5+ `reattribute` waits on a clinical-note surface. **The §5.4 John-Doe subsystem** —
  slices A–D, finishers, photo/text evidence, the `enroll-human` ceremony CLI; §5.12 push-alert open.
- **The §5.3/§5.8 search-before-create funnel** (ADR-0061) — the registration act, its db/045 floor and
  retained-set projection, the advisory db/046 search, `cairn-patient-search`, two CLI verbs, John Doe
  re-expressed onto the same act — plus its **precedence rule** (#345, db/005 step 8b; `patient.created`
  retired in db/047, which handed registration the `patient_chart` chart-birth projection).
- **`clinical.medication` slices 1–6b** — assert/cease + the E1 reconciliation flag · bitemporal dose
  timeline · cross-thread reconciliation (ADR-0047) · attestation responsibility overlay (ADR-0049) ·
  per-field dose correction (ADR-0050) · inline `substance.coding` + two coding-overlay verbs (ADR-0059);
  with the **twin-check registry** (ADR-0048) and the **contributor-role vocabulary floor** (ADR-0051).
- **Born-sealed clinical bodies** (ADR-0052), confidentiality-capable since #231 pinned the unwrap-cert
  `kid` to `trust_peer` · **per-write human authorship** (ADR-0053 — grading half-live until #245) ·
  **the §5.9 stream COMPLETE through its read surface** (parts A/B, the authority floor, the operator
  surface — see ⇒ NEXT). **§5.9 enforces nothing beyond display/emission.**
- **The L3 reference UI** — `cairn-gui/`, a standalone workspace, dependency direction one-way GUI →
  crates. The iced shell FAILED the accessibility bar (spike 0004) and was **retired 2026-08-03**; today
  it is **`cairn-gui-tauri`**, a Tauri 2 window on one patient's medication chart (whole-list sign-off,
  per-row cease, plain-JS semantic HTML, no npm). The pane/routing/freshness state machine survived the
  retirement, is tested, and is **not wired**.
- **The med-list node tier** (Cairn's first clinical READ path + whole-list sign-off + two CLI verbs) ·
  **generic reprojection** (ADR-0057) · the **ADR-0056 admit-uninterpreted floor** + the **residual
  refusal contract** on the clinical plane (a deliberate refusal is penned verbatim, pinned,
  auto-released on repair, and a frozen watermark fails loud).
---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-08-21 (evening) — two self-contained defects: the freeze that hid, the flake that lied

**Closes [#370](https://github.com/cairn-ehr/cairn-ehr/issues/370),
[#457](https://github.com/cairn-ehr/cairn-ehr/issues/457); opens
[#458](https://github.com/cairn-ehr/cairn-ehr/issues/458); no ADR, no migration file, SCHEMA stays 49.
Workspace sweep 1434 passed / 0 failed, `clinical_pull` green in the parallel run.**

1. **⇒ THE ISSUE NAMED ONE FIELD. MEASURED, THE FAMILY WAS NINE — plus four that raised nothing.**
   `cairn_learn_attachment_refs` (db/027) read three fields out of a signed body with no shape check at
   all; #370 named `digest_hex`. On PG 18.1 that one function had **nine** freeze paths across four
   SQLSTATE classes — 22023 (`attachments`/`renditions` non-array *including JSON null*, non-hex,
   odd-length) · 23502 (absent `digest_hex`, a scalar rendition, absent `media_type`) · 22P02
   (fractional `byte_len`) · 22003 (`byte_len` past bigint) — **and four SILENT paths that wrote
   something wrong**: an empty `digest_hex` (the address is `blob_store`'s PRIMARY KEY, so every empty
   reference from every peer collides into ONE row), a negative `byte_len`, a blank `media_type`, and a
   scalar attachment (**#458**, left open on purpose — it raises nothing and belongs to a structural
   floor, not to the reference learner). **Probe the family before fixing the member** — the #453 lesson,
   again, at four times the size.
2. **The rule the fix follows: refuse what already FAILED, plus what was silently WRONG; accept
   everything that already worked.** Every refusal added at a remote door is a new way for a peer's
   clinical event to be penned, so the shapes the old code happened to accept — uppercase hex, an absent
   `byte_len`, a `byte_len` encoded as a digit STRING — are accepted **deliberately** and pinned by
   tests, not left to chance. **Granularity, the question #370 left open, is answered *refuse the
   event*:** P0001 is not a loss (ADR-0056 decision 5 pens, re-offers, auto-releases, and a malformed
   digest is deterministic — exactly the pen's case), while admitting-and-dropping the reference would
   make the record *look* complete while an attachment it names is unrecorded. ADR-0060 governs
   independent **lines** of a composite object; a rendition reference is not another line.
3. **#457 — the harness polled a PORT and never the CHILD.** So EADDRINUSE, a missing `--key` and a panic
   during schema load all produced one 60 s message blaming startup latency, which is why #238's ceiling
   and #263's port floor both aimed at the wrong thing. `crates/cairn-sync/tests/common/serve.rs` now
   watches both, and captures stderr **to a file, never a pipe** — an unread pipe fills and blocks the
   child, i.e. a readiness harness causing the failure it reports. Spawn and wait are now ONE call, so a
   thirteenth test cannot serve on one port and wait on another. **The cause is named, not fixed:**
   always exactly three of twelve, always the full ceiling, macOS, only under a loaded parallel sweep —
   a child ALIVE but not yet at `main` fits, and this repo has hit macOS `_dyld_start` loader stalls
   before. A stall now reports `TimedOut` **with a live pid**, so the next one can be `sample`d.
4. **A load-bearing comment was factually wrong, and it had already steered two rounds of fixes.** The
   port header justified one-port-per-test with *"std's TcpListener does not set SO_REUSEADDR"*. It does,
   on every non-Windows target — verified in the **pinned** toolchain's own source (1.96.0,
   `library/std/src/sys/net/connection/socket/mod.rs:550-553`), and `serve` binds through exactly that
   call, so TIME_WAIT was never a suspect. **Check a socket-behaviour claim against the pinned std source
   before writing it into a comment.**
5. **⇒ AND MUTATION CAUGHT THE SAME SPECIES IN THIS PASS'S OWN WORK.** A new comment claimed dropping a
   `COALESCE` would make a guard fail OPEN; removing it left every test green, because
   `jsonb_array_elements(NULL)` yields zero rows rather than raising — wrong in mechanism *and* unpinned
   in substance. Both halves fixed: the comment now states the property that IS true (the coercion is
   **total** — always a jsonb array, never NULL, which is what makes a future `jsonb_typeof` check safe to
   add, #346) and a test asserts it. Two of seven #457 mutations also proved nothing until rewritten (one
   would not compile; one dropped a pid from a headline while leaving it in the advice below). **A
   mutation that does not change the property tests nothing.**

### The two 2026-08-21 passes — six trap-clearing fixes and three silent gates

**⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`** (#450;
#451 gives the matcher the same guard and the SAME variable — one rule, two languages).
`db_gate_actually_ran.rs` used to bind only when `$CI` was set, and `CI` is set in **zero** places in this
repo, so a scrubbed environment silently disabled the one guard whose whole argument is that unverified
assumptions are how a suite goes green. **The polarity subtlety is the thing to get wrong:** the old `$CI`
predicate read an unrecognised value as *yes, this is CI*, which BOUND the guard — the safe direction. An
**opt-out** must read one as *NOT permission* — the OPPOSITE default — or `CAIRN_ALLOW_DB_SKIP=please`
quietly restores fail-open. Only `1`/`true`/`yes`/`on` opt out. `scripts/run-db-gated-tests.sh` is
unaffected; `matcher.yml`'s deliberately database-free job declares the skip in its own `env:`.

**Three mechanics worth carrying.** (a) **PostgreSQL checks a function called inside a VIEW against the
INVOKING user, not the view owner** (unlike table access), and the INNER call is checked too — so #453's
bare REVOKE over the `cairn_twin_%` family (**four** functions, not the two the issue named) broke
`event_twin_provenance` for `cairn_agent` until two GRANTs were added. Measured on PG 18.1, pinned.
(b) **Ask the authority, do not re-implement it:** `cargo_lockfiles_tracked.rs` asks `cargo locate-project
--workspace` which manifests own a lockfile, and that paid a dividend a hand-rolled parser could not —
**`packaging/crates` was in no workspace and not excluded**, so every cargo command there had been erroring
since the crates graduated to the top-level workspace. (c) **`git check-ignore` needs `--no-index` and has
THREE exit codes** (0 / 1 / **128 error**); without the flag git *skips tracked paths*, and reading 128 as
"clean" was a second independent route to vacuity. The lockfile rule now has **zero** exemptions and
**every repo cargo invocation in `rust.yml` passes `--locked`** (a tracked lockfile is only half of
"pinned"), with `cargo fetch --locked` fronting `cargo pgrx install`, which has no such flag.

**Not done, deliberately:** unifying the 342 bare `else { return }` skip sites would add ~1000 lines of
boilerplate and make **#327**'s job bigger — recorded as a comment there. **Residual: #447** (cargo-deny
covers three of six cargo trees; the two poc trees are pinned but unaudited — not on the clinical path).
The pass's own mutation lessons are folded into the entry above, which found the same species again.
### The §5.9 build passes (Slices 65–69, 2026-08-02 → 08-20) — what outlives them

1. **⇒ A guard defined over the list it guards is not a guard.** Every #387 finding was that shape and
   none was a behaviour bug — `assert_eq!(SubjectKind::ALL.len(), 3)` over an `[SubjectKind; 3]` compared a
   compile-time constant to its own literal and could not fail. **Ask what independent source a guard
   checks against; if the answer is "itself", it is documentation wearing a test's clothes.** The
   constructive form: **where a family HAS an authoritative list, read the list** — #382's applier guard
   reads the `cairn_projection_apply` REGISTRY, not the `_apply` name suffix. And when reading a catalogue,
   **read `proacl` and never assume: a NULL ACL is the PERMISSIVE case** (Postgres's default for a function
   is EXECUTE to PUBLIC); inverting that polarity makes a guard pass by seeing nothing.
2. **⇒ AN OPTIMISATION REMOVED A REDUNDANCY THAT WAS LOAD-BEARING, AND THE COMMENT EXPLAINING IT ASSERTED
   THE EXACT OPPOSITE.** #385's first draft said widening §10b's thread-free list could only over-protect.
   Measured, it is the reverse: §11's conservative bound is gated on the NEGATION of the same predicate, so
   a type added to the list is EXCLUDED from the bound *and* stops resolving — all three thread arms fall
   silent and a standing `sequestered` grade reads back as `('routine','none')`. **Before #385 the
   identical edit was harmless**, because §11's resolved arm was an independent net. Carry: (a) when an
   optimisation makes two code paths share a predicate, ask what redundancy that share destroyed; (b) **a
   wrong safety argument is worse than none — it disarms the guard it describes.** Also: the five thread
   projections now index `content_address`, and **that win is still unmeasured on volume data.**
3. **NAME, NEVER COUNT** — `patient-sensitivity <chart>` is the one query that tells the whole truth, and a
   count cannot separate **custody-blind** from **genuinely empty**, the one question the line exists to
   answer. A later "simplification" back to a count is a regression. Related: **a union view whose arms
   mean opposite things must never get one summary sentence** — the worklist's `stranger-attested` arm DID
   take effect, yet the draft counted it under *"N withdrawal(s) did NOT take effect"*, telling the
   operator a completed, unaccountable removal of protection had not happened. Also **the report declares
   what it cannot contain**, each claim asserted over an **empty** list — the case where silence is most
   convincing and most wrong — and **peer text is not display text** (a newline forged a whole line).
4. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** —
   ADR-0064's KNOWN GAP. A withdrawal mis-stamped with the wrong chart's `patient_id` names a real
   assertion living elsewhere, and `cairn_sensitivity_standing` is patient-scoped on both sides
   (load-bearing — else chart B strips chart A), so the target IS genuinely absent from this chart's
   standing set and a naive membership test reports the withdrawal **effective** — a precise untruth in the
   *reassuring* direction, on a confidentiality surface. **#436 is the residual**, and it is a visibility
   problem, not a door problem: *"on another chart"* is indistinguishable from *"not arrived yet"*.
5. **A pinned `search_path` must deny the temp schema the FIRST look**, not merely end in `pg_temp`. `SET
   search_path = public` does not exclude it, so with a decoy `event_log` in place `submit_event` and
   `apply_remote_event` each **returned SUCCESS while the owner-privileged INSERT landed in the caller's
   temp table** — live data loss at both write doors, demonstrated as `cairn_agent`, a role with no write
   privilege on `event_log` at all. 21 headers gained `, pg_temp`; the guard is over `pg_proc`, not a name
   list. Still open: **#430** (~100 unpinned invoker-rights functions) and **#431**
   (`cairn_execute_shred`, catalogue-only — a diverted shred would report an erasure that never happened).
6. **A parameter name is not a security property.**
   `classify_authorship_confidence(&body.contributors, &body.signer_key_id, None)` compiled, read
   naturally, and graded a forgery `Attested`. Both key arguments are now a `VerifiedKid` newtype
   (mint-site allowlist unpinned: **#428**). **`attester_key` alone is NOT proof** — db/020's deferred arm
   stores a peer's token unverified, which is why SQL's R1 pairs it with `cairn_attestation_vouched`.
7. **Slice 68's two properties:** the authority floor **gates effect, never admission**, and only in the
   withholding direction, so a claim below the bar still lands and converges (no fork — the **#342**
   trap); and **computing the verdict at read cuts both ways** — both routes resolve through
   `actor_current`, so revoking someone *after* their withdrawal landed silently re-raises the grade
   (**#409**), while the Rust↔SQL authority mapping separately diverges on two shapes (**#408**, root
   cause **#413**). Also **flag what cannot self-heal, view what can** — the worklist is a VIEW,
   `safety_overclaim_flag` a LEDGER. The PR #410 review found **7 of 11 production-code mutations survived
   a green suite**. Two mechanics: R2's self-identity equality was unpinned because every un-attested
   fixture used the *device* as both asserter and withdrawer (pinning it needs two DISTINCT human actors),
   and **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** (`OTHERS` excludes `query_canceled`,
   57014). Filed, not fixed: #413–#420, #422; **#415** measures the SIGNER, so it fires on routine care —
   **expect noise**.
8. **Slice 67: the seal boundary is the coarsening boundary.** Precise `{class, severity}` travels sealed
   with the body, a grade-chosen **rung** rides the envelope in the clear, so *coarsen-but-survive* after a
   crypto-shred is structural. **Two coarsenings, load-bearing for DIFFERENT reasons:** emission binds a
   peer's raw-SQL client; read answers a peer that legitimately emitted a finer rung, and **read coarsening
   is a rendering choice, not a floor**. `safety_class_map` ships **EMPTY** — the seam drugref plugs into.
   Open: **#407** (the sealed precise claim is read by nothing), **#406** (no supersession — a ceased drug
   warns forever), #394–#402.
9. **Two Rust traps.** A type named only inside `#[cfg(test)]` compiles clean under `cargo test --lib` and
   fails the integration build under `-D warnings` — use `--all-targets`. And a **chart-scoped definer, not
   a table grant**: `event_deferred` is granted to `cairn_node`, NOT `cairn_agent`, so db/043 gained
   `cairn_patient_deferred_sensitivity(uuid)` granted to BOTH group roles (**#425** owns which role the
   runtime actually connects as).

### The clinical-surface slices (61–66, 2026-08-02 → 08-11) — what outlives them

**Slice 66** (closes #231) pinned the unwrap-cert kid to `trust_peer` (db/007) — before it, any
self-signed cert reaching the serve port obtained read-custody of every non-shredded sealed body.
**Withhold the key, never the bytes** (refusing the bytes forks the event set); repair is TWO steps
(`pull --full`, then `cairn_reproject()`). Same day, cargo-deny v2's `unsound = "none"` default let an
unsound advisory pass in silence — `unsound = "all"` is now set in **both** `deny.toml` trees, with
**#389** ignored with a reason and a review date enforced by `advisory_ignore_review_dates.rs` (there is
no `expires` field in cargo-deny 0.19.9). **Slice 63** (ADR-0061) — carry into any registration work:
**the attestation NAMES the displayed candidates, it does not count them** (*was the duplicate on screen
when the clerk clicked create?* has opposite fixes for yes and no, and `N = 3` cannot separate them); the
§1.2 write-cost half is **#360**. **Slices 61+62** — **a displayed row is a GROUP, an attestation is a
THREAD** (ADR-0047/0049 — nearly every defect lived on that seam); **a unit-tested safety control can
still be defeated by the surface that calls it** (the idle re-lock never fired because a shared accessor
counted every poll as activity, every `SessionKey` unit test passing — **test the path the product
actually calls**); and **a compensating control outside CI is not a control** (`cairn-gui` is a separate
workspace `cargo test --workspace` never covered; the `gui` job now does, ⚠️ still not REQUIRED —
**#444**). Slice 65's traps are in the ⇒ NEXT callout and the part C bullet.

> [!IMPORTANT]
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md):
> *partial validity — a defect on one line never invalidates another.*** Read before any
> composite-clinical-object work: **the system may fail to record an order, but it may never cancel
> one.** Hold decision 2 (partial completion must be reported, never implied) and decision 7 (check the
> transaction boundaries).

**Five repo conventions these runs learned the hard way:**
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it.** A new
  `cairn_decode_hex_or_raise` call site fails `hex_decode_helper.rs`'s
  `every_hex_door_still_calls_the_helper`, which asserts an exact per-file call-site list; the twin and
  projection registries carry the same shape. The count failing is the guard WORKING — fix the list, and
  say in a comment why the new site is there.
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`. Every existing suite does this in execution order.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`, so a `Uuid`
  parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant.** `actor_id` content-addresses the *pinned
  determinant set*, so enrolling two clinicians as `{"role":"clinician"}` collides into one actor and is
  refused (P0001, ADR-0044/[#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)). Use
  `enroll_human_with_role`. The floor working as designed.
- **`cargo test --lib` does not catch an import used only under `cfg(test)`** — it compiles the lib WITH
  `cfg(test)`. The integration build fails it under `-D warnings`. Use `--all-targets` (Slice 69).

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both
tech-debt-loop "Interlude" entries, every still-open issue). Two lessons from Slice 60: **a refusal that
persists nothing is a refusal you cannot audit**, and **when a call site cannot make a distinction, check
whether an intermediate layer threw it away** (`apply_signed` flattened `postgres::Error` to `String`,
discarding the SQLSTATE separating a deliberate refusal from a transient fault). Arc 2026-06-25 → 08-01:
demographics + matcher · identity/John-Doe/medication · the five-priority review course → ADR-0051–0058 ·
ADR-0059 + medication 6a/6b · the ADR-0056 admit-uninterpreted floor · floor determinism (#75) ·
tech-debt-loop launch.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in
[`scratch/ui-sketches/easygp-consult-screen-inventory.md`](../scratch/ui-sketches/easygp-consult-screen-inventory.md)
and `easygp-editing-area-inventory.md` (source screenshots git-ignored under
`docs/untracked_for_brainstorming/` — real photos, **never commit or publish**). Headline: easyGP's six
editing-area invariants ≅ Cairn's event envelope near line-for-line. **Open:** co-author questions in
that note §7; results-inbox screenshots pending — three-zone vs two-pane rides on them, **don't
improvise it**. **Scope:** the easyGP co-author may lead GP-facing GUI, HH designs ED & ward; the
role-manifest layer is the seam (ADR-0021).

**Status of this file:** Disposable working scaffolding, **not** a source of truth. Regenerate at the end
of each session, and **keep it under 500 lines** — its value is inversely proportional to its length
(#368). If it disagrees with the canonical docs, **the canonical docs win.** The *why* lives in the ADR
log, the *what* in the spec; this file carries only what lives *between* them — current build state, open
threads, time-sensitive items.

---

## Read these first (the durable state)

- **`docs/spec/index.md`** — canonical architecture spec (mission prose + document map + spec version).
  One file per aspect; cross-refs like *§5.7* stay valid inside the aspect file.
- **`docs/spec/decisions/README.md`** — the **ADR log index** (the *why*). Numbered, dated, **immutable**
  — a reversal is a new superseding ADR, a factual correction is an appended erratum. **Read the relevant
  ADR before reopening a settled question.**
- **`docs/ROADMAP.md`** — the foundation build order, *below* the policy/GUI line, plus the per-slice
  narrative. Disposable scaffolding like this file; the spec/ADRs win on any disagreement.
- **`docs/spikes/`** — 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor
  — C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓); 0004 (iced UI — FAIL on a11y →
  Tauri 2). **`docs/case-studies/`** — 0001 (2026-07-11): 16 Australian GP-software failure modes, all
  absorbed, **0 new architecture**. **`docs/ecosystem/`** — 0001, 0003. **`docs/principles/`** —
  mission/governance; root **`README.md`** repeats the founding principles.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (2026-06-21, first implementation of
  [ADR-0017](spec/decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)) —
  `cairn-node` (Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the trust set, set-union
  `node_event` sync) + the `db/007` doors with a deny-all admission gate; genesis-stable `node_id`.
  **Every honest gap declared at build time is CLOSED**, including all four ADR-0026 durability slices
  A–D — only optional escrow *rungs* (Shamir/QR/TPM) remain. The `localstate` read/apply **seams** are
  where the clinical tier plugs DEKs/drafts/config.
- **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern to node-local `bigint`
  surrogates (`db/008` + the leakage guard). The load-bearing guarantee is the typed signed plane.
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`); they self-serialize
  cluster-wide via a Postgres advisory lock (`db::test_serial_guard`). Connection strings and the
  DB-slice runner are under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one
  fresh headless session per issue until the ready backlog is dry (`tail -f ~/.cairn-loop/run.log`).
  Auto-merge **ENABLED**; **works unattended** (12 PRs across two runs); **stopped** by maintainer
  decision — see ⇒ NEXT. Cold-start ladder: `--dry-run`, `--max-issues 1` watched, then unbounded. Live
  gaps: **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) — A, B and the authority
  floor all shipped (Slices 65/67/68); **C is unblocked**, its open decision is the dial question (⇒ NEXT).
  Related: **#235** (shred authorization policy hooks), **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059 fully implemented 2026-07-28). **Next
  candidates:** the **drugref term→anchor lookup** (⇒ NEXT item 2); fuzzy/automatic reconciliation + a
  Tier-A drug dictionary; structured sig/frequency (lands with prescriptions); correcting a dose event's
  *effective date* on the statement-level `started`. **Cross-cutting debt: #185** (cross-thread
  correction *suppression* — single-column PK eviction; pre-existing db/032, needs a PK/design decision).
  Spine to reuse: `db/031`–`db/033`, `db/041`, `db/042` + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`; everything under "Built so far" is DONE). **Next (B3
  measurement-driven):** a **large hand-crafted gold set** to re-run the learner for authoritative
  magnitudes (slice 24's is a PoC on synthetic data); locale comparator packs; the hub-tier duplicate
  sweep; proposal retraction; richer §7.5 matcher-actor determinants. **Next identity:** C5+
  `reattribute` (**waits on a clinical-note surface**); the §5.12 push-alert. Deferred: **#168**
  (entity→role-actor 1:many), **#287** (sweep re-scores standing orphans); unfiled ones are in ROADMAP's
  "Still open from slices 36–56".
- **Test env:** DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx), plus `CAIRN_TEST_PG2`/`PG3` (`cairn_test2`/`3`, same
  cluster) for the multi-node convergence suites — without them those **self-skip and cargo counts them
  as passed**, so a workspace count alone cannot distinguish skip from pass (CI sets all three, #199).
  **Since #450 a run without them FAILS unless it declares `CAIRN_ALLOW_DB_SKIP=1`** — in both the Rust
  and the Python suite; only `1`/`true`/`yes`/`on` opts out, an unrecognised value is not permission.
  Matcher integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`; the pure suite is
  dependency-free (`uv run pytest`) — uv, never venv/pip. The `db/tests/*.sql` **mirrors run only via
  `scripts/run-db-sql-tests.sh`**, which drops, recreates and marks a throwaway `cairn_sqltest`: since
  #169 each mirror refuses a database lacking the `cairn_scratch_database` marker, because the mirrors are
  destructive (eight commit; `017` drops constraints). `scripts/run-db-gated-tests.sh` runs the mirrors
  *and* the full workspace with all three connection strings baked in — the one command for the DB slice
  of the local gate. Local gap: [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314) (it does not run
  the matcher DB-gated pytest suite; CI does). **`clinical_pull` used to flake under a full-workspace
  run** — always exactly three of its 12 serve-spawning tests, always the full 60s. #457 fixed the
  DIAGNOSTIC, not a cause: `tests/common/serve.rs` now watches the child as well as the port, so a dead
  child reports its exit status and stderr at once instead of a 60 s message blaming latency. **The
  cause is still unnamed** — if it recurs, the message will now say whether the child died or stalled,
  and a stall names a live pid to `sample`. Serialising (`--test-threads=2`) remains the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the event-overlay +
  key-custody + actor primitives have absorbed every case so far without new architecture. Bring a real
  ED/hospital failure mode; record in [`docs/case-studies/`](case-studies/README.md). Open action items
  from Case 0001: **① re-affirmation-without-change currency** ([#163](https://github.com/cairn-ehr/cairn-ehr/issues/163));
  **② open-loop/obligation** (order/recall/referral with no closing ack) may warrant a named projection,
  surfaced by salience not a modal; **③ impossible-vs-uncertain** constraint rule for the in-DB floor.
- **Landing-page polish** — non-developer page for the generated site (frontend-design; `web/` already
  advanced across PRs #15–#17; draft plans under `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#9-bet-b--results-raspberry-pi-5--8-gb-2026-06-25--pass-with-two-honest-caveats)):
  **PASS twice** (clean 2026-07-07 re-run, PG 18.4 + NVMe HAT, both caveats resolved — B1 p95 **3.99 ms @
  2,004,000 events**, 13× under budget; B4 confirms ADR-0015's BLAKE3 default). **Remaining:** fold the
  un-caveated B4 number into ADR-0015 to drop "provisional" from the blob-digest line, and
  [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (reproject bench on the Pi rig).
- **easyGP session** — port the [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)
  deferred items with live easyGP schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon (validates ADR-0001 from production). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` and open the **results/inbox design session**
  (three-zone vs two-pane is parked there — don't improvise it).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice (the production object-store tier). The §8.2 availability + windowing/resume work shipped.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction (German
  Stiftung/Verein, US 501(c)(3), or an umbrella) — deferred until momentum/funding geography is clearer;
  formal trademark / wordmark registration — principle recorded (stewardship doc), legal instrument
  deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — the working
  conventions, the twelve founding principles (the first four being the lens for every design choice),
  and the §9 defect-blast-radius language rule. Not restated here; canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
