//! Sign-in slice 2: the registry's new ops and `approved_by`/`approval` columns survive the
//! local-state export and `restore_actor_registry` (db/052) field for field.

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

/// Registry with enroll (approved), rotate-key, confirm, repudiate, grant-enroller rows.
async fn build_registry(c: &tokio_postgres::Client) {
    c.batch_execute(
        "DELETE FROM local_node;
         TRUNCATE event_log, actor_event, patient_chart CASCADE;",
    )
    .await
    .unwrap();
    let (su, k1, k2) = (key(), key(), key());
    let token: Vec<u8> = (0..16u8).map(|i| i.wrapping_mul(7)).collect();
    let aid: Vec<u8> = c
        .query_one(
            "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"A\"}'::jsonb, $1, $2, $3)",
            &[&k1, &su, &token],
        )
        .await
        .unwrap()
        .get(0);
    let ops: [(&str, Option<&String>, Option<&String>, Option<&Vec<u8>>); 4] = [
        ("rotate-key", Some(&k2), None, None),
        ("confirm", Some(&k2), Some(&su), Some(&token)),
        ("repudiate", Some(&k1), Some(&su), None),
        ("grant-enroller", None, Some(&su), None),
    ];
    for (op, kid, by, appr) in ops {
        c.execute(
            "SELECT cairn_actor_append($1, $2, $3, $4, $5)",
            &[&aid, &op, &kid, &by, &appr],
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
async fn the_new_ops_survive_a_restore() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    build_registry(&c).await;
    let before = dump(&c).await;
    assert!(before.len() >= 5, "fixture too small: {before:?}");
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
        "restored rows differ field for field"
    );
    clear(&c).await;
}

#[tokio::test]
async fn a_present_row_with_a_different_approval_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    build_registry(&c).await;
    let json = export_json(&c).await;
    c.batch_execute("DELETE FROM local_node").await.unwrap();
    // Tamper: the enroll row's approval token changes; the present row must be refused.
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let row = v
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r.get("approval").is_some())
        .expect("fixture has an approved row");
    row["approval"] = serde_json::json!("00ff00ff");
    let err = c
        .query_one(
            "SELECT restore_actor_registry($1::text::jsonb)",
            &[&v.to_string()],
        )
        .await
        .expect_err("a different approval must be refused");
    let msg = err
        .as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("approval"),
        "refusal must name the field: {msg}"
    );
    clear(&c).await;
}
