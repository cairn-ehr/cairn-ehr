//! #345 — the §5.3/§5.8 precedence rule: **the first event carrying a `patient_id` must be that
//! chart's registration.** This is what turns the search-before-create funnel from complete into
//! unbypassable; without it a client mints a chart simply by asserting a name on a fresh UUID, and
//! §5.8's obligation to record the search that preceded the create has nothing to attach to.
//!
//! Three properties are pinned here, and the middle one is the easiest to lose:
//!
//! 1. **The strict local door refuses** a first event that is not a registration (db/005 step 8b
//!    over db/001's `cairn_patient_has_events`).
//! 2. **The remote door does NOT** — set-union sync has no ordering, so a peer's clinical event
//!    legitimately arrives before the registration licensing it, and a fail-closed remote door
//!    would freeze the puller's watermark on entirely honest traffic
//!    ([ADR-0061](../../../docs/spec/decisions/0061-registration-is-an-act-that-carries-its-search.md)
//!    decision 3 — the same lesson as ADR-0056, ADR-0058 and issue #268). A future reader who
//!    "makes the doors symmetric" breaks replication, so the asymmetry is tested, not merely
//!    commented.
//! 3. **The legacy `patient.created` is retired** rather than grandfathered, because a permitted
//!    second birth act is exactly the "unless" ADR-0061 decision 2 removed by giving all three
//!    §5.3 classes one event type.

mod common;

use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::{sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_node::db;
use common::{cs, db_msg, setup, submit_registration, submit_signed, EventSpec};
use tokio_postgres::Client;
use uuid::Uuid;

/// Mid-2026 in epoch milliseconds — comfortably inside db/005's 24 h clock-drift ceiling, and the
/// same shape the other suites use so a reader can compare walls across files.
const WALL: i64 = 1_780_000_000_000;

/// Projection tables this suite writes to, beyond `common::setup`'s core list.
const EXTRA_TABLES: [&str; 1] = ["patient_registration"];

/// One ordinary §4.2 date-of-birth assertion — the "something about a patient" a client would use
/// to mint a chart if the funnel could be bypassed. Returned unwrapped so a test can assert either
/// the refusal or the acceptance.
async fn submit_dob(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body("1980-01-01", "day", None, "patient-stated"),
            plaintext_twin: Some(render_dob_twin("1980-01-01", "day", "patient-stated")),
            wall,
        },
    )
    .await
}

/// The same dob assertion as a PEER's event, for the remote door. `node_origin` differs from the
/// local suites' `"n"` so the row is visibly not this node's own authoring.
fn peer_dob(kid: &str, p: Uuid, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: dob_assertion_body("1980-01-01", "day", None, "patient-stated"),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin("1980-01-01", "day", "patient-stated")),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// THE BYPASS, CLOSED. A bare demographic assertion on a fresh `patient_id` is refused — before
/// this rule it silently brought a chart into being with no registration, no search, and therefore
/// no way to diagnose the duplicate that surfaces six months later.
#[tokio::test]
async fn a_first_event_that_is_not_a_registration_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    let err = submit_dob(&c, &sk, &kid, p, WALL)
        .await
        .expect_err("a chart-less first event must be refused");

    assert!(
        db_msg(&err).contains("must be its registration"),
        "the refusal must name the rule so the author knows what to do, not merely fail: {}",
        db_msg(&err)
    );
    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 0, "a refused submit must store nothing");
}

/// The rule gates the ORDER, never the content: the identical assertion succeeds once the chart has
/// been registered. Without this the first test would also pass if the door had simply started
/// refusing demographic events.
#[tokio::test]
async fn the_same_event_succeeds_after_registration() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL - 1).await;
    submit_dob(&c, &sk, &kid, p, WALL)
        .await
        .expect("accepted once the chart exists");
}

/// THE LOAD-BEARING LENIENT CASE. `apply_remote_event` must ADMIT a peer's event for a chart with
/// no registration on file. Set-union sync has no ordering, so this is honest traffic, and a
/// fail-closed remote door would freeze the puller's watermark on it forever.
#[tokio::test]
async fn the_remote_door_admits_an_event_for_an_unregistered_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    let signed = sign(&peer_dob(&kid, p, WALL), &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door must never refuse on the precedence rule");

    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 1, "the out-of-order peer event must be stored");
}

/// A chart that exists only from a peer's out-of-order events stays READABLE and FINDABLE. The
/// lenient remote door means such charts exist by design until the registration syncs; a read path
/// that hid them would turn a sync-ordering artefact into a chart that vanishes from the search a
/// clerk runs before creating a duplicate — the exact failure the funnel exists to prevent.
#[tokio::test]
async fn an_unregistered_chart_is_still_readable() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    let signed = sign(&peer_dob(&kid, p, WALL), &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    let readable: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_demographic d \
               WHERE d.patient_id = $1::text::uuid \
                 AND NOT EXISTS (SELECT 1 FROM patient_registration_current r \
                                  WHERE r.patient_id = d.patient_id)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        readable, 1,
        "an unregistered chart must project and stay queryable — this is also the one-line \
         query the future 'chart with no registration on file' flag (#354) will read"
    );
}

/// A local write to that same peer-seeded chart is NOT refused afterwards. The rule is
/// self-satisfying: once any event has landed, the next one is no longer a FIRST event — so the
/// lenient remote admission never costs a later local refusal, and a partitioned node can keep
/// documenting care on a chart whose registration is still in flight.
#[tokio::test]
async fn a_local_write_to_a_peer_seeded_chart_is_not_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    let signed = sign(&peer_dob(&kid, p, WALL), &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    submit_dob(&c, &sk, &kid, p, WALL + 1)
        .await
        .expect("the chart already has events, so this is not a first event");
}

/// A SECOND registration for a chart that already has events is accepted. Registration is exempt
/// from the rule with no "unless" — and a duplicate registration is EVIDENCE (someone registered
/// the same patient twice, or two nodes minted the same chart), which db/045's retained set keeps
/// precisely so an investigation can see it.
#[tokio::test]
async fn a_later_registration_is_never_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL).await;
    submit_registration(&c, &sk, &kid, p, WALL + 1).await;

    let retained: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_registration WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(retained, 2, "both registrations are retained as evidence");
}

/// Registration takes over the chart-birth projection the retired `patient.created` used to own, so
/// every chart-shaped read (`person_chart` and the trust reads composed on it, the candidate list's
/// last-activity column) sees a chart from its birth rather than from its first demographic write.
#[tokio::test]
async fn registration_materialises_the_chart_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL).await;

    let row = c
        .query_one(
            "SELECT name, note_count, last_activity IS NOT NULL \
               FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .expect("a registered chart has a patient_chart row");
    assert_eq!(
        row.get::<_, Option<String>>(0),
        None,
        "a registration asserts NO demographics — the name comes from db/010-014, and writing one \
         here would invent a fact the act never carried"
    );
    assert_eq!(row.get::<_, i32>(1), 0, "a fresh chart has no notes");
    assert!(
        row.get::<_, bool>(2),
        "the birth act is activity: a chart registered today must not read as 'nothing ever \
         happened here'"
    );
}

/// `patient.created` is RETIRED, not grandfathered. Retirement is expressed as declassification, so
/// the door's own fail-closed arm (db/005 step 3) is what refuses it — and the projection
/// registration must already be gone, because deleting the class row first would leave the
/// registered-but-unclassified state db/005's registry trigger exists to make unreachable.
#[tokio::test]
async fn the_legacy_patient_created_type_is_retired() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &EXTRA_TABLES).await;
    let p = Uuid::now_v7();

    // Registered first, so the refusal below cannot be the precedence rule wearing another hat.
    submit_registration(&c, &sk, &kid, p, WALL).await;
    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "patient.created",
            schema_version: "patient/1",
            payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
            plaintext_twin: None,
            wall: WALL + 1,
        },
    )
    .await
    .expect_err("the retired type must be refused even on an existing chart");
    assert!(
        db_msg(&err).contains("unknown event_type"),
        "declassification is the retirement mechanism, so the fail-closed arm reports it: {}",
        db_msg(&err)
    );

    for (table, column) in [
        ("cairn_projection_apply", "event_type"),
        ("event_type_class", "event_type"),
    ] {
        let rows: i64 = c
            .query_one(
                &format!("SELECT count(*) FROM {table} WHERE {column} = 'patient.created'"),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            rows, 0,
            "db/047 must leave no {table} row for the retired type"
        );
    }
}
