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

async fn pending(c: &Client, p: Uuid) -> bool {
    c.query_one("SELECT cairn_chart_check_pending($1::text::uuid)", &[&p.to_string()])
        .await
        .unwrap()
        .get::<_, bool>(0)
}

async fn notices(c: &Client, p: Uuid) -> i64 {
    c.query_one("SELECT count(*) FROM match_pending WHERE patient_id = $1::text::uuid", &[&p.to_string()])
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
        .execute("DELETE FROM match_pending WHERE patient_id = $1::text::uuid", &[&p.to_string()])
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
    assert!(pending(&c, p).await, "no worker has ever run: nothing is checked");
    c.execute(
        "INSERT INTO match_worker_state (matcher_version, last_drained_at) VALUES ('v', now())",
        &[],
    )
    .await
    .unwrap();
    assert!(!pending(&c, p).await, "worker ran, no notice");
    c.execute(
        "INSERT INTO match_pending (patient_id, reason) VALUES ($1::text::uuid, 'change')",
        &[&p.to_string()],
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
         ($1::text::uuid, 'change', now() - interval '10 minutes'), ($1::text::uuid, 'change', now() - interval '9 minutes'), \
         ($2::text::uuid, 'config', now() - interval '1 minute')",
        &[&a.to_string(), &b.to_string()],
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
    c.execute("INSERT INTO match_pending (patient_id, reason) VALUES ($1::text::uuid, 'config')", &[&p.to_string()])
        .await
        .expect("cairn_agent queues a config re-check");
    c.execute("DELETE FROM match_pending WHERE patient_id = $1::text::uuid", &[&p.to_string()])
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
