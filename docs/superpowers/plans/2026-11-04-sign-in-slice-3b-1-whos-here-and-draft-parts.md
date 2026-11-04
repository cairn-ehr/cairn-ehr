# GUI sign-in, slice 3b-1 — who's here, the idle re-lock, and a draft's parts — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One gesture says who is at a shared terminal (a claim, never a login), the terminal re-locks itself when
idle, and a note draft becomes a sequence of **parts, one per writer**, that nobody can rub out. A draft can be
**continued at another terminal** by moving it there. Signing a draft commits **one event per part**, each with its
exact writer.

**Architecture:** `db/066` replaces 3a's three draft doors with part-aware ones (`db/065` loses them in place) and
adds a staff directory door. `cairn-event` gains the attesting-signer rewrite and the `addendum-to` ref.
`cairn_node::note` saves parts, moves drafts, and signs a draft as N events in one transaction. The window gains a
`whos_here` module (in-memory claim, recent people, bookmarks), an idle re-lock, and part-aware note editing.

**Tech Stack:** PostgreSQL 18 (new `db/066`; `db/065` edited in place), Rust (`cairn-event`, `cairn-node`,
`cairn-gui-tauri`), plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
§3.1, §3.2, §3.4 (first bullet), §3.7 (the `?`), §4.2, §4.3. Read [ADR-0087](../../spec/decisions/0087-the-first-note-stream-and-its-draft.md)
and `docs/developers/durable-rules.md`'s **sign-in slice 3a** block before Task 1. **This plan covers PR 3b-1 only.**
**3b-2** (the last-keystroke clock, commit door, runner, console knob, `Asserted` + its display, the audited
discard) gets its own plan against what this one builds.

## Decisions taken with the maintainer on 2026-11-04 (not in the design; the ADR records them)

1. **3b is split into two PRs**, 3b-1 (this plan) and 3b-2.
2. **"Continue" at another terminal MOVES the draft there** (supersedes the design's §3.2 rule I1, "Continue
   elsewhere starts an addendum draft"). The move is one atomic statement on the node that holds the draft. The old
   terminal's next save answers `moved`, and whatever that window had not yet saved is kept as a **new draft of its
   own**, so nothing typed is lost. When the draft's node cannot be reached, the clinician can only start a new note.
   Slice 5 (terminals become nodes) must answer the partitioned case: **#963**.
3. **The bookmark is OFFERED, pre-focused, never auto-opened** (§10 finding 1). The maintainer rules this is not an
   `M > N` defect: paper's counterpart includes walking back to wherever the folder was left, or having it fetched,
   and one pre-focused press replaces that walk.
4. **ADR-0087 decision 9 is confirmed:** drafts are in no backup and no export. Moving a draft between terminals
   never touches a backup.
5. **Not in 3b-1, by the design's own table:** the picker's pending-sign-off counts and the person menu (§3.6, 3c),
   the `Asserted` grade (3b-2, together with its only producer, the auto-commit).
6. **(3b-2's, recorded here so it is not lost)** A draft whose writer is unknown may be discarded by **any** signer,
   via the signed, audited discard event.

## Global Constraints

- **Who's here is a claim, never authentication** (design §3.1). It never signs, never releases a key, and is
  never required to READ a chart. It is required only to begin writing, where "Don't know" is an allowed answer.
- **Switching who's here, and the idle re-lock, forget any held key** (design §3.1, review finding). Reuse the
  `forget_held_key` path (`commands.rs:137`). Never carry a held key across a change of claim.
- **The writer of a save is captured when the editor opens and travels with every save** (`noteDraft.writer`). It is
  never re-read from the current claim at save time: a debounced save landing after a switch must still name the
  person who typed it.
- **A writer writes only the TAIL part of a draft, and only when it is theirs.** Otherwise their save APPENDS a new
  part. No door can address, change or remove another writer's part. *Shortcut:* a writer's own earlier non-tail
  part is read-only too. Upgrade if clinicians ask to fix their own earlier part after a colleague wrote below.
- **The writer must be an enrolled member, or NULL (unknown).** The save door normalises it with
  `cairn_vault_staff_id` and answers `no-such-writer` for anything else.
- **Draft tables keep 3a's posture:** `REVOKE ALL … FROM PUBLIC`, no table grant. Every door is `SECURITY DEFINER
  SET search_path = public, pg_temp`, revoked from PUBLIC and granted to `cairn_window` only. The directory door
  likewise.
- **A door's return shape is final** (3a's rule (c)). So 3a's `cairn_note_draft_save`, `cairn_note_drafts` and
  `cairn_note_draft_spend` are **removed** (db/065 in place, plus `DROP FUNCTION IF EXISTS` in db/066), and the
  part-aware doors take **new names**. Never `CREATE OR REPLACE` a same-named door with a new shape: 065's replay
  would then fail on every existing database.
- **One event per non-empty part, in one transaction, with no savepoint** (the 3a durable rule). Part 1 is the note.
  Part *k* > 1 carries `refs: [{"rel": "addendum-to", "event": <part k−1's event id>}]`. The spend records **part
  1's** event id (db/065's trigger is unchanged). HLC ticks run **inside** the transaction.
- **Contributors per part** (design §3.4, first bullet; checked against `cairn_authorship_bound` and
  `classify_authorship_confidence`, both of which grade it `Attested`):
  - the signer's own part → `[{signer, "authored"}, {node, "recorded"}]` (3a's `with_human_author`);
  - anyone else's part → `[{writer_kid, "drafted"}, {signer, "attested"}, {node, "recorded"}]`, signed by the
    signer. `writer_kid` is the writer's current key (`cairn_vault_member_key`);
  - an unknown writer's part → `[{signer, "attested"}, {node, "recorded"}]`.
- **No wire change.** `clinical.note/1`'s `refs` slot already exists and the floor already checks `addendum-to`
  (db/064). Nothing is added to the payload.
- **Bookmarks and recent people live in window memory only**: no file, no browser storage. A bookmark holds a chart
  header, i.e. a patient's name, which must not sit on disk in the clear. A window restart forgets them.
- **The `?` is text and is spoken**: a claimed name shows as `?Nurse A`, and a screen reader hears "unverified
  Nurse A". It is never a glyph alone and never colour alone.
- **House rules:** TDD; junior-legible docs on every non-trivial fn; files under 500 lines (`state.rs` is 751: add
  ONE field there, and put everything else in the new `whos_here.rs`); `scripts/check_closing_keywords.py` before
  every commit; DB-gated runs show no `skipped:` (`--nocapture`); a DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`; run
  `cairn-gui-tauri`'s DB-gated tests locally (#905: CI never does).
- **Pinned guards to update** (memory "pinned-count guards"): `cairn-node/src/db.rs` `SCHEMA` list + `cairn-sync`'s
  list if it carries db/065 (check) · `SCHEMA_GENERATION` **65 → 66** (`crates/cairn-event/src/schema_generation.rs:45`)
  · `floor_execute_grants.rs` · `window_role_grants_notes.rs` (every new door driven to its deep path under
  `cairn_window`, every removed door gone) · `db_errors_stay_legible.rs` (a new `LocalDbFault`-wrapped read) ·
  `commands_tests.rs`'s script list (+ `whos_here.js`, `idle.js`) · `signer_webview_tests.rs` · `tests/common` helper
  lists if a helper is added. **No new event type** in 3b-1, so the twin/projection counts do not move.

## Review Focus

1. **A switch mid-typing:** Nurse A has typed and not yet saved, then Dr B presses the hot key. Expected: A's text
   is saved under A before the chart leaves the screen, and nothing B types later lands in A's part. Pinned in
   Task 6 (the switch awaits the editor's save queue; the save carries `noteDraft.writer`).
2. **The old terminal keeps typing after a move:** A's window saves after B took the draft. Expected: `moved`, the
   unsaved text is kept as a new draft at A, and A sees one sentence saying where the note went. Pinned in Tasks 1
   and 6.
3. **The same person types again after a colleague wrote below:** Nurse A, then Dr B, then Nurse A. Expected: three
   parts (A, B, A), never A's new text merged into part 1. Pinned in Task 1.
4. **The re-lock fires while a Sign is in flight, or while the passphrase field has focus:** Expected: the re-lock
   waits until the sign settles, and a keystroke in any field counts as activity. A poll (`lock_state`, the waiting
   bar) never does (main.js:462's lesson). Pinned in Task 6.
5. **A refused second part rolls back the first** (#657's untested multi-event rollback). Expected: no note, the
   draft intact with all its parts. Pinned in Task 4.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the progress sheet in the bedside folder. Whoever picks up the pen writes below what is
  there; nobody rubs out a colleague's ink; you carry the sheet to wherever you sit down to finish it.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Reading a chart: 0 → 0 → 0. No name is asked; "just reading" or going straight to search is fine.
  - Writing and signing a note: 3 (pick up the sheet, write, sign) → 3 (say who's here once per stint, type, sign:
    the staff ID is then prefilled, so only the passphrase is typed) → 3.
  - Finishing a draft at another terminal: 1 (keep writing on the sheet you carried) + the walk → 1 (on the open
    chart, press **Continue here** on the draft) → 1.
  - Resuming after the re-lock: 2 + the walk back to the folder → 3 (shared unlock, pick yourself, press the
    pre-focused **Return to …**) → 3. The maintainer's ruling (Decision 3) is that this is not an architecture
    defect.
  - Adding to a colleague's draft: 1 (write below) → 1 (type below their part) → 1.
- **Time + cognitive load:** hot key to "named" ≤ 3 s with a type-ahead over 50 staff; re-lock to back at the
  chart ≤ 10 s; Continue here ≤ 1 s. One loud bar names who is writing, so a person who forgot to switch sees a name
  that is not theirs. Measurement is owed by a human in **RUNBOOK §19** (Task 7): this PR is the first runnable
  who's-here surface.

---

### Task 1: The part-aware draft store and the staff directory (`db/066`)

**Files:**
- Create: `db/066_note_draft_parts.sql`, `db/tests/066_note_draft_parts_test.sql`
- Modify: `db/065_note_draft.sql` (remove the three draft doors' definitions and their REVOKE/GRANT entries, plus a
  header line pointing at db/066); `db/tests/065_note_draft_test.sql` (cases that called the removed doors move to
  066's test or call the new ones); `crates/cairn-node/src/db.rs` (`SCHEMA` entry); `schema_generation.rs` (66)

**Interfaces:**
- Produces (all `SECURITY DEFINER`, `cairn_window` only, one advisory lock key `'note_draft:' || draft_id`, as 3a):
  - Tables `note_draft_part(draft_id uuid REFERENCES note_draft ON DELETE CASCADE, seq int, writer text NULL,
    narrative text NOT NULL, stated_time timestamptz, recording_started timestamptz NOT NULL, last_keystroke
    timestamptz NOT NULL, PRIMARY KEY (draft_id, seq))` and `note_draft_revision(draft_id uuid, seq int, rev bigint
    GENERATED ALWAYS AS IDENTITY, writer text, terminal_id text, narrative text, stated_time timestamptz, saved_at
    timestamptz)`. `note_draft` keeps `draft_id, patient_id, terminal_id, recording_started, last_keystroke` (the
    header: 3b-2's clock reads `last_keystroke` = the latest over its parts), and loses `narrative` and `stated_time`.
  - `cairn_note_draft_part_save(p_draft uuid, p_patient uuid, p_terminal text, p_writer text, p_narrative text,
    p_stated_time timestamptz) RETURNS text`. Answers: `saved` · `spent` · `moved` (same chart, another terminal) ·
    `elsewhere` (another chart; #958's split) · `no-chart` · `no-such-writer`.
  - `cairn_note_draft_parts(p_patient uuid) RETURNS TABLE(draft_id uuid, terminal_id text, seq int, writer text,
    writer_name text, narrative text, stated_time timestamptz, recording_started timestamptz, last_keystroke
    timestamptz)`, one row per part, ordered by draft `recording_started`, `draft_id`, `seq`. `writer_name` =
    `vault_member.display_name`, else the staff id, NULL for an unknown writer.
  - `cairn_note_draft_take(p_draft uuid, p_patient uuid, p_terminal text) RETURNS text`. Answers: `taken` · `spent`
    · `elsewhere` · `gone`. It sets `note_draft.terminal_id`; nothing else changes.
  - `cairn_note_draft_spend_parts(p_draft uuid, p_event uuid) RETURNS TABLE(patient_id uuid, seq int, writer text,
    narrative text, stated_time timestamptz, recording_started timestamptz, last_keystroke timestamptz)`. It returns
    the NON-empty parts (`btrim(narrative) <> ''`) in `seq` order, deletes the header (parts cascade) and the
    revisions, and inserts `note_draft_spent(draft_id, patient_id, p_event)`. Zero rows when the draft is absent or
    spent.
  - `cairn_whos_here_directory() RETURNS TABLE(staff_id text, display_name text, role text)`: members for whom
    `cairn_vault_purpose_allowed(role, false, 'clinical')`, ordered by `display_name`.

- [ ] **Step 1: Write the failing SQL test** `db/tests/066_note_draft_parts_test.sql`, in 065's test style, with
  these cases (each `RAISE`s on the wrong answer):
  - `first_save_makes_part_1`: one header and part 1 (writer A), with `recording_started = last_keystroke`.
  - `same_writer_rewrites_the_tail`: A saves twice, so still one part, holding the second text.
  - `another_writer_appends`: A, then B, gives parts 1 (A) and 2 (B). Part 1's text is unchanged.
  - `a_b_a_is_three_parts` (Review Focus 3).
  - `b_cannot_reach_part_1`: no argument combination changes part 1's text after B's save. Assert part 1 is
    byte-equal to A's last save.
  - `an_empty_save_by_a_new_writer_makes_no_part`.
  - `every_real_change_is_a_revision`: three changed saves and one unchanged save give 3 revision rows; an
    unchanged save moves neither `last_keystroke` (R1 kept) nor the revision count.
  - `an_unknown_staff_id_answers_no_such_writer` and `a_staff_id_in_other_case_is_normalised`.
  - `moved_vs_elsewhere`: a save from terminal T2 answers `moved`; one naming another patient answers `elsewhere`.
  - `take_moves_and_the_old_terminal_is_moved`: after a take by T2, T1's save answers `moved` and T2's answers
    `saved`.
  - `take_of_spent_is_spent`, `take_of_unknown_is_gone`, `take_naming_another_chart_is_elsewhere`.
  - `spend_returns_nonempty_parts_in_order_and_clears_revisions`.
  - `spend_still_needs_its_note_in_this_transaction`: 3a's committed-earlier case, re-run against the new door.
  - `migrated_3a_draft_is_part_1_unknown_writer`: insert a 3a-shaped row, run db/066's migration block, and expect
    part 1 with a NULL writer and the old text.
  - `directory_lists_clinical_roles_only`: a superuser and an admin are absent.
  - `the_3a_doors_are_gone`: `to_regprocedure('cairn_note_draft_save(uuid,uuid,text,text,timestamptz)') IS NULL`,
    and the same for the other two.
  - `no_table_grant`: `has_table_privilege('cairn_window', 'note_draft_part', 'SELECT')` is false, and the same for
    `note_draft_revision`.
- [ ] **Step 2: Run it and see it fail** (`scripts/run-db-gated-tests.sh` or `psql -f` against `cairn_test` with
  `PGHOST=127.0.0.1 PGPORT=5532`; see #862). Expected: FAIL, because `cairn_note_draft_part_save` does not exist.
- [ ] **Step 3: Write `db/066_note_draft_parts.sql`.** Order: create the tables. Then a guarded migration block: when
  `note_draft.narrative` still exists, copy every header without parts into part 1 (writer NULL), then
  `ALTER TABLE note_draft DROP COLUMN IF EXISTS narrative, DROP COLUMN IF EXISTS stated_time`. Then `DROP FUNCTION IF
  EXISTS` the three 3a signatures, the new doors, and REVOKE/GRANT. In the save door, take the tail with
  `ORDER BY seq DESC LIMIT 1 FOR UPDATE`. Header `last_keystroke` = `GREATEST(clock_timestamp(),
  recording_started)`, as 3a. Write a revision row only when the text or the stated time changed. Edit db/065 in place
  as listed under Files.
- [ ] **Step 4: Run 065's and 066's tests.** Expected: PASS. Run `cargo test -p cairn-node --test
  schema_generation` (or the pinned guard that checks the list's tail) and expect PASS.
- [ ] **Step 5: Commit** — `feat(note): a draft is a sequence of writers' parts; move and directory doors (db/066)`.

### Task 2: The attesting-signer rewrite and the addendum ref (`cairn-event`, pure)

**Files:**
- Modify: `crates/cairn-event/src/contributor.rs` (beside `with_human_author`), `crates/cairn-event/src/note.rs`
- Test: in-file tests in both

**Interfaces:**
- Produces:
  - `pub fn with_attesting_signer(body: EventBody, signer_kid: &str, writer_kid: Option<&str>) -> EventBody`. It
    prepends `{writer_kid, "drafted"}` (when known) and then `{signer_kid, "attested"}`, keeps the device
    `recorded` entry after them, and sets `signer_key_id = signer_kid`.
  - `NoteAssertion` gains `pub addendum_to: Option<&'a str>`. When it is `Some(id)`, `note_payload` writes
    `"refs": [{"rel": "addendum-to", "event": id}]`; when it is `None`, it writes `[]` exactly as before.

- [ ] **Step 1: Failing tests.**
  - `attesting_signer_shape`: `with_attesting_signer(device_body, "S", Some("W"))` gives contributors exactly
    `[{"actor_id":"W","role":"drafted"},{"actor_id":"S","role":"attested"},{"actor_id":"N","role":"recorded"}]` and
    `signer_key_id == "S"`.
  - `attesting_signer_unknown_writer` gives `[{S, attested}, {N, recorded}]`.
  - `attested_part_grades_attested`: `classify_authorship_confidence` of that set, with `signer` = `S`, is
    `Attested`. (`VerifiedKid` comes from a test-only verified event, or from `from_event_log_column` in a test, which
    is acceptable here.)
  - `addendum_ref_written` and `no_addendum_keeps_empty_refs`. Every existing 3a payload test still passes with
    `addendum_to: None`.
- [ ] **Step 2: Run** `cargo test -p cairn-event contributor note`. Expected: FAIL (unknown fn and field).
- [ ] **Step 3: Implement.** Update every `NoteAssertion { … }` literal in the tree (grep it) with
  `addendum_to: None`.
- [ ] **Step 4: Run** `cargo test -p cairn-event`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(note): the attesting signer of another's part, and the addendum ref`.

### Task 3: The node's part-aware draft calls (`cairn_node::note::draft`)

**Files:**
- Modify: `crates/cairn-node/src/note/draft.rs`, `crates/cairn-node/src/note/read.rs` (drafts now carry parts),
  `crates/cairn-node/src/db.rs` (`next_hlc` takes `&(impl GenericClient + Sync)`; every existing `&Client` caller
  still compiles, so check `cairn-sync` with the full workspace build, per memory "cross-crate call-site")
- Create: `crates/cairn-node/src/note/whos_here.rs` (`directory`)
- Test: `crates/cairn-node/tests/note_draft_parts.rs` (DB-gated)

**Interfaces:**
- Consumes: Task 1's doors.
- Produces:
  - `SaveOutcome` gains `Moved` and `NoSuchWriter`; `outcome_from_word` maps `"moved"` and `"no-such-writer"`.
  - `pub async fn save_part(client, draft: Uuid, patient: Uuid, terminal: &str, writer: Option<&str>, narrative:
    &str, stated: Option<DateTime<Utc>>) -> anyhow::Result<SaveOutcome>` replaces `save_draft`.
  - `pub struct DraftPart { pub seq: i32, pub writer: Option<String>, pub writer_name: Option<String>, pub
    narrative: String, pub stated_time: Option<DateTime<Utc>>, pub recording_started: DateTime<Utc>, pub
    last_keystroke: DateTime<Utc> }`. `NoteDraft` keeps `draft_id`, `terminal_id`, `recording_started` and
    `last_keystroke`, and replaces `narrative`/`stated_time` with `pub parts: Vec<DraftPart>`. `open_drafts` groups
    the door's rows.
  - `pub enum TakeOutcome { Taken, Spent, Elsewhere, Gone }` and `pub async fn take_draft(client, draft, patient,
    terminal) -> anyhow::Result<TakeOutcome>`. An unknown word is an error (3a's rule).
  - `pub(crate) async fn spend_parts(client, draft, event) -> anyhow::Result<Vec<SpentPart>>` (empty = absent or
    spent), with `SpentPart { patient_id, seq, writer, narrative, stated_time, recording_started, last_keystroke }`.
  - `whos_here::directory(client) -> anyhow::Result<Vec<StaffEntry { staff_id, display_name, role }>>`.

- [ ] **Step 1: Failing DB-gated tests** in `note_draft_parts.rs`:
  - `two_writers_read_back_as_two_parts`;
  - `a_take_then_the_old_terminal_saves_moved`;
  - `the_directory_omits_superusers`;
  - `every_door_word_maps_and_an_unknown_one_is_an_error`: an in-file unit test extended for `Moved` and
    `NoSuchWriter`, and `TakeOutcome`'s four words plus one unknown word.
  All of them run under the `cairn_window` role, as `note_read.rs` does.
- [ ] **Step 2: Run** with `--nocapture`. Expected: FAIL to compile (`save_part` missing).
- [ ] **Step 3: Implement.** Update the callers of `save_draft` and `open_drafts` (`sign.rs`, the window's
  `notes.rs`; the window's are finished in Task 5, so stub them only enough to compile).
- [ ] **Step 4: Run** the test and `cargo test -p cairn-node note`. Expected: PASS, with no `skipped:` line.
- [ ] **Step 5: Commit** — `feat(note): save a writer's part, move a draft, read the staff directory`.

### Task 4: Sign a draft as one event per part (`cairn_node::note::sign`, `read`)

**Files:**
- Modify: `crates/cairn-node/src/note/sign.rs`, `crates/cairn-node/src/note/read.rs`, `crates/cairn-node/src/note/view.rs`
- Test: `crates/cairn-node/tests/note_sign.rs` (extend)

**Interfaces:**
- Consumes: Task 2's `with_attesting_signer` and `addendum_to`; Task 3's `save_part` and `spend_parts`.
- Produces:
  - `SignNoteInput` gains `pub writer: Option<&'a str>` (the on-screen part's writer) and `pub signer_staff_id: &'a
    str` (whose credential signs; used to tell "own part" from "another's").
  - `SignRefusal` gains `Moved` and `NoSuchWriter`.
  - `sign_note` returns `Committed<Vec<Uuid>>` (the parts' event ids in order; `event_id` = part 1's).
  - `NoteRow` gains `pub addendum_to: Option<Uuid>` (from `clinical_note.refs`) and `pub drafted_by: Option<String>`
    (the `drafted` contributor's name, through `cairn_note_signer_name`).
  - `view::byline_words(grade: &NoteGrade, drafted_by: Option<&str>) -> String` gives `"written by ?Nurse A · signed by
    Dr B"` when a `drafted` name exists; otherwise it gives `grade_words` exactly as 3a.
- Algorithm (the order is the decision): save the on-screen part, then `spend_parts(draft, first_id)`. For each part
  *i*, `next_hlc(&tx, …)` → body (part *i*'s times, `addendum_to` = part *i−1*'s id) → contributor rewrite (own:
  `prepare_sealed` with the author, as 3a; another's or unknown: `with_attesting_signer`, signed by the signer; check
  how `prepare_sealed` applies `AuthorParams`, and add a sibling that takes the already-rewritten body rather than
  calling `with_human_author` twice) → `submit_event`. Then commit. "Own part" means `writer` equals
  `cairn_vault_staff_id(signer_staff_id)`; normalise both in Rust with the same rule, or ask the door.

- [ ] **Step 1: Failing tests** in `note_sign.rs`:
  - `a_one_writer_draft_signs_as_the_signers_own_authored_note` (3a's case, unchanged in outcome);
  - `a_two_writer_draft_signs_as_two_events_chained_by_addendum_to`: part 2's `refs[0].event` is part 1's id; part 1
    carries `{A, drafted}` and `{B, attested}` when B signs, and part 2 carries `{B, authored}`. Both grade
    `Attested`;
  - `an_unknown_writers_part_is_attested_by_the_signer`;
  - `a_refused_second_part_rolls_back_the_first` (Review Focus 5): force part 2's submit to fail (a fault-injection
    seam per memory "SET ROLE fault-injection", or a malformed stated time that the floor refuses), and expect zero
    notes and the draft with both parts intact;
  - `empty_parts_are_skipped_and_an_all_empty_draft_is_refused_empty`;
  - `a_moved_draft_is_refused_moved_and_writes_nothing`.
  Also a unit test of `byline_words`.
- [ ] **Step 2: Run** with `--nocapture`. Expected: FAIL.
- [ ] **Step 3: Implement** per the Interfaces block. Keep `sign.rs` under 500 lines: move the pure body builders to
  `note/body.rs` if needed.
- [ ] **Step 4: Run** `note_sign`, `note_read`, `note_floor` and `db_errors_stay_legible`. Expected: PASS.
- [ ] **Step 5: Commit** — `feat(note): a draft signs as one event per writer's part`.

### Task 5: Who's here, the idle re-lock and part-aware note commands (window Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/whos_here.rs`, `src/whos_here_tests.rs`
- Modify: `src/state.rs` (ONE field: `pub whos_here: tokio::sync::Mutex<crate::whos_here::Here>`), `src/main.rs`
  (register the commands; the `--idle-lock-minutes` flag), `src/commands.rs` (`LockState` gains `whos_here` and
  `idle_lock_secs`), `src/notes.rs` (+ `src/notes_tests.rs`)

**Interfaces:**
- Produces:
  - `pub enum Claim { NotAsked, Unknown, Named { staff_id: String, display_name: String } }`. `pub struct Here {
    pub claim: Claim, recent: Vec<(String, String)> /* ≤ 8, newest first */, bookmarks: HashMap<String,
    cairn_patient_search::Candidate> }`. Pure methods: `pick(&mut self, claim: Claim)`, `forget_claim(&mut self)`
    (sets `NotAsked`, keeps recent and bookmarks), `remember_chart(&mut self, c: &Candidate)` (only when
    `Named`), `offer(&self) -> Option<&Candidate>`.
  - `pub fn claimed_name(name: Option<&str>) -> ClaimedName { shown: String, spoken: String }`:
    `Some("Nurse A")` → `("?Nurse A", "unverified Nurse A")`; `None` → `("writer not known", "writer not known")`.
  - Commands: `whos_here_directory() -> Vec<StaffEntryView>`; `whos_here_pick(staff_id: Option<String>) ->
    PickView { claim, offer: Option<ChartHeaderView> }`. The pick forgets the held key, and an offered candidate is
    inserted into `AppState::shown` because the offer IS a list on screen, so `open_chart`'s rule holds. Also
    `relock()`, which forgets the held key and the claim. `None` = "Don't know" (`Unknown`).
  - `open_chart_impl` calls `remember_chart` on success.
  - `note_draft_save(…, writer: Option<String>)` and `note_sign(…, writer: Option<String>)`: the writer is passed
    from the webview's `noteDraft.writer`, never read from `whos_here` here (Global Constraints). `note_draft_take(
    patient_id, draft_id) -> TakeView`. `SaveView` gains `moved: bool`. `DraftView` replaces
    `narrative`/`stated_time` with `parts: Vec<PartView { writer_shown, writer_spoken, text, stated_time, words,
    editable }>` (`editable` only for the tail part, when it is the CURRENT claim's and the draft is `here`) and keeps
    `here`.
  - `--idle-lock-minutes <1..=240>`, default 15 (design §3.3), with clap parser `parse_idle_lock_minutes`. The figure
    is separate from `IDLE_TIMEOUT`; never reuse that constant.
- Sentences (exact): moved → `"This note is now being continued at another terminal. What you typed since the last
  save is kept here as a new draft."` · no-such-writer → `"the name at this terminal is not an enrolled member —
  say who's here again"` · take elsewhere/gone/spent → reuse 3a's `ELSEWHERE`/`SPENT` wording, plus `"this draft is
  no longer open"`.

- [ ] **Step 1: Failing tests** in `whos_here_tests.rs` and `notes_tests.rs`:
  - `pick_named_then_forget_keeps_bookmarks`; `remember_chart_ignored_unless_named`; `recent_is_capped_at_8_and_deduped`;
  - `claimed_name_words`;
  - `pick_forgets_the_held_key` (against `AppState::mock`, as `superuser_panel_tests.rs` checks the wipe);
  - `relock_forgets_claim_and_key`;
  - `an_offered_chart_becomes_openable` (`open_chart_impl` succeeds after the pick's offer);
  - `idle_lock_minutes_bounds` (0 and 241 refused, 15 accepted);
  - `save_passes_the_given_writer_not_the_claim`, using mock state: pick B, save with writer A → the door call names
    A (inspect the mock port, or test the arg builder as a pure fn);
  - `only_the_current_writers_tail_is_editable`; `moved_sentence_exact`.
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test` in `cairn-gui/` (its own workspace). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the cairn-gui gate (memory "three Cargo trees": test, clippy `--locked`, `cargo doc` with
  `RUSTDOCFLAGS=-D warnings`), and run its DB-gated tests locally (#905). Expected: PASS.
- [ ] **Step 5: Commit** — `feat(window): who's here, the idle re-lock, and part-aware note commands`.

### Task 6: The webview: picker, loud bar, re-lock, parts and Continue here

**Files:**
- Create: `src-ui/whos_here.js`, `src-ui/idle.js`, `src/whos_here_webview_tests.rs`
- Modify: `src-ui/index.html`, `src-ui/style.css`, `src-ui/notes.js`, `src-ui/signer.js` (the prefill; the re-lock
  shows `#terminal-lock` again), `src-ui/main.js` (load order), `src/commands_tests.rs` (script list),
  `src/notes_webview_tests.rs`

**Interfaces:**
- Consumes: Task 5's commands.
- Produces (JS globals other scripts call): `whosHere()` → the current claim view; `askWhosHere(then)`, which opens
  the picker and runs `then` once a claim is made; `relockTerminal()`.
- Behaviour that the tests pin (layout is the builder's; the sketch `switch-user.html` is not a spec):
  - Hot key **Ctrl+Shift+H** (Cmd+Shift+H on macOS), and a visible **"Who's here?"** button in the bar. Both run
    `closeChart()` and **await its save queue** before calling `whos_here_pick`, so Review Focus 1 holds. The picker
    is a full-screen region, not a dialog box: a type-ahead over the directory by name or staff ID, the recent
    people first, and **"Don't know / just reading"**. No passphrase field.
  - The bar is full-width at the top, with the name in large text and an `aria-live="polite"` announcement on
    change. It shows `?`-less text for the claim itself ("Writing as Nurse A"), since the claim is the user's own
    statement on this screen; the `?` is for what OTHERS read later.
  - After a pick: an offer → one pre-focused button **"Return to SURNAME, Given — born …"**, which opens it through
    `open_chart`; Esc or search dismisses it.
  - The signer strip's `#staff-id` is set to the claim's staff ID on every pick, and cleared on "Don't know" and on
    the re-lock. The passphrase field never is set.
  - **New note** while the claim is `NotAsked` → `askWhosHere(newNote)`. `noteDraft.writer` is set once, when the
    editor opens.
  - Parts render as read-only blocks above the editor (`cell`, never `innerHTML`, which is 3a's durable rule), each
    headed by `writer_shown` with `writer_spoken` as its accessible name. The textarea holds only the editable tail,
    or is empty for a new part.
  - A draft that is not `here` shows **"Continue here"** → `note_draft_take` → reopen the draft in the editor.
  - A save answering `moved` → mint a new draft id, keep the editor's text and time in it, show the moved sentence,
    and save it as the new draft (only when there was unsaved text; otherwise close the editor with the sentence).
  - `idle.js`: `keydown`, `pointerdown`, `input` and `wheel` reset the timer; nothing else does. At
    `idle_lock_secs`, if a Sign is in flight (`setSigning(true)`), wait for it to settle; then `closeChart()`, await
    saves, `relock()`, and show `#terminal-lock`.

- [ ] **Step 1: Failing webview tests** (the source-text style of `notes_webview_tests.rs`/`signer_webview_tests.rs`;
  memory "webview mock walk" for a live walk):
  - `switch_awaits_the_save_queue_before_picking`;
  - `the_writer_is_set_once_at_editor_open`;
  - `idle_resets_only_on_user_input` (no `setInterval` or poll callback calls the reset);
  - `relock_waits_for_a_sign_in_flight`;
  - `moved_forks_a_new_draft`;
  - `parts_drawn_through_cell`;
  - `staff_id_prefilled_passphrase_never`;
  - `the_bar_is_aria_live`.
  Add both scripts to the script list.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the tests, then do a headless mock walk (memory recipe) of: pick → offer → New note → type →
  switch → second writer → Continue here from a second mock terminal id. Assert computed VISIBILITY, not just text.
  Expected: PASS.
- [ ] **Step 5: Commit** — `feat(window): the who's-here picker and bar, the idle re-lock, a draft's parts`.

### Task 7: ADR, runbook, durable rules, tracking documents, issues

**Files:**
- Create: `docs/spec/decisions/0088-whos-here-and-a-drafts-parts.md`
- Modify: `docs/spec/index.md` (v0.89 → **v0.90**, ADR index), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`
  (new **§19**, plus a `TEMPLATE.md` section), `docs/developers/durable-rules.md` (a **sign-in slice 3b-1** block),
  `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: ADR-0088** records: who's here is a claim, the writer captured at editor open, the parts rule and
  its tail-only shortcut, the revision rows, the move (superseding the design's rule I1, Decision 2) and its
  partition limit, the per-part commit shape and contributors (no wire change), the bookmark offer (Decision 3), the
  in-memory-only bookmarks, the confirmation of ADR-0087 decision 9 (Decision 4), and the 3a doors' removal. Under
  "what is not guaranteed": a misclaim writes under another's name (asserted, findable in revisions); a window
  restart forgets bookmarks; the re-lock is a window posture, not a node one; the revision rows cost one full-text
  copy per real save for the life of the draft. Fact-check every sentence against the SQL before the PR (memory "SDD
  evidence + ADR fact-check").
- [ ] **Step 2: RUNBOOK §19** (human-owed): hot key to named ≤ 3 s (50 staff); re-lock to back at the chart ≤ 10 s;
  Continue here ≤ 1 s; switch mid-typing loses nothing; VoiceOver reads the bar on change and "unverified" before a
  claimed name; keyboard-only pick.
- [ ] **Step 3: Durable-rules block:** tail-only writes; the writer travels with the save; a door's shape is final
  (new names, never a reshaped `CREATE OR REPLACE`); one event per part in one transaction with HLC ticks inside;
  bookmarks never on disk. Each with its pin and its tempting wrong fix.
- [ ] **Step 4: File issues** (run `scripts/check_closing_keywords.py` on every body): a writer's own earlier part is
  read-only (the shortcut); the revision rows' size. Already filed at planning (2026-11-04), so cite them in the ADR:
  **#963** (slice 5 carries drafts to the other terminal-nodes, partitioned case included), **#964** ("unknown is
  allowed" as a deployment policy, §10 finding 5), **#965** (draft visibility gating once §5.9 sealed episodes reach
  notes, §10 finding 4). §10 finding 3 (record the terminal of an unattributed commit) is 3b-2's.
- [ ] **Step 5: HANDOVER + ROADMAP**: 3b-1 built, 3b-2 next; prune.
- [ ] **Step 6: Full gate in CI's order after the last edit** (memory "a gate is evidence only for the tree it ran
  against"), then push and open the PR. #958 asks for exactly the door split, one
  variant on each enum and one sentence each, all of which Tasks 1, 3, 4 and 5 build, so the PR body closes it.
