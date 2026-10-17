# #797 — "This record changed": re-read, say what changed, and a "Read this chart again" control

> [!NOTE]
> **As built (PR #805) — this plan is the design as it was before review, not the code.** Where they differ, the code
> and `docs/developers/durable-rules.md` (the #797 block) win:
> - The baseline is a separate HELD set (`heldCharts`, `chart_stale.js`), moved by the clinician's own acts through
>   `applyOwnAct` — not `renderedCharts`, and not a per-call `seenAfterOwnAct` argument.
> - `charts_after` is `Option<ChartSet>` (`null` when the act changed no set), not `Vec<String>`.
> - `refresh(lead, behind)` takes two arguments and always sends `heldCharts`; "the list is behind the record" is
>   WINDOW state (`behindAfter`), not one read's flag, so whichever read fails last marks the list.
> - The funnel maps `DataError::Stale` to `Retry::Never` (it has no chart to re-read), not `AfterReread`.
> - The "Read this chart again" button is wired in `chart_stale.js` with `rereadLead()`, not in `main.js`.
> - The RUNBOOK step is §14, not §13.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a refusal says the record changed under the clinician, the window re-reads the whole chart by itself, says what changed (worded in Rust), and never retries the act. A failed read gets an always-visible "Read this chart again" control instead of "reload the chart" or "close the chart and open it again".

**Architecture:** The cause travels as a TYPE, never as text:
- a new `RefusalScope::Stale` in `cairn-node` maps to a new `DataError::Stale` in `cairn-gui-data`, which maps to `Retry::AfterReread`, broadened to mean "the screen is behind the record";
- the window's own set-changed refusals use the same retry through a `stale()` helper.

`med_list` takes the set of charts the window had on screen and returns a Rust-worded `set_change` sentence when the set it just read differs. The webview reacts to `after_reread` everywhere by re-reading the whole chart. Since #799 a re-read keeps typed stop reasons, and since #800 it cannot race another read.

**Tech Stack:** Rust (`cairn-node`, `cairn-gui-data`, `cairn-gui-live`, `cairn-gui-tauri`), plain-JS webview (no npm), Rust source guards for the JS halves.

**Spec:** the design approved in chat on 2026-10-17 (summarised in the PR body and in `docs/HANDOVER.md`'s session entry); issue #797 and its two comments. Follow-ups decided in the same brainstorm and NOT in this plan: #803 (withhold) and #804 (differences-only reconciliation on link).

**Base:** branch `feat/797-record-changed-reread`, cut from `fix/799-800-801-chart-reread` (PR #802, open). Rebase onto `main` once #802 merges.

## Global Constraints

- **Never retry the refused act.** A stale picture means the clinician must see the new record and press again (paper parity, below).
- **Route by type, never by sentence text.** No new `e == SOME_SENTENCE` comparisons. The existing `e == CHANGED` checks inside `cairn-gui-tauri` are replaced by `ChartSetError` (Task 3).
- **Node sentences say only what happened.** They are shown by the CLI too, so no node sentence names a window control. The window adds what IT did.
- **A sentence the window words itself must be true in every state that reaches it.** "Has been read again" is said only AFTER a successful re-read.
- Every `cairn-gui-tauri` source file stays under 500 lines; `src-ui/main.js` is at 481 today.
- House rules: TDD (RED first; mutation-check every source guard); no test asserting identity-built text with `{}` echo (CodeQL); `CAIRN_ALLOW_DB_SKIP=1` for DB-free runs; scratch `CARGO_TARGET_DIR=/tmp/…` while an IDE is open.

## Paper-parity benchmark (§1.2)

- **Paper counterpart** (maintainer, 2026-10-17): you write up a patient's drug chart. A colleague then says they reviewed the patient this morning, could not find the chart and wrote a new one meanwhile (or a nurse reports it), and the two are reconciled. **More commonly the discrepancy is never noticed at all**, because the two charts are kept in different places. A failed read has no paper counterpart.
- **Steps:**
  - Record changed: paper **N = 3** (write the chart, be told, reconcile). Architecture-forced **M = 3** (the refused press, review the re-read list, press again). UI bundling target **K = 3**.
  - Failed read: today 3 acts (close the chart, find it at the front door, reopen it) → **M = 1** (press "Read this chart again").
  - M ≤ N throughout. In the common paper case (never noticed) Cairn is strictly better: it catches the discrepancy.
- **Time + cognitive load:**
  - Budget: the automatic re-read and its sentence land within the single-chart open budget (RUNBOOK §8/§11, ≤ the single-chart open). One sentence names what changed, using each chart's own identity line; nothing else to read.
  - Measurement is owed by the human runbook pass (a new RUNBOOK step under §13, Task 7). It cannot be measured headlessly.

## When the "what changed" sentence appears (maintainer rule, 2026-10-17)

> **The governing rule, in the maintainer's words:** "The chart I hold in my hand when I make changes is my
> responsibility — my obligation to know its contents. I need only to be informed of what changed between me taking
> the chart and finishing amends that I cannot know (that is, anything that wasn't in the chart when I took it and I
> didn't change myself)."
>
> "The chart I hold" is what the window last SHOWED (`renderedCharts`): once a re-read has drawn a change and named
> it, it is part of the chart held. The baseline moves only by being shown.

**Only for a change that came from ANOTHER source** — never for the clinician's own act. Every write checks the set
the clinician saw, under the link lock, before it records anything. So a successful act proves nothing foreign
changed before it; what remains is anything arriving during or after it:

- **The clinician's own Link / Unlink / Confirm.** The act's report carries `charts_after`: the record's set as THIS
  act left it, read inside the act's own transaction. The re-read compares against `charts_after`, so their own
  change is silent and a colleague's arriving in the same moment is named.
- **Sign-off, cease, additions.** These never change the set; the re-read compares against the set on screen.
- **A stale refusal.** The change came from another source by definition; the re-read compares against the set on
  screen.

**Scope:** the sentence names charts joining or leaving the record. A colleague's concurrent change to a MEDICATION
LINE is shown by the re-read but not named; naming it needs a before/after comparison of the list itself, which
belongs with #804's differences-only view.

## Review Focus

1. **A re-read that fails after a stale refusal.** The old list must not stay up looking current under a live "Sign off N" (Task 5, `markListStale`).
2. **A stale refusal with the set UNCHANGED** ("this pair has been judged since the list was drawn"). No "now part of it" sentence may appear (Task 4: `set_change` is `None` when the sets are equal).
3. **A departed chart whose identity cannot be read.** It is still named, as unread, never dropped (Task 4).
4. **An unreadable `seenCharts`** (empty or unparseable). No sentence, never a guessed change (Task 4).
5. **The clinician's own Link followed by a re-read.** No "Now part of it" sentence for the chart they just linked; a chart a colleague linked in the same moment IS named (Task 4: `charts_after`; Task 5: the re-read sends it as `seenCharts`).
6. **Overlapping re-reads, e.g. a sign-off's and a judgement's.** The `set_change` sentence comes only from an answer `mayDraw` accepts (#800); an overtaken answer says nothing (Task 5).

---

### Task 1: `RefusalScope::Stale` in `cairn-node`, raised where the record changed under the caller

**Files:**
- Modify: `crates/cairn-node/src/db_diagnosis.rs` (the `RefusalScope` enum at ~350; add `stale_picture_refusal` beside `deliberate_refusal` / `node_state_refusal`; extend its unit tests at ~950–1045)
- Modify: `crates/cairn-node/src/chart_link.rs:~229` (record no longer holds the pair)
- Modify: `crates/cairn-node/src/chart_link/admit.rs:~94,~104` (`Unusable::LacksPair` scope and remedy)
- Modify: `crates/cairn-node/src/chart_link/judge.rs:~303` (the scope → refusal match)
- Modify: `crates/cairn-node/src/medication/signoff.rs:~448` (`ensure_same_charts`) and its pure test at ~540
- Tests: `crates/cairn-node/tests/{chart_link,unlink_from_record,unlink_as_shown,confirm_link,judgement_as_shown}.rs` — the asserts that pin `RefusalScope::Input` or the old wording for THESE refusals

**Interfaces:**
- Produces:
  - `pub enum RefusalScope { Input, NodeState, Stale }`. `Stale` is documented as: "a verdict about the caller's PICTURE: the record changed since the caller read it. The same call is refused until the caller re-reads; re-reading is the whole remedy."
  - `pub fn stale_picture_refusal(text: impl Into<String>) -> anyhow::Error`, marked so that `refusal_scope(&e) == Some(RefusalScope::Stale)`.

- [ ] **Step 1: Write the failing tests.**
  - In `db_diagnosis.rs` tests: `a_stale_picture_refusal_is_marked_stale` — `refusal_scope(&stale_picture_refusal("x")) == Some(RefusalScope::Stale)`, and the `{:#}` text is exactly `"x"`.
  - In `signoff.rs`'s pure test: rename it to `a_changed_set_is_refused_as_stale_naming_both_sets`. Expected text: `"the linked charts changed while this list was on screen (shown: {one}; now: {one}, {two}); nothing was signed"` (no "reload"), and `refusal_scope == Some(RefusalScope::Stale)`.
  - In the DB tests: flip each assert pinning `Input` for the "record no longer reads both" / `LacksPair` refusal to `Stale`. Change the expected wording to end at `"— nothing was done"` (chart_link) or `". Nothing was done"` (admit).
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib db_diagnosis signoff`. Expected: FAIL (no `Stale`).
- [ ] **Step 3: Implement.**
  - Add the variant and `stale_picture_refusal`.
  - `ensure_same_charts` returns `Err(stale_picture_refusal(...))` instead of `bail!`.
  - `chart_link.rs`'s record check uses `stale_picture_refusal`.
  - `Unusable::LacksPair` → `RefusalScope::Stale`, remedy `". Nothing was done"`.
  - `judge.rs` maps `RefusalScope::Stale => stale_picture_refusal(refusal.text)`.
  - Every exhaustive match on `RefusalScope` must name `Stale` explicitly; add no `_ =>` arm.
- [ ] **Step 4: Run** the lib tests (PASS). Then run the DB suites touched, with the three DB strings set, via `scripts/run-db-gated-tests.sh` restricted to those test binaries (`--nocapture`, and confirm no `skipped:` line). Expected: PASS.
- [ ] **Step 5: Commit** — `feat(#797): a refusal over a changed record is marked RefusalScope::Stale; node sentences say only what happened`.

### Task 2: `DataError::Stale` → `Retry::AfterReread` through the window's classifier

**Files:**
- Modify: `cairn-gui/cairn-gui-data/src/port.rs` (`DataError`, ~76: add `Stale(String)` with a doc mirroring `RefusalScope::Stale`)
- Modify: `cairn-gui/cairn-gui-live/src/error.rs:~164` (the `refusal_scope` match)
- Modify: every `DataError` match the compiler lists. Known: `cairn-gui-tauri/src/link/judgement_view.rs` (`judgement_error_from`), `cairn-gui-tauri/src/funnel/view.rs`, `cairn-gui-tauri/src/main.rs`, `cairn-gui-data/src/mock/{mod,funnel}.rs`.
- Modify: `cairn-gui/cairn-gui-tauri/src/funnel/view.rs:35-52` — rewrite `Retry::AfterReread`'s doc
- Tests: `cairn-gui-live/src/error.rs` tests; `cairn-gui-tauri/src/link/judgement_view_tests.rs`

**Interfaces:**
- Consumes: `RefusalScope::Stale` (Task 1).
- Produces:
  - `DataError::Stale(String)`.
  - `judgement_error_from(act, DataError::Stale(t)) == ErrorView { text: format!("The {act} was not recorded: {t}"), retry: Retry::AfterReread }`.

- [ ] **Step 1: Write the failing tests.**
  - `error.rs`: `a_stale_scope_is_a_stale_data_error` (build with `stale_picture_refusal`, expect `DataError::Stale(text)`).
  - `judgement_view_tests.rs`: `a_stale_refusal_is_not_recorded_and_re_reads` (`Retry::AfterReread`; text starts `"The link was not recorded: "`).
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live -p cairn-gui-tauri stale`. Expected: FAIL to compile (no `Stale`).
- [ ] **Step 3: Implement.**
  - Add the variant and map `Some(RefusalScope::Stale) => DataError::Stale(text)`.
  - Add the `judgement_error_from` arm.
  - The funnel's match gives `Stale` `Retry::AfterReread` too. Registration never raises it, but a match must not hide a new cause.
  - New `AfterReread` doc: "The screen is behind the record: the window re-reads the WHOLE chart and takes the act's button away — never a live retry. Two causes share it because the window does the same for both: the record changed under the clinician (a `Stale` refusal, or the window's own set check — nothing was recorded), or a write's COMMIT was interrupted (#713 — it may have been). The sentence says which; the class says only what the window does."
  - Fix the four JS comments that gloss it as "outcome unknown": `unlink.js:10`, `unlink.js:297`, `link.js:410`, `link.js:428`.
- [ ] **Step 4: Run** the same command. Expected: PASS. Then run the whole `cairn-gui-tauri` crate.
- [ ] **Step 5: Commit** — `feat(#797): DataError::Stale; Retry::AfterReread means "the screen is behind the record"`.

### Task 3: the window's own stale refusals use `AfterReread`, typed; no "reload the chart"

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/chart_set.rs:33-67`
- Modify: `cairn-gui/cairn-gui-tauri/src/link/judgement_view.rs` (add `stale()` beside `refused()` at :33; the commit-unknown sentence at :155)
- Modify: `src/link/same_person.rs:~50-60,~140-150`, `src/link/unlink.rs:~78,~125-135`, `src/link/unlink_view.rs:~30`, `src/link/view.rs:~20-33`, `src/duplicates/mod.rs:~245-262`
- Modify: `cairn-gui/cairn-gui-tauri/src/commands.rs` (`sign_off`/`sign_off_impl`/`cease`/`cease_impl` return `Result<_, ErrorView>`)
- Tests: `chart_set_tests.rs`, `chart_set_command_tests.rs`, `commands_tests.rs`, `link/*_tests.rs`, `duplicates/tests.rs`, `link/confirm.rs` tests, plus every test pinning an edited sentence

**Interfaces:**
- Produces:
  - `pub enum ChartSetError { Unreadable, Changed }` with `fn text(&self) -> &'static str`, so callers match on the variant. `check_displayed_set(read, displayed) -> Result<ChartSet, ChartSetError>`.
  - `pub fn stale(text: impl Into<String>) -> ErrorView` (`retry: Retry::AfterReread`), in `link/judgement_view.rs`.
  - `sign_off_impl(...) -> Result<SignOffReport, ErrorView>`, `cease_impl(...) -> Result<CeaseReport, ErrorView>`:
    - a `ChartSetError::Changed` → `stale(CHANGED)`;
    - node failures via `cairn_gui_live::error::data_error_from` → `Stale` → `AfterReread`, `Refused` → `Never`, `NotProvisioned` → `AfterOperator`, `Unavailable` → `Now`;
    - every other window refusal → `refused(…)`.
- Exact copy:
  - `CHANGED = "the linked charts changed while this list was on screen — nothing was done"`
  - `THIS_CHANGED = "this record changed while you were comparing — nothing was done"`
  - `LINE_STALE = "this pair has been judged since the list was drawn — nothing was done"`
  - `LINK_GONE = "that link is no longer part of this record — nothing was done"`
  - The commit-unknown sentence: `"Whether the {act} was recorded is NOT known — the connection was lost while event {event} was being saved. If it was recorded but a later judgement outranks it, the chart will look unchanged after it is read again."`
  - `OTHER_CHANGED` keeps "compare again" (its remedy re-reads the OTHER record) and stays `Retry::Never`.

- [ ] **Step 1: Write the failing tests.**
  - `chart_set_tests.rs`: `a_changed_set_is_a_typed_change` — `check_displayed_set` → `Err(ChartSetError::Changed)`; an empty list → `Err(ChartSetError::Unreadable)`.
  - For each of THIS_CHANGED (Compare, via `resolve_pair` and the duplicates admit), LINE_STALE and LINK_GONE: an existing refusal test (`same_person_tests.rs`, `unlink_tests.rs`, `unlink_offer_tests.rs`, `duplicates/tests.rs`, `confirm.rs`) gains `assert_eq!(err.retry, Retry::AfterReread)` and the new exact text.
  - `commands_tests.rs`: `a_changed_set_refuses_sign_off_and_cease_as_stale` (mock state, displayed set ≠ read set → `err.retry == Retry::AfterReread`, `err.text == CHANGED`).
  - New source guard in `chart_view_webview_tests.rs`: `no_window_sentence_says_reload_the_chart` — no `.rs` file under `cairn-gui-tauri/src` (outside `*_tests.rs`) and no `src-ui/*.js` contains `reload the chart`.
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri`. Expected: FAIL (compile errors, then red asserts).
- [ ] **Step 3: Implement.**
  - Replace every `e == CHANGED` with a match on `ChartSetError`.
  - Route the stale refusals through `stale()`.
  - Convert sign-off/cease to `ErrorView`.
- [ ] **Step 4: Run** the crate. Expected: PASS. Then run `cargo clippy --manifest-path cairn-gui/Cargo.toml --locked --workspace --tests -- -D warnings`.
- [ ] **Step 5: Commit** — `feat(#797): the window's stale refusals re-read (AfterReread), typed — never "reload the chart"`.

### Task 4: `med_list` says what changed — one pure Rust sentence over the set seen vs the set read

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/chart_set/change.rs` — pure. Move `chart_set.rs` to `chart_set/mod.rs` if needed; keep both under 500 lines.
- Create: `cairn-gui/cairn-gui-tauri/src/chart_set/change_tests.rs`
- Modify: `src/chart_set.rs` (`ChartPane` gains `set_change: Option<String>`; `chart_pane` takes it; factor an identity reader out of `linked_members`)
- Modify: `src/commands.rs` (`med_list` / `med_list_impl` gain `seen_charts: Option<Vec<String>>`, IPC name `seenCharts`)
- Modify: the Link / Unlink / Confirm reports (`link/judgement_view.rs`'s `LinkReportView` and the unlink/confirm report views) gain `charts_after: Vec<String>`, filled from the record set the node read inside the act's transaction. The implementer verifies where `cairn-node`'s judgement already reads `person_charts` under CARNLK (`chart_link.rs` ~223), and returns it through the `Asserted` report instead of re-reading it outside the transaction. A re-read outside the transaction could already include a colleague's change and would silence it.
- Test: `commands_tests.rs` (`the_webview_reads_…` field guards), `change_tests.rs`

**Interfaces:**
- Produces:
  - `pub fn set_change_sentence(seen: &ChartSet, now: &ChartSet, joined: &[MemberLine], departed: &[DepartedLine]) -> Option<String>`.
  - `pub enum DepartedLine { Read(MemberLine), Unread(Uuid) }`.
  - `pub async fn chart_identities(state: &AppState, ids: &[Uuid]) -> Result<Vec<MemberLine>, String>`, factored out of `linked_members`, which then calls it.
  - `med_list_impl(state, patient_id, seen_charts: Option<Vec<String>>) -> Result<ChartPane, String>`.
- Exact copy (built from each chart's `MemberLine::text`, in `ChartSet` order):
  - `"This record changed while it was on screen. Now part of it: {joined lines joined by "; "}. No longer part of it: {departed lines joined by "; "}."`
  - Each half appears only when non-empty.
  - An unread departed chart reads `"chart {id} (its name could not be read here)"`.
  - `None` when `seen == now`, when `seen_charts` is `None`, empty or unparseable, or when nothing on screen was a list (`seen_charts` absent).
  - `joined` lines come from the pane's own `members` (already read, so no second read). If `members` failed to read, a joined chart is also said as `"chart {id} (its name could not be read here)"`.

- [ ] **Step 1: Write the failing tests** in `change_tests.rs`:
  - `an_unchanged_set_says_nothing`;
  - `a_joined_chart_is_named_by_its_own_line`;
  - `a_departed_chart_is_named_and_an_unread_one_is_said_unread`;
  - `both_halves_in_one_sentence_joined_first`;
  - `the_sentence_names_only_identity_lines_never_medications` (the sentence equals the expected string exactly; build it from fixture `MemberLine`s, not `{}`-echoed identity text).
  - In `commands_tests.rs`: `med_list_says_nothing_without_a_seen_set` (mock: `seen_charts: None` → `set_change == None`) and `med_list_names_a_change_from_the_seen_set` (mock: seen = fixture + one extra id, read = fixture → `set_change` contains `"No longer part of it: chart "`).
  - A node DB test (`crates/cairn-node/tests/chart_link.rs`): `a_link_reports_the_set_it_left` — after linking A–B, `charts_after == {A, B}`.
- [ ] **Step 2: Run** `… cargo test … -p cairn-gui-tauri change`. Expected: FAIL.
- [ ] **Step 3: Implement** the pure function, the reader factoring and the parameter. `fields_read_by_the_webview("pane")` must come to contain `set_change` in Task 5; add that assert now, so it is RED until then.
- [ ] **Step 4: Run** the crate. Expected: PASS except the one field guard owned by Task 5.
- [ ] **Step 5: Commit** — `feat(#797): a re-read names what changed in the record, worded in Rust`.

### Task 5: the webview — re-read on every `after_reread`, say the change, mark a list that could not be re-read

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/main.js` (`refresh`, `signOff`, `cease`; new `markListStale`)
- Modify: `src-ui/link.js` (`compare` gains an `after_reread` arm; `sendJudgement` passes the stale flag)
- Modify: `src-ui/unlink.js` (`showUnlinkFailure` passes the stale flag)
- Modify: `src-ui/index.html` (a `<p id="chart-stale" hidden>` inside `#chart-warnings`, before `#chart-unshown`)
- Test: `src/chart_view_webview_tests.rs` (or a new `src/reread_webview_tests.rs` if that file would pass 500 lines)

**Interfaces:**
- Consumes: `ChartPane.set_change` (Task 4); `retry: "after_reread"` (Tasks 2–3).
- Produces:
  - `async function refresh(lead, staleIfFails, seenAfterOwnAct)`. It sends `{ patientId, seenCharts: seen }`, where `seen` is `seenAfterOwnAct` when given (an own judgement's `charts_after`), else `renderedCharts`. On an accepted answer, after `render`:
    - when `staleIfFails` is true, `say(join(lead, READ_AGAIN, pane.set_change))`;
    - otherwise, when `pane.set_change` is set, `say(join(lead, pane.set_change))`;
    - otherwise the outcome line is untouched.
    
    In the catch, after the existing lines, `if (staleIfFails && renderedPatient !== null) markListStale();`.
  - `const READ_AGAIN = "The chart has been read again — review it before you act on it.";`
  - `function markListStale()`:
    - sets `#chart-stale` to `LIST_OUT_OF_DATE` and un-hides `#chart-warnings`;
    - disables `#sign-off` with text `"Nothing to sign off — this list is out of date"`;
    - leaves the rows visible: a stale list, flagged, is an honest near-truth (principle 4); every write over it is refused by the set check.
    
    `drawList` clears `#chart-stale` like `#chart-unshown`; so does `clearDrawnList`.
  - `const LIST_OUT_OF_DATE = 'This list is out of date: the record changed while it was on screen, and reading it again failed. Press "Read this chart again".';`
  - `signOff` / `cease` catch `failure`:
    - `say(prefix + failureText(failure))`;
    - then `await refresh(el("outcome").textContent, failure && failure.retry === "after_reread")`;
    - the success path is unchanged (`refresh(outcome)`).

- [ ] **Step 1: Write the failing source guards** (each mutation-checked in Step 4):
  - `refresh_sends_the_set_on_screen` — `invoke("med_list", { patientId: patient, seenCharts: seen })`, with `seen` defaulting to `renderedCharts`.
  - `an_own_judgement_re_reads_against_the_set_it_left` — the recorded-verdict paths of `sendJudgement` (link.js) and of unlink.js pass `report.charts_after` as `refresh`'s third argument.
  - `read_again_is_said_only_after_an_accepted_answer` — `READ_AGAIN` appears after `render(answer, patient);` and before `catch`.
  - `a_failed_stale_re_read_marks_the_list_out_of_date` — `markListStale()` in the catch, guarded by `staleIfFails`.
  - `mark_list_stale_disables_sign_off_and_warns_above_the_table` — the HTML order `chart-warnings` < `chart-stale` < `med-table`.
  - `every_after_reread_handler_re_reads_the_whole_chart_stale` — `compare`, `sendJudgement`, `showUnlinkFailure`, `signOff` and `cease` each call `refresh(` with a second argument derived from `"after_reread"`.
  - `the_list_out_of_date_sentence_names_the_control` — `LIST_OUT_OF_DATE` contains `Read this chart again`.
  - Also update `duplicates/webview_tests.rs::a_decided_refusal_re_reads_the_banner_never_the_whole_chart`: `compare` now contains `refresh(` on its `after_reread` arm only. Keep the `"never"` arm banner-only and assert it by slicing.
- [ ] **Step 2: Run** `… cargo test … -p cairn-gui-tauri webview`. Expected: FAIL.
- [ ] **Step 3: Implement** in the JS and the HTML. Keep `main.js` under 500 lines; if it would pass, move `markListStale` and its two sentences into a new `src-ui/chart_stale.js` (a classic script after `stop_reasons.js`, added to `commands_tests.rs`'s script list).
- [ ] **Step 4: Run** (PASS). **Mutation-check** each new guard: break the fix, see red, restore by `cp` from a scratchpad copy, never `git checkout`. Then **walk headlessly**: copy `src-ui` to the scratchpad, inject a stub `invoke` with payloads dumped from fixture mode (a throwaway `#[ignore]` test), and run the same script against this branch and against `fix/799-800-801-chart-reread`. Scenarios:
  - a `CHANGED` sign-off refusal (`retry: "after_reread"`) → the list re-read, outcome = refusal + `READ_AGAIN` + the set-change sentence;
  - the same with the re-read failing → `#chart-stale` visible (computed style + ancestors), sign-off disabled;
  - `LINE_STALE` with an unchanged set → no "Now part of it";
  - an own Link recorded (`charts_after` = old ∪ other) → no set-change sentence; the same with one extra chart in the re-read (a colleague's concurrent link) → that one chart named;
  - a `Compare` refused stale → the whole chart re-read.
- [ ] **Step 5: Commit** — `feat(#797): every stale refusal re-reads the whole chart and says what changed; an un-re-readable list is marked out of date`.

### Task 6: "Read this chart again" — always visible; the failed-read sentences name it

**Files:**
- Modify: `src-ui/index.html` (`<button id="reread" type="button">Read this chart again</button>` directly under the identity header, inside `#chart-view`)
- Modify: `src-ui/main.js` (`el("reread").addEventListener("click", () => refresh());`)
- Modify: `src/link/record_links.rs:~278,~317,~327`, `src/duplicates/view.rs:~268`, `src-ui/duplicates.js` (`REVIEW_WAITS_FOR_LIST`)
- Test: `link/record_links_tests.rs` (its 3 asserts), `duplicates/view_tests.rs:~287`, `duplicates/webview_tests.rs::the_windows_own_sentences_about_a_missing_list_are_true`, `chart_view_webview_tests.rs`

**Interfaces:**
- Exact copy:
  - in `record_links.rs`, each `reload the chart` becomes `press "Read this chart again"`;
  - `duplicates/view.rs:268`: `"the other record changed while the banner was read — press \"Read this chart again\""`;
  - `REVIEW_WAITS_FOR_LIST`: `'Review needs this chart\'s medication list on screen, and it could not be read or shown, or is still being read. If it could not be read, press "Read this chart again".'`.

- [ ] **Step 1: Write the failing tests.**
  - Update the record_links and view sentence asserts to the new copy.
  - The missing-list guard expects `If it could not be read, press "Read this chart again"`.
  - New guard `the_read_again_control_is_always_there_and_re_reads`: the HTML has `id="reread"` inside `#chart-view` and after `#identity`; `main.js` wires its click to `refresh()`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS. Mutation-check the new guard. Run the full cairn-gui gate in CI's order, each step judged by its own exit code: `fmt --check`, `clippy --locked -D warnings`, `test --locked --workspace`, `RUSTDOCFLAGS=-D warnings cargo doc --locked --no-deps --workspace`, `cargo deny check`. Run the root workspace's `cargo test -p cairn-node`, and the DB-gated suites Task 1 touched.
- [ ] **Step 5: Commit** — `feat(#797): "Read this chart again" — always there; failed reads name it`.

### Task 7: docs

**Files:** `docs/developers/durable-rules.md` (a new block, newest first), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (a §13 step: a colleague's link lands mid-review, press Sign off → read the sentence → sign again; time it), `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1:** Durable rules:
  - `Stale` is routed by TYPE (`RefusalScope::Stale` → `DataError::Stale` → `Retry::AfterReread`), never by text;
  - node sentences name no window control;
  - `READ_AGAIN` only after an accepted answer;
  - `markListStale` keeps the rows and disables sign-off;
  - `set_change` is `None` for an equal or unreadable seen set;
  - the "Read this chart again" control is always visible.
  
  For each, the tempting wrong fix: matching `e == CHANGED` again; saying "has been read again" before the read lands; hiding the stale rows.
- [ ] **Step 2:** The RUNBOOK step and HANDOVER/ROADMAP. Run `scripts/check_closing_keywords.py` on the PR body.
- [ ] **Step 3: Commit** — `docs(#797): durable rules, runbook step, HANDOVER, ROADMAP`.
