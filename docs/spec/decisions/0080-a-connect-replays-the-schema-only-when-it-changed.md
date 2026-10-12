# ADR-0080 — A connect replays the schema only when it changed

- **Status:** Accepted
- **Date:** 2026-10-12
- **Spec version at acceptance:** 0.82
- **Issues:** [#726](https://github.com/cairn-ehr/cairn-ehr/issues/726) — a maintainer decision of 2026-10-12
- **Relates to:** [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md) (the heal this leaves gated on
  the generation, and whose context assumes a replay on every connect) ·
  [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (the code plane: the schema is a local
  node property) · [ADR-0001](0001-fat-postgres-thin-daemon.md) (the floor lives in the database, so the loader is
  how it arrives)

## Context

`cairn-node`'s loader (`db::connect_and_load_schema`) replayed every embedded `db/*.sql` on **every** connect. That
was the migration model: each file is idempotent, so replaying it is harmless, and an in-place fix to a function
body reaches every database on its next connect.

Idempotent is not lock-free. Measured on PostgreSQL 18 (2026-10-12): replaying the files a second time, against a
database they had already loaded, took **97 strong relation locks**:

- ACCESS EXCLUSIVE on 21 tables, including **`event_log`**, `patient_chart`, `patient_link`, `patient_demographic`
  and `patient_identifier`;
- ACCESS EXCLUSIVE on 34 views;
- SHARE on 27 tables and SHARE ROW EXCLUSIVE on 13.

They come from `CREATE OR REPLACE VIEW`, `DROP TRIGGER IF EXISTS` + `CREATE TRIGGER`,
`ALTER TABLE … ADD COLUMN IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS` and `CREATE OR REPLACE TRIGGER`. PostgreSQL
takes each lock **before** it discovers there is nothing to do.

Each such lock waits behind any open reader or writer of the relation, and PostgreSQL then queues every later
statement on that relation behind the waiting lock. So any node process connecting (the window at launch, a CLI
command, `cairn-sync` start) while a long reader was open (the matcher worker's multi-second blocking query, #725; a
reprojection) stalled clinical writes for the rest of that read. A clinician's write could wait on a tool nobody at
the bedside started.

The matcher was the trigger, not the fault. It reads committed data and takes only ACCESS SHARE, which conflicts with
nothing but those DDL locks. The maintainer's requirement is that the matcher works with what has already been
committed and never locks anything a clinical write needs. That requirement constrains the loader, not the matcher.

The issue proposed three options:

1. **A catalogue guard on each statement** (`IF NOT EXISTS` checked against `pg_attribute` / `pg_trigger` first).
   This cannot cover `CREATE OR REPLACE VIEW` or a changed trigger: those replays exist precisely so a changed
   definition converges. Telling "unchanged" from "changed" would mean comparing `pg_get_viewdef` with the source
   text, which is not a reliable comparison.
2. **Gate the replay on a change.** Taken, with a content hash rather than the generation number (decision 2).
3. **A `lock_timeout` on the loader.** Mitigation only: a connect fails loudly instead of stalling writes.

## Decision

### 1. A connect replays the migration files only when the schema it carries differs from the one recorded

After a successful replay the loader stamps `node_schema` (db/038) with three facts: the generation, **who** replayed
(`loader_build`), and **what** it replayed (`schema_hash`, db/058). A later connect skips the whole file replay, and
with it every DDL lock, when all three match this binary
(`db::replay_can_be_skipped`, pure, unit-tested). A skipped replay writes nothing: it does not re-stamp.

What still runs on every connect:

- the #188 downgrade guard;
- the load lock;
- re-adjudication of deferred events (ADR-0056 decision 4). It is driven by data that can change with no code change,
  and it is one indexed probe on a healthy node.

The heal stays gated on a generation change exactly as ADR-0057 decision 4 says. A generation change always replays
too, since the generation is one of the three facts.

### 2. A content hash, not the generation number

`SCHEMA_GENERATION` changes only when a new file lands. An in-place edit of an existing function body (a floor fix to
`db/005`, say) leaves it unchanged, so a generation-gated replay would never deliver that fix to an existing database.
That would silently undo the property that made every-connect replay attractive.

`schema_content_hash` (cairn-event) is a sha2-256 over each migration's name and text in replay order,
length-prefixed so no two lists can collide by moving a boundary. Any byte of any file changes it.

### 3. The loader's identity is part of the stamp

`cairn-sync init` replays only its subset (#198) and stamps its own `loader_build`. Its ON CONFLICT update leaves the
hash column as it was. If the hash alone were checked, that stale hash would vouch for cairn-node files cairn-sync
never replayed.

Requiring `loader_build` to match closes this: after cairn-sync stamps, cairn-node's next connect replays in full.
Every binary that links cairn-node's loader (the node, the reference window) stamps the same name and replays the
same list, so they share one stamp.

### 4. Every doubt resolves toward replaying

No row, a NULL hash (stamped before db/058, or cleared), another loader, another generation, or another hash: each one
replays. A needless replay costs locks for a moment; a wrongly skipped one would leave a stale safety floor in place.

`db::force_schema_replay` clears the hash, so the next connect replays. It is both the test lever and an operator's
lever.

### 5. The tests keep the old isolation

DB-gated tests share databases, and some deliberately sabotage a schema object to simulate an older database. They
relied on the next test's connect to repair it. `db::test_serial_guard` therefore clears the stamp, so every test
still begins from a full replay. Within a test, a second connect is gated like production, and a test that wants a
replay says so.

`tests/schema_replay_gate.rs` pins the guarantee mechanically. Another session holds ROW EXCLUSIVE (what any clinical
writer holds) on every table and view, and a second connect must complete under a 2-second `lock_timeout`. With the
gate disabled the same test fails on `001_envelope` with SQLSTATE 55P03.

## Consequences

- **A steady-state connect takes no DDL lock.** It cannot stall a clinical write behind a long reader, however long
  the read.
- **An upgrade still does.** A real schema change still replays, takes those locks once, and can wait behind a long
  reader once. That is inherent to applying DDL in PostgreSQL. A `lock_timeout` with retry on the upgrade path
  (option 3) would turn that one wait into a loud, retried failure; it is not built here.
- **Drift is no longer repaired on connect.** Every-connect replay used to repair a hand edit to the schema (a dropped
  index, an edited registry row) on the next connect. It no longer does: a database the stamp vouches for is trusted
  to be what the files say. Hand-editing a production node's schema was never supported. `force_schema_replay` is the
  remedy when it happens.
- **Hand-loaded rigs.** A rig loaded via `psql` after a stamp keeps the stamp's hash and is not replayed by the next
  connect. The SQL mirrors (`scripts/run-db-sql-tests.sh`) use their own throwaway database and are unaffected.
- **ADR-0057's context sentence** ("replays every migration each connect") describes the loader as it was. Its
  decision 4 (heal on a generation change, never on every connect) is unchanged and is what this ADR keeps.

## Rejected

- **Per-statement catalogue guards** (option 1): cannot cover views or changed triggers, which are most of the locks.
- **Gating on `SCHEMA_GENERATION`:** silently strands in-place fixes (decision 2).
- **A hash without the loader's identity:** cairn-sync's subset stamp would vouch for files it never replayed
  (decision 3).
- **Making the matcher take no lock:** a PostgreSQL read always takes ACCESS SHARE, and that lock was never the
  problem. It conflicts only with the DDL this ADR removes from the steady state.
