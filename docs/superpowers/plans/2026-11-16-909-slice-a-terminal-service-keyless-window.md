# #909 slice A — the terminal service and a keyless window — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** No window holds the node's signing key. Registration and the quarantine commit, the only two node-signed
window acts, move to a node-side `cairn-node terminal-service`.

**Architecture:** A borrowed `NodeKey<'a>` (full key, or public id only) replaces `&SigningKey` on every path the
window reaches, so the window can hold only the public id. A new `db/083` adds a registration request queue and a
per-draft "last commit failed" fact, each behind definer doors for a new `cairn_terminal_service` group role. The
service claims a request under its row lock, re-runs the search, registers in that same transaction and settles the
row. It also runs the 60 s commit tick the window ran until now.

**Tech Stack:** Rust (tokio, tokio-postgres), PL/pgSQL (PostgreSQL 18), Tauri 2 window (plain JS). No new crate.

**Spec:** [`docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md`](../specs/2026-11-16-pinned-terminal-link-909-design.md)
(D1, D6, D7, D8; slice B (D2–D5, the gateway) and the rollout (§6) are later plans).

## Global Constraints

- No wire change: registration events stay node-signed with `{node, recorded}` exactly as today (design D6).
- No Postgres restart, reload or `pg_hba`/`postgresql.conf` edit anywhere in this slice (design goal 3).
- The window never loads `node.key`, never calls `cairn_node::keystore::load`, and never builds `NodeKey::Full` (design
  D8). The `cairn-node` CLI keeps prompting for the node passphrase as today (its ceremony is unchanged).
- `cairn_window` gets no claim, settle or failure-write door. Those are `cairn_terminal_service`'s alone
  (`cairn_terminal_service` inherits `cairn_window`). **Slice B's gateway must refuse a login that is a member of
  `cairn_terminal_service`.** Record this in the ADR.
- The registration request deadline is **10 s** from submission (`now()` at the door). The window polls every
  **200 ms**, and after the deadline calls the expire door once.
- The service polls for requests every **250 ms**, commits overdue drafts every **60 s** (`COMMIT_TICK`, unchanged),
  and purges settled or expired requests older than **24 h** (they hold names).
- New `db/*.sql` file: `db/083_terminal_service.sql`. `SCHEMA_GENERATION` 82 → **83**, with an entry in
  `crates/cairn-node/src/db.rs`'s list.
- House rules: TDD; files under 500 lines where feasible (`notes.rs` is 465 lines, so do not grow it); inline docs
  for a junior; durable rules in `docs/developers/durable-rules.md`; `CAIRN_ALLOW_DB_SKIP` unset for DB-gated runs.
- Pinned-count guards (memory): a new `LocalDbFault`-wrapped read +1 in `db_errors_stay_legible.rs`; new definer
  functions need `REVOKE … FROM PUBLIC` (`floor_execute_grants.rs`); the window-role grant mirrors
  (`window_role_grants*.rs`) pin `cairn_window`'s reach. Run every root source guard DB-free, since some sweep
  `cairn-gui/*/src`.

## Review Focus

1. **The window loses its connection while waiting for a registration.** The chart may exist, so the window must
   answer `OutcomeUnknown`, never `NotProvisioned` or `Unavailable` (#815). Pinned in Task 5.
2. **The window's expire and the service's claim race at the deadline.** Exactly one outcome may result: expire
   blocks behind the claim's row lock and then reports `registered`. Pinned in Task 2.
3. **Two service processes run at once** (a misconfigured second unit). `SKIP LOCKED` means one registers and the
   other passes over the row. Pinned in Task 4.
4. **A poison request** (malformed `query`/`displayed` JSON sent by raw SQL) settles as `refused` with its reason and
   never stalls the queue or kills the loop. Pinned in Task 4.
5. **The service dies mid-transaction.** The request stays `pending` and is then served or expired, never registered
   twice. Pinned in Task 4.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a clerk searches the card index, finds no card, writes a new card, files it.
- **Steps:** paper *N* = 3 human acts (search, decide no match, write the card). Architecture-forced *M* = 3, unchanged
  from today: the request and the service's re-run are machine steps the clerk never sees. UI bundling target *K* = 3.
  A new possible match found by the re-run costs one more look, and that is the funnel's own purpose (ADR-0061), not
  an added step.
- **Time + cognitive load:** budget: Register's round trip grows by at most **0.5 s** median on the database's machine
  (one poll cycle plus a re-run search). It is measured in RUNBOOK §38 by the slice that first exposes it (this one).
  The cognitive load is unchanged except one new sentence when the service is down.

---

### Task 1: `NodeKey` — the window can hold the node's public id only

**Files:**
- Create: `crates/cairn-node/src/node_key.rs` (+ `pub mod node_key;` in `lib.rs`)
- Modify: `crates/cairn-node/src/medication/sealed_submit.rs` (`apply_author`, `prepare_sealed`, `seal_sign_submit`),
  and every function the window reaches that takes `node_sk`: `allergy/sign.rs` (4), `medication/{cessation,
  withhold, draft, draft_versions_sign, version_settle, stat_give, signoff}.rs`, `note/{sign, discard}.rs`.
- Modify: every caller (`cairn-node/src/main.rs`, `cairn-gui/…`, `crates/cairn-sync/tests/clinical_pull.rs`, the tests).
  CLI-only paths (`assert`, `coding`, `dose`, `reconciliation`, `charting`, `identify`, `shred`, `note/commit`) keep
  `&SigningKey` and wrap it as `NodeKey::Full(sk)` at their `prepare_sealed` call.
- Test: `crates/cairn-node/src/node_key.rs` (unit), `crates/cairn-node/tests/node_key_public_writes.rs` (DB-gated).

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy)]
  pub enum NodeKey<'a> { Full(&'a SigningKey), Public(&'a str /* node kid, hex */) }
  impl<'a> NodeKey<'a> {
      pub fn kid(&self) -> String;                       // hex of the verifying key, either arm
      pub fn signer(&self) -> anyhow::Result<&'a SigningKey>; // Public → Err(NODE_KEY_NOT_HELD)
  }
  pub const NODE_KEY_NOT_HELD: &str;
  pub fn apply_author<'a>(body: EventBody, author: Option<&'a AuthorParams<'a>>, node: NodeKey<'a>)
      -> anyhow::Result<(EventBody, &'a dyn EventSigner)>;
  ```
  `prepare_sealed` and `seal_sign_submit` take `node: NodeKey<'_>` in place of `node_sk: &SigningKey`. Each converted
  orchestrator does the same. `signoff`'s `_node_sk` becomes `_node: NodeKey<'_>`.

- [ ] **Step 1: Write the failing unit tests** `public_key_gives_its_kid_and_refuses_to_sign` (`NodeKey::Public(k).kid()
  == k`; `signer()` is an `Err` whose text is `NODE_KEY_NOT_HELD`) and `full_key_kid_matches_verifying_key`, with keys
  derived at runtime (house rule 6). `NODE_KEY_NOT_HELD` = `"this act needs the node's own signing key, which this
  window does not hold; it runs on the node's terminal service"`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib node_key`. Expected: FAIL (module missing).
- [ ] **Step 3: Implement `node_key.rs`; convert `apply_author` (the `None` arm calls `node.signer()?`), `prepare_sealed`,
  `seal_sign_submit` and the listed orchestrators.** Each `hex::encode(node_sk.verifying_key()…)` becomes
  `node.kid()`. The compiler finds every caller; the CLI passes `NodeKey::Full(&sk)`.
- [ ] **Step 4: Write the DB-gated test** `an_authored_cessation_signs_with_only_the_public_node_id`: call
  `cease_medication` with `NodeKey::Public(&identity.pubkey_hex)` and a human `AuthorParams`; the event lands signed by
  the human key and its contributors name the node kid as `recorded`. Also
  `a_node_signed_body_refuses_with_only_the_public_id`: `seal_sign_submit(…, NodeKey::Public(..), body, None, None)`
  errs with `NODE_KEY_NOT_HELD` and writes no row.
- [ ] **Step 5: Run** `cargo test -p cairn-node --lib node_key && cargo test -p cairn-node --test
  node_key_public_writes -- --nocapture` (no `skipped:` line), then `cargo test -p cairn-sync --no-run` and
  `cargo build` in `cairn-gui/`. Expected: PASS / builds.
- [ ] **Step 6: Commit** `refactor(#909): NodeKey — authored writes need only the node's public id`.

### Task 2: `db/083` — the terminal-service role, the registration queue, the failed-commit fact

**Files:**
- Create: `db/083_terminal_service.sql`
- Modify: `crates/cairn-node/src/db.rs` (list entry), `crates/cairn-event/src/schema_generation.rs` (82 → 83)
- Test: `crates/cairn-node/tests/terminal_service_doors.rs` (DB-gated)

**Interfaces:**
- Produces (SQL):
  - Role `cairn_terminal_service` NOLOGIN; `GRANT cairn_window TO cairn_terminal_service`.
  - Table `registration_request(request_id uuid PK, name text NULL, query jsonb NOT NULL, displayed jsonb NOT NULL,
    today text NOT NULL, submitted_at timestamptz NOT NULL DEFAULT now(), deadline timestamptz NOT NULL,
    status text NOT NULL CHECK (status IN ('pending','registered','refused','expired')), patient_id uuid,
    registration_event uuid, refusal text, settled_at timestamptz)`. No grants to any role; doors only. Not in
    any backup or export.
  - `cairn_registration_submit(p_request uuid, p_name text, p_query jsonb, p_displayed jsonb, p_today text) RETURNS
    text` → `cairn_window`. Inserts with `deadline = now() + interval '10 seconds'`; on a repeated id with identical
    content returns the existing status; with different content raises. Returns the status.
  - `cairn_registration_status(p_request uuid) RETURNS TABLE(status text, patient_id uuid, refusal text)` →
    `cairn_window`.
  - `cairn_registration_expire(p_request uuid) RETURNS text` → `cairn_window`. `SELECT … FOR UPDATE` (blocking), sets
    `expired` only when still `pending`, and returns the status it then holds.
  - `cairn_registration_claim() RETURNS SETOF registration_request` → `cairn_terminal_service`. One row,
    `status='pending' AND deadline > now()`, `FOR UPDATE SKIP LOCKED`, oldest first. It locks within the caller's
    transaction.
  - `cairn_registration_settle(p_request uuid, p_status text, p_patient uuid, p_event uuid, p_refusal text) RETURNS
    void` → `cairn_terminal_service`. Only from `pending`, only to `registered` (needs patient + event) or `refused`
    (needs refusal).
  - `cairn_registration_purge() RETURNS int` → `cairn_terminal_service`. Deletes non-pending rows settled more than 24 h
    ago, and pending rows whose deadline passed more than 24 h ago.
  - Table `note_draft_commit_failure(draft uuid PK REFERENCES` the draft table's key `ON DELETE CASCADE, failed_at
    timestamptz NOT NULL)`, no cause text (the service's log has it). Doors: `cairn_note_draft_commit_failed(p_draft
    uuid, p_failed boolean) RETURNS void` → `cairn_terminal_service` (upsert or delete);
    `cairn_note_drafts_commit_failed(p_patient uuid) RETURNS SETOF uuid` → `cairn_window`.
  - Every function: `SECURITY DEFINER SET search_path = public, pg_temp`, `REVOKE EXECUTE … FROM PUBLIC`.
    Replay-safe (guarded `CREATE ROLE`, `CREATE TABLE IF NOT EXISTS`, idempotent grants).

- [ ] **Step 1: Write the failing tests** (as a `cairn_window` probe login and a `cairn_terminal_service` probe login,
  per the existing role tests' helper):
  `a_resubmitted_request_returns_its_first_status_and_a_changed_one_is_refused`;
  `claim_skips_an_expired_request_and_a_locked_one`;
  `expire_changes_only_a_pending_request`;
  `expire_waits_behind_a_claim_and_then_reports_registered` (session A claims and settles `registered` inside an open
  transaction; session B's expire blocks; A commits; B returns `registered`);
  `the_window_cannot_claim_settle_purge_or_write_a_failure` (42501 each);
  `settle_refuses_an_illegal_transition` (`registered` without a patient; any settle of a non-pending row);
  `purge_deletes_only_old_settled_rows`;
  `a_failure_row_goes_when_its_draft_goes` (cascade).
- [ ] **Step 2: Run** `cargo test -p cairn-node --test terminal_service_doors -- --nocapture`. Expected: FAIL
  (functions missing).
- [ ] **Step 3: Write `db/083_terminal_service.sql`**; add it to `db.rs`'s list; bump `SCHEMA_GENERATION` to 83.
- [ ] **Step 4: Run** the step 2 command plus `floor_execute_grants`, `window_role_grants`, `window_role_grants_notes`,
  `migration_replay_widening`, and the `db.rs` generation unit test. Expected: PASS, with no `skipped:` line.
- [ ] **Step 5: Commit** `feat(#909): db/083 — the registration queue and the failed-commit fact`.

### Task 3: `register_patient` runs inside its caller's transaction

**Files:**
- Modify: `crates/cairn-node/src/patient/register.rs` (966 lines: move the new function and its tests to
  `patient/register_tx.rs` rather than grow it)
- Test: `crates/cairn-node/tests/register_in_tx.rs` (DB-gated)

**Interfaces:**
- Produces: `pub async fn register_patient_in_tx(tx: &tokio_postgres::Transaction<'_>, sk: &SigningKey, kid: &str,
  node_origin: &str, name: Option<&str>, query: &SearchQuery, displayed: &CandidateList) -> anyhow::Result<Committed<Uuid>>`.
  It does every precondition, ticks the HLC on `tx` and submits each event, but never commits; the caller commits.
  `register_patient_returning_event` becomes: open a transaction, call it, commit with the existing #815
  outcome-unknown mapping. Its behaviour is otherwise unchanged.

- [ ] **Step 1: Write the failing tests** `a_rolled_back_registration_leaves_no_chart_and_no_event` (call `_in_tx`,
  then drop the transaction; no `event_log` row and no projection row for the patient) and
  `register_in_tx_then_commit_equals_register_patient` (same events, same contributors).
- [ ] **Step 2: Run** `cargo test -p cairn-node --test register_in_tx -- --nocapture`. Expected: FAIL.
- [ ] **Step 3: Implement** by moving the body. The HLC ticks move inside the transaction, as `note::commit` already
  does: a rollback now also rolls the tick back.
- [ ] **Step 4: Run** the step 2 command plus every existing registration test (`cargo test -p cairn-node --test
  'patient_register*' --test funnel_*`; list them with `ls crates/cairn-node/tests | grep -i regist`). Expected: PASS.
- [ ] **Step 5: Commit** `refactor(#909): registration can run inside a caller's transaction`.

### Task 4: the service serves one registration request

**Files:**
- Create: `crates/cairn-node/src/terminal_service/mod.rs`, `…/registration.rs` (+ `pub mod terminal_service;`)
- Test: `crates/cairn-node/tests/terminal_service_registration.rs` (DB-gated); unit tests in `registration.rs`

**Interfaces:**
- Consumes: Task 2's doors, Task 3's `register_patient_in_tx`,
  `cairn_node::patient::search::search_patients(&impl GenericClient, &SearchQuery, &str) -> anyhow::Result<CandidateList>`.
- Produces:
  ```rust
  pub fn unseen_candidates(fresh: &CandidateList, displayed: &CandidateList) -> Vec<Uuid>; // pure
  pub const NEW_MATCH_REFUSAL: &str;
  pub enum Served { Idle, Registered { request: Uuid, patient: Uuid }, Refused { request: Uuid, reason: String } }
  pub async fn serve_one(client: &mut Client, sk: &SigningKey, node_origin: &str) -> anyhow::Result<Served>;
  ```
  `unseen_candidates` returns the charts in `fresh.displayed_charts()` that are absent from
  `displayed.displayed_charts()`. A displayed chart that is no longer found is not unseen. `NEW_MATCH_REFUSAL` =
  `"a new possible match has appeared since the search — search again"`.
  `serve_one`: in one transaction: claim (none → `Idle`); decode `query`/`displayed`/`name` (a decode error → settle
  `refused` with the error and commit); re-run the search with the row's `today`; any unseen → settle `refused` with
  `NEW_MATCH_REFUSAL`; else `register_patient_in_tx` and settle `registered`; commit. A registration error that is a
  floor refusal settles `refused` with its legible text. Any other error rolls back and returns `Err` (the request
  stays pending).

- [ ] **Step 1: Write the failing unit tests** `nothing_unseen_when_fresh_is_a_subset`,
  `a_chart_found_now_but_not_shown_is_unseen`, `a_chart_shown_but_gone_is_not_unseen`.
- [ ] **Step 2: Write the failing DB tests:** `an_honest_request_registers_and_settles_registered`;
  `a_forged_saw_no_one_is_refused_when_someone_matches` (seed a matching patient, submit `displayed` = empty, expect
  `refused`/`NEW_MATCH_REFUSAL` and no new chart); `an_expired_request_is_never_served`;
  `a_poison_request_is_refused_and_the_next_one_is_served` (raw `INSERT` via the owner of a malformed `query`);
  `two_servers_register_once` (two `serve_one` calls on two connections, concurrently, one request: one `Registered`
  and one `Idle`, one chart); `a_dropped_transaction_leaves_the_request_pending` (inject a failure after the claim,
  for example by dropping `node_unwrap_key`'s grant per the SET ROLE seam, or a test-only hook; then the request is
  still `pending` and a later `serve_one` registers it once).
- [ ] **Step 3: Run** `cargo test -p cairn-node --lib terminal_service && cargo test -p cairn-node --test
  terminal_service_registration -- --nocapture`. Expected: FAIL.
- [ ] **Step 4: Implement** `registration.rs`.
- [ ] **Step 5: Run** the step 3 commands. Expected: PASS, with no `skipped:`.
- [ ] **Step 6: Commit** `feat(#909): the terminal service registers a request, re-running its search first`.

### Task 5: the window registers through the queue

**Files:**
- Create: `crates/cairn-node/src/patient/register_request.rs` (the client half; `pub mod` in `patient/mod.rs`)
- Modify: `cairn-gui/cairn-gui-live/src/funnel.rs` (`PatientRegistration for LiveData`), `cairn-gui/cairn-gui-live/src/lib.rs`
  (`LiveData` holds `node_kid: String`, no `node_sk`; `sharing(db, identity)` takes the kid from `identity.pubkey_hex`)
- Test: unit tests in `register_request.rs`; `cairn-gui/cairn-gui-live` DB tests (where the existing funnel tests live)

**Interfaces:**
- Produces:
  ```rust
  pub enum RequestStatus { Pending, Registered(Uuid), Refused(String), Expired }
  pub async fn submit(client: &impl GenericClient, request: Uuid, name: Option<&str>, query: &SearchQuery,
                      displayed: &CandidateList, today: &str) -> anyhow::Result<RequestStatus>;
  pub async fn status(client: &impl GenericClient, request: Uuid) -> anyhow::Result<RequestStatus>;
  pub async fn expire(client: &impl GenericClient, request: Uuid) -> anyhow::Result<RequestStatus>;
  pub enum Next { Wait, Expire, Done(RequestStatus) }
  pub fn next_step(status: &RequestStatus, waited: Duration, deadline: Duration) -> Next; // pure
  pub const SERVICE_NOT_RUNNING: &str;
  pub const POLL: Duration; // 200 ms
  pub const DEADLINE: Duration; // 10 s — must equal db/083's interval (a test pins both)
  ```
  `LiveData::register` takes `self.db.lock()` **per call** (submit, each poll, expire), never across a wait.
  Mapping: `Registered(id)` → `Ok(id)`; `Refused(NEW_MATCH_REFUSAL)` → `DataError::Stale`; any other `Refused(t)` →
  `DataError::Refused(t)`; `Expired` → `DataError::NotProvisioned(SERVICE_NOT_RUNNING)`; a lost connection or a
  failed poll after `submit` succeeded → `DataError::OutcomeUnknown` (Review Focus 1). `SERVICE_NOT_RUNNING` =
  `"nothing was registered: registration needs the node's terminal service (cairn-node terminal-service), which did
  not answer; tell the site's operator"`. The attestation goes back in every `Err`, as today.
  The port gains `today` where `register` needs it; the funnel already holds the search's `today`. If it does not,
  the funnel stores it with the search (`NamedAttestation`).

- [ ] **Step 1: Write the failing unit tests:** `next_step` table — pending before the deadline → `Wait`; pending at or
  after → `Expire`; any settled → `Done`; and `deadline_matches_db_083` (reads `db/083_terminal_service.sql`
  with `include_str!` and finds `interval '10 seconds'`, and `DEADLINE == 10 s`).
- [ ] **Step 2: Write the failing DB tests:** `registers_when_the_service_runs` (spawn a task looping `serve_one`
  as a `cairn_terminal_service` login); `says_service_not_running_and_registers_nothing_when_it_does_not`;
  `a_new_match_is_stale_and_keeps_the_attestation`; `a_lost_connection_after_submit_is_outcome_unknown` (cut the
  backend with `pg_terminate_backend` between submit and the first poll).
- [ ] **Step 3: Run** them. Expected: FAIL.
- [ ] **Step 4: Implement** `register_request.rs` and the `LiveData` changes.
- [ ] **Step 5: Run** the step 3 tests plus the whole `cairn-gui` tree's gate (`cargo test` in `cairn-gui/`,
  DB-gated). Expected: PASS.
- [ ] **Step 6: Commit** `feat(#909): the window registers through the node's terminal service`.

### Task 6: the service runs the quarantine commit and records failures

**Files:**
- Create: `crates/cairn-node/src/terminal_service/commit.rs`
- Test: `crates/cairn-node/tests/terminal_service_commit.rs` (DB-gated)

**Interfaces:**
- Consumes: `note::commit::{overdue_drafts, commit_overdue}` (unchanged; `commit_overdue` keeps `&SigningKey`), and
  Task 2's failure doors.
- Produces: `pub async fn commit_tick(client: &mut Client, sk: &SigningKey, node_origin: &str) ->
  anyhow::Result<CommitReport>`. It runs one pass over every chart: each draft is committed in its own call; a failure
  calls `cairn_note_draft_commit_failed(draft, true)` and a success calls it with `false`. Each failed draft is
  logged to stderr by id and cause only, never text or name, and each stuck draft once per process run (move
  `newly_stuck` from the window's `notes_commit.rs`, as is). The commit stays one door call per draft, so a
  clinician's press waits behind one draft at most.

- [ ] **Step 1: Write the failing tests:** `an_overdue_draft_goes_in_node_signed` (graded `Asserted`, signer = node kid);
  `a_failing_commit_is_recorded_and_a_later_success_clears_it` (stuck writer → `cairn_note_drafts_commit_failed`
  lists it; re-enrol, tick, the list is empty and the note exists); `a_not_yet_due_draft_is_untouched`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --test terminal_service_commit -- --nocapture`. Expected: FAIL.
- [ ] **Step 3: Implement** `commit.rs`.
- [ ] **Step 4: Run** the step 2 command plus `note_commit`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): the terminal service commits overdue drafts`.

### Task 7: the `terminal-service` and `provision-terminal-service-role` verbs

**Files:**
- Create: `crates/cairn-node/src/terminal_service/run.rs`
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` arms, dispatch only), `crates/cairn-node/src/window_access.rs`
  (generalise `provision_window_login` over the group role)
- Test: `crates/cairn-node/tests/provision_terminal_service_role.rs` (DB-gated, modelled on `provision_window_role.rs`);
  unit tests in `run.rs`

**Interfaces:**
- Produces:
  - `pub enum LoginGroup { Window, TerminalService }` with `fn role(&self) -> &'static str` (`cairn_window` /
    `cairn_terminal_service`). `provision_login(db, role, password, group)` replaces `provision_window_login`, which
    stays as a one-line wrapper so existing callers and tests are untouched. `existing_role_refusal` names the
    group rather than the literal `cairn_window`.
  - Verb `provision-terminal-service-role --role <name>` (default `cairn_terminal_service_login`), password from
    `CAIRN_TERMINAL_SERVICE_DB_PASSWORD`, with the same refusals, order and `trust`/stale-hba checks as
    `provision-window-role`.
  - Verb `terminal-service` (global `--conn`, `--key`): `require_local(&cli.conn)`, then
    `load_signing_key(&cli.key, false)` (never prompt, fail fast, as `serve`), and refuses the schema-stamp mismatch
    as the window does (`window_schema_verdict`). Then `run::run(conn, sk, origin)` forever.
  - `pub fn due(now: Instant, last: &Schedule) -> Due` (pure): which of {requests every 250 ms, commits every 60 s,
    purge every hour} is due. `run` loops on it, reopening a lost connection with a 1 s back-off. Each job's error is
    logged and never ends the loop.

- [ ] **Step 1: Write the failing tests:** `due` unit table (first tick runs requests only; 60 s → commits; 3600 s →
  purge); `provisioned_service_login_is_a_member_of_cairn_terminal_service_only`;
  `an_existing_window_login_is_not_taken_over_as_the_service_login`; and `terminal_service_refuses_a_remote_database`
  (run the binary with `host=10.0.0.1`; non-zero exit and the `require_local` sentence).
- [ ] **Step 2: Run** them. Expected: FAIL.
- [ ] **Step 3: Implement** `run.rs`, the `window_access` generalisation, and the two `Cmd` arms.
- [ ] **Step 4: Run** the step 2 tests plus `provision_window_role`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): cairn-node terminal-service and its own login`.

### Task 8: the window drops the node key and its commit runner

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (remove `--key`, `keystore::load`, the `run_ticks` spawn),
  `state.rs` (`node_sk: Option<SigningKey>` → `node_kid: Option<String>`; remove `commit_failed`),
  `notes_commit.rs` (keep `due_words` and its copy only; delete `commit_overdue_impl`, `hold`, `run_ticks`,
  `pause_change`, `newly_stuck`), `notes.rs:179-181` (the failed set comes from
  `cairn_note_drafts_commit_failed(patient)` via a new `cairn_node::note::commit::drafts_commit_failed(client,
  patient) -> anyhow::Result<HashSet<Uuid>>`), and every file that read `state.node_sk` (it passes
  `NodeKey::Public(kid)`).
- Modify tests: `notes_commit_tests.rs` (drop the runner source guards), and every window test fixture building
  `AppState` with a node key.
- Test: `cairn-gui/cairn-gui-tauri/src/node_key_guard_tests.rs`

**Interfaces:**
- Consumes: Task 1's `NodeKey`, Task 2's `cairn_note_drafts_commit_failed`.
- Copy (exact, replacing the two runner sentences; ADR-0089 decision 12 is amended to match):
  - `OVERDUE` = `"overdue — not in the record yet; the node's terminal service puts it in unsigned at its next check"`
  - `FAILED` = `"overdue — could not go into the record by itself (the node's last try failed; the cause is in the
    terminal service's log); sign it, or tell the site's operator"`

- [ ] **Step 1: Write the failing guard tests** `the_window_never_loads_the_node_key` (no `keystore::load`, no
  `NodeKey::Full`, no `SigningKey::from_bytes` outside the human-key hold, and no `--key` arg across
  `cairn-gui-tauri/src` and `cairn-gui-live/src`, non-test files; use `crate::source_guard`) and
  `the_window_runs_no_quarantine_commit` (no `commit_overdue` or `run_ticks`). Update the `due_words` copy tests to
  the exact strings above.
- [ ] **Step 2: Run** `cargo test` in `cairn-gui/` for those tests. Expected: FAIL.
- [ ] **Step 3: Make the changes.** A notes read whose failed-set read fails still reads the notes (Review Focus 5 of
  slice 3b-2a stands); it logs and shows no FAILED.
- [ ] **Step 4: Run** the full `cairn-gui` gate (fmt, clippy `--locked -D warnings`, `cargo doc` with
  `RUSTDOCFLAGS=-D warnings`, DB-gated tests; memory: three-cargo-trees) and the root source guards DB-free. Expected:
  PASS.
- [ ] **Step 5: Commit** `feat(#909): no window holds the node key or runs the quarantine commit`.

### Task 9: ADR, spec, RUNBOOK, durable rules, tracking documents

**Files:**
- Create: `docs/spec/decisions/0109-the-terminal-service-and-the-keyless-window.md`
- Modify: `docs/spec/decisions/README.md` (index), `docs/spec/index.md` (v0.110 → **v0.111**), the spec aspect file
  ADR-0089 lives in (one paragraph pointing at 0109), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (new **§38**, and
  every window launch line loses `--key`), `docs/developers/durable-rules.md` (a "#909 slice A" block),
  `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0109:** D1, D6, D7, D8 as built; the amendments to ADR-0089 decisions 11 and 12, with their
  new copy, and to ADR-0084's statement that the window holds the node key; the stated limits (one service per node
  is assumed, a second is harmless; registration and timed commits need the service, the local window included;
  the gateway must refuse `cairn_terminal_service` members). Fact-check each sentence against the SQL and Rust before
  committing (memory: SDD evidence + ADR fact-check).
- [ ] **Step 2: Write RUNBOOK §38:** start the service under launchd/systemd; register with it running (median
  Register round trip against the 0.5 s budget); stop it and register (the not-running sentence, no chart); stop it
  mid-registration; a forged search via `psql` as the window login is refused; a draft past quarantine goes in within
  60 s; a stuck draft shows FAILED.
- [ ] **Step 3: Write the durable-rules block** (do-not-undo: the claim is the row lock; expire blocks behind it; the
  window never holds `NodeKey::Full`; `DEADLINE` equals db/083's interval; the failed fact is per draft in the DB, not
  window memory).
- [ ] **Step 4: Run** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`, `cargo test -p
  cairn-node --test paper_parity_plan_section`, and `python3 scripts/check_closing_keywords.py` on the PR body.
  Expected: clean.
- [ ] **Step 5: Commit** `docs(#909): ADR-0109, RUNBOOK §38, durable rules for slice A`.
