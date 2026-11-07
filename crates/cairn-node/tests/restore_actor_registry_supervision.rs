//! Sign-in slice 3c-3a: the six supervision ops (and the `related_actor` column the link ops use)
//! survive the local-state export and `restore_actor_registry` (db/052) field for field.

use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::localstate::{actor_registry_row_from_cbor, actor_registry_rows_to_json};
use cairn_node::localstate_read::read_local_state;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

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
                || '|' || recorded_at::text FROM actor_event ORDER BY seq",
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
         ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;",
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
        "restored rows differ field for field, related_actor included"
    );
    clear(&c).await;
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
