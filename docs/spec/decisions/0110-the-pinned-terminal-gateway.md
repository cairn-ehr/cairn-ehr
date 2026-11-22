# ADR-0110 — The pinned terminal gateway

- **Status:** Accepted (#909 slice B: a window on another machine reaches the node's database only through a gateway
  pinned in both directions. The rollout slice, C, follows, and this slice must not ship to a site before it is
  settled)
- **Date:** 2026-11-22
- **Spec version at acceptance:** 0.112
- **Issues:** [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909), PR
  [#1163](https://github.com/cairn-ehr/cairn-ehr/pull/1163). Design (maintainer-approved 2026-11-16):
  `docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md` (D2–D5, and §5's restore question); plan:
  `docs/superpowers/plans/2026-11-20-909-slice-b-pinned-gateway.md`. Filed by the build:
  [#1164](https://github.com/cairn-ehr/cairn-ehr/issues/1164), and by its final review #1165 to #1170 ("What is not
  guaranteed").
- **Supersedes in part:** [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) decision 5 (a remote database
  is refused) for the window's launch only; [ADR-0109](0109-the-terminal-service-and-the-keyless-window.md) decision
  5's registration lock (its first sentence and its bullet "Two services register once") and the first and last
  bullets of its "What is not guaranteed". Every other decision of both stands.
- **Builds on:** ADR-0109 (the terminal service, `cairn_terminal_service`, the keyless window) ·
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md) (the node's Ed25519 identity and its
  pinned peering trust, which `transport.rs`'s mTLS implements) ·
  ADR-0084 decisions 1–3 (`cairn_window`, the `pg_hba` checks, the over-powered predicate) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (a new `db/084`, `SCHEMA_GENERATION` 84)
- **Code:** `db/084_terminal_gateway.sql`; `crates/cairn-node/src/terminal_service/{gateway/, run.rs, startup.rs,
  terminals.rs, registration.rs}`, `terminal_link/`, `transport.rs`, `db.rs`, `patient/register.rs`;
  `cairn-gui/cairn-gui-live/src/connection.rs`; `cairn-gui/cairn-gui-tauri/src/{main,terminal_launch}.rs`

## Context

Since ADR-0109 no window holds the node's signing key, so a window could sit on another machine. ADR-0084 decision 5
still refused that, because the window's connection carries passphrases and released signing seeds, and it was clear
text (#899). Opening Postgres to the network would mean changing `listen_addresses`, which takes a restart, and a
hospital has no scheduled downtime (design §1). So the node's terminal service gains a third job: a gateway on the
database's machine that terminates a TLS link pinned in both directions and forwards each admitted connection to
Postgres over loopback. Postgres is not restarted, reloaded or reconfigured, and it still listens only to its own
machine.

## Decision

1. **The gateway runs inside the terminal service** (design D1). `cairn-node terminal-service --gateway-listen
   <addr:port>` runs it beside the job loop. The flag has no default: without it no gateway runs. With it, before
   the loop starts, `startup::check_gateway` refuses a login that lacks `SELECT` on `terminal_trust` or
   `terminal_gateway`, or `EXECUTE` on `cairn_terminal_attempt(text)`, or cannot read its own login facts, naming
   `provision-terminal-service-role`. It then refuses a node with no recorded route, naming `gateway-setup`. The
   gateway opens its own database connection on the service's login, loads the trust set, reads the recorded route
   and the node's database name, and only then accepts. Each run is its own task under a supervisor
   (`run::supervise`): a run that ends, by an error or a panic, is logged and started again after 1 s. A repeated
   identical failure is logged once, then once per 60 more, and a run that lasted 60 s or more clears that memory. It
   never takes the job loop down.

2. **Each connection walks design D2's steps, in order, and a startup is forwarded only after all of them.**
   - **The opening.** An `SSLRequest` is answered `S`. A `GSSENCRequest` is answered `N`, and the next 8 bytes must
     be an `SSLRequest`. Anything else, a plain-text startup or a plain-text `CancelRequest` included, is closed with
     no reply.
   - **TLS 1.3 only, mutual, with no fallback verifier** (`transport::gateway_server_config`). The server certificate
     is the node's identity key. The client certificate's Ed25519 key must be `active` in the trust set (decision 4).
     Session resumption and tickets are off, so every handshake is judged against the current set. A key that is
     not active fails the handshake with TLS's `access_denied` alert; a transient condition never does (below).
   - **One packet inside TLS**, of 8 to 10 000 bytes (Postgres's own `MAX_STARTUP_PACKET_LENGTH`). A
     `CancelRequest` is forwarded to Postgres as is, and the connection is closed. A startup is forwarded only if its
     `user` is a LOGIN role that is a member of `cairn_window`, is not a member of `cairn_terminal_service` (which
     inherits `cairn_window`), and is not over-powered by ADR-0084 decision 3's predicate; its `database` is the one
     the gateway's own login is connected to; and its keys are only `user`, `database`, `client_encoding` and
     `application_name`, with none repeated. Any other key is refused, `options`, `replication` and every GUC
     included, because Postgres applies an unknown startup key as a setting at connect, after the login was vetted.
     The login's facts are read on the gateway's own connection for every startup, never cached.
   - **A refusal says only what the window needs.** A login that may not pass gets an `ErrorResponse`, severity
     `FATAL`, SQLSTATE **28000**, message exactly "this login may not connect through the node's terminal gateway",
     naming no role (the gateway's log names the reason). A check the gateway cannot make now (its own connection is
     closed, the facts read failed in the database, or the route to Postgres failed) gets **57P03** after the
     handshake, "the node's terminal gateway cannot check logins right now; try again shortly", which the window
     reads as an outage: its next press tries again. A transient condition is never 28000 and never `access_denied`.
   - **A refused connection is closed gently**: the alert or `ErrorResponse` goes out, the write half is shut, and
     whatever the peer still sends is read and discarded for at most 1 s. Dropping a socket with unread data sends a
     reset, which can discard the alert, and the window would then read a refusal as an outage.
   - **Then the bytes are piped.** The gateway opens the recorded route to Postgres (decision 6), replays the startup
     byte for byte, and copies both directions until either side closes, then closes the other. SCRAM runs end to
     end between the window and Postgres, without channel binding, since the TLS session ends at the gateway.
   - **LAN-side keepalive** from `cairn_db_keepalive`: 30 s idle, 10 s between probes, 3 probes. On Linux and
     Android the socket also gets a 30 s `TCP_USER_TIMEOUT`, which is what ends a terminal that vanished with a
     result still in flight. When either fires, the pipe ends and the Postgres side is closed, so the vanished
     terminal's backend exits and releases its locks: at once if it is idle or idle in a transaction, and only when it
     next reads or writes if it is running a statement (decision 5's cancel is for a revoke, not a vanished terminal).

3. **The flood limits** (designer's calls). A connection holds one of **128** slots from `accept` until it is
   piped or closed; with all 128 taken, a new one is closed at once with no reply, so a LAN host opening idle sockets
   cannot use up the service's file descriptors. At most **64** of them walk the steps above at once; the rest wait
   for a permit. Each has **10 s counted from `accept`**, which covers the wait for a permit, the opening, the
   handshake, the startup and opening the route: at 10 s it is dropped. A failed `accept` (`EMFILE`, `ENFILE`) is
   counted and followed by a 100 ms pause, never returned, so it neither busy-loops nor ends the gateway. The log
   gets one line a second for refused handshakes and one for failed accepts, naming the last error, not one line per
   attempt. Log lines quote a login name escaped (`{:?}`), so a crafted name cannot forge a line.

4. **The terminal trust set** (db/084, design D3). `terminal_trust` has one row per terminal key the gateway has
   seen: the key, its fingerprint, a label, a status (`pending`, `active` or `revoked`), first and last seen, and who
   decided and when. `cairn_terminal_service` may only `SELECT` it and call `cairn_terminal_attempt`, which records
   an unknown key `pending`, only touches `last_seen` on a known one (so a revoked terminal cannot be re-pended by
   knocking), and then deletes every pending row beyond the **64 newest** by `last_seen` (designer's call: a flood of
   fresh keys cannot grow the table, and a real terminal re-records itself at its next launch). `cairn_window` has no
   grant on it. Only the owner admits and revokes:
   - `cairn-node admit-terminal <fingerprint> --label "<where>"` turns the one pending row with that fingerprint
     `active`. It refuses an empty label, an unknown fingerprint, an ambiguous one (more than one row) and a row that
     is not pending: an active one needs nothing, and **revoked is final** (designer's call). A revoked terminal
     comes back only with a new key.
   - `cairn-node revoke-terminal <fingerprint>` revokes the one pending or active row with it. Revoking a revoked one
     is a no-op.
   - `cairn-node list-terminals` lists every row, most recently seen first: an admitted terminal was last seen
     when it last connected, a refused one when it last tried.

   Each refuses a database on another machine (`require_local`). A typed fingerprint may use either letter case and
   leave out the dashes. The fingerprint is `cairn_event::short_fingerprint`: the first 10 bytes (80 bits) of the
   SHA-256 of the 32 key bytes, as five groups of four upper-case hex digits, the node fingerprint pairing already
   shows; db/084 computes it in SQL, and a test pins the two equal.

   **The gateway holds the set in memory, refreshed every 1 s** (designer's call). Design D3 said the service "reads
   it per handshake", but rustls judges a client certificate synchronously, inside the handshake, where no database
   read can be awaited. The handshake reads the in-memory copy. Every key it judges is queued (64 deep, dropped when
   full, never awaited) for the next tick to record through `cairn_terminal_attempt`: a refused key as an attempt,
   an active one only for its `last_seen`. Only refused handshakes are counted in the 1 s log line. **A failed refresh
   keeps the last set**: a stale set beats an empty one, which would shut every admitted terminal out over a database
   hiccup.
   Until its first load succeeds the gateway accepts nothing, and its handshake check refuses every key and records
   none.

5. **A revoke ends a terminal's open sessions, a running statement included** (design D3). Every piped session is
   registered by terminal key. Each 1 s tick refreshes the set, then aborts every session whose key is no longer
   active and closes both its sockets (designer's call: a poll, not a NOTIFY). Closing the loopback socket alone does
   not stop a backend that is busy running a statement: Postgres notices a gone client only when it next reads or
   writes. So the gateway watches the server's first messages of each session, forwarding them unchanged, to keep the
   `BackendKeyData` key, and on a revoke it also sends Postgres a `CancelRequest` with that key on a fresh
   connection. The statement is cancelled, the backend finds its client gone and exits, and its locks are released.
   The watch ends at the first `ReadyForQuery` or `ErrorResponse`, and a session whose server sends more than 64 KiB
   without one whole message is closed. Pinned: a running statement ends within 2 s of the revoke, and an
   idle-in-transaction session's locks are released within 5 s.

   **No session outlives the run that sweeps it.** Only a run's own tick sweeps its sessions, and a run can end (a
   panic, which the supervisor restarts) while its sessions still pipe. So however a run ends, every session it piped
   is aborted and, as for a revoke, its running statement is sent a `CancelRequest`; a startup admitted afterwards by
   a connection still walking the steps under it is aborted as it opens, before its backend runs any statement
   (`sessions::CloseOnDrop`). A restart therefore ends every open remote session, which the window's reopen brings
   back through the new run, and a revoke reaches every session there is. The cancel is best-effort. One that is sent
   but cannot reach Postgres (the route is down too) is logged. One that is never sent, because no runtime is left to
   send it or the runtime shuts down first, is not logged. Either way that statement runs until it next writes, as a
   vanished terminal's does. Pinned: `terminal_gateway_e2e.rs::a_session_ends_when_its_gateway_run_ends` and
   `a_running_statement_ends_when_its_gateway_run_ends` (red without the cancel).

6. **`gateway-setup` checks the route once and records it** (design D2 step 4, D4 step 1).
   `cairn-node gateway-setup --upstream <route>` runs with the owner's connection, who must be a database superuser
   because it reads `pg_hba_file_rules`; the service's own login cannot. A route is a loopback `host:port`
   (`127.0.0.1:5432`, `[::1]:5432`) or an absolute path to `.s.PGSQL.<port>` (no `..`, no trailing `/`). Any other
   address is refused before any connect, then a database on another machine. Its checks come before its one write: a
   non-superuser is refused, then access rules changed since the server last read them, then ADR-0084's verdict (any
   `trust` rule, any rule the server could not read), then the route's own check (designer's call, deliberately
   conservative: every line of the route's connection type counts, whatever its database, user or address, because the
   gateway cannot know which line a login will match):
   - for a Unix socket, every `local` line must use `scram-sha-256`, `md5`, `password` or `reject`. `peer`, `ident`
     and every other method are refused, since they would let the gateway's own operating-system user in without the
     window's password;
   - for loopback TCP, every `host`, `hostssl`, `hostnossl`, `hostgssenc` and `hostnogssenc` line using `ident`, `gss`
     or `sspi` is refused;
   - `md5` or `password` on a line of the route's type gives a warning, as does ADR-0084's `peer` with no map.

   Each refusal of a rule names its file and line and says to change it, reload the server, and run the verb again.
   The verb never edits, reloads or restarts Postgres. It then writes the **gateway record** (designer's call):
   `terminal_gateway`'s one row, `{upstream, checked_at, checked_by}`, readable by `cairn_terminal_service` and
   written by the owner only, and prints any warnings and "node fingerprint: XXXX-XXXX-XXXX-XXXX-XXXX" for the
   ceremony. The gateway reads the route from that row each time it starts.

7. **The window reaches a remote database only through the pinned link** (design D4 step 2, D5).
   `terminal_link::prepare` chooses a `Link` before any connect, from the connection string and the window's two new
   flags, `--terminal-dir <dir>` and `--node-fingerprint <fp>`:
   - a local database (ADR-0084's `require_local` passes) is `Link::Local`, connected as before. Either flag is
     refused there as meaningless, so a mistyped host is never read as "local, ignore the flags";
   - a remote one is `Link::Pinned`, or refused: without `--terminal-dir` (ADR-0084's refusal, plus the two flags to
     give), with a `hostaddr`, with more than one host, with no pin and no fingerprint, with a pin that the typed
     fingerprint contradicts (a pin is never replaced by a flag), or with a malformed fingerprint. There is no flag
     that skips the pin, and nothing falls back to plain text.

   The terminal keeps two files in its directory, which is made at 0700 and refused if group or others can write it:
   `terminal.key`, its own Ed25519 key, made at first launch at 0600, unsealed because the link must open before
   anyone has typed anything, and refused if group or others have any permission on it; and `node.pin`, the node's
   full public key, refused if group or others can write it. **At first launch** the terminal dials the gateway,
   within one 10 s deadline for the TCP connect, the `SSLRequest` and the handshake. It accepts the server's key only
   if its short fingerprint is the typed one, and **writes the pin only after the handshake has completed**, because
   only then has the node signed the handshake with that key. The gateway records the terminal `pending`, and the
   launch ends with the not-admitted refusal followed by the one command for the operator:
   `cairn-node admit-terminal <this terminal's fingerprint> --label "<where this terminal is>"`. Neither fingerprint
   is compared by eye: each is typed and checked by a machine, and an ambiguous admit is refused.

   Every later connect, the launch's and every reopen's (#807), opens through the same `Link` with
   `connector::PinnedTls`: TLS 1.3 only, the terminal's key as its certificate, the pinned key as the only server key
   accepted. `sslmode` is forced to `require` and `sslnegotiation` to `postgres`, whatever the string says. The link's
   own refusals are client refusals with no SQLSTATE, so the window's reopen names an operator instead of retrying:
   - **not admitted** (`access_denied`): "this terminal is not admitted by the node (it may be new, or it may have
     been revoked). Ask the node's operator: a new terminal is admitted with cairn-node admit-terminal <fp> --label
     "<where this terminal is>"; a revoked terminal needs a new terminal key." A revoked terminal gets the same alert
     as a new one, so the text gives both remedies. The reopen keeps `Refusal::Client`'s clause ("This workstation
     could not complete the sign-in the node asked for") after it;
   - **another key** (any other rejected certificate): "the node presented a different key than this terminal pinned
     (or could not prove it holds the pinned one), so nothing was sent to it. …", with no override;
   - **not a gateway** (a certificate that is not an Ed25519 key): "the server at this address is not a Cairn node's
     terminal gateway (check the port): …".

   A local `hostaddr=127.0.0.1` stays `Link::Local`: the `hostaddr` and multi-host refusals apply to a remote string
   only.

8. **The registration turn is a row lock, not an advisory lock.** db/084's `registration_turn` holds one row, and
   `cairn_registration_take_turn(p_if_pending)` locks it `FOR UPDATE` until the caller's transaction ends. Slice A's
   advisory lock could be taken by any login, the window's included, so a window could hold it and starve
   registration. The door's `EXECUTE` is granted to `cairn_terminal_service` only, and the owner, who owns it. The
   service takes the turn with `p_if_pending = true` as its first statement: it locks nothing and returns false when
   no request `cairn_registration_claim` would return exists (designer's call: a row lock writes WAL, and the service
   polls every 250 ms). The CLI's registration (`register_patient`, the owner's `patient-register`) always takes it.
   A missing turn row raises rather than letting registrations run in parallel. **The window's own direct
   registration is refused** (`42501`): since ADR-0109 it registers only through the queue.

9. **What the superseded passages now read.**
   - **ADR-0084 decision 5's first bullet** ("in the window's launch, first, before any connect") **now reads:** *in
     the window's launch, first, before any connect, a database on another machine is reached only through the pinned
     link (this ADR's decision 7), and refused as before without a terminal directory.* The decision is "local, or
     pinned", and there is still no override flag. `vault-bootstrap-superuser`, `vault-set-terminal-passphrase` and
     `provision-window-role` stay local-only, unchanged.
   - **ADR-0109 decision 5's first sentence now reads:** `serve_one` takes the registration turn
     (`cairn_registration_take_turn(true)`, a row lock) as its first statement, then claims a request, and so on as
     written. **Its bullet "Two services register once" now reads:** each service takes the turn before it claims,
     and the CLI takes it too, so registrations take turns node-wide: the second waits for the first to commit. The
     protection is one-way. A service searches after it takes the turn, so it finds a chart the first registration
     made; the CLI's `patient-register` has already searched before it takes the turn, so a CLI registration racing
     a service's can still make a duplicate. The rest of the bullet stands.
   - **ADR-0109's first "not guaranteed" bullet now reads:** one service per node is assumed. A second is harmless:
     the registration turn makes the two take turns, the second waiting until the first commits. Each runs its own
     gateway only if given `--gateway-listen`, on its own address.
   - **ADR-0109's last "not guaranteed" bullet** ("the local window still connects only to a database on its own
     machine") is no longer true: see the first item.

10. **The trust set is in no Cairn backup and no local-state export** (design §5's question, designer's call). A
    backup carries signed events only (ADR-0026), and the local-state export reads only its own named tables, neither
    `terminal_trust` nor `terminal_gateway`. After a restore by Cairn's own `restore` every terminal must be admitted
    again, and `gateway-setup` run again; a restored node with a new key is refused by every terminal as another key
    until its operator removes the terminal's pin. A restore is the moment a stolen terminal must not quietly regain
    access. A whole-database copy made outside Cairn is the exception ("What is not guaranteed").

**Paper-parity.** The paper counterpart is a clinician at a ward desk reading and writing the chart that was carried
to them. Paper *N* = 0 extra human acts to read or sign at a remote desk → forced *M* = 0 per act → target *K* =
0: the link is set up once per terminal by an operator, which is not a clinical workflow (design §7), and every
later read or sign is the same press as on the database's machine. Registration stays 3 → 3 → 3 (ADR-0109). The
budget: on a wired facility LAN, opening a chart and pressing Sign each take at most 0.2 s longer at the median
through the gateway than on the database's machine, measured in RUNBOOK §39, the first slice to expose it. Two new
launch sentences, not admitted and another key, are for an operator, not a clinician.

## Consequences

- A facility can have more than one usable terminal. A window on another machine of its network reads and signs
  over a link pinned in both directions, and no terminal holds the node key (ADR-0109).
- A lost or stolen terminal is shut out on its own, at once, without changing anyone's password: `revoke-terminal`
  ends its sessions within the next tick and cancels what they are running.
- Postgres is not restarted, reloaded or reconfigured by any verb of this slice, and still listens only to its own
  machine. A site whose access rules fail the route check (decision 6) is refused, with the line it must change;
  the reload is the site's to schedule.
- Postgres sees every remote terminal as a loopback client. The gateway's log names each session's peer address,
  terminal fingerprint and login when it opens and when it closes.
- The terminal service is now the remote terminals' one way in, as well as the node's registrar and commit runner.

## What is not guaranteed

- **The service is a single point for remote terminals.** With it stopped, or its gateway restarting, they have no
  database, and a gateway restart ends every open remote session (decision 5); the window's reopen brings it back,
  though no test yet cuts a session off and reopens it through the pinned link (#1168). The window on the
  database's machine still reads and signs, but cannot register or commit drafts on the timer (ADR-0109).
- **A Postgres restart still disconnects every terminal**, remote or local (slice 5 is the answer). The window's
  reopen brings it back.
- **A stolen, still-admitted terminal holds its key and the window login's password until it is revoked.** Signing
  still needs a clinician's passphrase.
- **The node key is in the service's memory**, as it already is in `serve`'s.
- **A fingerprint is 80 bits.** Each is typed and checked by a machine, never compared by eye, and an ambiguous admit
  is refused; the pin itself is the full key.
- **While the gateway cannot refresh the trust set** (its own connection is down, or the read fails), it keeps the
  last set. A revoke written meanwhile ends that terminal's sessions, and refuses its handshakes, only from the first
  refresh that succeeds. While the gateway's connection is down every new startup is refused with 57P03 anyway.
- **A whole-database copy brings the trust set back as it was.** Neither `terminal_trust` nor `terminal_gateway` is
  in a Cairn backup or a local-state export (decision 10), but an operator's own `pg_dump`, or any other copy of the
  whole database outside Cairn's backup and restore, carries both, `active` rows included. Restoring one readmits
  every terminal that was active when it was taken, a stolen one included: after such a restore the operator must
  revoke every lost terminal again (`list-terminals` shows them).
- **The route is checked when `gateway-setup` runs, not after.** An access rule changed later (a `peer` line added
  for the route, say) is not seen until the verb runs again.
- **#1159 is now reachable from an admitted terminal.** A window login inherits `cairn_agent`, which may run
  `submit_event`, and db/045 has no signer rule for a registration. So a raw-SQL session through an admitted
  terminal, holding a clinician's released key and the window login's password, may be able to submit a
  registration directly, around the queue and its re-search. Before this slice that needed a session on the
  database's machine.
- **The rollout is not settled.** Delivering this slice is itself an upgrade that can disturb clinical work: a schema
  replay takes locks (#726, #875), a window of an older build is refused by a newer database (ADR-0084 decision 2),
  and the service must be running before any window depends on it. **This slice must not ship to a site until slice
  C, the rollout, is settled** (design §6).
- **No cap per terminal or per host.** One admitted terminal may open any number of sessions (#1165), and one LAN
  host may hold all 128 pre-startup slots, each for up to 10 s and again as soon as it is freed, shutting every
  other terminal out while it does (#1166).
- **SCRAM through the gateway has no automated test** (#1167): the test cluster is `trust`, so only RUNBOOK §39
  shows a password login passing end to end.
- **The window tells "not admitted" from other refusals by its wording** (#1170), not by a typed marker. An ordinary
  close (a revoke, a killed backend) logs rustls's "peer closed connection without sending TLS close_notify" and its
  docs URL on both sides, which reads as alarming for a normal close (#1169).
- **`connection.rs` is over the 500-line house limit** (650 lines; its verbatim split is #1164).

Pins: `crates/cairn-node/tests/{terminal_gateway_doors,terminal_gateway_e2e,terminal_gateway_e2e_refusals,
terminal_gateway_login_facts,terminal_link,terminal_admin_verbs,gateway_setup,terminal_service_gateway_check,
transport,window_role_grants}.rs`; the unit tests of `terminal_service/gateway/{wire,admit,accept,trust,sessions,
pipe,serve,upstream}.rs`, `terminal_service/{run,startup,terminals}.rs` and `terminal_link/`;
`cairn-gui-live/tests/pinned_reopen.rs`; `cairn-gui-tauri`'s `launch_posture_tests.rs` and `terminal_launch.rs`.
RUNBOOK §39 carries the human measurements and the success path of `gateway-setup` on a `scram-sha-256` cluster,
which no test runs (the test cluster is `trust`).
