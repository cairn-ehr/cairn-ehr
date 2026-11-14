# Charting slice 3 — the drug names (adrenaline, lignocaine, acetaminophen) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician who types the name they actually use, *adrenaline*, *lignocaine*, *frusemide*, *amoxycillin*
or *acetaminophen*, finds the drug in both picks (allergy and medication). The pick row reads "adrenaline →
epinephrine". The signed term is the name they used, and the medication list and the allergy list show
"epinephrine (adrenaline)". Closes #1065 and #1073.

**Architecture:**
- **drugref supplies the names** (drugref#231, closed 2026-11-13): the view `drugref.moiety_name(moiety_uuid
  uuid, name text, name_kind text)`. Its `name_kind` is `display`, `unii_official` (drugref db/070, upper case,
  e.g. `ACETAMINOPHEN`) or `mesh_synonym` (drugref db/071, mixed case, e.g. `Lignocaine`). A synonym equal to the
  same moiety's display or official name is already left out by the view. **A name may name several moieties**
  (salts, isomers). The pick shows each one and never picks between them.
- **db/076 stays the one file that reads drugref**, edited in place (ADR-0080: the content hash replays it, so
  there is no new file and no `SCHEMA_GENERATION` bump). It gets:
  - `cairn_drug_names_available()`, a catalogue check like `cairn_drug_available()` that never raises;
  - `cairn_drug_moiety_matches(p_pat text)`, an internal plpgsql helper. It returns one row per matching moiety:
    `(moiety_uuid uuid, display text, matched text)`, where `matched` is NULL when the display name matched.
    Without the view it searches display names only, as today.
  - The two pick doors gain a trailing `matched` column. **No drugref names → exactly today's answers.**
- **The term signed is the matched name when the clinician typed a prefix of it.** The medication path does this
  in `term_to_sign`; the allergy path does it in `choosePick`, which writes the matched name into the field.
  Without this, "adren" picked as epinephrine would sign the term "adren" (`term_to_sign` today compares the
  typed text only with the display name).
- **The bracket is one pure function**, `shown_name(display, term)`. It moves from
  `cairn_node::medication::line` to `cairn_medication_view` so both trees can call it. The medication list's name
  cell and every allergy line call it, the latter for a `drugref-moiety`-coded entry only. A group-coded entry
  stays as typed: "Penicillins (allergy group) (penicillin)" would be noise. Sorting and the by-name allergy check
  are unchanged; they keep `display_name()` and `term`.
- **No wire change.** The signed coding stays `{system, code, display}`. `matched` reaches the window and goes
  back to Rust only so that `term_to_sign` can complete the term. It is never signed.

**Tech Stack:** PostgreSQL ≥ 18 (plpgsql), Rust (cairn-node, cairn-medication-view, cairn-gui-tauri,
cairn-gui-tab-medications), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-10-charting-a-medication-design.md`](../specs/2026-11-10-charting-a-medication-design.md)
§3.3 (names: generics shown, aliases funnel), §5 item 3, §7 risk 3. Issue #1065 (what is needed), #1073 (the
bracket). **Maintainer rulings 2026-11-15:** (R1) **names only**: PBS forms/strengths are deferred, since no
site has PBS data and `drugref.local_product` is empty in every local database. Brand → combination (Augmentin)
needs a drugref source that does not exist; it is filed there in Task 4. (R2) A pick row found by another name
reads **"adrenaline → epinephrine"**. (R3) The bracket goes on the **medication list and the allergy list**
(strip, lines, earlier versions, entered-in-error, historical). The CLI list is unchanged.

## Global Constraints

- db/076 is the ONLY db/ file that may name `drugref` (`no_drugref_dependency.rs`, `drug_door_callers.rs`). Every
  definer has `SET search_path = pg_catalog, pg_temp`, and every name inside it is schema-qualified.
- A door answers nothing, never an error, when drugref is absent or unreadable. A drugref without `moiety_name`
  (any release before drugref db/070) must give exactly today's answers.
- The helper is not granted to `cairn_window`: `REVOKE EXECUTE … FROM PUBLIC`, and no GRANT. The two doors keep
  their grants.
- `RETURNS TABLE` changes need db/076's guarded-DROP idiom (as for `cairn_drug_facts`). A converged node's replay
  drops nothing.
- Text rules are unchanged: < 3 characters (trimmed) → nothing; `\`, `%`, `_` escaped; a match is a word start;
  the limit stays (`cairn_drug_search` ≤ 20, the window asks 12; `cairn_drug_medication_search` ≤ 20).
- `matched` is lower-cased by the door (`lower(name)`), since UNII spells names in capitals.
- Test uuids are computed (`md5('stub …')::uuid`), never literals (house rule 6). The stubs load only inside a
  transaction that rolls back.
- Webview JS is plain, and a new or changed JS function is pinned by a `source_guard::js_fn` test (the house
  idiom; there is no JS runner).
- Every file stays under 500 lines. New tests go in new files where an existing one would cross the cap.

## Review Focus

1. **A name two moieties claim** ("cefalexin" → *cephalexin* by MeSH synonym and *cephalexin anhydrous* by UNII
   official name, as on `drugref_synonyms`) must list both, each with its own arrow. Pinned in Task 1
   (`a_shared_name_lists_every_moiety_it_names`).
2. **A moiety matched by its display name AND a synonym** ("amox" → *amoxicillin* and *Amoxycillin*) is listed
   once, as its display (matched NULL). Pinned in Task 1 (`one_moiety_is_listed_once_display_first`).
3. **An older or reshaped drugref** (no view; a view lacking a column; a view the owner cannot SELECT) answers as
   today, never with an error, in both doors. Pinned in Task 1 (`without_names_the_doors_answer_as_before`,
   `an_unreadable_or_reshaped_name_view_is_ignored`).
4. **"adren" picked as epinephrine signs "adrenaline"**, and a typed word that is a prefix of neither name keeps
   the typed words (the brand case). Pinned in Task 2 (`term_to_sign_completes_to_the_matched_name`).
5. **A line whose term already equals its display** (different case) gets no bracket ("amoxicillin", never
   "amoxicillin (Amoxicillin)"). A combination row with no single coding gets none either. A group-coded allergy
   keeps its term. Pinned in Task 3.

---

### Task 1: db/076 — the names in both doors

**Files:**
- Modify: `db/076_drug_read_door.sql` (header "What the doors read" list; new §1b and §1c; §2 and §5 bodies;
  guarded drops; REVOKE lines)
- Create: `crates/cairn-node/tests/fixtures/drug_names_stub.sql`: the two drugref name tables and the view, copied
  from drugref `db/071_moiety_mesh_synonym.sql` §2–§3 and `db/070_moiety_name.sql` §2 (the columns and the
  view body verbatim; no `ingest_run` FK; `ingest_run bigint` kept as a plain column). It adds rows for:
  epinephrine (mesh `Adrenaline`), lidocaine (mesh `Lignocaine`), paracetamol (unii `ACETAMINOPHEN`),
  amoxicillin (mesh `Amoxycillin`), cephalexin (mesh `Cefalexin`) + cephalexin anhydrous (unii `CEFALEXIN`). The
  five moieties not already in `drug_stub.sql` are inserted here. It loads after `drug_stub.sql`, in the same
  rolled-back transaction. Its header says why it is separate: a test without it is the older drugref.
- Create: `crates/cairn-node/tests/drug_names_door.rs`
- Modify: `db/tests/076_drug_read_door_test.sql`: add a section "5. names", the SQL mirror of the first two
  tests below (`\ir` the names stub after the drug stub).
- Modify: the existing callers' column counts if a test reads by position (`drug_read_door.rs` `options()` reads
  by name, so no change is expected; grep `cairn_drug_search(` and `cairn_drug_medication_search(` under
  `crates/` and `cairn-gui/` and fix any `SELECT *`).

**Interfaces:**
- Produces (SQL):
  - `cairn_drug_names_available() RETURNS boolean`, STABLE SECURITY DEFINER. True only when
    `cairn_drug_available()` is true AND `drugref.moiety_name` exists (`relkind` `v`/`m`/`r`), the owner holds
    SELECT on it, and it has the columns `moiety_uuid uuid`, `name text`, `name_kind text` (read from
    `pg_attribute` + `format_type`). Read from the catalogue only, so it never raises (the T2-a rule in §1).
    `REVOKE … FROM PUBLIC`, no GRANT.
  - `cairn_drug_moiety_matches(p_pat text) RETURNS TABLE(moiety_uuid uuid, display text, matched text)`,
    plpgsql STABLE SECURITY DEFINER. `p_pat` is ALREADY escaped (the doors escape it). With names, it returns one
    row per moiety: `DISTINCT ON (moiety_uuid)` over `moiety_name` joined to `substance_moiety`, where
    `(' ' || name) ILIKE ('% ' || p_pat || '%') ESCAPE '\'`. It prefers the display row, then the shorter name,
    then `lower(name) COLLATE "C"`. `matched` = `NULL` when the winning row is `display` or
    `lower(name) = lower(display_name)`, else `lower(name)`. Without names, it runs today's display-only `WHERE`
    with `matched` NULL. `REVOKE … FROM PUBLIC`, no GRANT.
  - `cairn_drug_search(p_text text, p_limit int) RETURNS TABLE(system text, code text, display text, matched
    text)`: the drug half reads the helper; groups keep `matched` NULL. Order: `tier`, then `(matched IS NOT
    NULL)`, then `length(coalesce(matched, display))`, then `display COLLATE "C"`.
  - `cairn_drug_medication_search(p_text text, p_limit int) RETURNS TABLE(code text, display text, uses bigint,
    matched text)`: the moiety rows come from the helper, joined to `counts` as today. Order: `uses DESC`, then
    `(matched IS NOT NULL)`, then `length(coalesce(matched, display))`, then `display COLLATE "C"`.
  - Guarded DROP before each door, as for `cairn_drug_facts`: drop when `pg_get_function_result(...)` is not
    the NEW shape string and the function exists.

- [ ] **Step 1: Write the failing tests** in `drug_names_door.rs`, using `drug_read_door.rs`'s rig and
  `with_stub!` pattern, with the names stub loaded as well. Each test asserts `"display|matched"` rows (matched
  `-` for NULL), in door order:
  - `alias_names_find_the_drug_and_say_which_name_matched`: `cairn_drug_search('adren', 12)` drug rows contain
    `epinephrine|adrenaline`; `'acetam'` → `paracetamol|acetaminophen`; `'ligno'` → `lidocaine|lignocaine`;
    `cairn_drug_medication_search('adren', 20)` → `epinephrine|adrenaline`.
  - `one_moiety_is_listed_once_display_first` (RF2): `'amox'` → exactly one `amoxicillin|-` row (the medication
    door included).
  - `a_shared_name_lists_every_moiety_it_names` (RF1): `'cefalexin'` → both `cephalexin|cefalexin` and
    `cephalexin anhydrous|cefalexin`.
  - `without_names_the_doors_answer_as_before` (RF3): drug stub only; `'amox'` from both doors equals the
    pre-change expectation (copy the existing assertions' lists), every `matched` NULL; `'adren'` → no rows.
  - `an_unreadable_or_reshaped_name_view_is_ignored` (RF3): with the names stub, (a) `REVOKE SELECT ON
    drugref.moiety_name FROM <owner>` using `drug_read_door.rs`'s owner-role idiom (its
    `an_unreadable_drugref_is_unavailable_not_a_fault`), (b) `CREATE OR REPLACE VIEW` without `name_kind` (drop
    and recreate); each → `cairn_drug_names_available()` false, `'adren'` → no rows, `'amox'` → `amoxicillin|-`,
    no error.
  - `the_alias_text_is_still_literal`: `'%%%'` and `'ad_en'` → no rows.
  - `the_helper_is_not_the_windows`: `has_function_privilege('cairn_window', 'cairn_drug_moiety_matches(text)',
    'EXECUTE')` and the same for `cairn_drug_names_available()` are false.
  - `a_replay_converges`: `db::force_schema_replay` twice, then both doors' `pg_get_function_result` equal the
    new shapes.
- [ ] **Step 2: Run** `CAIRN_TEST_PG=… cargo test -p cairn-node --test drug_names_door -- --nocapture` → FAIL (no
  `matched` column). Check the output carries no `skipped:` line.
- [ ] **Step 3: Implement** the four SQL objects above in db/076. Update its header list "What the doors read" to
  include `moiety_name(moiety_uuid, name, name_kind)  optional; drugref db/070 + db/071`, and say in §2/§5 that
  `matched` is display-only intent, never signed.
- [ ] **Step 4: Run** the new file, `--test drug_read_door`, `--test medication_drug_search_door`, `--test
  drug_read_door_shape`, `--test drug_door_callers`, `--test no_drugref_dependency`, and
  `scripts/run-db-gated-tests.sh`'s SQL step for `db/tests/076_drug_read_door_test.sql` → all PASS. The stub's
  names are those `drugref_synonyms` (:5532) holds for these moieties; say so in the stub's header.
- [ ] **Step 5: Commit** `feat(medication): the drug picks search drugref's moiety names (db/076)`.

### Task 2: cairn-node — `matched` on an option, and the term completes to it

**Files:**
- Modify: `crates/cairn-node/src/allergy/drug_facts.rs` (`DrugOption`, `search_drugs`,
  `try_search_medication_drugs`)
- Modify: `crates/cairn-node/src/medication/line.rs` (`term_to_sign`) + `line_tests.rs`
- Test: `crates/cairn-node/tests/drug_names_door.rs` (one Rust-reader test)

**Interfaces:**
- Consumes: Task 1's door shapes.
- Produces:
  - `pub struct DrugOption { pub system: String, pub code: String, pub display: String, pub matched:
    Option<String> }`. Read it from the door by column NAME (`r.try_get("matched")`), not by position.
  - `pub fn term_to_sign(typed: &str, picked_display: Option<&str>, picked_matched: Option<&str>) -> String`:
    the name to compare with is `picked_matched.or(picked_display)`. When the typed text (trimmed,
    case-insensitive) is a prefix of it, return that name; otherwise return the typed text trimmed.

- [ ] **Step 1: Write the failing tests.** In `line_tests.rs`, `term_to_sign_completes_to_the_matched_name` (RF4):
  `term_to_sign("adren", Some("epinephrine"), Some("adrenaline")) == "adrenaline"`;
  `term_to_sign("Adrenaline", Some("epinephrine"), Some("adrenaline")) == "adrenaline"`;
  `term_to_sign("amox", Some("amoxicillin"), None) == "amoxicillin"`;
  `term_to_sign("Augmentin", Some("amoxicillin"), None) == "Augmentin"`;
  `term_to_sign("epi", Some("epinephrine"), Some("adrenaline")) == "epi"` (the door gave the alias, so the
  typed prefix of the display did not match; keep the words). Update the existing
  `term_to_sign_keeps_what_was_typed_unless_unfinished` calls to pass `None`. In `drug_names_door.rs`,
  `the_node_reads_the_matched_name`: `search_drugs(&tx, "adren")` and `search_medication_drugs(&tx, "adren", 20)`
  each hold `DrugOption { display: "epinephrine", matched: Some("adrenaline"), .. }`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib medication::line` and the DB test → FAIL (arity, field).
- [ ] **Step 3: Implement** both. Grep for every `term_to_sign(` and `DrugOption {` in BOTH trees (`crates/`,
  `cairn-gui/`) and fix each call site. The GUI's own call site is Task 3's, so here only make it compile by
  passing `None`, with a `// Task 3` note removed in Task 3.
- [ ] **Step 4: Run** `cargo test -p cairn-node --lib`, the DB test, and `cargo check` in `cairn-gui/` → PASS.
- [ ] **Step 5: Commit** `feat(medication): the signed term completes to the name that matched`.

### Task 3: the window — the arrow in both picks, the bracket on both lists

**Files:**
- Modify: `crates/cairn-medication-view/src/row.rs`: move `shown_name` here as `pub fn shown_name(display: &str,
  term: &str) -> String` (body unchanged), add `impl MedicationRow { pub fn shown_name(&self) -> String }`,
  which returns `shown_name(display, term)` when `coding_display` is `Some(display)` and `display_name()`
  otherwise. Move its test from `cairn-node/src/medication/line_tests.rs`. Delete it from `line.rs`.
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/row_view.rs:180`:
  `primary: row.shown_name()`.
- Modify: `crates/cairn-node/src/allergy/view.rs`: a new `pub fn allergen_name(row: &AllergyRow) -> String`.
  It returns `shown_name(&coding.display, &row.term)` when `row.coding` is `Some` with system `MOIETY_SYSTEM`
  (`allergy::check::MOIETY_SYSTEM`), and `row.term` otherwise. `entry_line_text` passes it in place of
  `&row.term`. Modify `crates/cairn-node/src/allergy/history_words.rs::historical_text`: `allergen_name(r)` in
  place of `r.term`.
- Modify: `cairn-gui/cairn-gui-tauri/src/allergy_check.rs`:
  `CodingArg { …, #[serde(default, skip_serializing_if = "Option::is_none")] pub matched: Option<String> }`.
  `drug_search_answer` and `med_chart.rs`'s medication answer copy `matched`. Every other constructor of
  `CodingArg` passes `matched: None` (grep `CodingArg {`).
- Modify: `cairn-gui/cairn-gui-tauri/src/med_line_sign.rs::parts_before_the_key`: pass
  `coding.matched.as_deref()` as `term_to_sign`'s third argument.
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/drug_pick.js`: in `pickLabel(option)`, when `option.matched` is set,
  return `option.matched + " → " + option.display`. In `choosePick`, set the field to `option.matched ||
  option.display` and hold `{system, code, display}` only. The held coding never carries `matched`: what
  `allergy_record` sends is unchanged.
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/med_chart.js`: `drawMedPick` draws `option.matched ? option.matched +
  " → " + option.display : option.display`. `chooseMedPick` holds `matched` in the coding, so that it reaches
  `parts_before_the_key`.
- Test: `cairn-gui/cairn-gui-tauri/src/drug_pick_webview_tests.rs`, `med_chart_webview_tests.rs`
  (source guards); `crates/cairn-node/src/allergy/view_tests.rs`; `row.rs` tests; a `med_chart` /
  `med_line_sign` pure test.

**Interfaces:**
- Consumes: Task 2's `DrugOption.matched` and `term_to_sign(typed, display, matched)`.
- Produces: `cairn_medication_view::shown_name`, `MedicationRow::shown_name()`,
  `cairn_node::allergy::view::allergen_name`, `CodingArg.matched`.

- [ ] **Step 1: Write the failing tests.**
  - `row.rs`: `shown_name_brackets_the_term_only_when_coded_and_different` (RF5): coded epinephrine / term
    "adrenaline" → `"epinephrine (adrenaline)"`; coded amoxicillin / term "Amoxicillin" → `"amoxicillin"`;
    uncoded "little white pill" → `"little white pill"`.
  - `view_tests.rs`: `a_moiety_coded_line_reads_generic_first` (RF5): an `entry(…)` with term "lignocaine" and
    coding `{drugref-moiety, <md5 uuid>, "lidocaine"}` gives `entry_line_text` starting `"Allergy: lidocaine
    (lignocaine)"`, and the view's strip contains `"lidocaine (lignocaine)"`; the same row with a
    `drugref-class` coding reads `"Allergy: lignocaine"`; a historical (de-labelled) line reads
    `"Historically reported: lidocaine (lignocaine) allergy"`.
  - `med_line_sign` pure test `a_picked_alias_signs_the_matched_name` (RF4): `parts_before_the_key(&[PartArg {
    term: "adren".into(), coding: Some(CodingArg { system: MOIETY_SYSTEM.into(), code: <md5 uuid text>,
    display: "epinephrine".into(), matched: Some("adrenaline".into()) }) }], "adren")` → term `"adrenaline"`.
  - The webview source guards: `js_fn(DRUG_PICK_JS, "pickLabel")` contains `" → "` and `option.matched`;
    `js_fn(…, "choosePick")` contains `option.matched || option.display` and its `setHeldCoding` argument has
    no `matched`; `js_fn(MED_JS, "drawMedPick")` contains `" → "`; `js_fn(MED_JS, "chooseMedPick")` holds
    `matched: option.matched`.
  - `row_view` test: a coded row with a different term draws `primary == "epinephrine (adrenaline)"`.
- [ ] **Step 2: Run** `cargo test -p cairn-medication-view`, `cargo test -p cairn-node --lib allergy::view`, and
  in `cairn-gui/`: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri --bins` (the webview and pure tests) and
  `cargo test -p cairn-gui-tab-medications` → FAIL.
- [ ] **Step 3: Implement** the edits listed under Files. Run every existing allergy-view and med-list test: a test
  asserting old text for a coded entry whose term differs from its display is now WRONG by ruling R3. Update
  its expected text and say so in the commit body. Never weaken a by-name or check assertion: those read `term`,
  which has not changed.
- [ ] **Step 4: Run** the same commands, `cargo clippy --all-targets -- -D warnings` and `RUSTDOCFLAGS="-D
  warnings" cargo doc --no-deps` in BOTH trees, plus every ROOT source guard DB-free (`CAIRN_ALLOW_DB_SKIP=1
  cargo test -p cairn-node --tests`). These read `cairn-gui/*/src`. → all PASS.
- [ ] **Step 5: Commit** `feat(window): "adrenaline → epinephrine" in the picks, "epinephrine (adrenaline)" on the lists`.

### Task 4: docs — ADR-0106, spec v0.108, RUNBOOK §37, durable rules, the drugref issue

**Files:**
- Create: `docs/spec/decisions/0106-drug-names-in-the-pick.md`. Decisions: R1–R3. The view is optional and read
  by catalogue. One row per moiety, display first. A shared name lists every moiety. `matched` is lower-cased
  and never signed. The term completes to the matched name. The bracket: moiety-coded allergy entries only,
  sorting unchanged, the CLI unchanged. Limits: no brand → combination (drugref issue, filed in Step 3); no PBS
  forms (#1081 stays open); a salt's synonym can list a salt beside the base (drugref owner's call on #231,
  "salt/base cross-matches stay").
- Modify: `docs/spec/decisions/README.md` (the ADR log's table), `mkdocs.yml` nav,
  `docs/spec/index.md` (**Spec version:** 0.108), and the spec's medication section, data-model.md §3.3 (never
  §3.15/§3.16; see memory), with a two-line note.
- Modify: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`: `## 37. Drug names in the pick (charting slice 3,
  ADR-0106)`. With `drugref_synonyms` copied in, it checks: type "adren" in each pick and see "adrenaline →
  epinephrine"; choose it, sign, and read "epinephrine (adrenaline)" on the list; "cefalexin" lists two; a
  §1.2 stopwatch on a local-name pick.
- Modify: `docs/developers/durable-rules.md`: a "Charting slice 3" block. (a) `matched` is never signed. (b) The
  names view is optional, so never add it to `cairn_drug_available()`'s required list, or every pre-db/070
  drugref goes dark. (c) The allergy bracket keys on `drugref-moiety` only. (d) Prefix-complete to the matched
  name, never to the display.
- [ ] **Step 1:** Write the ADR. Fact-check each sentence against db/076 and the code (memory "SDD evidence + ADR
  fact-check").
- [ ] **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict` → PASS;
  `cargo test -p cairn-node --test paper_parity_plan_section` → PASS.
- [ ] **Step 3:** File the drugref issue: `gh issue create -R cairn-ehr/drugref`, "Brand names → their
  composition for consumers' picks (Augmentin → amoxicillin + clavulanic acid)", linking #1065 and design §3.3.
  Comment on #1081 that PBS forms are still deferred (ruling R1). Run `scripts/check_closing_keywords.py` before
  each commit.
- [ ] **Step 4: Commit** `docs(medication): ADR-0106, spec v0.108, RUNBOOK §37, durable rules`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the clinician writes the drug in the word they use, "adrenaline 1 mg IM" on the drug
  chart or "lignocaine — rash" on the allergy sheet, and the next reader knows what it means.
- **Steps:** paper N = 1 (write the word) → architecture M = 1 (pick + the existing Sign; no new event or act) →
  UI K = 1 pick from the list the typed word already brings up. Today a local name costs a failed search, a retry
  under the other name or an uncoded entry, so this slice removes steps (K falls from 2–3 to 1). `M ≤ N`.
- **Time + cognitive load:** budget: a local-name pick costs no more than a display-name pick (≤ 10 s to chart a
  common drug from the line, slice 1a's budget). The arrow names both words, so the clinician does not have to
  translate in their head, and the list shows the generic with their word beside it. Measured by RUNBOOK §37
  (human).
