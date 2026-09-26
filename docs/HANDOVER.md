# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ #671 IS DECIDED AND BUILT: THE STEP-3 PROMPT IS A NUDGE, NOT A COMPLETENESS CLAIM
> ([ADR-0075](spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md), spec
> v0.77, PR [#678](https://github.com/cairn-ehr/cairn-ehr/pull/678), 2026-09-26).** Maintainer's
> clinical call: duplicates are common (typos in hard names) and the person at the desk cannot be
> made to browse, so **accept duplicates and make repair (`link`) easy** — the safety measure is how
> fast a duplicate is FOUND. So: `search.incomplete` means only that the SEARCH was partial
> (ADR-0061's meaning, restored; no wire change); being cut to five is `PromptList::withheld`, shown
> as *"the 5 closest of N"* and never signed; ranking gained **name tokens matched** and a **DOB
> near-miss**. Measured over 50,000 real names: every single slip (wrong DOB, surname typo) is now
> in the five 500/500; a surname typo AND a simply wrong DOB 204/500 — that residue is the repair
> path's (`cairn-gui/cairn-gui-tauri/results/2026-09-26-funnel-prompt-ranking.md`).
>
> **⇒ NEXT, in order:**
> 1. **The repair path — brainstorm first, with the maintainer:** **#679** (commit-time local
>    duplicate check by the §5.2 matcher), **#680** (duplicate worklist), **#681** (link gesture —
>    show each chart's allergies/active meds at link time, the window's hazard). ADR-0075 decision 2
>    is the brief. Probably one design covering all three, then slices.
> 2. **The human acts 2c exposed** (an agent cannot do them): runbook §8's stopwatch figures (find
>    ≤ 5 s, register ≤ 20 s, live AND `--mock`) and the front-door accessibility checks, recorded
>    in a dated copy of `results/TEMPLATE.md`. See also *Four things still owed are HUMAN acts* below.
> 3. **#620**, a wire-contract DECISION (the COSE unprotected header is hashed into the content
>    address but lies outside the signature, so a relay can re-wrap an event): the only open item
>    that can still change the wire; needs a brainstorm first. Then **#626** (the clinical twin of
>    #621: db/020's raw casts and `do_pull`'s single freeze arm; kept out by maintainer decision
>    because db/020 is the 100k-event hot path). Then **#652 + #655** together (the P0001 rule's
>    three homes, and its `false` half: `42501`/`42P01`/class-23 are floor decisions that land in
>    `Unavailable`). Small, advisory-tier: **#640**, **#641**.
>
> **⇒ THE FUNNEL'S DURABLE RULES — do not undo any of these** (full text: the design page's
> dated notes and ROADMAP's 2a → 2c entry):
> - **`search_patients` RANKS BY FOUR KEYS** (`cairn_patient_search::rank_candidates`, inputs read
>   in `patient/search_rank.rs`): passes matched → name tokens matched → DOB near-miss → chart
>   age. It only REORDERS. `db/046` is a disjunction, so in plain id order the prompt showed the
>   OLDEST charts; "simplifying" back to `ids.sort()` or to passes-only fails
>   `patient_search_ranking.rs`. Name tokens are counted over the RETAINED set (`patient_name`,
>   repudiated names included — #349), never `patient_name_current`.
> - **`incomplete` is the SEARCH's partiality only; truncation is `withheld`** (ADR-0075). Folding
>   `withheld` back into `incomplete` re-creates #671 (a flag set on 92% of registrations).
>   `attestation_through_the_port.rs` pins both halves: a cut prompt signs `false`, a search that
>   could not read a chart signs `true`. **Do not raise `PROMPT_CAP` to make a number look better.**
> - **The raw typed name travels WITH its token** (`FunnelSession`); `register` takes only the
>   token. A search for an older form revision is DROPPED, never recorded over a newer one; the
>   webview forgets its held token synchronously on every edit.
> - **`require_provisioned` runs in the window's register command, BEFORE `take`**, not inside
>   `PatientRegistration::register`: the port suites use an unenrolled signer to reach db/005
>   INSIDE the transaction, and a pre-check in the port would leave those proofs green and empty.
> - **Only a candidate some list on screen showed can be opened** (`AppState::shown`).
> - **EVERY CHART COMMAND NAMES THE CHART ON SCREEN** (`AppState::displayed_patient`) — the
>   2c review's Critical. With charts switching, "the open chart" and "the chart on screen" can
>   differ while a read is in flight; a sign-off that resolved only the open chart signed patient
>   B on the strength of a review of patient A's list. `med_list`/`sign_off`/`cease` carry the
>   displayed id and refuse a mismatch; the webview clears the chart view on every switch and
>   drops a late read. **Never let a new chart command resolve `open_patient()` alone.**
> - **A click within 800 ms of a step-3 prompt landing is "show me", never "register"**
>   (`PROMPT_READ_GUARD_MS`): the background search can flip the button's meaning under the
>   pointer. `funnel_status` reports the revision floor so a reloaded webview resumes above it.
> - **The launch probe matches all four `ActorStanding` arms** and reuses `cairn-node`'s own refusal
>   sentences. Never a boolean: a `Retired` key sent to `enroll-device-actor` meets db/004's
>   resurrection refusal (#152). Pair the standing with the key it was probed for (#670).
> - **Every sentence and its retry advice lives in `funnel/view.rs`** (`Retry::{Now, AfterOperator,
>   Never}`). A failed search says NOT-a-no-match in capitals. A refusal and an outage are different
>   clinical facts (#648, closed 2026-09-23; the non-`P0001` remainder is #655); the discriminators are `P0001` from the floor and `DeliberateRefusal`
>   from Rust, with `RefusalScope::NodeState` → `NotProvisioned`.
> - **`TokenStore::settle` is the sanctioned end of a `take` for callers** (`FunnelSession`'s
>   defensive branch calls `restore` itself); a success INVALIDATES (a mid-flight
>   re-search must not mint a second chart); `discard` deliberately does NOT clear `in_flight`.
>   **A dropped `register` future still latches the store (#669), and `register` is
>   cancellation-unsafe (#649): never race it against a timeout or `select!`.**
> - **The step-3 trigger is advisory, never a gate**: a mononymous patient or an unknown DOB still
>   registers; the first Register click runs the search and SHOWS it, and the next one registers.
> - **Nothing provisions an actor on a write path.** `init` enrols; `enroll-device-actor` is the
>   remedy; fifteen CLI write commands `require_device_actor` (count-guarded in `main.rs`);
>   `enrolment_is_never_a_write_side_effect.rs` scans every shipped `.rs`. **When it goes red, do
>   not add your call site to `ALLOWED`.** `device_actor_enrolled` is kind-agnostic on purpose.
>   Scoped to the node's own device actor: `resolve_matcher_actor` still enrols (#663). What a
>   SUPERSEDED key classifies as is undecided, and db/004 contradicts itself: #666 first, then
>   #664. **`init` must not `?` its enrolment.**
> - **`cairn-gui-live` is where a DB-backed port implementation goes** (not `cairn-gui-data`, not
>   `/crates`). **The P0001 rule has three homes** (#652). The `cairn-gui` DB suites run in CI's
>   `test` job; the `gui` job declares `CAIRN_ALLOW_DB_SKIP=1` on the STEP; deleting that step is
>   invisible (#656). Fixture truncate lists are derived from the catalogue, and the predicate
>   misses identity-stream tables (#658).
> - **`--mock` holds ONE `MockData` for the window's life** (half of #668). The arming affordance
>   and typed slots are still open, and `fail_next` stays unreachable from the shipped binary on
>   purpose. The mock's matching rule is NOT db/046's: never generalise a timing from it.
>
> **Open from the funnel run:** #355 · #645 · #647 · #649 · #650 · #652 · #655 · #656 · #657
> (multi-event rollback untested in both trees) · #658 · #662 (seven `init` effects unpinned) ·
> #663 · #664 · #665 (the orchestrator-level half) · #666 · #667 · #668 · #669 · #670 ·
> #672 (identifier entry) · #673 (the header shows age, not DOB) · #676 (the clerk reads
> `operator_chain` text, `[P0001]` included) · the repair path #679 · #680 · #681 · #682 (an NFD
> trailing accent is lost: `SearchQuery` tokenises before NFC; changes signed tokens). #675's four gaps were fixed in PR #674's third review
> round. **Decided 2026-09-23 (#677):** the 800 ms prompt read guard is SOFT POLICY and stays in
> `funnel.js` only; do not move it into `FunnelSession` without reopening that decision.
>
> **⇒ Hold these two rules from that round.** A chart command acts on the chart DRAWN
> (`renderedPatient` in `main.js`), resolved in Rust only through `AppState::displayed_patient`
> — `open_patient` is now private to `funnel`, and each chart command has a `*_impl` pinned by a
> not-on-screen test. And a registration never writes, or switches charts, behind an open chart
> (`register_impl` + `open_after_registering`): "This is them" during "Saving…" wins.
>
> **⇒ THE NODE PLANE AND DR ARE CLOSED OUT; NO DECIDED-AND-UNBUILT ITEM REMAINS.** Newest first:
> #621 (PR #627, [ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md):
> the three node doors are total, and the puller pens a non-local non-`P0001` failure instead of
> freezing), #619 (PR #623, ADR-0073), #614 + #615 (ADR-0072), #594 (ADR-0071: `restore` exits **3
> INCOMPLETE**, **1 = BLOCKED**), #584 (ADR-0070), the non-interactive recovery code (ADR-0069),
> the §1.2 restore measurement (PR #573: 100 003 events in 116.7 s against 600 s, linear 1.17
> ms/event, ceiling ~510 000; **#512** stays open, `M > N`), DR slices 2c/2d (ADR-0067/0068). A
> solo clinic can lose its disk, restore, **open a chart** and rehearse it unattended. The durable
> rules are traps 1–14 below; the per-slice narrative is ROADMAP's.
> - **⚠️ Citation discipline.** Rows and custody coming back is NOT a body opening: cite
>   `restore_reads_the_clinical_plane.rs` and
>   `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`, never
>   `dr_clinical_guarantee_gap.rs`'s counts. **Never cite ADR-0026 decision 1's promise 2**
>   (*"node-default data-at-rest keys survive"*) as met: no node-default key tier exists (ADR-0067).
> - **The pen-release rule (#578, PR #582):** a pen row carrying a wrapped DEK is released only when
>   custody for its event is SETTLED (`cairn_release_pen_row`, db/052;
>   `pen_rows_leave_through_one_door.rs`). `requeue` and `restore` share exit 3. #585: nothing reads
>   Postgres notices.
> - **`verify-backup` (#567, PR #588)** fails `backup SHORT` only on evidence. ⚠️ Operators: run it
>   AFTER the nightly `backup`. Residuals #551 · #553 (an unmarked foreign legacy medium can still
>   be destroyed by succession) · #589 · #590 · #591 · #592.
> - **#527/#562's triage note is false**; the real fix is **#575** (the minted recovery code
>   reaches stderr). A retry after a crashed restore must move the installed `<key>.unwrap` aside
>   first (**#596**; test 19).
> - **Open decisions (none a patch):** #575 · #602 (any client can set `cairn.remote_apply`) · #611
>   · #613 · #620. **Restore residuals:** #616 · #617 · #596–#599. **Races (reasoned, not
>   reproduced):** #603 · #604. **PR #601's wave:** #605 · #606 · #607 · #608 · #609 · #610. **Node
>   plane:** #268 · #301 (both `loop:needs-human`) · #569 (the actor registry's silent
>   content-conflict discard). **From #619/#621:** #622 · #624 · #625 · #626 · #628 · #629 · #631 ·
>   #632 · #634. **Search:** #637 (the materialised token table: the right fix for the ~860 ms Pi
>   floor, its own slice) · #640 · #641 · #643.
> - **Still broken, named rather than assumed away:** #549 · #552 (read-side peak memory
>   unbudgeted) · #536 · #502 item 4 · #101 items 2–3 · #583 · #586 · #587 · #556–#563.

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS (measured 2026-09-12), KEPT THAT WAY BY A MODEL PACK — AND ONE HUMAN
> ACT IS DUE: make `CodeQL (rust)` a required check (#444).** PR **#576** replaced default setup
> with a committed workflow + model pack (`.github/codeql/packs/cairn/codeql-models`, one
> `barrierModel` row per NAME-heuristic source, each with its reason; 44 → 3, the 3 dismissed). The
> flip needed the **organization-level** configuration as well as the repository's. **Read the alert
> list with `scripts/codeql-alerts.sh`, never assume it**; CONTRIBUTING has the local reproduction.

> [!IMPORTANT]
> **⇒ GITHUB CLOSES ON ADJACENCY, NOT ON SENTENCES (2026-09-04).** Seven issues (#101, #115, #434,
> #441, #468, #500, #534) were closed by prose *disclaiming* the close; all reopened. Guarded by
> `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml` (**#444** would make
> it required). **`fix(#500):` is SAFE** — the parenthesis breaks the adjacency. Residuals: **#547**,
> **#548**.

> [!IMPORTANT]
> **Eighteen traps. Each is a step a next session takes in good faith.** (Five came from slice 1;
> trap 5 was minted by #511, trap 7 by DR slice 2c, trap 8 by #578, trap 9 by the #582 review —
> **retired by #584 and kept as history** — trap 10 by #584, trap 11 by #594, trap 12 by #615,
> trap 13 by #619, trap 14 by #621, traps 15–17 by #639, and trap 18 by #661.)
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
> 9. **RETIRED — HISTORY, NOT ADVICE (#584 built, ADR-0070).** It warned that a key landing on an
>     already-admitted event opened the body and left the chart empty; the door now projects a late
>     key and `reproject_owed` is gone. **What survives as live advice: do not remove the
>     `cairn_project_late_custody` calls or move them.** In BOTH doors they sit AFTER the
>     substitution guard (a rival body must never reach an applier), test-pinned in
>     `late_custody_reaches_the_chart.rs`, each with its OWN positive control so a probe that stopped
>     being registered cannot leave them passing vacuously. The strict door's POSTURE is pinned too:
>     wrapping db/005's call in `cairn.remote_apply = 'on'` "to match db/020" turns a strict refusal
>     into a flag, and now fails. In db/020 the call sits BEFORE the `cairn.remote_apply` clear —
>     after it, three projection guards RAISE and the key could never land. (#597's notice is still
>     misleading; it no longer hides an empty chart.)
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
> 14. **⇒ A DOOR THAT LETS POSTGRES RAISE ON CALLER-SUPPLIED BYTES IS BREAKING THE P0001 CONTRACT BY
>     OMISSION (#621, ADR-0074, 2026-09-20).** Every field the three node doors read out of signed
>     bytes goes through a helper that raises P0001 — `cairn_uuid_or_raise` (db/001, on
>     `pg_input_is_valid`), `cairn_hlc_nonneg_or_raise` (db/001), `cairn_node_role_or_raise` (db/007) —
>     and `node_door_input_guards.rs` fails on a bare `::uuid` in any door body or on a door that
>     stopped calling one. **The tempting wrong moves:** (a) writing a REGEX instead of
>     `pg_input_is_valid` — a validator narrower than the cast it replaces refuses events the log can
>     already hold, the mirror of PR #623's finding 1 (mutation M9; only the odd-spelling positive
>     control sees it); (b) re-inlining the role list into the CHECK "since it is only three values" —
>     the door and the floor then hold two lists, and the day one is widened an older node meets
>     `23514`, a frozen link, instead of a P0001 it could skip (M10; behaviour is identical TODAY, so
>     only the structural guard sees it); (c) giving any of these refusals `USING ERRCODE` — that turns
>     `cairn-sync`'s clinical pen into a freeze (trap 13a); (d) deleting the CHECK constraints as
>     redundant — they are the floor for raw SQL, the door is the privilege (principle 12).
>     ⚠️ **The puller's default for an UNKNOWN SQLSTATE is DETERMINISTIC (pen), and "tightening" it to
>     freeze reinstates #621.** A wrongly penned event is delayed, held, re-offered and auto-released;
>     a wrongly frozen link is permanent and has no remedy. The local classes — `08 40 42 53 55 57 58`
>     and *no SQLSTATE at all* — are claimed explicitly and must stay equal to `cairn-sync`'s
>     `apply_failure_is_local` (`sqlstate_classes_agree.rs`; merging the two copies is **#626**).
>     ⚠️ **`XX001`/`XX002` are LOCAL, claimed by full code ahead of the class match, on BOTH planes.**
>     Class `XX` is otherwise the adversarial-bytes case that must never wedge a link, but a corrupt
>     page or index is this machine's disk: without the exception the puller pens a peer's WHOLE log
>     while writing "will fail on these bytes identically every time" onto every row. The drift guard
>     compares five-character codes as well as classes for exactly this reason.
>     ⚠️ **A pen row of this kind leaves ONLY by applying or by an ack** — never because the door
>     later reaches a P0001 verdict about the same bytes (the deny-all arm cannot tell which KIND of
>     row it would delete without reading the reason TEXT, and a substitution row must never
>     auto-release). Every operator sentence says so; do not let one drift back to "fix the cause".
>     ⚠️ **The role CHECK is `NOT VALID` and must stay so:** the migrations replay on every connect,
>     so a validating pair re-scans `node_event` each time and one row left by a downgrade after a
>     vocabulary widening stops the node STARTING — before an operator can reach the database to
>     widen `cairn_node_roles()` again or drop the constraint. (Those two ARE the repair, one line
>     each; the branch review's "unrepairably" was too strong. The decision stands anyway: a fleet
>     node may not refuse to START over a vocabulary it once admitted, however easy the repair is
>     to type, because the node is what you would be typing it into.)
>     **Known exception to "total":** `cairn_body` raises `22P05` before every guard on a NUL in any
>     body string (**#628**) — the puller pens it, and a test pins only that the link keeps moving.
>     Residuals: **#626** (the clinical plane still freezes on all of them), **#628**, **#629**,
>     **#605**, **#268**, and from the four-reviewer pass on the finished branch: **#625** (the
>     peer-blind pen — it gained this pass's two extra findings; **#630** was filed for them and
>     closed as its duplicate), **#631** (a bumped row keeps its original `reason`),
>     **#632** (the claimed-local set misses door-confined codes like `P0004`/`21000`), **#633**
>     (nothing pins the `USING ERRCODE` contract itself), **#634** (pen-at-quota, auto-release of
>     this pen kind, and the deliberate NO-release-on-later-verdict invariant lack tests).
> 15. **⇒ THREE THINGS IN `db/046`'s PASS 3 LOOK LIKE NOISE AND ARE EACH WORTH HUNDREDS OF
>     MILLISECONDS ON A PI (#639, 2026-09-21).** All three are the kind a tidy-up deletes.
>     (a) **`OFFSET 0` IS AN OPTIMISATION FENCE, NOT A LIMIT.** It is what stops the planner pulling
>     the query-token subquery back up and re-inlining `lower(normalize(t, NFC))` at all three sites,
>     per (stored token × query token) pair. **A plain subquery was measured and does not work** —
>     removing the fence reinstates #639 while changing no result, so nothing fails.
>     (b) **THE LATERAL IS `UNION ALL` ON PURPOSE.** "Surely that should be `UNION`, the two sources
>     overlap" costs a dedup sort per `patient_name` row (~30%) to remove a duplicate the branch's own
>     `SELECT DISTINCT` and the outer `UNION` already remove. ⚠️ The file's dedup block is now down to
>     **two** layers, and the one remaining sentence about the lateral says it is NOT a dedup: if a
>     later change makes the branch `DISTINCT` non-load-bearing, duplicate rows reach the caller.
>     (c) **THE PARTS-BRANCH SKIP MUST BE TESTED ON `lower(normalize(pn.value, NFC))` — LOWERED AND
>     NORMALISED, AND THE `lower` IS THE ONE THAT BIT.** The guard must ask about the exact string
>     the splitter is handed. It shipped in review asking about `normalize(...)` alone, and **one
>     code point in all of Unicode** walks through that gap: U+0130 `İ` is `[:alnum:]` but lowercases
>     to `i` + U+0307 COMBINING DOT ABOVE, which is not — so `İnce` reads unpunctuated to the guard,
>     punctuated to the splitter, the branch is skipped, and `nce` stops finding the chart. Dropping
>     the `normalize` is merely conservative (the branch runs unnecessarily); dropping the `lower`
>     LOSES A TOKEN. Fires only under FULL case mapping — ICU providers yes, libc no, so it was live
>     on every local `cairn*` DB and invisible in CI.
>     ⚠️ **A PROBE LIST IS A SAMPLE, NOT AN ARGUMENT.** The 13-value probe was green throughout,
>     because it stressed scripts, combining marks and whitespace but not case mapping. If a claim
>     reduces to a property of character classes, ENUMERATE THE CLASSES:
>     `the_subset_argument_holds_for_every_unicode_code_point` now checks the two facts the whole
>     subset argument rests on (`\s` IS `[[:space:]]`; nothing is both `[:space:]` and `[:alnum:]`)
>     over **all 1,114,111 code points in ~0.6 s**. The sampled probe survives beside it as the layer
>     that catches a bad COMPOSITION rather than a bad class — which is what this defect was — and
>     `the_subset_probe_still_describes_the_query_db046_runs` pins the **composed** expression, so
>     dropping the `lower` fails independently of any corpus.
>     ⚠️ **A neutrality claim needs a test that can see a GAIN.** Every pre-#639 pass-3 test asserts a
>     chart IS found, so a rewrite returning EXTRA charts passed all of them; the mutation that
>     dropped the parts branch's callsign guard was caught only by a gesture expecting the EMPTY set.
>     ⚠️ **AND A NEUTRALITY DIFFERENTIAL IS ONLY AS WIDE AS ITS CORPUS.** The 394-token / 14,447-row
>     differential returned 0 lost / 0 gained and was *right about the names it drew* — all
>     Latin-script Australian. It could not have found U+0130.
>     **Re-measure with `scripts/measure_patient_search.py`, do not reason** — #637's diagnosis was
>     reasoned, confident and wrong, and its remedy followed the wrong cause. Residuals: **#641**
>     (the parts branch cuts a Devanagari name at its vowel signs), **#640**, **#643** (the rig times
>     `count(*)`, not the row transfer), and the remaining ~860 ms floor, which is the whole-token
>     split over every row and needs #637's materialised token table — for the right reason this
>     time.
> 16. **⇒ A GUARD AND THE THING IT GUARDS MUST BE ASKED ABOUT THE SAME STRING (#639 review,
>     2026-09-21).** Generalised from 15(c), because the shape is not about Unicode. Whenever a cheap
>     predicate decides whether to run expensive work, the predicate and the work must read the
>     *identical* expression — not a simplification of it, however obviously equivalent. Here the two
>     differed by one `lower(`, agreed on every test value, and disagreed on exactly one input in the
>     whole domain. The defence that works is pinning the COMPOSED expression as a literal (this
>     repo's `include_str!` + `contains` idiom), not pinning its pieces: the piece-wise list was
>     present, passing, and blind.
> 17. **⇒ LOCAL POSTGRES IS ICU, CI's IS libc, AND `lower()` IS NOT THE SAME FUNCTION ON BOTH
>     (#639 review, 2026-09-22).** Every `cairn*` database on the dev Mac is `datlocprovider = 'i'`;
>     CI's `initdb -D "$PGDATA" -U postgres --auth=trust` (`rust.yml`) inherits libc, `datlocprovider
>     = 'c'`. Under ICU, `lower('İ')` applies **full** case mapping and yields TWO characters, `i` +
>     U+0307; under libc it is **simple**, one-to-one, and yields plain `i`. So a name containing
>     U+0130 genuinely has different pass-3 tokens on the two servers, and **both are correct**.
>     ⚠️ **This cuts both ways and burned a CI run in each direction.** The U+0130 recall defect
>     itself was live locally and invisible in CI. Then the regression test written for it pinned the
>     ICU answer unconditionally, passed on every local database, and **failed in CI** — where `nce`
>     had never been a token at all, before the rewrite or after, so neutrality held trivially.
>     ⚠️ **A contract suite must derive such a row from the SERVER, not assume a provider**:
>     `patient_search_equivalence.rs`'s `full_case_mapping` asks, and the gesture expects `[turkish]`
>     or `[]` accordingly. The layering that results is the durable lesson — **the gesture bites
>     where the defect is real (ICU), the composed-literal pin bites everywhere**, which is why the
>     pin is not redundant with it.
>     **To reproduce CI's locale locally** (this is how the fix was verified, and it takes seconds):
>     `CREATE DATABASE cairn_test_libc TEMPLATE template0 LOCALE_PROVIDER libc LOCALE 'en_US.UTF-8'
>     ENCODING UTF8;` then `CREATE EXTENSION cairn_pgx;` in it, and run the suite with
>     `CAIRN_TEST_PG=…dbname=cairn_test_libc`. **Any test that touches case, collation or character
>     classes should be run against both before pushing** — a local-only green is not evidence.
> 18. **⇒ A TARGETED `cargo test --test X` REPORTING `ok` IS NOT PROOF — ONLY THE SWEEP IS
>     (#661, 2026-09-23).** `device_actor_enrolment::a_revoked_actor_…` reported **`ok` twice** in
>     one session against a source tree it contradicts: it asserted the refusal did NOT contain
>     `enroll-device-actor`, while the refusal it was reading has contained that string since the
>     commit that introduced both. The full `cargo test --workspace` sweep failed it; re-running
>     the same targeted command afterwards then failed it **deterministically, three times**, with
>     no source change in between.
>     ⚠️ **The mechanism is unexplained** and is recorded that way rather than guessed at. The
>     only shape that fits is a **stale test binary** — the targeted run executing something not
>     built from the source on disk — which this tree is already known to be exposed to whenever
>     another cargo or a running rust-analyzer is touching the shared `target/`. The old failure
>     mode in that family is *loud* (a killed binary exits 101 with zero `test result: FAILED`
>     lines); **this one is silent and green**, which is strictly worse.
>     **What to do about it:** a targeted run is for the red→green loop, never for the claim that
>     work is done. **Gate on the sweep**, and if a targeted suite has been green while you were
>     editing the code under it, re-run it once more from a quiet tree before believing it. Use
>     `CARGO_TARGET_DIR=/tmp/…` when an IDE is open.

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

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 stopwatch figures** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy: sections 1–7 for the med-list gestures (only the *write* half is measured, median 222 ms,
**PARTIAL**; write-cost half **#360** unwired) and **section 8 for the front door** (find ≤ 5 s, register ≤ 20 s,
live and `--mock`; db/044's `gesture_kind` CHECK still refuses a registration timing row until widened);
(2) **the accessibility pass** — a live VoiceOver run through the runbook's checks, now including the front
door's, keyboard-only; DOM assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks**
(**#444**, admin-only — "clippy + cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`, now
DUE — the CodeQL callout above), matching job names exactly, per `CONTRIBUTING.md`'s dated table. **If a
measurement falls outside its budget, that is the finding — file an issue, never adjust the budget.**

**Other build candidates** (nothing blocks a choice): the **drugref term→anchor lookup** (the §9 advisory tier; closes the
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

**Session date:** 2026-09-26 (**#671 decided and built — the step-3 prompt is a nudge**, [ADR-0075](spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md), spec v0.77, PR **[#678](https://github.com/cairn-ehr/cairn-ehr/pull/678)**; filed **#679**, **#680**, **#681**) · 2026-09-23 (**funnel UI slice 2c built — the front door is runnable**; PR **[#674](https://github.com/cairn-ehr/cairn-ehr/pull/674)**; no ADR, no migration; `search_patients` now ranks by passes matched; filed **#671**, **#672**, **#673**) · 2026-09-23 (**2c's four prerequisites**, PR #661) · 2026-09-22 (**funnel slices 2a + 2b**, PRs #646, #653) · 2026-09-21 (**#636 slice 1 + #639**, PRs #635, #642, #644) · 2026-09-20 (**#621**, ADR-0074, spec v0.76, PR #627) · 2026-09-19 (**#619**, ADR-0073, PR #623) · 09-17 **#614 + #615** (ADR-0072, db/053, PR #618) · 09-16 **#594** (ADR-0071, PR #612) · 09-15/16 **#584** (ADR-0070, PR #601) · earlier: ROADMAP. · **Spec:** **v0.77** (newest [ADR-0075](spec/decisions/0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md); [ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes **ADR-0026 decision 2's implementation wording** only) · **`SCHEMA_GENERATION`:** **53** (`db/053`) · **Phase:** architecture complete (every original §11 question closed); **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door onto a medication chart.

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
(spike 0004, retired 08-03), so today it is **`cairn-gui-tauri`**: the funnel front door onto one patient's
medication chart (plain JS, no npm), pane/routing/freshness state machine tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and **every open issue number** (including an index of the ones
its prose does not name). This section keeps only what a *next* session needs — the traps, and the lessons
that generalise past the slice that found them.

### 2026-09-26 — #671: the step-3 prompt is a nudge (ADR-0075, PR #678)

Brainstorm → ADR → plan → inline TDD (six tasks). Design
`docs/superpowers/specs/2026-09-26-step3-prompt-is-a-nudge-671-design.md`; plan
`docs/superpowers/plans/2026-09-26-step3-prompt-is-a-nudge-671.md`.
- **⇒ THE MAINTAINER'S CLINICAL FRAME SETTLED WHAT THE DESIGN COULD NOT.** The first framing asked
  what the signed record should prove about the clerk; the maintainer answered that the clerk cannot
  be made to browse and duplicates are repaired by `link` — which moved the safety question from
  "did they look?" to "how fast is it found?" (#679–#681). **Ask what the person at the desk will
  actually do before designing what they must attest.**
- **⇒ THE MEASUREMENT FALSIFIED THE ADR DRAFT, BEFORE MERGE.** "A name typo never enters the candidate
  set" was repeated from #671 into ADR-0075; the new `name` arm found every one-token typo (other
  tokens + DOB still match). And an arm built from the same slips the near-miss key rewards grades the
  rule on its own test — the `-any` controls were added to find the real boundary (204/500).
  **Measure the claim in the ADR, and give every arm a control the feature cannot help.**
- **⇒ INVERTING A TEST CAN DELETE THE OTHER HALF OF A PAIR.** Two live tests asserted `incomplete`
  `false` and `true`; flipping the `true` one to match ADR-0075 left the flag free to be a constant,
  so a positive case (a nameless chart makes the search partial) was added.
- **⇒ A LATENCY INSTRUMENT MUST SEE THE CODE PATH.** `measure_patient_search.py` times only the SQL
  function; the new reads are Rust-side, so they were timed directly (2–5 ms over 968 ids).

### 2026-09-23 — funnel UI slice 2c: the window (PR #674)

Plan `docs/superpowers/plans/2026-09-23-funnel-ui-slice-2c-window.md` (its ledger's rulings are in
the PR). Executed inline, one whole-branch review. What generalises:

- **⇒ A DESIGN'S QUANTITATIVE ASSUMPTION IS A CLAIM — MEASURE IT BEFORE BUILDING ON IT.** The design
  said a full-name-plus-DOB search "returns few candidates by construction". Five minutes reading
  `db/046` showed a disjunction, and `search_patients` sorted by chart age: the signed five-row
  prompt showed the five OLDEST charts. The measurement then made the size of it undeniable (the
  duplicate shown 20% of the time). **Read the query the UI sits on before wiring the UI.**
- **⇒ A MEASUREMENT RIG NEEDS A POPULATION WITH THE RIGHT SHAPE.** `measure_patient_search.py`'s
  synthetic names are unique by construction (an index suffix), which is right for timing and
  useless for truncation, since nothing shares a token. The new rig skews common names and, better,
  draws real ones; its self-test pins the Python twins of the tokeniser and the ranking.
- **⇒ A MOCK-MODE WINDOW CAN BE WALKED WITHOUT TOUCHING THE MAINTAINER'S SCREEN.** A headless browser
  over `src-ui/` with a stand-in `invoke` returning Rust-shaped payloads exercised the JS
  revision/token bookkeeping end to end. What it cannot catch is a Tauri-IPC-only defect (argument
  casing), which stays the human pass's.
- **⇒ THE REVIEW'S CRITICAL WAS A PROPERTY THE SLICE ITSELF CREATED.** Before 2c a window had one
  patient for life, so "open chart" = "chart on screen" was true by construction and nothing
  needed to say so. Making `--patient` optional silently broke that invariant for three
  pre-existing commands. **When a slice makes a constant variable, audit every reader of it** —
  the new code was careful; the old code it made unsafe was not re-read. And the reviewer's
  measurement-scope finding (exact duplicates only) was confirmed by adding the arm, which showed
  ranking does nothing for a wrong-DOB duplicate: **measure the case the feature exists for, not
  the easy one.**
- **⇒ A SWEEP WITHOUT ALL THREE DB STRINGS IS NOT A SWEEP.** Setting only `CAIRN_TEST_PG` let every
  multi-node suite self-skip; `db_gate_actually_ran` refused it, correctly. Use
  `scripts/run-db-gated-tests.sh` (with a scratch `CARGO_TARGET_DIR`).

### 2026-09-23 — slice 2c's four prerequisites (PR #661), condensed

#659, #660, #651 and #654, closed before their only consumer existed ("close a trap while its only
victim is code that does not exist yet"; three of the four issues said so themselves).
- **⇒ A SECOND REVIEW ROUND PAID FOR ITSELF:** the first asked whether the code did what it said, the
  second whether what it said was TRUE — and found a reachable duplicate-chart path, two hollow
  guards, and a hazard documented backwards three times in one file.
- **⇒ EVERY CLAIM A TEST MAKES ABOUT A MUTATION IS VERIFIED BY APPLYING IT** (two of four would have
  been wrong otherwise). **⇒ An agent's claim about the tree is a claim** (a reviewer's "the tree's
  own `Drop` pattern" did not exist). **⇒ An issue's account of its blast radius is a claim — grep
  before planning** (#654 said one call site; there were fifteen).
- **⇒ A fixture's recall is not the real search's** (the mock matches per token; db/046 does not).
  **⇒ Retiring a convenience is a paper-parity question** — folding enrolment into `init` kept
  `M = 0` for an ordinary operator.

### 2026-09-22 — funnel UI slices 2a and 2b (PRs #646, #653), condensed

- **⇒ A DESIGN PAGE'S SENTENCE IS A PREDICTION UNTIL CODE MEETS IT** — both slices falsified one;
  both are dated revision notes on the page, never edited away.
- **⇒ The obvious probe for a floor refusal is often not one** (a malformed DOB refuses in Rust, with
  no SQLSTATE; #651, fixed by #661). **⇒ A mutation that survives twice marks the uncovered path**
  (borrow a real `DbError` from the server to bury under context layers).
- **⇒ A test can pass for a reason that is not the one in its name** — mutate against the sentence in
  the test's name. **⇒ An atomicity test whose probe never reaches the server is decoration**, and
  its fix still did not pin multi-event rollback (#657). **⇒ A signed flag nothing reads back is a
  claim nothing checks.** **⇒ A constant threaded through every call and never read back proves
  nothing** (`TODAY`).
- **⇒ A copied truncate list is a second-run failure waiting; a derived one is only as good as its
  predicate** (#658). **⇒ Four same-typed strings is an API defect** (`LiveData::new` takes
  `&Identity`). **⇒ A new DB-gated suite in a non-root tree runs nowhere until wired**, and **a guard
  cannot detect not being invoked** (#656). **⇒ A slice held to one tree keeps its gate honest**
  (#652 was deferred rather than touching `crates/`).

### 2026-08-20 → 09-20 — the restore, node-plane and door slices (condensed to one-liners)

Per-slice narrative: ROADMAP; the durable rules are traps 9–14; plans in `docs/superpowers/plans/`
carry each review ledger (#621 ADR-0074 · #619 ADR-0073 · #584/#594/#614+#615 ADR-0070–0072 · slice
2d, #567, #576 CodeQL pack, #593). What generalises, one line each:

- **An issue's failure scenario, scope and blast radius are CLAIMS** — read the code for what the
  issue MISSED (#621 missed a fourth raise; #567's warning could never fire; #654 said one site, not 15).
- **Validate a SQL value with the parser that will parse it** (`pg_input_is_valid`); **two parsers for
  one value are two protocols** (#624); no `CASE WHEN pg_input_is_valid … THEN $1::uuid` over a bound
  parameter.
- **A guard that has only ever been green has proved nothing**; a guard needs a positive control that
  it sees the code it guards (#586); **every harness needs a control for the run not happening**.
- **"Untestable" is a claim — try a `SET ROLE` seam first** (M10, #619); before recording a survivor
  as unobservable, ask whether a probe observes it.
- **A mutation is the RED phase of a pin over shipped behaviour** and must fail at the assertion naming
  its claim; a mutation anchor must be unique in both directions.
- **Negative assertions must name what they negate** — `!status.success()` stopped being one when exit
  3 appeared; write `Some(n)`.
- **Copying a guard spreads its fail-open** (#608's `<>`): extract, never paste a third copy (cf. #652).
- **Review the review's fixes** (three of four rounds found a defect the last fix created); when a new
  status is added, audit the old ones for the same state (found #614/#615).
- **A door returning `Ok` is not the record coming back** (#585, nothing reads notices) — assert the
  projection, and make the headline test DECRYPT a body.
- **Content-addressing over unsigned bytes is not content-addressing** (#620). **Cite a contract where
  it is written** (#228's comment above `cairn_decode_hex_or_raise`).
- **An unpushed branch is invisible** (2026-09-07 lost a session — house rule 8). **A round-trip test
  proves self-consistency, not correctness** (slice 2a: 19/19 mutations survived until golden bytes).
  **A scanner reads names, not values** (#527, rule 6b). **A deferral is honest only while its
  precondition holds** (#511).
- **Mechanics:** a fixture can manufacture a SQLSTATE production never sees (`serve_raw` → `23505`);
  `restore_node_event` refuses an enrolled node; a red gate can be a predecessor's (#583 — truncate
  `local_node`); fault injection via a test-scoped `cairn_test_*` trigger or role; a new definer
  writes `SET search_path = public, pg_temp` (#426); `nohup … &` inside a backgrounded call and
  `cmd; echo exit=$?` both lie — read the log's last line; subagent briefs say FOREGROUND ONLY and put
  `cargo fmt --check` + the `-D warnings` doc build in every commit step; a new ADR needs its
  `mkdocs.yml` nav line; zsh needs `${=T}` to word-split; rust-analyzer holds `target/` — use a
  scratch `CARGO_TARGET_DIR`. Still open from this stretch: **#569**, **#598**.

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
  recovery code, #584, #594, #614/#615, and #619 + #621 on the node plane). What remains is the ⇒ NEXT list.
  Two things a reader is led to expect and will not find: **2d does NOT drive `cairn-sync`'s puller
  through `MediumTransport`** (the pure `within(verified_through) → sort by source_seq` derivation
  lives in `cairn-medium`), and **the per-peer quarantine quota does not apply to a restore-originated
  pen** (pinned at volume by `restore_pen_is_uncapped.rs`). Open issues the chain filed: **#549**,
  **#551**, **#552**, **#553**, **#525**, **#541** (no CI job compiles `cairn_pgx`'s `pg_test` module),
  **#531**/**#329** (decompose `cairn-sync/src/main.rs` — a maintainer decision on which to keep),
  **#532**, **#534**, **#535**, **#536**, **#537**, **#538**, **#556**–**#563**, **#569**, **#575**,
  **#589**–**#592**, **#596**–**#599**, **#602**–**#611**, **#613**, **#616**, **#617**,
  **#620**, **#622**, **#624**–**#626**, **#628**, **#629**.
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
