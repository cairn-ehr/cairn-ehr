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
>   inherit and open its own custody.**
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
> **[#502](https://github.com/cairn-ehr/cairn-ehr/issues/502) — item 1 ONLY is fixed** (a
> present-but-unreadable export now **refuses the restore** before the door fences shut, rather than being
> skipped in silence); **items 2–4 stay open** — `verify-backup` printing `backup OK: 0/0` over a medium
> that restores nothing, a corrupt `.lsk` diagnosed as "absent" with a remedy that then refuses, and a
> discarded keystore-load reason.
>
> **The reusable lesson, and the reason this hid for weeks:** *a deferral is only honest while its stated
> precondition holds, and nothing in the repo watches for one expiring.* The `localstate.rs` module header
> declared its seam truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052
> made that false without reopening it, while ROADMAP kept recording slices A–D as ✓ done.
> **Before trusting any ✓, check whether the sentence that justified it is still true.**

> [!IMPORTANT]
> **Four traps slice 1 minted. Each is a step a next session takes in good faith.**
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY.** It survives so a pre-ADR-0066 node can
>    re-derive its old secret **exactly once**, inside `keystore::adopt_derived_unwrap_secret`, keeping its
>    existing `event_dek` rows openable. **Calling it anywhere else re-creates the #495 coupling.** Pinned
>    by `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, which sweeps `crates/*/src/**` and whose
>    allow-list also asserts **every entry is still live** — a dead entry fails the guard, so the list
>    cannot quietly widen. **When it fails, do not add a line to `ALLOWED`.**
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6).
>    `ensure_unwrap_key` and `submit_event` now **refuse**. **A node whose database is recreated under an
>    existing key file needs `cairn-node establish-unwrap-key` before its first sealed write** — six test
>    suites (one in another crate) depended on the old implicit behaviour. Never make a red fixture green
>    by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync` still DERIVES its unwrap secret** — no production dependency on `cairn-node`, so it
>    cannot read the keystore file, and a freshly-provisioned node registers an independent key while the
>    daemon derives a different one. It now **fails fast at startup** rather than degrading into a serve arm
>    indistinguishable from a peer with no custody. Real fix: a shared `cairn-keystore` crate, **#503**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It adopts a
>    secret derived from the **new** signing seed and registers it, and `node_unwrap_key`'s singleton
>    registrar then refuses the real exported key **permanently**. It is the operator's obvious next step
>    — `submit_event`'s own refusal text names that command — so `restore` warns about it explicitly.
>    Recover the export first.

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

**Session date:** 2026-08-24 (**DR slice 1** — the node unwrap key stops dying with the signing seed: #495 CLOSED, #500 still open; opened #503–#508) · **Spec/ADRs:** v0.68 (through **ADR-0066** — *identity dies with the disk; custody must not*) · **`SCHEMA_GENERATION`:** 50 (`db/050`; slice 1 adds no migration) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

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
### 2026-08-24 (last) — DR slice 1: the unwrap key stops dying with the signing seed

**Closes [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495)
([ADR-0066](spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md), spec v0.68); opens
#503–#508. 20 commits, 40 files, +6054/−461; full workspace sweep EXIT 0 over 139 binaries. No migration —
SCHEMA stays 50.** Shipped: an independent X25519 unwrap keypair sealed in its own `<key>.unwrap` file
(same dual-recipient envelope as the signing key); a **lossless adoption path** for pre-ADR nodes
(`keystore::adopt_derived_unwrap_secret` re-derives the old secret **once**, so existing `event_dek` rows
stay openable); the secret and the surviving custody rows riding the `CAIRNL1` export with a **shredded
event's DEK excluded by construction**; and `restore` INSTALLING the inherited key instead of minting one.
**#495's status, #500's, and the four traps are in ⇒ NEXT — read that split before citing this anywhere.**
Opened: **#503** (cairn-sync derives — extract a shared `cairn-keystore`), **#504** (dead `_node_sk` on two
orchestrators — removing it would silently drop a passphrase ceremony, so it is a decision, not a
refactor), **#505** (the migration path mints a SECOND recovery code, contradicting ADR-0066 decision 1 —
needs an erratum), **#506** (`establish-unwrap-key`'s CLI arm has no integration test), **#507**
(provisioning duplicated across 8 fixtures), **#508** (CBOR serialization leaves unwiped copies of the
unwrap secret in freed heap — the real fix is a container-format decision, not a patch). What generalises:

1. **⇒ BREAKAGE HID FROM A GATE IN THREE DISTINCT WAYS IN ONE SLICE.** (a) **fail-fast** masked 13
   failures in `medication_coding.rs` — one red binary and cargo stops looking; (b) **`cargo test … | tail`
   masked cargo's exit status entirely**, so a run was reported as "exit 0" while containing a real failure
   (the pipeline's status is `tail`'s); (c) a **cross-crate** suite (`cairn-sync/tests/clinical_pull.rs`)
   was invisible because `-p cairn-node` never builds it while `cargo check --workspace` compiles it
   **without running it**. ⇒ **`scripts/run-db-gated-tests.sh` is the only gate that catches all three.**
   Use `--no-fail-fast`, never pipe cargo to `tail`, and never accept `cargo check --workspace` as proof
   another crate's tests pass.
2. **⇒ FOUR OF THIS SLICE'S DEFECTS WERE IN THE TASK BRIEFS, NOT THE IMPLEMENTATIONS.** Verbatim brief code
   that was not rustfmt-clean (the CI fmt gate would have failed the PR); a gate invocation naming one of
   the three required DB variables; a leaf shape (`event_id` as TEXT) that would have left a
   shredded-event safety assertion **permanently vacuous** against its raw-16-byte scan; and a key install
   placed where control flow could never reach it. **Root cause common to all four: the instructions were
   checked against what the code should do, never against the gates and control flow the project actually
   runs.** ⇒ **A plan that supplies verbatim code and commands must be run against the project's own gates
   before it is handed to anyone.** Each was caught by an implementer or reviewer pushing back on the
   brief — the process working, expensively.
3. **⇒ REGISTERING CUSTODY AS A WRITE-PATH SIDE EFFECT IS WHAT LET THE DEFECT HIDE.** The old
   `ensure_unwrap_key` registered a key on the first sealed write, so a node with no custody key never said
   so — it silently minted one from whatever seed it held. Making it provisioning turned **six** suites red:
   the measure of how much behaviour rested on the implicit act.
4. **⇒ WHERE NO TEST CARRIES THE VALUE ACROSS THE DISK, THE ONE LINK THAT MATTERS IS PROVEN BY NOTHING.**
   `unwrap_secret` carries `#[serde(default)]`, so a `skip_serializing` mutant deserializes to `None`
   **silently**: before a populated round-trip test existed, that mutant left every DR test green and every
   restore keyless — this slice's own failure shape, one layer down. Mutation found it; the green suite
   could not.

### 2026-08-23 (four passes) — the DR audit, §5.9 part C's design, and the misclassification cluster

**Pass 4 — the DR-guarantee audit that produced the slice above.** Confirmed #495 in code, split #500 out
of it, opened #502; added `dr_clinical_guarantee_gap.rs` (5 tests, every assertion mutation-checked).
Status is in ⇒ NEXT. What still binds:

1. **⇒ THE CEREMONY SUCCEEDING IS THE WORST SHAPE OF THIS BUG.** The backup arm sealed an **empty** bundle
   into a valid `CAIRNL1` container and reported success; `verify-backup` passed; `backup-status.json`
   recorded a true count of what the medium actually held. **Every surface honest, the composite a precise
   untruth** — principle 4, and ADR-0026 decision 7's *"must say so"*, violated by a system in which no
   single component lies.
2. **⇒ TWO DEFECTS THAT LOOK LIKE ONE MUST BE SPLIT WHEN FIXING EITHER ALONE IS USELESS.** #500 is the
   bytes, #495 the key. Filed apart so neither could be closed on the strength of the other — which is
   what let slice 1 close one and leave the other visibly, quotably open.
3. **⇒ WHERE A GUARANTEE IS ALREADY FALSE, PIN THE DEFECT, NOT THE PROMISE.** A permanently-red test
   blocks the gate for every unrelated change, so the suite asserts what is true **today**, each assertion
   naming what it must be INVERTED to. Anti-vacuity explicit: the node is provisioned, the DEK is written
   by the **production door**, and the pure test asserts the happy path *first*.
4. **Cross-ADR claims about the same key material need checking where they meet, which is code.** The
   derivation bought *"no new key-management mechanism"* and paid with a contradiction against ADR-0026
   decision 4 that **neither ADR could see from inside itself**.

**Pass 3 — §5.9 parts C+D designed** (ADR-0065, spec v0.66 → v0.67; opened #494–#496, answered #376,
merged #377). The ladder is in ⇒ NEXT; **#494**, **#496**, **#498**, **#499** stay open. Four lessons, all
argued in full in the ADR: **⇒ a control a faithful peer defeats *by computing correctly* is not weak — it
is incoherent** (the real quiet leaks are registry divergence and replication lag, not thread resolution) ·
**⇒ "conservative" is a property of a DIRECTION, not of a value** — before reusing a bound, ask what it now
drives · **⇒ *refuse at a door only what that door can drop whole*** — the question is never how defective
the bytes are, it is what else dies with them · **⇒ fail-closed was affordable only because something else
guaranteed reachability**, and that guarantee is already bounded (**#498**). ⇒ **That pass's ROADMAP
condensation deleted the "Open-issue index", orphaning 22 live numbers in one edit. A line cap is never a
reason to drop a live issue.**

**Passes 1–2 — the misclassification cluster and the sweep's tail** (closed #489, #482, #480, #490 items
1–2, #481, #479, #477; opened #485, #487–#492). All one species: **a failure wearing another subsystem's
clothes.** Traps that still bind:

1. **⇒ A CLASS IS AN OPERATOR INSTRUCTION, AND A DEFAULT-BY-ELIMINATION IS NOT ONE.** Both pull classifiers
   used `partition` (*go and look at the link*) as catch-all **and** as a diagnosis. **⇒ The recogniser is
   a TYPE or an `io::ErrorKind`, never the message text — and a TYPE outranks a KIND**, so `LocalFault` is
   checked FIRST and a broad `ErrorKind` net cannot re-label what a concrete type already claimed.
2. **⇒ FLATTENING A CAUSE IS WORSE THAN MIS-CLASSIFYING IT.** `format!`/`anyhow!("…: {e}")` consume the
   source, so a classifier can never be *taught* to recognise it. **And a reachable cause can print TWICE.**
3. **⇒ THE FIX HAD THE DEFECT IT WAS FIXING, ONE MATCH ARM ABOVE ITSELF**, and the existing `40001` test
   was one assertion away from proving it. Sibling: **⇒ a guard for an ordering property that could not
   observe the ordering** — `.context()` values are NOT reachable from `chain()`, so assert both signals
   are present *before* asserting which wins.
4. **⇒ WHERE A PIN'S FIXTURE IS BUILT BY THE TEST, THE PRODUCTION SITE IS UNPINNED**, and **a guard only
   runs when its own crate is tested** — #450's DB-skip guard lives in `tests/common/db_gate.rs`,
   `#[path]`-included by both crates, the obligation DERIVED from which crates read a gate variable. And
   **`file!()` is the path the INCLUDING file wrote, not a canonical one** — caught only because the
   assertion checks it fired **exactly once**.

**Still open from the sweep:** **#490** item 3 · **#483** (`connection_label` will not compile on Windows;
no exposure — CI is all `ubuntu-24.04`) · **#484** · **#487** · **#488** · **#491** · **#492** · **#485**
(23 further cairn-node files, 89 postgres call sites, name no operation) · **#476** (~124 test-guard
comments calling a per-database advisory lock "cluster-wide").

### 2026-08-22 → 08-20 — the db-error sweep, the freeze that hid, the flake that lied (condensed)

**Closed #460, #465, #467, #469, #471, #473–#475 (`db/050`, SCHEMA 49 → 50, no new ADR); #370, #457,
#449–#453, #386, #381/#382/#385/#439, #446/#442/#443; opened #458.** ROADMAP carries each pass in full.

1. **⇒ `tokio_postgres::Error`'s `Display` IS THE STRING `"db error"`** — a bare kind match never chains to
   the source holding the message, DETAIL and SQLSTATE. **`LocalDbFault` IS NOT A RENDERING and must not be
   "tidied" into an `anyhow!`**: `Display` is the legible text, `source()` is what a classifier walks, and
   `anyhow!` takes a formatted `String`, **silently reverting every local fault to `partition`.**
2. **⇒ A FROZEN CURSOR LOOKED EXACTLY LIKE A HEALTHY CYCLE.** All three of `pull_into`'s freeze paths
   `break` and return `Ok` (correct — freezing is the deliberate availability choice), so a `53100`
   disk-full emitted **neither** `LOCAL FAULT` nor `PARTITION`.
3. **⇒ THE CATEGORY TEST:** a sensitivity assertion **IS** an event (refusing a malformed one drops that
   assertion alone); `safety`, `clock_grade` and a rendition reference are **FIELDS ON** a clinical event,
   and refusing those forks the event set between honest peers — the **#342** trap, hit five times (ADR-0065
   NAMES the rule). Do not "align" db/027, which raises where db/050 records: **`WHEN OTHERS` there would
   write a disk error into the ledger as peer garbage.**
4. **⇒ A FLAG CAN BE BORN ON A RE-APPLY**, so the report is keyed on the admitted addresses **and** a
   `flag_id` watermark. **A failed read reports `null`, never `0`.** And **peer text is not display text**:
   `custody_withheld` is unbounded prose from an unadmitted peer, printed raw. Related: **a guard that
   punishes the precise description of its own bug** pushes future writers toward vaguer prose, **and a
   rename is not proof.**
5. **⇒ PROBE THE FAMILY BEFORE FIXING THE MEMBER.** #370 named one field; measured, that function had
   **nine** freeze paths across four SQLSTATE classes **and four SILENT paths that wrote something wrong**.
   The rule: **refuse what already FAILED plus what was silently WRONG; accept everything that worked** —
   every refusal at a remote door is a new way to pen a peer's clinical event.
6. **⚠️ A DATABASE-FREE `cargo test` FAILS UNLESS YOU DECLARE IT: `export CAIRN_ALLOW_DB_SKIP=1`** (#450;
   #451 the matcher, #481 the per-crate runs). An **opt-out** must read an unrecognised value as *NOT
   permission*. And **check a claim against the pinned source before writing it down** — a false comment
   about `std`'s `TcpListener` and `SO_REUSEADDR` steered two rounds of fixes; **a mutation that does not
   change the property tests nothing**; #457's harness polled a PORT and never the CHILD (cause named, not
   fixed — a macOS `_dyld_start` stall).
7. **Mechanics worth reusing.** To force a write failure in a SHARED test database take a LOCK from a
   second connection under a short `lock_timeout` — never a trigger or a `REVOKE`, which persist past a
   panic and poison every later suite (`FOR UPDATE` for a write, `ACCESS EXCLUSIVE` for a read). **`Debug`
   must delegate to `Display`** on any error reaching `main`. PostgreSQL checks a function called inside a
   VIEW against the **INVOKING** user — the INNER call too. `git check-ignore` needs `--no-index` and has
   **THREE** exit codes (0/1/**128**).

**Still open from those passes:** #463 (attachment-flag resolution — a DECISION, overlay vs delete) ·
#464 · #458 (non-object attachment element — a loud UI, NOT a floor rule) · #470 · **#447** · **#327**.

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

**Five repo conventions these runs learned the hard way:**
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
(`cairn-event`, `cairn-sync`, `cairn-node`, `cairn-medication-view`, `cairn-patient-search`),
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
  `cairn_test`/`2`/`3`). A warm `CARGO_TARGET_DIR` makes it ~15 min, not 2 h; last full pass **EXIT 0 over
  139 binaries** (2026-08-24). Without the three strings the DB-gated suites **self-skip and cargo counts
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
