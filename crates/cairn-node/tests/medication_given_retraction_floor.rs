//! Charting slice 2, Task 3 — "Entered in error" for a given dose (db/082 §2).
//!
//! `clinical.medication-given-retraction.asserted` strikes a given (`target`) and names the stop
//! the given caused (`voids`). The stop's appliers skip a voided stop, and the retraction's
//! applier deletes the voided stop's rows and re-folds the thread, so a stat line whose give was
//! struck reads CURRENT again — unless a REAL stop of the same line still stands (Review Focus 1).
//! This file pins that outcome in every arrival order, through a heal and a rebuild, and through
//! a shred of each of the three events (Review Focus 2 and 3), plus the floor's two tiers.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` (inside `rig!`).
use cairn_event::medication::{
    given_retraction_body, render_given_retraction_twin, GivenRetraction,
    GIVEN_RETRACTION_EVENT_TYPE, GIVEN_RETRACTION_SCHEMA,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{build_cease_body, CeaseMedicationInput};
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

/// A signed-ready retraction striking `target` on `patient`'s line `med`, voiding `voids`.
async fn retraction(
    c: &Client,
    kid: &str,
    patient: Uuid,
    med: Uuid,
    target: Uuid,
    voids: Option<Uuid>,
) -> EventBody {
    let (m, t, v) = (
        med.to_string(),
        target.to_string(),
        voids.map(|u| u.to_string()),
    );
    let r = GivenRetraction {
        medication_id: &m,
        target: &t,
        voids: v.as_deref(),
        text: Some("wrong patient"),
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: GIVEN_RETRACTION_EVENT_TYPE.into(),
        schema_version: GIVEN_RETRACTION_SCHEMA.into(),
        hlc: db::next_hlc(c, "test-node").await.unwrap(),
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: given_retraction_body(&r),
        attachments: vec![],
        plaintext_twin: Some(render_given_retraction_twin(&r)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// A real stop of `med` (a clinician's decision, not a give's).
async fn real_stop(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, med: Uuid) -> Uuid {
    let input = CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some("patient refused"),
    };
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    submit(
        c,
        sk,
        build_cease_body(Uuid::now_v7(), med, p, &input, kid, hlc),
    )
    .await
    .expect("a real stop")
}

/// A stat line given, and that give struck. Returns (patient, line, stop, given, retraction).
async fn struck(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
    let (p, med) = chart(c, sk, kid, true).await;
    let (stop, g) = give_stat(c, sk, kid, p, med).await.expect("given");
    let r = submit(c, sk, retraction(c, kid, p, med, g, Some(stop)).await)
        .await
        .expect("struck");
    (p, med, stop, g, r)
}

async fn is_current(c: &Client, med: Uuid) -> bool {
    let sql =
        "SELECT count(*) FROM patient_medication_current WHERE medication_id = $1::text::uuid";
    count(c, sql, med).await == 1
}

/// The stop reason the line SHOWS (db/031's one winner), when it reads stopped.
async fn shown_reason(c: &Client, med: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .and_then(|r| r.get(0))
}

const STAT_REASONS: &str = "SELECT count(*) FROM medication_cessation_reason \
     WHERE medication_id = $1::text::uuid AND reason = 'stat dose given'";
const FLOOR: &str = "SELECT cairn_check_medication_given_retraction(\
     'clinical.medication-given-retraction.asserted', $1::text::jsonb)";

async fn shred(c: &mut Client, sk: &SigningKey, kid: &str, id: Uuid) {
    shred_event(c, sk, kid, "test-node", id, "retention ceiling", None)
        .await
        .expect("shred");
}

#[tokio::test]
async fn a_retraction_returns_the_stat_line_to_current() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (stop, g) = give_stat(&c, &sk, &kid, p, med).await.unwrap();
    assert!(
        !is_current(&c, med).await,
        "given: the stat line reads stopped"
    );
    assert_eq!(count(&c, STAT_REASONS, med).await, 1);
    submit(&c, &sk, retraction(&c, &kid, p, med, g, Some(stop)).await)
        .await
        .expect("the strike is admitted");
    assert!(
        is_current(&c, med).await,
        "struck: waiting to be given again"
    );
    assert_eq!(count(&c, STAT_REASONS, med).await, 0, "its reason is gone");
}

/// Review Focus 1: a real stop and a give of the same stat line. Striking the give must leave
/// the real stop standing, whichever of the two was signed first.
#[tokio::test]
async fn a_struck_given_leaves_a_real_stop_standing() {
    rig!(c, sk, kid);
    for real_first in [true, false] {
        let (p, med) = chart(&c, &sk, &kid, true).await;
        if real_first {
            real_stop(&c, &sk, &kid, p, med).await;
        }
        let (stop, g) = give_stat(&c, &sk, &kid, p, med).await.unwrap();
        if !real_first {
            real_stop(&c, &sk, &kid, p, med).await;
        }
        submit(&c, &sk, retraction(&c, &kid, p, med, g, Some(stop)).await)
            .await
            .unwrap();
        assert!(
            !is_current(&c, med).await,
            "real_first={real_first}: still stopped"
        );
        assert_eq!(
            shown_reason(&c, med).await.as_deref(),
            Some("patient refused"),
            "real_first={real_first}: the real stop shows"
        );
    }
}

/// Review Focus 2: a peer's stop, given and retraction arriving in each of the six orders.
#[tokio::test]
async fn every_arrival_order_converges() {
    rig!(c, sk, kid);
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let (p, med) = chart(&c, &sk, &kid, true).await;
        let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
        let mut stop = stop_body(&c, &kid, p, med).await;
        stop.hlc = peer_hlc(wall);
        let stop_id: Uuid = stop.event_id.parse().unwrap();
        let mut g = given(&c, &kid, p, med, Some(stop_id)).await;
        g.hlc = peer_hlc(wall + 1);
        let g = effective_at_wall(&c, g).await;
        let mut r = retraction(&c, &kid, p, med, g.event_id.parse().unwrap(), Some(stop_id)).await;
        r.hlc = peer_hlc(wall + 2);
        let bodies = [stop, g, r];
        for i in order {
            apply_remote(&c, &sk, &bodies[i])
                .await
                .map_err(|e| db_msg(&e))
                .unwrap_or_else(|e| panic!("{order:?}: event {i}: {e}"));
        }
        assert!(
            is_current(&c, med).await,
            "{order:?}: the struck line reads current"
        );
        assert_eq!(count(&c, STAT_REASONS, med).await, 0, "{order:?}");
    }
}

/// Review Focus 2: a heal and a whole-medication rebuild keep the struck line current and an
/// unstruck given's line stopped.
#[tokio::test]
async fn a_reprojection_keeps_a_struck_givens_line_current() {
    rig!(c, sk, kid);
    let (_p, med, ..) = struck(&c, &sk, &kid).await;
    let (p2, unstruck) = chart(&c, &sk, &kid, true).await;
    give_stat(&c, &sk, &kid, p2, unstruck).await.unwrap();
    for (prefix, rebuild) in [("", false), ("clinical.medication", true)] {
        c.execute(
            "SELECT count(*) FROM cairn_reproject($1, $2, 'test')",
            &[&prefix, &rebuild],
        )
        .await
        .unwrap();
        assert!(
            is_current(&c, med).await,
            "rebuild={rebuild}: struck reads current"
        );
        assert!(
            !is_current(&c, unstruck).await,
            "rebuild={rebuild}: unstruck reads stopped"
        );
        assert_eq!(count(&c, STAT_REASONS, med).await, 0);
        assert_eq!(count(&c, STAT_REASONS, unstruck).await, 1);
    }
}

/// db/039 refuses a narrow rebuild whose tables another type also feeds: the retraction type
/// lists medication_cessation, so rebuilding it alone would truncate every stop and replay only
/// retractions. It must be refused, every stop intact.
#[tokio::test]
async fn a_narrow_rebuild_of_the_retraction_type_keeps_every_stop() {
    rig!(c, sk, kid);
    let (_p, med, ..) = struck(&c, &sk, &kid).await;
    let (p2, stopped) = chart(&c, &sk, &kid, false).await;
    real_stop(&c, &sk, &kid, p2, stopped).await;
    for prefix in [
        "clinical.medication-given-retraction",
        "clinical.medication-given",
    ] {
        let e = c
            .execute(
                "SELECT count(*) FROM cairn_reproject($1, true, 'test')",
                &[&prefix],
            )
            .await
            .map_err(|e| db_msg(&e))
            .expect_err("a narrow rebuild over medication_cessation is refused");
        assert!(
            e.contains("also fed by event types outside"),
            "{prefix}: {e}"
        );
        assert!(
            !is_current(&c, stopped).await,
            "{prefix}: the real stop stands"
        );
        assert!(
            is_current(&c, med).await,
            "{prefix}: the struck line is current"
        );
    }
}

/// Review Focus 3: the void keys on the retraction alone, so the given's shred changes nothing.
#[tokio::test]
async fn shredding_the_given_keeps_the_line_current() {
    rig!(c, sk, kid);
    let (_p, med, _stop, g, _r) = struck(&c, &sk, &kid).await;
    shred(&mut c, &sk, &kid, g).await;
    assert!(is_current(&c, med).await);
}

/// Review Focus 3: the strike is gone, so the give's stop is in force again ("given" stands).
#[tokio::test]
async fn shredding_the_retraction_stops_the_line_again() {
    rig!(c, sk, kid);
    let (_p, med, _stop, _g, r) = struck(&c, &sk, &kid).await;
    shred(&mut c, &sk, &kid, r).await;
    assert!(!is_current(&c, med).await, "the give's stop is back");
    assert_eq!(
        shown_reason(&c, med).await.as_deref(),
        Some("stat dose given")
    );
    assert_eq!(count(&c, STAT_REASONS, med).await, 1);
    let n = "SELECT count(*) FROM medication_given_retraction WHERE event_id = $1::text::uuid";
    assert_eq!(
        count(&c, n, r).await,
        0,
        "the shred takes the retraction row"
    );
}

#[tokio::test]
async fn shredding_the_paired_stop_leaves_the_line_current() {
    rig!(c, sk, kid);
    let (_p, med, stop, _g, _r) = struck(&c, &sk, &kid).await;
    shred(&mut c, &sk, &kid, stop).await;
    assert!(is_current(&c, med).await);
}

#[tokio::test]
async fn voids_differing_from_the_givens_completes_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (_stop, g) = give_stat(&c, &sk, &kid, p, med).await.unwrap();
    let real = real_stop(&c, &sk, &kid, p, med).await;
    for voids in [Some(real), Some(Uuid::now_v7()), None] {
        let e = submit(&c, &sk, retraction(&c, &kid, p, med, g, voids).await)
            .await
            .expect_err(&format!("voids {voids:?} is not what the given completes"));
        assert!(e.contains("voids"), "{e}");
    }
    // An ordinary give completes nothing, so its strike voids nothing.
    let (p2, plain) = chart(&c, &sk, &kid, false).await;
    let g2 = submit(&c, &sk, given(&c, &kid, p2, plain, None).await)
        .await
        .unwrap();
    let e = submit(
        &c,
        &sk,
        retraction(&c, &kid, p2, plain, g2, Some(real)).await,
    )
    .await
    .expect_err("voids on a give that stopped nothing");
    assert!(e.contains("voids"), "{e}");
    assert!(!is_current(&c, med).await, "the real stop stands");
}

#[tokio::test]
async fn another_charts_or_an_unknown_given_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, true).await;
    let (stop, g) = give_stat(&c, &sk, &kid, p, med).await.unwrap();
    let (other, _m) = chart(&c, &sk, &kid, false).await;
    let sibling = line(&c, &sk, &kid, p, true).await;
    for (patient, line_id, target) in [
        (p, med, Uuid::now_v7()), // no such given
        (other, med, g),          // the given is on another chart
        (p, sibling, g),          // the given is of another line
    ] {
        let e = submit(
            &c,
            &sk,
            retraction(&c, &kid, patient, line_id, target, Some(stop)).await,
        )
        .await
        .expect_err("not a given of this line on this chart");
        assert!(e.contains("not a given dose of this line"), "{e}");
    }
    let mut b = retraction(&c, &kid, p, med, g, Some(stop)).await;
    b.schema_version = "clinical.medication-given-retraction/2".into();
    let e = submit(&c, &sk, b).await.expect_err("tag");
    assert!(e.contains("schema_version"), "{e}");
    assert!(!is_current(&c, med).await, "nothing was struck");
}

#[tokio::test]
async fn the_remote_door_admits_an_orphan_retraction() {
    rig!(c, sk, kid);
    let (p, med) = (Uuid::now_v7(), Uuid::now_v7());
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = retraction(&c, &kid, p, med, Uuid::now_v7(), Some(Uuid::now_v7())).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body)
        .await
        .map_err(|e| db_msg(&e))
        .expect("a peer's strike whose given has not synced");
    let n = "SELECT count(*) FROM medication_given_retraction WHERE event_id = $1::text::uuid";
    assert_eq!(
        count(&c, n, body.event_id.parse().unwrap()).await,
        1,
        "held"
    );
}

#[tokio::test]
async fn a_retraction_of_a_given_with_no_completes_stops_nothing() {
    rig!(c, sk, kid);
    let (p, med) = chart(&c, &sk, &kid, false).await;
    let g = submit(&c, &sk, given(&c, &kid, p, med, None).await)
        .await
        .unwrap();
    real_stop(&c, &sk, &kid, p, med).await;
    submit(&c, &sk, retraction(&c, &kid, p, med, g, None).await)
        .await
        .expect("an ordinary give is struck, voiding nothing");
    assert!(!is_current(&c, med).await, "the real stop is untouched");
    assert_eq!(
        shown_reason(&c, med).await.as_deref(),
        Some("patient refused")
    );
}

#[tokio::test]
async fn a_second_retraction_is_admitted_and_harmless() {
    rig!(c, sk, kid);
    let (p, med, stop, g, _r) = struck(&c, &sk, &kid).await;
    submit(&c, &sk, retraction(&c, &kid, p, med, g, Some(stop)).await)
        .await
        .expect("a second strike of the same give");
    assert!(is_current(&c, med).await);
    let n = "SELECT count(*) FROM medication_given_retraction WHERE medication_id = $1::text::uuid";
    assert_eq!(count(&c, n, med).await, 2);
}

#[tokio::test]
async fn a_bad_shape_a_safety_claim_or_a_charting_key_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    let u = || json!(Uuid::now_v7().to_string());
    let ok = json!({"medication_id": u(), "target": u(), "voids": null, "text": null});
    let with = |k: &str, v: Value| {
        let mut o = ok.clone();
        o[k] = v;
        json!({"schema_version": GIVEN_RETRACTION_SCHEMA, "patient_id": u(), "payload": o})
    };
    for remote in [false, true] {
        for (b, frag) in [
            (json!({"payload": "nope"}), "payload"),
            (with("medication_id", json!("nope")), "medication_id"),
            (with("target", json!(7)), "target"),
            (with("voids", json!("nope")), "voids"),
            (with("text", json!(3)), "text"),
            (with("stat", json!(true)), "stat"),
            (with("charted_as", Value::Null), "charted_as"),
        ] {
            let e = floor_raw(&mut c, &b, remote)
                .await
                .expect_err(&format!("remote={remote} must refuse {b}"));
            assert!(e.contains(frag), "remote={remote}: wanted {frag}: {e}");
        }
        let mut b = with("text", Value::Null);
        b["safety"] = json!({"rung": "existence"});
        let e = floor_raw(&mut c, &b, remote).await.expect_err("safety");
        assert!(e.contains("safety"), "{e}");
    }
    floor_raw(&mut c, &with("text", Value::Null), true)
        .await
        .expect("a well-shaped orphan passes the remote door's structural tier");
}

async fn floor_raw(c: &mut Client, body: &Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx.execute(FLOOR, &[&body.to_string()]).await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

#[tokio::test]
async fn the_retraction_resolves_to_its_lines_thread() {
    rig!(c, sk, kid);
    let (_p, med, _stop, _g, r) = struck(&c, &sk, &kid).await;
    let thread: Option<String> = c
        .query_one(
            "SELECT cairn_event_thread($1::text::uuid)::text",
            &[&r.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(thread, Some(med.to_string()));
}
