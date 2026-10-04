//! Repair path R4 (#679, ADR-0076 decision 7): db/056's notice log. A change to any of the
//! matcher's six input projections leaves an append-only NOTICE that the node's matcher worker
//! drains. The hook that writes it cannot fail the clinical write on its own and never waits on
//! the worker (it is a plain insert of a fresh key — nothing to conflict with, no RAISE); its
//! NOTIFY adds only a brief commit-time serialisation.
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

async fn pending(c: &Client, p: Uuid) -> bool {
    c.query_one(
        "SELECT cairn_chart_check_pending($1::text::uuid)",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get::<_, bool>(0)
}

async fn notices(c: &Client, p: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM match_pending WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

async fn assert_name(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: Uuid,
    name: &str,
    wall: i64,
) {
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
    assert!(
        notices(&c, p).await >= 2,
        "patient_chart insert + patient_name insert"
    );
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

/// The six hooks, read from the catalogue (the thing that runs), not from the SQL text. Two
/// silent failures this pins:
/// - the argument must name an existing `uuid` column of that table: a renamed column makes the
///   hook read NULL, and the null guard then queues NOTHING — no error, just no checks;
/// - the whole trigger shape (`tgtype`): ROW, AFTER (never BEFORE — the hook `RETURN NULL`s, and
///   a BEFORE row trigger returning NULL silently DROPS the clinical row), INSERT always, UPDATE
///   exactly where expected (never on patient_chart: db/002 updates it on every clinical event),
///   and enabled (a disabled trigger also queues nothing).
#[tokio::test]
async fn every_input_projection_carries_the_hook_with_the_right_events_and_column() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // pg_trigger.tgtype bits (PostgreSQL's TRIGGER_TYPE_*): ROW 1, BEFORE 2, INSERT 4, DELETE 8,
    // UPDATE 16, TRUNCATE 32, INSTEAD 64. Equality pins every bit, so BEFORE/INSTEAD/DELETE/
    // TRUNCATE must all be clear.
    const ROW: i16 = 1;
    const INSERT: i16 = 4;
    const UPDATE: i16 = 16;
    // (table, id column, fires on UPDATE too?)
    let expected = [
        ("chart_identity_state", "subject", true),
        ("name_repudiation", "subject", true),
        ("patient_chart", "patient_id", false),
        ("patient_demographic", "patient_id", true),
        ("patient_identifier", "patient_id", true),
        ("patient_name", "patient_id", true),
    ];
    // tgargs is NUL-terminated per argument; escape-encoding renders NUL as "\000". The column
    // type is looked up on the trigger's OWN table (tgrelid) by the argument's name; NULL when no
    // live column carries that name.
    let rows = c
        .query(
            "SELECT c.relname::text, a.arg, t.tgtype, \
                    (SELECT ty.typname::text FROM pg_attribute att \
                       JOIN pg_type ty ON ty.oid = att.atttypid \
                      WHERE att.attrelid = t.tgrelid AND att.attname = a.arg \
                        AND att.attnum > 0 AND NOT att.attisdropped), \
                    t.tgenabled::text \
             FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid \
             JOIN pg_proc p ON p.oid = t.tgfoid \
             CROSS JOIN LATERAL (SELECT replace(encode(t.tgargs, 'escape'), '\\000', '') AS arg) a \
             WHERE p.proname = 'cairn_match_enqueue' AND NOT t.tgisinternal ORDER BY 1",
            &[],
        )
        .await
        .unwrap();
    type Shape = (String, String, i16, Option<String>, String);
    let got: Vec<Shape> = rows
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4)))
        .collect();
    let want: Vec<Shape> = expected
        .iter()
        .map(|(t, col, upd)| {
            let tgtype = ROW | INSERT | if *upd { UPDATE } else { 0 };
            // 'O' = enabled in the default ("origin") replication role.
            (
                t.to_string(),
                col.to_string(),
                tgtype,
                Some("uuid".to_string()),
                "O".to_string(),
            )
        })
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
    let n: i64 = c
        .query_one("SELECT count(*) FROM match_pending", &[])
        .await
        .unwrap()
        .get(0);
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
        .execute(
            "DELETE FROM match_pending WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap();
    // A wait would hit lock_timeout and fail loudly — never a sleep.
    c.batch_execute("SET lock_timeout = '2s'").await.unwrap();
    assert_name(&c, &sk, &kid, p, "Mary Smyth", 20).await;
    c.batch_execute("RESET lock_timeout").await.unwrap();
    worker.batch_execute("COMMIT").await.unwrap();
    assert_eq!(
        notices(&c, p).await,
        1,
        "the notice written mid-delete survives it"
    );
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
    // `p` is a chart this node HOLDS: a patient_chart row, whose insert notice the "worker" has
    // already cleared. `stranger` is an id this node has never seen (a mistyped id, or a linked
    // member held only on another node).
    let (p, stranger) = (Uuid::now_v7(), Uuid::now_v7());
    c.execute(
        "INSERT INTO patient_chart (patient_id) VALUES ($1::text::uuid)",
        &[&p.to_string()],
    )
    .await
    .unwrap();
    c.batch_execute("TRUNCATE match_pending").await.unwrap();
    assert!(
        pending(&c, p).await,
        "no worker has ever run: nothing is checked"
    );
    c.execute(
        "INSERT INTO match_worker_state (matcher_version, last_drained_at) VALUES ('v', now())",
        &[],
    )
    .await
    .unwrap();
    assert!(!pending(&c, p).await, "worker ran, no notice");
    assert!(
        pending(&c, stranger).await,
        "a chart this node does not hold was never checked here, whatever the worker did"
    );
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES ($1::text::uuid, 'change')",
        &[&p.to_string()],
    )
    .await
    .unwrap();
    assert!(pending(&c, p).await, "a notice is waiting");
    c.execute(
        "DELETE FROM patient_chart WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap();
}

#[tokio::test]
/// `quiet_age_s` is the seconds since the oldest waiting notice or the worker's last completed
/// work, whichever is later (rulings R13, R17). With no progress stamp it is the OLDEST notice's
/// age; a recent stamp lowers it; nothing waiting makes it NULL.
async fn the_status_reports_waiting_charts_and_the_quiet_age() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let r = c
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .unwrap();
    assert_eq!(r.get::<_, i64>("charts_waiting"), 0);
    assert_eq!(r.get::<_, Option<i64>>("quiet_age_s"), None);
    assert!(!r.get::<_, bool>("worker_seen"));
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    c.batch_execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
        .await
        .unwrap();
    c.execute(
        "INSERT INTO match_pending (patient_id, reason, queued_at) VALUES \
         ($1::text::uuid, 'change', now() - interval '10 minutes'), ($1::text::uuid, 'change', now() - interval '9 minutes'), \
         ($2::text::uuid, 'config', now() - interval '1 minute')",
        &[&a.to_string(), &b.to_string()],
    )
    .await
    .unwrap();
    let r = c
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .unwrap();
    assert_eq!(r.get::<_, i64>("charts_waiting"), 2);
    let age = r.get::<_, Option<i64>>("quiet_age_s").unwrap();
    assert!(
        (599..=610).contains(&age),
        "no progress yet, so the quiet age is the OLDEST notice's (10 minutes), got {age}"
    );
    assert!(r.get::<_, bool>("config_recheck"));
    assert!(r.get::<_, bool>("worker_seen"));
    assert_eq!(r.get::<_, Option<String>>("last_drained_hhmm"), None);
    // The worker makes progress 10 s ago: the queue has been quiet for only that long.
    c.batch_execute(
        "UPDATE match_worker_state SET last_drained_at = now() - interval '10 seconds'",
    )
    .await
    .unwrap();
    let r = c
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .unwrap();
    let age = r.get::<_, Option<i64>>("quiet_age_s").unwrap();
    assert!(
        (9..=20).contains(&age),
        "recent progress lowers the quiet age, got {age}"
    );
}

#[tokio::test]
async fn the_worker_role_can_do_exactly_its_job() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let p = Uuid::now_v7();
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES ($1::text::uuid, 'config')",
        &[&p.to_string()],
    )
    .await
    .expect("cairn_agent queues a config re-check");
    c.execute(
        "DELETE FROM match_pending WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .expect("cairn_agent clears notices");
    c.batch_execute(
        "INSERT INTO match_worker_state (matcher_version) VALUES ('v') \
         ON CONFLICT (singleton) DO UPDATE SET last_drained_at = clock_timestamp()",
    )
    .await
    .expect("cairn_agent stamps the worker state");
    c.batch_execute("RESET ROLE; SET ROLE cairn_node")
        .await
        .unwrap();
    c.query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .expect("the node role reads the status");
    // The per-chart answer reads patient_chart, which the runtime role cannot SELECT; the
    // function is a definer so the node (and R5's banner) can still ask it.
    let p_pending: bool = c
        .query_one(
            "SELECT cairn_chart_check_pending($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .expect("the node role reads one chart's state")
        .get(0);
    assert!(p_pending, "a chart this node does not hold reads pending");
    c.batch_execute("RESET ROLE").await.unwrap();
}

/// A database that loaded db/056's pre-merge shape (`newest_age_s`) is healed on the next connect:
/// `CREATE OR REPLACE` cannot change a function's OUT columns, so the file's guarded DROP removes
/// the old shape first. Without it every connect to such a database would fail.
#[tokio::test]
async fn a_database_with_the_old_status_shape_is_healed_on_connect() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DROP FUNCTION cairn_duplicate_check_status(); \
         CREATE FUNCTION cairn_duplicate_check_status(OUT charts_waiting bigint, \
           OUT newest_age_s bigint, OUT config_recheck boolean, OUT worker_seen boolean, \
           OUT last_drained_hhmm text) LANGUAGE sql AS $$ SELECT 0::bigint, NULL::bigint, \
           false, false, NULL::text $$;",
    )
    .await
    .unwrap();
    let healed = db::connect_and_load_schema(&base)
        .await
        .expect("loading the schema over the old shape must not fail");
    let r = healed
        .query_one("SELECT * FROM cairn_duplicate_check_status()", &[])
        .await
        .unwrap();
    assert!(r.columns().iter().any(|col| col.name() == "quiet_age_s"));
}

/// The status function's `last_drained_hhmm` column, which the test below expects to be set.
async fn last_ran_text(c: &Client) -> String {
    c.query_one(
        "SELECT last_drained_hhmm FROM cairn_duplicate_check_status()",
        &[],
    )
    .await
    .unwrap()
    .get::<_, Option<String>>(0)
    .expect("a worker-state row with last_drained_at set")
}

/// "Behind — last ran 09:41" is only honest when 09:41 was TODAY. A worker that stopped two days
/// ago must read with its date, or the line suggests it ran this morning. The OUT column keeps its
/// name (`last_drained_hhmm`) so db/056's CREATE OR REPLACE needs no DROP; its value is "HH:MM
/// today, else YYYY-MM-DD HH:MM" (database-local date and time).
#[tokio::test]
async fn the_last_ran_time_carries_a_date_unless_it_was_today() {
    let Some((c, _g)) = fresh_db().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    c.batch_execute(
        "INSERT INTO match_worker_state (matcher_version, last_drained_at) \
         VALUES ('v', now() - interval '2 days')",
    )
    .await
    .unwrap();
    let old = last_ran_text(&c).await;
    let want: String = c
        .query_one(
            "SELECT to_char(now() - interval '2 days', 'YYYY-MM-DD HH24:MI')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(old, want, "two days ago renders with its date");
    c.batch_execute("UPDATE match_worker_state SET last_drained_at = now()")
        .await
        .unwrap();
    let today = last_ran_text(&c).await;
    assert_eq!(today.len(), 5, "today renders as HH:MM alone, got {today}");
    assert_eq!(
        today.as_bytes()[2],
        b':',
        "today renders as HH:MM alone, got {today}"
    );
}
