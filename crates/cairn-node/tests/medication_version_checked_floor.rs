//! Batch charting slice 2b, Task 3 — "checked: not current" as its own event type
//! (`clinical.medication-set-aside-checked.asserted`): its floor (db/080 §2), its projection
//! (`medication_set_aside_checked`), its shred and its reprojection.
//!
//! A check says a reviewing clinician looked at a set-aside version again and it is still not
//! current. It is never a medication line and carries no safety claim. Door rules (ADR-0056):
//! structure refuses at BOTH doors; "the version is a set-aside version of THIS chart" refuses
//! at the AUTHORING door only, so a peer's check that arrives before its version never wedges
//! sync (it is held and shown nowhere: readers join on version_id AND patient_id).
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`).
use cairn_event::medication::{
    render_version_checked_twin, version_checked_body, SourceNote, VersionChecked,
    VERSION_CHECKED_EVENT_TYPE, VERSION_CHECKED_SCHEMA,
};
use cairn_event::EventBody;
use cairn_node::db;
use cairn_node::shred::shred_event;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;
#[path = "common/med_restore_kit.rs"]
mod med_restore_kit;
use med_restore_kit::*;

/// A signed-ready check of `version` on `patient`; `tweak` edits the payload (to break it).
async fn check_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
    tweak: impl FnOnce(&mut Value),
) -> EventBody {
    let v = version.to_string();
    let k = VersionChecked {
        version_id: &v,
        info_source: "external-record",
        source_note: Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: Some("dispensed 25 mg"),
        }),
    };
    let mut payload = version_checked_body(&k);
    tweak(&mut payload);
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: VERSION_CHECKED_EVENT_TYPE.into(),
        schema_version: VERSION_CHECKED_SCHEMA.into(),
        hlc: db::next_hlc(c, "test-node").await.unwrap(),
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(render_version_checked_twin(&k)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Submit a check through the strict (authoring) door; returns its event id.
async fn add_check(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    version: Uuid,
) -> Result<Uuid, String> {
    let body = check_body(c, kid, patient, version, |_| {}).await;
    let id: Uuid = body.event_id.parse().unwrap();
    seal_and_submit(c, sk, body)
        .await
        .map(|_| id)
        .map_err(|e| db_msg(&e))
}

/// The floor called directly on `payload` at one door, in a throwaway transaction.
async fn floor_of(
    c: &mut Client,
    patient: Uuid,
    payload: Value,
    safety: Option<Value>,
    remote: bool,
) -> Result<(), String> {
    let mut b = json!({"schema_version": VERSION_CHECKED_SCHEMA,
                       "patient_id": patient.to_string(), "payload": payload});
    if let Some(s) = safety {
        b["safety"] = s;
    }
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            "SELECT cairn_check_medication_version_checked($1, $2::text::jsonb)",
            &[&VERSION_CHECKED_EVENT_TYPE, &b.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

const N: &str = "SELECT count(*) FROM medication_set_aside_checked WHERE event_id = $1::text::uuid";

#[tokio::test]
async fn a_check_of_a_version_on_this_chart_projects() {
    rig!(c, sk, kid);
    let (p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let ev = add_check(&c, &sk, &kid, p, version)
        .await
        .expect("admitted");
    let r = c
        .query_one(
            "SELECT patient_id::text, version_id::text, info_source, source_detail, why, \
                    t_recorded IS NOT NULL, cairn_event_thread(event_id)::text \
               FROM medication_set_aside_checked WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .expect("one projected row");
    assert_eq!(r.get::<_, String>(0), p.to_string());
    assert_eq!(r.get::<_, String>(1), version.to_string());
    assert_eq!(r.get::<_, String>(2), "external-record");
    assert_eq!(r.get::<_, Option<String>>(3).as_deref(), Some("pharmacy"));
    assert_eq!(
        r.get::<_, Option<String>>(4).as_deref(),
        Some("dispensed 25 mg")
    );
    assert!(r.get::<_, bool>(5));
    // The check's thread is its version's companion (so a sensitivity grade on it covers this).
    let med: String = c
        .query_one("SELECT alternative_to::text FROM medication_set_aside WHERE version_id = $1::text::uuid",
                   &[&version.to_string()]).await.unwrap().get(0);
    assert_eq!(r.get::<_, Option<String>>(6), Some(med));
}

#[tokio::test]
async fn a_check_of_another_charts_or_unknown_version_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (_p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let (other, _m2) = chart_with_one_med(&mut c, &sk, &kid).await;
    let e = add_check(&c, &sk, &kid, other, version)
        .await
        .expect_err("another chart's version");
    assert!(e.contains("not a set-aside version on this chart"), "{e}");
    let e = add_check(&c, &sk, &kid, other, Uuid::now_v7())
        .await
        .expect_err("unknown version");
    assert!(e.contains("not a set-aside version on this chart"), "{e}");
    // A wrong schema tag is refused at the authoring door too.
    let body = check_body(&c, &kid, other, version, |_| {}).await;
    let mut body = body;
    body.schema_version = "clinical.medication-set-aside-checked/2".into();
    let e = seal_and_submit(&c, &sk, body)
        .await
        .map_err(|e| db_msg(&e))
        .expect_err("tag");
    assert!(e.contains("schema_version"), "{e}");
}

#[tokio::test]
async fn the_remote_door_admits_an_orphan_check() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = check_body(&c, &kid, p, Uuid::now_v7(), |_| {}).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body)
        .await
        .expect("a peer's check whose version has not synced");
    let id: Uuid = body.event_id.parse().unwrap();
    assert_eq!(count(&c, N, id).await, 1, "held");
    // Shown nowhere: no set-aside row joins it on version AND chart.
    let shown: i64 = c.query_one(
        "SELECT count(*) FROM medication_set_aside_checked k JOIN medication_set_aside s \
           ON s.version_id = k.version_id AND s.patient_id = k.patient_id WHERE k.event_id = $1::text::uuid",
        &[&id.to_string()]).await.unwrap().get(0);
    assert_eq!(shown, 0);
}

#[tokio::test]
async fn two_checks_of_one_version_both_project() {
    rig!(c, sk, kid);
    let (p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let a = add_check(&c, &sk, &kid, p, version).await.unwrap();
    let b = add_check(&c, &sk, &kid, p, version)
        .await
        .expect("a second check is admitted");
    assert_eq!(count(&c, N, a).await + count(&c, N, b).await, 2);
}

#[tokio::test]
async fn current_line_keys_and_a_safety_claim_are_refused_at_both_doors() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    let p = Uuid::now_v7();
    let v = Uuid::now_v7().to_string();
    let ok = json!({"version_id": v, "info_source": "external-record"});
    for remote in [false, true] {
        let w = format!("remote={remote}");
        let with = |k: &str, val: Value| {
            let mut o = ok.clone();
            o[k] = val;
            o
        };
        for (payload, frag) in [
            (json!({"info_source": "x"}), "version_id"),
            (with("version_id", json!("nope")), "version_id"),
            (with("info_source", json!("  ")), "info_source"),
            (with("info_source", json!(3)), "info_source"),
            (with("charted_as", json!("brand")), "charted_as"),
            (with("charted_as", Value::Null), "charted_as"),
            (
                with("warning_override", json!({"reason": "x"})),
                "warning_override",
            ),
            (
                with("restores", json!(Uuid::now_v7().to_string())),
                "restores",
            ),
            (with("why", json!(true)), "why"),
        ] {
            let e = floor_of(&mut c, p, payload.clone(), None, remote)
                .await
                .expect_err(&format!("{w} must refuse {payload}"));
            assert!(e.contains(frag), "{w}: wanted {frag}: {e}");
        }
        let e = floor_of(
            &mut c,
            p,
            ok.clone(),
            Some(json!({"rung": "existence"})),
            remote,
        )
        .await
        .expect_err("a safety claim");
        assert!(e.contains("safety"), "{e}");
        // A JSON null safety reads as absent; the structural tier passes at the remote door.
        if remote {
            floor_of(&mut c, p, ok.clone(), Some(Value::Null), true)
                .await
                .expect("accepted");
        }
    }
}

#[tokio::test]
async fn a_shred_removes_the_row() {
    rig!(c, sk, kid);
    let (p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let ev = add_check(&c, &sk, &kid, p, version).await.unwrap();
    assert_eq!(count(&c, N, ev).await, 1);
    shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        ev,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred");
    assert_eq!(
        count(&c, N, ev).await,
        0,
        "the check's note is clinical text: the shred takes it"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_it() {
    rig!(c, sk, kid);
    let (p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let ev = add_check(&c, &sk, &kid, p, version).await.unwrap();
    c.batch_execute("TRUNCATE medication_set_aside_checked")
        .await
        .unwrap();
    assert_eq!(count(&c, N, ev).await, 0);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication-set-aside-checked', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(count(&c, N, ev).await, 1);
}

#[tokio::test]
async fn a_check_is_never_a_current_medication() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let ev = add_check(&c, &sk, &kid, p, version).await.unwrap();
    let ids = [version.to_string(), ev.to_string()];
    let n = |sql: &'static str| {
        let c = &c;
        let ids = &ids;
        async move {
            c.query_one(sql, &[&ids[0], &ids[1]])
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    assert_eq!(n("SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid OR content_address IN (SELECT content_address FROM event_log WHERE event_id = $2::text::uuid)").await, 0);
    // The chart's current readers still show exactly the one companion line.
    let shown: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(shown, 1, "only the companion {med}");
    let sig: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_patient_safety($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(sig, 0, "no safety signal from a check");
    let safe: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid AND safety IS NULL",
            &[&ev.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(safe, 1, "event_log.safety IS NULL");
}
