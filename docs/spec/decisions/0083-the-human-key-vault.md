# ADR-0083 — The human key vault

- **Status:** Accepted
- **Date:** 2026-10-31
- **Spec version at acceptance:** 0.85
- **Issues:** draft PR [#897](https://github.com/cairn-ehr/cairn-ehr/pull/897) (GUI sign-in, slice 1) — design approved
  by the maintainer on 2026-10-31; design: `docs/superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md`;
  plan: `docs/superpowers/plans/2026-10-31-gui-sign-in-slice-1.md`
- **Builds on:** [ADR-0008](0008-point-of-care-identity-possession-and-salvage.md) (gatekeeping is separate from
  attribution) · [ADR-0053](0053-per-write-human-authorship.md) (the human signs, the node seals) ·
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) (a node's signing key is never backed up) ·
  [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) /
  [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md) (the actor registry and its fail-closed enrol) ·
  [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md) (hard policy in the database, soft policy in the UI)
- **Code:** `db/061_vault.sql`, `crates/cairn-node/src/vault.rs`, `crates/cairn-event/src/signer.rs`,
  `cairn-gui/cairn-gui-tauri/src/{signer,state,enrol,commands}.rs`, `src-ui/{signer,enrol}.js`, and the
  `cairn_vault_unseal` primitive in `extensions/cairn_pgx` (0.4.0)
- **Errata:** **E1**–**E3**, appended 2026-11-01 when sign-in slice 2 built what decision 2, decision 4 and the
  Consequences said slice 2 owed ([ADR-0085](0085-in-person-enrolment-and-re-key.md)). Decision 7's rules on who
  may enrol and the release door's role → purpose map are *amended* by ADR-0085, not corrected here.

## Context

Every clinical write needs its human author's Ed25519 signature (ADR-0053). Until this slice the only way the
reference window got one was a key file named at launch and unsealed at the keyboard. That works for one clinician
on their own machine. It fails a ward, where three or four clinicians share a machine, and it fails an emergency
department, where about ten nurses share one terminal. If the next nurse must log the last one out and herself in
before she can write, the documentation does not get done.

The paper chart has the answer, and ADR-0008 already names it. The chart lies open, anyone on staff reads it, and
each entry carries its writer's own signature. Deciding who may touch the terminal (gatekeeping) is rare and coarse.
Deciding who wrote *this* entry (attribution) happens on every write and must cost about as much as a pen.

So each clinician's key has to be reachable from any terminal with nothing but their staff ID and passphrase. It
has to live somewhere other than a file on one machine. Four questions follow, and this ADR answers them:

1. Where does the key live, and does it go into backups?
2. How does a signature get made without the key being handed around?
3. Who is allowed to put a person into the vault?
4. How long does a key stay usable at a terminal after one signature?

## Decision

### 1. The keys live in one table on the node, and no role is granted a read of them

- Each member's 32-byte Ed25519 seed is stored in `vault_member.sealed`, sealed under that member's own passphrase.
  It uses the same sealed-key format the key file used (`cairn_keystore::seal`, Argon2id then an AEAD).
- `vault_member`, `vault_attempt` and `vault_terminal` are granted to nobody: not `PUBLIC`, not `cairn_agent`, not
  `cairn_node` (db/061 `REVOKE ALL …`). No function or view returns the `sealed` column. The runtime role
  `cairn_agent` gets `insufficient_privilege` if it tries to read it.
- The unsealing primitive `cairn_vault_unseal` is revoked from `PUBLIC` too. For every role but the owner, the only
  way to reach it is the release door, `cairn_vault_release` (a `SECURITY DEFINER`), which counts every guess
  (decision 4). db/061 refuses to load if that REVOKE did not take. Both run only when the schema is replayed, so a
  `cairn_pgx` reinstall, which re-grants `PUBLIC`, goes unseen until the next replay
  ([#908](https://github.com/cairn-ehr/cairn-ehr/issues/908)). That is defence in depth lost rather than a bypass:
  the primitive needs a sealed blob, which the runtime role cannot read.
- **What the grants cannot do.** The table owner and any PostgreSQL superuser can still read `sealed`; grants never
  bind them. And **today's window connects as the owner**: it loads the schema at launch with the same connection
  string it then works on, and it does not switch to `cairn_agent`. So the promise holds against the runtime role
  and against a client given only that role, not yet against a modified window. Running the window as a role that
  is only `cairn_agent` is owed ([#898](https://github.com/cairn-ehr/cairn-ehr/issues/898)). Until then this is the same interim boundary as decision 3: the
  window is trusted code.
- Nothing syncs the vault. No sync, capture or export code reads these tables. The keys stay on the node whose
  database the window is connected to. In the interim deployment (decision 3) that is the facility node.

Why it matters: a sealed blob that leaves the database is an offline brute-force target, where no back-off can
reach the guesser. While it stays inside, every guess goes through the back-off, for every connection that is only
`cairn_agent`. Two things stand outside that today: the window connects as the owner, which can read the blobs
([#898](https://github.com/cairn-ehr/cairn-ehr/issues/898)), and over a remote link with `NoTls` the passphrase and
seed travel in clear ([#899](https://github.com/cairn-ehr/cairn-ehr/issues/899)).

### 2. The vault is not in any backup, and after a disk loss every clinician needs a new key

ADR-0026 keeps a node's private signing key out of every backup, so a stolen backup can never bring a signing
identity back to life. The same argument applies to a person. A stolen backup holding wrapped human keys would let
an offline guesser try passphrases until one opens, and then sign as that clinician. Clinicians' passphrases are
weak, so that is a real attack. The vault therefore stays out of backups.

- **What Cairn's own backup carries.** The backup medium captures the two signed event planes, `node_event` and
  `event_log`. The sealed local-state export carries the custody rows, the node's unwrap secret and the actor
  registry (`actor_event`). Neither reads a vault table.
- **What Cairn cannot stop.** An operator's own `pg_dump`, a filesystem snapshot or a physical replica of the
  cluster copies every table, the vault included. Those copies are outside Cairn's backup path. An operator who
  makes one must leave out the `vault_*` tables, or keep that copy as carefully as the live node.
- **What recovery is today, stated honestly.** The planned recovery is re-key by token: `rotate-key` gives the same
  actor a new key, so their identity survives (design §5.5). That is slice 2, and it is **not built**. After a
  restore today:
  - the actor registry comes back with every clinician's old public key, and the vault comes back empty, so
    nobody can sign through the window;
  - enrolling the same person again with the same role and staff ID is **refused** by the registry's fail-closed
    guard (ADR-0044), whatever the staff ID's case or surrounding spaces. The `actor_id` is computed from the pinned
    set, which holds the role and the normalised staff ID (decision 6), and the restored registry already binds that
    `actor_id` to the lost key;
  - so the only path today is to enrol each person under a **new** staff ID. That makes a new actor, and the link
    between them and what their old key signed is lost. This is one reason for the design's rule that there is
    **no real clinical use before slice 2**.
- **A forgotten passphrase** has the same answer today: a superuser enrols the person under a new staff ID. There
  is no revoke door yet, so the old member row and key stay enrolled.

  > **Erratum E1 (2026-11-01) — factual; the decision is unchanged.** Recovery is now built, in person, not by
  > token ([ADR-0085](0085-in-person-enrolment-and-re-key.md) decision 5). A forgotten passphrase is a re-key: a
  > superuser or an enroller, with the person present, gives the SAME actor a new key by a `rotate-key` row, and the
  > person keeps their staff ID and history. After a restore the re-key door re-creates the member row on the
  > existing actor: the first superuser is re-keyed at the console (`cairn-node vault-rekey-superuser`), then
  > everyone else in person (from the window only once its form sends a display name and contact; ADR-0085 says
  > why). Enrolling under a new staff ID is no longer the path. **Re-key by token** (to an outside contact) is
  > still **not** built: it waits for [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909) or slice 5.

### 3. Interim: the vault releases the key to the window for one act

The passphrase is checked **inside PostgreSQL**. `cairn_vault_release(staff_id, passphrase, purpose)` unseals the
blob there and, if the passphrase is right, returns the seed. The seed travels over the window's node connection to
the window, which builds the signing key and checks that it matches the key ID it was filed under (a mismatch is
refused, never signed with). The window signs the act and then drops the key, unless the terminal holds keys
(decision 8).

This is a stated interim, not the end state, and it is **wider** than the boundary the window already had. The
window holds the **node's own** signing key (`--key`), which has custody of every sealed body. Custody is not human
authorship: the node key alone never let anyone sign as a clinician. Owner access to the vault, which the window has
today ([#898](https://github.com/cairn-ehr/cairn-ehr/issues/898)), is a path to exactly that: the owner can read
every sealed blob, and any clinician whose passphrase falls to an offline guess against their blob can then be
signed for. The interim is tolerable for slice 1 only because the design allows **no real clinical use before
slice 2** (see "Not yet guaranteed" under Consequences).

**Slice 5 owes an in-database signer.** The signature primitives (`cairn_event::sign`, `sign_attestation`) and every
window call site take the `cairn_event::EventSigner` trait, so a signer whose key never leaves the database can be a
new implementation of that trait rather than a rewrite of every call site. Two console-side APIs still take a
human's `&SigningKey` and must move before slice 5: `identify::LinkParams.human_sk` and `apply_accepted_proposal`.
New code must take a signer (`&dyn EventSigner`), never a human's `&SigningKey`.

Two more facts belong to this interim, and the in-database signer must cover them too:

- **Enrolment mints the new member's key in the window's process.** `cairn_node::vault::enrol_member` generates the
  key and seals it under the new member's passphrase, then sends only the sealed blob to the database. The new
  member's passphrase never reaches the database, but it and the fresh seed are briefly in the window's memory.
- **The connection is not encrypted.** The node's connection code opens PostgreSQL connections with `NoTls`. A
  window on the same machine as the node, connected over loopback (`host=127.0.0.1`, as in RUNBOOK section 0) or a
  Unix socket, never puts the passphrase on the network, which is fine. A window on another machine sends the
  passphrase, and receives the seed, in clear over the network. Until the connection carries TLS
  ([#899](https://github.com/cairn-ehr/cairn-ehr/issues/899)), a terminal on another machine must reach the node only over an encrypted link, such as a VPN tunnel.

### 4. Every guess is counted, before anything is revealed, and the wait is capped at five minutes

- **The schedule.** Failures 1 to 3 are free, because people mistype. From the 4th failure, the member must wait
  `least(2^(failures-4), 300)` seconds before the next try: 1 s, 2 s, 4 s and so on, capped at five minutes
  (`cairn_vault_backoff_seconds`). A success resets the count.
- **A try during the wait** answers `waiting` with the milliseconds left. It does not check the passphrase and does
  not count as a failure, so a clinician pressing again too soon cannot dig herself deeper.
- **There is no hard lock-out.** A clinician locked out in an emergency is a paper-parity failure: paper never
  refuses a pen. Five minutes per guess already makes an online guess at a real passphrase hopeless.
- **But someone else's wrong guesses can keep a member waiting.** Failures do not decay; only a success resets them.
  Anyone who knows a staff ID (the strip shows the holder's) can push that member to the five-minute cap, and one
  wrong guess per wait keeps them there. At the window she still gets in by pressing after a wait ends and before
  the next wrong guess. A script on the runtime role can guess as each wait ends and hold her out for as long as it
  runs. Nothing but the owner's hand SQL resets the count today. This is an open availability defect, not a
  settled trade-off ([#903](https://github.com/cairn-ehr/cairn-ehr/issues/903): decay, a console reset verb, or
  per-source throttling).
- **The count lives where a rollback cannot undo it.** An ordinary counter column is transactional. A caller could
  read the answer `wrong` and then roll back, and the count would be gone, so they could guess forever at full
  speed. So each member has two PostgreSQL sequences, created by a trigger when the member is inserted: an attempt
  counter and an attempt clock (milliseconds since the epoch). Sequence changes are never rolled back. The release
  door moves both **before** it unseals, so the guess is on record whatever the caller does next. The only
  transactional piece is `vault_attempt.success_mark`, the attempt number of the last success, and
  failures = attempts − success_mark. Rolling back a success therefore leaves that attempt counted as a failure. It
  can only ever cost the caller.
- **`setval`, not `nextval`.** Both are non-transactional, but `nextval` writes the sequence to the WAL 32 values
  ahead of what it hands out, and crash recovery restores that value. A member who mistyped once just before a
  power cut would wake up with about 32 phantom failures: made to wait up to 300 s with the right passphrase. The
  door sets the exact next value with `setval`, which logs that value exactly, so recovery never invents an attempt.
  (It can lose one: a rolled-back guess's WAL record is flushed by the WAL writer, not by a commit, so a crash in
  that short window may forget that single count — [#907](https://github.com/cairn-ehr/cairn-ehr/issues/907).)
- **A clock stepped back never locks anyone out.** The last attempt is stamped with the wall clock. If the clock is
  then stepped back (an NTP correction, a dead clock battery after a power cut), that stamp lies in the future and
  the time since the attempt is unknown. The door then checks the guess rather than wait an unknown time, and
  restamps the clock at now, so the back-off resumes from that attempt. A guesser gains at most one guess per
  backward step (`a_clock_stepped_back_never_locks_out`, `a_clock_stepped_back_frees_one_guess_and_no_more`).
- **A missing attempt row fails closed.** The `vault_attempt` row is both the lock and the success mark. A member
  without one (only a hand `DELETE` can do that) could not be backed off, so the door answers `unknown`, unseals
  nothing and writes nothing.
- **Order inside the door.** Look up the member, check the purpose, and check that the member's actor is still
  current: a revoked or superseded key answers `not-current`, since the floor would refuse everything it signed
  (no writes, nothing counted). Lock the attempt row
  `FOR UPDATE`, read the failures and the last attempt time (no writes), and answer `waiting` if still inside the
  wait. Then count the guess, then unseal. Two terminals guessing the same staff ID at once are serialised by the
  row lock (`a_second_terminal_waits_for_the_attempt_row_lock`).

  > **Erratum E2 (2026-11-01) — factual; the decision is unchanged.** *"a revoked or superseded key answers
  > `not-current`"* was not guaranteed for a superseded key when written: `actor_current` did not drop a superseded
  > actor ([#664](https://github.com/cairn-ehr/cairn-ehr/issues/664)), so whether the old key read as current hung on
  > which actor_id a `supersede` row sat on, which [#666](https://github.com/cairn-ehr/cairn-ehr/issues/666) had left
  > open (no door wrote a `supersede` then). Since sign-in
  > slice 2 (db/004) it drops a superseded actor, and `not-current` also covers a key that was **rotated away** or
  > **repudiated** ([ADR-0085](0085-in-person-enrolment-and-re-key.md) decision 1). The door also gained two answers,
  > `replaced` and `expired` (ADR-0085 decisions 7 and 10); the order of checks and counting above is unchanged.
- **The residual, stated plainly.** A raw-SQL client that calls the door and then keeps its transaction open holds
  that row lock, and delays that one member's next sign-in until it lets go; each press meanwhile fails when the
  window cuts its own command off (`ANSWER_WITHIN`, 15 s). That is a denial of service by someone who already holds
  the runtime role. It is never a way around the count.
- **Whose guesses these are.** Everything above binds a guess made through the release door, which is every guess
  from a connection that is only `cairn_agent`. A connection that can read `vault_member.sealed` can guess offline,
  where no count reaches: the window today connects as the owner
  ([#898](https://github.com/cairn-ehr/cairn-ehr/issues/898)), and so does anyone holding a copy of the database
  (decision 2). And over a remote link with `NoTls` the passphrase itself travels in clear
  ([#899](https://github.com/cairn-ehr/cairn-ehr/issues/899)).

### 5. The passphrase is never in an error or a log line, as far as Cairn controls it

- `cairn_vault_release` **never raises** for a wrong passphrase, an unknown staff ID, a forbidden purpose or a wait.
  Each is an answer: a row with an `outcome`. `cairn_vault_unseal` likewise returns `NULL` rather than raising. A
  raised error can carry the statement's bind parameters, the passphrase among them, into the server log. (At
  `REPEATABLE READ` or `SERIALIZABLE` the row lock can raise a serialisation failure, and a timeout can cancel the
  call. None of these loses a count.)
- The enrol doors re-raise a violated constraint with PostgreSQL's message alone, which names the constraint but
  not the values, because PostgreSQL's own error detail would print every column, the sealed blob included.
- The window builds every refusal sentence from an enum (`KeyRefusal`, `EnrolRefusal`). No database text and no
  passphrase reaches the screen through one. A fault's detail goes to the operator's stderr. (What the screen does
  show besides: the held staff ID, the member's own hint, which is database text, and an enrolment's success line,
  which names the new member.)
- An enrolment that fails at any step with no ordinary answer from the server (a lost connection, a cut-off) is
  said to be of unknown outcome: "check before enrolling them again, and never under a new staff ID", not "nothing
  was recorded". A retry would be told the staff ID is taken, and enrolling the person under another one makes two
  actors. It errs towards "unknown" even when the failure came before the write, which claims less, never more.
- **Statement logging can still write the passphrase and the sealed blob, and only the operator can stop it.**
  PostgreSQL logs bind parameters with the statement when `log_statement = 'all'`, `log_min_duration_statement`,
  `log_min_duration_sample` (with `log_statement_sample_rate`) or `log_transaction_sample_rate` logs the call (unless
  `log_parameter_max_length = 0`), and on an error when `log_parameter_max_length_on_error` is not 0. The
  passphrase is a bind parameter of `cairn_vault_release`; the sealed blob is one of `cairn_vault_enrol` and
  `cairn_vault_bootstrap`, which raise on every refusal, so error logging writes the blob, an offline guessing
  target, on every refused enrolment. **Operators must keep these settings off for the role
  the window connects as.** RUNBOOK §16 says how to check.

### 6. Staff IDs are normalised in one place, in SQL

`cairn_vault_staff_id(x)` is `lower(btrim(x))`, with the `pg_unicode_fast` collation pinned so every node maps case
the same way. The table's CHECK, the enrolment address and every lookup call it, so `" JSmith "` typed at a terminal
finds the `jsmith` who was enrolled. There is no Rust copy of this rule: the Rust code asks the database for the
enrolment address and the normalised staff ID instead of computing them. (The window trims spaces from the enrol form's fields before it sends
them, which the database rule then does again; case is only ever folded in SQL.)

The new actor's pinned set follows the same rule. It is exactly `{role, handle}`, and its person-distinguishing
determinant, `handle`, is the normalised staff ID: `cairn_node::vault` reads it back from `cairn_vault_staff_id`, and
both enrol doors rebuild the set themselves and refuse any other (another handle, an extra determinant, or a handle
that is not normalised). So `Boss` and `boss` are one actor, and a case variant of a staff ID can never mint a second actor for one person.
The role is compared exactly: `Nurse` and `nurse` under one staff ID would be two pinned sets (the vault's primary
key refuses the second member, but a restore that lost the row would not).

### 7. The superuser enrols people and signs nothing clinical

- A **superuser** is a vault member whose role is `superuser`. The role administers the site. It confers no clinical
  authorship. A person who is both a clinician and a superuser has two members, with two staff IDs and two keys.
- **Only a superuser can enrol, and the database checks it.** `cairn_vault_enrol` refuses unless the named superuser
  is a member with role `superuser`, their key is a current (enrolled, not revoked) actor, the pinned set is exactly
  the one the door builds for this request (decision 6), and the request carries that superuser's attestation token over the enrolment address.
  The address is a sha2-256 over the normalised staff ID, the new key and the role (`cairn_vault_enrol_address`), so
  a token cannot be reused for another person, key or role. A database login alone is not enough: the window's own
  connection has one. (As in decision 1, this binds every role but the owner, who can write the tables directly.)
- **The token is kept** in `vault_member.enrol_token`, so who approved whom can be checked again for as long as the
  row stands. It is custody state on that row, not an append-only event. A console bootstrap stores `NULL`, because
  no superuser approved it.
- **The release door matches each key to its one purpose.** A superuser's key is released only for `enrol`. Every
  other member's key is released only for `clinical`. Any other pairing answers `refused-purpose`, checked before
  anything is unsealed. This holds at the vault, not only in the window.
- **The window never holds a superuser's key.** Enrolment releases it, uses it once to sign the token, and drops it,
  whatever the terminal's key hold says. It does not go through the code path that holds keys.
- **The runtime role now has one gated path to `enroll_actor`.** `cairn_agent` still has no grant on
  `enroll_actor` and no write grant on `actor_event`. It may call `cairn_vault_enrol`, a definer that calls `enroll_actor` only after
  the checks above (db/004 records this exception).
- **The first superuser** is made at the node's console: `cairn-node vault-bootstrap-superuser`, through
  `cairn_vault_bootstrap`, which is granted to nobody, so only the database owner (or a PostgreSQL superuser) can
  run it. Whoever holds those credentials already holds the site.
- **Known gap: the purpose rule is enforced only at the vault.** A modified client that has a superuser's key
  released for `enrol` could use it to sign a clinical event, and the strict door would admit it, because the
  superuser is an enrolled, current actor. That event is honestly attributed to the superuser role-actor, so it is
  not impersonation of a clinician, but a superuser could chart as "superuser". Closing it needs the strict door to
  refuse a `superuser` author on clinical event types, a db/005 change outside this slice
  ([#900](https://github.com/cairn-ehr/cairn-ehr/issues/900)).
- **Known gap: a client-built sealed blob sets its own unsealing cost.** `cairn_vault_enrol` stores the blob the
  client sends, and unsealing uses the Argon2 cost parameters written inside that blob. A hostile client holding a
  superuser's credential could file a blob whose cost exhausts a backend's memory when it is released. Bounding
  those parameters in `cairn_vault_unseal` needs an extension release
  ([#901](https://github.com/cairn-ehr/cairn-ehr/issues/901)).

### 8. How long a key stays usable is a per-terminal setting

- The window's `--key-hold-seconds N` sets it. **The default is 0**, a shared terminal. The maximum is 900 s, the
  window's 15-minute idle timeout; a larger value is refused at launch, never quietly cut down.
- **With 0, nothing is held.** The released key signs one act and is dropped. Every act carries a staff ID and
  passphrase, and the next person can never write as the last one.
- **With N > 0** (a personal workstation), the key is held for up to N seconds without activity, so the clinician's
  next acts need no passphrase while they keep working (#288). (A whole-list sign-off is one press, and one
  credential, at any hold.) Using the key counts as activity; the window's own
  timer asking whether a key is held does not.
- **A typed passphrase always wins.** It wipes any held key before the vault answers, so a mistyped passphrase never
  leaves the last person's key signing the next act. An enrolment does the same with the superuser's passphrase.
- **A staff ID with no passphrase never reaches the release door**, where it would count as a wrong guess (it does
  reach the uncounted hint lookup). A passphrase of spaces alone is no passphrase, at sign-in and at every enrolment
  door alike (`cairn_node::vault::passphrase_is_usable`): a member can never be enrolled under one sign-in refuses. On a terminal
  with a hold the strip keeps the staff ID after each act, so the holder's own ID alone (spaces trimmed, otherwise
  exact) signs with her live held key. Anyone else's ID alone wipes the held key and asks for a passphrase.
- **A late release never puts a key back.** The window releases its session lock while the vault checks a
  passphrase. A generation counter is bumped on every wipe (a typed passphrase, another person's staff ID, or the Lock
  button), and a release
  stores its key only if no wipe has happened since its own (`Held::keep_for_next_act`). So a Lock pressed while a release is in flight, or a
  newer signer's press, is never undone.

### 9. The terminal lock is a screen lock, not a security boundary

- One shared staff passphrase per site unlocks the window. It is stored as an Argon2id hash in `vault_terminal`, a
  one-row table, and set at the console (`cairn-node vault-set-terminal-passphrase`, reading
  `CAIRN_TERMINAL_PASSPHRASE` for a script — never the superuser's `CAIRN_VAULT_PASSPHRASE`, or the shared passphrase
  silently becomes the superuser's). The window reads the hash and checks the passphrase itself. The lock is shown
  once, when a live window opens; unlike a screensaver it does not re-lock when idle.
- It is **soft policy** (ADR-0021), and gatekeeping only (ADR-0008). Nothing in the database depends on it. Anyone
  holding the window's database credentials can skip it, and the hash is readable by the window's role. It has no
  back-off, because it guards nothing the doors rely on. Every signature still needs a personal passphrase.
- A node fault while unlocking, or a stored hash that cannot be parsed, is said as a fault, never as a wrong
  passphrase.

### 10. Deferred and gated

- **Sealed emergency tokens move to slice 4.** The design's slice 1 said they would be printed at installation but
  not yet usable. They are not printed in slice 1: a sheet of tokens that do nothing invites someone to try one in
  an outage. Slice 4 prints them when they work. Until then a locked-out last superuser is recovered at the console,
  by bootstrapping a new superuser under a new staff ID.
- **The extension gate.** The release door calls `cairn_vault_unseal`, which is in `cairn_pgx` 0.4.0. db/061 checks
  for that function first and refuses to load without it, so a stale extension fails when the schema loads, with a
  plain message, not at a clinician's first signature.

## Consequences

- **Easier.** The next nurse writes with no log-out and no log-in: nothing is logged in to switch away from. Any
  window connected to the node can sign for any member. "Only a superuser may enrol" and "a superuser's key is never
  released for a clinical act" hold at the vault against a client talking raw SQL with the runtime role. (A
  superuser's key released for enrolment can still sign a clinical event until
  [#900](https://github.com/cairn-ehr/cairn-ehr/issues/900) closes.)
- **Harder.** The vault is new safety-critical surface in the database: a definer that unseals keys, dynamic
  per-member sequence DDL, and a lock held across an Argon2 check. The rules are pinned by tests wherever a test can hold them (`vault_floor.rs`,
  `vault_lib.rs`, the window's `signer_tests.rs`, `enrol_tests.rs` and `state.rs` tests), and
  `docs/developers/durable-rules.md` lists the ones a tidy-up must not undo, including one held only in review.
- **Not yet guaranteed.** Until these close, the vault's promises hold only against a connection that is only
  `cairn_agent`, which is why there is **no real clinical use before slice 2**:
  - the window connects as the owner, so it can read every sealed blob and guess offline
    ([#898](https://github.com/cairn-ehr/cairn-ehr/issues/898));
  - a remote terminal's `NoTls` connection carries the passphrase and the seed in clear
    ([#899](https://github.com/cairn-ehr/cairn-ehr/issues/899));
  - a copy of the database an operator makes (`pg_dump`, a snapshot, a replica) carries the sealed blobs out of
    reach of the back-off (decision 2);
  - another person's wrong guesses can keep a named member waiting at the five-minute cap
    ([#903](https://github.com/cairn-ehr/cairn-ehr/issues/903), decision 4);
  - an unreadable sealed blob reads as a wrong passphrase, and is counted
    ([#904](https://github.com/cairn-ehr/cairn-ehr/issues/904)).
- **What we are betting on, and how we would know it failed.**
  - That the interim release is safe enough until slice 5. We would know it failed if a window process were
    compromised: it would see seeds as they are used, and, while it connects as the owner, could read every
    sealed blob and guess at them offline. The same compromise gets the node's key, but that is custody of sealed
    bodies, not human authorship: the vault is what would let it sign as any clinician whose passphrase falls to
    an offline guess.
  - That no operator logs bind parameters for the window's role. A passphrase in a server log is that failure.
  - That a back-off capped at five minutes beats a lock-out. A successful online guess at a real passphrase would
    show it is not; a clinician held out by someone else's guesses ([#903](https://github.com/cairn-ehr/cairn-ehr/issues/903)) shows the other side of
    the same trade.
- **Owed by later slices.**
  - Slice 2: re-key by token and `rotate-key`, so a forgotten passphrase or a restore keeps the person's identity.
    This is the human key-loss ceremony security §7.5 named as an honest gap; this ADR places the key, slice 2
    still owes the ceremony.

    > **Erratum E3 (2026-11-01) — factual; the decision is unchanged.** Slice 2 built `rotate-key` and the
    > ceremony **in person** (a superuser or an enroller re-keys the same actor), and the re-key after a restore
    > ([ADR-0085](0085-in-person-enrolment-and-re-key.md)). It did **not** build re-key **by token**, which waits for
    > [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909) or slice 5.
  - Slice 4: sealed emergency tokens and the emergency key.
  - Slice 5: the in-database signer (behind `EventSigner`), with key generation in the database too, and actor
    sync with specified nodes.
  - Follow-up issues: the window running as the owner, [#898](https://github.com/cairn-ehr/cairn-ehr/issues/898)
    (decision 1); TLS on the window's connection, [#899](https://github.com/cairn-ehr/cairn-ehr/issues/899)
    (decision 3); the superuser-as-clinical-author gap,
    [#900](https://github.com/cairn-ehr/cairn-ehr/issues/900), and the Argon2 cost bound,
    [#901](https://github.com/cairn-ehr/cairn-ehr/issues/901) (decision 7); the back-off held open by another's guesses,
    [#903](https://github.com/cairn-ehr/cairn-ehr/issues/903), and integrity faults that read as user errors,
    [#904](https://github.com/cairn-ehr/cairn-ehr/issues/904) (decision 4); the window's DB-gated enrolment tests
    that CI never runs, [#905](https://github.com/cairn-ehr/cairn-ehr/issues/905); the REVOKE a `cairn_pgx` reinstall
    undoes unseen, [#908](https://github.com/cairn-ehr/cairn-ehr/issues/908) (decision 1).
