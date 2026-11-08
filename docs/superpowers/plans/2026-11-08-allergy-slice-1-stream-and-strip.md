# Allergy list, slice 1 — the stream and the header strip — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician opens a chart and sees, in the patient header, one terse line saying what the patient is
allergic to — or "No known allergies (asked …)", "NKDA (asked …)", or "Allergies not yet asked". From that line
they record a drug or non-drug allergy or adverse effect in a five-field form, or press "No known allergies" /
"NKDA", and sign with the held key. Nothing is ever blank.

**Architecture:** Two new born-sealed clinical event types in one file (`db/074`): `clinical.allergy.asserted`
(`clinical.allergy/1`, one entry) and `clinical.allergy-none.asserted` (`clinical.allergy-none/1`, a "none known"
statement), each with a floor check, a twin, a one-row-per-event projection and a shred hook. `cairn-event` gains
the pure bodies and twins; `cairn-node` gains an `allergy` module (sign, read over a linked set, and a pure view
that derives the strip). The window gains three commands and one script, `allergies.js`, drawing into
`#identity`.

**Tech Stack:** PostgreSQL 18 (new `db/074`; in-place edits to `db/037`, `db/044`), Rust (`cairn-event`,
`cairn-node`, `cairn-gui-tauri`), Tauri 2 + plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-08-allergy-stream-design.md`](../specs/2026-11-08-allergy-stream-design.md)
(approved 2026-11-08). Read §2–§4 and §5's slice 1 row before Task 1. **This plan covers slice 1 only.** Slice 2
(overlays, differing accounts) and slice 3 (drugref, the medication check) get their own plans.

## Global Constraints

- **Wire shape is frozen once the first entry is signed** (principle 11). `clinical.allergy.asserted`,
  `schema_version = "clinical.allergy/1"`, payload exactly these keys:
  - `allergen_kind`: `"drug"` | `"other"`
  - `substance`: `{ "term": string, "coding": null | {system, code, display} }` — ADR-0059's shape; `term` is
    what the clinician saw or typed
  - `kind`: `"allergy"` | `"adverse-effect"`
  - `reaction`: `{ "category": c, "text": null | string }`, `c` ∈ `anaphylaxis` · `angioedema` · `urticaria` ·
    `rash` · `gi-upset` · `other` · `not-known`
  - `onset`: `{ "precision": p, "value": v }`, `p` ∈ `day` (`v` = `YYYY-MM-DD`) · `month` (`YYYY-MM`) · `year`
    (`YYYY`) · `not-known` (`v` = `null`)
  - `source`: `"patient-stated"` | `"seen-documented"` | `"another-record"`
  - `refs`: `[]` in this slice; the array of `{rel, event}` slice 2's corrections use (`rel` `"correction-of"`)
- `clinical.allergy-none.asserted`, `schema_version = "clinical.allergy-none/1"`, payload exactly
  `{ "scope": "drug" | "all" }`. "Asked" is the event's recording time (its HLC wall); no separate field.
- Envelope `t_effective` is `None` on both (the reaction's time is `onset`, inside the body). Contributors:
  `[{human, "authored"}, {node, "recorded"}]` via the existing `apply_author` (ADR-0053). **`safety: None`**
  (design §4: no clear signal until #406/#401).
- **Floor: two tiers** (ADR-0051, the db/041 idiom). *Structural* rules refuse at **both** doors: every key
  present with the right JSON type; `term` non-empty after `btrim`; `onset.value` null exactly when precision is
  `not-known`; `coding`, when an object, three non-empty strings. *Closed vocabularies* (every enumerated value
  above, and `onset.value`'s shape for its precision, and an onset after the recording day) refuse at the
  **authoring door only** (`cairn.remote_apply` ≠ `'on'`), so a newer peer's term never wedges sync (ADR-0056).
  Authoring door only, this slice: `coding` must be `null` and `refs` must be `[]` (slice 3 and slice 2 open them).
- **No markup handling.** Allergy text is never rendered as markdown: the twin is plaintext and the window sets
  `textContent` only. No escape function exists or is needed.
- **Both event types stay OFF db/048's `cairn_event_type_has_no_thread` list** (the conservative sensitivity bound,
  as the note did). Pin it in `db/tests/074`; do not edit the list.
- **Replay traps** (repo memory "migration replay"): the timing kinds go into **db/044 in place** (its `CREATE
  TABLE` line AND its drop/re-add); the shred hook goes into **db/037's `cairn_execute_shred` in place**, guarded by
  `to_regclass`; a projection table's columns are final.
- **A linked record with a doubted link** — *my call, confirm at plan review:* entries on the other chart are
  **shown, marked** "(on a chart whose link to this one is in doubt)", never withheld. Notes and medications
  withhold; for allergies the over-warning is the safe error.
- **No co-sign** (maintainer): a student's entry reads exactly like anyone's.
- **House rules:** TDD; every non-trivial fn documented for a junior reader; files under 500 lines; no
  `salt`/`nonce`/`iv` names; `scripts/check_closing_keywords.py` before every commit; DB-gated tests show no
  `skipped:` line (`--nocapture`); a DB-free run needs `CAIRN_ALLOW_DB_SKIP=1`; SDD subagents run FOREGROUND only.
- **Pinned guards a new member must join**, today's values: `db/tests/034_twin_registry_test.sql` (29 → **31**) ·
  `crates/cairn-node/tests/twin_registry.rs:103` (29 → **31**, plus two `expected` vec entries) ·
  `db/tests/039_projection_registry_test.sql` (35 → **37**) · `crates/cairn-node/tests/projection_registry.rs:66`
  (33 → **35**) · `crates/cairn-node/src/db.rs` `SCHEMA` list + `SCHEMA_GENERATION` 73 → **74**
  (`crates/cairn-event/src/schema_generation.rs:45`) · `late_custody_guards.rs` (appliers `heal_safe = TRUE`) ·
  `floor_execute_grants.rs` (both floor fns revoked from PUBLIC) · `db_errors_stay_legible.rs` `MARKED_COMMIT_SITES`
  (+ `allergy/sign.rs`) · `window_role_grants.rs` (every new entry point driven under `cairn_window`) ·
  `commands_tests.rs` script list · `signer_webview_tests.rs` `WRITERS` · the root source guards that sweep
  `cairn-gui/*/src` (memory "root source guards") — run them DB-free before the PR. Re-read each value at its
  line before editing: a merged PR may have moved it.

## Review Focus

1. **Sign pressed with "What kind" unchosen** (the anti-goal): nothing is signed, nothing is sent to the node, and
   an inline line under the field says "Choose Allergy or Adverse effect". No dialog. Pinned in Task 5 (the command
   refuses a missing kind before the key) and Task 6 (the form says so).
2. **"No known allergies" then an allergy, in either HLC order, or from two linked charts**: the strip shows the
   allergy, and the statement appears only as "previously recorded: no known allergies (asked …)" in the expanded
   list. Pinned in Task 4 with both orders.
3. **Many allergies** (six drug allergies): every one appears on the strip; none is cut to "+N more". Only adverse
   effects and the historical count are collapsed. Pinned in Task 4.
4. **Text that looks like markup or is unusual** (`<script>`, `"quoted"`, `5 < 10`, an emoji, 300 characters):
   signed verbatim, twin verbatim, drawn with `textContent` (never `innerHTML`). Pinned in Task 1 (twin) and
   Task 6 (a source guard that `allergies.js` never assigns `innerHTML`).
5. **An onset typed partly or nonsensically** (`2019-13`, `31/02/2019`, a future year): the window says what is
   wrong beside the field and signs nothing; the authoring floor refuses the same body from a raw-SQL client.
   Pinned in Task 2 (floor) and Task 5 (the command's own parse, before the key).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the allergy box on the drug chart and the red sticker on the folder: write "Penicillin —
  anaphylaxis 2014", or "NKDA", and initial it.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Reading whether a dangerous allergy exists: 0 → 0 → 0 (the strip is in the header; no click).
  - Recording an entry: 1 (write the line) → 1 (one signed event) → 1 (fill the form + Sign with the held key).
  - Recording "No known allergies" / NKDA: 1 → 1 → 1 press.
  - `M = N` everywhere: no architecture defect to file.
- **Time + cognitive load:** free-text entry ≤ 10 s, "none known" ≤ 3 s, strip readable with no click (design §6).
  Timed through `record_timing("allergy-record", …)` and `record_timing("allergy-none", …)`; the measurement is
  owed by a human in **RUNBOOK §27** (Task 7) — this slice is the first runnable allergy surface.

---

### Task 1: The bodies and twins (`cairn-event`, pure)

**Files:**
- Create: `crates/cairn-event/src/allergy.rs`
- Modify: `crates/cairn-event/src/lib.rs` (`pub mod allergy;` beside `pub mod note;`)
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - consts `ALLERGY_EVENT_TYPE = "clinical.allergy.asserted"`, `ALLERGY_SCHEMA_VERSION = "clinical.allergy/1"`,
    `ALLERGY_NONE_EVENT_TYPE = "clinical.allergy-none.asserted"`, `ALLERGY_NONE_SCHEMA_VERSION =
    "clinical.allergy-none/1"`.
  - enums, each with `as_str(self) -> &'static str` returning the wire word and `parse(&str) -> Option<Self>`:
    `AllergenKind { Drug, Other }`, `EntryKind { Allergy, AdverseEffect }`, `ReactionCategory { Anaphylaxis,
    Angioedema, Urticaria, Rash, GiUpset, Other, NotKnown }`, `Source { PatientStated, SeenDocumented,
    AnotherRecord }`, `NoneScope { Drug, All }`.
  - `pub enum Onset<'a> { Day(&'a str), Month(&'a str), Year(&'a str), NotKnown }` (values pre-validated by the
    caller, Task 5).
  - `pub struct AllergyEntry<'a> { pub allergen_kind: AllergenKind, pub term: &'a str, pub kind: EntryKind, pub
    category: ReactionCategory, pub reaction_text: Option<&'a str>, pub onset: Onset<'a>, pub source: Source }`
  - `pub fn allergy_payload(e: &AllergyEntry) -> serde_json::Value` — Global Constraints' keys, `coding: null`,
    `refs: []`.
  - `pub fn allergy_none_payload(scope: NoneScope) -> serde_json::Value`.
  - `pub fn render_allergy_twin(e: &AllergyEntry) -> String` and `pub fn render_allergy_none_twin(scope) ->
    String` — the clinician wording of Task 4's `entry_line` / "No known allergies" / "No known drug allergies",
    without dates the twin cannot know (no "asked …").

- [ ] **Step 1: Write the failing tests:** `payload_has_exactly_the_frozen_keys` (key set, `coding == null`,
  `refs == []`); `every_enum_round_trips_its_wire_word` (`parse(x.as_str()) == Some(x)` for every variant, and
  `parse("Allergy") == None` — case matters); `not_known_onset_has_a_null_value`; `twin_keeps_text_verbatim`
  (term `<script>"5 < 10"` 🙂 appears unchanged in the twin); `twin_says_patient_stated_and_effect_not_known`
  (patient-stated + not-known → `"Patient stated allergy: penicillin. Effect not known."`);
  `none_twins` (`All` → `"No known allergies"`, `Drug` → `"No known drug allergies"`).
- [ ] **Step 2: Run** `cargo test -p cairn-event allergy::` — Expected: FAIL (module missing).
- [ ] **Step 3: Implement** `allergy.rs`. The twin's entry wording is the same rule as Task 4's `entry_line`;
  write it once here (`pub fn entry_words(e: &AllergyEntry) -> String`, no dates) and let Task 4 add the onset.
- [ ] **Step 4: Run** the same — Expected: PASS.
- [ ] **Step 5: Commit** `feat(allergy): the clinical.allergy/1 and allergy-none/1 bodies and twins`

### Task 2: The floor, the projections and the shred hook (`db/074_clinical_allergy.sql`)

**Files:**
- Create: `db/074_clinical_allergy.sql`, `db/tests/074_clinical_allergy_test.sql`
- Modify: `db/037_born_sealed.sql` (`cairn_execute_shred`, in place: two guarded deletes by `content_address`
  beside `clinical_note`'s); `db/044_ui_gesture_timing.sql` in place (`'allergy-record'`, `'allergy-none'` in
  `ui_gesture_timing_kind_ck`, both places); `crates/cairn-node/src/db.rs` (`SCHEMA`);
  `crates/cairn-event/src/schema_generation.rs` (→ 74); the pinned counts (Global Constraints)
- Test: the SQL file (`\ir _scratch_database_guard.sql`), and `crates/cairn-node/tests/allergy_floor.rs`

**Interfaces:**
- Produces (SQL): `event_type_class` rows for both types (`'additive'`, `FALSE`); `cairn_check_allergy_assertion(p_type
  text, b jsonb)` and `cairn_check_allergy_none_assertion(p_type text, b jsonb)`, revoked from PUBLIC, registered in
  `cairn_event_twin_check` with the #214 `ON CONFLICT DO UPDATE … IS DISTINCT FROM` idiom (copy db/064's).
- Tables, `GRANT SELECT … TO cairn_agent` (db/064's posture), indexed on `patient_id` and `content_address`:
  - `clinical_allergy(event_id uuid PK, patient_id uuid NOT NULL, allergen_kind text, term text, coding jsonb,
    kind text, reaction_category text, reaction_text text, onset_precision text, onset_value text, source text,
    refs jsonb NOT NULL, hlc_wall bigint, hlc_counter int, origin text, content_address bytea NOT NULL)` — the
    vocabulary columns are `text` with **no CHECK** (a peer's newer word is admitted and must project).
  - `clinical_allergy_none(event_id uuid PK, patient_id uuid NOT NULL, scope text NOT NULL, hlc_wall bigint,
    hlc_counter int, origin text, content_address bytea NOT NULL)`.
- Appliers `clinical_allergy_apply(e event_log)` / `clinical_allergy_none_apply(e event_log)` (read
  `cairn_clear_payload(e)`, return on NULL, `ON CONFLICT (event_id) DO NOTHING`), registered in
  `cairn_projection_apply` with order 20 and `heal_safe = TRUE`, as db/064 registers `clinical_note_apply`.
- Each floor rule its own `RAISE EXCEPTION 'allergy assertion: …'` / `'allergy-none assertion: …'`.

- [ ] **Step 1: Write the failing SQL test:** one accepted body of each type; one refused body per structural rule
  at both doors; one refused body per closed-vocabulary rule at the authoring door, and the same body **accepted**
  inside a transaction with `SET LOCAL cairn.remote_apply = 'on'` (db/tests/064's idiom); refused at the authoring
  door, accepted remotely: a non-null `coding`, a non-empty `refs`; refused at the authoring door: onset
  `2019-13`, `2019-02-31`, a year after the current year; `SELECT NOT cairn_event_type_has_no_thread(…)` for both.
- [ ] **Step 2: Write the failing Rust test** `allergy_floor.rs`: `a_sealed_entry_submits_and_projects` (build with
  Task 1 + the `note_body` skeleton in `note/sign.rs:103`, submit through `seal_sign_submit` with a human
  `AuthorParams`, assert one `clinical_allergy` row whose `term` equals the input);
  `a_none_statement_submits_and_projects`; `a_shred_scrubs_both_projections` (grep `cairn_execute_shred(` under
  `crates/cairn-node/tests/` for the existing helper).
- [ ] **Step 3: Run** `PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-gated-tests.sh` (#862) for the SQL file and
  `cargo test -p cairn-node --test allergy_floor -- --nocapture`. Expected: FAIL.
- [ ] **Step 4: Implement** db/074, the db/037 and db/044 edits, the SCHEMA entry, generation 74, the counts.
- [ ] **Step 5: Run** both, plus `cargo test -p cairn-node --test twin_registry --test projection_registry --test
  late_custody_guards --test floor_execute_grants` and `cargo test -p cairn-event --test schema_generation`.
  Expected: PASS, no `skipped:`.
- [ ] **Step 6: Commit** `feat(allergy): floor, projections and shred hook (db/074)`

### Task 3: `cairn_node::allergy` — sign and read

**Files:**
- Create: `crates/cairn-node/src/allergy/mod.rs`, `allergy/sign.rs`, `allergy/read.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod allergy;`); `crates/cairn-node/tests/db_errors_stay_legible.rs`
  (`MARKED_COMMIT_SITES` += `allergy/sign.rs`)
- Test: `crates/cairn-node/tests/allergy_sign.rs`, `allergy_read.rs`

**Interfaces:**
- Consumes: Task 1's types; `seal_sign_submit`, `AuthorParams`; `crate::patient::person::person_charts`;
  `crate::medication::read::set_has_doubted_link`; `crate::db::next_hlc`.
- Produces:
  - `pub async fn sign_allergy(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str,
    node_origin: &str, patient: Uuid, entry: &AllergyEntry<'_>, author: &AuthorParams<'_>) -> anyhow::Result<Uuid>`
  - `pub async fn sign_allergy_none(…same…, scope: NoneScope, author: &AuthorParams<'_>) -> anyhow::Result<Uuid>`
  - `pub struct AllergyRow { pub event_id: Uuid, pub chart: Uuid, pub allergen_kind: String, pub term: String,
    pub kind: String, pub category: String, pub reaction_text: Option<String>, pub onset_precision: String, pub
    onset_value: Option<String>, pub source: String, pub recorded_at: chrono::DateTime<Utc>, pub hlc: (i64, i32),
    pub signer: Option<String> }` — vocabulary fields stay `String` (a peer's newer word must still read).
  - `pub struct NoneRow { pub event_id: Uuid, pub chart: Uuid, pub scope: String, pub recorded_at: DateTime<Utc>,
    pub hlc: (i64, i32), pub signer: Option<String> }`
  - `pub struct PatientAllergies { pub entries: Vec<AllergyRow>, pub none: Vec<NoneRow>, pub doubted_charts:
    Vec<Uuid> }` — the member charts other than `patient` when the set holds a doubted link, else empty.
  - `pub async fn list_patient_allergies(client: &(impl GenericClient + Sync), patient: Uuid) ->
    anyhow::Result<PatientAllergies>` — every member chart's rows, one statement per table in the caller's
    snapshot, `recorded_at` from the HLC wall; `signer` from the same signer-name door the note list uses
    (`cairn_note_signer_name`) — reused, not rebuilt.

- [ ] **Step 1: Write the failing tests:** `signing_an_entry_records_human_authorship` (one `clinical_allergy` row;
  `event_log.contributors` has the human `authored` and the node `recorded`); `signing_none_known_records_its_scope`;
  `a_linked_chart_entries_are_read_with_their_chart` (two linked charts — reuse the linking helper the note-read
  tests use — an entry on each, both returned with their own `chart`); `a_doubted_link_names_the_other_charts`
  (`doubted_charts == [other]`, entries still returned); `an_unknown_word_from_a_peer_still_reads` (a row
  projected with `category = 'bronchospasm'` reads back as that string).
- [ ] **Step 2: Run** `cargo test -p cairn-node --test allergy_sign --test allergy_read -- --nocapture` — FAIL.
- [ ] **Step 3: Implement** the module; each `sign_*` builds the body as `note_body` does (`t_effective: None`,
  `safety: None`), then `seal_sign_submit(client, node_sk, body, Some(author), None)`.
- [ ] **Step 4: Run** the same plus `--test db_errors_stay_legible` — Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(allergy): sign an entry or a none-known statement; read a linked set`

### Task 4: The strip and the expanded list — pure view (`allergy/view.rs`)

**Files:**
- Create: `crates/cairn-node/src/allergy/view.rs`, `allergy/view_tests.rs` (`#[cfg(test)] #[path]`, as note does)

**Interfaces:**
- Consumes: Task 3's `PatientAllergies`; Task 1's `entry_words`.
- Produces:
  - `pub enum ListStatus { Entries, NoneKnown { asked: NaiveDate }, NotYetAsked }`
  - `pub struct EntryLine { pub event_id: Uuid, pub text: String, pub prominent: bool, pub chart: Uuid, pub
    doubted_chart: bool, pub signer: Option<String>, pub recorded: NaiveDate }`
  - `pub struct AllergyView { pub strip: String, pub drug: ListStatus, pub other: ListStatus, pub drug_allergies:
    Vec<EntryLine>, pub other_allergies: Vec<EntryLine>, pub adverse_effects: Vec<EntryLine>, pub
    previously_none: Vec<String> }`
  - `pub fn build_view(a: &PatientAllergies, opened: Uuid) -> AllergyView`
  - `pub fn entry_line_text(row: &AllergyRow) -> String` (wording + onset)
- The rules (design §3.3–§3.4; *my calls* marked in the design):
  - **drug status:** `Entries` if any drug row has kind `allergy`; else `NoneKnown` from the newest none row whose
    scope is `drug` or `all`; else `NotYetAsked`. **other status:** the same over `other` rows with scope `all`
    only. Adverse effects never set a status; they are counted.
  - A none row is **one statement**: once any allergy entry exists in a list it covers, it is overridden for every
    list it covers (an `all` row overridden by a penicillin allergy no longer says anything about other allergies).
  - Every none row that a status did not use (overridden by an entry, or older) becomes a `previously_none` line:
    `"Previously recorded: no known allergies (asked 8 Nov 2026)"` / `"… no known drug allergies …"`.
  - **Wording**, exact: seen-documented allergy `"Allergy: {term} → {reaction}"`; patient-stated `"Patient stated
    allergy: {term} → {reaction}"`, or with `not-known` `"Patient stated allergy: {term}. Effect not known."`;
    another-record `"Allergy (from another record): {term} → {reaction}"`; adverse effect `"Previous adverse
    effect: {term} → {reaction}"` with `" ({onset})"` after "effect" when known, and `" (patient stated)"` /
    `" (from another record)"` at the end when not seen-documented. `{reaction}` = the category's words
    (`anaphylaxis`, `angioedema`, `urticaria`, `rash`, `GI upset`) with `": {text}"` when text is given; `other` =
    the text alone (or `"other reaction"`); a seen-documented `not-known` = `"effect not known"`. An unknown peer
    word reads as itself. Allergy onset follows as `" · {onset}"`. Onset formats: `14 Mar 2019`, `Mar 2019`, `2019`
    (unambiguous by construction, #955).
  - **Order** within each list: anaphylaxis, not-known, angioedema, urticaria, rash, other, GI upset, unknown peer
    word last; ties newest first, then `event_id`.
  - `prominent` = drug or other entry of kind `allergy`. `doubted_chart` = `chart ∈ a.doubted_charts`; the line
    then ends `" (on a chart whose link to this one is in doubt)"`.
  - **The strip**, exact, joined by `" · "`: the drug part — every drug allergy line (never truncated), or `"No
    known allergies (asked {d})"` when both statuses are `NoneKnown` from one `all` row, or `"NKDA (asked {d})"`,
    or `"Allergies not yet asked"` when both are `NotYetAsked`, or `"Drug allergies not yet asked"`; then the other
    part — `"Other: "` + every other allergy line, or `"other allergies: not yet asked"` when drug is `NoneKnown`
    and other is `NotYetAsked`, or nothing when it says nothing new; then `"+{n} adverse effect(s)"` when `n > 0`.

- [ ] **Step 1: Write the failing tests** (pure, no DB), one per row of design §3.4's table with its exact strip
  string; `an_allergy_overrides_no_known_allergies_in_either_order` (two HLC orders, same view; the none row lands
  in `previously_none`); `an_allergy_on_a_linked_chart_overrides_a_none_on_this_one`;
  `six_drug_allergies_all_reach_the_strip`; `adverse_effects_are_counted_not_listed_on_the_strip`;
  `an_adverse_effect_does_not_override_nkda` (NKDA + metformin adverse effect → `"NKDA (asked 8 Nov 2026) · +1
  adverse effect"`); `the_maintainers_three_examples_read_exactly` (design §3.3 rows 1–3, with
  `"Allergy: penicillin → anaphylaxis · 2014"` for an uncoded term "penicillin");
  `order_puts_anaphylaxis_first_and_gi_upset_last`; `a_doubted_charts_entry_is_shown_and_marked`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib allergy::view` — Expected: FAIL.
- [ ] **Step 3: Implement** `view.rs` as small pure fns (`drug_status`, `other_status`, `strip_text`,
  `entry_line_text`, `order_key`).
- [ ] **Step 4: Run** the same — Expected: PASS.
- [ ] **Step 5: Commit** `feat(allergy): the header strip and the expanded list, pure`

### Task 5: The window's commands (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/allergies.rs`, `src/allergies_tests.rs`
- Modify: `src/main.rs` (`generate_handler!` += `allergy_list`, `allergy_record`, `allergy_none`);
  `crates/cairn-node/tests/window_role_grants.rs` (drive all three under `cairn_window`)

**Interfaces:**
- Consumes: Tasks 3–4; `AppState::{displayed_patient, signer_for_act, node_sk, is_mock}`, `check_displayed_set`,
  `refused`, `key_locked`, `node_cannot_write`, `record_timing` — exactly as `commands.rs::cease_impl` uses them.
- Produces:
  - `#[derive(Deserialize)] pub struct EntryInput { pub allergen_kind: String, pub term: String, pub kind:
    Option<String>, pub category: Option<String>, pub reaction_text: Option<String>, pub onset: Option<String>,
    pub source: String }`
  - `pub async fn allergy_list(state, patient_id: String) -> Result<AllergyView, ErrorView>` (serialize
    `AllergyView` as the other views are)
  - `pub async fn allergy_record(state, patient_id: String, charts: Vec<String>, entry: EntryInput, credential:
    Option<Credential>) -> Result<AllergyView, ErrorView>` — returns the re-read view
  - `pub async fn allergy_none(state, patient_id: String, charts: Vec<String>, scope: String, credential:
    Option<Credential>) -> Result<AllergyView, ErrorView>`
  - `pub fn parse_onset(s: Option<&str>, today: NaiveDate) -> Result<Onset, String>` — accepts empty/`None` →
    `NotKnown`, `2019`, `03/2019`, `14/03/2019`, and ISO `2019-03`, `2019-03-14`; refuses an impossible date or one
    after `today` with a sentence naming what was typed.
- Order of refusals in `allergy_record`, each before the key is asked (a passphrase that writes nothing is a step
  paper never has): displayed patient → displayed set → mock → empty term (`"Name the substance."`) → missing kind
  (`"Choose Allergy or Adverse effect."`) → missing category (`"Choose what happened, or Not known."`) → onset parse.
  Then `signer_for_act`, sign, `record_timing("allergy-record", 1, …)`, re-read.

- [ ] **Step 1: Write the failing tests** (`allergies_tests.rs`, following `commands_tests.rs`):
  `parse_onset_accepts_the_five_shapes`, `parse_onset_refuses_2019_13_and_31_02_2019_and_next_year`;
  `record_refuses_a_missing_kind_before_the_key` (no credential given; the error is the kind sentence, not a key
  sentence); `record_refuses_a_chart_not_on_screen`; DB-gated `record_then_list_shows_the_entry_on_the_strip`.
  In `window_role_grants.rs`: each command reaches its deep path as `cairn_window`.
- [ ] **Step 2: Run** the cairn-gui gate (memory "three Cargo trees": from `cairn-gui/`, `cargo test -p
  cairn-gui-tauri allergies -- --nocapture`) and `cargo test -p cairn-node --test window_role_grants` — FAIL.
- [ ] **Step 3: Implement** `allergies.rs` (under 500 lines; `parse_onset` pure).
- [ ] **Step 4: Run** the same — Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(window): allergy_list, allergy_record and allergy_none`

### Task 6: The strip and the form on the chart (`allergies.js`)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/allergies.js`, `src/allergies_webview_tests.rs`
- Modify: `src-ui/index.html` (inside `#identity`, after `#identity-id`: `<div id="allergy-strip" role="status">`
  and a `<details id="allergy-details">` holding the three lists, the "previously recorded" lines, the two none
  buttons and the form; the `<script>` tag); `src-ui/style.css` (`.allergy-prominent`, `.allergy-quiet`,
  `.allergy-doubted`); `src-ui/main.js` (call `drawAllergies(patientId)` wherever the chart's lists are drawn and
  on "Read this chart again"); `commands_tests.rs` script list; `signer_webview_tests.rs` `WRITERS` (+
  `recordAllergy`, `recordNoneKnown`)
- Test: `allergies_webview_tests.rs` (source-guard pins via `source_guard::js_fn` / `js_const`, as
  `notes_webview_tests.rs` does), and the mock walk (memory "webview mock walk recipe")

**Interfaces:**
- Consumes: Task 5's three commands; `signer.js`'s credential flow, as `notes.js` uses it.
- Produces: `drawAllergies(patientId)`, `recordAllergy()`, `recordNoneKnown(scope)`.
- The form (design §3.2): Substance (text), What kind (two toggle buttons, **none pressed initially**, `aria-pressed`),
  What happened (seven toggle buttons + optional text), When (text, placeholder `2019 · 03/2019 · 14/03/2019`, empty
  = not known), Source (three toggle buttons, *patient stated* pressed), Sign. Allergen kind (drug / other) is a
  two-button choice above Substance, *drug* pressed. A refusal from the command lands as text under its field.
- "No known allergies" and "NKDA" buttons show in the expanded list always, and on the strip itself when both
  statuses are `NotYetAsked`.

- [ ] **Step 1: Write the failing source-guard tests:** `allergies_js_never_assigns_inner_html`;
  `no_kind_button_starts_pressed`; `the_strip_is_inside_the_identity_header` (index.html order);
  `allergies_js_is_listed_and_loaded`.
- [ ] **Step 2: Run** `cargo test -p cairn-gui-tauri allergies_webview` (from `cairn-gui/`) — FAIL.
- [ ] **Step 3: Implement** `allergies.js` (under 500 lines; `funnel.js` is at 499 — touch it not at all).
- [ ] **Step 4: Run** the same and the full cairn-gui gate (`cargo clippy --locked`, `cargo test`, `cargo doc`
  with `RUSTDOCFLAGS=-D warnings`) — PASS. Then the mock walk in Chromium **and** WebKit: the four strip states of
  design §3.4, Sign with no kind (the inline line visible: assert computed `display` and no hidden ancestor), a
  six-allergy strip, a doubted-chart entry.
- [ ] **Step 5: Commit** `feat(window): the allergy strip and its form in the patient header`

### Task 7: ADR-0096, spec, RUNBOOK §27, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0096-the-allergy-list.md`
- Modify: `docs/spec/index.md` (v0.97 → **v0.98**), `docs/spec/decisions/README.md`, `docs/spec/data-model.md`
  §3.3 (allergy prose there — memory "medication spec-section mislabel"), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (§27),
  `docs/developers/durable-rules.md` (an "allergy slice 1" block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0096**: the two event types and their frozen payloads; the two-tier floor; none-known as
  a statement and the override rule; no clear safety signal and why (#406/#401; design §4, §7 risk 1); the doubted-
  chart choice; no co-sign; what is not guaranteed (a custody-less reader sees nothing; group checks are slice 3).
  Fact-check every sentence against the SQL before merge (memory "SDD evidence + ADR fact-check").
- [ ] **Step 2: RUNBOOK §27**: time a free-text entry (≤ 10 s) and NKDA (≤ 3 s) in the real window; read the strip
  in each of the four states; VoiceOver and keyboard over the toggles and the strip.
- [ ] **Step 3: File** the slice's findings as issues; run `scripts/check_closing_keywords.py`; update HANDOVER and
  ROADMAP (prune both).
- [ ] **Step 4: Run** the paper-parity guard `cargo test -p cairn-node --test paper_parity_plan_section` and the
  docs build `uv run --with-requirements docs/requirements.txt -- mkdocs build`. Expected: PASS.
- [ ] **Step 5: Commit** `docs(allergy): ADR-0096, spec v0.98, RUNBOOK §27, handover`
