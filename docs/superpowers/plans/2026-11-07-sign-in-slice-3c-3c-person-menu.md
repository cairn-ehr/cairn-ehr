# GUI sign-in, slice 3c-3c — the person menu ("Waiting for you") — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The person named in who's here sees, in one place on the who's-here bar, everything that waits for their
signature: their own drafts and unsigned parts (group 1), their supervisees' parts that need their co-signature
(group 2), and their supervisees' recent notes to review (group 3, collapsed and never counted). "Waiting for you: N"
on the bar counts groups 1 and 2 only. "Open note" opens that chart on that note, and the who's-here picker shows each
person's count.

**Architecture:** No new event, wire, floor or projection. **db/073** adds two read-only definer doors for what the
window cannot read itself: `cairn_note_people()` (every actor the menu may name, with their keys) and
`cairn_note_drafts_by_writer()` (who has a non-blank part in which open draft, and when it goes in). The cross-chart
note read is the chart list's own query and row parser (`note::read`), unchanged except for its `WHERE`. It reads the
parts signed or contributed by a set of keys, so every grade, vouch and supervision annotation is computed exactly as
the chart computes it. Grouping is one pure module, `note::menu`, built from the predicates the chart already uses
(`counts_as_unsigned`, `part_is_signed`, `cosign::awaits_cosign`, `review::reviewable`), so the menu can never
disagree with the byline. The window adds three commands and one script, `person_menu.js`.

**Tech Stack:** PostgreSQL 18 (db/073 new), Rust (`cairn-node`, `cairn-gui-tauri`), plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md`](../specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md)
§3.4 and §5's 3c-3c row (maintainer-approved; mockup "exactly as I envisioned it"). Read with
[ADR-0094](../../spec/decisions/0094-supervision-and-the-mandatory-co-sign.md) (the record and the co-sign rules this
reads), [ADR-0093](../../spec/decisions/0093-a-repudiated-signature-counts-as-unsigned.md) (group 1's repudiated rows),
[ADR-0089](../../spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md) (a draft's due time; `Asserted`),
[ADR-0088](../../spec/decisions/0088-whos-here-and-a-drafts-parts.md) (who's here is a claim). Before Task 1 read the
sign-in 3c-3a, 3c-3b, 3c-2, 3b-2a and 3b-1 blocks of `docs/developers/durable-rules.md`.

**Open for the maintainer's plan review** (each has a default the plan builds if you say nothing):
- **Q1, the picker's counts.** One count per person means reading the parts of EVERY member: every note part on the
  node, on every picker opening. Default: build it, draw the list first and fill in the counts when they land, carry a
  `shortcut:` comment, and measure it in RUNBOOK §26. If it misses ≤ 1 s, file an issue for a projection of open
  obligations. The alternative is to move picker counts to a later slice.
- **Q2, the projection's text.** The shared query reads `clinical_note.narrative`, the CLEAR projection the chart list
  already reads. That is not a sealed body (design §3.4 forbids decrypting for a list), and the menu shows none of it.
  Default: share the query as is, because a second column list would be a second thing to keep in step.
- **Q3, "Open note" from an open chart** goes through `closeChart()` first, so queued saves land and panels close.
  The front door flashes for a moment. Default: accept it; it is the path "Who's here?" already takes.

## Global Constraints

- **Whose menu:** the person a who's-here claim NAMES (`Claim::Named`). "Not asked" or "Don't know" → no menu, no
  count, no database read; the bar's menu element is `hidden` (design §3.4, *my call*).
- **What N counts:** `own.len() + cosign.len()`. Never the review group (design §3.4; maintainer: it must not nag).
- **Group 1 — Your signature** (all of these, for the person `me`, by actor):
  - each open draft holding a non-blank part whose writer is `me`'s staff id, with its due time and "stuck" flag
    (db/067's formula exactly);
  - each part with `signed_by == None`, `!part_is_signed(p)` and `drafted_actor == me`. It went in unsigned, and a
    colleague's later sign-off takes it off the list;
  - each part with `counts_as_unsigned(p)` and `signed_by.actor == me` (the repudiated key still belongs to the real
    person's actor, ADR-0085).
- **Group 2 — Needs your co-signature:** each part with `cosign::awaits_cosign(p)`, whose `supervision::writer_actor(p)`
  is not `me`, and whose `writer_supervisors` (NOW) contains `me`. A writer with no supervisor recorded now is in
  nobody's group 2 (design §3.4). The row names the OTHER current supervisors.
- **Group 3 — Your supervisees' recent notes:** each part with `review::reviewable(p, me)` and
  `p.recorded_at >= now - REVIEW_DAYS`, where `pub const REVIEW_DAYS: i64 = 7` (design §3.4, *my call*).
- **Order:** drafts by due time, soonest first; every other row newest `recorded_at` first, with `event_id` breaking a
  tie. Within group 1, drafts come before parts.
- **Copy, exact:**
  - bar: `"Waiting for you: {N}"`; its accessible name `"Waiting for you: {N} — open the list"`.
  - headings: `"Your signature"`, `"Needs your co-signature"`, `"Your supervisees' recent notes (last 7 days)"`.
  - an empty group: `"Nothing here."`. The review group's `<details>` summary keeps its heading and adds no count.
  - row `what`: `"Progress note"`; a draft row: `"Draft note"`.
  - row state: a draft reuses `notes_commit::due_words` verbatim (no new copy). Went in unsigned:
    `"Went in unsigned — sign it"`. Repudiated: `"Counts as unsigned — check it is yours, then sign or correct it"`.
    Co-sign: `"By {writer} — needs your co-signature"`, plus `" (you or {a}, {b} or {c})"` when other current
    supervisors exist (names in display-name order, `" or "` before the last). Review: `"By {writer} — not yet reviewed"`.
    `{writer}` and the supervisors' names come from `cairn_note_people()`'s `display_name`, or `"a writer this node
    cannot name"` when that is NULL.
  - the button on every row: `"Open note"`; its accessible name `"Open note: {patient}, {when}"`.
  - "Open note" refused because the row is gone: `"That note no longer waits in your list — the list was read again."`
  - picker entry suffix when the count is positive: `" · {N} waiting"`; a failed count read leaves no suffix and sets
    `#whos-here-status` to `"Waiting counts could not be read."`.
- **No passphrase and no dialog** (design anti-goals; principle 3). Reading the menu signs nothing.
- **Wrong-chart rule unchanged:** a chart opens only through `open_chart_impl` and `AppState::shown`. The menu admits
  its rows' charts to `shown` the way the duplicate tray does (`worklist/mod.rs`).
- **One snapshot:** the people, the supervision record, the parts and the drafts are read in ONE `in_snapshot`
  (REPEATABLE READ), as `note_list` reads.
- House rules: TDD; inline docs for a junior; files under 500 lines (new modules below; `read.rs` is 365 lines and
  shrinks when its loop body moves into `note_row`); test key material minted at runtime; no `salt`/`nonce`/`iv`
  names. Pure tests: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib note::`. If an IDE holds `target/`, prefix
  `CARGO_TARGET_DIR=/tmp/cairn-3c3c`. DB suites run with `--nocapture` and must print no `skipped:` line.
- **Pinned-count guards this slice moves:** `SCHEMA_GENERATION` 72 → 73 (`crates/cairn-event/src/schema_generation.rs`);
  `crates/cairn-node/src/db.rs`'s `SCHEMA` list gains `073_person_menu`; `window_role_grants_notes.rs` (two new window
  doors); `floor_execute_grants.rs` (the two doors revoked from PUBLIC); `search_path_pg_temp.rs` (definers pin
  `pg_temp` last); any new `LocalDbFault`-wrapped read in `db_errors_stay_legible.rs`. The new `person_menu.js` joins
  `commands_tests.rs`'s script list AND `waiting_webview_tests.rs`'s `SCRIPTS` (count +1), and gets its `<script>`
  tag in `index.html` before `whos_here.js`. A new `pub fn` in `tests/common/mod.rs` joins
  `identity_scaffolding_shared.rs`'s list. A new `cairn-gui/*/src` file is read by the ROOT tree's source guards: run
  them DB-free (memory: "root source guards sweep cairn-gui src").

## Review Focus

1. **A part signed under a requirement that has since been lifted stays in the supervisor's group 2** (design
   anti-goal). The menu reads `awaits_cosign`, which judges `needs_cosigning` at the part's time. Pinned in Task 4
   (DB: require → sign → lift → still listed and counted).
2. **A supervisor who has stepped away sees nothing of that supervisee in groups 2–3**, while a co-sign they gave
   before stepping away still clears the part, so the remaining supervisor's group 2 drops it. Pinned in Task 3
   (pure) and Task 4 (DB).
3. **A part signed with a since-repudiated key lands in the REAL person's group 1, not in anyone's group 2, and
   leaves it once they re-sign.** A colleague's sign-off does not take it off. Pinned in Task 3 with key ≠ claim
   fixtures, and in Task 4.
4. **The count never includes the review group, and the picker's count for a person equals the bar's count once they
   are named.** Both come from `PersonMenu::count` over the same read. Pinned in Task 3 (pure) and Task 5 (one
   fixture, both commands).
5. **"Open note" never opens a chart the re-read menu does not list**, and from an open chart the queued saves land
   first. Pinned in Task 5 (a row that vanished between draw and click is refused with the exact copy; no chart is
   set) and Task 6 (source guard: `openMenuRow` awaits `signsSettled()` and `closeChart()` before invoking).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the supervisor walking the ward or a stack of bedside folders to find the entries that wait
  for their countersignature, and pulling files at random to audit a supervisee's work; a clinician finding their own
  unsigned entries the same way.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Find what waits for my signature: one walk per folder (*N* ≥ 1 per folder) → *M* = 2 (open the menu, read it) →
    *K* = 1 (the bar's count already says whether anything waits).
  - Countersign one note from the menu: paper 2 (open the folder, sign) → *M* = 3 ("Open note", "Sign off this note"
    or "Co-sign as reviewed", the passphrase) → *K* = 3, the passphrase counted as slice 1 counts it. **Ruled not an
    `M > N` defect** (maintainer, 2026-11-07, design §6): paper's "open the folder" includes finding it and walking to
    it, and the menu's "Open note" does not.
  - Audit a supervisee's recent work: pull files at random (unbounded) → *M* = 2 (open the menu, open the review
    group) → *K* = 2.
- **Time + cognitive load:** the menu opens ≤ 1 s with 20 supervisees and a week of notes (design §6); the picker's
  counts land ≤ 1 s after the list draws (Q1); co-sign from the opened note ≤ 3 s (Sign's budget, RUNBOOK §16/§23).
  Cognitive load: the count names only obligations, the review list is opt-in and collapsed, and no decision is
  forced. A human owes the measurement in **RUNBOOK §26** (Task 7). The menu is a READ, and db/044's timing kinds
  are all writes (`note-sign`, `note-sign-off`, …), so its ≤ 1 s is measured by stopwatch only; no db/044 widening.
  The co-sign from an opened note is already timed as `note-sign-off`.

---

### Task 1: db/073 — the two read doors

**Files:**
- Create: `db/073_person_menu.sql`, `db/tests/073_person_menu_test.sql`
- Modify: `crates/cairn-node/src/db.rs` (`SCHEMA` list), `crates/cairn-event/src/schema_generation.rs` (73),
  `crates/cairn-node/tests/window_role_grants_notes.rs`, `crates/cairn-node/tests/floor_execute_grants.rs`
- Test: `crates/cairn-node/tests/note_menu_doors.rs`

**Interfaces:**
- Produces:
  - `cairn_note_people() RETURNS TABLE(actor_id text, staff_id text, display_name text, keys text[])`: one row per
    actor that is a `vault_member` OR appears in `cairn_supervision_record()` as subject or related. `actor_id` is
    lower hex. `staff_id` is `cairn_supervision_staff_of(actor)` (the pinned handle when the member row is gone).
    `display_name` is the member's, else NULL. `keys` holds every `signing_key_id` of that actor's `enroll` and
    `rotate-key` rows, repudiated ones included, sorted. STABLE SECURITY DEFINER, `search_path = public, pg_temp`.
  - `cairn_note_drafts_by_writer() RETURNS TABLE(writer text, draft_id uuid, patient_id uuid, due timestamptz,
    stuck boolean)`: one row per (open draft, distinct non-NULL writer of a part that passes
    `cairn_twin_is_present(narrative)`). `due` and `stuck` are exactly `cairn_note_drafts_due`'s expressions (copy
    them; do not call the per-patient door in a loop). No text, no terminal.
  - Both are REVOKEd from PUBLIC and GRANTed to `cairn_window` only.

- [ ] **Step 1: Write the failing DB tests** in `note_menu_doors.rs`, using `tests/common/vault_enroller_kit.rs` to
  enrol staff:
  - `people_lists_a_member_with_every_key_it_ever_held`: enrol A, `rotate-key` A → one row for A, `keys.len() == 2`,
    both keys present.
  - `people_lists_a_supervisor_whose_member_row_is_gone`: link B under A, then delete A's `vault_member` row as the
    owner → A's row is present with `display_name IS NULL` and `staff_id` = A's pinned handle.
  - `drafts_by_writer_lists_each_writer_once_and_skips_blank_parts`: a draft with parts by A (text), B (blank) and an
    unknown writer → exactly one row, `writer = A`.
  - `drafts_by_writer_due_agrees_with_the_per_chart_door`: for the same draft, `due` and `stuck` equal
    `cairn_note_drafts_due(patient)`'s.
  - `the_window_may_call_both_and_public_may_not`: as a `cairn_window` member probe, both succeed; as a role with no
    grant, both fail with SQLSTATE `42501`.
- [ ] **Step 2: Run them; expect FAIL** (`function cairn_note_people() does not exist`).
  `cargo test -p cairn-node --test note_menu_doors -- --nocapture`
- [ ] **Step 3: Write `db/073_person_menu.sql`** (header comment: why each door exists, the window having no grant on
  `vault_member` or `actor_event`'s actor columns, per db/062); add it to `db.rs`; bump `SCHEMA_GENERATION` to 73;
  add the db/tests mirror (`\ir _scratch_database_guard.sql` first); extend the two grant guards.
- [ ] **Step 4: Run Step 1's suite, `floor_execute_grants`, `window_role_grants_notes` and `search_path_pg_temp`;
  expect PASS** with no `skipped:` line.
- [ ] **Step 5: Commit** `feat(menu): db/073 — the people and drafts-by-writer doors`.

### Task 2: The chart list's query, read by keys

**Files:**
- Modify: `crates/cairn-node/src/note/read.rs`, `crates/cairn-node/src/note/supervision.rs`
- Test: `crates/cairn-node/tests/note_parts_by_keys.rs`, the `supervision.rs` unit tests

**Interfaces:**
- Consumes: Task 1's doors (none directly).
- Produces:
  - `pub async fn parts_by_keys(client: &(impl GenericClient + Sync), keys: &[String], sup: &Supervision) ->
    anyhow::Result<Vec<NoteRow>>` in `read.rs`: every `clinical_note` part whose `e.signer_key_id = ANY($1)` OR any
    `e.contributors` element's `actor_id = ANY($1)`, each annotated (`supervision::annotate`). Unordered; the caller
    orders.
  - `NOTES_SQL` becomes `notes_sql!("<where clause>")`, a macro holding today's column list and joins, so the chart's
    query and the key query differ ONLY in their `WHERE`. `list_patient_notes`'s per-row body moves into
    `fn note_row(r: &Row, sup: &Supervision) -> anyhow::Result<NoteRow>`, shared by both. No behaviour change for the
    chart: its existing suites are the regression test.
  - `Supervision::supervisees_now(&self, supervisor: &str) -> Vec<String>`: every actor whose `supervisors_now`
    contains `supervisor`, sorted, deduplicated.

- [ ] **Step 1: Write the failing tests.**
  - Unit (`supervision.rs`): `supervisees_now_lists_each_standing_supervisee_once`: link B→A, C→A, unlink C→A, link
    C→A again, link D→E → `supervisees_now("a") == ["b","c"]`, and `supervisees_now("e") == ["d"]`.
  - DB (`note_parts_by_keys.rs`, using `tests/common/note_vouch_kit.rs`):
    - `a_signed_part_is_found_by_its_signers_key`;
    - `a_part_that_went_in_unsigned_is_found_by_its_drafted_writers_key` (the quarantine commit's node-signed part);
    - `a_part_signed_with_a_since_rotated_key_is_found_by_that_key`;
    - `another_writers_part_on_the_same_chart_is_not_returned`;
    - `a_part_read_by_keys_equals_the_same_part_read_by_chart` (same `NoteRow`, field for field, including
      `vouches`, `needs_cosigning` and `writer_supervisors`).
  - Existing #412 guard in `read.rs`: extend it to assert neither expansion of `notes_sql!` contains `attester_key`.
- [ ] **Step 2: Run; expect FAIL** (no `parts_by_keys`, no `supervisees_now`).
- [ ] **Step 3: Implement** the macro, `note_row`, `parts_by_keys` and `supervisees_now`. The key query's `WHERE` is
  `e.signer_key_id = ANY($1::text[]) OR EXISTS (SELECT 1 FROM jsonb_array_elements(e.contributors) c WHERE c ->>
  'actor_id' = ANY($1::text[]))`. Add the comment `// shortcut: scans every clinical_note row; upgrade to a
  projection of open obligations if RUNBOOK §26 misses its 1 s budget`.
- [ ] **Step 4: Run the new suites plus every existing note DB suite** (`note_*.rs`, `cairn-gui-tauri`'s
  `notes_*_db_tests`); expect PASS.
- [ ] **Step 5: Commit** `refactor(note): one note query, by chart or by keys; supervisees_now`.

### Task 3: `note::menu` — the three groups (pure)

**Files:**
- Create: `crates/cairn-node/src/note/menu.rs`, `crates/cairn-node/src/note/menu_tests.rs`
- Modify: `crates/cairn-node/src/note/mod.rs` (module + its line in the doc list), `crates/cairn-node/src/note/review.rs`
  (`reviewable` becomes `pub`, doc unchanged)

**Interfaces:**
- Consumes: `NoteRow`, `Supervision`, `vouch::{counts_as_unsigned, part_is_signed, same_actor}`,
  `cosign::awaits_cosign`, `supervision::writer_actor`, `review::reviewable`.
- Produces:
  ```rust
  pub const REVIEW_DAYS: i64 = 7;
  pub struct Person { pub actor: String, pub staff_id: String, pub display_name: Option<String>, pub keys: Vec<String> }
  pub struct MenuDraft { pub writer: String /* staff id */, pub draft_id: Uuid, pub chart: Uuid,
                         pub due: DateTime<Utc>, pub stuck: bool }
  pub enum Why { Draft { draft_id: Uuid, due: DateTime<Utc>, stuck: bool }, WentInUnsigned, KeyRepudiated,
                 NeedsCoSign { others: Vec<String> /* actors */ }, Review }
  pub struct MenuItem { pub chart: Uuid, pub part: Option<Uuid> /* None for a draft */,
                        pub when: DateTime<Utc> /* recorded_at, or a draft's due */,
                        pub writer: Option<String> /* actor, groups 2–3 */, pub why: Why }
  pub struct PersonMenu { pub own: Vec<MenuItem>, pub cosign: Vec<MenuItem>, pub review: Vec<MenuItem> }
  impl PersonMenu { pub fn count(&self) -> usize }          // own + cosign, never review
  pub fn classify(me: &Person, parts: &[NoteRow], drafts: &[MenuDraft], now: DateTime<Utc>) -> PersonMenu
  ```
  `classify` applies Global Constraints' group rules and order exactly. A part may qualify for one group only, checked
  in the order own → co-sign → review. A `me` whose actor is empty gets an empty menu.

- [ ] **Step 1: Write the failing tests** in `menu_tests.rs`, built on `vouch::fixture::{part, who, repudiated,
  student, vouch}` and `supervision::annotate` over a hand-built `Supervision`:
  - `a_draft_with_my_part_is_in_my_group_and_counted`;
  - `a_part_that_went_in_unsigned_is_mine_until_anyone_signs_it` (a colleague's attested sign-off removes it);
  - `a_part_signed_with_my_repudiated_key_is_mine_until_i_re_sign` (a colleague's co-sign does NOT remove it; my vouch
    under a standing key does);
  - `a_repudiated_part_is_in_nobodys_cosign_group`;
  - `a_supervisees_part_awaiting_cosign_is_in_my_cosign_group_naming_the_others` (`others == ["p"]` for a second
    supervisor P; empty when I am the only one);
  - `a_requirement_lifted_after_signing_keeps_the_part_waiting` (Review Focus 1);
  - `a_supervisor_who_stepped_away_sees_nothing_of_that_supervisee` (Review Focus 2);
  - `a_cosign_by_a_since_unlinked_supervisor_still_clears_it_for_the_other` (Review Focus 2);
  - `a_writer_with_no_supervisor_now_is_in_nobodys_cosign_group`;
  - `a_reviewable_part_is_listed_for_seven_days_and_never_counted` (at `now - 7d` listed; at `now - 7d - 1ms` not;
    `count()` unchanged by it) (Review Focus 4);
  - `a_part_one_supervisor_reviewed_leaves_every_supervisors_review_group`;
  - `my_own_signed_part_is_in_none_of_my_groups`;
  - `rows_are_ordered_drafts_by_due_then_parts_newest_first`;
  - `a_person_with_no_actor_gets_an_empty_menu`.
- [ ] **Step 2: Run; expect FAIL** (`no module menu`). `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib note::menu`
- [ ] **Step 3: Implement `menu.rs`.** Its module doc explains to a junior why each group's predicate is the chart's
  own (so the menu and the byline can never disagree), and why a group-3 part is never counted.
- [ ] **Step 4: Run; expect PASS.** Then run the whole `note::` lib suite.
- [ ] **Step 5: Commit** `feat(menu): note::menu — the three groups and the count, pure`.

### Task 4: The menu read

**Files:**
- Create: `crates/cairn-node/src/note/menu_read.rs`
- Modify: `crates/cairn-node/src/note/mod.rs`
- Test: `crates/cairn-node/tests/note_person_menu.rs`

**Interfaces:**
- Consumes: Task 1's doors, Task 2's `parts_by_keys` + `supervisees_now`, Task 3's types.
- Produces:
  ```rust
  pub struct MenuRead { pub people: Vec<Person>, pub drafts: Vec<MenuDraft>, pub parts: Vec<NoteRow> }
  /// `Some(staff)`: the person's keys and their current supervisees' keys only (the bar).
  /// `None`: every person's keys (the picker's counts, Q1).
  pub async fn read_menus(client: &(impl GenericClient + Sync), scope: Option<&str>) -> anyhow::Result<MenuRead>
  impl MenuRead {
      pub fn person(&self, staff_id: &str) -> Option<&Person>          // by cairn_vault_staff_id-normalised id
      pub fn menu_for(&self, staff_id: &str, now: DateTime<Utc>) -> Option<PersonMenu>
      pub fn counts(&self, now: DateTime<Utc>) -> Vec<(String, usize)> // every person with a staff id
      pub fn name_of(&self, actor: &str) -> Option<&str>
  }
  ```
  The order of reads inside `read_menus`: people → `supervision::load` → drafts → `parts_by_keys`, all on the
  caller's client (the caller supplies the snapshot). A staff id with no person → `menu_for` answers `None`.

- [ ] **Step 1: Write the failing DB tests** (`note_person_menu.rs`; real enrolments, a real quarantine commit, a real
  repudiation through the existing kits):
  - `a_student_part_waits_in_each_supervisors_group_until_one_cosigns`;
  - `require_sign_lift_keeps_the_part_waiting_and_counted` (Review Focus 1);
  - `a_part_gone_in_unsigned_is_in_its_writers_group_until_signed`;
  - `a_repudiated_part_is_in_the_real_persons_group_until_re_signed` (Review Focus 3);
  - `an_open_draft_is_in_its_writers_group_with_the_doors_due_time`;
  - `the_bar_read_and_the_all_read_give_the_same_menu_for_the_person` (`read_menus(Some(a))` vs `read_menus(None)`,
    both `menu_for(a)` equal) (Review Focus 4);
  - `counts_equal_each_persons_menu_count`.
- [ ] **Step 2: Run; expect FAIL.** `cargo test -p cairn-node --test note_person_menu -- --nocapture`
- [ ] **Step 3: Implement `menu_read.rs`.**
- [ ] **Step 4: Run; expect PASS**, no `skipped:` line. Update `db_errors_stay_legible.rs` if it counts the new reads.
- [ ] **Step 5: Commit** `feat(menu): read_menus — one read for the bar, one for the picker`.

### Task 5: The window's commands and words

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/person_menu.rs` (commands + `*_impl`), `src/person_menu_view.rs` (pure words),
  `src/person_menu_tests.rs`, `src/person_menu_db_tests.rs`
- Modify: `src/main.rs` (module + `generate_handler!`), `src/notes_compound.rs` (`CompoundView::part_ids:
  Vec<String>`, every part's event id in part order), `src/notes_parts.rs` (each draft view already carries its
  `draft_id`; confirm, else add it)

**Interfaces:**
- Consumes: Task 4's `read_menus`, `MenuRead::{menu_for, counts, name_of}`; `candidates_by_id`; `open_chart_impl`;
  `notes_commit::due_words`; `view::instant`; the window's `in_snapshot`.
- Produces:
  ```rust
  #[derive(serde::Serialize)] pub struct MenuRowView { pub patient: String, pub what: String, pub when: String,
      pub state: String, pub open_label: String, pub patient_id: String, pub focus: String /* "part:<id>" | "draft:<id>" */ }
  #[derive(serde::Serialize)] pub struct MenuGroupView { pub heading: String, pub empty: Option<String>, pub rows: Vec<MenuRowView> }
  #[derive(serde::Serialize)] #[serde(tag = "kind")] pub enum PersonMenuView {
      Nobody, Menu { count: usize, count_words: String, count_label: String, own: MenuGroupView,
                     cosign: MenuGroupView, review: MenuGroupView } }
  pub fn menu_view<Tz: TimeZone>(menu: &PersonMenu, read: &MenuRead, patients: &HashMap<Uuid, Candidate>,
                                 tz: &Tz, now: DateTime<Utc>) -> PersonMenuView           // pure
  #[tauri::command] person_menu() -> Result<PersonMenuView, ErrorView>
  #[tauri::command] person_menu_open(patient_id: String) -> Result<ChartHeaderView, String>
  #[tauri::command] whos_here_counts() -> Result<Vec<(String, usize)>, ErrorView>
  ```
  `person_menu_impl`: claim not `Named` → `Nobody` without touching the database. Otherwise, in one `in_snapshot`:
  `read_menus(Some(staff))`, then `candidates_by_id` for every row's chart, then insert each candidate into
  `state.shown`. `person_menu_open_impl` re-reads the menu the same way; if no row has that `patient_id` it answers the
  exact "no longer waits" copy and sets no chart; otherwise it calls `open_chart_impl`. `--mock` (no database): `Nobody`
  if nobody is named, else an empty `Menu` with count 0; `whos_here_counts` answers `[]`.

- [ ] **Step 1: Write the failing tests.**
  - Pure (`person_menu_tests.rs`): every exact string in Global Constraints from `menu_view` over a hand-built
    `PersonMenu`, including `"you or A, B or C"` joining, the unnamed writer, the empty group, and `count_words` for
    0 and 3; `nobody_named_reads_no_database` (`AppState::mock` with the claim `NotAsked` and with `Unknown` →
    `Nobody`).
  - DB (`person_menu_db_tests.rs`, with `cairn-gui-tauri`'s existing DB fixture):
    `a_row_that_vanished_is_refused_and_opens_no_chart` (read the menu, co-sign the part through the node, call
    `person_menu_open` → the exact copy, `state.chart` still `None`) (Review Focus 5);
    `open_note_opens_the_rows_chart`; `the_pickers_count_equals_the_bars` (Review Focus 4).
  - `notes_compound_tests.rs`: `part_ids_lists_every_part_in_order`.
- [ ] **Step 2: Run; expect FAIL.**
- [ ] **Step 3: Implement.** The words live in `person_menu_view.rs` only; the commands are thin, as `notes.rs`'s are.
- [ ] **Step 4: Run** `cargo test --bins` in `cairn-gui/` with the DB env (and `CAIRN_ALLOW_DB_SKIP=1` for a DB-free
  pass), plus clippy and `cargo doc` with `RUSTDOCFLAGS=-D warnings`. Expect PASS.
- [ ] **Step 5: Commit** `feat(window): the person menu's commands and words`.

### Task 6: The window — the bar, the menu, "Open note", the picker's counts

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/person_menu.js`, `src/person_menu_webview_tests.rs`
- Modify: `src-ui/index.html` (in `#whos-here-bar`: `<details id="person-menu" hidden><summary
  id="person-menu-count"></summary><div id="person-menu-body"></div></details>`; the script tag),
  `src-ui/whos_here.js` (`takeClaim` and `showClaim` call `refreshMenuCount()` when the staff id changes;
  `openPicker` fills in the counts), `src-ui/notes.js` (`readNotes` calls `refreshMenuCount()` after drawing;
  `renderNotes` honours `pendingMenuFocus`), `src-ui/notes_compound.js` (`li.dataset.parts =
  compound.part_ids.join(" ")`), the draft item (`dataset.draft`), `src-ui/funnel.js` (`closeChart` calls
  `refreshMenuCount()`), `src-ui/style.css`, `src/commands_tests.rs` + `src/waiting_webview_tests.rs` (script lists)

**Interfaces:**
- Consumes: Task 5's three commands and views; `closeChart`, `enterChart`, `signInFlight`, `signsSettled`,
  `claimedStaffId`.
- Produces (JS, `person_menu.js`): `refreshMenuCount()`, `drawMenu(view)`, `openMenuRow(row)`, `takeMenuFocus()`
  (returns and clears `pendingMenuFocus`), `fillPickerCounts(entries)`.

  Behaviour: one revision counter per read kind (the `worklist.js` pattern, #752), so an older answer never draws over
  a newer one. `Nobody` → `#person-menu` hidden. The summary shows `count_words` with `aria-label` `count_label`. The
  review group is a nested `<details>`, closed by default and never auto-opened. Opening `#person-menu` re-reads the
  menu. `openMenuRow`: `if (signInFlight()) await signsSettled();` → `if (displayedPatient) await closeChart();` →
  `invoke("person_menu_open", {patientId})` → `enterChart(header)` with `pendingMenuFocus = row.focus`. A refusal is
  shown in the menu's own status line and the menu re-reads. After the note pane draws, the element whose
  `data-parts` contains the id (or `data-draft` equals it) gets `scrollIntoView()` and focus (`tabIndex = -1`). A
  60 s `setInterval(refreshMenuCount, 60_000)` keeps the count current while nobody acts (the quarantine commit
  moves drafts into group 1 by itself).

- [ ] **Step 1: Write the failing webview/source tests** (`source_guard::js_fn`, as the other `*_webview_tests.rs`):
  `open_menu_row_settles_signs_and_closes_the_chart_before_opening` (order of the three calls) (Review Focus 5);
  `the_review_group_is_a_closed_details`; `the_menu_is_hidden_for_nobody`; `stale_menu_answers_are_dropped` (the
  revision counter is checked before drawing); `the_picker_draws_before_counts_land`; the script-list counts.
- [ ] **Step 2: Run; expect FAIL.**
- [ ] **Step 3: Implement.** No string the clinician reads is written in JS except `"Waiting counts could not be
  read."` (a failure Rust cannot word, as `loadTray`'s is).
- [ ] **Step 4: Walk it** with the mock walk recipe (memory: "webview mock walk recipe"; stub `invoke` with Rust-shaped
  payloads for a three-group menu). Assert VISIBILITY (computed display, no hidden ancestor), not only `textContent`.
  Check keyboard-only: Tab reaches the summary, Enter opens it, Tab reaches each "Open note". Then run the GUI tree's
  tests, clippy and `cargo doc`, plus the ROOT source guards DB-free. Expect PASS.
- [ ] **Step 5: Commit** `feat(window): "Waiting for you" — the person menu, Open note, the picker's counts`.

### Task 7: ADR-0095, the spec, the runbook, the durable rules

**Files:**
- Create: `docs/spec/decisions/0095-the-person-menu.md`
- Modify: `docs/spec/index.md` (v0.97), the spec section that carries §5.11's sign-in prose (the 3c-3a ADR's home
  section; ADR-0094 names it), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav line),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (§26) + `TEMPLATE.md` (§26 section),
  `docs/developers/durable-rules.md` (a "sign-in 3c-3c" block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0095.** Decisions: whose menu (a claim); what counts; each group's predicate is the chart's
  own; the review window is 7 days; the two doors and why each is a definer; the menu reads no sealed body (Q2 as
  ruled); "Open note" re-reads and keeps the wrong-chart rule; the picker's counts (Q1 as ruled) and the shortcut;
  ADR-0092's under-claim (c) ends. Not guaranteed: the count is as fresh as its last read (≤ 60 s, or the last act);
  a draft on another node is not listed (#963); the menu spans this node's charts only.
- [ ] **Step 2: Fact-check the ADR sentence by sentence against the code** (memory: "SDD evidence + ADR fact-check").
  An independent reviewer does this, never the writer.
- [ ] **Step 3: RUNBOOK §26:** the menu opens ≤ 1 s with 20 supervisees and a week of notes; the picker's counts
  land ≤ 1 s; Open note → the note focused; co-sign from it ≤ 3 s; the count leaves out the review group; keyboard
  and VoiceOver (the summary's name, each row's "Open note" name); WebKit.
- [ ] **Step 4: The durable-rules block:** the menu's groups use the chart's predicates and must never grow their
  own; never count group 3; `person_menu_open` re-reads, never trusts a drawn row.
- [ ] **Step 5: `uv run --with-requirements docs/requirements.txt -- mkdocs build`**, then the paper-parity plan
  guard (`cargo test -p cairn-node --test paper_parity_plan_section`) and `scripts/check_closing_keywords.py`. Expect
  PASS.
- [ ] **Step 6: Commit** `docs(menu): ADR-0095 — the person menu; spec v0.97, RUNBOOK §26`.
