# HANDOVER — Cairn

## ⇒ NEXT

> [!IMPORTANT]
> **⇒ THE NEXT GOAL (maintainer, 2026-10-31): a GUI milestone** — register a clinician as a user, log in, search /
> select / register a patient, enter demographics, write a progress note, record an allergy, chart a medication,
> sign everything off, log out. **Four decisions taken in conversation, not yet in any ADR:**
> 1. **A superuser role assigns new users.** The installer creates at least one superuser at installation time.
> 2. **A key vault holds the clinicians' signing keys**, so login is "choose who you are + passphrase", not a
>    `--attester-key` path given at launch.
> 3. **A draft auto-committed after the quarantine timer (the clinician has logged out) is RECORDED, NOT ATTESTED** —
>    the clinician's key is gone, so it cannot carry their signature; attesting it later is an overlay.
> 4. **"Prescribe" in this milestone means CHARTED** (a medication added to the chart from the window). Printed and
>    electronic prescriptions come much later.
>
> **Its sign-in / authorship part:** design
> [`superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`](superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md)
> (builds ADR-0008 §5.11; slices 1 sign-in → 2 re-key (#247, #664 first) → 3 drafts + `Asserted` grade + its display →
> 4 emergency key → 5 autonomous terminal nodes that sync credentials with specified nodes). **Slice 1 MERGED
> 2026-10-31** (PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897), [ADR-0083](spec/decisions/0083-the-human-key-vault.md),
> `db/061`, `cairn_pgx` **0.4.0**): the key vault, per-act signing (`--key-hold-seconds`, 0 = shared terminal), the
> signer strip, the terminal lock, "Enrol staff", the `vault-*` console verbs, one `EventSigner` seam.
>
> **#898 + #899 BUILT in PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)** (2026-11-01; design
> `superpowers/specs/2026-11-01-window-connection-posture-898-899-design.md`, plan of the same date,
> [ADR-0084](spec/decisions/0084-the-window-never-holds-the-owners-keys.md), spec **v0.86**, `db/062`,
> `SCHEMA_GENERATION` **62**). The window runs on its own login (`cairn_window`: inherits `cairn_agent`, never
> `cairn_node`), never loads the schema (a stamp mismatch refuses the launch), refuses a database on another machine
> (no TLS exists, so no override), and `release_for_act` refuses — before any passphrase is sent — over a connection
> that is over-powered through ANY role it is a member of. **`cairn-node provision-window-role --role <name>`** is the
> one owner step for install AND upgrade; it refuses a cluster with any `trust` rule or an unreloaded `pg_hba.conf`.
> **⚠️ RUNBOOK §0 changed:** the measurement cluster must use `scram-sha-256` (provisioning refuses `trust`), and every
> window launch uses `$WINDOW_CONN`. Tests are unaffected (they log in as a probe role, never through the verb).
>
> > [!WARNING]
> > **NOT YET GUARANTEED — gates on any pilot:** until **#909** (pinned, installer-generated TLS) or slice 5 (every
> > terminal its own node) lands, a facility has ONE usable terminal: the one on the database's machine. **#912**: no
> > long-running process re-adjudicates deferred events (the window's launch used to, incidentally). **#910**: a
> > Cairn-installed Postgres (brainstorm first). **#913**: PR #911's wording residuals (RUNBOOK §0's `pg_config` may be PG16 on this Mac; a db/062 overclaim; two ADR-0084 errata). **Slice 5's design must answer** (design §4): synced sealed blobs on a
> > stolen terminal disk can be guessed offline, and the attempt counter is per node. Open from slice 1: **#900**
> > (a superuser key could sign a clinical event) · **#901** (bound stored Argon2 parameters) · **#902** (handle case) ·
> > **#903** (another's guesses can hold a member at the 300 s cap — a decision) · **#904** (an unreadable blob counts
> > as a wrong guess; `cairn_pgx` 0.5.0) · **#905** (CI never runs `cairn-gui-tauri`'s DB-gated tests — run them
> > locally) · **#906** · **#907** · **#908** (a `cairn_pgx` reinstall re-grants `cairn_vault_unseal` to PUBLIC unseen).
> > Human-key `&SigningKey` parameters slice 5 must move onto the seam: `identify.rs:83`, `apply_proposal.rs:94`.
> > **Local `cairn_pgx` reinstall on this Mac:** patch the dylib with `scripts/macos-patch-pgx-dylib.py` (CI unaffected).
> > **Key-hold terminals:** the strip keeps the last staff ID, so a next person typing only a passphrase costs the
> > PREVIOUS person a counted try (never a wrong author) — revisit with slice 3's "who's writing".

> [!NOTE]
> **PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895) MERGED 2026-10-31** (closes #893): a command the window's
> watchdog cut off now SAYS so ("the command was not finished within 15 s, so this window cut the connection
> off") instead of the bare `connection closed` a node restart leaves too. Every `NodeConnection` call site whose
> error reaches the screen passes it through `ConnectionGuard::explain` while it holds the guard (pinned per
> function by `node_lock_explains_tests.rs`); outcome-line causes use `cause_of`; failures already worded as text
> (sign-off lines, worklist sides) get `name_cut_in`; text only, no class changes (durable-rules' #893 bullet). PR #892 (#875 option 2, the 15 s cut + waiting bar) merged
> 2026-10-30; its brainstorm filed **#891** (an open chart's events go first in sync; brainstorm before building)
> and its review **#894** (measure long multi-statement holds over a WAN).
> **Merged 2026-10-24/30 (ROADMAP has each):** PR #892 (#875 option 2) · #890 (#810) · #889 (#884 + #888) · #887 (#882 + #885) · #886 (#878) · #883 (#880 +
> #881) · #879 (#876) · #877 (#875 option 1; option 2 → #892) · #874 (#807; RUNBOOK §14 (c) human-owed) · #873
> (#855 + #837 item 1, ADR-0082 E1) · #872 (#869) · #868 (#861; filed **#870**, **#871**, decisions) · #865 (#863 +
> #864; filed **#866**, **#867**, decisions).
> **Merged 2026-10-18/23 (ROADMAP has each; the hold and stop-reason blocks are in `developers/durable-rules.md`):**
> PRs #860 · #856 · #853 ([ADR-0082](spec/decisions/0082-a-release-of-an-ended-hold-is-refused-at-the-local-door.md), spec **v0.84**) · #851 · #847 · #845 · #842 ·
> #838 (the window's **W**; **owed by a human: RUNBOOK §15** — W ≤ 10 s, Release ≤ 5 s, VoiceOver/keyboard) · #831
> ([ADR-0081](spec/decisions/0081-medication-withhold-is-a-time-bounded-hold.md), `db/059`) · #830 · #829 · #827 · #824 · #821 · #819 · #816 · #813 · #805 (#797, "Read this
> chart again"; RUNBOOK §14 is its owed timing) · #802. Still open from them: **#858** · **#850** · **#837** · **#825**
> · **#185** · **#862** (`run-db-gated-tests.sh` with `CAIRN_TEST_PG*` but no `PGPORT` runs its SQL step against
> :5432 — pass `PGHOST=127.0.0.1 PGPORT=5532`). The repair path (#679 · #680 · #681)
> is built through R7. Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`; R4's
> runbook `docs/developers/running-the-duplicate-check.md`; human passes RUNBOOK §11–§14. spec **v0.84** (newest
> ADR-0082). DOB-near-miss follow-ups: #759 · #760 · #762 · #764 · #765 · #766. Trap: a new blocking key can generate
> a pair an older test asserts absent — fix the fixture date, never the assertion.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **Review / merge PR #911 (#898 + #899), then brainstorm slice 2 (re-key; #247 and #664 first).** The
>    maintainer picks the order of **#909** vs slice 5 (either gates a multi-terminal pilot) and of **#912**.
>    Owed by a human: switch the measurement cluster to `scram-sha-256` before the next RUNBOOK run (§0); **RUNBOOK §16** (sign ≤ 3 s, next-nurse
>    switch, VoiceOver/keyboard on the strip, the terminal lock). Still owed from
>    PR #895 (merged): run #838's RUNBOOK §15 (now incl. #861's current-line check) and
>    §14 checks (c) (the cause on screen; a refused reopen offers no retry; the front door's top line clears) and
>    (d) (the waiting bar: none on an ordinary press; a locked `medication_statement` → "Waiting…", then cut off by 15 s,
>    and since #893 the outcome says the window cut it off). **#894** needs the satellite rig. #891 needs a
>    brainstorm before any build. From PR #868's
>    second review, decisions: **#870** (the wrong-chart warning names the dose, not another patient's stop reason) · **#871**
>    (a current line's angioedema shows only on hover of "current" — paper-parity). From PR #865's second review: **#866** (an
>    earlier-shredded plaintext event comes back on a heal or re-fold — a decision) · **#867** (should a failed
>    re-fold block the erasure — a decision). Decisions owed: **#825** (where the window's
>    operator diagnostics live), **#850** (acknowledging a dealt-with report). Hold follow-ups: **#837** item 3 (shredding a release brings its hold back — a decision;
>    items 1–2 done) · #834 (needs a decision on shape). From PR #856's second review: **#858** (a peer release on a
>    sibling thread is shown nowhere). An idempotent re-send of a registration
>    (the same signed batch; `submit_event` is a no-op on a repeated `event_id`) is possible now that the window
>    reopens a lost connection (#807), with #649. **#728** (the Pi re-measure) can run beside. #699 stays open until the maintainer closes it.
> 2. **Decided in the #797 brainstorm (2026-10-17):** #803 (slice A PR #831, slice B PR #838, both merged; follow-ups #833 ward-level
>    list, #834 no marker for an undecryptable event) → **#804** (on link, show only where the two charts' medication
>    lists DIFFER, with continue / discontinue / withhold per line; needs #803 and **#690**). From #797's reviews:
>    **#812** (window type/test tightening) · **#808** (two outcome-line races) · **#809** (window tech debt).
>    **Open repair-path issues:** #712's items 1, 2, 4, 5 · from R7 **#790** · **#791** (both decisions) ·
>    from R6 **#784** · **#785** · from R5b **#750** · **#751** · **#753** · **#755** · **#756** · **#757** · from R5a
>    **#738** · **#739** · **#746** (a decision) · **#747** · #722 · from R4 **#730** (a decision) · **#731** ·
>    **#734**. Earlier: **#709** · **#710** · **#712** · **#714** · **#715** · **#718** · **#719** · **#720** ·
>    **#699** · **#703** · **#705** · **#706** · from R1: #690 (a decision) · #691 · #692 · #693 · #694 · #695 · #696;
>    #333, #220 and #335 gained comments.
> 3. **Human acts still owed** — see *Four things still owed* below; the repair path adds a **linked chart's open**,
>    **runbook §10's unlink** (≤ 15 s), the **live Tauri-IPC + VoiceOver pass on a linked pair** (the #699 (a) third-chart
>    unlink, a doubted pair's withheld lines, R3's person row), **RUNBOOK §11** (R5a: Review → judgement ≤ 20 s; a chart
>    open with the banner ≤ the single-chart open; the banner's VoiceOver checks), **RUNBOOK §12** (R5b: tray →
>    judgement ≤ 25 s, tray closed and open; the find ≤ 5 s with the tray OPEN; the tray's VoiceOver/keyboard checks),
>    **RUNBOOK §13** (R6: confirm a doubted link and re-link a bridged separation ≤ 15 s each; the front door's doubted
>    row; VoiceOver/keyboard — `TEMPLATE.md` has no §13 section yet), and R4's **running a supervised worker** on a real
>    node per the runbook (launchd/systemd; one worker per node; its role's `statement_timeout` ≥ 30 s).
> 4. **#620**, the only open item that can still change the wire (brainstorm first). Then **#769**, **#652 + #655**,
>    the advisory-tier **#640**, **#641**, and **#683–#686** (#685 needs the maintainer's permission to clear
>    `cairn_test`'s fixtures).
>
> **⇒ DURABLE RULES — READ BEFORE TOUCHING THE CODE THEY NAME:**
> [`developers/durable-rules.md`](developers/durable-rules.md) carries every slice's "do not undo" block (#745·#752·#740, R7, R6, R5b,
> R5a, #725, R4, R3, R1b, R2a, R2b-1, R2b-2, R1, the funnel) and the **eighteen traps**, each with its pin and its
> tempting wrong fix. A slice that adds rules adds its block there, not here.
>
> **Open from the funnel run:** #355 · #645 · #647 · #649 · #650 · #652 · #655 · #656 · #657 (multi-event rollback
> untested in both trees) · #658 · #662 · #663 · #664 · #665 · #666 · #667 · #668 · #669 · #670 · #672 (identifier
> entry) · #673 (the header shows age, not DOB) · #676 (the clerk reads `operator_chain` text) · #682 (an NFD trailing
> accent is lost — fixed in PR #776) · #683 · #684 (`1980-3-7` misses `1980-03-07` in db/046's DOB pass — a SET gap) ·
> #685 · #686.
>
> **⇒ THE NODE PLANE AND DR ARE CLOSED OUT; NO DECIDED-AND-UNBUILT ITEM REMAINS THERE.** Newest first:
> #621 ([ADR-0074](spec/decisions/0074-a-deterministic-door-failure-is-a-refusal-not-a-fault.md)), #619
> (ADR-0073), #614 + #615 (ADR-0072), #594 (ADR-0071: `restore` exits **3 INCOMPLETE**, **1 = BLOCKED**),
> #584 (ADR-0070), ADR-0069, the §1.2 restore measurement (100 003 events in 116.7 s against 600 s;
> **#512** stays open, `M > N`), DR slices 2c/2d (ADR-0067/0068). A solo clinic can lose its disk,
> restore, **open a chart** and rehearse it unattended. The durable rules are traps 1–18 in `developers/durable-rules.md`.
> - **⚠️ Citation discipline.** Rows coming back is NOT a body opening: cite
>   `restore_reads_the_clinical_plane.rs` and `restore_cli_surface.rs::a_scripted_restore_brings_the_clinical_record_back`,
>   never `dr_clinical_guarantee_gap.rs`'s counts. **Never cite ADR-0026 decision 1's promise 2** as met: no
>   node-default key tier exists (ADR-0067).
> - **The pen-release rule (#578):** a pen row carrying a wrapped DEK is released only when custody for its event is
>   SETTLED (`cairn_release_pen_row`, db/052; `pen_rows_leave_through_one_door.rs`). #585: nothing reads Postgres
>   notices. **`verify-backup` (#567)** fails `backup SHORT` only on evidence; operators run it AFTER `backup`.
>   Residuals #551 · #589 · #590 · #591 · #592 (#553 fixed in PR #774).
> - **#527/#562's triage note is false**; the real fix is **#575** (the minted recovery code reaches stderr). A retry
>   after a crashed restore must move the installed `<key>.unwrap` aside first (#596).
> - **Open decisions (none a patch):** #575 · #611 · #613 · #620. **Restore residuals:** #617 · #596–#599. **PR #601's
>   wave:** #605 · #606 · #607 · #609 · #610. **Node plane:** #268 · #301 (both `loop:needs-human`). **From
>   #619/#621:** #622 · #624 · #625 · #628 · #629 · #631 · #632 · #633 · #634. (Fixed by the 2026-10-12/13 cloud passes:
>   #602 · #603 · #604 · #608 · #616 · #569 · #626.) **Search:** #636 · #637
>   (the materialised token table — the right fix for the ~860 ms Pi floor, its own slice) · #639 · #640 · #641 · #643.
> - **Still broken, named rather than assumed away:** #549 · #552 · #502 item 4 · #101 items 2–3 · #583 · #586 ·
>   #587 · #556–#563 (ranges absorb issues that leave them; #536 is closed).

> [!WARNING]
> **⇒ CODEQL: ZERO OPEN ALERTS (measured 2026-09-12), KEPT THAT WAY BY A MODEL PACK — AND ONE HUMAN
> ACT IS DUE: make `CodeQL (rust)` a required check (#444).** PR #576 replaced default setup with a
> committed workflow + model pack (`.github/codeql/packs/cairn/codeql-models`, one `barrierModel` row per
> NAME-heuristic source, each with its reason). The flip needed the **organization-level** configuration as
> well as the repository's. **Read the alert list with `scripts/codeql-alerts.sh`, never assume it.**

> [!IMPORTANT]
> **⇒ GITHUB CLOSES ON ADJACENCY, NOT ON SENTENCES (2026-09-04).** Seven issues (#101, #115, #434,
> #441, #468, #500, #534) were closed by prose *disclaiming* the close; all reopened. Guarded by
> `scripts/check_closing_keywords.py` + `.github/workflows/closing-keywords.yml` (**#444** would make it
> required). **`fix(#500):` is SAFE** — the parenthesis breaks the adjacency. Run the script BEFORE you
> commit: the slip lives in the commit message and fixing it needs a force-push. Residuals **#547**, **#548**.

**The §5.9 thread ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)): parts A+B (authority floor + operator
surface) BUILT, enforcing nothing beyond display/emission; C+D DESIGNED, C1 the next §5.9 BUILD.** Read
**ADR-0062/0063/0064/0065** first. The authority floor is ONE predicate `cairn_claim_authority` (db/005) at ONE site
(db/048's `NOT EXISTS`) — **#245**'s first SQL counterpart, not its mirror; operator-surface §1.2 budget met (residual
**#436**). **C+D (ADR-0065)** are a custody ladder (admission → named nodes → named actors) under one invariant:
**narrowing changes the cost and noise of reading, never whether content can be REACHED** (audited break-glass at every
rung; rung-1 glass is a NETWORK act, **#498**). The node's DEK is the keyring, the floor the glass (LOCAL); custody
composes by INTERSECTION, which can EMPTY (**#499**); it narrows on `event`/`patient`, never `thread`. **C1** = rung 1
(`custody.nodes`, both doors, serve-door withholding) + audited break-glass + in-chart location signal; rung 2 is
**#496** (needs a reader identity, §5.11). Related: **#494** (ADR-0052's `event_dek` sentence vs the built table),
**#377**, **#235** (shred authorization hooks), **#236** (FTS/RAG must build on `event_clear`).
**Two §5.9 facts that outlive their slices.** `REVOKE SELECT (column)` is inert while a table-level grant stands, so
`cairn_agent` holds an explicit 23-column grant on `event_log` omitting `safety` — a new column must be granted in db/049
§8 (`safety_read_grants.rs`), and that grant is cost-raising, not a floor (**#425**, **#427**; **#432** asks whether a
node should attempt one at all). Slice-65 follow-ons: **#374**, **#378** (withdrawal rationale is clear text forever —
the UI must warn today), **#379**, **#436** (#374/#379 each need a DECISION). The `arrayref` residue is **#454**.

> [!IMPORTANT]
> **Two code traps that outlive their slices, because both look like tidy-ups.** (1) **`content_address IS NOT
> NULL` is the "did anything win" test — never `subject_kind <> 'none'`** (`none` is a legal open-vocabulary value;
> ADR-0062 E6). (2) **Unknown ranks MAX in `db/048`/`db/049`, inverting `db/040`'s `ELSE 0`.** There rank 0
> withholds reject power (safe); here it would withhold protection. ADR-0065 adds a third member that agrees for a
> DIFFERENT reason (it withholds quiet access, reachable by break-glass) — do not carry that justification into a
> site where reachability is not guaranteed.

**Four things still owed are HUMAN acts an agent cannot do:** (1) **the §1.2 stopwatch figures** — follow
[`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`](../cairn-gui/cairn-gui-tauri/results/RUNBOOK.md) into a dated
`TEMPLATE.md` copy: sections 1–7 for the med-list gestures (only the *write* half is measured, median 222 ms,
**PARTIAL**; write-cost half **#360** unwired; also a **linked chart's open**, budget ≤ the single-chart open) and
**section 8 for the front door** (find ≤ 5 s, register ≤ 20 s, live and `--mock`; db/044's `gesture_kind` CHECK still
refuses a registration timing row until widened); (2) **the accessibility pass** — a live VoiceOver run through the
runbook's checks, front door included, keyboard-only, and on a **linked pair** (the source-chart label, #691; R3's
person-row label); DOM assertions automated by **#332**; (3)+(4) **make CI jobs REQUIRED status checks** (**#444**,
admin-only — "clippy + cargo test (cairn-gui)", "cargo doc (API surface)", and `CodeQL (rust)`). **If a measurement
falls outside its budget, that is the finding — file an issue, never adjust the budget.**

**Other build candidates** (nothing blocks a choice): the **drugref term→anchor lookup** (the §9 advisory tier; closes
the coded↔uncoded case ADR-0059 decision 5 leaves open; needs a connection-model decision; `safety_class_map` its empty
seam) · **the node/actor plane's two divergences** — db/007 fail-closes on an unmappable type (**#301**), and the node
puller skips-and-advances a verifiable refusal where the clinical one pens it (**#268**); both `loop:needs-human`.

**Standing gate:** whole-project review cycles repeat periodically; no release for clinical use before repeated cycles
pass cleanly. Last full pass 2026-07-15 (#187–#217), fully closed; the runnable clinical surface has never been
through one — include it next.

> [!TIP]
> **The tech-debt loop is stopped, and stays stopped** (maintainer decision, 2026-08-09) while a human session
> holds the main repo — they contend on one cargo lock and one `test_serial_guard` advisory lock. **A live IDE
> contends the same way** (rust-analyzer holds `target/`); use a scratch `CARGO_TARGET_DIR=/tmp/…`, never kill
> the IDE. Loop gaps **#326**, **#312**, **#322**.

---

**Session date:** 2026-11-01 (#898 + #899, PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)) · 2026-10-31/11-01 (GUI milestone: sign-in design, plan, slice 1 built, PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897)) · 2026-10-31 (#893, PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895)) · 2026-10-30 (#875 option 2, PR [#892](https://github.com/cairn-ehr/cairn-ehr/pull/892)) · 2026-10-30 (#810, PR [#890](https://github.com/cairn-ehr/cairn-ehr/pull/890)) · 2026-10-30 (#882 + #885, PR [#887](https://github.com/cairn-ehr/cairn-ehr/pull/887)) · 2026-10-29 (#878, PR [#886](https://github.com/cairn-ehr/cairn-ehr/pull/886)) · 2026-10-29 (#880 + #881, PR [#883](https://github.com/cairn-ehr/cairn-ehr/pull/883)) · 2026-10-29 (#876, PR [#879](https://github.com/cairn-ehr/cairn-ehr/pull/879)) · 2026-10-28 (#875 option 1, PR [#877](https://github.com/cairn-ehr/cairn-ehr/pull/877)) · 2026-10-28 (#807, PR [#874](https://github.com/cairn-ehr/cairn-ehr/pull/874)) · 2026-10-28 (#855 + #837 item 1, PR [#873](https://github.com/cairn-ehr/cairn-ehr/pull/873)) · 2026-10-27 (#869, PR [#872](https://github.com/cairn-ehr/cairn-ehr/pull/872)) · 2026-10-26 (#861, PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)) · 2026-10-24 (#863 + #864, PR [#865](https://github.com/cairn-ehr/cairn-ehr/pull/865)) · 2026-10-23/24 (#857 + #859, PR [#860](https://github.com/cairn-ehr/cairn-ehr/pull/860)) · 2026-10-23 (#835 + #854, PR [#856](https://github.com/cairn-ehr/cairn-ehr/pull/856)) · 2026-10-23 (#843 + #849, PR [#853](https://github.com/cairn-ehr/cairn-ehr/pull/853)) · 2026-10-23 (#848, PR [#851](https://github.com/cairn-ehr/cairn-ehr/pull/851)) · 2026-10-22 (#846, PR [#847](https://github.com/cairn-ehr/cairn-ehr/pull/847)) · 2026-10-22 (#844 + #836, PR [#845](https://github.com/cairn-ehr/cairn-ehr/pull/845)) · 2026-10-22 (#839 + #840 + #841, PR [#842](https://github.com/cairn-ehr/cairn-ehr/pull/842)) · 2026-10-21 (#832, PR [#838](https://github.com/cairn-ehr/cairn-ehr/pull/838)) · 10-21 (#803 slice A, PR [#831](https://github.com/cairn-ehr/cairn-ehr/pull/831)) · 10-20 (#828, PR [#830](https://github.com/cairn-ehr/cairn-ehr/pull/830)) · 10-20 (#826, PR [#829](https://github.com/cairn-ehr/cairn-ehr/pull/829)) · 2026-10-19 (#822 + #823, PR [#827](https://github.com/cairn-ehr/cairn-ehr/pull/827)) · 2026-10-18 (#817, PR #821) · 10-18 (#815 + #818, PR #819) · 10-18 (#814, PR #816) · 10-18 (#811, PR #813) · 10-18 (PR #805's second review, fixed) · 2026-10-17 (#797, PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805)) · 10-17 (#799 · #800 · #801, PR [#802](https://github.com/cairn-ehr/cairn-ehr/pull/802)) · 10-17 (PR #798's second review round) · 2026-10-16 (#745 · #752 · #740, PR [#798](https://github.com/cairn-ehr/cairn-ehr/pull/798)) · 10-16 (#793 · #737, PR #795) · 10-16 (#781 · #789 · #732, PR #794) · 10-16 (R6 residuals, PR #792) · 10-15 (R7, #742 + #743 part 2, PR #788) · 10-14/15 (R6, #716 + #723, PR #777) · 10-12/13 three cloud tech-debt passes (PRs #770–#776, ADR-0080) · 10-11 DOB near miss (PR #758, ADR-0079) · 10-08/09 R5b (PR #749, ADR-0078) · 10-06/07 R5a (PR #735) · 10-05 #725 (PR #733) · 10-04/05 R4 (ADR-0076 decision 7, PR #724) ·
10-03 R3 (PR #721) and R1b (PR #717) · 09-30 R2b-2 (ADR-0077, PR #711) · 09-29 R2b-1 (PR #707) · 09-27 R2a (PR #698,
`db/055`) and R1 (ADR-0076, `db/054`, PR #688) · 09-26 #671 (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · earlier:
ROADMAP. · **Spec:** **v0.86** (newest ADR-0084; v0.85 ADR-0083; #857 and #861 needed no ADR — a projection-side fix, no wire change;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **62** (`db/062`, PR #911; `db/061` PR #897; `db/060`, PR #860; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
complete; **first production clinical surface RUNNING** — `cairn-node` plus a Tauri 2 window: the funnel front door (one
row per person) onto a medication chart that reads linked charts as one; a per-node matcher worker proposes duplicates,
a banner on the chart shows each one for a human to judge, a tray at the front door lists them all, and a
doubted link can be confirmed from the window.

**Built so far** (orientation only; ROADMAP + ADR log + git carry the detail): demographics 1–5 · §5.2 advisory Python
matcher · §5.7 identity core C1–C5 (C5+ `reattribute` waits on a clinical-note surface) · §5.4 John-Doe (§5.12
push-alert open) · §5.3/§5.8 funnel (ADR-0061; precedence #345 at db/005 step 8b; ranking ADR-0075; one row per
PERSON since R3, PR #721) · `clinical.medication` 1–6b under born-sealed bodies (ADR-0052) + per-write human authorship
(ADR-0053 — grading half-live until #245) · §5.9 stream through its read surface · med-list node tier (read + whole-list
sign-off over a linked set, R1) · human link/unlink judgements (`chart_link`, attested-first `patient_link`, R2a) · the
compare-and-link panel (R2b-1, PR #707) + unlink from a record (R2b-2, PR #711) + a doubted set withholds every line
not on the opened chart (R1b, PR #717) · the commit-time duplicate check (`db/056`, `cairn-matcher watch`, `cairn-node
duplicate-check`; R4, PR #724; per-chart range blocking anchored, #725) · the possible-duplicate banner + "Different
people" (`db/057`; R5a, PR #735) + the front door's possible-duplicate tray (R5b, PR #749) · confirming a doubted link
from the window and a doubted row at the front door (R6, PR #777) · a "running" line on a checked chart and a note on an unverified link (R7, PR #788) · generic reprojection (ADR-0057; ADR-0070) ·
ADR-0056 admit-uninterpreted floor · **the L3 reference UI** `cairn-gui/` (standalone workspace, one-way GUI → crates;
`cairn-gui-tauri`, the iced shell FAILED a11y, spike 0004; plain JS, no npm); pane/routing/freshness state machine
tested but **not wired**.

---

## Recent sessions — what to carry forward

ROADMAP carries the per-slice narrative and every open issue number; this keeps only lessons that generalise (each
slice's own rules are its block in `developers/durable-rules.md`). **Every session since 10-16 found HANDOVER stale on
arrival** (a merged PR still "awaiting") — house rule 8's check is not optional.

### 2026-11-01 — #898 + #899 (PR [#911](https://github.com/cairn-ehr/cairn-ehr/pull/911)): brainstorm → design → plan → SDD

Found #897 merged and HANDOVER saying "review it" (stale again). Brainstormed with the maintainer (no assumption
about where a machine sits or who installs it; long term every terminal is a node; no override for either refusal);
7 tasks, each reviewed; filed #909, #910, #912.
- **⇒ An issue's suggested fix is a claim** (again): `SET ROLE` after the schema load closes nothing — `RESET ROLE`
  undoes it and the owner's password stays in a copyable string. The hole is the credential HANDED OUT.
- **⇒ `has_*_privilege` and `pg_has_role(…, 'USAGE')` see only INHERITED power.** A `WITH INHERIT FALSE` member can
  still `SET ROLE` into the owner or `cairn_node`. Judge every fact over every role the login is a MEMBER of
  (the plan's own `'USAGE'` was the defect; the task review caught it). **And judge `session_user`, never
  `current_user`:** a connection string's `options='-c role=…'` changes `current_user` without changing the login — the
  final review passed the check on the OWNER's login that way (fixed before merge).
- **⇒ A permission walk that runs a read on EMPTY data proves nothing** — search before anything is registered
  returns after its first query. Drive each call to success on a fixture that reaches its deep path.
- **⇒ An owner verb that takes a role name must refuse an existing role that is not exactly what it makes** — the
  first cut would have reset a superuser's password. **`pg_hba_file_rules` is the FILE**, not the loaded rules
  (compare `pg_stat_file(hba_file).modification` with `pg_conf_load_time()`).
- **⇒ Removing a call removes its side effects:** the window's `connect_and_load_schema` was, incidentally, the only
  routine runner of the deferred-event re-adjudication (#912). Grep what a function does before dropping its caller.

### 2026-10-31 → 11-01 — GUI sign-in slice 1 (PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897)): brainstorm → design → plan → SDD

Brainstormed with the maintainer (shared ED terminal, ~10 nurses: open for reading, a personal signature per act —
which turned out to be ADR-0008 already); planned; built by subagent-driven development, 8 tasks, every task reviewed,
a whole-branch review and two fix waves. Filed #898–#902.
- **⇒ A transactional counter is no throttle against a raw-SQL client.** The first vault back-off was an ordinary
  UPDATE: `BEGIN; guess; ROLLBACK;` revealed the answer and undid the count. Count in state a rollback cannot undo
  (sequences), BEFORE revealing anything — and with `setval`, not `nextval`, whose 32-value WAL pre-log replays phantom
  failures after a crash. A clock that stepped BACK must allow, not lock out.
- **⇒ Check WHICH role the client actually connects as before promising a role-based protection.** The vault's
  grants are right and tested — and the window connects as the owner (#898). The ADR now says so.
- **⇒ An Argon2-latency gap is a race window.** A release in flight stored its key after Lock; a generation counter
  stamped at the wipe fixed it. **An empty passphrase must never reach a counting door** — two paths sent one.
- **⇒ A rule that normalises in SQL must also BUILD the derived value in SQL** (the pinned handle), or a raw-SQL caller
  and a case variant mint a second actor.
- **⇒ Visibility again:** the signer strip sat inside the hidden `#chart-view`; only the headless walk saw it.

### 2026-10-31 — #893 (PR [#895](https://github.com/cairn-ehr/cairn-ehr/pull/895)): a cut-off command says the window cut it off

Found HANDOVER stale on arrival again (#892 merged, listed as awaiting); no decision had landed; the maintainer
picked #893. TDD (the search-through-a-lock test red first); the cease pin mutation-checked. A five-agent review
then found the sentence false for a hold of many ANSWERED statements ("did not answer" → "was not finished"), three
failures that come back inside an `Ok` outcome and so skipped `explain` (sign-off failed lines, "Different people"
pairs, worklist sides), and the explain-after-drop rule stated backwards; all fixed, plus the per-function source
guard (mutation-checked on both crates).
- **⇒ An aborted tokio-postgres task gives every pending query the same `Error::closed()`** (tokio-postgres
  `client.rs`, `Responses::poll_next`: the response channel's sender is dropped), so no error can carry its cause
  from the source: the marker is a context layer added by whoever holds BOTH the guard and the error. Read the cut
  before the guard drops — the flag is the CLIENT's, so later a next hold's cut could set it; a transaction that
  borrows the guard mutably takes a `CutWatch` first (`record_links`).
- **⇒ "Every call site" is not "every error path".** An orchestrator that keeps going after a failure returns it
  INSIDE `Ok` (a failed line, a judged pair, a worklist side), often already as text; wrapping the `Err` arm misses
  it. Look at what the outcome type carries, not only at `map_err`.
- **⇒ Grep for a call across line breaks.** `\.lock()\.await` missed four sites (unlock, link, unlink, confirm) that
  rustfmt had split as `.lock()\n.await`; and my first mutation matched nothing for the same reason.
- **⇒ A `"{text}"` assert message reads like a crash**: a panic printing `connection closed` was my own assert, not
  the reader dying. Read the panic's line before debugging the fixture.

### 2026-10-28 → 10-30 — #807 (PR #874), #875 (PRs #877, #892), #876 (PR #879), #880 + #881 (PR #883), #878 (PR #886), #882 + #885 (PR #887), #810 (PR #890), #855 + #837 item 1 (PR #873)

Condensed 2026-11-01; ROADMAP has each, durable-rules carries each slice's rules. All TDD, each pin mutation-checked.
#892's brainstorm filed **#891** (an open chart goes first in sync). Decisions still owed then: #870 · #871 · #866 ·
#867 · #825 · #850.
- **⇒ A timeout around a tokio-postgres call does not free the connection** — abort the connection TASK
  (`connect_with_task`); **aborting the client never tells the server** — `pg_terminate_backend` the old backend before
  trusting a read; a `tokio::time::timeout` over a guard-holding future is the same trap — time a spawned task. A
  deferred constraint trigger that sleeps makes COMMIT the slow statement (how `answer_limit.rs` tests a cut commit).
- **⇒ An operator procedure is a claim** (three times): walk each RUNBOOK step against the JS state it needs; a renamed
  database refuses under any auth where `REVOKE CONNECT` does not. **A new timeout around a call with its own must be
  LONGER than it.** An issue's suggested fix is a claim: run it against the test that pins the output (#880).
- **⇒ Routing a new cause into an existing variant reaches every renderer of it** — grep every `match` arm and every
  `to_string()`. **A `From` impl makes `?` compile where it is wrong** — pin the sites that must not use it. **Say what
  the USER must do, not what the machine will.** State touched only under a lock lives INSIDE it (`MutexGuard::map`).
- **⇒ Check a mutation APPLIED before trusting the green after it** (rustfmt reflows lines; print the mutated line). A
  counting source guard needs fixtures of its own. A root crate add moves `cairn-gui/Cargo.lock` too. `pub use child::*`
  keeps `pub(crate)` paths through a split; a moved doc link breaks only in `cargo doc`.
- **⇒ A floor `P0001` that may be stale is re-asked, never parsed** (#855); before a park-on-the-lock race test, check
  nothing else can park the victim; two humans with the same determinants are ONE actor (#152); an ADR consequence a
  later fix closes gets a dated erratum (ADR-0082 E1).

### 2026-10-18 → 10-26 — the W and its follow-ups (PRs #813–#868)

Condensed 2026-11-01; ROADMAP has each, durable-rules carries each slice's rules. PRs: interrupted commits #813–#830
(#811, #814, #815, #820, #826, #828), the W #831 + #838 (#803, #832), #842 (#841), #847 (#846), #851 (#848), #853
(#843 + #849, ADR-0082), #856 (#835 + #854), #860 (#857 + #859), #865 (#863 + #864; filed **#866**, **#867**), #868
(#861; filed #869, **#870**, **#871**). Not done then: #812 items 7–10, #814; nothing reads Postgres notices (#585).
- **⇒ A new meaning gets a new field, not a status-dependent reading of an old one**; a new state goes BESIDE an enum
  other code reads as "on the list"; a new model field breaks its literals, not its readers — grep the SIBLING fields'
  readers. **Widen a set-returning read with a sibling predicate**, not a new column (`CREATE OR REPLACE` refuses a
  return-type change). A new projection beside a one-winner one: a second apply fn, a generation bump, the shred
  delete, BOTH registry counts, the fixture truncate lists.
- **⇒ Walk every remedy, absence or future-event sentence through every state it is shown in** ("Release at 16:00"
  on a hold ending 14:00; "record it again" beside a listed drug); fixing what a list SHOWS changes what every
  sentence ABOUT it may claim. When a fix makes a caller-chosen claim true everywhere, delete the choice.
- **⇒ "Never over-erase" is half a rule** — a scrub of a winner-takes-all projection owes a re-fold; never SKIP a
  locked sibling (serialize and wait); compare ids canonically; raise isolation only for reads. "Its clear view is
  gone" is true only of a SEALED event. A flag written at arrival sees only what had arrived — read at list time.
- **⇒ The logical clock is not a wall clock** (#803); a fallback in the floor bypasses the rule the client follows;
  an `IF NOT EXISTS` index never picks up an edited predicate. **A floor `P0001` carries no scope** — reuse the
  floor's predicate as a Rust pre-check; a maintainer decision in an issue can still reverse an ADR sentence.
- **⇒ An outcome that MAY have written settles like a success**; only an ERROR answer proves a rollback; a flag
  carried by one async call dies with it — make it window state. A real mouse finds what `element.click()` cannot;
  hiding a button hides what its refusal said. A surviving mutation can mean a second witness, not a dead predicate.
- **Mechanics:** read the projection before an order assertion ("the shown stop" is the NEWEST RECORDED); a
  source-guard slice at `at - N` bytes panics on an em dash (`floor_char_boundary`); zsh does not word-split `$T`;
  env exports do not survive between tool calls (a 0.00 s DB "ok" is a self-skip); `cargo doc -D warnings` catches
  what clippy cannot.

### 2026-10-04 → 10-17 — R4–R7 (PRs #724, #733, #735, #749, #777, #788), cloud passes (PRs #770–#776, #779), #797 (PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805))

Condensed 2026-10-31; ROADMAP has each slice. #797: brainstorm → plan → SDD; the maintainer's rule (durable-rules):
*tell the clinician only what changed from ANOTHER source since they took the chart*; #803/#804 decided there. Also
#745 · #752 · #740 (PR #798), #799 · #800 · #801 (PR #802), PR #792 (R6 residuals #787 · #786 · #783 · #782), splits
#781 · #789 · #732 · #793 · #712 item 3 · #737 · #796 (PRs #794, #795); R6 = #716 + #723; R7 = #742 + #743 part 2.
Filed then: #781 · #782 · #783 · #784 · #785 · #786 · #787 · #789 · #790 · #791 · #755 · #756 · #757 · #778; commented #718 · #738; decisions left: #649 · #613 · #628 · #780
(`casefold()`), Turkish dotless `ı` (the ADR-0014 profile's job); #427 is lower than it reads (#424); #220 gained a test; #626's `do_pull` non-P0001 arm open.
- **⇒ Route a cause by TYPE end to end** (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`). **A
  baseline that must survive overlapping reads is window STATE.** **Rule on the evidence, not on who proposed it**
  (a reviewer's counter-case beat my `applyOwnAct` formula). Pin "after apply, before commit" on BOTH sides.
- **⇒ Walk the old code with the same script** (a walk passing only on the fix proves the rig). Webview JS is pinned
  without a JS runner (`source_guard::js_fn` / `js_const`); pin the exact expression (`a && b`, not either half).
  Draw unless something newer is DRAWN; say a failure only if newest STARTED.
- **⇒ A remedy that names a command is a claim: run it, name the LIGHTEST** (#787); keep one in-memory truth (#786);
  prove a pure move by the sorted test LIST; punctuation is not speech (VoiceOver reads "—" as a pause).
- **⇒ #726: a connect replays the schema only when it changed** (ADR-0080) — never "simplify" the gate to the
  generation number; tests repairing the schema call `db::force_schema_replay`. A case-mapping change on one side of
  a subset invariant breaks it (#775); COMMITTED IS NOT VOUCHED (#556); a judgement carries what was compared (#708);
  an unknown outcome is its own ROOT marker (#713); a per-row lock cannot guard a whole-log scan (#773); an issue's
  fix can be wrong about MVCC (#560) or predate a guard (#608 vs #633); a count is not a set (#553); a lenient arm can
  be worse than silent (#768, #606).
- **⇒ A title is a claim too** (ADR-0078); "the front door never waits" in every UI state; a test whose two
  orientations coincide pins neither; count steps against the as-built window (#754); widening "who may be proposed"
  checks every CLOSED status; read the design's filter against its issue (#700); diagnose before choosing a candidate
  fix (#725, `EXPLAIN ANALYZE`); a CI job with a different dependency set cannot be gated locally.
- **⇒ ROLLBACK HAZARD:** with `SCHEMA_GENERATION` unchanged, an OLDER binary on a database this branch loaded fails
  "cannot change return type" on db/056 — `DROP FUNCTION cairn_duplicate_check_status();` before switching back.
- **Cloud rig** (runs the FULL DB gate): micromamba + conda-forge `postgresql=18`, `cargo install cargo-pgrx --version
  =0.18.1 --locked`, `cargo pgrx init --pg18 <conda pg_config>` with an `xlocale.h` shim in `CFLAGS` and
  `BINDGEN_EXTRA_CLANG_ARGS`, `initdb` as `postgres`, `CREATE ROLE root SUPERUSER`; never pre-create `cairn_pgx` in
  `template1`; `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`; `target/` ~23 GB. A new db/tests
  mirror needs `\ir _scratch_database_guard.sql`.
- **Mechanics:** `cargo doc -D warnings` reads `<date>` as an unclosed HTML tag; a struct literal gaining a field
  breaks every test that builds it; a callee's `rollback()` undoes a test's `SET ROLE`; editing an `include_str!`'d
  `db/*.sql` relinks every cairn-node test binary; stub payloads are dumped from Rust, never hand-written; a test
  assert must never echo identity-built text (CodeQL failed #795 after merge).

### 2026-08-20 → 10-03 — restore, node plane, doors, search, funnel UI 2a–2c, #671, repair path R1–R3

Narrative: ROADMAP; durable rules: `developers/durable-rules.md` (traps 7–18, per-slice blocks); each plan carries its review ledger.
- **⇒ A remedy or count sentence is a safety claim — walk it through every case it is shown in** ("either judgement
  lifts this hold" was false in three reachable cases). A plan can mandate a false claim and silently drop a design test
  — diff the two test lists; fact-check an ADR sentence by sentence against the SQL; ask what the desk will do before
  designing what it must attest (#671); a design's quantitative assumption is a claim.
- **⇒ "Inside the transaction" is not "serialized"** — take the lock every writer holds (CARNLK) first; test by parking
  the victim on it (`pg_stat_activity`), never a sleep. Recorded is not took effect. A migration that changes a
  projection's order must re-decide what the old order decided (db/055; #703). A predicate keyed on standing LINKS
  misses standing UNLINKS; keep an old flag, add reasons beside it, route every reader through one method.
- **⇒ A test that cannot see its own predicate proves nothing — mutate to check** (a mutation is the RED phase of a pin
  over shipped behaviour; SQL predicates need a REBUILD). A guard only ever green proved nothing (#586); copying a guard
  spreads its fail-open (#608); "untestable" is a claim — try a `SET ROLE` seam; a DB-suite "ok" can be a self-skip; a
  headless walk asserts visibility; an issue's scenario and scope are CLAIMS; a door returning `Ok` is not the record
  coming back; content-addressing over unsigned bytes is not content-addressing (#620).
- **Mechanics:** a sweep without all three DB strings is not a sweep; grep every claim about the tree (#654); an
  atomicity probe that never reaches the server is decoration (#657); a new DB suite in a non-root tree runs nowhere
  until wired (#656); a derived truncate list is only its predicate (#658); a red gate can be a predecessor's (#583); a
  new definer writes `SET search_path = public, pg_temp`; `nohup … &` and `cmd; echo exit=$?` lie; a new ADR needs its
  `mkdocs.yml` nav line. Filed: #712 · #718 · #719 · #720 · #722; #335 comments. Still open: **#569**, **#598**, #624.

**⇒ OPEN ISSUES OLDER SESSIONS OPENED, INDEXED** (never drop an open number while condensing):

[#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) · [#327](https://github.com/cairn-ehr/cairn-ehr/issues/327) · [#394](https://github.com/cairn-ehr/cairn-ehr/issues/394) · [#402](https://github.com/cairn-ehr/cairn-ehr/issues/402) · [#406](https://github.com/cairn-ehr/cairn-ehr/issues/406) · [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407) · [#408](https://github.com/cairn-ehr/cairn-ehr/issues/408) · [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409) · [#413](https://github.com/cairn-ehr/cairn-ehr/issues/413) · [#420](https://github.com/cairn-ehr/cairn-ehr/issues/420) · [#422](https://github.com/cairn-ehr/cairn-ehr/issues/422) · [#428](https://github.com/cairn-ehr/cairn-ehr/issues/428) · [#430](https://github.com/cairn-ehr/cairn-ehr/issues/430) · [#431](https://github.com/cairn-ehr/cairn-ehr/issues/431) · [#447](https://github.com/cairn-ehr/cairn-ehr/issues/447) · [#458](https://github.com/cairn-ehr/cairn-ehr/issues/458) · [#463](https://github.com/cairn-ehr/cairn-ehr/issues/463) · [#464](https://github.com/cairn-ehr/cairn-ehr/issues/464) · [#470](https://github.com/cairn-ehr/cairn-ehr/issues/470) · [#483](https://github.com/cairn-ehr/cairn-ehr/issues/483) · [#484](https://github.com/cairn-ehr/cairn-ehr/issues/484) · [#485](https://github.com/cairn-ehr/cairn-ehr/issues/485) · [#487](https://github.com/cairn-ehr/cairn-ehr/issues/487) · [#488](https://github.com/cairn-ehr/cairn-ehr/issues/488) · [#490](https://github.com/cairn-ehr/cairn-ehr/issues/490) · [#491](https://github.com/cairn-ehr/cairn-ehr/issues/491) · [#492](https://github.com/cairn-ehr/cairn-ehr/issues/492) · [#494](https://github.com/cairn-ehr/cairn-ehr/issues/494) · [#504](https://github.com/cairn-ehr/cairn-ehr/issues/504) · [#505](https://github.com/cairn-ehr/cairn-ehr/issues/505) · [#506](https://github.com/cairn-ehr/cairn-ehr/issues/506) · [#507](https://github.com/cairn-ehr/cairn-ehr/issues/507) · [#508](https://github.com/cairn-ehr/cairn-ehr/issues/508) · [#509](https://github.com/cairn-ehr/cairn-ehr/issues/509) · [#513](https://github.com/cairn-ehr/cairn-ehr/issues/513) · [#518](https://github.com/cairn-ehr/cairn-ehr/issues/518) · [#521](https://github.com/cairn-ehr/cairn-ehr/issues/521) · [#522](https://github.com/cairn-ehr/cairn-ehr/issues/522) · [#529](https://github.com/cairn-ehr/cairn-ehr/issues/529) · [#530](https://github.com/cairn-ehr/cairn-ehr/issues/530) · [#543](https://github.com/cairn-ehr/cairn-ehr/issues/543) · [#545](https://github.com/cairn-ehr/cairn-ehr/issues/545) · [#557](https://github.com/cairn-ehr/cairn-ehr/issues/557) · [#558](https://github.com/cairn-ehr/cairn-ehr/issues/558) · [#559](https://github.com/cairn-ehr/cairn-ehr/issues/559) · [#560](https://github.com/cairn-ehr/cairn-ehr/issues/560) · [#561](https://github.com/cairn-ehr/cairn-ehr/issues/561)

---

## Read these first (the durable state)

CLAUDE.md carries the document hierarchy in full; this adds only what it does not.
**`docs/developers/durable-rules.md`** — the per-slice rules and the numbered trap list every session must hold (moved out
of this file in R7; a new rule or trap goes there, never here). **`docs/spikes/`** — 0001 (walking
skeleton — Bet A ✓ → ADR-0015; Bet B ✓ twice); 0002 (advisory-actor, C1–C5 ✓ → ADR-0029/0030); 0003 (Postgres on
Android, G0–G3 ✓); 0004 (iced UI — FAIL on a11y → Tauri 2). **`docs/case-studies/0001`**: 16 GP-software failure modes,
all absorbed, **0 new architecture**. **`docs/ecosystem/`** 0001, 0003 · **`docs/principles/`** — mission/governance.
Code workspace: `/crates` (`cairn-db-keepalive`, `cairn-event`, `cairn-keystore`, `cairn-medium`, `cairn-wire`, `cairn-sync`, `cairn-node`,
`cairn-medication-view`, `cairn-patient-search`), `/extensions` (`cairn_pgx`), `/db`, `/cairn-gui` (separate workspace);
`poc/` is frozen historical spikes. **Where the build is:**

- **First federating node** (ADR-0017) — `cairn-node`: Ed25519 keystore, pairing/`peers`/`unpeer`, mTLS pinned to the
  trust set, set-union `node_event` sync, `db/007`'s doors with a deny-all admission gate, genesis-stable `node_id`.
  Custody travels, the clinical event log reaches the medium, and a restore reads it back (ADR-0067); optional escrow
  rungs (Shamir/QR/TPM) remain. **Dual-identifier discipline** (ADR-0031): the canonical plane (UUIDv7 + multihash) is
  the only identifier on the wire; the projection plane may intern node-local `bigint` surrogates (`db/008`).
- **Test rig:** DB-gated tests need local PG18 + `cairn_pgx`, self-serializing via a Postgres advisory lock
  (`db::test_serial_guard`). **Advisory locks are scoped PER DATABASE** (#467; ~124 comments still say otherwise,
  **#476**), so every caller takes the guard against `CAIRN_TEST_PG`. The tech-debt loop (`/techdebt-loop`,
  `/techdebt-next`) is STOPPED (above).

---

## Open threads — pick one (today's-work menu)

**Desk-doable now (no external dependency):**
- **⇒ The repair path: the open repair-path issues** — see ⇒ NEXT.
- **⇒ DR — closed and rehearsable end to end.** Not there, though a reader expects it: **2d does NOT drive
  `cairn-sync`'s puller through `MediumTransport`**; **the per-peer quarantine quota does not apply to a
  restore-originated pen** (`restore_pen_is_uncapped.rs`). Filed by the chain, beyond ⇒ NEXT's node-plane list: #525 ·
  #541 (no CI job compiles `cairn_pgx`'s `pg_test`) · #531/#329 (decompose `cairn-sync/src/main.rs` — maintainer picks
  one) · #532 · #534 · #535 · #537 · #538 · #624–#626. (Ranges absorb issues that leave them: re-check against GitHub.)
- **§5.9 parts C/D** (#232) — above.
- **`clinical.medication` — slices 1–6b DONE, read over a linked set since R1.** Next: #690 (reconciling across linked
  charts), the **drugref term→anchor lookup**, fuzzy/automatic reconciliation + a Tier-A dictionary, structured
  sig/frequency, correcting a dose event's effective date. Cross-cutting debt **#185**. Spine: `db/031`–`db/035`,
  `db/041`, `db/042`, `db/054`. **No allergy stream exists yet** — the repair path's "active medications" becomes
  "allergies and active medications" when one does.
- **Demographics / matcher / identity — next slices** (`db/010`–`db/030` + `cairn-event::demographics`). B3-driven: gold
  set, locale packs, hub-tier duplicate sweep (R4 is its per-node sibling), proposal retraction. Identity: C5+
  `reattribute` (waits on a clinical-note surface); §5.12 push-alert. Deferred **#168**, **#287**; rest in ROADMAP.
- **⇒ Test env — `scripts/run-db-gated-tests.sh` is the ONE command for the local gate**, the only one catching all
  three demonstrated hiding modes (fail-fast · a piped exit status · a cross-crate suite `-p <crate>` never builds).
  **The CLUSTER is discovered** (`scripts/pg-target.sh`, PG ≥ 18; set `PGPORT` to name one). **Cost depends on what
  changed — measure, never budget from a figure here.** Without the three env strings the DB suites self-skip, so a
  bare run FAILS unless `CAIRN_ALLOW_DB_SKIP=1` is declared (#450). Mirrors are DESTRUCTIVE, refusing any DB lacking the
  `cairn_scratch_database` marker (#169). Matcher: `cd matcher && CAIRN_TEST_PG=… uv run --extra pipeline pytest` (uv,
  never pip; gap **#314**). `clinical_pull` flakes under a full run: `--test-threads=2`.
- **Clinical case-mining** — historically the highest-signal generative mode. Bring a real ED/hospital failure mode;
  record in [`docs/case-studies/`](case-studies/README.md). Open from Case 0001: **① re-affirmation-without-change
  currency** (#163); **② open-loop/obligation**, a projection surfaced by salience not a modal; **③ impossible-vs-uncertain**
  for the in-DB floor.
- **Landing-page polish** — a non-developer page for the generated site (`web/`).

**Blocked on hardware / external access:** **Bet B — Pi compute-cost run**
([Spike 0001 §9](spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)): PASS twice; fold the un-caveated B4 number into
ADR-0015 to drop "provisional"; **#272** (reproject bench on the Pi). **easyGP session** — port
[ADR-0020](spec/decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md)'s deferred items with
live schema access (`rx!`/`tx!` parser + state machine; formulation/drug source + forced-manual rule table; prefetch
warming daemon); pre-read `scratch/ui-sketches/easygp-prefetch-notes.md`. **Byte-tier throughput lever** — connection
reuse / persistent streaming instead of one TCP connection per slice.

---

## Parked · Working context

- **Parked (don't re-litigate without new reason):** legal entity & jurisdiction — deferred until momentum/funding
  geography is clearer; trademark registration — principle recorded, instrument deferred.
- **CLAUDE.md carries the working context in full and is loaded every session.** Canonical docs win.
- **Governance done** ([GOVERNANCE.md](principles/GOVERNANCE.md) + root `CONTRIBUTING.md`): AGPL-3.0
  inbound=outbound, DCO, **no CLA**; mission as tie-breaker. Names/domains/packages secured.
