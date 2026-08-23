# HANDOVER — Cairn

## ⇒ NEXT

> [!WARNING]
> **⇒ THE DISASTER-RECOVERY HOLE OUTRANKS EVERY BUILD CANDIDATE BELOW, AND IT NEEDS A MAINTAINER
> DECISION BEFORE IT CAN BE BUILT.** Confirmed in code 2026-08-23 (fourth pass), not inferred from
> prose. **[ADR-0026](spec/decisions/0026-node-durability-and-disaster-recovery.md) decision 1 makes
> three promises about a restored node's clinical tier — the clinical event log survives,
> node-default data-at-rest keys survive, sealed-episode DEKs survive — and ALL THREE ARE FALSE.**
> Two independent defects, both pinned by `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`
> (4 tests, mutation-checked, asserting the DEFECT so they go red on the commit that fixes them):
>
> - **#500 — the bytes.** `backup.rs:138` exports `SELECT signed_bytes FROM node_event`: the medium
>   is the **federation plane only**. A solo clinic backs up nightly, `verify-backup` passes, health
>   is reported honestly — and restore recovers who it peered with and **zero clinical records**.
> - **#495 — the key.** `restore.rs` mints a fresh seed by design (ADR-0026 decision 4); the X25519
>   unwrap secret is HKDF-derived from it (ADR-0052 decision 4), so every inherited `event_dek` row
>   is unopenable. `LocalState`'s two DEK slots are empty by construction and `read_local_state`'s
>   `_db` parameter is **unused** — yet `main.rs:349` runs the export ceremony on the live backup
>   path and every surface reports success over a bundle carrying nothing.
>
> **Fixing either alone is useless**: one leaves a key with nothing to open, the other bodies with no
> key. **#495 carries the three fix options** (escrow the secret / break the derivation / declare the
> loss) — they are not symmetric, and picking one supersedes an ADR either way.
>
> **The reusable lesson, and the reason this hid for weeks:** *a deferral is only honest while its
> stated precondition holds, and nothing in the repo watches for one expiring.* `localstate.rs:10`
> declared its seam truthfully — *"the federation-node tier has no clinical surface yet"* — and
> ADR-0052 made that false without reopening it, while ROADMAP kept recording slices A–D as ✓ done.
> **Before trusting any ✓, check whether the sentence that justified it is still true.**

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems. A, B,
the authority floor and the operator surface are BUILT; C+D are now DESIGNED (ADR-0065) and ⇒ C1 is the
next BUILD.** Read [ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md),
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) and
[ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md) before touching the rest; **do not
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
- **⇒ Parts C+D — custody narrowing and break-glass, DECIDED 2026-08-23**
  ([ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md), spec v0.67; #376 answered,
  #377 merged; #231's close lifted the hard block). A **custody ladder** — admission (default) → named
  **nodes** → named **actors** — under one invariant: **narrowing changes the cost and the noise of
  reading, never whether content can be REACHED — at a node that holds the key or can reach one**,
  because audited break-glass sits at every rung. **The bound is load-bearing**: rung-1 glass is a
  NETWORK act, so a partitioned non-holder cannot reach it — **#498**, the one paper-parity row where
  this ladder loses to the paper envelope. **Node custody is the NORM, per-clinician the EXCEPTION**,
  which is what keeps break-glass rare enough to mean anything. **Read the ADR; five things not to
  re-derive:** the node's own DEK is the keyring and the floor is the glass (LOCAL — a remote keyring
  fails at 3am under partition) · **C and D are NOT separable and #377's dependency is REVERSED** ·
  custody is an **additive field on the sensitivity assertion**, which **forces composition to be
  INTERSECTION** — and intersection can EMPTY (**#499**) · it narrows on `event`/`patient` **never
  `thread`** · an unparseable custody **holds NOBODY while the grade still STANDS** (the local/remote
  split turns on **retryability, not defectiveness**).
- **⇒ C1 is the buildable slice:** rung 1 (`custody.nodes`, both doors, serve-door withholding), the
  audited break-glass path, and the **in-chart location signal** (it needs no channel, and it is the
  only one of the three notification directions that actually restrains). **Rung 2 is #496** — blocked
  on a *reader* identity that does not exist (§5.11; today's surfaces attribute writes only). Patient
  and custodian notification is part D. **Scope moved OUT of C1 by the PR review: the chart-wide
  (`patient`) subject is blocked on #499** — until the empty-intersection collapse is decided, a
  chart-wide narrowing can make every read on that chart a break-glass read.

**Two §5.9 leaks were closed 2026-08-16** (#412, #405), leaving two facts. **`REVOKE SELECT (column)` is
inert while a table-level grant stands**, so `cairn_agent` holds an explicit 23-column grant on
`event_log` omitting `safety`, and **adding an `event_log` column now requires granting it in db/049
section 8** (fail-closed; `safety_read_grants.rs` names the missing one). And the correction that matters
most: **that grant is cost-raising, not a floor** (the column copies a *clear* field of the signed body,
and the runtime role keeps the table grant — **#425**, **#427**). **Never cite db/049 section 8 as a
confidentiality boundary**; ADR-0063 decision 2 binds. Whether a node should attempt one below the
envelope AT ALL is **#432**.

Slice 65's follow-ons still open: **#374** (thread resolution resolves only a thread's *current head*),
**#378** (the withdrawal rationale is clear text forever and replicates — the UI must warn at entry
today), **#379** (the grade in the twin) and **#436** (the mis-chart withdrawal arriving by replication).
**#374 and #379 each need a DECISION, not a patch.** The `arrayref` supply-chain incident (#445) is
**closed** (a typosquat reached `cairn-event` via `blake3 → bao`, fixed upstream same-day, no code
change); one residue, **#454** (`bao` is stale — evaluate `bao-tree`).

> [!IMPORTANT]
> **Two code traps that outlive their slices, repeated here because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`.**
>    The catch-all arm reports `'coarsened'`, and `none` is a legal open-vocabulary value that collided
>    with the sentinel (ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds
>    *reject power* (safe); in the sensitivity and safety ladders it would withhold *protection* or mute a
>    warning. Aligning them is the cleanup most likely to be attempted in good faith, and it reopens a
>    leak. Each `ELSE` carries a shouting comment, and each is pinned by a test. **ADR-0065 adds a THIRD
>    member that agrees for a DIFFERENT reason** (it withholds *quiet access*, and break-glass keeps the
>    content reachable) — so do not carry its justification into a site where reachability is not
>    guaranteed, or fail-closed stops being affordable and starts destroying access.

**Three things still owed are HUMAN acts an agent cannot do:**

1. **The §1.2 time budget is a seeded figure, not a measured one.** Follow
   [`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md)
   into a dated copy of `TEMPLATE.md`. Only the *write* half is measured (median 222 ms, hence
   **PARTIAL**). Slice 63 owes BOTH halves for registration (≤ 5 s to find, ≤ 20 s to register);
   its write-cost half is **#360** — nothing is wired, and db/044's `gesture_kind` CHECK refuses a
   registration row until widened.
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`. The fixture
   chart carries a cross-patient line and an invisible group, so the ADR-0060 warnings are exercised.
   Automating the DOM assertions is **#332** (needs a JS-toolchain decision: plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** (**#444**, admin-only) — "clippy + cargo test
   (cairn-gui)" and "cargo doc (API surface)". Both run on every PR; neither is in main's branch
   protection. **Match the job names exactly** — a mismatch orphans the check and blocks every PR
   silently. `CONTRIBUTING.md` carries the dated "jobs that run but do not yet block" table.

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

**Session date:** 2026-08-23 (four passes — the sweep's tail: **#481**/**#479**/**#477**; the misclassification cluster: **#489**/**#482**/**#480**/**#490** items 1–2; the §5.9 part C+D design pass; then the **DR-guarantee audit** — #495 confirmed in code and #500 split out, pinned by a new guard suite) · **Spec/ADRs:** v0.67 (through **ADR-0065** — *narrow the custody, never the reach*) · **`SCHEMA_GENERATION`:** 50 (`db/050`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail:

- **Demographics slices 1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex /
  gender-identity · §4.3 address) · **the §5.2 advisory Python matcher** (in-DB veto floor, scoring core,
  veto-gated pipeline/blocking, B3 eval harness, compound keys, generator, weight-learning).
- **The §5.7 identity core C1–C5** (C5+ `reattribute` waits on a clinical-note surface) · **the §5.4
  John-Doe subsystem** (slices A–D, photo/text evidence, `enroll-human`; §5.12 push-alert open) · **the
  §5.3/§5.8 search-before-create funnel** (ADR-0061 — the registration act, db/045 floor + projection,
  the advisory db/046 search, `cairn-patient-search`, **precedence rule** #345 at db/005 step 8b).
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

### 2026-08-23 (last, fourth pass) — the DR-guarantee audit: three promises, none of them true

**Confirmed #495 in code and split #500 out of it; added
`crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` (4 tests, all four mutation-checked) and
corrected the expired comments at their source (`localstate.rs` header + `read_local_state`,
`backup.rs::read_event_set`, and the stale justification on `tests/localstate.rs`'s emptiness
assertion). No behaviour change, no migration, no ADR, SCHEMA stays 50.** Finding and fix options:
⇒ NEXT. What generalises past it:

1. **⇒ A DEFERRAL IS ONLY HONEST WHILE ITS STATED PRECONDITION HOLDS, AND NOTHING WATCHES FOR ONE
   EXPIRING.** `localstate.rs:10` declared its empty seam truthfully — *"the federation-node tier has
   no clinical surface yet"*. ADR-0052 made that sentence false and nothing reopened the seam. The
   first defect here whose cause is a **true comment going stale**, and it is whole-record loss.
   **Every ✓ in ROADMAP rests on a sentence; the sentence is what to re-check.**
2. **⇒ THE CEREMONY SUCCEEDING IS THE WORST SHAPE OF THIS BUG.** `main.rs:349` runs the local-state
   export on the live backup path, seals an empty bundle, writes the `.lsk` sidecar and reports
   success; `verify-backup` passes; `backup-status.json` records a true count of what the medium
   actually holds. **Every surface is honest and the composite is a precise untruth** — principle 4
   violated by a system in which no single component lies.
3. **⇒ TWO DEFECTS THAT LOOK LIKE ONE MUST BE SPLIT WHEN FIXING EITHER ALONE IS USELESS.** #500 is
   the bytes, #495 the key: one fix leaves a working key with nothing to open, the other sealed
   bodies with no key. Filed apart so neither can be closed on the strength of the other.
4. **⇒ WHERE A GUARANTEE IS ALREADY FALSE, THE TDD MOVE IS TO PIN THE DEFECT, NOT THE PROMISE.** No
   `#[ignore]` exists in this crate and a permanently-red test blocks the gate for every unrelated
   change, so the suite asserts what is true **today**, each assertion naming what it must be
   INVERTED to. Anti-vacuity is explicit: the node is provisioned so the medium is genuinely
   non-empty, the DEK is written by the **production door** (not the test), and the pure test asserts
   the happy-path unwrap *first* so the refusal cannot pass for the wrong reason.
5. **A design-level coupling worth remembering:** deriving the unwrap secret from the signing seed
   bought "no new key-management mechanism" (ADR-0052 decision 4) and paid for it with a
   contradiction against ADR-0026 decision 4 that **neither ADR could see from inside itself**.
   Cross-ADR claims about *the same key material* need checking where they meet, which is code.

### 2026-08-23 (third pass) — §5.9 part C decided: narrow the custody, never the reach (condensed)

**Design session, no code. Produced [ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md)
(spec v0.66 → v0.67), a design doc, a §5.9 revision; opened #494–#496; answered #376, merged #377.**
The ladder and its five non-re-derivable decisions are in ⇒ NEXT. What generalises past the ADR:

1. **⇒ A CONTROL A FAITHFUL PEER DEFEATS *BY COMPUTING CORRECTLY* IS NOT WEAK — IT IS INCOHERENT.**
   Why the grade-derived custody dial was rejected. The handoff argument was off-target and the ADR
   corrects it: the real quiet leaks are **registry divergence** (A revoked actor Z, B has not → the
   same withdrawal is inert on A and authorised on B, so **B serves the DEK**; both HONEST) and
   replication lag — *not* thread resolution, where decision 9 is working.
2. **⇒ "CONSERVATIVE" IS A PROPERTY OF A DIRECTION, NOT OF A VALUE.** ADR-0062 decision 9's bound is
   right for *disclosure* and wrong for *custody*; inheriting it would make break-glass routine on the
   nodes that see the patient least. Second ADR to hit this asymmetry. **Before reusing a bound, ask
   what it now drives.**
3. **⇒ *REFUSE AT A DOOR ONLY WHAT THAT DOOR CAN DROP WHOLE*** — the three-implementations-no-name rule
   finally named. **The question is never how defective the bytes are; it is what else dies with them.**
4. **⇒ FAIL-CLOSED WAS AFFORDABLE ONLY BECAUSE SOMETHING ELSE GUARANTEED REACHABILITY.** Unparseable
   custody holds nobody *because* break-glass exists; lift that anywhere reachability is not guaranteed
   and it destroys access. **And the guarantee is ALREADY bounded, today: it fails for a partitioned
   rung-1 non-holder (#498).** Related: unknown ranks MAX here as in db/048/049 but for a DIFFERENT
   reason (it withholds *quiet access*, not protection) — do not carry the wrong justification onward.
5. **⇒ CRYPTOGRAPHY THAT BUYS NOISE RATHER THAN PROTECTION IS NOT WORTH A SILENT LOSS MODE.** Per-actor
   DEK wrapping is available but against node-level DB access buys noise — that access can break glass
   anyway — while creating permanent unreadability with **no escrow** and **no `erasure_shred_log` row
   to say so**. *An EHR may lose a record deliberately, audibly and by ceremony; never by a forgotten
   passphrase.*
6. **Two ADR divergences found by checking rather than assuming.** **#494** — ADR-0052 decision 4
   describes `event_dek` as `(event_id, holder, dek_wrapped)`; the built table has **no `holder` column**
   (erratum, not a migration). **#495** — the unwrap secret derives from a signing seed ADR-0026 says is
   never backed up. **READ 2026-08-23 (fourth pass): NOT covered — both halves confirmed defective and
   #500 split out. See the ⇒ NEXT warning.**
7. **⇒ THE PR REVIEW OF A DESIGN-ONLY PR IS A CLAIMS AUDIT, AND IT FOUND FOUR THINGS.** Every citation
   verified line-exact, but two claims about the ADR's *own* reasoning did not survive. **#498** — the
   invariant was stated UNBOUNDED while decision 2 makes rung-1 glass a network act; the paper-parity
   table claimed `M = N` at steps where the partitioned row is *impossible*. **#499** — the custody
   composition rule was never stated; intersection is FORCED, and it collapses to ∅ on two honest
   chart-wide narrowings. Two prose defects fixed in place (break-glass sits BESIDE the ladder, not on
   it; the door-treatment separator is **retryability, not defectiveness**). ⇒ **The ROADMAP
   condensation had also deleted the "Open-issue index", orphaning 22 live numbers in one edit.
   Restored. A line cap is never a reason to drop a live issue.**

### 2026-08-23 (first + second pass) — the misclassification cluster and the sweep's tail (condensed)

**Closed #489, #482, #480, #490 items 1–2, #481, #479, #477; opened #485, #487–#492.
`crates/cairn-sync` + `crates/cairn-node`; no migration, no ADR, SCHEMA stays 50.** ROADMAP carries both
passes in full. The traps that still bind:

1. **⇒ A CLASS IS AN OPERATOR INSTRUCTION, AND A DEFAULT-BY-ELIMINATION IS NOT ONE.** Both pull
   classifiers used `partition` (*go and look at the link*) as catch-all **and** as a diagnosis.
2. **⇒ THE RECOGNISER IS A TYPE OR AN `io::ErrorKind`, NEVER THE MESSAGE TEXT — AND A TYPE OUTRANKS A
   KIND.** `LocalFault` is checked FIRST so a broad `ErrorKind` net cannot re-label what a concrete type
   already claimed. (`tokio-rustls` maps a handshake `rustls::Error` to `InvalidData`; a failure
   *constructing* the connection is `ErrorKind::Other`.) Accepted blur: a badly lossy link reads as a
   peer problem — the safe direction.
3. **⇒ FLATTENING A CAUSE IS WORSE THAN MIS-CLASSIFYING IT.** `format!`/`anyhow!("…: {e}")` consume the
   source, so the classifier can never be *taught* to recognise it. **And a reachable cause can print
   TWICE** — `operator_chain` drops a layer only when the layer above ENDS WITH it; pinned by counting.
4. **⇒ THE FIX HAD THE DEFECT IT WAS FIXING, ONE MATCH ARM ABOVE ITSELF** (same PR, review round):
   `also_local_fault` came from the pen write alone, so an apply failing on *this node's* database still
   reached `bet_a.py` as `integrity`. The existing `40001` test was one assertion away from proving it.
5. **⇒ A GUARD FOR AN ORDERING PROPERTY THAT COULD NOT OBSERVE THE ORDERING.** `.context()` values are
   NOT reachable from `chain()`, so swapping the two arms left the test green. Assert both signals are
   present *before* asserting which wins, or the vacuity returns silently.
6. **⇒ WHERE A PIN'S FIXTURE IS BUILT BY THE TEST, THE PRODUCTION SITE IS UNPINNED.** All six sites are
   now driven the real way (deny-all `TrustStore`; a hostile stub peer; row and `ACCESS EXCLUSIVE` locks
   under a short `lock_timeout`). Sibling rule: **a guard only runs when its own crate is tested** —
   #450's DB-skip guard now lives in `tests/common/db_gate.rs`, `#[path]`-included by both crates, with
   the obligation DERIVED from which crates read a gate variable.
7. **⇒ `file!()` IS THE PATH THE INCLUDING FILE WROTE, NOT A CANONICAL ONE.** Two spellings stopped a
   self-exclusion firing; caught only because the assertion checks it fired **exactly once**.
8. **A SQLSTATE class, not a door verdict.** `apply_failure_is_local` (pure, claims explicit, defaults
   `false`) both sets the pull's `local_fault` and decides whether a requeue may halt, and says *"NOT a
   deliberate floor refusal"* in the DATABASE's voice — saying it in the door's voice is #480 in miniature.

**Still open from the sweep:** **#490** item 3 (two stderr-only signals never reach the JSONL) · **#483**
(`connection_label` will not compile on Windows; no exposure — CI is all `ubuntu-24.04`) · **#484**
(`do_requeue` reports through an untyped `serde_json::Value`) · **#487** (cairn-node's `Termination` path
double-renders a `LocalDbFault` chain) · **#488** (auto_apply swallows its advisory unlock, builds
`Skipped(reason)` then discards it) · **#491** (the `break` has no non-DB coverage; the `operator_chain`
twins have no drift test and differ in hop limit) · **#492** (the per-crate ratchet is blind to a crate
reading its gate variable through a shared helper) · **#485** (23 further cairn-node files, 89 postgres
call sites, name no operation) · **#476** (~124 test-guard comments calling a per-database advisory lock
"cluster-wide").

### 2026-08-22 — the db-error sweep, in four passes (condensed)

**Closed #460, #465, #467, #469, #471, #473, #474, #475; `db/050`, SCHEMA 49 → 50; no new ADR.**
ROADMAP carries each pass in full. What still binds:

1. **⇒ `tokio_postgres::Error`'s `Display` IS THE STRING `"db error"`** — a bare kind match never chains
   to the source holding the message, DETAIL and SQLSTATE. **`LocalDbFault` IS NOT A RENDERING and must
   not be "tidied" into an `anyhow!`**: `Display` is the legible text, `source()` is what a classifier
   walks, and `anyhow!` takes a formatted `String`, **silently reverting every local fault to
   `partition`.** The trap most likely to be sprung in good faith.
2. **⇒ A FROZEN CURSOR LOOKED EXACTLY LIKE A HEALTHY CYCLE.** All three of `pull_into`'s freeze paths
   `break` and return `Ok` (correct — freezing is the deliberate availability choice), so a `53100`
   disk-full emitted **neither** `LOCAL FAULT` nor `PARTITION`.
3. **⇒ THE CATEGORY TEST (#370's fix contradicted an ADR written eight days earlier):** a sensitivity
   assertion **IS** an event (refusing a malformed one drops that assertion and nothing else); `safety`,
   `clock_grade` and a rendition reference are **FIELDS ON** a clinical event, and refusing those forks
   the event set between honest peers — the **#342** trap, hit five times. **ADR-0065 NAMES this rule**
   (*refuse at a door only what that door can drop whole*). Do not "align" db/027, which raises where
   db/050 records: **`WHEN OTHERS` there would write a disk error into the ledger as peer garbage.**
4. **⇒ A FLAG CAN BE BORN ON A RE-APPLY.** db/020 calls the lenient learner **unconditionally**, so a
   node upgrading onto db/050 flags its whole pre-#460 back-catalogue. The report is keyed on the
   admitted addresses **and** a `flag_id` watermark; drop either and five tests go red. **A failed read
   reports `null`, never `0`** — zero is a claim that would mute a monitor exactly when this node
   stopped being able to see.
5. **⇒ PEER TEXT IS NOT DISPLAY TEXT.** `custody_withheld` is unbounded prose from an unadmitted peer,
   printed raw — enough to forge a `0 attachment reference(s)` all-clear. Related: **a guard that
   punishes the precise description of its own bug** pushes every future writer toward vaguer prose,
   **and a rename is not proof** — two renamed bindings genuinely held database errors.
6. **Two test mechanics worth reusing.** To force a write failure in a SHARED test database, take a LOCK
   from a second connection under a short `lock_timeout` — never a trigger or a `REVOKE`, which persist
   past a panic and poison every later suite. Match the lock to the statement: `FOR UPDATE` for a write,
   `ACCESS EXCLUSIVE` when the target is a read. And **`Debug` must delegate to `Display`** on any error
   reaching `main` — `fn main() -> R<()>` prints `Termination`'s `{err:?}`.

**Still open from those passes:** #463 (attachment-flag resolution — a DECISION, overlay vs delete) ·
#464 (unbounded per-rendition subtransactions) · #458 (non-object attachment element — a loud UI, NOT a
floor rule) · #468 (the unlearnable-reference alert fires ONCE EVER while its stated precedent re-fires
every cycle) · #470 (the per-cycle ledger read is owner-privileged).
### 2026-08-21 → 08-20 — the freeze that hid, the flake that lied, three silent gates (condensed)

**Closed #370, #457, #449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458.**

- **⇒ PROBE THE FAMILY BEFORE FIXING THE MEMBER.** #370 named one field; measured, that one function had
  **nine** freeze paths across four SQLSTATE classes **and four SILENT paths that wrote something wrong**.
  The rule: **refuse what already FAILED plus what was silently WRONG; accept everything that already
  worked** — every refusal at a remote door is a new way to pen a peer's clinical event.
- **⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`** (#450;
  #451 the matcher, #481 the per-crate runs). An **opt-out** must read an unrecognised value as *NOT
  permission*, or `CAIRN_ALLOW_DB_SKIP=please` quietly restores fail-open.
- **Check a claim against the pinned source before writing it down** — a load-bearing comment about
  `std`'s `TcpListener` and `SO_REUSEADDR` was false and had steered two rounds of fixes. Likewise
  **#457**: the harness polled a PORT and never the CHILD (cause named, not fixed — a macOS
  `_dyld_start` stall). **A mutation that does not change the property tests nothing.**
- **Three mechanics.** (a) PostgreSQL checks a function called inside a VIEW against the **INVOKING**
  user — the INNER call too. (b) **Ask the authority:** `cargo locate-project` found `packaging/crates`
  was in no workspace. (c) `git check-ignore` needs `--no-index` and has **THREE** exit codes
  (0/1/**128**). Residual: **#447**, **#327**.

### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **⇒ A guard defined over the list it guards is not a guard.** `assert_eq!(SubjectKind::ALL.len(), 3)`
   over an `[SubjectKind; 3]` compared a compile-time constant to its own literal. **Ask what
   independent source a guard checks against; if the answer is "itself", it is documentation wearing a
   test's clothes.** Constructively: **where a family HAS an authoritative list, read the list** — and
   when reading a catalogue, **`proacl`'s NULL ACL is the PERMISSIVE case**.
2. **⇒ AN OPTIMISATION REMOVED A LOAD-BEARING REDUNDANCY, AND ITS COMMENT ASSERTED THE OPPOSITE.**
   Widening §10b's thread-free list reads as over-protective; measured, §11's bound is gated on the
   NEGATION of the same predicate, so a type added to the list leaves a standing `sequestered` grade
   reading back `('routine','none')`. **Before #385 the identical edit was harmless.** When an
   optimisation makes two paths share a predicate, ask what redundancy that destroyed — and **a wrong
   safety argument is worse than none.**
3. **NAME, NEVER COUNT** — a count cannot separate **custody-blind** from **genuinely empty**, the one
   question `patient-sensitivity <chart>` exists to answer. Related: **a union view whose arms mean
   opposite things must never get one summary sentence**, and **the report declares what it cannot
   contain**, asserted over an **empty** list.
4. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** —
   ADR-0064's KNOWN GAP. `cairn_sensitivity_standing` is patient-scoped on both sides (load-bearing —
   else chart B strips chart A), so a mis-charted withdrawal's target IS absent and a naive membership
   test reports it **effective**: a precise untruth in the reassuring direction on a confidentiality
   surface. **#436** is the residual, and it is visibility, not a door.
5. **Two floor traps whose issues carry the detail.** **A pinned `search_path` must deny the temp schema
   the FIRST look** — `SET search_path = public` does not exclude it, so with a decoy `event_log` both
   write doors **returned SUCCESS while the owner-privileged INSERT landed in the caller's temp table**
   (live data loss): **#430**, **#431**. And **a parameter name is not a security property** —
   `classify_authorship_confidence(…, &body.signer_key_id, None)` compiled, read naturally, and graded a
   forgery `Attested`; both key arguments are now a `VerifiedKid` newtype (**#428**), and
   **`attester_key` alone is NOT proof**.
6. **Slice 68:** the authority floor **gates effect, never admission**, and only in the withholding
   direction, so no fork (the **#342** trap); **computing the verdict at read cuts both ways** — revoking
   an actor silently re-raises grades they lawfully declassified (**#409**), while the Rust↔SQL mapping
   diverges on two shapes (**#408**, root cause **#413**). **Flag what cannot self-heal, view what can.**
   PR #410's review: **7 of 11 production mutations survived a green suite.** Mechanic: **`EXCEPTION WHEN
   OTHERS` does not catch a statement timeout** (57014). Open: #413–#420, #422; **#415** measures the
   SIGNER, so **expect noise**.
7. **Slices 66–67 — the seal boundary is the coarsening boundary, and *withhold the key, never the
   bytes*.** Precise `{class, severity}` travels sealed; a grade-chosen **rung** rides the envelope in
   the clear, so *coarsen-but-survive* after a crypto-shred is structural. Emission coarsening binds a
   peer's raw-SQL client; **read coarsening is a rendering choice, not a floor**. `safety_class_map`
   ships **EMPTY** — drugref's seam. The unwrap-cert kid is pinned to `trust_peer` (db/007) because
   refusing the bytes would fork the event set; repair is TWO steps (`pull --full`, then
   `cairn_reproject()`). Open: **#406**, **#407**, #394–#402.
8. **Slices 61–63 — the seam and the surface.** An attestation **NAMES** the displayed candidates, it
   does not count them (§1.2 write-cost half: **#360**); **a displayed row is a GROUP, an attestation is
   a THREAD** (ADR-0047/0049 — nearly every defect lived on that seam); **a unit-tested safety control
   can still be defeated by the surface that calls it**; and **a compensating control outside CI is not
   a control** (**#444**).


> [!IMPORTANT]
> **Two maintainer decisions to hold before any composite-clinical-object work.**
>
> **The loud failure belongs in the UI, not the floor** (2026-08-22, from #458): a defective
> attachment fails loud **in the UI** with **no blast radius for the rest of the clinical event** —
> validate before submit (the door refusing the whole event is a backstop that should never fire),
> fail **at the attachment, not at the save**, **no confirmation dialog** (principle 3). The same
> decision refused a mandatory `descriptor` as a floor rule: **principle 4 forbids a required field
> satisfiable only by fabrication** — a rushed clinician types `x`, and an honest absence becomes a
> precise untruth.
>
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
> — *a defect on one line never invalidates another*: the system may fail to record an order, but it
> may never cancel one.** Hold decision 2 (partial completion reported, never implied) and decision 7
> (check the transaction boundaries).

**Five repo conventions these runs learned the hard way:**
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it.** A new
  `cairn_decode_hex_or_raise` call site fails `hex_decode_helper.rs`'s exact per-file call-site list;
  the twin and projection registries and `db_errors_stay_legible.rs`'s three counts carry the same
  shape. The count failing is the guard WORKING — fix the list, and say in a comment why.
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
whether an intermediate layer threw it away** — `apply_signed` flattened `postgres::Error` to `String`,
and the *residue* was still misrouting a transient fault as a door verdict three weeks later (#480).

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, **never commit or
publish**. Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope near line-for-line;
the role-manifest layer is the seam (ADR-0021).

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
  genesis-stable `node_id`. Every honest gap declared at build time was closed **except one, and it
  has since expired**: the `localstate` seams where the clinical tier plugs DEKs/drafts/config are
  **still empty**, and the clinical tier now exists (**#495**/**#500** — the ⇒ NEXT warning).
  Optional escrow *rungs* (Shamir/QR/TPM) also remain. **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 +
  multihash) is the *only* identifier on the wire/in signed bodies; the projection plane may intern
  node-local `bigint` surrogates (`db/008` + the leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`) and self-serialize
  via a Postgres advisory lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are
  scoped PER DATABASE** (#467; ~124 test-guard comments still say otherwise, **#476**), so every caller
  must take the guard against `CAIRN_TEST_PG` specifically, whatever database its own work then uses.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one
  fresh headless session per issue (`tail -f ~/.cairn-loop/run.log`). Auto-merge **ENABLED**; **works
  unattended** (12 PRs); **stopped** by maintainer decision — see ⇒ NEXT. Live gaps: **#326**, **#312**,
  **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization policy hooks),
  **#236** (FTS/RAG must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059). **Next candidates:** the **drugref
  term→anchor lookup** (⇒ NEXT); fuzzy/automatic reconciliation + a Tier-A drug dictionary; structured
  sig/frequency (lands with prescriptions); correcting a dose event's *effective date* on the
  statement-level `started`. **Cross-cutting debt: #185.** Spine: `db/031`–`db/033`, `db/041`, `db/042`
  + `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`). **Next (B3 measurement-driven):** a large hand-crafted gold set to re-run
  the learner for authoritative magnitudes; locale comparator packs; the hub-tier duplicate sweep;
  proposal retraction. **Next identity:** C5+ `reattribute` (**waits on a clinical-note surface**); the
  §5.12 push-alert. Deferred: **#168**, **#287**; the rest are in ROADMAP.
- **Test env:** **`scripts/run-db-gated-tests.sh` is the one command for the DB slice of the local
  gate** — the `db/tests/*.sql` mirrors *and* the full workspace with `CAIRN_TEST_PG`/`PG2`/`PG3` baked
  in (PG18 + cairn_pgx on `127.0.0.1:5532`, databases `cairn_test`/`2`/`3`). A warm `CARGO_TARGET_DIR`
  makes it ~15 min, not 2 h; last full pass **1568 passed / 0 failed** over 139 binaries (2026-08-23).
  Without the three strings the DB-gated suites **self-skip and cargo counts them as passed**, so
  **since #450 a run without them FAILS unless it declares `CAIRN_ALLOW_DB_SKIP=1`** (only
  `1`/`true`/`yes`/`on` opts out). The mirrors are DESTRUCTIVE and refuse any database lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra
  pipeline pytest` (uv, never venv/pip). Local gap: **#314** (the script skips the matcher DB-gated
  pytest suite; CI runs it). **`clinical_pull` used to flake under a full-workspace run** — #457 fixed
  the DIAGNOSTIC, not the cause; **the cause is still unnamed**, `--test-threads=2` is the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have
  absorbed every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack) — a named
  projection surfaced by salience, not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — non-developer page for the generated site (`web/`; draft plans under
  `docs/superpowers/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  **PASS twice** (clean 2026-07-07 re-run — B1 p95 **3.99 ms @ 2,004,000 events**, 13× under budget).
  **Remaining:** fold the un-caveated B4 number into ADR-0015 to drop "provisional" from the blob-digest
  line, and **#272** (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access: the `rx!`/`tx!` type-through parser + state machine; the
  formulation/drug data source + renal/hepatic/pregnancy/paediatric **forced-manual** rule table; the
  prefetch/materialization warming daemon. Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
  **GUI-mining continues** — more consult-screen screenshots incoming; they should answer most of
  `scratch/ui-sketches/easygp-consult-screen-inventory.md`'s open §4.4 questions and open the
  **results/inbox design session** (three-zone vs two-pane is parked there — don't improvise it).
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
