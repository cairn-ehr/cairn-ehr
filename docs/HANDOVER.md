# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ THE DISASTER-RECOVERY PATH IS CLOSED AND NOW REHEARSABLE. #495, #500, #554, #572 AND #570
> ARE ALL SHUT.**
>
> A solo clinic can lose its disk, restore from the medium plus its export, and **open a chart** —
> and, since 2026-09-11, can **rehearse that** without a human at the terminal. The pieces: the KEY
> (#495, ADR-0066), the BYTES' write half (#500, slice 2c), the READ half (#554, slice 2d,
> [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md)), the §1.2 measurement
> (#512's time half, PR #573) and the non-interactive path
> ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md), PR #574).
>
> **What "closed" does and does not mean.** A record a restore cannot apply is **quarantined with
> its custody**, not dropped, and the restore exits **non-zero** saying so. The actor registry
> re-enters on the export container's AEAD alone — the one part of a restore that is **not**
> verify-on-apply, accepted deliberately and printed to the operator. And **rows and custody
> coming back is not the same claim as a body opening**: the tests to cite are
> `restore_reads_the_clinical_plane.rs` (library) and
> `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back` (the shipped
> command), both of which decrypt a real sealed body back to its twin text — never the row counts
> in `dr_clinical_guarantee_gap.rs`.
>
> **The measurement stands and is not re-run.** 100 003 events restore in **116.7 s against a
> 600 s budget**, linear at **1.17 ms/event with no bend**, so the ceiling is predictable near
> **510 000 events** on an M3 Max. Rig `scripts/measure_dr_restore.py`; write-up
> `crates/cairn-node/results/2026-09-10-macos-m3max.md`. ADR-0069 changed how a secret arrives,
> not what a restore costs.

> [!IMPORTANT]
> **⇒ #527/#562's TRIAGE NOTE IS FALSE, AND ADR-0069 CORRECTS IT RATHER THAN BREAKING IT.**
> The note reads *"no cron-run command reaches `print_recovery_code`"*. It was **already untrue
> before 2026-09-11**: a medium with **no local-state export sibling** never reaches the
> recovery-code prompt at all, so a sealed `restore` of one has always run unattended and printed
> a fresh code to stderr. Every other read in the restore arm is print-only, which is ADR-0068's
> ruling rather than an accident. **Do not read ADR-0069's date as the day this stopped being
> true.** The real fix — a `--new-recovery-code-file` sink, or a refusal to mint a sealed key when
> nothing can show its code to a human — is **[#575](https://github.com/cairn-ehr/cairn-ehr/issues/575)**.

> **⇒ #568 IS DONE (2026-09-12). THE REMAINING DR TEST DEBT IS #567, AND IT IS NOW THE SHARPEST
> SAFETY GAP LEFT ON THIS PATH.** **#567**: `verify-backup`'s OK is still federation-only, so an
> operator reads green and rotates the drive while the clinical plane on that medium may be
> chain-broken, gapped or **empty**. The one command whose entire purpose is *"can I still recover
> from this?"* does not ask the question about the half a solo clinic depends on. ⚠️ The notice
> functions it needs are NOT all there — `untrusted_clinical_notice` exists in
> `cairn-node/src/backup.rs`, but **`clinical_gap_notice`, which #567's scope section says was
> added in PR #566, does not exist anywhere in the tree**. Check that before sizing the work.
>
> **What #568 bought, and the one thing to carry from it.** Four DB-gated tests in
> `crates/cairn-sync/tests/requeue_releases_custody.rs` drive the shipped `cairn-sync requeue`
> binary; the assertion is that a sealed body **OPENS** (`event_clear.twin` reads back the dead
> node's text), and all three custody outcomes are pinned separately. **FIVE MUTATIONS WERE RUN AND
> ALL ARE KILLED — and the fourth SURVIVED the first draft**, because the missing-key test named a
> path inside a subdirectory that did not exist either, so the minting loader failed on the absent
> parent rather than being refused. The matrix is in the file header. **A test written against
> behaviour that already works proves nothing until a deliberate break has been shown to fail it.**
>
> Two shipped-code facts the fixture had to learn by failing: `sync_quarantine.refused_seq` is
> **NOT NULL** (a restore passes the record's own `source_seq`), and `cairn_quarantine_event`'s
> BOOLEAN return is **`acked`**, not "was it penned" — a fresh row comes back FALSE.
>
> **SEVEN §7 design tests remain unwritten** — the behaviour is built and green, the pins are not.
> **Test 4 (custody survives the pen → requeue → the body opens) is WRITTEN**, by #568 above.
> Still owed: 7 (the pen uncapped **at a volume above
> the row cap**, which the design demands explicitly because a handful of events passes against the
> unfixed quota), 14 end-to-end through the CLI, 16 (duplicate `source_seq` no-op / substitution
> refused), 17, 19 behaviourally (the mid-restore-crash re-restore), 22 and 23. **PR
> [#566](https://github.com/cairn-ehr/cairn-ehr/pull/566) carries the table.** The design's own
> standard is that *"a decision in §2–§6 with no entry here is a decision this slice is not
> entitled to claim"*.
>
> **⇒ `M > N` STILL STANDS AND #512 STAYS OPEN.** ADR-0068 deleted the provenance confirmation DR
> slice 1's plan blamed for `M = 3`; the real third act is the **recovery code**, a second,
> separately-prompted secret asked for after the node plane is already applied. ADR-0069 does not
> change the count — in a drill it *replaces* one human act with a file read, and an attended
> restore is untouched. What it does change is that #512's *"unattended"* wording is
> **operationally true** for the first time.
>
> **⇒ #552 IS CONFIRMED WITH A NUMBER.** A single capture is linear (~0.15 ms/event); the cost that
> bites never goes away. **A second capture of the same 100 003-event medium with ZERO new events
> cost 9.95 s**, against 15.0 s for the one that wrote all of them — about **two thirds of a nightly
> capture is independent of how much is new**. Its ~23 000-event estimate for crossing a 2 s budget
> is optimistic; interpolation puts it near **14 000**.
>
> **⇒ "2e" IS RETIRED AS A LABEL** (ADR-0067 took its ADR and its spec bump). What was under the
> name is operational: **#551** (the kit-restorability figure has no per-kit home; a same-mount-point
> rotation still false-greens) and **#553** (an unmarked foreign legacy medium can still be destroyed
> by succession). Do not leave "2e" standing as an empty container — that is how future sessions
> defer into one.
>
> **Still broken, all named rather than assumed away:** **#549** (a burned identity `seq` is
> indistinguishable from a lost clinical event; the `seq_gaps` operator surface is re-deferred) ·
> **#552** (a capture is O(whole medium), and **read-side peak memory is unbudgeted** — a Pi or
> Android node is a legitimate restore target, so streaming stays deferred) · **#536** (an unopenable
> DEK is counted on the RESTORE path only; the sync half is open) · **#569** (db/052's registry door
> silently discards a **content** conflict and leaves `actor_event_id`/`seq` unvalidated) · **#502
> item 4** (a discarded keystore-load reason) · **#101 items 2–3** · **#512** · **#575** (new — the
> minted recovery code still reaches stderr on both paths) · **#556**–**#563** (the 2b/2c review
> wave; see ROADMAP).
>
> **Never cite ADR-0026 decision 1's promise 2** — *"node-default data-at-rest keys survive"* — as
> met by any of this. It has **no subject at all**: no node-default key tier exists, so it is
> neither honoured nor violated, and ADR-0067 says so in as many words.

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS, MEASURED 2026-09-12 — AND THE MODEL PACK IS WHAT KEEPS IT THAT
> WAY. PR #576 IS MERGED; ONE HUMAN ACT IS NOW DUE.** `scripts/codeql-alerts.sh` prints *"none —
> the alert gate should be green"*. PR **#576** moved CodeQL from GitHub's
> default setup (languages + suite only, **no tuning possible**) to a committed workflow,
> config and **model pack** (`.github/codeql/packs/cairn/codeql-models`). The finding that shaped
> it, and that #562's triage got wrong: `rust/cleartext-logging`'s sources are **NAME heuristics**
> — a call to any function whose name contains `key`/`cert`/`secret`/`password`/`identifier`/
> `trusted` is a source, and there is no hook to declare a function innocent. The pack holds one
> `barrierModel` row per such function on its **return value**, each with its return type and
> reason beside it. Proven on a CodeQL database of the whole workspace at CI's exact versions
> (CLI 2.27.0, `rust-queries@0.1.42`): **44 → 3**, the three being a variable literally named
> `patient_id` that a CLI echoes back to the operator — unbarrierable, and already dismissed.
> 44, not 10, because the other 34 were dismissed in earlier rounds and would resurface the
> moment a line moved.
>
> **The flip is DONE (maintainer, 2026-09-12) and the PR's `CodeQL (rust)` is GREEN — with a
> lesson that cost a round trip: disabling default setup in the REPOSITORY settings was not
> enough.** GitHub's own `Analyze (…)` jobs kept running on the next push and the upload kept
> being rejected (*"CodeQL analyses from advanced configurations cannot be processed when the
> default setup is enabled"*) until the **organization-level security configuration** was
> changed too. Repository Settings alone is silently overridden by an org configuration that
> has CodeQL default setup *Enabled*; the org option that permits a committed workflow is
> *Enabled with advanced setup allowed*, or *Disabled*. The observable test is whether
> `Analyze (…)` jobs still appear on a fresh push. **THE MERGE HAS HAPPENED, SO THE ONE HUMAN
> ACT IS NOW DUE:** make `CodeQL (rust)` a required check (**#444**). It was deliberately held
> until after the merge — the job names changed with the switch, and a required name that no job
> reports blocks every PR.
> **Read the alert list with `scripts/codeql-alerts.sh`, never assume it** (`gh api` stays
> deny-listed). Reproducing an alert locally now takes three minutes — CONTRIBUTING's CodeQL
> section has the recipe, and `gh codeql` is installed on the Mac at 2.27.0.

> [!IMPORTANT]
> **⇒ #500 SPENT THREE DAYS *CLOSED ON GITHUB*, AND SIX OTHERS WITH IT (2026-09-04):** #101, #115,
> #434, #441, #468, #500, #534, all reopened. GitHub reads `close`/`fix`/`resolve` **adjacent** to a
> reference and never the sentence around it, so the sentences disclaiming the close performed it.
> Now guarded by `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml`;
> promotion to a required check is **#444**. **The commit convention `fix(#500):` is SAFE** — the
> parenthesis breaks the adjacency. Residuals: **#547**, **#548**.
>
> **The reusable lesson:** *a deferral is only honest while its stated precondition holds, and
> nothing in the repo watches for one expiring.* `localstate.rs`'s header declared its seam
> truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052 made that
> false without reopening it, while ROADMAP kept recording slices A–D as ✓ done. **Before trusting
> any ✓, check whether the sentence that justified it is still true.** Slice 2b's grep found SEVEN
> more of this shape in FOUR crates where memory said one; **#511 then found two more inside
> `seal.rs` itself**. **Grep, do not recall.**

> [!IMPORTANT]
> **Seven traps. Each is a step a next session takes in good faith.** (Five came from slice 1; trap 5 was minted by #511, trap 7 by DR slice 2c.)
>
> 1. **`derive_unwrap_secret` is the ADOPTION MIGRATION ONLY** — a pre-ADR-0066 node re-derives its old
>    secret exactly once, inside `keystore::adopt_derived_unwrap_secret`, keeping its `event_dek` rows
>    openable; **calling it anywhere else re-creates the #495 coupling.** Pinned by
>    `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, which sweeps **every shipping tree**
>    (`sources::PRODUCTION_TREES` — `crates/`, `extensions/`, `cairn-gui/`, both `exclude`d trees) and
>    asserts every allow-list entry is still live, so it cannot quietly widen: **when it fails, delete the
>    entry; never add one.** The sweep and the test-gate matcher (`is_a_test_gate_attribute`, which must
>    recognise pgrx's `#[cfg(any(test, feature = "pg_test"))]`) move **together** — widening either alone
>    reddens the guard on correct code.
> 2. **Registering the unwrap key is PROVISIONING, not a write-path side effect** (ADR-0066 decision 6).
>    `ensure_unwrap_key`/`submit_event` now refuse. **A node whose database is recreated under an
>    existing key file needs `cairn-node establish-unwrap-key` before its first sealed write.** Never
>    make a red fixture green by weakening `ensure_unwrap_key`.
> 3. **`cairn-sync` LOADS its unwrap secret now (#503) — with ONE derived fallback, and the reason it is
>    safe is the reason not to widen it.** With no `<key>.unwrap` file AND a derived key that equals the
>    registered one, the daemon starts on the derived secret and warns on every startup (provably
>    pre-ADR-0066). A restored node derives a key that does NOT match and is refused — the #495 shape,
>    caught. Load-bearing asymmetry: an absent file may fall back, a present-but-unusable one (corrupt,
>    or no `CAIRN_KEY_PASSPHRASE`) never may, since a successful derive would mask the rot of the only
>    file carrying this node's custody off the machine. **Never simplify those two into one arm.**
>    Retiring the fallback is **#514**.
> 4. **⇒ NEVER RUN `establish-unwrap-key` ON A RESTORED NODE WHOSE EXPORT COULD NOT BE READ.** It adopts a
>    secret derived from the NEW signing seed and registers it, and `node_unwrap_key`'s singleton registrar
>    then refuses the real exported key permanently (`restore` warns explicitly; `submit_event`'s refusal
>    names the command). Recover the export first; `apply_local_state`'s refusal explains the resulting
>    state and that the way out is another restore into a fresh database.
> 5. **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511, 2026-09-04).** An unwrap
>    secret, an Ed25519 signing seed and a DEK are all `Secret32`, so `Secret32::from_bytes(sk.to_bytes())`
>    compiles — that conversion IS the #495 coupling wherever it is not deliberate. **Do not take the
>    count from any comment: it is pinned in `crates/cairn-node/tests/secret32_conversions_are_named.rs`,
>    per file and by count.** Six production `Secret32::from_bytes` sites exist; exactly **two** turn the
>    signing seed into an unwrap secret — `keystore::adopt_derived_unwrap_secret` (the ADR-0066 migration)
>    and `cairn-sync`'s pre-ADR-0066 fallback (trap 3) — and the rest mint fresh CSPRNG output, seal the
>    seed AS the seed, or compare. A third of the first kind is the defect returning; the
>    newtypes make the PUBLIC-for-secret mix-up a compile error and nothing more. `unwrap_secret_is_the_signing_seed`
>    is still the only check for the secret-for-secret one, and `secret_opens_the_carried_custody` is
>    still the only proof a restored key OPENS anything — neither is made redundant by the types.
> 6. **`init` now refuses a database that already has custody registered.** The file check
>    (`refuse_to_replace_existing_unwrap_key`) only fires when `<key>.unwrap` EXISTS, so a node that lost
>    its keystore could still run `init`, overwrite its signing key and mint a doomed custody key before
>    the registrar failed. `init` reads `node_unwrap_key` first now; the remedy it names is
>    `establish-unwrap-key`, idempotent — see trap 4 before running it on a restored node.
> 7. **⇒ A BODY SHREDDED AFTER A CAPTURE KEEPS ITS DEK ON THAT MEDIUM. THAT IS NOT A LEAK — DO NOT "FIX"
>    IT (DR slice 2c, 2026-09-06).** It looks like an erasure that failed to propagate and it is the
>    definition of a backup: *"a backup is only a backup if it can restore the state of the system at the
>    time the backup was taken. Taking care of invalidated backups is a policy issue, not a core
>    enforcement one. The core will only guarantee availability and integrity of data"* (maintainer). At
>    the moment that medium was written the body WAS readable; a medium that dropped the key later would
>    report a state the node was never in, and could only do so by **rewriting a segment it has already
>    signed** — forfeiting the integrity guarantee that is the core's job. **Never filter old segments.**
>    (The mirror half is equally deliberate: a body shredded BEFORE its first capture never has its DEK
>    written, while its ciphertext still travels — a shred destroys the key, never the event.) Pinned by
>    `crates/cairn-node/tests/medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`, named
>    so nobody repairs it, with the framing and design §2.1 in its header. **What core does NOT do, and a
>    practice must be told:** completing an erasure across backups is **rotation** — capture fresh, destroy
>    old — and that interval IS the maximum time an erasure takes to complete across all copies. The
>    clinic's policy call, not Cairn's (principle 9; ADR-0005's *deletion is best-effort and declared*).
>    ⚠️ **THE ADR THAT OWED THAT SENTENCE NO LONGER EXISTS**: this said "2e's ADR owes it", and
>    "2e" was retired as a label when ADR-0067 took its ADR and its spec bump. The rotation
>    sentence is therefore UNWRITTEN in any decision record, and this trap is currently its only
>    home. Whichever slice next touches backup policy owes it a written home.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A and B
(authority floor + operator surface) are BUILT, enforcing nothing beyond display/emission; C+D are DESIGNED and C1 is
the next §5.9 BUILD — behind #500's slice 2d, which outranks it.** Read **ADR-0062/0063/0064/0065** (`spec/decisions/`) before
touching any of it; do not re-derive their decisions. The authority floor is ONE predicate `cairn_claim_authority`
(db/005) at exactly ONE site (db/048's `NOT EXISTS`), so display coarsening, safety-rung emission and part C's dial
all inherit it — it gives **#245** its first SQL counterpart, not its mirror. Operator-surface §1.2 budget MET and
pinned (residual **#436**).

**Parts C+D (ADR-0065; #377 merged, dependency REVERSED)** are a custody ladder — admission (default) → named nodes →
named actors — under one invariant: **narrowing changes the cost and noise of reading, never whether content can be
REACHED** (audited break-glass at every rung; rung-1 glass is a NETWORK act, so a partitioned non-holder cannot reach
it, **#498**). Node custody is the NORM, per-clinician the EXCEPTION. Not to re-derive: the node's own DEK is the
keyring and the floor is the glass (LOCAL); C and D are NOT separable; custody is an additive field forcing
composition to INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`; unparseable
custody holds NOBODY while the grade still STANDS. **C1** is rung 1 (`custody.nodes`, both doors, serve-door
withholding) + audited break-glass + the in-chart location signal; rung 2 is **#496** (blocked on a reader identity,
§5.11); chart-wide `patient` is OUT of C1, blocked on **#499**.

**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in
db/049 §8 (`safety_read_grants.rs` names it), and that grant is cost-raising, not a floor (**#425**, **#427** — never
cite db/049 §8 as a confidentiality boundary; **#432** asks whether a node should attempt one at all). Slice 65
follow-ons open: **#374** (thread resolution resolves only the current head), **#378** (withdrawal rationale is clear
text forever and replicates — the UI must warn today), **#379** (grade in the twin), **#436** (**#374**/**#379** each
need a DECISION, not a patch). The `arrayref` incident (#445) is closed; residue **#454**.

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

**Three things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 time budget is a seeded figure, not a
measured one** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy; only the *write* half is measured (median 222 ms, **PARTIAL**), Slice 63 owes both halves for
registration (≤5s find, ≤20s register), write-cost half **#360** unwired, and db/044's `gesture_kind` CHECK refuses a
registration row until widened; (2) **the accessibility pass** — a live VoiceOver run through the runbook's eight
checks, keyboard-only (`cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`), DOM
assertions automated by **#332**; (3) **make CI jobs REQUIRED status checks** (**#444**, admin-only — "clippy + cargo
test (cairn-gui)", "cargo doc (API surface)"), matching job names exactly, per `CONTRIBUTING.md`'s dated table; (4)
**making `CodeQL (rust)` a required check, AFTER PR #576 merges** — the alerts are dismissed
(2026-09-11) and the advanced-setup flip is done at repo AND org level (2026-09-12); see ⇒ NEXT. **If a measurement falls outside its budget, that is the finding — file an
issue, never adjust the budget.**

**Other build candidates** (after #500; nothing blocks a choice): the **registration/search UI slice**
(the wrong-chart affordance paper has and the med-list window does not; per Slice 63 must **open** a
chart, never *retarget* one) · the **drugref term→anchor lookup** (the §9 advisory tier; closes the
coded↔uncoded case ADR-0059 decision 5 leaves open, needs a connection-model decision first,
`safety_class_map` its empty seam) · **the node/actor plane's two divergences** — db/007 fail-closes on
an unmappable type (**#301**), the clinical plane skips-and-advances instead (**#268**); neither is a
symmetric fix, both `loop:blocked`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before
repeated cycles pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical
surface has never been through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human
> session holds the main repo. **Never start it alongside one**: they contend on one cargo lock and one
> `test_serial_guard` advisory lock (a stray loop once stretched a session's suites ~3 → ~90 min). **A live
> IDE contends the same way** — rust-analyzer holds the shared `target/` lock, so a narrow `cargo test`
> blocks before it compiles, then times out. Fix is a scratch `CARGO_TARGET_DIR=/tmp/…`, never killing the
> IDE. **Do not read a warm target as a time estimate**: what the gate costs depends on what CHANGED (a
> cross-crate edit relinks every test binary), and the figures in this file have been wrong in both
> directions — #503 budgeted ~15 min and took six hours, #511 relinked the whole tree and finished in
> well under one. Measure the run you are in.

---

**Session date:** 2026-09-12 (**CodeQL moves to advanced setup with a model pack** — the ten false positives are gone by construction, not by dismissal: `rust/cleartext-logging`'s sources are NAME heuristics, eight `barrierModel` rows on named functions' return values take the workspace from **44 → 3** at CI's exact versions; the Settings flip to advanced is done at repo AND org level (the org configuration was the real blocker); PR #576 green; CONTRIBUTING gains the recipe. ⚠️ Branched from `main`, so this line does not know about PR #574 (2026-09-11, #572/#570) — reconcile whichever merges second.) · previous: 2026-09-10, second session (**the DR restore's budget, measured — and the ruling 2d never wrote down.** Closes **#571** with **ADR-0068** (*provenance warns, never gates*; spec v0.69 → **v0.70**); **measures #512's §1.2 budget** — 100 003 events restore in **116.7 s against 600 s**, linear at 1.17 ms/event, 85 000 sealed bodies opening on the restored node. `M > N` still stands but the excess act **changed identity**. Opened **#572** (a restore cannot be scripted at all); **confirmed #552** with a number. No product-behaviour change, no migration, no SCHEMA bump.) · earlier that day: (**DR slice 2d — the record comes home.** `restore` reads the clinical plane back and a restored node's sealed body OPENS; closes **#554**, adds **ADR-0067** (spec v0.68 → v0.69) and **`db/052`** (SCHEMA 51 → 52); the pin `nothing_yet_restores_a_clinical_event_from_a_medium` **inverted, not deleted**; the quarantine pen moved into the database with two callers and gained custody; `ActorRegistryRow::recorded_at` lost its serde default. **Its §1.2 residual is discharged by the session above.**) · previous: 2026-09-07 (**the CAIRNB3 section-framing guard, #523** — a header vouches for its own length, so a corrupt length stops reading as an interrupted append; folded into the 2c branch, the last moment a pre-field format change is free. **The same session found 2c itself unmerged and un-PR'd**) · 2026-09-06 (**DR slice 2c** — the medium carries the clinical record, and nothing yet restored one; closed #522/#524, fixed #550 in-branch, opened #549/#551/#552) · 2026-09-04 (**the closing-keyword guard**: seven issues GitHub had closed that nobody closed, reopened + a CI guard) and, earlier that day, **#511** (**the custody newtypes**; opened #541) · 2026-09-02 (**DR slice 2b** — the transport seam and the paged pull; opened #531, #532, #534–#538) and, earlier, **#527** (the CodeQL backlog; opened #529, #530) · 2026-09-01/08-31 (**DR slice 2a** + its review wave) · 2026-08-30 (**#503**, the shared keystore crate) · 2026-08-24 (**DR slice 1**: #495 CLOSED). Earlier sessions: see *Recent sessions* below. · **Spec/ADRs:** **v0.70** ([ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; and [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **52** (`db/052`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.
**Session date:** 2026-09-11 (**the restore's recovery code gets a non-interactive path, and the CLI surface gets its first tests.** Closes **#572** and **#570** with **[ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md)** (spec v0.70 → **v0.71**); opens **#575**. `--old-recovery-code-file` — a path, never a flag value or an env var — so a DR drill can be **rehearsed**; six CLI tests on plain pipes, one of which reads a sealed body back in clear through the shipped binary; the measurement rig drops its pseudo-terminal. **No migration, no SCHEMA bump, no wire change.** PR **#574**.) · previous: 2026-09-10 second session (**the §1.2 budget measured** — 116.7 s against 600 s, linear at 1.17 ms/event — plus **ADR-0068**, *provenance warns, never gates*; closed #571, opened #572, confirmed #552) · 2026-09-10 earlier (**DR slice 2d — the record comes home**; closed #554, **ADR-0067**, **`db/052`** SCHEMA 51 → 52; the pin `nothing_yet_restores_a_clinical_event_from_a_medium` **inverted, not deleted**) · 2026-09-07 (**the CAIRNB3 section-framing guard, #523** — and the session that found 2c unmerged and un-PR'd) · 2026-09-06 (**DR slice 2c**) · 2026-09-04 (**the closing-keyword guard**, and earlier **#511** the custody newtypes) · 2026-09-02 (**DR slice 2b**, and earlier **#527** the CodeQL backlog) · 2026-09-01/08-31 (**DR slice 2a**) · 2026-08-30 (**#503**) · 2026-08-24 (**DR slice 1**: #495 CLOSED). Earlier: see *Recent sessions* below. · **Spec/ADRs:** **v0.71** ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md); [ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **52** (`db/052`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices
1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/gender-identity · §4.3
address) · **the §5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute`
waits on a clinical-note surface) · **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the
§5.3/§5.8 search-before-create funnel** (ADR-0061; precedence rule #345 at db/005 step 8b) ·
**`clinical.medication` slices 1–6b** (ADR-0047/0048/0049/0050/0051/0059) under **born-sealed bodies**
(ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9
stream complete through its read surface**, enforcing nothing beyond display/emission · **the med-list node
tier** (first clinical READ path + whole-list sign-off), **generic reprojection** (ADR-0057), the
**ADR-0056 admit-uninterpreted floor** and the **residual refusal contract** · **the L3 reference UI** —
`cairn-gui/`, a standalone workspace, one-way GUI → crates; the iced shell FAILED the accessibility bar
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`** on one patient's medication chart (plain
JS, no npm), pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-09-12 (last) — CodeQL: advanced setup, and a model pack instead of dismissals

**Opens PR `ci/codeql-advanced-setup-barrier-models`; closes nothing yet** (the merge waits on a
Settings flip). New: `.github/workflows/codeql.yml`, `.github/codeql/codeql-config.yml`,
`.github/codeql/packs/cairn/codeql-models` (8 rows), a CONTRIBUTING section. What generalises:

- **⇒ #562's TRIAGE WAS THE WRONG SHAPE, AND THE ALERT TEXT SAID SO.** *"writes `foo(...)` to a
  log file"* names a **call**: CodeQL's Rust sources are name heuristics on function calls,
  variables and fields, not taint through an argument. There is no model hook to declare a
  function innocent; there IS a `barrierModel` extensible, kind `log-injection`, on the return.
  One row per function, return type in the comment, or it cannot be reviewed.
- **⇒ FOUR RULES ONLY A RUN COULD ESTABLISH, each after a variant that failed:** `ReturnValue` is
  the CALL node and is what a barrier needs, async or not (`ReturnValue.Future` alone and
  `neutralModel` do nothing); a barrier cannot be narrower than the taint (a tuple built from
  one tainted part is tainted whole, so `Field[0]` cannot spare `bind_serve`'s ServeConfig — the
  row says what that costs); read the SARIF `codeFlows` before choosing the function (the
  passphrase alerts rode the `?` early-return of the *caller*, never touching
  `localstate::apply_local_state`); the binary crate is rooted at the package name **with its
  hyphen** (`cairn-node::f`), the lib at `cairn_node::m::f`, test crates at the target name.
- **⇒ 44, NOT 10.** A full-workspace database at CI's versions found 44; GitHub showed 10 open
  because 34 were dismissed in earlier rounds. A dismissed alert resurfaces when its line moves.
- **⇒ DEFAULT SETUP ALLOWS NO TUNING, AND AN ADVANCED WORKFLOW IS REJECTED WHILE IT IS ON.** The
  settings flip is a human act that must precede the merge. On the far side, rename →
  required-check orphaning (CONTRIBUTING).
- **In-repo model pack without publishing:** `CODEQL_ACTION_EXTRA_OPTIONS` reaches
  `database run-queries` with `--additional-packs` + `--model-packs`; the config's `packs:`
  resolves only registry names at `database init`. Fallback: publish to GHCR.
- **Local reproduction is three minutes now** (`gh codeql`; CONTRIBUTING has the commands). Copy
  the database and run variants concurrently — one CodeQL process locks a database.

### 2026-09-10 — the restore's budget, measured; and the ruling 2d never wrote down

**Closes #571 (ADR-0068, spec v0.70). Measures #512's time half. Opens #572. Confirms #552.** No
product-behaviour change, no migration, no SCHEMA bump. PR #573. What generalises past the slice:

- **⇒ A DESIGN SENTENCE WITH TWO READINGS AND NO TEST SURVIVED A MERGE, AND THAT IS THE LESSON.**
  2d's §5.2 said clinical segments *"inherit the same `Provenance` treatment the node plane already
  gets"*. The node plane's treatment is a **printed warning**; the same paragraph then called it a
  *"safety gate"* forcing `M = 3`. One reading was already built, the other existed nowhere, ADR-0067
  recorded **neither**, and the divergence was invisible because design test 18 was never written.
  The restatement leaves the accepted wording **struck through and standing** for exactly that reason.
- **⇒ THE ERRATA RULE SAID NO TO WHAT THE ISSUE ASKED FOR.** #571 asked for a paragraph appended to
  ADR-0067. `decisions/README.md` allows an erratum only where a passage is factually false **about
  the code**, placed below that passage — and ADR-0067 never mentions provenance, so there was nothing
  to sit under; decision-shaped content takes a new ADR. **Check the rule before honouring the ask.**
- **⇒ THE MEASUREMENT'S REAL RESULT IS THE SHAPE, NOT THE HEADLINE.** 116.7 s against 600 s is
  comfortable, but the useful fact is **linear at 1.17 ms/event with no bend**, which converts the
  budget from a pass/fail into a ceiling at ~510 000 events. A single dot could not have said that,
  which is why the rig runs a curve.
- **⇒ THE RIG REFUSES TO TIME AN INCOMPLETE RESTORE, AND THAT GUARD IS NOT PARANOIA.** A restore that
  applies nothing is **fast**. Driving the real binary produced that outcome on the first attempt (the
  piped recovery code, #572), and a rig that timed it would have written a flattering wrong number
  into a dated file that outlives the session.
- **⇒ A TEST CAUGHT A DUPLICATED COUNT BEFORE IT REACHED THE RESULTS FILE.** `Measurement` carried
  both a seeder-derived event count and the restore's own; they disagree, because the medium also
  holds the warm-up registration. The field was **deleted**, not reconciled — a second spelling of a
  count is a second thing that can be wrong, and this one would have been the published one.
- **⇒ THE SEEDER GOES THROUGH THE PRODUCTION ORCHESTRATORS, AND THE MIX IS LOAD-BEARING.** 85% of the
  corpus is born-sealed. A demographics-only medium carries **no `event_dek` rows**, so the per-record
  unwrap and re-wrap — the expensive half and ADR-0067's whole subject — would never run. Every
  medication assert is authored by an **enrolled human**, so the restore genuinely re-resolves authors
  through `actor_current` rather than leaving decision 1's registry re-entry barely exercised.
- **⇒ THE NODE'S OWN `device` ACTOR IS ENROLLED BY A CLI CEREMONY THE LIB DOES NOT EXPOSE.**
  `ensure_registration_actor` is private to `main.rs`, so the rig runs one real `patient-register`
  first rather than re-spelling it — the mirror-list defect class, avoided by paying one process start.
- **A `results/` directory beside the crate is the existing pattern** (`cairn-gui/cairn-gui-tauri/results/`):
  runbook, template, dated file. The runbook's own precedent holds — *a runbook nobody has executed is
  a runbook that does not work* — and executing this one is what found the pty problem.

### 2026-09-10 (earlier) — DR slice 2d: the record comes home (condensed)

**Closed #554.** ADR-0067, spec v0.69, `db/052` (SCHEMA 52), four crates. The per-slice narrative is
ROADMAP's; the traps that outlive it are in the trap list and ⇒ NEXT above.

**Filed, not fixed — all still open except #571:** **#567** (`verify-backup`'s OK is federation-only,
so a green verify says nothing about the plane a restore now applies; the comment that used to call
that scoping a safety property now calls it a gap) · **#568** (`do_requeue`'s custody-carrying arm —
the remedy every penned reason advertises — has zero tests; every test call site passes `None`) ·
**#569** (db/052's registry door silently discards a **content** conflict and leaves
`actor_event_id`/`seq` unvalidated) · **#570** (the restore CLI surface is untested, exit code
included) · **#571** (provenance does not gate the clinical plane and ADR-0067 does not say so —
**closed by the session above, ADR-0068**).

What still generalises:

- **⇒ THE HEADLINE TEST DECRYPTS A BODY, AND THAT IS THE POINT.** `apply_remote_event`'s `p_dek`
  feeds into `cairn_wrap_dek(p_dek, v_pub)` — **the door wraps what it is handed** — and both
  carriers hold keys already wrapped to this node. Piping either through would double-wrap every
  key in the clinic's record: rows present, well-formed, exactly the right length, counts agreeing,
  `verify-backup` green, and the defect surfacing months later when a clinician opens a chart. **A
  test that counted rows would have shipped it.**
- **⇒ A DESIGN SENTENCE WITH TWO READINGS AND NO TEST SURVIVED A MERGE.** 2d's §5.2 said clinical
  segments *"inherit the same `Provenance` treatment the node plane already gets"*; that treatment
  is a printed warning, and the same paragraph called it a *"safety gate"*. One reading was built,
  the other existed nowhere, ADR-0067 recorded neither, and the divergence was invisible because
  design test 18 was never written.
- **⇒ THE ERRATA RULE SAID NO TO WHAT THE ISSUE ASKED FOR.** #571 asked for a paragraph appended to
  ADR-0067. `decisions/README.md` allows an erratum only where a passage is factually false **about
  the code**, and ADR-0067 never mentions provenance. **Check the rule before honouring the ask.**
- **⇒ THE MEASUREMENT'S REAL RESULT IS THE SHAPE, NOT THE HEADLINE.** 116.7 s against 600 s is
  comfortable; the useful fact is **linear at 1.17 ms/event with no bend**, which converts the
  budget into a ceiling near 510 000 events. A single dot could not have said that.
- **⇒ THE RIG REFUSES TO TIME AN INCOMPLETE RESTORE, AND THAT GUARD IS NOT PARANOIA.** A restore
  that applies nothing is **fast**. Driving the real binary produced exactly that on the first
  attempt (the piped recovery code, #572), and a rig that timed it would have written a flattering
  wrong number into a dated file that outlives the session.
- **⇒ THE SEEDER GOES THROUGH THE PRODUCTION ORCHESTRATORS, AND THE MIX IS LOAD-BEARING.** 85% of
  the corpus is born-sealed; a demographics-only medium carries **no `event_dek` rows**, so the
  per-record unwrap and re-wrap — the expensive half — would never run.
- **Filed by 2d and still open:** **#567**, **#568**, **#569**. (**#570** and **#571** are closed.)

### 2026-09-07 → 08-20 — the sessions before slice 2d (condensed to what generalises)

The per-slice narrative is **ROADMAP's**, and every issue number these sessions opened lives there.
What a next session still needs from them:

- **⇒ AN UNPUSHED BRANCH IS INVISIBLE, AND IT COST A WHOLE SESSION (2026-09-07).** DR slice 2c had
  been built, reviewed and finished on 09-06 and left un-PR'd when the editor restarted. HANDOVER
  and ROADMAP on that branch recorded it correctly and `main` knew nothing, so the next session
  verified the tracking documents against `main`, found them consistent, and re-designed and
  part-rebuilt a slice that already existed. **Checking the working tree and `main` is not checking
  the repository** — `gh pr list --state all`, then `git branch -a`, then `git log --all`.
- **⇒ A ROUND-TRIP TEST PROVES A CODEC IS SELF-CONSISTENT, NEVER THAT IT IS CORRECT (slice 2a).** A
  mutation audit found **19 of 19 single-line mutations surviving** — plane tags, magic,
  discriminants, chunk endianness, field order, record flag bits — because every test round-tripped
  through the same encoder/decoder pair. Golden bytes in `src/wire_pins.rs` killed 18/18 on re-run.
- **⇒ A SCANNER READS NAMES, NOT VALUES (#527).** `cairn-medium`'s fixture helpers
  `salted_record(salt, n)` / `chain_of(n, salt)` built nothing cryptographic and minted **eighteen
  critical CodeQL alerts at once**, while sibling helpers running identical arithmetic under the
  names `bytes(seed, …)` / `placeholder(seed, …)` were unflagged. Reserve `salt`/`nonce`/`iv` for
  real constructions; call a discriminator a `lineage`, a `variant`, a `seed`. Enforced by
  `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`.
- **⇒ A DEFERRAL IS ONLY HONEST WHILE ITS PRECONDITION HOLDS, and nothing watches for one expiring
  (slice 2b).** Seven comments across four crates asserted a deferral one slice had retired; #511
  then found two more inside `seal.rs` describing a coupling ADR-0066 had deleted eleven days
  earlier. **Grep, do not recall.**
- **⇒ `Secret32` DOES NOT SEPARATE ONE SECRET ROLE FROM ANOTHER (#511)** — see trap 5 above, which
  is the durable form of this and carries the pinned counts.
- **⇒ THE CLOSING-KEYWORD TRAP (2026-09-04).** GitHub reads `close`/`fix`/`resolve` **adjacent** to
  a reference and never the sentence around it, so seven sentences disclaiming a close performed
  one. Now guarded in CI; `fix(#500):` is safe because the parenthesis breaks the adjacency.

**⇒ THE OPEN ISSUES THOSE SESSIONS OPENED, INDEXED RATHER THAN NARRATED.** Condensing the prose
above deleted these once already, and 25 of them were in no other tracking document. They are kept
here as bare numbers on purpose: the rule is never to drop an **open** issue number while
condensing, and an index satisfies it where a paragraph does not.

[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) · [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327) · [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394) · [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402) · [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) · [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407) · [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408) · [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409) · [#413](https://github.com/cairn-ehr/cairn-ehr/issues/413) · [#420](https://github.com/cairn-ehr/cairn-ehr/issues/420) · [#422](https://github.com/cairn-ehr/cairn-ehr/issues/422) · [#428](https://github.com/cairn-ehr/cairn-ehr/issues/428) · [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430) · [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431) · [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447) · [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458) · [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463) · [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464) · [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470) · [#483](https://github.com/cairn-ehr/cairn-ehr/issues/483) · [#484](https://github.com/cairn-ehr/cairn-ehr/issues/484) · [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485) · [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487) · [#488](https://github.com/cairn-ehr/cairn-ehr/issues/488) · [#490](https://github.com/cairn-ehr/cairn-ehr/issues/490) · [#491](https://github.com/cairn-ehr/cairn-ehr/issues/491) · [#492](https://github.com/cairn-ehr/cairn-ehr/issues/492) · [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494) · [#504](https://github.com/cairn-ehr/cairn-ehr/issues/504) · [#505](https://github.com/cairn-ehr/cairn-ehr/issues/505) · [#506](https://github.com/cairn-ehr/cairn-ehr/issues/506) · [#507](https://github.com/cairn-ehr/cairn-ehr/issues/507) · [#508](https://github.com/cairn-ehr/cairn-ehr/issues/508) · [#509](https://github.com/cairn-ehr/cairn-ehr/issues/509) · [#513](https://github.com/cairn-ehr/cairn-ehr/issues/513) · [#518](https://github.com/cairn-ehr/cairn-ehr/issues/518) · [#521](https://github.com/cairn-ehr/cairn-ehr/issues/521) · [#522](https://github.com/cairn-ehr/cairn-ehr/issues/522) · [#529](https://github.com/cairn-ehr/cairn-ehr/issues/529) · [#530](https://github.com/cairn-ehr/cairn-ehr/issues/530) · [#543](https://github.com/cairn-ehr/cairn-ehr/issues/543) · [#545](https://github.com/cairn-ehr/cairn-ehr/issues/545) · [#557](https://github.com/cairn-ehr/cairn-ehr/issues/557) · [#558](https://github.com/cairn-ehr/cairn-ehr/issues/558) · [#559](https://github.com/cairn-ehr/cairn-ehr/issues/559) · [#560](https://github.com/cairn-ehr/cairn-ehr/issues/560) · [#561](https://github.com/cairn-ehr/cairn-ehr/issues/561)

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not. **`docs/spikes/`** —
0001 (walking skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ →
ADR-0029/0030); 0003 (Postgres on Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2).
**`docs/case-studies/0001`**: 16 GP-software failure modes, all absorbed, **0 new architecture**.
**`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance. Code workspace: `/crates`
(`cairn-event`, `cairn-keystore`, `cairn-medium`, **`cairn-wire`**, `cairn-sync`, `cairn-node`,
`cairn-medication-view`, `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate
workspace); `poc/` is frozen historical spikes.

---

## Where the build actually is (the live, in-progress state)

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS
  pinned to the trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate,
  genesis-stable `node_id`. Every honest gap declared at build time is closed **except the `localstate`
  clinical seams — custody travels (slice 1) and the clinical event log now REACHES the medium (slice 2c),
  and a restore reads one back (slice 2d, **#554**, ADR-0067) — what remains on this path is TEST
  DEBT, see ⇒ NEXT); optional escrow rungs (Shamir/QR/TPM) remain. **Dual-identifier
  discipline** (ADR-0031) — the canonical plane (UUIDv7 + multihash) is the only identifier on the
  wire/in signed bodies; the projection plane may intern node-local `bigint` surrogates (`db/008` +
  leakage guard).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx`, self-serializing via a Postgres advisory
  lock (`db::test_serial_guard`). **Not "cluster-wide" — advisory locks are scoped PER DATABASE** (#467;
  ~124 comments still say otherwise, **#476**), so every caller takes the guard against `CAIRN_TEST_PG`.
- **Tech-debt loop** — `/techdebt-loop` triages into `loop:*` labels, `/techdebt-next` runs one fresh
  headless session per issue. Auto-merge ENABLED; works unattended (12 PRs); STOPPED by maintainer
  decision (⇒ NEXT). Live gaps **#326**, **#312**, **#322**.

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **⇒ DR — #554 IS BUILT AND MERGED (slice 2d, PR #565/#566, 2026-09-09/10); so are the §1.2
  measurement (#512's time half, PR #573) and the non-interactive recovery code (#572/#570, PR
  #574). What remains on this path is the §7 TEST DEBT — see ⇒ NEXT.** The whole 2a→2d chain has
  landed: 2a the format (08-31), 2b the transport seam + paged pull (09-02), **#511** the custody
  newtypes (09-04), 2c the capture (09-06), 2d the read-back (09-09/10). **Two things a reader is
  otherwise led to expect and will not find:**
  1. **2d does NOT drive `cairn-sync`'s puller through `MediumTransport`.** `MediumTransport` is a
     *serving* abstraction (paging, label, logging latch) and `cairn-node` does not depend on
     `cairn-wire`. The pure `within(verified_through) → sort by source_seq` derivation was lifted
     into `cairn-medium` instead, which both already depend on — one implementation of 2a
     invariant 5, no new dependency edge.
  2. **The per-peer quarantine quota does not apply to a restore-originated pen**, and the pen is
     an in-DB door (`cairn_quarantine_event`, `db/052`) that `cairn-sync` and `cairn-node` share.
     The quota's `Err` says *"the watermark freezes instead"*, which needs a cursor and a
     re-serving peer; a restore has neither, so inheriting it would lose the record at exit.

  Open issues this chain filed, none closed by it: **#549**, **#551**, **#552**, **#525** (2a),
  **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module), **#531**/**#329** (decompose
  `cairn-sync/src/main.rs` — a maintainer decision is wanted on which to keep), **#532**,
  **#534**–**#538**, **#556**–**#563**, **#567**–**#572**, **#575**.
- **§5.9 parts C/D** (#232) — see ⇒ NEXT. Related: **#235** (shred authorization hooks), **#236** (FTS/RAG
  must build on `event_clear`).
- **`clinical.medication` — slices 1–6b DONE** (ADR-0059). Next: **drugref term→anchor lookup** (⇒
  NEXT); fuzzy/automatic reconciliation + a Tier-A dictionary; structured sig/frequency; correcting a
  dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/033`, `db/041`, `db/042`.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` +
  `cairn-event::demographics`). B3-driven: gold set, locale packs, hub-tier duplicate sweep, proposal
  retraction. Identity: C5+ `reattribute` (waits on a clinical-note surface); §5.12 push-alert.
  Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, the only one
  catching all three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite
  `-p <crate>` never builds): the `db/tests/*.sql` mirrors plus the full workspace with
  `CAIRN_TEST_PG`/`PG2`/`PG3` baked in (DBs `cairn_test`/`2`/`3`). **The CLUSTER is discovered, not
  assumed** — `scripts/pg-target.sh` finds a running PostgreSQL meeting the `db/001_envelope.sql` floor
  (>= 18), refuses one below it by NAME AND VERSION, and refuses to guess when two qualify; set `PGPORT`
  to name one. It used to default to `127.0.0.1:5532`, true of one machine in the world, and a bare
  `run-db-sql-tests.sh` reached whatever the libpq socket did — on this Mac a PG16 instance, which
  failed 22 migrations deep on `max(bytea)` and cost a wrong diagnosis (2026-09-07).
  **⇒ Cost depends on what changed, not target-dir warmth** (#503's 6-hour surprise): a cross-crate change
  relinks every test binary under macOS's one-time-per-binary Gatekeeper assessment — but #511 relinked
  the whole tree and still finished in well under an hour on a warm target, so **measure rather than
  budget from either figure**. A slice confined to one crate reruns almost nothing (`-p cairn-sync` DOES
  build the cross-crate `clinical_pull.rs`). Without the three env strings the DB-gated suites self-skip
  and cargo counts them as passed, so since #450 a bare run FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is
  declared. Mirrors are DESTRUCTIVE, refusing any DB lacking the `cairn_scratch_database` marker (#169).
  Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest` (uv, never pip; gap **#314**).
  `clinical_pull` used to flake under a full-workspace run — #457 fixed the diagnostic, not the cause;
  `--test-threads=2` is the workaround.
- **Clinical case-mining** — historically the highest-signal generative mode; the primitives have absorbed
  every case so far without new architecture. Bring a real ED/hospital failure mode; record in
  [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation** (order/recall/referral with no closing ack), a named
  projection surfaced by salience not a modal; **③ impossible-vs-uncertain** for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:**
- **Bet B — Pi compute-cost run** ([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)):
  PASS twice. Remaining: fold the un-caveated B4 number into ADR-0015 to drop "provisional"; **#272**
  (reproject bench on the Pi rig).
- **easyGP session** — port [ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s
  deferred items with live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source +
  forced-manual rule table; prefetch warming daemon). Pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`.
  GUI-mining continues, opens the results/inbox design session (three-zone vs two-pane parked there).
- **Byte-tier throughput lever** — connection reuse / persistent streaming instead of one TCP connection
  per slice (§8.2 availability + windowing/resume shipped).

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until
  momentum/funding geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
