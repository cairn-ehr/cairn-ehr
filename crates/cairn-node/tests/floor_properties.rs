//! Property tests against the LIVE in-DB floor (#212): arbitrary attacker-shaped
//! JSONB through the twin dispatcher. DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard (same idiom as twin_registry.rs).
//!
//! WHY: the floor fns take attacker-controlled JSONB (Spike 0002's hostile enrolled
//! writer talks raw SQL), and every existing test feeds them WELL-FORMED bodies with
//! one field broken at a time. Generative junk probes the space between the examples.
//! The invariant pinned here is the ADR-0039 hard-require in its attack direction:
//! for a twin-REQUIRED registered type, a body without a non-empty authored twin must
//! ALWAYS raise — legibly — no matter what else the body contains, and the backend
//! must survive to serve the next query (a floor that can be crashed is a floor that
//! can be bypassed by retry-into-fallback).
//!
//! Uses proptest's TestRunner API (not the macro) so the DB gate + one shared
//! connection + one tokio runtime wrap the whole run.

use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestError, TestRunner};

use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Arbitrary bounded JSON — the junk an enrolled-but-hostile client can hand the
/// dispatcher. Depth/size bounded so shrinking stays fast; strings include the
/// key names the medication check looks for, so some junk lands NEAR-valid (the
/// interesting region — total garbage exercises only the first refusal).
fn arb_body() -> impl Strategy<Value = serde_json::Value> {
    let key = prop_oneof![
        Just("schema_version".to_string()),
        Just("patient_id".to_string()),
        Just("medication_id".to_string()),
        Just("payload".to_string()),
        Just("term".to_string()),
        ".{0,8}",
    ];
    let leaf = prop_oneof![
        Just(serde_json::Value::Null),
        any::<bool>().prop_map(serde_json::Value::from),
        any::<i64>().prop_map(serde_json::Value::from),
        ".{0,16}".prop_map(serde_json::Value::from),
        Just(serde_json::Value::from("clinical.medication/1")),
        Just(serde_json::Value::from(
            "00000000-0000-0000-0000-000000000001"
        )),
    ];
    let node = leaf.prop_recursive(3, 32, 4, move |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(serde_json::Value::from),
            prop::collection::btree_map(key.clone(), inner, 0..5)
                .prop_map(|m| serde_json::Value::Object(m.into_iter().collect())),
        ]
    });
    // Top level is always an object (the dispatcher's input shape) WITHOUT a
    // plaintext_twin — the twin-absent attack direction this property pins.
    prop::collection::btree_map(
        prop_oneof![
            Just("schema_version".to_string()),
            Just("patient_id".to_string()),
            Just("payload".to_string()),
            ".{0,8}",
        ],
        node,
        0..5,
    )
    .prop_map(|m| {
        let mut o: serde_json::Map<String, serde_json::Value> = m.into_iter().collect();
        o.remove("plaintext_twin"); // belt-and-braces: the strategy never generates it either
        serde_json::Value::Object(o)
    })
}

#[test]
fn twinless_bodies_always_raise_legibly_and_never_kill_the_backend() {
    let Some(base) = cs() else { return };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (_guard, client) = rt.block_on(async {
        let guard = db::test_serial_guard(&base).await.unwrap();
        let client = db::connect_and_load_schema(&base).await.unwrap();
        (guard, client)
    });

    let mut runner = TestRunner::new(Config {
        cases: 48, // bounded: each case is a live DB round-trip
        ..Config::default()
    });

    let result = runner.run(&arb_body(), |body| {
        rt.block_on(async {
            let body_text = body.to_string();
            // $1::text::jsonb — a bare $1::jsonb cast false-greens under
            // tokio-postgres (see twin_registry.rs).
            let res = client
                .query_one(
                    "SELECT cairn_event_twin('clinical.medication.asserted', $1::text::jsonb)",
                    &[&body_text],
                )
                .await;
            let err = match res {
                Ok(_) => {
                    return Err(TestCaseError::fail(format!(
                        "twin-less body was ACCEPTED by the dispatcher: {body_text}"
                    )))
                }
                Err(e) => e,
            };
            // Legibility: a real DB-side RAISE with a non-empty message — not a
            // broken connection, not an opaque protocol error.
            let msg = err
                .as_db_error()
                .map(|d| d.message().to_string())
                .unwrap_or_default();
            if msg.is_empty() {
                return Err(TestCaseError::fail(format!(
                    "refusal was not a legible DB error (connection-level failure?): {err} — body: {body_text}"
                )));
            }
            // Survivability: the same connection serves the next query.
            client
                .query_one("SELECT 1::int4", &[])
                .await
                .map_err(|e| {
                    TestCaseError::fail(format!("backend did not survive the refusal: {e}"))
                })?;
            Ok(())
        })
    });

    if let Err(TestError::Fail(reason, value)) = result {
        panic!("floor property failed: {reason}\nminimal failing body: {value}");
    }
    result.unwrap();
}
