# GUI sign-in, slice 3c-1 — the compound note and "Add to this note" — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A note and everything later added to it read as **one note**: one continuous text, headed by its first
writer and time, with a state line ("includes an unsigned part") and an **"N parts"** disclosure that shows each
part with who typed it, when, and how it went in. Any clinician can press **"Add to this note"** on a note of the
chart on screen, type, and Sign (or let the timer commit it): the result is an addendum to that note, never a new
one. #948 is answered: an addendum that reaches a peer before its note is **penned and released by itself** once
the note lands.

**Architecture:** No new event type and no wire change: an addendum is a `clinical.note.asserted` whose first
part carries `refs: [{rel: "addendum-to", event: <the note's root>}]`, already in ADR-0041 decision 8's closed
enum, and later parts chain to the part before as today (ADR-0088). The draft header gains an immutable
`addendum_to` (`db/065` in place); a new door creates an addendum draft's header and checks its target
(`db/070`); Sign, the timer commit and the reader take the target from the header through a second new door. The
floor (`db/064` in place) refuses, **at the authoring door only**, an `addendum-to` naming anything but a note of
the same chart; the remote door keeps its existence check, which pens a too-early addendum (#948). Grouping parts
into compounds is a pure function in `cairn-node` so any UI can use it (ADR-0021); the window draws it.

**Tech Stack:** PostgreSQL 18 (new `db/070`; in-place edits to `db/064`, `db/065`), Rust (`cairn-node`), Tauri 2 +
plain JS (`cairn-gui-tauri`, no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
§3.4 (addenda), §3.7 (the compound note), §4.3, §5 (3c), read with what has narrowed it since: ADR-0087 (R4, R5,
R8, R9), ADR-0088 (parts; Sign = one event per part chained by `addendum-to`), ADR-0089 (`Asserted`; the timer
commit), ADR-0090 (discard). **Maintainer, 2026-11-05:** 3c is three PRs — **3c-1** (this: compound + addendum +
#948, no wire change), **3c-2** (the vouch overlay — **one event per vouched part**, a new type
`clinical.note-attestation.asserted`, contributors `{human, attested | co-signed, responsibility}`, payload naming
the target event, its content address and the act `addendum` | `sign-off`; offline-first at the remote door; the
strict door checks the target and its content address — plus "Sign off this note", the `student` label, "not
co-signed", the per-part "signer unconfirmed"), **3c-3** (the pending-sign-offs menu + #918's display half);
**#948 stays penned** (a test and an ADR line, no code). Read ADR-0087…0090 and the sign-in blocks of
`docs/developers/durable-rules.md` before Task 1.

## Global Constraints

- **No wire change.** No new event type, payload key, profile or `refs` rel. `clinical.note/1` is unchanged; the
  pinned twin/projection registry counts do not move.
- **What an addendum names:** the **root** of the compound the clinician pressed on (the note with no
  `addendum-to`, or with one this node does not hold — see "orphan" below), never its latest part. Only the draft's
  FIRST non-blank part carries it; later parts of the same draft keep chaining to the part before (ADR-0088).
- **`note_draft.addendum_to uuid` (NULL = a new note), created in `db/065` with its paired
  `ALTER TABLE … ADD COLUMN IF NOT EXISTS`** (3b-2a's durable rule: a column a door reads lives with its table).
  Set once, when the header is inserted; **no door ever updates it**, so a reader may read it without the draft's
  lock.
- **The floor rule, authoring door only (`v_authoring` in `cairn_check_note_assertion`, db/064 rule 6, in place):**
  an `addendum-to` target must be an `event_log` row of type `clinical.note.asserted` with the event's own
  `patient_id`. Message: `note assertion: an addendum-to target must be a note on this chart, got %`. The remote
  door is NOT tightened: it keeps the existence check alone, so its refusal stays the bare `RAISE` (`P0001`) that the
  puller pens (`cairn-sync`'s `is_deliberate_refusal`; `clinical_pull.rs::refused_apply_pens_the_bytes_and_recovers_without_loss`).
  `correction-of` and `transcript-of` are untouched (nothing writes them yet).
- **New doors (`db/070_note_addendum.sql`), `SECURITY DEFINER SET search_path = public, pg_temp`, `EXECUTE`
  revoked from `PUBLIC` and granted to `cairn_window` (as db/066's):**
  - `cairn_note_draft_addendum_part_save(p_draft uuid, p_patient uuid, p_terminal text, p_writer text,
    p_narrative text, p_stated_time timestamptz, p_addendum_to uuid) RETURNS text`. A NULL in any argument but
    `p_writer`/`p_stated_time` raises (a caller bug, as db/066's). Takes the advisory lock
    `'note_draft:' || p_draft` first. If the draft is spent: the same `spent` | `committed` | `discarded` words as
    db/066. If no header exists: answers `no-such-note` unless `clinical_note` holds `p_addendum_to` with
    `patient_id = p_patient`; answers `saved` with NO header when `p_narrative` is blank (`cairn_twin_is_present`);
    otherwise inserts the header with `addendum_to` and `recording_started = last_keystroke = clock_timestamp()`.
    If a header exists with `addendum_to IS DISTINCT FROM p_addendum_to`: `elsewhere`. Then it returns
    `cairn_note_draft_part_save(…)` with the same first six arguments, so every rule of the part save (writer
    check, tail rule, revision row) is the one rule.
  - `cairn_note_draft_addendum_to(p_draft uuid) RETURNS uuid` — the header's `addendum_to`, NULL for a new note's
    draft or no such draft. `STABLE`.
- **`SCHEMA_GENERATION` 69 → 70** (`crates/cairn-event/src/schema_generation.rs`), `db/070` appended to
  `crates/cairn-node/src/db.rs`'s `SCHEMA` list.
- **The compound (pure, `crates/cairn-node/src/note/compound.rs`):** a part's root is found by following
  `addendum_to` through the notes in the read; a part whose target is not in the read is a root marked
  **orphan** (a target withheld on a doubted linked chart, or a newer profile ruling R8 keeps off the screen). The
  walk is bounded by the read's length (a cycle cannot be signed, but a bound costs nothing). Parts: the root first,
  then the rest by `(recording_started, event_id)`. Compounds: newest root `t_effective` first (today's order).
- **The compound's state (pure), in exactly these words:** every part `NoteGrade::Signed` → `None` for a one-part
  note (its byline already says it), `"every part signed"` otherwise; any `WentInUnsigned` or `WriterNotKnown` →
  `"includes an unsigned part"`; else (only `Claimed` besides `Signed`) →
  `"includes a part whose signature this node cannot verify"`. The orphan heading is prefixed
  `"Adds to a note not shown here · "`. The disclosure's label is `"{n} parts"` (n ≥ 2; a one-part note has none).
  The continuous text is the parts' texts in part order joined by `"\n\n"`.
- **"Add to this note" is offered** only on a compound whose root's `chart` is the chart on screen, while the
  editor is free, behind the same who's-here gate as "New note" (`claimAsked()` / `askWhosHere`). A linked chart's
  note shows no button (the draft would live on a chart this pane does not list; filed as an issue in Task 7).
- **The editor of an addendum draft** shows, above the text, `"Adding to the note of {when}"` with the root's
  `view::time_words`; a draft whose target this pane does not show says `"Adding to a note not shown here"`. The
  drafts list prefixes the same words to the draft's line.
- **The window carries the target on every save, Sign and Discard** (`addendum_to: Option<String>`, the root's
  event id): Sign or Discard pressed inside the one-second debounce must still create an addendum's header, never a
  new note's (Review Focus 2). Rust picks the door: `Some` → the addendum door, `None` → db/066's.
- **Sign and the timer read the target from the header, never from the window**: after `save_part` (Sign, under
  the draft's lock) or before `commit_overdue_parts` (timer; the column is immutable), via
  `cairn_note_draft_addendum_to`. The first part's body gets `addendum_to` = that target; later parts as today.
- **Pinned guards a change here must satisfy** (memory "pinned-count guards"): `floor_execute_grants.rs` (the two
  doors revoked from `PUBLIC`) · `window_role_grants_notes.rs` (both doors driven to their deep path under
  `cairn_window`) · `db_errors_stay_legible.rs` per-file counts for any new `LocalDbFault`-wrapped read ·
  `tests/common` helper lists if a helper is added (`identity_scaffolding_shared.rs`). Struct-literal sites of
  `SignNoteInput` (gains `addendum_to`): `crates/cairn-node/tests/{note_sign,note_sign_parts,note_discard,note_read,
  note_commit,note_draft_parts,authority_lockstep,window_role_grants_notes}.rs` and the window's `notes*.rs` — and
  run the **full-workspace** build (memory "cross-crate call-site signature changes").
- **House rules:** TDD; every non-trivial fn documented for a junior reader; files under 500 lines
  (`notes_webview_tests.rs` is at 485 — new pins go in a new file); no `salt`/`nonce`/`iv` names;
  `scripts/check_closing_keywords.py` before every commit; DB-gated tests show no `skipped:` line (`--nocapture`); a
  DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`. A task touching `cairn-gui/*/src` runs every root source guard DB-free
  (`grep -ln production_rust_files crates/*/tests/*.rs`) with its own `CARGO_TARGET_DIR`.

## Review Focus

1. **Two clinicians add to the same note at once, on two terminals.** Both addenda name the root; both show, in
   recording order, inside the one compound; neither is lost or shown as a separate note. Pinned in Task 3 (pure)
   and Task 4 (DB, two drafts signed).
2. **Sign pressed inside the debounce of a brand-new addendum** (no header yet). Sign's own save creates the
   addendum header, so the signed event is an addendum. A save that predates the header must never make it a new
   note. Pinned in Task 4.
3. **The timer commits an addendum draft** (the writer walked away). Its first part is `addendum-to` the root,
   exactly as a Sign's would be. Pinned in Task 4.
4. **A raw-SQL client** names another chart's note, a medication event, or an unknown id as the `addendum-to`
   target. The authoring door refuses all three. At the remote door only the unknown id is refused, as `P0001`.
   Pinned in Task 1 and Task 2.
5. **The window restarts mid-addendum.** The draft reopens as an addendum, with its "Adding to the note of …"
   line, and still signs as one. An addendum whose target is withheld (a doubted link) reads as an orphan, not
   as a note of its own. Pinned in Tasks 3, 5 and 6.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the progress sheet in the bedside folder. A colleague (or the same nurse, coming back)
  writes below an earlier entry in their own hand and signs. The page reads as one entry, top to bottom, and the
  handwriting shows who wrote which lines.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Adding to a note and signing it: 3 (find the entry, write below it, sign) → 3 (press "Add to this note" on it,
    type, Sign + passphrase, one act as slice 1 counts it) → 3.
  - Finishing one's own note that went in unsigned: 3 → 3 → 3. Until 3c-2, the earlier part still reads "went in
    unsigned": an honest under-claim, never an over-claim.
  - Reading a note with additions: 0 → 0 → 0. The compound and its state line show at once.
  - Seeing who wrote which part: 0 on paper (the handwriting) → 0 architecture-forced (the read carries every part's
    writer) → **1** in the UI (open "N parts"). `M = N`, so this is no architecture defect. `K > N` is the
    maintainer's chosen display (design §3.7: one continuous text "with a flag that allows to drill down"). The
    state line keeps the safety-relevant fact, an unsigned part, at 0 presses.
  - Adding to a linked chart's note: 3 on paper → 3 architecture-forced → **4** in this UI (open that chart
    first). A UI limit, not `M > N`. Filed in Task 7.
- **Time + cognitive load:** signing an addendum stays within RUNBOOK §16's sign ≤ 3 s (the `note-sign` timing kind,
  unchanged). Opening a compound's parts takes ≤ 1 s. One button per note and the same editor and strip as "New
  note"; the editor's "Adding to the note of …" line says what the text will join. A human owes the measurement in
  **RUNBOOK §22** (Task 7).

---

### Task 1: The addendum draft and the same-chart floor rule (`db/065`, `db/064`, new `db/070`)

**Files:**
- Modify: `db/065_note_draft.sql` (the column + paired `ALTER`), `db/064_clinical_note.sql` (rule 6, authoring arm),
  `crates/cairn-node/src/db.rs` (`SCHEMA`), `crates/cairn-event/src/schema_generation.rs` (70)
- Create: `db/070_note_addendum.sql`, `db/tests/070_note_addendum_test.sql` (with `\ir _scratch_database_guard.sql`)
- Test: the SQL mirror; `crates/cairn-node/tests/note_addendum_doors.rs` (new); `window_role_grants_notes.rs`;
  `floor_execute_grants.rs`

**Interfaces:**
- Produces: the column and two doors exactly as in Global Constraints; the floor message above.

- [ ] **Step 1: Write the failing tests** (`note_addendum_doors.rs`, DB-gated, `test_serial_guard`):
  `an_addendum_draft_names_its_note` (save with a target → `saved`; `cairn_note_draft_addendum_to` = target);
  `a_blank_first_save_makes_no_header` (`saved`, `cairn_note_draft_addendum_to` NULL, `cairn_note_draft_parts`
  empty); `a_target_on_another_chart_is_no_such_note` and `a_target_that_is_not_a_note_is_no_such_note` (a
  medication event id); `a_new_notes_draft_refuses_a_target_later` (header with NULL target, then the addendum door
  → `elsewhere`); `a_later_plain_save_keeps_the_target` (addendum door, then db/066's door → still the target);
  `the_floor_refuses_an_addendum_to_another_charts_note` and `…_to_a_medication_event` (`submit_event` of a signed
  note body; assert the new message); `the_remote_door_still_admits_a_held_target_on_another_chart` (documents
  that only existence is checked there). In `window_role_grants_notes.rs`, drive both doors to their `saved` path
  as `cairn_window`; in `floor_execute_grants.rs`, both doors revoked from `PUBLIC`.
- [ ] **Step 2:** run them (`cargo test -p cairn-node --test note_addendum_doors -- --nocapture`). Expected: FAIL
  (no such function).
- [ ] **Step 3: Implement** the column, the floor arm, `db/070`, the SQL mirror, `SCHEMA` and generation 70.
- [ ] **Step 4:** the same command, plus `window_role_grants_notes`, `floor_execute_grants`, `note_sign_parts` (the
  in-transaction chain must still pass the new floor arm), and the SQL mirror via `scripts/run-db-gated-tests.sh`'s
  SQL step. Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(note): an addendum draft names its note; the floor holds it to the chart (db/070)`.

### Task 2: #948 — a too-early addendum is a deliberate refusal, admitted once its note lands

**Files:**
- Create: `crates/cairn-node/tests/note_addendum_ahead_of_target.rs`

**Interfaces:**
- Consumes: `apply_remote_event` (db/020), the note body builder `cairn_event::note` (as `note_sign_parts.rs`
  builds a peer's note), Task 1's floor.

- [ ] **Step 1: Write the test** `an_addendum_ahead_of_its_note_is_penned_then_admitted`: author a note N and an
  addendum A (`addendum-to` N) as an enrolled peer actor. Apply A through `apply_remote_event`: assert the error's
  SQLSTATE is exactly `P0001`, the class `cairn-sync` pens and re-offers (cite
  `clinical_pull.rs::refused_apply_pens_the_bytes_and_recovers_without_loss` in a comment, which proves pen →
  re-offer → release), and that its message contains `is not held on this node`. Then apply N, re-apply A's same
  bytes, and assert both are admitted and both are in `clinical_note`.
- [ ] **Step 2:** run it. It should PASS with no code change (the behaviour exists). If it fails, stop and report:
  the maintainer's #948 ruling assumed it. Mutate to check that the test can see a failure: temporarily change
  db/064 rule 6's `RAISE` to raise with SQLSTATE `XX000`. The test must FAIL. Revert, then confirm with
  `git status --short` that only the test file is new.
- [ ] **Step 3: Commit** `test(note): #948 — an addendum ahead of its note is penned, then admitted`.

### Task 3: The compound note (pure, `crates/cairn-node/src/note/compound.rs`)

**Files:**
- Create: `crates/cairn-node/src/note/compound.rs`
- Modify: `crates/cairn-node/src/note/mod.rs` (`pub mod compound;`)
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `read::NoteRow`, `view::NoteGrade`.
- Produces:
  - `pub struct CompoundNote { pub root: Uuid, pub chart: Uuid, pub orphan: bool, pub parts: Vec<NoteRow> }`
    (`parts[0]` is the root).
  - `pub fn compounds(notes: Vec<NoteRow>) -> Vec<CompoundNote>`
  - `pub enum CompoundState { OnePart, EverySigned, IncludesUnsigned, IncludesUnverified }` with
    `pub fn compound_state(parts: &[NoteRow]) -> CompoundState` and `pub fn state_words(s: CompoundState) ->
    Option<&'static str>` (the exact words in Global Constraints; `OnePart` → `None`).

- [ ] **Step 1: Write the failing tests:** `a_note_without_addenda_is_one_compound`;
  `a_multi_part_sign_is_one_compound_in_writing_order` (part 2 → part 1 → root chain);
  `two_addenda_to_one_root_join_it_in_recording_order` (Review Focus 1; equal `recording_started` ordered by
  `event_id`); `an_addendum_whose_note_is_not_read_is_an_orphan_root`; `compounds_are_newest_root_first`;
  `a_cycle_is_bounded` (two rows naming each other each end up in exactly one compound; no hang);
  `the_state_words_are_exact` (each of the four states, including `IncludesUnverified` for a Signed + Claimed pair).
- [ ] **Step 2:** `cargo test -p cairn-node --lib note::compound` (`CAIRN_ALLOW_DB_SKIP=1`). Expected: FAIL.
- [ ] **Step 3: Implement.** A `HashMap<Uuid, usize>` from event id to index. Walk each row's `addendum_to` up to
  `notes.len()` steps to find its root. Group, then sort as specified.
- [ ] **Step 4:** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): a note and its addenda group as one compound (pure)`.

### Task 4: Sign, the timer and the read carry the target (`cairn-node`)

**Files:**
- Modify: `crates/cairn-node/src/note/draft.rs` (`save_part` gains `addendum_to: Option<Uuid>`; `SaveOutcome::NoSuchNote`
  for `no-such-note`; `NoteDraft.addendum_to: Option<Uuid>`, read in `open_drafts` through
  `cairn_note_draft_addendum_to`; `pub async fn draft_addendum_to(client, draft) -> Result<Option<Uuid>>`),
  `sign.rs` (`SignNoteInput.addendum_to: Option<Uuid>`, passed to `save_part`; after the save,
  `draft_addendum_to` gives the first part's `addendum_to`; `refusal_for` maps `NoSuchNote` to a new
  `SignRefusal::NoSuchNote`), `commit.rs` (the target read before `commit_overdue_parts`), `discard.rs` (passes
  `input.addendum_to` through)
- Test: `crates/cairn-node/tests/note_addendum_sign.rs` (new), plus the struct-literal sites named in Global
  Constraints

**Interfaces:**
- Consumes: Task 1's doors.
- Produces: the fields and variants above. `body_from_spent`'s last argument for the first part becomes the
  header's target (today `None`).

- [ ] **Step 1: Write the failing tests** (`note_addendum_sign.rs`): `signing_an_addendum_names_the_root`;
  `sign_inside_the_debounce_still_makes_an_addendum` (Sign with no earlier save, `addendum_to: Some(root)`);
  `a_two_writer_addendum_chains_part_two_to_part_one` (part 1 → root, part 2 → part 1);
  `the_timer_commits_an_addendum_as_an_addendum` (quarantine set to 0 minutes through the console door);
  `two_addenda_signed_at_once_both_join_the_note` (two drafts, two connections, each `addendum-to` the root;
  `compounds(read)` gives one compound with three parts); `an_addendum_target_on_another_chart_refuses_the_sign`
  (`SignRefusal::NoSuchNote`, nothing written); `the_drafts_list_carries_the_target`.
- [ ] **Step 2:** run them. Expected: FAIL (no field).
- [ ] **Step 3: Implement**, then fix every `SignNoteInput { … }` and `save_part(` call site (`addendum_to: None`
  where the call is not about addenda).
- [ ] **Step 4:** the new tests, the eight note test binaries named above, then `cargo build --workspace --tests` and
  the `cairn-gui` tree's `cargo build --tests`. Expected: PASS / clean.
- [ ] **Step 5: Commit** `feat(note): Sign, the timer and the read carry an addendum's target`.

### Task 5: The window's compound and addendum views (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/notes_compound.rs`, `notes_compound_tests.rs`
- Modify: `notes_parts.rs` (`NotePane.notes: Vec<CompoundView>`; `DraftView` gains `addendum_to: Option<String>`
  and the "Adding to…" prefix on `words`), `notes.rs` (`note_draft_save`, `note_sign` take
  `addendum_to: Option<String>`; `note_list_impl` builds compounds), `notes_discard.rs` (its command takes it too),
  `notes_words.rs` (the `NoSuchNote` save and sign words: `"The note this adds to is not on this chart; nothing was
  saved."`), `main.rs` only if a command is registered anew (none is)
- Test: `notes_compound_tests.rs`, existing `notes_parts_tests.rs` / `notes_tests.rs` updated to the new pane shape

**Interfaces:**
- Consumes: Task 3's `compounds`, `compound_state`, `state_words`; Task 4's fields.
- Produces (serde names are what the webview reads):
  `pub struct CompoundView { pub root_event_id: String, pub text: String, pub byline: String, pub byline_spoken:
  String, pub when: String, pub state: Option<String>, pub parts_label: Option<String>, pub parts: Vec<NoteRowView>,
  pub can_add: bool }`. `byline`/`when` are the root's `NoteRowView` words, `byline` prefixed by the orphan
  words when orphaned. `parts` is empty for a one-part note. `can_add` = root chart is the chart on screen.
  `pub(crate) fn compound_view<Tz>(c: &CompoundNote, opened: Uuid, tz: &Tz, today: NaiveDate) -> CompoundView`.

- [ ] **Step 1: Write the failing tests:** `a_one_part_note_has_no_parts_label_or_state`;
  `a_three_part_note_reads_as_one_text` (joined by `"\n\n"`, `parts_label` `"3 parts"`, state
  `"includes an unsigned part"` when one part went in unsigned); `a_linked_charts_note_cannot_be_added_to`;
  `an_orphan_says_it_adds_to_a_note_not_shown`; `an_addendum_draft_says_what_it_adds_to` (`DraftView.words`
  begins `"Adding to the note of "` + the root's `when`; a target not in the pane → `"Adding to a note not shown
  here"`); `no_such_note_has_its_own_words`.
- [ ] **Step 2:** the `cairn-gui` tree's `cargo test -p cairn-gui-tauri notes_compound` (`CAIRN_ALLOW_DB_SKIP=1`,
  own `CARGO_TARGET_DIR`). Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** the `cairn-gui` tree's full gate (clippy `-D warnings`, `cargo test`, `cargo doc` with
  `RUSTDOCFLAGS=-D warnings`) and every root `production_rust_files` guard DB-free. Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): the compound note's view; saves, signs and discards carry the target`.

### Task 6: The compound and "Add to this note" in the webview (`notes.js`, new `notes_compound.js`)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/notes_compound.js` (`compoundItem(compound)`, `addToNote(compound)`),
  `cairn-gui/cairn-gui-tauri/src/notes_compound_webview_tests.rs`
- Modify: `src-ui/notes.js` (`renderNotes` maps `pane.notes` through `compoundItem`; `noteDraft` gains `addendumTo`
  — `null` from `newNote`, the root id from `addToNote`, `draft.addendum_to` from `openDraft`; the editor's
  "Adding to…" line), `notes_save.js` and `notes_discard.js` (send `addendumTo: noteDraft.addendumTo` on every
  `note_draft_save` / `note_sign` / discard invoke), `index.html` (the script tag before `notes.js`; an
  `id="note-adds-to"` line above the editor's text box)

**Interfaces:**
- Consumes: Task 5's `CompoundView` and `DraftView.addendum_to`.
- `compoundItem` draws: the continuous text (`textContent`), the byline as `noteItem` does today (shown text
  `aria-hidden`, spoken text visually hidden, #973), the `state` line when present, a
  `<details><summary>{parts_label}</summary>` holding each part via the existing `noteItem`, and when `can_add` and
  `noteDraft === null`, a button `Add to this note` with `aria-label` `"Add to this note: " + when`.
  `addToNote` copies `newNote`'s who's-here gate and opens the editor with `addendumTo = compound.root_event_id`
  and the "Adding to the note of {when}" line.

- [ ] **Step 1: Write the failing pins** (`notes_compound_webview_tests.rs`, `source_guard::js_fn` /
  `js_const` on exact expressions): the button is offered only under `can_add && noteDraft === null`; `addToNote`
  calls `askWhosHere(` when `!claimAsked()`; every `note_draft_save`, `note_sign` and discard invoke passes
  `addendumTo`; `openDraft` copies `draft.addendum_to`; the parts sit inside a `details` element; the text and the
  parts are set by `textContent`, never `innerHTML`.
- [ ] **Step 2:** run them. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** the pins, the `cairn-gui` gate, and a headless walk of the webview (memory "webview mock walk
  recipe"; assert VISIBILITY, not just text): a three-part compound, its disclosure opened by keyboard; "Add to this
  note" → type → Sign with a stubbed `note_sign`, checking that the invoke carries `addendumTo`; a reopened
  addendum draft shows its "Adding to" line. Also walk the OLD `notes.js` with the same script. It must fail (the
  rig is discriminating).
- [ ] **Step 5: Commit** `feat(window): the compound note and "Add to this note"`.

### Task 7: ADR-0091, spec, RUNBOOK §22, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0091-the-compound-note-and-the-addendum.md`
- Modify: `docs/spec/index.md` (v0.92 → **v0.93**), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav),
  `docs/spec/data-model.md` (a "Since sign-in slice 3c-1" paragraph beside ADR-0090's, near line 520),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (**§22**: add-and-sign ≤ 3 s; open the parts ≤ 1 s; the orphan
  heading; the disclosure and button by keyboard and VoiceOver; WebKit) and `TEMPLATE.md`,
  `docs/developers/durable-rules.md` (a 3c-1 block: the target is the ROOT, set once and never updated; the floor
  arm is authoring-only, so never tighten the remote door — that would turn #948's pen into a permanent refusal
  for a peer whose floor is older; grouping is pure and lives in `cairn-node`; each with its tempting wrong fix and
  its pin), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1:** write ADR-0091: Context; decisions (the addendum is a note naming the root; the header column and
  its immutability; the authoring-only same-chart rule; the compound and its state words; the orphan; "Add to this
  note" on the opened chart only); **#948 decided: penned** (maintainer, 2026-11-05), with what the receiving node
  shows meanwhile (nothing of the addendum) and the cost (one pen row, a loud cycle); the interim under-claim
  until 3c-2; **3c-2's vouch shape as decided** (maintainer, 2026-11-05, Spec section above), to be built and
  recorded in full by 3c-2's ADR. **Fact-check every sentence against the SQL and Rust before committing** (memory
  "SDD evidence + ADR fact-check").
- [ ] **Step 2:** the rest of the files; `uv run --with-requirements docs/requirements.txt -- mkdocs build`.
  Expected: no warnings about the new pages.
- [ ] **Step 3:** file the issues: adding to a linked chart's note (paper-parity `K = 4 > N = 3`, a UI limit), and
  any review residual. Issue #948 may be closed by the PR body only once ADR-0091 records the ruling.
  Run `scripts/check_closing_keywords.py`, then commit
  `docs(note): ADR-0091 — the compound note and the addendum; spec v0.93, RUNBOOK §22`.
