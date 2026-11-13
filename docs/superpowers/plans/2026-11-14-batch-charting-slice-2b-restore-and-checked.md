# Batch charting slice 2b — "Use this version instead" and "Checked: not current" — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The next morning, a reviewing clinician opens a charted line's collapsed set-aside versions and settles each
one in one signed act: **"Use this version instead"** opens an editable list line pre-filled from the version, and
its Sign stops the old line and charts the new one in one transaction; **"Checked: not current"** signs that the
version was checked and is not current. Both record what was checked ("From:" + "why"), and nothing is erased.

**Architecture:**
- **One new optional wire key**, `restores` (the version's `version_id`), on `clinical.medication.asserted`, carried
  by `Charting` (it says how the line came to be charted) and projected into `medication_charting.restores`.
- **The floor enforces the pairing** (spec §3.3): at the authoring door a `restores` line must name a set-aside
  version of the same chart, and a **deferred** check holds that version's companion to be **stopped at commit**
  (the db/065 constraint-trigger idiom). A raw-SQL client cannot leave two current lines. The remote door keeps
  structure only.
- **One new event type**, `clinical.medication-set-aside-checked.asserted` (`db/080` §2: class row, floor, twin
  check, projection, apply; shred hook in db/037; thread union in db/048).
- **The draft list (db/078, in place)** gains `restores`, set once by a new door that creates the pre-filled line;
  the save door never changes it. At Sign the line goes in with the companion's stop, if it is still current.
- **The window**: two buttons on each collapsed version of the opened chart, an inline "Checked" row (its own
  signed act, like the allergy overlays), the restore line's mark in the list, and each version's state words.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri, cairn-gui-tab-medications), PostgreSQL ≥ 18
(plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md`](../specs/2026-11-13-batch-charting-slice-2-versions-wire-design.md)
§3.3 and §3.4 (and §5's benchmark), built on slice 2a ([ADR-0102](../../spec/decisions/0102-set-aside-versions-and-the-source-of-a-line.md),
plan [`2026-11-13-batch-charting-slice-2a-from-and-set-aside.md`](2026-11-13-batch-charting-slice-2a-from-and-set-aside.md)).
**Not here:** slice 3 (the warning threshold as administrator policy); a "Waiting for you" group for unsettled
versions (spec §6 risk 3: countable, no menu group this slice).

## Global Constraints

- AGPL-3.0. No new dependency. **Wire is forever (principle 11):** additions only — one key on
  `clinical.medication.asserted`, one new type. An assertion without `restores` serialises byte-identically to today
  (pinned).
- **Every slice 1 / 1a / 1b / 2a durable rule holds** (`docs/developers/durable-rules.md`, read the batch charting 1
  and 2a blocks first): the default tick derived, never stored; a content change clears a hand tick IN THE DOOR; a
  line's events and its spend in ONE transaction; whole-Sign refusals before the key for a changed picture only; the
  webview's answer never trusted (every restore and companion is re-derived in Rust from STORED rows on the fresh
  read); a set-aside version is never current.
- **A restore is a stop + a new line, never an edit of the version** (ADR-0102 Consequences). The floor does NOT
  compare the new line's drug fields with the version's (the reviewer may edit: "right drug, dose changed since").
- **A restore stops the line that currently stands for the companion** **[ruling R3, 2026-11-14, Task 2 review]**:
  the companion itself if it is current on Sign's fresh read, else every current line that restored a version of
  the same companion (a second restore replaces the first). A companion stopped for another reason with no current
  replacement: no cessation (spec §3.3). The floor holds it at commit: neither the companion nor any OTHER current
  line restoring a version of it may stay current. Spec §3.3's "a restore then emits no cessation" for a companion
  "already replaced by another version's restore" is superseded: it left two current lines. Versions of a stopped
  companion no longer count as unsettled, and both buttons stay offered.
- **The restored line inherits the companion's `charted_as`** (`medication_charting.charted_as`); a companion with
  none (charted before slice 1a) gives `pre-existing` **[maintainer, 2026-11-14]**. The reviewer may change it.
- **A restore line with no source of its own signs `unknown`**, never the press default — the maintainer's ruling for
  a version (ADR-0102 item 4) applied to its sibling: the press says who reported a line being charted, nothing
  about who corroborated a version **[maintainer, 2026-11-14]**. "From:" never stamps a restore
  line (it is created stored, so `stamp_new_line` never sees it as new).
- **Buttons only on the OPENED chart's versions** (spec §2 non-goal: never a linked chart's, never a doubted link's).
  A restored version shows no buttons; a checked one keeps both ("it locks nothing", spec §3.4).
- **Copy (exact; pinned by tests):**
  - buttons: `RESTORE = "Use this version instead"`; `CHECK = "Checked: not current"`
  - the cessation's reason: `"replaced by the {source} version"` (`{source}` = the version's `source_detail`, else its
    `info_source` word — `SetAsideRow::source`)
  - the restored line's twin tail (in `Charting`'s tail): `" · restores a set-aside version"`
  - the checked twin: `"Set-aside version checked: not current — from {source}"` + `" — {why}"` when given
  - the list line's mark: `"restores the {source} version"`
  - under a charted line, each version's line as 2a's, then (as a second line of the same `li`):
    `"checked by {who}, {when}: not current — from {source}"` + `" — {why}"` per check (oldest first), or
    `"restored by {who}, {when}"`
  - under the NEW line: `"replaced: {old drug line} — {who}, {when}"`
  - the summary: `"{n} other version(s)"` + `", {u} to check"` when `u > 0` (`u` = versions with no check and no
    restore, whose companion is current)
  - `{who}` = the vouched signer's name (`cairn_note_signer_name(cairn_note_vouched_attester(event_id))`, db/065),
    else `"signer not known"`; `{when}` = the tab crate's `local_instant` formatting
  - refusals: `VERSION_GONE = "That version is no longer on this chart; read it again"`;
    `RESTORE_VERSION_GONE = "the version it restores can no longer be read"` (a per-line Sign refusal)
- No confirmation dialog. **Files stay under 500 lines:** `med_list_versions.rs` is at **510** (Task 6 moves
  `version_to_sign` + `sign_alone` out, verbatim), `med_list.rs` 493, `med_list_sign.rs` 481, `med_list.js` 494,
  `cairn-event/.../charting.rs` 516 (Task 1 moves its tests out, verbatim), `row_view.rs` 331. New code goes in new
  files.
- `SCHEMA_GENERATION` 79 → **80**; new file **`db/080_medication_version_settle.sql`**; db/031, db/037, db/048,
  db/077, db/078, db/079 edited in place (ADR-0080: the content hash replays them). DB-gated suites printing
  `skipped:` are not a pass; `CAIRN_ALLOW_DB_SKIP=1` only for a deliberate DB-free run.

## Review Focus

1. **A raw-SQL restore that leaves the companion current** (the `restores` line submitted with no cessation): refused
   **at commit**, the whole transaction rolled back — never two current lines from one act. Pinned in Task 2
   (`a_restore_without_the_companions_stop_fails_at_commit`).
2. **The restore line is not a duplicate of the line it replaces** — else it is never default-ticked and the morning
   act costs an extra hand tick — but it still duplicates ANOTHER current line of the same drug. Pinned in Task 4
   (pure) and Task 6.
3. **Two restore lines of the same companion in one Sign** (two versions, both believed): the first stops it, the
   second goes in with no stop; both lines are drawn as duplicates of each other, so neither is default-ticked.
   Pinned in Task 6 (`two_restores_of_one_line_duplicate_each_other`).
4. **The version is gone at Sign** (shredded, or its chart's custody lost since the press): the restore line is
   refused per line with `RESTORE_VERSION_GONE` and stays in the list; nothing is stopped. Pinned in Task 6.
5. **The companion was stopped by someone else between the fresh read and the commit**: the restore's own
   cessation meets an already-stopped line. Expected: the line still goes in and the chart shows one stop (find what
   db/031 does with a second cessation of a thread FIRST; if it refuses, the restore is refused per line and kept —
   either way never a stop without its line). Pinned in Task 5
   (`a_companion_stopped_since_the_read_still_leaves_one_current_line`).

---

### Task 1: The wire builders (cairn-event)

**Files:**
- Create: `crates/cairn-event/src/medication/version_checked.rs`,
  `crates/cairn-event/src/medication/charting_tests.rs` (charting.rs's `#[cfg(test)] mod tests` moved VERBATIM via
  `#[path]`, so charting.rs drops under 500; test count unchanged)
- Modify: `crates/cairn-event/src/medication/charting.rs` (`Charting` gains `restores`; `insert_charting` writes it
  top-level when `Some`; `charting_twin_parts`' tail appends the restores tail),
  `crates/cairn-event/src/medication/set_aside.rs` (`set_aside_body` strips `restores` beside `charted_as` /
  `warning_override`), `crates/cairn-event/src/medication/mod.rs` (exports).

**Interfaces — Produces:**
```rust
// charting.rs — Charting gains (Default = None):
pub restores: Option<&'a str>,   // a set-aside version's version_id; top-level key "restores"
// version_checked.rs
pub const VERSION_CHECKED_EVENT_TYPE: &str = "clinical.medication-set-aside-checked.asserted";
pub const VERSION_CHECKED_SCHEMA: &str = "clinical.medication-set-aside-checked/1";
pub struct VersionChecked<'a> { pub version_id: &'a str, pub info_source: &'a str,
                                pub source_note: Option<SourceNote<'a>> }
pub fn version_checked_body(c: &VersionChecked) -> Value;   // {version_id, info_source} + source note keys
pub fn render_version_checked_twin(c: &VersionChecked) -> String;   // Global Constraints copy
```

- [ ] **Step 1: Failing tests:**
  - charting: `an_assertion_without_restores_is_byte_identical` (today's fixture serialised before/after),
    `restores_is_a_top_level_key_and_named_in_the_twin` (twin ends `" · restores a set-aside version"` before any
    source note), `a_set_aside_body_never_carries_restores`
  - version_checked: `the_checked_body_is_version_id_info_source_and_the_note`, `a_none_note_key_is_absent`,
    `the_checked_twin_names_the_source_and_the_why`, `falls_back_to_the_info_source_word`
- [ ] **Step 2: Run** `cargo test -p cairn-event medication::` → FAIL (and note the test count before the move).
- [ ] **Step 3: Implement.** Module header for `version_checked.rs`: why "checked" is its own type with no verdict
  field (the type IS the verdict; the opposite outcome is the restore, spec §3.4).
- [ ] **Step 4: Run** → PASS, test count = before + new; `cargo build --workspace --all-targets`.
- [ ] **Step 5: Commit** `feat(medication): the restores key and the checked builder (wire)`.

### Task 2: The `restores` floor, projection and the commit-time pairing (db/080 §1)

**Files:**
- Create: `db/080_medication_version_settle.sql` (header + §1), `crates/cairn-node/tests/medication_restore_floor.rs`
- Modify: `db/031_medication.sql` (one `PERFORM cairn_check_medication_restores(b);` beside 2a's source-note call, in
  place), `db/077_medication_charting.sql` (in place: `ALTER TABLE medication_charting ADD COLUMN IF NOT EXISTS
  restores uuid`; `medication_charting_apply` reads it, and its "nothing present → RETURN" test counts it),
  `db/079_medication_versions.sql` (in place: `cairn_check_medication_set_aside` refuses `restores` at BOTH doors,
  with charted_as / warning_override), `crates/cairn-node/tests/migration_replay_widening.rs` (`WIDENED`),
  `crates/cairn-node/src/db.rs` (register `080_medication_version_settle`), `crates/cairn-event/src/schema_generation.rs`
  (80), the node's `read_charting` (returns `restores`).

**Interfaces — Produces:**
```sql
cairn_check_medication_restores(b jsonb) RETURNS void
-- both doors: `restores`, when present, is a JSON string that is a valid uuid, or JSON null (= absent).
-- authoring door only (after the cairn.remote_apply return): it names a medication_set_aside row whose
--   patient_id is this body's chart, else RAISE; then INSERT (event_id, version_id, patient_id) into
--   medication_restore_pending.
CREATE TABLE IF NOT EXISTS medication_restore_pending (event_id uuid PRIMARY KEY, version_id uuid NOT NULL,
  patient_id uuid NOT NULL);   -- empty outside a transaction; REVOKE ALL FROM PUBLIC; in no backup/export
-- CONSTRAINT TRIGGER medication_restore_pairs AFTER INSERT ON medication_restore_pending
--   DEFERRABLE INITIALLY DEFERRED FOR EACH ROW: at commit, every set-aside row with that version_id AND
--   patient_id has its alternative_to in medication_cessation, else RAISE 'a restore must stop the line it
--   replaces in the same transaction'; then DELETE the pending row.
```
The pending table, not a GUC, carries "this was the authoring door": a deferred trigger runs at COMMIT, after the
strict door's `SET cairn.remote_apply = ''` has gone out of scope, so it must not read the setting. **Find first**
whether a reprojection or heal re-runs db/031's check; the pending insert must happen only on a fresh authoring
submit (`ON CONFLICT (event_id) DO NOTHING`), never on a re-fold — `a_reprojection_rebuilds_restores` pins it.

- [ ] **Step 1: Failing DB tests** (`medication_restore_floor.rs`, `db::test_serial_guard`; reuse
  `tests/common/set_aside_kit.rs`):
  - `a_restore_with_its_companions_stop_commits_and_projects_restores`
  - `a_restore_without_the_companions_stop_fails_at_commit` (Review Focus 1: nothing of the transaction lands)
  - `a_restore_of_an_already_stopped_companions_version_needs_no_stop`
  - `a_restore_naming_another_charts_version_or_none_is_refused_at_the_authoring_door`
  - `the_remote_door_admits_an_orphan_restore` (no version here; projected; no pending row)
  - `a_non_uuid_restores_is_refused_at_both_doors`, `a_set_aside_carrying_restores_is_refused_at_both_doors`
  - `the_pending_table_is_empty_after_commit`, `a_reprojection_rebuilds_restores`
- [ ] **Step 2: Run** `cargo test -p cairn-node --test medication_restore_floor -- --nocapture` → FAIL (no `skipped:`).
- [ ] **Step 3: Implement.** Header: why the check is deferred (the stop and the line are two events; either order),
  why a pending table rather than the GUC.
- [ ] **Step 4: Run** → PASS, plus `medication_charting_floor`, `medication_charting_projection`,
  `medication_set_aside_floor`, `migration_replay_widening`, the generation guard.
- [ ] **Step 5: Commit** `feat(medication): a restore must stop the line it replaces, at commit (db/080)`.

### Task 3: The "checked" event type (db/080 §2)

**Files:**
- Modify: `db/080_medication_version_settle.sql` (§2), `db/037_born_sealed.sql` (`cairn_execute_shred`: a
  `to_regclass`-guarded `DELETE FROM medication_set_aside_checked WHERE content_address = v_ca`, beside 2a's), `db/048`
  (`cairn_event_thread`: the checked event's thread is its version's `alternative_to`, joined on `version_id` AND
  `patient_id`, behind its own `to_regclass` probe), the four pinned counts (twin_registry.rs + its verbatim list +
  `db/tests/034_twin_registry_test.sql`; projection_registry.rs + `db/tests/039_projection_registry_test.sql` — each
  **+1 from the figure in the file today**), the test wipe lists (`tests/common/restore_kit.rs`,
  `tests/common/medication_kit.rs`).
- Create: `crates/cairn-node/tests/medication_version_checked_floor.rs`,
  `db/tests/078_medication_version_checked_test.sql` (`\ir _scratch_database_guard.sql`).

**Interfaces — Produces:**
```sql
-- event_type_class row ('clinical.medication-set-aside-checked.asserted', 'additive', FALSE)
cairn_check_medication_version_checked(p_type text, b jsonb) RETURNS void
-- both doors: version_id uuid string; info_source a non-blank string; charted_as / warning_override / restores
--   ABSENT; a non-null `safety` refused (2a's reason); PERFORM cairn_check_medication_source_note(b).
-- authoring door only: the schema tag; version_id names a medication_set_aside row of THIS chart.
CREATE TABLE IF NOT EXISTS medication_set_aside_checked (content_address bytea PRIMARY KEY, event_id uuid NOT NULL,
  patient_id uuid NOT NULL, version_id uuid NOT NULL, info_source text NOT NULL, source_detail text, why text,
  t_recorded timestamptz NOT NULL);   -- index (patient_id), (version_id); GRANT SELECT TO cairn_agent
-- medication_set_aside_checked_apply(e event_log): reads cairn_clear_payload(e), SKIPS a bad shape (never raises),
--   ON CONFLICT DO NOTHING; cairn_projection_apply row (run_order 20, heal_safe TRUE); #214 converge idiom.
```
An orphan check (version not here) is admitted at the remote door and shown nowhere: readers join on `version_id`
AND `patient_id`. A second check of the same version is admitted and shown (spec §3.4).

- [ ] **Step 1: Failing DB tests:** `a_check_of_a_version_on_this_chart_projects`,
  `a_check_of_another_charts_or_unknown_version_is_refused_at_the_authoring_door`,
  `the_remote_door_admits_an_orphan_check`, `two_checks_of_one_version_both_project`,
  `current_line_keys_and_a_safety_claim_are_refused_at_both_doors`, `a_shred_removes_the_row`,
  `a_reprojection_rebuilds_it`, `a_check_is_never_a_current_medication` (not in `medication_statement`, the active
  read, the safety projection; `safety IS NULL`).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `twin_registry` and
  `projection_registry` DB-free, and the db/tests SQL suite (`PGHOST=127.0.0.1 PGPORT=5532`, #862).
- [ ] **Step 5: Commit** `feat(medication): "checked: not current" as its own event type (db/080)`.

### Task 4: The draft list carries `restores` (db/078 in place) + pure rules

**Files:**
- Modify: `db/078_medication_draft_list.sql` (in place), `crates/cairn-node/src/medication/draft.rs` (`DraftLine.restores:
  Option<Uuid>`; the lines read; `add_restore_line`), `crates/cairn-node/src/medication/draft_versions.rs` + its tests,
  `crates/cairn-node/tests/medication_draft_list_store.rs`, `migration_replay_widening.rs` (`WIDENED`).

**Interfaces — Produces:**
```sql
-- medication_draft_line ADD COLUMN IF NOT EXISTS restores uuid;   cairn_med_draft_lines(…) returns it (guarded DROP)
cairn_med_draft_restore_add(p_line uuid, p_patient uuid, p_terminal text, p_writer text, p_version uuid,
  p_charted_as text, p_text text, p_parts jsonb, p_typed_drug text, p_directions text) RETURNS text
-- inserts a NEW row exactly as the save door's new-row branch (same writer resolution, same answers), with
--   restores = p_version and held = false; 'no-such-version' unless medication_set_aside has p_version for
--   p_patient. Definer; REVOKE FROM PUBLIC; GRANT EXECUTE TO cairn_window.
-- The save door: never writes `restores` (no parameter); forces set_aside_of NULL on a row whose restores is set.
```
```rust
// draft.rs
pub async fn add_restore_line(client: &(impl GenericClient + Sync), line: Uuid, patient: Uuid, terminal: &str,
  writer: Option<&str>, version: Uuid, content: &RestoreContent<'_>) -> anyhow::Result<String /* the answer */>;
pub struct RestoreContent<'a> { pub charted_as: &'a str, pub line_text: &'a str, pub typed_drug: &'a str,
  pub directions: &'a str, pub parts: &'a serde_json::Value }
// draft_versions.rs (pure)
/// The companions the list's restore lines would stop: `(restore line, companion)` for each line whose
/// `restores` names a version in `versions` (the opened chart's, `SetAsideRow`).
pub fn replaced_by_restores(draft: &[DraftLine], versions: &[SetAsideRow]) -> Vec<(Uuid, Uuid)>;
```
`companion_candidates` gives a restore line no candidates (no "Set aside" on it).

- [ ] **Step 1: Failing tests:**
  - DB: `a_restore_line_is_added_with_its_version`, `restore_add_refuses_another_charts_version`,
    `a_save_never_clears_or_sets_restores`, `a_restore_line_cannot_be_set_aside`,
    `deleting_a_restore_line_leaves_no_record`
  - pure: `a_restore_line_has_no_set_aside_candidates`, `replaced_by_restores_names_each_lines_companion`,
    `a_restore_naming_an_unknown_version_replaces_nothing`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `medication_draft_list_ticks` and
  `draft_versions` / `draft_tick` lib tests.
- [ ] **Step 5: Commit** `feat(medication): the list keeps a restore line's version (db/078)`.

### Task 5: The node signs a restore with its stop, signs a check, and reads both back

**Files:**
- Modify: `crates/cairn-node/src/medication/draft.rs` (`chart_draft_line` gains `restore` — if that passes 500 lines,
  move `chart_draft_line` into `draft_versions_sign.rs`), `crates/cairn-node/src/medication/set_aside_read.rs`
  (`SetAsideRow.patient_id`), `crates/cairn-node/src/medication/mod.rs`, every `chart_draft_line` caller (pass `None`;
  `cairn-gui-tauri/src/med_list_sign.rs` and two test files today — run the full-workspace build).
- Create: `crates/cairn-node/src/medication/version_settle.rs` (`check_version`),
  `crates/cairn-node/src/medication/version_settle_read.rs`, `crates/cairn-node/tests/medication_version_settle.rs`.

**Interfaces — Produces:**
```rust
/// A restore line's extra at Sign: the version it restores, and — when the companion is current on the fresh
/// read — the companion to stop and the cessation's reason.
pub struct RestoreToSign<'a> { pub version_id: Uuid, pub stop: Vec<(Uuid, &'a str)> }  // ruling R3: 0..n lines
/// chart_draft_line(…, versions, restore: Option<&RestoreToSign<'_>>): when Some, one cessation per `stop` entry
/// (build_cease_body, `reason`, no stop date) is submitted FIRST, then the line with `charting.restores` set from
/// `restore.version_id` (ONE source of truth: a caller's own `charting.restores` must be None), then the spends.
pub struct VersionCheck<'a> { pub version_id: Uuid, pub info_source: &'a str, pub source_note: Option<SourceNote<'a>> }
/// The check's event in its own transaction. The floor refuses a version not on this chart.
pub async fn check_version(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str,
  node_origin: &str, patient: Uuid, c: &VersionCheck<'_>, author: &AuthorParams<'_>)
  -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>>;
// version_settle_read.rs
pub enum Settlement { Checked { source: String, why: Option<String> },
                      Restored { line: Uuid /* the new thread */, replaced_line: String } }
pub struct SettlementRow { pub version_id: Uuid, pub patient_id: Uuid, pub settlement: Settlement,
                           pub who: Option<String>, pub at: DateTime<Utc> }
/// Every check and restore over a chart set, one query each, oldest first; joined on version_id AND patient_id,
/// so an orphan is never returned. `replaced_line` is the companion as `drug_line_twin` spells it, rebuilt from
/// medication_statement + medication_charting (as `read_set_asides` does for a version).
pub async fn read_settlements(client: &(impl GenericClient + Sync), charts: &[Uuid]) -> anyhow::Result<Vec<SettlementRow>>;
pub struct VersionForRestore { pub alternative_to: Uuid, pub companion_current: bool,
  pub companion_charted_as: Option<String>, pub source: String, pub term: String,
  pub coding: Option<(String, String, String)>, pub formulation: Option<String>, pub dose_amount: Option<String>,
  pub dose_unit: Option<String>, pub sig: Option<String>, pub route: Option<String>, pub strength: Option<String> }
/// One version of `patient`, for the restore press and for Sign. None: not on this chart (or gone).
pub async fn version_for_restore(client: &(impl GenericClient + Sync), patient: Uuid, version: Uuid)
  -> anyhow::Result<Option<VersionForRestore>>;
```

- [ ] **Step 1: Failing DB tests** (`medication_version_settle.rs`):
  - `a_restore_stops_its_companion_and_charts_the_line_in_one_transaction`
  - `a_restore_of_a_stopped_companions_version_emits_no_stop`
  - `a_second_restore_stops_the_first_restores_line` (ruling R3)
  - `a_failed_spend_rolls_back_the_stop_too`
  - `a_companion_stopped_since_the_read_still_leaves_one_current_line` (Review Focus 5)
  - `check_version_signs_one_event`, `check_version_refuses_another_charts_version`
  - `read_settlements_names_the_signer_and_the_replaced_line`, `an_unvouched_signer_reads_as_none`
  - `version_for_restore_carries_the_companions_charted_as_and_currency`
  - `chart_draft_line_with_no_restore_is_unchanged` (`medication_draft_line_sign`, `medication_set_aside_sign`
    pass unedited but for the added `None`)
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `medication_draft_line_sign`,
  `medication_set_aside_sign`, `medication_chart`, and `cairn-sync`'s `clinical_pull` (`--test-threads=2`).
- [ ] **Step 5: Commit** `feat(medication): sign a restore with its stop, a check, and read them back`.

### Task 6: The window commands — restore open, Sign, the check

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/med_list_versions_sign.rs` (`version_to_sign` + `sign_alone` moved VERBATIM
  from `med_list_versions.rs`, 510 → under 500; re-export so callers are unedited),
  `src/med_list_restore.rs` (the restore command, the prefill, `restore_to_sign`), `src/med_version_check.rs` (the
  check command), `src/med_list_restore_tests.rs`, `src/med_list_restore_db_tests.rs`,
  `src/med_version_check_db_tests.rs`
- Modify: `src/med_list.rs` (`ListLineView.restores_words: Option<String>`), `src/med_list_judge.rs` (the fresh read
  carries the opened chart's versions; duplicates exclude each restore line's own companion), `src/med_list_sign.rs`
  (`sign_line` passes `restore_to_sign`'s answer; a `RESTORE_VERSION_GONE` line fails per line), `src/main.rs`.

**Interfaces — Produces:**
```rust
#[tauri::command] pub async fn med_list_restore_open(state, patient_id: String, charts: Vec<String>,
  version_id: String) -> Result<RestoreOpened, ErrorView>;
pub struct RestoreOpened { pub list: MedListView, pub line_id: String }   // the webview opens the editor on it
/// The prefill: typed_drug = term (+ " " + strength), directions = [amount unit] [route] [sig], line_text = both
/// joined, parts = one part with the version's coding. Pure.
pub(crate) fn restore_prefill(v: &VersionForRestore) -> (String, String, String, serde_json::Value);
/// At Sign, from the STORED row on the fresh read: Ok(None) for an ordinary line; Ok(Some) whose `stop` is the
/// companion when current, else each current line restoring a version of it (ruling R3);
/// Err(RESTORE_VERSION_GONE) when the version is not on the opened chart.
pub(crate) fn restore_to_sign<'a>(line: &DraftLine, version: Option<&'a VersionForRestore>, reason: &'a str)
  -> Result<Option<RestoreToSign<'a>>, &'static str>;
#[tauri::command] pub async fn med_version_check(state, patient_id: String, charts: Vec<String>, version_id: String,
  preset: Option<String>, detail: Option<String>, why: Option<String>, credential: Option<Credential>)
  -> Result</* the re-read pane, as allergy_overlays.rs's acts return it */, ErrorView>;
```
The restore command refuses (no row written): fixture mode, a version not on the OPENED chart (`VERSION_GONE`), a
list held on another terminal (the list's own elsewhere words). The check command follows `allergy_retract_impl`'s
shape: on-screen check, one key release, `check_version`, re-read; its "From:" maps through `info_source_of` and
`stamp` (2a), and with no preset its source is `unknown`. A restore line's `info_source` at Sign: its own, else
`unknown` (Global Constraints), applied in `list_input`'s caller, not in `charted_input`.

- [ ] **Step 1: Failing tests:**
  - pure: `the_prefill_reads_back_as_the_versions_dose_sig_and_route` (round-trip through `read_directions`, the
    reader Sign uses), `restore_to_sign_stops_only_a_current_companion`, `a_gone_version_refuses_the_line`,
    `a_restore_line_with_no_source_signs_unknown`, `the_restore_line_inherits_the_companions_charted_as`
    (none → `pre-existing`)
  - DB: `restore_open_adds_a_prefilled_line_with_its_mark`, `restore_open_refuses_a_linked_charts_version`,
    `one_sign_stops_the_companion_and_charts_the_restore` (cessation reason exact), 
    `a_restore_line_is_not_a_duplicate_of_its_companion_but_is_of_another` (Review Focus 2),
    `two_restores_of_one_line_duplicate_each_other` (Review Focus 3),
    `a_restore_whose_version_is_gone_is_kept` (Review Focus 4), `a_check_signs_and_the_pane_shows_it`,
    `a_check_with_no_preset_signs_unknown`
- [ ] **Step 2: Run** → FAIL (check the filter matched tests). **Step 3: Implement.** **Step 4: Run** → PASS, plus
  `med_list_tests`, `med_list_db_tests`, `med_list_versions_tests`, `med_list_versions_db_tests`,
  `med_list_versions_kept_db_tests`, `med_list_source_db_tests`, `med_chart_tests` unedited.
- [ ] **Step 5: Run every ROOT source guard DB-free** and register what the new files move (commit sites,
  `enroll_device_actor`, `db_errors_stay_legible`, `hex_decode_helper` per-file lists).
- [ ] **Step 6: Commit** `feat(window): "Use this version instead" and "Checked: not current"`.

### Task 7: The pane — each version's state, its buttons, the check row, the restore mark

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/med_versions_settle.js` (the two buttons on a collapsed version; the
  inline check row — preset select from 2a's `FROM_PRESETS`, detail, Why, the strip's Sign; the restore press →
  `med_list_restore_open` → draw the list, open the editor on `line_id`), `src/med_versions_settle_webview_tests.rs`
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/row_view.rs` (`VersionView` gains `version_id:
  String`, `settled: Vec<String>`, `open: bool`, `can_settle: bool`, `restored: bool`; `MedListRowView.replaced:
  Vec<String>`; a pure `settle_words(…)` building the Global Constraints lines) + `row_view_tests.rs`,
  `src/chart_versions.rs` (`read_settlements` in the same snapshot and savepoint as the versions; `attach_versions`
  takes the settlements and the opened chart), `src-ui/med_list_versions.js` (`versionRows`: state lines, summary
  with `", {u} to check"`, a hook for the buttons; `drawLineVersions`: the restore mark from `restores_words`),
  `src-ui/main.js` (the replaced rows), `src-ui/index.html`, `src-ui/style.css`.

Behaviour: the buttons are drawn only where `can_settle` (the version's own chart is the opened one) and not on a
restored version; disabled while any Sign or list act is in flight (`medSigning`), and while the list is held
elsewhere for the restore. The check row signs nothing until its Sign; it closes on success and shows the re-read
pane. Failure words appear beside the version, never only in a status bar.

- [ ] **Step 1: Failing tests** (webview: assert VISIBILITY — computed display + hidden ancestors — never text only):
  `a_version_of_the_opened_chart_shows_both_buttons`, `a_linked_charts_version_shows_none`,
  `a_restored_version_shows_restored_and_no_buttons`, `a_checked_version_shows_who_and_keeps_both_buttons`,
  `the_summary_counts_versions_to_check`, `restore_opens_the_prefilled_line_in_the_editor`,
  `the_check_row_signs_with_its_source_and_why`, `the_new_line_shows_what_it_replaced`;
  `row_view_tests.rs`: `settle_words_reads_checked_and_restored`, `a_stopped_companions_versions_are_not_open`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_list_versions_webview_tests`,
  `med_list_webview_tests`, `med_chart_webview_tests`, `delabel_webview_tests`, `row_view_tests`.
- [ ] **Step 5: Commit** `feat(window): settle a set-aside version from under its line`.

### Task 8: Docs — ADR-0103, spec, durable rules, RUNBOOK

- Create `docs/spec/decisions/0103-settling-a-set-aside-version.md`: the restore as a stop + a `restores` line in one
  transaction, enforced at commit by the floor; the remote door structure-only; no drug-field comparison; the
  inherited `charted_as`; `unknown` for a source-less restore and check; "checked" as its own type with no verdict
  field, locking nothing; the display; what is **not** guaranteed (no menu group for unsettled versions; a restore
  may chart a different drug than its version; drafts do not cross nodes, #963; the figures owed by a human). **Fact-
  check it sentence by sentence against the SQL and Rust** before review.
- Modify `docs/spec/index.md` (v0.105, ADR-0103), `docs/spec/data-model.md` §3.3 (one paragraph — medication prose is
  §3.3), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav), `docs/developers/durable-rules.md` (a slice 2b block:
  "a restore's pairing is checked at commit, by a pending row, never by the GUC"; "a restore line never takes a press
  default or a From: stamp"; "a restore line is not a duplicate of the line it replaces"),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §34 + `TEMPLATE.md` (restore and check from the collapsed version,
  timed), `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- [ ] **Step 1:** Write. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` clean.
  **Step 3:** `python3 scripts/check_closing_keywords.py` on the message. **Step 4:** Commit
  `docs(medication): ADR-0103, spec v0.105, RUNBOOK §34, durable rules`.

### Task 9: The whole gate

- [ ] After the last edit, in CI's order: fmt; clippy (root, GUI tree `--locked`); `cargo doc` with
  `RUSTDOCFLAGS=-D warnings` (both trees); `scripts/run-db-gated-tests.sh` (detached `nohup`, worktree at the PR
  head, ~2 h); the GUI tree's DB-gated `--bins`; `cargo deny`; the closing-keyword script on the PR body. Sum the
  passed counts; no `skipped:` line.
- [ ] Push; mark the PR ready. Write `Refs #1072` (it closes after slice 3, the threshold).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the admission medication history with a crossed-out alternative beside a line; next day
  the reviewer rings the GP, writes "confirmed with GP: 50 mg bd" and rewrites the line (or initials the
  alternative as checked).
- **Steps:**
  - Restore: paper N = 2 (rewrite the line + note the call) → architecture M = 1 signed act (the stop and the new
    line are two events under ONE signature) → UI K = 1 press + "From:" + "why" + Sign (+ the edits when the dose
    changed, as paper's rewrite carries them). `M ≤ N`.
  - Checked, not current: paper N = 1 (initial the alternative) → M = 1 signed act → K = 1 press + "why" + Sign.
  - Reading what was settled: paper N = 0 (it is on the sheet) → K = 1 (open the collapsed versions).
- **Time + cognitive load:** a restore from the press to the line in the editor ≤ 2 s; a check row's Sign ≤ 3 s
  (slice 1's Sign budget); a version's state is readable without opening anything but its line's collapse, and a
  version never reads as current. Measured by RUNBOOK §34 (human), owed by this slice.
