# Batch charting slice 2a — "From:", the "why", and the set-aside version — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician sets "From:" once at the top of the medication list, may add a one-line "why" to any line, and presses "Set aside" on a version of a drug they do not believe — and one Sign charts the believed lines with their sources and signs each set-aside version into the record as *"{source} reported … Set aside, not current"*, shown collapsed under its line next morning.

**Architecture:**
- **Two optional free-text keys** (`source_detail`, `why`) on `clinical.medication.asserted`, built by a small new
  `cairn-event` module, floored by one new db/079 function called from db/031, projected into `medication_charting`.
- **One new event type** `clinical.medication-set-aside.asserted` (db/079: class row, floor, twin check, projection,
  apply; shred hook in db/037; thread UNION in db/048). Its drug fields are built by the assertion's own builder, so
  both spell them identically.
- **The draft list (db/078, in place)** gains per-line source fields, a set-aside mark naming its companion, and a
  per-(writer, chart) "From:" row. A version signs **in its companion's transaction**, or alone when its companion is
  already charted.
- **The window** gets the "From:" control, the "why" field, "Set aside", and a read-only collapsed version under a
  charted line. Restore and "checked: not current" are **slice 2b** (its own plan, after this merges).

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri, cairn-gui-tab-medications), PostgreSQL ≥ 18 (plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md`](../specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md)
(builds §3.1, §3.2, §3.5 and §3.3/§3.4's display of an unsettled version), extending
[`2026-11-11-batch-charting-1072-design.md`](../specs/2026-11-11-batch-charting-1072-design.md) §3.4.
**Slice 2b, not here:** `restores`, "Use this version instead", `clinical.medication-set-aside-checked.asserted`,
"checked" / "replaced" / "restored" display. **Slice 3, not here:** the threshold.

## Global Constraints

- AGPL-3.0. No new dependency. **Wire is forever (principle 11):** additions only — two keys on
  `clinical.medication.asserted`, one new type. No existing key changes meaning; an assertion without the new keys
  serialises byte-identically to today (pinned).
- **Every batch-charting-1 and charting-1a/1b durable rule holds** (`docs/developers/durable-rules.md`): the default
  tick derived never stored; a content change clears a hand tick IN THE DOOR; a line's event and its spend in one
  transaction, never one across unrelated lines; whole-Sign refusals before the key for a changed picture only; the
  webview's answer never trusted.
- **A set-aside version is never current.** Its payload carries no `charted_as`, no `warning_override`; its event
  carries no §5.9 safety claim; it is never allergy-checked, never in `medication_statement` or any active-list read,
  never ticked.
- **A version exists only beside a companion** (maintainer): a current line of the same coded moiety (single part,
  `MOIETY_SYSTEM`), on the list or charted, on the **opened** chart. Uncoded lines and combinations get no "Set aside".
- **"From:" presets (maintainer; exact labels and categories):** `Patient` → `patient-reported`; `Carer / family` →
  `patient-reported`; `GP letter` → `external-record`; `Pharmacy` → `external-record`; `Packets / Webster pack seen`
  → `clinician-observed`; `Other` → `unknown`. The category is never guessed from the free text. With "From:" unset,
  a line's `info_source` is the press's default (`charted_input`: Start → `clinician-observed`, Already taking →
  `patient-reported`). `source_detail` = the free text, or the preset label when the free text is blank (never for
  an unset "From:").
- **Copy (exact; pinned by tests):**
  - `FROM_LABEL = "From:"`; `WHY_LABEL = "Why"`; `SET_ASIDE = "Set aside"`; `SET_ASIDE_MARK = "set aside — not current"`
  - `WHICH_LINE = "Version of which line?"` (shown only with two or more candidates)
  - set-aside twin: `"{source} reported {drug line}. Set aside, not current"` + `" — {why}"` when present, where
    `{source}` = `source_detail` if present, else the `info_source` word
  - Sign outcome: slice 1's `"Charted {k} of {n}"`, then `"; {v} version(s) set aside"` when v > 0, and per version
    not signed `"{drug line} — version kept: its line was not charted"`
  - collapsed under a charted line: summary `"{n} other version(s)"`; each `"{source}: {drug line} — set aside, not current"` + `" — {why}"`
- No confirmation dialog. **Files stay under 500 lines:** `med_list.js` is at 498, `cairn-event/.../charting.rs` at
  514, `med_list_sign.rs` 421, `med_list.rs` 405 — new code goes in new files.
- `SCHEMA_GENERATION` 78 → **79**; new file **`db/079_medication_versions.sql`**. DB-gated suites printing
  `skipped:` are not a pass; `CAIRN_ALLOW_DB_SKIP=1` only for a deliberate DB-free run.

## Review Focus

1. **A companion refused at Sign** (blank part, a reason missing, a lost commit): its versions are **not** signed and
   stay in the list with "version kept: its line was not charted" — never a version in the record pointing at
   nothing. Pinned in Task 6 (`a_refused_companion_keeps_its_versions_unsigned`).
2. **A companion edited to another drug after "Set aside"** (metoprolol → atenolol): at Sign the version no longer has
   a same-moiety companion and is refused per line, kept in the list. Pinned in Task 4 (pure) and Task 6.
3. **A companion removed from the list:** the delete door clears `set_aside_of` on every version naming it (they
   become ordinary held lines), so no dangling mark survives. Pinned in Task 4.
4. **An already-charted companion stopped between the read and the Sign:** the authoring floor refuses the version
   (companion not current); it stays in the list, reported per line. Pinned in Task 3 and Task 5.
5. **An older or custody-less reader:** a set-aside never appears in `medication_statement`, the active list, the
   allergy check, or the §5.9 safety projection. Pinned in Task 3 (`a_set_aside_is_never_a_current_medication`).

---

### Task 1: The wire builders (cairn-event)

**Files:**
- Create: `crates/cairn-event/src/medication/source_note.rs`, `crates/cairn-event/src/medication/set_aside.rs`
- Modify: `crates/cairn-event/src/medication/assert.rs` (field `source_note`; extract the drug-line part of
  `render_medication_twin`, line 143, into `pub fn drug_line_twin(a: &MedicationAssertion) -> String` with no output
  change), `crates/cairn-event/src/medication/mod.rs` (exports), every `MedicationAssertion { … }` literal in the
  workspace (`source_note: None`) — **including `cairn-sync/tests/clinical_pull.rs`**: run the full-workspace build.

**Interfaces — Produces:**
```rust
// source_note.rs
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceNote<'a> { pub source_detail: Option<&'a str>, pub why: Option<&'a str> }
pub fn insert_source_note(payload: &mut serde_json::Map<String, Value>, n: &SourceNote); // keys only when Some
pub fn source_note_twin(n: &SourceNote) -> String;  // "" when both None; else " — from {detail}" / " — why: {why}"
// assert.rs: MedicationAssertion gains `pub source_note: Option<SourceNote<'a>>`
// set_aside.rs
pub const SET_ASIDE_EVENT_TYPE: &str = "clinical.medication-set-aside.asserted";
pub const SET_ASIDE_SCHEMA: &str = "clinical.medication-set-aside/1";
pub struct MedicationSetAside<'a> {
    pub version_id: &'a str, pub alternative_to: &'a str,
    /// The drug fields, spelled exactly as a charted line. `safety` and `charting.charted_as` /
    /// `warning_override` must be None (the builder debug_asserts and strips them).
    pub drug: &'a MedicationAssertion<'a>,
}
pub fn set_aside_body(s: &MedicationSetAside) -> Value;   // medication_assertion_body(drug) − medication_id
                                                         // + version_id + alternative_to (+ drug's source_note)
pub fn render_set_aside_twin(s: &MedicationSetAside) -> String;
```

- [ ] **Step 1: Failing tests** (in each module's `#[cfg(test)]`):
  - `an_assertion_without_a_source_note_is_byte_identical` (serialise today's fixture before/after)
  - `source_note_keys_are_exactly_source_detail_and_why`, `a_none_key_is_absent_not_null`
  - `the_assertion_twin_names_the_source_and_the_why`
  - `a_set_aside_body_has_the_assertions_drug_fields_and_no_medication_id`
  - `a_set_aside_body_never_carries_charted_as_or_warning_override` (fixture passes both; body has neither)
  - `the_set_aside_twin_reads_source_reported_set_aside_not_current` (`"GP letter reported {drug_line_twin(fixture)}. Set aside, not current — packet 25 mg, daughter confirms"`, the drug line exactly as `drug_line_twin` renders the fixture), and `falls_back_to_the_info_source_word`
- [ ] **Step 2: Run** `cargo test -p cairn-event medication::` → FAIL.
- [ ] **Step 3: Implement.** Junior-legible module headers: why two keys never one (spec §2 anti-goal 3), why a
  version reuses the assertion builder (one spelling of the drug fields).
- [ ] **Step 4: Run** → PASS; `cargo build --workspace --all-targets` (cross-crate literals).
- [ ] **Step 5: Commit** `feat(medication): source_detail, why, and the set-aside builder (wire)`.

### Task 2: The floor and projection for "From:" and the "why"

**Files:**
- Create: `db/079_medication_versions.sql` (this task writes its header and §1), `crates/cairn-node/tests/medication_source_note_floor.rs`
- Modify: `db/031_medication.sql` (one `PERFORM cairn_check_medication_source_note(b);` in
  `cairn_check_medication_assertion`, after the charting call at line 69 — in place), `db/077_medication_charting.sql`
  (in place: `ALTER TABLE medication_charting ADD COLUMN IF NOT EXISTS source_detail text, … why text`;
  `medication_charting_apply` reads both and its "nothing present → RETURN" test (lines 257-261) counts them),
  `crates/cairn-node/tests/migration_replay_widening.rs` (`WIDENED`: the two columns, **and db/078's `typed_drug`,
  `directions`, `held`, which are missing today**), `crates/cairn-node/src/db.rs` (register `079_medication_versions`),
  `crates/cairn-event/src/schema_generation.rs` (79), and the node's charting read (`read_charting`) to return both.

**Interfaces — Produces:** `cairn_check_medication_source_note(b jsonb) RETURNS void` — at both doors: each key, when
present, is a JSON string; after the `cairn.remote_apply` early return (the db/075 idiom): non-blank after `btrim`,
and `!~ '[[:cntrl:]]'`. `ReadCharting` gains `source_detail: Option<String>`, `why: Option<String>`.

- [ ] **Step 1: Failing DB tests** (`medication_source_note_floor.rs`, `db::test_serial_guard`):
  `a_charted_line_with_source_and_why_projects_both`, `a_non_string_key_is_refused_at_both_doors`,
  `a_blank_or_control_char_is_refused_at_the_authoring_door_only`, `a_shred_removes_them_with_the_charting_row`,
  `a_reprojection_rebuilds_them`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --test medication_source_note_floor -- --nocapture` → FAIL (no `skipped:`).
- [ ] **Step 3: Implement.** **Step 4: Run** → PASS, plus `medication_charting_floor`, `medication_charting_projection`,
  `migration_replay_widening`, and the generation guard.
- [ ] **Step 5: Commit** `feat(medication): the floor and projection for source_detail and why (db/079)`.

### Task 3: The set-aside event type (db/079 §2)

**Files:**
- Modify: `db/079_medication_versions.sql` (§2), `db/037_born_sealed.sql` (`cairn_execute_shred`: a
  `to_regclass`-guarded `DELETE FROM medication_set_aside WHERE content_address = v_ca`, as lines 376-381; its comment
  at 134-147), `db/048_*.sql` (`cairn_event_thread` UNION at 579-587: the version's thread is `alternative_to`, so its
  sensitivity scope is the companion's, never the whole chart), the four pinned counts — `twin_registry.rs` (33 → 34
  and its verbatim list) + `db/tests/034_twin_registry_test.sql`; `projection_registry.rs` (38 → 39) +
  `db/tests/039_projection_registry_test.sql` (40 → 41) — and the test wipe lists (`tests/common/restore_kit.rs`,
  `tests/common/medication_kit.rs`).
- Create: `crates/cairn-node/tests/medication_set_aside_floor.rs`, `db/tests/0NN_medication_set_aside_test.sql`
  (next free number, `\ir _scratch_database_guard.sql`).

**Interfaces — Produces:**
```sql
-- event_type_class row for clinical.medication-set-aside.asserted (as db/075:26-29)
cairn_check_medication_set_aside(p_type text, b jsonb) RETURNS void
-- both doors: version_id + alternative_to uuid-shaped; substance.term non-empty string; info_source a string;
--   charted_as / warning_override ABSENT (no writer ever produces them, so refusing at the remote door wedges nothing);
--   PERFORM cairn_check_medication_coding(b) and cairn_check_medication_source_note(b).
-- authoring door only (after the remote_apply return): alternative_to is a medication thread whose
--   cairn_medication_thread_patient(...) is this event's chart (the db/075:45,121 idiom) AND it is current
--   (in medication_statement, no medication_cessation row); version_id not already in medication_set_aside.
CREATE TABLE IF NOT EXISTS medication_set_aside (
  content_address text PRIMARY KEY, event_id uuid NOT NULL, patient_id uuid NOT NULL,
  version_id uuid NOT NULL, alternative_to uuid NOT NULL,
  term text NOT NULL, coding_system text, coding_code text, coding_display text,
  formulation text, dose_amount text, dose_unit text, sig text, route text, strength text,
  info_source text NOT NULL, source_detail text, why text, t_recorded timestamptz NOT NULL);
-- indexes (patient_id), (alternative_to); apply fn medication_set_aside_apply reads cairn_clear_payload(e),
-- skips (never raises) on a bad shape, and calls cairn_guard_medication_patient(alternative_to, patient, ca) so a
-- companion on another chart is FLAGGED on remote apply, as a cessation's is (spec §3.2); cairn_projection_apply row (run_order 20, heal_safe TRUE).
```
The remote door admits an orphan (companion absent) and a companion on another chart; readers join on
`alternative_to`, so an orphan is shown nowhere (spec §3.2). `alternative_to` is **not** added to
`cairn_medication_thread_patient`. **The §5.9 claim:** find where an event's `safety` envelope column is admitted for a
type (db/049's per-type rules); refuse a non-null claim for this type there at both doors. If no per-type rule exists,
the builder carrying none is the guarantee, and the test below pins `safety IS NULL`.

- [ ] **Step 1: Failing DB tests** (`medication_set_aside_floor.rs`):
  - `a_set_aside_beside_a_current_line_projects_its_fields`
  - `a_set_aside_is_never_a_current_medication` (Review Focus 5: not in `medication_statement`, the active read, the
    allergy check's line set, or the safety projection; `safety IS NULL`)
  - `charted_as_or_warning_override_is_refused_at_both_doors`
  - `a_companion_on_another_chart_or_unknown_is_refused_at_the_authoring_door`
  - `a_stopped_companion_is_refused_at_the_authoring_door` (Review Focus 4)
  - `the_remote_door_admits_an_orphan_and_no_reader_shows_it`
  - `a_shred_removes_the_row`, `a_reprojection_rebuilds_it`
  - `a_companion_in_the_same_transaction_satisfies_the_floor`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `twin_registry` and
  `projection_registry` DB-free and the db/tests SQL suite via `scripts/run-db-gated-tests.sh`'s SQL step (pass
  `PGHOST=127.0.0.1 PGPORT=5532`, #862).
- [ ] **Step 5: Commit** `feat(medication): the set-aside version as its own event type (db/079)`.

### Task 4: The draft list carries sources and the set-aside mark (db/078 in place) + pure rules

**Files:**
- Modify: `db/078_medication_draft_list.sql` (in place), `crates/cairn-node/src/medication/draft_tick.rs` (or a new
  `draft_versions.rs` if it would pass 500), its tests, `crates/cairn-node/src/medication/draft.rs`
  (`DraftLine` fields; `read_draft_from`), `crates/cairn-node/tests/medication_draft_list_store.rs`,
  `migration_replay_widening.rs` (`WIDENED`).

**Interfaces — Produces:**
```sql
-- medication_draft_line ADD COLUMN IF NOT EXISTS: info_source text, source_detail text, why text,
--   set_aside_of text   -- a draft line_id ('line:<uuid>') or a charted medication_id ('med:<uuid>'); NULL = not set aside
-- CHECK info_source IS NULL OR info_source IN ('patient-reported','clinician-observed','external-record','unknown')
CREATE TABLE IF NOT EXISTS medication_draft_from (patient_id uuid, writer text, preset text, detail text,
  saved_at timestamptz NOT NULL, UNIQUE NULLS NOT DISTINCT (patient_id, writer));
cairn_med_draft_line_save(…, p_info_source text DEFAULT NULL, p_source_detail text DEFAULT NULL,
  p_why text DEFAULT NULL, p_set_aside_of text DEFAULT NULL)  -- drop the old signature (guarded, as lines 137-140)
cairn_med_draft_lines(…) RETURNS TABLE(…, info_source, source_detail, why, set_aside_of)  -- guarded DROP (218-223)
cairn_med_draft_from_save(p_patient uuid, p_writer text, p_preset text, p_detail text) RETURNS text  -- 'saved'
cairn_med_draft_from(p_patient uuid, p_writer text) RETURNS TABLE(preset text, detail text)
```
Door rules added to the save: a change of `info_source`, `source_detail`, `why` or `set_aside_of` is a **content
change** (clears `hand_ticked`, `ticked_against`, `held`, as `v_changed` at 175-179); a non-NULL `set_aside_of`
forces `hand_ticked = false`. The delete door, after deleting line X, sets `set_aside_of = NULL` (and `held = true`)
on every line of the same (patient, writer) whose `set_aside_of = 'line:' || X` (Review Focus 3). Both new doors:
definer, `REVOKE … FROM PUBLIC`, `GRANT EXECUTE … TO cairn_window`; `medication_draft_from` in no backup/export,
exactly as `medication_draft_line` (find its exclusion and add the table beside it).

```rust
// pure (draft_tick.rs or draft_versions.rs)
pub enum Companion { Draft(Uuid), Charted(Uuid) }
/// Lines (draft or current charted) of the same single-part MOIETY_SYSTEM code as `line`, excluding itself
/// and any line that is itself set aside. Empty → no "Set aside" button.
pub fn companion_candidates(line: &DraftLine, draft: &[DraftLine], current: &[(Uuid, &str)]) -> Vec<Companion>;
/// At Sign: Ok(companion) while `set_aside_of` still names a candidate; Err(reason) otherwise.
pub fn version_holds(line: &DraftLine, draft: &[DraftLine], current: &[(Uuid, &str)]) -> Result<Companion, &'static str>;
pub fn info_source_of(preset: Option<&str>) -> Option<&'static str>; // the Global Constraints table; None for unset/unknown preset
```
A set-aside line is excluded from `default_tick`, `duplicate_codes`'s grouping of ticked lines, and every tick.

- [ ] **Step 1: Failing tests:**
  - DB (`medication_draft_list_store.rs`): `sources_and_why_read_back_byte_for_byte`,
    `a_source_or_why_edit_clears_a_hand_tick_in_the_door`, `a_set_aside_line_is_never_hand_ticked`,
    `deleting_a_companion_clears_its_versions_marks`, `the_from_row_survives_and_is_per_writer`,
    `the_window_role_reaches_the_from_row_only_through_its_doors`
  - pure: `uncoded_and_combinations_have_no_candidates`, `two_oxycodone_lines_are_two_candidates`,
    `an_edited_companion_of_another_drug_no_longer_holds` (Review Focus 2), `a_charted_current_line_is_a_candidate`,
    `a_set_aside_line_is_never_default_ticked`, `each_preset_maps_to_its_category`, `other_is_unknown`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** (header comments: why provenance edits clear a tick — the signature
  vouches for the source too). **Step 4: Run** → PASS, plus `medication_draft_list_ticks`, `draft_tick` lib tests.
- [ ] **Step 5: Commit** `feat(medication): the list keeps each line's source, why and set-aside mark (db/078)`.

### Task 5: The node signs a version with its companion

**Files:**
- Modify: `crates/cairn-node/src/medication/draft.rs` (or new `draft_versions_sign.rs` past 500 lines),
  `crates/cairn-node/src/medication/mod.rs`; Create: `crates/cairn-node/tests/medication_set_aside_sign.rs`,
  `crates/cairn-node/src/medication/set_aside_read.rs`.
- Modify: the root commit-site source guard's inventory if a new transaction site is added.

**Interfaces — Produces:**
```rust
pub struct VersionToSign<'a> { pub line: Uuid, pub saved_at: &'a str, pub drug: MedicationAssertion<'a> }
/// chart_draft_line gains `versions: &[VersionToSign]`: after the companion's event, each version is prepared
/// (prepare_sealed, alternative_to = the companion's new medication_id), submitted, and its draft row spent
/// (delete door with its saved_at) — all in the companion's ONE transaction; any spend not 'deleted' rolls back all.
/// Committed carries (companion thread, version event ids).
pub async fn chart_draft_line(/* existing params */, versions: &[VersionToSign<'_>]) -> anyhow::Result<Committed<(Uuid, Vec<Uuid>)>>;
/// A version whose companion is already charted: event + spend in its own transaction.
pub async fn set_aside_draft_line(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str,
  node_origin: &str, patient: Uuid, terminal: &str, writer: Option<&str>, v: &VersionToSign<'_>,
  alternative_to: Uuid, author: &AuthorParams<'_>) -> anyhow::Result<Committed<Uuid>>;
pub struct SetAsideRow { pub version_id: Uuid, pub alternative_to: Uuid, pub drug_line: String,
  pub source: String /* source_detail or the info_source word */, pub why: Option<String>, pub t_recorded: DateTime<Utc> }
/// The set-asides over a chart set, keyed for joining under each row's medication_id. One query, by patient ids.
pub async fn read_set_asides(client: &(impl GenericClient + Sync), charts: &[Uuid]) -> anyhow::Result<Vec<SetAsideRow>>;
```
Each version is signed with the same author params (the one key release). `chart_draft_line`'s existing callers
pass `&[]` and read the thread from `.0` of the new tuple; nothing else about them changes.

- [ ] **Step 1: Failing DB tests** (`medication_set_aside_sign.rs`):
  `a_line_and_its_version_commit_together_and_leave_the_list`,
  `a_version_spend_that_fails_rolls_back_the_companion_too`,
  `a_version_beside_a_charted_line_signs_alone`,
  `a_charted_companion_stopped_since_refuses_and_keeps_the_version` (Review Focus 4),
  `read_set_asides_joins_only_present_companions`,
  `chart_draft_line_with_no_versions_is_unchanged` (the existing `medication_draft_line_sign` suite passes unedited).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `medication_draft_line_sign`,
  `medication_chart`, and `cairn-sync`'s `clinical_pull` (`--test-threads=2`).
- [ ] **Step 5: Commit** `feat(medication): a set-aside version signs in its companion's transaction`.

### Task 6: The window commands

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/med_list_versions.rs` (the "From:" commands, the version grouping for Sign,
  the outcome words), `src/med_list_versions_tests.rs`, `src/med_list_versions_db_tests.rs`
- Modify: `src/med_list.rs` (`LineArg` + `ListLineView` gain `info_source`, `source_detail`, `why`, `set_aside_of`,
  `companions: Vec<String>`; `MedListView` gains `from: Option<FromView>`), `src/med_list_sign.rs` (call into
  `med_list_versions` — keep it under 500), `src/med_chart_write.rs` (`charted_input` takes
  `info_source: Option<&str>` and `source_note: Option<SourceNote>`; `None` keeps today's default), `src/main.rs`.

**Interfaces — Produces:**
```rust
pub async fn med_list_from_save(state, patient_id: String, preset: Option<String>, detail: Option<String>) -> Result<MedListView, ErrorView>;
pub struct FromView { pub preset: Option<String>, pub detail: Option<String> }
/// Sign: ticked companions sign with the versions naming them; versions whose companion is charted sign alone;
/// versions whose companion is unticked, refused, or no longer holds stay, worded per Global Constraints.
pub(crate) fn group_versions(lines: &[DraftLine], ticked: &BTreeSet<Uuid>, current: &[(Uuid, &str)])
  -> (BTreeMap<Uuid, Vec<Uuid>> /* companion → versions */, Vec<(Uuid, Uuid)> /* alone: version, charted */, Vec<(Uuid, &'static str)> /* kept */);
pub(crate) fn outcome_words(charted: usize, of: usize, versions: usize, kept: &[(String, &str)]) -> String;
```
`current` is the **opened** chart's current single-coded lines only, never a linked chart's (spec §2). Rust re-derives everything at Sign on the fresh read: `version_holds` per set-aside line; the webview's
`set_aside_of` / `companions` are never trusted. A companion that fails per line (any slice-1/1a refusal, or `unknown`)
takes its versions into "kept"; a lost commit still stops the loop.

- [ ] **Step 1: Failing tests:**
  - pure: `the_outcome_counts_versions_and_names_each_kept`, `a_versions_companion_unticked_keeps_it`,
    `group_versions_sends_a_charted_companions_versions_alone`
  - DB: `one_sign_charts_the_line_and_its_version_together`,
    `a_refused_companion_keeps_its_versions_unsigned` (Review Focus 1),
    `an_edited_companion_refuses_its_version_per_line` (Review Focus 2),
    `from_sets_each_new_lines_source_and_a_line_can_change_its_own`,
    `unset_from_keeps_the_press_default`, `a_why_is_signed_into_the_event`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_list_tests`,
  `med_list_db_tests`, `med_list_held_db_tests`, `med_list_review_db_tests`, `med_chart_tests` unedited.
- [ ] **Step 5: Run every ROOT source guard DB-free** and register what the new files move (commit sites,
  `db_errors_stay_legible`, `hex_decode_helper` per-file lists).
- [ ] **Step 6: Commit** `feat(window): "From:", the why, and signing a set-aside version`.

### Task 7: The list pane and the collapsed version

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/med_list_versions.js` (the "From:" control above the list, the per-line
  "Why" field — closed until opened —, "Set aside" + `WHICH_LINE` pick, the set-aside line drawing), `src/med_list_versions_webview_tests.rs`
- Modify: `src-ui/med_list.js` (mount hooks only — it is at 498 lines; move code out if a hook would pass 500),
  `src-ui/index.html`, `src-ui/style.css`; the charted list: `cairn-gui-tab-medications/src/row_view.rs`
  (`MedListRowView.versions: Vec<VersionView { line: String }>` — the collapsed text, built in Rust from
  `read_set_asides` joined on the row's medication_id, never in JS) and the pane's JS row drawing (a closed
  `<details>` with the summary `"{n} other version(s)"`).

Behaviour: "From:" offers the six presets + a free-text box; a change saves through `med_list_from_save` and applies
to lines saved after it. A line shows its source and can change it. "Set aside" appears only when `companions` is
non-empty; with one candidate it is taken, with more the pick `WHICH_LINE` shows (no dialog). A set-aside line has no
tick box and reads `SET_ASIDE_MARK`; pressing "Set aside" again on it undoes the mark (an ordinary held line).

- [ ] **Step 1: Failing webview tests** (assert visibility — computed display + hidden ancestors — never text only):
  `from_sets_the_next_lines_source`, `the_why_field_is_closed_until_opened`,
  `set_aside_appears_only_with_a_companion`, `two_candidates_ask_which_line`,
  `a_set_aside_line_has_no_tick_and_reads_not_current`,
  `a_charted_line_shows_its_versions_collapsed`, `removing_a_companion_unmarks_its_version`.
  Plus a `row_view_tests.rs` unit test: `versions_join_under_their_companion_only`.
- [ ] **Step 2: Run** → FAIL (check the filter matched tests). **Step 3: Implement.** **Step 4: Run** → PASS, plus
  `med_list_webview_tests`, `delabel_webview_tests`, `med_chart_webview_tests`, `row_view_tests`.
- [ ] **Step 5: Commit** `feat(window): the list's From:, Set aside, and a version shown under its line`.

### Task 8: Docs — ADR-0102, spec, durable rules, RUNBOOK

- Create `docs/spec/decisions/0102-set-aside-versions-and-the-source-of-a-line.md`: the two keys and why never one;
  "From:" presets and their categories (maintainer); the set-aside type, its mandatory companion, never current,
  orphan-tolerant at the remote door; the version in its companion's transaction; a version only beside a companion;
  what is **not** guaranteed yet (no restore, no "checked" — slice 2b; the threshold — slice 3; drafts do not cross
  nodes, #963). **Fact-check it sentence by sentence against the SQL and Rust.**
- Modify `docs/spec/index.md` (v0.104, ADR-0102), `docs/spec/data-model.md` §3.3 (one paragraph — memory: medication
  prose is §3.3), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav), `docs/developers/durable-rules.md` (a slice 2a
  block: "a set-aside version is its own type, never a flag"; "a version signs only in or after its companion";
  "provenance edits clear a tick in the door"; "readers join on `alternative_to`, so an orphan shows nowhere"),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §33 + `TEMPLATE.md` (set a version aside; read it next morning),
  `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- [ ] **Step 1:** Write. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` clean.
  **Step 3:** `python3 scripts/check_closing_keywords.py` on the message. **Step 4:** Commit
  `docs(medication): ADR-0102, spec v0.104, RUNBOOK §33, durable rules`.

### Task 9: The whole gate

- [ ] After the last edit, in CI's order: fmt; clippy (root, GUI tree `--locked`); `cargo doc` with
  `RUSTDOCFLAGS=-D warnings` (both trees); `scripts/run-db-gated-tests.sh` (detached `nohup`, worktree at the PR
  head, ~2 h); the GUI tree's DB-gated `--bins`; `cargo deny`; the closing-keyword script on the PR body. Sum the
  passed counts; no `skipped:` line.
- [ ] Push; mark the PR ready. Write `Refs #1072` (it closes only after slice 2b).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the admission medication history with crossed-out alternatives beside it, each line's
  source written in the margin.
- **Steps:**
  - Setting the source for a 12-line list taken from one document: paper N = 1 margin note ("per GP letter") → M = 0
    forced acts (the source is optional) → K = 1 ("From:" set once).
  - Setting a version aside: paper N = 1 line written + 1 cross-out → M = 1 (the version's line, signed inside the
    list's one Sign) → K = 1 line + 1 press.
  - Reading it next morning: paper N = 0 (it is on the sheet) → K = 1 (open the collapsed versions).
- **Time + cognitive load:** "From:" ≤ 3 s; "Set aside" ≤ 2 s; the list's Sign stays within slice 1's ≤ 3 s for a
  10-line list with 2 versions. A set-aside version never reads as current anywhere on screen. Measured by RUNBOOK §33
  (human), owed by this slice.
