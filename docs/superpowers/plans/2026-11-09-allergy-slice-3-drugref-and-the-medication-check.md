# Allergy list, slice 3 — drugref and the medication check — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A drug allergen can be **picked from drugref** (one drug, or a drug group) instead of typed. Every current
medication line is **checked against the live drug allergies**: a hit is a mark that is always visible on the line,
an adverse effect is a quiet note, and the allergy strip says plainly, for each allergen, when the check **could not be
complete**.

**Architecture:** `substance.coding` (ADR-0096's reserved slot) opens for allergy entries with two systems:
`drugref-moiety` (one drug, ADR-0059) and the new `drugref-class` (a group). Cairn reads drugref through **one SQL file,
`db/076_drug_read_door.sql`**. It holds three `SECURITY DEFINER` doors that read the co-resident `drugref` schema when it
is present and readable, and answer "unavailable" otherwise. So `cairn_window` gains no drugref grants, and a drugref
schema change touches that one file. A pure matcher (`cairn_node::allergy::check`) turns the live heads, the current
medication lines and the door's facts into per-line marks and per-allergen "not fully checked" lines. The window computes
the marks inside `med_list` and the lines inside `allergy_list`.

**Tech Stack:** PostgreSQL 18 (new `db/076`; in-place edits to `db/074`, `db/041`, `db/042`), Rust (`cairn-event`,
`cairn-node`, `cairn-medication-view`, `cairn-gui-tab-medications`, `cairn-gui-tauri`), Tauri 2 + plain JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-08-allergy-stream-design.md`](../specs/2026-11-08-allergy-stream-design.md)
§3.2 (substance coding), §3.7 (the check and its honest degrade), §4 (drugref is optional and co-resident), §7 risks 2–3.
Read [ADR-0096](../../spec/decisions/0096-the-allergy-list.md), [ADR-0097](../../spec/decisions/0097-allergy-overlays-and-differing-accounts.md),
[ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) and `docs/developers/durable-rules.md`'s
**ALLERGY SLICE 1** and **ALLERGY SLICE 2** blocks before Task 1. Every rule there still holds.

## Decisions taken (maintainer, 2026-11-09)

- **M1. The read door is SQL, in Cairn's own database.** drugref is installed into the Cairn database as its `drugref`
  schema. One db file reads it. `no_drugref_dependency.rs` is narrowed to exempt **that file only**: the floor and the
  projections still never read a drug database.
- **M2. A drug group is coded `{system: "drugref-class", code: <class_uuid>, display: <class name at coding time>}`.**
  drugref's `class_uuid` is an immortal UUIDv5 derived from the source's id. It is source-neutral, so drugref's later
  curated groups fit the same token. The token is wire-forever once the first coded group is signed.
- **M3. Salts and hydrates count as the same drug, through drugref's composition tree.** A moiety's **family** is
  itself plus the active components of the composite its current UNII names. Measured 2026-11-09 in `drugref_profiles`:
  *amoxicillin* and *amoxicillin sodium* both have the active component *amoxicillin anhydrous*, which is the one member
  of *Penicillin-class Antibacterial [EPC]*. Without the walk, the penicillin check misses amoxicillin.

## Designer's calls (open to correction; each named in the ADR)

- **D1. Only drug entries are coded.** The authoring door refuses a coding on an `other` entry. The remote door checks
  structure only (ADR-0051 two tiers).
- **D2. A de-label's `tolerated.coding` stays null.** The tolerated drug feeds no check: a de-labelled line already
  leaves every check. Opening it is additive later.
- **D3. `drugref-class` is registered in `medication_coding_system`** (the one coding registry), noted as "a drug GROUP —
  allergen coding only". The medication doors (db/041's assertion check, db/042's overlay check) refuse it at the
  authoring door. A medication is one drug, never a group.
- **D4. The pick offers moieties and groups whose `concept_type` is `EPC` or `PA` only.** PE, PK, MoA, TC and APC
  classes ("Renal Excretion [PK]", "Transpeptidase Inhibitors [MoA]") are not allergy groups a clinician means. This is a
  constant in db/076.
- **D5. Membership counts any `class_membership.relationship`**, over the class and its descendants (`class_parent`),
  reached by any member of the medication's family.
- **D6. No group is declared complete today.** drugref has no completeness statement, so the door's `complete` fact
  is never produced and every group reads "not fully checked". The ADR writes down the contract drugref implements to
  change that (Task 7).
- **D7. When a name matches, it always counts** (over-warning is the safe error). A mark is made when the allergen and
  the line name the same drug by normalised name (`thread::same_allergen`, against the line's term and its coding
  display), **or** by code (same code, or a shared family member), **or** by group membership. One mark per allergen
  (per `same_allergen_rows` group, Task 3), worded with the most severe live head. `not-known` ranks with anaphylaxis
  (design §4).
- **D8. Who gets a "not fully checked" line.** It goes on each live drug **allergy** head, never on an adverse effect
  (risk 3), deduplicated per allergen, when:
  - the allergen is uncoded; or
  - it is group-coded and drugref is unavailable, or the group is not declared complete; or
  - it is moiety-coded and drugref is unavailable (salts are then unwalked).
- **D9. Every live head counts**, including a doubted chart's heads and a suspect key's (ADR-0097: a suspect act may
  only ADD a warning). Ceased medication lines are never checked. Withheld lines are current and are checked.
- **D10. Two accounts are the same allergen** (differing accounts, ADR-0097 D5) when their terms match **or** they carry
  the same `(system, code)`. Slice 2 left coded equality to this slice.

## Global Constraints

- **Wire**, frozen once each first event is signed (principle 11). No new event type, no new `schema_version`:
  - `clinical.allergy/1`'s `substance.coding` is `null` or exactly `{system, code, display}`, three non-blank strings
    (db/041's `cairn_check_coding_object`).
  - At the authoring door: `system` is `drugref-moiety` or `drugref-class`; `code` is a canonical lowercase uuid;
    `allergen_kind` is `drug`.
  - Challenge `tolerated.coding` stays `null` (D2).
  - `safety: None` stays (ADR-0096 decision 21).
- **Twin**, exact copy. A coded entry's twin is the slice-1/2 twin followed by `" (coded as {display} [{system}])"`
  (ADR-0059's "coded as" words). An uncoded twin is byte-identical to today's.
- **The doors** (db/076), each `SECURITY DEFINER`, `SET search_path = pg_catalog, pg_temp`, every `drugref.` name
  schema-qualified, `REVOKE … FROM PUBLIC`, `GRANT EXECUTE … TO cairn_window`. **No function name contains "drugref"**,
  so no Rust source names drugref:
  - `cairn_drug_available() RETURNS boolean`: true only when each table below exists (`to_regclass`) **and** the owner
    holds `SELECT` on it (`has_table_privilege`). It never raises. The tables are `drugref.substance_moiety`,
    `identity_claim`, `substance_class`, `class_parent`, `class_membership` and `substance_composition`.
  - `cairn_drug_search(p_text text, p_limit int) RETURNS TABLE(system text, code text, display text)`. Zero rows when
    unavailable, or when `btrim(p_text)` is shorter than 3 characters. `%`, `_` and `\` in the text match literally.
    - Moieties match when `display_name` starts with the text, or a word in it does.
    - Groups (D4) match when `class_name` contains the text.
    - Order: moieties first, then groups. Within each, shorter names first, then by `COLLATE "C"`.
    - `p_limit` is capped at 20.
  - `cairn_drug_facts(p_allergens uuid[], p_meds uuid[]) RETURNS TABLE(fact text, allergen uuid, med uuid)`. Zero rows
    when unavailable. Otherwise:
    - `'same'`: the allergen is a moiety whose family meets the med's family.
    - `'member'`: the allergen is a class, and the med's family meets the members of the class's subtree.
    - `'complete'`: never produced today (D6).
  - Family (M3) = the moiety, plus `substance_composition.component_moiety` where `substance_unii` is one of the moiety's
    UNII claims with `superseded_by IS NULL` and `is_active_component`.
- **Clinician words** (exact):
  - Same-drug mark: `"Allergy recorded: {allergen term} → {reaction}"`.
  - Group mark: `"{line name} is a {allergen term}; allergy recorded: {reaction}"`.
  - Adverse-effect note: `"Previous adverse effect: {allergen term} → {reaction}"`.
  - Uncoded line note, when any live drug allergy exists: `"Not coded: checked against allergies by name only"`.
  - Strip line (D8): `"{allergen term}: medications not fully checked against it"`.
  - Med pane, when the allergy read failed: `"Allergies could not be read: medications not checked against them"`.
  - Pick hint, when no drug list is available: `"No drug list on this terminal: the name is recorded as typed"`.
  - A group option in the pick reads `"{display} (group)"`.

  `{reaction}` is `cairn_event::allergy`'s reaction phrase (made `pub` in Task 1). `{line name}` is
  `MedicationRow::display_name()`.
- **Timing kinds:** none new. `allergy-record` already times the form.
- `SCHEMA_GENERATION` 76. Files stay under 500 lines: new code goes in new files.

## Review Focus

1. **Typed wildcards and short text in the pick** (`%`, `pen_`, `a\`, two letters). These match literally, and fewer
   than 3 characters gives no options rather than the whole table. Pinned in Task 2:
   `search_text_is_literal_and_short_text_finds_nothing`.
2. **Picking a drug and then editing the name.** The coding is dropped, so the term and the code never disagree. Pinned in
   Task 6: `editing_the_name_after_a_pick_drops_the_coding`.
3. **drugref installed but unreadable by the schema owner.** `cairn_drug_available()` is false (never a fault) and the
   medication list still draws. Pinned in Task 2: `an_unreadable_drugref_is_unavailable_not_a_fault`.
4. **The allergy read fails while the medication list is read.** The list still draws, with the worded line. Pinned in
   Task 5: `med_list_draws_when_the_allergy_read_fails` (`SET ROLE` to a role without `SELECT` on `clinical_allergy`,
   per the fault-injection seam).
5. **A salt of the allergen with drugref absent.** There is no mark, and the allergen's "not fully checked" line is
   present, so silence never reads as safe. Pinned in Task 4: `a_salt_without_drugref_is_not_fully_checked`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing "Penicillin — anaphylaxis" in the drug chart's allergy box, and the prescriber or
  pharmacist reading that box against every line of the drug chart before giving a dose.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Record a coded entry: 1 (write the line) → 1 (one signed event; the coding rides on it) → 1 (type, choose from the
    list or keep typing, Sign). Picking is optional, never forced.
  - Check the medication list: 1 (read the box against the chart) → 0 human acts (computed on every read) → 0.
  - `M ≤ N` everywhere: no architecture defect to file.
- **Time + cognitive load:**
  - A coded entry within ≤ 15 s and a free-text one within ≤ 10 s (the design's budgets). The pick list appears within
    ≤ 300 ms of the third letter.
  - A marked line must be legible with no click or hover (#871). Every "not fully checked" line is on the strip,
    visible with no click.
  - The medication pane's open time must not grow by more than 100 ms with drugref present.
  - Owed by a human in **RUNBOOK §29** (Task 7).

---

### Task 1: The coded entry (`cairn-event`, pure)

**Files:**
- Modify: `crates/cairn-event/src/allergy.rs`:
  - `AllergyEntry` gains `pub coding: Option<SubstanceCoding<'a>>`, reusing `cairn_event::medication::SubstanceCoding`.
  - `allergy_payload` writes it with the same object shape db/041 checks.
  - `render_allergy_twin` appends the coded suffix.
  - `reaction_phrase` becomes `pub`.
- Modify: every `AllergyEntry {` constructor (`rg -n "AllergyEntry \{" crates cairn-gui`), each passing `coding: None`.

**Interfaces — Produces:** `AllergyEntry::coding`; `pub fn reaction_phrase(category: &str, text: Option<&str>) -> String`.

- [ ] **Step 1: Failing tests** in `allergy.rs`'s test module:
  - `a_coded_entrys_payload_carries_the_coding_object`: `p["substance"]["coding"] == json!({"system": "drugref-class",
    "code": <uuid>, "display": "Penicillin-class Antibacterial [EPC]"})`.
  - `a_coded_entrys_twin_says_what_it_is_coded_as`: the twin ends with
    `" (coded as Penicillin-class Antibacterial [EPC] [drugref-class])"`.
  - `an_uncoded_twin_is_unchanged`: the slice-1 twin, byte for byte.
- [ ] **Step 2:** `cargo test -p cairn-event allergy` → FAIL.
- [ ] **Step 3:** Implement. Then fix the callers.
- [ ] **Step 4:** `cargo test -p cairn-event` → PASS. Then `cargo check --workspace` and, in `cairn-gui/`, `cargo check`.
- [ ] **Step 5:** Commit `feat(allergy): a drug entry may carry its coding; the twin says so`.

### Task 2: The floor opens the slot; the drug read door (`db/076`; db/074, db/041, db/042 in place)

**Files:**
- Create:
  - `db/076_drug_read_door.sql` (Global Constraints' three doors);
  - `db/tests/076_drug_read_door_test.sql`;
  - `crates/cairn-node/tests/fixtures/drug_stub.sql`;
  - `crates/cairn-node/tests/drug_read_door.rs`.
- Modify in place:
  - `db/074_clinical_allergy.sql`: the "reserved slot" refusal becomes D1's rule plus the system list. The messages are
    `allergy assertion: substance.coding is for a drug entry only` and `allergy assertion: substance.coding.system must
    be drugref-moiety or drugref-class, got %`. Check that the applier copies `substance.coding` into
    `clinical_allergy.coding` (the column exists).
  - `db/041_medication_coding.sql`: the seed gains `('drugref-class', 'uuid', 'a drug GROUP (drugref class_uuid) —
    allergen coding only; the medication doors refuse it')`. `cairn_check_medication_coding` refuses `drugref-class` at
    the authoring door with `medication assertion: substance.coding names a drug group (drugref-class), never one drug`.
  - `db/042_*` (the overlay check): the same refusal, with its own prefix.
  - `db/tests/074`'s expected messages.
  - `crates/cairn-node/tests/no_drugref_dependency.rs`:
    - add `drugref-class` to the registered-token exemption (d);
    - add a new exemption for exactly the path `db/076_drug_read_door.sql`, with a doc paragraph saying why (M1) and that
      no other file may join it.
- Pinned counts move:
  - `crates/cairn-event/src/schema_generation.rs` (76);
  - `crates/cairn-node/src/db.rs` (the `include_str!` list);
  - every per-file list `rg -n "075_allergy_overlays" crates db` finds;
  - the window's grant inventory (`rg -n "TO cairn_window" db` and the test pinning it).

**Interfaces — Produces:** the three doors exactly as in Global Constraints. `drug_stub.sql` creates a minimal `drugref`
schema: the six tables, with only the columns the door reads, plus rows for:
- moieties amoxicillin, amoxicillin sodium, amoxicillin anhydrous and metformin, each with a UNII claim, plus one
  superseded claim;
- the class `Penicillin-class Antibacterial [EPC]` with a child class;
- `Renal Excretion [PK]`;
- the composition rows of M3.

All its uuids are computed (`uuid_generate_v5`-free: use `md5(...)::uuid`), never literals (house rule 6).

- [ ] **Step 1: Failing SQL test** `076_drug_read_door_test.sql`. Every section that needs drugref creates the stub
  **inside one transaction and ends with `ROLLBACK`**, so no other test on the shared `cairn_test` ever sees a `drugref`
  schema:
  - (0) Absent schema: `cairn_drug_available()` is false, and search and facts return zero rows.
  - (1) Search (`search_text_is_literal_and_short_text_finds_nothing`):
    - "amox" finds the three amoxicillin moieties, shortest first;
    - "penicillin" finds the EPC group and never the PK class;
    - "am" finds nothing;
    - "am%x" finds nothing;
    - the limit is capped at 20.
  - (2) Facts:
    - an amoxicillin-sodium allergen against an amoxicillin med is `same` (shared family member);
    - the EPC class against an amoxicillin med is `member` (through the family);
    - the parent class against an amoxicillin med is `member` (through the subtree);
    - metformin gives nothing;
    - a superseded UNII claim does not join a family;
    - `complete` is never produced.
  - (3) `an_unreadable_drugref_is_unavailable_not_a_fault`: `REVOKE SELECT` on one stub table from the owner →
    `cairn_drug_available()` is false, and the doors return zero rows with no error.
  - (4) The floor:
    - a drug entry coded `drugref-class` and one coded `drugref-moiety` are accepted;
    - an `other` entry with a coding is refused;
    - a `drugref-product` system and a non-canonical uuid are refused;
    - at the remote door, an unknown system is admitted;
    - a medication coded `drugref-class` is refused at both of its authoring doors.
- [ ] **Step 2:** Run against `cairn_test` (`PGHOST=127.0.0.1 PGPORT=5532`, #862) → FAIL.
- [ ] **Step 3:** Write db/076 and the in-place edits.
- [ ] **Step 4:** Run the SQL test → PASS. Add the Rust mirror `drug_read_door.rs` (stub loaded with `include_str!`
  inside a transaction that is rolled back, one test per section). Run it with `allergy_floor.rs`, `no_drugref_dependency`,
  `medication_coding.rs`, `medication_coding_overlay.rs` and the moved count guards, using `--nocapture`, and check there
  is no `skipped:`.
- [ ] **Step 5:** Commit `feat(allergy): the coding slot opens; the drug read door (db/076)`.

### Task 3: Read the coding and the drug facts (`cairn_node::allergy`, `cairn-medication-view`)

**Files:**
- Create: `crates/cairn-node/src/allergy/drug_facts.rs` (async, `GenericClient`).
- Modify:
  - `allergy/read.rs`: `AllergyRow` gains `pub coding: Option<Coding>`, read from `clinical_allergy.coding`. A JSON
    object missing a key reads as `None`, and is never a fault.
  - `allergy/test_fixtures.rs`: `coding: None`.
  - `allergy/thread.rs`: add `same_allergen_rows(a: &AllergyRow, b: &AllergyRow) -> bool` (D10), and use it where
    `differing` compares rows.
  - `allergy/mod.rs`: export.
- Modify: `crates/cairn-medication-view/src/row.rs`. `MedicationRow` gains `coding_system: Option<String>` and
  `coding_code: Option<String>`, each `#[serde(skip_serializing_if = "Option::is_none")]` so the never-linked `--json`
  golden is unchanged. `crates/cairn-node/src/medication/read.rs` selects them beside `coding_display`. Fix every
  constructor that `rg -n "coding_display:" crates cairn-gui` finds.

**Interfaces — Produces:**
- `pub struct Coding { pub system: String, pub code: String, pub display: String }` in `read.rs`.
- `pub enum DrugFacts { Unavailable, Available(KnownFacts) }`.
- `pub struct KnownFacts { pub same: HashSet<(Uuid, Uuid)>, pub member: HashSet<(Uuid, Uuid)>, pub complete: HashSet<Uuid> }`.
  Each pair is `(allergen code, med code)`.
- `pub async fn read_drug_facts(c: &impl GenericClient, allergens: &[Uuid], meds: &[Uuid]) -> anyhow::Result<DrugFacts>`
  calls `cairn_drug_available()` and then `cairn_drug_facts` on the same client (the caller's snapshot).
- `pub struct DrugOption { pub system: String, pub code: String, pub display: String }`.
- `pub async fn search_drugs(c: &impl GenericClient, text: &str) -> anyhow::Result<Option<Vec<DrugOption>>>`. `None`
  means no drug list is available. `Some(vec![])` means none matched. Limit 12.

- [ ] **Step 1: Failing tests:**
  - `same_allergen_rows_matches_on_code_or_term` (unit, in `thread_tests.rs`);
  - `a_coded_entry_reads_back_its_coding` (DB, `crates/cairn-node/tests/allergy_read.rs` or its neighbour, through
    `sign_allergy`);
  - `drug_facts_reads_the_stub` and `drug_facts_without_drugref_is_unavailable` (DB, in `drug_read_door.rs`).
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement. Wrap the two reads with the crate's existing local-DB-fault wording, and bump
  `db_errors_stay_legible.rs`'s per-file count if it applies.
- [ ] **Step 4:** Run those tests, `cargo test -p cairn-medication-view`, and the `--json` golden test (`rg -n golden
  crates/cairn-node/tests`) → PASS.
- [ ] **Step 5:** Commit `feat(allergy): read an entry's coding and the drug facts`.

### Task 4: The check (`cairn_node::allergy::check`, pure)

**Files:**
- Create: `crates/cairn-node/src/allergy/check.rs` and `check_tests.rs`.
- Modify: `view.rs`. Make the reaction rank (line ~163) `pub(crate)` for D7. `AllergyView` gains
  `pub not_fully_checked: Vec<String>`, filled by `build_view`'s caller through `with_check_lines` (below), so
  `build_view`'s signature is unchanged.

**Interfaces — Consumes:** `Resolved`/`Head` (thread.rs), `AllergyRow::coding`, `DrugFacts` (Task 3), `reaction_phrase`
(Task 1). **Produces:**
- `pub struct MedLine<'a> { pub group_id: Uuid, pub name: &'a str, pub term: &'a str, pub coding_display: Option<&'a str>, pub moiety: Option<Uuid>, pub current: bool }`.
  `moiety` is the code when the system is `drugref-moiety`.
- `pub struct LineCheck { pub marks: Vec<String>, pub notes: Vec<String> }`.
- `pub fn codes_to_ask(r: &Resolved, meds: &[MedLine]) -> (Vec<Uuid>, Vec<Uuid>)` returns the live drug heads'
  `drugref-*` codes and the current lines' moieties, sorted and deduplicated.
- `pub fn check_medications(r: &Resolved, meds: &[MedLine], facts: &DrugFacts) -> HashMap<Uuid, LineCheck>` follows D7
  and D9. A line with nothing to say has no key.
- `pub fn not_fully_checked(r: &Resolved, facts: &DrugFacts) -> Vec<String>` follows D8, in the strip's head order.
- `pub fn with_check_lines(view: AllergyView, lines: Vec<String>) -> AllergyView`.

- [ ] **Step 1: Failing unit tests** (`check_tests.rs`, built on `test_fixtures` plus a `coded(row, system, code)` helper):
  - `same_code_marks_the_line`;
  - `same_name_marks_an_uncoded_line`;
  - `a_shared_family_member_marks_the_line` (a `same` fact);
  - `a_group_member_marks_the_line_naming_the_group` (exact group-mark words);
  - `an_adverse_effect_is_a_note_never_a_mark`;
  - `a_delabelled_or_withdrawn_entry_checks_nothing`;
  - `a_ceased_line_is_never_checked`;
  - `differing_accounts_mark_once_with_the_more_severe` (rash + anaphylaxis → one "anaphylaxis" mark);
  - `not_known_ranks_with_anaphylaxis`;
  - `an_uncoded_line_says_name_only_when_a_drug_allergy_exists`;
  - `a_doubted_charts_entry_still_marks`;
  - `latex_never_touches_the_medication_list`;
  - `every_group_is_not_fully_checked_today`;
  - `an_uncoded_allergy_is_not_fully_checked`;
  - `a_salt_without_drugref_is_not_fully_checked` (Review Focus 5);
  - `a_moiety_with_drugref_is_fully_checked`;
  - `one_line_per_allergen`;
  - `no_line_for_an_adverse_effect`.
- [ ] **Step 2:** `cargo test -p cairn-node --lib allergy::check` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run → PASS. Then `cargo test -p cairn-node --lib allergy`.
- [ ] **Step 5:** Commit `feat(allergy): check the medication list against the live drug allergies`.

### Task 5: The window's commands (`cairn-gui-tauri`, Rust)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/allergy_check.rs` and `allergy_check_tests.rs`.
- Modify:
  - `allergies.rs`: `allergy_record` takes `coding: Option<CodingArg>`, with `CodingArg { system, code, display }`.
    `code` is parsed as `Uuid` and re-spelled canonically; an unparsable one is refused `"That drug's code could not
    be read; type the name instead"`. `allergy_list_impl` reads `DrugFacts` in the same snapshot as the rows and fills
    `not_fully_checked`. On both panes a failed facts read is `DrugFacts::Unavailable`, never a failed pane: names and
    exact codes still mark, and every coded allergen reads not fully checked.
  - `allergy_overlays.rs`: Correct takes `coding` the same way.
  - `commands.rs::med_list_impl`: reads the allergies (`allergies::read_list`) and the facts, and hands the
    `HashMap<Uuid, LineCheck>`, or the failure line, to `chart_pane`.
  - `chart_set/mod.rs::chart_pane` gains that parameter. `MedListRowView` (`cairn-gui-tab-medications/src/row_view.rs`)
    gains `pub allergy_marks: Vec<String>` and `pub allergy_notes: Vec<String>`. The pane gains
    `pub allergy_check_failed: Option<String>`.
  - Add a `drug_search` command, registered in `main.rs` and the source guard's command list.

**Interfaces — Produces** (JSON as `serde` writes it):
- `drug_search(text) -> DrugSearch { available: bool, options: Vec<{system, code, display}> }`. Fixture mode gives
  `available: false`.
- `allergy_record(..., coding)`.
- `AllergyPane.not_fully_checked: string[]`.
- `MedListRowView.allergy_marks` and `MedListRowView.allergy_notes`.
- `ChartPane.allergy_check_failed`.

- [ ] **Step 1: Failing tests:**
  - `allergy_check_tests.rs`: `med_lines_come_from_the_rows` (the `MedicationRow` → `MedLine` mapping, pure);
  - DB tests, beside `allergies_tests.rs`:
    - `a_picked_group_is_signed_with_its_coding`;
    - `a_bad_code_is_refused_before_signing`;
    - `med_list_marks_a_line_against_a_coded_allergy` (stub loaded in the test's own transaction, as Task 2);
    - `med_list_draws_when_the_allergy_read_fails` (Review Focus 4);
    - `allergy_list_names_the_not_fully_checked_allergens`.
- [ ] **Step 2:** Run → FAIL. Remember `cairn-gui-tauri`'s DB tests do not run in CI (#905). Run them locally.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the `cairn-gui` gate (memory: the full ~2-min command set, `cargo doc` included) and the root
  source guards DB-free (they sweep `cairn-gui` sources) → PASS.
- [ ] **Step 5:** Commit `feat(window): drug search, coded entries, and the check on both panes`.

### Task 6: The pick, the marks and the lines in the window (JS)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/drug_pick.js`. The type-ahead under `#allergy-term`:
  - it shows only while the kind is *drug*, and asks `drug_search` 150 ms after the last keystroke, from the third
    character;
  - arrow keys, Enter and click choose an option; Escape closes the list;
  - a choice sets the term to the option's display and holds the coding;
  - **any later edit of the term drops the coding**;
  - with `available: false` it shows the hint line instead of a list.
  - It is loaded before `allergies.js` in `index.html`.
- Modify:
  - `allergies.js`: send `coding` with Record and Correct, and clear it with the form. Draw `not_fully_checked` as strip
    lines, visible with no click.
  - `main.js`: draw each row's `allergy_marks` on the line as visible text, prominent (never a `title`), and
    `allergy_notes` muted. Draw `allergy_check_failed` above the list. Split `refresh` so its `med_list` half is
    `readMedList(lead, behind)`. `allergies.js` calls `readMedList()` after any act answered Ok, so a new allergy marks
    the list at once.
- Test: webview tests in `allergies_webview_tests.rs` (or a new `drug_pick_webview_tests.rs` if that file would pass
  500 lines):
  - `the_pick_offers_drugs_and_groups`;
  - `editing_the_name_after_a_pick_drops_the_coding` (Review Focus 2);
  - `no_drug_list_shows_the_hint`;
  - `a_marked_line_shows_its_mark_without_hover` (computed display and no hidden ancestor, per the walk-recipe memory);
  - `the_strip_shows_not_fully_checked_lines`;
  - `recording_an_allergy_rereads_the_medication_list`.

- [ ] **Step 1:** Write the failing webview tests. **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run the
  webview tests and the `cairn-gui` gate → PASS. Walk the window headlessly once in Chromium (memory: the mock-walk
  recipe), assert visibility, and note in RUNBOOK §29 that WebKit is owed.
- [ ] **Step 5:** Commit `feat(window): pick a drug or group; marks on the medication list; not-fully-checked lines`.

### Task 7: ADR-0098, spec, RUNBOOK §29, durable rules, issues, handover

**Files:**
- Create: `docs/spec/decisions/0098-allergy-coding-and-the-medication-check.md`. It records M1–M3 and D1–D10, the no-safety-signal
  ruling restated, and **the drugref contract**: the three questions the door asks, their answer shapes, the
  `complete` fact drugref must add (a per-group completeness statement the door can read in place of today's "never"),
  and the rule that only db/076 changes when drugref ships it.
- Modify:
  - `docs/spec/decisions/README.md`;
  - `docs/spec/index.md` (v0.100);
  - `docs/spec/data-model.md` §3.3 (allergies; memory: medication and allergy prose live in §3.3) and §3.16's drug
    axis (the `drugref-class` token);
  - `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §29: install drugref into the measurement database with
    `pg_dump -n drugref drugref_profiles | psql <cairn db>`, the grants the owner needs, then time the budgets, VoiceOver
    on a mark, and WebKit;
  - `docs/developers/durable-rules.md` (an **ALLERGY SLICE 3** block: the one-file exemption, no "drugref" in a door
    name, the family walk, "not fully checked" never simplified to hit-only marks (risk 2), adverse effects never mark);
  - `docs/HANDOVER.md` and `docs/ROADMAP.md`.
- File issues for anything deferred (each named in the ADR). At least:
  - drugref's completeness statement (in the drugref repo, if the maintainer agrees; otherwise a Cairn issue naming the
    contract);
  - coding the tolerated drug (D2);
  - the check at the moment of charting (it rides the charting slice).
  Run `scripts/check_closing_keywords.py` before committing.
- [ ] **Step 1:** Write the docs. Fact-check the ADR sentence by sentence against db/076 and `check.rs` (memory: ADR-0077
  passed two reviews with a false sentence).
- [ ] **Step 2:** Build the docs: `uv run --with-requirements docs/requirements.txt -- mkdocs build`. Run
  `paper_parity_plan_section.rs`.
- [ ] **Step 3:** Commit `docs(allergy): ADR-0098, spec v0.100, RUNBOOK §29, durable rules`.
