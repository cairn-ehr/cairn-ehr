//! Sign-in slice 3c-3a: the six supervision ops (and the `related_actor` column the link ops use)
//! survive the local-state export and `restore_actor_registry` (db/052) field for field — and,
//! since #1014 (ADR-0107), each supervision row's HLC stamp with them, the restore moving the
//! node's HLC forward past every stamp it brings back.

use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::localstate::{actor_registry_row_from_cbor, actor_registry_rows_to_json};
use cairn_node::localstate_read::read_local_state;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// How far ahead of the wall clock the fixture's HLC is ratcheted.
const AHEAD_MS: i64 = 7_200_000;

fn key() -> String {
    generate_key().unwrap().1
}

/// Two enrolled humans plus one row of each supervision op on the first (the supervisee).
async fn build_registry(c: &tokio_postgres::Client) {
    c.batch_execute(
        "DELETE FROM local_node;
         TRUNCATE event_log, actor_event, patient_chart CASCADE;",
    )
    .await
    .unwrap();
    let mut ids: Vec<Vec<u8>> = Vec::new();
    for handle in ["A", "B"] {
        let k = key();
        let pinned = format!("{{\"role\":\"clinician\",\"handle\":\"{handle}\"}}");
        ids.push(
            c.query_one(
                "SELECT enroll_actor('human', $1::text::jsonb, $2)",
                &[&pinned, &k],
            )
            .await
            .unwrap()
            .get(0),
        );
    }
    let (sv, sup) = (&ids[0], &ids[1]);
    // A peer ratcheted the HLC 2 h ahead: the stamps are then well ahead of any wall clock, which
    // is what makes the restore's HLC merge observable below.
    c.batch_execute(&format!(
        "UPDATE hlc_state SET hlc_wall = (extract(epoch FROM clock_timestamp()) * 1000)::bigint \
         + {AHEAD_MS}, hlc_counter = 0"
    ))
    .await
    .unwrap();
    for op in ["link-supervisor", "unlink-supervisor"] {
        c.execute(
            "SELECT cairn_actor_append($1, $2, NULL, 'console', NULL, $3)",
            &[sv, &op, sup],
        )
        .await
        .unwrap_or_else(|e| panic!("append {op}: {e:?}"));
    }
    for op in [
        "require-cosign",
        "lift-cosign",
        "grant-assigner",
        "withdraw-assigner",
    ] {
        c.execute(
            "SELECT cairn_actor_append($1, $2, NULL, 'console', NULL)",
            &[sv, &op],
        )
        .await
        .unwrap_or_else(|e| panic!("append {op}: {e:?}"));
    }
}

async fn dump(c: &tokio_postgres::Client) -> Vec<String> {
    c.query(
        "SELECT actor_event_id::text || '|' || encode(actor_id,'hex') || '|' || op || '|' || \
                COALESCE(kind,'~') || '|' || COALESCE(pinned::text,'~') || '|' || \
                COALESCE(signing_key_id,'~') || '|' || COALESCE(encode(superseded_by,'hex'),'~') \
                || '|' || COALESCE(approved_by,'~') || '|' || COALESCE(encode(approval,'hex'),'~') \
                || '|' || COALESCE(encode(related_actor,'hex'),'~') \
                || '|' || recorded_at::text || '|' || COALESCE(hlc_wall::text,'~') || '.' \
                || COALESCE(hlc_counter::text,'~') FROM actor_event ORDER BY seq",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

async fn export_json(c: &tokio_postgres::Client) -> String {
    let ls = read_local_state(c, None).await.unwrap();
    let rows: Vec<_> = ls
        .actor_registry()
        .iter()
        .map(|b| actor_registry_row_from_cbor(b).unwrap())
        .collect();
    actor_registry_rows_to_json(&rows)
}

async fn clear(c: &tokio_postgres::Client) {
    c.batch_execute(
        "DELETE FROM local_node;
         ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update;
         DELETE FROM actor_event;
         ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;
         UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0;",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn the_supervision_ops_survive_a_restore() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    build_registry(&c).await;
    let before = dump(&c).await;
    assert_eq!(
        before.len(),
        8,
        "2 enrolls + 6 supervision rows: {before:?}"
    );
    assert_eq!(
        before.iter().filter(|r| r.contains("-supervisor|")).count(),
        2,
        "the link rows must be in the fixture"
    );
    let json = export_json(&c).await;
    clear(&c).await;
    let n: i32 = c
        .query_one("SELECT restore_actor_registry($1::text::jsonb)", &[&json])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n as usize, before.len());
    assert_eq!(
        dump(&c).await,
        before,
        "restored rows differ field for field, related_actor and the HLC stamp included"
    );
    assert_eq!(
        before.iter().filter(|r| r.ends_with("~.~")).count(),
        2,
        "the six supervision rows carry a stamp, the two enrolls none: {before:?}"
    );
    // The restore moved the HLC past every stamp it brought back: otherwise a part signed now
    // would sort BEFORE a restored require-cosign and escape it.
    let (max_wall, max_counter, wall, counter): (i64, i32, i64, i32) = {
        let r = c
            .query_one(
                "SELECT (SELECT max(hlc_wall) FROM actor_event), \
                        (SELECT max(hlc_counter) FROM actor_event WHERE hlc_wall = \
                            (SELECT max(hlc_wall) FROM actor_event)), \
                        s.hlc_wall, s.hlc_counter FROM hlc_state s",
                &[],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1), r.get(2), r.get(3))
    };
    assert!(
        (wall, counter) >= (max_wall, max_counter),
        "hlc_state ({wall}, {counter}) must be at or past the newest restored stamp \
         ({max_wall}, {max_counter})"
    );
    clear(&c).await;
}

#[tokio::test]
async fn a_tampered_hlc_stamp_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    build_registry(&c).await;
    let json = export_json(&c).await;
    c.batch_execute("DELETE FROM local_node").await.unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = v
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r.get("hlc_wall").is_some())
        .expect("fixture has a stamped row");
    row["hlc_wall"] = serde_json::json!(1);
    let msg = refusal(&c, &v).await;
    assert!(
        msg.contains("hlc_wall"),
        "refusal must name hlc_wall: {msg}"
    );

    // A wall with no counter is refused by shape, naming both fields, before any row is read.
    clear(&c).await;
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = v
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r.get("hlc_wall").is_some())
        .unwrap();
    row.as_object_mut().unwrap().remove("hlc_counter");
    let msg = refusal(&c, &v).await;
    assert!(
        msg.contains("hlc_wall") && msg.contains("hlc_counter"),
        "refusal must name the pair: {msg}"
    );
    // A JSON null in one half is that half absent: the same refusal, never a bare CHECK error.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = stamped(&mut v);
    row["hlc_wall"] = serde_json::Value::Null;
    let msg = refusal(&c, &v).await;
    assert!(msg.contains("both or neither"), "{msg}");
    // A number carried as a JSON string is refused too, as `seq` is (the export writes numbers).
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = stamped(&mut v);
    row["hlc_counter"] = serde_json::json!(row["hlc_counter"].to_string());
    let msg = refusal(&c, &v).await;
    assert!(msg.contains("both or neither"), "{msg}");
    // A stamp past now + the 24 h drift ceiling would park the HLC in the future: refused.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = stamped(&mut v);
    let far = row["hlc_wall"].as_i64().unwrap() + 2 * 86_400_000;
    row["hlc_wall"] = serde_json::json!(far);
    let msg = refusal(&c, &v).await;
    assert!(msg.contains("drift ceiling"), "{msg}");
    clear(&c).await;
}

/// The first row of `rows` that carries an HLC stamp.
fn stamped(rows: &mut serde_json::Value) -> &mut serde_json::Value {
    rows.as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r.get("hlc_wall").is_some())
        .expect("fixture has a stamped row")
}

/// The message of `restore_actor_registry`'s refusal of `rows` (it must refuse).
async fn refusal(c: &tokio_postgres::Client, rows: &serde_json::Value) -> String {
    let err = c
        .query_one(
            "SELECT restore_actor_registry($1::text::jsonb)",
            &[&rows.to_string()],
        )
        .await
        .expect_err("the restore must refuse");
    err.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_tampered_related_actor_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    build_registry(&c).await;
    let json = export_json(&c).await;
    // Fence 1 needs an un-enrolled node; the rows stay present, so every one is a "present row".
    c.batch_execute("DELETE FROM local_node").await.unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = v
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r.get("related_actor").is_some())
        .expect("fixture has a link row");
    row["related_actor"] = serde_json::json!("00ff00ff");
    let err = c
        .query_one(
            "SELECT restore_actor_registry($1::text::jsonb)",
            &[&v.to_string()],
        )
        .await
        .expect_err("a tampered present row must be refused");
    let msg = err
        .as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("related_actor"),
        "refusal must name related_actor: {msg}"
    );
    clear(&c).await;
}
