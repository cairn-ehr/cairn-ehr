# GUI sign-in, slice 3c-3a — supervision, `needs_cosigning`, and "Co-sign as reviewed" — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Who supervises whom, and who must have every signed note co-signed, is recorded as signed, append-only
registry rows. "— not co-signed" then follows that record **as it stood when the part was signed** (students
always). A mandatory co-sign is cleared only by one of the writer's supervisors **as of the co-sign**. A writer
with no supervisor recorded falls back to today's rule and reads "(no supervisor recorded)". A supervisor may
"Co-sign as reviewed" a supervisee's note that needed no co-sign. Assigners get a panel to manage this,
supervisors a "My supervisees" panel, and the owner console verbs.

**Architecture:** The record is **six new no-key ops on `actor_event`** (db/004), the table where
`grant-enroller` already lives, plus one new column, `related_actor` (the supervisor of a link). That is the
builder's choice design §3.1 leaves open. It is carried by the registry's existing backup slot and restore door
(db/052), which a separate table would have had to duplicate (`vault_*` tables are in **no** backup, ADR-0083).
It stays node-local, as the registry does. The new doors live in **db/072** (`SCHEMA_GENERATION` 72): one
approver rule, one address + prior, one window act door, one console door, and two readers. The note read loads
the whole record once, in the note snapshot (`cairn_supervision_record()`), and **annotates** each part and vouch
with four plain facts (`note::supervision::annotate`). Every co-sign rule (`note::cosign`) is then pure over
`NoteRow`. A fixture whose facts stay at their defaults gets exactly today's rule. No wire change: no event type,
twin, floor or projection changes.

**Tech Stack:** PostgreSQL 18 (db/004, db/052 in place; db/072 new), Rust (`cairn-node`, `cairn-gui-tauri`), plain
JS (no npm).

**Spec:** [`docs/superpowers/specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md`](../specs/2026-11-06-sign-in-slice-3c-3-supervision-and-pending-signatures-design.md)
§3.1–§3.3 and §5's 3c-3a row (maintainer-approved 2026-11-06/07). Read with
[ADR-0092](../../spec/decisions/0092-the-vouch-and-the-sign-off.md) decisions 1, 11, 12 (this slice supersedes
parts of each), [ADR-0093](../../spec/decisions/0093-a-repudiated-signature-counts-as-unsigned.md),
[ADR-0085](../../spec/decisions/0085-in-person-enrolment-and-re-key.md) (the grant pattern). Before Task 1 read the
sign-in 3c-3b, 3c-2 and slice-2 blocks of `docs/developers/durable-rules.md`.

**Maintainer, 2026-11-07 (plan review):** the record is ops on `actor_event` (as planned); the wider self-dealing
rule below is accepted; and, closing 3c-3b's open question, **a re-sign under an UNCONFIRMED key counts**, read
"(signer unconfirmed)" as built (ADR-0094 states it; ADR-0085: unconfirmed never blocks signing). Executed by
subagent-driven development.

## Global Constraints

- **The six ops, exact words**, each a row with NO key and `approved_by NOT NULL`:
  `link-supervisor` / `unlink-supervisor` (on the SUPERVISEE's `actor_id`, `related_actor` = the supervisor),
  `require-cosign` / `lift-cosign` (on the person's `actor_id`, `related_actor` NULL),
  `grant-assigner` / `withdraw-assigner` (on the holder's `actor_id`, `related_actor` NULL). Every other op
  carries `related_actor IS NULL`.
- **"As of t"** = the latest row of the op's family for that actor (and, for a link, that `related_actor`) whose
  `recorded_at <= t`, latest by `(recorded_at, seq)`, as `actor_current` orders. **A part's t** is its
  `event_log.hlc_wall` (ms since the epoch), never `t_effective`. **A vouch's t** is its own `hlc_wall`.
- **A student needs co-signing by role** (pinned per actor, ADR-0092 decision 10): nothing is stored for it,
  `require-cosign` and `lift-cosign` on a student are refused.
- **Who may act** (db/072's `cairn_supervision_act`; the console door may do every op, approved by `'console'`).
  The approver must be a vault member whose key is current; anyone but a superuser also needs a key that is
  `confirmed` or `outside` (as `cairn_vault_enroller_problem` asks). An **assigner** is a member whose latest
  assigner row is a grant.

  | op | approved by | refused when |
  |---|---|---|
  | `link-supervisor` | superuser, assigner | supervisee = supervisor; either party a `superuser` or `admin`; the supervisor a `student`; approver = supervisee; the link already stands |
  | `unlink-supervisor` | superuser, assigner, the supervisor themself | approver = supervisee; no link stands |
  | `require-cosign` | superuser, assigner | a student; a `superuser`/`admin`; approver = the person; already required |
  | `lift-cosign` | superuser, assigner, a current supervisor of the person | a student; approver = the person; not required |
  | `grant-assigner` / `withdraw-assigner` | superuser only | a `superuser` holder; a no-op |

  *My call, wider than design §3.1's "no self-dealing":* **nobody acts on their own supervision as the
  supervisee.** Otherwise a supervisee holding the right could unlink their supervisors and fall into the
  no-supervisor fallback, where any clinician clears the co-sign. A supervisor stepping away is the one act on a
  link its own party may make.
- **Address, exact preimage** (pure, IMMUTABLE, as `cairn_vault_grant_address`):
  `'\x1220' || sha256('cairn-supervision' \n op \n staff(subject) \n COALESCE(staff(supervisor), '') \n prior)`.
  Here `staff()` is `cairn_vault_staff_id` and `prior` is the `seq` of the family's latest row for that
  subject (+ related), or 0.
- **Copy, exact:**
  - The pending mark stays `" — not co-signed"`. When the writer has NO supervisor now, it reads
    `" — not co-signed (no supervisor recorded)"`.
  - The button: `"Co-sign as reviewed"`; accessible name `"Co-sign as reviewed: " + when`.
  - Review outcome: `"Your review signature was recorded — the chart shows which parts it covers."` (role-neutral: an attested review reads " · signed off by",
    controller ruling after the final review); refusals
    `"This note changed since it was shown — read the chart again, then co-sign it. Nothing was signed."`
    and `"There is nothing here for you to co-sign as reviewed. Nothing was signed."`.
  - The vouch it files reads as any co-sign does (`" · co-signed by …"`): it IS a `sign-off`/`co-signed` vouch
    (or `attested`, where the floor admits only that, ADR-0092 decision 11). Design §3.3.
- **No dialog anywhere** (principle 3). Every note act still checks `signInFlight()` (ADR-0092 decision 17).
- **One-way export:** an `ActorRegistryRow` carrying `related_actor` does not decode on an older build
  (`deny_unknown_fields`). The field is `#[serde(default, skip_serializing_if = "Option::is_none")]`, so an export
  with no link rows is byte-identical to today's (`localstate_wire_pins.rs` must stay green, unchanged).
- House rules: TDD; inline docs for a junior; files under 500 lines (new modules below; `vouch.rs` is 419, so new
  rules go in `cosign.rs` and the gesture in `review.rs`); test key material minted at runtime; no `salt`/`nonce`/`iv`
  names. **Pure-test command:** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib note::`. If an IDE holds
  `target/`, prefix `CARGO_TARGET_DIR=/tmp/cairn-3c3a`. DB suites: `scripts/run-db-gated-tests.sh` or a named
  `--test`, with `--nocapture` and no `skipped:` line (memory: a self-skip reads "ok").
- **Pinned-count guards this slice moves:** `SCHEMA_GENERATION` 71 → 72 (`crates/cairn-event/src/schema_generation.rs`);
  `crates/cairn-node/src/db.rs`'s `SCHEMA` list gains `072_supervision`; db/052's op list; any new `.commit()` site in
  `db_errors_stay_legible.rs`; a new `.js` joins `commands_tests.rs`'s script list AND `waiting_webview_tests.rs`'s
  `SCRIPTS` (count +1); a new `pub fn` in `tests/common/mod.rs` joins `identity_scaffolding_shared.rs`'s list.

## Review Focus

1. **A note signed under mandatory supervision stays "— not co-signed" after the requirement is lifted** (the
   anti-goal). `needs_cosigning` is read at the PART's `hlc_wall`, not now. Pinned in Task 4 (pure) and Task 5
   (through the database: require → sign → lift → the part still waits).
2. **A co-sign by someone who is not (or not yet) the writer's supervisor does not clear a mandatory co-sign**
   while the writer has a supervisor. That includes a supervisor linked only AFTER the co-sign: "supervisor" is
   judged at the vouch's own `hlc_wall`. And a supervisor who has since stepped away still counts. Pinned in
   Task 4.
3. **A student's part signed before this slice is still clearable as today** (no supervisor recorded then ⇒
   fallback: any non-student's `co-signed` clears). It now reads "(no supervisor recorded)" until co-signed.
   Pinned in Task 5, where every existing `view_tests.rs` student test keeps passing with only that copy change.
4. **Self-dealing through the act door**: a supervisee who holds the assigner right cannot unlink their own
   supervisor, lift their own requirement, or link themselves to themselves, even with a valid token. An
   unconfirmed key cannot approve anything. Pinned in Task 2's door matrix.
5. **"Co-sign as reviewed" never vouches another writer's part**, a part needing a sign-off, or a part the presser
   does not supervise, even when the presser is named in who's here but signs with another person's key (the
   key, not the claim, decides; ADR-0092 decision 16). Pinned in Task 6 with key ≠ claim.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the supervisor's countersignature beneath an entry in the bedside folder; supervision
  itself = a list or a note in a staff file; a probationer's entries are countersigned until the supervisors end
  the probation.
- **Steps** (paper *N* → architecture-forced *M* → UI target *K*):
  - Record "B supervises C": 1 (write it on the list) → 1 act + the strip's passphrase (slice 1 counts these as
    one) → 1.
  - Set or lift `needs_cosigning`: 1 → 1 → 1.
  - Co-sign a reviewed note, from the opened chart: 2 (open the folder, sign) → 2 ("Co-sign as reviewed",
    passphrase) → 2. Finding it is 3c-3c's menu; until then chart by chart, as ADR-0092 (d) states.
  - Seeing that a part still needs its supervisor: 1 (look for the countersignature) → 0 (the byline says it) → 0.
- **Time + cognitive load:** a supervision act ≤ 3 s; co-sign as reviewed ≤ 3 s (Sign's budget, RUNBOOK
  §16/§23). The co-sign is timed as the existing `note-sign-off` kind (no db/044 widening). The byline names what
  is pending and why; no decision is forced. A human owes the measurement in **RUNBOOK §25** (Task 9).

---

### Task 1: The six ops on the registry, carried by backup and restore

**Files:**
- Modify: `db/004_actors.sql` (header algebra comment; `related_actor` column; a SECOND guarded `DO` block that
  widens `actor_event_op_check` and `actor_event_op_shape`; `cairn_actor_append`; its REVOKE line 453)
- Modify: `db/052_restore_doors.sql` (`restore_actor_registry`: op list L179-183, a `v_related` variable, decode,
  INSERT column, a compare `WHEN` arm)
- Modify: `crates/cairn-node/src/localstate.rs` (`ActorRegistryRow.related_actor`, `actor_registry_rows_to_json`),
  `crates/cairn-node/src/localstate_read.rs` (the registry query L131-159)
- Test: `db/tests/004_actors_test.sql`; create `crates/cairn-node/tests/restore_actor_registry_supervision.rs`
  (model: `restore_actor_registry_new_ops.rs`)

**Interfaces:**
- Produces: `actor_event.related_actor BYTEA` (additive); `cairn_actor_append(p_actor_id BYTEA, p_op TEXT,
  p_key TEXT, p_approved_by TEXT, p_approval BYTEA, p_related BYTEA DEFAULT NULL)`. The 5-argument form is
  DROPPED first (`DROP FUNCTION IF EXISTS … (bytea, text, text, text, bytea)`), so existing 5-argument calls
  resolve to the default. `ActorRegistryRow.related_actor: Option<Vec<u8>>`, travelling as hex.
- `cairn_actor_append` accepts the six new ops. For `link-supervisor`/`unlink-supervisor`, `p_related` must
  be an enrolled `human` actor other than `p_actor_id`. The existing clock-behind refusal applies to every op.

- [ ] **Step 1: Write the failing tests.**
  - db/tests/004: one row per new op through `cairn_actor_append`. A `link-supervisor` with NULL `p_related`,
    one with `p_related = p_actor_id`, and a `require-cosign` WITH a `p_related` are each refused by name. An
    old op with a `related_actor` is refused by the shape CHECK.
  - `restore_actor_registry_supervision.rs::the_supervision_ops_survive_a_restore`: write one row of each new op,
    export (`read_local_state`), clear, restore (`apply_local_state`), and compare every field, `related_actor`
    included.
  - `a_tampered_related_actor_is_refused`: a present row differing only in `related_actor` refuses, as the
    `approval` tamper test does.
- [ ] **Step 2: Run them; they fail** (unknown op / missing column).
- [ ] **Step 3: Implement.**
  - The new guard tests the CHECK's text: `pg_get_constraintdef(...) LIKE '%link-supervisor%'`. The first block
    is never edited (its own comment forbids it).
  - The shape adds `related_actor IS NULL` to every old op's arm and states the two new arms.
- [ ] **Step 4: Run** the two suites with `--nocapture`, plus `localstate_wire_pins` and `localstate`
  (unchanged and green). Expected: PASS, no `skipped:`.
- [ ] **Step 5: Commit** `feat(supervision): six no-key ops on the registry, carried by restore`.

### Task 2: The supervision doors (db/072)

**Files:**
- Create: `db/072_supervision.sql`, `db/tests/072_supervision_test.sql` (starts `\set ON_ERROR_STOP on`,
  `\ir _scratch_database_guard.sql`)
- Modify: `crates/cairn-node/src/db.rs` (`SCHEMA` gains `("072_supervision", include_str!(…))`),
  `crates/cairn-event/src/schema_generation.rs` (72)
- Test: create `crates/cairn-node/tests/supervision_doors.rs` (helpers: `tests/common/vault_enroller_kit.rs`'s
  `sign`/`bytes`; extend that kit, not `common/mod.rs`, if it needs a helper); add a step to
  `tests/window_role_vault_doors.rs`; add `public_cannot_execute_the_supervision_doors` to
  `tests/floor_execute_grants.rs` (pattern L440)

**Interfaces:** every door is `SECURITY DEFINER SET search_path = public, pg_temp`, revoked from PUBLIC. The
helpers come first in the file, ahead of the doors.
- `cairn_supervision_address(p_op text, p_subject_staff text, p_supervisor_staff text, p_prior bigint) RETURNS bytea`:
  pure; PUBLIC keeps EXECUTE.
- `cairn_supervision_prior(p_op text, p_subject_staff text, p_supervisor_staff text) RETURNS bigint`: granted to
  `cairn_agent`.
- `cairn_supervision_approver(p_approver_staff text, p_address bytea, p_token bytea) RETURNS text`: invoker,
  owner-only. Returns `'superuser' | 'assigner' | 'member'` or RAISES naming the failed check, never echoing the
  token.
- `cairn_supervision_act(p_op text, p_subject_staff text, p_supervisor_staff text, p_approver_staff text, p_token bytea) RETURNS void`:
  granted to `cairn_agent`. Order: lock the subject's member row (`FOR UPDATE`), compute the prior, check the
  approver over the address, check the matrix, then `cairn_actor_append`. Every refusal starts
  `cairn_supervision_act: `.
- `cairn_supervision_console_act(p_op text, p_subject_staff text, p_supervisor_staff text) RETURNS void`: no
  grant; `approved_by = 'console'`. It applies the matrix's party/state refusals but no approver checks. Its body
  and the window door's share one invoker helper, `cairn_supervision_apply(…)`, so the refusals cannot drift.
- `cairn_supervision_record() RETURNS TABLE(op text, actor_id text, related_actor text, recorded_at timestamptz)`:
  the six ops, hex actors, `ORDER BY recorded_at, seq`; granted to `cairn_window`, as db/071's note doors are.
- `cairn_supervision_staff() RETURNS TABLE(staff_id text, display_name text, role text, needs_cosigning boolean, assigner boolean, supervisors text[])`:
  every non-superuser member, as of now. `needs_cosigning` is true for a student. `supervisors` are staff IDs
  (the pinned handle when the member row is gone). Granted to `cairn_agent`.

- [ ] **Step 1: Write the failing tests.**
  - db/tests/072 pins the address preimage, the grants, and that PUBLIC cannot run the definers.
  - `supervision_doors.rs` has one test per row of the Global Constraints matrix: each allowed approver succeeds,
    each refusal fires with its own words. Plus:
    - `a_stale_token_is_refused` (prior moved);
    - `an_unconfirmed_member_cannot_approve`;
    - `a_supervisee_cannot_act_on_their_own_supervision_even_as_assigner` (Review Focus 4);
    - `a_supervisor_may_step_away_and_lift`;
    - `the_record_lists_every_act_in_order`;
    - `the_staff_list_says_who_supervises_whom_now`.
  - The window-role step runs `cairn_supervision_act`, `_prior`, `_staff` and `_record` as a real
    `cairn_window` login.
- [ ] **Step 2: Run; they fail** (no such function).
- [ ] **Step 3: Implement db/072**, header explaining WHY for a junior: the record, the trust boundary, why
  rows sit on the supervisee.
- [ ] **Step 4: Run** `supervision_doors`, `window_role_vault_doors`, `floor_execute_grants`, the db/tests/072
  mirror, `cairn-event`'s `schema_generation`, and `db.rs`'s unit tests. Expected: PASS.
- [ ] **Step 5: Commit** `feat(supervision): db/072 — the approver rule, the act door, the readers`.

### Task 3: The node library and the console verbs

**Files:**
- Create: `crates/cairn-node/src/supervision_admin.rs` (+ `pub mod` in `lib.rs`)
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` variants beside `NoteSetQuarantine`; arms beside L4949)
- Test: create `crates/cairn-node/tests/supervision_admin_lib.rs`; extend `tests/cli_vault_refuses_remote.rs`

**Interfaces:**
- `#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum SupervisionOp { LinkSupervisor, UnlinkSupervisor, RequireCosign, LiftCosign, GrantAssigner, WithdrawAssigner }`
  with `as_sql(self) -> &'static str` (the six words) and `from_cli(&str) -> Option<Self>`, exhaustive.
- `pub async fn act(db: &Client, op: SupervisionOp, subject: &str, supervisor: Option<&str>, approver_staff_id: &str, approver: &dyn EventSigner) -> anyhow::Result<()>`.
  It reads `_prior`, then `_address`, signs with `vault_admin::approve`'s role (`"approved"`; make `approve`
  `pub(crate)`), and executes `cairn_supervision_act`. Errors go through `LocalDbFault::new("supervision act", e)`.
- `pub async fn console_act(db: &Client, op: SupervisionOp, subject: &str, supervisor: Option<&str>) -> anyhow::Result<()>`.
- `pub struct StaffSupervision { pub staff_id: String, pub display_name: String, pub role: String, pub needs_cosigning: bool, pub assigner: bool, pub supervisors: Vec<String> }`;
  `pub async fn staff(db: &Client) -> anyhow::Result<Vec<StaffSupervision>>`.
- CLI:
  - `cairn-node supervision-set --op <word> --staff-id <id> [--supervisor <id>]`. `--supervisor` is required
    exactly for the two link ops, a parse error otherwise.
  - `cairn-node supervision-list`, which prints one line per member.
  - Both start with `conn_posture::require_local`.

- [ ] **Step 1: Failing tests:**
  - `act_links_and_lifts_through_the_door`;
  - `console_act_needs_no_token`;
  - `staff_reads_the_panel_rows`;
  - CLI `supervision_set_refuses_a_remote_database`, and `supervision_set_parses_strictly` (an unknown op word;
    `--supervisor` given or missing wrongly).
- [ ] **Step 2: Run; fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the two suites. Expected: PASS.
- [ ] **Step 5: Commit** `feat(supervision): the node library and the console verbs`.

### Task 4: Annotate each part with the record (pure)

**Files:**
- Create: `crates/cairn-node/src/note/supervision.rs` (+ `pub mod` in `note/mod.rs`)
- Modify:
  - `crates/cairn-node/src/note/read.rs`: `NoteRow` gains two fields; `NOTES_SQL` gains `e.hlc_wall` as its
    LAST column; `list_patient_notes` reads `cairn_supervision_record()` once, on the same `client` (the caller's
    snapshot), and annotates every row.
  - `crates/cairn-node/src/note/vouch_read.rs`: `PartVouch` gains two fields; `parse_vouches` keeps `hlc_wall`
    as `at`.
  - The `NoteRow {…}`/`PartVouch {…}` literals: `note/{vouch,read,compound}.rs`,
    `cairn-gui-tauri/src/{notes_sign_off_tests,notes_parts_tests,notes_compound_tests}.rs`. Defaults below.

**Interfaces:**
- `NoteRow.recorded_at: DateTime<Utc>` (the part's `hlc_wall`); `NoteRow.needs_cosigning: bool` (the RECORD said
  the writer needs co-signing at `recorded_at`, a student's role aside; default false);
  `NoteRow.writer_supervisors: Vec<String>` (the writer's supervisors NOW, actors; default empty).
- `PartVouch.at: DateTime<Utc>`; `PartVouch.by_supervisor: bool` (its signer supervised the part's writer at
  `at`; default false); `PartVouch.writer_supervised: bool` (the writer had any supervisor at `at`; default
  false). With every default, the rules of Task 5 are today's rule exactly.
- In `supervision.rs`:
  - `pub enum SupervisionOp` is NOT redefined; rows hold the op word, parsed by an exhaustive
    `fn family(op: &str) -> anyhow::Result<Family>`. An unknown word is an error, never ignored.
  - `pub struct SupervisionRow { pub op: String, pub actor: String, pub related: Option<String>, pub at: DateTime<Utc> }`.
  - `pub struct Supervision { rows: Vec<SupervisionRow> }`, in the door's order, with:
    - `pub fn from_rows(rows: Vec<SupervisionRow>) -> anyhow::Result<Self>`;
    - `pub fn needs_cosigning_at(&self, actor: &str, t: DateTime<Utc>) -> bool`;
    - `pub fn supervises_at(&self, supervisor: &str, supervisee: &str, t: DateTime<Utc>) -> bool`;
    - `pub fn has_supervisor_at(&self, actor: &str, t: DateTime<Utc>) -> bool`;
    - `pub fn supervisors_now(&self, actor: &str) -> Vec<String>`.
    - Every actor comparison goes through `vouch::same_actor`: an empty actor matches nothing.
  - `pub fn writer_actor(p: &NoteRow) -> Option<&str>`: `signed_by`'s actor, else `drafted_actor`.
  - `pub fn annotate(p: &mut NoteRow, sup: &Supervision)` fills `needs_cosigning`, `writer_supervisors` and each vouch's `by_supervisor` and `writer_supervised` for that writer. It leaves the
    defaults when there is no writer.

- [ ] **Step 1: Failing tests** in `supervision.rs`'s `mod tests`:
  - `the_requirement_is_read_as_it_stood_at_t` (require at 10, lift at 20: true at 15, false at 25, false at 5);
  - `a_link_is_read_as_it_stood_at_t`;
  - `a_tie_on_recorded_at_takes_the_later_row`;
  - `an_unknown_op_word_is_an_error`;
  - `an_empty_actor_is_nobodys_supervisor`;
  - `annotate_judges_the_requirement_at_the_parts_time` (Review Focus 1);
  - `annotate_judges_each_vouchs_supervisor_at_its_own_time` (Review Focus 2: linked after the vouch ⇒ false;
    unlinked after it ⇒ still true);
  - `annotate_leaves_a_part_with_no_writer_alone`.
- [ ] **Step 2: Run** the pure-test command; fail.
- [ ] **Step 3: Implement**, keeping `read.rs` < 500 lines (it is 342).
- [ ] **Step 4: Run** the pure tests and the GUI tree's `cargo test --bins` (literals compile). Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): read the supervision record with the notes and annotate each part`.

### Task 5: The co-sign rules and their words (pure + one DB suite)

**Files:**
- Create: `crates/cairn-node/src/note/cosign.rs` (+ `pub mod`), with its tests in `cosign_tests.rs`
- Modify: `crates/cairn-node/src/note/vouch.rs` (`needs_sign_off`), `note/view.rs` (`part_byline_marked`)
- Modify (copy only): every expectation of `" — not co-signed"` for a writer with no supervisor:
  - `note/view_tests.rs` (8 sites) and `vouch_tests.rs` (1);
  - `tests/note_sign_off.rs` (4) and `tests/note_repudiated_signer.rs` (2);
  - `cairn-gui-tauri/src/notes_sign_off_webview_tests.rs` (1).
  - Each becomes `" — not co-signed (no supervisor recorded)"`, and nothing else changes in those tests
    (Review Focus 3).
- Test: create `crates/cairn-node/tests/note_supervision.rs`

**Interfaces** (`cosign.rs`, pure over `NoteRow`):
- `pub fn writer_signature(p: &NoteRow) -> Option<&Signer>`: `signed_by`, else the first vouch whose `by` is
  the `drafted_actor` (`same_actor`) under a standing key. That is the signature a co-sign countersigns
  (design §3.2 Q4).
- `pub fn cosign_required(p: &NoteRow) -> bool`: `part_is_signed(p) && !counts_as_unsigned(p)` and the
  writer's signature is a student's or `p.needs_cosigning`.
- `pub fn clears_cosign(p: &NoteRow, v: &PartVouch) -> bool`. The vouch must be under a standing key with a
  known actor that is not the writer's, and then either:
  - `v.by_supervisor` (any role, design §3.2 Q4), or
  - `!v.writer_supervised && v.role == VouchRole::CoSigned && !v.by.student` (today's rule, the fallback).
- `pub fn awaits_cosign(p: &NoteRow) -> bool`: required, and no vouch clears it.
- `needs_sign_off(parts)` becomes `signed_off_by(parts).is_none() || parts.iter().any(awaits_cosign)`
  (design §3.2 Q2).
- Words: `part_byline` puts the mark where today's goes (straight after `signed_by`'s words). For a part with no
  `signed_by`, it goes straight after the words of the vouch `writer_signature` picked. The mark is
  `" — not co-signed"`, plus `" (no supervisor recorded)"` when `p.writer_supervisors` is empty. The
  repudiated `" — counts as unsigned"` still takes its place (never both).

- [ ] **Step 1: Failing tests** in `cosign_tests.rs`:
  - `a_probationers_part_reads_not_co_signed`;
  - `a_non_supervisors_co_sign_does_not_clear_it`;
  - `a_supervisors_attested_sign_off_clears_it`;
  - `with_no_supervisor_any_clinicians_co_sign_clears_it` (the fallback);
  - `a_part_signed_later_by_its_writer_still_needs_its_co_sign`;
  - `the_offer_stays_while_a_mandatory_co_sign_is_missing` (a non-student signed every part, still
    `needs_sign_off`);
  - `a_repudiated_part_reads_counts_as_unsigned_not_both`.
  - In `note_supervision.rs` (DB): `a_lifted_requirement_leaves_the_earlier_part_waiting` (Review Focus 1);
    `only_a_supervisor_clears_a_mandatory_co_sign`; `a_pre_slice_students_part_is_cleared_by_any_clinician`.
- [ ] **Step 2: Run; fail.**
- [ ] **Step 3: Implement**; update the copy expectations listed above.
- [ ] **Step 4: Run** the pure tests, `note_sign_off`, `note_repudiated_signer`, `note_supervision`
  (`--nocapture`) and the GUI `--bins` (count the total; never trust a filter). Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): "— not co-signed" from the record as of signing; only a supervisor clears it`.

### Task 6: "Co-sign as reviewed" — the node gesture

**Files:**
- Create: `crates/cairn-node/src/note/review.rs` (+ `pub mod`)
- Modify: `crates/cairn-node/src/note/vouch.rs`. Extract `fn planned_role(p: &NoteRow, presser: &str) -> VouchRole`
  from `sign_off_vouches`' role arm, unchanged. Extract the transaction body of `sign_off_note` into
  `pub(crate) async fn vouch_as_shown(client, input: &SignOffInput<'_>, author, plan: impl Fn(&[NoteRow], &str) -> Vec<(Uuid, VouchRole)>) -> anyhow::Result<Result<Vec<Uuid>, SignOffRefusal>>`;
  `sign_off_note` becomes one call to it.
- Test: create `crates/cairn-node/tests/note_review_cosign.rs`

**Interfaces:**
- `pub fn review_vouches(parts: &[NoteRow], presser: &str) -> Vec<(Uuid, VouchRole)>`. It plans each part that:
  - `part_is_signed`, `!counts_as_unsigned`, `!cosign_required`;
  - has a writer (`writer_actor`) other than the presser, with the presser in `p.writer_supervisors`;
  - has no vouch with `by_supervisor` under a standing key, and that the presser has not signed.
  The role is `planned_role`.
- `pub fn can_review(parts: &[NoteRow], actor: &str) -> bool` = `!review_vouches(parts, actor).is_empty()`.
- `pub async fn review_note(client: &mut Client, input: &SignOffInput<'_>, author: &AuthorParams<'_>) -> anyhow::Result<Result<Vec<Uuid>, SignOffRefusal>>`.
  It reuses `vouch_as_shown` with `review_vouches`. `NothingToSign` is its empty plan.

- [ ] **Step 1: Failing tests:**
  - pure, in `review.rs`:
    - `review_vouches_only_the_pressers_supervisees_parts`;
    - `review_skips_a_part_that_needs_a_co_sign`;
    - `once_any_supervisor_reviewed_it_no_one_is_offered_it`.
  - DB, `note_review_cosign.rs`:
    - `a_supervisor_co_signs_a_reviewed_note`;
    - `review_never_vouches_another_writers_part`;
    - `the_key_not_the_claim_decides` (sign with Dr Zed's key while the who's-here claim is Dr M., Review Focus 5);
    - `a_part_added_after_it_was_shown_refuses_changed`.
- [ ] **Step 2: Run; fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** pure + `note_review_cosign` + `note_sign_off` (unchanged behaviour). Expected: PASS.
- [ ] **Step 5: Commit** `feat(note): "Co-sign as reviewed" — a supervisor vouches their supervisees' parts`.

### Task 7: The window's "Co-sign as reviewed"

**Files:**
- Modify:
  - `cairn-gui/cairn-gui-tauri/src/notes_compound.rs`: `CompoundView.can_review: bool` = `claim_actor` some
    and `can_review`.
  - `src/notes_sign_off.rs`: `note_review_cosign` + `_impl`, the same order as `note_sign_off_impl`; outcome
    words from the Global Constraints; timing kind `note-sign-off`.
  - `src/main.rs`: register the command.
  - `src-ui/notes_compound.js`: a `reviewButton(compound)` drawn when
    `compound.can_review && noteDraft === null`, behind `claimAsked()`, refusing while `signInFlight()`. It
    settles like `pressSignOff` (the same three outcomes; reuse its words in `whos_here.js`).
- Test: `src/notes_sign_off_tests.rs` (or a new `notes_review_tests.rs` if the file passes 470 lines),
  `src/notes_sign_off_webview_tests.rs`, `src/notes_sign_off_db_tests.rs`

**Interfaces:** `#[tauri::command] pub async fn note_review_cosign(state, patient_id: String, root_event_id: String, shown_parts: Vec<String>, credential: Option<Credential>) -> Result<NoteSignReport, ErrorView>`.

- [ ] **Step 1: Failing tests:**
  - `can_review_follows_the_claims_supervision`;
  - `review_is_offered_only_when_the_view_says_so` (source guard on `notes_compound.js`);
  - `review_sits_behind_the_whos_here_gate_and_refuses_in_flight`;
  - `its_accessible_name_is_co_sign_as_reviewed_plus_the_time`;
  - `no_dialog`;
  - DB-gated `a_supervisor_reviews_from_the_window`.
- [ ] **Step 2: Run; fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the GUI tree's full gate (clippy `--locked`, `cargo test` unfiltered, `cargo doc` with
  `RUSTDOCFLAGS=-D warnings`), the GUI DB-gated tests locally (#905), and the root source guards DB-free
  (memory: they read `cairn-gui/*/src`). Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): "Co-sign as reviewed" on a supervisee's note`.

### Task 8: The window's supervision panels

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/supervision_panel.rs` (+ tests `supervision_panel_tests.rs`),
  `src-ui/supervision.js`
- Modify:
  - `src/main.rs`: register the commands;
  - `src-ui/index.html`: two `<details>`, `#supervision-panel` (assigner) and `#my-supervisees`, beside
    `#superuser-panel`, plus the `<script>`;
  - `src/commands_tests.rs` (script list) and `src/waiting_webview_tests.rs` (`SCRIPTS`, count 21 → 22).

**Interfaces:**
- `list_supervision() -> Result<Vec<StaffSupervisionView>, String>` (all rows; no credential).
- `list_my_supervisees() -> Result<Vec<StaffSupervisionView>, String>`: the rows whose `supervisors` contain the
  who's-here claim's staff ID; empty with no named claim.
- `supervision_act(credential: Credential, op: String, staff_id: String, supervisor: Option<String>) -> Result<String, String>`.
- The key, without holding it: `async fn supervision_key(state, credential) -> Result<(ConnectionGuard, SigningKey), String>`.
  Same order as `superuser_panel::superuser_key`:
  1. `has_passphrase`;
  2. `forget_held_key`;
  3. lock;
  4. release via `vault::release_for_act(…, Purpose::Clinical)`, and on `Release::RefusedPurpose` once more with
     `Purpose::Enrol`. A superuser or admin serves only `enrol`; the purpose is refused before any count, so the
     retry costs no try.
  Every other answer goes through `from_release_noting`. It never calls `signer_for_act`, so nothing is held.
- The assigner panel lists every member: needs-co-signing, supervisors, assigner. A member who needs co-signing
  with no supervisor reads "needs co-signing · no supervisor recorded", so the gap has an owner (design §3.4).
  Buttons per op.
- "My supervisees" offers only `lift-cosign` and `unlink-supervisor` (step away).
- Every act clears the strip, re-reads, and says the door's outcome. A refusal is said plainly, and a failed
  re-read never says "nothing was recorded" (`superuser_panel`'s `READ_FAILED` rule).

- [ ] **Step 1: Failing tests:**
  - `a_supervision_act_wipes_then_releases_and_holds_nothing` (the `rust_fn` order guard, as
    `a_superuser_act_wipes_then_releases_once_and_holds_nothing`);
  - `a_superuser_key_is_released_for_enrol_after_a_refused_clinical`;
  - `my_supervisees_needs_a_named_claim`;
  - `the_gap_row_reads_no_supervisor_recorded`;
  - `supervision_act_clears_the_strip_both_ways`;
  - `no_dialog`;
  - DB-gated `an_assigner_links_and_a_supervisor_lifts_from_the_window`.
- [ ] **Step 2: Run; fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the GUI tree's full gate as in Task 7. Expected: PASS.
- [ ] **Step 5: Commit** `feat(window): the assigner's supervision panel and "My supervisees"`.

### Task 9: ADR-0094, the spec, the runbook, the durable rules

**Files:**
- Create: `docs/spec/decisions/0094-supervision-and-the-mandatory-co-sign.md`
- Modify:
  - `mkdocs.yml` (nav line after ADR-0093);
  - `docs/spec/index.md` (0.95 → 0.96);
  - `docs/spec/decisions/README.md` (index row);
  - `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (§25) and `TEMPLATE.md`;
  - `docs/developers/durable-rules.md` (a 3c-3a block above 3c-3b's);
  - `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1: Write ADR-0094.** It records:
  - the decisions of this plan's Global Constraints and Tasks;
  - which parts of ADR-0092 decisions 1, 11 and 12 it supersedes: the offer rule, the "not co-signed" clearing
    rule, and the student-only mark;
  - the one-way export;
  - "what is not guaranteed":
    - the record is node-local, so the same note may read differently on another node until slice 5 (design
      §3.1 Q6);
    - an earlier attested sign-off under the fallback leaves the button offered but answering `NothingToSign`
      (#1010's residual);
    - a raw client can still file a `co-signed` vouch by a non-supervisor; it clears nothing.
- [ ] **Step 2: Fact-check every ADR sentence against the SQL and Rust** (memory: two reviews once passed one
  false sentence). Fix the ADR, never the code, unless the code is the defect.
- [ ] **Step 3: RUNBOOK §25:**
  - a supervision act ≤ 3 s;
  - co-sign as reviewed ≤ 3 s;
  - the "(no supervisor recorded)" byline;
  - the gap row;
  - keyboard + VoiceOver on both panels and the button;
  - WebKit.
- [ ] **Step 4: Build the docs** (`uv run --with-requirements docs/requirements.txt -- mkdocs build`) and run
  `paper_parity_plan_section`. Expected: clean.
- [ ] **Step 5: Commit** `docs(supervision): ADR-0094 — supervision and the mandatory co-sign; spec v0.96, RUNBOOK §25`.
