# GUI sign-in, slice 3a — the text progress note and its autosave — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician opens a chart, presses **New note**, types a text progress note (optionally stating when it
happened), and signs it with staff ID + passphrase. It goes into the record attested, sealed, and listed on the
chart with its grade. Everything typed is saved to the node as it is typed, so a crash or disconnect loses
nothing.

**Architecture:** A new born-sealed clinical stream `clinical.note.asserted` (`clinical.note/1`, ADR-0041's
text-only subset) with a floor check, a twin-registry row and a one-row-per-event projection (`db/064`). A mutable
draft store reached only through `SECURITY DEFINER` doors (`db/065`). Signing spends the draft and submits the
note in **one transaction**, and a deferred constraint makes a spend without its note impossible. `cairn-node`
gains a `note` module (body builder, sign, read, draft calls). The Tauri window gains three commands and one
script, `notes.js`.

**Tech Stack:** PostgreSQL 18 (new `db/064`, `db/065`; in-place edits to `db/037`, `db/044`), Rust (`cairn-event`,
`cairn-node`), Tauri 2 + plain JS (`cairn-gui-tauri`, no npm).

**Spec:** [`docs/superpowers/specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md`](../specs/2026-11-03-sign-in-slice-3-notes-drafts-and-whos-here-design.md)
(PR #930, merged). Read §2, §3.2, §4 and §5's 3a row, then ADR-0041 in full, before Task 1. **This plan covers
PR 3a only.** 3b and 3c get their own plans, written against what 3a built. The design's §10 findings all touch
3b/3c and are deferred to those plans (maintainer, 2026-11-03).

## Global Constraints

- **Wire shape is frozen once the first note is signed** (ADR-0041 Context). Payload of `clinical.note.asserted`,
  `schema_version = "clinical.note/1"`, exactly these keys:
  `narrative` (string), `media` (`[]`), `refs` (`[]` from this window), `encounter` (`null`),
  `recording_started`, `recording_ended` (RFC 3339 UTC strings stamped by the node's clock),
  `effective_basis` (`"stated"` | `"recording-started"`).
  Envelope `t_effective` is **always set**: the writer's stated time, or `recording_started` when they stated none.
  Contributors: `[{human, "authored"}, {node, "recorded"}]` via the existing `apply_author` (ADR-0053).
- **The floor refuses only what ADR-0041 excludes forever, plus what it makes integrity.** That is: raw HTML,
  non-`cairn:` link targets, `cairn:event/` anchors (not valid in `/1`), media anchors without a matching
  manifest entry, an unknown `refs.rel`, a `refs.event` not in `event_log`, an empty narrative, and a malformed
  time or basis. It refuses nothing a v1 profile merely leaves out (tables, code blocks, footnotes): those render
  as literal text, which ADR-0041 decision 5 already calls degraded but honest.
- **Non-empty `media` is refused at the authoring door only** (`cairn.remote_apply` ≠ `'on'`, the db/031
  `inn_code` idiom). At the remote door the general anchor↔manifest rule applies instead, so a future media-capable
  peer is never wedged (ADR-0056).
- **A clinician is never refused for typing.** The window escapes markup before it signs
  (`cairn_event::note::escape_note_markup`). The floor's refusal is for raw-SQL clients and is never shown for
  text typed in the window.
- **Draft prose is reachable only through definer doors:** `note_draft` and `note_draft_spent` get
  `REVOKE ALL … FROM PUBLIC` and no table grant to anyone. The doors are
  `SECURITY DEFINER SET search_path = public, pg_temp`, revoked from PUBLIC and granted to **`cairn_window`
  only** (design §4.2). Drafts are **not** in `backup` (a draft is not the record). The ADR states it.
- **The window never sends a time for recording.** `recording_started` and `last_keystroke` are stamped by
  `clock_timestamp()` in the save door. The only time the window sends is the writer's *stated* time.
- **Linked charts read as one, as the med list does** (ADR-0076). Notes on a chart other than the opened one
  are shown only when the set holds no doubted link (`set_has_doubted_link`, R1b). Otherwise they are counted
  and withheld, with a sentence saying so.
- **`note.added` is not retired** (design §4.3). File the retirement issue in Task 7.
- **`clinical.note.asserted` stays OFF db/048's `cairn_event_type_has_no_thread` list.** It keeps the
  conservative sensitivity bound. Pin that in `db/tests/064`, and do not edit the list.
- **Replay traps** (repo memory "migration replay"): every `db/*.sql` replays on a schema change, so (a) the
  `ui_gesture_timing_kind_ck` widening goes **in db/044 in place**: both its `CREATE TABLE` line and its
  drop/re-add. A later file's widening would be undone by 044's replay, which then fails on a stored
  `note-sign` row. (b) The shred hook goes **in db/037's `cairn_execute_shred` in place**, guarded by
  `to_regclass`. (c) A door's return shape is final: add outcomes, never columns.
- **House rules:** TDD; every non-trivial fn documented for a junior reader; files under 500 lines
  (`sealed_submit.rs` is 477: Task 4's extraction must not grow it, so move its tests or the new fn out if
  needed); no `salt`/`nonce`/`iv` names; `scripts/check_closing_keywords.py` before every commit; DB-gated tests
  must show no `skipped:` line (`--nocapture`). A DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`.
- **Pinned guards a new member must join** (memory "pinned-count guards"), with today's values:
  `db/tests/034_twin_registry_test.sql:34` (26 → 27) · `crates/cairn-node/tests/twin_registry.rs:103` (26 → 27,
  plus an `expected` vec entry) · `db/tests/039_projection_registry_test.sql:32` (32 → 33) ·
  `crates/cairn-node/tests/projection_registry.rs:65` (30 → 31) · `cairn-node/src/db.rs` `SCHEMA` list +
  `SCHEMA_GENERATION` 63 → **65** (`crates/cairn-event/src/schema_generation.rs:45`) · `late_custody_guards.rs`
  (the applier is `heal_safe = TRUE`) · `floor_execute_grants.rs` (the floor fn is revoked from PUBLIC) ·
  `db_errors_stay_legible.rs:720` `MARKED_COMMIT_SITES` (+ `note/sign.rs`) · `window_role_grants.rs:312`
  (every new entry point driven to its deep path under `cairn_window`) · `commands_tests.rs:367` script list ·
  `signer_webview_tests.rs:21` `WRITERS` (7 → 8) · `reread_webview_tests.rs:503` · `node_lock_explains_tests.rs`.

## Review Focus

1. **Text clinicians really type that looks like markup:** `BP <90`, `<5 min`, `pt said "<b>"`, `[L] arm (R)`,
   a trailing backslash, a URL pasted from a discharge letter. Expected: signed exactly as typed, never refused,
   and shown back exactly as typed. Pinned in Task 1 (escape round-trip corpus) and Task 2 (the floor accepts
   every escaped corpus line).
2. **The connection drops while typing:** the text stays in the editor, a visible "not saved yet — your text is
   still here" line appears, the next keystroke retries, and Sign says plainly that it cannot sign right now.
   Pinned in Task 6. No `localStorage` copy: that would put draft prose on disk in the clear.
3. **A debounced save fires after Sign was pressed:** it must not resurrect a draft. The save answers `spent`
   and writes nothing (Task 3), and the window cancels the pending timer on Sign (Task 6).
4. **Sign racing the last keystroke:** the note signs the text sent **with** Sign, not the last autosave. Pinned
   in Task 4.
5. **A draft saved against a different chart from the one now on screen** (the clinician switched charts mid-
   debounce): the save names its own draft's patient, and a mismatch answers `elsewhere`. Sign refuses unless the
   draft's chart is the displayed one. Pinned in Tasks 3 and 5.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the progress sheet in the bedside folder: pick it up, write (heading the entry with the
  time it happened, if not now), sign.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Reading the notes on a chart: 0 → 0 → 0 (listed on the chart; no name asked).
  - Writing and signing a note: 3 (pick up the sheet, write, sign) → 3 (press **New note**, type, sign: staff ID +
    passphrase + press, slice 1's strip) → 3. Stating a time is part of writing, as on paper, and optional.
  - Getting a half-written note back after the window crashed: paper has no crash. The architecture forces 0
    extra steps: the chart reopens with the draft in the editor. Target 0.
  - `M = N` everywhere: no architecture defect to file.
- **Time + cognitive load:** sign ≤ 3 s (RUNBOOK §16's figure, timed through `record_timing("note-sign", 1, …)`);
  opening the editor ≤ 1 s; autosave has no visible step and no prompt. One field to type into, one optional
  time, and the grade in words. Measurement is owed by a human in **RUNBOOK §18** (Task 7): this PR is the first
  runnable note surface.

---

### Task 1: The note body, its twin and the markup escape (`cairn-event`, pure)

**Files:**
- Create: `crates/cairn-event/src/note.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`pub mod note;` beside `pub mod medication;`, line 57)
- Test: in-file `#[cfg(test)] mod tests` (pure, no DB)

**Interfaces:**
- Produces:
  - `pub const NOTE_EVENT_TYPE: &str = "clinical.note.asserted";` and
    `pub const NOTE_SCHEMA_VERSION: &str = "clinical.note/1";`
  - `pub enum StatedTime<'a> { Stated(&'a str), NotStated }`: the writer's claim, RFC 3339.
  - `pub struct NoteAssertion<'a> { pub narrative: &'a str, pub stated: StatedTime<'a>, pub recording_started: &'a str, pub recording_ended: &'a str }`
  - `pub fn note_payload(a: &NoteAssertion) -> serde_json::Value`: the payload exactly as in Global Constraints.
  - `pub fn note_t_effective(a: &NoteAssertion) -> String`: the stated time, else `recording_started`.
  - `pub fn render_note_twin(a: &NoteAssertion) -> String`: the narrative verbatim (ADR-0041 decision 4 with no
    media).
  - `pub fn escape_note_markup(text: &str) -> String`: backslash-escapes every `<`, and every `(` or `:` that
    directly follows `]` (ruling R3), and doubles every existing `\`. Drafts hold the raw text; only signing escapes it.
  - `pub fn unescape_note_markup(text: &str) -> String`: removes a backslash before ASCII punctuation, so the
    window shows what was typed.

- [ ] **Step 1: Write the failing tests**
  - `payload_has_exactly_the_frozen_keys`: keys are `{narrative, media, refs, encounter, recording_started,
    recording_ended, effective_basis}`; `media == []`, `refs == []`, `encounter == null`.
  - `no_stated_time_uses_recording_start_and_says_so`: `NotStated` → `effective_basis == "recording-started"`
    and `note_t_effective == recording_started`.
  - `a_stated_time_is_kept_verbatim`: `Stated("2026-11-03T03:00:00Z")` → basis `"stated"`, t_effective equals it.
  - `twin_is_the_narrative`: `render_note_twin(a) == a.narrative`.
  - `escape_corpus_round_trips`: for each of `["BP <90", "<5 min", "pt said \"<b>\"", "[L] arm (R)",
    "see [x](http://e.org)", "ends with \\", "a\\<b", "plain"]`,
    `unescape_note_markup(&escape_note_markup(s)) == s`.
  - `escaped_text_has_no_unescaped_tag_or_link`: for the same corpus, the escaped text matches neither
    `(^|[^\\])(\\\\)*<[A-Za-z/!?]` nor `(^|[^\\])(\\\\)*\]\(` (use the `regex` crate only if the workspace already
    depends on it; otherwise a small hand-written scan is the test helper).

- [ ] **Step 2: Run:** `cargo test -p cairn-event note::` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** `note.rs`. Escape scans chars once: emit `\` before `<`, and before `(` when the
  previous emitted char was an unescaped `]`. A pre-existing backslash is itself escaped (`\` → `\\`) so the
  round trip holds.
- [ ] **Step 4: Run:** `cargo test -p cairn-event note::` — Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): the clinical.note/1 body, twin and markup escape (slice 3a)`

### Task 2: The note stream's floor and projection (`db/064_clinical_note.sql`)

**Files:**
- Create: `db/064_clinical_note.sql`, `db/tests/064_clinical_note_test.sql`
- Modify: `db/037_born_sealed.sql` (`cairn_execute_shred`, in place: one guarded
  `DELETE FROM clinical_note WHERE content_address = v_ca`, beside `medication_statement`'s at `:330`)
- Modify: `crates/cairn-node/src/db.rs` (`SCHEMA` list), `crates/cairn-event/src/schema_generation.rs` (→ 64 here;
  Task 3 takes it to 65)
- Modify the four pinned counts (Global Constraints) and `twin_registry.rs`'s `expected` vec
- Test: `db/tests/064_clinical_note_test.sql` (needs `\ir _scratch_database_guard.sql`), and a Rust round trip in
  `crates/cairn-node/tests/note_floor.rs`

**Interfaces:**
- Produces (SQL):
  - `event_type_class` row `('clinical.note.asserted', 'additive', FALSE)`.
  - `cairn_check_note_assertion(p_type text, b jsonb) RETURNS void`, revoked from PUBLIC, registered in
    `cairn_event_twin_check` with the #214 `ON CONFLICT DO UPDATE … WHERE … IS DISTINCT FROM` idiom (copy db/031:79).
  - `clinical_note(event_id uuid PRIMARY KEY, patient_id uuid NOT NULL, narrative text NOT NULL, refs jsonb NOT
    NULL, t_effective timestamptz NOT NULL, effective_basis text NOT NULL, recording_started timestamptz NOT NULL,
    recording_ended timestamptz NOT NULL, hlc_wall bigint, hlc_counter int, origin text, content_address bytea
    NOT NULL)`, an index on `content_address` and one on `patient_id`, `GRANT SELECT … TO cairn_agent` (the
    medication posture, db/031:236).
  - `clinical_note_apply(e event_log) RETURNS void`: reads `cairn_clear_payload(e)`, returns on NULL, inserts
    `ON CONFLICT (event_id) DO NOTHING`. Registered in `cairn_projection_apply` as
    `('clinical.note.asserted', 'clinical_note_apply', ARRAY['clinical_note'], 20, TRUE)`.
- The floor's rules, in order, each its own `RAISE EXCEPTION 'note assertion: …'` (no `USING ERRCODE`):
  1. `schema_version` is `clinical.note/1`.
  2. `narrative` is a string, non-empty after `btrim`.
  3. No unescaped raw HTML: `narrative ~ '(^|[^\\])(\\\\)*<[A-Za-z/!?]'` refuses (comment why `<9` and `< b` pass).
  4. Every unescaped link or image `](target)` has a target starting `cairn:att/`; `cairn:event/` is refused by
     name ("not valid in clinical.note/1, ADR-0041 decision 9").
  4a. No unescaped link reference definition or reference: `narrative ~ '(^|[^\\])(\\\\)*\]:'` refuses
     (ruling R3: `[ref]: http://…` is a load-bearing external URL; the window's escape emits `]\:`).
  5. `media` is an array. Authoring door: it must be empty. Both doors: every `![text](cairn:att/<id>)` anchor
     matches exactly one entry whose `id = <id>` and whose `descriptor` is non-empty and equal to `text`.
  6. `refs` is an array of `{rel, event}` with `rel IN ('addendum-to','correction-of','transcript-of')` and
     `event` a uuid present in `event_log` (both doors, ADR-0041 decision 8).
  7. The `encounter` key is present; authoring door: `null`.
  8. `recording_started` and `recording_ended` cast to `timestamptz`, with started ≤ ended.
  9. `effective_basis IN ('stated','recording-started')`; the envelope `t_effective` (`b ->> 't_effective'`) is
     non-null; with `recording-started` it equals `recording_started` (compare as `timestamptz`).

- [ ] **Step 1: Write the failing SQL test** `db/tests/064_clinical_note_test.sql`: one accepted body; then one
  refused body per rule above, each asserting its message fragment. Also accepted: the escaped Task 1 corpus
  lines, `"BP <90"` unescaped, a heading and a list. At the remote door (`SET LOCAL cairn.remote_apply = 'on'`
  inside the test's own transaction, as db/tests/031 does), a body with one media entry and its matching anchor
  is accepted. Plus `SELECT NOT cairn_event_type_has_no_thread('clinical.note.asserted')`.
- [ ] **Step 2: Write the failing Rust test** `crates/cairn-node/tests/note_floor.rs`:
  `a_sealed_note_submits_and_projects_its_clear_text`. Build with Task 1's fns plus the medication body skeleton
  (`build_assert_body`, `medication/assert.rs:90`, as the template), submit through `seal_sign_submit` with a
  human `AuthorParams`, then assert one `clinical_note` row whose `narrative` equals the input. Also
  `a_shred_scrubs_the_note_projection` (drive `cairn_execute_shred` as `crates/cairn-node/tests/` already does for
  medication; grep `cairn_execute_shred(` for the existing helper).
- [ ] **Step 3: Run** `scripts/run-db-gated-tests.sh` for the SQL file (with `PGHOST=127.0.0.1 PGPORT=5532`, #862)
  and `cargo test -p cairn-node --test note_floor -- --nocapture`. Expected: FAIL.
- [ ] **Step 4: Implement** db/064, the db/037 edit, the SCHEMA entry, generation 64 and the pinned counts.
- [ ] **Step 5: Run** both again, plus `cargo test -p cairn-node --test twin_registry --test projection_registry
  --test late_custody_guards --test floor_execute_grants` and `cargo test -p cairn-event --test
  schema_generation`. Expected: PASS, no `skipped:`.
- [ ] **Step 6: Commit** `feat(note): clinical.note.asserted floor, projection and shred hook (db/064)`

### Task 3: The draft store (`db/065_note_draft.sql`) and the `note-sign` timing kind

**Files:**
- Create: `db/065_note_draft.sql`, `db/tests/065_note_draft_test.sql`
- Modify: `db/044_ui_gesture_timing.sql` in place (`'note-sign'` added to `ui_gesture_timing_kind_ck` in BOTH the
  `CREATE TABLE` line and the drop/re-add at `:40`), `db.rs` `SCHEMA`, `SCHEMA_GENERATION` → 65
- Test: the SQL file; `crates/cairn-node/tests/window_role_grants.rs` gains the new doors in Task 5

**Interfaces:**
- Produces (SQL):
  - `note_draft(draft_id uuid PRIMARY KEY, patient_id uuid NOT NULL, terminal_id text NOT NULL CHECK (btrim(terminal_id) <> '' AND terminal_id !~ '[[:cntrl:]]'), narrative text NOT NULL, stated_time timestamptz, recording_started timestamptz NOT NULL, last_keystroke timestamptz NOT NULL)`.
  - `note_draft_spent(draft_id uuid PRIMARY KEY, patient_id uuid NOT NULL, event_id uuid NOT NULL, spent_at timestamptz NOT NULL DEFAULT clock_timestamp())`
    with a `DEFERRABLE INITIALLY DEFERRED` constraint trigger `note_draft_spent_names_its_note`. At commit it
    refuses unless `event_log` holds `event_id` with `event_type = 'clinical.note.asserted'` and
    `patient_id = note_draft_spent.patient_id`. The patient is copied here so the check never joins to the
    deleted draft row.
  - `cairn_note_draft_save(p_draft uuid, p_patient uuid, p_terminal text, p_narrative text, p_stated_time timestamptz) RETURNS text`.
    Outcomes: `'saved'`; `'spent'` (in `note_draft_spent`, nothing written); `'elsewhere'` (row exists under
    another patient or terminal, nothing written); `'no-chart'` (`p_patient` has no event in `event_log`).
    Insert stamps `recording_started = last_keystroke = clock_timestamp()`; update stamps `last_keystroke` only.
    Takes `FOR UPDATE` on an existing row.
  - `cairn_note_drafts(p_patient uuid) RETURNS TABLE(draft_id uuid, terminal_id text, narrative text, stated_time timestamptz, recording_started timestamptz, last_keystroke timestamptz)`,
    ordered by `recording_started`.
  - `cairn_note_draft_spend(p_draft uuid, p_event uuid) RETURNS TABLE(patient_id uuid, narrative text, stated_time timestamptz, recording_started timestamptz, last_keystroke timestamptz)`.
    Locks the row, moves it to `note_draft_spent`, returns it. Zero rows when it is absent or already spent.
  - `cairn_note_signer_name(p_key text) RETURNS text`, a definer wrapper over `cairn_vault_key_holder`:
    `COALESCE(display_name, staff_id)`, NULL for an unknown key.
  - All five functions: `SECURITY DEFINER SET search_path = public, pg_temp`, `REVOKE EXECUTE … FROM PUBLIC`,
    `GRANT EXECUTE … TO cairn_window`. Tables: `REVOKE ALL … FROM PUBLIC`, no grants.

- [ ] **Step 1: Write the failing SQL test** `db/tests/065_note_draft_test.sql`:
  - save → `'saved'`, and a second save keeps `recording_started` and moves `last_keystroke`;
  - save on another terminal or patient → `'elsewhere'`; on a patient with no events → `'no-chart'`;
  - spend returns the row, then a save of the same id → `'spent'` and `note_draft` has no row (Review Focus 3);
  - a spend that commits with no note event fails at COMMIT on `note_draft_spent_names_its_note` (raw-SQL case);
  - `cairn_window`, or a probe role holding it, cannot `SELECT` from either table (`42501`);
  - `has_function_privilege('public', …)` is false for all five functions;
  - a `ui_gesture_timing` row with `gesture_kind = 'note-sign'` inserts, and replaying db/044 afterwards succeeds.
- [ ] **Step 2: Run** the SQL test. Expected: FAIL.
- [ ] **Step 3: Implement** db/065, the db/044 edit, SCHEMA + generation 65.
- [ ] **Step 4: Run** the SQL test, plus `cargo test -p cairn-event --test schema_generation` and
  `cargo test -p cairn-node --lib db::` (the SCHEMA-list pin). Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): the draft store and its doors (db/065)`

### Task 4: `cairn_node::note`: draft calls, signing in one transaction, the chart's notes

**Files:**
- Create: `crates/cairn-node/src/note/mod.rs`, `note/draft.rs`, `note/sign.rs`, `note/read.rs`, `note/view.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod note;`), `crates/cairn-node/src/medication/sealed_submit.rs`
  (extract `prepare_sealed`; `seal_sign_submit` calls it and behaves identically), `medication/read.rs`
  (`set_has_doubted_link` → `pub(crate)`)
- Modify: `crates/cairn-node/tests/db_errors_stay_legible.rs` (`MARKED_COMMIT_SITES` += `note/sign.rs`; mark the
  commit with `db_diagnosis::commit_failure` as `seal_sign_submit` does)
- Test: `crates/cairn-node/tests/note_sign.rs`, `note_read.rs`; pure tests in `note/view.rs`

**Interfaces:**
- Consumes: Task 1's `note_payload`, `note_t_effective`, `render_note_twin`, `escape_note_markup`; Task 3's doors;
  `AuthorParams` and `apply_author` (`sealed_submit.rs:29/48`); `person_charts` (`patient/person.rs:15`);
  `Committed<T>` (`db_diagnosis/refusal.rs:382`); `db::next_hlc`.
- Produces:
  - `pub(crate) struct Prepared { pub event_id: Uuid, pub signed_bytes: Vec<u8>, pub dek: Secret32 }` and
    `pub(crate) async fn prepare_sealed(client: &impl GenericClient, node_sk: &SigningKey, body: EventBody, author: Option<&AuthorParams<'_>>) -> anyhow::Result<Prepared>`.
    This is the author, safety-rung, seal-and-sign and unwrap-key prefix of `seal_sign_submit`. If
    `apply_safety_rung` and `ensure_unwrap_key` take `&Client`, widen them to `&impl GenericClient`.
  - `draft::SaveOutcome { Saved, Spent, Elsewhere, NoChart }` and
    `pub async fn save_draft(client: &impl GenericClient, draft: Uuid, patient: Uuid, terminal: &str, narrative: &str, stated: Option<DateTime<Utc>>) -> anyhow::Result<SaveOutcome>`
    (an unknown outcome string is an `Err`, never a default).
  - `draft::NoteDraft { draft_id, terminal_id, narrative, stated_time: Option<DateTime<Utc>>, recording_started, last_keystroke }`
    and `pub async fn open_drafts(client, patient: Uuid) -> anyhow::Result<Vec<NoteDraft>>`.
  - `sign::SignNoteInput<'a> { draft: Uuid, patient: Uuid, terminal: &'a str, narrative: &'a str, stated: Option<DateTime<Utc>> }`
    and `sign::SignRefusal { Empty, Spent, Elsewhere, NoChart }` (a typed refusal the window words).
  - `pub async fn sign_note(client: &mut Client, node_sk: &SigningKey, input: &SignNoteInput<'_>, author: &AuthorParams<'_>) -> anyhow::Result<Result<Committed<Uuid>, SignRefusal>>`.
    Flow: refuse `Empty` when `narrative.trim()` is empty, before any DB call → mint the event id (UUIDv7) and
    the HLC (`db::next_hlc` takes `&Client`, so before the transaction, as `seal_sign_submit`'s attest arm does) →
    then ONE transaction: `save_draft` with the escaped final text (≠ `Saved` → the matching refusal) → spend → build
    the body from the **spent row's** narrative, times and stated time → `prepare_sealed` → `submit_event` → commit
    (`commit_failure` on a lost answer).
  - `view::NoteGrade { Signed { by: String }, Claimed, WriterNotKnown }` built from `AuthorshipConfidence` with an
    **exhaustive** match (no `_` arm; contributor.rs explains why). `pub fn grade_words(&NoteGrade) -> String`:
    `"signed by {by}"` · `"authorship claimed, not verified here"` · `"writer not known"`.
    `pub fn time_words(basis, t_effective, recording_started) -> String`: `"03:00 (stated)"` or
    `"no stated time · writing began 03:12"` (design §4.3). Local-time formatting reuses `local_instant`'s idiom
    from `cairn-gui-tab-medications` if it is reachable; otherwise take the formatter as an argument.
  - `read::NoteRow { event_id, chart: Uuid, text: String /* unescaped */, time: String, grade: NoteGrade }`,
    `read::PatientNoteList { notes: Vec<NoteRow> /* newest t_effective first */, withheld_on_linked: usize, drafts: Vec<NoteDraft> }`,
    and `pub async fn list_patient_notes(client, patient: Uuid) -> anyhow::Result<PatientNoteList>`.
    It reads `person_charts`, then `clinical_note` joined to `event_log` for the signer
    (`VerifiedKid::from_event_log_column(signer_key_id)`), the contributors, and the attester only when
    `cairn_attestation_vouched(event_id)` (the #412 rule in `VerifiedKid`'s docs). It grades with
    `classify_authorship_confidence` and names with `cairn_note_signer_name`. A note on another chart of a set
    holding a doubted link is counted in `withheld_on_linked` and left out of `notes`.

- [ ] **Step 1: Write the failing tests**
  - `note_sign.rs::a_signed_note_is_attested_and_spends_its_draft`: save, sign; then `clinical_note` has the row,
    `open_drafts` is empty, and the read's grade is `Signed { by: <the enrolled display name> }`.
  - `…::sign_takes_the_text_sent_with_it_not_the_last_autosave` (Review Focus 4).
  - `…::a_second_sign_of_the_same_draft_is_refused_spent_and_writes_one_note`.
  - `…::an_empty_note_is_refused_before_the_database`.
  - `…::a_failed_submit_leaves_the_draft_in_place`: force a floor refusal, e.g. a `refs` entry naming a missing
    event through a test-only body hook, or use the `SET ROLE` fault seam from memory; then the draft survives.
  - `…::no_stated_time_signs_recording_started_as_t_effective`.
  - `note_read.rs::a_doubted_set_withholds_notes_on_the_other_chart` and
    `…::a_peer_note_signed_by_the_node_alone_reads_writer_not_known`.
  - `view.rs` pure: every `AuthorshipConfidence` variant maps to its words; `time_words` both shapes.
  - `seal_sign_submit`'s existing tests stay green, unchanged (the extraction is a pure move).
- [ ] **Step 2: Run** `cargo test -p cairn-node --test note_sign --test note_read -- --nocapture` and
  `cargo test -p cairn-node --lib note::`. Expected: FAIL.
- [ ] **Step 3: Implement** the module and the extraction.
- [ ] **Step 4: Run** the same, plus `--test db_errors_stay_legible` and every `medication` test binary
  (`cargo test -p cairn-node medication`). Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(note): sign a note in one transaction; list a chart's notes with their grade`

### Task 5: The window's commands and its terminal id (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/notes.rs` (the three commands and their `_impl`s),
  `cairn-gui/cairn-gui-tauri/src/terminal_id.rs`
- Modify: `main.rs` (`generate_handler!` += the three; read the terminal id in setup from
  `app.path().app_config_dir()` into `AppState`), `state.rs` (`pub terminal_id: String`)
- Modify: `crates/cairn-node/tests/window_role_grants.rs` (drive `save_draft`, `open_drafts`, `sign_note` and
  `list_patient_notes` to success under `cairn_window`, on a fixture with a draft and a note — never on empty data)
- Test: in-file tests in `terminal_id.rs`; `notes_tests.rs` beside the commands (follow `commands_tests.rs`)

**Interfaces:**
- Consumes: Task 4's API; `AppState::displayed_patient` (`funnel/window.rs:79`), `signer_for_act`
  (`state.rs:324`), `refused` / `stale` (`link/judgement_view.rs:96/108`), `record_timing` (`commands.rs:664`),
  `ConnectionGuard::explain` (pinned by `node_lock_explains_tests.rs`).
- Produces:
  - `pub fn load_or_mint_terminal_id(dir: &Path) -> std::io::Result<String>`: reads `dir/terminal-id`, or writes a
    fresh UUIDv7 there (creating `dir`) and returns it. A file holding blank or control text is replaced, never
    trusted.
  - `#[tauri::command] note_list(state, patient_id: String) -> Result<NotePane, ErrorView>`, where
    `NotePane { notes: Vec<NoteRowView>, withheld_sentence: Option<String>, drafts: Vec<DraftView> }`. A
    `DraftView` carries `here: bool` (its terminal is this one) and the words
    `"unsigned draft · started 03:12 · last typed 03:40"`, plus `" · being written at another terminal"` when
    `!here`.
  - `#[tauri::command] note_draft_save(state, patient_id: String, draft_id: String, narrative: String, stated_time: Option<String>) -> Result<SaveView, ErrorView>`
    with `SaveView { saved: bool, sentence: Option<String> }`. `Spent` → `"this note has already been signed"`;
    `Elsewhere` → `"this draft belongs to another chart or terminal — nothing was saved"`.
  - `#[tauri::command] note_sign(state, patient_id: String, draft_id: String, narrative: String, stated_time: Option<String>, credential: Option<Credential>) -> Result<NoteSignReport, ErrorView>`.
    Order as `cease_impl` (`commands.rs:473`): `displayed_patient` first → mock refuses → `signer_for_act` →
    `sign_note` under `db.lock()` with `explain` → `record_timing(state, "note-sign", 1, ms)`. Text is escaped
    inside `sign_note` (Task 4), not here.
  - `pub fn stated_time_from_local(s: &str) -> Result<DateTime<Utc>, String>`. `datetime-local` sends
    `"2026-11-03T03:00"`, which has no zone: read it in the machine's local zone (chrono `Local`). A time that
    happens twice at a clock change takes the earlier one. A time that never happens (the spring-forward gap) is
    refused with a sentence that names it. Both commands use it; an empty string means `None`.

- [ ] **Step 1: Write the failing tests:** `terminal_id.rs`: `mints_once_then_reads_the_same_id`,
  `a_blank_file_is_replaced`. `stated_time_from_local`: an ordinary time, an empty string, and the gap and
  doubled hour at a DST change (set `TZ=Australia/Sydney` for that test, or take the zone as a parameter of an
  inner pure fn). `notes_tests.rs`: `sign_refuses_a_chart_not_on_screen`,
  `sign_in_fixture_mode_refuses`, `save_words_each_outcome` (pure over a `SaveOutcome` → `SaveView` fn), and the
  `window_role_grants.rs` walk.
- [ ] **Step 2: Run** `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri notes terminal_id` and
  `cargo test -p cairn-node --test window_role_grants -- --nocapture`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same, plus `node_lock_explains_tests` and `commands_tests` in the GUI tree. Expected:
  PASS. Remember `cairn-gui/Cargo.lock` if any dependency changed (memory: three lockfiles).
- [ ] **Step 5: Commit** `feat(window): note commands and a stable terminal id`

### Task 6: The note editor on the chart (`notes.js`)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/notes.js`
- Modify: `src-ui/index.html` (a `#notes` section inside `#chart-view` under the medication list: list, drafts,
  **New note**, a `<textarea>`, an optional `datetime-local` "when it happened", a status line, **Sign note**;
  `<script src="notes.js">` after `signer.js`), `src-ui/main.js` (`refresh` also invokes `note_list`)
- Modify: `commands_tests.rs:367` script list, `signer_webview_tests.rs:21` `WRITERS` (+ `signNote`, length 8),
  `reread_webview_tests.rs:503` lists
- Test: `src/notes_webview_tests.rs` (source-guard pins via `source_guard::js_fn` / `js_const`, as
  `chart_view_webview_tests.rs` does)

**Interfaces:**
- Consumes: Task 5's three commands; `signerCredential()`, `clearSigner(signed)` (`signer.js:24/39`);
  `invoke` (`main.js:13`).
- Produces (JS): `renderNotes(pane)`, `newNote()`, `onNoteInput()` (debounce `NOTE_SAVE_DEBOUNCE_MS = 1000`),
  `saveNoteNow()`, `signNote()`, `noteStatus(text)`. Draft ids come from `crypto.randomUUID()`.

- [ ] **Step 1: Write the failing pins** in `notes_webview_tests.rs`:
  - `signNote` cancels the pending save timer before it invokes `note_sign` (Review Focus 3);
  - a failed `note_draft_save` keeps the textarea's value and sets the status to the exact sentence
    `"not saved yet — your text is still here"` (Review Focus 2);
  - notes and drafts are rendered with `textContent`, never `innerHTML`;
  - `notes.js` never calls `localStorage` or `sessionStorage`;
  - a `here` draft reopens into the editor on `renderNotes`; a draft that is not `here` gets no editor and no
    Sign button;
  - `NOTE_SAVE_DEBOUNCE_MS` is `1000`.
- [ ] **Step 2: Run** `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri notes_webview signer_webview commands_tests reread_webview`.
  Expected: FAIL.
- [ ] **Step 3: Implement.** Grade and time are shown as the words the command sent. The editor is labelled for
  VoiceOver ("Progress note", "When it happened — leave empty if it is now"), and the status line is
  `aria-live="polite"`.
- [ ] **Step 4: Run** the same. Expected: PASS. Then do a headless walk per memory "webview mock walk recipe":
  stub `invoke` with payloads **dumped from the Rust types**, then type → save → sign → listed, plus a save
  failure and a reopened draft. Assert **visibility** (computed display and hidden ancestors), not only
  `textContent`. Record the walk in the PR body.
- [ ] **Step 5: Commit** `feat(window): write, autosave and sign a progress note on the chart`

### Task 7: ADR-0087, spec, RUNBOOK §18, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0087-the-first-note-stream-and-its-draft.md`
- Modify: `docs/spec/index.md` (v0.88 → **v0.89**), `docs/spec/decisions/README.md`, `docs/spec/data-model.md`
  (§3.19 and §3.10: a pointer to ADR-0087 for what was built), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`
  (+ §18) and `TEMPLATE.md`, `docs/developers/durable-rules.md` (a slice-3a block), `docs/HANDOVER.md`,
  `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0087.** It records, for 3a: the text-only subset as built (floor rules 1–9, and why v1
  exclusions are rendered literally rather than refused); the escape at the window; `recording_started` /
  `recording_ended` / `effective_basis` and `t_effective` (a **partial supersession of ADR-0041 decision 2**:
  three new body keys); the deferred `note.added` retirement (**partial supersession of decision 1**); the draft
  store's posture (definer-only, not in backup, one terminal, spend-with-note at commit); and that 3b/3c extend
  it. Then fact-check it sentence by sentence against the SQL (memory: ADR-0077 passed two reviews with one false
  sentence).
- [ ] **Step 2: RUNBOOK §18:** sign ≤ 3 s; open the editor ≤ 1 s; kill the window mid-note and reopen it, and the
  draft is back; VoiceOver reads the editor, the time field, the status line and each note's grade; keyboard
  only. `TEMPLATE.md` gets its §18 section.
- [ ] **Step 3: File issues** (check closing keywords first): retire `note.added` (ADR-0041 decision 1, deferred
  by ADR-0087); and "a `refs` target that arrives after the note naming it, at the remote door": penned or
  refused? This becomes live with 3c's addenda. Name both issues in the ADR and HANDOVER.
- [ ] **Step 4: Durable rules block** "slice 3a": the two replay traps (044 and 037 in place), the
  spend-needs-its-note constraint, "drafts have no table grant", and "never `innerHTML` a note".
- [ ] **Step 5: HANDOVER + ROADMAP:** ⇒ NEXT becomes "3b plan". Prune both toward 500 lines.
- [ ] **Step 6: Full gate**, detached and in CI's order (memory "long gates"): fmt, clippy (both trees),
  `cargo doc` with `RUSTDOCFLAGS=-D warnings`, `scripts/run-db-gated-tests.sh`, the workspace and cairn-gui
  tests, and `paper_parity_plan_section`. Then commit, push and open the PR.
