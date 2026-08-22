# HANDOVER — Cairn

## ⇒ NEXT

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the cross-cutting authority floor and the operator surface over all three are built; ⇒ C is next.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) and
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) before touching the rest; **do not
re-derive their decisions.**

- **BUILT.** **Part A** (Slice 65, ADR-0062) — graded append-only assertions over an event / a thread /
  a whole chart, effective grade = **max** over all three; computes and reports only. **Part B**
  (Slice 67, ADR-0063) — the precise `{class, severity}` is captured **pre-seal** and sealed with the
  body, while a **rung** chosen by the standing grade rides the envelope in the clear; emits a
  *signal*, enforces nothing. **The operator surface** (Slice 69) — `patient-sensitivity <chart>`,
  ADR-0064's §1.2 budget **MET** and pinned by a test (residual **#436**).
- **The authority floor** (Slice 68, ADR-0064, spec v0.66) — a protection-removing claim takes effect
  only when a human this node can hold responsible stands behind it: ONE predicate
  (`cairn_claim_authority`, db/005) at exactly ONE site (the `NOT EXISTS` in
  `cairn_sensitivity_standing`, db/048), so display coarsening, safety-rung emission and part C's dial
  all inherit it structurally. It gives **#245** its first SQL counterpart — NOT its "mirror" (a word
  both `contributor.rs` and ADR-0064 retract), and NOT its display half. **Part C keys on this floor.**
- **⇒ Part C — sequester / custody narrowing** ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)):
  Slice 66 pinned custody to admission and Slice 68 closed the un-attested-strip hole a grade-keyed
  dial would otherwise have inherited. **What remains is the dial question, sharpened by
  ADR-0064 §8**: a custody dial *derived from* the effective grade is only as strong as its
  most-custodial holder — the grade is node-relative (ADR-0062 decision 9), so a well-custodied peer
  legitimately computes a *lower* grade and hands out the DEK on it. An **explicit custody act** (a
  signed `custody.narrowed`-shaped event) has no such property. **An input to #376, not a decision
  taken — do not treat it as settled.**
- **Part D — break-glass** ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)): audited key-*use*,
  partition-honest. Blocked on C.

**Two §5.9 leaks were closed 2026-08-16** (#412, #405), leaving two facts. **`REVOKE SELECT (column)`
is inert while a table-level grant stands**, so `cairn_agent` holds an explicit 23-column grant on
`event_log` omitting `safety` — and **adding a column to `event_log` now requires granting it in db/049
section 8** (fail-closed; `safety_read_grants.rs` names the missing one). And the correction that matters
most: **that grant is cost-raising, not a floor** (the column copies a *clear* field of the signed body,
and the runtime role keeps the table grant — **#425**, **#427**). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 binds. Whether a node should attempt one below the
envelope AT ALL is **[#432](https://github.com/cairn-ehr/cairn-ehr/issues/432)**.

Slice 65's own follow-ons still open: **#374** (thread resolution resolves only a thread's *current head*),
**#378** (the withdrawal rationale is clear text forever and replicates — the UI must warn at entry today),
**#379** (the grade in the twin) and **#436** (the mis-chart withdrawal when it arrives by replication).
**#374 and #379 each need a DECISION, not a patch** — #374 puts a body read on the safety-critical grade
path, and #379 must choose *which* grade an immutable artefact states, landing with #283 or the
demographic twin-match floor refuses a one-sided widening. The `arrayref` supply-chain incident (#445) is
**closed** — a typosquat reached `cairn-event` via `blake3 → bao`, fixed upstream the same day, no code
change; one residue, **#454** (`bao` is stale — evaluate `bao-tree`).

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
   (commands verified) into a dated copy of `TEMPLATE.md`. Only the *write* half is measured (median
   222 ms, hence **PARTIAL** in that result's title). Slice 63 owes BOTH halves for registration (≤ 5 s
   to find, ≤ 20 s to register); its write-cost half is
   [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) — nothing is wired, and db/044's
   `gesture_kind` CHECK refuses a registration row until widened.
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart deliberately carries a cross-patient line and an invisible group so the ADR-0060 warnings are
   exercised. Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** ([#444](https://github.com/cairn-ehr/cairn-ehr/issues/444),
   admin-only) — "clippy + cargo test (cairn-gui)" and "cargo doc (API surface)". Both run on every PR;
   neither is in main's branch protection, so both can go red without blocking a merge. **Match the job
   names exactly** — a mismatch orphans the required check and blocks every PR silently. `CONTRIBUTING.md`
   carries the current state in a dated "jobs that run but do not yet block" table.

**If a measurement falls outside its budget, that is the finding — file an issue; never adjust the budget.**

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

**Standing gate:** whole-project review cycles repeat periodically; there will be **no release for
clinical use before repeated cycles pass cleanly.** Last full pass 2026-07-15
([report](code_reviews/2026-07-15-whole-project-architecture-review.md), #187–#217), **fully closed**. A
runnable clinical surface exists that has never been through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
> session holds the main repo. **Never start it alongside one**: they contend on one cargo lock and one
> `test_serial_guard` advisory lock (a stray loop once stretched a session's suites ~3 → ~90 min). **A
> live IDE contends the same way**: rust-analyzer's `cargo check --workspace --all-targets` holds the
> shared `target/` lock, so a narrow `cargo test` blocks before it compiles, then times out. Fix is a
> scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the IDE — and keeping one warm is also what makes
> the full gate ~15 min. (The old "recreate cairn_test/2/3 after an `event_log` column add" note is
> OBSOLETE — #296 closed it structurally.)

---

**Session date:** 2026-08-23 (the db-error sweep's tail: **#481**, the DB-skip guard that could not bind the crate it guarded · **#479**, the run loop's own `db error` · **#477**, the §5.7 auto-apply ceremony) · **Spec/ADRs:** v0.66 (through **ADR-0064**; no new ADR) · **`SCHEMA_GENERATION`:** 50 (`db/050`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — one line each; ROADMAP + the ADR log + git carry the detail:

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address) · **the §5.2 advisory Python matcher** (in-DB veto floor, scoring core,
  veto-gated pipeline/blocking, B3 eval harness, compound keys, generator, weight-learning).
- **The §5.7 identity core C1–C5** (link · apply seam · auto-apply band · dispute · identify · repudiate
  + the alias pool; C5+ `reattribute` waits on a clinical-note surface) · **the §5.4 John-Doe subsystem**
  (slices A–D, photo/text evidence, the `enroll-human` ceremony; §5.12 push-alert open) · **the §5.3/§5.8
  search-before-create funnel** (ADR-0061 — the registration act, db/045 floor + projection, the advisory
  db/046 search, `cairn-patient-search`, and the **precedence rule** #345 at db/005 step 8b).
- **`clinical.medication` slices 1–6b** — assert/cease · bitemporal dose timeline · cross-thread
  reconciliation (ADR-0047) · attestation overlay (ADR-0049) · per-field dose correction (ADR-0050) ·
  inline `substance.coding` (ADR-0059); with the twin-check registry (ADR-0048) and the contributor-role
  floor (ADR-0051). **Born-sealed bodies** (ADR-0052) · **per-write human authorship** (ADR-0053 —
  grading half-live until #245) · **the §5.9 stream COMPLETE through its read surface** (see ⇒ NEXT),
  which **enforces nothing beyond display/emission**.
- **The L3 reference UI** — `cairn-gui/`, a standalone workspace, one-way GUI → crates. The iced shell
  FAILED the accessibility bar (spike 0004, retired 08-03); today it is **`cairn-gui-tauri`**, a Tauri 2
  window on one patient's medication chart (plain JS, no npm). The pane/routing/freshness state machine
  survived the retirement, is tested, and is **not wired**.
- **The med-list node tier** (first clinical READ path + whole-list sign-off) · **generic reprojection**
  (ADR-0057) · the **ADR-0056 admit-uninterpreted floor** + the **residual refusal contract** (a
  deliberate refusal is penned verbatim, auto-released on repair; a frozen watermark fails loud).

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-08-23 (last) — the sweep's tail, and the guard that could not bind the crate it guarded

**Closes [#481](https://github.com/cairn-ehr/cairn-ehr/issues/481),
[#479](https://github.com/cairn-ehr/cairn-ehr/issues/479),
[#477](https://github.com/cairn-ehr/cairn-ehr/issues/477); opens
[#485](https://github.com/cairn-ehr/cairn-ehr/issues/485). `crates/cairn-node` +
`crates/cairn-sync` — no migration file, no ADR, SCHEMA stays 50.** Every item
mutation-checked.

1. **⇒ A GUARD ONLY RUNS WHEN ITS OWN CRATE IS TESTED.** #450's fail-closed DB-skip guard was
   correct and lived in a `cairn-node` integration test, so `cargo test -p cairn-sync` printed
   `101 passed` with no database — over the crate holding the ONLY test of a real mid-loop
   requeue interruption (#471) and the whole of #475's acceptance criterion. The guard moved to
   `tests/common/db_gate.rs` and both crates pull it in with `#[path]`: **one implementation,
   two binaries, deliberately not two copies** (#452's lesson). CI was never exposed —
   `run-db-gated-tests.sh` runs a *workspace* test — but a per-crate job would have been.
2. **⇒ `file!()` IS THE PATH THE INCLUDING FILE WROTE, NOT A CANONICAL ONE.** So the shared
   module has two spellings, the self-exclusion stopped firing, and the walk started feeding
   the file's OWN fixture names (`CAIRN_TEST_PG8`, …) into the requirement list. Caught by name
   the first time it ran, because the existing assertion checks the exclusion **fired exactly
   once** rather than assuming it did — an assertion that had never had a worked example until
   then. New pure `lexically_normalized` folds `..` without touching the filesystem
   (`canonicalize` needs a path that exists relative to the process cwd, which under
   `cargo test -p X` is the crate dir, not the workspace root).
3. **The durable half is `every_crate_with_db_gated_tests_runs_this_guard`** — the obligation is
   DERIVED from which crates read a gate variable, so a third crate growing DB-gated tests
   cannot reopen the hole silently.
4. **⇒ THE FIX THAT LANDED THE DAY BEFORE PRINTED THE WHOLE DIAGNOSIS TWICE.**
   `db_diagnosis::operator_chain` walked past the `tokio_postgres::Error` it had just rendered
   into that error's own `DbError`, so a server error read
   `… refused [P0001] — detail — HINT: hint: ERROR: … refused DETAIL: detail HINT: hint`. Its
   suffix rule cannot catch this: `compose_db_diagnosis` and `DbError::Display` format the same
   three fields differently, so neither ends with the other. **It survived because every
   fixture built its error from an unparseable connection string** — `Kind::ConfigParse`, whose
   rendering ends with its cause's text, so the suffix rule fired and the dedupe LOOKED
   correct. The arm every in-DB refusal actually takes had no coverage at all, and could not:
   a `DbError` cannot be constructed by hand. Fix is one `break` — `legible_db_error` has
   already consumed that error's whole source subtree — plus a DB-gated test in **both** crates.
5. **#479 — the run loop's own species, on the surface `bet_a.py` reads.** `cycle 118: PULL
   FAILED: db error`, every cycle for the life of the process, because `cmd_run` builds its
   client ONCE outside the loop. New pure `operator_chain` (the `Box<dyn Error>` twin of
   `cairn-node`'s) now feeds both the terminal line and the JSONL `pull_error` key. Eight sites
   in all: `do_pull`'s two pre-network statements, `do_requeue`'s opening query (PR #478
   converted the three INSIDE the loop and left the one that opens the function), the byte
   tier's chunk insert, the serve trust-set lookup — the AUTHORIZATION path for an inbound
   peer, answered with eight characters — and `do_fingerprint`'s call site, which had **no
   `else` arm at all**: a schema skew dropped the `fingerprint` key from every later JSONL line
   with zero evidence why.
6. **⇒ THE LANDMINE THE ISSUE WARNED ABOUT, DISARMED RATHER THAN AVOIDED.** Naming the failing
   operation means wrapping, and `downcast_ref` on a `dyn Error` inspects the OUTERMOST type
   only — so a wrapper would have pushed the `postgres::Error` out of
   `classify_pull_failure`'s reach and logged this node's dead database as link downtime,
   charging the Bet A availability figure. #469's defect, reinstated by its own fix.
   `chain_reaches_a_postgres_error` walks instead, **last**, after the two arms that match on
   the outermost type deliberately. **The HANDOVER's old claim that cairn-sync's classifier
   "cannot" walk the chain is retired.**
7. **#477 — the §5.7 auto-apply ceremony, converted as a SUBSYSTEM.** `auto_apply.rs` alone
   would have left `resolve_failure_line` — the line that fires when an epoch's actor cannot be
   resolved at all — rendering `matcher_actor.rs`'s three unwrapped registry reads. Both files
   are now in `GUARDED` (five files), each with its own count pin. **The two count pins
   deliberately count different shapes:** `sync.rs` counts bare `LocalDbFault::new(` because one
   of its twelve sites spans several lines; `auto_apply.rs` counts the `.map_err(|e| …` shape,
   because its TEST module builds one too and the bare form would report a real drop as healthy.
8. **Where an interpolation scan cannot reach, pin the SITES.** `cairn-sync/src/main.rs` stays
   out of `GUARDED` for the reason #479 itself gives (10.1k lines, most `{e}` sites hold errors
   that are not database errors). Its eight fixed sites are pinned by exact shape plus a count,
   in `cairn-node`'s guard file — which reads by repo path, so a fourth copy of the machinery
   was not needed. **Honest about being narrower than a scan:** it protects the sites that were
   fixed and NOT the next one somebody writes.
9. **Two prose corrections the work forced.** `ApplyError` was called "legible by construction"
   in the guard's own header — it is not (#480). And the guard's scope sentence gains its
   measured residual rather than an estimate: 23 further files under `crates/cairn-node/src/`
   execute SQL and are outside `GUARDED` (**#485**); those sites are ugly-but-not-silent — a
   bare `?` preserves `source()`, so anyhow's chain printing still reaches the `DbError` — but
   they name no operation.

**Still open from the sweep** (all four raised by PR #478's review, none fixed here):
[**#480**](https://github.com/cairn-ehr/cairn-ehr/issues/480) (`ApplyError` conflates a door
refusal with a transient local fault, so `requeue` can annotate a pen row with a claim the door
never adjudicated) · [**#482**](https://github.com/cairn-ehr/cairn-ehr/issues/482) (an mTLS pin
mismatch is logged `PARTITION`, so a revoked peer key reads as link downtime) ·
[**#483**](https://github.com/cairn-ehr/cairn-ehr/issues/483) (`connection_label` will not
compile on Windows; no exposure, all CI is `ubuntu-24.04`) ·
[**#484**](https://github.com/cairn-ehr/cairn-ehr/issues/484) (`do_requeue` reports through an
untyped `serde_json::Value`). Also **#476** (~124 test-guard comments calling a per-database
advisory lock "cluster-wide").

### 2026-08-22 — the db-error sweep, in four passes (condensed)

**Closed #460, #465, #467, #469, #471, #473, #474, #475; `db/050`, SCHEMA 49 → 50; no new ADR in
any of them.** ROADMAP carries each pass in full. What still binds a next session:

1. **⇒ `tokio_postgres::Error`'s `Display` IS THE STRING `"db error"`** — a bare match on kind
   that never chains to the source holding the message, the DETAIL and the SQLSTATE. And
   **`anyhow!("…: {e}")` discards the source too**, so the wrapper meant to add context
   subtracts the diagnosis. `db_diagnosis` renders `message [SQLSTATE] — DETAIL — HINT`,
   byte-identical to `cairn-sync`'s `legible_db_error`, so an operator learns one format.
2. **⇒ `LocalDbFault` IS NOT A RENDERING AND MUST NOT BE "TIDIED" INTO AN `anyhow!`.** `Display`
   is the legible text, `source()` is the original error a classifier walks. `anyhow!` takes a
   formatted `String`, so the `tokio_postgres::Error` is consumed by the `format!` and is never
   anyone's `source()` — **silently reverting every local fault to `partition`.** The trap in
   that sweep most likely to be sprung in good faith.
3. **⇒ A FROZEN CURSOR LOOKED EXACTLY LIKE A HEALTHY CYCLE.** All three of `pull_into`'s freeze
   paths `break` and return `Ok` (correct — freezing is the deliberate availability choice), so
   a `53100` disk-full emitted **neither** `LOCAL FAULT` nor `PARTITION` and a monitor keyed on
   those two tokens would watch a stuck node forever. `PullStats.frozen` + `frozen_cursor_line`.
4. **⇒ #370's FIX CONTRADICTED AN ADR WRITTEN EIGHT DAYS EARLIER.** ADR-0063 decides that exact
   shape for `safety` — *mint-strict, arrive-permissive*. **THE CATEGORY TEST:** a sensitivity
   assertion **IS** an event (refusing a malformed one drops that assertion and nothing else);
   `safety`, `clock_grade` and a rendition reference are **FIELDS ON** a clinical event, and
   refusing those forks the event set between honest peers — the **#342** trap, hit five times.
   The rule has three implementations and no name, which is why it keeps breaking. The one thing
   not to "align": db/027 raises where db/050 records, and **`WHEN OTHERS` there would be a
   disaster** — a disk error written into the ledger as "the peer sent garbage".
5. **⇒ A FLAG CAN BE BORN ON A RE-APPLY.** db/020 calls the lenient learner **unconditionally**,
   so a node upgrading onto db/050 flags its whole pre-#460 back-catalogue. The report is keyed
   on the admitted addresses **and** a `flag_id` watermark; drop either clause and five tests go
   red. **A failed read reports `null`, never `0`** — zero is a claim, and after a failed read it
   would mute a monitor exactly when this node stopped being able to see.
6. **⇒ PEER TEXT IS NOT DISPLAY TEXT.** `custody_withheld` is **unbounded prose from an
   unadmitted peer**, printed raw — enough to forge a `0 attachment reference(s)` all-clear on
   the alarm #465 had just installed.
7. **⇒ A GUARD THAT PUNISHES THE PRECISE DESCRIPTION OF ITS OWN BUG** pushes every future writer
   toward vaguer prose. Widening `GUARDED` failed on exactly three lines, all comments quoting
   the shape that caused the defect. Whole-line comments are skipped; a trailing comment after
   code is still scanned. **And a rename is not proof** — two renamed bindings genuinely held
   database errors, and the widened guard reported green over both.
8. **Two test mechanics worth reusing.** A `FOR UPDATE` row lock from a second connection under
   a short `lock_timeout` forces a write failure in a SHARED test database (a trigger or a
   `REVOKE` persists if the test panics and poisons every later suite; a row lock dies with its
   connection) — and note *which* statement it catches: MVCC readers do not block. And
   **`Debug` must delegate to `Display`** on any error that can reach `main`: `fn main() -> R<()>`
   has no error printer, so `Termination` prints `{err:?}`.

**Still open from those passes:** #463 (attachment-flag resolution — a DECISION, overlay vs
delete) · #464 (unbounded per-rendition subtransactions) · #458 (non-object attachment element —
a loud UI, NOT a floor rule) · #468 (the unlearnable-reference alert fires ONCE EVER while its
stated precedent re-fires every cycle) · #470 (the per-cycle ledger read is owner-privileged).

### 2026-08-21 — the freeze that hid, the flake that lied, six trap-clearing fixes, three silent gates

**Closes #370, #457, #449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458.**

- **⇒ PROBE THE FAMILY BEFORE FIXING THE MEMBER.** #370 named one field; measured, that one function
  had **nine** freeze paths across four SQLSTATE classes **and four SILENT paths that wrote something
  wrong**. The rule the fix follows: **refuse what already FAILED plus what was silently WRONG;
  accept everything that already worked** — every refusal added at a remote door is a new way for a
  peer's clinical event to be penned.
- **⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`**
  (#450; #451 the matcher, #481 the per-crate runs). An **opt-out** must read an unrecognised value
  as *NOT permission*, or `CAIRN_ALLOW_DB_SKIP=please` quietly restores fail-open.
- **#457 — the harness polled a PORT and never the CHILD**, so three unrelated causes produced one
  message blaming startup latency. Stderr goes **to a file, never a pipe** (an unread pipe blocks the
  child). **Cause named, not fixed:** a macOS `_dyld_start` loader stall.
- **Check a claim against the pinned source before writing it down** — a load-bearing comment saying
  `std`'s `TcpListener` does not set `SO_REUSEADDR` was false and had steered two rounds of fixes.
- **Three mechanics.** (a) PostgreSQL checks a function called inside a VIEW against the **INVOKING**
  user — the INNER call too. (b) **Ask the authority:** `cargo locate-project` found `packaging/crates`
  was in no workspace. (c) `git check-ignore` needs `--no-index` and has **THREE** exit codes
  (0/1/**128**). **A mutation that does not change the property tests nothing.** Residual: **#447**,
  **#327**.
### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **⇒ A guard defined over the list it guards is not a guard.** `assert_eq!(SubjectKind::ALL.len(), 3)`
   over an `[SubjectKind; 3]` compared a compile-time constant to its own literal and could not fail.
   **Ask what independent source a guard checks against; if the answer is "itself", it is documentation
   wearing a test's clothes.** Constructively: **where a family HAS an authoritative list, read the
   list** — and when reading a catalogue, **`proacl`'s NULL ACL is the PERMISSIVE case**.
2. **⇒ AN OPTIMISATION REMOVED A LOAD-BEARING REDUNDANCY, AND ITS COMMENT ASSERTED THE OPPOSITE.**
   #385's draft said widening §10b's thread-free list could only over-protect; measured, it is the
   reverse — §11's bound is gated on the NEGATION of the same predicate, so a type added to the list
   is EXCLUDED from the bound *and* stops resolving, and a standing `sequestered` grade reads back
   `('routine','none')`. **Before #385 the identical edit was harmless.** Carry: when an optimisation
   makes two paths share a predicate, ask what redundancy that destroyed; and **a wrong safety
   argument is worse than none.**
3. **NAME, NEVER COUNT** — a count cannot separate **custody-blind** from **genuinely empty**, the one
   question `patient-sensitivity <chart>` exists to answer. Related: **a union view whose arms mean
   opposite things must never get one summary sentence**; **the report declares what it cannot
   contain**, asserted over an **empty** list; and **peer text is not display text**.
4. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** —
   ADR-0064's KNOWN GAP. `cairn_sensitivity_standing` is patient-scoped on both sides (load-bearing —
   else chart B strips chart A), so a mis-charted withdrawal's target IS absent here and a naive
   membership test reports it **effective**: a precise untruth in the reassuring direction on a
   confidentiality surface. **#436** is the residual, and it is visibility, not a door.
5. **A pinned `search_path` must deny the temp schema the FIRST look.** `SET search_path = public` does
   not exclude it, so with a decoy `event_log` in place both write doors **returned SUCCESS while the
   owner-privileged INSERT landed in the caller's temp table** — live data loss, from a role with no
   write privilege on `event_log` at all. Open: **#430** (~100 unpinned invoker-rights fns), **#431**.
6. **A parameter name is not a security property.** `classify_authorship_confidence(&body.contributors,
   &body.signer_key_id, None)` compiled, read naturally, and graded a forgery `Attested`; both key
   arguments are now a `VerifiedKid` newtype (mint-site allowlist unpinned: **#428**). **`attester_key`
   alone is NOT proof** — db/020's deferred arm stores a peer's token unverified.
7. **Slice 68:** the authority floor **gates effect, never admission**, and only in the withholding
   direction, so no fork (the **#342** trap); and **computing the verdict at read cuts both ways** —
   revoking an actor silently re-raises grades they lawfully declassified (**#409**), while the Rust↔SQL
   mapping diverges on two shapes (**#408**, root cause **#413**). **Flag what cannot self-heal, view
   what can.** PR #410's review: **7 of 11 production mutations survived a green suite.** Mechanic:
   **`EXCEPTION WHEN OTHERS` does not catch a statement timeout** (`OTHERS` excludes `query_canceled`,
   57014). Open: #413–#420, #422; **#415** measures the SIGNER, so **expect noise**.
8. **Slice 67 — the seal boundary is the coarsening boundary:** precise `{class, severity}` travels
   sealed, a grade-chosen **rung** rides the envelope in the clear, so *coarsen-but-survive* after a
   crypto-shred is structural. Emission coarsening binds a peer's raw-SQL client; **read coarsening is
   a rendering choice, not a floor**. `safety_class_map` ships **EMPTY** — drugref's seam. Open:
   **#406**, **#407**, #394–#402.
9. **Slice 66 — withhold the key, never the bytes** (the rule #460 applies one level down): the
   unwrap-cert kid is pinned to `trust_peer` (db/007), because refusing the bytes would fork the event
   set; repair is TWO steps (`pull --full`, then `cairn_reproject()`).
10. **Slices 61–63 — the seam and the surface.** An attestation **NAMES** the displayed candidates, it
    does not count them (§1.2 write-cost half: **#360**); **a displayed row is a GROUP, an attestation
    is a THREAD** (ADR-0047/0049 — nearly every defect lived on that seam); **a unit-tested safety
    control can still be defeated by the surface that calls it** — **test the path the product actually
    calls**; and **a compensating control outside CI is not a control** (**#444**).

> [!IMPORTANT]
> **The loud failure belongs in the UI, not the floor** (maintainer decision 2026-08-22, from #458).
> *If an attachment — or anything like it — is defective or unacceptable for any reason, the **user
> interface** is where it must fail loud, with immediate feedback, and **without blast radius for the
> rest of the clinical event**.* Three consequences for the attachment UI: **validate the rendition
> reference before submit** (the submit door refuses the whole event, correct only as a backstop that
> never fires); **fail at the attachment, not at the save**, while the clinician is still looking at it
> — a photo that will not stick is obvious when you try to stick it, and does not invalidate what is
> already on the page; and **no confirmation dialog** (principle 3). The same decision refused a
> mandatory `descriptor` as a floor rule: **principle 4 forbids a required field satisfiable only by
> fabrication** — a rushed clinician types `x`, and an honest absence becomes a precise untruth.

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
- **Guard before connect** — take `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
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
demographics + matcher · identity/John-Doe/medication · five-priority review → ADR-0051–0058 · ADR-0059
+ medication 6a/6b · the ADR-0056 admit-uninterpreted floor · floor determinism (#75) · loop launch.

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, **never commit or
publish**. Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope near line-for-line.
The role-manifest layer is the seam (ADR-0021); the open half is under "Blocked on external access".

**Status of this file:** disposable scaffolding, **not** a source of truth; canonical docs win.
Regenerate each session, **under 500 lines** (#368) — *why* in the ADRs, *what* in the spec.

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not.
- **`docs/spikes/`** — 0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor
  — C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on Android — G0–G3 ✓); 0004 (iced UI — FAIL on a11y →
  Tauri 2). **`docs/case-studies/0001`**: 16 Australian GP-software failure modes, all absorbed, **0 new
  architecture**. **`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance.
- Code workspace: `/crates` (`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
  `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace).
  `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. **Every honest gap declared at build time is CLOSED** — only optional escrow
  *rungs* (Shamir/QR/TPM) remain; the `localstate` seams are where the clinical tier plugs
  DEKs/drafts/config.
- **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern node-local `bigint`
  surrogates (`db/008` + the leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`) and self-serialize
  via a Postgres advisory lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are
  scoped PER DATABASE** (#467; ~124 test-guard comments still say otherwise, **#476**), which is why
  every caller must take the guard against `CAIRN_TEST_PG` specifically, whatever database its own
  work then uses. Strings and runner under Open threads → Test env.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one
  fresh headless session per issue (`tail -f ~/.cairn-loop/run.log`). Auto-merge **ENABLED**; **works
  unattended** (12 PRs); **stopped** by maintainer decision — see ⇒ NEXT. Live gaps: **#326**,
  **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization policy hooks),
  **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059, 2026-07-28). **Next candidates:** the
  **drugref term→anchor lookup** (⇒ NEXT item 2); fuzzy/automatic reconciliation + a Tier-A drug
  dictionary; structured sig/frequency (lands with prescriptions); correcting a dose event's *effective
  date* on the statement-level `started`. **Cross-cutting debt: #185** (cross-thread correction
  *suppression* — needs a PK/design decision). Spine: `db/031`–`db/033`, `db/041`, `db/042` +
  `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`). **Next (B3 measurement-driven):** a large hand-crafted gold set to re-run
  the learner for authoritative magnitudes; locale comparator packs; the hub-tier duplicate sweep;
  proposal retraction. **Next identity:** C5+ `reattribute` (**waits on a clinical-note surface**); the
  §5.12 push-alert. Deferred: **#168**, **#287**; the rest are in ROADMAP.
- **Test env:** DB-gated + matcher integration tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532
  user=hherb dbname=cairn_test"` (PG18+cairn_pgx), plus `CAIRN_TEST_PG2`/`PG3` (`cairn_test2`/`3`, same
  cluster) for the multi-node convergence suites — without them those **self-skip and cargo counts them
  as passed** (CI sets all three, #199). **Since #450 a run without them FAILS unless it declares
  `CAIRN_ALLOW_DB_SKIP=1`** — Rust and Python both; only `1`/`true`/`yes`/`on` opts out. Matcher
  integration: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest`; the pure suite is
  dependency-free (`uv run pytest`) — uv, never venv/pip. The `db/tests/*.sql` **mirrors run only via
  `scripts/run-db-sql-tests.sh`**, which drops, recreates and marks a throwaway `cairn_sqltest`: since
  #169 each mirror refuses a database lacking the `cairn_scratch_database` marker, because the mirrors are
  destructive. **`scripts/run-db-gated-tests.sh` runs the mirrors *and* the full workspace with all three
  strings baked in — the one command for the DB slice of the local gate** (a warm `CARGO_TARGET_DIR`
  makes it ~15 min, not the 2 h a cold one costs). Local gap:
  [#314](https://github.com/cairn-ehr/cairn-ehr/issues/314) (it does not run the matcher DB-gated pytest
  suite; CI does). **`clinical_pull` used to flake under a full-workspace run.** #457 fixed the
  DIAGNOSTIC, not the cause: a stall now says whether the child died or hung and names a live pid to
  `sample`. **The cause is still unnamed**; serialising (`--test-threads=2`) remains the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have
  absorbed every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack) — a named
  projection surfaced by salience, not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — non-developer page for the generated site (`web/`; draft plans under
  `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  **PASS twice** (clean 2026-07-07 re-run — B1 p95 **3.99 ms @ 2,004,000 events**, 13× under budget; B4
  confirms ADR-0015's BLAKE3 default). **Remaining:** fold the un-caveated B4 number into ADR-0015 to
  drop "provisional" from the blob-digest line, and
  [#272](https://github.com/cairn-ehr/cairn-ehr/issues/272) (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon. Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
- **easyGP GUI-mining continuation** — more consult-screen/module screenshots incoming from the co-author;
  they should answer most of the remaining §4.4 open questions in
  `scratch/ui-sketches/easygp-consult-screen-inventory.md` and open the **results/inbox design session**
  (three-zone vs two-pane is parked there — don't improvise it).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice. The §8.2 availability + windowing/resume work shipped.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction — deferred
  until momentum/funding geography is clearer; formal trademark registration — principle recorded
  (stewardship doc), legal instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — the working
  conventions, the twelve founding principles (the first four being the lens for every design choice),
  and the §9 defect-blast-radius language rule. Not restated here; canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
