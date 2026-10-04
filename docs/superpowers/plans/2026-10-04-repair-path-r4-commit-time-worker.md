# Repair path R4 — the commit-time duplicate check (#679, ADR-0076 decision 7) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Whenever a chart's identity evidence changes (a registration here, an assertion, anything
arriving by sync, restore or rebuild), the node's own advisory matcher checks THAT chart against the
population within seconds and writes `match_proposal` rows. It never links. The node can say,
honestly, whether the check is current, catching up, stalled, or has never run, and whether one chart
has been checked since it last changed.

**Architecture:**
- `db/056` adds an append-only notice log `match_pending`. A `SECURITY DEFINER` row trigger on the
  matcher's six input projections INSERTs a notice and NOTIFYs; it cannot conflict, wait or raise.
  It also adds a one-row `match_worker_state` and two read functions:
  `cairn_duplicate_check_status()` and `cairn_chart_check_pending(uuid)`.
- The Python matcher gains:
  - one-chart blocking (`targeted.py`, the sweep's own SQL filtered to groups containing the chart);
  - a skip rule (`judged.py`);
  - an `assess`/`persist` split of `propose()`;
  - queue helpers (`queue_db.py`);
  - a pure planner (`worker_plan.py`);
  - the worker (`worker.py`): per chart, newest first, one short write transaction per chart; one
    opted-in sweep for a large backlog;
  - a `cairn-matcher watch` CLI.
- `cairn-node` gains `duplicate_check.rs` (a pure classifier and wording, plus two thin reads) and a
  `duplicate-check` subcommand.

**Tech Stack:** PostgreSQL ≥ 18 + `cairn_pgx` (PL/pgSQL); Python ≥ 3.11 with psycopg 3.3 (the
matcher's existing optional `pipeline` extra), stdlib only otherwise; Rust (tokio-postgres, anyhow,
clap).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`, section
**"R4 — the commit-time worker (#679)"** and its sub-section **"R4 — designed 2026-10-04"**. The
sub-section wins wherever the two differ. ADR-0076 decision 7. The maintainer's three decisions:
- the node read and the CLI ship now, the window's lines with R5;
- a standalone, operator-run worker;
- an append-only notice log.

## Global Constraints

- **AGPL-3.0; no new dependency** in either language (psycopg is already the `pipeline` extra).
- **TDD.** Every behaviour starts with a test that fails for the right reason. The `assess`/`persist`
  split (Task 2) is a refactor guarded by the existing suites staying green, plus one new test.
- **The worker NEVER applies a link.** Nothing in it writes `patient_link`, `event_log` or any
  identity event; its only writes are `match_proposal` (via `persist`), `match_pending` and
  `match_worker_state`. It needs no actor and no key, and connects as `cairn_agent`.
- **The hook can never fail or delay a clinical write.** It is structural: no `RAISE`, no `EXCEPTION`
  block, a null guard, and a plain INSERT of a fresh `bigserial` key. Never "fix" a hook problem by
  wrapping it in `EXCEPTION WHEN OTHERS`: a swallowed failure is a silently skipped check.
- **`patient_chart`'s hook is INSERT-only.** db/002 updates that row on every clinical event
  (`last_activity`, `note_count`); an UPDATE hook would queue a check on every medication write.
- **A notice is deleted only up to the id the worker read** (`id <= upto`). Never delete by
  `patient_id` alone: that loses a change arriving mid-check.
- **"Behind" is measured by the NEWEST waiting notice**, never the oldest (a restore backlog is not
  an alarm). The drain is newest-first.
- **No node has a false "checked".** `cairn_chart_check_pending` is TRUE for every chart until the
  worker has run once (no `match_worker_state` row): charts created before db/056 have no notices.
- **`SCHEMA_GENERATION` 55 → 56.** db/056 joins `cairn-node`'s loader list. cairn-sync's list lags
  legitimately (#284) and does not get it.
- **Migration replay:** every db/*.sql re-runs on every connect. Use `CREATE TABLE IF NOT EXISTS`,
  `CREATE INDEX IF NOT EXISTS`, `CREATE OR REPLACE FUNCTION`, `CREATE OR REPLACE TRIGGER`. Seed no
  rows. SQL is `include_str!`, so a db/*.sql edit needs a REBUILD before a Rust test sees it.
- **Every new definer has `SET search_path = public, pg_temp`** (`search_path_pg_temp.rs`).
- **Files under 500 lines.** `matcher/src/cairn_matcher/pipeline/db.py` (541) gets no new code; new
  modules hold it. `cairn-node/src/main.rs` grows by the subcommand only (≤ 20 lines).
- **House rule 6:** no literal key material; no binding named `salt`/`nonce`/`iv`.
- **Commit messages** say `Refs #679`, never a closing keyword. Run
  `python3 scripts/check_closing_keywords.py <msgfile>` before every commit.
- **Subagents run foreground tests only.** DB-gated suites are run by the controller with
  `--nocapture` (Rust) or `-rs` (pytest), checking no `skipped:` line or `SKIPPED` appears.
- **DB env** (`scripts/pg-target.sh` prints the cluster; use its port):
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  Rust: `CARGO_TARGET_DIR=/tmp/cairn-r4-target` when an IDE is open (trap 18). Matcher:
  `cd matcher && uv run --extra pipeline pytest …` (uv, never pip).

## Review Focus

1. **A chart that existed before db/056, on a node whose worker has never run.** It has no notice,
   yet it has never been checked. `cairn_chart_check_pending` must say TRUE, and the first worker start
   must queue it (reason `config`). Pinned in Task 1 (DB) and Task 5 (DB).
2. **A name corrected while its chart is being checked.** The newer notice survives the check's
   delete and the chart is checked again. Pinned in Task 4 (DB, injected mid-check).
3. **A medication write on a chart.** It queues NO notice (db/002's `patient_chart` UPDATE path).
   Pinned in Task 1 (DB, a second event on an existing chart).
4. **A chart whose check raises every time (a poison chart).** The worker does not spin on it: it is
   held for the retry interval, the others drain, and its notices stay so it reads "not yet
   checked". Pinned in Task 4 (pure `RetryBook`) and Task 5 (DB, drain with a failing chart).
5. **A sweep pair that errors in bulk mode.** Its two charts keep their notices; everything else up
   to the watermark is cleared. Pinned in Task 5 (DB).
6. **A pair already linked, or with a standing human unlink.** It is never proposed by either mode.
   Pinned in Task 3 (DB, per chart) and Task 5 (DB, bulk).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `db/056_match_pending.sql` | **create** | notice log, worker state, the hook + six triggers, two read fns, grants |
| `crates/cairn-event/src/schema_generation.rs` | modify | `SCHEMA_GENERATION` = 56 |
| `crates/cairn-node/src/db.rs` | modify | loader list entry for db/056 |
| `crates/cairn-node/tests/match_pending.rs` | **create** | DB tests for db/056 + the hook source guard |
| `matcher/src/cairn_matcher/pipeline/runner.py` | modify | `Assessment`, `assess`, `persist`; `propose` = assess + persist + commit |
| `matcher/src/cairn_matcher/pipeline/targeted.py` | **create** | `candidate_pairs_for`, `pairs_with` (pure) |
| `matcher/src/cairn_matcher/pipeline/judged.py` | **create** | `judged_partners`, `judged_pairs`, `drop_judged` (pure) |
| `matcher/src/cairn_matcher/pipeline/queue_db.py` | **create** | the worker's queue/state SQL |
| `matcher/src/cairn_matcher/pipeline/worker_plan.py` | **create** | `Mode`, `choose_mode`, `RetryBook` (pure) |
| `matcher/src/cairn_matcher/pipeline/worker.py` | **create** | `Settings`, `check_chart`, `run_bulk`, `drain`, `watch` |
| `matcher/src/cairn_matcher/pipeline/sweep.py` | modify | opt-in `skip_pairs` |
| `matcher/src/cairn_matcher/cli.py` | **create** | `cairn-matcher watch` |
| `matcher/src/cairn_matcher/eval/measure_check.py` | **create** | the per-chart latency measurement |
| `matcher/pyproject.toml` | modify | `[project.scripts]` |
| `matcher/tests/conftest.py` | modify | truncate the new tables + `patient_link`, `person_member` |
| `matcher/tests/test_*.py` (new files per task) | **create** | as named in each task |
| `crates/cairn-node/src/duplicate_check.rs` | **create** | `QueueSnapshot`, `CheckState`, `classify`, `status_line`, `chart_line`, reads |
| `crates/cairn-node/src/lib.rs` | modify | `pub mod duplicate_check;` |
| `crates/cairn-node/src/main.rs` | modify | `Cmd::DuplicateCheck` |
| `crates/cairn-node/tests/duplicate_check.rs` | **create** | DB tests for the reads + the CLI line |
| `docs/developers/running-the-duplicate-check.md` | **create** | operator runbook + launchd/systemd units |
| `matcher/README.md`, design page, HANDOVER, ROADMAP, `mkdocs.yml` | modify | Task 7 |

---

### Task 1: db/056 — the notice log, the hook, the worker state, the read functions

**Files:**
- Create: `db/056_match_pending.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs:45` (55 → 56) and its doc line naming the newest file
- Modify: `crates/cairn-node/src/db.rs` (append the db/056 entry after `055_link_precedence_refold`)
- Test: `crates/cairn-node/tests/match_pending.rs`

**Interfaces:**
- Produces (SQL, used by Tasks 3–6):
  - table `match_pending(id bigserial PK, patient_id uuid NOT NULL, reason text CHECK IN ('change','config'), queued_at timestamptz)`;
  - table `match_worker_state(singleton bool PK CHECK (singleton), matcher_version text NOT NULL, last_drained_at timestamptz)`;
  - `cairn_duplicate_check_status() → (charts_waiting bigint, newest_age_s bigint, config_recheck boolean, worker_seen boolean, last_drained_hhmm text)`;
  - `cairn_chart_check_pending(uuid) → boolean`;
  - NOTIFY channel `cairn_match_pending`.

- [ ] **Step 1: Write the failing tests** in `crates/cairn-node/tests/match_pending.rs`:

```rust
//! Repair path R4 (#679, ADR-0076 decision 7): db/056's notice log. A change to any of the
//! matcher's six input projections leaves an append-only NOTICE that the node's matcher worker
//! drains; the hook that writes it can never fail or delay the clinical write (it is a plain
//! insert of a fresh key — nothing to conflict with, nothing to wait on, no RAISE).
mod common;
use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_node::db;
use common::{chart_named, cs, setup, submit_signed, EventSpec};
use tokio_postgres::Client;
use uuid::Uuid;

/// The guard is the advisory-lock-holding connection `test_serial_guard` returns; keep it alive.
async fn fresh_db() -> Option<(Client, Client)> {
    let base = cs()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state")
        .await
        .unwrap();
    Some((c, guard))
}

async fn notices(c: &Client, p: Uuid) -> i64 {
    c.query_one("SELECT count(*) FROM match_pending WHERE patient_id = $1", &[&p])
        .await
        .unwrap()
        .get(0)
}

async fn assert_name(c: &Client, sk: &cairn_event::SigningKey, kid: &str, p: Uuid, name: &str, wall: i64) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: name_assertion_body(name, Some("legal"), "patient-stated"),
            plaintext_twin: Some(render_name_twin(name, Some("legal"), "patient-stated")),
            wall,
        },
    )
    .await
    .expect("name assertion accepted");
}

#[tokio::test]
async fn a_registration_and_its_name_leave_notices() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (sk, kid) = setup(&c, &["patient_name", "match_pending"]).await;
    let p = chart_named(&c, &sk, &kid, 10, "Mary Smith").await;
    assert!(notices(&c, p).await >= 2, "patient_chart insert + patient_name insert");
}

#[tokio::test]
async fn a_losing_reassertion_and_a_later_event_on_the_chart_queue_nothing() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (sk, kid) = setup(&c, &["patient_name", "match_pending"]).await;
    let p = chart_named(&c, &sk, &kid, 10, "Mary Smith").await;
    let before = notices(&c, p).await;
    // The same name at an OLDER wall: db/012's conditional upsert changes no row, so no
    // trigger fires. The event also touches patient_chart through db/002's ON CONFLICT DO
    // UPDATE path (an UPDATE, never an INSERT) — the INSERT-only hook must stay silent.
    assert_name(&c, &sk, &kid, p, "Mary Smith", 5).await;
    assert_eq!(notices(&c, p).await, before);
}

#[tokio::test]
async fn every_input_projection_carries_the_hook_with_the_right_events_and_column() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // (table, id column, fires on UPDATE too?) — read from the catalogue, the thing that runs.
    let expected = [
        ("chart_identity_state", "subject", true),
        ("name_repudiation", "subject", true),
        ("patient_chart", "patient_id", false),
        ("patient_demographic", "patient_id", true),
        ("patient_identifier", "patient_id", true),
        ("patient_name", "patient_id", true),
    ];
    let rows = c
        .query(
            "SELECT c.relname::text, encode(t.tgargs, 'escape'), (t.tgtype & 16) <> 0 \
             FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
             JOIN pg_proc p ON p.oid = t.tgfoid \
             WHERE p.proname = 'cairn_match_enqueue' AND NOT t.tgisinternal ORDER BY 1",
            &[],
        )
        .await
        .unwrap();
    let got: Vec<(String, String, bool)> = rows
        .iter()
        // tgargs is NUL-terminated per argument; escape-encoding renders NUL as "\000".
        .map(|r| (r.get(0), r.get::<_, String>(1).replace("\\000", ""), r.get(2)))
        .collect();
    let want: Vec<(String, String, bool)> = expected
        .iter()
        .map(|(t, col, upd)| (t.to_string(), col.to_string(), *upd))
        .collect();
    assert_eq!(got, want);
}

#[tokio::test]
async fn a_null_id_queues_nothing_and_raises_nothing() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // Every real input column is NOT NULL, so the guard is defence in depth; prove it on a
    // scratch table carrying the same hook.
    c.batch_execute(
        "CREATE TEMP TABLE r4_null_probe (pid uuid); \
         CREATE TRIGGER r4_probe AFTER INSERT ON r4_null_probe \
           FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('pid'); \
         INSERT INTO r4_null_probe VALUES (NULL);",
    )
    .await
    .expect("a NULL id must not raise");
    let n: i64 = c.query_one("SELECT count(*) FROM match_pending", &[]).await.unwrap().get(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_clinical_write_never_waits_on_a_worker_deleting_that_patients_notices() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (sk, kid) = setup(&c, &["patient_name", "match_pending"]).await;
    let p = chart_named(&c, &sk, &kid, 10, "Mary Smith").await;
    // The "worker": an open transaction holding a DELETE on p's notices.
    let worker = db::connect_and_load_schema(&cs().unwrap()).await.unwrap();
    worker.batch_execute("BEGIN").await.unwrap();
    worker
        .execute("DELETE FROM match_pending WHERE patient_id = $1", &[&p])
        .await
        .unwrap();
    // A wait would hit lock_timeout and fail loudly — never a sleep.
    c.batch_execute("SET lock_timeout = '2s'").await.unwrap();
    assert_name(&c, &sk, &kid, p, "Mary Smyth", 20).await;
    c.batch_execute("RESET lock_timeout").await.unwrap();
    worker.batch_execute("COMMIT").await.unwrap();
    assert_eq!(notices(&c, p).await, 1, "the notice written mid-delete survives it");
}

#[test]
fn the_hook_has_no_raising_path() {
    let sql = include_str!("../../../db/056_match_pending.sql");
    let start = sql
        .find("CREATE OR REPLACE FUNCTION cairn_match_enqueue")
        .expect("the hook function exists");
    let body = &sql[start..start + sql[start..].find("$$;").expect("body ends")];
    for banned in ["RAISE", "EXCEPTION", "ON CONFLICT"] {
        assert!(!body.contains(banned), "the hook must not contain {banned}");
    }
}

#[tokio::test]
async fn a_chart_is_pending_until_the_worker_has_run_and_while_it_has_notices() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let p = Uuid::now_v7();
    let pending = |c: &Client, p: Uuid| async move {
        c.query_one("SELECT cairn_chart_check_pending($1)", &[&p])
            .await
            .unwrap()
            .get::<_, bool>(0)
    };
    assert!(pending(&c, p).await, "no worker has ever run: nothing is checked");
    c.execute(
        "INSERT INTO match_worker_state (matcher_version, last_drained_at) VALUES ('v', now())",
        &[],
    )
    .await
    .unwrap();
    assert!(!pending(&c, p).await, "worker ran, no notice");
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES ($1, 'change')",
        &[&p],
    )
    .await
    .unwrap();
    assert!(pending(&c, p).await, "a notice is waiting");
}

#[tokio::test]
async fn the_status_reports_waiting_charts_and_the_newest_age() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let r = c.query_one("SELECT * FROM cairn_duplicate_check_status()", &[]).await.unwrap();
    assert_eq!(r.get::<_, i64>("charts_waiting"), 0);
    assert_eq!(r.get::<_, Option<i64>>("newest_age_s"), None);
    assert!(!r.get::<_, bool>("worker_seen"));
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    c.batch_execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')").await.unwrap();
    c.execute(
        "INSERT INTO match_pending (patient_id, reason, queued_at) VALUES \
         ($1, 'change', now() - interval '10 minutes'), ($1, 'change', now() - interval '9 minutes'), \
         ($2, 'config', now() - interval '1 minute')",
        &[&a, &b],
    )
    .await
    .unwrap();
    let r = c.query_one("SELECT * FROM cairn_duplicate_check_status()", &[]).await.unwrap();
    assert_eq!(r.get::<_, i64>("charts_waiting"), 2);
    let age = r.get::<_, Option<i64>>("newest_age_s").unwrap();
    assert!((59..=70).contains(&age), "the NEWEST notice is a minute old, got {age}");
    assert!(r.get::<_, bool>("config_recheck"));
    assert!(r.get::<_, bool>("worker_seen"));
    assert_eq!(r.get::<_, Option<String>>("last_drained_hhmm"), None);
}

#[tokio::test]
async fn the_worker_role_can_do_exactly_its_job() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let p = Uuid::now_v7();
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    c.execute("INSERT INTO match_pending (patient_id, reason) VALUES ($1, 'config')", &[&p])
        .await
        .expect("cairn_agent queues a config re-check");
    c.execute("DELETE FROM match_pending WHERE patient_id = $1", &[&p])
        .await
        .expect("cairn_agent clears notices");
    c.batch_execute(
        "INSERT INTO match_worker_state (matcher_version) VALUES ('v') \
         ON CONFLICT (singleton) DO UPDATE SET last_drained_at = clock_timestamp()",
    )
    .await
    .expect("cairn_agent stamps the worker state");
    c.batch_execute("RESET ROLE; SET ROLE cairn_node").await.unwrap();
    c.query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .expect("the node role reads the status");
    c.batch_execute("RESET ROLE").await.unwrap();
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p cairn-node --test match_pending -- --nocapture
```
Expected: compile error or FAIL. `the_hook_has_no_raising_path` panics: the file does not exist
(`include_str!` fails to compile).

- [ ] **Step 3: Write `db/056_match_pending.sql`**

```sql
-- db/056_match_pending.sql
-- Repair path R4 (#679, ADR-0076 decision 7): the commit-time duplicate check's queue.
--
-- WHAT: whenever one of the advisory §5.2 matcher's INPUTS changes for a chart (a name, a DOB or
-- sex, an identifier, the §5.4 identity state, a repudiated name, or a new chart), a NOTICE row
-- is appended here. The node's matcher worker (`cairn-matcher watch`, matcher/) drains the
-- notices: it checks each chart against the population and writes match_proposal rows (db/017).
-- It NEVER links — a hit is a proposal a human resolves.
--
-- WHY APPEND-ONLY NOTICES, NOT ONE ROW PER PATIENT (design page, "R4 — designed 2026-10-04"):
--   * the hook below is a plain INSERT of a fresh bigserial key. It cannot conflict with
--     anything, so it never waits on the worker's open transaction and never raises a
--     serialization error, whatever isolation level a future writer picks. A keyed upsert
--     (ON CONFLICT) would make a clinical write wait behind the worker.
--   * the worker deletes only the notices it READ (id <= the highest id it saw for that
--     chart), so a change landing while the chart is being checked survives and is checked
--     again. Deleting "the patient's row" would lose it.
--
-- WHY IT CAN NEVER FAIL A CLINICAL WRITE: structurally, like db/029's collision recorder — no
-- RAISE, no EXCEPTION block, a null guard, an insert that cannot conflict. It is deliberately
-- NOT wrapped in EXCEPTION WHEN OTHERS: a swallowed failure is a silently skipped check. What
-- can still raise (a full disk, a dropped table) would fail the clinical write anyway.
-- Guarded by crates/cairn-node/tests/match_pending.rs::the_hook_has_no_raising_path.
--
-- WHICH WRITES QUEUE A CHECK: the projection upserts are conditional (DO UPDATE ... WHERE new >
-- old), and a row trigger fires only on a row that actually changes. So normal use queues one
-- notice per real change, an upgrade heal queues only charts whose winner changed, and a
-- `reproject --rebuild`, a restore or a new node's first pull queues every chart (correct: the
-- inputs were rewritten; the worker switches to one sweep for a large backlog).

CREATE TABLE IF NOT EXISTS match_pending (
    id         BIGSERIAL   PRIMARY KEY,
    patient_id UUID        NOT NULL,
    -- 'change': an input changed. 'config': the worker's matcher_version changed (or it ran for
    -- the first time), so every chart is re-checked. Labels the status; does not order the drain.
    reason     TEXT        NOT NULL CHECK (reason IN ('change', 'config')),
    queued_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- The worker's two reads: "the newest notices, grouped by chart" and "this chart's notices up to
-- id N".
CREATE INDEX IF NOT EXISTS match_pending_patient_idx ON match_pending (patient_id, id);

-- One row, written only by the worker: the matcher_version it last ran (a change re-queues every
-- chart) and when it last finished a drain (the status line's "last ran HH:MM"). Its ABSENCE
-- means no worker has ever run on this node — see cairn_chart_check_pending.
CREATE TABLE IF NOT EXISTS match_worker_state (
    singleton       BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    matcher_version TEXT        NOT NULL,
    last_drained_at TIMESTAMPTZ
);

-- The hook. TG_ARGV[0] names the chart-id column (patient_id, or subject on the two identity
-- tables). SECURITY DEFINER so every writer — the submit door, the sync door, a restore, a
-- rebuild, a test seeding rows directly — can append a notice without its own grant.
CREATE OR REPLACE FUNCTION cairn_match_enqueue()
RETURNS trigger
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
AS $$
BEGIN
    INSERT INTO match_pending (patient_id, reason)
    SELECT (to_jsonb(NEW) ->> TG_ARGV[0])::uuid, 'change'
    WHERE to_jsonb(NEW) ->> TG_ARGV[0] IS NOT NULL;
    -- One wake-up per transaction: NOTIFY collapses identical payloads within a transaction, so
    -- a rebuild that writes 500k rows wakes the worker once, at commit.
    PERFORM pg_notify('cairn_match_pending', '');
    RETURN NULL;
END $$;
REVOKE EXECUTE ON FUNCTION cairn_match_enqueue() FROM PUBLIC;

-- The matcher's inputs. patient_chart is INSERT-ONLY: db/002 updates that row on EVERY clinical
-- event (last_activity, note_count), so an UPDATE hook there would queue a duplicate check on
-- every medication write. A new chart is the only patient_chart event the matcher cares about.
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT ON patient_chart
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_name
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_demographic
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON patient_identifier
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('patient_id');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON chart_identity_state
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('subject');
CREATE OR REPLACE TRIGGER match_enqueue AFTER INSERT OR UPDATE ON name_repudiation
    FOR EACH ROW EXECUTE FUNCTION cairn_match_enqueue('subject');

-- Has THIS chart been checked since its identity evidence last changed? FALSE only when the
-- worker has run at least once AND no notice for the chart is waiting. Before the first worker
-- run every chart is pending: charts created before db/056 have no notices, and saying
-- "checked" about them would be a false claim (principle 4). R5's banner reads this.
CREATE OR REPLACE FUNCTION cairn_chart_check_pending(p_patient uuid)
RETURNS boolean
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT EXISTS (SELECT 1 FROM match_pending WHERE patient_id = p_patient)
        OR NOT EXISTS (SELECT 1 FROM match_worker_state)
$$;

-- The node-wide status (cairn-node `duplicate-check`; R5's front door later). newest_age_s is the
-- age of the NEWEST waiting notice: with a newest-first drain, only a stopped or stuck worker
-- lets the newest notice grow old, so a restore backlog is never mistaken for a stall.
-- VOLATILE (the default): it reads clock_timestamp().
CREATE OR REPLACE FUNCTION cairn_duplicate_check_status(
    OUT charts_waiting    bigint,
    OUT newest_age_s      bigint,
    OUT config_recheck    boolean,
    OUT worker_seen       boolean,
    OUT last_drained_hhmm text)
LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT
        (SELECT count(DISTINCT patient_id) FROM match_pending),
        (SELECT floor(extract(epoch FROM clock_timestamp() - max(queued_at)))::bigint
           FROM match_pending),
        EXISTS (SELECT 1 FROM match_pending WHERE reason = 'config'),
        EXISTS (SELECT 1 FROM match_worker_state),
        (SELECT to_char(last_drained_at, 'HH24:MI') FROM match_worker_state)
$$;

-- The worker (cairn_agent) reads and clears notices, queues a 'config' re-check, and keeps its
-- state row. The node's runtime role only reads.
GRANT SELECT, INSERT, DELETE ON match_pending TO cairn_agent;
GRANT USAGE, SELECT ON SEQUENCE match_pending_id_seq TO cairn_agent;
GRANT SELECT, INSERT, UPDATE ON match_worker_state TO cairn_agent;
GRANT SELECT ON match_pending, match_worker_state TO cairn_node;
GRANT EXECUTE ON FUNCTION cairn_chart_check_pending(uuid) TO cairn_agent, cairn_node;
GRANT EXECUTE ON FUNCTION cairn_duplicate_check_status() TO cairn_agent, cairn_node;
```

Then: `SCHEMA_GENERATION` → 56 (and the doc line that names `db/055_link_precedence_refold.sql →
55` names db/056 → 56); add the loader entry in `crates/cairn-node/src/db.rs` after db/055:

```rust
    // db/056 (repair path R4, #679, ADR-0076 decision 7): the commit-time duplicate check's
    // append-only notice log, its never-failing hook on the matcher's six inputs, the worker's
    // state row, and the two status reads. cairn-sync's list lags legitimately (#284).
    (
        "056_match_pending",
        include_str!("../../../db/056_match_pending.sql"),
    ),
```

- [ ] **Step 4: Run to verify they pass**, then the guards that pin the loader and the generation:

```bash
cargo test -p cairn-node --test match_pending -- --nocapture
cargo test -p cairn-event schema_generation
cargo test -p cairn-node --test search_path_pg_temp --test floor_execute_grants --test schema_version_guard -- --nocapture
cargo test -p cairn-node --lib db::
```
Expected: all PASS, with no `skipped:` line. If a pinned-count guard elsewhere fails because it
counts definers, triggers or loader entries, update its count. Read its header first: a guard
that says "never add to the list" means the defect is in db/056, not in the guard.

- [ ] **Step 5: Commit**

```bash
git add db/056_match_pending.sql crates/cairn-event/src/schema_generation.rs crates/cairn-node/src/db.rs crates/cairn-node/tests/match_pending.rs
git commit -F <msgfile>   # "feat(R4): db/056 — the duplicate check's notice log and its never-failing hook (Refs #679)"
```

---

### Task 2: `assess` / `persist` — split `propose()` so a caller owns the transaction

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/runner.py`
- Test: `matcher/tests/test_assess_persist.py`

**Interfaces:**
- Produces:
  ```python
  @dataclass(frozen=True)
  class Assessment:
      low: str                 # canonical lowercase uuid text (canonical_pair)
      high: str
      band: Band | None        # None = below the review floor
      payload: ProposalPayload | None   # None iff band is None

  def assess(conn, a, b, *, thresholds=DEFAULT_THRESHOLDS, weights=DEFAULT_WEIGHTS,
             config=DEFAULT_CONFIG, aliases=None, trust=None) -> Assessment   # reads only
  def persist(conn, assessment: Assessment) -> bool   # writes, NEVER commits; True if a row changed
  def propose(...) -> Band | None                     # unchanged signature and behaviour
  ```

- [ ] **Step 1: Write the failing test** (`matcher/tests/test_assess_persist.py`):

```python
"""R4 Task 2: assess() reads and decides; persist() writes without committing.

The worker needs a chart's proposals AND the delete of its notices in ONE transaction (a crash
re-checks the chart, never loses it), which propose() — committing per pair — cannot give.
propose() itself must behave exactly as before; its existing suites guard that.
"""

from cairn_matcher.pipeline.banding import Band
from cairn_matcher.pipeline.runner import assess, persist
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"


def _proposals(conn):
    with conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM match_proposal")
        return cur.fetchone()[0]


def test_assess_decides_without_writing(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:hospital-a", "12345", "12345")])
    a = assess(pg_conn, PB, PA)
    assert (a.low, a.high) == (PA, PB)           # canonical order whatever the call order
    assert a.band is Band.REVIEW and a.payload is not None
    assert _proposals(pg_conn) == 0


def test_persist_writes_but_never_commits(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:hospital-a", "12345", "12345")])
    assert persist(pg_conn, assess(pg_conn, PA, PB)) is True
    assert _proposals(pg_conn) == 1
    pg_conn.rollback()                            # the caller owns the transaction
    assert _proposals(pg_conn) == 0


def test_persist_of_a_below_floor_assessment_retracts_a_pending_row(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, sex=("female", 0))
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (PA, PB))
    pg_conn.commit()
    a = assess(pg_conn, PA, PB)
    assert a.band is None and a.payload is None
    assert persist(pg_conn, a) is True
    pg_conn.commit()
    with pg_conn.cursor() as cur:
        cur.execute("SELECT status FROM match_proposal")
        assert cur.fetchone()[0] == "retracted"
```

- [ ] **Step 2: Run to verify it fails**

```bash
cd matcher && uv run --extra pipeline pytest tests/test_assess_persist.py -rs
```
Expected: `ImportError: cannot import name 'assess'`.

- [ ] **Step 3: Implement.** In `runner.py`, move the body of `propose()` up to the persistence into
`assess()`, and the persistence into `persist()`:

```python
@dataclass(frozen=True)
class Assessment:
    """One pair's verdict, decided but not yet written.

    `band` None means the pair is below the review floor: persisting it retracts a still-pending
    proposal (#135) and otherwise writes nothing. `payload` is None exactly when `band` is.
    """

    low: str
    high: str
    band: Band | None
    payload: ProposalPayload | None


def assess(conn, a, b, *, thresholds=DEFAULT_THRESHOLDS, weights=DEFAULT_WEIGHTS,
           config=DEFAULT_CONFIG, aliases=None, trust=None) -> Assessment:
    """Score the pair, gate on the in-DB veto, band it — and write NOTHING.

    Everything propose() used to do before persisting, unchanged (see the comments carried over
    below). Reads only, so a caller can assess many pairs and then persist them all, plus its own
    bookkeeping, in one transaction it controls (the R4 worker).
    """
    ...  # the existing load / score / veto / alias / trust / band code, verbatim
    low, high = canonical_pair(a, b)
    if band_value is None:
        return Assessment(low, high, None, None)
    ...  # trust_evidence as before
    payload = build_payload(match_score, vetoes, band_value, weights, alias_evidence,
                            trust_evidence, thresholds=thresholds, config=config)
    return Assessment(low, high, band_value, payload)


def persist(conn, assessment: Assessment) -> bool:
    """Write an assessment; NEVER commit. Returns True when a row was written or retracted.

    A banded pair is upserted (a human's status is preserved — db.upsert_proposal). A below-floor
    pair retracts a still-pending proposal, if any (#135). The caller owns the commit.
    """
    from cairn_matcher.pipeline import db

    if assessment.band is None:
        return db.retract_pending_proposal(conn, assessment.low, assessment.high) > 0
    db.upsert_proposal(conn, assessment.low, assessment.high, assessment.payload)
    return True


def propose(conn, a, b, *, thresholds=DEFAULT_THRESHOLDS, weights=DEFAULT_WEIGHTS,
            config=DEFAULT_CONFIG, aliases=None, trust=None) -> Band | None:
    """(docstring unchanged)"""
    verdict = assess(conn, a, b, thresholds=thresholds, weights=weights, config=config,
                     aliases=aliases, trust=trust)
    # Commit boundary owned here (unchanged): a write is made durable; a pure read (nothing
    # retracted, nothing banded) still ends its transaction so a batch caller does not pin the
    # xmin horizon across a whole run.
    if persist(conn, verdict):
        conn.commit()
    else:
        conn.rollback()
    return verdict.band
```
Keep every existing comment, moving each with its code. Update `__all__` to
`["Assessment", "assess", "canonical_pair", "persist", "propose"]`.

- [ ] **Step 4: Run the new test and every existing suite that drives `propose()`**

```bash
cd matcher && uv run --extra pipeline pytest -rs
uv run ruff check .
```
Expected: all PASS, ruff clean, no `SKIPPED` lines for DB tests when `CAIRN_TEST_PG` is set.

- [ ] **Step 5: Commit** (`refactor(R4): propose() splits into assess() and persist() so a caller owns the transaction (Refs #679)`).

---

### Task 3: one-chart blocking, the drift canary, and the skip rule

**Files:**
- Create: `matcher/src/cairn_matcher/pipeline/targeted.py`, `matcher/src/cairn_matcher/pipeline/judged.py`
- Modify: `matcher/tests/conftest.py` (`_PROJECTION_TABLES` gains `"match_pending"`,
  `"match_worker_state"`, `"patient_link"`, `"person_member"`)
- Test: `matcher/tests/test_targeted_blocking.py`, `matcher/tests/test_judged.py`

**Interfaces:**
- Consumes: `db._GROUPS_SQL`, `db._RANGE_GROUPS_SQL`, `db._PLACEHOLDER_USES_PARAM`,
  `adapter.VALUE_SENTINELS_PARAM`, `blocking.canonical_pair`, `blocking.require_registered`,
  `blocking.SYMMETRIC_PASSES`, `blocking.ANCHORED_PASSES`, `db.generate_candidate_pairs`.
- Produces:
  ```python
  # targeted.py
  DEFAULT_TARGETED_CAP: int = 1000          # revised from Task 7's measurement
  def pairs_with(me: str, members) -> set[tuple[str, str]]                       # pure
  def candidate_pairs_for(conn, patient, *, max_block_size=DEFAULT_TARGETED_CAP
                          ) -> tuple[list[tuple[str, str]], list[tuple[str, str, int]]]
  # judged.py
  def judged_partners(conn, patient) -> frozenset[str]
  def judged_pairs(conn) -> frozenset[tuple[str, str]]
  def drop_judged(pairs, patient, partners) -> list[tuple[str, str]]               # pure
  ```

- [ ] **Step 1: Write the failing tests.**

`matcher/tests/test_targeted_blocking.py`:

```python
"""R4 Task 3: one chart's candidate pairs equal the full sweep's pairs that include it.

The drift canary is the load-bearing test: targeted.py composes the SAME CTE constants as the
sweep and only filters its groups, so this proves the filter, for every chart of a generated
population, with no cap on either side.
"""

import uuid

from cairn_matcher.eval.blocking_eval import record_uuid, seed_dataset
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset
from cairn_matcher.pipeline.db import generate_candidate_pairs
from cairn_matcher.pipeline.targeted import candidate_pairs_for, pairs_with
from tests.conftest import seed_patient

UNCAPPED = 10**9


def test_pairs_with_pairs_me_with_each_other_member_canonically():
    me = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
    a = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
    c = "CCCCCCCC-CCCC-CCCC-CCCC-CCCCCCCCCCCC"
    assert pairs_with(me, [a, me, c]) == {
        (a, me), (me, c.lower()),
    }


def test_targeted_pairs_equal_the_sweeps_pairs_for_every_chart(pg_conn):
    # Range-heavy AND name-heavy, so all eight passes and both range shapes (the chart as anchor,
    # the chart as a member of someone else's window) are exercised.
    ds = load_dataset(generate_dataset(GenSpec(seed=7, n_entities=80, p_dob_estimate=0.4)))
    seed_dataset(pg_conn, ds)          # no commit: the rows live in this transaction
    sweep_pairs, sweep_skipped = generate_candidate_pairs(pg_conn, max_block_size=UNCAPPED)
    assert sweep_skipped == []
    checked = 0
    for rec in ds.all_records():
        me = record_uuid(rec.record_id)
        mine, skipped = candidate_pairs_for(pg_conn, me, max_block_size=UNCAPPED)
        assert skipped == []
        assert set(mine) == {p for p in sweep_pairs if me in p}, rec.record_id
        checked += 1
    assert checked >= 160
    pg_conn.rollback()


def test_an_oversized_block_is_reported_never_silently_dropped(pg_conn):
    ids = [str(uuid.uuid4()) for _ in range(5)]
    for p in ids:
        seed_patient(pg_conn, p, names=[("Commonname Person", 20)])
    pairs, skipped = candidate_pairs_for(pg_conn, ids[0], max_block_size=4)
    assert pairs == []
    assert ("name", "commonname", 5) in skipped
```

`matcher/tests/test_judged.py`:

```python
"""R4 Task 3: a pair already judged — one component, or ANY patient_link row — is never proposed."""

import hashlib
import uuid

from cairn_matcher.pipeline.blocking import canonical_pair
from cairn_matcher.pipeline.judged import drop_judged, judged_pairs, judged_partners

A, B, C, D = (str(uuid.UUID(int=i)) for i in (1, 2, 3, 4))


def _link(conn, x, y, state):
    low, high = canonical_pair(x, y)
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address) VALUES (%s,%s,%s,1,0,'seed','test:link',%s)",
            (low, high, state, b"\x12\x20" + hashlib.sha256(f"{low}{high}".encode()).digest()))
    conn.commit()


def _member(conn, patient, person):
    with conn.cursor() as cur:
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s)",
                    (patient, person))
    conn.commit()


def test_drop_judged_keeps_only_pairs_whose_other_side_is_unjudged():
    pairs = [canonical_pair(A, B), canonical_pair(A, C), canonical_pair(A, D)]
    assert drop_judged(pairs, A, frozenset({B, C})) == [canonical_pair(A, D)]


def test_partners_are_the_component_and_every_link_row_either_state(pg_conn):
    # A–B linked (one component A,B,C via B–C); A–D a standing human unlink.
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link")
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink")
    assert judged_partners(pg_conn, A) == frozenset({A, B, C, D})
    assert judged_partners(pg_conn, D) == frozenset({D, A})


def test_judged_pairs_cover_transitive_members_and_unlinks(pg_conn):
    for p in (A, B, C):
        _member(pg_conn, p, A)
    _link(pg_conn, A, B, "link")
    _link(pg_conn, B, C, "link")
    _link(pg_conn, A, D, "unlink")
    want = {canonical_pair(x, y) for x, y in [(A, B), (B, C), (A, C), (A, D)]}
    assert judged_pairs(pg_conn) == frozenset(want)
```

Note on `_link`: `patient_link` has NOT NULL `provenance` and `content_address`; check db/018 for
any other NOT NULL column without a default before running, and add it with a literal.

- [ ] **Step 2: Run to verify they fail**

```bash
cd matcher && uv run --extra pipeline pytest tests/test_targeted_blocking.py tests/test_judged.py -rs
```
Expected: `ModuleNotFoundError: cairn_matcher.pipeline.targeted` / `.judged`.

- [ ] **Step 3: Implement.**

`targeted.py`:

```python
# matcher/src/cairn_matcher/pipeline/targeted.py
"""One chart's candidate pairs (repair path R4): the sweep's blocking, kept to ONE chart.

The commit-time duplicate check (#679) asks "who might this ONE chart be a duplicate of?". It
answers with the sweep's own blocking SQL (db._GROUPS_SQL / db._RANGE_GROUPS_SQL, composed from
the same CTE constants) wrapped in a filter that keeps only the groups containing the chart. The
SQL is SHARED, not copied, so a new blocking pass reaches this module the moment it reaches the
sweep; tests/test_targeted_blocking.py's drift canary proves the filter over a generated
population (targeted pairs == the sweep's pairs that include the chart).

Pairs are the chart x each other member only — never member x member, which is the sweep's
business. So a block's pair count grows LINEARLY here, and the cap (DEFAULT_TARGETED_CAP) can sit
far above the sweep's 100. An oversized block is still reported, never silently dropped.

Cost: each call still evaluates the blocking CTEs over the whole population (one scan of the
names), as the sweep does once. That is fine for a fresh change; for a large backlog the worker
runs ONE sweep instead (worker.run_bulk). A materialised token table (#637) would make this
cheaper later.

Requires the optional `pipeline` extra (psycopg) at call time.
"""

import uuid

from cairn_matcher.pipeline.adapter import VALUE_SENTINELS_PARAM
from cairn_matcher.pipeline.blocking import (
    ANCHORED_PASSES,
    SYMMETRIC_PASSES,
    canonical_pair,
    require_registered,
)

# Shared with the sweep on purpose (see the module docstring). Private names, imported
# deliberately: they are the one definition of blocking, and duplicating them here would be the
# drift this module exists to avoid.
from cairn_matcher.pipeline.db import _GROUPS_SQL, _PLACEHOLDER_USES_PARAM, _RANGE_GROUPS_SQL

# Set from the Task 7 measurement (per-chart latency on a generated population); see the design
# page's R4 as-built note. A block above this is non-discriminating even when paired linearly.
DEFAULT_TARGETED_CAP = 1000

# The symmetric groups that contain the chart. The trailing %s is the chart id; the first two
# are _GROUPS_SQL's own binds (placeholder uses, value sentinels), in its order.
_TARGETED_GROUPS_SQL = (
    f"SELECT g.pass_name, g.key, g.members FROM ({_GROUPS_SQL}) g "
    "WHERE %s::uuid = ANY(g.members)"
)

# The anchored range groups the chart is in — as the anchor (its own estimated-age window) or
# as a member of another chart's window. The first %s is _RANGE_GROUPS_SQL's sentinel bind.
_TARGETED_RANGE_SQL = (
    f"SELECT g.pass_name, g.anchor, g.members FROM ({_RANGE_GROUPS_SQL}) g "
    "WHERE g.anchor = %s::uuid OR %s::uuid = ANY(g.members)"
)


def pairs_with(me: str, members) -> set[tuple[str, str]]:
    """Canonical pairs of `me` with every OTHER member (pure). Self-pairs are skipped."""
    out: set[tuple[str, str]] = set()
    for m in members:
        if str(uuid.UUID(str(m))) != me:
            out.add(canonical_pair(me, m))
    return out


def candidate_pairs_for(conn, patient, *, max_block_size=DEFAULT_TARGETED_CAP):
    """Every candidate pair involving `patient`, plus the blocks skipped for size.

    Returns (pairs, skipped_blocks) in generate_candidate_pairs' shapes: sorted canonical
    lowercase-uuid pairs, and (pass_name, key, size) for each block over the cap. Block size is
    measured exactly as the sweep measures it (a symmetric group's member count; an anchored
    window's members + its anchor), so "oversized" means the same thing in both.

    Read-only; opens a read transaction the caller must close.
    """
    me = str(uuid.UUID(str(patient)))
    pairs: set[tuple[str, str]] = set()
    skipped: list[tuple[str, str, int]] = []
    with conn.cursor() as cur:
        cur.execute(_TARGETED_GROUPS_SQL, (_PLACEHOLDER_USES_PARAM, VALUE_SENTINELS_PARAM, me))
        for pass_name, key, members in cur.fetchall():
            require_registered(pass_name, SYMMETRIC_PASSES)
            if len(members) > max_block_size:
                skipped.append((pass_name, key, len(members)))
            else:
                pairs.update(pairs_with(me, members))
        cur.execute(_TARGETED_RANGE_SQL, (VALUE_SENTINELS_PARAM, me, me))
        for pass_name, anchor, members in cur.fetchall():
            require_registered(pass_name, ANCHORED_PASSES)
            size = len(members) + 1
            if size > max_block_size:
                skipped.append((pass_name, str(anchor), size))
            elif str(anchor) == me:
                pairs.update(pairs_with(me, members))      # my window: me x each member
            else:
                pairs.add(canonical_pair(anchor, me))      # their window holds me: one pair
    return sorted(pairs), skipped
```

`judged.py`:

```python
# matcher/src/cairn_matcher/pipeline/judged.py
"""Pairs a human (or the identity algebra) has already judged — never proposed again.

ADR-0076's skip rule for the commit-time check: a pair already in ONE link component is the same
person already; a pair with ANY patient_link row has been judged — a link, or a "not the same
person" unlink (decision 4). Proposing either again would put a settled question back on the
worklist. A pending proposal that predates a judgement is filtered from the worklist by R5's view,
not here.

Requires the optional `pipeline` extra (psycopg) at call time, except drop_judged (pure).
"""

from cairn_matcher.pipeline.blocking import canonical_pair


def judged_partners(conn, patient) -> frozenset[str]:
    """Every chart already judged against `patient` (its component, and any link-row partner).

    Includes `patient` itself (cairn_person_charts always returns the chart), which is harmless:
    a self-pair is never generated.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT c::text FROM cairn_person_charts(%s::uuid) AS c "
            "UNION SELECT (CASE WHEN low = %s::uuid THEN high ELSE low END)::text "
            "FROM patient_link WHERE low = %s::uuid OR high = %s::uuid",
            (patient, patient, patient, patient),
        )
        return frozenset(r[0] for r in cur.fetchall())


def judged_pairs(conn) -> frozenset[tuple[str, str]]:
    """Every judged pair node-wide, canonical — for the bulk sweep's skip filter.

    Two members of one component are judged even with no direct link row between them (A–B and
    B–C linked make A–C the same person), so the component self-join is needed as well as the
    link rows.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT a.patient_id::text, b.patient_id::text FROM person_member a "
            "JOIN person_member b ON a.person_id = b.person_id AND a.patient_id < b.patient_id "
            "UNION SELECT low::text, high::text FROM patient_link"
        )
        return frozenset(canonical_pair(x, y) for x, y in cur.fetchall())


def drop_judged(pairs, patient, partners) -> list[tuple[str, str]]:
    """The pairs whose OTHER side is not in `partners` (pure)."""
    me = str(patient).lower()
    return [p for p in pairs if (p[1] if p[0] == me else p[0]) not in partners]
```

- [ ] **Step 4: Run, then the whole matcher suite**

```bash
cd matcher && uv run --extra pipeline pytest -rs && uv run ruff check .
```
Expected: all PASS. If the canary fails, the filter is wrong. Never cap or prune a population to
make it pass. Print the differing pairs and the pass that produced them.

- [ ] **Step 5: Commit** (`feat(R4): one chart's candidate pairs, a drift canary, and the skip rule (Refs #679)`).

---

### Task 4: the worker's one-chart check, its queue helpers, and the pure planner

**Files:**
- Create: `matcher/src/cairn_matcher/pipeline/queue_db.py`, `matcher/src/cairn_matcher/pipeline/worker_plan.py`,
  `matcher/src/cairn_matcher/pipeline/worker.py` (first part: `Settings`, `ChartResult`, `check_chart`)
- Test: `matcher/tests/test_worker_plan.py`, `matcher/tests/test_check_chart.py`

**Interfaces:**
- Consumes: `targeted.candidate_pairs_for`, `judged.judged_partners`, `judged.drop_judged`,
  `runner.assess`, `runner.persist`, `db.load_aliases_for`, `db.load_trust_for`.
- Produces:
  ```python
  # worker_plan.py (pure)
  class Mode(Enum): PER_CHART = "per-chart"; SWEEP = "sweep"
  def choose_mode(charts_waiting: int, bulk_threshold: int) -> Mode
  @dataclass class RetryBook:  # backoff_s: float
      def failed(self, patient: str, now: float) -> None
      def succeeded(self, patient: str) -> None
      def held(self, now: float) -> list[str]
  # queue_db.py
  def charts_waiting(conn) -> int
  def next_charts(conn, limit: int, exclude: list[str]) -> list[tuple[str, int]]   # newest first
  def pending_pairs_involving(conn, patient) -> list[tuple[str, str]]
  def clear_chart(conn, patient, upto_id: int) -> int                              # no commit
  def watermark(conn) -> int | None
  def clear_upto(conn, upto_id: int, keep: list[str]) -> int                       # no commit
  def stamp_drained(conn) -> None                                                  # no commit
  def ensure_version(conn, version: str) -> bool                                   # commits
  # worker.py
  @dataclass(frozen=True) class Settings: ...  (fields below)
  @dataclass(frozen=True) class ChartResult: patient: str; proposed: int; retracted: int; skipped_blocks: list
  def check_chart(conn, patient: str, upto_id: int, settings: Settings) -> ChartResult    # commits
  ```

- [ ] **Step 1: Write the failing tests.**

`matcher/tests/test_worker_plan.py`:

```python
"""R4 Task 4: the worker's pure decisions."""

from cairn_matcher.pipeline.worker_plan import Mode, RetryBook, choose_mode


def test_a_backlog_over_the_threshold_is_swept_once():
    assert choose_mode(0, 500) is Mode.PER_CHART
    assert choose_mode(500, 500) is Mode.PER_CHART
    assert choose_mode(501, 500) is Mode.SWEEP


def test_a_failing_chart_is_held_for_the_backoff_then_released():
    book = RetryBook(backoff_s=300.0)
    book.failed("p", now=1000.0)
    assert book.held(1000.0) == ["p"]
    assert book.held(1299.9) == ["p"]
    assert book.held(1300.1) == []


def test_success_clears_a_hold():
    book = RetryBook(backoff_s=300.0)
    book.failed("p", now=0.0)
    book.succeeded("p")
    assert book.held(1.0) == []
```

`matcher/tests/test_check_chart.py`:

```python
"""R4 Task 4: one chart's check — proposals and the notice delete in ONE transaction.

Seeds go through conftest.seed_patient, whose committed projection rows fire db/056's hook, so
every seeded chart has real notices to drain.
"""

import uuid

import pytest

from cairn_matcher.pipeline import queue_db, runner
from cairn_matcher.pipeline.worker import Settings, check_chart
from tests.conftest import seed_patient

A, B = (str(uuid.UUID(int=i)) for i in (11, 12))


def _upto(conn, p):
    with conn.cursor() as cur:
        cur.execute("SELECT max(id) FROM match_pending WHERE patient_id = %s", (p,))
        upto = cur.fetchone()[0]
    conn.rollback()
    return upto


def _count(conn, sql, *args):
    with conn.cursor() as cur:
        cur.execute(sql, args)
        return cur.fetchone()[0]


def _near_duplicates(conn):
    seed_patient(conn, A, dob=("1950-01-07", 60, "day"), names=[("Mary Smith", 60)],
                 identifiers=[("mrn:a", "77", "77")])
    seed_patient(conn, B, dob=("1950-01-07", 60, "day"), names=[("Mary Smyth", 60)],
                 identifiers=[("mrn:a", "77", "77")])


def test_a_near_duplicate_becomes_a_proposal_and_the_notices_are_cleared(pg_conn):
    with pg_conn.cursor() as cur:            # stamp_drained UPDATEs the worker's state row
        cur.execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
    pg_conn.commit()
    _near_duplicates(pg_conn)
    result = check_chart(pg_conn, B, _upto(pg_conn, B), Settings())
    assert result.proposed == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_worker_state "
                           "WHERE last_drained_at IS NOT NULL") == 1


def test_a_strong_pair_is_proposed_and_never_linked(pg_conn):
    _near_duplicates(pg_conn)
    check_chart(pg_conn, B, _upto(pg_conn, B), Settings())
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT count(*) FROM patient_link") == 0


def test_a_change_landing_mid_check_survives_and_is_checked_again(pg_conn, monkeypatch):
    import psycopg

    from tests.conftest import cairn_test_dsn

    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)
    real_assess = runner.assess

    def assess_then_a_colleague_corrects_the_name(conn, a, b, **kw):
        with psycopg.connect(cairn_test_dsn()) as other:
            other.execute("INSERT INTO match_pending (patient_id, reason) VALUES (%s,'change')",
                          (B,))
        return real_assess(conn, a, b, **kw)

    monkeypatch.setattr(runner, "assess", assess_then_a_colleague_corrects_the_name)
    check_chart(pg_conn, B, upto, Settings())
    ids = _count(pg_conn, "SELECT array_agg(id) FROM match_pending WHERE patient_id = %s", B)
    assert ids is not None and all(i > upto for i in ids)


def test_a_crash_before_commit_loses_nothing(pg_conn, monkeypatch):
    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)

    def boom(*_a, **_k):
        raise RuntimeError("crash between scoring and the delete")

    monkeypatch.setattr(queue_db, "clear_chart", boom)
    with pytest.raises(RuntimeError):
        check_chart(pg_conn, B, upto, Settings())
    pg_conn.rollback()
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) > 0


def test_a_linked_pair_is_never_proposed(pg_conn):
    _near_duplicates(pg_conn)
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (A, A, B, A))
    pg_conn.commit()
    assert check_chart(pg_conn, B, _upto(pg_conn, B), Settings()).proposed == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0


def test_a_stale_pending_proposal_no_longer_blocked_is_reassessed(pg_conn):
    # A pending proposal for (A, C) where C now shares nothing with A: the per-chart #210
    # reconciliation re-assesses it, and below the floor it is retracted.
    c = str(uuid.UUID(int=13))
    seed_patient(pg_conn, A, names=[("Mary Smith", 20)])
    seed_patient(pg_conn, c, names=[("Zed Quux", 20)])
    lo, hi = sorted([A, c])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (lo, hi))
    pg_conn.commit()
    result = check_chart(pg_conn, A, _upto(pg_conn, A), Settings())
    assert result.retracted == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "retracted"


def test_the_worker_role_suffices(pg_conn):
    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)
    with pg_conn.cursor() as cur:
        cur.execute("SET ROLE cairn_agent")
    check_chart(pg_conn, B, upto, Settings())
    with pg_conn.cursor() as cur:
        cur.execute("RESET ROLE")
    pg_conn.commit()
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
```

If `_near_duplicates` does not band at least REVIEW under the default thresholds, strengthen it by
giving A and B the SAME name. Check with a one-off `runner.assess` in the test before relying on it.

- [ ] **Step 2: Run to verify they fail**

```bash
cd matcher && uv run --extra pipeline pytest tests/test_worker_plan.py tests/test_check_chart.py -rs
```
Expected: `ModuleNotFoundError` for `worker_plan` / `worker` / `queue_db`.

- [ ] **Step 3: Implement.**

`worker_plan.py`:

```python
# matcher/src/cairn_matcher/pipeline/worker_plan.py
"""The commit-time worker's pure decisions (repair path R4): which mode, and which charts to hold.

Pure and psycopg-free, so the policy is testable with no database.
"""

from dataclasses import dataclass, field
from enum import Enum


class Mode(Enum):
    """How the next round drains the queue."""

    PER_CHART = "per-chart"   # check each waiting chart on its own, newest change first
    SWEEP = "sweep"           # a backlog too big for per-chart checks: one full sweep instead


def choose_mode(charts_waiting: int, bulk_threshold: int) -> Mode:
    """Sweep when MORE than `bulk_threshold` charts wait.

    Per-chart blocking scans the population once per chart, so N waiting charts cost ~N scans;
    one sweep costs one. Above the threshold (set from Task 7's measurement) the sweep is cheaper.
    """
    return Mode.SWEEP if charts_waiting > bulk_threshold else Mode.PER_CHART


@dataclass
class RetryBook:
    """Charts whose check raised, held back for `backoff_s` so the worker never spins on one.

    In memory on purpose: a restarted worker retries at once. A held chart keeps its notices, so
    the node keeps reading it as "not yet checked" — the hold is never a silent skip.
    """

    backoff_s: float
    _until: dict[str, float] = field(default_factory=dict)

    def failed(self, patient: str, now: float) -> None:
        self._until[patient] = now + self.backoff_s

    def succeeded(self, patient: str) -> None:
        self._until.pop(patient, None)

    def held(self, now: float) -> list[str]:
        return sorted(p for p, until in self._until.items() if until > now)
```

`queue_db.py`:

```python
# matcher/src/cairn_matcher/pipeline/queue_db.py
"""The worker's SQL over db/056's notice log and state row (repair path R4).

None of these commit unless the docstring says so: check_chart and run_bulk own the transaction,
so a chart's proposals and the delete of its notices land together or not at all.
"""

_ALL_CHARTS_SQL = (
    "SELECT patient_id FROM patient_chart UNION SELECT patient_id FROM patient_name "
    "UNION SELECT patient_id FROM patient_demographic "
    "UNION SELECT patient_id FROM patient_identifier"
)


def charts_waiting(conn) -> int:
    with conn.cursor() as cur:
        cur.execute("SELECT count(DISTINCT patient_id) FROM match_pending")
        return cur.fetchone()[0]


def next_charts(conn, limit: int, exclude: list[str]) -> list[tuple[str, int]]:
    """Up to `limit` waiting charts, NEWEST change first, as (patient, highest notice id).

    Newest first so a fresh registration is checked within seconds even behind a restore's
    backlog. `exclude` is the RetryBook's held charts, so a poison chart never starves the rest.
    """
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id::text, max(id) FROM match_pending "
            "WHERE NOT (patient_id = ANY(%s::uuid[])) "
            "GROUP BY patient_id ORDER BY max(id) DESC LIMIT %s",
            (exclude, limit),
        )
        return [(p, int(i)) for p, i in cur.fetchall()]


def pending_pairs_involving(conn, patient) -> list[tuple[str, str]]:
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_low::text, patient_high::text FROM match_proposal "
            "WHERE status = 'pending' AND (patient_low = %s::uuid OR patient_high = %s::uuid)",
            (patient, patient),
        )
        return [(lo, hi) for lo, hi in cur.fetchall()]


def clear_chart(conn, patient, upto_id: int) -> int:
    """Delete the chart's notices up to the highest id the check READ — never by patient alone,
    which would lose a change that landed mid-check."""
    with conn.cursor() as cur:
        cur.execute("DELETE FROM match_pending WHERE patient_id = %s::uuid AND id <= %s",
                    (patient, upto_id))
        return cur.rowcount


def watermark(conn) -> int | None:
    with conn.cursor() as cur:
        cur.execute("SELECT max(id) FROM match_pending")
        return cur.fetchone()[0]


def clear_upto(conn, upto_id: int, keep: list[str]) -> int:
    """After a bulk sweep: delete every notice up to the watermark EXCEPT the charts in `keep`
    (charts in a pair the sweep failed to score — they stay "not yet checked")."""
    with conn.cursor() as cur:
        cur.execute(
            "DELETE FROM match_pending WHERE id <= %s AND NOT (patient_id = ANY(%s::uuid[]))",
            (upto_id, keep),
        )
        return cur.rowcount


def stamp_drained(conn) -> None:
    with conn.cursor() as cur:
        cur.execute("UPDATE match_worker_state SET last_drained_at = clock_timestamp()")


def ensure_version(conn, version: str) -> bool:
    """Record the running matcher_version; when it differs (or no worker ever ran), queue every
    chart for a 'config' re-check. One transaction; COMMITS. Returns True when it re-queued.

    The first run on a node also lands here (no state row): charts created before db/056 have no
    notices and have never been checked.
    """
    with conn.cursor() as cur:
        cur.execute("SELECT matcher_version FROM match_worker_state FOR UPDATE")
        row = cur.fetchone()
        if row is not None and row[0] == version:
            conn.rollback()
            return False
        cur.execute(f"INSERT INTO match_pending (patient_id, reason) "
                    f"SELECT patient_id, 'config' FROM ({_ALL_CHARTS_SQL}) charts")
        cur.execute(
            "INSERT INTO match_worker_state (singleton, matcher_version) VALUES (TRUE, %s) "
            "ON CONFLICT (singleton) DO UPDATE SET matcher_version = EXCLUDED.matcher_version",
            (version,),
        )
    conn.commit()
    return True
```

`worker.py` (first part):

```python
# matcher/src/cairn_matcher/pipeline/worker.py
"""The commit-time duplicate check (repair path R4, #679, ADR-0076 decision 7).

db/056 appends a notice whenever a chart's identity evidence changes. This worker drains them:
for each chart it finds the candidate pairs (targeted.py), drops pairs already judged
(judged.py), assesses each (runner.assess) and writes the outcomes plus the delete of the
chart's notices in ONE short transaction. It writes match_proposal and nothing else that
matters: it NEVER links — a hit is a proposal a human resolves (R2's panel, R5's worklist).

Requires the optional `pipeline` extra (psycopg).
"""

from dataclasses import dataclass, field

from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig
from cairn_matcher.pipeline import judged, queue_db, runner, targeted
from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Thresholds
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights


@dataclass(frozen=True)
class Settings:
    """The worker's knobs. Defaults are set from Task 7's measurement (design page note)."""

    max_block_size: int = targeted.DEFAULT_TARGETED_CAP
    sweep_block_size: int = 100        # the sweep's own all-pairs cap, unchanged
    bulk_threshold: int = 500
    batch: int = 50
    retry_after_s: float = 300.0
    poll_s: float = 60.0
    pace_ms: int = 0
    thresholds: Thresholds = DEFAULT_THRESHOLDS
    weights: Weights = DEFAULT_WEIGHTS
    config: ComparatorConfig = DEFAULT_CONFIG


@dataclass(frozen=True)
class ChartResult:
    patient: str
    proposed: int
    retracted: int
    skipped_blocks: list = field(default_factory=list)


def check_chart(conn, patient: str, upto_id: int, settings: Settings) -> ChartResult:
    """Check one chart and clear its notices up to `upto_id`, in ONE transaction; COMMITS.

    Reads first (pairs, skip rule, stale pending proposals, the assessments), then writes every
    outcome, deletes the notices it read, stamps the drain time and commits. Nothing is locked
    until the writes, so a clinical write never waits on this. If anything raises, the caller
    rolls back: no proposal lands and the notices stay, so the chart is checked again (a crash
    loses nothing).
    """
    from cairn_matcher.pipeline import db

    pairs, skipped = targeted.candidate_pairs_for(
        conn, patient, max_block_size=settings.max_block_size)
    pairs = judged.drop_judged(pairs, patient, judged.judged_partners(conn, patient))
    # #210 per chart: a PENDING proposal involving this chart that blocking no longer produces
    # (a Doe identified since) is re-assessed, so a stale row is retracted rather than left.
    generated = set(pairs)
    stale = [p for p in queue_db.pending_pairs_involving(conn, patient) if p not in generated]
    everyone = {pid for pair in pairs + stale for pid in pair}
    aliases = db.load_aliases_for(conn, everyone)
    trust = db.load_trust_for(conn, everyone)
    verdicts = [
        runner.assess(conn, low, high, thresholds=settings.thresholds,
                      weights=settings.weights, config=settings.config,
                      aliases=aliases, trust=trust)
        for low, high in pairs + stale
    ]
    proposed = retracted = 0
    for v in verdicts:
        wrote = runner.persist(conn, v)
        if v.band is not None:
            proposed += 1
        elif wrote:
            retracted += 1
    queue_db.clear_chart(conn, patient, upto_id)
    queue_db.stamp_drained(conn)
    conn.commit()
    return ChartResult(patient, proposed, retracted, skipped)
```

Calls go through the module objects (`runner.assess`, `queue_db.clear_chart`) on purpose: the tests
monkeypatch them. Check that `db.load_aliases_for` / `load_trust_for` accept an empty set. If either
issues `= ANY('{}')` and returns `{}`, good; if it raises, guard with `if everyone else {}`.

- [ ] **Step 4: Run**

```bash
cd matcher && uv run --extra pipeline pytest -rs && uv run ruff check .
```
Expected: all PASS.

- [ ] **Step 5: Commit** (`feat(R4): check one chart — proposals and its notices in one transaction (Refs #679)`).

---

### Task 5: bulk mode, the version re-check, the drain loop, and `cairn-matcher watch`

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/sweep.py` (opt-in `skip_pairs`)
- Modify: `matcher/src/cairn_matcher/pipeline/worker.py` (`run_bulk`, `DrainReport`, `drain`, `watch`)
- Create: `matcher/src/cairn_matcher/cli.py`
- Modify: `matcher/pyproject.toml` (`[project.scripts]`)
- Test: `matcher/tests/test_worker_drain.py`, `matcher/tests/test_cli_watch.py`

**Interfaces:**
- Consumes: Task 4's `check_chart`, `Settings`, `queue_db.*`, `worker_plan.*`; `judged.judged_pairs`;
  `banding.matcher_version`.
- Produces:
  ```python
  def sweep(conn, *, max_block_size=100, thresholds=..., weights=..., config=...,
            skip_pairs: frozenset[tuple[str, str]] | None = None) -> SweepResult
  @dataclass(frozen=True) class DrainReport: checked: int; failed: int; swept: bool
  def run_bulk(conn, settings: Settings) -> SweepResult                  # commits
  def drain(conn, settings, book: RetryBook, clock=time.monotonic, sleep=time.sleep) -> DrainReport
  def watch(dsn: str, settings: Settings, *, once: bool = False) -> int  # process exit code
  # cli.py
  def main(argv: list[str] | None = None) -> int
  ```

- [ ] **Step 1: Write the failing tests.**

`matcher/tests/test_worker_drain.py`:

```python
"""R4 Task 5: the drain — newest first, a poison chart held, a big backlog swept once."""

import uuid

from cairn_matcher.pipeline import queue_db, worker
from cairn_matcher.pipeline.banding import matcher_version
from cairn_matcher.pipeline.worker import Settings, drain
from cairn_matcher.pipeline.worker_plan import RetryBook
from tests.conftest import seed_patient

A, B, C = (str(uuid.UUID(int=i)) for i in (21, 22, 23))


def _count(conn, sql, *args):
    with conn.cursor() as cur:
        cur.execute(sql, args)
        n = cur.fetchone()[0]
    conn.rollback()
    return n


def _state(conn):
    with conn.cursor() as cur:
        cur.execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
    conn.commit()


def test_the_newest_change_is_checked_first(pg_conn):
    for p in (A, B, C):                       # C is seeded last: its notice is newest
        seed_patient(pg_conn, p, names=[(f"Name {p[-2:]}", 20)])
    assert [p for p, _ in queue_db.next_charts(pg_conn, 10, [])] == [C, B, A]
    pg_conn.rollback()


def test_a_poison_chart_is_held_while_the_rest_drain(pg_conn, monkeypatch):
    _state(pg_conn)
    for p in (A, B):
        seed_patient(pg_conn, p, names=[(f"Name {p[-2:]}", 20)])
    real = worker.check_chart

    def check(conn, patient, upto, settings):
        if patient == A:
            raise RuntimeError("poison")
        return real(conn, patient, upto, settings)

    monkeypatch.setattr(worker, "check_chart", check)
    book = RetryBook(backoff_s=300.0)
    report = drain(pg_conn, Settings(), book, clock=lambda: 0.0, sleep=lambda s: None)
    assert (report.checked, report.failed) == (1, 1)
    assert book.held(0.0) == [A]
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", A) > 0
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 0


def test_a_big_backlog_is_swept_once_up_to_the_watermark(pg_conn):
    _state(pg_conn)
    ids = [str(uuid.UUID(int=100 + i)) for i in range(6)]
    for p in ids:
        seed_patient(pg_conn, p, names=[("Mary Smith", 20)])
    with pg_conn.cursor() as cur:            # A and B linked: the sweep must skip them
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (ids[0], ids[0], ids[1], ids[0]))
    pg_conn.commit()
    report = drain(pg_conn, Settings(bulk_threshold=3), RetryBook(300.0),
                   clock=lambda: 0.0, sleep=lambda s: None)
    assert report.swept
    assert _count(pg_conn, "SELECT count(*) FROM match_pending") == 0
    lo, hi = sorted(ids[:2])
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal "
                           "WHERE patient_low = %s AND patient_high = %s", lo, hi) == 0


def test_a_sweep_pair_that_errors_keeps_its_charts_notices(pg_conn, monkeypatch):
    _state(pg_conn)
    ids = [str(uuid.UUID(int=200 + i)) for i in range(5)]
    for p in ids:
        seed_patient(pg_conn, p, names=[("Mary Smith", 20)])
    from cairn_matcher.pipeline import runner
    real = runner.propose

    def propose(conn, a, b, **kw):
        if ids[0] in (str(a), str(b)):
            raise RuntimeError("unscoreable")
        return real(conn, a, b, **kw)

    monkeypatch.setattr("cairn_matcher.pipeline.sweep.propose", propose)
    drain(pg_conn, Settings(bulk_threshold=2), RetryBook(300.0),
          clock=lambda: 0.0, sleep=lambda s: None)
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s",
                  ids[0]) > 0


def test_a_new_version_or_a_first_run_queues_every_chart(pg_conn):
    for p in (A, B):
        seed_patient(pg_conn, p, names=[("Mary Smith", 20)])
    with pg_conn.cursor() as cur:
        cur.execute("TRUNCATE match_pending")    # as on a node that existed before db/056
    pg_conn.commit()
    v = matcher_version()
    assert queue_db.ensure_version(pg_conn, v) is True
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE reason = 'config'") == 2
    assert queue_db.ensure_version(pg_conn, v) is False
    assert _count(pg_conn, "SELECT count(*) FROM match_pending") == 2
```

`matcher/tests/test_cli_watch.py`:

```python
"""R4 Task 5: `cairn-matcher watch --once` drains and exits; nothing links."""

import uuid

from cairn_matcher.cli import main
from tests.conftest import cairn_test_dsn, seed_patient

A, B = (str(uuid.UUID(int=i)) for i in (31, 32))


def test_watch_once_checks_every_chart_and_exits_zero(pg_conn):
    for p in (A, B):
        seed_patient(pg_conn, p, dob=("1950-01-07", 60, "day"), names=[("Mary Smith", 60)])
    assert main(["watch", "--once", "--dsn", cairn_test_dsn()]) == 0
    with pg_conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM match_pending")
        assert cur.fetchone()[0] == 0
        cur.execute("SELECT count(*) FROM match_proposal")
        assert cur.fetchone()[0] == 1
        cur.execute("SELECT count(*) FROM patient_link")
        assert cur.fetchone()[0] == 0
        cur.execute("SELECT last_drained_at IS NOT NULL FROM match_worker_state")
        assert cur.fetchone()[0] is True
```

- [ ] **Step 2: Run to verify they fail**

```bash
cd matcher && uv run --extra pipeline pytest tests/test_worker_drain.py tests/test_cli_watch.py -rs
```
Expected: `ImportError` (`drain`, `cairn_matcher.cli`) and a `TypeError` for `skip_pairs`.

- [ ] **Step 3: Implement.**

`sweep.py`: add the keyword argument `skip_pairs: frozenset[tuple[str, str]] | None = None` to
`sweep()`, documented as *"opt-in (repair path R4's bulk mode): candidate pairs in this set — already
judged by the identity algebra — are neither scored nor reconciled. None (the default) keeps the
sweep's historical behaviour."* Right after `generate_candidate_pairs` add:

```python
    if skip_pairs:
        pairs = [p for p in pairs if p not in skip_pairs]
```

`worker.py` (append):

```python
import logging
import time

from cairn_matcher.pipeline.banding import matcher_version
from cairn_matcher.pipeline.sweep import SweepResult, sweep
from cairn_matcher.pipeline.worker_plan import Mode, RetryBook, choose_mode

log = logging.getLogger("cairn_matcher.worker")


@dataclass(frozen=True)
class DrainReport:
    checked: int
    failed: int
    swept: bool


def run_bulk(conn, settings: Settings) -> SweepResult:
    """A backlog too big for per-chart checks: ONE sweep, then clear up to the watermark; COMMITS.

    The watermark is read BEFORE the sweep, so a notice arriving during it survives and gets a
    per-chart check. Charts in a pair the sweep failed to score keep their notices (`keep`), so
    they still read "not yet checked". The sweep keeps its own all-pairs cap (100): a block it skips
    is reported in the result, as it always has been.
    """
    mark = queue_db.watermark(conn)
    skip = judged.judged_pairs(conn)
    conn.rollback()
    result = sweep(conn, max_block_size=settings.sweep_block_size,
                   thresholds=settings.thresholds, weights=settings.weights,
                   config=settings.config, skip_pairs=skip)
    keep = sorted({pid for e in result.errors for pid in e.pair})
    if mark is not None:
        queue_db.clear_upto(conn, mark, keep)
    queue_db.stamp_drained(conn)
    conn.commit()
    return result


def drain(conn, settings: Settings, book: RetryBook, clock=time.monotonic,
          sleep=time.sleep) -> DrainReport:
    """Drain the queue until nothing is left but held charts.

    One sweep first when the backlog is over the threshold; then per-chart checks, newest change
    first, skipping charts the RetryBook holds. A chart whose check raises is rolled back, logged,
    and held; its notices stay. `clock`/`sleep` are injectable for tests.
    """
    swept = False
    waiting = queue_db.charts_waiting(conn)
    conn.rollback()
    if choose_mode(waiting, settings.bulk_threshold) is Mode.SWEEP:
        result = run_bulk(conn, settings)
        swept = True
        log.info("swept a backlog of %d charts: %d pairs, %d errors, %d blocks skipped",
                 waiting, result.generated, len(result.errors), len(result.skipped_blocks))
    checked = failed = 0
    while True:
        batch = queue_db.next_charts(conn, settings.batch, book.held(clock()))
        conn.rollback()
        if not batch:
            break
        for patient, upto in batch:
            try:
                check_chart(conn, patient, upto, settings)
                book.succeeded(patient)
                checked += 1
            except Exception as exc:  # noqa: BLE001 — one bad chart must not stop the drain
                conn.rollback()
                book.failed(patient, clock())
                failed += 1
                log.warning("duplicate check failed for %s (held %.0fs): %s: %s",
                            patient, settings.retry_after_s, type(exc).__name__, exc)
            if settings.pace_ms:
                sleep(settings.pace_ms / 1000)
    if waiting == 0:
        queue_db.stamp_drained(conn)       # an empty round still says "last ran HH:MM"
        conn.commit()
    return DrainReport(checked, failed, swept)


def watch(dsn: str, settings: Settings, *, once: bool = False) -> int:
    """Run the worker: LISTEN, re-queue on a version change, drain, wait, repeat.

    LISTEN is issued BEFORE the first drain, so a notice committed between a drain and the wait
    still wakes us. A 60 s poll (settings.poll_s) backs NOTIFY up across a reconnect. With
    `once`, drain and return 0 (1 if any chart failed). A lost connection reconnects with a
    backoff capped at 60 s; with `once` it is fatal (exit 2).
    """
    import psycopg

    version = matcher_version(settings.weights, settings.thresholds, settings.config)
    book = RetryBook(settings.retry_after_s)
    backoff = 1.0
    while True:
        try:
            with psycopg.connect(dsn, autocommit=True) as listen, psycopg.connect(dsn) as work:
                listen.execute("LISTEN cairn_match_pending")
                if queue_db.ensure_version(work, version):
                    log.info("matcher %s: every chart queued for a re-check", version)
                backoff = 1.0
                while True:
                    report = drain(work, settings, book)
                    if once:
                        return 0 if report.failed == 0 else 1
                    for _ in listen.notifies(timeout=settings.poll_s, stop_after=1):
                        pass
        except psycopg.OperationalError as exc:
            if once:
                log.error("cannot reach the database: %s", exc)
                return 2
            log.warning("database connection lost (%s); retrying in %.0fs", exc, backoff)
            time.sleep(backoff)
            backoff = min(backoff * 2, 60.0)
```

`cli.py`:

```python
# matcher/src/cairn_matcher/cli.py
"""`cairn-matcher` — the advisory matcher's command line (repair path R4).

`cairn-matcher watch` runs the commit-time duplicate check: it drains db/056's notices and writes
match_proposal rows. It never links. Connect it as a role holding cairn_agent; with no --dsn the
standard libpq environment (PGHOST, PGPORT, PGUSER, PGDATABASE, …) is used.
"""

import argparse
import logging

from cairn_matcher.pipeline.worker import Settings, watch


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="cairn-matcher")
    sub = p.add_subparsers(dest="cmd", required=True)
    w = sub.add_parser("watch", help="run the commit-time duplicate check")
    w.add_argument("--dsn", default="", help="libpq connection string (default: PG* env)")
    w.add_argument("--once", action="store_true", help="drain the queue once and exit")
    d = Settings()
    w.add_argument("--poll-seconds", type=float, default=d.poll_s)
    w.add_argument("--bulk-threshold", type=int, default=d.bulk_threshold)
    w.add_argument("--max-block-size", type=int, default=d.max_block_size)
    w.add_argument("--pace-ms", type=int, default=d.pace_ms)
    args = p.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(message)s")
    settings = Settings(poll_s=args.poll_seconds, bulk_threshold=args.bulk_threshold,
                        max_block_size=args.max_block_size, pace_ms=args.pace_ms)
    return watch(args.dsn, settings, once=args.once)


if __name__ == "__main__":
    raise SystemExit(main())
```

`pyproject.toml`:

```toml
[project.scripts]
cairn-matcher = "cairn_matcher.cli:main"
```

`worker.py` is now about 230 lines. If it passes ~300, move `watch` into `pipeline/watch.py`.

- [ ] **Step 4: Run**

```bash
cd matcher && uv run --extra pipeline pytest -rs && uv run ruff check .
uv run --extra pipeline cairn-matcher watch --help
```
Expected: all PASS; `--help` lists the flags.

- [ ] **Step 5: Commit** (`feat(R4): cairn-matcher watch — newest first, one sweep for a backlog, never a link (Refs #679)`).

---

### Task 6: the node's status reads and `cairn-node duplicate-check`

**Files:**
- Create: `crates/cairn-node/src/duplicate_check.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod duplicate_check;`), `crates/cairn-node/src/main.rs`
- Test: unit tests in `duplicate_check.rs`; `crates/cairn-node/tests/duplicate_check.rs`

**Interfaces:**
- Consumes: Task 1's `cairn_duplicate_check_status()`, `cairn_chart_check_pending(uuid)`.
- Produces (R5 will call these):
  ```rust
  pub const STALLED_AFTER_SECS: i64 = 300;
  pub struct QueueSnapshot { pub charts_waiting: i64, pub newest_age_secs: Option<i64>,
      pub config_recheck: bool, pub worker_seen: bool, pub last_drained_hhmm: Option<String> }
  pub enum CheckState { NeverRun { waiting: i64 }, Stalled { waiting: i64, last_ran: Option<String> },
      CatchingUp { waiting: i64, config_recheck: bool }, Current { last_ran: Option<String> } }
  pub fn classify(s: &QueueSnapshot, stalled_after_secs: i64) -> CheckState
  pub fn status_line(state: &CheckState) -> String
  pub fn chart_line(pending: bool) -> &'static str
  pub async fn read_snapshot(client: &tokio_postgres::Client) -> anyhow::Result<QueueSnapshot>
  pub async fn chart_check_pending(client: &tokio_postgres::Client, patient: uuid::Uuid) -> anyhow::Result<bool>
  ```

- [ ] **Step 1: Write the failing tests.** Unit tests, at the foot of `duplicate_check.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn snap(waiting: i64, newest: Option<i64>, config: bool, seen: bool) -> QueueSnapshot {
        QueueSnapshot {
            charts_waiting: waiting,
            newest_age_secs: newest,
            config_recheck: config,
            worker_seen: seen,
            last_drained_hhmm: seen.then(|| "09:41".to_string()),
        }
    }

    #[test]
    fn no_worker_row_is_never_run_whatever_is_waiting() {
        assert_eq!(classify(&snap(3, Some(9999), false, false), 300), CheckState::NeverRun { waiting: 3 });
    }

    #[test]
    fn an_empty_queue_is_current() {
        assert_eq!(
            classify(&snap(0, None, false, true), 300),
            CheckState::Current { last_ran: Some("09:41".into()) }
        );
    }

    #[test]
    fn behind_is_judged_by_the_newest_notice_strictly_past_the_threshold() {
        assert!(matches!(classify(&snap(5, Some(300), false, true), 300), CheckState::CatchingUp { .. }));
        assert!(matches!(classify(&snap(5, Some(301), false, true), 300), CheckState::Stalled { .. }));
    }

    #[test]
    fn the_wording_golden() {
        assert_eq!(
            status_line(&CheckState::NeverRun { waiting: 0 }),
            "Duplicate check has never run on this node."
        );
        assert_eq!(
            status_line(&CheckState::NeverRun { waiting: 2 }),
            "Duplicate check has never run on this node — 2 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::Stalled { waiting: 1, last_ran: Some("09:41".into()) }),
            "Duplicate check is behind — last ran 09:41; 1 chart waiting."
        );
        assert_eq!(
            status_line(&CheckState::Stalled { waiting: 4, last_ran: None }),
            "Duplicate check is behind — it has not finished a round yet; 4 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::CatchingUp { waiting: 7, config_recheck: false }),
            "Duplicate check running — 7 charts waiting."
        );
        assert_eq!(
            status_line(&CheckState::CatchingUp { waiting: 7, config_recheck: true }),
            "Duplicate check running — 7 charts waiting (re-checking all charts after a matcher update)."
        );
        assert_eq!(
            status_line(&CheckState::Current { last_ran: Some("09:41".into()) }),
            "Duplicate check up to date — last ran 09:41."
        );
        assert_eq!(
            chart_line(true),
            "This chart: duplicate check not yet run since its identity details last changed."
        );
        assert_eq!(chart_line(false), "This chart: duplicate check up to date.");
    }
}
```

`crates/cairn-node/tests/duplicate_check.rs`:

```rust
//! Repair path R4: the node reads db/056's status, and the CLI prints it.
mod common;
use cairn_node::db;
use cairn_node::duplicate_check::{chart_check_pending, classify, read_snapshot, CheckState, STALLED_AFTER_SECS};
use common::cs;
use std::process::Command;
use uuid::Uuid;

#[tokio::test]
async fn a_node_with_no_worker_reads_never_run_and_every_chart_pending() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state").await.unwrap();
    let s = read_snapshot(&c).await.unwrap();
    assert_eq!(classify(&s, STALLED_AFTER_SECS), CheckState::NeverRun { waiting: 0 });
    assert!(chart_check_pending(&c, Uuid::now_v7()).await.unwrap());
}

#[tokio::test]
async fn a_stale_newest_notice_reads_stalled() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE match_pending, match_worker_state; \
         INSERT INTO match_worker_state (matcher_version, last_drained_at) VALUES ('v', now()); \
         INSERT INTO match_pending (patient_id, reason, queued_at) \
           VALUES (gen_random_uuid(), 'change', now() - interval '6 minutes');",
    )
    .await
    .unwrap();
    let s = read_snapshot(&c).await.unwrap();
    assert!(matches!(classify(&s, STALLED_AFTER_SECS), CheckState::Stalled { waiting: 1, .. }));
}

#[tokio::test]
async fn the_cli_prints_the_status_and_the_charts_line() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE match_pending, match_worker_state").await.unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args(["--conn", &base, "duplicate-check", "--patient", &Uuid::now_v7().to_string()])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Duplicate check has never run on this node."), "{text}");
    assert!(text.contains("This chart: duplicate check not yet run"), "{text}");
}
```

Check where `--conn` sits on the command line: if it is a global flag on `Cli` (as `cli.conn`
suggests), it goes before the subcommand, as above. Confirm with `cairn-node --help`.

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p cairn-node --lib duplicate_check
cargo test -p cairn-node --test duplicate_check -- --nocapture
```
Expected: compile errors (module missing).

- [ ] **Step 3: Implement** `crates/cairn-node/src/duplicate_check.rs`:

```rust
//! The commit-time duplicate check's status, as the node reads it (repair path R4, #679).
//!
//! The matcher worker (`cairn-matcher watch`, matcher/) drains db/056's notice log. This module
//! answers two questions without ever claiming more than is true (principle 4):
//! - node-wide: is the check current, catching up, stalled, or has it never run?
//!   ([`classify`] over [`read_snapshot`]);
//! - one chart: has it been checked since its identity evidence last changed?
//!   ([`chart_check_pending`]).
//!
//! "Stalled" is judged by the NEWEST waiting notice. The worker drains newest first, so only a
//! stopped or stuck worker lets the newest notice age. A restore backlog — thousands of OLD notices
//! being worked through while fresh registrations are checked within seconds — reads as catching
//! up, never as an alarm. All wording lives here, in pure functions with a golden test; R5's window
//! will reuse them.

use anyhow::Context;
use tokio_postgres::Client;
use uuid::Uuid;

/// How old the newest waiting notice may be before the check counts as stalled. Soft policy: a
/// worker drains a fresh change in seconds, so five minutes is generous.
pub const STALLED_AFTER_SECS: i64 = 5 * 60;

/// One read of db/056's `cairn_duplicate_check_status()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueSnapshot {
    pub charts_waiting: i64,
    /// Age in seconds of the NEWEST waiting notice; None when nothing waits.
    pub newest_age_secs: Option<i64>,
    /// A 'config' re-check (every chart, after a matcher update or a first run) is in progress.
    pub config_recheck: bool,
    /// A worker has run on this node at least once (its state row exists).
    pub worker_seen: bool,
    /// When the worker last finished a drain, as the database's local HH:MM.
    pub last_drained_hhmm: Option<String>,
}

/// What the node may honestly say about its duplicate check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    NeverRun { waiting: i64 },
    Stalled { waiting: i64, last_ran: Option<String> },
    CatchingUp { waiting: i64, config_recheck: bool },
    Current { last_ran: Option<String> },
}

/// Classify a snapshot (pure). A missing worker row wins over everything: until a worker has run
/// once, charts that predate db/056 have never been checked, whatever the queue shows.
pub fn classify(s: &QueueSnapshot, stalled_after_secs: i64) -> CheckState {
    if !s.worker_seen {
        return CheckState::NeverRun { waiting: s.charts_waiting };
    }
    match s.newest_age_secs {
        None => CheckState::Current { last_ran: s.last_drained_hhmm.clone() },
        Some(age) if age > stalled_after_secs => CheckState::Stalled {
            waiting: s.charts_waiting,
            last_ran: s.last_drained_hhmm.clone(),
        },
        Some(_) => CheckState::CatchingUp {
            waiting: s.charts_waiting,
            config_recheck: s.config_recheck,
        },
    }
}

fn charts(n: i64) -> String {
    if n == 1 { "1 chart".into() } else { format!("{n} charts") }
}

/// The one sentence for a state (pure; golden-tested).
pub fn status_line(state: &CheckState) -> String {
    match state {
        CheckState::NeverRun { waiting: 0 } => "Duplicate check has never run on this node.".into(),
        CheckState::NeverRun { waiting } => format!(
            "Duplicate check has never run on this node — {} waiting.",
            charts(*waiting)
        ),
        CheckState::Stalled { waiting, last_ran: Some(t) } => format!(
            "Duplicate check is behind — last ran {t}; {} waiting.",
            charts(*waiting)
        ),
        CheckState::Stalled { waiting, last_ran: None } => format!(
            "Duplicate check is behind — it has not finished a round yet; {} waiting.",
            charts(*waiting)
        ),
        CheckState::CatchingUp { waiting, config_recheck } => format!(
            "Duplicate check running — {} waiting{}.",
            charts(*waiting),
            if *config_recheck { " (re-checking all charts after a matcher update)" } else { "" }
        ),
        CheckState::Current { last_ran: Some(t) } => {
            format!("Duplicate check up to date — last ran {t}.")
        }
        CheckState::Current { last_ran: None } => "Duplicate check up to date.".into(),
    }
}

/// One chart's line (pure; golden-tested).
pub fn chart_line(pending: bool) -> &'static str {
    if pending {
        "This chart: duplicate check not yet run since its identity details last changed."
    } else {
        "This chart: duplicate check up to date."
    }
}

/// Read the node-wide snapshot.
pub async fn read_snapshot(client: &Client) -> anyhow::Result<QueueSnapshot> {
    let r = client
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .context("reading the duplicate-check status")?;
    Ok(QueueSnapshot {
        charts_waiting: r.get("charts_waiting"),
        newest_age_secs: r.get("newest_age_s"),
        config_recheck: r.get("config_recheck"),
        worker_seen: r.get("worker_seen"),
        last_drained_hhmm: r.get("last_drained_hhmm"),
    })
}

/// Has `patient` been checked since its identity evidence last changed? (db/056's
/// `cairn_chart_check_pending`: TRUE while a notice waits, and for every chart before the
/// first worker run.)
pub async fn chart_check_pending(client: &Client, patient: Uuid) -> anyhow::Result<bool> {
    Ok(client
        .query_one("SELECT cairn_chart_check_pending($1)", &[&patient])
        .await
        .context("reading whether this chart's duplicate check is pending")?
        .get(0))
}
```

`main.rs`: add to the `Cmd` enum (next to `Reproject`):

```rust
    /// Report the commit-time duplicate check (repair path R4): whether the matcher worker is
    /// current, catching up, stalled, or has never run — and, with --patient, whether that chart
    /// has been checked since its identity details last changed. The worker itself is
    /// `cairn-matcher watch` (matcher/); this command only reads.
    DuplicateCheck {
        /// Also report this chart's own state.
        #[arg(long)]
        patient: Option<Uuid>,
    },
```

and its arm:

```rust
        Cmd::DuplicateCheck { patient } => {
            use cairn_node::duplicate_check as dc;
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            let snap = dc::read_snapshot(&db).await?;
            println!("{}", dc::status_line(&dc::classify(&snap, dc::STALLED_AFTER_SECS)));
            if let Some(p) = patient {
                println!("{}", dc::chart_line(dc::chart_check_pending(&db, p).await?));
            }
        }
```

- [ ] **Step 4: Run**

```bash
cargo test -p cairn-node --lib duplicate_check
cargo test -p cairn-node --test duplicate_check -- --nocapture
cargo clippy -p cairn-node --all-targets -- -D warnings
cargo fmt --all --check
```
Expected: all PASS; no `skipped:` line.

- [ ] **Step 5: Commit** (`feat(R4): the node reads the duplicate check's status; cairn-node duplicate-check (Refs #679)`).

---

### Task 7: the measurement, the runbook, the docs, the follow-ons, full gates, PR

**Files:**
- Create: `matcher/src/cairn_matcher/eval/measure_check.py`; `docs/developers/running-the-duplicate-check.md`
- Modify: `matcher/README.md` (a "Running the duplicate check" section pointing to the runbook);
  `mkdocs.yml` (nav entry for the runbook); the design page (an as-built note under "R4 — designed
  2026-10-04": every deviation, or "none", plus the measured numbers); `targeted.DEFAULT_TARGETED_CAP`,
  `Settings.bulk_threshold` (set from the measurement, each with a comment citing it);
  `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1: The measurement.** `measure_check.py` seeds a generated population inside ONE
transaction (`seed_dataset`, no commit), then:
- times `candidate_pairs_for` + `assess` (no persist) for a fixed sample of charts → p50/p95 per chart;
- times one `generate_candidate_pairs` + `assess` over all its pairs → the sweep's cost;
- rolls back.

It prints a small table for N = 2 000, 10 000 and 50 000 records (GenSpec `n_entities` = N/2) and
exits.

```python
# matcher/src/cairn_matcher/eval/measure_check.py
"""Measure the commit-time duplicate check (repair path R4): per-chart latency vs one sweep.

Everything is seeded and measured inside ONE transaction that is rolled back, so the database is
left as found. Usage: CAIRN_TEST_PG=... uv run --extra pipeline python -m
cairn_matcher.eval.measure_check [--sizes 2000 10000 50000] [--sample 50]
"""

import argparse
import os
import random
import statistics
import time

import psycopg

from cairn_matcher.eval.blocking_eval import record_uuid, seed_dataset
from cairn_matcher.eval.dataset import load_dataset
from cairn_matcher.eval.generator import GenSpec, generate_dataset
from cairn_matcher.pipeline.db import generate_candidate_pairs
from cairn_matcher.pipeline.runner import assess
from cairn_matcher.pipeline.targeted import candidate_pairs_for


def _per_chart(conn, charts, cap):
    times = []
    for me in charts:
        t0 = time.perf_counter()
        pairs, _ = candidate_pairs_for(conn, me, max_block_size=cap)
        for lo, hi in pairs:
            assess(conn, lo, hi)
        times.append(time.perf_counter() - t0)
    return times


def main(argv=None) -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--sizes", type=int, nargs="+", default=[2000, 10000, 50000])
    p.add_argument("--sample", type=int, default=50)
    p.add_argument("--cap", type=int, default=1000)
    args = p.parse_args(argv)
    with psycopg.connect(os.environ["CAIRN_TEST_PG"]) as conn:
        for n in args.sizes:
            ds = load_dataset(generate_dataset(GenSpec(seed=11, n_entities=n // 2)))
            seed_dataset(conn, ds)
            ids = [record_uuid(r.record_id) for r in ds.all_records()]
            sample = random.Random(11).sample(ids, min(args.sample, len(ids)))
            per = _per_chart(conn, sample, args.cap)
            t0 = time.perf_counter()
            pairs, _ = generate_candidate_pairs(conn, max_block_size=100)
            for lo, hi in pairs:
                assess(conn, lo, hi)
            sweep_s = time.perf_counter() - t0
            q = statistics.quantiles(per, n=20)
            print(f"N={n:>6}  per-chart p50={statistics.median(per)*1000:7.0f} ms  "
                  f"p95={q[18]*1000:7.0f} ms  sweep={sweep_s:7.1f} s  "
                  f"break-even≈{sweep_s / statistics.median(per):6.0f} charts")
            conn.rollback()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
```

Run it. Then set `Settings.bulk_threshold` to roughly the measured break-even at the largest N, and
`DEFAULT_TARGETED_CAP` so the per-chart p95 at that N stays under ~2 s. Each constant gets a comment
naming the date, machine and figures. Record the table in the design page's as-built note. **If the
per-chart p95 at 10 000 records exceeds 5 s, that is a finding: file it (with #637 as the likely
fix). Never shrink the population to make the number look better.**

- [ ] **Step 2: The runbook** `docs/developers/running-the-duplicate-check.md`:
- what the worker does and never does;
- the role it connects as (`GRANT cairn_agent TO <login role>`);
- `cairn-matcher watch` and its flags;
- `cairn-node duplicate-check [--patient]` and its four states;
- what a restore or rebuild does to the queue (one sweep, then newest-first);
- a **launchd** plist (macOS: `KeepAlive`, `RunAtLoad`, `EnvironmentVariables` for `PG*`) and a
  **systemd** unit (`Restart=always`, `RestartSec=5`, `Environment=PGHOST=…`), both running
  `uv run --project <repo>/matcher --extra pipeline cairn-matcher watch`.

Add it to `mkdocs.yml`'s developers nav. Build the docs with the pinned requirements:
`uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`.

- [ ] **Step 3: File the follow-ons** (`gh issue create`; body says "Found in R4 (#679)", never a
closing keyword):
1. the R4 measurement on the Pi (and `bulk_threshold`/cap there);
2. R5 must show the window's duplicate-check lines (`classify`/`status_line`/`chart_line` are
   ready) — comment on #680 instead of a new issue if that fits better;
3. the worker has no monitoring contract (an exit code or metric for a supervisor) — only if
   the review asks for one.

- [ ] **Step 4: As-built note, HANDOVER, ROADMAP.**
- The as-built note: deviations (or "none"), the measured table, the chosen constants.
- HANDOVER ⇒ NEXT: R5 next. Add an **R4 durable rules** block, each rule with the test that pins it:
  - the hook has no raising path (`the_hook_has_no_raising_path`);
  - `patient_chart` is INSERT-only (`a_losing_reassertion_and_a_later_event_on_the_chart_queue_nothing`);
  - delete only up to the id read (`test_a_change_landing_mid_check_survives_and_is_checked_again`);
  - behind = the NEWEST notice (`behind_is_judged_by_the_newest_notice_strictly_past_the_threshold`);
  - pending until the first worker run (`a_chart_is_pending_until_the_worker_has_run_and_while_it_has_notices`);
  - the worker never links (`test_a_strong_pair_is_proposed_and_never_linked`, `test_watch_once_…`);
  - targeted == sweep (the drift canary).
- ROADMAP: an R4 entry.
- Prune both toward 500 lines without dropping an open issue number: diff the set of `#NNN` before
  and after.
- Verify the plan guard: `cargo test -p cairn-node --test paper_parity_plan_section`.

- [ ] **Step 5: Gates, in CI's order, AFTER the last edit (trap 18).**
  - `cargo fmt --all --check`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`;
  - `cargo deny check`;
  - `cd matcher && uv run ruff check . && CAIRN_ALLOW_DB_SKIP=1 uv run pytest` (the pure lint-test
    job) and `uv run --extra pipeline pytest -rs` with `CAIRN_TEST_PG` set (the DB job);
  - `scripts/run-db-gated-tests.sh` (background, about two hours; do the docs pass while it runs);
  - the `cairn-gui` tree's clippy and tests (it pins cairn-node's lockfile; a root crate change must
    not break it);
  - `python3 scripts/check_closing_keywords.py` on every commit message and on the PR body.
  
  Every gate must pass. Report any failure with its output.

- [ ] **Step 6:** Push; rewrite PR #724's body (what each task built, the measurement, the human
acts owed); take it out of draft. It references #679 without closing it (R5 is #680; #679 closes
when the maintainer says so).

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk's "possible duplicate" tray, filled by someone
  re-checking new registrations against the card index after hours.
- **Steps:** at the desk, paper 0 human acts (the clerk registering a patient does nothing about
  duplicates) → architecture-forced 0 (the check is triggered by the database and run by a worker;
  no dialog, no gate) → UI target 0. `M ≤ N`. Resolving a proposal is R2's gesture and R5's
  surface, unchanged by R4.
- **Time + cognitive load:** zero added at the desk; the registration's latency is unchanged (the
  hook is one append-only insert per changed projection row, measured by Task 1's lock test as
  never waiting). The operator's `cairn-node duplicate-check` line is not a clinical gesture. The
  slice's own number is the registration-to-proposal latency, measured in Task 7 on a generated
  population (budget: p95 ≤ 2 s at 10 000 charts on the development machine). The Pi figure is a
  filed follow-on.
