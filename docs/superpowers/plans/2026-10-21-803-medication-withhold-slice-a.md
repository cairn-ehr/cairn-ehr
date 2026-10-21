# Medication withhold ("W"), slice A: the record and the CLI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A clinician can withhold a current medication line with a reason and a time span (or "until reviewed"), release it early, and see it as "W" on `medication-list`. A span that ends needs no act; the line reads active again.

**Architecture:** Two new additive, born-sealed clinical event types on the existing `medication_id` thread, each with a structural floor, its own content-addressed projection table, and local-door refusals. Whether a line is withheld is **folded at read time** against `now` by one SQL function, beside (never replacing) the existing active/ceased group status. Both types join the ADR-0049 sign-off commitment.

**Tech Stack:** Rust (`cairn-event`, `cairn-node`, `cairn-medication-view`), PostgreSQL 18 (PL/pgSQL, `db/059`), clap CLI.

**Spec:** `docs/superpowers/specs/2026-10-21-medication-withhold-803-design.md` (approved 2026-10-21). Slice B (the window) is a separate plan.

## Global Constraints

- Event types: `clinical.medication-withhold.asserted` (schema `clinical.medication-withhold/1`) and `clinical.medication-withhold-release.asserted` (schema `clinical.medication-withhold-release/1`). Both `'additive', targets_other_author FALSE`.
- Withhold payload: `medication_id` (uuid string, required), `reason` (non-empty after trim, required), `from` (optional instant), `until` (required: an instant, or the literal string `"reviewed"`). Optional fields are omitted, never `null`.
- Release payload: `medication_id` (required), `releases` (non-empty array of **withhold event ids**, uuid strings), `reason` (optional).
- **Amendment to the design (§4.1):** a release names the withholds it ends by **event id**, not content address. Same guarantee (it ends only holds its author saw), and event ids are what the CLI prints and `cairn-node event <id>` reads.
- **Amendment to the design (§4.2, §5 item 12):** there is **no** `MedicationStatus::Withheld`. `MedicationStatus` stays `{Active, Ceased}`; a row carries `holds: Vec<MedicationHold>` beside it. Reason: `signoff.rs:267`, the window's `view.rs:81` and `row_view.rs:87,93` read `status == Active` as "on the current list / signable / ceasable"; a third variant would silently drop withheld drugs out of sign-off (the design's own anti-goal).
- Instants on the wire: ISO-8601 with an explicit UTC offset, the `cairn_t_effective` pin (db/001:206). An offset-less `from`/`until` is refused.
- Twins: withhold `Withheld medication from <from|"now"> until <until|"reviewed"> — <reason>`; release `Released withheld medication (<n> hold(s))` plus ` — <reason>` when given. **No drug name** (cessation precedent).
- `SCHEMA_GENERATION` 58 → 59; new file `db/059_medication_withhold.sql`.
- The clinical word "withheld" means the hold only. Clinician-facing "withheld" meaning *hidden* becomes "not shown" / "left out of sign-off" / "hidden" (design §4.6). Internal identifiers are not renamed.
- Every new commit routes through `commit_failure`; every new `LocalDbFault`-wrapped read bumps `db_errors_stay_legible.rs`; new `pub fn` in `tests/common/mod.rs` must be added to `identity_scaffolding_shared.rs`'s list (memory notes).
- No new dependency.

## Review Focus

1. **`until` exactly equal to `now`** — the hold has ended (`until_at > now` is strict). Pinned in Task 4.
2. **Mixed offsets** — `until` given as `+10:00` and `now` in UTC compare as instants. Pinned in Task 2 (floor accepts) and Task 4 (fold).
3. **A line whose group holds a ceased member and an active member** reads active, so a withhold on it is admitted. Pinned in Task 3.
4. **Two holds in force, a release names one** — the line stays withheld by the other, and both the shown hold and the count are right. Pinned in Task 4.
5. **A release arriving by sync before the withhold it names** — admitted, and the hold is ended once it arrives. Pinned in Task 4.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing W (with initials) in the drug chart's administration slot(s), the reason as the slot code or a short note; resuming = giving the next dose; rescinding = a note.
- **Steps:** withhold: paper **N = 2** (write W + initials; the reason) → architecture-forced **M = 2** (name the line, give the reason; "until" is part of the same act; authorship from possession, ADR-0053) → UI bundling target **K = 2** (slice B). Resume: paper 0, Cairn 0 (expiry is a read-time fold, no event). Rescind: paper 1, Cairn 1 (release). `M ≤ N` throughout.
- **Time + cognitive load:** budget ≤ the paper W, about 10 s from chart open to recorded; one line on the list names the hold, its end and its reason. This slice exposes only the operator CLI; the measurement is owed by slice B's RUNBOOK section.

---

### Task 1: ADR-0081 and the spec

**Files:**
- Create: `docs/spec/decisions/0081-medication-withhold-is-a-time-bounded-hold.md`
- Modify: `docs/spec/decisions/README.md` (index row), `mkdocs.yml` (nav line), `docs/spec/index.md:9` (`0.82` → `0.83`), `docs/spec/data-model.md` §3.3 (one paragraph after the ADR-0059 paragraph), the design doc (the two amendments above, one line each in §4.1 and §4.2)

- [ ] **Step 1:** Write ADR-0081 in the house shape (Status, Date 2026-10-21, Derives from: principle 3 §1.2 and principle 4; Applies: ADR-0012, ADR-0043, ADR-0047, ADR-0049, ADR-0052, ADR-0053). Decisions, one each: (1) the two event types and payloads as in Global Constraints; (2) a hold is a time span, `until` required, `"reviewed"` the explicit open end, exact instants only and why; (3) expiry is a read-time fold, no event, default resume; (4) a release names withholds by event id, ends them from its effective time, cancels a not-yet-started one, is a no-op on an ended one; (5) cessation wins over any hold; (6) the hold is beside the active/ceased status, a withheld line stays current and signable; (7) sign-off: both types join the commitment, a W after a sign-off stales it, expiry does not; signing upholds; (8) local-door refusals only, sync apply admits; (9) "withheld" is the clinical word, "not shown" for hidden. Consequences: a generation-58 node shows a withheld drug as active (pre-clinical, acceptable); a node with a wrong clock expires holds early or late (the list prints `until`); no MAR — the ward-level list and dose slots are later work.
- [ ] **Step 2:** Fact-check every ADR sentence against this plan's Global Constraints (memory: ADR-0077 passed two reviews with a false sentence). Add the README row, the `mkdocs.yml` nav line, the version bump and the §3.3 paragraph (two sentences pointing at ADR-0081).
- [ ] **Step 3:** Run `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`. Expected: build succeeds, no warning naming `0081`.
- [ ] **Step 4:** Commit `docs(#803): ADR-0081 — a medication withhold is a time-bounded hold`.

### Task 2: Pure payload builders and twins (`cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/medication/withhold.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs` (module + re-exports, module doc gains the two verbs)

**Interfaces:**
- Produces:
  - `pub enum HoldUntil<'a> { At(&'a str), Reviewed }`
  - `pub struct MedicationWithhold<'a> { pub medication_id: &'a str, pub reason: &'a str, pub from: Option<&'a str>, pub until: HoldUntil<'a> }`
  - `pub fn medication_withhold_body(w: &MedicationWithhold) -> serde_json::Value`
  - `pub fn render_medication_withhold_twin(w: &MedicationWithhold) -> String`
  - `pub struct MedicationWithholdRelease<'a> { pub medication_id: &'a str, pub releases: &'a [String], pub reason: Option<&'a str> }`
  - `pub fn medication_withhold_release_body(r: &MedicationWithholdRelease) -> serde_json::Value`
  - `pub fn render_medication_withhold_release_twin(r: &MedicationWithholdRelease) -> String`

- [ ] **Step 1: Write the failing tests** in `withhold.rs`'s `mod tests`:
  - `withhold_body_carries_until_reviewed_and_omits_from`: `until: Reviewed, from: None` → `v["until"] == "reviewed"`, no `from` key, `v["reason"] == "BP 85/50"`.
  - `withhold_body_carries_an_until_instant_and_a_from`: `from: Some("2026-10-21T20:00:00+10:00")`, `until: At("2026-10-22T08:00:00+10:00")` → both strings verbatim.
  - `withhold_twin_names_span_and_reason_not_the_drug`: twin for `from: None, until: Reviewed, reason: "discuss with GP"` == `"Withheld medication from now until reviewed — discuss with GP"`.
  - `release_body_lists_event_ids_and_omits_absent_reason`: two ids → `v["releases"]` is that array; no `reason` key.
  - `release_twin_counts_holds`: two ids, reason `"BP recovered"` → `"Released withheld medication (2 holds) — BP recovered"`; one id, no reason → `"Released withheld medication (1 hold)"`.
- [ ] **Step 2:** Run `cargo test -p cairn-event medication::withhold`. Expected: FAIL to compile (types missing).
- [ ] **Step 3:** Implement the types and four functions; follow `cessation.rs`'s insert-only-when-present idiom.
- [ ] **Step 4:** Run `cargo test -p cairn-event medication::withhold`. Expected: 5 passed.
- [ ] **Step 5:** Commit `feat(#803): withhold and release payload builders`.

### Task 3: `db/059` — types, floor, projections, local-door refusals; the node orchestrators

**Files:**
- Create: `db/059_medication_withhold.sql`, `crates/cairn-node/src/medication/withhold.rs`, `crates/cairn-node/tests/medication_withhold.rs`
- Modify: `crates/cairn-node/src/db.rs` (~:377, list the file), `crates/cairn-event/src/schema_generation.rs:45` (59, doc line naming db/059), `crates/cairn-node/src/medication/mod.rs` (module + re-exports), `crates/cairn-node/tests/twin_registry.rs` (both types in the exhaustive list), `db/tests/034_twin_registry_test.sql` (24 → 26), `db/tests/039_projection_registry_test.sql` (29 → 31), `crates/cairn-node/tests/projection_registry.rs:63` (27 → 29 — confirm each file's current figure first; they differ)

**Interfaces:**
- Consumes: Task 2's builders.
- Produces (SQL):
  - tables `medication_withhold(content_address BYTEA PRIMARY KEY, event_id UUID NOT NULL UNIQUE, medication_id UUID NOT NULL, patient_id UUID NOT NULL, reason TEXT NOT NULL, starts_at TIMESTAMPTZ NOT NULL, until_at TIMESTAMPTZ, until_reviewed BOOLEAN NOT NULL, withholder_kid TEXT NOT NULL, hlc_wall BIGINT NOT NULL, hlc_counter INTEGER NOT NULL, origin TEXT NOT NULL, CHECK (until_reviewed = (until_at IS NULL)))` and `medication_withhold_release(content_address BYTEA PRIMARY KEY, event_id UUID NOT NULL UNIQUE, medication_id UUID NOT NULL, patient_id UUID NOT NULL, released_event_ids UUID[] NOT NULL, released_at TIMESTAMPTZ NOT NULL, reason TEXT, hlc_wall …, hlc_counter …, origin …)`, index on `medication_id` for both, `GRANT SELECT … TO cairn_agent`.
  - `cairn_check_medication_withhold(p_type text, b jsonb) RETURNS void` (both types; `REVOKE EXECUTE … FROM PUBLIC`).
  - apply fns `medication_withhold_apply(e event_log)`, `medication_withhold_release_apply(e event_log)`, registered in `cairn_projection_apply` with `projection_tables` `{medication_withhold, medication_patient_conflict_flag}` / `{medication_withhold_release, medication_patient_conflict_flag}`, `run_order` 10, `heal_safe` TRUE.
- Produces (Rust, `cairn_node::medication`):
  - `pub struct WithholdMedicationInput<'a> { pub reason: &'a str, pub from: Option<&'a str>, pub until: cairn_event::medication::HoldUntil<'a> }`
  - `pub async fn withhold_medication(client, node_sk, node_kid, node_origin, patient: Uuid, medication_id: Uuid, input: &WithholdMedicationInput<'_>, author: Option<&AuthorParams<'_>>) -> anyhow::Result<Uuid>` (returns the withhold event id)
  - `pub async fn release_medication_holds(client, node_sk, node_kid, node_origin, patient: Uuid, medication_id: Uuid, withholds: &[Uuid], reason: Option<&str>, author: Option<&AuthorParams<'_>>) -> anyhow::Result<Uuid>` (returns the release event id)

Semantics the SQL must implement (apply fns):
- Both read `cairn_clear_payload(e)` (NULL → return) and call `cairn_guard_medication_patient` first. Insert with `ON CONFLICT (content_address) DO NOTHING` — every event is its own row; there is no overlay.
- `starts_at = COALESCE(<payload from>, e.t_effective, to_timestamp(e.hlc_wall / 1000.0))`; `released_at = COALESCE(e.t_effective, to_timestamp(e.hlc_wall / 1000.0))`; `withholder_kid = e.signer_key_id` (the human's key when authored, ADR-0053).
- Floor (both doors): reason non-empty after `btrim`; `until` is `"reviewed"` or a valid instant; `from`, if present, a valid instant; `until > from` when both are instants; `releases` a non-empty array of valid uuid strings. Instants are parsed by a helper `cairn_medication_hold_instant(p_raw text, p_field text) RETURNS timestamptz` that calls `cairn_t_effective` and re-raises naming the field (`'medication withhold: until must be …'`).
- Local door only (`current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on'`): refuse a withhold whose thread's group has `medication_group_status.status = 'ceased'` (`'medication withhold: this line is ceased — nothing to withhold'`); refuse `until_at <= starts_at` after `from` is resolved; refuse a release naming an event id that is not a row in `medication_withhold` with the same `medication_id` (`'medication release: % is not a withhold on this line'`).
- Redefine `cairn_medication_thread_patient` in db/059 with a fourth `COALESCE` arm, `(SELECT patient_id FROM medication_withhold WHERE medication_id = p_med LIMIT 1)`, after the coding arm (an orphan withhold claims its chart, #192).

- [ ] **Step 1: Write the failing DB tests** in `tests/medication_withhold.rs` (scaffold: copy `cs`, `db_msg`, `setup_node`, `sample_input` from `tests/medication.rs`; `setup_node`'s truncate list gains both new tables; `common::submit_registration` before any write):
  - `a_withhold_is_recorded_with_its_resolved_span`: assert a medication, withhold with `from: None, until: Reviewed`, then one `medication_withhold` row: `until_reviewed = true`, `until_at IS NULL`, `reason = 'BP 85/50'`, `starts_at` within 1 minute of `now()`.
  - `an_empty_reason_is_refused_by_the_floor`, `an_offset_less_until_is_refused`, `until_not_after_from_is_refused`, `a_missing_until_is_refused` (build the body by hand through `build_withhold_body` with `until` removed): each `db_msg` contains `medication withhold:`.
  - `a_withhold_on_a_ceased_line_is_refused_at_the_local_door`.
  - `a_line_with_one_ceased_member_still_admits_a_withhold` (Review Focus 3): two asserts of one drug, reconcile them (`medication-reconcile` orchestrator), cease the older member with an earlier `stopped` than the other's start → group status `active` → withhold admitted.
  - `a_release_naming_an_unknown_event_is_refused`, `a_release_naming_another_lines_withhold_is_refused`.
  - `a_sync_applied_withhold_on_a_ceased_line_is_admitted`: inject through `apply_remote_event` (the `inject_assert` pattern in tests/medication.rs) → row present.
- [ ] **Step 2:** Run `cargo test -p cairn-node --test medication_withhold -- --nocapture` with `CAIRN_TEST_PG` set. Expected: compile failure, then (after stubs) failures. Confirm no `skipped:` line (a self-skip is not a pass).
- [ ] **Step 3:** Write `db/059_medication_withhold.sql` (header comment in db/031's style: what, why, ADR-0081), list it in `db.rs`, bump `SCHEMA_GENERATION`, write `crates/cairn-node/src/medication/withhold.rs` (`build_withhold_body`, `build_release_body` mirroring `build_cease_body`; orchestrators through `seal_sign_submit`; both return their event id). Bump the four pinned counts.
- [ ] **Step 4:** Run the new test binary, then `twin_registry`, `projection_registry`, `db_errors_stay_legible`, and `scripts/run-db-gated-tests.sh`'s db/tests SQL mirrors for 034 and 039. Expected: all pass.
- [ ] **Step 5:** Commit `feat(#803): db/059 — withhold and release events, floor, projections`.

### Task 4: The read-time fold

**Files:**
- Modify: `db/059_medication_withhold.sql`, `crates/cairn-node/tests/medication_withhold.rs`

**Interfaces:**
- Produces: `cairn_medication_group_holds(p_groups uuid[], p_now timestamptz) RETURNS TABLE (group_id uuid, medication_id uuid, withhold_event_id uuid, reason text, starts_at timestamptz, until_at timestamptz, until_reviewed boolean, withholder_kid text, pending boolean)`, `LANGUAGE sql STABLE`, `GRANT EXECUTE … TO cairn_agent`.

The rule (design §4.2 with the builder-check correction): a withhold on a member thread of one of `p_groups` (via `medication_thread_group`) is **returned** when `(until_reviewed OR until_at > p_now)` AND no `medication_withhold_release` row with the same `medication_id`, `withhold_event_id = ANY(released_event_ids)` and `released_at <= p_now` exists. `pending = starts_at > p_now`. A hold is **in force** when returned and not pending. Order: `group_id`, then `starts_at DESC`, then `withhold_event_id`.

- [ ] **Step 1: Write the failing tests** (each calls the function with a fixed `p_now`, never sleeps):
  - `a_hold_is_in_force_inside_its_span_and_gone_after` — `until = T`; at `T − 1 min`: one row, `pending = false`; at `T`: no row (Review Focus 1); at `T + 1 min`: no row.
  - `until_in_another_offset_compares_as_an_instant` (Review Focus 2): `until = 2026-10-22T08:00:00+10:00`; `p_now = 2026-10-21T21:59:00Z` → row; `22:00:00Z` → none.
  - `an_advance_hold_is_pending_until_it_starts`.
  - `a_release_cancels_a_pending_hold` (an advance W released before its `from` never returns a row afterwards).
  - `a_release_of_an_ended_hold_is_a_no_op`.
  - `releasing_one_of_two_holds_leaves_the_other` (Review Focus 4).
  - `until_reviewed_holds_until_released`.
  - `a_hold_on_any_member_thread_shows_on_the_group`: reconciled pair, W on the non-representative member → returned under the group id.
  - `a_separation_moves_nothing`: after separating the pair, the hold is returned under its own thread's group only.
  - `a_release_synced_before_its_withhold_ends_it_on_arrival` (Review Focus 5): apply the release by `apply_remote_event`, then the withhold → no row at a `p_now` after the release.
- [ ] **Step 2:** Run the test binary. Expected: FAIL (function missing).
- [ ] **Step 3:** Add the function to db/059.
- [ ] **Step 4:** Run. Expected: all pass. Mutation check: flip `<=` to `<` on `released_at` and `>` to `>=` on `until_at` one at a time; the matching test must go red; revert.
- [ ] **Step 5:** Commit `feat(#803): the read-time withhold fold`.

### Task 5: Sign-off commitment, shred, thread resolution, inventories

**Files:**
- Modify: `db/034_medication_attestation.sql` (the commitment `IN` list ~:204 and readable-count list ~:237 gain both types; the partial index is renamed `event_log_medication_thread_idx2` with all six types — edit in place, `IF NOT EXISTS` keeps the replay idempotent), `db/059_medication_withhold.sql` (`DROP INDEX IF EXISTS event_log_medication_thread_idx;`), `crates/cairn-node/tests/medication_attestation.rs:1007` (hand copy of the list), `db/037_born_sealed.sql` (~:184–236, two `DELETE FROM medication_withhold[_release] WHERE content_address = v_ca` arms beside cessation's, comment naming db/059), `db/048_sensitivity_stream.sql` (~:573, two `UNION ALL` arms), `crates/cairn-node/tests/thread_resolution_cost.rs:58` (`[&str; 6]` → `[&str; 8]`), `crates/cairn-node/src/safety.rs` (~:564 test table), `crates/cairn-node/tests/common/mod.rs` (~:809), `crates/cairn-node/tests/common/restore_kit.rs` (~:374) truncation lists

- [ ] **Step 1: Write the failing tests** in `tests/medication_withhold.rs`:
  - `a_withhold_after_sign_off_makes_the_line_stale`: assert, attest (sign-off orchestrator), withhold → `medication_thread_attestation.stale = true`.
  - `expiry_does_not_stale_a_sign_off`: withhold until `T`, then attest, then read staleness through a commitment computed at a later `p_now` — still `stale = false` (expiry writes no event).
  - `a_thread_without_holds_keeps_its_commitment`: compute `cairn_medication_thread_commitment` for a plain thread before and after db/059 replay (`db::force_schema_replay`) — equal.
  - `shredding_a_withhold_removes_its_row` (the shred CLI orchestrator pattern from `tests/shred_cli.rs`).
  - `cairn_event_thread_resolves_a_withhold_and_a_release` (content address → thread id).
- [ ] **Step 2:** Run. Expected: FAIL.
- [ ] **Step 3:** Make the edits listed above. (`attestation.rs::thread_commitment_on` calls the two SQL functions and holds no type list of its own, so it needs no edit — the builder check's "fourth list" is only the test's hand copy.)
- [ ] **Step 4:** Run this binary, `medication_attestation`, `thread_resolution_cost`, `safety` unit tests, `shred_cli`. Expected: pass.
- [ ] **Step 5:** Commit `feat(#803): withholds join sign-off currency, shred and thread resolution`.

### Task 6: The read model carries holds and a cessation's reason

**Files:**
- Modify: `crates/cairn-medication-view/src/row.rs` (new type + two fields + a method), `crates/cairn-medication-view/src/lib.rs` (re-export), `crates/cairn-node/src/medication/read.rs` (two reads), every `MedicationRow { … }` literal: `cairn-medication-view/src/{fixtures,targeting,row}.rs`, `cairn-node/src/medication/{list_text,signoff}.rs`, `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/test_rows.rs`, `cairn-gui/cairn-gui-tauri/src/chart_set/tests.rs` (`plan_row`)
- Test: `crates/cairn-node/tests/medication_read.rs`, `row.rs` unit tests

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, PartialEq, Eq, Serialize)] pub struct MedicationHold { pub withhold_event_id: Uuid, pub medication_id: Uuid, pub reason: String, pub starts_at: String, pub until: Option<String>, pub withholder: String, pub pending: bool }` (`until: None` = until reviewed; instants as RFC 3339 text from the DB, `to_char(… AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI"Z"')`).
  - `MedicationRow` gains `pub holds: Vec<MedicationHold>` and `pub cessation_reason: Option<String>`.
  - `impl MedicationRow { pub fn is_withheld(&self) -> bool }` — any hold with `pending == false`.
  - `read.rs`: `async fn read_group_holds(client, groups: &[Uuid]) -> anyhow::Result<HashMap<Uuid, Vec<MedicationHold>>>` calling `cairn_medication_group_holds($1, clock_timestamp())`; `async fn read_cessation_reasons(client, groups: &[Uuid]) -> anyhow::Result<HashMap<Uuid, String>>` from `medication_group_cessation` (non-empty reasons only).

- [ ] **Step 1: Write the failing tests:** unit `is_withheld_ignores_pending_holds` in `row.rs`; DB tests in `medication_read.rs`: `a_withheld_line_stays_current_and_signable` (status `Active`, `is_withheld()`, and `sign_off_targets` still contains its thread); `a_ceased_line_carries_its_reason`; `a_pending_hold_is_listed_but_the_line_is_not_withheld`.
- [ ] **Step 2:** Run. Expected: compile failure.
- [ ] **Step 3:** Implement. Literal sites get `holds: vec![], cessation_reason: None`.
- [ ] **Step 4:** Run `cargo test -p cairn-medication-view`, the `medication_read` binary, and the cairn-gui tree's tests (`cd cairn-gui && cargo test --workspace` and `cargo clippy --all-targets -- -D warnings`). Expected: pass, no lockfile change.
- [ ] **Step 5:** Commit `feat(#803): rows carry holds and a cessation's reason`.

### Task 7: CLI verbs, list rendering, and the wording

**Files:**
- Create: `crates/cairn-node/src/medication/hold_text.rs` (pure line builders; `list_text.rs` is at 493 lines)
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` variants beside `MedicationCease` ~:2096, handlers beside ~:4933, the list loop ~:5334), `crates/cairn-node/src/medication/mod.rs`, `crates/cairn-node/src/medication/list_text.rs:182`, `crates/cairn-node/src/patient/search_person.rs:134`, `crates/cairn-node/src/main.rs:4547`, `crates/cairn-node/src/sensitivity/render.rs:384`, and any test asserting those strings

**Interfaces:**
- Produces:
  - `pub fn hold_line(h: &MedicationHold) -> String` → `"W withheld until 2026-10-22T08:00Z: BP 85/50 (by 1a2b3c4d)"`, `"W withheld until reviewed: discuss with GP (by …)"`, pending: `"W from 2026-10-21T20:00Z until …: … (by …)"`. The `by` value is `short_kid(withholder)`.
  - `pub fn ceased_reason_line(reason: &str) -> String` → `"ceased: <reason>"`.
  - `pub fn withheld_line(event_id: Uuid, medication_id: Uuid) -> String` → `"withheld medication thread <mid>; event <eid>"`; `pub fn released_line(event_id: Uuid, medication_id: Uuid, n: usize) -> String` → `"released <n> hold(s) on medication thread <mid>; event <eid>"` (pure builders per the #828 lesson: two `Uuid`s on one line).
  - CLI: `medication-withhold <patient> <medication_id> --reason <text> --until <instant|reviewed> [--from <instant>]` + `AuthorFlags`; `medication-release <patient> <medication_id> --withhold <event-id>… [--reason <text>]` + `AuthorFlags`. `--until` parses `"reviewed"` to `HoldUntil::Reviewed`.
  - `medication-list` prints each row's hold lines under the row, and a ceased row's reason line; `--json` carries `holds` and `cessation_reason` through serde.
  - Rewording: `"…are withheld from sign-off"` → `"…are left out of sign-off"`; `"(name withheld)"` → `"(name not shown)"`; `"nothing is withheld"` → `"nothing is hidden"` (both sites).

- [ ] **Step 1: Write the failing unit tests** in `hold_text.rs`: one per output form above, plus `withheld_line_puts_each_id_in_its_slot` (distinct ids, swap check). Update the existing tests that assert the three old strings to the new ones.
- [ ] **Step 2:** Run `cargo test -p cairn-node --lib medication::hold_text` and the updated tests. Expected: FAIL.
- [ ] **Step 3:** Implement; grep `docs/`, `scripts/` and RUNBOOKs for each reworded string and for any parse of `medication-list` output before changing it (the #828 lesson); update what you find.
- [ ] **Step 4:** Run the unit tests; then end to end against a scratch DB: `medication-assert`, `medication-withhold --until reviewed`, `medication-list` shows the W line, `medication-release --withhold <id>`, `medication-list` shows no W line. Paste the output in the PR.
- [ ] **Step 5:** Commit `feat(#803): medication-withhold, medication-release; W lines and ceased reasons on medication-list; "withheld" now means held`.

### Task 8: Docs, follow-ups, full gate

**Files:**
- Modify: `docs/developers/durable-rules.md` (a #803 "do not undo" block: holds beside status, never a third status value; expiry writes no event; a release names event ids; db/034's index was renamed, never edit its text expecting the index to change; the four pinned counts), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1:** File the follow-ups (`gh issue create`): slice B (the window: W display, click/focus reveal of hold and cessation reasons, Withhold/Release gestures, gesture-timing CHECK, RUNBOOK section); the ward-level "new withholds" list (needs §5.11 reader identity). Run `scripts/check_closing_keywords.py` on the PR body before committing.
- [ ] **Step 2:** Run every CI gate after the final edit, in CI's order: `cargo fmt --all --check` (both trees), `cargo clippy --workspace --all-targets -- -D warnings` (both trees), `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps` (both trees), `scripts/run-db-gated-tests.sh` (in the background; ~2 h), `cargo deny check`. Expected: all green; a killed binary (exit 101, no `FAILED` line) is a rerun, not a pass.
- [ ] **Step 3:** Update HANDOVER and ROADMAP (prune; keep every open issue number), commit, push, mark the PR ready.
