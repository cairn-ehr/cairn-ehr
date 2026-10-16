# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ R6'S FOUR RESIDUALS ARE MERGED (PR [#792](https://github.com/cairn-ehr/cairn-ehr/pull/792), 2026-10-15/16):
> #787 (`standing_link` panicked on a NULL address), #786 (the doubt mark on `CandidateList`'s wire), #783 (the bridge
> remedy's door), #782 (the lock hint's script-order race). **BUILT ON PR [#794](https://github.com/cairn-ehr/cairn-ehr/pull/794) (2026-10-16, branch
> `refactor/789-781-732-file-splits`), AWAITING THE MAINTAINER'S REVIEW AND MERGE:** #781 + #789 (every `link/` and
> `duplicates/` file in `cairn-gui-tauri` under 500 lines, pure moves — the same 307 test names before and after) and
> #732 (the matcher fixture truncates `patient_chart`; the version re-queue test asserts equality). Filed **#793** (three
> more files over 500 lines).** The whole repair path (#679 · #680 · #681) is MERGED through R7
> (PR [#788](https://github.com/cairn-ehr/cairn-ehr/pull/788), 2026-10-15), plus #775/#556 (PR #779). R7, as the
> maintainer decided it: **#742** — a checked chart's banner shows the node's "running" line while a `change` notice
> waits (`config` notices alone never trigger it); **#743 part 2** — an un-attested link whose pair has a `degrade_hold`
> identifier finding (computed now) carries a NON-holding note on its links-list line (db/016: a soft veto never demotes
> a link, so it is deliberately outside `cairn_chart_set_doubts`). No ADR, no schema generation (db/054 and db/056 in
> place). Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (every slice designed +
> as built); R4's worker runbook `docs/developers/running-the-duplicate-check.md`; the human passes are
> `cairn-gui-tauri`'s RUNBOOK §11 (banner; R7's check 3), §12 (tray) and §13 (R6's confirm; R7's check 4). spec
> **v0.82** (newest ADR-0080). DOB-near-miss follow-ups: #759 · #760 · #762 · #764 · #765 · #766. Trap: a new blocking
> key can generate a pair an older test asserts absent — fix the fixture date, never the assertion.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **PR [#794](https://github.com/cairn-ehr/cairn-ehr/pull/794)** (#781 · #789 · #732, above): read `gh pr checks`, then review and merge. **#728** (the Pi re-measure; now also the
> find with the tray OPEN) can run beside. #699 stays open until the maintainer closes it.
> 2. **Open repair-path issues:** from PR #794 **#793** (three more `cairn-gui-tauri` files over 500 lines) · from R7 **#790** (the
>    "running" line on every checked chart through a rebuild/restore backlog — a decision) · **#791** (a chart whose
>    check keeps failing keeps that line on every checked record of a BUSY node, never reaching "behind" — a decision) · from R6 **#784** (a verdict is not re-checked against identity
>    facts or vetoes that changed while the panel was open) · from R6's review **#785** (type tightening: `RecordsShown`
>    constructors, a `LinkState` enum, `ShownPair` as an enum) · from R5b **#750** ("Registered more recently" trusts a
>    UUIDv7 time) · **#751** (accepted pairs read in a second statement, a TOCTOU) · **#752** (unordered tray reads, a
>    stale list after return, a context-free read error) · **#753** (an un-attested unlink that loses to a matcher link
>    by HLC is shown nowhere — ADR-0078 vs R1b) · **#755** (a link/unlink queues no duplicate re-check) · **#756**
>    (worklist/banner type tidy-up) · **#757** (the banner fails whole on one unreadable record; one failed flags read
>    fails the whole tray) · from R5a **#737** (identical entry headings) · **#738** (`cairn-gui-tauri` has no DB-gated
>    tests for its live branches; R5b's tray added two, R6 commented its fixture/locked-key arms) · **#739** (an unheld
>    member's "not yet run" line) · **#740** ("Different people" failure wording) · **#745** (a missing or stale banner
>    after a failure) · **#746** (Different people with an unreadable drug list — a decision) · **#747** (the banner
>    query's index) · #722 (no proposal fixture) · from R4 **#730** (the 101–1000 skipped-block band — a decision) ·
>    **#731** (no "charts failing" signal) · **#734** (db/046's comment
>    points at `db.py`). Earlier: **#709** · **#710** · **#712** · **#714** · **#715** · **#718** · **#719** · **#720** ·
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
> [`developers/durable-rules.md`](developers/durable-rules.md) carries every slice's "do not undo" block (R7, R6, R5b,
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

**Session date:** 2026-10-16 (#781 · #789 · #732, PR [#794](https://github.com/cairn-ehr/cairn-ehr/pull/794)) · 10-16 (R6 residuals, PR #792) · 10-15 (R7, #742 + #743 part 2, PR #788) · 10-14/15 (R6, #716 + #723, PR #777) · 10-12/13 three cloud tech-debt passes (PRs #770–#776, ADR-0080) · 10-11 DOB near miss (PR #758, ADR-0079) · 10-08/09 R5b (PR #749, ADR-0078) · 10-06/07 R5a (PR #735) · 10-05 #725 (PR #733) · 10-04/05 R4 (ADR-0076 decision 7, PR #724) ·
10-03 R3 (PR #721) and R1b (PR #717) · 09-30 R2b-2 (ADR-0077, PR #711) · 09-29 R2b-1 (PR #707) · 09-27 R2a (PR #698,
`db/055`) and R1 (ADR-0076, `db/054`, PR #688) · 09-26 #671 (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · earlier:
ROADMAP. · **Spec:** **v0.82** (newest ADR-0080;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **58** (`db/058`; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
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
slice's own rules are its block in `developers/durable-rules.md`).

### 2026-10-16 (second) — #781 · #789 split; #732 (PR [#794](https://github.com/cairn-ehr/cairn-ehr/pull/794))

HANDOVER was stale on arrival a third time (it called #792 awaiting merge; it had merged). Picked a decision-free bundle
whose gate stays cheap: no `db/*.sql` edit, so no relink of every cairn-node test binary.
- **⇒ Prove a pure move by the test LIST, not by a green run.** Capture `cargo test -p X -- --list` before, diff the
  sorted LEAF names after (module paths legitimately change). Same count + same leaves = nothing dropped or duplicated.
- **⇒ Move code by slicing exact line ranges with a script, never by retyping;** let the compiler name the stale
  imports (three, each used only by the moved tests). Two test modules can share fixtures as `pub(super) fn` in one
  and `use super::tests::{…}` in the other.
- **⇒ A fixture-gap RED can show the bug is already live:** #732's new test found 3 rows, not the 1 it wrote — two
  nameless charts a Rust suite had left in the shared `cairn_test`.

### 2026-10-16 — R6's residuals #787 · #786 · #783 · #782 (PR #792)

HANDOVER was stale on arrival again (PR #788 had merged); a bundle of four specified, decision-free fixes, TDD each.
- **⇒ A remedy that names a command is a claim: run it, and name the LIGHTEST one that works.** #787's first draft
  named `reproject --prefix identity. --rebuild` (refused by `cairn_reproject`); the second named a whole-log
  `--rebuild`, which works but truncates every projection and holds chart reads (availability). The review found a
  plain heal (`--prefix identity.`) refills the address; it is now named first, the rebuild as the fallback, and the
  test runs both. I had misread a refusal guarded by `IF p_rebuild` as the whole command's.
- **⇒ Keep one in-memory truth and project it at the boundary.** #786's suggested sibling field would have been a second
  source of the doubt; serde `try_from`/`into` through a private wire struct carries it without touching the 41 struct
  literals.
- **⇒ A first-load race is reproducible headlessly:** an `async` stub `invoke` resolves at the microtask checkpoint after
  `main.js`, before `link.js` runs. Old `main.js`: `keyUnlocked` false under an unlocked key; new: true.
- **zsh does not word-split `$var`** — a `for n in $names` loop silently ran once; wrap such loops in `bash -c`.

### 2026-10-15 — R7: #742 and #743 part 2, as decided (PR #788)

Decisions asked first (both maintainer calls); then design section, plan, TDD inline (small slices), mutation checks.
- **⇒ HANDOVER WAS STALE ON ARRIVAL:** it called #777 and #779 drafts, but both had merged. A ⇒ NEXT that names a PR goes
  stale the moment the PR merges; `gh pr list --state all` settled it in one call (house rule 8, the other direction).
- **⇒ A node-wide flag can answer a per-record question when the record's own half is already on screen.** "A change
  waits for a chart OUTSIDE this record" needed no per-record query: a change for a chart INSIDE it already shows that
  chart's pending line. Check what the screen already says before adding a read.
- **⇒ An OUT-column heal guard keyed on the NEWEST column's absence covers every older shape at once** (db/056; the
  `newest_age_s` guard it replaced covered one).
- **⇒ Keep a soft signal out of a predicate a hold is `EXISTS` over.** `cairn_chart_set_unverified_links` is a separate
  function precisely so the R1b hold cannot see it; the pin asks the hold, not the new function.
- **⇒ A struct literal gaining a field breaks every test that builds it** (eleven here); field order is free, so insert
  it after the opening brace. **Moving an inline test module to a `#[path]` file** kept `record_links.rs` under 500.
- **⇒ ROLLBACK HAZARD (not new):** with `SCHEMA_GENERATION` unchanged, an OLDER binary on a database this branch
  loaded fails "cannot change return type" on db/056. Before switching the shared `cairn_test` back to an older
  `main`, run `DROP FUNCTION cairn_duplicate_check_status();`.
- **Review (one agent pass, nothing safety-critical):** the R7 tests moved out so no file it touched crosses 500;
  filed **#789** (`duplicates/view_tests.rs` was already 563 lines) and **#790** (a rebuild/restore queues `change`
  notices in bulk, so the "running" line shows on every checked chart through that backlog — a decision); commented
  #718 (a fourth definer, `cairn_chart_set_unverified_links`).
- **Second review (2026-10-16, five agents in parallel), all fixed in the PR:** the banner and tray read the open pairs
  BEFORE the check state, so a proposal committed between the reads read as "checked, none open" (now status first,
  pinned by a source-order guard — the GUI has no DB tests, #738); a failed unverified-notes read took down the whole
  links list and every repair door (now a `Result` of its own); `read_snapshot` panicked on a pre-R7 status shape
  read without replay (now `try_get`); db/054 negatives (an end outside the set, an unlinked pair — both
  mutation-checked); comment fixes (a false `check_lines` sentence, db/020's trap cited by number, db/001's
  `attested`-reader list). Filed **#791** (the poison-chart over-warn). `techdebt-next` Step 8 now routes durable
  rules to `durable-rules.md`.
- **⇒ Two autocommit reads racing a separate process that never clears a notice before its proposals commit: read
  the side that proves completeness FIRST.** "Nothing waiting" read first guarantees the later pairs read sees every proposal.
- **⇒ The per-slice rules and the eighteen traps moved to [`developers/durable-rules.md`](developers/durable-rules.md)**
  (HANDOVER 803 → ~400 lines); the four per-pass 10-12/10-13 session entries, duplicated by the combined one, were
  dropped after a script confirmed no issue number was lost. ROADMAP's R1–R6 entries became one table (581 → ~460).

### 2026-10-13 (fourth) — #775 option A and #556, as decided (cloud session, PR #779, merged)

- **⇒ A CASE-MAPPING CHANGE ON ONE SIDE OF A SUBSET INVARIANT BREAKS THE INVARIANT.** Option A was first scoped as
  "search only, matcher untouched" — wrong: the sweep's blocking SQL must lowercase by the same rule or the drift
  test's sweep-paired ⊆ search-found fails. Grep every `lower(` over names, in Rust, SQL AND Python, before claiming a
  change is one-sided.
- **⇒ A COLLATE moves more than `lower()`.** It changes what `\s` and `[:alnum:]` match for that input. Fixtures for
  NBSP, Turkish `İ` and Greek capitals now sit in `patient_search_drift.rs`.
- **⇒ A test that ranks "first" among zero rivals proves nothing.** The raw-capital ranking test first matched only the
  target; it now has six older same-surname namesakes, and both query sites were mutation-checked.
- **⇒ Never run matcher pytest while the cargo DB gate runs** — both load the schema into `cairn_test`; the collision
  surfaced as a 40P01 deadlock in an unrelated restore test. **Disk:** a clean `target/` + `cairn-gui/target` filled
  the allowance twice; delete both outright when it does (the rebuild is ~20 min).
- **⇒ COMMITTED IS NOT VOUCHED.** #556 binds the token's bytes into the segment chain so stripping is detected; it does
  not make an unvouched token (db/020's deferred arm) trustworthy. The sync wire is still unbound: **#778**.
- Left for a decision: **#780** (`casefold()` for σ/ς and ß/ss, and the Python scorer's agreement); Turkish dotless `ı`
  vs `YILMAZ` (a capital `I` lowercases to dotted `i` under every Unicode-default mapping — only a Turkish-tailored
  comparator fixes it, which is the ADR-0014 profile's job, not db/046's).

### 2026-10-14/15 — R6: confirm a doubted link; name the doubt at the front door (PR #777, merged)

Brainstorm (maintainer: one door per line, "Review this link…"; every un-attested link offers Confirm; the bridge is
listed and re-linkable; #723 is a reworded label, never split rows) → design → plan (nine tasks) → SDD → opus final
review → one fix wave → docs. No ADR, no schema generation change. Filed #781 · #782 · #783 · #784; commented #738.
- **⇒ A pinned-count guard can go red where no per-task review looks.** Task 1 gave `edges.rs` a second wrapped call;
  `db_errors_stay_legible.rs`' pinned count (1) went red, and no per-task review ran it — only the full suite did. A
  task that adds a DB call, an event type or a helper re-runs the guards that count them (memory: pinned-count guards).
- **⇒ When you guard one verdict, ask the same question of every verdict on that panel.** The design made the confirm
  re-check the pair under CARNLK; the final review (I1) found the panel's Unlink had the identical mid-judgement gap
  (a colleague's confirm landing meanwhile, overruled by our later unlink). Same for R5a's twin: an Outranked verdict
  had to hide BOTH buttons in `unlink.js` AND `link.js`.
- **⇒ A prefix inside a fixed-width cell can eat the very name it qualifies.** The designed "↳ linked, in doubt: "
  (20 chars) left a doubted member's name at ~8 chars of the CLI's 28-char cell ("Mary SM…"); "↳ in doubt: " keeps it
  legible. Check a label against the width it renders in, not the sentence it reads as.
- **2026-10-15, a /pr-review-toolkit pass on the PR and a /fixall round (eight commits).** Fixed: the pre-click race
  (a colleague's confirm between drawing the list and "Review…"), the two-snapshot links read, the tray's "One person"
  on a doubted side, the Outranked disarm (then widened: any recorded answer), a node-side empty-address refusal,
  pins for the window's routing to the `_as_shown` doors and the Compare's guard, an isolated #220 clash test, and
  rustls 0.23.46 (four RUSTSEC advisories failed cargo-deny on every PR). Filed #785 · #786 · #787.
  **⇒ A fix wave gets its own review.** The fixes' own review found a hole in one of them: the disarm keyed on
  `reload`, which "Different people" sets even when outranked. **⇒ Two classic `<script>`s share one global scope**:
  a helper added to both under one name silently becomes the later file's.

### 2026-10-12 → 10-13 — four cloud tech-debt passes (PRs #770–#776)

Picked by blast radius each time. **10-12:** #602 (the strict door pins `cairn.remote_apply` off), #763 (auto-apply
never links while a re-check is pending), #689 (a misfiled medication attestation refused), #626 (db half: the sync
door's raw casts go through `cairn_uuid_or_raise`; `do_pull`'s generic non-P0001 arm still open), #726. **10-13:**
#604, #608, #431, #464, #768 · then #773, #603, #346, #553, #616, #744, #704 · then #708, #713, #569, #560, #682, #509.
- **⇒ #726: A CONNECT REPLAYS THE SCHEMA ONLY WHEN IT CHANGED ([ADR-0080](spec/decisions/0080-a-connect-replays-the-schema-only-when-it-changed.md)).**
  A no-op replay took 97 strong locks; `node_schema` now records the loader + a sha2-256 of the exact file texts (db/058).
  **Do not "simplify" the gate to the generation number** (strands in-place fixes — R6's db/054 edit relies on it) **or
  drop the loader check** (cairn-sync's subset stamp would vouch for files it never replayed). `test_serial_guard`
  clears the stamp; a test reconnecting to repair the schema calls `db::force_schema_replay`
  (`tests/schema_replay_gate.rs`).
- **⇒ A judgement made from a comparison carries what was compared** (#708): `RecordsShown`, re-read under CARNLK after
  the proposal-row lock. **An unknown outcome is its own marker** (#713): only an UNANSWERED commit gets it; the marker
  is the ROOT error with the cause as `source()` (anyhow's `.context(marker)` is invisible to a `chain()` downcast).
- **⇒ A race "found by reasoning" can be made deterministic:** hold transaction 1 open, run 2 on another connection
  until it has FINISHED (the bug) or is PARKED (`pg_stat_activity.wait_event_type = 'Lock'`), then commit 1 — no sleeps
  (`tests/shred_custody_race.rs`; R2b-2 and R6 use it).
- **⇒ A per-row lock cannot guard a whole-log scan** (#773: one lock in two modes, every shred SHARED, the heal
  EXCLUSIVE; order erasure-heal, then custody). **An issue's proposed fix can be wrong about MVCC** (#560: an in-flight
  lower seq is invisible inside one snapshot too; the export waits out `RowExclusiveLock` holders on `event_log`, read
  after `MAX(seq)`, CACHE 1 pinned) **or predate a guard** (#608's `USING ERRCODE` vs #633's file-granular guard).
- **⇒ A "belt and braces" re-check with no test is not braces** (mutate it; one changed nothing and was removed). **A
  count is not a set** (#553: name, never count). **A "silent" lenient arm can be worse than silent** (#768: a misfiled
  vouch displayed as a current signature; the predicate also feeds `stale`, #606). **A per-row `BEGIN … EXCEPTION`
  that WRITES costs a subtransaction XID per row** (`pg_stat_get_backend_subxact`).
- **⇒ The gate catches house rules:** a new db/tests mirror needs `\ir _scratch_database_guard.sql`; `cargo test` stops
  at the first failing binary; parallel output hides FAILED lines — read the `failures:` block. A fixture passing a
  "record" no window could show hides behind a new check.
- **⇒ Cloud rig** (runs the FULL DB gate): fetch micromamba from `github.com/mamba-org/micromamba-releases`
  (`micro.mamba.pm` is blocked), conda-forge `postgresql=18`, `cargo install cargo-pgrx --version =0.18.1 --locked`,
  `cargo pgrx init --pg18 <conda pg_config>` with an `xlocale.h` shim (`#include <locale.h>`) in both `CFLAGS` and
  `BINDGEN_EXTRA_CLANG_ARGS`, `initdb` as `postgres`, `CREATE ROLE root SUPERUSER`, export `PGHOST/PGPORT/PGUSER`. Do
  NOT pre-create `cairn_pgx` in `template1`. `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev` builds
  `cairn-gui-tauri`. `target/` reaches ~23 GB — delete `target/debug/incremental` before a second target dir. Never
  edit tracked files while the gate compiles.
- Left for a decision: **#775** (signed tokens' case vs the cluster's `lower()`), **#556** (whether `attestation` /
  `attester_key` join `segment_commitment` — before the first release shipping a CAIRNB3 writer), #649 (caller-minted
  patient id vs a cancellation contract), #613 (is INCOMPLETE about records or the recovery?), #628 (the NUL fix needs a
  `cairn_pgx` bump). #427 is lower than it reads (`safety` is a clear field of `signed_bytes`, which `cairn_agent` reads
  anyway, #424; ADR-0080 made replays rare).

### 2026-10-04 → 10-09 — R4 (PR #724), #725 (PR #733), R5a (PR #735), R5b (PR #749)

- **⇒ A title is a claim too** (ADR-0078's first title was false; fact-check titles and Consequences, not only bodies).
  **⇒ "The front door never waits" must hold in every UI state** — cost a read by what is SHOWN, never by the backlog.
  **⇒ A test whose two orientations coincide pins neither** (seed the asymmetric case and mutate). **⇒ Count steps
  against the as-built window, not the plan** (#754). **⇒ Widening "who may be proposed" must check every CLOSED status**
  (an `auto_applied` row overruled by an agent's unlink reached no human until `upsert_proposal` reopened it; filed
  #755–#757).
- **⇒ Read the design's own filter rule against the issue that asked for it** (#700's *attested*). **⇒ A second button
  over one shared state doubles every guard** (R5a I1). **⇒ "Not recorded" is a claim** an outcome-unknown commit makes
  false. **⇒ A stored, propose-time fact goes stale in exactly the case it exists for.** **⇒ Script load order is a
  correctness property.**
- **⇒ A health signal must be walked through every way its input fills** (R4's "behind"). **⇒ A CI job with a different
  dependency set cannot be gated locally — turn the difference into a test**; read `gh pr checks` before calling a PR
  ready. **⇒ Diagnose before choosing an issue's candidate fix** (#725: `EXPLAIN ANALYZE` it).
- **Mechanics:** a callee's `rollback()` undoes a test's `SET ROLE`; an interrupted subagent's uncommitted edits are
  evidence; editing an `include_str!`'d `db/*.sql` relinks every cairn-node test binary; HTTPS push:
  `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push`.

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
Code workspace: `/crates` (`cairn-event`, `cairn-keystore`, `cairn-medium`, `cairn-wire`, `cairn-sync`, `cairn-node`,
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
- **⇒ The repair path: R7 (PR #788)'s review and merge, then the open repair-path issues** — see ⇒ NEXT.
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
