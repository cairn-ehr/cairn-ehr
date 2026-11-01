# ADR-0084 — The window never holds the owner's keys, and never talks in clear across a wire

- **Status:** Accepted
- **Date:** 2026-11-01
- **Spec version at acceptance:** 0.86
- **Amends:** [ADR-0083](0083-the-human-key-vault.md) (decisions 1, 3 and 4: the interim "the window connects as the
  owner" and the `NoTls` caveat). ADR-0083 is immutable and stays as written; this ADR says what changed.
- **Issues:** [#898](https://github.com/cairn-ehr/cairn-ehr/issues/898) (the window works on the owner's connection),
  [#899](https://github.com/cairn-ehr/cairn-ehr/issues/899) (a passphrase and a seed cross a wire in clear); design
  (maintainer-approved 2026-11-01): `docs/superpowers/specs/2026-11-01-window-connection-posture-898-899-design.md`
- **Builds on:** [ADR-0083](0083-the-human-key-vault.md) (the vault and its counted guesses) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (the schema stamp the window now only reads) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (hard policy lives in the database, soft policy in the UI)
- **Code:** `db/062_window_role.sql`, `crates/cairn-node/src/{conn_posture,window_access,vault,db}.rs`,
  `crates/cairn-node/src/main.rs` (`ProvisionWindowRole`, and the two vault console verbs),
  `cairn-gui/cairn-gui-tauri/src/{main,state,enrol,signer}.rs`

## Context

ADR-0083 put each clinician's sealed signing key in the node's database and made every guess at a passphrase pass
through a counted door. It said plainly that the promise held only for a connection that is no more than
`cairn_agent`, and that the window did not meet that: it ran on whatever connection string it was launched with,
which in every documented setup was the schema owner, and the owner can read `vault_member.sealed` and guess offline
where no count reaches. It also said every database connection is opened `NoTls`, so a window on another machine
sends the passphrase and receives the seed in clear.

Two questions follow:

1. How does the window stop holding the owner's power, without a protection that depends on an operator remembering
   a step?
2. What happens when the window is wrongly connected anyway, or when its database is on another machine?

## Decision

### 1. The window has its own login, in a group role that holds only what it needs (db/062)

- `cairn_window` is a `NOLOGIN` group role (db/062). It inherits `cairn_agent`, the clinical runtime role, and is
  **never** `cairn_node`: `cairn_node` holds the peer-admission doors (`apply_remote_event`, `submit_node_event`), and
  `apply_remote_event` legitimately skips the registration-first rule, so a clinician's window must not reach it.
- Beyond `cairn_agent` it holds exactly: `SELECT` on `local_node` and `node_schema`; `EXECUTE` on `node_hlc_tick()`;
  `SELECT (signing_key_id)` on `actor_event`; `SELECT (singleton)` on `node_unwrap_key`; `SELECT (subject, value)` on
  `name_repudiation` (not `reason`, which db/025 grants to nobody); and on `ui_gesture_timing` `SELECT`, `INSERT` and
  `UPDATE (n, p50_ms, p95_ms)`. It is the only writer that table names; `cairn_agent` still has none.
- **Never granted:** any read of the vault's tables (the check reads `vault_member`, where the sealed blobs are), `cairn_vault_unseal`, `INSERT` on `event_log`, and either
  peer-admission door. Decision 3's check reads each of these as a reason to refuse.
- Each grant was found by running the window's own entry points as a real login in this role, not by inspection
  (`crates/cairn-node/tests/window_role_grants.rs`). Before this slice nothing ran the window under anything but the
  owner, so a missing grant would have failed only in production.

### 2. One owner-run step installs and upgrades; it refuses a cluster that trusts anyone

`cairn-node provision-window-role --role <name>` (default `cairn_window_login`) is run with the **owner's**
connection, and the owner must be a database superuser because it reads `pg_hba_file_rules`. In order; refusals 1
to 4 come before the schema load and the role DDL:

1. The connection must be local (decision 5).
2. It reads the server's access rules. A non-superuser gets "run this step as a database superuser".
3. It refuses if `pg_hba.conf`, or any file it `include`s, was modified after the server last loaded its
   configuration (an operator who removed `trust` but did not reload would otherwise pass on the file's text).
4. It refuses if **any** rule is `trust`, whatever its connection type, database or user, and any rule the server
   could not parse; each refusal names the file and the line (PG16+ lists included files' rules too). A `peer` rule with no `map=` only prints a warning.
5. It loads or upgrades the schema (`connect_and_load_schema`). This is how a node is installed and upgraded: the
   window itself no longer does it (decision 4).
6. It creates the login, or refreshes it, and grants it `cairn_window`. The password comes from
   `CAIRN_WINDOW_DB_PASSWORD`, or, for a NEW role only, a prompt asked twice (rotating an existing login's password
   needs the variable; with neither, the existing login and its verifier are kept), and reaches the server only as a SCRAM-SHA-256 verifier
   computed in the process.
7. An **existing** role of that name is taken over only if it is exactly a window login: it can log in, has none of
   superuser, createrole, createdb, replication or bypassrls, it is a direct member of no role other than `cairn_window` (no membership at all is also accepted), and it
   is not over-powered by decision 3's facts. Otherwise the verb refuses and changes nothing about that role.

Why the `trust` refusal: on a cluster whose rules let local or loopback connections in with no password (Homebrew and
many defaults do), the window's own password protects nothing, since anyone at the machine can rewrite `user=` to the
owner's name. The check is deliberately broader than the window's own rules (a `trust` line for an unrelated
database also refuses), because the rule that matters is that no one can name the owner and be let in.

### 3. A key is released only over a connection that holds no more than a window may

- On **every** `release_for_act`, before anything is sent to the vault, the window reads its own login's power and
  refuses if it has any of: superuser; ownership of a relation in `public` (or membership of the role that owns one);
  `SELECT` on any column of `vault_member`; `INSERT` on any column of `event_log`; `EXECUTE` on `apply_remote_event`,
  `submit_node_event` or `cairn_vault_unseal`. The answer is `Release::OverPowered`.
- It is judged over **every role the login is a member of**, itself included, in one query, including memberships
  that pass no inherited privilege: a member can `SET ROLE` into a role and take its whole power. It is therefore
  over-inclusive on purpose: a membership with `INHERIT FALSE` and `SET FALSE` also counts, and the check fails
  closed (the objects are named schema-qualified, so a missing one is an error, never a pass).
- The verdict is **never cached**. An operator may fix the login while the window's connection stays open (#807);
  the next release reads again (`a_member_of_the_owner_role_is_refused_until_the_membership_is_revoked`).
- It covers the clinical signature (`state.rs`) and "Enrol staff" (`enrol.rs`), the two call sites of
  `release_for_act`. Reading a chart still works
  over an over-powered connection: a wrongly connected window reads but cannot sign. The clinician is told
  "This workstation is connected to the node with more access than a window may have, so it cannot sign. Nothing was
  signed. Ask your IT support to run cairn-node provision-window-role and start this window with its login."
  The specific reason goes to the operator's stderr, never the screen.
- The refusal comes before any passphrase is sent, so a refused attempt is not a counted guess
  (`a_wrong_passphrase_over_an_over_powered_connection_is_never_counted`).
- This is the window refusing before it sends a secret. The vault's own doors are unchanged and remain the defence in
  depth.

### 4. The window never loads the schema

The window's launch connects with the non-loading `db::connect`, reads the `node_schema` stamp and applies the same
test the loader's replay gate uses (`window_schema_verdict`). If the stamp is not exactly this build's, the **launch**
is refused: a database newer than the window says to install the matching window; an older or unstamped one names
`cairn-node provision-window-role` with the owner's connection. Reading through code that does not match the schema
risks a wrong rendering, and the remedy is one command.

**A consequence, stated plainly:** `connect_and_load_schema` also re-adjudicates deferred events on EVERY connect,
because an `overlay targets unknown event` deferral can heal with no code change, and each window launch used to retry
it. The window no longer runs that pass. The daemon's `serve` and `run` connect with `db::connect` and do not run it
either; only one-off loader verbs (`init`, `provision-window-role`, `deferred`, `reproject`, and others) do. So a
deferred event whose target later arrives now stays unpromoted until such a verb runs
([#912](https://github.com/cairn-ehr/cairn-ehr/issues/912)).

### 5. A database on another machine is refused where a secret would cross the connection

`conn_posture::require_local` parses the connection string and refuses unless every host is a Unix socket, `localhost`
or a loopback address (`127.0.0.0/8`, `::1`). A `hostaddr`, when given, is what is judged, since it is what is dialled.
An unparsable string is refused. The error names the host, never the connection string. It runs:

- in the window's launch, first, before any connect;
- in `vault-bootstrap-superuser` and `vault-set-terminal-passphrase`, before the passphrase prompt;
- in `provision-window-role`, before any connect.

There is **no override flag**. An SSH tunnel whose end is `localhost` passes, as an encrypted link should. A
plain-TCP port forward to `localhost` cannot be detected and is out of scope. ADR-0083's advice that a terminal on
another machine reach the node "over a VPN tunnel" is now **refused**: a VPN address is not loopback, so only a tunnel
that ends at `localhost` on the terminal's machine is accepted.

## What is still not guaranteed

- **The daemon's login is outside this.** `provision-runtime-role` still creates the `cairn_node` login with no
  password and does not run decision 2's `pg_hba` check (the check itself is about the whole cluster, not one login).
- **Deferred events heal later** ([#912](https://github.com/cairn-ehr/cairn-ehr/issues/912)): the window and the
  daemon's `serve`/`run` no longer run the deferred-event re-adjudication; only one-off loader verbs do.
- **The connection is still not encrypted.** The `cairn-node` daemon, `cairn-sync`, the matcher and the test rigs are
  not subject to `require_local`; their `NoTls` link stays [#102](https://github.com/cairn-ehr/cairn-ehr/issues/102).
  A pinned, installer-generated encrypted link for terminals that stay thin is
  [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909); a Cairn-installed Postgres with secure defaults is
  [#910](https://github.com/cairn-ehr/cairn-ehr/issues/910). Until #909 or the one-node-per-terminal design (sign-in
  slice 5) lands, a facility has one usable terminal: the one on the database's machine. That is acceptable only
  because nothing clinical runs yet.
- **The `pg_hba` check reads the files, with limits.** It judges the rules the server parsed from the file as it is
  now, included files too (PG16+). The staleness test compares each file's modification time (one-second
  resolution) with the server's load time, so an edit within the same second as a reload reads fresh. `ident` on a `host` line is close to `trust` and is **not**
  refused; only `trust` is. A `peer` rule with no `map=` is reported, not refused.
- **"Refuse before mutating" holds for the role DDL, not for the schema load.** The refusal of an existing
  non-window role, and of an unsafe role name (`is_safe_role_ident`), come after the (idempotent) schema load.
- **The over-power check reads five facts about `public` objects.** It does not prove a login is harmless; it proves
  it cannot reach the vault's blobs, the event log's rows or the peer doors by those routes.
- **Sign-in slice 5 owes an answer to a new risk.** If the vault's sealed blobs sync to every node, a stolen
  terminal disk holds every clinician's sealed key, open to offline guessing with no counter, and the attempt counter
  is per node, so an attacker with N nodes gets N times the online guesses. This is for slice 5's design, not this one.
- **ADR-0083's purpose rule is untouched.** It binds only at the vault, and a client holding a released superuser
  key could still sign a clinical event ([#900](https://github.com/cairn-ehr/cairn-ehr/issues/900)).

## Consequences

- ADR-0083's two named gaps are closed for the window (the owner connection and the clear-text connection). For the
  two vault console verbs, which the owner runs by design, only the clear-text gap is closed. Both remain open for the
  daemon and the other processes listed above.
- Installing a node now has a step that can refuse: a facility on a `trust` cluster must change `pg_hba.conf` first.
  That is intended; the step names the line.
- The window can no longer be pointed at an out-of-date or un-prepared database and left to fix it. An upgrade is
  an owner-run step before the new window starts.
- We are betting that the over-inclusive predicate does not refuse a legitimate deployment. If it does, the operator
  sees the specific reason on stderr and the fix is to give the window the login this verb makes.
