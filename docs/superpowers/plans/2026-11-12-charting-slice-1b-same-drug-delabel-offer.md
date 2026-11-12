# Charting slice 1b — the same-drug de-label offer — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An unsigned medication-list line whose drug is coded to the identical moiety as a live drug allergy offers that allergy's de-label; pressing it opens the allergy list's De-label row pre-filled, signed at once as its own act; and a de-label may record the tolerated drug coded (#1053).

**Architecture:**
- **No new event type.** `clinical.allergy-challenge/1` already reserves `tolerated.coding`. `cairn-event` writes it, and db/075's authoring door, which refuses it today, admits a `drugref-moiety` coding.
- **The offer rule is one pure function** in `cairn-node`: live drug-allergy heads coded to the line's moiety, never a name, group or drug-list match. The list's judge derives the offers in Rust.
- **The window adds no new door.** `allergy_challenge` gains an optional `tolerated_coding`. A new webview file draws the offer on a list line and opens the existing De-label row with values filled in. A second new file gives that row a drugs-only pick.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri), PostgreSQL ≥ 18 (plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-12-charting-slice-1b-same-drug-delabel-offer-design.md`](../specs/2026-11-12-charting-slice-1b-same-drug-delabel-offer-design.md) (approved 2026-11-12). It supersedes the charting design's §3.5 *Already taking* rows, group comment and shared transaction.

## Global Constraints

- AGPL-3.0. No new dependency. **No new event type, no new payload key.** An uncoded challenge's payload and twin stay byte-identical to today's.
- **Charting never writes the allergy list.** No medication command calls an allergy door.
- **The offer keys on code equality only:** a live head with `allergen_kind = "drug"`, `kind = "allergy"`, coding system `drugref-moiety`, and a code equal (as a `Uuid`) to a line part's `drugref-moiety` code. It applies to an allergy entry only, never an adverse effect, a "none known" statement, a historical (de-labelled) head or a withdrawn one. Never a head on a doubted chart (`PatientAllergies::doubted_charts`). Never on a line already charted, and never in slice 1a's lone-line editor.
- **One record per offer, one record per de-label.** Several matching heads give one offer each.
- **The de-label signs at once, as its own act**, through the existing `allergy_challenge` and its signer strip, never with the list's Sign.
- **A tolerated coding is a drug, never a group:** the window refuses any system other than `drugref-moiety` before the key, and re-spells the code with `canonical_code`. db/075's authoring door admits a non-null `tolerated.coding` only with system `drugref-moiety` and a shape that passes `cairn_check_coding_object`. The remote door is unchanged.
- **A coding vouches for the words it was picked with:** any edit of the De-label row's *tolerated* field drops it (drug_pick.js's rule).
- **Copy (exact; pinned by tests):**
  - the offer: `"De-label the {allergy term} allergy: tolerates {drug}"`, plus `" (recorded by {signer})"` when the head's signer is named;
  - the row's coding line: `"Coded as: {display}"`, as drug_pick.js words it;
  - an offer pressed whose allergy is no longer drawn: `"That allergy is no longer on the list; look at it again"`;
  - the pre-filled date is today as `dd/mm/yyyy` (what `parse_onset` reads as a day).
- No confirmation dialog. Files stay under 500 lines: `med_list.js` is at 497, so the offer goes in a new `med_list_delabel.js`, and `med_list.js` gains at most one call line.
- `SCHEMA_GENERATION` 78 → **79** (db/075 changes in place, so an existing database must replay it). A DB-gated suite that prints `skipped:` is not a pass.

## Review Focus

1. **A pick edited after choosing:** the clinician picks "amoxicillin", then edits the tolerated text to "amoxicillin + clav". The coding is dropped, so the edited words are signed uncoded. Pinned in Task 5.
2. **The allergy struck elsewhere between the draw and the press:** another terminal de-labels the entry. The press finds no drawn line and says so; a stale row's Sign is refused before the key by `target_before_the_key`. Pinned in Tasks 4 and 5.
3. **A combination where one part matches:** `amox + clav` against an amoxicillin allergy. The offer names amoxicillin and codes amoxicillin, never the combination. Pinned in Task 3.
4. **An uncoded allergy "amoxicillin" against a coded amoxicillin line:** the warning shows (a name match), but no offer appears. Pinned in Task 2.
5. **After the de-label signs:** the list re-reads. The line's mark and its offer are gone, and a *Start* line's reason gate goes with them. A hand tick given against that mark still holds (ADR-0100). Pinned in Task 3.

---

### Task 1: The wire and the floor admit a coded tolerated drug

**Files:**
- Modify: `crates/cairn-event/src/allergy_overlay.rs` (`AllergyChallenge` gains `pub tolerated_coding: Option<SubstanceCoding<'a>>`; `allergy_challenge_payload` writes `{"system","code","display"}` or `null`; the twin is unchanged); every constructor of `AllergyChallenge` (grep; add `tolerated_coding: None`)
- Modify: `db/075_allergy_overlays.sql` (the authoring-door clause at the `#1053` comment)
- Modify: `crates/cairn-event/src/schema_generation.rs` (79), and any pinned generation guard the bump moves
- Test: `crates/cairn-event/src/allergy_overlay.rs` tests, `crates/cairn-node/tests/allergy_overlay_floor.rs`, `db/tests/075_allergy_overlays_test.sql`

**Interfaces — Produces:** `AllergyChallenge { target, tolerated, tolerated_coding: Option<SubstanceCoding<'a>>, date, text }` (`SubstanceCoding` is `cairn_event::medication::SubstanceCoding`, re-exported or imported).

- [ ] **Step 1: Failing tests.**
  - cairn-event: `a_coded_challenge_writes_its_coding` (`p["tolerated"]["coding"] == json!({"system":"drugref-moiety","code":<id>,"display":"amoxicillin"})`); `an_uncoded_challenge_is_byte_identical` (the existing payload and twin tests pass unedited); `the_twin_names_the_term_only` (the coded twin equals the uncoded twin).
  - `allergy_overlay_floor.rs`: `the_authoring_door_admits_a_moiety_coding` (it lands, and `clinical_allergy_challenge.tolerated_coding->>'code'` reads it back); `the_authoring_door_refuses_a_group_coding` (`drugref-class` raises `tolerated.coding.system must be drugref-moiety`); `a_malformed_coding_is_refused` (missing `display`); `the_remote_door_admits_any_well_shaped_system` (`cairn.remote_apply = on`, `system = "drugref-clinical-drug"` lands).
  - db/tests/075: the same refusal and admission, as the file's other cases.
- [ ] **Step 2: Run** `cargo test -p cairn-event allergy_overlay` and `cargo test -p cairn-node --test allergy_overlay_floor -- --nocapture` → FAIL. Confirm there is no `skipped:` line.
- [ ] **Step 3: Implement.** In db/075, replace the null-only clause with: a non-null coding must have `system = 'drugref-moiety'`, else `RAISE EXCEPTION 'allergy challenge: tolerated.coding.system must be drugref-moiety, got %'`. Keep the header comment true (why: a tolerance is of a drug, never a group).
- [ ] **Step 4: Run** → PASS. Also run `twin_registry` and `projection_registry` (neither count moves) and `scripts/run-db-sql-tests.sh` for db/tests/075.
- [ ] **Step 5: Commit** `feat(allergy): a de-label may code the tolerated drug (#1053)`.

### Task 2: The same-drug rule (pure)

**Files:**
- Create: `crates/cairn-node/src/allergy/delabel_offer.rs`, `crates/cairn-node/src/allergy/delabel_offer_tests.rs`
- Modify: `crates/cairn-node/src/allergy/mod.rs` (`pub mod delabel_offer;`)

**Interfaces — Produces:**
```rust
/// The live drug-ALLERGY heads coded to exactly `moiety` (MOIETY_SYSTEM, code equal as a Uuid),
/// excluding heads on `doubted` charts. Never a name, group or drug-list match: a de-label offered
/// on a guess lifts a true allergy (design §3.1). In `r.live` order.
pub fn same_drug_heads<'a>(r: &Resolved<'a>, doubted: &[Uuid], moiety: Uuid) -> Vec<&'a AllergyRow>;
/// The offer's words (Global Constraints' copy).
pub fn offer_words(allergy_term: &str, drug: &str, signer: Option<&str>) -> String;
```

- [ ] **Step 1: Failing tests** (`delabel_offer_tests.rs`, built on `resolve` over hand-made `PatientAllergies`, as `check_tests.rs` does):
  - `an_allergy_coded_to_the_same_moiety_is_offered`
  - `a_group_coded_allergy_is_never_offered`
  - `an_uncoded_allergy_of_the_same_name_is_never_offered` (Review Focus 4)
  - `an_adverse_effect_is_never_offered`; `an_unknown_kind_is_never_offered`
  - `a_delabelled_or_withdrawn_head_is_never_offered`
  - `a_head_on_a_doubted_chart_is_never_offered`
  - `two_entries_on_one_moiety_are_both_offered`
  - `a_code_spelled_in_capitals_still_matches` (parsed as a `Uuid`)
  - `offer_words_name_the_record_and_its_signer` (`"De-label the Amoxicillin allergy: tolerates amoxicillin (recorded by Dr A)"`, and without the tail when `None`)
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib delabel_offer` → FAIL. Check that the filter matched tests.
- [ ] **Step 3: Implement.** **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** `feat(allergy): the same-drug de-label offer rule (pure)`.

### Task 3: The list's lines carry their offers

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/med_list_judge.rs` (`Judged` gains `pub offers: Vec<DelabelOffer>`, derived in `judge` only when `use_allergies` and the line is `signable`); `cairn-gui/cairn-gui-tauri/src/med_list.rs` (`ListLineView` gains `pub delabel_offers: Vec<DelabelOffer>`, from `line_view`)
- Create: `cairn-gui/cairn-gui-tauri/src/med_list_delabel.rs` (the struct and its pure builder), `src/med_list_delabel_tests.rs`, `src/med_list_delabel_db_tests.rs`

**Interfaces:**
- Consumes: Task 2's `same_drug_heads`, `offer_words`; `cairn_node::allergy::thread::resolve`.
- Produces:
```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct DelabelOffer { pub target: String /* the allergy head's event id */, pub words: String,
                          pub tolerated: String /* the part's coding display */, pub coding: CodingArg }
/// Every offer for one signable line: for each part coded MOIETY_SYSTEM, one per same-drug head,
/// deduplicated by target. Pure.
pub(crate) fn offers_for(r: &Resolved, doubted: &[Uuid], parts: &[PartArg]) -> Vec<DelabelOffer>;
```

- [ ] **Step 1: Failing tests:**
  - pure (`med_list_delabel_tests.rs`):
    - `a_combination_offers_only_its_matching_part` (Review Focus 3: `tolerated` and `coding` are amoxicillin's);
    - `an_uncoded_part_offers_nothing`;
    - `one_head_met_by_two_parts_is_offered_once`.
  - DB-gated (`med_list_delabel_db_tests.rs`, taking `db::test_serial_guard`):
    - `a_list_line_on_an_allergy_coded_to_its_drug_carries_the_offer`;
    - `a_group_allergy_line_carries_no_offer`;
    - `after_the_delabel_signs_the_offer_and_the_mark_are_gone` (Review Focus 5: sign `allergy_challenge_impl` on the target, re-read: no offer, no mark, and a hand-ticked line still `ticked`);
    - `a_changed_chart_set_carries_no_offer` (`use_allergies` false).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_list_tests`, `med_list_db_tests`, `med_list_held_db_tests`, `med_list_review_db_tests` and `med_list_webview_tests` (its `every_field_read_is_one_rust_sends` sees the new field).
- [ ] **Step 5: Run the ROOT source guards DB-free** (`db_errors_stay_legible`, `enrolment_is_never_a_write_side_effect`, `medication_list_one_snapshot`, `secret32_conversions_are_named`, `unwrap_secret_is_not_derived`, `window_role_grants`). Register whatever the new files move.
- [ ] **Step 6: Commit** `feat(window): a list line offers the same-drug de-label`.

### Task 4: `allergy_challenge` takes the tolerated coding

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/allergy_overlays.rs` (`allergy_challenge` / `allergy_challenge_impl` gain `tolerated_coding: Option<CodingArg>`, checked before the key); `src/allergy_check.rs` (a pure `tolerated_coding_before_the_key`)
- Test: `src/allergy_overlays_pure_tests.rs`, `src/allergy_overlays_tests.rs`

**Interfaces — Produces:** `pub(crate) fn tolerated_coding_before_the_key(c: &CodingArg) -> Result<String /* canonical code */, ErrorView>`. It refuses a system other than `MOIETY_SYSTEM` with `NOT_FROM_THE_LIST`, and a bad code with `BAD_CODE` (via `canonical_code`). The challenge is built with `SubstanceCoding { system: MOIETY_SYSTEM, code: &canonical, display: c.display.trim() }`.

- [ ] **Step 1: Failing tests:**
  - pure: `a_group_coding_is_not_a_tolerated_drug`; `a_moiety_code_is_respelled_canonically`.
  - DB-gated:
    - `a_coded_delabel_lands_with_its_coding`;
    - `an_uncoded_delabel_is_unchanged` (the existing challenge tests pass unedited, given `None`);
    - `a_delabel_of_a_head_struck_since_is_refused_before_the_key` (Review Focus 2: sign one de-label, then a second against the same target. It is refused with the existing words, the vault is not asked, and one event is written).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `allergy_overlays_webview_tests`.
- [ ] **Step 5: Commit** `feat(window): a de-label signs the tolerated drug coded`.

### Task 5: The webview — the De-label row's pick, and the offer on a list line

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/delabel_pick.js` (a drugs-only type-ahead on a De-label row's *tolerated* field: `med_drug_search`, its result shaped as `med_chart.js`'s pick reads it); `src-ui/med_list_delabel.js` (draws each `delabel_offers` entry under its list line as a button with `words` as its text; a press opens the De-label row); `src/delabel_webview_tests.rs`
- Modify:
  - `src-ui/allergy_overlays.js`: `openDelabel(line, li, opener, prefill)` takes an optional `{tolerated, coding, date}`, fills the row, holds the coding on the row, and sends `toleratedCoding` (the held coding, or `null`);
  - `src-ui/med_list.js`: one call from `drawLine` into `med_list_delabel.js` (stay ≤ 500 lines);
  - `src-ui/index.html`: the two script tags, `delabel_pick.js` before `allergy_overlays.js` and `med_list_delabel.js` after `med_list.js`;
  - `src-ui/style.css` if needed.

**Interfaces:**
- Consumes: Task 3's `ListLineView.delabel_offers` (`target`, `words`, `tolerated`, `coding`); Task 4's `allergy_challenge` argument `toleratedCoding`.
- Produces (JS):
  - `pressDelabelOffer(offer, opener)`: brings the allergy pane's drawn live line whose `event_id === offer.target` into view and calls `openDelabel` with `{tolerated: offer.tolerated, coding: offer.coding, date: <today as dd/mm/yyyy>}`. With no such drawn line, it sets the copy `"That allergy is no longer on the list; look at it again"` and opens nothing;
  - after the de-label's re-read, the list re-reads (`readMedDraft`).

- [ ] **Step 1: Failing webview tests** (source guards, the existing rig; assert visibility as `med_list_webview_tests.rs` does, never text alone):
  - `the_offer_is_a_button_with_rusts_words_and_no_dialog`
  - `a_press_opens_the_delabel_row_prefilled_and_signs_nothing` (`pressDelabelOffer` calls `openDelabel` and never `invoke`)
  - `a_press_on_an_allergy_no_longer_drawn_says_so`
  - `editing_the_tolerated_field_drops_its_coding` (Review Focus 1: the `input` handler nulls the held coding before anything else, and the row's "Coded as" line is hidden)
  - `the_row_sends_the_held_coding_or_null`
  - `the_list_rereads_after_a_delabel`
  - `med_list_js_stays_under_500_lines`
- [ ] **Step 2: Run** → FAIL; check that the filter matched tests. **Step 3: Implement.** **Step 4: Run** → PASS, plus `allergy_overlays_webview_tests`, `med_list_webview_tests`, `med_list_review_webview_tests` and `drug_pick_webview_tests`.
- [ ] **Step 5: Walk it headlessly** (memory recipe: stub `invoke` with Rust-shaped payloads, Playwright). Pressing an offer shows the row filled and visible, and nothing is signed until its Sign. Note the walk in the commit.
- [ ] **Step 6: Commit** `feat(window): the de-label offer on a list line, and the row's drug pick`.

### Task 6: Docs — ADR-0101, spec, durable rules, RUNBOOK

**Files:**
- Create: `docs/spec/decisions/0101-the-same-drug-delabel-offer.md`. It records:
  - lifting an allergy is always a deliberate act;
  - charting never writes the allergy list, because a drug on the list does not attest tolerance;
  - the offer, by code equality, on unsigned list lines only;
  - the de-label signs at once, as its own act;
  - the tolerated drug is coded at the authoring door (system `drugref-moiety` only);
  - the charting design's group comment and shared transaction are dropped (maintainer, 2026-11-12);
  - what is not guaranteed: no conflict flag (#1085), no allergy comment (#1084), no offer on charted lines, no offer on a name match.

  **Fact-check it sentence by sentence against the SQL and Rust.**
- Modify:
  - `docs/spec/index.md` (v0.103);
  - `docs/spec/decisions/README.md` (the row);
  - `mkdocs.yml` (the nav line);
  - the spec paragraph on allergy de-labels (grep `de-label` in `docs/spec/data-model.md`): one sentence on the coded tolerated drug and the offer;
  - `docs/developers/durable-rules.md` (a slice block: the offer keys on code equality only; charting never writes the allergy list; a tolerated coding is a drug, never a group; an edit drops the pick);
  - `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §32 + `TEMPLATE.md`;
  - `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- [ ] **Step 1:** Write the docs. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict` is clean. **Step 3:** Run `python3 scripts/check_closing_keywords.py` on the commit message. **Step 4:** Commit `docs(allergy): ADR-0101, spec v0.103, RUNBOOK §32, durable rules`.

### Task 7: The whole gate

- [ ] Run every CI gate in CI's order **after the last edit**:
  - fmt (all three trees);
  - clippy (root; the GUI tree with `--locked`);
  - `cargo doc` with `RUSTDOCFLAGS=-D warnings` (both trees);
  - `scripts/run-db-gated-tests.sh` (detached with `nohup`, in a worktree at the PR head);
  - the GUI tree's `cargo test --locked --workspace` with the `CAIRN_TEST_PG*` strings;
  - `cargo deny`;
  - the closing-keyword script on the PR body.

  Sum the passed counts and confirm there is no `skipped:` line.
- [ ] Push, mark PR #1086 ready, and write `Closes #1053` and `Closes #1054` in the PR body (both close with this slice).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** strike the allergy line, write "tolerated amoxil 12/11/26" beside it and initial it, early in the medication history.
- **Steps:**
  - De-label from the list: paper N = 1 act (strike + note + initial) → architecture M = 1 signed de-label event → UI K = 1 press (the offer) + 1 Sign on the pre-filled row. Coding the drug adds 0 steps, because the offer fills it.
  - De-label from the allergy list: as today, K = De-label + type + Sign, with the pick optional (M ≤ N).
  - Charting a drug the patient is allergic to adds 0 allergy steps: nothing is written to the allergy list.
- **Time + cognitive load:** ≤ 10 s from the press of the offer to the struck line, key held. Before the press, the line shows its drug and the offer names the allergy record it would strike (its term and signer), with no click. Measured by RUNBOOK §32 (human), owed by this slice.
