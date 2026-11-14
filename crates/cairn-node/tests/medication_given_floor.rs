//! Charting slice 2, Task 2 — a stat ORDER and the dose GIVEN (db/077's `stat` key, db/082 §1).
//!
//! A stat order is an ordinary `clinical.medication.asserted` line carrying the top-level key
//! `"stat": true` (absent otherwise, never `false`). The dose being given is its own event type,
//! `clinical.medication-given.asserted`: its envelope `t_effective` is the time given, and its
//! payload's `completes` names the stop (cessation) that the give completes, so a stat line reads
//! stopped once given. This file pins both floors (ADR-0056: structure at BOTH doors, references
//! at the AUTHORING door only, so a peer's given that arrives before its line never wedges sync),
//! the `medication_given` projection, its shred, its reprojection and its thread.
//!
//! The struck given (the retraction type) is Task 3 and not here.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` (inside `rig!`).
use cairn_event::medication::GIVEN_SCHEMA;
use cairn_node::db;
use cairn_node::medication::{build_cease_body, read_charting, CeaseMedicationInput};
use cairn_node::shred::shred_event;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[path = "common/given_kit.rs"]
mod given_kit;
use given_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;

/// Call `check` (a floor fn taking `(text, jsonb)` or `(jsonb)`) at one door, rolled back.
async fn floor_at(c: &mut Client, sql: &str, args: &Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx.execute(sql, &[&args.to_string()]).await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

const GIVEN_FLOOR: &str =
    "SELECT cairn_check_medication_given('clinical.medication-given.asserted', $1::text::jsonb)";
const CHARTING_FLOOR: &str = "SELECT cairn_check_medication_charting($1::text::jsonb)";
const N: &str = "SELECT count(*) FROM medication_given WHERE event_id = $1::text::uuid";

#[tokio::test]
async fn stat_true_projects_and_absent_reads_false() {
    rig!(c, sk, kid);
    let (p, stat) = chart(&c, &sk, &kid, true).await;
    let plain = line(&c, &sk, &kid, p, false).await;
    let rows = read_charting(&c, &[stat, plain]).await.unwrap();
    // A stat order with no other charting key still has a row: `stat` alone counts.
    assert!(rows[&stat].stat, "a stat order reads stat");
    assert!(!rows[&plain].stat, "a line without the key reads not-stat");
}

#[tokio::test]
async fn stat_false_or_a_string_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    for remote in [false, true] {
        for bad in [json!(false), json!("true"), json!(1), json!({})] {
            let e = floor_at(&mut c, CHARTING_FLOOR, &json!({ "stat": bad }), remote)
                .await
                .expect_err(&format!("remote={remote} must refuse stat={bad}"));
            assert!(e.contains("stat"), "{e}");
        }
        for ok in [json!(true), Value::Null] {
            floor_at(&mut c, CHARTING_FLOOR, &json!({ "stat": ok }), remote)
                .await
                .expect("true, or null (= absent), is admitted");
        }
    }
}

#[tokio::test]
async fn a_set_aside_carrying_stat_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    for remote in [false, true] {
        for v in [json!(true), Value::Null] {
            let b = version_body(&c, &kid, p, Uuid::now_v7(), med, |o| o["stat"] = v.clone()).await;
            let e = floor(&mut c, &floor_body(b.payload), remote)
                .await
                .expect_err("a set-aside version was never ordered stat");
            assert!(e.contains("stat"), "{e}");
        }
    }
}

#[tokio::test]
async fn a_given_completing_its_stat_lines_stop_projects() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (stop, g) = give_stat(&c, &sk, &kid, p, med).await.expect("admitted");
    let r = c
        .query_one(
            "SELECT patient_id::text, medication_id::text, completes::text, \
                    given_at = (SELECT t_effective FROM event_log WHERE event_id = g.event_id), \
                    hlc_wall > 0 \
               FROM medication_given g WHERE event_id = $1::text::uuid",
            &[&g.to_string()],
        )
        .await
        .expect("one projected row");
    assert_eq!(r.get::<_, String>(0), p.to_string());
    assert_eq!(r.get::<_, String>(1), med.to_string());
    assert_eq!(r.get::<_, Option<String>>(2), Some(stop.to_string()));
    assert!(
        r.get::<_, bool>(3),
        "given_at is the envelope's t_effective"
    );
    assert!(r.get::<_, bool>(4));
    let stopped = "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid";
    assert_eq!(
        count(&c, stopped, med).await,
        1,
        "the stat line reads stopped"
    );
}

#[tokio::test]
async fn a_given_with_no_completes_projects_and_stops_nothing() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let g = submit(&c, &sk, given(&c, &kid, p, med, None).await)
        .await
        .expect("an ordinary give is admitted");
    let completes: Option<String> = c
        .query_one(
            "SELECT completes::text FROM medication_given WHERE event_id = $1::text::uuid",
            &[&g.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(completes, None);
    let stopped = "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid";
    assert_eq!(
        count(&c, stopped, med).await,
        0,
        "the line is still current"
    );
}

#[tokio::test]
async fn a_given_with_no_t_effective_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let mut b = given(&c, &kid, p, med, None).await;
    b.t_effective = None;
    let e = submit(&c, &sk, b).await.expect_err("no time given");
    assert!(e.contains("t_effective"), "{e}");
}

#[tokio::test]
async fn completes_naming_another_threads_or_no_stop_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let other = line(&c, &sk, &kid, p, true).await;
    // Another thread's stop, on the same chart.
    let theirs = submit(&c, &sk, stop_body(&c, kid.as_str(), p, other).await)
        .await
        .unwrap();
    // A line's own assertion: an event of this chart that is not a stop at all.
    let assertion: String = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND patient_id = $1::text::uuid LIMIT 1",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    for completes in [theirs, Uuid::now_v7(), assertion.parse().unwrap()] {
        let e = submit(&c, &sk, given(&c, &kid, p, med, Some(completes)).await)
            .await
            .expect_err(&format!("completes {completes} is not this line's stop"));
        assert!(e.contains("completes"), "{e}");
    }
}

#[tokio::test]
async fn completes_on_a_line_that_is_not_a_stat_order_is_refused() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let e = give_stat(&c, &sk, &kid, p, med)
        .await
        .expect_err("not a stat order");
    assert!(e.contains("stat order"), "{e}");
}

/// A stop signed in an EARLIER act is a real decision ("patient refused"). A given naming it
/// would let a later strike of that given void the real stop, so `completes` may only name the
/// stop signed in the same transaction as the given.
#[tokio::test]
async fn completes_naming_a_stop_from_an_earlier_transaction_is_refused() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let refused = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some("patient refused"),
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let real = build_cease_body(Uuid::now_v7(), med, p, &refused, &kid, hlc);
    let real = submit(&c, &sk, real).await.expect("the doctor's real stop");
    let e = submit(&c, &sk, given(&c, &kid, p, med, Some(real)).await)
        .await
        .expect_err("a given may not claim an earlier, real stop");
    assert!(e.contains("signed in this transaction"), "{e}");
    let given = "SELECT count(*) FROM medication_given WHERE medication_id = $1::text::uuid";
    assert_eq!(count(&c, given, med).await, 0, "nothing was given");
}

#[tokio::test]
async fn another_charts_line_is_refused() {
    rig!(c, sk, kid);
    let (_p, med) = chart(&c, &sk, &kid, true).await;
    let (other, _m) = chart(&c, &sk, &kid, false).await;
    let e = submit(&c, &sk, given(&c, &kid, other, med, None).await)
        .await
        .expect_err("another chart's line");
    assert!(e.contains("not a medication on this chart"), "{e}");
    let e = submit(&c, &sk, given(&c, &kid, other, Uuid::now_v7(), None).await)
        .await
        .expect_err("an unknown line");
    assert!(e.contains("not a medication on this chart"), "{e}");
    // A wrong schema tag is refused at the authoring door too.
    let mut b = given(&c, &kid, other, med, None).await;
    b.schema_version = "clinical.medication-given/2".into();
    let e = submit(&c, &sk, b).await.expect_err("tag");
    assert!(e.contains("schema_version"), "{e}");
}

#[tokio::test]
async fn the_remote_door_admits_an_orphan_given() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = given(&c, &kid, p, Uuid::now_v7(), Some(Uuid::now_v7())).await;
    body.hlc = peer_hlc(wall);
    let body = effective_at_wall(&c, body).await;
    apply_remote(&c, &sk, &body)
        .await
        .map_err(|e| db_msg(&e))
        .expect("a peer's given whose line has not synced");
    let id: Uuid = body.event_id.parse().unwrap();
    assert_eq!(count(&c, N, id).await, 1, "held");
    // Shown nowhere: no line joins it on thread AND chart.
    let shown: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_given g JOIN medication_statement m \
               ON m.medication_id = g.medication_id AND m.patient_id = g.patient_id \
              WHERE g.event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(shown, 0);
}

#[tokio::test]
async fn a_safety_claim_or_a_charting_key_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    let ok = json!({"medication_id": Uuid::now_v7().to_string(), "completes": null});
    let body = |payload: Value| {
        json!({"schema_version": GIVEN_SCHEMA, "patient_id": Uuid::now_v7().to_string(),
               "t_effective": "2026-11-14T10:00:00Z", "payload": payload})
    };
    let with = |k: &str, v: Value| {
        let mut o = ok.clone();
        o[k] = v;
        o
    };
    for remote in [false, true] {
        let w = format!("remote={remote}");
        for (payload, frag) in [
            (json!("nope"), "payload"),
            (with("medication_id", json!("nope")), "medication_id"),
            (with("medication_id", json!(7)), "medication_id"),
            (with("completes", json!("nope")), "completes"),
            (with("completes", json!(3)), "completes"),
            (with("charted_as", json!("started-here")), "charted_as"),
            (with("warning_override", Value::Null), "warning_override"),
            (
                with("restores", json!(Uuid::now_v7().to_string())),
                "restores",
            ),
            (with("stat", json!(true)), "stat"),
        ] {
            let e = floor_at(&mut c, GIVEN_FLOOR, &body(payload.clone()), remote)
                .await
                .expect_err(&format!("{w} must refuse {payload}"));
            assert!(e.contains(frag), "{w}: wanted {frag}: {e}");
        }
        let mut b = body(ok.clone());
        b["safety"] = json!({"rung": "existence"});
        let e = floor_at(&mut c, GIVEN_FLOOR, &b, remote)
            .await
            .expect_err("a safety claim");
        assert!(e.contains("safety"), "{e}");
        // The structural tier passes at the remote door; a JSON null safety reads as absent.
        if remote {
            b["safety"] = Value::Null;
            floor_at(&mut c, GIVEN_FLOOR, &b, true)
                .await
                .expect("accepted");
        }
    }
}

#[tokio::test]
async fn a_shred_removes_the_row() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let g = submit(&c, &sk, given(&c, &kid, p, med, None).await)
        .await
        .unwrap();
    assert_eq!(count(&c, N, g).await, 1);
    shred_event(&mut c, &sk, &kid, "test-node", g, "retention ceiling", None)
        .await
        .expect("shred");
    assert_eq!(count(&c, N, g).await, 0, "the shred takes the given row");
}

#[tokio::test]
async fn a_reprojection_rebuilds_it() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (_stop, g) = give_stat(&c, &sk, &kid, p, med).await.unwrap();
    c.batch_execute("TRUNCATE medication_given").await.unwrap();
    assert_eq!(count(&c, N, g).await, 0);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication-given.', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(count(&c, N, g).await, 1);
}

#[tokio::test]
async fn the_given_event_resolves_to_its_lines_thread() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let g = submit(&c, &sk, given(&c, &kid, p, med, None).await)
        .await
        .unwrap();
    let thread: Option<String> = c
        .query_one(
            "SELECT cairn_event_thread($1::text::uuid)::text",
            &[&g.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(thread, Some(med.to_string()));
}
