# The window's connection posture (#898 + #899) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
> **Every test command runs in the FOREGROUND** (a subagent waiting on a background job never wakes).

**Goal:** The window is never handed owner credentials, never signs over an over-powered connection, and never
talks to a database on another machine in clear text.

**Architecture:** One new `cairn-node` module, `conn_posture`, holds the two pure rules (is this connection
local-only? is this role over-powered?) and the one DB read behind the second. A new migration `db/062` defines the
window's group role. One new owner verb, `provision-window-role`, is both the install and the upgrade step: it
loads the schema, refuses a cluster that trusts anyone, and creates or refreshes the window's login. The window
stops loading the schema and checks the stamp instead.

**Tech Stack:** Rust (tokio-postgres 0.7, clap), PL/pgSQL, `postgres-protocol` (already in `Cargo.lock`; MIT/Apache-2.0) for a client-side SCRAM verifier.

**Spec:** [`docs/superpowers/specs/2026-11-01-window-connection-posture-898-899-design.md`](../specs/2026-11-01-window-connection-posture-898-899-design.md)

Paper-parity: not clinical-surface — connection and install posture; a correctly installed terminal's sign gesture is unchanged in steps and time (RUNBOOK §16's budget stands), and the new refusals appear only on a misconfigured install.

## Global Constraints

- **No override flag** for either refusal (design anti-goal). No `--allow-insecure-remote`, no env escape.
- **Never `SET ROLE` as the production fix**; tests may log in as a probe login (Task 3), never `SET ROLE`.
- Local means: every `host` is a Unix socket path or `localhost` / an IP in `127.0.0.0/8` / `::1`; every `hostaddr`,
  when given, is loopback. An empty host list counts as local (libpq's default is the socket).
- Over-powered means ANY of: `rolsuper`; member of the owner of any `public` table; `SELECT` on `vault_member`;
  `INSERT` on `event_log`; `EXECUTE` on `apply_remote_event(bytea, bytea, bytea, bytea)`, `submit_node_event(bytea)`
  or `cairn_vault_unseal(bytea, text)`.
- The window role is `cairn_window` (NOLOGIN group role, db/062). It inherits `cairn_agent`.
- Install refuses on ANY `pg_hba_file_rules` row whose `auth_method = 'trust'`, and on any row with a non-NULL
  `error`. A `peer` rule with no `map=` option is REPORTED (a printed warning), not refused — say so in the ADR.
- The window login's password reaches the server only as a SCRAM verifier (`postgres_protocol::password::scram_sha_256`), never in clear (server logs).
- Wording rules (durable-rules): say what the USER must do, name the lightest command, full stops not dashes.
- New `db/` file: guarded `CREATE ROLE` (no `IF NOT EXISTS` exists), replay-safe, `SCHEMA_GENERATION` → **62**.
- Refresh `cairn-gui/Cargo.lock` after any root-crate `Cargo.toml` change (three-lockfiles rule).
- A new `pub fn` in `crates/cairn-node/tests/common/mod.rs` must ALSO be added to the expected-helper array in
  `identity_scaffolding_shared.rs`.

## Review Focus

1. **A reopen after the role changed** (#807: an operator fixes the login while the window is open) — the
   over-power check runs on EVERY release, never cached, so a reopened connection is re-judged. Task 4 pins it by
   calling `release_for_act` twice on one client with a role change between.
2. **`host=localhost hostaddr=10.0.0.3`** — `hostaddr` wins for the TCP connect; a host-only check would pass a
   remote link. Task 1 pins it.
3. **The owner's login inheriting the owner role through a group** (`pg_has_role … 'USAGE'`, not `relowner =
   current_user`) — Task 4 pins a login that is merely a MEMBER of the owner role as over-powered.
4. **A wrong passphrase over an over-powered connection must not be counted** — the refusal comes before the vault
   is asked. Task 4 asserts `vault_attempt` is unchanged.
5. **An upgrade run of `provision-window-role` with no password** must keep the existing login and its password,
   not fail or blank it. Task 6 pins it.

---

### Task 1: `conn_posture::require_local` — refuse a non-local connection string

**Files:**
- Create: `crates/cairn-node/src/conn_posture.rs` (+ `mod conn_posture;` / `pub mod` in `crates/cairn-node/src/lib.rs`)
- Test: unit tests in `crates/cairn-node/src/conn_posture_tests.rs` (the crate's `#[path]` sibling pattern)
- Test: `crates/cairn-node/tests/cli_vault_refuses_remote.rs`
- Modify: `crates/cairn-node/src/main.rs` (the `VaultBootstrapSuperuser` and `VaultSetTerminalPassphrase` arms, before `connect`)

**Interfaces:**
- Produces: `pub fn require_local(conn: &str) -> anyhow::Result<()>` — `Err` text (never the conn string; it can
  carry a password): `"the node's database is on another machine ({host}), and this connection would carry
  passphrases and signing keys in clear text. Run this on the node's own machine (a Unix socket or localhost)."`
  where `{host}` is the first non-local host or hostaddr.

- [ ] **Step 1: Write the failing unit tests** — `local_forms_pass`: `"host=/tmp dbname=x"`, `"host=localhost"`,
  `"host=127.0.0.1"`, `"host=127.4.5.6"`, `"host=::1"`, `"dbname=x"` all `Ok`. `remote_forms_refuse`:
  `"host=10.0.0.3"`, `"host=db.example"`, `"host=localhost,10.0.0.3"` all `Err`, and the text contains `"10.0.0.3"` /
  `"db.example"`. `hostaddr_wins`: `"host=localhost hostaddr=10.0.0.3"` → `Err`; `"host=db.example hostaddr=127.0.0.1"`
  → `Ok`. `password_never_echoed`: `"host=10.0.0.3 password=s3cret"` → `Err` whose text lacks `"s3cret"`. An
  unparsable string → `Err`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib conn_posture` — FAIL (module missing).
- [ ] **Step 3: Implement** over `tokio_postgres::Config` (`get_hosts()` → `Host::Unix | Host::Tcp`, `get_hostaddrs()`); `localhost` matched case-insensitively; IP via `std::net::IpAddr::is_loopback`.
- [ ] **Step 4: Run** the unit tests — PASS.
- [ ] **Step 5: Write the failing CLI test** — run `CARGO_BIN_EXE_cairn-node --conn "host=192.0.2.1 dbname=x"
  vault-set-terminal-passphrase` with the passphrase from its env variable (see the arm's `resolve_new_vault_passphrase`),
  and the same for `vault-bootstrap-superuser` with its required args; assert non-zero exit and stderr contains
  `"another machine"`, within 5 s (proves it refused BEFORE connecting: 192.0.2.1 is TEST-NET, it would hang).
- [ ] **Step 6: Call `cairn_node::conn_posture::require_local(&cli.conn)?`** first in both arms. Run the CLI test — PASS.
- [ ] **Step 7: Commit** `feat(#899): the vault's console verbs refuse a database on another machine`.

### Task 2: `db/062` — the `cairn_window` group role

**Files:**
- Create: `db/062_window_role.sql`, `db/tests/062_window_role_test.sql` (with `\ir _scratch_database_guard.sql`)
- Modify: `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION` 61 → 62), `crates/cairn-node/src/db.rs` (`SCHEMA` list gains `062`)

**Interfaces:** Produces the role `cairn_window` (NOLOGIN), `GRANT cairn_agent TO cairn_window`,
`GRANT SELECT ON local_node, node_schema TO cairn_window`. Task 3 adds the grants its inventory finds HERE.

- [ ] **Step 1: Write the failing SQL test** — asserts as `cairn_window` (via `has_*_privilege('cairn_window', …)`):
  SELECT on `local_node`, `node_schema` TRUE; SELECT on `vault_member`, `vault_attempt`, `vault_terminal` FALSE;
  EXECUTE on `cairn_vault_unseal(bytea, text)`, `apply_remote_event(bytea, bytea, bytea, bytea)`,
  `submit_node_event(bytea)` FALSE; INSERT on `event_log` FALSE; `pg_has_role('cairn_window','cairn_agent','MEMBER')` TRUE;
  `rolcanlogin` FALSE.
- [ ] **Step 2: Run** it via `scripts/run-db-gated-tests.sh`'s SQL step (pass `PGHOST=127.0.0.1 PGPORT=5532`, #862) — FAIL.
- [ ] **Step 3: Write `db/062`** — a junior-readable header (why the role exists, why it inherits `cairn_agent` and
  NOT `cairn_node`, link the design); guarded `CREATE ROLE`; the grants, each with a one-line reason.
- [ ] **Step 4: Bump the generation and the `SCHEMA` list.** Run the SQL test and `cargo test -p cairn-event` (the
  fs-derived generation guard) and `cargo test -p cairn-node --lib` — PASS.
- [ ] **Step 5: Commit** `feat(#898): db/062 — the window's own group role, never the owner's`.

### Task 3: The window's day under its own login — the grant inventory

**Files:**
- Modify: `crates/cairn-node/tests/common/mod.rs` (+ `identity_scaffolding_shared.rs`'s expected array)
- Modify: `cairn-gui/cairn-gui-live/tests/common/mod.rs` (`reopening` and `connect_for_live` connect as the probe login; the setup `reader` stays owner)
- Create: `crates/cairn-node/tests/window_role_grants.rs`
- Modify: `db/062_window_role.sql` (each missing grant found, with its reason)

**Interfaces:**
- Produces (both `common` modules): `pub async fn window_login(owner: &Client, cs: &str) -> String` — ensures a
  `cairn_window_probe` LOGIN role (no password: test clusters trust) granted `cairn_window`, and returns
  `format!("{cs} user=cairn_window_probe")`. Pin with a unit test that tokio-postgres keeps the LAST `user=`.

- [ ] **Step 1: Write `window_role_grants.rs`** — as the probe login, one fixture walk through every entry point the
  window calls (list: `grep -rhoE 'cairn_node::[a-z_]+(::[a-z_]+)+' cairn-gui/cairn-gui-tauri/src cairn-gui/cairn-gui-live/src`):
  `identity::load_local`; `patient::register::register_patient`, `search::search_patients`,
  `candidate_read::candidates_by_id`, `person::{person_charts, chart_identities}`, `compare::{chart_facts, cross_vetoes}`,
  `edges::{record_edges, record_doubts, record_unverified_links, record_is_doubted}`; `medication::{read::list_patient_medications,
  signoff::sign_off_medication_list, cease_medication, withhold_medication, release_medication_holds}`;
  `chart_link::{link_charts_as_shown, unlink_charts_as_shown_from, confirm_link_as_shown, standing_link, shown}`;
  `duplicate_review::worklist`; `duplicate_check::status_line`; `ui_timing::record_gesture`;
  `vault::{terminal_phc, staff_id_taken, enrol_member}`. Each asserts the call is not a `42501`
  (`sqlstate_of`-style: `insufficient_privilege`); a call's ordinary refusal is fine. Reuse the root kit's fixtures; build no new ones.
- [ ] **Step 2: Run** it (foreground, `CAIRN_TEST_PG` set; see the long-gates memory: exec the printed test binary if cargo stalls) — expect FAILs naming objects.
- [ ] **Step 3: For each `42501`, add the narrowest grant to `db/062`** with a one-line reason. Never a grant on a vault table, a peer door, or `event_log` INSERT (Task 2's test refuses them).
- [ ] **Step 4: Switch `cairn-gui-live`'s `reopening` / `connect_for_live` to `window_login`.** Run `cairn-gui-live`'s DB suites (`cd cairn-gui && cargo test -p cairn-gui-live`, `CAIRN_TEST_PG` set) — PASS, and no `skipped:` in `--nocapture`.
- [ ] **Step 5: Commit** `test(#898): the window's whole day runs under its own login`.

### Task 4: Over-powered connections release no key

**Files:**
- Modify: `crates/cairn-node/src/conn_posture.rs` (+ tests), `crates/cairn-node/src/vault.rs` (`Release`, `release_for_act`)
- Modify: `cairn-gui/cairn-gui-tauri/src/signer.rs` (`KeyRefusal`, `from_release`), `cairn-gui/cairn-gui-tauri/src/enrol.rs` (its mapping of a release)
- Modify: `crates/cairn-node/tests/vault_lib.rs`, `cairn-gui/cairn-gui-tauri/src/enrol_tests.rs` (release as `window_login`)
- Test: `crates/cairn-node/tests/conn_posture_db.rs`

**Interfaces:**
- Produces: `pub struct PostureFacts { pub superuser: bool, pub owns_cairn_table: bool, pub reads_vault: bool,
  pub inserts_event_log: bool, pub runs_peer_door: bool }`; pure `pub fn over_powered(f: &PostureFacts) ->
  Option<&'static str>` (the first reason, plain words); `pub async fn read_posture(db: &Client) -> anyhow::Result<PostureFacts>`
  (ONE query, every fact judged over EVERY role the login is a member of — `pg_has_role(current_user, r.oid, 'MEMBER')`, `bool_or` — because a non-inheriting member can still `SET ROLE` into it (Task 4 review); `owns_cairn_table` = any `public` relation whose owner the login is a MEMBER of;
  `runs_peer_door` also covers `cairn_vault_unseal`).
- Produces: `Release::OverPowered { why: &'static str }`; `KeyRefusal::OverPowered`, sentence: `"This workstation is
  connected to the node with more access than a window may have, so it cannot sign. Nothing was signed. Ask your IT
  support to run cairn-node provision-window-role and start this window with its login."`

- [ ] **Step 1: Unit tests for `over_powered`** — all-false → `None`; each field alone → `Some`.
- [ ] **Step 2: DB tests** in `conn_posture_db.rs`: owner connection → `Some`; `window_login` → `None`; a login
  granted `cairn_node` → `Some` (peer door); a login that is a MEMBER of the owner role → `Some` (Review Focus 3);
  `release_for_act` as owner with a WRONG passphrase → `Release::OverPowered` and `vault_attempt` unchanged (Review
  Focus 4); one client, `release_for_act` → `OverPowered`, then the owner `REVOKE`s the extra membership (the
  member-of-owner login) and the same client's next call is judged again: no longer `OverPowered` (Review Focus 1).
- [ ] **Step 3: Run** — FAIL.
- [ ] **Step 4: Implement** `read_posture` + `over_powered`; `release_for_act` calls them FIRST, before any vault query, returning `Release::OverPowered`. Map it in `from_release` and enrol; the compiler lists every match.
- [ ] **Step 5: Move `vault_lib.rs`'s and `enrol_tests.rs`'s releases onto `window_login`** (they release as owner today and will now be refused). Run both suites + the new one — PASS.
- [ ] **Step 6: Commit** `feat(#898): no key is released over a connection that could read the vault`.

### Task 5: The window never loads the schema, and refuses a remote database

**Files:**
- Modify: `crates/cairn-node/src/db.rs` (extract `read_schema_stamp`; `connect_and_load_schema` calls it — a pure move; `LOADER_BUILD` becomes `pub(crate)` for the verdict)
- Modify: `crates/cairn-node/src/conn_posture.rs` (+ tests): `window_schema_verdict`
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (`build_live_state`), `cairn-gui/cairn-gui-live/src/lib.rs:84` (its doc names `connect_and_load_schema`)
- Test: a source guard in `cairn-gui/cairn-gui-tauri/src/` using `source_guard.rs`

**Interfaces:**
- Produces: `pub async fn read_schema_stamp(client: &Client) -> anyhow::Result<Option<SchemaStamp>>` (db.rs);
  pure `pub fn window_schema_verdict(stamp: Option<&SchemaStamp>) -> Result<(), String>` — `Ok` iff
  `replay_can_be_skipped(stamp, embedded_schema_version(), LOADER_BUILD, &embedded_schema_hash())`; a recorded
  version above ours → `"This window is older than the node's database (generation {db} against {ours}). Install
  the window that matches the node."`; anything else → `"The node's database has not been prepared for this window
  (generation {db or 'none'} against {ours}). At the node, run cairn-node provision-window-role with the owner's
  connection, then start this window again."`

- [ ] **Step 1: Unit tests for `window_schema_verdict`** — matching stamp → `Ok`; newer → text has `"older than"`; older, `None`, a NULL hash and a `cairn-sync` `loader_build` → text has `"provision-window-role"`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement**; extract `read_schema_stamp` with no behaviour change (the existing `schema_replay_gate.rs` stays green).
- [ ] **Step 4: `build_live_state`** — `require_local(&cli.conn)?` first, then `db::connect`, `read_schema_stamp`, `window_schema_verdict` (a launch refusal: `anyhow::bail!` with the text), then `identity::load_local`. No `connect_and_load_schema` left in `cairn-gui-tauri/src`.
- [ ] **Step 5: Source guard** — `build_live_state`'s body calls `require_local` before any `connect`, and the crate's non-test source contains no `connect_and_load_schema`. Mutation-check both halves.
- [ ] **Step 6: Run** the cairn-gui gate (clippy, test, `cargo doc` with `RUSTDOCFLAGS=-D warnings`, per the three-lockfiles memory) — PASS.
- [ ] **Step 7: Commit** `feat(#898, #899): the window checks the schema, never loads it, and refuses a remote database`.

### Task 6: `cairn-node provision-window-role` — install and upgrade, refusing a trusting cluster

**Files:**
- Create: `crates/cairn-node/src/window_access.rs` (+ `window_access_tests.rs`)
- Modify: `crates/cairn-node/src/main.rs` (the `ProvisionWindowRole { role }` verb), `crates/cairn-node/Cargo.toml` (`postgres-protocol`, same version as the lock), `cairn-gui/Cargo.lock`
- Test: `crates/cairn-node/tests/provision_window_role.rs`

**Interfaces:**
- Produces: `pub struct HbaRule { pub line_number: i32, pub kind: String, pub auth_method: String, pub options:
  Option<Vec<String>>, pub error: Option<String> }`; pure `pub fn hba_verdict(rules: &[HbaRule]) -> Result<Vec<String>, String>`
  (`Err` names every refused line: `"line {n}: '{kind}' connections are let in with no password (trust). Change it
  to scram-sha-256 in pg_hba.conf, reload the server, and run this again."`; `Ok` carries the peer-without-map
  warnings); `pub async fn read_hba_rules(db) -> anyhow::Result<Vec<HbaRule>>` (a permission error →
  `"could not read this server's access rules; run this step as a database superuser"`);
  `pub async fn provision_window_login(db, role: &str, password: Option<&str>) -> anyhow::Result<()>` (validates
  `role` with `db::is_safe_role_ident`; creates the LOGIN role if absent, which REQUIRES a password; grants
  `cairn_window`; with a password, `ALTER ROLE … PASSWORD '<scram verifier>'`; with none on an existing role, leaves it).
- The verb: `require_local` → `connect_and_load_schema` → `read_hba_rules` → `hba_verdict` (print warnings) →
  password from `CAIRN_WINDOW_DB_PASSWORD` or an `rpassword` prompt when creating (as the vault verbs resolve theirs) →
  `provision_window_login` → print `start the window with --conn "<this connection> user={role} password=…"`.

- [ ] **Step 1: Unit tests for `hba_verdict`** — scram/md5/reject/cert → `Ok(vec![])`; one `trust` → `Err` naming its line; two → both lines; `error` set → `Err`; `peer` with no `map=` → `Ok` with one warning; `peer` with `map=x` → none.
- [ ] **Step 2: DB tests** (owner = the test superuser): `provision_window_login` creates the role, `pg_authid.rolpassword` starts `SCRAM-SHA-256$` (Review Focus: never clear), member of `cairn_window`; a re-run with `None` keeps the verifier (Review Focus 5); a new role with `None` → `Err`. CLI: the binary against `CAIRN_TEST_PG` (a trust cluster) exits non-zero naming `trust` — the refusal is the expected result here.
- [ ] **Step 3: Run** — FAIL. **Step 4: Implement.** **Step 5: Run** — PASS, plus `cargo deny check` (the new direct dep).
- [ ] **Step 6: Commit** `feat(#898): provision-window-role installs and upgrades, and refuses a cluster that trusts anyone`.

### Task 7: ADR, spec, RUNBOOK

**Files:** Create `docs/spec/decisions/0084-the-window-never-holds-the-owners-keys.md`; modify `docs/spec/index.md`
(v0.85 → **v0.86**), `docs/spec/decisions/README.md` (row), `mkdocs.yml` (nav), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`,
`docs/developers/durable-rules.md` (this slice's block).

- [ ] **Step 1: ADR-0084** — D1–D5 as built; amends ADR-0083's "holds only for a `cairn_agent` connection, owed
  #898" and its `NoTls` caveat; states what is still NOT guaranteed (the daemon's `provision_runtime_role` login has
  no password and is outside D1a; `cairn-sync`/matcher/daemon `NoTls` stays #102 / #909; a `peer` rule without a map
  is only reported; the window no longer runs the connect-time deferred re-adjudication — the daemon does). Fact-check
  every sentence against the code (sdd-evidence memory).
- [ ] **Step 2: RUNBOOK** — §0: the measurement cluster must use `scram-sha-256` (provision refuses `trust`), then
  `cairn-node --conn "$CONN" provision-window-role --role cairn_window_login` and a `WINDOW_CONN`; every
  `-p cairn-gui-tauri -- --conn "$CONN"` becomes `"$WINDOW_CONN"`; console steps keep `$CONN`. Walk each step against
  the code (an operator procedure is a claim).
- [ ] **Step 3: durable-rules block** — the two do-not-undo rules: the over-power check runs per release, never
  cached; the window never calls `connect_and_load_schema`.
- [ ] **Step 4: Build docs** `uv run --with-requirements docs/requirements.txt -- mkdocs build` — no warnings for the new pages.
- [ ] **Step 5: Commit** `docs(#898, #899): ADR-0084, spec v0.86, RUNBOOK on the window's own login`.

### Final gate (controller, not a subagent)

Re-run every CI gate AFTER the last edit, in CI's order: `cargo fmt --check` (both trees), clippy (both trees),
`scripts/run-db-gated-tests.sh` (background, ~2 h), the cairn-gui gate, `cargo doc -D warnings`, `cargo deny`,
`scripts/check_closing_keywords.py` on the PR body. Then HANDOVER/ROADMAP, then mark PR #911 ready.
