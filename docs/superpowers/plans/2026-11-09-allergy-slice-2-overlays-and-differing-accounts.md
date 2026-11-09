# Allergy list, slice 2 — overlays and differing accounts — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician can **correct** an allergy line (a better reaction, a date, an adverse effect signed as an
allergy), **de-label** it ("Challenged: tolerated amoxicillin Mar 2022"), or withdraw it as **entered in error**,
each as one signed overlay and never an edit. Two accounts of one allergen that disagree are marked **differing
accounts**. Bylines say when a signer is unconfirmed or their key was repudiated (#1027).

**Architecture:** A correction is an ordinary `clinical.allergy.asserted` whose reserved `refs` slot (ADR-0096
decision 5) carries one `{rel: "correction-of", event}`. De-label and entered in error are two new born-sealed types
in `db/075`: `clinical.allergy-challenge.asserted` and `clinical.allergy-retraction.asserted`. Nothing is edited or
deleted. A pure resolver (`cairn_node::allergy::thread`) folds the rows into **threads** (an entry and its
corrections). Every thread is *live*, *historical* (de-labelled) or *withdrawn*, and the existing view logic runs over
the live heads only. The window gains three gestures on each expanded line.

**Tech Stack:** PostgreSQL 18 (new `db/075`; in-place edits to `db/074`, `db/037`, `db/044`), Rust (`cairn-event`,
`cairn-node`, `cairn-gui-tauri`), Tauri 2 + plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-08-allergy-stream-design.md`](../specs/2026-11-08-allergy-stream-design.md)
§3.3 (the de-label wording), §3.5, §3.6, §3.8, §4, §7 risk 4. Read [ADR-0096](../../spec/decisions/0096-the-allergy-list.md) and
`docs/developers/durable-rules.md`'s **ALLERGY SLICE 1** block before Task 1: every rule there still holds.

## Decisions taken (maintainer, 2026-11-09)

- **M1.** De-label and entered in error are **two event types**, one per act (the db/042 precedent).
- **M2.** A correction's `correction-of` names **the line it replaces**, not the root. Corrections form a chain. Two
  concurrent corrections of one line are two current accounts (a fork), shown together and flagged when they differ.
- **M3.** **A de-label can itself be withdrawn as entered in error**, which puts the allergy back in force.
- **M4.** Scope: overlays, differing accounts and #1027. #1032 stays its own decision.

## Designer's calls (open to correction; each named in the ADR)

- **D1. Overlays are thread-level.** A de-label or withdrawal aimed at any version of an entry covers every version
  of it (its thread). A correction made after a de-label leaves the thread de-labelled. To undo a de-label, withdraw
  the de-label (M3).
- **D2. A correction, de-label or withdrawal is filed under its target's own chart** (the vouch precedent,
  ADR-0092). The authoring door refuses a target on another chart. The window sends the target's chart.
- **D3. A withdrawal is never itself withdrawn.** A wrong withdrawal is repaired by recording the entry again,
  as on paper. This is named as a residual.
- **D4. A since-repudiated key never takes a warning away** (#1027's under-warning direction, ADR-0093's rule
  carried over). Its entries still show, marked " (key had been repudiated)". Its correction is shown as an account
  of its own and supersedes nothing. Its de-label and its withdrawal count for nothing and are listed in history,
  marked. Its "none known" statement stands for no list. An *unconfirmed* signer's act counts and is marked
  " (signer unconfirmed)" (ADR-0094 precedent).
- **D5. Two accounts are "the same allergen"** when their terms match after Unicode lowercasing, trimming and
  collapsing whitespace. Coding equality is slice 3's job. They **differ** when their kinds differ (allergy vs
  adverse effect), or when both reactions are known words, neither is `not-known`, and the words differ. A head
  carrying an unknown kind or reaction word never differs. Duplicates (the same account twice) do not differ.
- **D6. The remote door stays lenient about `refs` contents.** It checks only that `refs` is an array (slice 1's
  rule). A ref the reader cannot parse is ignored, so the entry reads as uncorrected, which over-warns.

## Global Constraints

- **Wire, frozen once each first event is signed** (principle 11):
  - Correction: `clinical.allergy.asserted` / `clinical.allergy/1`, ADR-0096's payload unchanged. `refs` is `[]` or
    exactly `[{"rel": "correction-of", "event": "<uuid>"}]` (an object with exactly those two keys).
  - `clinical.allergy-challenge.asserted`, `schema_version` `clinical.allergy-challenge/1`, payload exactly
    `{target, tolerated: {term, coding}, date: {precision, value}, text}`. `target` is a uuid string;
    `tolerated.term` is a non-blank string; `coding` is `null` (slice 3); `date` has the shape of ADR-0096's `onset`;
    `text` is a string or null.
  - `clinical.allergy-retraction.asserted`, `schema_version` `clinical.allergy-retraction/1`, payload exactly
    `{target, text}`.
  - Both new types are `additive`, `targets_other_author = FALSE` (a nurse may strike a doctor's line, as on
    paper; ADR-0043's owner gate would refuse that). No `t_effective`, and `safety: None` (ADR-0096 decision 21
    stands).
- **Twins**, exact copy:
  - Correction: `"Corrects an earlier entry: " + entry_words_dated(…)`, with the ISO onset as in R17.
  - De-label: `"De-labelled by challenge: allergy record {target} — tolerated {term}"`, then `" · {ISO date}"` when
    the date is known, then `" — {text}"` when text is present.
  - Withdrawal: `"Entered in error: withdraws allergy record {target}"`, then `" — {text}"`.
- **The two tiers** (db/041 idiom, ADR-0096 decision 6). Structural rules apply at both doors: types, a uuid
  `target`, a non-blank `tolerated.term`, and `date.value` null exactly when `not-known`. Closed vocabularies and
  references apply at the authoring door only: `schema_version`, the `date.precision` words, the date's shape and
  not-future (via `cairn_allergy_onset_problem`), `coding` null, and the target's type and chart.
- **Clinician words** (exact):
  - A de-labelled line reads `"Historically reported: {term} {allergy|adverse effect|<kind>}. Challenged: tolerated
    {tolerated}"`, then `" {date as read}"` (`14 Mar 2022` / `Mar 2022` / `2022`), then `" — {text}"`.
  - A withdrawn entry reads `"Entered in error: {line words} (withdrawn {day} by {name})"`.
  - A withdrawn "none known" statement reads `"Entered in error: no known allergies (asked {day} by {name})"`, with
    the same wording for "no known drug allergies".
  - An older version reads `"Earlier: {line words} (recorded {day} by {name})"`, newest first.
  - The differing mark is `" — differing accounts"`, appended to every head in a differing group, on the strip as
    well as in the list.
  - The byline markers are `" (signer unconfirmed)"` and `" (key had been repudiated)"`, the note view's words
    (`note::view::signer_words`). Allergies carry no `(student)` mark (ADR-0096 decision 22).
  - The strip adds a quiet count `"+N historical"` after the adverse-effect count. A withdrawn entry is not on the
    strip at all.
- **Timing kinds**: `allergy-correct`, `allergy-challenge`, `allergy-retract`.
- `SCHEMA_GENERATION` 75. Files stay under 500 lines: new code goes in new files where a file would pass that.

## Review Focus

1. **A de-labelled allergy beside a later "No known allergies".** The de-labelled thread overrides nothing, so the
   statement stands. Pinned in Task 4: `a_delabelled_allergy_overrides_no_none_statement`.
2. **An overlay this terminal cannot read** (no custody). It counts in `unreadable` and leads the strip, and its
   target still reads live (over-warning). Pinned in Task 3: `an_unreadable_overlay_is_counted_and_its_target_stays_live`.
3. **A repudiated key withdraws an anaphylaxis.** The anaphylaxis stays on the strip, and the withdrawal is listed and
   marked. Pinned in Task 4: `a_repudiated_keys_withdrawal_takes_nothing_away`.
4. **A correction aimed at a linked chart's entry from the opened chart.** It is filed under the target's chart.
   The floor refuses a mismatch. Pinned in Tasks 2 (`075` section 2) and 5 (`correct_files_under_the_targets_chart`).
5. **Two concurrent corrections of one line.** Both are heads, flagged when they differ, and neither is lost.
   Pinned in Task 4: `two_corrections_of_one_line_are_two_accounts`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** strike the line in the drug chart's allergy box and write the corrected one beside it,
  initialled; write "tolerated amoxil 03/22" against a struck penicillin line; strike a slip and initial it.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Correct: 1 (strike and rewrite) → 1 (one entry carrying `correction-of`) → 1 (Correct, edit the form, Sign).
  - De-label: 1 → 1 (one challenge event) → 1 (De-label, name what was tolerated, Sign).
  - Entered in error: 1 → 1 (one withdrawal event) → 1 (Entered in error, Sign).
  - `M = N` everywhere: no architecture defect to file.
- **Time + cognitive load:** correct ≤ 15 s, de-label ≤ 15 s, entered in error ≤ 5 s, all with the key held.
  Differing accounts must be legible on the strip with no click. Timed with `allergy-correct`, `allergy-challenge`
  and `allergy-retract`. The measurement is owed by a human in **RUNBOOK §28** (Task 7).

---

### Task 1: The overlay bodies and twins (`cairn-event`, pure)

**Files:**
- Modify: `crates/cairn-event/src/allergy.rs` — `AllergyEntry` gains `pub corrects: Option<&'a str>`;
  `allergy_payload` writes `refs` from it; `render_allergy_twin` prefixes the correction words.
- Create: `crates/cairn-event/src/allergy_overlay.rs` (with `pub mod allergy_overlay;` in `lib.rs`).
- Update every `AllergyEntry { … }` literal: `rg -n "AllergyEntry \{" crates cairn-gui`.

**Interfaces — Produces:**
- `pub const ALLERGY_CHALLENGE_EVENT_TYPE: &str = "clinical.allergy-challenge.asserted"`,
  `ALLERGY_CHALLENGE_SCHEMA_VERSION = "clinical.allergy-challenge/1"`, `ALLERGY_RETRACTION_EVENT_TYPE`,
  `ALLERGY_RETRACTION_SCHEMA_VERSION`.
- `pub struct AllergyChallenge<'a> { pub target: &'a str, pub tolerated: &'a str, pub date: Onset, pub text: Option<&'a str> }`
- `pub struct AllergyRetraction<'a> { pub target: &'a str, pub text: Option<&'a str> }`
- `pub fn allergy_challenge_payload(c: &AllergyChallenge) -> Value`,
  `pub fn render_allergy_challenge_twin(c: &AllergyChallenge) -> String`,
  `pub fn allergy_retraction_payload(r: &AllergyRetraction) -> Value`,
  `pub fn render_allergy_retraction_twin(r: &AllergyRetraction) -> String`.

- [ ] **Step 1: Failing tests.** In `allergy.rs`:
  - `correction_writes_one_correction_of_ref`: `refs == [{"rel":"correction-of","event":"E1"}]`.
  - `correction_twin_says_it_corrects`: the twin starts with `"Corrects an earlier entry: Allergy: penicillin → rash"`.
  - `uncorrected_entry_keeps_empty_refs_and_twin`: refs and twin are byte-identical to slice 1's.

  In `allergy_overlay.rs`:
  - `challenge_payload_has_exactly_the_frozen_keys` (`target, tolerated{term,coding:null}, date{precision,value}, text`).
  - `challenge_twin_words_are_exact`: with a month date and text, the twin is
    `"De-labelled by challenge: allergy record E1 — tolerated amoxicillin · 2022-03 — oral challenge"`.
    With an unknown date and no text, it ends at `"amoxicillin"`.
  - `retraction_payload_and_twin`: the payload is `{target, text}`, and the twin is
    `"Entered in error: withdraws allergy record E1 — wrong patient"` (no `" — "` tail without text).
- [ ] **Step 2:** `cargo test -p cairn-event allergy` → FAIL (missing items).
- [ ] **Step 3:** Implement. The date reuses `Onset`'s (precision, value) mapping. Factor slice 1's `match` into a
  `pub(crate) fn onset_wire(&Onset) -> (&'static str, Option<&str>)` and use it from both files.
- [ ] **Step 4:** Run `cargo test -p cairn-event` → PASS. Then run `cargo build --workspace` and the cairn-gui tree's
  build, with every `AllergyEntry` literal given `corrects: None` (cross-crate rule).
- [ ] **Step 5:** Commit `feat(allergy): correction refs and the challenge / retraction bodies and twins`.

### Task 2: The floor and projections (`db/075_allergy_overlays.sql`; db/074, db/037, db/044 in place)

**Files:**
- Create: `db/075_allergy_overlays.sql`, `db/tests/075_allergy_overlays_test.sql`,
  `crates/cairn-node/tests/allergy_overlay_floor.rs`.
- Modify in place: `db/074_clinical_allergy.sql` step 3. Its last rule becomes "refs is `[]` or exactly one
  `correction-of` whose `event` is a `clinical.allergy.asserted` in `event_log` with this event's `patient_id`". The
  messages are `refs must be [] or one {rel: correction-of, event}` and
  `correction-of target % is not an allergy entry on this chart`. Also step 2:
  `cairn_allergy_onset_problem` returns its reason **without** the leading field name; each caller prefixes
  `onset.value ` / `date.value `. Update `db/tests/074`'s expected messages to match.
- Modify in place: `db/037_born_sealed.sql` (the shred deletes from `clinical_allergy_challenge` and
  `clinical_allergy_retraction` by content address, behind `to_regclass` like its neighbours) and
  `db/044_ui_gesture_timing.sql` (both CHECK lists gain the three timing kinds).
- Pinned counts move: `crates/cairn-event/src/schema_generation.rs` (75); `crates/cairn-node/src/db.rs` (the
  `include_str!` list); `twin_registry.rs` and `db/tests/034` (31 → 33); `projection_registry.rs` and
  `db/tests/039`. Run `rg -n "074_clinical_allergy" crates db` to find every other per-file list.

**Interfaces — Produces:**
- `cairn_check_allergy_challenge(text, jsonb)` and `cairn_check_allergy_retraction(text, jsonb)`, revoked from
  PUBLIC, registered in `cairn_event_twin_check` with `'allergy challenge requires a non-empty authored twin (§3.13/§3.3)'`
  and the retraction's equivalent.
- Tables (final columns, `GRANT SELECT … TO cairn_agent`, indexes on `patient_id` and `content_address`):
  `clinical_allergy_challenge(event_id UUID PK, patient_id UUID NOT NULL, target UUID NOT NULL, tolerated_term TEXT,
  tolerated_coding JSONB, date_precision TEXT, date_value TEXT, text TEXT, hlc_wall BIGINT, hlc_counter INTEGER,
  origin TEXT, content_address BYTEA NOT NULL)` and
  `clinical_allergy_retraction(event_id, patient_id, target UUID NOT NULL, text, hlc_wall, hlc_counter, origin,
  content_address)`.
- Appliers `clinical_allergy_challenge_apply` and `clinical_allergy_retraction_apply`, both `heal_safe`, run order
  20, `ON CONFLICT DO NOTHING`, no schema_version filter (ADR-0096 decision 8's reasoning applies).

- [ ] **Step 1: Failing SQL test** `075_allergy_overlays_test.sql`, using 074's helpers and section style:
  - (0) Both types are registered additive, not targeting another author, and off db/048's no-thread list.
  - (1) Accepted: a correction of an entry; a de-label with every date precision; a withdrawal of an entry, of a
    none statement, and of a challenge.
  - (2) Refused at the authoring door: two refs; a `rel` other than `correction-of`; a correction target that is a
    none statement or sits on another chart; a challenge whose target is not an entry; a withdrawal of a withdrawal
    (D3); a future challenge date; a `/2` schema; filled coding.
  - (3) Refused at both doors: a non-uuid `target`; a blank `tolerated.term`; a `date.value` present with
    `not-known`.
  - (4) Admitted at the remote door: a newer word in `date.precision`, and a peer's `refs` of another `rel`.
  - (5) The shred deletes a challenge's row and a withdrawal's row.
- [ ] **Step 2:** Run against `cairn_test` (`PGHOST=127.0.0.1 PGPORT=5532`, #862) → FAIL.
- [ ] **Step 3:** Write db/075 and the in-place edits.
- [ ] **Step 4:** Run the SQL test → PASS. Add the Rust mirror `allergy_overlay_floor.rs`, one test per section,
  and run it together with `allergy_floor.rs`, `twin_registry`, `projection_registry`, `hex_decode_helper` and
  `db_errors_stay_legible` (`--nocapture`, no `skipped:`).
- [ ] **Step 5:** Commit `feat(allergy): correction refs open; challenge and retraction floor + projections (db/075)`.

### Task 3: Sign the overlays; read them (`cairn_node::allergy::{sign, read}`)

**Files:**
- Modify: `crates/cairn-node/src/allergy/sign.rs` (adds `sign_allergy_challenge` and `sign_allergy_retraction`, each
  `async fn(client, node_sk, node_kid, node_origin, patient: Uuid, &AllergyChallenge|&AllergyRetraction, author) ->
  anyhow::Result<Uuid>`, with the shape of `sign_allergy_none`).
- Modify: `crates/cairn-node/src/allergy/read.rs`. Split the row types into `read_rows.rs` if the file passes 500
  lines.
- Create: `crates/cairn-node/tests/allergy_overlays.rs`.

**Interfaces — Produces:**
- `pub enum KeyState { Standing, Unconfirmed, Repudiated }`, mapped from `cairn_key_confirmation`:
  `unconfirmed` | `doubtful` → `Unconfirmed`; `repudiated` → `Repudiated`; `confirmed` | `outside` | `unknown`
  → `Standing`. Any other word is an error that fails the read (`note::vouch_read::signer_of`'s rule, reusing
  `CONFIRMATION_WORDS`).
- `AllergyRow` gains `corrects: Option<Uuid>` (the first `correction-of` ref whose `event` parses; other refs are
  ignored, D6) and `key: KeyState`. `NoneRow` gains `key`.
- `pub struct ChallengeRow { event_id, chart, target: Uuid, tolerated: String, date_precision: String,
  date_value: Option<String>, text: Option<String>, recorded_at, hlc, signer: Option<String>, key: KeyState }`
- `pub struct RetractionRow { event_id, chart, target: Uuid, text: Option<String>, recorded_at, hlc, signer, key }`
- `PatientAllergies` gains `challenges: Vec<ChallengeRow>` and `retractions: Vec<RetractionRow>`.
  `UNREADABLE_SQL` counts all four types.

- [ ] **Step 1: Failing DB tests** in `allergy_overlays.rs`:
  - `signing_a_challenge_and_a_retraction_records_human_authorship` (`{human authored, node recorded}`,
    `safety IS NULL`).
  - `a_correction_reads_back_with_its_target`.
  - `overlays_read_back_over_a_linked_set`.
  - `a_repudiated_signer_reads_as_repudiated` (repudiate via the console door the vault tests use).
  - `an_unreadable_overlay_is_counted_and_its_target_stays_live` (insert an overlay with no custody, as
    `allergy_read.rs::an_allergy_event_this_node_cannot_read_is_counted` does).
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. Each SQL statement adds
  `cairn_key_confirmation(e.signer_key_id)`. **Step 4:** Run with `allergy_read.rs`, `allergy_sign.rs` and
  `window_role_grants.rs` (the window's login must run every new read) → PASS.
- [ ] **Step 5:** Commit `feat(allergy): sign a de-label or a withdrawal; read overlays and key state`.

### Task 4: Threads, overlays and differing accounts in the view (pure)

**Files:**
- Create: `crates/cairn-node/src/allergy/thread.rs` (the resolver) and `allergy/thread_tests.rs`.
- Modify: `crates/cairn-node/src/allergy/view.rs`. Move `Strip` and `strip_text` into `allergy/strip.rs` if
  `view.rs` passes 500 lines. Put new view tests in `allergy/view_overlay_tests.rs` (`view_tests.rs` is already at
  549 lines).

**Interfaces — Produces:**
- `pub fn same_allergen(a: &str, b: &str) -> bool` (D5's normalisation).
- `pub struct Resolved<'a> { pub live: Vec<Head<'a>>, pub historical: Vec<Historical<'a>>, pub withdrawn: Vec<&'a AllergyRow>,
  pub withdrawn_none: Vec<&'a NoneRow>, pub ignored_overlays: Vec<String> }`, where
  `Head<'a> { row: &'a AllergyRow, earlier: Vec<&'a AllergyRow>, differing: bool }` and
  `Historical<'a> { head: &'a AllergyRow, challenge: &'a ChallengeRow }`.
- `pub fn resolve(a: &PatientAllergies) -> Resolved<'_>`. The algorithm, in order:
  1. Drop every challenge and every withdrawal signed by a `Repudiated` key (D4). Each dropped one's line goes to
     `ignored_overlays`, marked.
  2. The effective withdrawals are the remaining withdrawals. A challenge stands when it is not withdrawn.
  3. Threads: link each non-repudiated entry with `corrects = Some(t)` to `t` when `t` is an entry on the **same
     chart** (D2). Otherwise the entry is its own root. Each connected component is one thread, keyed by its
     smallest event id (deterministic, and safe against a forged cycle).
  4. A thread is **withdrawn** if any member is the target of an effective withdrawal. Otherwise it is
     **historical** if any member is the target of a standing challenge, using the newest such challenge. Otherwise
     it is **live**. Withdrawn outranks historical.
  5. Heads are the members that no member's link targets. `earlier` holds the other members, newest by HLC first.
  6. `differing`: over all live heads (every list, every chart), group by `same_allergen`. A head is differing when
     its group holds another head that differs by D5.
- `AllergyView` gains `historical: Vec<EntryLine>` (never prominent), `entered_in_error: Vec<String>`, and
  `ignored_overlays: Vec<String>`.
- `EntryLine` gains `earlier: Vec<String>` and `challenge: Option<Uuid>` (the standing challenge on a historical
  line, for Task 6's withdraw button).
- `standing_none` and `previously_none` become `Vec<NoneLine { event_id: Uuid, text: String }>` (Task 6 withdraws
  them).
- `EntryLine.signer` and the none lines' " by …" carry the D4 byline marker.
- `build_view` runs slice 1's list, override and strip logic over the **live heads only**, plus the
  `" — differing accounts"` suffix, the `"+N historical"` count, and the D4 rule that a `Repudiated` none
  statement stands for nothing and goes to history marked.

- [ ] **Step 1: Failing tests** (`thread_tests.rs` and `view_overlay_tests.rs`). Every slice 1 test must still pass
  unchanged.
  - `a_correction_replaces_its_line_and_keeps_the_older_as_earlier`
  - `a_chain_of_corrections_shows_only_the_newest` (A ← B ← C)
  - `two_corrections_of_one_line_are_two_accounts` (and they are differing when their reactions differ)
  - `correcting_an_allergy_to_an_adverse_effect_moves_it_off_the_strip` (the metformin case)
  - `a_delabelled_line_reads_historically_reported` (the exact Global Constraints wording) and is not prominent
  - `a_delabelled_allergy_overrides_no_none_statement` (Review Focus 1)
  - `withdrawing_a_delabel_puts_the_allergy_back` (M3)
  - `a_withdrawn_entry_leaves_the_strip_and_reads_entered_in_error`
  - `withdrawing_any_version_withdraws_the_thread` (D1)
  - `a_correction_after_a_delabel_stays_historical` (D1)
  - `a_withdrawn_none_statement_stands_for_nothing`
  - `a_repudiated_keys_withdrawal_takes_nothing_away` (Review Focus 3)
  - `a_repudiated_keys_correction_is_its_own_account`
  - `a_repudiated_keys_none_statement_does_not_stand`
  - `an_unconfirmed_signer_counts_and_is_marked`
  - `a_correction_naming_another_charts_entry_is_its_own_root` (D2, the lenient remote case)
  - `same_allergen_ignores_case_and_spacing_not_spelling` (`"Penicillin  "` matches `"penicillin"`;
    `"amoxycillin"` does not match `"amoxicillin"`)
  - `not_known_never_differs_and_duplicates_do_not_differ`
  - `an_allergy_and_an_adverse_effect_on_one_allergen_differ`
  - `the_strip_marks_differing_accounts`
- [ ] **Step 2:** `cargo test -p cairn-node --lib allergy` → FAIL. **Step 3:** Implement. **Step 4:** PASS, then
  `cargo clippy -p cairn-node --all-targets -- -D warnings`.
- [ ] **Step 5:** Commit `feat(allergy): threads, de-labels, withdrawals and differing accounts in the view`.

### Task 5: The window's commands (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/allergy_overlays.rs` and `allergy_overlays_tests.rs`. Register the commands
  where `allergies.rs`'s are registered.
- Modify: `allergies.rs`. `EntryInput` gains `corrects: Option<String>`. `AllergyLine` gains `earlier`,
  `challenge`, and the raw row words the Correct form prefills from (`allergen_kind, term, kind, category,
  reaction_text, onset_precision, onset_value, source`). `AllergyPane` gains `historical`, `entered_in_error` and
  `ignored_overlays`, and its none lines become `{event_id, text}`.

**Interfaces — Produces:**
- `allergy_record` with `corrects` set signs a correction and is timed `allergy-correct`.
- `#[tauri::command] allergy_challenge(patient_id, charts, target, tolerated: String, date: Option<String>, text:
  Option<String>, …signer args as allergy_record)` → `Result<AllergyWrite, ErrorView>`.
- `#[tauri::command] allergy_retract(patient_id, charts, target, text: Option<String>, …)` →
  `Result<AllergyWrite, ErrorView>`.
- All three reuse `on_screen_and_writable`, `sign_then_reread`, `written` and `sign_failure` (R11, #814 unchanged).
- Before the key, the target must be a row of a **fresh** read over the same chart set: an entry head for Correct and
  De-label; an entry, a none statement or a standing challenge for Entered in error. The event is filed under **that
  row's chart** (D2). Refusals, each named: `"That allergy line is no longer on this chart — read it again."`,
  `"Name what was tolerated."`, and an unreadable or future date (reuse `parse_onset`).

- [ ] **Step 1: Failing tests** (DB-gated, as `allergies_tests.rs`; CI does not run them, #905):
  - `correct_files_under_the_targets_chart`
  - `a_challenge_and_a_retraction_land_and_reread`
  - `a_target_not_in_the_fresh_read_is_refused_before_the_key`
  - `a_challenge_needs_what_was_tolerated`
  - `each_act_is_timed_with_its_own_kind`

  Pure tests:
  - `a_landed_overlay_is_an_ok_with_or_without_its_pane`
  - `an_interrupted_overlay_is_not_known_and_never_retried`
- [ ] **Step 2–4:** FAIL → implement → PASS. Run the cairn-gui gate (`cargo test`, `clippy -D warnings`, and
  `cargo doc` with `RUSTDOCFLAGS=-D warnings`) and the root source guards (memory: root guards sweep the GUI src).
- [ ] **Step 5:** Commit `feat(window): correct, de-label and entered-in-error commands`.

### Task 6: The gestures on the expanded list (`allergy_overlays.js`)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/allergy_overlays.js` and
  `cairn-gui/cairn-gui-tauri/src/allergy_overlays_webview_tests.rs` (source guards, as `allergies_webview_tests.rs`).
- Modify: `allergies.js` (`lineItem` draws `earlier` as a nested muted list and the line's buttons; it draws the
  historical, entered-in-error and ignored lists) and `index.html` (the script tag; the historical and
  entered-in-error lists inside `#allergy-details`).

**Behaviour:**
- Each live line in the expanded list carries three buttons: **Correct**, **De-label**, **Entered in error**.
  A historical line carries **Withdraw de-label**. A standing or previous none line carries **Entered in error**.
- **Correct** fills the existing form from the line's raw words, says `"Correcting: {line text}"` above it, and
  sends `corrects` on Sign. Clearing the form clears the correction. No dialog.
- **De-label** opens an inline row under the line: "Tolerated" (prefilled with the line's term), "When", an
  optional note, and **Sign**.
- **Entered in error** opens an inline row with an optional reason and **Sign**. Nothing is struck until it is
  signed.
- Every sentence is Rust's or the pane's. Text goes through `textContent` only (slice 1 rule). Late-answer, fault
  and one-press rules are those of `pressOnce` and `afterSigned` (R13), reused, not copied.

- [ ] **Step 1: Failing source guards:**
  - `overlay_js_never_assigns_inner_html`
  - `correct_prefills_and_sends_corrects`
  - `delabel_and_withdraw_rows_are_inline_not_dialogs` (no `confirm(`, no `<dialog`)
  - `overlay_presses_go_through_press_once`
  - `historical_and_entered_in_error_lists_are_drawn`
- [ ] **Step 2–4:** FAIL → implement → PASS. Walk the window headlessly by the webview-mock recipe (memory), in
  Chromium and WebKit. Assert visibility, never just text.
- [ ] **Step 5:** Commit `feat(window): correct, de-label and entered-in-error on the allergy list`.

### Task 7: ADR-0097, spec, RUNBOOK §28, durable rules, issues, handover

- [ ] ADR-0097 *"Allergy overlays and differing accounts"* covers M1–M4, D1–D6, the frozen wire and twins, and what
  is not guaranteed: D3's no-undo of a withdrawal; slice 3's coding equality; a repudiated key's withdrawal of a
  *challenge* counts for nothing, so its challenge still stands; a concurrent de-label and correction across a
  partition stay de-labelled (D1). Fact-check each sentence against the SQL (memory: ADR-0077).
- [ ] Spec v0.99 (`index.md`, the data-model §3.3 allergy note); RUNBOOK §28 (the three budgets, VoiceOver on the
  new buttons, a real WebKit walk); a durable-rules block **ALLERGY SLICE 2**, one rule per D-call with its pin and
  its tempting wrong fix; close #1027 in the PR body (check closing keywords before committing); file the review's
  residuals; update HANDOVER and ROADMAP.
- [ ] Run every CI gate in CI's order **after the final edit**, then push and open the PR.
