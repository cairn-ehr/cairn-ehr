# Charting slice 2 — a stat dose, ordered then given — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ceftriaxone 1g IV stat` charted on a Start line is a **stat order**: it reads "· not yet given" on the
current list until whoever gives it presses **Given**, states the time (default now, editable to earlier) and
signs. The given dose then leaves the current list and its row reads, in time order, "ordered 12:34 Dr A · given
13:22 RN B". A given recorded by mistake is struck by **Entered in error**, and the line returns to the current
list, "not yet given" again. Closes #1069.

**Architecture:**
- **One new optional wire key**, `stat` (JSON `true`, or absent), on `clinical.medication.asserted`, carried by
  `Charting` and projected into `medication_charting.stat`. The window signs it only on a **pure one-off Start
  line**: the directions read `stat`, with no frequency and no `prn`, and the press is *Start medication*
  **[maintainer, 2026-11-14, Q2]**.
- **Given = two events under one signature, in one transaction [maintainer, Q1]:** a cessation (reason
  `"stat dose given"`, no stop date) and then a new `clinical.medication-given.asserted` whose `completes` names
  that cessation's `event_id`. "Current" keeps meaning "no `medication_cessation` row", so every currency reader
  is unchanged (the allergy check, sign-off, reconciliation, the restore lineage, the CLI, sync). An older node
  reads "stopped: stat dose given". The given type stays general: a future MAR's given carries no `completes` and
  stops nothing.
- **Entered in error = `clinical.medication-given-retraction.asserted` [maintainer, Q4]**, which names the given
  (`target`) **and the stop it voids** (`voids`, copied from the given's `completes` at signing, and held equal
  to it at the authoring door). The cessation appliers (db/031, db/060, both edited in place) skip a voided stop.
  The retraction's applier deletes the voided stop's rows and re-folds the thread through db/037's
  `cairn_refold_after_shred`, so a *real* stop of the same thread wins again. The void keys on the retraction
  alone, so shredding the given cannot bring the stop back (Review Focus 3).
- **The display [maintainer, Q3]:** no new section. A given stat dose keeps its own row where stopped lines are,
  `status_label` `"given"`. One line of words in time order sits under the drug line. A struck given stays
  visible, with who struck it.

**Tech Stack:** Rust (cairn-event, cairn-node, cairn-gui-tauri, cairn-gui-tab-medications), PostgreSQL ≥ 18
(plpgsql), plain JS + Tauri 2 (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-10-charting-a-medication-design.md`](../specs/2026-11-10-charting-a-medication-design.md)
§3.6, §5 item 2, §6 (benchmark) and §10 Q3. It builds on [ADR-0099](../../spec/decisions/0099-charting-a-medication.md)
(limit (f) is this slice) and [ADR-0097](../../spec/decisions/0097-allergy-overlays-and-differing-accounts.md)
(the entered-in-error shape mirrored here). **Not here:** a dose-by-dose MAR; a "Waiting for you" group for
ungiven stat doses (filed in Task 7); an allergy re-check at the moment of giving; slice 3 (aliases, PBS forms).

## Global Constraints

- AGPL-3.0. No new dependency. **Wire is forever (principle 11):** additions only: one key on
  `clinical.medication.asserted` and two new types. An assertion without `stat` serialises byte-identically to
  today (pinned).
- **Every earlier durable rule holds** (`docs/developers/durable-rules.md`: read the charting 1a, batch charting
  1, 2a, 2b and 3 blocks first). The webview's answer is never trusted: the line, its stat flag, its not-yet-given
  state and the given being struck are all re-derived in Rust from a FRESH read before the key. No confirmation
  dialog. Acts only on the OPENED chart's lines (never a linked chart's, never a doubted link's: 2b's rule).
- **Wire shapes (exact):**
  - `clinical.medication.asserted` payload: top-level `"stat": true` when the line is a stat order; absent
    otherwise, never `false`. Both doors: absent, JSON null (= absent) or JSON `true`, else refused. `stat` is
    not mentioned in the twin, whose sig already reads "stat".
  - `clinical.medication-given.asserted`, `schema_version` `clinical.medication-given/1`, payload exactly
    `{medication_id, completes}`: `completes` is a uuid string or JSON null. The envelope's **`t_effective` is the
    time given**, required at the authoring door. Twin: `"Dose given: medication {medication_id}"`, then
    `" — completes its stat order"` when `completes` is set.
  - `clinical.medication-given-retraction.asserted`, `schema_version` `clinical.medication-given-retraction/1`,
    payload exactly `{medication_id, target, voids, text}`: `target` is the given's `event_id`; `voids` is a uuid
    string or null; `text` is a string or null, and blank is written null (ADR-0097 decision 3). Twin:
    `"Entered in error: withdraws given dose {target}"`, then `" — {text}"`.
  - Both new types: `event_type_class` `additive`, `targets_other_author = FALSE` (a nurse may strike a doctor's
    entry, as on paper); `safety` must be null, and no charting key (`charted_as`, `warning_override`,
    `restores`, `stat`) may appear, at both doors; born-sealed; contributors `{human, authored}` +
    `{node, recorded}` (ADR-0053).
  - The paired cessation: `reason` `"stat dose given"`, no `stopped`, human-signed like the given.
- **Copy (exact; pinned by tests):**
  - buttons: `GIVE = "Given"`, `STRIKE = "Entered in error"`; the give row's field label `"Time given"`; the
    strike row's field label `"Why (optional)"`; both rows sign through the strip, as the allergy overlay rows do
  - the stat line under the drug line: `"stat · ordered {o} {orderer} · not yet given"`;
    `"stat · ordered {o} {orderer} · given {g} {giver}"`; and, when the time given is earlier than the order's
    `t_recorded`, `"stat · given {g} {giver}, before it was charted {o} {orderer}"` (§10 Q3: shown, never refused,
    never reordered). Several gives (one struck, one standing) chain in time order, joined by `" · "`.
  - a struck give, as its own line: `"given {g} {giver} — entered in error by {who}, {when}"`, then
    `" — {why}"`
  - `status_label` `"given"`. main.js styles it as it styles `"ceased"` (`data-ceased`).
  - `{o}` is the order event's `t_recorded`, `{g}` the given's `t_effective` and `{when}` the retraction's
    `t_recorded`, each through the tab crate's `local_instant`. `{orderer}`, `{giver}` and `{who}` are
    `cairn_note_signer_name(cairn_note_vouched_attester(event_id))` (db/065), else `"signer not known"` (2b's
    rule).
  - outcomes: `"Given."`; `"Entered in error: the stat dose is waiting to be given again."` (or `"Entered in
    error."` when the struck given stopped nothing); a landed act whose re-read failed answers Ok with 2b's
    `checked()` wording ("Signed — but the medication list could not be read again …").
  - refusals, before the key: `STAT_NOT_OPEN = "That stat dose is no longer waiting to be given; read the chart
    again"`; `GIVEN_LATER_THAN_NOW = "The time given cannot be later than now"`; `GIVE_GONE = "That given dose is
    no longer on this chart, or is already struck; read it again"`
  - above the list when the stat read fails: `STAT_NOT_READ = "Stat doses could not be read: no line shows whether
    it was given"`. The list still opens, and a given line still reads stopped.
- **Files stay under 500 lines.** Today: `chart_set/mod.rs` 473, `main.js` 490, `med_list.js` 494,
  `row_view.rs` 413, `index.html` 677 (markup; a new `<script>` tag only). New code goes in new files.
- `SCHEMA_GENERATION` 81 → **82**; new file **`db/082_medication_given.sql`**. db/031, db/037, db/048, db/060 and
  db/077 are edited in place (ADR-0080: the content hash replays them). A DB-gated suite printing `skipped:` is
  not a pass; set `CAIRN_ALLOW_DB_SKIP=1` only for a deliberate DB-free run.

## Review Focus

1. **A real stop and a given of the same stat line** (two terminals, or a peer): when the given is struck, the
   real stop must still stand, so the line stays stopped. Pinned in Task 3
   (`a_struck_given_leaves_a_real_stop_standing`, both arrival orders).
2. **A heal, a rebuild or a restore, in any arrival order** (retraction before the given, cessation last): a
   struck given's line reads current and an unstruck given's line reads stopped. Pinned in Task 3
   (`every_arrival_order_converges`, `a_reprojection_keeps_a_struck_givens_line_current`) and Task 4 (a restore).
3. **Shredding**: shredding the given leaves a struck line current, because the void keys on the retraction.
   Shredding the retraction stops the line again ("given" stands). Shredding the paired stop leaves the line
   current. Pinned in Task 3.
4. **The time given**: a time later than now is refused before the key (`GIVEN_LATER_THAN_NOW`). A time before
   the order is signed and worded "before it was charted". A partly typed time is refused in words, never
   signed as midnight. Pinned in Task 5 (pure) and Task 6 (webview).
5. **A stale press**: Given on a line that was stopped, given or set aside since the draw, or Entered in error on
   a given struck since, is refused before the key with no event written. Pinned in Task 5.

---

### Task 1: The wire builders (cairn-event)

**Files:**
- Create: `crates/cairn-event/src/medication/given.rs` (both new types)
- Modify: `crates/cairn-event/src/medication/charting.rs` (`Charting` gains `stat: bool`; `insert_charting` writes
  `"stat": true` only when true), `crates/cairn-event/src/medication/set_aside.rs` (`set_aside_body` strips `stat`
  beside the other charting keys), `crates/cairn-event/src/medication/mod.rs` (exports). Every `Charting { … }`
  literal gains `stat: false` (`grep -rn "Charting {"`: version_settle_read.rs, draft.rs, set_aside_read.rs,
  set_aside.rs, med_line_sign.rs, med_list_versions_sign.rs, plus tests). Run the full-workspace build.

**Interfaces — Produces:**
```rust
// charting.rs — Charting gains:
pub stat: bool,   // a stat ORDER (pure one-off); top-level `"stat": true`, absent when false
// given.rs
pub const GIVEN_EVENT_TYPE: &str = "clinical.medication-given.asserted";
pub const GIVEN_SCHEMA: &str = "clinical.medication-given/1";
pub const GIVEN_RETRACTION_EVENT_TYPE: &str = "clinical.medication-given-retraction.asserted";
pub const GIVEN_RETRACTION_SCHEMA: &str = "clinical.medication-given-retraction/1";
pub const STAT_GIVEN_REASON: &str = "stat dose given";
pub struct Given<'a> { pub medication_id: &'a str, pub completes: Option<&'a str> }
pub fn given_body(g: &Given) -> Value;                 // {medication_id, completes} (null when None)
pub fn render_given_twin(g: &Given) -> String;         // Global Constraints copy
pub struct GivenRetraction<'a> { pub medication_id: &'a str, pub target: &'a str,
                                 pub voids: Option<&'a str>, pub text: Option<&'a str> }
pub fn given_retraction_body(r: &GivenRetraction) -> Value;   // blank text → null
pub fn render_given_retraction_twin(r: &GivenRetraction) -> String;
```

- [ ] **Step 1: Failing tests:**
  - charting: `an_assertion_without_stat_is_byte_identical` (today's fixture, before and after),
    `stat_is_a_top_level_true_and_absent_when_false`, `a_set_aside_body_never_carries_stat`
  - given: `the_given_body_is_exactly_medication_id_and_completes`, `a_none_completes_is_json_null`,
    `the_given_twin_says_it_completes_its_stat_order`, `the_retraction_body_is_exactly_four_keys`,
    `blank_retraction_text_is_null_and_absent_from_the_twin`, `the_retraction_twin_names_its_target`
- [ ] **Step 2: Run** `cargo test -p cairn-event medication::` → FAIL.
- [ ] **Step 3: Implement.** The module header of `given.rs` explains why the given type is general (a future
  MAR) while `completes` ties one given to the stop that completes a stat order, and why the retraction carries
  `voids` itself (Review Focus 3).
- [ ] **Step 4: Run** → PASS; then `cargo build --workspace --all-targets` and the GUI tree's build.
- [ ] **Step 5: Commit** `feat(medication): the stat key and the given / retraction builders (wire)`.

### Task 2: The `stat` floor + projection, and the given type (db/082 §1)

**Files:**
- Create: `db/082_medication_given.sql` (header + §1), `crates/cairn-node/tests/medication_given_floor.rs`,
  `db/tests/082_medication_given_test.sql` (`\ir _scratch_database_guard.sql`)
- Modify: `db/077_medication_charting.sql` (in place: `cairn_check_medication_charting` judges `stat`; `ALTER
  TABLE medication_charting ADD COLUMN IF NOT EXISTS stat boolean NOT NULL DEFAULT false`; the apply reads it, and
  its "nothing present → RETURN" test counts it), `db/079_medication_versions.sql` (the set-aside refuses `stat`
  at both doors, with the other charting keys), `db/037_born_sealed.sql` (`cairn_execute_shred`: a
  `to_regclass`-guarded `DELETE FROM medication_given WHERE content_address = v_ca`), `db/048` (`cairn_event_thread`:
  `UNION ALL SELECT medication_id, content_address FROM medication_given` behind its own `to_regclass` probe, as 2b's
  checked table), `crates/cairn-node/src/db.rs` (register `082_medication_given`),
  `crates/cairn-event/src/schema_generation.rs` (82), `migration_replay_widening.rs` (`WIDENED`), the node's
  `read_charting` (returns `stat`), the pinned counts (twin_registry.rs + its verbatim list +
  `db/tests/034_twin_registry_test.sql`; projection_registry.rs + `db/tests/039_projection_registry_test.sql`:
  each **+1 from the figure in the file today**), and the test wipe lists (`tests/common/restore_kit.rs`,
  `tests/common/medication_kit.rs`).

**Interfaces — Produces:**
```sql
-- event_type_class ('clinical.medication-given.asserted', 'additive', FALSE)
cairn_check_medication_given(p_type text, b jsonb) RETURNS void
-- both doors: payload an object; medication_id a uuid string; completes a uuid string or null; safety null;
--   no charting key.
-- authoring door only: the schema tag; t_effective present (b ->> 't_effective'); medication_id names a
--   medication_statement row of THIS chart; when completes is set: it names an event_log row of type
--   clinical.medication-cessation.asserted, same patient_id, whose cairn_clear_payload ->> 'medication_id' is
--   this medication_id (submitted EARLIER in the same transaction), and the line's medication_charting.stat is true.
CREATE TABLE IF NOT EXISTS medication_given (content_address bytea PRIMARY KEY, event_id uuid NOT NULL,
  patient_id uuid NOT NULL, medication_id uuid NOT NULL, completes uuid, given_at timestamptz NOT NULL,
  hlc_wall bigint NOT NULL);   -- index (patient_id), (medication_id); GRANT SELECT TO cairn_agent
-- medication_given_apply(e event_log): cairn_clear_payload(e); SKIPS a bad shape or a NULL e.t_effective (never
--   raises); ON CONFLICT DO NOTHING; cairn_projection_apply row (run_order 20, heal_safe TRUE; projection_tables
--   ARRAY['medication_given']); #214 converge idiom.
```
The t_effective ceiling (db/001, `teffective_ceiling.rs`) already bounds a forward-dated time; **find first**
what it does with a time later than `t_recorded` (clamp-and-flag or refuse) and state it in the header.

- [ ] **Step 1: Failing DB tests** (`medication_given_floor.rs`, `db::test_serial_guard`; reuse
  `tests/common/medication_kit.rs`):
  - `stat_true_projects_and_absent_reads_false`, `stat_false_or_a_string_is_refused_at_both_doors`,
    `a_set_aside_carrying_stat_is_refused_at_both_doors`
  - `a_given_completing_its_stat_lines_stop_projects`, `a_given_with_no_completes_projects_and_stops_nothing`
  - `a_given_with_no_t_effective_is_refused_at_the_authoring_door`
  - `completes_naming_another_threads_or_no_stop_is_refused_at_the_authoring_door`,
    `completes_on_a_line_that_is_not_a_stat_order_is_refused`, `another_charts_line_is_refused`
  - `the_remote_door_admits_an_orphan_given` (no line here; projected; shown nowhere)
  - `a_safety_claim_or_a_charting_key_is_refused_at_both_doors`, `a_shred_removes_the_row`,
    `a_reprojection_rebuilds_it`, `the_given_event_resolves_to_its_lines_thread`
- [ ] **Step 2: Run** `cargo test -p cairn-node --test medication_given_floor -- --nocapture` → FAIL (no
  `skipped:`).
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** → PASS, plus `medication_charting_floor`, `medication_charting_projection`,
  `medication_set_aside_floor`, `migration_replay_widening`, `twin_registry` + `projection_registry` DB-free, and
  the db/tests SQL suite (`PGHOST=127.0.0.1 PGPORT=5532`, #862).
- [ ] **Step 5: Commit** `feat(medication): the stat key and the given event type (db/082)`.

### Task 3: Entered in error, and the voided stop (db/082 §2)

**Files:**
- Modify: `db/082_medication_given.sql` (§2), `db/031_medication.sql` (in place: `medication_cessation_apply`
  RETURNs before its insert when `cairn_medication_cessation_voided(e.event_id, med)`), `db/060` (in place: the
  reason apply skips the same way), `db/037` (shred hook for `medication_given_retraction`), `db/048` (thread probe),
  the four pinned counts again (+1 each), the wipe lists.
- Create: `crates/cairn-node/tests/medication_given_retraction_floor.rs`

**Interfaces — Produces:**
```sql
-- event_type_class ('clinical.medication-given-retraction.asserted', 'additive', FALSE)
cairn_check_medication_given_retraction(p_type text, b jsonb) RETURNS void
-- both doors: medication_id + target uuid strings; voids a uuid string or null; text a string or null; safety null.
-- authoring door only: the schema tag; target names a medication_given row of THIS chart and THIS medication_id;
--   voids IS NOT DISTINCT FROM that row's completes.
CREATE TABLE IF NOT EXISTS medication_given_retraction (content_address bytea PRIMARY KEY, event_id uuid NOT NULL,
  patient_id uuid NOT NULL, medication_id uuid NOT NULL, target uuid NOT NULL, voids uuid, text text,
  hlc_wall bigint NOT NULL);
cairn_medication_cessation_voided(p_cessation uuid, p_med uuid) RETURNS boolean STABLE
-- EXISTS a medication_given_retraction row with voids = p_cessation AND medication_id = p_med.
-- medication_given_retraction_apply(e): insert (ON CONFLICT DO NOTHING); then, when voids is set: DELETE the
--   voided stop's medication_cessation and medication_cessation_reason rows (by the stop's content_address,
--   looked up in event_log by event_id = voids), and PERFORM cairn_refold_after_shred(voids,
--   'clinical.medication-cessation.asserted', ARRAY[medication_id::text]) so another stop of the thread wins again.
--   Registered with projection_tables ARRAY['medication_given_retraction', 'medication_cessation',
--   'medication_cessation_reason'] (it writes all three), run_order 30, heal_safe TRUE.
```
**Find first, and state each in the header:**
- (a) whether `cairn_refold_after_shred` must run under `cairn_lock_shred_refold()` when called outside a
  shred, and whether calling it from an applier nests safely inside `cairn_reproject`'s heal (its posture save and
  restore);
- (b) what db/039's narrow rebuild does with a table that two types list (`medication_cessation` is listed by the
  cessation type and now by the retraction type). It must never truncate the stops and replay only retractions:
  pin `a_narrow_rebuild_of_the_retraction_type_keeps_every_stop`;
- (c) that a shred of the retraction re-folds the cessations (they share `medication_cessation`, so
  `cairn_refold_after_shred` picks them up, and the retraction payload's `medication_id` names the thread via
  `cairn_medication_threads_named`).

- [ ] **Step 1: Failing DB tests** (`medication_given_retraction_floor.rs`):
  - `a_retraction_returns_the_stat_line_to_current` (the line is in `patient_medication_current` again, its
    "stat dose given" reason gone from `medication_cessation_reason`)
  - `a_struck_given_leaves_a_real_stop_standing` (Review Focus 1: the real stop signed before the given, then
    after it, by HLC)
  - `every_arrival_order_converges` (remote door: the six orders of cessation / given / retraction)
  - `a_reprojection_keeps_a_struck_givens_line_current`,
    `a_narrow_rebuild_of_the_retraction_type_keeps_every_stop`
  - `shredding_the_given_keeps_the_line_current`, `shredding_the_retraction_stops_the_line_again`,
    `shredding_the_paired_stop_leaves_the_line_current` (Review Focus 3)
  - `voids_differing_from_the_givens_completes_is_refused_at_the_authoring_door`,
    `another_charts_or_an_unknown_given_is_refused_at_the_authoring_door`
  - `the_remote_door_admits_an_orphan_retraction`, `a_retraction_of_a_given_with_no_completes_stops_nothing`,
    `a_second_retraction_is_admitted_and_harmless`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** The header explains why the void keys on the retraction
  alone, never on the given row (Review Focus 3).
- [ ] **Step 4: Run** → PASS, plus Task 2's suite, `medication_cessation_*`, `medication_chart`,
  `shred_refold*` (whatever #863's tests are named: `grep -ln cairn_refold_after_shred crates/cairn-node/tests`)
  and the db/tests SQL suite.
- [ ] **Step 5: Commit** `feat(medication): entered in error voids a given's stop and re-folds the thread (db/082)`.

### Task 4: The node signs a give and a strike, and reads stat orders back

**Files:**
- Create: `crates/cairn-node/src/medication/stat_give.rs` (`give_stat`, `strike_given`),
  `crates/cairn-node/src/medication/stat_read.rs` (`read_stat_orders`), `crates/cairn-node/tests/medication_stat_give.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs`; `restore_reads_the_clinical_plane.rs` (or the restore kit it
  uses) gains a given and a struck given that must come back as they were.

**Interfaces — Produces:**
```rust
/// Give a stat order: ONE transaction, the cessation (STAT_GIVEN_REASON, no stop date) FIRST, then the given
/// (completes = the cessation's event_id, t_effective = given_at). Both signed by `author` (prepare both, then
/// submit both, as chart_draft_line's prepare_stops does). Returns the given's id.
pub async fn give_stat(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str,
  node_origin: &str, patient: Uuid, medication_id: Uuid, given_at: DateTime<Utc>, author: &AuthorParams<'_>)
  -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>>;
/// Strike a given: reads the given's `completes` from medication_given (this chart) and signs
/// `voids` = it, in its own transaction. Errors when the given is not on this chart.
pub async fn strike_given(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str,
  node_origin: &str, patient: Uuid, given_event: Uuid, text: Option<&str>, author: &AuthorParams<'_>)
  -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>>;
// stat_read.rs
pub struct Give { pub event_id: Uuid, pub given_at: DateTime<Utc>, pub giver: Option<String>,
                  pub completes: bool, pub struck: Option<Strike> }
pub struct Strike { pub who: Option<String>, pub at: DateTime<Utc>, pub why: Option<String> }
pub struct StatOrder { pub medication_id: Uuid, pub patient_id: Uuid, pub ordered_at: DateTime<Utc>,
                       pub orderer: Option<String>, pub gives: Vec<Give> /* oldest given_at first */ }
impl StatOrder { /// Waiting to be given: no standing (unstruck) give that completes it.
                 pub fn waiting(&self) -> bool; }
/// Every stat order over a chart set (medication_charting.stat = true) with its gives and their strikes, joined
/// on medication_id AND patient_id (an orphan is never returned), one query each. Names as 2b's read_settlements.
pub async fn read_stat_orders(client: &(impl GenericClient + Sync), charts: &[Uuid])
  -> anyhow::Result<Vec<StatOrder>>;
```

- [ ] **Step 1: Failing DB tests** (`medication_stat_give.rs`):
  - `give_stat_stops_the_line_and_records_the_given_in_one_transaction` (two events, one key release; the line
    leaves `patient_medication_current`; the stop's reason is exact)
  - `a_failed_given_rolls_back_the_stop_too` (force the given's floor to refuse: a non-stat line)
  - `strike_given_returns_the_line_to_current`, `strike_given_refuses_another_charts_given`
  - `read_stat_orders_names_orderer_and_giver_in_time_order`, `an_unvouched_signer_reads_as_none`,
    `a_struck_give_is_read_with_its_striker_and_why`, `waiting_is_true_until_a_standing_give`
  - `a_restore_brings_back_a_given_and_a_struck_given_as_they_were` (Review Focus 2)
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `medication_chart`,
  `medication_draft_line_sign`, and `cairn-sync`'s `clinical_pull` (`--test-threads=2`; the memory rule on
  cross-crate call sites).
- [ ] **Step 5: Commit** `feat(medication): give a stat dose, strike a given, read them back`.

### Task 5: The window — the stat key at Sign, the two commands, the snapshot read

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/med_stat.rs` (the two commands + pure helpers),
  `src/med_stat_tests.rs`, `src/med_stat_db_tests.rs`, `src/chart_stat.rs` (the read and the attach)
- Modify: `src/med_line_sign.rs` (`charting()` sets `stat` from `is_stat_order`, so BOTH Sign paths, the one
  line and the list, sign it: confirm `med_list_sign` calls `charting()`), `src/chart_versions.rs`
  (`read_chart_and_versions` reads `read_stat_orders` under its OWN savepoint, failing to `STAT_NOT_READ`), its
  callers in `src/commands.rs`, `src/main.rs` (register the commands).

**Interfaces — Produces:**
```rust
/// A stat ORDER: Start, directions read `stat`, no frequency, no prn. Pure.
pub(crate) fn is_stat_order(charted_as: ChartedAs, d: &Directions) -> bool;
#[tauri::command] pub async fn med_stat_give(state, patient_id: String, charts: Vec<String>,
  medication_id: String, given_at: Option<String> /* datetime-local text; None = now */,
  credential: Option<Credential>) -> Result<MedChartWrite, ErrorView>;
#[tauri::command] pub async fn med_stat_strike(state, patient_id: String, charts: Vec<String>,
  given_event: String, why: Option<String>, credential: Option<Credential>) -> Result<MedChartWrite, ErrorView>;
/// The time given from the row's field: None → `now`; otherwise notes_time's `stated_time_from_local`; later
/// than `now` → Err(GIVEN_LATER_THAN_NOW); a partly typed time → its own words, never midnight. Pure.
pub(crate) fn given_time(text: Option<&str>, now: DateTime<Utc>) -> Result<DateTime<Utc>, String>;
```
Both commands follow `med_version_check_impl`'s order: `on_screen_and_writable`; a FRESH `read_stat_orders` over
the opened chart only. Give: the line is a stat order of the opened chart, still in `patient_medication_current`
and `waiting()`, else `STAT_NOT_OPEN`. Strike: the give is on the opened chart and unstruck, else `GIVE_GONE`.
Then the key ONCE, the act, the re-read, and 2b's `checked()` shape for the answer (an error would invite a
second give).

- [ ] **Step 1: Failing tests:**
  - pure: `ceftriaxone_1g_iv_stat_is_a_stat_order`, `stat_then_daily_and_prn_stat_are_not`,
    `an_already_taking_stat_is_not`, `given_time_defaults_to_now`, `a_later_time_is_refused`,
    `an_earlier_time_is_kept`, `a_partly_typed_time_is_refused_in_words`
  - DB: `a_charted_stat_line_signs_stat_true` (both the one-line and the list Sign paths),
    `give_signs_and_the_line_leaves_the_current_list`, `give_on_a_line_stopped_since_is_refused_before_the_key`,
    `give_on_a_linked_charts_line_is_refused`, `strike_returns_the_line_and_answers_waiting_again`,
    `strike_on_a_given_struck_since_is_refused`, `a_failed_stat_read_still_opens_the_list` (`STAT_NOT_READ`)
- [ ] **Step 2: Run** → FAIL (check the filter matched tests). **Step 3: Implement.** **Step 4: Run** → PASS,
  plus `med_list_*`, `med_chart_*` and `med_version_check_*`, unedited.
- [ ] **Step 5: Run every ROOT source guard DB-free** and register what the new files move (commit sites,
  `enroll_device_actor`, `db_errors_stay_legible`, `hex_decode_helper` per-file lists).
- [ ] **Step 6: Commit** `feat(window): give a stat dose and strike a given`.

### Task 6: The pane — the stat line, its buttons, the two rows

**Files:**
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/stat_view.rs` (+ `stat_view_tests.rs`),
  `cairn-gui/cairn-gui-tauri/src-ui/med_stat.js`, `src/med_stat_webview_tests.rs`
- Modify: `row_view.rs` (`MedListRowView.stat: Option<StatView>`), the tab crate's `lib.rs` (exports),
  `src/chart_stat.rs` (`attach_stat` sets `stat` and, for a given row, `status_label = "given"`), `src-ui/main.js`
  (one hook call per row; `"given"` styled as `"ceased"`), `src-ui/index.html` (the script tag), `src-ui/style.css`.

**Interfaces — Produces:**
```rust
pub struct StatView { pub medication_id: String, pub line: String /* Global Constraints words */,
  pub struck: Vec<String>, pub can_give: bool, pub gives: Vec<GiveView> }
pub struct GiveView { pub event_id: String, pub can_strike: bool }
/// The stat line's words from the order and its STANDING gives, in time order (Global Constraints). Pure.
pub fn stat_words(o: &StatWordsIn, now: &DateTime<Local>) -> String;
```
`can_give`: the order is on the opened chart, its row is current, and it is `waiting()`. `can_strike`: an
unstruck give of the opened chart. The buttons are disabled while any Sign or list act is in flight
(`medSigning`). A row signs nothing until its Sign; it closes on success and draws the re-read pane. Failure words
appear beside the line, never only in a status bar. The time field is `datetime-local`, pre-filled with now
(notes_time's `datetime_local`), `max` = now.

- [ ] **Step 1: Failing tests** (webview: assert VISIBILITY, meaning computed display and hidden ancestors,
  never text alone; walk WebKit as well as Chromium for `datetime-local`, per the memory recipe):
  - `stat_view_tests.rs`: `not_yet_given_reads_ordered_then_not_yet_given`, `given_reads_in_time_order`,
    `given_before_charted_reads_before_it_was_charted`, `a_struck_give_reads_who_struck_it_and_why`,
    `a_signer_not_known_is_said`
  - webview: `a_waiting_stat_line_shows_given`, `a_linked_charts_stat_line_shows_no_button`,
    `the_give_row_prefills_now_and_signs`, `a_given_row_reads_given_and_offers_entered_in_error`,
    `the_strike_row_signs_with_its_why`, `a_refusal_is_worded_beside_the_line`
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run** → PASS, plus `med_list_webview_tests`,
  `med_chart_webview_tests`, `med_versions_settle_webview_tests`, `holds_webview_tests`, `row_view_tests`.
- [ ] **Step 5: Commit** `feat(window): the stat line, Given and Entered in error`.

### Task 7: Docs — ADR-0105, spec, durable rules, RUNBOOK, issues

- Create `docs/spec/decisions/0105-a-stat-dose-ordered-then-given.md`: the `stat` key and who sets it; given =
  a stop + a given under one signature (why: every currency reader and an older node follow); the general given
  type and `completes`; entered in error, and why it voids by naming the stop; the display and its time order;
  §10 Q3; what is **not** guaranteed: no MAR; no menu group for an ungiven stat; no allergy re-check at giving; a
  peer's retraction may void any stop of its thread (the remote door is structure only, the R4 class); drafts do
  not cross nodes (#963); the human figures owed. Record the maintainer's Q1–Q4 rulings. **Fact-check it
  sentence by sentence against the SQL and the Rust** before review.
- Modify `docs/spec/index.md` (v0.107, ADR-0105), `docs/spec/data-model.md` §3.3 (one paragraph; medication prose
  lives in §3.3), `docs/spec/decisions/README.md`, `mkdocs.yml` (nav). ADR-0099 is NOT edited (ADRs are
  immutable); ADR-0105 says it ends 0099's limit (f). Also modify `docs/developers/durable-rules.md`
  (a slice block: "a given's stop and the given in ONE transaction, the stop first"; "the void keys on the
  retraction's `voids`, never on the given row"; "`stat` is signed only on a pure one-off Start line"),
  `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §36 + `TEMPLATE.md` (give ≤ 3 s, strike, the before-charted
  wording, VoiceOver and keyboard on both rows), `docs/HANDOVER.md`, `docs/ROADMAP.md`.
- File: an ungiven stat has no "Waiting for you" group (a nurse's worklist); a give re-checks no allergy.
- [ ] **Step 1:** Write. **Step 2:** `uv run --with-requirements docs/requirements.txt -- mkdocs build` clean.
  **Step 3:** `python3 scripts/check_closing_keywords.py` on the message. **Step 4:** Commit
  `docs(medication): ADR-0105, spec v0.107, RUNBOOK §36, durable rules`.

### Task 8: The whole gate

- [ ] After the last edit, in CI's order: fmt; clippy (root; GUI tree `--locked`); `cargo doc` with
  `RUSTDOCFLAGS=-D warnings` (both trees); `scripts/run-db-gated-tests.sh` (detached `nohup`, worktree at the PR
  head, ~2 h); the GUI tree's DB-gated `--bins`; `cargo deny`; the closing-keyword script on the PR body. Sum the
  passed counts; no `skipped:` line.
- [ ] Push; mark the PR ready; `Closes #1069`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** "ceftriaxone 1 g IV stat" written and signed in the stat section of the drug chart; the
  nurse initials it and writes the time given in the stat column; a mistaken entry is struck through, initialled
  and annotated "not given — entered in error".
- **Steps:**
  - Order: paper N = 1 (write and sign) → architecture M = 1 signed act (the line, with `stat`) → UI K = 1 line +
    Sign (slice 1a's gesture, unchanged).
  - Give: paper N = 1 (initial + time) → M = 1 signed act (two events, the stop and the given, under one key
    release) → K = 1 press + Sign, the time pre-filled with now (+ an edit when it was earlier). `M ≤ N`.
  - Entered in error: paper N = 1 (strike + initial + note) → M = 1 signed act → K = 1 press + (optional why) +
    Sign.
  - Reading it: paper N = 0 (it is on the chart) → K = 0. The stat line's words are on the row, with no click.
- **Time + cognitive load:** ≤ 3 s from the press to a given recorded at now (design §6). A waiting stat dose is
  never visually a regular current line ("not yet given" on the row). A given dose never reads as current, and a
  dose given before it was charted is never shown in the wrong order. Measured by RUNBOOK §36 (human), owed by
  this slice.
