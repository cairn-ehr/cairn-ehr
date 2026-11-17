# ADR-0109 — The terminal service and the keyless window

- **Status:** Accepted (#909 slice A: no window holds the node's signing key. Slice B, the pinned gateway, and the
  rollout slice follow)
- **Date:** 2026-11-17
- **Spec version at acceptance:** 0.111
- **Issues:** [#909](https://github.com/cairn-ehr/cairn-ehr/issues/909), PR
  [#1152](https://github.com/cairn-ehr/cairn-ehr/pull/1152). Design (maintainer-approved 2026-11-16):
  `docs/superpowers/specs/2026-11-16-pinned-terminal-link-909-design.md` (D1, D6, D7, D8); plan:
  `docs/superpowers/plans/2026-11-16-909-slice-a-terminal-service-keyless-window.md`. Filed by the build:
  [#1153](https://github.com/cairn-ehr/cairn-ehr/issues/1153).
- **Supersedes in part:** [ADR-0089](0089-the-quarantine-commit-and-the-asserted-grade.md) decision 11 (the window
  is the runner), decision 12's two overdue sentences, and the first bullet of its "What is not guaranteed";
  [ADR-0083](0083-the-human-key-vault.md) decision 3's sentence "The window holds the **node's own** signing key
  (`--key`)". Every other decision of both stands. [ADR-0084](0084-the-window-never-holds-the-owners-keys.md) stands as
  written: none of its decisions says the window holds the node key, and its decision 5 (a remote database is refused)
  is slice B's to amend.
- **Builds on:** [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md) (registration carries its search) ·
  [ADR-0075](0075-the-step-3-prompt-is-a-nudge-not-a-completeness-claim.md) (the step-3 prompt is a bounded nudge) ·
  ADR-0084 decisions 1 and 2 (`cairn_window`, and one owner-run provisioning step) ·
  [ADR-0080](0080-a-connect-replays-the-schema-only-when-it-changed.md) (a new `db/083`, `SCHEMA_GENERATION` 83)
- **Code:** `db/083_terminal_service.sql`; `crates/cairn-node/src/{node_key.rs, terminal_service/,
  patient/register_tx.rs, patient/register_request.rs, window_access.rs, conn_posture.rs, note/commit.rs}`;
  `crates/cairn-patient-search/src/prompt_bound.rs`; `cairn-gui/cairn-gui-live/src/funnel.rs`;
  `cairn-gui/cairn-gui-tauri/src/{main,notes_commit,node_key_guard_tests}.rs`

## Context

A thin terminal on another machine (#909) would have needed the node's identity key on its own disk, because the
window loaded it (`--key node.key`) for two node-signed acts: registering a patient (ADR-0061) and the quarantine
commit of an abandoned note draft (ADR-0089 decision 6). Every other window write is signed by a clinician's key, and
names the node only by its public id, as the `recorded` contributor. A stolen terminal holding the node key could sign
as the node and present as it to its peers. So the two acts move to one process beside the database, and every window
drops the key, the one on the database's machine included (design D8).

## Decision

1. **The window holds the node's public id only.** The write paths take a `NodeKey` (`node_key.rs`): `Full` (the
   signing key) or `Public` (the hex id). `kid()` answers from either. `signer()` refuses `Public` with
   `NODE_KEY_NOT_HELD`. The window has no `--key` flag, opens no keystore and runs no commit. Its node id is
   `identity::load_local`'s `pubkey_hex`. Every write it makes is human-signed, so none reaches `signer()`. Pins:
   `node_key_guard_tests.rs` (the code of `cairn-gui-tauri` and `cairn-gui-live` never names `keystore::`,
   `NodeKey::Full`, `&cli.key`, `CAIRN_KEY_PASSPHRASE`, `load_signing_key`, `commit_overdue` or `run_ticks`) and
   `node_key_public_writes.rs`.

2. **`cairn-node terminal-service` does the two acts** (design D1). It refuses a database on another machine
   (`require_local`). It loads the node key without a prompt (`load_signing_key(&cli.key, false)`: a sealed key needs
   `CAIRN_KEY_PASSPHRASE`). Its login cannot replay DDL, so it checks the schema stamp: a database not yet prepared, or
   prepared by another build, is refused naming `provision-terminal-service-role`, and a newer database is refused as
   needing the service built to match it. Before its loop it refuses a login without EXECUTE on
   `cairn_registration_claim()`, naming `provision-terminal-service-role`, and a key whose kid is not the node
   identity's `pubkey_hex` (`terminal_service::startup`). Its loop serves registration requests every 250 ms (up to 50
   per pass), commits overdue drafts every 60 s, and purges old requests every hour. No job's error ends the loop. A
   lost connection is reopened after a 1 s back-off. If its first connection fails at startup, it exits with an error,
   so it runs under launchd or systemd with restart-on-failure (RUNBOOK §38). Its stderr names requests, drafts and
   patients by id only. Its own lines carry no name, note text or clerk's refusal words; a database error adds its
   SQLSTATE and primary message, never DETAIL or HINT (the primary message is the database's own sentence, and a floor
   refusal may word it with a value). A job's repeated identical failure (serving requests, the commit tick's listing,
   the purge) is said once, until it changes or the job succeeds. A draft whose commit fails is said on every commit
   tick, a stuck draft once per process, and a failed reconnect once a second.

3. **It has its own login, provisioned like the window's.** db/083 creates `cairn_terminal_service`, a `NOLOGIN` group
   granted `cairn_window` (never `cairn_node`), plus the queue's service doors below.
   `cairn-node provision-terminal-service-role --role <name>` (default `cairn_terminal_service_login`) runs
   `provision-window-role`'s body with the group, the wording and the password variable
   (`CAIRN_TERMINAL_SERVICE_DB_PASSWORD`) swapped (`window_access::LoginGroup`). It has the same refusals in the same
   order, and it refuses to take over an existing login that is a member of any other group (a window login
   included).

4. **The registration queue** (db/083, design D6). `registration_request` holds a registration's inputs (the name, the
   search's query, the candidates displayed, the day) under an id the window generates. No role is granted the table.
   Each door is a definer function:
   - `cairn_registration_submit` (window) writes the row `pending` with a deadline of `now() + interval '10 seconds'`.
     A re-send with the same inputs returns the row's current status. The same id with other inputs raises.
   - `cairn_registration_status` (window) reads the status, the chart and the refusal.
   - `cairn_registration_expire` (window) marks a still-pending row `expired`. It waits on the row lock (no
     `SKIP LOCKED`), so behind a claim it returns what the service committed.
   - `cairn_registration_claim` (service) returns the oldest pending row whose deadline is after
     `clock_timestamp()`, `FOR UPDATE SKIP LOCKED`. The claim IS that row lock, held until the service's transaction
     ends, so a service that dies leaves the request pending.
   - `cairn_registration_settle` (service) moves a pending row to `registered` (with the chart and its registration
     event) or `refused` (with a reason), and raises on anything else.
   - `cairn_registration_purge` (service) deletes a row a day after it settled or expired, or a day after its deadline
     if it is still pending.

   An unknown request id is refused by a bare `RAISE` (P0001, "no registration request"), as every floor refusal is
   (#633). An id is idempotent only until the purge.

5. **The service re-runs the search and registers in one transaction.** `serve_one` takes the registration lock
   (`REGISTRATION_LOCK`, a transaction-scoped advisory lock) as its first statement, then claims a request, decodes it,
   re-runs its search as of the request's day, cuts the result with `bound_for_prompt`, registers on a savepoint
   (`register_patient_in_tx`), settles the row and commits. The events are unchanged: node-signed, `{node, recorded}`.
   `register_patient` keeps its own transaction for the CLI.
   - **It compares the bounded prompt, not the whole search.** It refuses when the fresh bounded prompt (`PROMPT_CAP`
     = 5) names a chart that the request's `displayed` does not. `bound_for_prompt` is the window's own function,
     moved to `cairn-patient-search` so both binaries use one copy. The design's D6 said "any candidate the displayed
     list does not name", which was wrong: the window shows and attests only the bounded prompt, and that prompt is
     cut on about 92% of registrations (ADR-0075), so an uncut compare would refuse nearly every registration of a
     common name. The cut is safe because `rank_candidates` is a total order (ties break on the chart id), so an
     unchanged record yields the five people (rows) the clerk saw. A displayed chart that the re-run no longer finds
     is not a refusal.
   - **The name must be the one searched for.** A request whose `name` does not give its query's `name_tokens` is
     refused, or a forger could search for nobody and register somebody. The window builds its query from the same
     typed name, so this refuses nothing the window sends.
   - **A request carrying a NUL character is refused when it is decoded.**
   - **A re-run search that is incomplete, with no unseen chart, still registers if the request's displayed list is
     incomplete too.** A request that calls it complete is refused with the new-match words, so the node never signs
     `incomplete: false` over a search it found incomplete; the clerk's search again shows the warning.
   - **Verdicts settle `refused`; accidents stay pending.** A refusal at decode (unreadable, a NUL, a name that is not
     the one searched for), a new match, the floor's P0001 and a refusal decided in Rust before any SQL (a date of
     birth that no precision fits) each settle `refused` with the window's words. Any other error rolls the
     transaction back and leaves the request pending, to be tried again on every pass until its deadline (#1153).
   - **Two services register once.** `SKIP LOCKED` lets one claim a row while the other passes over it
     (`two_servers_register_once`). The registration lock makes registrations take turns across services: a second
     request for the same new patient is searched only after the first commits, and is refused as a new match
     (`two_servers_two_requests_for_one_patient_register_one_chart`), within the bounded prompt's reach (ADR-0075).

6. **The window waits, and is never told "nothing was saved" for a request that may yet go in.** `LiveData::register`
   makes a `Uuid::new_v4` id, reads `today` from the database (`current_date`), submits, reads the status every 200
   ms, and at 10 s (`DEADLINE`, pinned to db/083's interval by `deadline_matches_db_083`) calls expire once. A claim
   taken before the deadline may commit after it: the guarantee is about what the window is told, never wall-clock
   time. A registered request answers with its chart. The new-match refusal reads "Nothing was saved: a new possible
   match has appeared since the search — search again". Any other refusal is shown in the service's words. An expired
   request reads "Nothing was saved, and pressing Register again fails the same way until an operator acts:
   registration needs the node's terminal service (cairn-node terminal-service), which did not answer". The submit is
   the window's commit point: a submit that fails as an outage (not a verdict), and any failure after the submit,
   reads as "not known" (`OutcomeUnknown`, #815).

7. **The quarantine commit runs on the service's tick** (design D7). `terminal_service::commit::commit_tick` runs
   ADR-0089's `commit_overdue` on each overdue draft of every chart, each in its own transaction, at most
   `MAX_DRAFTS_PER_TICK` = 20 a tick, oldest due first and the drafts whose last try failed or was stuck after the
   others, so a backlog drains at 20 a minute. The window's tick and its commit before every notes read are gone, so a
   draft just past due can still read overdue for up to one tick while the service runs. The service writes a per-draft
   fact, `note_draft_commit_failure` (db/083), through `cairn_note_draft_commit_failed`, which only it may call. It
   records a failure for a commit that errs and for a stuck draft, and clears it for a draft that went in or was not
   written (not due, locked, gone). The fact carries no cause, and it goes with its draft (`ON DELETE CASCADE`). The
   door ignores only `foreign_key_violation`, so a draft deleted between its check and its insert writes nothing. The
   window reads the facts through `cairn_note_drafts_commit_failed(patient)`. If that read fails, it is logged and the
   pane shows no failure rather than no notes. The service says a stuck draft on stderr once per process.

8. **ADR-0089 decision 11 now reads:** *The node's terminal service is the runner.* Its loop commits every chart's
   overdue drafts every 60 s, each draft in its own transaction, and records per draft whether that commit failed
   (`note_draft_commit_failure`). The window commits nothing: no tick, and no commit before a chart read. Its notes
   read takes the failed set from the database, never from its own memory.

   **Decision 12's two overdue sentences now read as the first two below, and the third is added** (the other three are
   unchanged; `due_words` decides stuck, then failed, then not put in, then overdue):
   - "overdue — not in the record yet; the node's terminal service puts it in unsigned at its next check"
   - "overdue — could not go into the record by itself (the node's last try failed; the cause is in the terminal
     service's log); sign it, or tell the site's operator"
   - "overdue — the node's terminal service has not put it in the record; sign it, or tell the site's operator",
     once the window's clock is more than three commit ticks (`COMMIT_TICK`, 60 s) past the draft's due time: a
     running service with no backlog would have tried it three times, and the window cannot see whether one runs.

   **Its first "not guaranteed" bullet now reads:** only a running terminal service with an open connection commits.
   While none runs, a draft waits, safe, and every reader sees it overdue. Drafts still never sync.

9. **ADR-0083 decision 3's sentence now reads:** the window holds only the node's public id. The node's signing key is
   held in memory on the database's machine by the terminal service, as it already was by `serve`.

**Paper-parity.** Registration: paper *N* = 3 (search, decide no match, write the card) → forced *M* = 3 → target
*K* = 3. The request and the re-run are machine steps the clerk never sees, and a new match costs one more look, which
is the funnel's purpose (ADR-0061). The budget: Register's round trip grows by at most 0.5 s median on the database's
machine, measured in RUNBOOK §38. The timed commit asks no act of anyone, as before (0 → 0 → 0).

## Consequences

- No terminal needs the node's identity key. Slice B can put a window on another machine without a copy of
  `node.key` on its disk.
- A clinic with one machine runs one more process. Without it there is no registration and no timed commit, the
  window on the database's machine included. One window shape and one code path were preferred over a keyed local
  window (design D8).
- The failed-commit warning no longer depends on which window tried. Every window reading the chart sees it.
- A launch line or unit file that still passes `--key` to the window is refused at launch by its argument parser (clap),
  so the window does not open. The rollout slice owes the upgrade note.

## What is not guaranteed

- **One service per node is assumed.** A second is harmless: the registration lock makes the two take turns, the
  second waiting until the first commits.
- **Registration and the timed commit need the service.** With it stopped, every Register press waits the full 10 s
  before it says the service did not answer. That is slower than paper, where a card can always be written, and no
  window can register a chart until an operator starts it (the owner's `cairn-node patient-register` still can). Overdue
  drafts wait, safe, read "overdue — not in the record yet", and, more than three commit ticks past due, that the
  service has not put them in.
- **"Fails the same way until an operator acts" can be untrue.** A service that was only slow, or restarting, may
  answer the next press. The sentence is not changed in this slice (#1155).
- **The not-put-in sentence reads the window's clock against the node's due time.** A window clock minutes fast says it
  early, a slow one late.
- **Slice B's gateway must refuse any login that is a member of `cairn_terminal_service`.** That group inherits
  `cairn_window`, so a test for window membership alone would pass it, and it holds the claim, settle, purge and
  failure doors.
- **Head-of-line blocking (#1153).** A request that fails every time with an error that is not a refusal is retried on
  every pass and holds the queue behind it until its deadline. A refusal raised by the HLC tick would be such an error,
  because `db::next_hlc` drops the SQLSTATE (#1154); the tick raises none today.
- **Backups.** Neither `registration_request` nor `note_draft_commit_failure` is in a backup or a local-state export:
  a backup carries signed events only (ADR-0026), and the local-state export reads only its own named tables, neither
  of these. An operator's own `pg_dump` copies them, typed names included. While the service runs, its hourly purge
  deletes a request a day after it settled or expired, or a day after its deadline.
- **The window and the service must be built in lockstep.** `name_tokens`, `rank_candidates` and the prompt's bound
  live in `cairn-patient-search`, but the window and the service are separate binaries. If one build changes any of
  them, the service's bounded prompt can differ from the clerk's screen: it may then refuse a registration the clerk
  made correctly, or miss a new match. The schema stamp does not catch it: it names the schema's content and the
  crate's version, not that code.
- **The local window still connects only to a database on its own machine.** That is ADR-0084 decision 5, which
  slice B amends.

Pins: `crates/cairn-node/tests/{terminal_service_doors,terminal_service_registration,terminal_service_commit,
terminal_service_run,register_in_tx,provision_terminal_service_role,node_key_public_writes}.rs`;
`terminal_service/{registration,run,commit,startup}.rs` and `patient/register_request.rs` unit tests;
`cairn-gui-live/tests/registration_queue.rs` and `funnel.rs`'s tests; `cairn-gui-tauri`'s `node_key_guard_tests.rs`
and `notes_commit_tests.rs`. RUNBOOK §38 carries the human measurements.
