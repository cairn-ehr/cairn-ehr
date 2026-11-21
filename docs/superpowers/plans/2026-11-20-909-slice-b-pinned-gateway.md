# #909 slice B — the pinned terminal gateway — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A window on another machine of the facility's network can read and sign through an encrypted link pinned
in both directions, without Postgres being restarted, reloaded or reconfigured.

**Architecture:** The terminal service gains a third job, the **gateway**. It accepts Postgres's own `SSLRequest`,
does mutual TLS 1.3 with the node's identity key against a node-local **terminal trust set**, checks the startup's
login, and pipes bytes to Postgres over loopback. SCRAM still runs end to end, from the window to Postgres. On the
window side, a **`Link`** (`Local` or `Pinned`) opens every database connection, the reopen included. A pinned
`Link` carries the terminal's own Ed25519 key and the node's full public key, which is pinned at first launch from a
typed fingerprint. Owner console verbs make the gateway's route record and admit, revoke and list terminals. The
registration advisory lock becomes a row lock that only the service and the owner can take.

**Tech Stack:** Rust (tokio, tokio-postgres, rustls 0.23 / tokio-rustls 0.26, already locked), socket2 0.6 (already in
all three lockfiles, new as a *direct* dependency of `cairn-node`), PL/pgSQL (PostgreSQL 18). No new crate.

**Spec:** [`docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md`](../specs/2026-11-16-pinned-terminal-link-909-design.md)
(D2, D3, D4, D5, and §5's restore question; D1/D6/D7/D8 are slice A, ADR-0109; the rollout, §6, is slice C).
Read ADR-0109's "What is not guaranteed" first: two of its bullets are this slice's obligations.

## Global Constraints

- **No Postgres restart, reload, or `postgresql.conf` / `pg_hba.conf` edit**, by any verb or test (design goal 3,
  anti-goal 2). The gateway reaches Postgres only by a route the cluster already admits.
- **No override flag** lets a window reach a remote database without the pinned link, and nothing falls back to plain
  text (anti-goal 3). `sslmode` is forced to `require` on a pinned connection, whatever the string says.
- **TLS 1.3 only, mutual, with no fallback verifier**, on both the gateway and the window's connector (D2 step 2).
  The existing `transport::{server_config, client_config}` keep their versions; sync is unchanged.
- **The gateway refuses a startup** whose `user` is not a LOGIN member of `cairn_window`; or whose `user` is a member
  of `cairn_terminal_service` (ADR-0109); or whose `user` is over-powered by `conn_posture::over_powered`; or whose
  `database` is not the node's own; or that carries a `replication` parameter. The refusal is an `ErrorResponse`,
  SQLSTATE **28000**, message exactly `this login may not connect through the node's terminal gateway`, naming no
  role.
- Plain-text startup, plain-text `CancelRequest`, or any first message other than `SSLRequest` / `GSSENCRequest`: the
  connection is closed with **no reply** (D2 step 1).
- Numbers (designer's calls, recorded in the ADR): pre-startup deadline **10 s** per connection, **counted from
  `accept`**, so it covers the wait for a permit as well as the opening, handshake and startup; at most **64**
  connections at once before startup, and at most **64** more waiting for a permit (a connection beyond those **128**
  is closed at once with no reply, so a LAN host opening idle sockets cannot exhaust the service's file descriptors);
  startup packet cap **10 000 bytes** (Postgres's own `MAX_STARTUP_PACKET_LENGTH`); trust-set refresh and revoke
  sweep every **1 s**; pending rows capped at **64**, newest kept; LAN-side TCP keepalive and user timeout from
  `cairn_db_keepalive::{KEEPALIVE_IDLE, KEEPALIVE_INTERVAL, KEEPALIVE_RETRIES, TCP_USER_TIMEOUT}` (30 s, 10 s, 3
  probes, 30 s; the user timeout is what ends a terminal that vanishes with a result still in flight, which keepalive
  alone does not, and it is Linux only).
- **The window's role can neither read nor write the trust set or the gateway record**, and cannot take the
  registration turn. Grants go to `cairn_terminal_service` (read) and the owner (write) only.
- New file `db/084_terminal_gateway.sql`; `SCHEMA_GENERATION` 83 → **84**, with an entry in `db.rs`'s list. Spec
  0.111 → **0.112**; new **ADR-0110**; RUNBOOK **§39**.
- Adding `socket2` as a direct dependency of `cairn-node` changes `cairn-node`'s entry in **both** `Cargo.lock` and
  `cairn-gui/Cargo.lock` (memory: three trees, three lockfiles). Commit both, and run the GUI tree's `--locked` clippy.
- House rules: TDD; files under 500 lines (`main.rs` is 8 047 lines: verb bodies go in modules, `main.rs` only
  sequences them); inline docs for a junior; test key material computed at runtime and never named
  `salt`/`nonce`/`iv` (house rule 6; check `crypto_sink_names_are_genuine.rs`'s `ALLOWED` list for every new
  crypto-bearing file); `CAIRN_ALLOW_DB_SKIP` unset for DB-gated runs.
- Guards:
  - Every new definer function gets `REVOKE ALL … FROM PUBLIC`. `floor_execute_grants.rs` does **not** see these
    (it checks only its named families), so `terminal_gateway_doors.rs` asserts it for each of them itself.
  - `window_role_grants.rs::the_windows_reads_and_registration_run_under_its_own_login` **changes**: it registers on
    a window login today, and after Task 1 the window cannot take the turn. That is intended: since ADR-0109 the
    window registers only through the queue. The test now pins that a window's direct `register_patient` is refused
    with `42501` (this closes the Rust path around the queue, not raw `submit_event`, which stays #1159). The other
    mirrors pass unchanged.
  - New DB reads follow their neighbours (`conn_posture.rs`, `terminal_service/`) and use `.context(…)`, not
    `LocalDbFault`, so `db_errors_stay_legible.rs` has nothing to bump.
  - If `db/084` decodes hex through `cairn_decode_hex_or_raise`, add `("084_terminal_gateway.sql", <n>)` to
    `hex_decode_helper.rs`'s per-file list.
  - Run every root source guard DB-free, since some read `cairn-gui/*/src`.

## Review Focus

1. **A terminal revoked mid-session.** Its open session must end within 2 s, **including a statement that is running**:
   closing the loopback socket alone does not stop a busy backend (Postgres notices a gone client only when it next
   reads or writes; `client_connection_check_interval` is 0 by default, and any session may set it back to 0), so the
   gateway also sends Postgres a `CancelRequest` with the key it saw at startup. The window's next reopen must say the
   terminal is not admitted and name the operator. It must not loop as if this were an outage. Pinned in Tasks 6 and 7.
2. **An unauthenticated LAN host floods the gateway** with fresh keys or half-open sockets. Setup must not be blocked
   for good: the newest-64 eviction and a relaunch re-record a real terminal. The 65th pre-startup connection waits,
   a waiting connection is closed when its 10 s from `accept` run out, the 129th is closed at once, an `accept` error
   (`EMFILE`) neither busy-loops nor ends the job loop, and the log gets one summary line a second, not one per
   attempt. Pinned in Tasks 1 and 7.
3. **A node key changes under a pinned terminal** (a restore onto a node with a new key, or a man in the middle). The
   window refuses at launch and on every reopen, naming the mismatch, with no override. Pinned in Task 6.
4. **A window login that is over-powered, or is a `cairn_terminal_service` member,** is refused at startup even with
   the correct password from an admitted terminal. Pinned in Task 4 (pure) and Task 7 (end to end).
5. **The gateway's own database connection drops** (a Postgres restart). New startups are refused while it is down,
   never forwarded unchecked, and the gateway recovers on its own. Pinned in Task 7.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a clinician at a ward desk, away from the records room, reading and writing the paper chart
  that was carried to them.
- **Steps:** paper *N* = 0 extra human acts to read or sign at a remote desk. Architecture-forced *M* = 0 per act: the
  link is set up once per terminal by an operator (not a clinical workflow, design §7), and every later read or sign
  is the same press as on the database's machine. UI bundling target *K* = 0. Registration stays *N* = *M* = *K* = 3
  (ADR-0109).
- **Time + cognitive load:** budget: on a wired facility LAN, opening a chart and pressing Sign each take at most
  **0.2 s** longer at the median through the gateway than on the database's machine. RUNBOOK §39 measures it; this
  slice is the first to expose it. The cognitive load is unchanged except two new launch sentences, *not admitted yet*
  and *the node's key changed*, both for an operator, not a clinician.

---

### Task 1: `db/084` — the trust set, the gateway record, the registration turn

**Files:**
- Create: `db/084_terminal_gateway.sql`
- Modify: `crates/cairn-node/src/db.rs` (generation list), `crates/cairn-event/src/schema_generation.rs` (83 → 84),
  `crates/cairn-node/src/terminal_service/registration.rs:62,189-190` (the advisory lock → the turn),
  `crates/cairn-node/src/patient/register.rs` (`register_patient_returning_event` takes the turn),
  `crates/cairn-node/tests/terminal_service_registration.rs` (its doc names the turn),
  `crates/cairn-node/tests/window_role_grants.rs` (the window's direct registration is now refused; Global
  Constraints, Guards)
- Test: `crates/cairn-node/tests/terminal_gateway_doors.rs` (new, DB-gated)

**Interfaces:**
- Produces (SQL). Every function below is `SECURITY DEFINER`, `search_path = public, pg_temp`, with `REVOKE ALL …
  FROM PUBLIC`; every table has `REVOKE ALL … FROM PUBLIC` and only the grants named:
  - table `terminal_trust(terminal_pubkey text PRIMARY KEY CHECK (terminal_pubkey ~ '^[0-9a-f]{64}$'), fingerprint
    text NOT NULL, label text, status text NOT NULL CHECK (status IN ('pending','active','revoked')), first_seen
    timestamptz NOT NULL, last_seen timestamptz NOT NULL, decided_by text, decided_at timestamptz)`. `SELECT` is granted
    to `cairn_terminal_service` only.
  - `cairn_terminal_fingerprint(p_pubkey text) RETURNS text IMMUTABLE`: the same string as
    `cairn_event::short_fingerprint` (the first 10 bytes of sha256 over the key bytes, as five upper-case 4-hex groups
    joined by `-`).
  - `cairn_terminal_attempt(p_pubkey text) RETURNS void` (EXECUTE: `cairn_terminal_service`): an unknown key is
    inserted as `pending`; for a known key only `last_seen` is updated, and its status is never changed. Then every
    `pending` row beyond the **64** newest by `last_seen` is deleted.
  - `cairn_terminal_admit(p_fingerprint text, p_label text) RETURNS text` (owner only): turns the **one** `pending`
    row with that fingerprint `active`, sets `label`, `decided_by = session_user`, `decided_at = now()`, and returns
    its key. It raises (P0001) on an empty label, on no match, on more than one match, or when the only match is not
    `pending`. A `revoked` row is final: a revoked terminal comes back only with a new key.
  - `cairn_terminal_revoke(p_fingerprint text) RETURNS text` (owner only): `pending` or `active` → `revoked`; it raises
    on no match or an ambiguous match.
  - table `terminal_gateway(only_row boolean PRIMARY KEY DEFAULT true CHECK (only_row), upstream text NOT NULL,
    checked_at timestamptz NOT NULL, checked_by text NOT NULL)`. `SELECT` is granted to `cairn_terminal_service`; it
    is written by the owner (Task 5).
  - table `registration_turn(only_row boolean PRIMARY KEY DEFAULT true CHECK (only_row))` with its one row seeded
    `ON CONFLICT DO NOTHING`, and `cairn_registration_take_turn(p_if_pending boolean DEFAULT false) RETURNS boolean`
    (EXECUTE: `cairn_terminal_service`; the owner by ownership). With `p_if_pending` and no `registration_request`
    that `cairn_registration_claim` would return (`pending`, deadline not passed), it returns `false` and locks
    nothing. Otherwise it runs `PERFORM 1 FROM registration_turn FOR UPDATE` and returns `true`. *Why the flag:* a
    row lock assigns a transaction id and writes WAL, and `serve_one` polls every 250 ms; an unconditional turn would
    turn an idle service into about four WAL writes a second, forever (on a Pi's SD card, wear for nothing). Today's
    advisory lock writes nothing. The re-check under the lock is the claim itself, so a request that arrives just
    after a `false` waits one poll.
- Removes: `terminal_service::registration::REGISTRATION_LOCK`.

- [ ] **Step 1: Write the failing tests** in `terminal_gateway_doors.rs`:
  `fingerprint_in_sql_matches_rust` (the key is built from `std::array::from_fn`, and the SQL string equals
  `short_fingerprint`); `an_attempt_records_pending_and_never_changes_a_decided_row` (insert, admit, attempt again:
  still `active`); `pending_is_capped_at_the_64_newest` (70 attempts leave 64 rows, the 6 oldest gone);
  `admit_refuses_unknown_ambiguous_revoked_and_empty_label`; `revoke_then_admit_is_refused`;
  `the_window_role_cannot_read_the_trust_set_or_take_the_turn` (as a `cairn_window` login, `SELECT` on
  `terminal_trust` / `terminal_gateway` and `SELECT cairn_registration_take_turn()` each fail with `42501`; use the
  SET ROLE seam, memory); `a_held_turn_blocks_the_service_but_a_window_advisory_lock_does_not` (connection A, as the
  owner, holds the turn in an open transaction; connection B's `serve_one` blocks until A commits. Then a window
  login holding `pg_advisory_xact_lock(0x4341_524E_5247)` does **not** block `serve_one`);
  `cli_registration_takes_the_turn` (while A holds the turn, `register_patient` on B does not return within 500 ms;
  it returns once A commits); `an_idle_serve_one_takes_no_turn` (no pending request: while A holds the turn,
  `serve_one` on B returns `Served::Idle` within 500 ms, which it cannot if it tried to take the turn; then, with a
  pending request, it blocks until A commits); `every_new_door_is_closed_to_public` (`has_function_privilege('public', …, 'EXECUTE')` is
  false for each new function, and `has_table_privilege('public', …, 'SELECT')` for each new table). In
  `window_role_grants.rs`, the window's `register_patient` call now expects `42501`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --test terminal_gateway_doors -- --nocapture`. Expected: FAIL (no
  `db/084`), with no `skipped:` line.
- [ ] **Step 3: Write `db/084_terminal_gateway.sql`** to the interfaces above (replay-safe: `CREATE TABLE IF NOT
  EXISTS`, `CREATE OR REPLACE FUNCTION`, idempotent grants). Bump the generation. In `serve_one`, replace the advisory
  lock with `SELECT cairn_registration_take_turn(true)` as its first statement; on `false`, roll back and return
  `Served::Idle` without claiming. In `register_patient_returning_event`, run `SELECT cairn_registration_take_turn()`
  right after `client.transaction()`. Delete `REGISTRATION_LOCK`, and fix every code comment that names it
  (`git grep REGISTRATION_LOCK -- crates cairn-gui` must come back empty; ADR-0109 is immutable and keeps the name,
  and `docs/developers/durable-rules.md:23` is rewritten in Task 10).
- [ ] **Step 4: Run** the new test plus `terminal_service_registration`, `register_in_tx`, `patient_register`,
  `floor_execute_grants`, `window_role_grants*` (the one call changed in Step 1), and the schema-generation guard.
  Expected: all PASS.
- [ ] **Step 5: Commit** `feat(#909): db/084 — the terminal trust set, the gateway record, the registration turn`.

### Task 2: TLS 1.3-only configs for the gateway and the terminal

**Files:**
- Modify: `crates/cairn-node/src/transport.rs` (a private `versions` parameter on the two builders; two new public
  functions; keep the file under 500 lines)
- Test: `crates/cairn-node/tests/transport.rs` (the existing pinned-mTLS tests; `src/transport.rs` has no test
  module)

**Interfaces:**
- Produces:
  - `pub fn gateway_server_config(sk: &SigningKey, trust: TrustStore) -> anyhow::Result<Arc<ServerConfig>>`: TLS 1.3
    only, mutual, pinned, with resumption off exactly as `server_config`.
  - `pub fn terminal_client_config(terminal_sk: &SigningKey, node_pin: TrustStore) -> anyhow::Result<Arc<ClientConfig>>`:
    TLS 1.3 only, presents the terminal key, pins the server through `node_pin`, resumption off.
  - `server_config` / `client_config` are unchanged in behaviour (they pass `rustls::DEFAULT_VERSIONS`).

- [ ] **Step 1: Write the failing tests:** `gateway_and_terminal_handshake_when_both_pinned` (`test_handshake` →
  `Ok`); `gateway_refuses_an_unpinned_terminal`; `terminal_refuses_an_unpinned_gateway`;
  `gateway_refuses_a_tls12_only_client` (a client built with `&[&rustls::version::TLS12]` fails the handshake).
- [ ] **Step 2: Run** `cargo test -p cairn-node --test transport`. Expected: FAIL (the functions are not defined).
- [ ] **Step 3: Implement** by threading `versions: &[&'static rustls::SupportedProtocolVersion]` into the two
  builders (`builder_with_provider(..).with_protocol_versions(versions)`), with `&[&rustls::version::TLS13]` for the
  new pair.
- [ ] **Step 4: Run** the same command plus `cargo test -p cairn-node --test trust_refresh` (sync's other pinned-mTLS
  suite). Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): TLS 1.3-only pinned configs for the gateway and the terminal`.

### Task 3: the gateway's wire, pure

**Files:**
- Create: `crates/cairn-node/src/terminal_service/gateway/mod.rs` (module docs: D2's five steps, in order),
  `crates/cairn-node/src/terminal_service/gateway/wire.rs`
- Modify: `crates/cairn-node/src/terminal_service/mod.rs` (`pub mod gateway;`)

**Interfaces:**
- Produces, in `gateway::wire` (all pure, no I/O):
  - `pub enum Opening { Ssl, Gss, Other }` and `pub fn opening(first8: [u8; 8]) -> Opening`. `SSLRequest` is length 8
    with code `80877103`, and `GSSENCRequest` is code `80877104`. Anything else, a plain startup or a `CancelRequest`
    (`80877102`) included, is `Other`.
  - `pub enum Inside { Startup(Startup), Cancel(Vec<u8>) }` and `pub fn inside(packet: &[u8]) -> Result<Inside,
    &'static str>`. `packet` is the whole message, length prefix included. A `Startup` needs protocol major 3 (any
    minor: PG18's 3.2 is forwarded as sent); a `Cancel` has code `80877102` and is 16 to 268 bytes (protocol 3.0's
    key is 4 bytes; 3.2's is variable, at most 256). Anything else is `Err`.
  - `pub struct Startup { pub raw: Vec<u8>, pub params: Vec<(String, String)> }` with `pub fn get(&self, key: &str) ->
    Option<&str>`. `raw` is replayed upstream byte for byte.
  - `pub const MAX_STARTUP: usize = 10_000;`
  - `pub fn error_response(sqlstate: &str, message: &str) -> Vec<u8>`: an `'E'` message with fields `S` and `V`
    `FATAL`, `C` sqlstate, `M` message, terminated.
  - For the revoke's cancel (Review Focus 1): `pub fn frame(buf: &[u8]) -> Option<(u8, usize)>` (a complete
    backend message's type byte and total length, or `None` until it is whole); `pub fn backend_key(msg: &[u8]) ->
    Option<Vec<u8>>` (the process id and key of a `'K'` message, as the bytes a cancel carries); and `pub fn
    cancel_request(key: &[u8]) -> Vec<u8>` (a `CancelRequest` carrying them, which `inside` accepts as `Cancel`).

- [ ] **Step 1: Write the failing unit tests** in `wire.rs`: each opening code → its variant, and a `[0;8]` → `Other`;
  a tokio-postgres-shaped startup (`user`, `database`, `application_name`, `client_encoding`) round-trips its params;
  a protocol-2 startup, a length over `MAX_STARTUP`, a missing final NUL, an odd key/value count and non-UTF-8 each
  `Err`; a 16-byte cancel and a 3.2-shaped 44-byte cancel → `Cancel` with the same bytes, a 15- or 269-byte one
  `Err`; `frame` on a split message is `None` until its last byte; `backend_key` of a 3.0 and a 3.2 `'K'` message,
  fed to `cancel_request`, round-trips through `inside`; `error_response("28000", "x")` equals a hand-built byte
  vector.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib gateway::wire`. Expected: FAIL.
- [ ] **Step 3: Implement** to the interfaces.
- [ ] **Step 4: Run** it again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): the gateway's wire — opening, startup, cancel, error response`.

### Task 4: who may pass the gateway

**Files:**
- Create: `crates/cairn-node/src/terminal_service/gateway/admit.rs`
- Test: its unit tests; `crates/cairn-node/tests/terminal_gateway_login_facts.rs` (DB-gated)

**Interfaces:**
- Consumes: `wire::Startup` (Task 3); `conn_posture::{read_posture_of, over_powered, PostureFacts}`.
- Produces:
  - `pub struct LoginFacts { pub can_login: bool, pub window_member: bool, pub service_member: bool, pub posture:
    PostureFacts }`
  - `pub async fn read_login_facts(db: &Client, user: &str) -> anyhow::Result<LoginFacts>`. Membership comes from
    `pg_has_role(user, 'cairn_window' | 'cairn_terminal_service', 'MEMBER')`, plus `rolcanlogin`. A role that does not
    exist is an `Err`, and the caller refuses on it.
  - `pub fn startup_verdict(s: &Startup, node_db: &str, facts: &LoginFacts) -> Result<(), &'static str>`. The `Err`
    text is for the gateway's log only; the window always gets Global Constraints' fixed 28000 message.
  - `pub const REFUSED: &str = "this login may not connect through the node's terminal gateway";`

- [ ] **Step 1: Write the failing tests.** Pure, one per refusal: no `user`; `database` missing or another;
  `replication` present; not a login; not a window member; a service member (a window member too, as
  `cairn_terminal_service` really is); each `PostureFacts` field set to true. One positive case. DB-gated
  (`read_login_facts`): a provisioned window login → member, not a service member, not over-powered; the service login
  → both members; an unknown role → `Err`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib gateway::admit` and the DB test. Expected: FAIL.
- [ ] **Step 3: Implement.** The membership read carries `.context(…)` like `conn_posture.rs`'s reads (Global
  Constraints, Guards).
- [ ] **Step 4: Run** both again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): the gateway's startup verdict — window logins only, never the service's`.

### Task 5: the route to Postgres and `cairn-node gateway-setup`

**Files:**
- Create: `crates/cairn-node/src/terminal_service/gateway/upstream.rs`
- Modify: `crates/cairn-node/src/main.rs` (verb `GatewaySetup`, sequencing only)
- Test: `upstream.rs` unit tests; `crates/cairn-node/tests/gateway_setup.rs` (DB-gated)

**Interfaces:**
- Produces:
  - `pub enum Upstream { Tcp(SocketAddr), Unix(PathBuf) }` and `pub fn parse(s: &str) -> Result<Upstream, String>`.
    `Tcp` must be a loopback address. `Unix` must be an absolute path ending in `.s.PGSQL.<port>`.
    `pub async fn connect(&self) -> std::io::Result<Box<dyn AsyncReadWrite>>` (a tiny `trait AsyncReadWrite:
    AsyncRead + AsyncWrite + Unpin + Send` in this file).
  - `pub fn route_verdict(rules: &[HbaRule], up: &Upstream) -> Result<Vec<String>, String>`. It first applies
    `hba_verdict` (trust and unreadable lines refused). Then, **conservatively** (any matching line of that connection
    type, whatever its database, user or address), it refuses a `Unix` route if any `local` line's method is not in
    {`scram-sha-256`, `md5`, `password`, `reject`}, and refuses a `Tcp` route if any `host`, `hostnossl` or
    `hostgssenc` line's method is `ident`, `gss` or `sspi`. `md5` and `password` are warnings. Each refusal names
    file:line and says the remedy is the site's own reload, never this verb's.
  - `pub async fn gateway_setup(db: &Client, up: &Upstream) -> anyhow::Result<String>`. It requires a superuser
    (`pg_hba_file_rules`), and refuses a stale `pg_hba.conf` (`hba_file_is_stale`) and then any `route_verdict` refusal
    **before** writing. It then upserts `terminal_gateway` and returns the report: the warnings, then the node's
    fingerprint (`identity::load_local(db)`'s `fingerprint`, already `short_fingerprint` of its key) in the line
    `node fingerprint: XXXX-XXXX-XXXX-XXXX-XXXX`.
  - Verb: `cairn-node gateway-setup --upstream <127.0.0.1:5432 | /path/.s.PGSQL.5432>`, which runs `require_local` on
    `--conn` first.

- [ ] **Step 1: Write the failing tests.** `parse`: a remote IP, a relative path and a path not ending in
  `.s.PGSQL.N` each refused; `[::1]:5432` and `127.0.0.1:5432` accepted. `route_verdict`: Debian's stock rules
  (`local all postgres peer`, `local all all peer`, `host all all 127.0.0.1/32 scram-sha-256`) refuse `Unix` naming
  the peer lines and accept `Tcp`; a `host … ident` line refuses `Tcp`; an `md5` line warns. DB-gated:
  `gateway_setup` as superuser writes the row and the report ends with the node's fingerprint; run twice, it still
  holds one row; as a non-superuser it refuses and writes nothing.
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib gateway::upstream` and `--test gateway_setup`. Expected: FAIL.
- [ ] **Step 3: Implement** the interfaces above and the verb.
- [ ] **Step 4: Run** both, plus `provision_window_role` (whose source-order pin must still hold). Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): gateway-setup — the gateway's route is checked once and recorded`.

### Task 6: the window's side of the link

**Files:**
- Create: `crates/cairn-node/src/terminal_link/mod.rs` (module docs: the ceremony from the terminal's side),
  `crates/cairn-node/src/terminal_link/files.rs`, `crates/cairn-node/src/terminal_link/connector.rs`,
  `crates/cairn-node/src/terminal_link/enrol.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod terminal_link;`), `crates/cairn-node/src/db.rs` (`connect_with_task`
  becomes a thin call into `pub(crate) async fn open<T: MakeTlsConnect<Socket>>(mut cfg: Config, tls: T)`, which is
  where `cairn_db_keepalive::detect_dead_peers(&mut cfg)` now runs, so every link gets the #875 dead-peer settings;
  `pub(crate)` because `terminal_link` is a sibling module. The pin `db_connect_applies_the_dead_peer_settings`
  (`db.rs:1214`) moves with the call: it asserts `open`'s body applies them, that `connect_with_task` opens through
  `open(`, and that `terminal_link/connector.rs`'s `Pinned` arm does too; `ConnectFailed` gains
  `pub(crate) fn refused_by_link(message: String) -> Self`, which is `refused_by_client = true` with no SQLSTATE)
- Test: unit tests in each file; `crates/cairn-node/tests/terminal_link.rs` (in-memory handshake, no DB)

**Interfaces:**
- Produces:
  - `files::TerminalFiles::at(dir: &Path) -> Self`; `load_or_create_key(&self) -> anyhow::Result<SigningKey>`
    (`dir/terminal.key` via `crate::keystore::{generate_plaintext, load}`, mode 0600; it refuses to load a key file
    readable by group or others); `read_pin(&self) -> anyhow::Result<Option<String>>` and `write_pin(&self, hex: &str)`
    (`dir/node.pin`, 64 hex digits, written by `crate::fsio::atomic_write` at 0600).
  - `connector::PinnedTls` (`Clone`), implementing tokio-postgres's `MakeTlsConnect<Socket>` and `TlsConnect<Socket>`
    over `tokio_rustls::TlsConnector`. Its stream's `channel_binding()` is `ChannelBinding::none()`, because the TLS
    ends at the gateway and Postgres sees loopback. Server name `cairn-node`.
  - `pub enum Link { Local, Pinned(PinnedTls) }`, with
    `pub async fn connect_with_task(&self, conn: &str) -> anyhow::Result<(Client, AbortHandle)>`. `Pinned` parses the
    string, then forces `SslMode::Require` **and** `SslNegotiation::Postgres` (a string carrying `sslnegotiation=direct`
    would otherwise skip the `SSLRequest`, the gateway would close without a reply, and the window would retry as if it
    were an outage), then calls `db::open`. It maps a received TLS alert `AccessDenied` to
    `ConnectFailed::refused_by_link("this terminal is not admitted by the node: …the admit command with its fingerprint…")`,
    and a pin mismatch to `refused_by_link("the node presented a different key than this terminal pinned …")`. Both are
    refusals, so the reopen names the operator and does not retry as an outage.
  - `enrol::pin_from_fingerprint(addr: &str, typed: &str, terminal_sk: &SigningKey) -> anyhow::Result<String>`: raw
    TCP, `SSLRequest`, then a TLS handshake whose verifier accepts only a key whose `short_fingerprint` equals the typed
    one (normalised). It returns the full hex **only after the handshake completes**, so the node has proved it holds
    the key. It then sends one byte and reads to EOF, so the gateway records the attempt.
  - `pub async fn prepare(conn: &str, terminal_dir: Option<&Path>, typed_fingerprint: Option<&str>) ->
    anyhow::Result<Link>`. Local host: `Link::Local`, and both flags are refused as meaningless. Remote with no
    `terminal_dir`: `require_local`'s refusal plus one sentence naming `--terminal-dir` and `--node-fingerprint`.
    Remote with no pin and no fingerprint: refused, naming `--node-fingerprint`. A pin and a fingerprint that
    disagree: refused (no re-pin by flag; the remedy, deleting `node.pin` after asking the operator, is stated). More
    than one host, or a `hostaddr`, is refused.

- [ ] **Step 1: Write the failing tests:** `files` (a 0644 key is refused; a pin round-trips; a malformed pin file is
  an `Err`); `prepare` (each case above; the refusals never echo the connection string);
  `pin_from_fingerprint_captures_the_key_only_after_the_handshake` and `…_refuses_a_wrong_fingerprint` (an in-test
  server with `gateway_server_config`); `a_pinned_link_maps_access_denied_to_a_refusal` and
  `a_pin_mismatch_is_a_refusal` (`ConnectFailed::refused_by_client()` is true, `sqlstate()` is `None`);
  `a_pinned_link_overrides_sslmode_and_sslnegotiation` (`sslmode=disable` and `sslnegotiation=direct` in the string
  still produce an `SSLRequest` on the wire).
- [ ] **Step 2: Run** `cargo test -p cairn-node --lib terminal_link` and `--test terminal_link`. Expected: FAIL.
- [ ] **Step 3: Implement.** No TLS code in `cairn-gui`: the window only calls `prepare` and `Link`.
- [ ] **Step 4: Run** both, plus `db_diagnosis`, `crypto_sink_names_are_genuine` and the `db.rs` source pins.
  Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): the terminal link — pin at first launch, mTLS on every connect`.

### Task 7: the gateway runs inside the terminal service

**Files:**
- Create: `crates/cairn-node/src/terminal_service/gateway/trust.rs` (the live set, the attempt recorder, the refused
  counter), `crates/cairn-node/src/terminal_service/gateway/sessions.rs` (open sessions by terminal key),
  `crates/cairn-node/src/terminal_service/gateway/serve.rs` (accept loop and one connection)
- Modify: `crates/cairn-node/Cargo.toml` (`socket2 = "0.6"`), `crates/cairn-node/src/main.rs` (`TerminalService`
  gains `--gateway-listen <addr:port>`, with no default), `crates/cairn-node/src/terminal_service/startup.rs` (with
  the flag, the startup check also requires the `terminal_gateway` row and `SELECT` on `terminal_trust`, each refusal
  naming its owner step), `Cargo.lock`, `cairn-gui/Cargo.lock`
- Test: unit tests in `trust.rs` and `sessions.rs`; `crates/cairn-node/tests/terminal_gateway_e2e.rs` (DB-gated)

**Interfaces:**
- Consumes: Tasks 1–6 (the e2e client is Task 6's `Link`).
- Produces:
  - `trust::TerminalTrust::new() -> (TerminalTrust, mpsc::Receiver<String>)`, where `TerminalTrust::store(&self) ->
    TrustStore` is a closure that returns `true` for an active key; for any other key it `try_send`s the hex on the
    bounded (64) channel, bumps the refused counter and returns `false`. `refresh(&self, db) ->
    anyhow::Result<HashSet<String>>` replaces the active set and returns it.
  - `sessions::Sessions` with `open(&self, key: &str, handle: AbortHandle) -> SessionGuard` (the guard removes the
    entry on drop), `SessionGuard::set_cancel(&self, cancel: Vec<u8>)` (the `cancel_request` bytes for its backend,
    once seen), and `close_all_but(&self, active: &HashSet<String>) -> Vec<Vec<u8>>`, which aborts every other key's
    sessions and returns their cancel requests for the caller to send upstream.
  - `serve::run_gateway(listen: SocketAddr, sk: &SigningKey, conn: &str) -> anyhow::Result<()>`, which never returns
    while healthy. Before `accept`: a 128-slot pre-startup counter; a connection accepted with all 128 taken is
    closed at once. An `accept` error (`EMFILE`, `ENFILE`, …) is counted into the once-a-second summary and followed
    by a 100 ms sleep before the next `accept`; it never ends `run_gateway`. Per connection, in order: a 10 s
    deadline **from `accept`** over the permit wait and steps 1–3; a 64-permit semaphore held until startup;
    `opening()`, where `Gss` → write `N` and read the next 8 bytes, which must be `Ssl`, `Ssl` → write `S`,
    and `Other` → close; the TLS accept; read one length-prefixed packet ≤ `MAX_STARTUP`; `inside()`, where `Cancel`
    → forward upstream and close; `read_login_facts` + `startup_verdict` on the gateway's own client (if that client
    is closed or the read errs, **refuse**, then reconnect); then `Upstream::connect` and write `raw`. Then pipe:
    the client→server half is a plain copy from the start; the server→client half is passed through `wire::frame`
    until the first `'Z'` (ReadyForQuery) or `'E'`, forwarding every byte unchanged and handing a `'K'` message's
    `backend_key` to `SessionGuard::set_cancel`, then becomes a plain copy too. The gateway observes those first
    messages and never alters them (D2's "never speaks Postgres's protocol beyond the first messages" holds). socket2
    keepalive and `TCP_USER_TIMEOUT` (Global Constraints' numbers) are set on the accepted socket before the TLS
    accept.
  - One background task per gateway: every **1 s** it runs `refresh`, then `close_all_but`, sends each returned
    cancel request on a fresh `Upstream::connect` (a running statement is cancelled, its backend then finds the
    client gone and exits, releasing its locks), logs `N unadmitted
    handshakes refused` when the counter is non-zero (and resets it), and drains the attempt channel into
    `cairn_terminal_attempt`.
  - The log gets one line per session open (peer address, terminal fingerprint, login) and one per close (with its
    cause). It never logs a startup's parameters beyond `user`.

- [ ] **Step 1: Write the failing unit tests:** `an_unknown_key_is_refused_queued_and_counted`;
  `a_full_attempt_channel_drops_without_blocking_the_handshake`; `close_all_but_aborts_only_the_revoked_keys_sessions`;
  `a_dropped_guard_forgets_its_session`.
- [ ] **Step 2: Write the failing end-to-end tests** in `terminal_gateway_e2e.rs`, each with the gateway on
  `127.0.0.1:0` and Postgres's TCP route as upstream, the client built from Task 6's `Link`:
  `an_admitted_window_login_reads_through_the_gateway` (`SELECT 1`);
  `an_unadmitted_terminal_is_refused_and_recorded_pending`; `a_revoke_ends_the_open_session_within_2_s` (a session
  holding `pg_sleep(30)` errors within 2 s of `cairn_terminal_revoke`, and its backend is gone from `pg_stat_activity`
  within 5 s: this fails without the cancel); `a_revoke_ends_an_idle_in_transaction_session` (it held a row lock;
  another session takes that lock within 5 s); `the_service_login_is_refused_with_28000_and_the_fixed_message`;
  `a_plain_text_startup_gets_no_reply` (a raw TCP startup reads EOF with zero bytes);
  `startups_are_refused_while_the_gateways_db_connection_is_down` (terminate the gateway's own backend with
  `pg_terminate_backend`; the next startup is refused, and a later one succeeds after the reconnect);
  `the_65th_pre_startup_connection_waits_and_then_proceeds`; `a_waiting_connection_is_closed_10_s_after_accept` (with
  all 64 permits held by idle sockets, the 65th reads EOF by about 10 s);
  `the_129th_pre_startup_connection_is_closed_at_once`.
- [ ] **Step 3: Run** `cargo test -p cairn-node --lib gateway::` and `--test terminal_gateway_e2e -- --nocapture`.
  Expected: FAIL.
- [ ] **Step 4: Implement** the three files and the flag. `run` spawns `run_gateway` beside its job loop only when the
  flag is given. A gateway that ends is logged and restarted after `RECONNECT_BACKOFF` (`run.rs:22`, made
  `pub(super)`); it never takes the job loop
  down.
- [ ] **Step 5: Run** both again, then `cargo build` in both trees so both lockfiles refresh. Expected: PASS;
  `git diff --stat` shows both lockfiles.
- [ ] **Step 6: Commit** `feat(#909): the terminal gateway — pinned mTLS to Postgres over loopback`.

### Task 8: `admit-terminal`, `revoke-terminal`, `list-terminals`

**Files:**
- Create: `crates/cairn-node/src/terminal_service/terminals.rs`
- Modify: `crates/cairn-node/src/main.rs` (three verbs, sequencing only; each runs `require_local` first)
- Test: `crates/cairn-node/tests/terminal_admin_verbs.rs` (DB-gated)

**Interfaces:**
- Consumes: Task 1's doors.
- Produces: `pub async fn admit(db, fingerprint: &str, label: &str) -> anyhow::Result<String>`, `pub async fn
  revoke(db, fingerprint: &str) -> anyhow::Result<String>`, and `pub async fn list(db) -> anyhow::Result<Vec<TerminalRow>>`
  with `pub fn render(rows: &[TerminalRow]) -> String` (one line per terminal: fingerprint, status, label, last seen;
  newest first). The fingerprint is accepted in any letter case and with or without dashes, normalised to the stored
  form by `pub fn normalise_fingerprint(s: &str) -> Result<String, String>` (pure; 20 hex digits, else `Err`).
  `revoke`'s output says that open sessions close within 2 s and a running statement is cancelled.

- [ ] **Step 1: Write the failing tests:** `normalise_fingerprint` cases (lower case, no dashes, 19 digits refused);
  `admit` then `list` shows `active` with its label; `revoke` shows `revoked`; an ambiguous or unknown fingerprint
  gives the door's refusal text, not `db error`.
- [ ] **Step 2: Run** `cargo test -p cairn-node --test terminal_admin_verbs`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** it again. Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): admit, revoke and list terminals from the node's console`.

### Task 9: the window connects through the link

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (`Cli` gains `--terminal-dir <dir>` and `--node-fingerprint <fp>`;
  `build_live_state` calls `terminal_link::prepare` where it called `require_local`, and opens the checker and the
  shared connection with `link.connect_with_task`); `cairn-gui/cairn-gui-live/src/connection.rs`
  (`NodeConnection::reopening_via(client, task, conn, link: Link)`, which stores the link; `reopening` calls it with
  `Link::Local`; the free function `open_after(conn, cut_off)` at `connection.rs:507` gains a `link: &Link`
  parameter, its caller at `:464` passes the stored link, and it opens with `link.connect_with_task`)
- Test: `cairn-gui/cairn-gui-live/src/connection/tests.rs`; `cairn-gui/cairn-gui-live/tests/pinned_reopen.rs` (new,
  DB-gated)

**Interfaces:**
- Consumes: Task 6's `prepare`, `Link`; Task 7's `run_gateway` (in the DB-gated test).
- Produces: none for later tasks.

- [ ] **Step 1: Write the failing tests:** `a_reopen_goes_back_through_the_same_link` (the gateway in-test; kill the
  backend; the reopen succeeds through the gateway, not by NoTls); `a_revoked_terminals_reopen_is_a_refusal_naming_the_operator`
  (revoke; the next press's text carries `Refusal::Client`'s clause and the not-admitted sentence, and the next press
  inside the backoff does not dial); `the_window_cli_has_no_flag_that_skips_the_pin` (a source scan of `main.rs`'s
  `Cli`: no `insecure`, `no-pin`, `allow-remote` or `sslmode` flag).
- [ ] **Step 2: Run** the cairn-gui gate commands for `cairn-gui-live` (memory: three lockfiles; the ~2-min GUI gate).
  Expected: FAIL.
- [ ] **Step 3: Implement.** At the first launch, after `pin_from_fingerprint`, the launch itself fails with the
  not-admitted refusal. Its text is printed to stderr with the exact next command: `cairn-node admit-terminal
  <its fingerprint> --label "<where this terminal is>"`.
- [ ] **Step 4: Run** the full cairn-gui gate (clippy `--locked`, test, `cargo doc` with `-D warnings`), the
  `cairn-gui-tauri` DB-gated tests locally (CI does not run them, #905), and every root source guard DB-free.
  Expected: PASS.
- [ ] **Step 5: Commit** `feat(#909): the window reaches a remote node only through the pinned link`.

### Task 10: ADR-0110, spec, RUNBOOK §39, durable rules, tracking documents

**Files:**
- Create: `docs/spec/decisions/0110-the-pinned-terminal-gateway.md`
- Modify: `docs/spec/decisions/README.md`, `docs/spec/index.md` (0.111 → **0.112**), `docs/spec/security.md` (where
  ADR-0084 lives; one paragraph pointing at 0110), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (new **§39**),
  `docs/developers/durable-rules.md` (a "#909 slice B" block, and **rewrite** line 23's `REGISTRATION_LOCK` rule),
  `docs/HANDOVER.md`, `docs/ROADMAP.md`, the design doc's status line

- [ ] **Step 1: Write ADR-0110:** D2–D5 as built, with every number from Global Constraints. It **amends ADR-0084
  decision 5** to "local, or pinned", and **ADR-0109 decision 5** (the registration lock: an advisory lock becomes the
  `registration_turn` row lock, taken by the service only when a request is pending and by the CLI always) and the first
  bullet of ADR-0109's "What is not guaranteed" ("the registration lock makes the two take turns"). It records the
  designer's calls: the conservative route check; newest-64 pending; the 1 s revoke poll, and a trust set held in memory
  and refreshed every 1 s where design D3 said "read per handshake"; the cancel on revoke, and that the gateway observes
  the server's first messages to get its key; the 10 s from `accept` and the 128-socket cap; the only-if-pending turn;
  the window's direct registration refused; the gateway record; revoked is final. On the trust set **not** in the
  local-state export or a backup: after a restore every terminal is admitted again, because a restore is the moment a
  stolen terminal must not quietly regain access. Stated limits: the service is a single point for remote terminals; a
  Postgres restart still disconnects them (slice 5); a stolen, still-admitted terminal holds its key and the window
  password until revoked; the node key is in the service's memory; 80-bit fingerprints are typed and machine-checked,
  and an ambiguous admit is refused; #1159 (raw `submit_event` around the queue) is now reachable from an admitted
  terminal, so say so plainly; **the slice must not ship to a site until slice C (the rollout) is settled** (design §6).
  Then fact-check every sentence against the SQL and Rust (memory: SDD evidence + ADR fact-check).
- [ ] **Step 2: Write RUNBOOK §39:** `gateway-setup` on a stock cluster; the ceremony on a second machine (both
  fingerprints typed); chart open and Sign over the LAN against the 0.2 s median budget (ten of each, local and
  remote); a revoke during an open chart (the session ends ≤ 2 s and the window says so); the service stopped (the
  window's reopen says the gateway did not answer); a forged node (another key at the same address) refused at launch.
- [ ] **Step 3: Write the durable-rules block** (do not undo), and rewrite line 23's rule to the turn (its two-services
  reason stands; the name and the lock change): no fallback verifier; TLS 1.3 only; the startup check refuses on any
  read error; the turn is a row lock, never an advisory lock, and the idle poll never takes it; a revoke cancels as
  well as closes; the trust set is never granted to
  `cairn_window`; `sslmode` is forced on a pinned link; the pin is written only after a completed handshake.
- [ ] **Step 4: Run** `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`, `cargo test -p
  cairn-node --test paper_parity_plan_section`, and `python3 scripts/check_closing_keywords.py` on the PR body.
  Expected: clean.
- [ ] **Step 5: Commit** `docs(#909): ADR-0110, RUNBOOK §39, durable rules for slice B`.
