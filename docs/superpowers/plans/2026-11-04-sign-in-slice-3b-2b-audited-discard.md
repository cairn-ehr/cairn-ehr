# GUI sign-in, slice 3b-2b — the audited discard — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician can throw away a draft note they wrote: **Discard** + staff ID + passphrase, the same
gesture as Sign. The draft leaves the chart's drafts. The log keeps a signed, sealed record of what was thrown
away: who discarded it, when, and every part's text. The chart's notes list shows a one-line marker,
"Draft discarded by Dr X · 14:02", with the text behind a click. The marker is never shown as a note.

**Architecture:** A new born-sealed clinical stream `clinical.note-discard.asserted`
(`clinical.note-discard/1`) gets a floor check, a twin row and a one-row-per-event projection (`db/068`). One
new draft door (`db/069`, `cairn_note_draft_discard`) applies the rule, consumes the draft and records the spend
with the discarder's key. db/065's commit-time trigger (edited in place) then insists the spend names a discard
event of this chart, inserted in this transaction and **signed by that key**. So the rule holds at the floor, for
a raw-SQL client too, through a signature the database verified. `cairn_node::note::discard` runs the whole
discard in one transaction, the same shape as `sign_note`. The window gains one command, one button and the
marker.

**Tech Stack:** PostgreSQL 18 (new `db/068`, `db/069`; in-place edits to `db/037`, `db/044`, `db/065`, `db/066`),
Rust (`cairn-event`, `cairn-node`), Tauri 2 + plain JS (`cairn-gui-tauri`, no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
§3.5, read with the decisions that narrowed it since:
[ADR-0088](../../spec/decisions/0088-whos-here-and-a-drafts-parts.md) maintainer decision 6 (any signer may
discard an unknown writer's draft) and [ADR-0089](../../spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md)
decision 15 (a draft with any named writer other than the discarder is refused; a co-signed discard is #980).
**Maintainer, 2026-11-04:** the chart shows each discard as a one-line marker, its text behind a click, never as a
note. This replaces the design's "only in the chart's audit view", because no audit view exists. Read ADR-0087,
ADR-0088 and ADR-0089 and the 3b-2a block of `docs/developers/durable-rules.md` before Task 1.

## Global Constraints

- **The rule (ADR-0089 decision 15), exactly:** a draft may be discarded only when **every non-blank part** has a
  writer that is NULL (not known) or whose **current vault key** (`cairn_vault_member_key(writer)`) equals the
  discarder's key. Compare by KEY, never by staff id (ruling R-B). A named writer with no key makes the discard
  `not-yours`. It is never read as "not known": that would quietly lose who wrote it (principle 4). Blank parts
  never count: the blank test is `cairn_twin_is_present`, the floor's only blank test.
- **The discarder is a member whose role may act clinically** (`cairn_vault_purpose_allowed(role, false,
  'clinical')`), found by key through `vault_member.signing_key_id`. A superuser or an admin may not discard.
- **Wire shape, frozen once the first discard is signed.** Event type `clinical.note-discard.asserted`,
  `schema_version = "clinical.note-discard/1"`. Payload keys, exactly:
  `parts`: a non-empty array, in writing order, of objects with exactly the keys
  `writer` (`"discarder"` | `"not-known"`), `narrative` (string, escaped once by `EscapedNarrative::of_typed`, the
  note's own escape), `stated_time` (RFC 3339 string or `null`), `recording_started`, `recording_ended` (RFC 3339
  strings, the database's stamps for that part; `recording_ended` is the part's `last_keystroke`).
  Envelope `t_effective` is the **discard instant**: the door's `clock_timestamp()`, which the door returns. The
  window never sends a time. Contributors are `[{discarder, authored}, {node, recorded}]` through
  `prepare_sealed` with `Some(author)` (ADR-0053). `writer` names no key: under the rule, every named writer IS the
  discarder. #980 adds a key-bearing value additively.
- **The twin** (mechanical, pure, from the payload): the first line is exactly
  `Draft discarded; it was never entered in the record.` Then, per part, a blank line, a heading line
  `Part N, written by the discarder:` or `Part N, writer not known:`, then the part's text **unescaped**. No time
  goes in the twin (a time in the twin would be a second, unchecked copy of the envelope's).
- **The floor refuses** (both doors, `/1` only; at the remote door a newer profile is carried and never projected,
  ruling R8 as in db/064): a missing payload; a wrong schema tag (at the authoring door); `parts` not a non-empty
  array; a part not an object; a `writer` outside the enum; a narrative that is not a string or is blank
  (`cairn_twin_is_present`); a narrative failing `cairn_check_note_markup` or `cairn_check_note_anchors(text,
  '[]'::jsonb)` (no media exists, so every anchor dangles); a malformed or offset-less recording time;
  `recording_started` after `recording_ended`; a `stated_time` that is neither null nor an instant; `t_effective`
  unset.
- **The projection is read only for the marker.** `clinical_note_discard` is never joined into `clinical_note`,
  never counted as a note, and `cairn_node::note::read`'s `NOTES_SQL` is unchanged. ADR-0087 R8 still holds: a
  chart's NOTES are read only from `clinical_note`.
- **A discarded draft answers `discarded`.** The save and take doors (db/066, in place) and the new door answer
  `discarded` for a draft whose spend row carries a `discarded_by`, never `spent` (which the window words as
  "already signed"). The column is created in **db/065** with its paired `ALTER`, because db/066's doors read it
  (3b-2a's durable rule: "a column a door body reads lives with its table").
- **Discard carries the text on screen, as Sign does.** In one transaction: save the on-screen text as the
  on-screen writer's part (`save_part`), then the discard door, then seal and submit. A refused save writes
  nothing. Without this, the audit copy could miss the last second of typing.
- **An all-blank draft is deleted with no event and no spend** (design §3.5: nothing to audit), and that deletion
  commits: `DiscardOutcome::Empty`.
- **Lock order is every draft door's:** the advisory lock `'note_draft:' || draft_id` (WAITED for, as save, take and
  spend do; only the timer's door tries it), then the header `FOR UPDATE`, then its parts. NO savepoint around the
  submit (db/065's trigger compares the row's `xmin` to this transaction's id).
- **Replay traps** (memory "migration replay"): the `note-discard` timing kind goes into **db/044 in place**, in BOTH
  the `CREATE TABLE` line and the drop/re-add. The shred hook goes into **db/037's `cairn_execute_shred` in place**,
  guarded by `to_regclass`. db/065's trigger function and db/066's two doors are edited **in place**
  (`CREATE OR REPLACE`, same return shapes). A new door's return shape is final.
- **`clinical.note-discard.asserted` stays OFF db/048's `cairn_event_type_has_no_thread` list**, as notes do. Pin it
  in `db/tests/068`.
- **Pinned guards a new member must join** (memory "pinned-count guards"), with today's values:
  `db/tests/034_twin_registry_test.sql` (27 → 28) · `crates/cairn-node/tests/twin_registry.rs:103` (27 → 28, plus an
  `expected` vec entry) · `db/tests/039_projection_registry_test.sql:33` (33 → 34) ·
  `crates/cairn-node/tests/projection_registry.rs:65` (31 → 32) · `crates/cairn-node/src/db.rs` `SCHEMA` list +
  `SCHEMA_GENERATION` 67 → **69** (`crates/cairn-event/src/schema_generation.rs:45`) · `late_custody_guards.rs` (the
  applier is `heal_safe = TRUE`) · `floor_execute_grants.rs` (the floor fn revoked from PUBLIC) ·
  `db_errors_stay_legible.rs:720` `MARKED_COMMIT_SITES` (+ `crates/cairn-node/src/note/discard.rs`) ·
  `window_role_grants_notes.rs` (the new door driven to its deep path under `cairn_window`) · the window's lists
  named in Tasks 5 and 6.
- **House rules:** TDD; every non-trivial fn documented for a junior reader; files under 500 lines; no
  `salt`/`nonce`/`iv` names; `scripts/check_closing_keywords.py` before every commit; DB-gated tests show no
  `skipped:` line (`--nocapture`); a DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`. **A task touching
  `cairn-gui/*/src` runs every root source guard DB-free** (`grep -ln production_rust_files crates/*/tests/*.rs`),
  and uses its own `CARGO_TARGET_DIR` (memory "root source guards sweep cairn-gui src").

## Review Focus

1. **A discard racing the timer, or a Sign on another terminal.** Exactly one wins. A discard after a timer
   commit answers `committed`. After a Sign it answers `spent`. A Sign or save after a discard answers
   `discarded`. Pinned in Task 3 (SQL) and Task 4 (Rust, two connections).
2. **A raw-SQL client discards a colleague's draft:** it calls the door with its own key (refused `not-yours`),
   or with the colleague's key (the door consumes the draft; the COMMIT is refused, because no event signed by
   that key was inserted). Pinned in Task 3.
3. **Nurse A types, then Dr B (a different key) presses Discard on the same draft** (the who's-here pick does not
   matter; the signature does). The discard is refused `not-yours`. The window says the draft will go in by
   itself at its due time, or is stuck, reusing `cairn_note_drafts_due`'s words, and never offers a step that
   would be refused too. Pinned in Tasks 4 and 5.
4. **A discard pressed during the debounce.** The audit copy holds the text on screen, not the last autosave.
   Pinned in Task 4.
5. **A doubted linked set.** A discard marker on a chart other than the opened one is withheld and counted, as a
   note is. Pinned in Task 4.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** tearing up your own unfiled page before it goes in the folder. You cannot tear out a
  colleague's handwriting from a page that is already in the folder.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Discarding one's own draft: 1 (tear it up) → 1 (Discard + staff ID + passphrase: slice 1's strip, one act as
    Sign is counted) → 1.
  - Discarding a draft of unknown writing (typed into the wrong chart): 1 → 1 → 1.
  - A draft holding a colleague's part: paper permits no tear-up → refused, it goes in by the timer → no gesture.
    A co-signed discard is #980.
  - Seeing that a draft was discarded: paper leaves no trace (the trace is the gain). 0 → 0 (the marker is in the
    list) → 0. Reading the discarded text is one press.
  - `M = N` everywhere: no architecture defect to file.
- **Time + cognitive load:** discard ≤ 3 s, the same budget as sign (RUNBOOK §16), timed through
  `record_timing("note-discard", 1, …)`. One button beside Sign and the same strip; the refusal names who else
  wrote in the draft and what happens next. Measurement is owed by a human in **RUNBOOK §21** (Task 7).

---

### Task 1: The discard body and its twin (`cairn-event`, pure)

**Files:**
- Create: `crates/cairn-event/src/note_discard.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`pub mod note_discard;` beside `pub mod note;`)
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `cairn_event::note::{EscapedNarrative, StatedTime, unescape_note_markup}`.
- Produces:
  - `pub const NOTE_DISCARD_EVENT_TYPE: &str = "clinical.note-discard.asserted";`
  - `pub const NOTE_DISCARD_SCHEMA_VERSION: &str = "clinical.note-discard/1";`
  - `pub enum DiscardedWriter { Discarder, NotKnown }` with `fn wire(&self) -> &'static str` (`"discarder"` /
    `"not-known"`).
  - `pub struct DiscardedPart<'a> { pub writer: DiscardedWriter, pub narrative: EscapedNarrative, pub stated:
    StatedTime<'a>, pub recording_started: &'a str, pub recording_ended: &'a str }`
  - `pub fn discard_payload(parts: &[DiscardedPart]) -> serde_json::Value`
  - `pub fn render_discard_twin(parts: &[DiscardedPart]) -> String`

- [ ] **Step 1: Write the failing tests:**
  `the_payload_has_exactly_the_frozen_keys` (top level `{"parts"}` only; each part exactly `writer, narrative,
  stated_time, recording_started, recording_ended`; `stated_time` is `null` for `NotStated`),
  `the_narrative_is_escaped_once` (typed `BP <90 [L]` → payload `BP \<90 [L]`),
  `the_twin_is_the_typed_text_under_part_headings`: two parts (discarder `a *b*`, not known `c`) render exactly
  `"Draft discarded; it was never entered in the record.\n\nPart 1, written by the discarder:\na *b*\n\nPart 2, writer not known:\nc"`,
  and `the_twin_names_no_time`.
- [ ] **Step 2:** `cargo test -p cairn-event note_discard`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement** the module. The twin unescapes with `unescape_note_markup`, as the note twin does.
- [ ] **Step 4:** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): the discard body and its twin (clinical.note-discard/1)`.

### Task 2: The discard stream's floor and projection (`db/068_clinical_note_discard.sql`)

**Files:**
- Create: `db/068_clinical_note_discard.sql`, `db/tests/068_clinical_note_discard_test.sql` (opens with
  `\ir _scratch_database_guard.sql`)
- Modify: `db/037_born_sealed.sql` `cairn_execute_shred` in place (one guarded
  `DELETE FROM clinical_note_discard WHERE content_address = v_ca;` beside `clinical_note`'s)
- Modify: `crates/cairn-node/src/db.rs` (`SCHEMA` += `068_clinical_note_discard`),
  `crates/cairn-event/src/schema_generation.rs` (→ 68 here; Task 3 makes it 69), and the pinned counts in Global
  Constraints (twin 28, projection 34/32, `twin_registry.rs`'s `expected` entry)
- Test: the SQL file, and `crates/cairn-node/tests/note_discard_floor.rs` (a Rust round trip, following
  `note_floor.rs`)

**Interfaces:**
- Consumes: Task 1's constants and builders; db/064's `cairn_check_note_markup`, `cairn_check_note_anchors`;
  db/001's `cairn_t_effective`; db/005's `cairn_twin_is_present`, `cairn_clear_payload`.
- Produces: `event_type_class` row `('clinical.note-discard.asserted', 'additive', FALSE)`; floor
  `cairn_check_note_discard(p_type text, b jsonb) RETURNS void` (revoked from PUBLIC), registered in
  `cairn_event_twin_check` with the message `note discard requires a non-empty authored twin (§3.13/§3.19)`
  (the #214 converge idiom); table
  `clinical_note_discard(event_id uuid PK, patient_id uuid NOT NULL, parts jsonb NOT NULL, t_effective timestamptz
  NOT NULL, hlc_wall bigint, hlc_counter int, origin text, content_address bytea NOT NULL)` with
  `GRANT SELECT … TO cairn_agent` and indexes on `patient_id` and `content_address`; applier
  `clinical_note_discard_apply(e event_log)` (projects `/1` only; `NULL` clear payload → nothing; `ON CONFLICT DO
  NOTHING`), registered in `cairn_projection_apply` with `run_order 20, heal_safe TRUE`.

- [ ] **Step 1: Write the failing SQL tests**, one `DO` block each, named in a leading comment:
  `a_well_formed_discard_is_admitted_and_projected`, one refusal test per floor rule in Global Constraints
  (asserting its message), `a_remote_newer_profile_is_carried_not_projected`, `the_type_is_not_threadless`
  (absent from `cairn_event_type_has_no_thread`), `a_shred_scrubs_the_discard_row`. Copy db/tests/064's helpers for
  building and submitting a sealed body.
- [ ] **Step 2:** run the file with `psql` against a scratch database (memory "PG test substrate"). Expected: FAIL.
- [ ] **Step 3: Implement** db/068 on db/064's pattern: register type → floor → twin row → table → applier →
  projection registry, in one `BEGIN … COMMIT`. Then edit db/037, db.rs, the generation and the pinned counts.
- [ ] **Step 4:** the SQL file, then `cargo test -p cairn-node --test note_discard_floor --test twin_registry --test
  projection_registry --test late_custody_guards --test floor_execute_grants -- --nocapture`. Expected: PASS, no
  `skipped:`.
- [ ] **Step 5: Commit** `feat(note): the discard stream's floor and projection (db/068)`.

### Task 3: The discard door and the `discarded` answer (`db/069_note_draft_discard.sql`)

**Files:**
- Create: `db/069_note_draft_discard.sql`, `db/tests/069_note_draft_discard_test.sql`
- Modify in place: `db/065_note_draft.sql` (`note_draft_spent.discarded_by text` + its paired `ALTER TABLE … ADD
  COLUMN IF NOT EXISTS`; `note_draft_spent_names_its_note` gains the discard arm below), `db/066_note_draft_parts.sql`
  (save and take answer `'discarded'` when `discarded_by IS NOT NULL`, checked before `by_timer`'s `committed`),
  `db/044_ui_gesture_timing.sql` (`'note-discard'` in BOTH places)
- Modify: `crates/cairn-node/src/db.rs` (`SCHEMA` += `069_note_draft_discard`), generation → **69**
- Test: the SQL file

**Interfaces:**
- Consumes: db/065–067's tables and the lock idiom; `cairn_vault_member_key`, `cairn_vault_purpose_allowed`.
- Produces:
  - `cairn_note_draft_discard(p_draft uuid, p_patient uuid, p_terminal text, p_event uuid, p_discarder_key text)
    RETURNS TABLE(outcome text, seq int, by_discarder boolean, narrative text, stated_time timestamptz,
    recording_started timestamptz, last_keystroke timestamptz, discarded_at timestamptz)`, `SECURITY DEFINER SET
    search_path = public, pg_temp`, granted to `cairn_window` only. A refusal is ONE row whose `outcome` is
    `'spent' | 'committed' | 'discarded' | 'moved' | 'elsewhere' | 'gone' | 'not-yours' | 'no-such-signer'`, its
    other columns NULL, and nothing written. `'empty'` is one row, the draft and its revisions deleted, no spend
    recorded. Success is one row per non-blank part, `outcome = 'ok'` (never `'discarded'`, which is the refusal for a draft already discarded), in writing order, all carrying the
    same `discarded_at`. The header, parts and revisions are deleted, and
    `note_draft_spent(draft_id, patient_id, event_id, discarded_by)` is recorded with `discarded_by =
    p_discarder_key`.
  - The trigger's discard arm: when `NEW.discarded_by IS NOT NULL`, the named event must be
    `clinical.note-discard.asserted`, of `NEW.patient_id`, `xmin = pg_current_xact_id()::xid`, **and
    `signer_key_id = NEW.discarded_by`**. Otherwise the existing note arm, unchanged.
- Order inside the door: refuse NULL arguments (raise); `no-such-signer` unless a `vault_member` with
  `signing_key_id = p_discarder_key` may act clinically (a plain read, before the lock, as save's writer check);
  take the advisory lock (wait); spent row → `discarded` / `committed` / `spent`; header `FOR UPDATE` (absent →
  `gone`; other chart → `elsewhere`; other terminal → `moved`); any non-blank part breaking the rule →
  `not-yours`; no non-blank part → delete, `empty`; else return the parts, delete, record the spend.

- [ ] **Step 1: Write the failing SQL tests:** `own_draft_is_discarded_and_spent_with_the_key`,
  `unknown_writer_draft_any_clinical_signer_may_discard`, `a_colleagues_part_is_not_yours`,
  `a_named_writer_without_a_key_is_not_yours`, `a_blank_colleague_part_does_not_count`,
  `an_admin_key_is_no_such_signer`, `all_blank_is_deleted_empty_no_spend`,
  `the_commit_refuses_a_spend_without_its_signed_event` (door with the colleague's key, no event → COMMIT raises),
  `the_commit_refuses_an_event_signed_by_another_key`, `save_after_discard_answers_discarded`,
  `take_after_discard_answers_discarded`, `discard_after_timer_answers_committed`,
  `discard_after_sign_answers_spent`, `moved_and_elsewhere_and_gone`, `note_discard_is_a_timing_kind`.
- [ ] **Step 2:** run it. Expected: FAIL.
- [ ] **Step 3: Implement** db/069 and the in-place edits.
- [ ] **Step 4:** run 065, 066, 067 and 069's SQL tests, then `scripts/run-db-gated-tests.sh`'s SQL step only for
  db/tests (it replays every file: the in-place edits must replay clean). Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): the discard door; a discarded draft answers discarded (db/069)`.

### Task 4: `cairn_node::note::discard` and the chart's discard markers

**Files:**
- Create: `crates/cairn-node/src/note/discard.rs`; `crates/cairn-node/tests/note_discard.rs`
- Modify: `note/mod.rs` (`pub mod discard;`), `note/draft.rs` (`SaveOutcome::Discarded`, `TakeOutcome::Discarded`
  and their words), `note/sign.rs` (`SignRefusal::Discarded`, `refusal_for`), `note/read.rs`
  (`PatientNoteList.discards`, `discards_withheld_on_linked`), `crates/cairn-node/tests/db_errors_stay_legible.rs`
  (`MARKED_COMMIT_SITES` += `note/discard.rs`; `EVENTLESS_COMMITS` += `note/discard.rs` = 1, the `Empty` path's commit, as `note/commit.rs`'s), `crates/cairn-node/tests/window_role_grants_notes.rs`
- Test: `note_discard.rs`, additions to `note_read.rs`; pure tests in `discard.rs`

**Interfaces:**
- Consumes: Task 1; Task 3's door; `save_part`, `writer_key`, `prepare_sealed`, `AuthorParams`, `next_hlc`,
  `Committed`, `commit_failure`; `SignNoteInput` (reused unchanged as the discard's input: same fields, same
  meaning).
- Produces:
  - `pub enum DiscardRefusal { Spent, Committed, Discarded, Elsewhere, Moved, NoChart, NoSuchWriter, NotYours,
    NoSuchSigner, Gone }`
  - `pub enum DiscardOutcome { Recorded(Committed<Uuid>), Empty }`
  - `pub async fn discard_draft(client: &mut tokio_postgres::Client, node_sk: &SigningKey, input:
    &SignNoteInput<'_>, author: &AuthorParams<'_>) -> anyhow::Result<Result<DiscardOutcome, DiscardRefusal>>`
  - `pub struct DiscardMarker { pub event_id: Uuid, pub chart: Uuid, pub discarded_at: DateTime<Utc>, pub
    discarder: Option<String> /* cairn_note_signer_name(signer_key_id) */, pub parts: Vec<DiscardedText> }` and
    `pub struct DiscardedText { pub by_discarder: bool, pub text: String /* unescaped */, pub stated:
    Option<DateTime<Utc>>, pub recording_started: DateTime<Utc>, pub recording_ended: DateTime<Utc> }`
  - `PatientNoteList` gains `pub discards: Vec<DiscardMarker>` (newest `discarded_at` first) and
    `pub discards_withheld_on_linked: usize`, under the notes' own doubted-link rule.
- Shape: `sign_note`'s, step for step. Open the transaction, `save_part`, map a non-`Saved` answer, call the door
  with `first = Uuid::now_v7()` and `author.human_kid`, map a refusal word (an unknown word is an error, never a
  default), commit and return `Empty` on `empty`, else build one `DiscardedPart` per row (`by_discarder` →
  `Discarder`/`NotKnown`), take one HLC tick, `prepare_sealed(&tx, node_sk, body, Some(author))`, submit with no
  savepoint, commit through `commit_failure`.

- [ ] **Step 1: Write the failing tests** (DB-gated, `test_serial_guard`): `discarding_own_draft_records_one_signed_event`
  (event type, `signer_key_id` = human key, payload parts match, draft gone, `open_drafts` empty),
  `discard_carries_the_text_on_screen` (save "a", discard with "ab" → the payload narrative is `ab`),
  `a_colleagues_draft_is_refused_not_yours_and_kept`, `an_all_blank_draft_is_empty_and_gone`,
  `sign_after_discard_is_refused_discarded`, `discard_racing_sign_one_wins` (two connections),
  `the_marker_lists_the_discard_not_as_a_note` (`notes` unchanged, `discards.len() == 1`, discarder named, text
  unescaped), `a_doubted_set_withholds_a_linked_charts_marker`; pure: `refusal_words_map_every_door_word` and
  `an_unknown_word_is_an_error`. `window_role_grants_notes.rs` drives `discard_draft` to its deep path under
  `cairn_window`.
- [ ] **Step 2:** `CARGO_TARGET_DIR=/tmp/cairn-3b2b cargo test -p cairn-node --test note_discard -- --nocapture`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** `discard.rs` stays under 300 lines. Read the markers from `clinical_note_discard`
  joined to `event_log` for `signer_key_id`.
- [ ] **Step 4:** run `note_discard`, `note_read`, `note_sign`, `note_sign_parts`, `note_commit`,
  `window_role_grants_notes`, `db_errors_stay_legible`, all with `--nocapture`. Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(note): discard a draft in one transaction; the chart's discard markers`.

### Task 5: The window's discard command (`cairn-gui-tauri`, Rust)

**Files** (all under `cairn-gui/cairn-gui-tauri/`):
- Create: `src/notes_discard.rs` (the command, its `_impl`, its words), `src/notes_discard_tests.rs`
  (`notes.rs` is 388 lines and `notes_tests.rs` 399, so the discard goes beside them, not in them)
- Modify: `src/main.rs` (`mod notes_discard;` and its test mod; `generate_handler!` += `notes_discard::note_discard`
  beside `notes::note_sign`, :165), `src/notes_words.rs` (`save_view`: `SaveOutcome::Discarded`, a new
  `SaveView.discarded: bool`, the same fork words as `Spent`; `take_view`: `TakeOutcome::Discarded`;
  `sign_refusal_view`: `SignRefusal::Discarded` → `stale(DISCARDED)`), `src/notes_parts.rs` (`NotePane.discards:
  Vec<DiscardView>`; the withheld sentence counts discards too)
- Test: `src/notes_discard_tests.rs`, additions to `src/notes_parts_tests.rs` and `src/notes_tests.rs`

**Interfaces:**
- Consumes: Task 4's `discard_draft`, `DiscardOutcome`, `DiscardRefusal`, `DiscardMarker`; `note_sign_impl`'s
  helpers in `notes.rs` (`displayed_patient`, `FIXTURE`, `terminal_of`, `node_sk`), `AppState::signer_for_act`
  (`state.rs:337`), `key_locked` (`write_failure.rs:235`), `db_diagnosis::commit_outcome_unknown_event`,
  `record_timing`, `view::instant`.
- Produces:
  - `#[tauri::command] pub async fn note_discard(state, patient_id: String, draft_id: String, narrative: String,
    stated_time: Option<String>, writer: Option<String>, credential: Option<Credential>) -> Result<NoteSignReport,
    ErrorView>`: `note_sign`'s arguments exactly, so the webview sends the same object.
  - `pub(crate) async fn note_discard_impl(...)`: `note_sign_impl`'s order (`notes.rs:238-317`): displayed chart →
    fixture refusal → the time check before the key → `terminal_of` → `signer_for_act` → `discard_draft` → words →
    `record_timing(state, "note-discard", 1, ms)`. Make only the needed `notes.rs` helpers `pub(crate)`; do not
    copy them.
  - `pub(crate) fn discard_refusal_view(r: DiscardRefusal) -> ErrorView`: `Spent`, `Committed`, `Discarded` →
    `stale(…)` (the pane is behind the record); the rest → `refused(…)`.
  - `pub struct DiscardView { pub event_id: String, pub summary: String, pub parts: Vec<DiscardPartView> }` and
    `pub struct DiscardPartView { pub heading: String, pub text: String }`, built by the pure
    `pub(crate) fn discard_view<Tz: TimeZone>(m: &DiscardMarker, tz: &Tz, today: NaiveDate) -> DiscardView`.
- **Exact copy** (constants in `notes_discard.rs`; the tests pin each one):
  - Recorded: `Discarded. The draft is gone from the chart; the record keeps what it said.`
  - Empty: `Discarded. The draft was empty, so nothing was kept.`
  - `NotYours`: `Not discarded: someone else wrote part of this draft, and only its writer may throw their part
    away. The draft's own line says when it goes into the record by itself.`
  - `NoSuchSigner`: `Not discarded: this staff ID may not discard a clinical note.`
  - `Discarded` (as a save, take or sign refusal): `This draft was discarded; it is no longer open.`
  - Each other refusal reuses the sign refusal's sentence for the same case (`SPENT`, `SIGN_ELSEWHERE`,
    `SIGN_MOVED`, `NO_CHART`, `SIGN_NO_SUCH_WRITER`, `SIGN_COMMITTED`, `GONE`), prefixed `Not discarded: `.
  - Marker summary: `Draft discarded by <name> · <instant>` (`<name>` = the discarder's name, else `someone this
    node cannot name`; `<instant>` = `view::instant(discarded_at, tz, today)`). Part heading: `Part N, written by
    <name>` or `Part N, writer not known`, followed by ` · started <instant>`.

- [ ] **Step 1: Write the failing tests:** `discard_refuses_a_chart_not_on_screen`,
  `discard_in_fixture_mode_refuses`, `a_bad_time_is_refused_before_the_key`, `discard_words_each_refusal` (every
  `DiscardRefusal` variant against its exact sentence, and stale vs refused), `discard_words_each_outcome`,
  `a_discarded_save_reads_like_spent_and_says_discarded`, `take_words_discarded`,
  `the_marker_summary_and_headings_are_exact`, `the_withheld_sentence_counts_discards`, and a DB-gated
  `note_discard_impl_discards_under_the_window_login` beside the existing `*_impl` DB tests (#905: CI does not run
  this tree's DB tests, so run them locally).
- [ ] **Step 2:** `cd cairn-gui && CARGO_TARGET_DIR=/tmp/cairn-3b2b-gui cargo test -p cairn-gui-tauri notes_discard`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** `notes_discard.rs` under 300 lines.
- [ ] **Step 4:** the cairn-gui gate commands (memory "three Cargo trees": clippy `--locked -D warnings`, `cargo
  test`, `cargo doc` with `RUSTDOCFLAGS=-D warnings`), then every root source guard DB-free (Global Constraints).
  Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): the discard command and its words`.

### Task 6: The Discard button and the marker (`notes.js`)

**Files** (all under `cairn-gui/cairn-gui-tauri/`):
- Create: `src-ui/notes_discard.js` (`discardNote()` and `renderDiscards(pane)`; `notes.js` is 367 lines),
  `src/notes_discard_webview_tests.rs`
- Modify: `src-ui/index.html` (`<button id="note-discard" type="button">Discard draft</button>` right after
  `#note-sign` (:394); `<ul id="note-discards" aria-label="Discarded drafts"></ul>` after `#note-list`; the new
  script tag after `notes.js`), `src-ui/notes.js` (`renderNotes` calls `renderDiscards(pane)`; `setSigning`
  disables `#note-discard` too; the `#note-discard` click listener mirrors `#note-sign`'s at :357-366, setting the
  same `noteSignInFlight` / `noteSigning` flags so idle.js's re-lock and whos_here.js's switch wait for a discard
  as they wait for a Sign), `src-ui/notes_save.js` (:107-108: `view.discarded` is handled exactly as `view.spent`)
- Modify the hand-kept lists: `src/commands_tests.rs:369-389` (+ `notes_discard.js`), `src/signer_webview_tests.rs:24`
  `WRITERS` (8 → 9, + `("notes_discard.js", …, "discardNote")`), `src/reread_webview_tests.rs:~515` (+
  `discardNote`), `src/whos_here_webview_tests.rs:106-112` (+ `discardNote`), and `notes_webview_tests.rs:244`'s
  field-drift guard (+ `DiscardView`, `DiscardPartView`, `SaveView.discarded`)

**Interfaces:**
- Consumes: Task 5's `note_discard` command (same argument object as `note_sign`) and `NotePane.discards`;
  `signerCredential`, `clearSigner`, `failureText`, `behindTheRecord`, `refresh`, `closeEditor`, `noteStatus`,
  `saveNoteNow`.
- Produces: `async function discardNote()` returning `"discarded" | "not-discarded" | "unknown"`, `signNote`'s shape
  step for step (`notes.js:230-286`): the partly typed time check, cancel the pending save, set
  `held.mayBeSigned = true` (the press carries the text, so a late save answering `discarded` must not fork it),
  `setSigning(true)`, invoke, clear the strip both ways, word the outcome, re-read. `function renderDiscards(pane)`:
  one `<li>` per `pane.discards` entry holding a `<details>` whose `<summary>` is `summary`, and one block per part
  (`heading`, then `text`), all set as `textContent`.

- [ ] **Step 1: Write the failing source-guard tests** (`source_guard::js_fn` / `js_const` / `position`, as
  `notes_webview_tests.rs` does): `discard_cancels_the_pending_save_before_it_discards`,
  `discard_sends_the_credential_and_the_text_on_screen`, `the_discard_button_never_asks_to_confirm` (no
  `confirm(`, `alert(`, `prompt(` in `notes_discard.js`), `the_editor_is_read_only_while_a_discard_is_in_flight`,
  `a_discard_marks_the_press_as_carrying_the_text`, `a_discarded_save_is_handled_as_spent`,
  `markers_are_text_never_html` (no `innerHTML` in `notes_discard.js`), `a_marker_is_a_details_not_a_note` (it is
  written to `#note-discards`, never `#note-list`).
- [ ] **Step 2:** `cargo test -p cairn-gui-tauri notes_discard_webview`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** the cairn-gui gate and the root source guards, as Task 5. Then a mock walk (memory "webview mock
  walk recipe"): stub `invoke` with Rust-dumped payloads, and assert the marker and the button are VISIBLE
  (computed display and hidden ancestors, never just `textContent`) in Chromium and WebKit.
- [ ] **Step 5: Commit** `feat(window): Discard draft beside Sign; discarded drafts as one-line markers`.

### Task 7: ADR-0090, spec, RUNBOOK §21, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0090-the-audited-discard.md`
- Modify: `docs/spec/index.md` (v0.91 → **v0.92**), `docs/spec/decisions/README.md`, `mkdocs.yml` (the nav line),
  `docs/spec/data-model.md` §3.19 (the discard stream, a sentence and a link),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (**§21**: discard ≤ 3 s; the refusal's words for a colleague's
  draft; the marker and its text, by keyboard and VoiceOver; WebKit) and `TEMPLATE.md`,
  `docs/developers/durable-rules.md` (a 3b-2b block: the key comparison, the trigger's signer arm, `discarded` ≠
  `spent`, the marker is never a note — each with its tempting wrong fix and pin), `docs/HANDOVER.md`,
  `docs/ROADMAP.md`

- [ ] **Step 1:** write ADR-0090. Context; the rule as built; the wire shape and the twin; why the floor holds
  (the trigger's signer arm); the marker (maintainer 2026-11-04, superseding design §3.5's "audit view only"); the
  all-blank case; consequences: the discarded text is in the full-text/RAG substrate through the twin;
  a raw-SQL client holding its OWN released key can sign a discard whose text differs from the draft it consumed
  (the floor cannot see the deleted rows: a named residual, filed as an issue); #980 still open.
  **Fact-check every sentence against the SQL before committing** (memory "SDD evidence + ADR fact-check").
- [ ] **Step 2:** the rest of the files; `uv run --with-requirements docs/requirements.txt -- mkdocs build`.
  Expected: no warnings about the new pages.
- [ ] **Step 3:** file the residual issues; `scripts/check_closing_keywords.py`; commit
  `docs(note): ADR-0090 — the audited discard; spec v0.92, RUNBOOK §21`.
