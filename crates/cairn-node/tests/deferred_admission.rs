//! ADR-0056 decisions 1 + 4 (issues #265/#266): the clinical remote door admits an event
//! whose `event_type` it cannot classify — stored verbatim, no projection rows, no power —
//! and power is granted only after the classification-gated floor checks are re-run.
//!
//! Before this slice the door RAISED on an unclassifiable type, so the event was never
//! stored at all. A phone-tier node carrying a chart between two upgraded facilities (the
//! §6.1 sneakernet path, the case Cairn exists for) acquired NOTHING past the first
//! unknown-type event — not unrendered, absent. These tests pin the contract that replaces
//! it: custody is total, power is deferred.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21) so the t_effective ≤ t_recorded
/// ceiling compares against a sane "recorded" instant rather than 1970.
const WALL_2026: i64 = 1_782_000_000_000;

/// A type no migration classifies — a plausible FUTURE slice's event, which is exactly the
/// case ADR-0056 is about: an upgraded peer authors it, this node has no code for it.
const UNKNOWN_TYPE: &str = "clinical.medication.recall";

/// Truncate the clinical tables and enroll one agent signer + one human attester.
/// `TRUNCATE event_log ... CASCADE` clears `event_deferred` through its FK.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag, \
         t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    // Keys are DERIVED at runtime, never byte literals (house rule 6 / issue #146).
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build a signed event of an arbitrary type, "arriving from a peer".
fn peer_event(kid: &str, patient: Uuid, ty: &str, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: ty.into(),
        schema_version: "future/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "upgraded-peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"reason": "batch recall"}),
        attachments: vec![],
        // No authored twin: the mechanical skeleton must carry it (ADR-0039), which is the
        // rendering half of "coarseness varies; existence never disappears".
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention, see identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A projection may only be registered for a CLASSIFIED type.
///
/// This guard is load-bearing, not hygiene. The deferred marker row is written AFTER the
/// `event_log` INSERT, but the AFTER-INSERT projection dispatcher fires DURING it. So a type
/// registered for projection without an `event_type_class` row would be projected at
/// admission — granting exactly the power the marker exists to withhold. Making that state
/// unreachable at migration time is also one of the two legs (the other is
/// `cairn_replay_eligible`) holding up the guarantee that no projection apply fn ever sees a
/// deferred row, which is what lets db/018 and db/034 keep trusting `event_log.attester_key`.
#[tokio::test]
async fn projection_registration_requires_a_classified_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    // `patient_chart_apply` exists with the right signature and `patient_chart` is a real
    // relation, so the two pre-existing registration guards pass — only the classification
    // one can fire, which is what makes this test discriminate.
    let err = c
        .execute(
            "INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables) \
             VALUES ('unclassified.for.test', 'patient_chart_apply', ARRAY['patient_chart'])",
            &[],
        )
        .await
        .expect_err("registering a projection for an unclassified type must fail closed");
    assert!(
        db_msg(&err).contains("not classified in event_type_class"),
        "got: {}",
        db_msg(&err)
    );
}

/// The marker table is the EXPLICIT deferred state ADR-0056's corollary demands ("never
/// inferred from a null classification lookup falling through the gates by three-valued
/// logic"). Pin its shape so a later edit cannot quietly drop the column that records WHY a
/// re-adjudication failed — the whole of decision 4's "flagged legibly".
#[tokio::test]
async fn event_deferred_table_has_the_designed_shape() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    let cols: Vec<String> = c
        .query(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_name = 'event_deferred' ORDER BY column_name",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(
        cols,
        vec![
            "adjudication_error",
            "admitted_at",
            "event_id",
            "event_type",
            "last_attempt_at"
        ],
        "event_deferred shape drifted from the design"
    );
}

/// ADR-0056 decision 1: an unclassifiable type is ADMITTED — stored verbatim, no projection
/// rows, no power — and marked deferred. This is the §6.1 sneakernet case: a carrier node
/// must stop being a propagation barrier.
#[tokio::test]
async fn unknown_type_is_admitted_and_marked_deferred() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .expect("an unclassifiable type must be ADMITTED, not refused (ADR-0056 decision 1)");

    // Stored verbatim — the whole point. Before this slice the row did not exist at all.
    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 1, "the event must be in event_log verbatim");

    // Marked deferred — EXPLICITLY, not inferred from the absent classification row.
    let marked: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        marked, 1,
        "an admitted-uninterpreted event must carry a marker"
    );

    // Legible without its schema: the skeleton twin renders any type, registered or not.
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !twin.trim().is_empty(),
        "the skeleton twin must render an unregistered type (ADR-0039 honest degradation)"
    );
}

/// ADR-0056 decision 2: the STRICT door keeps failing closed. A node may CARRY a type it has
/// no code for; it may never AUTHOR one. This is the regression pin for the slice's whole
/// risk — over-relaxing the floor while relaxing the door.
#[tokio::test]
async fn strict_door_still_refuses_an_unclassifiable_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .expect_err("submit_event must still refuse a type this node cannot classify");
    assert!(
        db_msg(&err).contains("fail closed") || db_msg(&err).contains("unknown event_type"),
        "got: {}",
        db_msg(&err)
    );
    let marked: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 0, "a refused local author must leave no marker");
}
