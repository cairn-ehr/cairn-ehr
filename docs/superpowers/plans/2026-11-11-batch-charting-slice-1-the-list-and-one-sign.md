# Batch charting slice 1 — the medication list and one Sign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician types several medication lines into a durable, per-writer unsigned list on the chart, and one Sign charts every ticked line — one key release, each line its own transaction.

**Architecture:**
- **One new SQL file, `db/078`,** holds the mutable draft store `medication_draft_line`, reachable only through definer doors for `cairn_window`, as db/065 does for notes. Nothing in it is signed, synced, backed up or exported.
- **The tick rules are pure functions** in `cairn-node`: the default tick, whether a tick still holds at Sign, and the duplicate guard.
- **A node orchestrator signs one draft line.** It submits the line's event and deletes its draft row in one transaction, so a signed line can never stay in the list to be signed twice.
- **The window gets `med_list_*` commands and a list pane.** The pane reuses slice 1a's line editor, and its Sign drives the per-line orchestrator after one key release.
- **The person menu counts each writer's unsigned lines.**
- **No wire change.** Every event signed here is slice 1a's `clinical.medication.asserted`.

**Tech Stack:** Rust (cairn-node, cairn-gui-tauri), PostgreSQL ≥ 18 (plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-11-batch-charting-1072-design.md`](../specs/2026-11-11-batch-charting-1072-design.md).
- **Builds:** §3.1, §3.2, §3.3, §3.5's existing rule (a *Start* line with a warning needs a reason), and §3.4's duplicate guard (a second line of a drug already in the list or on the chart gets no default tick).
- **Slice 2, not here:** "Set aside", set-aside versions as their own event type, "Use this version instead", "From:" and the "why" (all wire).
- **Slice 3, not here:** the administrator's warning threshold. Until it lands, a warning is an allergy mark (`LineCheck.marks` non-empty), which is the design's default threshold.

## Global Constraints

- AGPL-3.0. No new dependency. No wire change: no new event type, no new payload key, no twin change.
- **The draft store is mutable working state, never the record.** Doors are `SECURITY DEFINER SET search_path = public, pg_temp`, `REVOKE ALL … FROM PUBLIC`, and `GRANT EXECUTE` to `cairn_window` only. The store is in no backup and no export (ADR-0087's ruling). Deleting a line leaves no record (design §9 Q3).
- **Never committed by a timer.** No door, tick or runner turns a draft line into an event; only Sign does.
- **Per writer.** `writer` is the vault staff id named in who's here when the line is first saved, normalised by `cairn_vault_staff_id`. It is NULL when nobody is named. A line's writer never changes. A list is shown only when who's here names its writer (a NULL-writer list only when nobody is named).
- **The default tick** = a named writer ∧ finished ∧ every part coded ∧ `form` read ∧ dose read (`dose_amount` and `dose_unit` both `Some`) ∧ directions read (`frequency` `Some`, or `prn`, or `stat`) ∧ the check ran (`!unverified`) ∧ no mark ∧ drug allergies ≠ `ListStatus::NotYetAsked` ∧ not a duplicate. It is **derived at every read and never stored**. Only a hand tick is stored.
- **A hand tick** is stored with `ticked_against`, the warning refs (`{source, ref}`) on the line when it was ticked. A save that changes `line_text`, `parts` or `charted_as` clears `hand_ticked` and `ticked_against` **in the door**, whatever the webview sends.
- **At Sign, every refusal of the whole Sign comes before the key.** Whole-Sign refusals are for a changed picture only: the chart set, the list, or a tick that no longer holds. A per-line defect (blank name, a pick that is not a drug, an unknown press) skips that line with its reason, and the others sign (ADR-0060 decision 1).
- **Each line commits in its own transaction:** the event plus its draft row's delete (ADR-0060 decision 7). There is no shared transaction across lines.
- **The webview's answer is never trusted.** Ticks, completeness, checks and duplicates are all re-derived in Rust at Sign, on a fresh read.
- **Copy (exact; pinned by tests):**
  - `MED_LIST_CHANGED = "{n} ticked line(s) changed since they were ticked: nothing was signed. Look at them again"`
  - `MED_LIST_NOT_YOURS = "This list was written by {name}; only they can sign it"`
  - `MED_LIST_STALE = "The list changed since it was shown: nothing was signed. Look at it again"`
  - the outcome `"Charted {k} of {n}"`, then for each line not charted, `"{drug line} — not charted: {reason}"`
  - the menu row `"{n} unsigned medication line(s)"`
  - the unfinished-line label `"unfinished"`
  - slice 1a's copy is unchanged: `NEEDS_A_REASON`, `CHECK_DID_NOT_RUN`, `NAME_IT`, `NOT_A_LISTED_DRUG`, and the `Whether this medication was charted is NOT known — …` lead.
- **No confirmation dialog.** Warnings sit on the line.
- **Files stay under 500 lines.** `med_chart.rs` is at 504 and `med_chart.js` at 475, so new code goes in new files, and what moves out of `med_chart.rs` shrinks it.
- `SCHEMA_GENERATION` 77 → **78**. `CAIRN_ALLOW_DB_SKIP=1` only for a deliberately DB-free run. A DB-gated suite that prints `skipped:` is not a pass.

## Review Focus

1. **A line edited after its hand tick** (dose 25 → 50): the tick is cleared by the door, not only in the webview, so a stale webview cannot sign the new text on the old tick. Pinned in Task 1.
2. **The same writer's list opened on a second terminal:** it reads "on another terminal" with Continue here, never a second copy. After the take, a save from the first terminal answers `moved`. Pinned in Task 1.
3. **Who's here switched mid-list:** A's lines leave the screen. B pressing Sign never covers them. A strip credential naming someone other than the list's writer is refused `MED_LIST_NOT_YOURS` before the key, and a held key whose kid is not the writer's current key is refused before anything is written. Pinned in Task 4.
4. **A connection lost mid-Sign** (after line 5 of 9): lines 1–5 are charted and gone from the list. Line 6's outcome is worded NOT known. Lines 7–9 are not attempted and stay in the list, ticked. Every line's spend is in that line's own transaction, so the re-read list is the truth. Pinned in Task 4.
5. **An unconscious patient, allergies not yet asked:** no line is ever default-ticked, every hand-ticked line signs, and the drug-allergy status stays "not yet asked". Pinned in Task 2 (pure) and Task 4.

---

### Task 1: The draft store (`db/078`)

**Files:**
- Create: `db/078_medication_draft_list.sql`, `crates/cairn-node/tests/medication_draft_list_store.rs`
- Modify: `crates/cairn-node/src/db.rs` (register `078_medication_draft_list` after 077), `crates/cairn-event/src/schema_generation.rs` (78), and any pinned replay/generation guard the bump moves.

**Interfaces — Produces:**
```sql
CREATE TABLE IF NOT EXISTS medication_draft_line (
  line_id uuid PRIMARY KEY, patient_id uuid NOT NULL,
  writer text,                                   -- normalised staff id; NULL = not known
  terminal_id text NOT NULL CHECK (btrim(terminal_id) <> '' AND terminal_id !~ '[[:cntrl:]]'),
  seq int NOT NULL,                              -- display order within (patient, writer)
  charted_as text NOT NULL CHECK (charted_as IN ('started-here','pre-existing')),
  line_text text NOT NULL,                       -- the whole line, raw, as typed
  parts jsonb NOT NULL DEFAULT '[]',             -- [{term, coding: {system,code,display} | null}]
  reason text, hand_ticked boolean NOT NULL DEFAULT false,
  ticked_against jsonb NOT NULL DEFAULT '[]',    -- [{source, ref}] seen when hand-ticked
  finished boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL, saved_at timestamptz NOT NULL);
-- index (patient_id, writer, seq)
cairn_med_draft_line_save(p_line uuid, p_patient uuid, p_terminal text, p_writer text, p_charted_as text,
  p_text text, p_parts jsonb, p_reason text, p_hand_ticked boolean, p_ticked_against jsonb, p_finished boolean)
  RETURNS text   -- 'saved' | 'moved' | 'elsewhere' | 'no-chart' | 'no-such-writer' | 'not-yours'
cairn_med_draft_lines(p_patient uuid, p_writer text)          -- the writer's lines (IS NOT DISTINCT FROM), by seq
  RETURNS TABLE(line_id uuid, terminal_id text, seq int, charted_as text, line_text text, parts jsonb,
                reason text, hand_ticked boolean, ticked_against jsonb, finished boolean, saved_at timestamptz)
cairn_med_draft_line_delete(p_line uuid, p_patient uuid, p_terminal text) RETURNS text  -- 'deleted'|'gone'|'moved'|'elsewhere'
cairn_med_draft_take(p_patient uuid, p_writer text, p_terminal text) RETURNS int        -- lines moved here
cairn_med_drafts_by_writer() RETURNS TABLE(writer text, patient_id uuid, lines bigint, newest timestamptz) -- named writers only
```
Save rules, in order:
1. NULL line, patient or terminal raises.
2. A named writer must be a vault member with the `clinical` purpose (db/066's test), else `no-such-writer`.
3. Take `pg_advisory_xact_lock(hashtextextended('med_draft:' || p_patient || ':' || coalesce(writer,''), 0))`.
4. If the row exists: a different patient answers `elsewhere`, a different terminal `moved`, and a different writer `not-yours`. If `line_text`, `parts` or `charted_as` changed, force `hand_ticked = false` and `ticked_against = '[]'`. Then update and stamp `saved_at`.
5. A new row needs a chart (`event_log` has the patient), else `no-chart`. It takes `seq = max + 1` for (patient, writer).

`seq` is assigned only on insert; reordering is not in this slice. The delete is a plain delete, with no record.

- [ ] **Step 1: Failing DB tests** (`medication_draft_list_store.rs`, taking `db::test_serial_guard`):
  - `a_saved_line_reads_back_byte_for_byte`
  - `an_edit_after_a_hand_tick_clears_it_in_the_door` (Review Focus 1: the save sends `p_hand_ticked = true` with new text and reads back `false`)
  - `a_second_terminal_answers_moved_until_it_takes` (Review Focus 2)
  - `another_writers_line_answers_not_yours`
  - `a_null_writer_list_is_its_own_bucket`
  - `the_window_role_holds_execute_and_no_table_privilege` (`SET ROLE cairn_window`; a direct `SELECT` gets `42501`)
  - `drafts_by_writer_counts_named_writers_only`
- [ ] **Step 2: Run** `cargo test -p cairn-node --test medication_draft_list_store -- --nocapture` → FAIL (no such function). Confirm no `skipped:` line.
- [ ] **Step 3: Implement db/078.** Write it replay-safe, with a header comment written for a junior developer: why the store is mutable, why it never commits itself, why deleting is unaudited.
- [ ] **Step 4: Run** → PASS. Confirm the store is in no backup or export: find how `backup`, `restore` and the local-state export exclude `note_draft` (by naming what they read, or by an exclusion list). If it is a list, add `medication_draft_line`, and add a test beside the note draft's. Also run `twin_registry`, `db_errors_stay_legible` and `hex_decode_helper` DB-free. None should move, since this adds no event type and no hex decode; if one does, register the new item.
- [ ] **Step 5: Commit** `feat(medication): the per-writer medication draft list store (db/078)`.

### Task 2: The tick rules (pure)

**Files:**
- Create: `crates/cairn-node/src/medication/draft_tick.rs`, `crates/cairn-node/src/medication/draft_tick_tests.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs` (`pub mod draft_tick;`)

**Interfaces — Produces:**
```rust
pub struct TickFacts<'a> {
    pub finished: bool,
    pub all_parts_coded: bool,          // ≥ 1 part and every part has a coding
    pub directions: &'a Directions,     // read_directions(the line's directions)
    pub check: &'a LineCheck,
    pub drug_allergies: &'a ListStatus,
    pub duplicate: bool,
    pub named_writer: bool,             // a NULL-writer list is hand-ticked only
}
pub fn default_tick(f: &TickFacts) -> bool;
/// A hand tick still holds when every warning ref the fresh check finds was seen at tick time,
/// and a Start line with a mark carries a non-blank reason. An unverified check holds a hand tick
/// with no reason; with a reason it does not (ADR-0099's CHECK_DID_NOT_RUN).
pub fn hand_tick_holds(charted_as: ChartedAs, fresh: &LineCheck, ticked_against: &[(String, Uuid)], reason: Option<&str>) -> bool;
/// The moiety codes (MOIETY_SYSTEM, single-part lines only) that occur on more than one line of
/// `draft` ∪ `current` (the chart's current medication codings). Uncoded lines and combinations never group.
pub fn duplicate_codes(draft: &[Option<&str>], current: &[&str]) -> BTreeSet<String>;
```

- [ ] **Step 1: Failing tests** (`draft_tick_tests.rs`):
  - `a_complete_clean_finished_line_is_ticked`: `amoxicillin` coded, directions `500 mg cap tds`
  - `unfinished_is_never_ticked`
  - `each_missing_part_withholds_the_tick`: no form; no dose unit; no frequency (but `prn` counts); an uncoded part
  - `not_checked_withholds_the_tick`
  - `a_mark_withholds_the_tick`, and `an_adverse_effect_note_alone_does_not` (the default threshold)
  - `allergies_not_yet_asked_withholds_every_default_tick` (Review Focus 5)
  - `a_duplicate_withholds_the_tick`; `a_list_with_no_named_writer_is_never_default_ticked`
  - `a_hand_tick_holds_against_the_warnings_it_saw`, and `fails_on_a_new_one`
  - `a_hand_tick_with_a_reason_fails_when_the_check_did_not_run`
  - `a_start_hand_tick_on_a_mark_needs_a_reason`
  - `duplicates_count_the_chart_and_the_list`; `combinations_and_uncoded_never_group`
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib draft_tick` → FAIL. Check that the filter matched tests.
- [ ] **Step 3: Implement.** The rules are the ones under Global Constraints. Comment each predicate with the design § it implements.
- [ ] **Step 4: Run** → PASS. **Step 5: Commit** `feat(medication): the list's tick rules (pure)`.

### Task 3: The node: read the list, sign one draft line

**Files:**
- Create: `crates/cairn-node/src/medication/draft.rs`, `crates/cairn-node/tests/medication_draft_line_sign.rs`
- Modify: `crates/cairn-node/src/medication/charting.rs`. Split `chart_medication` into `chart_body(client, node_kid, node_origin, patient, input, charting) -> anyhow::Result<(EventBody, Uuid /*thread*/)>` (HLC, ids, safety class, `build_charted_body`) and the existing submit, so `chart_medication`'s behaviour and callers are unchanged.
- Modify: `crates/cairn-node/src/medication/mod.rs`

**Interfaces — Produces:**
```rust
pub struct DraftLine { pub line_id: Uuid, pub terminal_id: String, pub seq: i32, pub charted_as: String,
  pub line_text: String, pub parts: serde_json::Value, pub reason: Option<String>, pub hand_ticked: bool,
  pub ticked_against: Vec<(String, Uuid)>, pub finished: bool }
pub async fn read_draft_lines(client: &(impl GenericClient + Sync), patient: Uuid, writer: Option<&str>) -> anyhow::Result<Vec<DraftLine>>;
pub async fn writer_drafts(client: &(impl GenericClient + Sync)) -> anyhow::Result<Vec<(String, Uuid, i64, DateTime<Utc>)>>; // cairn_med_drafts_by_writer
/// Chart one draft line: submit its event and delete its draft row in ONE transaction, so the
/// line leaves the list exactly when it enters the record. The delete must answer 'deleted',
/// else the transaction rolls back (the line moved or went: the caller re-reads).
pub async fn chart_draft_line(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str, node_origin: &str,
  patient: Uuid, terminal: &str, line: Uuid, input: &AssertMedicationInput<'_>, charting: &Charting<'_>,
  author: &AuthorParams<'_>) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>>;
```
`chart_draft_line` prepares the sealed body with `prepare_sealed` (through `chart_body`), then opens a transaction for `submit_event` + `cairn_med_draft_line_delete`. The commit's failure is mapped by `db_diagnosis::commit_failure` (#814), exactly as `seal_sign_submit`'s attested arm. **A new commit site:** update the root commit-site source guard's inventory, which names why the site exists.

- [ ] **Step 1: Failing DB tests** (`medication_draft_line_sign.rs`):
  - `a_signed_line_is_in_the_record_and_gone_from_the_list`
  - `a_line_taken_by_another_terminal_rolls_back_unwritten` (no event, the row is still there)
  - `chart_medication_is_unchanged` (an existing `medication_chart.rs` scenario passes unedited)
  - `read_draft_lines_parses_ticked_against`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** this suite plus `medication_chart`, `medication_charting_floor`, `medication_charting_projection` and `cairn-sync`'s `clinical_pull` (cross-crate call sites) → PASS.
- [ ] **Step 5: Commit** `feat(medication): sign one draft line in one transaction with its spend`.

### Task 4: The window commands

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/med_list.rs` (read, save, delete, take), `src/med_list_sign.rs` (Sign), `src/med_list_tests.rs`, `src/med_list_db_tests.rs`
- Modify: `src/med_chart.rs`. Move `parts_before_the_key`, `the_line`, `fresh_check` and the input/charting assembly out of `med_chart_impl` into `pub(crate)` functions that both paths call (`med_chart`'s tests stay green unedited). `src/main.rs` registers the commands.

**Interfaces — Produces** (Tauri commands; `charts` and `credential` as `med_chart`'s):
```rust
pub async fn med_list_read(state, patient_id: String, charts: Vec<String>) -> Result<MedListView, ErrorView>;
pub async fn med_list_save(state, patient_id: String, charts: Vec<String>, line: LineArg) -> Result<SaveAnswer, ErrorView>;  // the door's word
pub async fn med_list_delete(state, patient_id: String, line_id: String) -> Result<SaveAnswer, ErrorView>;
pub async fn med_list_take(state, patient_id: String) -> Result<MedListView, ErrorView>;
pub async fn med_list_sign(state, patient_id: String, charts: Vec<String>, ticked: Vec<String>,
                           credential: Option<Credential>) -> Result<MedListSigned, ErrorView>;
pub struct LineArg { pub line_id: String, pub charted_as: String, pub line_text: String, pub parts: Vec<PartArg>,
                     pub typed_drug: String, pub directions: String, pub reason: Option<String>,
                     pub hand_ticked: bool, pub finished: bool }
pub struct MedListView { pub writer_name: Option<String>, pub elsewhere: Option<String> /* terminal */,
                         pub lines: Vec<ListLineView> }
pub struct ListLineView { pub line_id: String, /* LineArg's fields */ pub check: LineCheckView,
                          pub directions: Directions, pub default_ticked: bool, pub ticked: bool,
                          pub duplicate: bool, pub unfinished: bool }
pub enum MedListSigned { Changed { lines: Vec<String>, message: String },  // nothing signed
                         Done { charted: usize, of: usize, not_charted: Vec<(String, String)>,
                                unknown: Vec<String>, pane: /* med_list_impl's answer */, list: MedListView } }
```
- **`med_list_read`.** The writer is who's here's claim (`Claim::Named` → `Some(staff_id)`). It reads, in one snapshot:
  - the writer's lines;
  - the allergies over the chart set;
  - `build_view(..).drug`;
  - the chart's current medication codings.

  It then checks every line (`check_lines`) and derives `default_ticked` and `duplicate` with Task 2's rules. `ticked = hand_ticked || default_ticked`. With `hand_ticked`, the `against` refs of the moment are what the webview sends back on its next save as `ticked_against` (built in Rust from a fresh check at save time, never taken from the webview).
- **`med_list_sign`.** Its refusal order:
  1. on screen and writable;
  2. **the writer**: a strip credential's staff id, normalised, must equal the list's writer, else `MED_LIST_NOT_YOURS`. A NULL-writer list has no default ticks, so anyone may hand-tick and sign it;
  3. the fresh read above, where every id in `ticked` must be a line in it, else `MED_LIST_STALE`;
  4. **for each ticked line, the tick holds**: a default tick is `default_tick` on the fresh facts; a hand tick is `hand_tick_holds`. Any line that fails → clear its hand tick through the save door, and return `Changed` with those ids and `MED_LIST_CHANGED`. **Nothing is signed.**

  Then the key, once (`signer_for_act`). The released kid must equal `cairn_vault_member_key(writer)` for a named writer, else `MED_LIST_NOT_YOURS`, with nothing written. Then, in list order, each holding line runs slice 1a's per-line checks (press, `parts_before_the_key`, the reason). A failing line goes into `not_charted` with its copy and is skipped. A passing line goes to `chart_draft_line`. A commit with no answer goes into `unknown` and **stops the loop** (Review Focus 4), and the untried lines stay in the list. Then `record_timing("medication-chart", charted, ms)`, and the pane and the list are read again.
- [ ] **Step 1: Failing tests:**
  - pure, in `med_list_tests.rs`:
    - `the_outcome_words_name_each_line_not_charted`
    - `a_credential_for_another_staff_is_not_yours`
  - DB-gated, in `med_list_db_tests.rs`:
    - `one_sign_charts_every_ticked_line_with_one_key_release` (the vault is asked once)
    - `an_allergy_entered_after_the_tick_refuses_and_unticks_only_that_line`
    - `the_second_press_signs_the_rest`
    - `a_blank_named_line_is_skipped_and_the_rest_sign`
    - `a_held_key_of_another_member_writes_nothing` (Review Focus 3)
    - `a_lost_commit_stops_and_keeps_the_rest` (Review Focus 4; use the existing commit-failure seam, the `SET ROLE` fault-injection pattern)
    - `not_yet_asked_hand_ticks_sign` (Review Focus 5)
    - `a_who_is_here_switch_hides_the_list`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_chart_tests`, `med_chart_db_tests` and `med_chart_rows_tests` unedited.
- [ ] **Step 5: Run every ROOT source guard DB-free** (commit sites, `enroll_device_actor`, the derived unwrap key, the `hex_decode_helper` and `db_errors_stay_legible` per-file counts) and register what the new files move.
- [ ] **Step 6: Commit** `feat(window): the medication list's commands and one Sign`.

### Task 5: The person menu counts unsigned medication lines

**Files:**
- Modify: `crates/cairn-node/src/note/menu.rs` (`Why::MedicationLines { lines: usize }`, in the `own` group, counted), `crates/cairn-node/src/note/menu_read.rs` (`MenuRead.med_drafts: Vec<(String, Uuid, i64, DateTime<Utc>)>` from `writer_drafts`, narrowed to the named staff as `drafts` is), `crates/cairn-node/src/note/menu_tests.rs`, `cairn-gui/cairn-gui-tauri/src/person_menu_view.rs` (the copy `"{n} unsigned medication line(s)"`; "Open" opens the chart on its medication pane), `src/person_menu_tests.rs`, `src/person_menu_db_tests.rs`
- **Interfaces — Produces:** `Why::MedicationLines { lines }`, with `MenuItem.part = None` and `when` = the door's `newest`.
- [ ] **Step 1: Failing tests:**
  - `a_writers_unsigned_lines_count_once_per_chart`
  - `another_writers_lines_are_not_mine`
  - `the_picker_counts_include_medication_lines`
  - the view's copy test
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS. **The #1021 rig need not re-run**: the door reads one small table, grouped. Say so in the ADR, not as a measured figure.
- [ ] **Step 5: Commit** `feat(window): "Waiting for you" counts unsigned medication lines`.

### Task 6: The list pane

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/med_list.js`, `src/med_list_webview_tests.rs`
- Modify: `src-ui/med_chart.js`. Expose the line editor as `MedLineEditor(container, line, callbacks)` without its own Sign button, so the list mounts one per line. `src-ui/index.html` (script tag after `med_chart.js`), `src-ui/style.css`, `src-ui/main.js` (only the mount; keep it under 500 lines).

The behaviour:
- *Start medication* / *Already taking* append a line to the list instead of opening a lone line.
- Each line has a tick box and a remove control. A line with no default tick and no hand tick reads visibly unsigned. A line still being typed shows `unfinished`.
- A line autosaves one second after its last change (`med_list_save`). `finished` is sent true on Enter or when focus leaves the line.
- Ticking by hand sends `hand_ticked = true`. On a *Start* line with a mark, the box stays disabled until the reason field holds non-blank text.
- One Sign under the list uses the signer strip (`signer.js`) and sends the ids of the lines drawn ticked. `Changed` redraws those lines unticked, with their warnings and the message. `Done` draws `Charted {k} of {n}` and each line not charted, with its reason.
- "On another terminal" shows Continue here (`med_list_take`).
- Opening the chart, and the who's-here bar changing, both re-read the list (`med_list_read`). That is the re-check on return.
- [ ] **Step 1: Failing webview tests** (the existing rig). Assert visibility (the computed display and hidden ancestors), never only the text:
  - `two_lines_then_one_sign`
  - `an_unfinished_line_is_never_drawn_ticked`
  - `a_start_mark_keeps_the_box_disabled_until_a_reason`
  - `changed_lines_come_back_unticked_with_their_warning`
  - `a_partial_outcome_names_each_line_not_charted`
  - `another_terminal_offers_continue_here`
  - `removing_a_line_asks_nothing` (no dialog)
- [ ] **Step 2: Run** → FAIL. Check that the filter matched tests. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_chart_webview_tests` and `med_chart_line_webview_tests`. Update only the tests that pinned the lone line's own Sign button, and say which in the commit.
- [ ] **Step 5: Commit** `feat(window): the medication list pane — ticks, one Sign`.

### Task 7: Docs — ADR, spec, durable rules, RUNBOOK

**Files:**
- Create: `docs/spec/decisions/0100-charting-a-list-under-one-signature.md`. It records:
  - the list is a durable per-writer draft, never committed by a timer, and deleting from it is unaudited;
  - the default-tick bar, including "allergies not yet asked";
  - untick on a later warning;
  - one Sign with per-line transactions under ADR-0060;
  - the writer-by-key rule;
  - what is not guaranteed: no versions or set-aside yet (slice 2); the threshold is fixed at allergy marks (slice 3); formulation must be typed, because the pick's display is not read as a form; drafts do not cross nodes (#963).

  **Fact-check it sentence by sentence against the SQL and Rust before the PR.**
- Modify:
  - `docs/spec/index.md` (v0.102, ADR-0100), `docs/spec/data-model.md` §3.3 (one paragraph), `docs/spec/decisions/README.md`, `mkdocs.yml` (the nav line);
  - `docs/developers/durable-rules.md` (a slice block: "the default tick is derived, never stored"; "a text edit clears a hand tick in the door"; "a line's spend is in its own event's transaction"; "never a shared transaction across lines");
  - `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §31 + `TEMPLATE.md` (time a 10-line list as one Sign vs ten; an interrupted list's resume);
  - `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- File an issue for "the pick's form is not read as the formulation", since it withholds most default ticks today.
- [ ] **Step 1:** Write the docs. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` is clean. **Step 3:** Run `python3 scripts/check_closing_keywords.py` on the commit message. **Step 4:** Commit `docs(medication): ADR-0100, spec v0.102, RUNBOOK §31, durable rules`.

### Task 8: The whole gate

- [ ] Run every CI gate, in CI's order, **after the last edit**:
  - fmt;
  - clippy (root, and the GUI tree with `--locked`);
  - `cargo doc` with `RUSTDOCFLAGS=-D warnings` (both trees);
  - `scripts/run-db-gated-tests.sh` (detached with `nohup`, in a worktree at the PR head; about 2 h);
  - the GUI tree's DB-gated `--bins`;
  - `cargo deny`;
  - the closing-keyword script on the PR body.

  Sum the passed counts and confirm there is no `skipped:` line.
- [ ] Push, mark PR #1078 ready (or open the slice PR against `main`), and link #1072. It closes only after slice 2, so write `Refs #1072`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the admission medication history: lines written under each other, one signature at the bottom.
- **Steps:**
  - A 12-line home list: paper N = 12 lines + 1 signature → architecture M = 1 human signature act (12 events) → UI K = 12 lines + 1 Sign. One more Sign only when a warning arrived meanwhile; paper has no counterpart, because paper never re-checks.
  - Resuming after being called away: paper N = 0 (the sheet lies there) → M = 0 → K = 0 (the list comes back on opening the chart).
  - Removing a line: N = 1 (cross it out) → K = 1 press.
- **Time + cognitive load:** ≤ 10 s per common line, as slice 1a; ≤ 3 s from Sign to the outcome for a 10-line list on the M3 Max. Each line's drug, dose, route and warning stay readable without a click. Measured by RUNBOOK §31 (human), owed by this slice.
