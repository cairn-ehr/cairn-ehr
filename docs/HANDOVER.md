# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ #619 IS BUILT: THE NODE PLANE REFUSES A SUBSTITUTION AT BOTH LIVE DOORS, AND PENS IT**
> (2026-09-19, [ADR-0073](spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md),
> spec **v0.75**, PR **[#623](https://github.com/cairn-ehr/cairn-ehr/pull/623)**; no migration,
> `SCHEMA_GENERATION` still **53**). **If #623 has not merged, none of this is on `main` — and #619 is
> closed BY HAND once it merges** (the closing-keyword guard stops a PR body doing it).
> `submit_node_event` and `apply_remote_node_event` — the live federation admission gate — each call
> `cairn_refuse_substitution` once, in a shared tail after the `IF/ELSE` (db/009's shape; the gate's
> three clock merges fold into one). The node puller asks the TABLE whether a refused event's
> `event_id` is already held under a different content address and, if so, **pens it** whichever
> check refused it; routine scoping refusals keep skip-and-advance. The door inventory is now a
> `pg_proc` catalogue rule. ADR-0072 gained **Errata E1–E2**. 11/11 mutations killed. Guarded by
> **trap 13**. **Maintainer rulings:** pen (not skip, not all of #268); one tail per door. **Filed from
> its review:** **#620** (the COSE unprotected header is hashed into the content address but lies
> outside the signature, so a relay can re-wrap an event into a different address — wire core, both
> planes, a DECISION) · **#621** (db/007 raises NON-P0001 codes deterministically on
> verifiable-but-malformed events — a uuid cast before any trust check, the HLC CHECK — so the node
> pull freezes that peer PERMANENTLY, with no pen/ack remedy; #228's class) · **#622** (the catalogue
> guards cannot see a `BEGIN ATOMIC` body).
>
> **⇒ WHAT IS NEXT: NO DECIDED-AND-UNBUILT DR ITEM REMAINS.** Pick from below, or leave DR for the
> *Other build candidates*. **Recommended: #621** — a live, network-reachable wedge (any trusted peer
> serving a stranger-signed event with a non-UUID `event_id` freezes that link forever), small, and
> #228 already shows the fix pattern. Then **#620**, a wire-contract decision.
>
> - **Open decisions (none a patch):** **#575** (the minted recovery code still reaches stderr on both
>   restore paths — re-deferred once) · **#602** (any client can set `cairn.remote_apply` and turn the
>   strict door's refusals into flags — a principle-12 weakness) · **#611** (a scripted restore missing
>   `--old-recovery-code-file`: exit 1 or usage error 2? The message is a raw errno either way) ·
>   **#613** (should INCOMPLETE widen from *records left behind* to *recovery left short*?) · **#620**.
> - **Restore residuals:** **#616** (a `finalize_identity` failure destroys the whole summary) ·
>   **#617** (the duplicate `registry_present` probe's error reaches the operator naked) ·
>   **#596**–**#599** (a crash message's remedy the pre-flight refuses; "All of them were applied"
>   before anything was; private fixtures; no CLI test for the §6.2 note or a CAIRNB1 medium) — **a
>   truthful exit code does not make a false sentence true.** Node-plane completeness accounting still
>   does not exist.
> - **Two cross-transaction races** (reasoned, not reproduced; ADR-0070's residuals): **#603** (a late
>   key racing connect-time re-adjudication) · **#604** (a shred racing a late key can resurrect
>   custody, and the projection too). Each fix is a locking decision with a deadlock shape to test first.
> - **PR #601's review wave:** **#605** (an in-place `db/` function edit is unprotected by the #188
>   downgrade guard — #619 is exposed to it too) · **#606** (two medication conflict-flag tables have no
>   reader) · **#607** (`requeue` lands a deferred event's key and says nothing about the chart) ·
>   **#608** (its `cairn_project_late_custody` half) · **#609** · **#610**.
> - **The node plane's divergences:** **#268** (ADR-0073 carved ONE class — substitution — out of it;
>   the rest is a refusal-class partition decision) · **#301** (an unknown node event type fails
>   closed) — both `loop:needs-human`. **#569** is the actor registry's own silent content-conflict
>   discard: the shape #619 closed, one table over.
>
> **The DR path is closed and rehearsable, newest first:** #619 (ADR-0073), #614+#615 (ADR-0072 — one
> shared substitution refusal; a deferred clinical record REPORTED at exit 0), #594 (ADR-0071 —
> `restore` exits **3 INCOMPLETE** for five causes; **1 means the ceremony was BLOCKED**), #584
> (ADR-0070 — a late key reaches the chart; `cairn-node reproject` still heals a database already
> missing one), the non-interactive recovery code (ADR-0069), the §1.2 measurement (PR #573), slice 2d
> (ADR-0067/0068), slice 2c, and the key (ADR-0066). A solo clinic can lose its disk, restore from the
> medium plus its export, **open a chart**, and rehearse it unattended. A record the restore is
> entitled to apply but cannot is **quarantined with its custody**; one it is not entitled to apply
> stays on the medium and is named — all exit 3. The actor registry re-enters on the export's AEAD
> alone (accepted, printed). **Rows and custody coming back is not a body opening**: cite
> `restore_reads_the_clinical_plane.rs` and
> `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`, never the row counts
> in `dr_clinical_guarantee_gap.rs`. **The measurement stands and is not re-run:** 100 003 events in
> **116.7 s against 600 s**, linear at **1.17 ms/event**, a ceiling near **510 000 events** on an M3
> Max (`scripts/measure_dr_restore.py`, `crates/cairn-node/results/2026-09-10-macos-m3max.md`).

> [!IMPORTANT]
> **⇒ #527/#562's TRIAGE NOTE IS FALSE** — *"no cron-run command reaches `print_recovery_code`"* was
> already untrue **before** ADR-0069: a medium with no local-state export sibling never reaches the
> recovery-code prompt, so a sealed `restore` of one has always run unattended and printed a fresh
> code to stderr. The real fix is **[#575](https://github.com/cairn-ehr/cairn-ehr/issues/575)** — a
> `--new-recovery-code-file` sink, or refusing to mint a sealed key nothing can show to a human.

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
> ADR-0070 a key it lands reaches the chart in the same run. **Since ADR-0071 `restore` speaks the
> same 3**, and the two constants are held equal by a test (trap 11). Its no-key message warns that
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
> **Thirteen traps. Each is a step a next session takes in good faith.** (Five came from slice 1;
> trap 5 was minted by #511, trap 7 by DR slice 2c, trap 8 by #578, trap 9 by the #582 review —
> **retired by #584 and kept as history** — trap 10 by #584, trap 11 by #594, trap 12 by #615 and
> trap 13 by #619.)
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
>     is not the chart has it — do not drop `reproject_owed` or exit 3"*: a key landing on an event
>     already admitted without it opened the body and left `medication_statement` empty, because the
>     projection trigger is `AFTER INSERT` and a re-apply inserts nothing. **The door now projects a
>     late key, and `reproject_owed` is gone.** What survives as live advice: **do not remove the
>     `cairn_project_late_custody` calls or move them.** In BOTH doors they sit AFTER the substitution
>     guard (a rival body must never reach an applier) and **both placements are test-pinned** —
>     `late_custody_reaches_the_chart.rs::a_rival_body_never_reaches_an_applier` (db/020, mutation M6)
>     and `::the_strict_door_refuses_a_rival_body_before_any_applier_runs` (db/005), each with its OWN
>     positive control so a raising probe that stopped being registered cannot leave them passing
>     vacuously. The strict door's POSTURE is pinned too
>     (`::a_contradiction_revealed_by_a_late_key_is_refused_at_the_strict_door`): wrapping db/005's
>     call in `cairn.remote_apply = 'on'` "to match db/020" turns a strict refusal into a flag, and now
>     fails. In `db/020` the call sits BEFORE the `cairn.remote_apply` clear — after it, three
>     projection guards RAISE and the key could never land. The heal is asserted by
>     `restore_one_event_id_one_body.rs::a_keyless_copy_first_still_reaches_the_chart` and
>     `requeue_retains_unlanded_custody.rs` arm 1. (#597's notice is still misleading; it no longer
>     hides an empty chart.)
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
> 11. **⇒ `restore` EXITS 3 FOR AN INCOMPLETE RECOVERY, AND A TEST EXPECTING 1 IS THE BUG (#594,
>     ADR-0071, 2026-09-16).** Five states share exit **3**: a torn tail, records past a mid-file
>     chain break, records in an unroutable plane, records penned, and no actor registry. **Exit 1
>     means only that the ceremony was BLOCKED**, and is checked FIRST — a run that never opened its
>     local-state bundle also pens everything sealed, so reading the verdict first would send an
>     operator to `requeue` instead of to their recovery code. **The three tempting wrong fixes:**
>     (a) "restoring a torn medium succeeded, so assert `success()`" — that was 2c round 2's pin and
>     ADR-0071 reversed exactly it; *not refusing* and *reporting success* are different claims, and
>     ADR-0068 decision 1 is about REFUSING, which nothing here does. (b) Making a WRONG recovery code
>     exit 1 because it "looks like a failure" — `apply_local_state_export` returns `Ok(None)` for it
>     on purpose, and its end state is the no-registry state, which is 3. What IS exit 1 is a prompt
>     that cannot be *asked* (no flag, no tty). (c) "Simplifying" `past_chain_break` to the medium's
>     clinical total — a CLEAN restore then exits 3 (mutation M8 catches it).
>     ⚠️ **The precedence has exactly ONE test**:
>     `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody`, the only
>     scenario carrying both verdicts at once. If it is ever weakened to `!success()`, the order
>     becomes unpinned — and since round 3 it also asserts that the run HAS an incomplete cause
>     ("NO actor registry"), because a `Some(1)` alone is compatible with a clean restore and would
>     have pinned nothing after any fixture simplification. `restore_exit_vocabulary.rs` pins the
>     VALUE 3 and `cairn_sync::requeue::tests::exit_incomplete_matches_cairn_nodes_restore` pins that
>     the two binaries AGREE — neither alone is sufficient (both could be changed to 7 together).
>     ⚠️ **`!status.success()` IS NO LONGER AN ASSERTION.** Before #594 it meant "failed"; now 3 is
>     non-zero too, so it passes under either verdict. Two tests silently lost their teeth this way
>     and now assert `Some(1)` — a mid-apply database fault and the pre-flight leftover-key refusal,
>     both ADR-named exit-1 causes that had nothing pinning their number. **Write `Some(n)`.**
>     ⚠️ **An ACKED pen row is counted in `penned` and `requeue` will NOT clear it** (`is_incomplete`
>     excludes `skipped_acked`), so an all-acked pen reports 3 while requeue exits 0: the drill cannot
>     go green. Counting them is right; the verdict's TEXT names the exception. Do not "fix" it by
>     subtracting acked rows — that lets a restore claim a completeness no human granted.
>     Residuals: **#611**, the FAILED path's message is a raw errno; **#616**, not every exit 1 is
>     this precedence (a DB fault is a `?` far above the verdict site, and it takes the summary with
>     it). (#614/#615, the two states that reached exit 0 having left a record behind, are BUILT —
>     ADR-0072.)
> 12. **⇒ THE SUBSTITUTION REFUSAL HAS ONE HOME, AND A DOOR THAT NEEDS IT *CALLS* IT (#615/#608,
>     ADR-0072, 2026-09-17).** `cairn_refuse_substitution` (db/053) is the only place in `db/` that
>     raises *"already exists with different content (substitution refused)"*, and
>     `substitution_guard_is_single_source.rs` fails on a second. **A fourth inline copy is the
>     #608 shape returning:** one invariant written twice (db/005, db/020) had already become
>     wrong in BOTH places at once — they compared with `<>`, which yields NULL and does not fire
>     when the read-back finds no row — and the third door, `restore_node_event`, had no guard at
>     all. It compares **`IS DISTINCT FROM`**; "cannot tell what is stored" is a refusal.
>     **What a door MAY still do is READ however it likes, and the two shapes differ on purpose:**
>     db/005 and db/020 read under their `GET DIAGNOSTICS` check because they are on the
>     100k-event clinical path; **db/009 reads unconditionally and must NOT be "tidied" into a
>     `ROW_COUNT` check** — that would be correct only while the INSERT stays the last statement of
>     both its branches, and a later edit inside either one would disarm it SILENTLY (the failure
>     db/020's own comment warns about). ⚠️ **Its guard sits AFTER the `IF/ELSE`, never before:**
>     above the branch `v_found` is always NULL, so a CLEAN restore refuses (mutation M7 catches
>     it). ⚠️ **A db/009 refusal aborts the WHOLE restore** — `apply_medium` propagates with `?` —
>     and that is correct: that door already aborts on four lesser things, and the node plane is
>     the TRUST SET, so the silently-dropped rival can be the clinic's own `peer.revoked`.
>     ⚠️ **A DEFERRED clinical record is REPORTED, never a sixth `Unrestored` cause** (ADR-0072
>     decision 3): it IS in the log, which is what exit 0 claims, and an upgrade heals it with
>     nothing left on the medium. Making it exit 3 calls the most cheaply-repaired outcome in the
>     vocabulary a failed recovery.
>     ⚠️ **SINCE #619 IT GUARDS ALL FIVE EVENT-LOG DOORS** (db/005, db/020, db/009, and db/007's
>     two — trap 13). `substitution_guard_is_single_source.rs` still proves only that nobody
>     DUPLICATES the sentence; the INVENTORY is derived from `pg_proc` by
>     `substitution_guard_covers_every_writer.rs`, which replaced the hand-written list the census
>     gap hid behind. Residuals: **#608**'s `cairn_project_late_custody` half, **#605**, **#569** (the
>     actor registry's door has the same silent-discard shape and is outside the rule), **#622**.
> 13. **⇒ THE NODE PLANE'S SUBSTITUTION REFUSAL LIVES IN EACH DOOR'S TAIL, AND THE PULLER ASKS THE
>     TABLE, NOT THE ERROR (#619, ADR-0073, 2026-09-19).** `submit_node_event` and
>     `apply_remote_node_event` (db/007) each call `cairn_refuse_substitution` ONCE, after the
>     `IF/ELSE`, with an unconditional read — trap 12's db/009 rules apply verbatim (never above the
>     branch, never a `ROW_COUNT` check). **An arm that falls through inherits the guard; an arm that
>     `RETURN`s early bypasses it** — `submit_node_event`'s genesis arm does, safely only because it has
>     no `ON CONFLICT`. A new arm written in its image WITH an `ON CONFLICT` bypasses the guard, and
>     `substitution_guard_covers_every_writer.rs` will NOT notice (it asks whether a function CALLS the
>     helper, not whether every INSERT path reaches the call). **The tempting wrong fixes on the pull
>     path:** (a) giving the refusal its own SQLSTATE so the puller can "see" it — P0001 is a contract
>     (the comment above `cairn_decode_hex_or_raise` in db/001, #228; db/048 for `cairn-sync`), and a
>     non-P0001 turns `cairn-sync`'s clinical pen into a freeze; (b) matching the door's sentence;
>     (c) penning only when the GUARD raised — a rival refused by an earlier check (an untrusted author)
>     can never apply either, pinned by `a_rival_refused_by_an_earlier_check_is_still_penned`;
>     (d) turning the failed-lookup FREEZE into a skip — `node_substitution_lookup_freezes.rs` kills it
>     (M10, a `SET ROLE` without SELECT on `node_event`); (e) computing `offered` over the whole frame,
>     seq prefix included — every held-and-equal re-offer would then be penned, #268's alarm fatigue
>     (M11). **A sixth event-log writer fails the catalogue rule: give it the call and add it to the
>     pinned list — never the reverse.** Residuals: **#620**, **#621**, **#622**, **#605**.

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
an unmappable type where the clinical door admits it uninterpreted (**#301**), and the node puller
skips-and-advances a verifiable refusal where the clinical one pens it (**#268**; ADR-0073 carved out
the substitution class); neither is a symmetric fix, both `loop:needs-human`.

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

**Session date:** 2026-09-19 (**#619 built — the node plane refuses a substitution at both live doors, and pens it.** **ADR-0073**, spec **v0.75**, no migration, `SCHEMA_GENERATION` still **53**; db/007's two doors get db/009's single-tail guard; the node puller classifies by STATE and pens; the door inventory becomes a `pg_proc` catalogue rule; ADR-0072 gains Errata E1–E2; 11/11 mutations killed after the harness caught its own unrevertable M9; the whole-branch review found the M10 "no seam" residual false (a `SET ROLE` seam killed it) and a wire-core finding; filed **#620–#622**; closed **#614** by hand; subagent-driven, seven tasks each spec- and quality-reviewed; PR **[#623](https://github.com/cairn-ehr/cairn-ehr/pull/623)**) · 2026-09-17 (**#614 + #615** — ADR-0072, spec v0.74, db/053, `SCHEMA_GENERATION` 52 → 53; filed #619; PR #618) · 2026-09-16 (**#594** — ADR-0071, exit 3 INCOMPLETE; filed #611, #613, #614–#617; PR #612) · 2026-09-15/16 (**#584** — ADR-0070; filed #602–#604; PR #601) · earlier, one line each: 09-15 **PR #595's review** (filed #596–#599; #600) · 09-14 **#593** (PR #595) · 09-13/14 **#567** (PR #588; opened #589–#592) · 09-13 **the PR #582 review** (opened #584–#587) · 09-12 **requeue custody** (PRs #577, #582; opened #583) and **the CodeQL model pack** (PR #576) · 09-11 **ADR-0069** (PR #574; opened #575) · 09-10 **DR slice 2d** + **ADR-0067/0068** · 09-07 → 08-24 **DR slices 1, 2a–2c**, **#503**, **#511**, **#527**, the closing-keyword guard. Detail: *Recent sessions* below and ROADMAP. · **Spec/ADRs:** **v0.75** ([ADR-0073](spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md), which amends ADR-0072's census; [ADR-0072](spec/decisions/0072-a-restore-loses-no-record-silently.md), now carrying Errata E1–E2; [ADR-0071](spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md); [ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md); [ADR-0069](spec/decisions/0069-the-restore-takes-its-recovery-code-from-a-file.md); [ADR-0068](spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md), refining 0067; [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md), which supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **53** (`db/053`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 med-list window.

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

### 2026-09-19 — #619: the node plane refuses a substitution at both live doors, and pens it

Design `docs/superpowers/specs/2026-09-19-node-plane-substitution-guard-619-design.md`; plan
`docs/superpowers/plans/2026-09-19-node-plane-substitution-guard-619.md` (its M1–M11 ledger);
[ADR-0073](spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md). The durable rule
is trap 13. What generalises past the slice:

- **⇒ A RESIDUAL RESTS ON A PREMISE — CHECK THE PREMISE BEFORE IT ENTERS AN IMMUTABLE ADR.** M10 (the
  failed-lookup freeze) was declared an unkillable survivor because "no fault-injection seam exists in
  the self-pull". The whole-branch review found one: `pull_into` is `pub` and takes the caller's
  client, and `SET ROLE` to a role without SELECT on `node_event` makes the lookup fail with 42501.
  The ruling had been made without trying. **"Untestable" is a claim; try the seam first.**
- **⇒ A MUTATION ANCHOR MUST BE UNIQUE IN BOTH DIRECTIONS.** M9 replaced three lines with a bare
  `RETURN v_eid;`, which db/007 already contained twice, so the REVERT anchor was ambiguous and the
  harness stopped with the mutation applied (#594's defect, caught this time). The harness now refuses,
  before touching the file, a mutation whose replacement text already occurs there, and an unknown id
  (a typo used to shrink a run silently and still print "tree is clean").
- **⇒ AN ISSUE'S FAILURE SCENARIO IS A CLAIM.** #619 said a compromised peer could make node A "keep
  trusting C". It cannot — `trust_peer` reads only events A itself authored. The real costs (silent
  divergence; a dropped rival genesis wedging that peer's key) are what ADR-0073 states. Check the
  scenario against the code before it becomes the ADR's motivation.
- **⇒ CITE A CONTRACT WHERE IT IS WRITTEN, NOT WHERE YOU REMEMBER IT.** "db/001's header makes P0001 a
  contract" had propagated into the design, the ADR draft, `substitution.rs` and `cairn-sync`'s
  `main.rs`; the contract is the comment above `cairn_decode_hex_or_raise` (#228), and db/048 states
  the clinical door's half. One misattribution copied four times — #608's lesson, in prose.
- **⇒ CONTENT-ADDRESSING OVER UNSIGNED BYTES IS NOT CONTENT-ADDRESSING.** The COSE unprotected header
  is hashed into the content address but lies outside the signature, so a relay can re-wrap an event
  into a different address (#620). Read #620 before reasoning "same address ⇔ same signed event".
- **Process (subagent-driven, seven tasks):** each task got a spec + quality review, and four needed a
  fix round — a plan step that omitted `cargo fmt`, the harness twice, and two false ADR sentences
  (a brief's false "Amends" line; an overclaim, "no future arm can forget the guard", that an early
  `RETURN` disproves). **Review the brief as hard as the code: two of the defects were in the plan.**

### 2026-09-17 — #614 + #615: a restore loses no record silently (condensed)

Plan `docs/superpowers/plans/2026-09-17-restore-loses-no-record-silently-614-615.md`;
[ADR-0072](spec/decisions/0072-a-restore-loses-no-record-silently.md) (now with Errata E1–E2). The
durable rule is trap 12. What still generalises:

- **⇒ THE OBVIOUS FIX FOR A MISSING GUARD IS TO COPY THE GUARD, AND THAT IS HOW A KNOWN FAIL-OPEN
  SPREADS.** Both existing copies carried #608's `<>` fail-open; extract, never paste a third.
- **⇒ A NEGATIVE ASSERTION MUST NAME WHAT IT IS NEGATIVE ABOUT.** `.is_some()` on a DB error passed
  against a tree with no helper at all (`42883` is some error) — #594's `!status.success()` again.
- **⇒ A REFACTOR'S TEST IS A SOURCE GUARD** (behaviour is green before and after), with an
  anti-vacuity control. **⇒ Write a harness's positive control first.** **⇒ `grep` for which test
  covers a line; do not reason from file names.** **⇒ Resolve every ADR link before merge.** **⇒ When a
  new report lands, the published contract (`--help`) is part of the diff.**

### 2026-09-16 — #594: a restore that left records behind exits INCOMPLETE (condensed)

Plan `docs/superpowers/plans/2026-09-16-restore-exits-incomplete-594.md`;
[ADR-0071](spec/decisions/0071-a-restore-that-left-records-behind-exits-incomplete.md). The durable
rule is trap 11. What still generalises:

- **⇒ REVIEW THE REVIEW'S FIXES, AND THEN REVIEW THOSE.** Three of four rounds found a defect the
  previous round's fix created; check new absolutes against the issues you just filed.
- **⇒ WHEN YOU ADD A NEW STATUS, AUDIT THE OLD ONES FOR THE SAME STATE — THEN AUDIT WHAT THE NEW ONE
  STILL CANNOT SAY** (that is what found #614/#615). **⇒ `!status.success()` stops being an assertion
  the moment a third status exists — write `Some(n)`.** **⇒ A test that pins an order must pin that
  both sides are present.**
- **⇒ A NEW ADR NEEDS ITS `mkdocs.yml` NAV LINE IN THE SAME COMMIT** (`--strict`). **⇒ `--help` is
  assembled at runtime — assert the SPAWNED help, and its status.** **⇒ A duplicate with a documented
  reason is not drift.**

### 2026-09-15/16 — #584: a late key reaches the chart (condensed)

Plan `docs/superpowers/plans/2026-09-15-late-custody-reaches-the-chart-584.md`;
[ADR-0070](spec/decisions/0070-a-late-key-reaches-the-chart.md). Traps 9 (retired) and 10.

- **⇒ The paper-parity plan guard wants its literal labels** ("Paper counterpart", "Steps", "Time +
  cognitive load"). **⇒ Before recording a survivor as unobservable, ask whether a raising probe
  observes it** (M6) — and, since #619, whether a `SET ROLE` seam does. **⇒ Deleting or reordering a
  statement can silently move which layer a fault-injection test hits** — re-check each such test.
  **⇒ Check an ADR sentence by sentence against the code, not against its design doc.** **⇒ A
  background wrapper `cmd; echo exit=$?` reports the echo's status** — read the logged exit.

### 2026-09-10 → 09-15 — slice 2d, its budget, requeue custody, the CodeQL pack, `verify-backup`, #593 (condensed)

Each plan in `docs/superpowers/plans/` carries its review ledger (PRs #573–#595). What still
generalises:

- **⇒ A PIN OVER SHIPPED BEHAVIOUR PASSES ON ITS FIRST RUN, SO THE MUTATION IS THE RED PHASE — AND IT
  MUST FAIL AT THE ASSERTION THAT NAMES ITS CLAIM.** Read the panic line. **⇒ A list of owed tests
  recalled from memory was wrong by one — grep, do not recall.**
- **⇒ A FIXTURE MODELLING "A FRESH MACHINE" MUST BE CHECKED TABLE BY TABLE, AND A COUNT ASSERTION CAN
  BE VACUOUS BY FIXTURE SIZE** (the shared wipe left `node_unwrap_key` registered — #598 tracks the
  older suite with the same gap; 10 001 small records crossed the row cap and never the byte cap).
- **⇒ A DOOR RETURNING `Ok` IS NOT THE RECORD COMING BACK** — db/020's lenient arms warn and return
  normally, and nothing reads Postgres notices (#585); assert the projection. **⇒ THE HEADLINE TEST
  DECRYPTS A BODY** — a row count would have shipped a double-wrapped key with every count agreeing.
- **⇒ REUSED OPERATOR TEXT CAN BE FALSE IN ITS NEW HOME, AND A REMEDY IN A MESSAGE IS CODE.** Read
  stdout and stderr APART. **⇒ Treat an issue's scope and a design's "why not X" as claims** (#567's
  asked-for warning could never have fired). **⇒ A design sentence with two readings and no test
  survives a merge** (ADR-0068 settled one); an erratum only under a passage false about the code.
- **⇒ A GUARD NEEDS A POSITIVE CONTROL THAT IT SEES THE CODE IT GUARDS** (a source guard skipped ~96% of
  `main.rs` after the first `#[cfg(test)] mod`, #586). **⇒ A new definer function copies its `SET`
  clause — write `public, pg_temp`** (`search_path_pg_temp.rs`, #426).
- **⇒ A RED GATE CAN BELONG TO A PREDECESSOR (#583)** — `cairn_test` is never recreated between sweeps;
  truncate `local_node` before trusting a red after a killed gate. **Fault injection without residue:**
  a `cairn_test_*` trigger (or role, since #619) scoped to one test, dropped at test start and end.
- **⇒ CodeQL (#576): `rust/cleartext-logging`'s sources are NAME heuristics** — one `barrierModel` row
  per function RETURN; read the SARIF `codeFlows` first; local reproduction via `gh codeql`.
- **⇒ A SUBAGENT THAT ENDS ITS TURN WAITING ON A BACKGROUND JOB NEVER WAKES** — brief every dispatch
  "foreground only", and put `cargo fmt --check` and the `-D warnings` doc build in every task's
  commit step (#619's plan missed exactly that once). **⇒ A MEASUREMENT'S RESULT IS ITS SHAPE**
  (linear, no bend, is what makes a ceiling predictable).
- Still open from this stretch: **#569** (db/052's registry door silently discards a content conflict
  and leaves `actor_event_id`/`seq` unvalidated). **Tooling:** rust-analyzer can hold `target/` for
  minutes — use a scratch `CARGO_TARGET_DIR`; zsh does not word-split `$T` (write `${=T}`).

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
  and a restore reads one back (slice 2d, **#554**, ADR-0067) — no decided-and-unbuilt
  item remains — open decisions, races and operational gaps, see ⇒ NEXT); optional escrow rungs
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
- **⇒ DR — closed and rehearsable end to end** (2a→2d, the §1.2 measurement, the non-interactive
  recovery code, #584, #594, #614/#615, and #619 on the node plane). What remains is the ⇒ NEXT list.
  Two things a reader is led to expect and will not find: **2d does NOT drive `cairn-sync`'s puller
  through `MediumTransport`** (the pure `within(verified_through) → sort by source_seq` derivation
  lives in `cairn-medium`), and **the per-peer quarantine quota does not apply to a restore-originated
  pen** (pinned at volume by `restore_pen_is_uncapped.rs`). Open issues the chain filed: **#549**,
  **#551**, **#552**, **#525**, **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module),
  **#531**/**#329** (decompose `cairn-sync/src/main.rs` — a maintainer decision on which to keep),
  **#532**, **#534**, **#535**, **#536**, **#537**, **#538**, **#556**–**#563**, **#569**, **#575**,
  **#589**–**#592**, **#596**–**#599**, **#602**–**#611**, **#613**, **#616**, **#617**,
  **#620**–**#622**.
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
