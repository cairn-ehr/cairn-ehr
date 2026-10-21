# #832 — Medication withhold in the window (slice B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Tauri window shows the paper "W" on a medication line (and a cessation's reason), reveals each one's reason on click or focus, and offers the Withhold and Release gestures, all on top of slice A's record (PR #831).

**Architecture:** Every wording and every rule stays in Rust under `cargo test`: the row view gains hold views with local-time labels (`cairn-gui-tab-medications`), a new `hold` module in `cairn-gui-tauri` holds the pure plans (which thread a W is written on, which holds a Release ends) and the two commands, built on the Cease command's shape. The webview gets one new classic script, `holds.js`, that draws what Rust worded and calls the two commands. db/044's `gesture_kind` CHECK widens to time the two gestures.

**Tech Stack:** Rust (Tauri 2 commands, `chrono` for local time, already in the GUI lockfile), plain JS (no npm, no bundler), PostgreSQL 18 (one CHECK widening).

**Spec:** `docs/superpowers/specs/2026-10-21-medication-withhold-803-design.md` §4.2a, §4.5, §6, §7; [ADR-0081](../../spec/decisions/0081-medication-withhold-is-a-time-bounded-hold.md); durable rules `docs/developers/durable-rules.md` → the #803 block.

## Global Constraints

- **A hold sits BESIDE `MedicationStatus`; no `Withheld` variant.** `MedListRowView::current` and `can_cease` stay `status == Active`; a withheld line stays on the list, signable and ceasable.
- **A release names only holds the window SHOWED** (ADR-0081 decision 4): the webview sends the `withhold_event_id`s it drew, never "all holds on the line".
- **Every thread is written on its OWN chart** (ADR-0076 decision 2), and a wrong-chart hazard line writes only the opened chart's threads (`chart_set::cease_plan`'s rule).
- **No confirmation dialog** anywhere (principle 3). Reasons are inline fields.
- **Paper-parity K = 2** for a W (type the reason, press Withhold; "until" defaults to **until reviewed**); Release K = 1. The controls are **always visible** on the line (maintainer, 2026-10-21).
- **"until reviewed" is an explicit choice** in the control, never an empty time field standing in for it (principle 4).
- **The reason and withholder are revealed by click or focus**, hover a convenience only; the W is a `<button>` whose accessible label carries the reason (maintainer, 2026-10-21).
- **Times on screen are the workstation's local time**; the wire and the CLI stay UTC.
- **The webview decides nothing:** every sentence a clinician reads is built in Rust; JS only converts the `datetime-local` value to an ISO instant and draws.
- **No new event types, no `SCHEMA_GENERATION` change.** db/044 is edited in place (replay-gated by ADR-0080).
- House: TDD; files under 500 lines (`commands.rs` is already 532 — add nothing to it; `main.js` is 477 — keep it under 500); inline docs for a junior; `scripts/check_closing_keywords.py` before each commit; commit footer `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.

## Review Focus

1. **A W typed while a re-read lands** (a colleague's link, another line's Stop): the reason, the until choice and the time must survive the redraw (#799's rule) — Task 5 extends `typedReasons`/`restoreReasons`, pinned by a guard.
2. **A Release pressed after a peer's hold arrived but before the re-read drew it:** the new hold is NOT ended (ADR-0081 decision 4) — Task 3's `release_plan` ignores holds not in `shown`, tested.
3. **A Release of a hold that ended meanwhile** (expired, or released by a colleague): the outcome says it was left as it is, never "released 0" silently; a release with nothing left to end is a stale refusal — Task 3 / Task 4 tests.
4. **"Until a time" chosen with the time left empty, or a time already past:** a worded refusal, never a hold that is silently "until reviewed" — Task 4 (`parse_until`) and the floor's local-door refusal (until ≤ start), surfaced through `node_failure`.
5. **A hold on a wrong-chart hazard line, or on a sibling thread of a reconciled line:** the W goes on the opened chart's thread; a release of a sibling thread's hold names that thread — Task 3 tests.

---

## File Structure

| File | Responsibility |
|---|---|
| `db/044_ui_gesture_timing.sql` (modify) | widen `gesture_kind` to `withhold`, `release` (paired `ALTER`, #207) |
| `crates/cairn-node/tests/ui_timing.rs` (modify) | the two new kinds record |
| `crates/cairn-medication-view/src/fixtures.rs` (modify) | sample chart gains holds + a cessation reason (the `--mock` window shows them) |
| `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/hold_view.rs` (create) | pure: a hold / a cessation as the line words it, in local time |
| `…/cairn-gui-tab-medications/src/row_view.rs` (modify) | `MedListRowView` gains `holds`, `can_withhold`, `can_release`, `cessation_detail`; withheld status wording |
| `…/cairn-gui-tab-medications/src/lib.rs` (modify) | the semantic tree announces the W, Withhold, Release |
| `cairn-gui/cairn-gui-tauri/src/hold/plan.rs` (create) | pure: `withhold_target`, `release_plan` |
| `cairn-gui/cairn-gui-tauri/src/hold/mod.rs` (create) | `withhold` / `release` commands + `_impl`s, `parse_until`, `HoldReport` |
| `cairn-gui/cairn-gui-tauri/src/hold/outcome.rs` (create) | pure: `withhold_outcome`, `release_outcome` sentences |
| `cairn-gui/cairn-gui-tauri/src/hold/*_tests.rs` (create) | tests for the above |
| `cairn-gui/cairn-gui-tauri/src/main.rs` (modify) | `mod hold;` + register the two commands |
| `cairn-gui/cairn-gui-tauri/src-ui/holds.js` (create) | draw the status cell and the W/Release controls; `withhold()`, `release()` |
| `…/src-ui/main.js`, `stop_reasons.js`, `index.html`, `style.css` (modify) | call holds.js; keep until fields across a re-read; load the script; style the W |
| `cairn-gui/cairn-gui-tauri/src/holds_webview_tests.rs` (create) | source guards over holds.js |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `TEMPLATE.md` (modify) | §15: the W gestures' human timing + a11y checks |

---

### Task 1: db/044 times the two gestures

**Files:** Modify `db/044_ui_gesture_timing.sql`; Test `crates/cairn-node/tests/ui_timing.rs`

**Interfaces:** Produces: `cairn_node::ui_timing::record_gesture(&c, "withhold" | "release", n, ms)` succeeds.

- [ ] **Step 1: Failing test** `the_withhold_gestures_are_timed`: `TRUNCATE ui_gesture_timing`, `record_gesture(&c, "withhold", 1, 800)` and `record_gesture(&c, "release", 1, 600)` both `Ok`; `read_aggregates` has keys `("withhold","1-3")` and `("release","1-3")`.
- [ ] **Step 2: Run** `CAIRN_TEST_PG=… cargo test -p cairn-node --test ui_timing -- --nocapture` → FAIL (23514 check violation). Confirm no `skipped:` line.
- [ ] **Step 3: Implement.** Leave the `CREATE TABLE`'s CHECK reading the full new list and, after it, the paired convergence for an existing table (the db/007 `node_event_role_check` idiom):
  ```sql
  ALTER TABLE ui_gesture_timing DROP CONSTRAINT IF EXISTS ui_gesture_timing_kind_ck;
  ALTER TABLE ui_gesture_timing ADD CONSTRAINT ui_gesture_timing_kind_ck
      CHECK (gesture_kind IN ('signoff', 'cease', 'withhold', 'release'));
  ```
  with a one-line comment naming #832. A db/044 edit needs a rebuild (it is `include_str!`'d).
- [ ] **Step 4: Run** the suite → PASS (all `ui_timing` tests).
- [ ] **Step 5: Commit** `feat(#832): db/044 times the withhold and release gestures`.

### Task 2: the row says W, in local time, and a cessation's reason

**Files:** Create `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/hold_view.rs`; modify `row_view.rs`, `lib.rs`, the crate's `Cargo.toml`; modify `crates/cairn-medication-view/src/fixtures.rs`.

**Interfaces:**
- Consumes: `cairn_medication_view::MedicationHold { withhold_event_id, medication_id, reason, starts_at, until, withholder, pending }` (RFC 3339 UTC text; `until` may be `"reviewed"`); `MedicationRow::{holds, cessation_reason, is_withheld()}`.
- Produces (all `pub`, `Serialize`):
  ```rust
  pub struct HoldView {
      pub withhold_event_id: String, // what Release sends back
      pub symbol: String,            // the button's text: "W until 20:00" | "W until reviewed" | "W from Thu 22 Oct 08:00"
      pub detail: String,            // revealed text AND accessible label
      pub pending: bool,
  }
  pub fn hold_view<Tz: chrono::TimeZone>(h: &MedicationHold, now: &chrono::DateTime<Tz>) -> HoldView
      where Tz::Offset: std::fmt::Display;
  pub fn local_instant<Tz: chrono::TimeZone>(utc: &str, now: &chrono::DateTime<Tz>) -> String
      where Tz::Offset: std::fmt::Display;
  pub fn cessation_detail(reason: &str) -> String; // "Stopped: <reason>"
  ```
  `MedListRowView` gains `holds: Vec<HoldView>`, `can_withhold: bool` (= `current`), `can_release: bool` (= `!holds.is_empty()`: a release also cancels a pending hold), `cessation_detail: Option<String>`.
- `local_instant`: parse with `DateTime::parse_from_rfc3339`, convert to `now`'s zone; `"%H:%M"` when on `now`'s local date, else `"%a %-d %b %H:%M"`; unparseable → the input text unchanged (never blank).
- Wording: `symbol` = pending → `"W from {start}"`; else `"W until reviewed"` / `"W until {until}"`. `detail` = `"Withheld until {until|reviewed} by {short_kid}: {reason}"`, pending → `"Withheld from {start} until {until|reviewed} by {short_kid}: {reason}"`. `status_label` = `"withheld"` when `row.is_withheld()`, else unchanged; `current` unchanged.

- [ ] **Step 1: Failing tests** in `hold_view.rs` (use `chrono::FixedOffset::east_opt(10*3600)`, `now` = 2026-10-21T09:00+10:00):
  - `a_hold_ending_today_shows_the_local_clock_time`: until `2026-10-21T10:00:00Z` → symbol `"W until 20:00"`.
  - `a_hold_ending_another_day_names_the_day`: until `2026-10-21T22:00:00Z` → `"W until Thu 22 Oct 08:00"`.
  - `an_open_hold_reads_until_reviewed` and its `detail` == `"Withheld until reviewed by 1a2b3c4d: BP 85/50"` (withholder `"1a2b3c4d5e6f"`).
  - `a_pending_hold_names_its_start` → symbol starts `"W from "`, detail starts `"Withheld from "`.
  - `an_unparseable_instant_is_shown_as_recorded`: until `"garbage"` → symbol `"W until garbage"`.
  - `a_ceased_line_reveals_its_reason`: `cessation_detail("rash") == "Stopped: rash"`.
  In `row_view.rs` tests: `a_withheld_line_stays_current_signable_and_ceasable` (in-force hold → `status_label == "withheld"`, `current`, `can_cease`, `can_withhold`, `can_release`, `will_be_signed` all true); `a_pending_hold_leaves_the_line_reading_current` (`status_label == "current"`, `can_release`); `a_ceased_line_carries_its_reason_and_no_controls` (`cessation_detail == Some("Stopped: rash")`, `!can_withhold`, `!can_release`). In `lib.rs` `semantic_tests`: `every_hold_and_hold_control_is_a_labelled_button` (fixture chart: a Button field `hold-<event id>` labelled with its `detail`; `withhold-<group>` labelled `"Withhold <primary>"` on each current row; `release-<group>` labelled `"Release the holds on <primary>"` on each row with holds; `assert_complete` still passes).
- [ ] **Step 2: Run** `cd cairn-gui && cargo test -p cairn-gui-tab-medications` → FAIL (missing items).
- [ ] **Step 3: Implement.** Add `chrono = { version = "0.4", default-features = false, features = ["clock"] }` (already in `Cargo.lock`; MIT/Apache-2.0 — check `cargo deny check licenses` passes). `build_view` takes `chrono::Local::now()` once and passes it to `build_row` (keep `build_view(&list)`'s signature; only the label functions are pure in `now`). Fixtures: `sample_rows` gains, on `stale` (sertraline) an in-force `"reviewed"` hold, on `signed_by_other` (amlodipine) a pending hold from `2098-12-31T22:00:00Z` until `"reviewed"`, and `ceased.cessation_reason = Some("gastric irritation")`; withholder `"fixture-clinician-b"` (never hex, house rule 6); event ids `uid(110)`, `uid(120)`. Update the fixture doc's shape list. Semantic tree: per row, after the cease button, one Button per hold (`detail` label), then Withhold, then Release; the row's ListItem label appends each hold's `symbol` and the `cessation_detail` when present.
- [ ] **Step 4: Run** `cargo test -p cairn-gui-tab-medications` and, in the root tree, `cargo test -p cairn-medication-view` → PASS. Fixture changes can move counts elsewhere: run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace` and fix any test that asserted the old fixture shape (never weaken a safety assertion; adjust the expected fixture value).
- [ ] **Step 5: Commit** `feat(#832): the row shows W and a cessation's reason in local time`.

### Task 3: which thread a W is written on, which holds a Release ends

**Files:** Create `cairn-gui/cairn-gui-tauri/src/hold/plan.rs`, `src/hold/plan_tests.rs`; `src/hold/mod.rs` (module skeleton: `pub(crate) mod plan;` `mod outcome;`).

**Interfaces:**
```rust
/// Err carries the refusal sentence.
pub fn withhold_target(row: &MedicationRow, opened: Uuid) -> Result<(Uuid /*thread*/, Uuid /*its chart*/), String>;
pub struct ReleasePlan {
    pub write: Vec<(Uuid /*thread*/, Uuid /*its chart*/, Vec<Uuid> /*withhold event ids*/)>, // thread order as members
    pub held_back: Vec<String>, // sentences, as CeasePlan::held_back
    pub gone: Vec<Uuid>,        // shown ids no longer on the line (ended or released meanwhile)
}
pub fn release_plan(row: &MedicationRow, shown: &[Uuid], opened: Uuid) -> ReleasePlan;
```
Rules: `withhold_target` → the member whose `medication_id == row.group_id` (the line's representative thread, design §4.2a) when the row is not a hazard or that member is on `opened`; else, on a hazard line, the first member on `opened`; else `Err("this line may carry another person's drug and has no record on this chart — withhold it from its own chart")`. `release_plan` → for each hold of `row.holds` whose id is in `shown` (distinct), grouped by `hold.medication_id`, chart = that member's `patient_id`; on a hazard line a thread off `opened` goes to `held_back` (`"hold {id} on thread {t} on chart {c}: NOT released from here — this line may carry another person's drug; release it from its own chart"`); a hold whose thread is not a member goes to `held_back` too; `shown` ids absent from `row.holds` go to `gone`. A hold present in `row.holds` but absent from `shown` is never written.

- [ ] **Step 1: Failing tests** (build rows with `cairn_medication_view` types directly, as `chart_set/tests.rs` does): `a_w_goes_on_the_lines_own_thread_on_its_own_chart` (linked line, representative thread on the other member chart → that chart); `a_hazard_line_withholds_on_the_opened_charts_thread`; `a_hazard_line_with_nothing_on_this_chart_is_refused`; `a_release_ends_only_the_holds_it_was_shown` (row has holds A, B; shown [A] → write names only A); `a_release_writes_one_event_per_thread` (reconciled pair, a hold on each member → two writes, each naming its own hold); `a_hold_that_ended_meanwhile_is_named_gone`; `a_hazard_line_holds_back_another_charts_hold`.
- [ ] **Step 2: Run** `cd cairn-gui && cargo test -p cairn-gui-tauri hold::plan` → FAIL.
- [ ] **Step 3: Implement** both functions in `plan.rs` (pure, doc-commented with the ADR rules above).
- [ ] **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(#832): pure plans for a window withhold and release`.

### Task 4: the `withhold` and `release` commands

**Files:** `src/hold/mod.rs`, `src/hold/outcome.rs`, `src/hold/outcome_tests.rs`, `src/hold/command_tests.rs`, `src/main.rs`; rename `write_failure::CeaseUnknown` → `UnknownThread` (pure rename, its three users).

**Interfaces:**
- Consumes: Task 3's plans; `cairn_node::medication::{withhold_medication, release_medication_holds, WithholdMedicationInput, AuthorParams}`; `cairn_event::medication::HoldUntil`; from `commands.rs` make `read_chart_of` and `record_timing` `pub(crate)` (record_timing is private today).
- Produces:
  ```rust
  #[derive(Debug, serde::Serialize)]
  pub struct HoldReport { pub text: String, pub behind_the_record: Option<Recorded> }
  #[tauri::command] pub async fn withhold(state, group_id: String, reason: String, until: String, patient_id: String, charts: Vec<String>) -> Result<HoldReport, ErrorView>;
  pub async fn withhold_impl(state: &AppState, patient_id: &str, charts: Vec<String>, group_id: &str, reason: &str, until: &str) -> Result<HoldReport, ErrorView>;
  #[tauri::command] pub async fn release(state, group_id: String, withholds: Vec<String>, reason: String, patient_id: String, charts: Vec<String>) -> Result<HoldReport, ErrorView>;
  pub async fn release_impl(state: &AppState, patient_id: &str, charts: Vec<String>, group_id: &str, withholds: &[String], reason: &str) -> Result<HoldReport, ErrorView>;
  pub(crate) fn parse_until(until: &str) -> Result<UntilChoice, String>; // UntilChoice { Reviewed, At(String) } — owned, mapped to HoldUntil at the call
  pub(crate) fn withhold_outcome(thread: Uuid, result: WithholdResult) -> (String, Option<Recorded>);
  pub(crate) fn release_outcome(released: usize, gone: usize, failed: &[String], unknown: &[UnknownThread]) -> String;
  ```
- Refusal order, as `cease_impl`: `displayed_patient` → `read_chart_of` → `check_displayed_set` → fixture-mode refusal → (withhold) empty reason `"withholding a drug needs a reason — the chart must say why it is not being given"` → `parse_until` → group id parse → row lookup (`stale("that drug is no longer on this chart — nothing was done")`) → plan → `live_key` (`key_locked("your signing key is locked — unlock it to withhold a drug")` / `"… to release a hold"`) → node key. Release: shown ids that fail to parse → `refused("not a hold id")`; a plan with empty `write` and empty `held_back` → `stale("the holds shown are no longer on this line — nothing was done")`; reason trimmed, empty → `None`.
- `parse_until`: `"reviewed"` → Reviewed; `""` → `Err("choose when the hold ends: a time, or until reviewed")`; RFC 3339 (`DateTime::parse_from_rfc3339`) → `At(text)`; anything else → `Err("not a time: <text>")`.
- Writes: withhold = ONE `withhold_medication` on the target's chart with `from: None`; a commit with unknown outcome (`db_diagnosis::commit_outcome_unknown_event`) → `behind_the_record: Some(Recorded::Unknown)` and a sentence saying it is NOT known whether the drug was withheld (name the event); any other error → `Err(node_failure(&e))`. Release = one `release_medication_holds` per `write` entry, each its own transaction (ADR-0060 decision 7), failures and unknowns collected as `cease_impl` does; `held_back` sentences join `failed`. Time each with `record_timing(state, "withhold" | "release", 1, ms)` only when no outcome is unknown.
- Sentences: withhold Ok → `"Withheld — the W is recorded on this drug."` (final review: was "the chart now shows the W", said before a re-read that may fail); release → `"Released {n} hold(s) on this drug."` (omitted when n == 0 and something is unknown, as `cease_outcome` does), then `"{k} hold(s) shown had already ended and were left as they are."` when `gone > 0`, then the NOT-released and NOT-known parts in `cease_outcome`'s words with "released" for "stopped".

- [ ] **Step 1: Failing tests.** `outcome_tests.rs`: every arm of both outcome builders (Ok, unknown, gone, failed, mixed; "Released 0" absent beside an unknown). `command_tests.rs` (against `AppState::mock`, like `commands_tests.rs`): `withhold_refuses_a_chart_that_is_not_on_screen`, `release_refuses_a_chart_that_is_not_on_screen`, `a_withhold_on_the_chart_on_screen_meets_the_fixture_refusal`; `parse_until` table (`"reviewed"`, `""`, `"2026-10-21T20:00:00.000Z"`, `"2026-10-21T20:00:00+10:00"`, `"tomorrow"`). In `commands_tests.rs`: `the_webview_calls_only_registered_commands` will need holds.js (Task 5) — not here.
- [ ] **Step 2: Run** `cd cairn-gui && cargo test -p cairn-gui-tauri hold::` → FAIL.
- [ ] **Step 3: Implement** in `hold/mod.rs` + `hold/outcome.rs`; register `hold::withhold, hold::release` in `main.rs`'s `generate_handler!`. If a new `LocalDbFault`-wrapped read or commit appears, run `db_errors_stay_legible.rs` / the commit inventory guard (memory: pinned-count guards) — none is expected, since both orchestrators already exist.
- [ ] **Step 4: Run** `cargo test -p cairn-gui-tauri` (and `cargo clippy -p cairn-gui-tauri --all-targets -- -D warnings`) → PASS.
- [ ] **Step 5: Commit** `feat(#832): the window's withhold and release commands`.

### Task 5: the webview draws the W and the two gestures

**Files:** Create `src-ui/holds.js`, `src/holds_webview_tests.rs`; modify `src-ui/main.js`, `stop_reasons.js`, `index.html`, `style.css`, `src/commands_tests.rs` (script inventory + `fields_read_by_the_webview` scans holds.js), `src/main.rs` (`#[cfg(test)] mod holds_webview_tests;`).

**Interfaces:** Consumes Task 2's row fields (`holds[].{withhold_event_id, symbol, detail, pending}`, `can_withhold`, `can_release`, `cessation_detail`) and Task 4's commands (`invoke("withhold", {groupId, reason, until, patientId, charts})`, `invoke("release", {groupId, withholds, reason, patientId, charts})`). Produces globals `statusCell(row)`, `holdControls(row)`, `withhold(...)`, `release(...)`.

- `statusCell(row)` → a `<td>`: `row.status_label`, then per hold a `<button type="button" class="hold-symbol">` with text `hold.symbol`, `aria-label` = `hold.detail`, `aria-expanded`, `aria-controls` → a sibling `<span id="hold-detail-<id>" hidden>` holding `hold.detail`. Reveal on `focus` and `mouseenter`; hide on `blur`/`mouseleave` unless pinned; `click` toggles pinned (WebKit does not focus a clicked button, so click must work alone). A ceased row with `cessation_detail`: the status word itself is such a button (`aria-label` = `"ceased — " + cessation_detail`).
- `holdControls(row)` → nodes appended to the action cell: when `can_withhold`: `input#reason-w-<g>` (aria-label `"Reason for withholding " + primary`, placeholder `"W reason"`), `select#until-kind-<g>` (options `reviewed` "until reviewed" — default — and `time` "until a time"), `input#until-at-<g>` type `datetime-local` (aria-label `"Withhold until (local time)"`), `button#withhold-<g>` "Withhold" (aria-label `"Withhold " + primary`). When `can_release`: `input#reason-r-<g>` (optional, aria-label `"Reason for releasing " + primary`), `button#release-<g>` "Release" (aria-label `"Release the holds on " + primary`), which sends `row.holds.map(h => h.withhold_event_id)` — the ids DRAWN.
- `withhold(groupId, reason, kind, local)`: `until = kind === "reviewed" ? "reviewed" : (local ? new Date(local).toISOString() : "")` (an empty or invalid time is sent as `""`, so Rust words the refusal; guard `isNaN(date)` → `""`). Then exactly `cease`'s shape: `say(report.text)` + `refresh(el("outcome").textContent, report.behind_the_record)`; catch → `say("Could not withhold this drug: " + failureText(failure))` + `refresh(…, behindTheRecord(failure))`. `release` likewise ("Could not release this hold: ").
- `main.js`: `renderRow` uses `statusCell(row)` for the status cell, sets `data-withheld="true"` on a `"withheld"` row, appends `...holdControls(row)` to the action cell. `index.html`: the action column header becomes `Stop or withhold`; load `holds.js` right after `stop_reasons.js`. `stop_reasons.js`: `typedReasons` also reads `[id^="until-"]` fields; it records the caret only for a field that has one, and `restoreReasons` calls `setSelectionRange` only when `typeof typed.focused.start === "number"` (a `<select>` or `datetime-local` has none and throws).

- [ ] **Step 1: Failing guards** in `holds_webview_tests.rs` (`source_guard::js_fn`, as `chart_view_webview_tests.rs`): `the_w_is_a_button_whose_label_is_its_reason` (`statusCell` contains `"button"`, `aria-label`, `hold.detail`, `aria-expanded`, and listeners for `"focus"` and `"click"`); `a_release_sends_only_the_holds_it_drew` (`holdControls` maps `row.holds` to `withhold_event_id` and passes it to `release(`; holds.js never calls `invoke("release"` with anything read from elsewhere); `until_reviewed_is_the_default_and_never_an_empty_time` (`reviewed` is the first `<option>`; `withhold` sends `"reviewed"` only when the kind is `reviewed`); `both_gestures_re_read_like_cease` (each catch block says before `refresh(` and passes `behindTheRecord(failure)`; success passes `report.behind_the_record`); `a_re_read_keeps_the_until_choice_and_time` (`typedReasons` selector contains `[id^="until-"]`; `restoreReasons` guards `setSelectionRange` with `typeof`). Extend `commands_tests.rs`: holds.js in the script inventory and in `fields_read_by_the_webview` (so `row.holds`, `hold.*` — add `"hold"` binding against a serialized `HoldView` — and `report.*` are checked against what Rust sends); the `after_reread` catch-block loop gains `("holds.js", HOLDS_JS, "withhold")` and `("holds.js", HOLDS_JS, "release")`.
- [ ] **Step 2: Run** `cd cairn-gui && cargo test -p cairn-gui-tauri` → FAIL.
- [ ] **Step 3: Implement** holds.js (header comment in main.js's style: why it exists, classic script sharing main.js's scope), the main.js/index.html/stop_reasons.js/style.css edits. `main.js` must stay < 500 lines.
- [ ] **Step 4: Run** `cargo test -p cairn-gui-tauri` → PASS. **Then walk it headlessly** (memory: webview mock walk recipe — copy `src-ui`, stub `window.__TAURI__.core.invoke` with payloads DUMPED from Rust, serve, drive with Playwright): the fixture chart shows "W until reviewed" on sertraline and "W from Fri 31 Dec …" on amlodipine; Tab onto a W reveals its detail (assert computed visibility, not textContent); click reveals it with focus elsewhere; ibuprofen's "ceased" reveals "Stopped: gastric irritation"; Withhold with kind "time" and no time invokes `withhold` with `until: ""`; Release invokes `release` with exactly the drawn ids; a stub re-read keeps a typed W reason, the until kind and the time.
- [ ] **Step 5: Commit** `feat(#832): the webview draws the W and offers Withhold and Release`.

### Task 6: RUNBOOK §15 and the durable rules

**Files:** `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `results/TEMPLATE.md`, `docs/developers/durable-rules.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1:** RUNBOOK `## 15. Withhold and release a drug (#832)`: setup (a live node, a chart with a current drug), the timed gestures (chart open → W recorded ≤ 10 s, budget per §7; Release ≤ 5 s), how to read the `withhold`/`release` aggregates back (§5's query), and the VoiceOver/keyboard checks (Tab reaches each W and VoiceOver reads its reason; the until select and time are labelled; Release is named by drug; a ceased line's reason reads on focus). TEMPLATE gains the matching section. Run every command the RUNBOOK names against a real database before writing it down (HANDOVER lesson: an operator procedure is a claim).
- [ ] **Step 2:** durable-rules: a `⇒ #832` block — the release sends only drawn ids (pin `a_release_ends_only_the_holds_it_was_shown` + the webview guard), the W is a button whose label is the reason (pin), `status_label` "withheld" is wording only (`current` stays the fact), until-reviewed is the default and never an empty time, the typed until fields survive a re-read.
- [ ] **Step 3: Commit** `docs(#832): RUNBOOK §15 and durable rules`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing W in the drug chart's administration slot(s) with initials, and the reason in the slot code or a short note; rescinding = a note; resuming = giving the next dose, no act.
- **Steps:** paper *N* = 2 (write W + initials; reason) → architecture-forced *M* = 2 (type the reason in the line's W field; press Withhold — "until reviewed" is the default; authorship from the unlocked key, ADR-0053) → UI bundling target *K* = 2, met by the always-visible controls (maintainer's choice, 2026-10-21). A W until a stated time adds the time entry, as paper's slot choice does. **Resume:** paper 0 → Cairn 0 (expiry is a read-time fold). **Rescind:** paper 1 → Cairn 1 (Release; its reason optional). `M > N` nowhere.
- **Time + cognitive load:** budget ≤ the paper W, about 10 s from the chart open to recorded; Release ≤ 5 s. Measurement is owed by this slice's human RUNBOOK pass (§15); the window records the write half in `ui_gesture_timing` (`withhold`, `release`). Cognitive load: the W reads on the line itself in local time, with its reason one focus away; no dialog.
