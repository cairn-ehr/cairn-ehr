# Allergy list, slice 3 — drugref and the medication check — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A drug allergen can be **picked from drugref** (one drug, or a drugref allergy group such as *Penicillins*)
instead of typed. Every current medication line is **checked against the live drug allergies**:
- a confirmed hit carries a **loud tag and its words**, always visible on the line;
- an adverse effect is a quiet note;
- a line that could not be fully checked carries a **small visible "not checked" tag**.

Only positive checks are named.

**Architecture:** `substance.coding` (ADR-0096's reserved slot) opens for allergy entries with two systems:
`drugref-moiety` (one drug, ADR-0059) and `drugref-class` (an allergy group).

Cairn reads drugref through **one SQL file, `db/076_drug_read_door.sql`**:
- Its `SECURITY DEFINER` doors call drugref's own **`drugref.allergy_check(prescribed, allergen)`** (drugref
  db/068–069, PR #225), which folds both sides through every composition component and answers
  `match` / `no-match` / `unplaced`.
- The doors answer "unavailable" when drugref is absent or unreadable.
- So `cairn_window` gains no drugref grants, and a drugref change touches that one file.

A pure matcher (`cairn_node::allergy::check`) turns the live heads, the current medication lines and the door's answers
into per-line marks, notes and an unverified flag. The window computes them inside `med_list`.

**Tech Stack:** PostgreSQL 18 (new `db/076`; in-place edits to `db/074`, `db/041`, `db/042`), Rust (`cairn-event`,
`cairn-node`, `cairn-medication-view`, `cairn-gui-tab-medications`, `cairn-gui-tauri`), Tauri 2 + plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-08-allergy-stream-design.md`](../specs/2026-11-08-allergy-stream-design.md)
§3.2 (substance coding), §3.7 (the check and its honest degrade), §4 (drugref is optional and co-resident), §7 risks 2–3.
drugref's side: `~/src/drugref` `docs/superpowers/specs/2026-11-08-drugref-allergy-groups-design.md` and
`db/067_allergy_groups.sql` / `db/068_allergy_check.sql`. Read [ADR-0096](../../spec/decisions/0096-the-allergy-list.md),
[ADR-0097](../../spec/decisions/0097-allergy-overlays-and-differing-accounts.md),
[ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) and `docs/developers/durable-rules.md`'s
**ALLERGY SLICE 1** and **ALLERGY SLICE 2** blocks before starting. Every rule there still holds.

## Decisions taken (maintainer)

2026-11-09:
- **M1. The read door is SQL, in Cairn's own database.** drugref is installed into the Cairn database as its `drugref`
  schema. One db file reads it. `no_drugref_dependency.rs` exempts **that file only**, and `drug_door_callers.rs` forbids
  any other `db/` or `extensions/` file from calling the doors.
- **M2. A drug group is coded `{system: "drugref-class", code: <class_uuid>, display: <group name at coding time>}`.**
  drugref's `class_uuid` is immortal and stable. The token is wire-forever once the first coded group is signed.
- **M3.** *Superseded by M4.* (Salts and hydrates count as the same drug through drugref's composition tree.)

2026-11-10, after drugref PR #225 shipped allergy groups:
- **M4. Delegate.** The facts door calls `drugref.allergy_check`, and Cairn keeps no fold or membership logic of its own.
  drugref folds **every** composition component, not only the active ones (its owner's ruling: what is
  pharmacologically active is not what triggers an allergy).
- **M5. The pick offers only drugref allergy groups** (`drugref.allergy_group`, `concept_type` `AG`) as groups, never
  MED-RT EPC/PA classes. `allergy_check` answers `allergen-not-a-group` for any other class.
- **M6. Only positive checks are named.**
  - A confirmed allergy hit gets a **loud tag** plus its words.
  - A line not fully verified gets a **small visible tag** and no words.
  - The strip lists nothing unverified. The design's per-allergen "not fully checked" line is dropped.
- **M7. The group's review status shows in the expanded allergy list only.** Every group is `structural-seed` today. The
  line on the medication list stays terse.

## Designer's calls (open to correction; each named in the ADR)

- **D1. Only drug entries are coded.** The authoring door refuses a coding on an `other` entry. The remote door checks
  structure only.
- **D2. A de-label's `tolerated.coding` stays null.** The tolerated drug feeds no check.
- **D3. `drugref-class` is registered in `medication_coding_system`**, and the medication doors refuse it at the
  authoring door (built in Task 2).
- **D7. Hits.** A pair (allergy head, current line) is a hit when any of these holds (over-warning is the safe error):
  - the names match by `thread::same_allergen`, comparing the head's term with the line's term or coding display; or
  - both are coded and the codes are identical (this works without drugref); or
  - `allergy_check` answered `match`.

  There is one mark per allergen (per `same_allergen_rows` group), worded with the most severe live head. `not-known`
  ranks with anaphylaxis.
- **D8. Unverified.** A line is **unverified** when it has no allergy mark and at least one live **allergy-kind** head was
  neither a hit nor answered `no-match` for it. That covers:
  - either side uncoded;
  - drugref unavailable;
  - `unplaced`, with any of its reasons, including `allergen-in-no-group`;
  - no answer row.

  Adverse-effect heads never make a line unverified (risk 3). A `no-match` is taken as drugref's honest verified
  negative. Its known gap (no cephalosporin cross-allergy rule) goes in the ADR, not on the screen.
- **D9. Every live head counts**, including a doubted chart's heads and a suspect key's. Ceased lines are never checked.
  Withheld lines are current and are checked.
- **D10. Two accounts are the same allergen** (differing accounts, ADR-0097 D5) when their terms match **or** they carry
  the same `(system, code)`.
- **D11. A group's review status reads as not reviewed** unless the door says `reviewed`. That covers `structural-seed`,
  an unknown future word, and a failed read.

## Global Constraints

- **Wire**, frozen once each first event is signed (principle 11). No new event type, no new `schema_version`:
  - `clinical.allergy/1`'s `substance.coding` is `null` or exactly `{system, code, display}`.
  - At the authoring door: `system` ∈ {`drugref-moiety`, `drugref-class`}, `code` is a canonical lowercase uuid, and
    `allergen_kind` is `drug`.
  - Challenge `tolerated.coding` stays `null`.
  - `safety: None` stays.
- **Twin:** a coded entry's twin ends `" (coded as {display} [{system}])"`. An uncoded twin is unchanged (built, Task 1).
- **The doors** (db/076). Each is `SECURITY DEFINER` with `SET search_path = pg_catalog, pg_temp`, every `drugref.` name
  schema-qualified, `REVOKE … FROM PUBLIC`, `GRANT EXECUTE … TO cairn_window`, and **no function name contains
  "drugref"**:
  - `cairn_drug_available() RETURNS boolean`. True only when all of these hold, read from the system catalogue (never
    `to_regclass`, ruling T2-a), and it never raises:
    - the owner holds USAGE on schema `drugref`;
    - `drugref.allergy_check(uuid, uuid)` exists and the owner may EXECUTE it;
    - `drugref.substance_moiety`, `drugref.substance_class` and `drugref.allergy_group` exist and the owner may SELECT
      them.
  - `cairn_drug_search(p_text text, p_limit int) RETURNS TABLE(system text, code text, display text)`, as built in
    Task 2 (literal match, at least 3 characters, limit capped at 20, moieties first — amended by the final review's
    ruling F2 (#1057): GROUPS first, then moieties, because with the real drugref the group was crowded out of the
    window's 12 options), except:
    - groups come from `drugref.allergy_group` joined to `drugref.substance_class` (M5);
    - `system` is `drugref-class` and `display` is the class name.
  - `cairn_drug_facts(p_allergens uuid[], p_meds uuid[]) RETURNS TABLE(allergen uuid, med uuid, outcome text, match_kind text, group_name text)`:
    - one row per row `drugref.allergy_check(med, allergen)` returns, for every pair;
    - zero rows when unavailable.
  - `cairn_drug_group_status(p_groups uuid[]) RETURNS TABLE(group_code uuid, review_status text)`: zero rows when
    unavailable.
- **Clinician words** (exact). `{reaction}` is `cairn_event::allergy::reaction_phrase`; `{line name}` is
  `MedicationRow::display_name()`:
  - Same drug (a name, code or `same-drug` hit): `"Allergy recorded: {allergen term} → {reaction}"`.
  - Group allergen, `same-group`: `"{line name} is a {allergen term}; allergy recorded: {reaction}"`.
  - Moiety allergen, `same-group`: `"{line name} is in the same group as {allergen term} ({group name}); allergy recorded: {reaction}"`.
  - Adverse-effect note: `"Previous adverse effect: {allergen term} → {reaction}"`.
  - Loud tag text: `Allergy`. Small tag text: `not checked`. Both are visible text, never a hover-only `title` (#871).
  - Expanded-list suffix for a group-coded entry whose group is not reviewed (D11):
    `" (group from drugref, not clinically reviewed)"`.
  - Med pane, when the allergy read failed: `"Allergies could not be read: medications not checked against them"`.
  - Pick hint with no drug list: `"No drug list on this terminal: the name is recorded as typed"`. A group option reads
    `"{display} (group)"`.
- `SCHEMA_GENERATION` 76 (built). Files stay under 500 lines: new code goes in new files.

## Review Focus

1. **Typed wildcards and short text in the pick:** they match literally, and fewer than 3 characters gives no options.
   Pinned in Task 2 and kept in Task 2R: `search_text_is_literal_and_short_text_finds_nothing`.
2. **Picking a drug and then editing the name** drops the coding. Pinned in Task 6:
   `editing_the_name_after_a_pick_drops_the_coding`.
3. **drugref installed without allergy groups, or unreadable by the owner**, gives `cairn_drug_available()` false, not
   a fault, and the medication list still draws. Pinned in Task 2R: `an_unreadable_drugref_is_unavailable_not_a_fault`
   and `a_drugref_without_allergy_check_is_unavailable`.
4. **The allergy read fails while the medication list is read:** the list still draws, with the worded line. Pinned in
   Task 5: `med_list_draws_when_the_allergy_read_fails`.
5. **A salt of the allergen with drugref absent:** no mark, and the line carries the small tag, so silence never reads
   as checked. Pinned in Task 4: `a_salt_without_drugref_is_not_checked`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing "Penicillin — anaphylaxis" in the drug chart's allergy box, and the prescriber or
  pharmacist reading that box against every line of the drug chart before giving a dose.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Record a coded entry: 1 (write the line) → 1 (one signed event; the coding rides on it) → 1 (type, choose from the
    list or keep typing, Sign). Picking is optional, never forced.
  - Check the medication list: 1 (read the box against the chart) → 0 human acts (computed on every read) → 0.
  - `M ≤ N` everywhere: no architecture defect to file.
- **Time + cognitive load:**
  - A coded entry within ≤ 15 s and a free-text one within ≤ 10 s. The pick list appears within ≤ 300 ms of the third
    letter.
  - A hit is legible with no click or hover: the loud tag plus its words on the line (#871). An unverified line is told
    apart from a checked one at a glance.
  - The medication pane's open time must not grow by more than 100 ms with drugref present.
  - Owed by a human in **RUNBOOK §29** (Task 7).

---

### Task 1: The coded entry (`cairn-event`, pure) — DONE (0275c811)

`AllergyEntry.coding`, the coded payload and twin suffix, and `reaction_phrase` made `pub`.

### Task 2: The floor opens the slot; the drug read door — DONE (47afa73e, a1d0e0d8)

- The floor admits a coded drug entry (db/074).
- `drugref-class` is registered, and the medication doors refuse it (db/041, db/042).
- The first db/076 has three doors, Cairn's own fold, and a stub.
- `no_drugref_dependency.rs` exempts db/076, and `drug_door_callers.rs` forbids other callers.
- `SCHEMA_GENERATION` 76.

Task 2R replaces its fold and its stub.

### Task 2R: The facts door delegates to `drugref.allergy_check` (db/076, its tests, the stub)

**Files:**
- Modify: `db/076_drug_read_door.sql`.
  - `cairn_drug_available` is redefined.
  - `cairn_drug_search`'s group branch reads `drugref.allergy_group`.
  - `cairn_drug_facts` returns the new columns and calls `drugref.allergy_check`. A changed `RETURNS TABLE` needs a
    guarded `DROP FUNCTION IF EXISTS` of the old signature before the `CREATE`, so every connect's replay converges.
  - Add `cairn_drug_group_status`.
  - **Delete** Cairn's family and membership SQL entirely, and say in the header that drugref owns the fold (M4).
- Rewrite: `crates/cairn-node/tests/fixtures/drug_stub.sql`. It holds:
  - the minimal `drugref.substance_moiety(moiety_uuid, display_name)`, `drugref.substance_class(class_uuid, class_name,
    concept_type)` and `drugref.allergy_group(class_uuid, review_status)` tables;
  - a `drugref.allergy_check(p_prescribed uuid, p_allergen uuid)` with **drugref db/068's exact `RETURNS TABLE`**
    (`outcome, match_kind, reason, group_class_uuid, group_name, group_mesh_ui, review_status, allergen_component,
    prescribed_component`), answering from a small stub table of canned rows.

  Rows: moieties amoxicillin, flucloxacillin, perindopril and metformin; the group `Penicillins` (AG, `structural-seed`);
  a MED-RT class `Penicillin-class Antibacterial [EPC]` (EPC), which search must never offer; and canned answers covering
  every outcome (`match` same-group, `match` same-drug, `no-match`, `unplaced`). Uuids are computed, never literal.
- Rewrite: `db/tests/076_drug_read_door_test.sql` sections 0–3 and the matching tests in
  `crates/cairn-node/tests/drug_read_door.rs`. Section 4 (the floor) stays as built.
- Modify: ADR wording is Task 7's. `drug_door_callers.rs` and `no_drugref_dependency.rs` stay valid. Re-run them.

**Interfaces — Produces:** the four doors exactly as in Global Constraints.

- [ ] **Step 1: Failing tests.** Every stub load stays inside a transaction that ends in ROLLBACK (ruling P1's role
  pattern for privilege cases):
  - (0) With no schema, `cairn_drug_available()` is false and every door returns zero rows.
  - (1) Search:
    - "amox" finds amoxicillin;
    - "penicillin" finds the `Penicillins` group and never the EPC class;
    - `search_text_is_literal_and_short_text_finds_nothing` is kept;
    - the limit cap is kept.
  - (2) Facts: for the 4 × 2 pairs the stub answers, the door returns exactly the stub's rows, mapped to `(allergen,
    med, outcome, match_kind, group_name)` and compared as a full set. Group status returns `structural-seed` for
    Penicillins.
  - (3) `an_unreadable_drugref_is_unavailable_not_a_fault` (the owner lacks EXECUTE on `allergy_check`, or SELECT on
    `allergy_group`) and `a_drugref_without_allergy_check_is_unavailable` (stub tables present, function absent): false,
    zero rows, no error.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run the SQL file, `drug_read_door.rs`,
  `drug_door_callers`, `no_drugref_dependency`, `allergy_floor.rs` with `--nocapture`, and check there is no `skipped:`.
  Then fmt and clippy.
- [ ] **Step 5:** Commit `refactor(allergy): the drug door delegates to drugref's allergy check`.

### Task 3: Read the coding and the drug answers (`cairn_node::allergy`, `cairn-medication-view`)

**Files:**
- Create: `crates/cairn-node/src/allergy/drug_facts.rs` (async, `GenericClient`).
- Modify:
  - `allergy/read.rs`: `AllergyRow` gains `pub coding: Option<Coding>`, read from `clinical_allergy.coding`. A JSON object
    missing a key reads as `None`, never a fault.
  - `allergy/test_fixtures.rs`: `coding: None`.
  - `allergy/thread.rs`: add `same_allergen_rows(a: &AllergyRow, b: &AllergyRow) -> bool` (D10), and use it where
    `differing` compares rows.
  - `allergy/mod.rs`: export.
- Modify: `crates/cairn-medication-view/src/row.rs`. `MedicationRow` gains `coding_system: Option<String>` and
  `coding_code: Option<String>`, each `#[serde(skip_serializing_if = "Option::is_none")]` so the `--json` golden is
  unchanged. `medication/read.rs` selects them beside `coding_display`. Fix every constructor that
  `rg -n "coding_display:" crates cairn-gui` finds.

**Interfaces — Produces:**
- `pub struct Coding { pub system: String, pub code: String, pub display: String }` in `read.rs`.
- `pub enum Answer { Match(Vec<MatchPath>), NoMatch, Unplaced }`, with
  `pub struct MatchPath { pub kind: String, pub group_name: Option<String> }`. `kind` is the wire word: `same-drug`,
  `same-group`, or a newer word.
- `pub enum DrugFacts { Unavailable, Available(HashMap<(Uuid, Uuid), Answer>) }`, keyed `(allergen code, med code)`. When
  rows disagree, a pair with any `match` row is `Match`. A pair with no row is absent.
- `pub async fn read_drug_facts(c: &impl GenericClient, allergens: &[Uuid], meds: &[Uuid]) -> anyhow::Result<DrugFacts>`
  calls `cairn_drug_available()` and then `cairn_drug_facts` on the same client. **Callers map `Err` to
  `DrugFacts::Unavailable`** (a drugref of unexpected shape makes the doors raise). Say so in the doc.
- `pub async fn read_group_status(c: &impl GenericClient, groups: &[Uuid]) -> anyhow::Result<HashMap<Uuid, String>>`.
- `pub struct DrugOption { pub system: String, pub code: String, pub display: String }`.
- `pub async fn search_drugs(c: &impl GenericClient, text: &str) -> anyhow::Result<Option<Vec<DrugOption>>>`: `None`
  when no drug list is available. Limit 12.

- [ ] **Step 1: Failing tests:**
  - `same_allergen_rows_matches_on_code_or_term` (unit);
  - `a_coded_entry_reads_back_its_coding` (DB, through `sign_allergy`);
  - `drug_facts_reads_the_stub`, `drug_facts_without_drugref_is_unavailable` and `group_status_reads_the_stub` (DB, in
    `drug_read_door.rs`);
  - **ruling T2-b:** `the_window_role_can_call_the_doors` (`SET ROLE` to a role holding only `cairn_window`'s grants,
    inside the rolled-back transaction).
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement, with the reads wrapped in the crate's local-DB-fault wording (bump
  `db_errors_stay_legible.rs`'s per-file count if it applies). **Step 4:** Run those tests,
  `cargo test -p cairn-medication-view`, the `--json` golden, `cargo test -p cairn-node --lib allergy` and
  `no_drugref_dependency` → PASS. Then fmt and clippy, and `cargo check` in `cairn-gui/`.
- [ ] **Step 5:** Commit `feat(allergy): read an entry's coding and drugref's answers`.

### Task 4: The check (`cairn_node::allergy::check`, pure)

**Files:**
- Create: `crates/cairn-node/src/allergy/check.rs` and `check_tests.rs`.
- Modify: `view.rs`. Make the reaction rank (line ~163) `pub(crate)`. Add the M7 / D11 suffix on the expanded list
  through a new pure `note_unreviewed_groups` (below), so `build_view`'s signature is unchanged and the strip is untouched.

**Interfaces — Consumes:** `Resolved`/`Head` (thread.rs), `AllergyRow::coding`, `DrugFacts`/`Answer` (Task 3),
`reaction_phrase`. **Produces:**
- `pub struct MedLine<'a> { pub group_id: Uuid, pub name: &'a str, pub term: &'a str, pub coding_display: Option<&'a str>, pub moiety: Option<Uuid>, pub current: bool }`.
  `moiety` is the code when the system is `drugref-moiety`.
- `pub struct LineCheck { pub marks: Vec<String>, pub notes: Vec<String>, pub unverified: bool }`. Invariant:
  `unverified` is false whenever `marks` is non-empty.
- `pub fn codes_to_ask(r: &Resolved, meds: &[MedLine]) -> (Vec<Uuid>, Vec<Uuid>)` returns the live drug heads'
  `drugref-*` codes and the current lines' moieties, sorted and deduplicated.
- `pub fn check_medications(r: &Resolved, meds: &[MedLine], facts: &DrugFacts) -> HashMap<Uuid, LineCheck>` follows
  D7, D8 and D9. A line with nothing to say has no key.
- `pub fn unreviewed_groups(r: &Resolved, status: &HashMap<Uuid, String>) -> HashSet<Uuid>` follows D11, over the live
  heads' `drugref-class` codes.
- `pub fn note_unreviewed_groups(view: AllergyView, a: &PatientAllergies, unreviewed: &HashSet<Uuid>) -> AllergyView`
  appends the suffix to the expanded entry line of every such entry.

- [ ] **Step 1: Failing unit tests** (`check_tests.rs`, built on `test_fixtures` plus a `coded(row, system, code)`
  helper):
  - `same_code_marks_the_line_without_drugref`;
  - `same_name_marks_an_uncoded_line`;
  - `a_same_drug_answer_marks_the_line`;
  - `a_group_allergen_hit_names_the_group_allergen` (exact words);
  - `a_moiety_allergen_same_group_hit_names_the_group` (exact words, the amoxicillin → flucloxacillin case);
  - `an_adverse_effect_is_a_note_never_a_mark`;
  - `a_delabelled_or_withdrawn_entry_checks_nothing`;
  - `a_ceased_line_is_never_checked`;
  - `differing_accounts_mark_once_with_the_more_severe`;
  - `not_known_ranks_with_anaphylaxis`;
  - `a_doubted_charts_entry_still_marks`;
  - `latex_never_touches_the_medication_list`;
  - `no_match_is_verified`;
  - `unplaced_is_not_checked`;
  - `an_uncoded_allergy_leaves_every_line_not_checked`;
  - `an_uncoded_line_is_not_checked`;
  - `a_salt_without_drugref_is_not_checked` (Review Focus 5);
  - `a_marked_line_is_never_also_unverified`;
  - `an_adverse_effect_never_makes_a_line_unverified`;
  - `a_group_not_reviewed_is_marked_in_the_expanded_list_only`;
  - `an_unknown_review_word_reads_as_not_reviewed`.
- [ ] **Step 2:** `cargo test -p cairn-node --lib allergy::check` → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
  Then `cargo test -p cairn-node --lib allergy`.
- [ ] **Step 5:** Commit `feat(allergy): check the medication list against the live drug allergies`.

### Task 5: The window's commands (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/allergy_check.rs` and `allergy_check_tests.rs`.
- Modify:
  - `allergies.rs`: `allergy_record` takes `coding: Option<CodingArg>`, with `CodingArg { system, code, display }`.
    `code` is parsed as `Uuid` and re-spelled canonically; an unparsable one is refused `"That drug's code could not be
    read; type the name instead"`. `allergy_list_impl` reads the group status in the same snapshot as the rows, and
    applies `note_unreviewed_groups`. A failed status read counts every group as not reviewed (D11).
  - `allergy_overlays.rs`: Correct takes `coding` the same way.
  - `commands.rs::med_list_impl`: reads the allergies (`allergies::read_list`) and the facts. A failed facts read is
    `DrugFacts::Unavailable`, never a failed pane. It hands the `HashMap<Uuid, LineCheck>`, or the allergy-read
    failure line, to `chart_pane`.
  - `chart_set/mod.rs::chart_pane` gains that parameter. `MedListRowView` (`cairn-gui-tab-medications/src/row_view.rs`)
    gains `pub allergy_marks: Vec<String>`, `pub allergy_notes: Vec<String>` and `pub allergy_unverified: bool`. The pane
    gains `pub allergy_check_failed: Option<String>`.
  - Add a `drug_search` command, registered in `main.rs` and the source guard's command list.

**Interfaces — Produces** (JSON as `serde` writes it):
- `drug_search(text) -> DrugSearch { available: bool, options: Vec<{system, code, display}> }`. Fixture mode gives
  `available: false`.
- `allergy_record(..., coding)`.
- `MedListRowView.allergy_marks`, `.allergy_notes` and `.allergy_unverified`.
- `ChartPane.allergy_check_failed`.

- [ ] **Step 1: Failing tests:**
  - `allergy_check_tests.rs`: `med_lines_come_from_the_rows` (pure);
  - DB tests beside `allergies_tests.rs`:
    - `a_picked_group_is_signed_with_its_coding`;
    - `a_bad_code_is_refused_before_signing`;
    - `med_list_marks_a_line_against_a_coded_allergy` (stub in the test's own transaction);
    - `med_list_flags_an_unplaced_line_not_checked`;
    - `med_list_draws_when_the_allergy_read_fails` (Review Focus 4, the `SET ROLE` fault seam);
    - `allergy_list_marks_an_unreviewed_group`.
- [ ] **Step 2:** Run → FAIL. `cairn-gui-tauri`'s DB tests do not run in CI (#905), so run them locally.
  **Step 3:** Implement. **Step 4:** Run the `cairn-gui` gate (fmt, clippy `-D warnings`, test, `cargo doc` with
  `RUSTDOCFLAGS=-D warnings`) and the root source guards DB-free → PASS.
- [ ] **Step 5:** Commit `feat(window): drug search, coded entries, and the check on the medication list`.

### Task 6: The pick and the tags in the window (JS)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/drug_pick.js`. The type-ahead under `#allergy-term`:
  - it shows only while the kind is *drug*, and asks `drug_search` 150 ms after the last keystroke, from the third
    character;
  - arrow keys, Enter and click choose an option; Escape closes the list;
  - a choice sets the term to the option's display and holds the coding;
  - **any later edit of the term drops the coding**;
  - with `available: false` it shows the hint line.
  - It is loaded before `allergies.js` in `index.html` (ruling P2: and wherever an src-ui inventory guard requires it).
- Modify:
  - `allergies.js`: send `coding` with Record and Correct, and clear it with the form.
  - `main.js`: a row with `allergy_marks` draws the loud `Allergy` tag and each mark as visible text on the line;
    `allergy_notes` are muted; `allergy_unverified` draws the small `not checked` tag. Draw `allergy_check_failed` above
    the list. Split `refresh` so its `med_list` half is `readMedList(lead, behind)`. `allergies.js` calls
    `readMedList()` after any act answered Ok.
- Test: webview tests in `allergies_webview_tests.rs`, or a new `drug_pick_webview_tests.rs` if that file would pass 500
  lines:
  - `the_pick_offers_drugs_and_groups`;
  - `editing_the_name_after_a_pick_drops_the_coding`;
  - `no_drug_list_shows_the_hint`;
  - `a_marked_line_shows_its_tag_and_mark_without_hover` (computed display and no hidden ancestor);
  - `an_unverified_line_shows_the_small_tag`;
  - `recording_an_allergy_rereads_the_medication_list`.

- [ ] **Step 1:** Failing webview tests. **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run the webview tests
  and the `cairn-gui` gate → PASS. Walk once headlessly in Chromium and assert visibility. WebKit is owed in RUNBOOK §29.
- [ ] **Step 5:** Commit `feat(window): pick a drug or group; tags and marks on the medication list`.

### Task 7: ADR-0098, spec, RUNBOOK §29, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0098-allergy-coding-and-the-medication-check.md`. It records:
  - M1–M7 and D1–D11, with ruling T2-a (the catalogue lookup) restated;
  - the no-safety-signal ruling;
  - **the drugref contract Cairn consumes**: `drugref.allergy_check` (db/068–069), its three outcomes and reasons,
    `drugref.allergy_group` (`review_status`), and the rule that only db/076 changes when drugref changes;
  - drugref's known gap: `no-match` is not "cephalosporins (or carbapenems) are safe".
- Modify:
  - `docs/spec/decisions/README.md`;
  - `docs/spec/index.md` (v0.100);
  - `docs/spec/data-model.md` §3.3 (allergies) and §3.16's drug axis (the `drugref-class` token, now naming an allergy
    group);
  - `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §29:
    - build drugref **with its allergy groups** (`drugref ingest allergy-groups`; `drugref_profiles` lacks them today,
      `drugref_test` has db/067–069);
    - install it with `pg_dump -n drugref <db> | psql <cairn db>`;
    - the grants the owner needs;
    - then time the budgets, and check VoiceOver on both tags and WebKit;
  - `docs/developers/durable-rules.md`, an **ALLERGY SLICE 3** block:
    - the one-file exemption and the callers guard;
    - no "drugref" in a door name;
    - drugref owns the fold, so never re-add one in Cairn;
    - an unverified line always carries its tag, so never trim it to hit-only marks (risk 2);
    - adverse effects never mark and never flag unverified;
  - `docs/HANDOVER.md` and `docs/ROADMAP.md`.
- File issues for deferred items, each named in the ADR: coding the tolerated drug (D2); the check at the moment of
  charting; the cephalosporin cross-allergy rule (on drugref's side); the deferred minors the final review keeps. Run
  `scripts/check_closing_keywords.py` before committing.
- [ ] **Step 1:** Write the docs. Fact-check the ADR sentence by sentence against db/076 and `check.rs`.
- [ ] **Step 2:** Build the docs with `uv run --with-requirements docs/requirements.txt -- mkdocs build`, and run
  `paper_parity_plan_section.rs`.
- [ ] **Step 3:** Commit `docs(allergy): ADR-0098, spec v0.100, RUNBOOK §29, durable rules`.
