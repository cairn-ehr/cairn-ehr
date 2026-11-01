# The window never holds the owner's keys, and never talks in clear across a wire (#898 + #899) — design

**Date:** 2026-11-01 · **Status:** approved by the maintainer 2026-11-01, awaiting a plan ·
**Closes:** [#898](https://github.com/cairn-ehr/cairn-ehr/issues/898), [#899](https://github.com/cairn-ehr/cairn-ehr/issues/899) ·
**Amends (by a new ADR, never an edit):** [ADR-0083](../../spec/decisions/0083-the-human-key-vault.md)'s interim boundary ·
**Related:** #102 (its `NoTls` bullet), the sign-in design
[`2026-10-31-gui-sign-in-and-authorship-design.md`](2026-10-31-gui-sign-in-and-authorship-design.md) (slice 5).

## 1. Why

Sign-in slice 1 (PR #897, ADR-0083) put every clinician's sealed signing key in the node's database. The vault's
promises ("no offline cracking; the back-off cannot be skipped") hold only against a connection that is merely the
runtime role `cairn_agent`. Two facts break them today:

- **#898:** the window keeps working on the connection string it was launched with, which in every documented
  setup is the schema **owner**. That connection can `SELECT sealed FROM vault_member` and guess offline, and can call
  `enroll_actor` directly. Anyone who copies the window's connection string gets the same.
- **#899:** every database connection is opened `NoTls`. A window on another machine sends the passphrase and
  receives the unwrapped signing seed in clear on the LAN, and every clinical read crosses it in clear too.

**Where the record will live (maintainer, 2026-11-01).** Cairn cannot assume where a computer sits or who installs
and maintains it: a server room, a comms cupboard, a doctor's office; dedicated IT (often poorly run, often
remote) or the clinician and front-desk staff. Long term, **every (or nearly every) terminal is its own node**
(sign-in slice 5): the window talks to the Postgres on its own machine, and clinical data, and later credentials,
travel between nodes over the existing pinned node-to-node link. That is how a clinician walks between configured
nodes without setting up a password on each one, and how a network outage stops being a clinical outage. This
design does NOT build that. It makes the window safe on the node it is on, until then and after.

## 2. Goals, non-goals, anti-goals

**Goals**
1. **Owner credentials are used only by install and upgrade.** Install creates the window's own limited login and
   that is what the window is given. The window never loads or replays the schema.
2. **A window connected with more power than its role (owner, superuser, or any role that can read the vault's
   sealed blobs or bypass the floor) does not sign.** Every key release is refused: an act's signature, "Enrol
   staff", the terminal lock's passphrase. Reading charts still works. The window says plainly, on screen, what an
   operator must fix. It never carries on quietly.
3. **A connection to a database on another machine is refused when it is unencrypted.** No TLS exists yet, so
   today that means every connection whose host is not this machine: a Unix socket or a loopback address only.
4. **The safe arrangement is the default and the unsafe one is impossible, not merely discouraged** (maintainer: a
   protection that depends on an operator remembering a step will be skipped somewhere).

**Non-goals**
- TLS on the database connection. A pinned, installer-generated encrypted link for terminals that stay thin is
  **#909**. Until it or slice 5 lands, a facility has one usable terminal: the one on the
  database's machine. That is acceptable only because nothing clinical runs yet, and it makes one of the two a gate
  on any multi-terminal pilot.
- Credentials syncing between nodes (slice 5).
- A graphical installer. "Install" here means the existing `cairn-node` console steps, extended.
- **A Cairn-installed Postgres** on its own port with secure defaults (maintainer's idea, 2026-11-01). It is the
  right default and its own project, **#910**: it makes Cairn responsible for patching Postgres
  through the signed distribution plane (ADR-0012), including major-version upgrades, and bring-your-own Postgres
  must stay supported (large hospital IT). D1a is exactly the check a bundled server passes by construction, so
  nothing here forecloses it.

**Anti-goals: a build that does any of these has failed even if it works**
- **An override flag** (`--allow-insecure-remote`, `--i-know-what-im-doing`) for either refusal. The maintainer
  agreed: no one opts into a signing seed in clear on a hospital network.
- **`SET ROLE` as the fix for #898.** `RESET ROLE` undoes it on the same session, and the owner password stays in
  a connection string anyone can copy. The window must never be HANDED owner credentials.
- **Locking a clinician out of reading** because the setup is wrong. A wrongly connected window reads (that is
  paper-parity for the record lying open) but cannot sign, and says why.
- **A confirmation dialog** standing in for either refusal (principle 3).

## 3. What was decided

**D1. Install provisions the window's login.** The owner-run install step (`cairn-node init`, or a verb beside it;
builder's choice) creates one login role for windows, granted ONE new NOLOGIN group role defined in a new `db/`
file that holds exactly the window's grants (D3). *Designer's call:* not `cairn_agent` (shared with the matcher, so
the window would be indistinguishable from the advisory actor) and not `cairn_node` (it carries the peer-admission
doors `apply_remote_event` / `submit_node_event`; `apply_remote_event` legitimately skips the registration-first
rule, #345, so a window must never hold it). It sits alongside the
existing `provision_runtime_role` (which provisions the daemon's `cairn_node` login, `db.rs`). The connection
string the operator gives the window names that role, and the role has **its own password** (SCRAM), so the
owner's password stays with whoever installed the node.

**D1a. Install refuses a database that lets anyone in without a credential (maintainer, 2026-11-01).** Without
this, D1 protects nothing: on a cluster whose `pg_hba` says `trust` for local or loopback connections (Homebrew and
many defaults do), anyone at the window rewrites `user=` to the owner's name and is in. So the owner-run install
step reads the cluster's own rules (`pg_hba_file_rules`, readable by the installing superuser) and **refuses to
finish** while any rule admits a local, loopback or reachable connection by `trust` (and names the line to change).
Tests do not go through the install step, so the dev and CI clusters (which use `trust`) are unaffected. A `peer`
rule with no user map is equally a hole if the window's OS user can name the owner; the builder decides whether the
check refuses it or reports it, and says which in the ADR.

**D2. The window never loads the schema.** It connects with the non-loading `db::connect` and reads the
`node_schema` stamp. If the stamp does not match this binary's embedded generation and hash, it applies the SAME
verdicts the loader's gate gives today (refuse a newer database; on an older or unstamped one, refuse and name the
owner-run command that upgrades it). The window's startup `connect_and_load_schema` goes. *Designer's call:* a
mismatch refuses the LAUNCH, as today's loader gate already does, rather than drawing a read-only window: reading
through code that does not match the schema risks a wrong rendering (a precise untruth), the remedy is one
command, and nothing a clinician can do today is lost. The refusal names the command.

**D3. The window's role holds only what its commands need.** Least privilege, with each grant pinned by a test that
runs that command as that role. Today nothing tests the window under anything but owner, so **this inventory is the
biggest hidden cost of the work** and may surface schema objects nobody granted to a runtime role. A missing grant
is fixed in its `db/` file per the replay rules (durable-rules; memory note on replay, no view-widening across
files); a grant that must NOT be given (the vault's three tables, `cairn_vault_unseal`) stays refused and is
pinned refused.

**D4. Signing is refused on an over-powered connection.** One pure predicate over facts read once per connection
(superuser? owns a Cairn table? can SELECT `vault_member`? can raw-INSERT `event_log`? can EXECUTE a peer-admission
door, `apply_remote_event` or `submit_node_event`? The last makes the daemon's own `cairn_node` login
over-powered for a window too), re-read when the window
reopens a lost connection (#807). Its "no" is window state the signer strip, enrol and terminal lock all read, so
no path releases a key around it. The vault's own doors stay as they are (defence in depth); this is the window
refusing before it ever sends a passphrase.

**D5. A non-local unencrypted connection is refused where a passphrase or a seed would cross it (maintainer,
2026-11-01).** A pure function over the parsed `tokio_postgres::Config`: every host must be a Unix socket or a
loopback address (`localhost`, `127.0.0.0/8`, `::1`), and `hostaddr`, when given, must be loopback too. It applies
to **the window** (every connection it opens, including #807's reopen) and to **the `cairn-node` console verbs that
send a passphrase** (`vault-bootstrap-superuser`, `vault-set-terminal-passphrase`, and any later vault verb). It is
NOT put in `db::connect_with_task` for every caller: the `cairn-node` daemon, `cairn-sync`, the matcher and the test
rigs (the DGX rig runs over WireGuard, which is encrypted but invisible in a connection string) stay as they are,
their clear-text link still tracked by #102 until database TLS exists. The window's refusal is a `ConnectFailed`
its reopen reads as a refusal, not an outage (#880/#884; the "must stay a `ConnectFailed`" rule in `db.rs`). An SSH tunnel to `localhost` is an
encrypted link and passes; a plain-TCP port forward to `localhost` cannot be detected and is out of scope (it takes
deliberate effort, which is the bar).

**D6. A new ADR records D1–D5** and amends ADR-0083's "holds only for a `cairn_agent` connection, owed #898" and
its `NoTls` caveat (ADR-0083 is immutable; the new ADR says what changed and why). Spec version bumps. Every
RUNBOOK step that launches the window with the owner `$CONN` (`cargo run … -p cairn-gui-tauri -- --conn "$CONN"`)
switches to the window's login; the owner `$CONN` stays for the `cairn-node` console steps.

## 4. Risk recorded for slice 5 (not this work)

If the vault's sealed blobs sync to every node, **a stolen terminal disk holds every clinician's sealed key**, open
to offline guessing at the full Argon2 cost per guess with no counter; and the attempt counter is per node, so an
attacker with N nodes gets N× the online guesses. Slice 5's design must answer both (for example a blob that opens
only on the node it was issued to, or an attempt budget shared across the node set). Copy this paragraph into
slice 5's brainstorm.

## 5. Left to the builder

- The window role's name and the exact verb shape of D1.
- How DB-gated window tests run under the limited role. `SET ROLE` is acceptable IN TESTS (the threat is a
  production connection string, not a test session); the over-power predicate reads `current_user`, which
  `SET ROLE` changes.
- The on-screen wording of the D2/D4 refusals, under the house rules: say what the USER must do, name the lightest
  command, walk the sentence through every state it is shown in.
- Whether `cairn-node`'s own console verbs also refuse on an over-powered connection. They are run by the owner by
  design; the window is the subject here.

## 6. Paper-parity

Not clinical-surface: this is the connection and install posture. A correctly installed terminal's sign gesture is
unchanged in steps and time (RUNBOOK §16's budget stands). The new refusals appear only on a misconfigured install.
