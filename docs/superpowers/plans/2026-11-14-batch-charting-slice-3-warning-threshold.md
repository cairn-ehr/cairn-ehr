# Batch-charting slice 3 — the warning threshold as administrator policy — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A site's administrator can lower the medication warning threshold, at the node's console only, so that a
previous adverse effect counts as a warning on an unsigned list line (withholding its default tick, asking a *Start*
line for a reason, named in the override). No setting can turn allergy warnings off.

**Architecture:** One console-only policy row (`db/081`, `note_policy`'s pattern) read by a definer door granted to
`cairn_window`. The pure check (`cairn_node::allergy::check`) keeps every adverse-effect note's record id and carries
the threshold it was judged at; two pure methods (`LineCheck::warns`, `LineCheck::warned_against`) replace every
"has an allergy mark" test on the draft path. The window's `allergy_check::check_lines` (the draft/editor path only)
reads the threshold and stamps it on each check; the charted list (`check_list`) is untouched.

**Tech Stack:** PostgreSQL 18 PL/pgSQL, Rust (`cairn-node`, `cairn-gui-tauri`), plain JS webview.

**Spec:** [`docs/superpowers/specs/2026-11-11-batch-charting-1072-design.md`](../specs/2026-11-11-batch-charting-1072-design.md)
§3.5 (the threshold), plus ADR-0099 decision 17.5 (the reason gate) and ADR-0100 decisions 8 and 11 (the ticks).

## Maintainer rulings (2026-11-14, this session)

- **Q1 — display:** a warned adverse-effect note on an unsigned list line keeps its words ("Previous adverse effect:
  …"), is drawn as loud as an allergy mark, and the tag reads **`Warning`** when there is no allergy mark. An adverse
  effect is never labelled `Allergy`.
- **Q2 — charted lines:** the threshold changes nothing on the current medication list (ADR-0098's display stays).
- **Q3 — restore:** the row is in no backup (like `note_policy`, `vault_policy`); a restore returns it to the default.
  Named in the ADR, the RUNBOOK and the verb's help; ONE issue filed covering every site-policy row a restore drops.

## Global Constraints

- Levels, highest first: **`allergy-hit`** (default) > **`adverse-effect`**. The threshold is the LOWEST level that
  warns. The stored word is one of exactly these two; nothing above `allergy-hit` exists, so no setting removes
  allergy warnings ("lowerable only", design §3.5).
- Set only at the console: the setter has no grant to any runtime role; the verb calls
  `cairn_node::conn_posture::require_local` before connecting (#899).
- An adverse-effect warning's override entry is `{source: "allergy", ref: <that adverse-effect record's event_id>}`
  — the same source as a hit (design §3.5). No wire change.
- Unknown or unreadable threshold → treat as `adverse-effect` (over-warning is the safe error, `check.rs` module doc),
  logged to stderr.
- `SCHEMA_GENERATION` 80 → **81**. Spec v0.105 → **v0.106**. New ADR **0104**. RUNBOOK **§35**.
- House rules: TDD, junior-legible comments, files < 500 lines (`med_chart.rs` 331, `check.rs` 378,
  `allergy_check.rs` 420 — check after each edit), no `| tail` on a test command.

## Review Focus

1. **A combination whose two parts meet two adverse-effect records** — `merge` must keep `adverse_against` aligned
   with `notes` (a note dropped as a duplicate by words drops its id too). Test in Task 2.
2. **A line with an adverse-effect warning AND an unverified allergy** — the tag must not hide "not checked":
   it reads `Warning · not checked`. Test in Task 3 (JS walk) and the view test.
3. **`not_read()`'s "allergy list could not be read" note** carries no record — it must never become a warning, at
   either threshold. Test in Task 2 (`warns` false) and Task 3.
4. **A hand tick made at the default threshold, then the administrator lowers it** — at Sign the adverse-effect ref
   is new, so `hand_tick_holds` fails and the line is unticked (the world moved under the tick). Test in Task 2.
5. **The charted list after lowering** — a charted line with an adverse-effect note still shows the muted note and
   no `Warning` tag (Q2). Test in Task 3.

---

### Task 1: The policy row, its doors, and the console verb

**Files:**
- Create: `db/081_medication_warning_policy.sql`, `db/tests/081_medication_warning_policy_test.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:45` (→ 81), `crates/cairn-node/src/main.rs` (new `Cmd`
  arm beside `NoteSetQuarantine`), `crates/cairn-node/tests/window_role_grants_notes.rs` (or `window_role_grants.rs`,
  wherever the window's granted doors are listed)
- Create: `crates/cairn-node/tests/cli_medication_warning_threshold.rs` (copy `cli_note_quarantine.rs`'s shape)

**Interfaces — Produces:**
- Table `medication_warning_policy (singleton boolean PK DEFAULT true CHECK (singleton), threshold text CHECK
  (threshold IN ('allergy-hit','adverse-effect')))`, `REVOKE ALL … FROM PUBLIC, cairn_agent, cairn_node`.
- `cairn_medication_set_warning_threshold(p_word text) RETURNS void` — definer; NULL = back to the default (delete
  or NULL the row); any other word raises naming the two allowed words. No grant.
- `cairn_medication_warning_threshold() RETURNS text` — definer, STABLE, `COALESCE(stored, 'allergy-hit')`; EXECUTE
  granted to `cairn_window` only.
- CLI: `cairn-node medication-set-warning-threshold (--level <allergy-hit|adverse-effect> | --default)`; prints the
  read-back, e.g. `previous adverse effects now count as warnings when charting` / `only allergy hits count as
  warnings when charting (the default)`. Help text says the setting is in no backup and a restore returns the default.

- [ ] **Step 1: Write the failing SQL test** (`db/tests/081_…`, the style of `db/tests/067_note_draft_commit_test.sql`):
  default reads `allergy-hit`; set `adverse-effect` reads it back; set NULL reads `allergy-hit`; `'nothing'`,
  `'none'`, `''` each raise; `cairn_window` can call the reader and cannot call the setter (`42501`); `cairn_window`
  cannot `SELECT` the table.
- [ ] **Step 2: Run it, see it fail** — `PGHOST=127.0.0.1 PGPORT=5532 scripts/run-db-gated-tests.sh` (SQL step) or
  the direct `psql -f` the 067 test uses. Expected: function does not exist.
- [ ] **Step 3: Write `db/081`** (replay-safe, header comment explaining the policy, the levels, "lowerable only",
  and the no-backup limit) and bump `SCHEMA_GENERATION` to 81.
- [ ] **Step 4: SQL test passes.**
- [ ] **Step 5: Failing CLI tests**: `set_threshold_refuses_a_remote_database` (both `--level adverse-effect` and
  `--default`, < 5 s, stderr contains "another machine"); `set_threshold_parses_strictly` (a bare verb, both flags
  together, and `--level nothing` all exit non-zero). Add the reader to the window-grant guard's expected list.
- [ ] **Step 6: Implement the `Cmd` arm** (clap `ArgGroup` required + exclusive, `--level` a `value_parser` of the two
  words), mirroring `NoteSetQuarantine`; tests pass.
- [ ] **Step 7: Commit** — `feat(medication): the warning threshold as a console-only policy row (db/081)`.

### Task 2: The pure rule — adverse-effect records kept, `warns`, `warned_against`, the ticks

**Files:**
- Modify: `crates/cairn-node/src/allergy/check.rs` (struct, `check_line`, `merge`), `crates/cairn-node/src/medication/draft_tick.rs`
- Test: `crates/cairn-node/src/allergy/check_tests.rs`, `check_combination_tests.rs`, `crates/cairn-node/src/medication/draft_tick_tests.rs`

**Interfaces — Produces** (in `cairn_node::allergy::check`):
- `#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)] pub enum WarningThreshold { #[default] AllergyHit, AdverseEffect }`
  with `pub fn parse(word: &str) -> WarningThreshold` (`"allergy-hit"` → `AllergyHit`, anything else →
  `AdverseEffect`, the safe error) and `pub fn as_word(self) -> &'static str`.
- `LineCheck` gains `pub adverse_against: Vec<Uuid>` (the record of `notes[i]` for `i < adverse_against.len()`;
  notes past that carry no record — `not_read`'s) and `pub threshold: WarningThreshold` (default `AllergyHit`; set
  by the caller, never by `check_medications`).
- `impl LineCheck { pub fn warns(&self) -> bool; pub fn warned_against(&self) -> Vec<Uuid>; pub fn warning_notes(&self) -> &[String]; pub fn quiet_notes(&self) -> &[String] }`
  — `warned_against` = `against` then, at `AdverseEffect`, `adverse_against`; `warns` = it is non-empty;
  `warning_notes` = `notes[..adverse_against.len()]` at `AdverseEffect`, else empty; `quiet_notes` = the rest.

- [ ] **Step 1: Failing tests.** In `check_tests.rs`: an adverse-effect hit yields `notes.len() == 1` and
  `adverse_against == [its event_id]`; at `AllergyHit` `warns()` is false and `warned_against()` empty; with
  `threshold = AdverseEffect` `warns()` is true and `warned_against() == [its id]`; a mark plus a note at
  `AdverseEffect` gives `[mark id, note id]` in that order; `not_read`-shaped check (`notes: ["…could not be read"]`,
  no ids) never warns at either threshold; `WarningThreshold::parse("adverse-effect")`, `parse("allergy-hit")`,
  `parse("bogus") == AdverseEffect`. In `check_combination_tests.rs`: two parts meeting two adverse records with
  different words keep both ids aligned; two parts with the SAME note words keep one note and one id (Review Focus 1).
  In `draft_tick_tests.rs`: a clean, complete line with an adverse note is default-ticked at `AllergyHit` and NOT at
  `AdverseEffect`; `hand_tick_holds` with `ticked_against = []` fails at `AdverseEffect` for that line, holds at
  `AllergyHit` (Review Focus 4); a tick that saw `("allergy", note_id)` holds at `AdverseEffect`.
- [ ] **Step 2: Run** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib allergy:: medication::draft_tick` —
  expected: compile errors / failures on the new fields.
- [ ] **Step 3: Implement.** `check_line` keeps the ids `per_allergen` already returns for notes; `merge` zips each
  of `b`'s notes with its id and pushes both only when the words are new; `default_tick` replaces
  `f.check.marks.is_empty()` with `!f.check.warns()` (rewrite the comment: a warning at the site's threshold);
  `hand_tick_holds` iterates `fresh.warned_against()`. Existing struct literals in tests take `..Default::default()`.
- [ ] **Step 4: Tests pass** (same command; also `cargo clippy -p cairn-node --all-targets -- -D warnings`).
- [ ] **Step 5: Commit** — `feat(medication): a warning is anything at the site's threshold, adverse effects included`.

### Task 3: The window — read the threshold on the draft path, gate, name, draw

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/allergy_check.rs` (`check_lines`), `med_chart.rs` (`override_before_the_key`,
  the `against` ids at :216, `view`, `LineCheckView`), `med_list.rs:440` (`against`), `med_list_sign.rs:327`,
  `med_list_judge.rs:365` (`warnings_now`), `src-ui/allergy_tags.js` (`WARNING_TAG = "Warning"`), `src-ui/med_chart.js`
  (`drawMarks`)
- Test: `med_chart_tests.rs`, `allergy_check_db_tests.rs` (or `med_list_db_tests.rs`), the list's webview test file
  that already walks `drawMarks` (grep `allergy-prominent` in `*_webview_tests.rs`)

**Interfaces:**
- Consumes: Task 1's `cairn_medication_warning_threshold()`; Task 2's `WarningThreshold`, `warns`, `warned_against`,
  `warning_notes`, `quiet_notes`.
- Produces: `LineCheckView { marks, warning_notes: Vec<String>, notes /* quiet only */, unverified, reason_needed }`;
  `reason_needed = check.warns()`.

- [ ] **Step 1: Failing tests.** `med_chart_tests.rs`: `override_before_the_key` on a Start line whose check has only
  an adverse note — `Ok(None)` at `AllergyHit`, `NEEDS_A_REASON` at `AdverseEffect`, `Ok(Some(r))` with a reason;
  an *Already taking* line never needs one; `view` at `AdverseEffect` puts the adverse note in `warning_notes`, keeps
  `not_read`'s note in `notes`, sets `reason_needed`. A DB test: with `cairn_medication_set_warning_threshold('adverse-effect')`
  (reset to NULL afterwards — add `medication_warning_policy` to that file's TRUNCATE list) a list line for a drug
  matching an adverse-effect record is not default-ticked and its stored `against` names the record; the CHARTED list's
  check for the same drug has `threshold == AllergyHit` and shows the note muted (Review Focus 5). A webview test:
  a check with only `warning_notes` shows tag text `Warning` with class `allergy-tag` and the note as
  `allergy-prominent`; with `unverified` too the tag reads `Warning · not checked` (Review Focus 2); assert
  visibility, not just text (memory: webview walks).
- [ ] **Step 2: Run them, see them fail** (`CAIRN_ALLOW_DB_SKIP` unset for the DB test; the cairn-gui gate commands in
  memory `three-cargo-trees-three-lockfiles`).
- [ ] **Step 3: Implement.** `check_lines` reads the threshold on `c` (`None` or an error → `AdverseEffect`, logged)
  and sets it on every check it returns; `check_lines_unsure`/`check_list` unchanged. Replace `marks.is_empty()` /
  `.against` on the draft path with `warns()` / `warned_against()` at the five sites listed. `drawMarks`: marks →
  `ALLERGY_TAG`; else `warning_notes` → `WARNING_TAG` (+ ` · not checked` when `unverified`); else unverified →
  `NOT_CHECKED_TAG`; list order marks, warning notes (prominent), quiet notes.
- [ ] **Step 4: Tests pass**; cairn-gui clippy + `cargo doc` clean.
- [ ] **Step 5: Commit** — `feat(window): a lowered threshold warns on an adverse effect, tagged Warning`.

### Task 4: Docs and the human pass

**Files:** `docs/spec/decisions/0104-the-warning-threshold.md`, `docs/spec/index.md` (v0.106), the ADR index,
`docs/spec/data-model.md` §3.3 (one paragraph; memory: medication prose lives in §3.3), `docs/developers/durable-rules.md`
(a "batch charting 3" block: the two words, over-warn on unknown, draft path only, `adverse_against` alignment),
`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §35, `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1:** Write ADR-0104 (decisions: the scale and default; console-only and lowerable-only; draft path only,
  Q2; display, Q1; over-warn on an unreadable threshold; adverse-effect override refs; the restore limit, Q3, with
  the filed issue's number). Fact-check every sentence against the SQL and Rust (memory: ADR fact-check).
- [ ] **Step 2:** File the restore issue (site-policy rows `note_policy`, `vault_policy`, `medication_warning_policy`
  return to defaults on restore) and cite it. Run `scripts/check_closing_keywords.py` before committing.
- [ ] **Step 3:** RUNBOOK §35: lower the threshold at the console; a Start line for a drug with a recorded adverse
  effect shows `Warning`, needs a reason, signs with it; an Already-taking line is unticked, no reason; reset to
  default; VoiceOver reads the tag.
- [ ] **Step 4:** Commit `docs(medication): ADR-0104, spec v0.106, RUNBOOK §35, durable rules`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the drug chart's allergy/adverse-reaction box, read by the prescriber before writing a new
  drug; a hospital's drug committee deciding which reactions must be acknowledged in writing.
- **Steps:** clinician: paper N = read the box + (when warned) write a justification → architecture M = the same
  (the warning appears on the line; a Start line takes a typed reason) → UI K = M; no new step at the default
  threshold. Administrator: paper = one committee memo → one console command. `M > N`: no.
- **Time + cognitive load:** budget unchanged from RUNBOOK §30/§31 (the line's check is read in the same pass); the
  added read is one STABLE function call per check. RUNBOOK §35 confirms a lowered site's Start line still signs
  within §30's budget.
