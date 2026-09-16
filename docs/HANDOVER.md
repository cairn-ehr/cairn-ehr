# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ #584 IS BUILT: A KEY THAT ARRIVES AFTER ITS EVENT NOW REACHES THE CHART** (2026-09-15/16,
> [ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md), spec **v0.72**, PR
> [#601](https://github.com/cairn-ehr/cairn-ehr/pull/601)). **If PR #601 has not merged, none of this
> is on `main`** — check before trusting this paragraph. When a write door newly writes `event_clear`
> for an event already in the log, it runs that event's heal-safe appliers once, through one helper
> (`cairn_project_late_custody`, db/005) whose dispatch loop db/043's gate 4 shares. So `pull --full`
> is ONE step, a `requeue` that lands every key exits 0 with the chart populated (`reproject_owed` and
> its exit-3 cause are retired), and a `restore` whose keyless copy sorts first reaches the chart. No migration file;
> `SCHEMA_GENERATION` stays **52**. Guarded by trap 10. **A database already missing such a record is
> not healed by upgrading** — `cairn-node reproject` still heals it (pre-clinical: none exists).
>
> **⇒ WHAT IS NEXT ON THE DR PATH: ONE DECIDED ITEM TO BUILD, ONE DEFERRED DECISION, A NEW DECISION
> ISSUE, TWO FILED CROSS-TRANSACTION RACES, AND NAMED OPERATIONAL GAPS.**
>
> - **#605–#609 — NEW, from PR #601's own review round (all filed, none built):** **#605** an
>   in-place edit of a `db/` function is unprotected by the #188 downgrade guard, so an older binary
>   at the SAME generation can replace the new floor and nothing reports it · **#606** the two
>   medication conflict-flag tables have no product reader · **#607** `requeue` lands a DEFERRED
>   event's key and says nothing about the chart gate 4 still owes · **#608** both substitution
>   guards fail open on a NULL comparison (`<>`, unreachable today) · **#609** the two late-custody
>   helpers fall outside `floor_execute_grants.rs` · **#610** a custody-reading applier that slips
>   past the CI-time catalogue guard fails silently at runtime (trap 10's residual, now tracked).
>
> - **#594 — DECIDED 2026-09-15, NOT BUILT, and the natural next DR item:** `restore` exits **3**
>   whenever any medium record was not restored — chain break, unknown plane AND torn tail — after
>   the full summary. Today records past a break or in an unknown plane leave exit **0** while a pen
>   exits non-zero. (Tests 14/17b assert no exit status, so they land clean.)
> - **#575 — still an OPEN decision, re-deferred by the maintainer:** the minted recovery code still
>   reaches stderr on both restore paths (the #527/#562 callout below).
> - **#602 — NEW, a decision issue:** any client can set `cairn.remote_apply` before calling
>   `submit_event`, turning the strict door's refusals into the lenient door's flags. Pre-existing — a
>   principle-12 weakness found in #584's review; ADR-0070 neither widens nor narrows it.
> - **#603 and #604 — NEW, two cross-transaction races from #584's final review (reasoned, not
>   reproduced; ADR-0070 names both as residuals, its decision 3 holding on every sequential path):**
>   **#603** — a late key racing connect-time re-adjudication of a deferred event can leave the
>   promoted record off the chart, and `requeue` then exits 0 · **#604** — a shred racing a late key
>   can resurrect custody, and since ADR-0070 the projection too (step 9's anti-resurrection check takes
>   no lock; pre-existing, widened). Each fix is a locking decision with a deadlock shape to test first.
> - **Operational gaps (PR #595's review residuals):** **#596** (a crashed restore says "restore
>   again", but the retry is refused until the leftover `<key>.unwrap` is moved) · **#597** (the
>   straddled-duplicate notice says "All of them were applied" before anything was) · **#598**
>   (`restore_reads_the_clinical_plane.rs` still has private fixtures) · **#599** (the §6.2 disk-cost
>   note and a CAIRNB1 medium have no CLI test). Nothing on the path is a slice any more; the other
>   build candidates are further down.
>
> **The DR path itself is closed and rehearsable.** A solo clinic can lose its disk, restore from the
> medium plus its export, **open a chart**, and rehearse that without a human at the terminal: the KEY
> (#495, ADR-0066), the BYTES' write half (#500, slice 2c), the READ half (#554, slice 2d,
> [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md)), the §1.2 measurement
> (#512's time half, PR #573) and the non-interactive path
> ([ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md), PR #574). A
> record a restore is entitled to apply but cannot is **quarantined with its custody** and the restore
> exits non-zero; one it is NOT entitled to apply (past a mid-file chain break, in a plane this build
> cannot route) stays on the medium and is named in the summary. The actor registry re-enters on the
> export container's AEAD alone — the one part of a restore that is **not** verify-on-apply, accepted
> deliberately and printed to the operator. **Rows and custody coming back is not a body opening**:
> cite `restore_reads_the_clinical_plane.rs` and
> `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`, both of which decrypt a
> real sealed body — never the row counts in `dr_clinical_guarantee_gap.rs`. **The measurement stands
> and is not re-run:** 100 003 events in **116.7 s against 600 s**, linear at **1.17 ms/event** with no
> bend, a ceiling near **510 000 events** on an M3 Max (`scripts/measure_dr_restore.py`,
> `crates/cairn-node/results/2026-09-10-macos-m3max.md`).

> [!IMPORTANT]
> **⇒ #527/#562's TRIAGE NOTE IS FALSE** — *"no cron-run command reaches `print_recovery_code`"* was
> **already untrue before ADR-0069 (2026-09-11)**: a medium with **no local-state export sibling** never
> reaches the recovery-code prompt, so a sealed `restore` of one has always run unattended and printed
> a fresh code to stderr (every other read in the restore arm is print-only — ADR-0068's ruling). The
> real fix — a `--new-recovery-code-file` sink, or refusing to mint a sealed key when nothing can show
> its code to a human — is **[#575](https://github.com/cairn-ehr/cairn-ehr/issues/575)**.

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
> **The pen-release rule (#578, PR #582) — do not undo.** `db/020` returns `Ok` while admitting a
> sealed event WITHOUT custody on four paths, so **a pen row carrying a wrapped DEK is released only
> when custody for its event is SETTLED** — held, shredded, or plaintext (`cairn_custody_state`),
> stated in `crates/cairn-sync/src/requeue.rs` and enforced by `cairn_release_pen_row` (`db/052`);
> `pen_rows_leave_through_one_door.rs` fails if anything else deletes a pen row. `requeue` exits **3
> (INCOMPLETE)** only when rows stay held in the pen or are still refused by the door — since
> ADR-0070 a key it lands reaches the chart in the same run. Its no-key message warns that
> `establish-unwrap-key` forecloses the real key on a restored node. `do_requeue` skips `acked` rows,
> which `do_pull` does NOT (argued in `requeue.rs`). **#585**: nothing reads Postgres notices. Fixture
> facts: `sync_quarantine.refused_seq` is **NOT NULL**, and `cairn_quarantine_event` returns
> **`acked`**, not "was it penned".
>
> **All 23 of slice 2d's §7 design tests are written** (test 4 by #568, 23 by PR #574, and 7, 14, 16,
> 17, 19, 22 by #593, each proven by a named mutation; shared fixtures in
> `tests/common/restore_kit.rs`). **A retry after a crashed restore must move the `<key>.unwrap` that
> attempt installed aside first** — the pre-flight refuses otherwise and says so, the crash message
> does not (**#596**); test 19 pins the refusal and follows its remedy.
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
> item 4** (a discarded keystore-load reason) · **#101 items 2–3** · **#512** · **#583** (a DB-gated
> suite can depend on global state a predecessor left, and only the hours-long full local gate can see
> it) · **#585** (no caller reads Postgres WARNING notices) · **#586** (two source guards stop scanning
> at a file's first test module) · **#587** (`cairn-sync` pull/requeue on a sync-only DB loaded by an
> older build fail with a raw 42883, not "run init") · **#556**–**#563** (the 2b/2c review wave; see
> ROADMAP) — plus every item in ⇒ NEXT and #567's residuals above.
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

> [!IMPORTANT]
> **Ten traps. Each is a step a next session takes in good faith.** (Five came from slice 1; trap 5
> was minted by #511, trap 7 by DR slice 2c, trap 8 by #578, trap 9 by the #582 review — **retired by
> #584 and kept as history** — and trap 10 by #584.)
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
>    IT (DR slice 2c, 2026-09-06).** It is the definition of a backup: *"a backup is only a backup if it
>    can restore the state of the system at the time the backup was taken. Taking care of invalidated
>    backups is a policy issue, not a core enforcement one. The core will only guarantee availability
>    and integrity of data"* (maintainer). A medium that dropped the key later would report a state the
>    node was never in, and could only do so by **rewriting a segment it has already signed**. **Never
>    filter old segments.** (The mirror half is equally deliberate: a body shredded BEFORE its first
>    capture never has its DEK written, while its ciphertext still travels.) Pinned, and named so nobody
>    repairs it, by `crates/cairn-node/tests/medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`.
>    Completing an erasure across backups is **rotation** — capture fresh, destroy old — and that
>    interval is the clinic's policy call, not Cairn's (principle 9): decided in
>    [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) decision 2 and stated in
>    `docs/spec/security.md`'s *Erasure survives DR*. Still unbuilt: the operator-facing half, **#589**.
> 8. **⇒ A PEN ROW WHOSE DEK "BELONGS TO ANOTHER NODE" IS STILL RETAINED. DO NOT RELEASE IT AS A
>    CLEANUP (#578, 2026-09-12).** `do_requeue` keeps every pen row carrying a `dek_wrapped` whose
>    custody did not land, including one whose key will not open here: *"did not open with the key we
>    have right now"* is **not** *"not ours"*. The operator may hold the right `<key>.unwrap` on a USB
>    stick not yet plugged in — the #495 shape this whole DR path exists to survive — and the pen row is
>    the only copy of that key. The escape is `db/021`'s `acked`, *"a recorded human decision, never an
>    automatic one"*. (A row whose custody is later settled — a peer's pull lands it — releases on the
>    next requeue.) Pinned by
>    `crates/cairn-sync/tests/requeue_releases_custody.rs::a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise`
>    (**inverted**) and by `requeue_retains_unlanded_custody.rs`. **The narrow fix that looks equivalent
>    and is not:** keying the guard on the opened `dek` rather than on the row's own `dek_wrapped` passes
>    the headline test and skips the check whenever the key did not open. `cairn_release_pen_row`
>    catches that in the database too — do not read the floor as licence to drop the Rust check, which
>    is what tells the operator WHY.
> 9. **RETIRED — HISTORY, NOT ADVICE (#584 built, ADR-0070, 2026-09-15/16).** It read *"the body opens
>    is not the chart has it — do not drop `reproject_owed` or exit 3"*: a key landing on an event
>    already admitted without it opened the body and left `medication_statement` empty, because the
>    projection trigger is `AFTER INSERT` and a re-apply inserts nothing; `requeue` reported it by one
>    run only and `restore` not at all. **The door now projects a late key, and `reproject_owed` is
>    gone. Do not remove the `cairn_project_late_custody` calls or move them:** in both doors they sit
>    AFTER the substitution guard (a rival body must never reach an applier). **BOTH doors' placements
>    are test-pinned** — `late_custody_reaches_the_chart.rs::a_rival_body_never_reaches_an_applier`
>    (db/020, mutation M6) and `::the_strict_door_refuses_a_rival_body_before_any_applier_runs`
>    (db/005; the strict twin was PR #601's own review finding, and killed the same mutation there).
>    Each carries its OWN positive control, so a raising probe that quietly stopped being registered
>    cannot leave them passing vacuously. The strict door's POSTURE is pinned too, by
>    `::a_contradiction_revealed_by_a_late_key_is_refused_at_the_strict_door`: wrapping db/005's call
>    in `cairn.remote_apply = 'on'` "to match db/020" turns a strict refusal into a flag, and now
>    fails. And in `db/020` the call sits BEFORE the
>    `cairn.remote_apply` clear (after it, three projection guards RAISE and the key could never land).
>    Its pins now assert the heal: `restore_one_event_id_one_body.rs::a_keyless_copy_first_still_reaches_the_chart`,
>    and `requeue_retains_unlanded_custody.rs` arm 1, which asserts the chart right after release and
>    exit 0. (#597's notice is still misleading; it no longer hides an empty chart.)
> 10. **⇒ WHEN `late_custody_guards.rs` FIRES, THE GUARD IS RIGHT (#584, 2026-09-15/16).** Two rules
>     over the catalogue (`pg_proc`, what actually runs), each the reason a late key can never leave a
>     debt on a sequential path (#603/#604 are the concurrent exceptions): **every PL/pgSQL or SQL
>     function that `INSERT`s into `event_clear` calls `cairn_project_late_custody`** — the
>     writer set is pinned by name, today exactly `apply_remote_event` and `submit_event` (the shred's
>     `DELETE FROM event_clear` is outside the rule: removing custody owes no projection); and **every
>     registered applier that reads custody** (`cairn_clear_payload` or `event_clear` in its own body)
>     **is `heal_safe = TRUE`**. **The tempting wrong fixes:** registering a custody-reading applier
>     `heal_safe = false` to stop a late landing re-running it (the door then skips it and the chart
>     silently owes a rebuild — the debt `reproject_owed` used to report, and nothing reports it now:
>     **#610**, which also lists how a custody read can slip past this CI-time text match);
>     flipping one to `TRUE` that is not idempotent just to turn the guard green; exempting a writer.
>     Make the applier idempotent; give a new writer the call — a third writer is a DECISION.
>     Residuals: a custody read hidden in a helper the applier calls is invisible to the guard, and a
>     `MERGE INTO event_clear` or a dynamic `EXECUTE format(...)` write is not recognised (none exists;
>     review a new one by hand).

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) is four subsystems: parts A and B
(authority floor + operator surface) are BUILT, enforcing nothing beyond display/emission; C+D are DESIGNED and C1 is
the next §5.9 BUILD — #500's slice 2d, which outranked it, has merged.** Read **ADR-0062/0063/0064/0065** (`spec/decisions/`) before
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
assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks** (**#444**, admin-only — "clippy +
cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`, now DUE — the CodeQL callout above), matching
job names exactly, per `CONTRIBUTING.md`'s dated table. **If a measurement falls outside its budget, that is the
finding — file an issue, never adjust the budget.**

**Other build candidates** (#500 is done; nothing blocks a choice): the **registration/search UI slice**
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
> IDE. **Do not read a warm target as a time estimate** — see the test-env bullet under *Open threads*.

---

**Session date:** 2026-09-15/16 (**#584 built — a late key reaches the chart.** **ADR-0070**, spec **v0.72**, no migration file, `SCHEMA_GENERATION` still 52; ten mutations run, M6 killed in the final fix wave by a raising probe; filed **#603** and **#604**, two cross-transaction races ADR-0070 names as residuals; the maintainer's decisions on **#594** (exit 3, not built) and **#575** (re-deferred) recorded; filed **#602**; PR **#601**) · 2026-09-15 (**PR #595's review round** — the kit's wipe truncates `node_unwrap_key` and the medication projections, test 7 crosses the byte cap, five more mutations killed; filed **#596–#599**; **#600**, the `rustls` → 0.23.45 lockfile bump) · 2026-09-14 (**#593** — slice 2d's last six design tests, test-only, eight mutations killed; filed #594; PR #595) · earlier, one line each: 09-13/14 **#567** (`verify-backup`'s clinical plane, PR #588; opened #589–#592) · 09-13 **the PR #582 review** (opened #584–#587) · 09-12 **requeue custody** (PRs #577, #582; opened #583) and **the CodeQL model pack** (PR #576) · 09-11 **ADR-0069** (PR #574; opened #575) · 09-10 **DR slice 2d** + **ADR-0067/0068** · 09-07 → 08-24 **DR slices 1, 2a–2c**, **#503**, **#511**, **#527**, the closing-keyword guard. Detail: *Recent sessions* below and ROADMAP. · **Spec/ADRs:** **v0.72** ([ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md); [ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md); [ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **52** (`db/052`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

**Built so far** — orientation only; ROADMAP + the ADR log + git carry the detail. **Demographics slices
1–5** (§4.4 identifiers · §4.2 DOB/sex-at-birth · names · administrative-sex/gender-identity · §4.3
address) · **the §5.2 advisory Python matcher** · **the §5.7 identity core C1–C5** (C5+ `reattribute`
waits on a clinical-note surface) · **the §5.4 John-Doe subsystem** (§5.12 push-alert open) · **the
§5.3/§5.8 search-before-create funnel** (ADR-0061; precedence rule #345 at db/005 step 8b) ·
**`clinical.medication` slices 1–6b** (ADR-0047/0048/0049/0050/0051/0059) under **born-sealed bodies**
(ADR-0052) and **per-write human authorship** (ADR-0053 — grading half-live until #245) · **the §5.9
stream complete through its read surface**, enforcing nothing beyond display/emission · **the med-list node
tier** (first clinical READ path + whole-list sign-off), **generic reprojection** (ADR-0057; a late key
projects at the door, ADR-0070), the **ADR-0056 admit-uninterpreted floor** and the **residual refusal
contract** · **the L3 reference UI** —
`cairn-gui/`, a standalone workspace, one-way GUI → crates; the iced shell FAILED the accessibility bar
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`** on one patient's medication chart (plain
JS, no npm), pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-09-15/16 — #584: a late key reaches the chart

Design `docs/superpowers/specs/2026-09-15-late-custody-reaches-the-chart-584-design.md`; plan
`docs/superpowers/plans/2026-09-15-late-custody-reaches-the-chart-584.md` (its review ledger and the
M1–M10 mutation table); [ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md); PR #601. The
durable rules are traps 9 (retired) and 10. What generalises past the slice:

- **⇒ THE PAPER-PARITY PLAN GUARD WANTS ITS LITERAL LABELS.** `paper_parity_plan_section.rs` requires
  **"Paper counterpart"**, **"Steps"** and **"Time + cognitive load"** inside the §1.2 section. The plan
  had the section and its content under other labels ("Architecture-forced, before → after", "Time
  budget") and it failed — stopping the whole fail-fast gate at 2 m 20 s. Copy the labels, not the idea.
- **⇒ DELETING A STATEMENT CAN SILENTLY MOVE WHICH LAYER A FAULT-INJECTION TEST HITS.** `requeue` arm 7
  locked `event_dek` to make its custody read fail. Once the pre-door read was deleted, that lock was met
  by the apply door's own write instead, so the arm stopped exercising the path its name claims — and
  no test said so; a review did. Re-targeted to a test-scoped `cairn_custody_state` that RAISEs `55P03`
  (the door never calls it; the schema replay restores the real one). **Whenever you delete or reorder
  a statement, re-check every fault-injection test that relied on it.**
- **⇒ AN ADR IS IMMUTABLE, SO CHECK IT SENTENCE BY SENTENCE AGAINST THE CODE BEFORE MERGE.** That check
  found two false sentences: "any writer of `event_clear`" (the rule and its guard are about `INSERT` —
  the shred DELETEs) and "permanent" deferred markers (db/043 retries a failed re-adjudication every
  pass). **One came from the design spec itself**, so checking the ADR against the design would have
  passed it.
- **⇒ A BACKGROUND WRAPPER `cmd; echo exit=$?` REPORTS THE ECHO'S EXIT STATUS TO THE HARNESS.** The
  notification said exit 0 over a run whose log said `exit=101`. Read the logged exit, never the
  notification.
- **⇒ A MUTATION OF STATEMENT ORDER INSIDE ONE TRANSACTION LOOKS UNOBSERVABLE UNTIL A PROBE RAISES.** M6
  (the late-custody call moved above the substitution guard) survived Task 7: no medication applier
  raises on a rival body, and the guard's RAISE rolls back whatever the appliers wrote. The final fix
  wave killed it cheaply — a test-scoped heal-safe applier that RAISEs (`install_raising_probe`), so an
  applier running first replaces "substitution refused" with the probe's message
  (`a_rival_body_never_reaches_an_applier`). **Before recording a survivor as "unobservable", ask
  whether a raising probe observes it.**
- **Gate:** a full workspace run at code-complete (173 binaries, 2110 passed) failed only
  `paper_parity_plan_section` — the labels above, fixed in the plan. What re-ran after that is in PR #601.

### 2026-09-14/15 — #593 and PR #595's review: slice 2d's last six design tests (condensed)

Plan `docs/superpowers/plans/2026-09-14-dr-2d-design-tests-593.md`; PR #595; test-only. What generalises:

- **⇒ THIS FILE'S LIST OF OWED TESTS WAS WRONG BY ONE** (test 23 had been written three days earlier).
  #593 was scoped from a grep of the tree, not from here. **Grep, do not recall.**
- **⇒ A PIN OVER SHIPPED BEHAVIOUR PASSES ON ITS FIRST RUN, SO THE MUTATION IS THE RED PHASE — AND IT
  MUST FAIL AT THE ASSERTION THAT NAMES ITS CLAIM.** One mutation, placed above the whole summary, failed
  test 22 at its reason line rather than at the next-step lines it was meant to prove. Read the panic line.
- **⇒ WITHOUT db/020's SUBSTITUTION GUARD, NOTHING ELSE CATCHES A KEYLESS FORGERY:** the door's `ON
  CONFLICT DO NOTHING` swallows a rival body under an existing `event_id`, and the restore counts it
  `applied`.
- **⇒ A FIXTURE THAT MODELS "A FRESH MACHINE" MUST BE CHECKED TABLE BY TABLE.** The shared wipe left
  `node_unwrap_key` registered (no FK reaches it, so `CASCADE` did not); with a restore whose key
  registration was made a no-op, the headline CLI test and all of test 16 still PASSED. #598 tracks the
  older suite with the same gap.
- **⇒ A COUNT ASSERTION CAN BE VACUOUS BY FIXTURE SIZE.** Test 7's 10 001 small records crossed the row
  cap and never the 64 MiB byte cap. And a fixture numbering records in capture order cannot tell "past
  the break" from "above the watermark" (test 14 needed a low-seq record past the break).
- **Fault injection without residue:** a `cairn_test_*` trigger scoped to one `event_id`, dropped BEFORE
  asserting and at test start (#583's reset-at-start rule); `pg_trigger`/`pg_proc` checked clean after.
- **Tooling:** rust-analyzer held `target/` for 10 minutes on one narrow build — use a scratch
  `CARGO_TARGET_DIR`. zsh does not word-split `$T`: write `cargo test ${=T}`.

### 2026-09-12 → 09-13/14 — requeue custody, the CodeQL model pack, `verify-backup`'s clinical plane (condensed)

PRs #576, #577, #582 and #588, all merged; each plan in `docs/superpowers/plans/` carries its review
ledger. What still generalises:

- **⇒ READ A COMMAND'S EXISTING REFUSALS BEFORE ADDING A WARNING TO IT (#567).** The "untrusted
  records" warning asked for could never fire — `verify-backup` already refuses a medium that is not
  `sound()`. The invariant was pinned instead of writing dead code.
- **⇒ AN ISSUE'S SCOPE AND A DESIGN'S "WHY NOT X" ARE CLAIMS.** #567's gap notice would have cried wolf
  on every federating node (#549); the design's "counts collapse duplicates" was false, and the rule
  gained a count axis in the final review. Three task reviews had checked code against spec, never spec
  against code.
- **⇒ REUSED OPERATOR TEXT CAN BE FALSE IN ITS NEW HOME, AND A REMEDY IN A MESSAGE IS CODE.**
  `restore`'s "were applied" printed by `verify-backup`, which applies nothing; `requeue`'s no-key line
  naming `establish-unwrap-key`, which forecloses the real key on a restored node. Read stdout and
  stderr APART — a stdout-only cron log had logged `records OK` for a SHORT medium.
- **⇒ A DOOR RETURNING `Ok` IS NOT THE RECORD COMING BACK** (#578, the #582 review). db/020's lenient
  arms warn and return normally, and nothing reads Postgres notices (#585). Assert the projection, not
  only `event_clear` — that is how #584 was found. Read a refusal back as a refusal.
- **⇒ A GUARD NEEDS A POSITIVE CONTROL THAT IT SEES THE CODE IT GUARDS.** A new source guard skipped
  ~96% of `main.rs` after the first `#[cfg(test)] mod`, a habit copied from two older guards (#586).
  Review the review's fixes: a second pass over the fix diff alone found five more defects.
- **⇒ A NEW DEFINER FUNCTION COPIES ITS `SET` CLAUSE FROM A NEIGHBOUR — WRITE `public, pg_temp`**
  (`search_path_pg_temp.rs`, #426). An existing `db/*.sql` is the cheap home for a shared predicate
  (replayed on every connect); a NEW file forces `SCHEMA_GENERATION` up and relinks the whole tree.
- **⇒ A RED GATE CAN BELONG TO A PREDECESSOR (#583).** **`cairn_test` is never recreated between Rust
  sweeps — truncate `local_node` before trusting a red after a killed gate.**
- **⇒ CodeQL (#576): `rust/cleartext-logging`'s sources are NAME heuristics.** One `barrierModel` row
  per function RETURN (`ReturnValue` is the CALL node; read the SARIF `codeFlows` first; the binary
  crate root is `cairn-node::`, with its hyphen); local reproduction via `gh codeql`, CONTRIBUTING.
- **⇒ A SUBAGENT THAT ENDS ITS TURN WAITING ON A BACKGROUND JOB NEVER WAKES, AND A PLAN'S CODE BLOCKS ARE
  CHECKED BY NOTHING UNTIL A GATE RUNS.** Brief every dispatch "foreground only"; put `cargo fmt --check`
  and the `-D warnings` doc build in every task's commit step.

### 2026-09-10 — DR slice 2d, and the restore's budget measured (condensed)

Slice 2d (ADR-0067, `db/052`) and the budget session (ADR-0068, PR #573, confirming #552). Still open
from that chain: **#569** (db/052's registry door silently discards a **content** conflict and leaves
`actor_event_id`/`seq` unvalidated). What still generalises:

- **⇒ THE HEADLINE TEST DECRYPTS A BODY.** The apply door wraps the DEK it is handed; piping an
  already-wrapped key through would double-wrap every key while every row count agreed and
  `verify-backup` stayed green. **A test that counted rows would have shipped it.**
- **⇒ A DESIGN SENTENCE WITH TWO READINGS AND NO TEST SURVIVED A MERGE** (2d's `Provenance` sentence;
  ADR-0068 settled it). And check `decisions/README.md`'s errata rule before honouring an ask: an
  erratum only under a passage false about the code; decision-shaped content takes a new ADR.
- **⇒ A MEASUREMENT'S RESULT IS ITS SHAPE** — linear with no bend is what makes a ceiling predictable.
  The rig refuses to time an incomplete restore (one that applies nothing is fast), and its seeder goes
  through the production orchestrators, or the expensive unwrap/re-wrap half never runs.

### 2026-09-07 → 08-20 — the sessions before slice 2d (condensed to what generalises)

The per-slice narrative is **ROADMAP's**, and every issue number these sessions opened lives there.
What a next session still needs from them:

- **⇒ AN UNPUSHED BRANCH IS INVISIBLE, AND IT COST A WHOLE SESSION (2026-09-07).** DR slice 2c had
  been built and reviewed on 09-06 and left un-PR'd when the editor restarted; the next session
  checked the tracking documents against `main`, found them consistent, and part-rebuilt a slice that
  already existed. **Checking the working tree and `main` is not checking the repository** — `gh pr
  list --state all`, then `git branch -a`, then `git log --all` (house rule 8).
- **⇒ A ROUND-TRIP TEST PROVES A CODEC IS SELF-CONSISTENT, NEVER THAT IT IS CORRECT (slice 2a).** A
  mutation audit found **19 of 19 single-line mutations surviving** because every test round-tripped
  through the same encoder/decoder pair; golden bytes in `src/wire_pins.rs` killed 18/18 on re-run.
- **⇒ A SCANNER READS NAMES, NOT VALUES (#527)** — house rule 6(b), enforced by
  `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`.
- **⇒ A DEFERRAL IS ONLY HONEST WHILE ITS PRECONDITION HOLDS, and nothing watches for one expiring
  (slice 2b).** `localstate.rs`'s header declared its seam truthfully and ADR-0052 made it false while
  ROADMAP kept recording ✓; seven more comments across four crates had the shape, and #511 found two
  inside `seal.rs`. **Before trusting any ✓, check the sentence that justified it. Grep, do not recall.**

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
  and a restore reads one back (slice 2d, **#554**, ADR-0067) — what remains on this path is one
  decided item, open decisions and operational gaps, see ⇒ NEXT); optional escrow rungs
  (Shamir/QR/TPM) remain. **Dual-identifier
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
  non-interactive recovery code (#572/#570) too; since #593 all 23 design tests are written, and #584
  (a late key reaches the chart, PR #601) is built. What remains: **#594** (decided, not built — the
  next DR item), **#575** (a deferred decision), **#602** (a new decision issue), **#603**/**#604**
  (two cross-transaction races ADR-0070 names as residuals) and the operational
  gaps **#596**–**#599** — see ⇒ NEXT. Two things a reader is led to
  expect and will not find: **2d does NOT drive `cairn-sync`'s puller through `MediumTransport`** (a
  serving abstraction; the pure `within(verified_through) → sort by source_seq` derivation lives in
  `cairn-medium` instead), and **the per-peer quarantine quota does not apply to a restore-originated
  pen** (its "watermark freezes instead" promise needs a re-serving peer; pinned at volume by
  `restore_pen_is_uncapped.rs`). Open issues the chain filed: **#549**, **#551**, **#552**, **#525**,
  **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module), **#531**/**#329** (decompose
  `cairn-sync/src/main.rs` — a maintainer decision on which to keep), **#532**, **#534**, **#535**,
  **#536**, **#537**, **#538**, **#556**–**#563**, **#569**, **#575**, **#589**, **#590**, **#591**, **#592**,
  **#594**, **#602**, **#603**, **#604**.
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
  to name one (a bare run once reached a PG16 socket and cost a wrong diagnosis).
  **⇒ Cost depends on what changed, not target-dir warmth** — #503 took six hours, #511 relinked the
  whole tree in under one, and #584's full workspace run took minutes; **measure, never budget from a
  figure here**. `-p cairn-sync` DOES build the cross-crate `clinical_pull.rs`. Without the three env strings the DB-gated suites self-skip
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
