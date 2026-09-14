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

> **⇒ #567 IS BUILT AND MERGED (2026-09-13/14, PR [#588](https://github.com/cairn-ehr/cairn-ehr/pull/588)).
> `verify-backup` NOW ASKS THE CLINICAL-PLANE QUESTION.**
> It fails **`backup SHORT` ONLY ON EVIDENCE** (maintainer decision): this node's own
> `backup-status.json` describes the `--from` path AND the medium is behind it on the newest trusted
> clinical seq OR the raw clinical record count (v2+ sidecars). Policy in
> `cairn-node/src/backup/clinical_verdict.rs`; ROADMAP has what it deliberately did not build and why.
> **Residuals:** **#551** (evidence is PATH-bound — a drive rotated through a DIFFERENT path stays
> green) · **#589** (cannot say a medium predates a crypto-shred) · **#590** (ADR-0068's
> unsigned-segment residual) · **#591** (a relative `--to` recorded verbatim) · **#592** (requires
> `--conn` though it never connects). ⚠️ **Operators: run `verify-backup` AFTER the nightly `backup`, not before** — a
> `verify-backup && backup` cron stops backing up after every same-mount-point rotation, because the
> drive that missed the latest backup reads SHORT until its own next backup catches it up.
>
> **⇒ WHAT IS NEXT ON THE DR PATH: TWO MAINTAINER DECISIONS, NO TEST DEBT.** Every one of slice 2d's
> 23 design tests is now written — the last six by **#593** (2026-09-14, see below). What is left is
> **#575** (the minted recovery code reaches stderr — three options in the issue; the maintainer
> deferred it on 2026-09-14) and **#594** (new — `restore` exits 0 when records past a chain break or
> in an unknown plane were not restored, while a pen exits non-zero). Nothing on the path is a slice
> any more. The other build candidates are further down.
>
> **The pen-release rule (#578, PR #582) — do not undo.** `db/020` returns `Ok` while admitting a
> sealed event WITHOUT custody on four paths, so **a pen row carrying a wrapped DEK is released only
> when custody for its event is SETTLED** — held, shredded, or plaintext (`cairn_custody_state`),
> stated in `crates/cairn-sync/src/requeue.rs` and enforced by `cairn_release_pen_row` (`db/052`);
> `pen_rows_leave_through_one_door.rs` fails if anything else deletes a pen row. `requeue` exits **3
> (INCOMPLETE)** when rows stay held or a chart needs `cairn-node reproject` (trap 9, **#584**); its
> no-key message warns that `establish-unwrap-key` forecloses the real key on a restored node.
> `do_requeue` skips `acked` rows, which `do_pull` does NOT (argued in `requeue.rs`). **#585**:
> nothing reads Postgres notices. Fixture facts: `sync_quarantine.refused_seq` is **NOT NULL**, and
> `cairn_quarantine_event` returns **`acked`**, not "was it penned".
>
> **ALL 23 OF SLICE 2d'S §7 DESIGN TESTS ARE WRITTEN.** Test 4 by #568, test 23 by PR #574 (this
> file listed it as owed until 2026-09-14 — it was not), and **7, 14, 16, 17, 19 and 22 by #593**,
> each proven by a named mutation that turns it red. Where they live: `restore_pen_is_uncapped.rs`
> (7, at 10 001 records), `restore_one_event_id_one_body.rs` (16), `restore_cli_applies_nothing_untrusted.rs`
> (14, 17), `restore_cli_survives_its_own_failure.rs` (19, 22); shared fixtures in
> `tests/common/restore_kit.rs`. **A retry after a crashed restore must move the `<key>.unwrap` that
> attempt installed aside first** — the pre-flight refuses otherwise, and says so; test 19 follows
> that remedy.
>
> **⇒ `M > N` STILL STANDS AND #512 STAYS OPEN.** The third act is the **recovery code**, a second
> secret asked for after the node plane is applied (ADR-0068 deleted the provenance confirmation
> once blamed). ADR-0069 swaps it for a file read in a drill; it does not change the count.
> **⇒ #552 IS CONFIRMED:** a re-capture of a 100 003-event medium with ZERO new events cost 9.95 s
> against 15.0 s for the full one — two thirds of a nightly capture is independent of what is new;
> a 2 s budget is crossed near **14 000** events. **⇒ "2e" IS RETIRED** — what was under it is
> **#551** and **#553** (an unmarked foreign legacy medium can be destroyed by succession).
>
> **Still broken, all named rather than assumed away:** **#549** (a burned identity `seq` is
> indistinguishable from a lost clinical event; the `seq_gaps` operator surface is re-deferred) ·
> **#552** (a capture is O(whole medium), and **read-side peak memory is unbudgeted** — a Pi or
> Android node is a legitimate restore target, so streaming stays deferred) · **#536** (an unopenable
> DEK is counted on the RESTORE path only; the sync half is open) · **#569** (db/052's registry door
> silently discards a **content** conflict and leaves `actor_event_id`/`seq` unvalidated) · **#502
> item 4** (a discarded keystore-load reason) · **#101 items 2–3** · **#512** · **#575** (the
> minted recovery code still reaches stderr on both paths) · **#583** (a DB-gated suite can
> depend on global state a predecessor left, and only the hours-long full local gate can see it) ·
> **#584** (new — late custody never re-projects; recovery owes a `cairn-node reproject`, reported by
> one run only) · **#585** (new — no caller reads Postgres WARNING notices) · **#586** (new — two
> source guards stop scanning at a file's first test module) · **#587** (new — `cairn-sync`
> pull/requeue on a sync-only DB loaded by an older build fail with a raw 42883, not "run init") ·
> **#589**, **#590**, **#591**, **#592** (the #567 residuals named in ⇒ NEXT above) · **#594** (new —
> `restore`'s exit code for records it could not restore; a decision, see ⇒ NEXT) ·
> **#556**–**#563** (the 2b/2c review wave; see ROADMAP).
>
> **Never cite ADR-0026 decision 1's promise 2** — *"node-default data-at-rest keys survive"* — as
> met by any of this. It has **no subject at all**: no node-default key tier exists, so it is
> neither honoured nor violated, and ADR-0067 says so in as many words.

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS (measured 2026-09-12), KEPT THAT WAY BY A MODEL PACK — AND ONE HUMAN
> ACT IS DUE: make `CodeQL (rust)` a required check (#444).** PR **#576** replaced default setup
> with a committed workflow + model pack (`.github/codeql/packs/cairn/codeql-models`, one
> `barrierModel` row per NAME-heuristic source, each with its reason; 44 → 3, the 3 dismissed). The
> flip needed the **organization-level** configuration as well as the repository's. **Read the alert
> list with `scripts/codeql-alerts.sh`, never assume it**; CONTRIBUTING has the local reproduction.

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
> **Nine traps. Each is a step a next session takes in good faith.** (Five came from slice 1; trap 5 was minted by #511, trap 7 by DR slice 2c, trap 8 by #578, trap 9 by the #582 review.)
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
>    **Its decision record is [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md)
>    decision 2** (*"Erasure does not propagate backwards into media already written, and completing
>    it across backups is rotation"*), and `docs/spec/security.md`'s *Erasure survives DR* bullet
>    states it for readers. (This trap said until 2026-09-14 that the sentence was UNWRITTEN in any
>    decision record — false since ADR-0067 landed, found while #567 retired the last "slice 2e"
>    references. What is still unbuilt is the operator-facing half: **#589**.)
> 8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED. DO NOT RELEASE IT AS A
>    CLEANUP (#578, 2026-09-12).** `do_requeue` keeps every pen row carrying a `dek_wrapped` whose
>    custody did not land — including one whose key simply will not open here. It looks like a row
>    the pen will hold forever for nothing, and releasing it looks like tidying. It is not:
>    *"did not open with the key we have right now"* is **not** *"not ours"*. The operator may hold
>    the right `<key>.unwrap` on a USB stick they have not plugged in, which is the #495 shape this
>    whole DR path exists to survive, and the pen row is the only copy of that key. The escape is
>    `db/021`'s `acked` — *"a recorded human decision, never an automatic one"* — which `do_requeue`
>    now honours; a human decides the key is unrecoverable, never the code. (A row whose custody is
>    later settled by other means — a peer's pull lands it — releases on the next requeue.) Pinned by
>    `crates/cairn-sync/tests/requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
>    (**inverted** from the arm that used to assert the opposite) and by
>    `requeue_retains_unlanded_custody.rs`. **The narrow fix that looks equivalent and is not:**
>    keying the guard on the opened `dek` rather than on the pen row's own `dek_wrapped` passes the
>    headline test and skips the check whenever the key did not open. `cairn_release_pen_row` now
>    catches that in the database too — do not read the floor as licence to drop the Rust check,
>    which is what tells the operator WHY.
> 9. **⇒ "THE BODY OPENS" IS NOT "THE CHART HAS IT". DO NOT DROP `reproject_owed` OR EXIT 3 AS
>    NOISE (#584, 2026-09-13).** When a pen row's key lands on an event that was already admitted
>    without it, `event_clear.twin` reads back and `medication_statement` stays empty, because the
>    projection trigger is `AFTER INSERT` and a re-apply inserts nothing. `requeue` says so and exits
>    3, naming `cairn-node reproject` (owner connection, heal mode). It looks like a spurious failure
>    on a run whose every row released. It is the only thing between an operator and a medication
>    list that silently omits a recovered record. Pinned by
>    `requeue_retains_unlanded_custody.rs::an_unregistered_unwrap_key_keeps_the_pen_row_and_the_fix_reaches_the_chart`,
>    which asserts the chart is 0 before the heal and 1 after. If #584 makes the door re-project,
>    retire the heal step there — the test's own premise assertion will say when. **`restore` reaches
>    the same state WITH NO SIGNAL AT ALL** (reproduced 2026-09-14, recorded on #584): two copies of
>    one event at one `source_seq`, the keyless copy first — body opens, chart empty, exit 0.

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

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 time budget is a seeded figure, not a
measured one** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy; only the *write* half is measured (median 222 ms, **PARTIAL**), Slice 63 owes both halves for
registration (≤5s find, ≤20s register), write-cost half **#360** unwired, and db/044's `gesture_kind` CHECK refuses a
registration row until widened; (2) **the accessibility pass** — a live VoiceOver run through the runbook's eight
checks, keyboard-only (`cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`), DOM
assertions automated by **#332**; (3) **make CI jobs REQUIRED status checks** (**#444**, admin-only — "clippy + cargo
test (cairn-gui)", "cargo doc (API surface)"), matching job names exactly, per `CONTRIBUTING.md`'s dated table; (4)
**making `CodeQL (rust)` a required check** (**#444**) — PR #576 has merged, so this is DUE; see
⇒ NEXT. **If a measurement falls outside its budget, that is the finding — file an
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

**Session date:** 2026-09-14 (**#593 — slice 2d's last six design tests (7, 14, 16, 17, 19, 22), each proven by a named mutation.** Test-only: four new suites, the restore fixtures moved to `tests/common/restore_kit.rs`, no production code touched. Eight mutations run, all killed. Filed **#594** (restore's exit code for records it did not restore); reproduced a second entrance to trap 9 through `restore` and recorded it on **#584**. Maintainer deferred **#575**. **No ADR, no spec bump, no migration, no SCHEMA bump.** PR **#595**.) · before that: 2026-09-13/14 (**#567 — `verify-backup` asks the clinical-plane question**; `backup SHORT` only on evidence; opened #589–#592; PR #588 merged) · 2026-09-13 (**the PR #582 review** — `cairn_custody_state` + `cairn_release_pen_row`, exit 3 = INCOMPLETE, `reproject_owed`; opened #584–#587) · 2026-09-12 (**`requeue` never counts a release it did not get**, closed #578–#581, full local gate GREEN, opened #583; **`requeue` releases custody**, closed #568, PR #577; **CodeQL advanced setup + model pack**, 44 → 3, PR #576) · 2026-09-11 (**non-interactive recovery code**, closes #572/#570, **ADR-0069**, spec **v0.71**, opens #575, PR #574) · 2026-09-10 (**§1.2 budget measured** — 116.7 s against 600 s — plus **ADR-0068**; closed #571, opened #572, confirmed #552; and **DR slice 2d**, closed #554, **ADR-0067**, `db/052`) · 2026-09-07 (**#523**, and 2c found un-PR'd) · 2026-09-06 (**DR slice 2c**) · 2026-09-04 (**closing-keyword guard**; **#511**) · 2026-09-02 (**DR slice 2b**; **#527**) · 2026-09-01/08-31 (**DR slice 2a**) · 2026-08-30 (**#503**) · 2026-08-24 (**DR slice 1**: #495 CLOSED). Earlier: see *Recent sessions* below. · **Spec/ADRs:** **v0.71** ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md); [ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **52** (`db/052`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

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

### 2026-09-14 — #593: slice 2d's last six design tests, each proven by a mutation

Plan `docs/superpowers/plans/2026-09-14-dr-2d-design-tests-593.md`. PR #595. **No production code
changed.** What generalises:

- **⇒ THIS FILE'S LIST OF OWED TESTS WAS WRONG BY ONE.** It said seven; test 23 had been written by
  PR #574 three days earlier. #593 was scoped from a grep of the tree, not from here. **Grep, do not
  recall** — this file's own rule, broken by this file.
- **⇒ A PIN OVER SHIPPED BEHAVIOUR PASSES ON ITS FIRST RUN, SO THE MUTATION IS THE RED PHASE.** All six
  passed first time; eight mutations were run and every one killed — but one had to be MOVED before it
  proved anything. "The pen's bail moved above the summary", placed above the WHOLE summary, failed
  test 22 at its reason-line assertion, not at the next-step lines it was meant to prove. **A mutation
  must fail the test at the assertion that names its claim — read the panic line, not just the red.**
- **⇒ WITHOUT db/020's SUBSTITUTION GUARD, NOTHING ELSE CATCHES A KEYLESS FORGERY.** With the guard
  removed, a rival body under an existing `event_id` carrying no DEK was reported `applied`: the door's
  `ON CONFLICT DO NOTHING` swallows it and the restore's newness probe counts it new. (A rival WITH a DEK
  would surface as `CustodyDidNotLand`.) The guard is load-bearing on its own for that shape.
- **⇒ TRAP 9 HAS A SECOND ENTRANCE, THROUGH `restore`** (reproduced with a throwaway probe, recorded on
  #584): two copies of one event at one `source_seq`, the keyless copy first → the body opens,
  `medication_statement` is EMPTY, the report is identical to the correct order, exit 0.
- **⇒ A RE-RESTORE AFTER A CRASH NEEDS ONE `mv` THE CRASH MESSAGE DOES NOT MENTION.** The failed attempt
  installed `<key>.unwrap`; the retry's pre-flight refuses to run over it and names the remedy, so the
  operator learns it one invocation late. Minor; not filed.
- **Exit codes are #594**: records past a chain break or in an unknown plane leave `restore` at exit 0
  while a pen exits non-zero. Tests 14/17b do not assert exit status, so either decision lands clean.
- **Fault injection without residue:** a `cairn_test_*` trigger scoped to one `event_id`, dropped BEFORE
  asserting and at test start (#583's reset-at-start rule); `pg_trigger`/`pg_proc` checked clean after.
- **Tooling:** rust-analyzer's clippy pass held `target/` for 10 minutes on one narrow build — a scratch
  `CARGO_TARGET_DIR` fixed it for the session. zsh does not word-split `$T`: write `cargo test ${=T}`.
- **Gate this time:** the 4 new suites, the refactored `restore_cli_surface`, 7 restore/verify
  neighbours and 14 source guards that scan test files — 26 binaries, all exit 0 against PG18 — plus
  `cargo fmt --check` and clippy `-D warnings` on the touched targets. **No full local sweep** (test-only
  change); CI's full job is the gate.

### 2026-09-12 → 09-13/14 — requeue custody, the CodeQL model pack, `verify-backup`'s clinical plane (condensed)

PRs #576, #577, #582 and #588, all merged; each plan in `docs/superpowers/plans/` carries its review
ledger. What still generalises:

- **⇒ READ A COMMAND'S EXISTING REFUSALS BEFORE ADDING A WARNING TO IT (#567).** The "untrusted
  records" warning asked for could never fire — `verify-backup` already refuses any medium that is not
  `sound()`. The invariant was pinned instead of writing dead code.
- **⇒ AN ISSUE'S SCOPE AND A DESIGN'S "WHY NOT X" ARE CLAIMS.** #567's gap notice would have cried wolf
  on every federating node (#549), and its "match restore's leniency" inverted the rule that the health
  check is STRICTER. The design rejected record counts as "collapsing duplicates" — false; the final
  review built the back-fill-below-the-newest-seq case and the rule gained a count axis. Three task
  reviews had checked code against spec, never spec against code.
- **⇒ REUSED OPERATOR TEXT CAN BE FALSE IN ITS NEW HOME, AND A REMEDY IN A MESSAGE IS CODE.**
  `restore`'s "were applied", printed by `verify-backup`, which applies nothing; `requeue`'s no-key line
  named `establish-unwrap-key`, which forecloses the real key on a restored node. Read a command's own
  warnings before printing it as a fix. And read stdout and stderr APART — a cron log that keeps only
  stdout had logged `records OK` for a SHORT medium.
- **⇒ A DOOR RETURNING `Ok` IS NOT THE RECORD COMING BACK, AND "THE BODY OPENS" IS NOT "THE CHART HAS
  IT"** (#578, the #582 review). db/020's lenient arms warn and return normally, and nothing reads
  Postgres notices (#585); when custody can arrive late, assert the projection, not only `event_clear`
  (#584). With the rule in the database the narrow mutation stopped losing the key — and exposed that
  a FALSE from the floor was being reported as "the row vanished". Read a refusal back as a refusal.
- **⇒ A GUARD NEEDS A POSITIVE CONTROL THAT IT SEES THE CODE IT GUARDS.** A new source guard skipped
  ~96% of `main.rs` after the first `#[cfg(test)] mod`, a habit copied from two older guards (#586).
  Review the review's fixes: a second pass over the fix diff alone found five more defects.
- **⇒ A NEW DEFINER FUNCTION COPIES ITS `SET` CLAUSE FROM A NEIGHBOUR — WRITE `public, pg_temp`.**
  `search_path_pg_temp.rs` (#426) caught `SET search_path = pg_catalog, public`, which let any caller
  shadow a table with a TEMP decoy and dictate whether `requeue` deletes the last copy of a key. An
  existing `db/*.sql` is the cheap home for a shared predicate (replayed on every connect); a NEW file
  forces `SCHEMA_GENERATION` up and relinks the whole tree.
- **⇒ A RED GATE CAN BELONG TO A PREDECESSOR (#583).** `restore_cli_surface` leaves a `local_node` row
  and an adjacent suite read it as its own fixture's fault; fixed at the consumer, reset-at-start.
  **`cairn_test` is never recreated between Rust sweeps — truncate `local_node` before trusting a red
  after a killed gate.**
- **⇒ CodeQL (#576): `rust/cleartext-logging`'s sources are NAME heuristics on calls, variables and
  fields**, and #562's triage had the wrong shape. One `barrierModel` row per function RETURN
  (`ReturnValue` is the CALL node; a barrier cannot be narrower than the taint; read the SARIF
  `codeFlows` first; the binary crate root is `cairn-node::`, with its hyphen). The in-repo pack reaches
  `database run-queries` through `CODEQL_ACTION_EXTRA_OPTIONS`; local reproduction is three minutes
  (`gh codeql`, CONTRIBUTING).
- **⇒ A SUBAGENT THAT ENDS ITS TURN WAITING ON A BACKGROUND JOB NEVER WAKES, AND A PLAN'S CODE BLOCKS ARE
  CHECKED BY NOTHING UNTIL A GATE RUNS.** Brief every dispatch "foreground only"; put `cargo fmt --check`
  and the `-D warnings` doc build in every task's commit step.
- **Gates then:** #582's full local sweep was GREEN (164 binaries / 2025 tests); #588's was stopped by a
  3 h 45 m Gatekeeper stall and CI's full job was its gate.

### 2026-09-10 — DR slice 2d, and the restore's budget measured (condensed)

**Slice 2d closed #554** (ADR-0067, spec v0.69, `db/052`, SCHEMA 52). **The budget session closed
#571** (ADR-0068, spec v0.70), measured #512's time half, opened #572 and confirmed #552 (PR #573).
Filed by the chain and still open: **#569** (db/052's registry door silently discards a **content**
conflict and leaves `actor_event_id`/`seq` unvalidated). Since closed: #567 (built, PR #588), #568,
#570 (PR #574), #571. What still generalises:

- **⇒ THE HEADLINE TEST DECRYPTS A BODY.** The apply door wraps the DEK it is handed; piping an
  already-wrapped key through would double-wrap every key in the record while every row count agreed
  and `verify-backup` stayed green. **A test that counted rows would have shipped it.**
- **⇒ A DESIGN SENTENCE WITH TWO READINGS AND NO TEST SURVIVED A MERGE.** 2d's "clinical segments
  inherit the node plane's `Provenance` treatment" read as both a printed warning and a *"safety
  gate"*; ADR-0067 recorded neither, and design test 18 was never written. ADR-0068 settled it.
- **⇒ CHECK THE ERRATA RULE BEFORE HONOURING THE ASK.** `decisions/README.md` allows an erratum only
  under a passage that is false about the code; decision-shaped content takes a new ADR.
- **⇒ A MEASUREMENT'S RESULT IS ITS SHAPE.** 116.7 s against 600 s, but the useful fact is linear at
  1.17 ms/event with no bend — a ceiling near 510 000 events. The rig refuses to time an incomplete
  restore (one that applies nothing is fast, and the piped-recovery-code bug, #572, produced exactly
  that), and a duplicated count field was deleted rather than reconciled.
- **The seeder goes through the production orchestrators** (85% born-sealed, human-authored), or the
  expensive unwrap/re-wrap half never runs; the node's own `device` actor is enrolled by one real
  `patient-register` rather than a re-spelled private helper.

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
- **⇒ DR — the whole 2a→2d chain has landed, the §1.2 measurement (#512's time half) and the
  non-interactive recovery code (#572/#570) too, and since #593 all 23 design tests are written.
  What remains is two decisions (**#575**, **#594**) — see ⇒ NEXT. Two things a reader is led to
  expect and will not find: **2d does NOT drive `cairn-sync`'s puller through `MediumTransport`** (a
  serving abstraction; the pure `within(verified_through) → sort by source_seq` derivation lives in
  `cairn-medium` instead), and **the per-peer quarantine quota does not apply to a restore-originated
  pen** (its "watermark freezes instead" promise needs a re-serving peer; pinned at volume by
  `restore_pen_is_uncapped.rs`). Open issues the chain filed: **#549**, **#551**, **#552**, **#525**,
  **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module), **#531**/**#329** (decompose
  `cairn-sync/src/main.rs` — a maintainer decision on which to keep), **#532**, **#534**, **#535**,
  **#536**, **#537**, **#538**, **#556**–**#563**, **#569**, **#575**, **#589**, **#590**, **#591**, **#592**, **#594**.
  (Ranges silently absorb issues that leave them: re-check each against GitHub before trusting it.)
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
