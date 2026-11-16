# A thin terminal on another machine: the pinned terminal link (#909) — design

**Date:** 2026-11-16 · **Status:** approved by the maintainer 2026-11-16, awaiting a plan ·
**Closes (when built):** [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909) ·
**Amends (by a new ADR, never an edit):** [ADR-0084](../../spec/decisions/0084-the-window-never-holds-the-owners-keys.md)
(D5, "a remote database is refused") and [ADR-0089](../../spec/decisions/0089-the-quarantine-commit-and-the-asserted-grade.md)
decisions 11 ("the window is the runner") and 12 (the due words) ·
**Related:** #102 (the daemon's and `cairn-sync`'s `NoTls`, untouched here), #910 (a Cairn-installed Postgres),
#912 (no long-running process re-adjudicates deferred events), sign-in slice 5 (every terminal its own node),
the #898/#899 design [`2026-11-01-window-connection-posture-898-899-design.md`](2026-11-01-window-connection-posture-898-899-design.md).

## 1. Why

Today a facility has **one usable terminal**: the one on the database's machine. ADR-0084 refuses a window whose
database is anywhere else, because the connection would carry passphrases and released signing seeds in clear
text (#899). This gates any multi-terminal pilot. #909 asked for an encrypted link whose server identity is made
by the installer and pinned by each terminal once at setup, with no hand-configured certificates: whoever installs
may not be IT.

**The brainstorm found a second, larger gap that #909 did not name.** The window loads the **node's own signing
key** (`--key node.key`, `cairn-gui-tauri/src/main.rs`). A thin terminal on another machine would therefore need a
copy of the node's identity key on its own disk. A stolen terminal would then let its holder sign as the node,
present as the node to its peers, and could not be shut out by banning a terminal. A traced inventory (2026-11-16)
found that the window uses the node key's **private** half for exactly two acts:

1. **Patient registration.** `patient::register::register_patient` signs `identity.registration.asserted` and its
   `demographic.*` events with the node key. There is no human-author path.
2. **The quarantine commit** of an overdue note draft. `note::commit::commit_overdue` signs with the node key, as
   ADR-0089 decision 6 requires (the "went in unsigned" grade depends on no human being the signer). It runs on the
   window's 60 s tick and before every notes read.

Every other window write is signed by the clinician's own key, and the node contributes only its public key id
(`medication/sealed_submit.rs::apply_author`). **Reading never touches the node key.** Custody wraps happen inside
the database to the node's registered *public* unwrap key, and the window reads clear text from projections. So
moving those two acts onto the database's machine lets the window drop the node key entirely.

**Hospitals have no scheduled downtime (maintainer, 2026-11-16).** No step of this work may restart Postgres or
disturb an open connection, and data loss is not acceptable under any circumstances.

## 2. Goals, non-goals, anti-goals

**Goals**
1. A window on another machine of the facility's network can read and sign, over a link that is encrypted and
   pinned **in both directions**. The terminal pins the node, and the node admits only terminals it has been told
   about.
2. **No window holds the node's signing key**, whether it is remote or on the database's machine.
3. **Setting it up never restarts, reloads or reconfigures Postgres**, and never disconnects anyone.
4. Setup is a short console ceremony a non-IT person can follow: two fingerprints of 20 characters each, one in each
   direction, as nodes already do when they pair (`pair-offer` / `pair-accept`).
5. A lost or stolen terminal can be shut out on its own, at once, without changing anyone's password.

**Non-goals**
- TLS for the `cairn-node` daemon's or `cairn-sync`'s own database connections (#102 stays open; they run on the
  database's machine).
- A graphical installer or setup screen. "Setup" is console verbs and window launch flags.
- Zero-downtime Postgres. A Postgres restart (minor-version patches, eventually a CVE) still disconnects every thin
  terminal. The design that makes that a non-event is slice 5. This work must say so in its ADR and never imply
  otherwise.
- Credential sync between nodes (slice 5). Per-terminal OS keychain storage of the terminal's key (a follow-up).

**Anti-goals: a build that does any of these has failed even if it works**
- Copying `node.key` (sealed or not) to any terminal.
- Anything that restarts or reloads Postgres, or edits `postgresql.conf` / `pg_hba.conf`.
- An override flag that lets a window talk to a remote database without the pinned link (ADR-0084's anti-goal
  stands).
- A LAN-reachable path to the owner's or a superuser's login.
- A registration that can go in after the registrar was told it failed (a silent duplicate chart).
- A confirmation dialog standing in for any check (principle 3).

## 3. Approaches considered

**Where the TLS ends.**
- *Postgres's own TLS* (`ssl=on`, a `hostssl` rule). Turning TLS on takes only a reload. But accepting
  connections from the LAN needs `listen_addresses` changed, and that takes a **restart**. A command writing the key
  file needs file access as the Postgres OS user, and a reload also applies every edit someone else left pending.
  Rejected: it breaks goal 3 on any site whose Postgres listens to its own machine only, which is the default.
- **A Cairn gateway on the database's machine (chosen).** It terminates the pinned TLS and forwards each connection
  to Postgres over loopback. Postgres is never touched and never listens on the network at all. A hospital gets one
  network-facing endpoint, pinned both ways. The gateway can present the node's own identity key, so the terminal
  pins the same fingerprint peers already compare. Costs: one more always-on process (if it is down, remote
  terminals have no database); Postgres sees every remote terminal as 127.0.0.1, so the gateway logs addresses.

**Who checks whom.**
- *Server-only pin.* The terminal checks the node, and the node relies on the window login's password alone. Anyone
  with that password and a LAN cable could read every chart from any laptop. Rejected.
- **Two-way pin (chosen, maintainer 2026-11-16).** Each terminal has its own key and must be admitted. It reuses
  `transport.rs`'s pinned client/server verifiers.

**The node key.**
- *Copy it to each terminal.* Rejected (§1).
- *A node-side service that does all key work for the window* (seal, sign, unwrap). Unnecessary: the window signs
  everything but two acts with human keys and never unwraps.
- **Move the two node acts to a node-side process (chosen).**

**Registration (maintainer, 2026-11-16: R2).**
- *R1: the registrar signs it with their own key.* That adds a staff ID and passphrase to every registration on a
  shared terminal, where paper has no signature: a probable `M > N` defect.
- **R2 (chosen): the node-side process signs it on the window's request.** Nothing changes for the registrar or on
  the wire. A terminal without the node cannot register anyway, so the process being required costs no
  availability that the link did not already cost (maintainer).

## 4. What was decided

**D1. One node-side process, the terminal service** (verb name builder's call, e.g. `cairn-node terminal-service`).
It runs on the database's machine, is always on and is supervised (launchd/systemd, one per node). It loads the
node key unattended, exactly as `serve` does (`load_signing_key(&cli.key, false)`: never prompt, fail fast). It
does three jobs:
(a) the gateway (D2–D4); (b) the quarantine commit tick (D7); (c) registration requests (D6).
*Designer's call:* a verb of its own, not folded into `serve`, so federation sync and the facility's terminals can
be restarted independently. Both may run, and each holds the node key in memory, as `serve` already does. It is
the natural later home of #912's re-adjudication runner (not built here).

**D2. The gateway forwards, it never speaks Postgres's protocol beyond the first messages.** Each connection, in
order:
1. Read the 8-byte opening message. An `SSLRequest` gets `S` and a TLS handshake. A `GSSENCRequest` gets `N` and
   must then be followed by an `SSLRequest`. Anything else, a plain-text startup or a plain-text `CancelRequest`
   included, is closed without a reply.
2. TLS 1.3 only, mutual. The server certificate is the node's identity (`transport::node_cert`). The client
   certificate's Ed25519 key must be `active` in the terminal trust set (D3). There is no fallback verifier.
3. Inside TLS, read the `StartupMessage` (or a `CancelRequest`, which is forwarded as is: its secret key is per
   backend). Forward a startup **only** if its `user` is a login that is a member of `cairn_window` and is not
   over-powered by ADR-0084 D4's predicate, and its `database` is the node's own. Anything else is closed with a
   Postgres-shaped `ErrorResponse` that names no role. ADR-0084 decision 3's over-powered predicate is read for that
   login when the gateway starts and again per startup (the window's own per-key-release check, `vault.rs`, stays
   as it is). The owner's and superusers' logins are never reachable from the network.
4. Open a connection to Postgres **by the same route the local window already uses** (its socket or loopback TCP),
   so no `pg_hba` line is added and nothing is reloaded. *Designer's call (builder check, 2026-11-16):* the route
   is acceptable only when the cluster's rules admit the window's login on it by a password method (`scram-sha-256`,
   or `md5` reported as a warning), never by `trust`, `peer` or `ident`: on a socket, a `peer` rule would let a LAN
   client in as whatever OS user runs the gateway. Replay the startup and pipe both directions until either side
   closes, then close the other.
5. TCP keepalive on the LAN side. When it fires, the Postgres side is closed, so a vanished terminal's session ends
   and releases its locks.

SCRAM authentication still happens, end to end, between the window and Postgres. Postgres configuration is not
changed. Reading `pg_hba_file_rules` needs a superuser, so the route check (step 4) is done by the **owner-run setup
verb** (D4 step 1), not by the service's own login. ADR-0084's install check (`window_access::hba_verdict`) refuses
only `trust`, so this is a new, stricter check, run for the gateway's route only. A site with no acceptable route is
refused with the rule it lacks named; adding it is that site's reload to schedule, never this verb's.

**D3. The terminal trust set** is a node-local table (new `db/` file) of `{terminal key, short fingerprint, label,
status (pending | active | revoked), first seen, decided by, decided at}`. Only the owner's console verbs make a row
`active` or `revoked`. *Designer's call:* the service records an unknown terminal's attempt through one definer door
that can insert `pending` and nothing else, with a cap on pending rows (an unauthenticated LAN host can mint any
number of keys), so a flood cannot block setup forever or fill the disk. The window's role cannot read or write the
table, and the service reads it per handshake. It is not synced: each node
admits its own terminals (slice 5 revisits this). A revoke also **closes that terminal's open sessions at once**,
not merely its next handshake. The gateway tracks sessions by terminal key, and the console verb signals it
(NOTIFY or a poll, builder's call).

**D4. The ceremony, per terminal (pairing's shape):**
1. On the database's machine: a verb prints the node's fingerprint (`cairn_event::short_fingerprint`, 20 characters,
   80 bits, the same one `pair-offer` shows).
2. On the terminal, first launch with the gateway's address and that fingerprint. The window generates its own
   Ed25519 key if it has none (a file in its config directory, mode 0600), connects, refuses unless the node's
   presented key matches the typed fingerprint, stores the node's **full** key as its pin, and prints its own
   fingerprint. The gateway records the attempt as a `pending` row. The connection itself is refused, because the
   terminal is not yet admitted.
3. On the database's machine: `admit-terminal <fingerprint> --label "<where>"` turns the one matching pending row
   `active`. It refuses an ambiguous or unknown fingerprint. `revoke-terminal` and `list-terminals` complete the set.
   After that, the terminal connects. Neither fingerprint is ever compared by eye alone: each is typed and checked by
   the machine.

**D5. The window connects remotely only through the pinned link.** A connection string whose host is not this
machine is opened with a TLS connector (over the existing `tokio-rustls`/`rustls`) carrying the terminal key and the
node pin. It never falls back to plain text. Without a pin it is refused exactly as today. ADR-0084's D5 becomes
"local, or pinned". Today ADR-0084's over-powered predicate (decision 3) runs on every key release and its schema-stamp
check runs at launch only; a reopen (#807) runs neither, and this work changes neither (the gateway's own check,
D2 step 3, is extra). The `cairn-node` vault console verbs stay local-only (`require_local` unchanged).

**D6. Registration through a request door** (new `db/` file). The window submits the registration's **inputs**
(the same ones `register_patient` takes today, the attested search included, ADR-0061) under a window-generated
request id, then waits on a NOTIFY (or polls) for its outcome. The service locks the request row, registers, signs
with the node key and writes the outcome **in that one transaction**. *Designer's call (builder check):* the claim
IS the row lock, never a separately committed state, so a service crash leaves the request pending (it then expires
or is retried) and never half-claimed. `register_patient` today opens and commits its own transaction and ticks the
HLC outside it, so it is refactored to run inside a caller's transaction; the CLI keeps its behaviour.

**The service re-runs the search before it signs (maintainer, 2026-11-16).** Under R2 anyone holding the window
login's password and an admitted terminal could send a false attested search by raw SQL ("searched, saw no one"),
and db/045 checks only its shape. So the service runs the same search for the request's query and **refuses** if it
finds any candidate the request's `displayed` list does not name. That refusal also catches a near-match that
appeared after the screen was drawn; the window says "a new possible match has appeared, search again". A
displayed candidate the re-run no longer finds is not a refusal.
Rules:
- The wire and the events are unchanged: node-signed, `{node, recorded}`, as today.
- Idempotent by request id: a re-sent request returns the first outcome and never registers twice.
- **No late registration.** Each request carries a deadline. Claiming and expiring are each one atomic step in the
  door: the service claims only an unexpired pending row, and the window can expire it only if it is unclaimed. A
  window that waited past its deadline reads the row and reports what actually happened (registered, or expired and
  not registered). It never says "failed" for a request that may yet go in.
- With the service down, the window says plainly that registration needs the node's terminal service, names it, and
  the request expires unregistered.

**D7. The quarantine commit moves to the service's tick** (the same 60 s; the same `commit_overdue` and its doors).
The window's tick and its commit before every notes read are removed. A draft just past its due time can therefore
still show as a draft for up to one tick. Two of ADR-0089 decision 12's sentences describe the window as the runner
("the window tries again at every check…", "the window's last try failed…"), and the second is driven by the
window's own memory (`AppState::commit_failed`). *Designer's call:* the service records a failed commit **per draft
in the database** (a new fact its door writes and the window reads), so a clinician whose abandoned draft keeps
failing is still told to act; the sentences are reworded to name the node. A new ADR amends decisions 11 and 12.

**D8. The window drops `--key`** on every terminal, local included. `node_kid` comes from `identity::load_local`'s
public key. *Designer's call, stated so the maintainer can reject it:* a single-machine clinic must then run the
terminal service for registration and timed commits. One window shape beats two code paths, one of which would keep
a key file on a terminal. The local window still connects over its socket and needs no pin.

**D9. A new ADR records D1–D8**, amends ADR-0084 D5 and ADR-0089 decisions 11 and 12, and states the stated limits (§5).
The spec version bumps. A RUNBOOK section covers the ceremony, a revoke during a live session, the service being
stopped mid-registration, and the timing below.

## 5. Risks and stated limits

- **The service is a single point for remote terminals.** If it is down, they have no database. The local window can
  still read and sign, but cannot register or commit drafts on the timer.
- **A Postgres restart still disconnects every thin terminal** (slice 5 is the answer). The window's reopen (#807)
  brings it back.
- **A stolen, still-admitted terminal** holds its key and the window login's password. Revoke shuts it out at once.
  Signing still needs a clinician's passphrase.
- **The node key is in the service's memory**, as it already is in `serve`'s.
- **Restore:** whether the trust set rides the local-state export or every terminal re-admits after a restore is left
  to the plan (§6).
- §4 of the #898/#899 design (sealed blobs on a stolen terminal disk, the per-node attempt counter) is untouched:
  thin terminals hold no blobs.

## 6. The rollout disturbs no one (maintainer, 2026-11-16: in scope)

Delivering this work is itself an upgrade, and today an upgrade can disturb clinical work in three ways:
1. `connect_and_load_schema` replays every `db/*.sql` whose content changed (ADR-0080), and even a no-op DDL replay
   takes its lock (#726). A lock held past a window command's 15 s cut-off (#875) cuts that window's act.
2. A window relaunched against an upgraded database before its own binary is upgraded is refused by the stamp check
   (ADR-0084 decision 2: a newer database refuses the launch). With many terminals the binaries cannot all change at
   one instant.
3. Moving the runners (D6, D7) needs an order: the service must be up before any window loses its runner, or
   drafts stop committing and remote registration stops.

**This is its own slice with its own brainstorm**, because (2) runs into an earlier decision. ADR-0084 refused a
mismatched window to avoid a wrong rendering. Rolling terminals forward without stopping anyone needs a window of
build N to keep working against a database of build N+1. That is only safe if the additive-only evolution rule
(ADR-0012) is enough to make an older window's reads correct. Whether to relax ADR-0084 decision 2 that way, how
DDL takes its locks (a `lock_timeout` with retry, or an online pattern), and the upgrade order are the maintainer's to
settle in that brainstorm. **The link and key slices must not ship to a site until it is settled.**

## 7. Paper-parity

Registration's step count is unchanged (`N` = `M`). Terminal setup is not a clinical workflow. The plan owes a
budget for the added round trip on Sign and on a chart read over the LAN, measured in its RUNBOOK section.

## 8. Left to the builder

- Verb and flag names; the service's own database login and grants. It needs the trust set, the request doors and
  the commit doors; the matcher's or `serve`'s login are candidates; least privilege applies.
- The gateway's listen address and port, and how the loopback Postgres address is configured.
- How a revoke reaches the gateway (NOTIFY or poll), and the deadline length for a registration request.
- Whether the trust set rides the local-state export (§5).
- The TLS client connector: hand-written over `tokio-rustls` (`MakeTlsConnect`) or an MIT/Apache crate. Check the
  licence and the three lockfiles before adding any crate.
