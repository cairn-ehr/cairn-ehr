# GUI sign-in, slice 3b-2a — the quarantine commit and the `Asserted` grade — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A draft nobody has typed into for the site's quarantine time (default 3 h) goes into the record **by
itself, unsigned**: signed by the node, one event per part, its writer named as `drafted` (or no writer at all when
not known), and it reads **"written by ?Nurse A · went in unsigned"**, a new `Asserted` grade, never "signed".

**Architecture:** `db/067` adds the site's quarantine policy (one console verb sets it) and two window doors: one
lists overdue drafts, the other commits one. The commit door decides "overdue" on the database's own clock and
consumes the draft in the same statement, so a draft can never go in twice and never early. `db/065` gains one
column, `note_draft_spent.by_timer`, so `db/066`'s save and take doors can answer `committed` rather than `spent`
for a timer's commit (both edited in place, same return shape). `cairn-event` gains the `Asserted` grade and the
drafting-writer rewrite. `cairn_node::note::commit` builds and submits the node-signed parts. The window commits
overdue drafts on a 60 s background tick, and on every chart read before reading.

**Tech Stack:** PostgreSQL 18 (new `db/067`; `db/065` and `db/066` edited in place), Rust (`cairn-event`,
`cairn-node`, `cairn-gui-tauri`), plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
§3.3 (the whole section), §3.7 (the grade words), §4.1, §7 (the runner is the window). Read
[ADR-0088](../../spec/decisions/0088-whos-here-and-a-drafts-parts.md),
[ADR-0087](../../spec/decisions/0087-the-first-note-stream-and-its-draft.md) and `docs/developers/durable-rules.md`'s
**sign-in slice 3b-1** and **3a** blocks before Task 1. **This plan covers PR 3b-2a only.** 3b-2b (the audited
discard) gets its own plan.

## Decisions taken with the maintainer on 2026-11-04 (not in the design; the ADR records them)

1. **3b-2 is split into two PRs**: 3b-2a (this plan: clock, knob, commit door, runner, `Asserted`), then 3b-2b (the
   audited discard, a new event type).
2. **(3b-2b's, recorded here so it is not lost)** A draft may be discarded only when every non-blank part is the
   discarder's own (by key) or by a writer not known (ADR-0088 decision 18). A draft with **two or more named
   writers** is refused and goes in by the timer. A co-signed discard is **#980**, not built.
3. **The terminal of an unattributed commit is NOT recorded** (design §10 finding 3). `clinical.note/1` has no slot,
   and a `/2` profile would hide the note from older nodes (ADR-0087 ruling R8). Deferred: **#979**.

## Global Constraints

- **The window never sends a time** (design §3.3). The save door already stamps `last_keystroke` from the
  database clock; the commit door compares `last_keystroke + quarantine` against `clock_timestamp()` in the same
  database. No terminal clock enters any decision. The quarantine is operational only: it never touches the stated
  time, `t_effective` or `t_recorded`.
- **"Overdue" is decided in ONE door, under the draft's advisory lock, in the same statement that consumes it**
  (design §3.3, review finding). Lock order as ADR-0088's "Harder" says: advisory lock `'note_draft:' || draft_id`,
  then the header row, then its parts.
- **A door's return shape is final** (3a rule (c)). The save and take doors keep their shapes and gain one answer
  word, `committed`, in place in `db/066`. Every new door is a new name.
- **A column a door body reads lives in the file that creates its table**: `by_timer` goes into `db/065` (with a
  paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, #207), never into `db/067`. PL/pgSQL binds a column only when the
  body runs, so a `db/066` body reading a column that only `db/067` adds would raise on the first upgrade until 067
  committed (slice 2's "a door's helpers live in the door's own file" rule).
- **The commit's contributors** (design §3.3, checked against `classify_authorship_confidence`):
  - a named writer's part → `[{writer_kid, "drafted"}, {node, "recorded"}]`, signed by the **node** key;
  - a part by a writer not known → `[{node, "recorded"}]`, which is `note_body`'s default (sign.rs);
  - `writer_kid` = `cairn_vault_member_key(writer)`. A named writer with **no** key (`NULL`) makes the draft
    **stuck**: the transaction rolls back, the draft stays, it is shown overdue, and it is **never** committed as
    "not known" (principle 4, mirroring `SignRefusal::NoSuchWriter`).
- **One event per non-blank part, in one transaction, with no savepoint, HLC ticks inside** (ADR-0088 decision 14).
  The chain is the same: the first non-blank part is the note, each later one `addendum-to` the one before it. The
  spend record names the first non-blank part's event id, so `db/065`'s commit-time trigger is unchanged.
- **An empty draft never commits** (design §3.5): an overdue draft whose every part is blank is deleted with its
  revisions, and writes no note and no spend record.
- **`Asserted` is the read-side grade of "no bearing contributor, a named HUMAN contributory one"** (design §4.1).
  "Human" is a registry fact passed in as resolved key ids, so the classifier stays pure. The enum stays exhaustive:
  the new variant must break every consumer's build (`contributor.rs`'s doc gives the argument).
- **Wire: no change.** No new event type, no new payload field; `drafted` is already a ratified contributory role.
- **Policy storage:** one row in `note_policy`, like `vault_policy` (db/061): not in any backup (backup is an
  allowlist; do not add it), so a restore comes back with the default 3 h. Range 1 to 43 200 minutes (30 days);
  no row or NULL = **180**. Set only at the console (no grant to any runtime role).
- **The runner is the window** (design §7): a 60 s background tick (a constant, `COMMIT_TICK`) plus a commit of the
  chart's overdue drafts before every chart read. It needs the node key (`AppState::node_sk`) and a connection. In
  fixture mode, or with no node key, it does nothing. A failed commit never blocks the read.
- **House rules:** TDD; junior-legible docs on every non-trivial fn; files under 500 lines (`state.rs` is 754 and
  `cairn-node/src/main.rs` is huge: add no logic there beyond the verb's arm and the spawn line); run
  `scripts/check_closing_keywords.py` before every commit; DB-gated runs must show no `skipped:` (`--nocapture`); a
  DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`; run `cairn-gui-tauri`'s DB-gated tests locally (#905: CI never does).
- **Pinned guards to update** (memory "pinned-count guards"): `cairn-node/src/db.rs` `SCHEMA` list (+ db/067) ·
  `SCHEMA_GENERATION` **66 → 67** (`crates/cairn-event/src/schema_generation.rs:45`) · `floor_execute_grants.rs` ·
  `window_role_grants_notes.rs` (each new window door driven to its deep path under `cairn_window`; the console door
  refused there) · `db_errors_stay_legible.rs` if a new `LocalDbFault`-wrapped read is added · `tests/common/mod.rs`
  line 1279's `TRUNCATE` list gains `note_policy` · the `tests/common` helper array if a helper is added. **No new
  event type**, so the twin and projection counts do not move.

## Review Focus

1. **Someone types into a draft the timer has just committed**, because the window stayed open on it (the quarantine
   is shorter than `--idle-lock-minutes`, or the clinician kept working elsewhere on the chart). Expected: the save
   answers `committed`. The unsaved text is kept as a new draft, with its own sentence ("…went into the record
   unsigned…"), **even when `mayBeSigned` is set**: a timer's commit is never this window's Sign. Pinned in Tasks 1
   and 4.
2. **Two windows tick at once, or a Sign races the timer.** Expected: exactly one set of notes. The loser commits
   nothing, and a Sign that loses reads `Committed`, never a second note. Pinned in Task 3 by a **two-connection**
   race test (#977 notes there is none for the advisory lock yet).
3. **The site shortens the quarantine while drafts wait.** Expected: the next tick commits every draft now past the
   new limit; nothing is grandfathered. Pinned in Task 1.
4. **A draft whose named writer has no vault key any more.** Expected: stuck, not committed as "not known". It is
   shown overdue and reported once per tick on stderr, and the other drafts still commit. Pinned in Task 3.
5. **The pre-read commit fails** (no node key, a refused submit, a lost connection). Expected: the chart still reads,
   and the draft shows as overdue. Pinned in Task 4.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** an entry left unsigned on the progress sheet. It stays on the sheet, in the folder, readable
  by anyone who opens it, never removed, plainly not signed. Nobody acts to put it there: it is simply in the record.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Walking away from a draft: 0 → 0 → 0. The timer commits it; no act is asked of anyone.
  - Reading a note that went in unsigned: 0 → 0 → 0. It reads "written by ?Nurse A · went in unsigned" in words.
  - Setting the site's quarantine: not a clinical act (the owner, at the console, once).
  - Finishing it later is 3c's addendum (design §6: 3 → 3 → 3).
- **Time + cognitive load:** an overdue draft goes in within one tick (≤ 60 s past due) while any window runs; a chart
  open that first commits one overdue draft stays within the single-chart open budget + 200 ms; the byline says
  "went in unsigned" in text and speech, never a glyph or a colour alone. Measurement is owed by a human in
  **RUNBOOK §20** (Task 5): this PR is the first runnable auto-commit surface.
- **Not an `M > N` case, but a delay worth naming:** paper's unsigned entry is in the folder at once; Cairn's is in
  the record only after the quarantine, and a peer site sees it only then (design §3.3, last bullet).

---

### Task 1: The quarantine policy, the `committed` answer, and the commit door (`db/067`)

**Files:**
- Create: `db/067_note_draft_commit.sql`, `db/tests/067_note_draft_commit_test.sql`
- Modify: `db/065_note_draft.sql` (`note_draft_spent.by_timer boolean NOT NULL DEFAULT false`, inline + paired
  `ALTER … ADD COLUMN IF NOT EXISTS`); `db/066_note_draft_parts.sql` (the save and take doors answer `committed`
  for a `by_timer` spend: the header comments' answer lists too); `crates/cairn-node/src/db.rs` (`SCHEMA` entry);
  `crates/cairn-event/src/schema_generation.rs` (**67**); `crates/cairn-node/tests/common/mod.rs` (TRUNCATE list)

**Interfaces:**
- Produces:
  - Table `note_policy(singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton), quarantine_minutes int CHECK
    (quarantine_minutes BETWEEN 1 AND 43200))`, `REVOKE ALL … FROM PUBLIC, cairn_agent, cairn_node` (db/061's posture).
  - `cairn_note_set_quarantine(p_minutes int) RETURNS void`: definer; a value outside 1..43200 raises; NULL stores
    NULL (back to the default). **No grant** (console only).
  - `cairn_note_quarantine_minutes() RETURNS int`: definer, STABLE, `COALESCE(stored, 180)`; to `cairn_window`.
  - `cairn_note_drafts_overdue(p_patient uuid) RETURNS TABLE(draft_id uuid)`: drafts whose header
    `last_keystroke + quarantine minutes <= clock_timestamp()`, oldest `last_keystroke` first; NULL patient = every
    chart. Definer, to `cairn_window`. It takes no lock: the commit door re-decides.
  - `cairn_note_draft_commit_overdue(p_draft uuid, p_event uuid) RETURNS TABLE(patient_id uuid, seq int, writer
    text, narrative text, stated_time timestamptz, recording_started timestamptz, last_keystroke timestamptz)`, the
    **same columns as `cairn_note_draft_spend_parts`**. Definer, to `cairn_window`. Under the draft's advisory lock
    and `FOR UPDATE` on the header: absent, spent or **not yet overdue** → zero rows, nothing written; overdue with
    every part blank → delete the revisions and the header, zero rows, **no** spend record; otherwise return the
    non-blank parts in `seq` order, delete revisions and header, insert `note_draft_spent(draft_id, patient_id,
    event_id, by_timer) VALUES (…, p_event, true)`. The `db/065` trigger then insists on the note in this transaction.
  - Save door (`cairn_note_draft_part_save`) and take door (`cairn_note_draft_take`): a spent draft answers
    `committed` when its `note_draft_spent.by_timer`, else `spent` as before.

- [ ] **Step 1: Write the failing SQL test** `db/tests/067_note_draft_commit_test.sql` (065/066's style; each case
  `RAISE`s on the wrong answer; `\ir _scratch_database_guard.sql` first). Make a draft overdue by setting
  its header's `last_keystroke` back as the owner (the test runs as the owner), never by sleeping:
  - `default_quarantine_is_180`: no row → 180; `cairn_note_set_quarantine(60)` → 60; `(NULL)` → 180.
  - `set_quarantine_bounds`: 0 and 43201 raise; 1 and 43200 store.
  - `not_overdue_commits_nothing`: a draft typed 179 min ago (policy 180): zero rows, the draft unchanged.
  - `overdue_returns_parts_and_spends_by_timer`: a two-writer draft 181 min idle: two rows in `seq` order, header
    and revisions gone, one `note_draft_spent` row with `by_timer = true`. (Insert a matching
    `clinical.note.asserted` in the same transaction using 066's test helper, or assert the trigger's refusal at
    COMMIT without one, as 065's test does.)
  - `an_all_blank_overdue_draft_is_deleted_without_a_spend`: zero rows, no header, no revision, no spent row.
  - `a_spent_draft_commits_nothing`: zero rows.
  - `a_shorter_policy_applies_to_waiting_drafts` (Review Focus 3): a draft 90 min idle is not overdue at 180, is
    listed by `cairn_note_drafts_overdue(NULL)` and committed after `cairn_note_set_quarantine(60)`.
  - `overdue_list_filters_by_chart`: two charts, each with an overdue draft: `(chart A)` lists only A's draft,
    `(NULL)` lists both.
  - `save_after_a_timer_commit_answers_committed`, `take_after_a_timer_commit_answers_committed`, and
    `save_after_a_sign_still_answers_spent`.
  - `the_console_door_has_no_window_grant`: `has_function_privilege('cairn_window',
    'cairn_note_set_quarantine(int)', 'EXECUTE')` is false; the other three are true; `has_table_privilege
    ('cairn_window', 'note_policy', 'SELECT')` is false.
- [ ] **Step 2: Run it and see it fail** (`psql -f` against `cairn_test` with `PGHOST=127.0.0.1 PGPORT=5532`, see
  #862). Expected: FAIL, because `cairn_note_set_quarantine` does not exist.
- [ ] **Step 3: Write `db/067_note_draft_commit.sql`** and the in-place edits listed under Files. Header comment says
  why the clock is the database's (design §3.3) and why the door, not the caller, decides "overdue". The overdue
  test is `d.last_keystroke + make_interval(mins => cairn_note_quarantine_minutes()) <= clock_timestamp()`. The
  blank test is `cairn_twin_is_present`, never a regex (ADR-0088 decision 10).
- [ ] **Step 4: Run 065's, 066's and 067's SQL tests**, then `cargo test -p cairn-node --test schema_generation`
  (or whichever guard pins the `SCHEMA` list's tail). Expected: PASS.
- [ ] **Step 5: Commit**: `feat(note): the quarantine policy and an atomic overdue-commit door (db/067)`.

### Task 2: The `Asserted` grade and the drafting-writer rewrite (`cairn-event`, pure; its one display arm)

**Files:**
- Modify: `crates/cairn-event/src/contributor.rs`; `crates/cairn-node/src/note/view.rs` (the one exhaustive consumer)
- Test: `contributor.rs`'s and `view.rs`'s `mod tests`

**Interfaces:**
- Produces:
  - `AuthorshipConfidence::Asserted`, documented: no responsibility-bearing contributor, and a contributory one whose
    actor is a known human. Rendered "went in unsigned", never "signed", and upgradable by a later vouch (3c).
  - `pub fn classify_authorship_with_humans(contributors: &serde_json::Value, signer: VerifiedKid<'_>,
    verified_attester: Option<VerifiedKid<'_>>, human_kids: &[&str]) -> AuthorshipConfidence`: as today, except that
    where today's answer is `Device`, a `Contributory` entry whose `actor_id` is in `human_kids` makes it
    `Asserted`. `classify_authorship_confidence` keeps its signature and calls it with `&[]`, so no other caller
    moves.
  - `pub fn with_drafting_writer(body: EventBody, writer_kid: Option<&str>) -> EventBody`: prepends `{writer_kid,
    "drafted"}` when `Some`, leaves `signer_key_id` (the node) untouched, and returns the body unchanged for `None`.
  - `NoteGrade::WentInUnsigned` (view.rs), produced by `note_grade(Asserted, _)`; `grade_words` → `"went in
    unsigned"`; `byline_words` → shown `"written by ?Nurse A · went in unsigned"`, spoken `"written by unverified
    Nurse A · went in unsigned"`.

- [ ] **Step 1: Write the failing tests.** In `contributor.rs`: `a_drafted_human_with_no_bearer_is_asserted`
  (`[{H,"drafted"},{N,"recorded"}]`, signer N, humans `[H]` → `Asserted`); `a_drafted_non_human_stays_device` (humans
  `[]` → `Device`); `recorded_is_never_the_human_role` (`[{N,"recorded"}]`, humans `[N]` → `Device`: the rule
  counts every `Contributory` role **except** `recorded`, the capture device's own); `a_bearer_wins_over_asserted`
  (`[{H,"drafted"},{S,"attested"}]`, signer S → `Attested`); `the_old_classifier_never_says_asserted`;
  `with_drafting_writer_shapes` (`Some` prepends exactly one entry, signer unchanged; `None` returns the body
  byte-for-byte). In `view.rs`: extend `every_authorship_grade_has_its_words` and `every_grade_and_writer_reads_true`
  with `WentInUnsigned` beside `Drafted("Nurse A")` (shown and spoken, exact strings above), beside
  `DraftedUnnamed` (`"written by someone not named here · went in unsigned"`) and beside `TheSigner`
  (`"went in unsigned"`).
- [ ] **Step 2: Run them and see them fail** (`cargo test -p cairn-event contributor`, `CAIRN_ALLOW_DB_SKIP=1 cargo
  test -p cairn-node --lib note::view`). Expected: compile errors for the missing names.
- [ ] **Step 3: Implement** in the two files. `recorded` is excluded from "human contributory" because it is the
  recording device's role (ADR-0051), so a node key that also sits in `human_kids` can never grade its own capture
  `Asserted`. Update `AuthorshipConfidence`'s doc (the lockstep paragraph names `Asserted` ↔ `'unverified'`) and
  `classify_authorship_confidence`'s.
- [ ] **Step 4: Run the tests and the property suite** (`cargo test -p cairn-event`). Expected: PASS; the property
  suite and `verified_kid.rs` are unchanged.
- [ ] **Step 5: Commit**: `feat(note): the Asserted grade — a named human writer, no one signed`.

### Task 3: Commit overdue drafts on the node (`cairn_node::note::commit`), the outcomes, the verb, the grade on read

**Files:**
- Create: `crates/cairn-node/src/note/commit.rs`; `crates/cairn-node/tests/note_commit.rs`;
  `crates/cairn-node/tests/cli_note_quarantine.rs`
- Modify: `note/mod.rs` (`pub mod commit;`); `note/sign.rs` (`body_from_spent` becomes `pub(super)`;
  `SignRefusal::Committed` and its `refusal_for` arm); `note/draft.rs` (`SaveOutcome::Committed`,
  `TakeOutcome::Committed`, their word maps; `NoteDraft` gains `due: DateTime<Utc>` and `overdue: bool`);
  `note/read.rs` (NOTES_SQL gains the drafted key; the read grades with `classify_authorship_with_humans`);
  `crates/cairn-node/src/main.rs` (the `note-set-quarantine` verb); `crates/cairn-node/tests/authority_lockstep.rs`
  (a fourth fixture); `crates/cairn-node/tests/window_role_grants_notes.rs`; `crates/cairn-node/tests/floor_execute_grants.rs`;
  `cairn-gui/cairn-gui-tauri/src/notes_words.rs` + `notes_tests.rs` (the new variants' sentences: the GUI tree
  must build at the end of this task)

**Interfaces:**
- Consumes: Task 1's doors; Task 2's `with_drafting_writer`, `classify_authorship_with_humans`.
- Produces:
  - `pub fn draft_due(last_keystroke: DateTime<Utc>, quarantine_minutes: i32) -> DateTime<Utc>` (pure).
  - `pub enum CommitOutcome { Committed(Committed<Vec<Uuid>>), NotDue, Stuck }`. `NotDue` covers absent, spent, not
    yet overdue, and an all-blank draft the door deleted. `Stuck` means a named writer has no vault key: rolled back.
  - `pub async fn overdue_drafts(client: &(impl GenericClient + Sync), patient: Option<Uuid>) ->
    anyhow::Result<Vec<Uuid>>`.
  - `pub async fn commit_overdue(client: &mut tokio_postgres::Client, node_sk: &SigningKey, draft: Uuid,
    node_origin: &str) -> anyhow::Result<CommitOutcome>`: one transaction, as `sign_note`: call the door with a fresh
    `Uuid::now_v7()`; for zero rows **commit** (so an all-blank deletion sticks) and answer `NotDue`; per part,
    `writer_key` (reuse sign.rs's; make it `pub(super)`), `body_from_spent`, `with_drafting_writer`,
    `prepare_sealed_as(&tx, body, node_sk)`, `submit_event` with no savepoint. A `None` key for a named writer →
    drop the transaction, `Stuck`.
  - `pub struct CommitReport { pub committed: usize, pub stuck: Vec<Uuid>, pub failed: Vec<(Uuid, String)> }` and
    `pub async fn commit_overdue_drafts(client: &mut tokio_postgres::Client, node_sk: &SigningKey, patient:
    Option<Uuid>, node_origin: &str) -> anyhow::Result<CommitReport>`: lists, then commits each draft in its own
    transaction; an error on one draft is recorded in `failed` and the loop goes on. Only the listing's own error
    is `Err`.
  - `SaveOutcome::Committed`, `TakeOutcome::Committed` (from the word `committed`), `SignRefusal::Committed`
    (`refusal_for(SaveOutcome::Committed)`).
  - Console: `cairn-node note-set-quarantine (--minutes <1..43200> | --default)`, exactly one of the two, behind
    `conn_posture::require_local` like `vault-set-unconfirmed-expiry`; prints `"a draft left untouched for N minutes
    goes into the record unsigned"` or `"… for 180 minutes (the default) …"`.
- [ ] **Step 1: Write the failing tests.**
  - `commit.rs` unit: `draft_due_adds_the_minutes` (exact instant); `a_named_writer_without_a_key_is_stuck` if the
    decision is factored as a pure fn (recommended: `fn part_writer_kid(writer: Option<&str>, kid: Option<String>) ->
    Result<Option<String>, Stuck>`).
  - `tests/note_commit.rs` (DB-gated, `test_serial_guard`, the fixtures `note_sign_parts.rs` uses):
    `an_overdue_two_writer_draft_goes_in_as_two_node_signed_events` (signer = node key; contributors exactly
    `[{A_kid,"drafted"},{node,"recorded"}]` then `[{B_kid,"drafted"},{node,"recorded"}]`; part 2's `addendum-to` is
    part 1; recording times are the parts'; the stated time survives as `t_effective`);
    `an_unknown_writers_part_is_the_node_alone` (`[{node,"recorded"}]`); `a_draft_not_yet_due_is_not_due`;
    `an_all_blank_overdue_draft_leaves_nothing` (no note, no draft); `a_writer_with_no_key_is_stuck_and_kept` (delete
    the member as the owner; the draft and its parts are intact afterwards); `one_stuck_draft_does_not_stop_the_others`;
    `two_commits_race_one_note` (Review Focus 2: two **separate** connections, `tokio::join!` of two
    `commit_overdue` calls on one draft: exactly one `Committed`, one `NotDue`, and `clinical_note` has the parts
    once); `a_sign_after_the_timer_is_refused_committed` (`sign_note` answers `Err(SignRefusal::Committed)` and
    writes nothing); `the_read_grades_a_committed_part_asserted` (`list_patient_notes`: grade `WentInUnsigned`,
    writer `Drafted("Nurse A")`); `an_open_draft_reads_due_and_overdue` (`NoteDraft.due` = last keystroke + policy;
    `overdue` false before, true after moving `last_keystroke` back).
  - `authority_lockstep.rs`: a fourth fixture, a committed part `[{H,"drafted"},{N,"recorded"}]` signed by N, read
    back from `event_log`: Rust `Asserted` (with `humans = [H]`) ↔ `cairn_claim_authority` `'unverified'`. Update the
    module doc's shape list and mapping line.
  - `window_role_grants_notes.rs`: under `cairn_window`, `cairn_note_quarantine_minutes()`, `cairn_note_drafts_overdue`
    and `cairn_note_draft_commit_overdue` each reach their deep path (an overdue draft actually listed and committed);
    `cairn_note_set_quarantine` is refused with 42501.
  - `cli_note_quarantine.rs`: refuses a remote database, and parses strictly (neither flag, both flags, 0, 43201 all
    refused), in `cli_vault_refuses_remote.rs`'s style.
  - `notes_tests.rs` (GUI tree): `committed_sentence_exact` for the save, the take and the sign answers (Task 4 uses
    the same words), e.g. save/fork `"This note went into the record unsigned after it was left untouched. What you
    typed since the last save is kept here as a new draft."` (plus its `nothing_new` and `everything` siblings in
    the shape of `SIGNED_ELSEWHERE*`), take `"this note has already gone into the record unsigned"`, sign `"this
    note had already gone into the record unsigned — nothing was signed; your text is still here"`, worded through
    `stale(…)` as `SignRefusal::Spent` is (the chart changed under the clinician, so a re-read is the remedy).
- [ ] **Step 2: Run them and see them fail.** Expected: compile errors, then DB failures for the missing behaviour.
  Show the DB suites ran: `--nocapture`, no `skipped:`.
- [ ] **Step 3: Implement.** In `read.rs`, add one column, the `drafted` contributor's `actor_id`, and pass it in
  `human_kids` only when `cairn_note_signer_name` names it (a known vault member is a human). The `TRUNCATE`,
  `SCHEMA` and grant lists come from Task 1.
- [ ] **Step 4: Run** `cargo test -p cairn-node --test note_commit --test authority_lockstep --test
  window_role_grants_notes --test cli_note_quarantine --test note_sign_parts --test note_read -- --nocapture`, the
  `cairn-node` lib tests, and the GUI tree's `cargo test` for `notes_tests`. Expected: PASS. If a fresh binary stalls
  on Gatekeeper, run the printed `target/debug/deps/<name>-<hash>` directly (memory "long gates").
- [ ] **Step 5: Commit**: `feat(note): commit an overdue draft as node-signed parts; note-set-quarantine`.

### Task 4: The window's runner, the pre-read commit, the due words, and `committed` forks

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/notes_commit.rs`, `notes_commit_tests.rs`
- Modify: `src/main.rs` (one spawn in `.setup`); `src/notes.rs` (`note_list_impl` commits first); `src/notes_parts.rs`
  (`DraftView.words` gains the due words); `src/notes_words.rs` (`SaveView.committed`); `src-ui/notes_save.js` (the
  fork condition); `src/notes_fork_webview_tests.rs`; `src/commands_tests.rs` if it lists modules or scripts

**Interfaces:**
- Consumes: Task 3's `commit_overdue_drafts`, `NoteDraft.{due, overdue}`, `SaveOutcome::Committed`.
- Produces:
  - `pub const COMMIT_TICK: Duration = Duration::from_secs(60);`
  - `pub async fn commit_overdue_impl(state: &AppState, patient: Option<Uuid>) -> Option<CommitReport>`: `None` in
    fixture mode, with no node key or no connection; otherwise locks `state.db`, runs `commit_overdue_drafts`, and
    writes one stderr line per stuck or failed draft (draft id and cause only, never text or a patient name). It
    never returns an error to a caller.
  - `pub async fn run_ticks(app: tauri::AppHandle)`: forever, sleep `COMMIT_TICK`, then `commit_overdue_impl(&state,
    None)`. Spawned once in `.setup` with `tauri::async_runtime::spawn`.
  - `note_list_impl` calls `commit_overdue_impl(state, Some(patient))` **before** its snapshot read, and ignores the
    result (Review Focus 5).
  - `pub(crate) fn due_words<Tz: TimeZone>(due: DateTime<Utc>, overdue: bool, tz: &Tz, today: NaiveDate) -> String`:
    `"goes into the record unsigned at 06:40 if nobody types in it"`, or `"overdue — goes into the record unsigned
    at the next check"`; the time through `cairn_node::note::view::instant`. Appended to `DraftView.words` after
    `" · "`.
  - `SaveView.committed: bool`; JS fork condition `view.moved || view.committed || (view.spent &&
    !sent.held.mayBeSigned)`.
- [ ] **Step 1: Write the failing tests.** `notes_commit_tests.rs`: `due_words_before_and_after` (exact strings,
  fixed zone and day); `fixture_mode_commits_nothing` (`AppState::mock`: `None`); `a_failed_commit_never_blocks_the_read`
  (DB-gated: a live state with **no** node key reads the chart and shows the overdue draft's words);
  `opening_a_chart_commits_its_overdue_draft_first` (DB-gated: the pane lists the note `WentInUnsigned`, no draft);
  `the_tick_is_spawned_once_in_setup` (a `source_guard` pin on `main.rs`: one `run_ticks` spawn).
  `notes_fork_webview_tests.rs`: `a_timer_commit_forks_even_after_a_sign_press` (a save answering `committed` while
  `mayBeSigned` is set keeps the unsaved text as a new draft with the committed sentence: Review Focus 1). Assert
  the sentence is **visible** (computed display, no hidden ancestor), never just `textContent` (memory "webview
  mock walk").
- [ ] **Step 2: Run them and see them fail** (the GUI tree; `CAIRN_ALLOW_DB_SKIP=1` for the DB-free run, and the
  DB-gated run with the three `CAIRN_TEST_PG*` strings). Expected: FAIL.
- [ ] **Step 3: Implement.** Keep `notes.rs` under 500 lines: everything new goes in `notes_commit.rs`.
- [ ] **Step 4: Run the GUI tree's gate in CI's order**: `cargo fmt --check`, `cargo clippy --locked --all-targets
  -- -D warnings`, `cargo test`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` (memory "three Cargo trees").
  Expected: PASS.
- [ ] **Step 5: Commit**: `feat(window): commit overdue drafts on a tick and before each chart read`.

### Task 5: ADR, runbook, durable rules, tracking documents, issues

**Files:**
- Create: `docs/spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md`
- Modify: `docs/spec/index.md` (v0.90 → **v0.91**, ADR index), `mkdocs.yml` (nav line),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (new **§20**) + `TEMPLATE.md`, `docs/developers/durable-rules.md`
  (a **sign-in slice 3b-2a** block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: ADR-0089** records: the quarantine rule (last keystroke, database clock, site policy, default
  180 min, console only, outside backups); the atomic commit door and why the door decides; `by_timer` and the
  `committed` answer; the commit's contributor shapes (node-signed, `drafted`, never attested) and the stuck case;
  the empty-draft deletion; `Asserted` and its words; the window as the runner (the tick, the pre-read commit, why a
  Rust task and not a webview timer: WebKit throttles the timers of a hidden window); the three maintainer decisions
  above (#979, #980). Under "what is not guaranteed": with no window running a draft waits (shown overdue) and no
  peer sees it; a pane already open on the chart keeps showing a committed draft until it is read again (the next
  save forks with the committed sentence); the commit door cannot prove the note a `cairn_window` client submits
  carries the draft's text (a raw-SQL client holding the node key could already submit any node-signed note);
  revision rows now live at most the quarantine plus one tick (**#968**: comment, do not close). Fact-check every
  sentence against the SQL before the PR (memory "SDD evidence + ADR fact-check").
- [ ] **Step 2: RUNBOOK §20** (human-owed): set the quarantine to 2 minutes at the console; leave a two-writer draft;
  measure overdue to "went in unsigned" on screen ≤ 60 s with the window open on another chart; a chart open that
  commits first ≤ the single-chart open + 200 ms; type into a just-committed draft and see the committed sentence;
  VoiceOver reads "written by unverified Nurse A · went in unsigned".
- [ ] **Step 3: Durable-rules block**: the door decides overdue (never the caller, never a terminal clock); a column a
  door reads lives with its table; `committed` forks regardless of `mayBeSigned`; a stuck draft is never committed
  as "not known"; `recorded` is never the human contributory role. Each with its pin and its tempting wrong fix.
- [ ] **Step 4: HANDOVER + ROADMAP**: 3b-2a built, 3b-2b next; prune. Run `scripts/check_closing_keywords.py`.
- [ ] **Step 5: Full gate in CI's order after the last edit** (memory "a gate is evidence only for the tree it ran
  against"; the root gate is `scripts/run-db-gated-tests.sh`, detached, ~2 h), then push and open the PR.
