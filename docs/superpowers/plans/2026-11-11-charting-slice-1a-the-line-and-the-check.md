# Charting slice 1a — the one line and the check at charting — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician charts a medication in the window from one typed line, as *Start medication* or *Already taking*. The allergy check runs on the line before Sign, and a hit on *Start* requires a typed reason.

**Architecture:** Five optional fields are added to `clinical.medication.asserted`: `charted_as`, `route`, `substance.strength`, `substance.components` and `allergy_override`. The event type and schema version stay the same, since every field is optional (principle 11). One new SQL file, `db/077`, holds:
- their floor, called from db/031's existing check;
- their own projection table, `medication_charting`, registered as a second apply fn on the same event, so no view widens across files;
- a drugs-only, site-ranked search door.

A pure parser reads the directions. A new node orchestrator `chart_medication` signs the event, and `assert_medication` delegates to it with no charting, so the 63 existing callers are untouched. The window gets a new pane file and four commands.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri), PostgreSQL ≥ 18 (plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-10-charting-a-medication-design.md`](../specs/2026-11-10-charting-a-medication-design.md). This plan builds §3.1, §3.2, §3.4, the *Start* rows of §3.5, and the line's half of §3.3. It does not build these, which are slice **1b**: the same-drug de-label offer (#1053), the group comment (a new allergy event type), and their one-transaction rule. Slice 2 (stat given) and slice 3 (aliases, PBS forms/strengths) come after that.

## Global Constraints

- AGPL-3.0. No new dependency.
- Wire is forever. Every key and every twin string below is frozen once merged. A payload without the new keys must serialise **byte-identically** to today's (existing golden tests stay green unedited).
- `charted_as` ∈ `started-here` | `pre-existing` | `unknown`, closed at the authoring door and admitted as any string at the remote door (ADR-0056: never wedge sync on a verifiable event).
- `route` is an open lower-case token (`po`, `iv`, `im`, `sc`, `sl`, `pr`, `top`, `inh`, `neb`, …). The floor requires only that it is a non-empty string.
- `substance.components` is an array of ≥ 2 `{term, coding}` objects, where `coding` is a `{system, code, display}` triple or `null`. It is never present together with `substance.coding`. Both rules are refused locally and admitted remotely.
- `allergy_override` = `{reason, against}`: `reason` is a non-empty trimmed string; `against` is an array of ≥ 1 allergy-record uuid strings.
- **The reason is required by the window, never by the floor** (design §3.5; ADR-0021 soft policy). The floor checks only the override's shape *when it is present*.
- The typed text is what is signed. `sig` = the directions exactly as typed. Parsed `dose`/`route` fill structured fields **only** when the parse is certain (design §3.1). Nothing is prefilled.
- Parse vocabulary is a fixed table (Task 2). It never guesses outside it.
- The window never shows a confirmation dialog. Warnings sit on the line.
- The drug list is optional. No drugref, or a failed search or check, never blocks charting. The line then says "not checked".
- Copy (exact; pinned by the webview tests): buttons `Start medication`, `Already taking`; reason label `Reason for charting despite the listed allergy`; refusal `Charting despite a listed allergy needs a reason`; tags as ADR-0098 (`Allergy`, `not checked`).
- Files stay under 500 lines; new window code lives in its own files (`funnel.js` is at 499, #1023).
- `CAIRN_ALLOW_DB_SKIP=1` only for a deliberately DB-free run. A DB-gated suite that prints `skipped:` is not a pass.

## Review Focus

1. **A decimal dose** (`1.5mg`, `0.5 mg`, `.5mg`, `12.5 mg`): the amount is kept as typed text (`"1.5"`, `"0.5"`, `".5"`), never re-formatted or truncated. A form the table cannot read, such as `1,5mg`, leaves `dose` unknown. Pinned in Task 2.
2. **A component edited after its pick** (pick amoxicillin, then retype it to "amoxapine"): that component's coding drops, as `drug_pick.js` does for allergies, and the line reads "not checked", not checked-clean. Pinned in Task 6's webview test.
3. **A combination with one component unpicked** (`amox + clav`, only amox picked): the check runs the coded part, and the line is "not checked" if no mark. It must never read clean. Pinned in Task 4.
4. **A whitespace-only reason** on a *Start* line with an allergy mark: Sign is refused before the key, with the exact refusal copy. A remote event with no `allergy_override` still reads. Pinned in Task 5 (window) and Task 1 (remote door).
5. **No drug list** (drugref absent, or the door raises): the search answers `available: false` and the line charts uncoded with "not checked". The command never returns an error for the drug list's sake. Pinned in Task 5.

---

### Task 1: The wire fields, their twin, and their floor

**Files:**
- Create: `crates/cairn-event/src/medication/charting.rs` (pure; with `mod tests`)
- Modify: `crates/cairn-event/src/medication/assert.rs` (`MedicationAssertion` gains `charting: Option<Charting<'a>>`; body and twin call into `charting.rs`), `crates/cairn-event/src/medication/mod.rs`
- Create: `db/077_medication_charting.sql` (§1 the floor fn only; later tasks append §2–§3)
- Modify: `db/031_medication.sql` (in place: `PERFORM cairn_check_medication_charting(p);` right after the `cairn_check_medication_coding(p)` call)
- Test: `crates/cairn-node/tests/medication_charting_floor.rs` (DB-gated)

**Interfaces — Produces:**
```rust
pub enum ChartedAs { StartedHere, PreExisting, Unknown }   // as_str(): "started-here" | "pre-existing" | "unknown"; parse()
pub struct Component<'a> { pub term: &'a str, pub coding: Option<SubstanceCoding<'a>> }
pub struct AllergyOverride<'a> { pub reason: &'a str, pub against: &'a [&'a str] }
#[derive(Default)] pub struct Charting<'a> {
    pub charted_as: Option<ChartedAs>, pub route: Option<&'a str>, pub strength: Option<&'a str>,
    pub components: &'a [Component<'a>], pub allergy_override: Option<AllergyOverride<'a>>,
}
pub fn insert_charting(payload: &mut serde_json::Map<String, Value>, substance: &mut serde_json::Map<String, Value>, c: &Charting);
pub fn charting_twin_parts(c: &Charting) -> TwinParts;   // the strings the twin splices in, see Step 3
```

- [ ] **Step 1: Write the failing unit tests** in `charting.rs`:
  - `no_charting_leaves_the_payload_byte_identical`: `medication_assertion_body` with `charting: None` and with `Some(Charting::default())` both equal today's `full_assertion()` JSON.
  - `charting_keys_are_exactly_the_frozen_ones`: a full `Charting` adds top-level `charted_as`, `route`, `allergy_override` and `substance.strength`, `substance.components`, and no other key. `allergy_override == {"reason": "benefit outweighs risk", "against": ["E1"]}`; `components[1] == {"term": "clav", "coding": null}`.
  - `twin_words_are_exact`: for term `Augmentin`, components amoxicillin + clavulanic acid (both coded), formulation `tablet`, strength `875/125 mg`, route `po`, dose `1` `tablet`… use dose `None`, sig `1 bd`, info_source `clinician-observed`, `StartedHere`, override reason `no alternative`, the twin is exactly
    `Augmentin tablet 875/125 mg po — 1 bd (clinician-observed) [amoxicillin + clavulanic acid] · started here · charted despite a listed allergy: no alternative`.
    `PreExisting` → ` · already taking`; `Unknown` → nothing; an uncoded component shows as its term inside the brackets (`[amoxicillin + clav]`).
- [ ] **Step 2: Run** `cargo test -p cairn-event charting` → FAIL (module missing).
- [ ] **Step 3: Implement.** Twin order (frozen): `{term}{ amt unit}{ formulation}{ strength}{ route} — {sig} ({info_source}), started {v}{ [coding display | components joined " + "]}{ · started here | · already taking}{ · charted despite a listed allergy: {reason}}`. Each `{…}` is omitted when absent. The existing `[display]` rule (omitted when it equals the term, case-folded) still applies to a single coding. A components bracket is always shown.
- [ ] **Step 4: Write the failing DB test** `medication_charting_floor.rs`:
  - `local_door_refuses_each_bad_shape`: the local door refuses each of `charted_as: "stat"`, `route: ""`, a one-element `components`, `components` together with `coding`, `allergy_override.reason: "  "`, and `against: []`, each with a message naming the field.
  - `remote_door_admits_a_newer_word`: under `cairn.remote_apply = 'on'`, the remote door admits `charted_as: "transferred-in"` and a one-element `components`.
  - `an_event_without_charting_still_passes`: an event with no charting fields passes both doors.

  Copy the door-calling helpers from `medication_remote_apply.rs`.
- [ ] **Step 5: Implement** `cairn_check_medication_charting(p jsonb) RETURNS void` in `db/077` §1. The two-tier split is db/041's: structural (types) at both doors, and vocabulary/cardinality local only. Revoke from PUBLIC, as the `cairn_check_*` family is (#382).
- [ ] **Step 6: Run** both tests, then commit: `git commit -m "feat(medication): charting fields on the wire, their twin and floor (db/077)"`.

### Task 2: The directions parser (pure)

**Files:**
- Create: `crates/cairn-node/src/medication/line.rs`, `crates/cairn-node/src/medication/line_tests.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs`

**Interfaces — Produces:**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
pub struct Directions { pub dose_amount: Option<String>, pub dose_unit: Option<String>, pub route: Option<String>,
                        pub frequency: Option<String>, pub form: Option<String>, pub strength: Option<String>,
                        pub prn: bool, pub stat: bool, pub unread: Vec<String> }
pub fn read_directions(text: &str) -> Directions;
pub fn term_to_sign(typed: &str, picked_display: Option<&str>) -> String;
pub fn shown_name(display: &str, term: &str) -> String;
```

- [ ] **Step 1: Write the failing tests** (`line_tests.rs`):
  - `reads_the_common_line`: `"500mg tds"` gives amount `"500"`, unit `"mg"`, frequency `"three times daily"`, and `unread` empty.
  - `reads_route_form_and_strength`: `"1g IV daily"` gives route `"iv"` and frequency `"daily"`. `"tab 875/125mg 1 bd po"` gives form `"tablet"`, strength `"875/125 mg"`, amount `"1"`, unit `None`, route `"po"`, frequency `"twice daily"`.
  - `keeps_decimals_as_typed`: `1.5mg` → `"1.5"`; `0.5 mg` → `"0.5"`; `.5mg` → `".5"`; `12.5 mg` → `"12.5"`. `1,5mg` reads no dose and lands in `unread`.
  - `prn_and_stat`: `"2 puffs inh prn"` gives `prn`, unit `"puffs"` and route `"inh"`. `"1g iv stat"` gives `stat`.
  - `two_doses_is_not_a_guess`: `"500mg 250mg tds"` leaves the amount and unit `None`, and both doses go to `unread`.
  - `unknown_words_are_unread_never_dropped`: `"500mg tds with food"` puts `["with", "food"]` in `unread`.
  - `term_to_sign`: `("amox", Some("amoxicillin"))` → `"amoxicillin"` (a prefix is unfinished typing). `("Augmentin", Some("amoxicillin + clavulanic acid"))` → `"Augmentin"`. `("amoxycillin", Some("amoxicillin"))` → `"amoxycillin"`. `("x", None)` → `"x"`.
  - `shown_name`: `("amoxicillin + clavulanic acid", "Augmentin")` → `"amoxicillin + clavulanic acid (Augmentin)"`. `("amoxicillin", "Amoxicillin")` → `"amoxicillin"`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib medication::line` → FAIL.
- [ ] **Step 3: Implement** as table-driven tokenising: lower-case, split on whitespace, and split a number glued to a unit (`500mg`). The tables are `const` arrays in `line.rs`, each with a one-line source comment citing the ACSQHC *Recommendations for terminology, abbreviations and symbols*:
  - frequency: `mane`, `nocte`, `daily`, `bd`, `tds`, `qid`, `q4h`, `q6h`, `q8h`, `q12h`, `weekly` → their words, e.g. "three times daily";
  - routes: `po`, `oral` → `po`; `iv`, `im`, `sc`/`subcut`, `sl`, `pr`, `top`, `inh`, `neb`, `pv`;
  - units: `mg`, `g`, `mcg`, `microgram(s)`, `ml`/`mL`, `units`, `puff(s)`, `drop(s)`, `tablet(s)`/`tab(s)`, `capsule(s)`/`cap(s)`;
  - forms: `tab`, `tablet`, `cap`, `capsule`, `liquid`, `mixture`, `syrup`, `inj`, `injection`, `patch`, `cream`, `inhaler`;
  - flags: `prn`, `stat`.

  A strength is a number with an optional `/number` and a unit, when it follows a form word. A token in no table goes to `unread`. A unit word with no number before it (`tablet`) is a form.
- [ ] **Step 4: Run** → PASS. Commit: `feat(medication): read the typed directions into fields, never guessing`.

### Task 3: The projection and the drug search door

**Files:**
- Modify: `db/077_medication_charting.sql`. Append §2: table `medication_charting` (`medication_id uuid PK, patient_id uuid NOT NULL, charted_as text, route text, strength text, components jsonb, override_reason text, override_against uuid[], hlc_wall, hlc_counter, origin, content_address`, the overlay columns as `medication_coding`'s), its `content_address` index, `GRANT SELECT … TO cairn_agent`, `medication_charting_apply(e event_log)`, and its `cairn_projection_apply` row (`'clinical.medication.asserted', 'medication_charting_apply', ARRAY['medication_charting'], 20, TRUE`). Append §3: `cairn_medication_drug_search(p_text text, p_limit int) RETURNS TABLE(code text, display text, uses bigint)`, `SECURITY DEFINER`, `SET search_path = pg_catalog, pg_temp`, gated on `cairn_drug_available()`. It matches drugs only (moieties; db/076's word-start rule and escaping) and ranks by `uses` = this node's count of that moiety in `medication_coding` plus in `medication_charting.components`, then by shorter name. `REVOKE … FROM PUBLIC; GRANT EXECUTE … TO cairn_window`.
- Modify in place: `db/037` (the shred deletes `medication_charting` rows by `content_address` beside `medication_statement`'s, ~line 330); `db/044` (add `'medication-chart'` to **both** `gesture_kind` CHECK lists, lines 30 and 48).
- Test: `crates/cairn-node/tests/medication_charting_projection.rs`, `crates/cairn-node/tests/medication_drug_search_door.rs` (DB-gated; the stub drugref schema the allergy door tests use).
- Modify the pinned counts: both `cairn_projection_apply` row counts (`durable-rules.md` names the two files) go +1. The twin registry is unchanged (no new event type).

**Interfaces — Produces:** table `medication_charting`; SQL fn `cairn_medication_drug_search(text, int)`.

- [ ] **Step 1: Failing tests.**
  - `a_charted_line_projects_its_fields`: the row holds `route = 'po'`, components as JSON, `override_against = {E1}`.
  - `an_event_without_charting_projects_no_row`.
  - `a_shred_removes_the_charting_row`: an override reason is clinical text.
  - `a_reprojection_rebuilds_it`: the generic rebuild (ADR-0057) restores the row.
  - `the_search_lists_drugs_only_ranked_by_use`: the stub has moiety amoxicillin, moiety amoxapine and a group "penicillins", with two charted amoxicillin lines. `'amox'` returns amoxicillin first (`uses = 2`) and no group.
  - `the_search_is_empty_without_drugref`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** §2–§3 and the in-place edits. **Step 4: Run** → PASS, plus `cargo test -p cairn-node --test twin_registry` and the two projection-count guards.
- [ ] **Step 5: Commit** `feat(medication): the charting projection and a drugs-only search door`.

### Task 4: The node: sign a charted line, read its charting, check it

**Files:**
- Create: `crates/cairn-node/src/medication/charting.rs` (`chart_medication`, `read_charting`)
- Modify: `crates/cairn-node/src/medication/assert.rs` (`assert_medication_returning_event` delegates to `chart_medication` with `&Charting::default()`; `build_assert_body` gains `charting: Option<&Charting>`, and its 4 `MedicationAssertion {` sites get `charting`)
- Modify: `crates/cairn-node/src/allergy/check.rs` (`LineCheck` gains `against: Vec<Uuid>`, one hitting head's `event_id` per mark, in mark order. Add `pub fn merge(a: LineCheck, b: LineCheck) -> LineCheck`. `check_medications` merges lines that share a `group_id` instead of overwriting.)
- Modify: `crates/cairn-node/src/allergy/drug_facts.rs` (`search_medication_drugs`)
- Test: `crates/cairn-node/tests/medication_chart.rs` (DB-gated), `crates/cairn-node/src/allergy/check_tests.rs` (pure)

**Interfaces — Produces:**
```rust
pub async fn chart_medication(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str, node_origin: &str,
    patient: Uuid, input: &AssertMedicationInput<'_>, charting: &Charting<'_>, author: Option<&AuthorParams<'_>>)
    -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>>;
pub struct ChartingRow { pub charted_as: Option<String>, pub route: Option<String>, pub strength: Option<String>,
    pub components: Vec<(String, Option<(String, String, String)>)>, pub override_reason: Option<String> }
pub async fn read_charting(tx: &impl GenericClient, medication_ids: &[Uuid]) -> anyhow::Result<HashMap<Uuid, ChartingRow>>;
pub async fn search_medication_drugs(client: &impl GenericClient, text: &str, limit: i32) -> anyhow::Result<Option<Vec<DrugOption>>>; // None = no drug list
pub fn merge(a: LineCheck, b: LineCheck) -> LineCheck;
```
`chart_medication`: no safety claim for a combination (`safety: None`; the withholding direction, as uncoded, ADR-0063). One claim for a single coding, as today.

- [ ] **Step 1: Failing pure tests** in `check_tests.rs`:
  - `merge_unions_marks_and_against_in_order_and_dedups`.
  - `merge_keeps_unverified_false_when_either_has_a_mark` (the ADR-0098 invariant).
  - `two_lines_one_group_both_count`: an Augmentin group as two MedLines (amoxicillin coded, clav uncoded) against a penicillin-group allergy whose drug list answers `same-group` gets one mark, `against` = that head.
  - `a_combination_with_an_unpicked_part_is_not_checked_clean`: no mark and an uncoded part → `unverified`.
- [ ] **Step 2: Failing DB tests** in `medication_chart.rs`:
  - `chart_medication_signs_and_projects` (human-authored; `medication_statement` + `medication_charting` rows).
  - `assert_medication_is_unchanged`: the event bytes of an existing-style assert equal a pinned content address from before the change. Take it from an existing golden if one exists, else assert payload equality.
  - `read_charting_reads_by_thread`.
  - `search_medication_drugs_none_without_drugref`.
- [ ] **Step 3: Run** → FAIL. **Step 4: Implement.** **Step 5: Run** the two files plus the whole `cargo test -p cairn-node --lib allergy` → PASS.
- [ ] **Step 6: Full-workspace check for call-site drift:** `cargo build --workspace --tests` and the GUI tree's `cargo build --tests` (`cairn-sync/tests/clinical_pull.rs` builds medication bodies; memory: cross-crate signature changes).
- [ ] **Step 7: Commit** `feat(medication): chart_medication; the check counts every part of a line`.

### Task 5: The window commands

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/med_chart.rs`, `cairn-gui/cairn-gui-tauri/src/med_chart_tests.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (register the commands), `cairn-gui/cairn-gui-tauri/src/allergy_check.rs` (`med_lines` takes `&HashMap<Uuid, ChartingRow>` keyed by member thread and yields one `MedLine` per component of each member; `med_list_check` reads `read_charting` for the list's member threads in the list's snapshot)

**Interfaces — Produces** (Tauri commands; `charts`/`credential` as `allergy_record`'s):
```rust
pub async fn med_line_read(text: String) -> Directions;                              // pure, no state
pub async fn med_drug_search(state, text: String) -> DrugSearch;                     // reuses allergy_check::DrugSearch; never fails
pub async fn med_line_check(state, patient_id: String, charts: Vec<String>, parts: Vec<PartArg>) -> LineCheckView;
pub async fn med_chart(state, patient_id: String, charts: Vec<String>, line: ChartArg, credential: Option<Credential>) -> Result<MedChartWrite, ErrorView>;
pub struct PartArg { pub term: String, pub coding: Option<CodingArg> }
pub struct ChartArg { pub parts: Vec<PartArg>, pub typed_drug: String, pub directions: String,
                      pub charted_as: String, pub reason: Option<String> }   // "started-here" | "pre-existing"
pub struct LineCheckView { pub marks: Vec<String>, pub notes: Vec<String>, pub unverified: bool, pub reason_needed: bool }
```

`med_chart` refusal order, all before the key:
1. on screen and writable (`allergies::on_screen_and_writable`);
2. `charted_as` parses;
3. each coding is canonical (`allergy_check::canonical_code`);
4. it **re-runs the check** on a fresh read. It never trusts the webview's earlier answer. If it is `started-here` and there is a mark, a blank trimmed reason refuses with `Charting despite a listed allergy needs a reason`, and `against` = the fresh check's.

The `AssertMedicationInput` comes from `term_to_sign` + `read_directions`:
- `sig` = `directions` as typed;
- `info_source` = `clinician-observed` for started-here and `patient-reported` for pre-existing;
- `started` = today (`day` precision) for started-here and `None` for pre-existing;
- one part → `coding`; ≥ 2 parts → `components`.

Then `chart_medication`, `record_timing("medication-chart", …)`, and re-read the medication pane. A landed sign is always Ok (as `allergies::written`); an unknown commit outcome is worded `Whether this medication was charted is NOT known — …` (as `sign_failure`).

- [ ] **Step 1: Failing tests** (`med_chart_tests.rs`; pure where they can be, DB-gated `med_chart_db_*` otherwise):
  - `a_start_line_with_an_allergy_mark_needs_a_reason`: `"   "` refused with the exact copy, before the key (no vault touched).
  - `an_already_taking_line_never_needs_a_reason`.
  - `the_check_is_reread_at_sign`: the webview says no mark, a fresh read has one, so the reason is required.
  - `no_drug_list_charts_uncoded`: drugref absent → `med_drug_search.available == false`; `med_chart` succeeds and the pane line is "not checked".
  - `the_signed_event_carries_the_override`: `allergy_override.against` = the hitting head's id.
  - `a_combination_signs_components`.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `allergy_check_tests.rs` and `allergy_check_db_tests.rs` (unchanged results for single-coded lines).
- [ ] **Step 5: Run every ROOT source guard DB-free** (memory: root guards sweep the GUI src: commit sites, `enroll_device_actor`, derived unwrap key; `hex_decode_helper.rs`, `db_errors_stay_legible.rs` per-file counts) and update any count the new file moves.
- [ ] **Step 6: Commit** `feat(window): chart a medication — commands, the re-read check, the reason`.

### Task 6: The window pane

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/med_chart.js`, `cairn-gui/cairn-gui-tauri/src/med_chart_webview_tests.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html` (script tag after `drug_pick.js`; the two buttons in the medication pane header), `src-ui/style.css`, `src-ui/main.js` (only the hook that shows the buttons on a writable chart and re-renders after a write; keep it under 500 lines)

The behaviour (design §3.1–§3.5; copy from Global Constraints):
- **The two buttons and the line.** The two buttons open one line. The pressed one is shown as pressed and sent as `charted_as`.
- **The pick.** It splits the line at ` + ` into parts. Typing 4 or more characters in a part (`MED_PICK_MIN_CHARS = 4`, design §3.1's "4–5") calls `med_drug_search` after `PICK_DELAY_MS` and lists options with their full display (Review Focus 2: the highlighted option's full name is visible).
- **Enter and the held coding.** Enter takes the highlighted option for that part, and the part then shows `Coded as: {display}` under the line. Editing that part's text drops its coding (drug_pick.js's rule, per part).
- **The read-back.** Whatever follows the drug parts is the directions. `med_line_read` runs on input, and its fields are drawn as the read-back row (Drug · Form · Strength · Dose · Route · How often), with any `unread` words shown as `not read: …`.
- **The check.** `med_line_check` runs on every coding change and on open. A mark draws the loud `Allergy` tag + words on the line. On *Start*, a mark also shows the reason field with its label. A note is muted, and unverified gets `not checked`.
- **Sign.** Sign uses the existing signer strip (`signer.js`). No dialog.
- **After Sign.** The pane re-renders from the write's answer. A refusal's text sits on the line.

- [ ] **Step 1: Failing webview tests** (the existing webview rig; assert VISIBILITY, not only text — memory: R2b-1):
  - `both_buttons_show_on_a_writable_chart_and_not_in_fixture_mode`.
  - `four_letters_list_drugs_with_full_names`.
  - `editing_a_picked_part_drops_its_coding` (Review Focus 2).
  - `the_read_back_row_shows_the_parse_and_unread_words`.
  - `a_start_hit_shows_the_reason_field_and_sign_waits_for_it`.
  - `an_already_taking_hit_shows_the_tag_but_no_reason`.
  - `no_drug_list_says_so_and_still_signs`.
- [ ] **Step 2: Run** `cargo test -p cairn-gui-tauri --test …` or `--bins med_chart_webview` as the existing webview tests run → FAIL. **Check the filter matched tests** (memory: a filter that matches nothing prints "passed").
- [ ] **Step 3: Implement.** **Step 4: Run** → PASS. **Step 5: Commit** `feat(window): the one-line medication entry and its check`.

### Task 7: Docs — ADR, spec, durable rules, RUNBOOK

**Files:**
- Create: `docs/spec/decisions/0099-charting-a-medication.md`. It records: *"prescribe" means charted* (the milestone's fourth decision); started-here / pre-existing / unknown; components, route and strength; the window-only reason; the check at charting re-read at sign; the drugs-only, site-ranked search; and what is not guaranteed (#1055 cephalosporin; aliases #1065; no safety claim for a combination; the coding overlay still single-coded; the CLI list does not yet show route/components). **Fact-check it sentence by sentence against the SQL before the PR** (memory: ADR-0077 passed two reviews with a false sentence).
- Modify: `docs/spec/index.md` (v0.101, ADR-0099), `docs/spec/data-model.md` §3.3 (one paragraph; the medication prose is §3.3, not §3.15/§3.16), `docs/spec/decisions/README.md`, `docs/developers/durable-rules.md` (a slice block: "the reason is a window rule"; "a combination emits no safety claim"; "never widen the medication views, read `medication_charting`"; "the check is re-run at sign"), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §30 + `TEMPLATE.md` (the §6 measurements below), `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- File issues for each deferral named in the ADR that has none yet: the medication coding overlay for combinations; the CLI list showing route and components; "stat" lines staying current until slice 2.
- [ ] **Step 1:** Write the docs. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` is clean. **Step 3:** `python3 scripts/check_closing_keywords.py` on the commit message. **Step 4:** Commit `docs(medication): ADR-0099, spec v0.101, RUNBOOK §30, durable rules`.

### Task 8: The whole gate

- [ ] Run every CI gate in CI's order **after the last edit** (memory: a gate is evidence only for the tree it ran against): fmt, clippy (root + GUI tree `--locked`), `cargo doc` with `RUSTDOCFLAGS=-D warnings` (both trees), `scripts/run-db-gated-tests.sh` (detached, `nohup`, a worktree at the PR head; ~2 h), the GUI tree's DB-gated `--bins`, `cargo deny`, and the closing-keyword script on the PR body. Sum the passed counts and confirm no `skipped:`.
- [ ] Push and open the PR against `main`, linking #1054 (partly built: at charting, without the de-label offer) and the design.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing "Amoxycillin 500 mg tds PO" and signing on the drug chart or the current-medications list; writing "penicillin allergy — benefit outweighs risk" beside a new order.
- **Steps:**
  - Chart a line: paper N = 1 (write and sign) → architecture M = 1 (one signed event) → UI K = 1 line + Sign.
  - Override: N = 1 written note → M = 0 extra (it is in the event) → K = 1 typed field.
  - Choosing *Start* vs *Already taking*: N = 0 on paper. It replaces the single Add press, so K is unchanged and M = 0 extra.
  - A combination before drugref has aliases: K = 2 picks where 1 would do (design §3.4, maintainer ruling). It is not `M > N`: the architecture forces one event.
- **Time + cognitive load:** ≤ 10 s to chart a common drug from the line (paper ≈ 8–15 s). The drug, dose, route and any allergy hit are readable on the line before Sign with no click. Measured by RUNBOOK §30 (human), owed by this slice.
