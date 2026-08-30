# HANDOVER — Cairn

## ⇒ NEXT

> [!WARNING]
> **⇒ THE DISASTER-RECOVERY HOLE IS HALF-CLOSED, AND *WHICH HALF* IS THE WHOLE POINT.
> [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) IS CLOSED.
> [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) IS NOT.** *"The DR hole is fixed"* is exactly
> the true-in-part sentence that gets quoted as complete six months later — the failure mode this slice
> exists to correct. **Under-claiming is safe here; over-claiming is the defect.**
>
> - **✓ #495 — THE KEY. CLOSED** by
>   [ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md) (spec v0.68) and the
>   slice built 2026-08-24. The node's X25519 unwrap secret is no longer HKDF-derived from its Ed25519
>   signing seed: it is an **independent keypair** sealed in its own `<key>.unwrap` file, it rides the
>   `CAIRNL1` local-state export beside the surviving `event_dek` rows (a **shredded** event's DEK excluded
>   by construction), and `restore` **adopts** it instead of minting one. **A restored solo node can now
>   inherit its custody KEY.** (Its own custody *records* are a different question — see #500 immediately
>   below.)
> - **✗ #500 — THE BYTES. STILL OPEN, AND IT IS THE NEXT BUILD (slice 2).** `backup.rs::read_event_set`
>   still exports `SELECT signed_bytes FROM node_event`: the medium is the **federation plane only**, so
>   NO `event_log` row travels — not clinical, not demographic, not identity, not registration. A solo
>   clinic backs up nightly, `verify-backup` passes, health is reported honestly, and restore recovers who
>   it peered with and **zero patients**. Pinned by
>   `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event`, which
>   slice 2 inverts.
>
> **Slice 1 therefore hands a restored node a working key and nothing yet to open with it.** Neither half
> is useful alone. **Never cite ADR-0026 decision 1's clinical promises as met.** Its promise 2 —
> *"node-default data-at-rest keys survive"* — has **no subject at all**: no node-default key tier exists,
> so it is neither honoured nor violated and must not be read as satisfied by anything slice 1 did.
> **[#502](https://github.com/cairn-ehr/cairn-ehr/issues/502) — items 1, 2 and 3 are fixed; item 4
> (a discarded keystore-load reason) stays open.** A present-but-unreadable export now **refuses the
> restore** instead of being skipped in silence; a `.lsk` escrow sidecar that is present but corrupt is
> diagnosed as **present-but-unusable** with the *move it aside first* remedy, rather than reported as
> "absent" and sending the operator to a command that then refuses; and `verify-backup` refuses a
> zero-event medium instead of reporting `backup OK: 0/0` over an artifact that restores nothing —
> checked in the CLI arm and NOT in `all_intact()`, because *is this medium internally consistent* and
> *is this medium worth anything* are different questions and a node with no events yet must still be
> able to write its first medium. That command also stopped printing an all-clear it had not
> established: it reports the sealed export sibling beside the medium and **declares that the export's
> contents were not checked**, because it holds no recovery code and cannot know whether the export
> carries a custody key.
>
> **✓ FEDERATED SYNC WORKS AGAIN — [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503) IS CLOSED**
> (2026-08-30). `cairn-sync` briefly could not start against any node provisioned by today's
> `cairn-node init`: it HKDF-derived its unwrap key from the signing seed while `init` generated an
> independent one (ADR-0066), and those disagree by construction. The new `crates/cairn-keystore` carries
> the sealed key-file format both binaries need, and `cairn-sync` now **LOADS** the provisioned key at
> startup (`<key>.unwrap`, or `--unwrap-key`; `CAIRN_KEY_PASSPHRASE` unseals it) — resolved **once**, then
> carried, instead of derived independently at six sites. **It still refuses to start** on a key that
> diverges from the registered one, on a restored node's fresh-seed derivation, and on a **corrupt or
> passphrase-less** key file. **One derived path survives by design** — see trap 3.
>
> **The reusable lesson, and the reason this hid for weeks:** *a deferral is only honest while its stated
> precondition holds, and nothing in the repo watches for one expiring.* The `localstate.rs` module header
> declared its seam truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052
> made that false without reopening it, while ROADMAP kept recording slices A–D as ✓ done.
> **Before trusting any ✓, check whether the sentence that justified it is still true.**

> [!IMPORTANT]
> **Five traps slice 1 minted. Each is a step a next session takes in good faith.**
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY.** It survives so a pre-ADR-0066 node can
>    re-derive its old secret **exactly once**, inside `keystore::adopt_derived_unwrap_secret`, keeping its
>    existing `event_dek` rows openable. **Calling it anywhere else re-creates the #495 coupling.** Pinned
>    by `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, which since the review wave sweeps
>    **every shipping tree, not just the Cargo workspace** (`sources::PRODUCTION_TREES` —
>    `crates/`, `extensions/`, `cairn-gui/`; the two `Cargo.toml` `exclude`s ship too, and
>    `extensions/cairn_pgx` runs *inside Postgres*). Its allow-list also asserts **every entry is still
>    live** — a dead entry fails the guard, so the list cannot quietly widen. **When it fails, do not add
>    a line to `ALLOWED`.** Note the paired requirement: the sweep and the test-gate matcher
>    (`is_a_test_gate_attribute`, which must recognise pgrx's `#[cfg(any(test, feature = "pg_test"))]`)
>    have to move **together** — widening either alone reddens the guard on correct code, and the obvious
>    fix for that red is the `ALLOWED` line that would gut it.
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6).
>    `ensure_unwrap_key` and `submit_event` now **refuse**. **A node whose database is recreated under an
>    existing key file needs `cairn-node establish-unwrap-key` before its first sealed write** — six test
>    suites (one in another crate) depended on the old implicit behaviour. Never make a red fixture green
>    by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync` LOADS its unwrap secret now (#503) — with ONE derived fallback, and the reason it is
>    safe is the reason not to widen it.** With no `<key>.unwrap` file AND a derived key that **equals**
>    the registered one, the daemon starts on the derived secret and **warns on every startup**: that node
>    is provably pre-ADR-0066, and the derived key is provably the one its `event_dek` rows are wrapped to.
>    A **restored** node derives a key that does *not* match and is refused — the #495 shape, caught. The
>    load-bearing asymmetry: an **absent** file may fall back, a **present-but-unusable** one (corrupt, or
>    sealed with no `CAIRN_KEY_PASSPHRASE`) **never** may, because a successful derive would mask the rot
>    of the only file carrying this node's custody off the machine. **Never "simplify" those two into one
>    arm.** The whole table is pure and unit-tested in `crates/cairn-sync/src/unwrap_key.rs`; retiring the
>    fallback is **#514**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It adopts a
>    secret derived from the **new** signing seed and registers it, and `node_unwrap_key`'s singleton
>    registrar then refuses the real exported key **permanently**. It is the operator's obvious next step
>    — `submit_event`'s own refusal text names that command — so `restore` warns about it explicitly.
>    Recover the export first. Since the review wave, if it *has* happened, `apply_local_state`'s refusal
>    now explains the resulting state rather than surfacing a raw Postgres error: the file just written
>    holds the dead node's real key and must be kept, the registration is the wrong one, and the way out
>    is another restore into a fresh database.
>
> 5. **`init` now refuses a database that already has custody registered.** Added in the review wave and
>    worth knowing before it surprises someone: the file check (`refuse_to_replace_existing_unwrap_key`)
>    only fires when `<key>.unwrap` EXISTS, so a node that lost its keystore could still run `init`,
>    overwrite its signing key, mint a doomed custody key and only then fail at the registrar. `init`
>    reads `node_unwrap_key` first now. The remedy it names is `establish-unwrap-key`, which is
>    idempotent — see trap 4 before running it on a restored node.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A
and B, the authority floor and the operator surface are BUILT (and enforce nothing beyond display and
emission); C+D are DESIGNED and C1 is the next §5.9 BUILD — behind #500, which now outranks it.** Read
[ADR-0062](spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md),
[ADR-0063](spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md),
[ADR-0064](spec/decisions/0064-admit-the-claim-withhold-the-power.md) and
[ADR-0065](spec/decisions/0065-narrow-the-custody-never-the-reach.md) before touching any of it; **do not
re-derive their decisions.** The **authority floor** is ONE predicate `cairn_claim_authority` (db/005) at
exactly ONE site (the `NOT EXISTS` in `cairn_sensitivity_standing`, db/048), so display coarsening,
safety-rung emission and part C's dial all inherit it structurally; it gives **#245** its first SQL
counterpart — NOT its "mirror", NOT its display half. The operator surface's §1.2 budget is **MET** and
pinned (residual **#436**).

**Parts C+D (ADR-0065; #377 merged with its dependency REVERSED)** are a **custody ladder** — admission
(default) → named **nodes** → named **actors** — under one invariant: **narrowing changes the cost and the
noise of reading, never whether content can be REACHED — at a node that holds the key or can reach one**,
because audited break-glass sits at every rung. **The bound is load-bearing:** rung-1 glass is a NETWORK
act, so a partitioned non-holder cannot reach it (**#498**). **Node custody is the NORM, per-clinician the
EXCEPTION.** Five things not to re-derive: the node's own DEK is the keyring and the floor is the glass
(LOCAL — a remote keyring fails at 3am under partition) · C and D are NOT separable · custody is an
**additive field** on the sensitivity assertion, which **forces composition to be INTERSECTION** — and
intersection can EMPTY (**#499**) · it narrows on `event`/`patient` **never `thread`** · an unparseable
custody **holds NOBODY while the grade still STANDS**. **C1** is rung 1 (`custody.nodes`, both doors,
serve-door withholding) + the audited break-glass path + the **in-chart location signal**; **rung 2 is
#496** (blocked on a *reader* identity that does not exist, §5.11) and **the chart-wide `patient` subject
is OUT of C1, blocked on #499**.

**Two §5.9 facts that outlive their slices.** **`REVOKE SELECT (column)` is inert while a table-level grant
stands**, so `cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety`, and **adding
an `event_log` column now requires granting it in db/049 section 8** (`safety_read_grants.rs` names it). And **that grant is cost-raising, not a floor** (**#425**, **#427**): **never cite
db/049 section 8 as a confidentiality boundary** — ADR-0063 decision 2 binds; **#432** asks whether a node
should attempt one below the envelope at all. Slice 65's follow-ons still open: **#374** (thread resolution
resolves only a thread's *current head*), **#378** (the withdrawal rationale is clear text forever and
replicates — the UI must warn at entry today), **#379** (the grade in the twin), **#436**; **#374 and #379
each need a DECISION, not a patch.** The `arrayref` incident (#445) is **closed**; residue **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, repeated here because both look like tidy-ups.**
>
> 1. **`content_address IS NOT NULL` is the "did anything win" test — never `subject_kind <> 'none'`.**
>    The catch-all arm reports `'coarsened'`, and `none` is a legal open-vocabulary value that collided
>    with the sentinel (ADR-0062 E6).
> 2. **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0 withholds
>    *reject power* (safe); in the sensitivity and safety ladders it would withhold *protection* or mute a
>    warning. Aligning them is the cleanup most likely to be attempted in good faith, and it reopens a
>    leak. **ADR-0065 adds a THIRD member that agrees for a DIFFERENT reason** (it withholds *quiet
>    access*, and break-glass keeps the content reachable) — do not carry that justification into a site
>    where reachability is not guaranteed.

**Three things still owed are HUMAN acts an agent cannot do:**

1. **The §1.2 time budget is a seeded figure, not a measured one.** Follow
   [`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a
   dated copy of `TEMPLATE.md`. Only the *write* half is measured (median 222 ms, hence **PARTIAL**); Slice
   63 owes BOTH halves for registration (≤ 5 s to find, ≤ 20 s to register), its write-cost half **#360** —
   nothing wired, and db/044's `gesture_kind` CHECK refuses a registration row until widened.
2. **The accessibility pass** — a live VoiceOver run through the runbook's eight checks, keyboard-only:
   `cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001` (the fixture
   chart exercises the ADR-0060 warnings). Automating the DOM assertions is **#332** (plain JS, no npm).
3. **Make two CI jobs REQUIRED status checks** (**#444**, admin-only) — "clippy + cargo test (cairn-gui)"
   and "cargo doc (API surface)". **Match the job names exactly** — a mismatch orphans the check and blocks
   every PR silently; `CONTRIBUTING.md` carries the dated "run but do not yet block" table.

**If a measurement falls outside its budget, that is the finding — file an issue, never adjust the budget.**
**The other build candidates** (any can be picked up after #500; nothing blocks a choice): **the
registration/search UI slice** — the picker is the wrong-chart affordance paper has and the med-list
window does not, and per Slice 63 it must **open** a chart, never *retarget* an open window (retargeting
re-creates the §5.8/§5.11 misfile possession semantics exist to prevent) · **the drugref term→anchor
lookup** — the §9 *advisory* tier, and what closes the **coded↔uncoded** duplicate case ADR-0059 decision
5 leaves open; needs a cross-service connection-model decision first, the slice-6a/6b source guard must
stay passing, and `safety_class_map` is its second, empty seam · **the node/actor plane's two
divergences** — `db/007` fail-closes on an unmappable type (**#301**) and skips-and-advances a verifiable
refusal where the clinical plane now pens (**#268**); **neither is a symmetric fix**, both `loop:blocked`.

**Standing gate:** whole-project review cycles repeat periodically; **no release for clinical use before
repeated cycles pass cleanly.** Last full pass 2026-07-15 (#187–#217), **fully closed**; the runnable
clinical surface has never been through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
> session holds the main repo. **Never start it alongside one**: they contend on one cargo lock and one
> `test_serial_guard` advisory lock (a stray loop once stretched a session's suites ~3 → ~90 min). **A live
> IDE contends the same way** — rust-analyzer holds the shared `target/` lock, so a narrow `cargo test`
> blocks before it compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the
> IDE; keeping one warm is also what makes the full gate ~15 min.

---

**Session date:** 2026-08-30 (**#503 — the shared keystore crate**: `cairn-sync` loads the node's provisioned unwrap key; federated sync restored; opened #514, #515) · previous: 2026-08-24 (**DR slice 1** — the node unwrap key stops dying with the signing seed: #495 CLOSED, #500 still open; opened #503–#509, #511–#513; review wave also closed #502 item 2) · **Spec/ADRs:** v0.68 (through **ADR-0066** — *identity dies with the disk; custody must not*) · **`SCHEMA_GENERATION`:** 50 (`db/050`; slice 1 adds no migration) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices
1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/gender-identity · §4.3
address) and **the §5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute`
waits on a clinical-note surface) · **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the
§5.3/§5.8 search-before-create funnel** (ADR-0061; precedence rule #345 at db/005 step 8b) ·
**`clinical.medication` slices 1–6b** (ADR-0047/0048/0049/0050/0051/0059) under **born-sealed bodies**
(ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9
stream complete through its read surface**, enforcing nothing beyond display/emission · **the med-list
node tier** (first clinical READ path + whole-list sign-off), **generic reprojection** (ADR-0057), the
**ADR-0056 admit-uninterpreted floor** and the **residual refusal contract** · **the L3 reference UI** —
`cairn-gui/`, a standalone workspace, one-way GUI → crates; the iced shell FAILED the accessibility bar
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`** on one patient's medication chart (plain
JS, no npm), with the pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.
### 2026-08-30 (last) — #503: the shared keystore crate, and federated sync comes back

**Closes [#503](https://github.com/cairn-ehr/cairn-ehr/issues/503); opens #514, #515. No ADR, no spec
bump, no migration — this IMPLEMENTS ADR-0066, it decides nothing ADR-0066 left open.** New crate
`crates/cairn-keystore` (the `CAIRNK1` sealed-bundle format + the key-file loader + the crash-safe
atomic write, moved verbatim out of `cairn-node`), which `cairn-sync` can depend on without depending
on a whole node application. `cairn-node` re-exports the three modules, so its **221** call sites across
~30 files compile untouched — that was the extraction's whole proof. `cairn-sync` then resolves its
custody key **once at startup** through a pure decision table (`src/unwrap_key.rs`) and threads a
`NodeCustody` value down, replacing six independent derivations. What generalises:

1. **⇒ A GUARD THAT REJECTS A DEAD ENTRY MAKES ITS OWN LIST A SEQUENCING CONSTRAINT.** The plan had the
   `derive_unwrap_secret` allow-list entry merely *reworded* at the end. But the call MOVES file
   mid-slice (`main.rs` → `unwrap_key.rs`) and then disappears, so the guard would have failed twice —
   an unlisted offender, then a dead entry. **Caught by the pre-flight plan scan, not by a test**, which
   is the argument for reading a plan against the gates before handing it to anyone. **When it fails,
   delete the entry it names; never add one.**
2. **⇒ `cargo test --bin X <filter>` COMPILES WITH `cfg(test)`, SO NEW ITEMS LOOK USED.** Tasks 2-3 left
   four items with no production caller; the scoped test command hid it and only a plain build (which
   `warnings = "deny"` fails on `dead_code`) surfaced it. Same trap already recorded for `--lib`:
   **use `--all-targets`** when the question is "is this reachable from production".
3. **⇒ DELETING A HELPER DELETES ITS TEST'S PIN, AND THE PIN MAY BE THE ONLY ONE.** Removing
   `unwrap_key_matches` also removed the only assertion that a divergence refusal names both keys,
   ADR-0066 and the issue. The surviving tests checked the file path alone — so a later edit shortening
   the message would stay green, at the one moment (a restored node, at 3am) the message is all the
   operator has. **Ask what a deleted test was pinning, not just what it was testing.**
4. **⇒ A FAIL-OPEN BRANCH PROTECTED ONLY BY A COMMENT IS PROTECTED BY NOTHING.** The refusal of a
   present-but-not-32-byte `node_unwrap_key` row sat inside a DB-touching function, unreachable by any
   test, with a comment warning against exactly the "simplification" that would break it. Extracted as
   the pure `registered_from_row` and tested. The rule: **if a comment has to warn future readers off a
   change, a test should be making that change fail.**
5. **The gate's real cost is macOS, not cargo.** A cross-cutting change (Cargo.lock + `cairn-node`)
   relinks ~134 test binaries, each drawing a one-time Gatekeeper assessment: **~6 hours**, and a warm
   `target/` does not help. Confined slices run `cargo test -p cairn-sync` — which **does** build
   `tests/clinical_pull.rs`, the two-node custody suite; the cross-crate blindness the full gate exists
   to cover belongs to `-p cairn-node`, not to it.

### 2026-08-24 — DR slice 1: the unwrap key stops dying with the signing seed (condensed)

**Closed [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) (ADR-0066, spec v0.68) and #502 items
1–3; opened #503–#509, #511–#513. No migration.** Shipped: an independent X25519 unwrap keypair sealed
in its own `<key>.unwrap` file; a **lossless adoption path** for pre-ADR nodes
(`keystore::adopt_derived_unwrap_secret`, the one place a node re-derives its old secret, **once**); the
secret and the surviving custody rows riding the `CAIRNL1` export with a **shredded event's DEK excluded
by construction**; and `restore` INSTALLING the inherited key instead of minting one. **#495's status,
#500's, and the five traps are in ⇒ NEXT — read that split before citing this anywhere.** Still open:
**#504** (dead `_node_sk` — a decision, not a refactor) · **#505** (the migration mints a SECOND recovery
code) · **#506** · **#507** · **#508** (CBOR leaves unwiped copies of the unwrap secret in freed heap —
a container-format decision) · **#509** · **#511** (`Secret32`/`PublicKey32` newtypes — every key in the
custody plane is a bare `[u8; 32]`, so public-for-secret compiles) · **#512** · **#513**. What
generalises:

0. **⇒ THE REVIEW WAVE FOUND THE SLICE'S OWN FAILURE SHAPE INSIDE THE SLICE — TWICE.** `restore` told an
   operator *"The export itself is intact; the code is what failed"* after a spent recovery-code budget —
   it cannot know that, since damage inside the sealed body and a mistyped code produce the identical
   `None`. **A precise untruth in the reassuring direction, at the one moment the operator has no second
   attempt.** And **ADR-0066 decision 1 asserted a property this branch's own code contradicted**; ADRs
   are immutable once merged, so it was scoped pre-merge instead. ⇒ **The window in which an ADR is
   ordinary editable prose closes at merge — spend it.**
1. **⇒ BREAKAGE HID FROM A GATE IN THREE DISTINCT WAYS IN ONE SLICE:** fail-fast masked 13 failures;
   **`cargo test … | tail` masked cargo's exit status entirely**; and a **cross-crate** suite was
   invisible because `-p cairn-node` never builds it while `cargo check --workspace` compiles it
   **without running it**. ⇒ Use `--no-fail-fast`, never pipe cargo to `tail`, and never accept
   `cargo check --workspace` as proof another crate's tests pass.
2. **⇒ FOUR OF THIS SLICE'S DEFECTS WERE IN THE TASK BRIEFS, NOT THE IMPLEMENTATIONS** — code that was
   not rustfmt-clean, a gate invocation naming one of three required DB variables, a leaf shape that
   would have left a safety assertion permanently vacuous, and a key install control flow could never
   reach. **Root cause: the instructions were checked against what the code should do, never against the
   gates and control flow the project actually runs.**
3. **⇒ REGISTERING CUSTODY AS A WRITE-PATH SIDE EFFECT IS WHAT LET THE DEFECT HIDE** — making it
   provisioning turned **six** suites red: the measure of how much rested on the implicit act.
4. **⇒ WHERE NO TEST CARRIES THE VALUE ACROSS THE DISK, THE ONE LINK THAT MATTERS IS PROVEN BY NOTHING.**
   `unwrap_secret` carries `#[serde(default)]`, so a `skip_serializing` mutant deserializes to `None`
   **silently** — every DR test green, every restore keyless. Mutation found it; the green suite could not.


### 2026-08-23 (four passes) — the DR audit, §5.9 part C's design, and the misclassification cluster (condensed)

**Pass 4 — the DR-guarantee audit** that produced DR slice 1: confirmed #495 in code, split #500 out of
it, opened #502, added `dr_clinical_guarantee_gap.rs` (5 tests, every assertion mutation-checked).
**Pass 3 — §5.9 parts C+D designed** (ADR-0065, spec v0.66 → v0.67; opened #494–#496, answered #376,
merged #377); **#494**, **#496**, **#498**, **#499** stay open. **Passes 1–2 — the misclassification
cluster** (closed #489, #482, #480, #490 items 1–2, #481, #479, #477; opened #485, #487–#492). Still
open from the sweep: **#490** item 3 · **#483** (`connection_label` will not compile on Windows; no
exposure — CI is all `ubuntu-24.04`) · **#484** · **#487** · **#488** · **#491** · **#492** · **#485**
(23 further cairn-node files, 89 postgres call sites, name no operation) · **#476** (~124 test-guard
comments calling a per-database advisory lock "cluster-wide"). What still binds:

1. **⇒ THE CEREMONY SUCCEEDING IS THE WORST SHAPE OF THIS BUG.** The backup arm sealed an **empty**
   bundle into a valid `CAIRNL1` container and reported success; `verify-backup` passed;
   `backup-status.json` recorded a true count of what the medium actually held. **Every surface honest,
   the composite a precise untruth.**
2. **⇒ TWO DEFECTS THAT LOOK LIKE ONE MUST BE SPLIT WHEN FIXING EITHER ALONE IS USELESS** (#500 the
   bytes, #495 the key) — which is what let slice 1 close one and leave the other quotably open. And
   **⇒ WHERE A GUARANTEE IS ALREADY FALSE, PIN THE DEFECT, NOT THE PROMISE**: a permanently-red test
   blocks the gate for every unrelated change, so each assertion names what it must be INVERTED to.
3. **⇒ A CLASS IS AN OPERATOR INSTRUCTION, AND A DEFAULT-BY-ELIMINATION IS NOT ONE.** **⇒ The
   recogniser is a TYPE or an `io::ErrorKind`, never the message text — and a TYPE outranks a KIND.**
   **⇒ FLATTENING A CAUSE IS WORSE THAN MIS-CLASSIFYING IT** (`format!`/`anyhow!("…: {e}")` consumes the
   source, so a classifier can never be *taught* to recognise it).
4. **⇒ WHERE A PIN'S FIXTURE IS BUILT BY THE TEST, THE PRODUCTION SITE IS UNPINNED**, and **a guard only
   runs when its own crate is tested**. **`file!()` is the path the INCLUDING file wrote, not a canonical
   one.** ⇒ **That pass's ROADMAP condensation deleted the "Open-issue index", orphaning 22 live numbers
   in one edit. A line cap is never a reason to drop a live issue.**

### 2026-08-22 → 08-20 — the db-error sweep, the freeze that hid, the flake that lied (condensed)

**Closed #460, #465, #467, #469, #471, #473–#475 (`db/050`, SCHEMA 49 → 50); #370, #457, #449–#453,
#386, #381/#382/#385/#439, #446/#442/#443; opened #458.** ROADMAP carries each pass in full. Still open:
**#463** (attachment-flag resolution — a DECISION, overlay vs delete) · **#464** · **#458** (non-object
attachment element — a loud UI, NOT a floor rule) · **#470** · **#447** · **#327**.

1. **⇒ `tokio_postgres::Error`'s `Display` IS THE STRING `"db error"`** — a bare kind match never chains
   to the source holding the message, DETAIL and SQLSTATE. **`LocalDbFault` IS NOT A RENDERING and must
   not be "tidied" into an `anyhow!`**, which takes a formatted `String` and **silently reverts every
   local fault to `partition`.**
2. **⇒ A FROZEN CURSOR LOOKED EXACTLY LIKE A HEALTHY CYCLE.** All three of `pull_into`'s freeze paths
   `break` and return `Ok` (correct — freezing is the deliberate availability choice), so a `53100`
   disk-full emitted **neither** `LOCAL FAULT` nor `PARTITION`.
3. **⇒ THE CATEGORY TEST:** a sensitivity assertion **IS** an event (refusing a malformed one drops that
   assertion alone); `safety`, `clock_grade` and a rendition reference are **FIELDS ON** a clinical
   event, and refusing those forks the event set between honest peers — the **#342** trap. Do not
   "align" db/027, which raises where db/050 records: **`WHEN OTHERS` there would write a disk error
   into the ledger as peer garbage.**
4. **⇒ A FLAG CAN BE BORN ON A RE-APPLY**, so the report is keyed on the admitted addresses **and** a
   `flag_id` watermark. **A failed read reports `null`, never `0`.** And **peer text is not display
   text**: `custody_withheld` is unbounded prose from an unadmitted peer, printed raw.
5. **⇒ PROBE THE FAMILY BEFORE FIXING THE MEMBER.** #370 named one field; measured, that function had
   **nine** freeze paths across four SQLSTATE classes **and four SILENT paths that wrote something
   wrong**. The rule: **refuse what already FAILED plus what was silently WRONG; accept everything that
   worked** — every refusal at a remote door is a new way to pen a peer's clinical event.
6. **⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`**
   (#450). An **opt-out** must read an unrecognised value as *NOT permission*. **A mutation that does
   not change the property tests nothing.**
7. **Mechanics worth reusing.** To force a write failure in a SHARED test database take a LOCK from a
   second connection under a short `lock_timeout` — never a trigger or a `REVOKE`, which persist past a
   panic and poison every later suite. **`Debug` must delegate to `Display`** on any error reaching
   `main`. PostgreSQL checks a function called inside a VIEW against the **INVOKING** user — the INNER
   call too. `git check-ignore` needs `--no-index` and has **THREE** exit codes (0/1/**128**).


### Older passes (Slices 61–69, 2026-08-02 → 08-20) — the lessons still worth holding

ROADMAP carries every slice in full. These are the ones a next session can still break.

1. **⇒ A guard defined over the list it guards is not a guard.** `assert_eq!(SubjectKind::ALL.len(), 3)`
   over an `[SubjectKind; 3]` compared a compile-time constant to its own literal. **Ask what independent
   source a guard checks against; if the answer is "itself", it is documentation wearing a test's
   clothes** — and where a family HAS an authoritative list, read the list (**`proacl`'s NULL ACL is the
   PERMISSIVE case**). Related: **NAME, NEVER COUNT** — a count cannot separate **custody-blind** from
   **genuinely empty**; **a union view whose arms mean opposite things must never get one summary
   sentence**; and **the report declares what it cannot contain**, asserted over an **empty** list.
2. **⇒ AN OPTIMISATION REMOVED A LOAD-BEARING REDUNDANCY, AND ITS COMMENT ASSERTED THE OPPOSITE.** §11's
   bound is gated on the NEGATION of §10b's thread-free list, so widening that list leaves a standing
   `sequestered` grade reading back `('routine','none')`. When an optimisation makes two paths share a
   predicate, ask what redundancy that destroyed — **a wrong safety argument is worse than none.**
3. **`TargetState::OnAnotherChart` must never collapse into `Held { still_standing: false }`** —
   ADR-0064's KNOWN GAP. `cairn_sensitivity_standing` is patient-scoped on both sides (load-bearing —
   else chart B strips chart A), so a mis-charted withdrawal's target IS absent and a naive membership
   test reports it **effective**: a precise untruth in the reassuring direction on a confidentiality
   surface. **#436** is the residual, and it is visibility, not a door.
4. **Two floor traps whose issues carry the detail.** **A pinned `search_path` must deny the temp schema
   the FIRST look** — with a decoy `event_log` both write doors **returned SUCCESS while the
   owner-privileged INSERT landed in the caller's temp table** (live data loss): **#430**, **#431**. And
   **a parameter name is not a security property** — `classify_authorship_confidence` graded a forgery
   `Attested`; both key arguments are now a `VerifiedKid` newtype (**#428**), and **`attester_key` alone
   is NOT proof**.
5. **Slice 68:** the authority floor **gates effect, never admission**, and only in the withholding
   direction, so no fork (the **#342** trap); **computing the verdict at read cuts both ways** — revoking
   an actor silently re-raises grades they lawfully declassified (**#409**), while the Rust↔SQL mapping
   diverges on two shapes (**#408**, root cause **#413**). **Flag what cannot self-heal, view what can.**
   PR #410's review: **7 of 11 production mutations survived a green suite.** **`EXCEPTION WHEN OTHERS`
   does not catch a statement timeout** (57014). Open: #413–#420, #422; **#415** measures the SIGNER, so
   **expect noise**.
6. **Slices 66–67 — the seal boundary is the coarsening boundary, and *withhold the key, never the
   bytes*.** Emission coarsening binds a peer's raw-SQL client; **read coarsening is a rendering choice,
   not a floor**. `safety_class_map` ships **EMPTY** — drugref's seam. The unwrap-cert kid is pinned to
   `trust_peer` (db/007) because refusing the bytes would fork the event set; repair is TWO steps
   (`pull --full`, then `cairn_reproject()`). Open: **#406**, **#407**, #394–#402.
7. **Slices 61–63 — the seam and the surface.** An attestation **NAMES** the displayed candidates, it does
   not count them (§1.2 write-cost half: **#360**); **a displayed row is a GROUP, an attestation is a
   THREAD** (ADR-0047/0049); **a unit-tested safety control can still be defeated by the surface that
   calls it**; and **a compensating control outside CI is not a control** (**#444**).

> [!IMPORTANT]
> **Two maintainer decisions to hold before any composite-clinical-object work.**
>
> **The loud failure belongs in the UI, not the floor** (2026-08-22, from #458): a defective attachment
> fails loud **in the UI** with **no blast radius for the rest of the clinical event** — validate before
> submit (the door refusing the whole event is a backstop that should never fire), fail **at the
> attachment, not at the save**, **no confirmation dialog** (principle 3). The same decision refused a
> mandatory `descriptor` as a floor rule: **principle 4 forbids a required field satisfiable only by
> fabrication** — a rushed clinician types `x`, and an honest absence becomes a precise untruth.
>
> **[ADR-0060](spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) —
> *a defect on one line never invalidates another*: the system may fail to record an order, but it may
> never cancel one.** Hold decision 2 (partial completion reported, never implied) and decision 7 (check
> the transaction boundaries).

**Six repo conventions these runs learned the hard way:**
- **⇒ THIS REPO HAS THREE CARGO TREES, AND A NEW CRATE LANDS IN ALL THREE LOCKFILES.** The root
  workspace, `extensions/cairn_pgx` and `cairn-gui` — the last two are `exclude`d from the root
  workspace but **ship anyway**, and both depend on the root crates **by path**. Adding
  `cairn-keystore` made `cairn-gui/Cargo.lock` stale, and CI runs clippy there with `--locked`, which
  **refuses to regenerate it** (`error: cannot update the lock file … because --locked was passed`).
  **No root-workspace gate can see this** — not `cargo test --workspace`, not the full local gate —
  because neither tree is a member. Refresh `cairn-gui/Cargo.lock` and
  `extensions/cairn_pgx/Cargo.lock` whenever the root workspace gains or loses a crate. Same shape as
  the `PRODUCTION_TREES` lesson: **workspace membership is a build-graph fact; "does it ship" is the
  question, and they are not the same question** (#503, 2026-08-30).
- **A pinned COUNT lives beside the thing it counts, and a new member must be added to it.** A new
  `cairn_decode_hex_or_raise` call site fails `hex_decode_helper.rs`'s exact per-file list; the twin and
  projection registries and `db_errors_stay_legible.rs`'s three counts carry the same shape. The count
  failing is the guard WORKING — fix the list, and say in a comment why.
- **Guard before connect** — take `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`. Bind
  `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **A second human actor needs a distinguishing determinant.** `actor_id` content-addresses the *pinned
  determinant set*, so two clinicians enrolled as `{"role":"clinician"}` collide into one actor and are
  refused (P0001, ADR-0044/[#152](https://github.com/cairn-ehr/cairn-ehr/issues/152)). Use
  `enroll_human_with_role`.
- **`cargo test --lib` does not catch an import used only under `cfg(test)`** — it compiles the lib WITH
  `cfg(test)`. Use `--all-targets`.

**Earlier sessions — condensed.** ROADMAP carries the per-slice detail (Slices 13–60, both tech-debt-loop
"Interlude" entries, every still-open issue). From Slice 60: **a refusal that persists nothing cannot be
audited**, and **when a call site cannot make a distinction, check whether a layer threw it away** (#480).

**GUI/L3 design threads (2026-07-16/18, design-only).** Detail in `scratch/ui-sketches/`; source
screenshots git-ignored under `docs/untracked_for_brainstorming/` — real photos, **never commit or
publish**. Headline: easyGP's six editing-area invariants ≅ Cairn's event envelope (ADR-0021's seam).

**Status of this file:** disposable scaffolding, **not** a source of truth; canonical docs win.
Regenerate each session, **under 500 lines** (#368) — *why* in the ADRs, *what* in the spec.

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not. **`docs/spikes/`** —
0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ →
ADR-0029/0030); 0003 (Postgres on Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2).
**`docs/case-studies/0001`**: 16 GP-software failure modes, all absorbed, **0 new architecture**.
**`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance. Code workspace: `/crates`
(`cairn-event`, `cairn-keystore`, `cairn-sync`, `cairn-node`, `cairn-medication-view`,
`cairn-patient-search`),
`/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace); `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. Every honest gap declared at build time was closed **except the `localstate`
  clinical seams — half of which slice 1 has now filled** (custody travels; the clinical event log still
  does not — **#500**, the ⇒ NEXT warning). Optional escrow *rungs* (Shamir/QR/TPM) also remain.
  **Dual-identifier discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the *only*
  identifier on the wire/in signed bodies; the projection plane may intern node-local `bigint` surrogates
  (`db/008` + the leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx` (`cargo pgrx install`) and self-serialize via a
  Postgres advisory lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are scoped PER
  DATABASE** (#467; ~124 test-guard comments still say otherwise, **#476**), so every caller must take the
  guard against `CAIRN_TEST_PG` specifically.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels and drives `/techdebt-next` one fresh
  headless session per issue (`tail -f ~/.cairn-loop/run.log`). Auto-merge **ENABLED**; **works unattended**
  (12 PRs); **stopped** by maintainer decision — see ⇒ NEXT. Live gaps: **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **DR slice 2 — #500**, the remaining half and the next build; see ⇒ NEXT.
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization hooks), **#236** (FTS/RAG
  must build on `event_clear`).
- **`clinical.medication` — slices 1–6b are DONE** (ADR-0059). **Next candidates:** the **drugref
  term→anchor lookup** (⇒ NEXT); fuzzy/automatic reconciliation + a Tier-A drug dictionary; structured
  sig/frequency (lands with prescriptions); correcting a dose event's *effective date* on the
  statement-level `started`. **Cross-cutting debt: #185.** Spine: `db/031`–`db/033`, `db/041`, `db/042` +
  `cairn-event::medication`.
- **Demographics / matcher / identity — next slices** (spine: `db/010`–`db/030` +
  `cairn-event::demographics`). **Next (B3 measurement-driven):** a large hand-crafted gold set to re-run
  the learner; locale comparator packs; the hub-tier duplicate sweep; proposal retraction. **Next
  identity:** C5+ `reattribute` (**waits on a clinical-note surface**); the §5.12 push-alert. Deferred:
  **#168**, **#287**; the rest are in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, and it is the
  **only** gate that catches all three of this repo's demonstrated hiding modes (fail-fast · a piped exit
  status · a cross-crate suite `-p <crate>` never builds): the `db/tests/*.sql` mirrors *and* the full
  workspace with `CAIRN_TEST_PG`/`PG2`/`PG3` baked in (PG18 + cairn_pgx on `127.0.0.1:5532`, databases
  `cairn_test`/`2`/`3`); last full pass **EXIT 0 over 139
  binaries** (2026-08-24; re-run 2026-08-30 after #503 added a crate). **⇒ COST DEPENDS ON WHAT CHANGED,
  NOT ON TARGET-DIR WARMTH — a warm `target/` does NOT make it ~15 min, and believing it does cost this
  project a 6-hour surprise** (#503, 2026-08-30). A change that touches `Cargo.lock` or `cairn-node`
  relinks ~134 test binaries, and macOS runs a one-time-per-binary Gatekeeper assessment on each: **budget
  HOURS.** A slice confined to one crate reruns almost nothing — run `cargo test -p <crate>` and let CI
  gate the rest. Note `-p cairn-sync` **does** build `tests/clinical_pull.rs`: the cross-crate blindness
  above belongs to `-p cairn-node`. Without the three strings the DB-gated suites **self-skip and cargo counts
  them as passed**, so **since #450 a run without them FAILS unless it declares `CAIRN_ALLOW_DB_SKIP=1`**
  (only `1`/`true`/`yes`/`on` opts out). The mirrors are DESTRUCTIVE and refuse any database lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline
  pytest` (uv, never venv/pip). Local gap: **#314** (the script skips the matcher DB-gated pytest suite;
  CI runs it). **`clinical_pull` used to flake under a full-workspace run** — #457 fixed the DIAGNOSTIC,
  not the cause; **the cause is still unnamed**, `--test-threads=2` is the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have absorbed
  every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack), a named
  projection surfaced by salience not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  **PASS twice.** Remaining: fold the un-caveated B4 number into ADR-0015 to drop "provisional" from the
  blob-digest line, and **#272** (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access (the `rx!`/`tx!` parser + state machine; the formulation/drug
  source + the **forced-manual** rule table; the prefetch warming daemon). Pre-read
  `scratch/ui-sketches/easygp-prefetch-notes.md`. **GUI-mining continues** and opens the **results/inbox
  design session** (three-zone vs two-pane is parked there — don't improvise it).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice. The §8.2 availability + windowing/resume work shipped.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** stewarding legal entity & jurisdiction — deferred
  until momentum/funding geography is clearer; formal trademark registration — principle recorded
  (stewardship doc), legal instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session** — the working conventions,
  the twelve founding principles, and the §9 defect-blast-radius language rule. Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured (`cairn-ehr`
  org; `cairn-ehr.org`+`.com`; PyPI/crates.io/npm `@cairn-ehr` placeholders).
