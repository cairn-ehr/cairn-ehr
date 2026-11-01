# GUI sign-in, slice 2 — in-person enrolment, re-key, change of passphrase — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A senior clinician or admin holding the *enroller* grant can, with the person present, enrol a newcomer
or re-key a forgotten passphrase. What they issue is unconfirmed, and never blocks signing, until a superuser
confirms or repudiates it. A clinician can change their own passphrase.

**Architecture:** Everything durable goes into the append-only actor registry (`actor_event`, db/004) as new ops
(`rotate-key`, `confirm`, `repudiate`, `grant-enroller`, `withdraw-enroller`) plus two columns (`approved_by`,
`approval`), so the existing local-state export and `restore_actor_registry` (db/052) carry it through a restore.
The vault (db/061, mutable custody) gains doors in a new `db/063`. Each door verifies an attestation signed by the
approver's own vault key, as `cairn_vault_enrol` already does. Readers of `actor_current` split into **admission**
(is the key current NOW) and **re-grade of an admitted event** (does the key still vouch: ever enrolled or rotated
in, actor not revoked, key not repudiated).

**Tech Stack:** PostgreSQL 18 (db/004, db/005, db/048, db/052, db/061, new db/063), Rust (cairn-node `vault`,
`localstate`, console verbs), Tauri 2 + plain JS (cairn-gui-tauri, no npm).

**Spec:** [`docs/superpowers/specs/2026-11-01-sign-in-slice-2-rekey-and-local-enrolment-design.md`](../specs/2026-11-01-sign-in-slice-2-rekey-and-local-enrolment-design.md)
(approved 2026-11-01). Read all of it, then ADR-0083 and db/061's header, before Task 1.

## Global Constraints

- **Unconfirmed never blocks signing by itself** (design §5.5). Only a repudiation, a revoke, or a site that set an
  expiry (Task 9) stops a key.
- **Approval rule:** an enroller act needs an approver whose grant is current AND whose own key is confirmed (design
  §5.1). An enroller may never grant or withdraw the role, enrol or re-key a `superuser` or an `admin`, or confirm
  or repudiate anything.
- **"Unconfirmed" is defined in ONE place:** a key whose `enroll`/`rotate-key` row has `approved_by IS NOT NULL` and
  no `confirm` row. A door approved by a superuser (or the console) writes the `confirm` row in the same
  transaction, so its key is never unconfirmed. Rows written before this slice (`approved_by` NULL) are outside the
  mechanism: not unconfirmed.
- **Repudiation is final for that key:** a `confirm` after a `repudiate` of the same key is refused; the remedy is a
  new re-key.
- **The release door's RETURN shape does not change** (`outcome, seed, signing_key_id, retry_after_ms`): a changed
  `RETURNS TABLE` fails `CREATE OR REPLACE` on every existing database at replay. New answers are new `outcome`
  values; extra facts come from companion functions.
- **The "replaced" notice is on FAILURE only** (plan-level call refining design §5.3): a successful release proves
  the signer holds the new passphrase, so the notice would only ever be shown to the person who set it.
- **Fixed arity everywhere** (db/005's FIXED ARITY note): a door's argument list is never widened later. `enroll_actor`
  is the one exception, deliberately, in Task 1: its 3-arg form is DROPPED before the 5-arg form is created.
- **The vault rules hold** (ADR-0083): no door returns `vault_member.sealed`; no door RAISES on a wrong passphrase;
  every guess is counted before anything is revealed; refusals never echo a token, a blob or a passphrase; every
  new door is `SECURITY DEFINER SET search_path = public, pg_temp`, revoked from PUBLIC, granted to `cairn_agent`
  only where the window needs it (the window's `cairn_window` inherits it, db/062).
- **House rules:** TDD; keys from runtime seeds, and no `salt`/`nonce`/`iv` names on non-crypto values (CLAUDE.md
  rule 6; the state-binding argument below is `p_prior`); `db/*.sql` refusals use no `USING ERRCODE`; files under
  500 lines (vault.rs is 452: new Rust goes in `vault_admin.rs`); `scripts/check_closing_keywords.py` before every
  commit.
- **Pinned guards a new member must be registered in** (memory: pinned-count guards): `vault_doors_have_one_caller.rs`
  (`cairn_vault_release(` stays called only from `vault.rs`), `hex_decode_helper.rs` per-file list (if a new file
  calls `cairn_decode_hex_or_raise`), `db_errors_stay_legible.rs` per-file count (each new `LocalDbFault`-wrapped
  read), `window_role_*` grant walk (each new door the window calls must be DRIVEN to its deep path under
  `cairn_window`, never run on empty data), `tests/common/mod.rs` + `identity_scaffolding_shared.rs` if a helper is
  added there.
- **Two PRs:** PR A = Tasks 1–5 (below the window). PR B = Tasks 6–10 (the window, the ADR, the docs). PR A's body
  says the window cannot yet reach the new doors.

## Review Focus

1. **A re-key between an event and a later re-grade:** a withdrawal attested by K1 stays `attested` after K1 is
   rotated to K2. Task 2 `a_rotated_attester_still_vouches_for_past_claims`.
2. **An impostor's re-key, repudiated:** K2 signs nothing new at ANY write door, its past claims fall from
   `attested`, and Dr A (the actor) is not revoked and can be re-keyed again. Task 2
   `a_repudiated_key_loses_its_past_authority_but_the_actor_survives`; Task 4 `rekey_after_repudiation_restores_the_actor`.
3. **A chain of enrollers:** an enroller whose own key came from another enroller's re-key cannot enrol until
   confirmed. Task 4 `an_unconfirmed_enroller_cannot_enrol`.
4. **A grant token replayed after a withdrawal** does not re-grant. Task 4 `a_stale_grant_token_is_refused`.
5. **A restore** brings back every new op, both new columns, and an enroller's grant. Task 3
   `the_new_ops_survive_a_restore`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the newcomer signs their own name on their first note, no ceremony; a forgotten
  "passphrase" has no paper equivalent (a pen needs none).
- **Steps:** in-person enrolment, paper **N = 0** acts before the first note. Architecture-forced **M = 2**, once per
  person per node: the enroller's form + signature, and the newcomer's passphrase typed twice. UI bundling target
  **K = 2** (one screen, one press). **`M > N`: an architecture defect by rule 7 — Task 10 files it**, with the case
  that the enrolment buys an attributable signature paper cannot give and replaces today's 20 min on the phone plus
  an afternoon. Re-key: paper 0, M = 2, K = 2. Change of passphrase: paper 0, M = 1 (current + new twice), K = 1.
  Superuser confirm: 1 press per row.
- **Time + cognitive load:**
  - Budgets: enrolment ≤ 2 min from "I'm new" to the newcomer's first own signature; re-key ≤ 1 min; change of
    passphrase ≤ 30 s; confirm ≤ 10 s per row.
  - Load: the newcomer never chooses a username or sees a key; the enroller sees neither passphrase.
  - Owed: stopwatch + VoiceOver/keyboard, **RUNBOOK §17** (Task 10). Not measurable headlessly.

---

### Task 1: The actor registry learns the new ops (db/004)

**Files:**
- Modify: `db/004_actors.sql` (table + ALTERs, `actor_current`, `cairn_key_actor_id_conflict`, `enroll_actor`, the
  REVOKE lines; header comments), `db/tests/004_actors_test.sql`
- Test: `crates/cairn-node/tests/actor_rotate_key.rs` (DB-gated, new)

**Interfaces:**
- Produces (SQL):
  - Columns `actor_event.approved_by text` (approver's hex key, or `'console'`) and `actor_event.approval bytea` (the
    attestation token; NULL for `'console'`), each added by `ADD COLUMN IF NOT EXISTS`.
  - Op CHECK widened to `('enroll','supersede','revoke','rotate-key','confirm','repudiate','grant-enroller','withdraw-enroller')`
    (drop `actor_event_op_check` and re-add, guarded), plus a named per-op shape CHECK `actor_event_op_shape`
    (settles **#666**): `enroll`/`rotate-key` → key NOT NULL, `superseded_by` NULL; `supersede` → key NULL,
    `superseded_by` NOT NULL; `revoke` → key NULL; `confirm`/`repudiate` → key NOT NULL, `approved_by` NOT NULL;
    `grant-enroller`/`withdraw-enroller` → key NULL, `approved_by` NOT NULL.
  - `actor_current` (same columns): latest row per `actor_id` among `enroll`/`rotate-key`/`supersede`; dropped when
    that row is a `supersede` (closes **#664**), when a later `revoke` exists (as now), or when a `repudiate` row
    names that row's key.
  - `actor_key_vouches` VIEW `(actor_id bytea, kind text, signing_key_id text)`: every `enroll`/`rotate-key` row
    whose actor has no `revoke` and whose key has no `repudiate`. `GRANT SELECT ... TO cairn_agent`.
  - `cairn_key_confirmation(p_key text) RETURNS text` — `STABLE`, one of `'confirmed'`, `'unconfirmed'`,
    `'doubtful'` (unconfirmed AND its `approved_by` key is repudiated), `'repudiated'`, `'outside'` (`approved_by`
    NULL: pre-slice rows, devices, agents), `'unknown'` (no such key). `GRANT EXECUTE ... TO cairn_agent`.
  - `cairn_key_actor_id_conflict`'s op list becomes `('enroll','rotate-key')`.
  - `enroll_actor(p_kind text, p_pinned jsonb, p_key text, p_approved_by text DEFAULT NULL, p_approval bytea DEFAULT NULL)`;
    `DROP FUNCTION IF EXISTS enroll_actor(text, jsonb, text)` first; every 3-arg caller in the tree still resolves.
  - `cairn_actor_append(p_actor_id bytea, p_op text, p_key text, p_approved_by text, p_approval bytea)` — invoker
    rights, revoked from PUBLIC; the ONLY other `INSERT INTO actor_event` site; refuses `enroll`/`supersede`/`revoke`
    (those keep their own paths); copies `kind`/`pinned` from the actor's `enroll` row for `rotate-key`; for
    `rotate-key` calls `cairn_key_actor_id_conflict(p_key, p_actor_id)` under the same two advisory locks as
    `enroll_actor`. Used by Task 4's doors.

- [ ] **Step 1: Write the failing tests** in `actor_rotate_key.rs` (opening `let Some(base) = cs() else { return };`,
  the serial guard, `connect_and_load_schema`; keys from runtime seeds), driving SQL as the owner:
  - `a_rotated_key_is_no_longer_current` — enroll A/K1, append `rotate-key` K2: `actor_current` holds K2 for A,
    not K1; `actor_key_vouches` holds both.
  - `a_rotated_in_key_cannot_be_enrolled_to_another_actor` — `enroll_actor` of B with K2 is refused (#166).
  - `a_superseded_actor_is_not_current` — a `supersede` row (key NULL, `superseded_by` B) removes A from
    `actor_current`; a `supersede` row carrying a key is refused by `actor_event_op_shape`.
  - `a_repudiated_latest_key_leaves_the_actor_without_a_current_key` — A's K2 repudiated: no `actor_current` row
    for A; `actor_key_vouches` keeps K1, drops K2; a later `rotate-key` K3 makes A current again.
  - `confirmation_has_one_definition` — every value of `cairn_key_confirmation`, including `'doubtful'`.
  - `the_three_arg_enroll_still_works` — `SELECT enroll_actor('device', $1::text::jsonb, $2)`.
- [ ] **Step 2: Mirror** the shape CHECK and the view's op list in `db/tests/004_actors_test.sql` (memory: a widened
  CREATE TABLE needs its paired ALTER + a guard-test, #207).
- [ ] **Step 3: Run** `cargo test -p cairn-node --test actor_rotate_key` → FAIL.
- [ ] **Step 4: Implement** in `db/004_actors.sql`. Rewrite the header's "closed algebra" line and the #666
  contradiction (lines ~80 and ~112) to the decided convention.
- [ ] **Step 5: Run** the new test, `actor_enroll_collision`, `enroll_human`, `device_actor_enrolment`,
  `vault_floor` and `scripts/run-db-sql-tests.sh` (004 mirror) → PASS.
- [ ] **Step 6: Commit** `feat(sign-in): the actor registry rotates a key on the same actor; #664/#666 settled`.

### Task 2: Every `actor_current` reader asks the right question

**Files:**
- Modify: `db/005_submit.sql` (`cairn_claim_authority` ~:1046/:1053), `db/048_sensitivity_stream.sql` (~:1008), and
  every other reader the walk below reclassifies; `crates/cairn-node/src/actor_enrolment.rs` (`ActorStanding` docs and
  arms, per #664 items 2–3)
- Test: `crates/cairn-node/tests/actor_readers_after_rotation.rs` (DB-gated, new); extend
  `crates/cairn-node/tests/device_actor_enrolment.rs`

**Interfaces:**
- Consumes: Task 1's `actor_key_vouches`, `cairn_actor_append`.
- Produces: a reader table (file:line → ADMISSION | RE-GRADE → kept / switched) pasted into the PR body and, in
  Task 10, into the ADR.

- [ ] **Step 1: Walk.** `grep -rn actor_current db crates cairn-gui --include='*.sql' --include='*.rs'` (exclude
  `target/`, tests). Classify each site: **ADMISSION** — deciding whether a new or deferred event gets in (db/005
  submit ~:1291/:1318, db/020 ~:183/:265, db/043 ~:198, db/061 release ~:321): keeps `actor_current`, and a K1 event
  arriving after the rotation is refused or deferred, which is the safe answer (when it was signed cannot be
  known). **RE-GRADE** of an already-admitted event (db/005 `cairn_claim_authority`, db/048's worklist, any Rust
  display read): switches to `actor_key_vouches`, keeping each site's existing guards (the `count(*) = 1` human
  check, `attester_key IS NOT NULL`).
- [ ] **Step 2: Write the failing tests** in `actor_readers_after_rotation.rs`:
  - `a_rotated_attester_still_vouches_for_past_claims` — an attested withdrawal by K1, then `rotate-key` K2:
    `cairn_claim_authority` still answers `'attested'`.
  - `a_repudiated_key_loses_its_past_authority_but_the_actor_survives` — the same claim with K2 repudiated after
    signing it: not `'attested'`; the actor has no `revoke` row.
  - `a_rotated_away_key_cannot_submit` — a fresh event signed by K1 is refused by `submit_event`.
  - `a_repudiated_key_cannot_submit` — refused at `submit_event` and at `apply_remote_event`.
  - In `device_actor_enrolment.rs`: `a_superseded_key_classifies_as_retired` (#664 item 3).
- [ ] **Step 3: Run** → FAIL on the first two and the last.
- [ ] **Step 4: Implement** the RE-GRADE switches. `cairn_claim_authority` keeps its fixed arity.
- [ ] **Step 5: Run** the new test plus `claim_authority`, `claim_authority_worklist`, `suppression_owner_gate`,
  `recall_epoch`, `floor_enforced` → PASS.
- [ ] **Step 6: Commit** `fix(sign-in): a re-grade asks whether a key still vouches, not whether it is current`.

### Task 3: The registry's new columns survive export and restore

**Files:**
- Modify: `crates/cairn-node/src/localstate.rs` (`ActorRegistryRow` :564, `actor_registry_rows_to_json` :595),
  `crates/cairn-node/src/localstate_read.rs` (:131 SELECT, :145 row build), `db/052_restore_doors.sql` (op list
  :176, INSERT column list, the field-by-field comparison :206)
- Test: `crates/cairn-node/tests/restore_actor_registry_new_ops.rs` (DB-gated, new); unit tests in `localstate.rs`

**Interfaces:**
- Produces: `ActorRegistryRow { …, #[serde(default)] pub approved_by: Option<String>, #[serde(default)] pub approval: Option<Vec<u8>> }`.
  `approval` travels as HEX in the JSON (decoded in db/052 through `cairn_decode_hex_or_raise`, like `actor_id`).

- [ ] **Step 1: Write the failing tests:**
  - Unit: `approval_travels_as_hex_and_is_omitted_when_absent`; `an_old_row_without_the_new_fields_still_decodes`.
  - DB: `the_new_ops_survive_a_restore` — a registry with `rotate-key`, `confirm`, `repudiate`, `grant-enroller`
    rows → export → `restore_actor_registry` into a fresh scratch DB → identical rows, field for field.
  - DB: `a_present_row_with_a_different_approval_is_refused` (the #569 comparison learns the new fields).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.** db/052 now calls `cairn_decode_hex_or_raise` once more — check `hex_decode_helper.rs`'s
  per-file count.
- [ ] **Step 4: Run** the new test, `restore_ceremony_order`, `restore_reads_the_clinical_plane`,
  `restore_cli_surface`, `localstate` unit tests → PASS.
- [ ] **Step 5: Commit** `feat(sign-in): a restore brings back rotations, confirmations and grants`.

### Task 4: The vault's new doors (db/063)

**Files:**
- Create: `db/063_vault_enrollers.sql`, `db/tests/063_vault_enrollers_test.sql` (starts `\ir _scratch_database_guard.sql`)
- Modify: `db/061_vault.sql` (`cairn_vault_release`'s purpose check, step 1, AND its "wrong" answer in step 4;
  `cairn_vault_enrol`'s approver check; `cairn_vault_bootstrap` writes a `confirm` row; header), `crates/cairn-node/src/db.rs`
  (`SCHEMA` gains 063), `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION = 63`)
- Test: `crates/cairn-node/tests/vault_enrollers.rs` (DB-gated, new); extend `vault_floor.rs`

**Interfaces:**
- Consumes: Task 1's `cairn_actor_append`, `enroll_actor` 5-arg, `cairn_key_confirmation`.
- Produces (SQL; all addresses `'\x1220'::bytea || sha256(convert_to(<text>, 'UTF8'))`, `IMMUTABLE` unless they read
  a table, lines joined by `E'\n'`, staff ID through `cairn_vault_staff_id`):
  - `cairn_vault_rekey_address(p_staff_id text, p_new_key text)` — `'cairn-vault-rekey'`, staff ID, key.
  - `cairn_vault_grant_address(p_staff_id text, p_grant boolean, p_prior bigint)` — `'cairn-vault-grant'`, staff ID,
    `'grant'`/`'withdraw'`, `p_prior` = `seq` of the target's latest grant/withdraw row, 0 if none (so a token is
    good for the state it changes, and only that).
  - `cairn_vault_judge_address(p_signing_key_id text, p_confirm boolean)` — `'cairn-vault-judge'`, key,
    `'confirm'`/`'repudiate'`.
  - `cairn_vault_reseal_address(p_staff_id text, p_new_sealed bytea)` — `STABLE SECURITY DEFINER` (reads the stored
    blob): `'cairn-vault-reseal'`, staff ID, hex `sha256` of the CURRENT sealed blob, hex `sha256(p_new_sealed)`.
    Returns only the digest-built address, never the blob.
  - Purpose map in `cairn_vault_release`, purposes `('clinical','enrol','reseal')`: `superuser` and `admin` →
    `enrol`, `reseal`; every other role → `clinical`, `reseal`, and `enrol` when `cairn_vault_is_enroller(staff_id)`.
  - `cairn_vault_is_enroller(p_staff_id text) RETURNS boolean` — the member's actor's latest grant/withdraw row is a
    grant AND `cairn_key_confirmation(member key) IN ('confirmed','outside')` AND the actor is current.
  - `cairn_vault_approver_ok(p_approver_staff_id text, p_address bytea, p_token bytea, p_may_enroller boolean) RETURNS text`
    — invoker, revoked from PUBLIC: `'superuser'`, `'enroller'`, or RAISES naming the failed check. The one place the
    approver rule lives; every door below calls it.
  - `cairn_vault_enrol(...)` keeps its 11 arguments (the parameter is still named `p_superuser_staff_id`: Postgres
    refuses a renamed parameter under `CREATE OR REPLACE`; the header says it means "approver"). An enroller may not
    enrol `superuser`/`admin`. Writes `enroll` with `approved_by`/`approval`, plus a `confirm` row when the approver
    is a superuser.
  - `cairn_vault_rekey(p_staff_id text, p_role text, p_display_name text, p_phone text, p_email text, p_new_key text, p_sealed bytea, p_hint text, p_approver_staff_id text, p_token bytea) RETURNS bytea`
    (the actor_id). With a member row: `p_role` must equal it; NULL display/phone/email keep the stored ones; the row's
    key and blob are replaced and its attempt count reset (`success_mark` = attempts consumed). With NO member row
    (after a restore): the actor `cairn_actor_id({role, handle})` must exist in the registry; display name and a
    contact are required; the member row is created. Either way: `rotate-key` row with approval, plus `confirm` when
    the approver is a superuser. Refuses a target whose role is `superuser`/`admin` when the approver is an enroller,
    and any `superuser` target (console only, Task 5).
  - `cairn_vault_bootstrap_rekey(p_staff_id, p_display_name, p_new_key, p_sealed, p_hint, p_phone, p_email) RETURNS bytea`
    — owner only, no grant: the restore case for a superuser; `approved_by = 'console'` + `confirm`.
  - `cairn_vault_set_enroller(p_staff_id text, p_grant boolean, p_superuser_staff_id text, p_token bytea) RETURNS void` — superuser only.
  - `cairn_vault_judge_key(p_signing_key_id text, p_confirm boolean, p_superuser_staff_id text, p_token bytea) RETURNS void`
    — superuser only; refuses `confirm` after `repudiate`; refuses a key not `'unconfirmed'`/`'doubtful'` for confirm.
  - `cairn_vault_reseal(p_staff_id text, p_new_sealed bytea, p_hint text, p_proof bytea) RETURNS void` — `p_proof`
    is the member's OWN attestation over `cairn_vault_reseal_address(p_staff_id, p_new_sealed)`, checked against the
    member's key. Replaces `sealed` and `hint` (NULL clears the hint: the form always sends what it shows). Writes no
    registry row. A caller who seals the wrong seed only locks themselves out, and `release_from_row`'s key-id check
    catches it at the next release.
  - `cairn_vault_unconfirmed() RETURNS TABLE(staff_id text, display_name text, signing_key_id text, approver_staff_id text, approved_at timestamptz, doubtful boolean)` — every unconfirmed/doubtful CURRENT key; granted to `cairn_agent`.
  - `cairn_vault_enrollers() RETURNS TABLE(staff_id text, display_name text, enroller boolean)` — every
    non-superuser member, for the grant panel; granted to `cairn_agent`.
  - Release door, step 4: a wrong passphrase on a member whose current key's `rotate-key` row is ≤ 14 days old or
    still unconfirmed answers **`'replaced'`** (counted exactly like `'wrong'`, same `retry_after_ms`).
    `cairn_vault_replaced_notice(p_staff_id text) RETURNS TABLE(replaced_at timestamptz, approver_display_name text)`
    supplies the sentence's facts; granted to `cairn_agent`. *14 days is this plan's call; one constant in one
    function.*

- [ ] **Step 1: Write the failing tests** in `vault_enrollers.rs` (members built through the doors; keys and blobs
  from `seal::seal` with runtime seeds):
  - `a_superuser_grants_and_an_enroller_enrols` — the new member's key is `'unconfirmed'`, signs a clinical act the
    floor accepts (`seal_sign_submit`), and appears in `cairn_vault_unconfirmed()`.
  - `a_superuser_enrolment_is_confirmed_at_once`; `bootstrap_writes_a_confirm_row`.
  - `an_enroller_cannot_enrol_a_superuser_or_an_admin`; `an_enroller_cannot_grant_or_judge`.
  - `an_unconfirmed_enroller_cannot_enrol` (Review Focus 3); `a_stale_grant_token_is_refused` (Review Focus 4).
  - `rekey_keeps_the_actor_and_retires_the_old_key` — same `actor_id`; old key not in `actor_current`; release with
    the old passphrase → `'replaced'`; `cairn_vault_replaced_notice` names the approver.
  - `rekey_after_repudiation_restores_the_actor` (Review Focus 2).
  - `rekey_without_a_member_row_recreates_it` — delete the vault rows (owner), keep the registry: `cairn_vault_rekey`
    by a superuser re-creates the member on the same actor.
  - `reseal_needs_the_members_own_proof` — a proof by another key is refused; a proof over a different blob is
    refused; the right one makes the NEW passphrase release and the old one answer `'wrong'`.
  - `a_repudiated_enrollers_unconfirmed_members_are_doubtful`.
  - `reseal_releases_the_key_for_reseal_only` — `'reseal'` purpose works for every role; a superuser key is still
    refused for `'clinical'`.
- [ ] **Step 2: Mirror** the address functions and purpose map in `db/tests/063_vault_enrollers_test.sql`.
- [ ] **Step 3: Run** → FAIL.
- [ ] **Step 4: Implement** db/063 and the db/061 edits; append 063 to `SCHEMA`; bump the generation.
- [ ] **Step 5: Run** `vault_enrollers`, `vault_floor`, `vault_lib`, `window_role_vault_doors`, the generation guards,
  then `scripts/run-db-gated-tests.sh` with `PGHOST=127.0.0.1 PGPORT=5532` (#862) → PASS.
- [ ] **Step 6: Commit** `feat(sign-in): db/063 — enrollers, re-key, confirm/repudiate, reseal`.

### Task 5: `cairn_node::vault_admin` and the console verb

**Files:**
- Create: `crates/cairn-node/src/vault_admin.rs` (`pub mod vault_admin;` in lib.rs)
- Modify: `crates/cairn-node/src/vault.rs` (`Purpose::Reseal`, `Release::Replaced`, `release_from_row`;
  drop the `mint_sealed` shortcut comment — slice 2 keeps the recovery wrap unused, say so),
  `crates/cairn-node/src/main.rs` (`Cmd::VaultRekeySuperuser` beside `VaultBootstrapSuperuser` :1990, handler beside :4807)
- Test: `crates/cairn-node/tests/vault_admin_lib.rs` (DB-gated); unit tests in `vault_admin.rs`

**Interfaces:**
- Consumes: Task 4's SQL; `vault::{NewMember, release_for_act, Release, Purpose}`; `sign_attestation`.
- Produces:
  ```rust
  pub enum Purpose { Clinical, Enrol, Reseal }                 // vault.rs
  pub enum Release { …, Replaced { retry_after: Option<Duration> } }   // vault.rs
  pub struct ReplacedNotice { pub at: String /* RFC 3339 */, pub approver: String }
  pub async fn replaced_notice(db: &Client, staff_id: &str) -> anyhow::Result<Option<ReplacedNotice>>;
  pub struct Rekey<'a> { pub staff_id: &'a str, pub role: &'a str, pub display_name: Option<&'a str>,
      pub phone: Option<&'a str>, pub email: Option<&'a str>, pub passphrase: &'a str, pub hint: Option<&'a str> }
  pub async fn rekey_member(db: &Client, r: &Rekey<'_>, approver_staff_id: &str, approver: &dyn EventSigner) -> anyhow::Result<String>;
  pub async fn rekey_superuser_from_console(db: &Client, r: &Rekey<'_>) -> anyhow::Result<String>;
  pub async fn set_enroller(db: &Client, staff_id: &str, grant: bool, su_staff_id: &str, su: &dyn EventSigner) -> anyhow::Result<()>;
  pub async fn judge_key(db: &Client, kid: &str, confirm: bool, su_staff_id: &str, su: &dyn EventSigner) -> anyhow::Result<()>;
  pub enum Reseal { Done, Refused(Release) /* any non-Released answer, passed through */ }
  pub async fn change_passphrase(db: &Client, staff_id: &str, current: &str, new: &str, hint: Option<&str>) -> anyhow::Result<Reseal>;
  pub struct Unconfirmed { pub staff_id: String, pub display_name: String, pub kid: String,
      pub approver_staff_id: String, pub approved_at: String, pub doubtful: bool }
  pub async fn unconfirmed(db: &Client) -> anyhow::Result<Vec<Unconfirmed>>;
  pub struct EnrollerRow { pub staff_id: String, pub display_name: String, pub enroller: bool }
  pub async fn enrollers(db: &Client) -> anyhow::Result<Vec<EnrollerRow>>;
  ```
  Every write is wrapped in `LocalDbFault` (refusal vs. lost answer), as `enrol_member` is. `change_passphrase`
  releases through `vault::release_for_act(.., Purpose::Reseal)` — never a second caller of the door — reseals the
  seed under `new` (`mint_sealed`'s sealing half, refactored to a shared `seal_seed`), signs the proof with the
  released key, and drops it.

- [ ] **Step 1: Write the failing tests:** unit — each address matches its SQL twin (DB-gated half);
  `release_from_row("replaced", …)` → `Release::Replaced`. DB — `a_rekeyed_member_signs_with_the_new_passphrase`;
  `change_passphrase_keeps_the_key` (same kid before and after); `a_wrong_current_passphrase_changes_nothing` (→
  `Reseal::Refused(Release::Wrong{..})`, blob unchanged); `the_console_rekeys_a_restored_superuser`.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement**; the console verb `vault-rekey-superuser --staff-id --display-name [--phone] [--email] [--hint]`
  prompts the passphrase twice with `rpassword` and connects with `db::connect` as the owner (refusing a remote
  database, as the other `vault-*` verbs do, #899).
- [ ] **Step 4: Run** the new tests, `vault_lib`, `vault_doors_have_one_caller`, `db_errors_stay_legible`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`,
  then the FULL workspace `cargo test` in the background (memory: ~2 h; cross-crate callers) → PASS.
- [ ] **Step 5: Commit** `feat(sign-in): vault_admin — re-key, grants, judgements, change of passphrase`. **Open PR A.**

### Task 6: The window enrols and re-keys as an enroller

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/enrol.rs` (approver may be an enroller; refusals), `src-ui/enrol.js`,
  `src-ui/index.html` (role list gains `admin`; a "Forgotten passphrase" `<details id="rekey-staff">`)
- Create: `cairn-gui/cairn-gui-tauri/src/rekey.rs` + `rekey_tests.rs`, `src-ui/rekey.js`
- Modify: `src/main.rs` (register `rekey_member`)

**Interfaces:**
- Consumes: Task 5's `rekey_member`, `Purpose::Enrol`.
- Produces: Tauri command `rekey_member(credential: Credential, member: RekeyForm) -> Result<String, String>`;
  `RekeyForm { staff_id, role, display_name, phone, email, passphrase, passphrase_again, hint }` with a hand-written
  `Debug` that redacts both passphrases (as `NewMemberForm`). New `EnrolRefusal` variants: `ApproverMayNotEnrolThisRole`,
  `ApproverNotConfirmed`; their sentences are built from the enum alone.

- [ ] **Step 1: Write the failing tests** (pure, in `rekey_tests.rs` / `enrol_tests.rs`): `check_rekey_form` refuses
  differing / empty passphrases before any database call; each new refusal's sentence names no form field and no
  passphrase; DB-gated (run locally, #905): `an_enroller_rekeys_from_the_window` — the old passphrase then answers
  `Replaced`.
- [ ] **Step 2: Run** `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tauri` → FAIL.
- [ ] **Step 3: Implement**, following enrol.rs's flow: wipe any held key; release the approver's key with
  `Purpose::Enrol`, use it once, drop it; never `signer_for_act`. Both new passphrases cleared after every attempt
  (as enrol.js does).
- [ ] **Step 4: Headless walk** (memory: webview mock walk recipe): both forms are VISIBLE when opened (computed
  display + hidden ancestors), not only present in the DOM.
- [ ] **Step 5: Run** the cairn-gui gate (clippy `--locked`, test, doc) → PASS. **Commit**
  `feat(sign-in): an enroller enrols and re-keys from the window`.

### Task 7: Change my passphrase, and the "replaced" sentence

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/reseal.rs` + `reseal_tests.rs`, `src-ui/reseal.js`
- Modify: `src/signer.rs` (`from_release` maps `Release::Replaced`), `src-ui/signer.js`, `src-ui/index.html`
  (`<details id="change-passphrase">`), `src/main.rs`

**Interfaces:**
- Produces: Tauri command `change_passphrase(form: ResealForm) -> Result<String, String>`;
  `ResealForm { staff_id, current, new, new_again, hint }` (redacting `Debug`). `KeyRefusal::Replaced { at, approver }`,
  whose sentence is exactly: **"This passphrase was replaced on {at}, approved by {approver}. If you didn't ask for
  that, tell a superuser."** (maintainer, 2026-11-01). `{at}` is a local date and time.

- [ ] **Step 1: Write the failing tests:** the sentence is byte-exact; `from_release(Replaced)` fetches the notice once
  (a missing notice degrades to the plain wrong-passphrase sentence, never an error); `check_reseal_form` refuses a
  `new` equal to `current`, differing `new`s, an unusable `new`; no sentence carries a passphrase.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Headless walk:** the replaced sentence is visible in
  the strip. **Step 5: Gate and commit** `feat(sign-in): change my passphrase; a replaced passphrase says so`.

### Task 8: The superuser's panel — grants and the unconfirmed worklist

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/superuser_panel.rs` + `_tests.rs`, `src-ui/superuser.js`
- Modify: `src-ui/index.html` (`<details id="superuser-panel">`), `src/main.rs`

**Interfaces:**
- Produces: Tauri commands `list_unconfirmed() -> Result<Vec<UnconfirmedView>, String>`,
  `list_enrollers() -> Result<Vec<EnrollerView>, String>`, `set_enroller(credential, staff_id, grant: bool)`,
  `judge_key(credential, kid, confirm: bool)`. Each write releases the superuser's key with `Purpose::Enrol`, uses it
  once, drops it. Lists are refreshed after every write.

- [ ] **Step 1: Write the failing tests:** each view row's text (who, approved by whom, when; "doubtful — approved by
  a repudiated enroller" when `doubtful`); a non-superuser credential's refusal sentence; Repudiate is not offered
  for a row already repudiated (the list holds none, pinned).
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** No confirmation dialog for Repudiate (principle 3): the act is
  signed, audited, and repaired by a re-key; say so in a code comment.
- [ ] **Step 4: Headless walk** (visible rows, keyboard reachable buttons). **Step 5: Gate and commit**
  `feat(sign-in): the superuser grants enrollers and confirms or repudiates keys`.

### Task 9: Optional site expiry for unconfirmed keys

**Files:**
- Modify: `db/063_vault_enrollers.sql` (`vault_policy` singleton, `cairn_vault_set_unconfirmed_expiry(p_days int)`
  owner only, `cairn_vault_unconfirmed_expires_at(p_staff_id text) RETURNS timestamptz` granted to `cairn_agent`;
  release step 1 answers `'expired'` before any counting), `crates/cairn-node/src/vault.rs` (`Release::Expired`),
  `main.rs` (`vault-set-unconfirmed-expiry --days N | --none`), the window strip (warning)
- Test: extend `vault_enrollers.rs`; `signer_tests.rs`

- [ ] **Step 1: Write the failing tests:** `no_expiry_by_default` (an unconfirmed key a year old still releases);
  `an_expired_unconfirmed_key_is_not_released` (and nothing is counted); `a_confirmed_key_never_expires`; window: a
  release within 7 days of expiry shows "Not yet confirmed by a superuser — signing stops on {date}" after the act;
  an expired key's sentence says to ask a superuser to confirm.
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** (`vault_policy` sits beside `vault_terminal`: custody state,
  outside backups, so a restore returns to the default — no expiry — which fails toward availability; say so in
  the header). **Step 4: Gate and commit** `feat(sign-in): a site may expire unconfirmed keys (off by default)`.

### Task 10: ADR-0085, spec, RUNBOOK, durable rules, handover

**Files:**
- Create: `docs/spec/decisions/0085-in-person-enrolment-and-re-key.md` (+ `mkdocs.yml` nav line, ADR index row)
- Modify: `docs/spec/index.md` (v0.87), `docs/spec/security.md` §7.5 (the key-loss ceremony is now built),
  ADR-0083 (dated erratum: its "owed by slice 2" bullets), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (§17),
  `docs/developers/durable-rules.md` (a slice-2 block), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write the ADR** — the enroller grant and its limits; unconfirmed / confirm / repudiate and the
  cascade; `rotate-key` on the same actor; the #247 decision (option (a): key id in the body, resolved when read
  through `actor_key_vouches`, true only where this node's registry is); #664 and #666; Task 2's reader table; the
  failure-only notice and its 14 days; the expiry default; what Cairn cannot verify (physical presence, a contact
  reachable without a hospital login). **Fact-check it sentence by sentence against the SQL** (memory: ADR-0077
  passed two reviews with a false sentence).
- [ ] **Step 2: RUNBOOK §17** — the four budgets, VoiceOver/keyboard on the three new forms and the panel.
- [ ] **Step 3: File** the `M > N` issue (paper-parity benchmark above) and any finding the final review cannot fix.
- [ ] **Step 4: Run** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`,
  `paper_parity_plan_section`, `scripts/check_closing_keywords.py` on the PR text. **Commit**
  `docs(sign-in): ADR-0085 — in-person enrolment and re-key`. **Open PR B.**
