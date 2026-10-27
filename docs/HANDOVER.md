# HANDOVER — Cairn

## ⇒ NEXT

> [!NOTE]
> **⇒ ONE PR AWAITS THE MAINTAINER: PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)** (#861, branch
> `fix/861-stopped-member-reason-on-current-line`): a line that reads current carries its stopped member threads'
> reasons (`MedicationRow::stopped_member_reasons`), said "a stopped record of this drug gave: …" by the CLI list and
> the window, never "Stopped"; the cease note's list claim is one sentence everywhere. Read-side only. Two reviews,
> no code defect (stale comments and a stale test fixed in-PR, the status split made a pure tested fn; filed **#869**,
> the CLI list's reason loops are untested, and the decisions **#870** and **#871**). Touched
> DB-gated binaries, the full cairn-gui tree, fmt, clippy and `cargo doc -D warnings` on both trees green; CI gates the
> full sweep. RUNBOOK §15 gained the current-line check.
> **Merged 2026-10-24: PR [#865](https://github.com/cairn-ehr/cairn-ehr/pull/865)** (#863 + #864): a shred re-folds the
> threads its event named (`cairn_refold_after_shred`, db/037); every production medication-list read is one
> `REPEATABLE READ READ ONLY` snapshot. Filed **#866**, **#867** (both decisions).
> **Merged 2026-10-21/23 (ROADMAP has each; the hold and stop-reason blocks are in `developers/durable-rules.md`):**
> PR #860 (#857 + #859: db/060, gen **60**; filed #861, #863, #864, **#862** — `run-db-gated-tests.sh` with
> `CAIRN_TEST_PG*` but no `PGPORT` runs its SQL step against :5432, pass `PGHOST=127.0.0.1 PGPORT=5532`) · PR #856
> (#835 + #854; filed #857 · **#858** · #859) · PR #853 (#843 + #849:
> [ADR-0082](spec/decisions/0082-a-release-of-an-ended-hold-is-refused-at-the-local-door.md), spec **v0.84**; filed
> **#854**, **#855**) · PR #851 (#848; filed #849, **#850**) · PR #847 (#846; db/032's false comment → **#185**) · PR
> #845 (#844 + #836) · PR #842 (#839–#841) · PR #838 (#832, the window's **W**; **owed by a human: RUNBOOK §15** — W ≤
> 10 s, Release ≤ 5 s, VoiceOver/keyboard; #809 gained item 5) · PR #831 (#803 slice A,
> [ADR-0081](spec/decisions/0081-medication-withhold-is-a-time-bounded-hold.md), `db/059`; filed #835 · #836 · **#837**).
> **Merged 2026-10-18/20 (ROADMAP has each):** PR #830 (#828) · PR #829 (#826: `assert-observed-evidence` prints its event ids) · PR [#827](https://github.com/cairn-ehr/cairn-ehr/pull/827) (#822
> `cairn-node event <id>`; #823 auto-apply stops at an unknown commit with its summary) · PR #824 (#820: a real lost
> COMMIT driven end to end; review filed **#825**, the window drops an unknown outcome's error chain — a decision on
> where operator diagnostics live) · PR #821 (#817) · PR #819 (#815 + #818) · PR #816 (#814) · PR #813 (#811) · PR #805
> (#797, "Read this chart again"; RUNBOOK §14 is its owed human timing) · PR #802. The repair path (#679 · #680 · #681)
> is built through R7. Design `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`; R4's
> runbook `docs/developers/running-the-duplicate-check.md`; human passes RUNBOOK §11–§14. spec **v0.84** (newest
> ADR-0082). DOB-near-miss follow-ups: #759 · #760 · #762 · #764 · #765 · #766. Trap: a new blocking key can generate
> a pair an older test asserts absent — fix the fixture date, never the assertion.
>
> **⇒ NEXT, in order:**
> 0. Check `gh pr list` before trusting this list (house rule 8).
> 1. **Review and merge PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)** (#861); run #838's RUNBOOK §15
>    (now incl. #861's current-line check); **#869** (cheap: test the CLI list's reason lines). From PR #868's second
>    review, decisions: **#870** (the wrong-chart warning names the dose, not another patient's stop reason) · **#871**
>    (a current line's angioedema shows only on hover of "current" — paper-parity). From PR #865's second review: **#866** (an
>    earlier-shredded plaintext event comes back on a heal or re-fold — a decision) · **#867** (should a failed
>    re-fold block the erasure — a decision). Decisions owed: **#825** (where the window's
>    operator diagnostics live), **#850** (acknowledging a dealt-with report). Buildable hold follow-ups: **#855** (two release races) · **#837** (test gaps; item 2 settled by
>    ADR-0082) · #834 (needs a decision on shape). From PR #856's second review: **#858** (a peer release on a
>    sibling thread is shown nowhere). An idempotent re-send of a registration
>    (the same signed batch; `submit_event` is a no-op on a repeated `event_id`) is possible later, after #807, with
>    #649. **#728** (the Pi re-measure) can run beside. #699 stays open until the maintainer closes it.
> 2. **Decided in the #797 brainstorm (2026-10-17):** #803 (slice A PR #831, slice B PR #838, both merged; follow-ups #833 ward-level
>    list, #834 no marker for an undecryptable event) → **#804** (on link, show only where the two charts' medication
>    lists DIFFER, with continue / discontinue / withhold per line; needs #803 and **#690**). From #797's reviews:
>    **#812** (window type/test tightening) · **#807** (the window never
>    reconnects after a lost connection) · **#808** (two outcome-line races) · **#809** (window tech debt) · **#810**
>    (cairn-node files over 500 lines).
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

**Session date:** 2026-10-26 (#861, PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)) · 2026-10-24 (#863 + #864, PR [#865](https://github.com/cairn-ehr/cairn-ehr/pull/865)) · 2026-10-23/24 (#857 + #859, PR [#860](https://github.com/cairn-ehr/cairn-ehr/pull/860)) · 2026-10-23 (#835 + #854, PR [#856](https://github.com/cairn-ehr/cairn-ehr/pull/856)) · 2026-10-23 (#843 + #849, PR [#853](https://github.com/cairn-ehr/cairn-ehr/pull/853)) · 2026-10-23 (#848, PR [#851](https://github.com/cairn-ehr/cairn-ehr/pull/851)) · 2026-10-22 (#846, PR [#847](https://github.com/cairn-ehr/cairn-ehr/pull/847)) · 2026-10-22 (#844 + #836, PR [#845](https://github.com/cairn-ehr/cairn-ehr/pull/845)) · 2026-10-22 (#839 + #840 + #841, PR [#842](https://github.com/cairn-ehr/cairn-ehr/pull/842)) · 2026-10-21 (#832, PR [#838](https://github.com/cairn-ehr/cairn-ehr/pull/838)) · 10-21 (#803 slice A, PR [#831](https://github.com/cairn-ehr/cairn-ehr/pull/831)) · 10-20 (#828, PR [#830](https://github.com/cairn-ehr/cairn-ehr/pull/830)) · 10-20 (#826, PR [#829](https://github.com/cairn-ehr/cairn-ehr/pull/829)) · 2026-10-19 (#822 + #823, PR [#827](https://github.com/cairn-ehr/cairn-ehr/pull/827)) · 2026-10-18 (#817, PR #821) · 10-18 (#815 + #818, PR #819) · 10-18 (#814, PR #816) · 10-18 (#811, PR #813) · 10-18 (PR #805's second review, fixed) · 2026-10-17 (#797, PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805)) · 10-17 (#799 · #800 · #801, PR [#802](https://github.com/cairn-ehr/cairn-ehr/pull/802)) · 10-17 (PR #798's second review round) · 2026-10-16 (#745 · #752 · #740, PR [#798](https://github.com/cairn-ehr/cairn-ehr/pull/798)) · 10-16 (#793 · #737, PR #795) · 10-16 (#781 · #789 · #732, PR #794) · 10-16 (R6 residuals, PR #792) · 10-15 (R7, #742 + #743 part 2, PR #788) · 10-14/15 (R6, #716 + #723, PR #777) · 10-12/13 three cloud tech-debt passes (PRs #770–#776, ADR-0080) · 10-11 DOB near miss (PR #758, ADR-0079) · 10-08/09 R5b (PR #749, ADR-0078) · 10-06/07 R5a (PR #735) · 10-05 #725 (PR #733) · 10-04/05 R4 (ADR-0076 decision 7, PR #724) ·
10-03 R3 (PR #721) and R1b (PR #717) · 09-30 R2b-2 (ADR-0077, PR #711) · 09-29 R2b-1 (PR #707) · 09-27 R2a (PR #698,
`db/055`) and R1 (ADR-0076, `db/054`, PR #688) · 09-26 #671 (ADR-0075, PR #678) · 09-23 funnel 2c (PR #674) · earlier:
ROADMAP. · **Spec:** **v0.84** (newest ADR-0082; #857 and #861 needed no ADR — a projection-side fix, no wire change;
[ADR-0067](spec/decisions/0067-a-restore-reads-the-clinical-plane.md) supersedes ADR-0026 decision 2's implementation
wording only) · **`SCHEMA_GENERATION`:** **60** (`db/060`, PR #860; R6 and R7 edit db/054, R7 db/056, in place) · **Phase:** architecture
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

### 2026-10-26 — #861 (PR [#868](https://github.com/cairn-ehr/cairn-ehr/pull/868)): a current line shows its stopped members' reasons

Built to the maintainer's 2026-10-24 decision. TDD, two reviews (no code defect; filed #869; the second renamed a
test the PR had made false, pinned holds.js's reveal gate, made the status split the pure `reasons_for_status` with
the shown-stop-gave-none case tested, and filed the decisions #870 and #871).
- **⇒ A new meaning gets a new field, not a status-dependent reading of an old one.** Filling `cessation_reason` on a
  current line would have made every renderer that does not know the change say "Stopped" of a current drug; a field
  of its own fails safe (a renderer that ignores it shows nothing, as before).
- **⇒ "The shown stop" is the thread's NEWEST RECORDED stop** (db/031 keeps one row per thread), not its
  latest-effective one — my first test assumed the latter. Read the projection before writing an order assertion.
- **⇒ When a fix makes a caller-chosen claim true everywhere, delete the choice** (both `LIST_*`
  constants and `already_stopped_sentence`'s `list` parameter), then grep docs and comments for the old claim (the review found
  three in durable-rules, four in comments, one in RUNBOOK).

### 2026-10-24 — #863 + #864 (PR [#865](https://github.com/cairn-ehr/cairn-ehr/pull/865)): a shred hands its rows on; a list is one snapshot

Maintainer chose #863 + #864 and decided #861's shape. TDD (red first on both one-winner keys), each pin
mutation-checked, an independent review, then a second four-agent review (fixed in-PR; filed **#866**, **#867**).
- **⇒ "Never over-erase" is half a rule.** Deleting only your own rows is right; leaving a one-winner key EMPTY is
  under-statement. Any scrub of a winner-takes-all projection owes a re-fold of what remains.
- **⇒ Never SKIP a locked sibling on the theory that its holder will write it** — a holder that rolls back, or
  only re-receives the event, writes nothing (the review's catch; my first cut try-locked). Serialize the shreds and
  wait instead; a door cycle is a transient 40P01. **Compare ids canonically** — the floor admits any uuid spelling.
- **⇒ Raise isolation only for reads.** A door write at REPEATABLE READ reopens the custody/shred race; the snapshot
  lives at the callers that own a bare connection, and the readers keep `GenericClient` (tests' failure seam).
- **⇒ A source-guard slice at `at - N` bytes panics on an em dash** — use `floor_char_boundary`.
- **⇒ "Its clear view is gone" is true only of a SEALED event.** `cairn_clear_payload` returns a plaintext body
  straight from the log, and the sync door shreds a peer's plaintext event — so the re-fold re-inserted its own
  target's rows (second review; three reviewers found it independently). Now excluded by id; an EARLIER-shredded
  plaintext sibling still comes back on a heal or re-fold (**#866**). A sibling whose replay raises aborts the
  shred; a P0001 is re-raised bare naming the sibling, any other code propagates untouched (a first cut used
  `USING ERRCODE = v_state`, and CI's `floor_refusals_carry_no_errcode.rs` — not in my targeted run — refused it); whether that should block the erasure is **#867**.
- **⇒ zsh does not word-split `$T`** — `cargo test $T` with `T="--test a --test b"` passes ONE filter and runs
  every binary with all tests filtered out, exit 0. Use an array (`T+=(--test a)`, `"${T[@]}"`).

### 2026-10-23/24 — #857 + #859 (PR [#860](https://github.com/cairn-ehr/cairn-ehr/pull/860)): every stop's reason; a one-thread stop says the line stays current

Maintainer chose the shape ("every stop's reason"). TDD, two independent reviews (all findings fixed or filed: #861, #863, #864),
mutation-checked pins.
- **⇒ A new projection beside a one-winner one is the cheap honest fix**: register a SECOND apply fn for the same
  event type (the dispatcher runs every fn by `run_order, apply_fn`), bump the generation so the loader's heal
  backfills, add the shred delete, and bump BOTH registry counts. Fixture truncate lists (three `tests/common` files,
  two cairn-sync lists) and `combined_read.rs`'s golden strip are the places a new table/field also reaches.
- **⇒ Fixing what a list SHOWS changes what every sentence ABOUT the list may claim.** "The list keeps every stop's
  reason" was true for a stopped LINE and false for the CLI's thread note on a line still current; "no reason was
  recorded" became false beside a reasonless latest stop. Grep the sentences, not only the renderers.
- **⇒ An undated stop is effective NOW**: under latest-effective-wins it outranks a sibling's older dose, so a test of
  "a sibling keeps the line current" must backdate the stop.
- **⇒ Env exports do not survive between tool calls**: a DB suite "ok" in 0.00 s was a self-skip. Source an env file
  in the same call.

### 2026-10-23 (third) — #835 + #854 (PR #856): a release still ahead is said; the CLI cease says "already stopped"

Chosen with the maintainer. TDD, then an independent review, one fix wave; full local gate green on the final tree.
- **⇒ A sentence about a FUTURE event must check that the event will still act.** "Release at 16:00" on a hold that
  ends by its own `until` at 14:00 announces something that ends nothing (the review caught it; 7c now requires
  `released_at < until_at`). Walk a "will happen" sentence through every way the thing can be over first.
- **⇒ Widen a read with a sibling predicate, not a new column on a set-returning function** (a return-type change
  `CREATE OR REPLACE` refuses, and an older binary's replay then fails); ask both at ONE instant read in `FROM`.
- **⇒ A display-only read must not block a safety write** (the CLI cease's note), and "nothing to say" is not
  "not known": an unseen thread says it cannot tell.
- **⇒ A rule shared by two surfaces is only shared if they act on the same unit** (second review). The window stops
  a LINE, the CLI one THREAD; the line rule was silent for a stopped thread on a line kept current by a sibling.
- **⇒ "Release at 16:00" after "from 08:00" reads as an interval** — a release before a pending start is a cancel and
  must say so. And a sentence that says what the list keeps must be checked against the projection (it overwrites).

### 2026-10-22 → 10-23 — #846 (PR #847), #848 (PR [#851](https://github.com/cairn-ehr/cairn-ehr/pull/851)), #843 + #849 (PR [#853](https://github.com/cairn-ehr/cairn-ehr/pull/853))

- **⇒ A maintainer decision in an issue can still reverse an ADR sentence** — grep the ADR before building and write
  the amending ADR (#843 → ADR-0082). **A floor `P0001` carries no scope**: reuse the floor's predicate as a Rust
  pre-check (`cairn_medication_hold_ended`), never match message text. **`retry_when_nothing_written` returns `None`
  whenever an outcome is unknown**, so "returned above" comments lie about that path.
- **⇒ A surviving mutation can mean a second witness, not a dead predicate** (#848's `standing_patient`: a moved
  thread's assert also seeds a point-0 dose row) — find the case, then pin it. **A new model field breaks its literals,
  not its readers**: grep the readers of the SIBLING fields (the semantic tree was found that way, by the review).
- **⇒ Walk a remedy or absence sentence through every state** ("record it again" beside a drug already listed; "No
  medications recorded" beside the report; "on another chart" of a thread no chart lists). **Two statements are two
  snapshots**: a truth the report depends on is re-checked inside the statement that says it.
- **⇒ A flag written at arrival can only see what had arrived** (#846): where a contradiction can appear from either
  side, read it at list time (a view), and check the plan reaches the base tables (`EXPLAIN`; it needed a
  `medication_id` index).

### 2026-10-18 → 10-22 — interrupted commits (PRs #813–#830), the W (PRs #831, #838), its follow-ups (PR #842)

- **⇒ The logical clock is not a wall clock** (#803): a peer may push `hlc_state` 24 h ahead (db/020), so a clinical
  "now" comes from the node's PHYSICAL clock (`medication_withhold_clock.rs`); a test-side workaround for product
  behaviour is a finding, not a fix. **A fallback in the floor is a bypass of the rule the client follows** (ADR-0021).
- **⇒ A new value for an enum other code reads as "on the list" is a silent filter** — put a new state BESIDE the enum
  (`Withheld` would have dropped held drugs from sign-off). **An `IF NOT EXISTS` index never picks up an edited
  predicate** (rename it). **A guard probing one table per source file needs a probe per new file.**
- **⇒ A real mouse finds what `element.click()` cannot** (#832: a reflowing W flickered under the pointer). **One press,
  one W needs the button disabled.** **A control that overrides another must be undone by it.** **Hiding a button
  hides what its refusal said** (#841) — say it on the line. **A view that cannot see a rule's input must not guess
  the rule.** **Read the code before the issue's fix list.** `cargo doc -D warnings` catches what clippy cannot.
- **⇒ An outcome that MAY have written settles like a success**; only an ERROR answer proves a rollback; one
  `commit_failure` marks every commit, a tree inventory counts them and detects autocommits; a port that flattens an
  error loses its marker (#815). To learn a rolled-back id, have the kill switch SAY it (#820). A flag carried by one
  async call dies with it — make it window state (#811). **An issue's survey is a claim** (#826); **a printed line can
  be a parse contract** (#828); widen a return with a `*_returning_event` sibling. Not done (optional in #820): one
  renderer for the CLI's and the window's interruption sentence; nothing reads Postgres notices (#585); #812 items
  7–10 and #814 came from the reviews; #713 first stated "unanswered ⇒ unknown" inline.

### 2026-10-17 (third) — #797 brainstormed, planned, built SDD (PR [#805](https://github.com/cairn-ehr/cairn-ehr/pull/805))

Brainstorm with the maintainer → plan → subagent-driven (7 tasks, a task review each, 9 fix rounds, an opus final
review, one fix wave). **The maintainer's rule** (now in durable-rules): *tell the clinician only what changed from
ANOTHER source since they took the chart; never their own act.* Decided in the same brainstorm: #803 (withhold), #804
(differences-only view on link). Paper counterpart per the maintainer: you are TOLD after writing and then reconcile —
or, commonly, the two charts never meet; so the refused press is not an `M > N` defect.
- **⇒ Route a cause by TYPE end to end** (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`); the
  window had been telling refusals apart by `e == CHANGED`.
- **⇒ A baseline that must survive overlapping reads is window STATE, not an argument to one call.** The own-act
  baseline passed to one `refresh` let ANOTHER re-read announce the clinician's own link back (task-5 review, I1).
- **⇒ A reviewer's counter-case beat my formula.** `(held − (sent − after)) ∪ (after − sent)` announces one's own link
  (the `applyOwnAct` table's fourth row: a Link answered on the other chart, drawn before its commit); the implementer
  pushed back with the walk, the re-reviewer reconstructed it over 17 cases. Rule on the evidence, not on who proposed
  it.
- **⇒ "After apply, before commit" must be pinned on BOTH sides** — a DB test caught only "after apply"; a source
  guard now pins the commit side (`judge_tests.rs`).
- **⇒ An operator procedure is a claim too:** RUNBOOK §14 first told testers to stop Postgres and recover with the
  button — the window never reconnects (#807); a REVOKE cannot fail a superuser; the fault is a renamed function now.

### 2026-10-16 → 10-17 — #745 · #752 · #740 (PR [#798](https://github.com/cairn-ehr/cairn-ehr/pull/798)) and #799 · #800 · #801 (PR [#802](https://github.com/cairn-ehr/cairn-ehr/pull/802))

- **⇒ Walk the old code with the same script** — a walk that passes only on the fix proves the rig. **Webview JS can
  be pinned without a JS runner** (`source_guard::js_fn` / `js_const`); a guard for both halves of `a && b` passes
  `a || b` — pin the exact expression.
- **⇒ Draw unless something newer is DRAWN; say a failure only if newest STARTED** (two predicates). A per-step lock
  is not a per-read lock. A window sentence must be true of EVERY state reaching it; emptying a list is not
  cancelling its read (bump the revision).
- **⇒ Read the code before the issue** (#740 was absorbed by #713's `judgement_error_view`); a fix that SHOWS
  something in a new state reaches code that state never reached; a fix can falsify other files' rationale (after
  #799, six comments). Re-read the narrowest stale thing; a rule change can make a "belt and braces" line
  load-bearing (`clearChart`'s floor).
- Mechanics: stub payloads dumped from Rust, never hand-written; a test assert must never echo identity-built text
  (CodeQL failed #795 after merge); zsh does not word-split `$step` (use `bash -c`).

### 2026-10-15 → 10-16 — R7 (PR #788), R6's residuals (PR #792), file splits (PRs #794, #795)

R7 = #742 + #743 part 2; residuals #787 · #786 · #783 · #782; splits #781 · #789 · #732 · #793 + #712 item 3 · #737 ·
#796. Filed #789 · #790 · #791; commented #718.
- **⇒ A remedy that names a command is a claim: run it, and name the LIGHTEST that works** (#787). **Keep one
  in-memory truth, project it at the boundary** (#786). **Prove a pure move by the test LIST** (diff sorted
  `cargo test -- --list`); `#[cfg(test)] #[path = "x_tests.rs"] mod tests;` keeps the module path.
- **⇒ A heading must tell entries apart on the input the list exists for; punctuation is not speech** (VoiceOver
  reads "—" as a pause). **Read the side that proves completeness FIRST** when two autocommit reads race. **Keep a
  soft signal out of a predicate a hold is `EXISTS` over** (`cairn_chart_set_unverified_links`).
- **⇒ ROLLBACK HAZARD:** with `SCHEMA_GENERATION` unchanged, an OLDER binary on a database this branch loaded fails
  "cannot change return type" on db/056 — `DROP FUNCTION cairn_duplicate_check_status();` before switching back.
- Mechanics: `cargo doc -D warnings` reads `<date>` as an unclosed HTML tag (backtick it); a struct literal gaining a
  field breaks every test that builds it.

### 2026-10-12 → 10-15 — cloud tech-debt passes (PRs #770–#776, #779), R6 (PR #777)

**Cloud passes** (picked by blast radius): #602 · #763 · #689 · #626 (db half; `do_pull`'s non-P0001 arm open) · #726 ·
#604 · #608 · #431 · #464 · #768 · #773 · #603 · #346 · #553 · #616 · #744 · #704 · #708 · #713 · #569 · #560 · #682 ·
#509; then #775 option A + #556 (PR #779).
- **⇒ #726: A CONNECT REPLAYS THE SCHEMA ONLY WHEN IT CHANGED ([ADR-0080](spec/decisions/0080-a-connect-replays-the-schema-only-when-it-changed.md)).**
  `node_schema` records the loader + a sha2-256 of the file texts (db/058). **Never "simplify" the gate to the generation
  number** (strands in-place fixes) **or drop the loader check**; a test repairing the schema calls
  `db::force_schema_replay`.
- **⇒ A case-mapping change on ONE side of a subset invariant breaks it** (#775: grep every `lower(` over names in Rust,
  SQL AND Python); **a COLLATE moves `\s` and `[:alnum:]` too** (NBSP/`İ`/Greek fixtures in `patient_search_drift.rs`);
  **a test ranking "first" among zero rivals proves nothing.** **COMMITTED IS NOT VOUCHED** (#556 detects stripping, not
  trust; the sync wire is still unbound, **#778**). Left for a decision: **#780** (`casefold()`), Turkish dotless `ı`
  (the ADR-0014 profile's job).
- **⇒ A judgement carries what was compared** (#708, `RecordsShown`); **an unknown outcome is its own ROOT marker**
  (#713; `.context(marker)` is invisible to a `chain()` downcast). **A race found by reasoning can be made
  deterministic**: park the victim on the lock (`pg_stat_activity`), never sleep (`tests/shred_custody_race.rs`).
- **⇒ A per-row lock cannot guard a whole-log scan** (#773); **an issue's fix can be wrong about MVCC** (#560) **or
  predate a guard** (#608 vs #633); **a belt-and-braces re-check with no test is not braces**; **a count is not a set**
  (#553); a lenient arm can be worse than silent (#768, feeds `stale`, #606); a per-row `BEGIN … EXCEPTION` that WRITES
  burns a subtransaction XID per row.
- **R6** (#716 + #723): one door per line, "Review this link…", Confirm on every un-attested link, the bridge
  re-linkable. **⇒ A pinned-count guard can go red where no per-task review looks** (run the guards that count what a
  task adds). **⇒ Guard one verdict, ask it of every verdict on the panel** (Unlink had Confirm's gap; an Outranked
  verdict hides both buttons). **⇒ A prefix in a fixed-width cell can eat the name** ("↳ in doubt: "). **⇒ A fix wave
  gets its own review**; two classic `<script>`s share one global scope. Filed #781 · #782 · #783 · #784 · #785 · #786
  · #787; commented #738.
- **Cloud rig** (runs the FULL DB gate): micromamba from `github.com/mamba-org/micromamba-releases`, conda-forge
  `postgresql=18`, `cargo install cargo-pgrx --version =0.18.1 --locked`, `cargo pgrx init --pg18 <conda pg_config>` with
  an `xlocale.h` shim in `CFLAGS` and `BINDGEN_EXTRA_CLANG_ARGS`, `initdb` as `postgres`, `CREATE ROLE root SUPERUSER`;
  never pre-create `cairn_pgx` in `template1`; `apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev`;
  `target/` reaches ~23 GB. Mechanics: a new db/tests mirror needs `\ir _scratch_database_guard.sql`; read the
  `failures:` block. Left for a decision: #649 · #613 · #628; #427 is lower than it reads (#424). #220 gained a test.

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
