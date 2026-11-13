//! Batch charting slice 2a, Task 3 — the set-aside version of a medication line
//! (`clinical.medication-set-aside.asserted`): its floor (db/079 §2), its projection
//! (`medication_set_aside`), its shred and its reprojection.
//!
//! A set-aside version is what a source reported that the clinician did NOT believe (a GP
//! letter's old dose). It is signed into the record as what the source said and is **never a
//! current medication**: it is not a thread, so it must appear in no reader of current lines.
//!
//! Door rules (ADR-0056): structure refuses at BOTH doors; "the companion thread is on this
//! chart and is current" refuses at the AUTHORING door only, so a peer's orphan version (its
//! companion not yet synced) never wedges sync — it is held, and shown nowhere.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_node::db;
use serde_json::json;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;

#[tokio::test]
async fn a_set_aside_beside_a_current_line_projects_its_fields() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med)
        .await
        .expect("a version beside a current line is admitted");
    let r = c
        .query_one(
            "SELECT patient_id::text, alternative_to::text, term, formulation, dose_amount, dose_unit, sig, \
                    info_source, source_detail, why \
               FROM medication_set_aside WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .expect("one projected row");
    assert_eq!(r.get::<_, String>(0), p.to_string());
    assert_eq!(r.get::<_, String>(1), med.to_string());
    let text = |i| r.get::<_, Option<String>>(i);
    assert_eq!(text(2).as_deref(), Some("Lipitor"));
    assert_eq!(text(3).as_deref(), Some("tablet"));
    assert_eq!(text(4).as_deref(), Some("40"));
    assert_eq!(text(5).as_deref(), Some("mg"));
    assert_eq!(text(6).as_deref(), Some("one BD"));
    assert_eq!(text(7).as_deref(), Some("external-record"));
    assert_eq!(text(8).as_deref(), Some("GP letter, 2026-03"));
    assert_eq!(text(9).as_deref(), Some("packet says 25 mg"));
}

#[tokio::test]
async fn a_set_aside_is_never_a_current_medication() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med).await.unwrap();
    let version: Uuid = c
        .query_one(
            "SELECT version_id::text FROM medication_set_aside WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .unwrap()
        .get::<_, String>(0)
        .parse()
        .unwrap();
    // Not a thread: neither the statement table nor the current/past readers know it.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid",
            version
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM patient_medication WHERE patient_id = $1::text::uuid",
            p
        )
        .await,
        1,
        "only the real line"
    );
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM patient_medication_current WHERE patient_id = $1::text::uuid",
            p
        )
        .await,
        1
    );
    // No thread claims the version id either, so it can never be ceased or dosed into life.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid",
            version
        )
        .await,
        0
    );
    // The chart's safety report lists nothing for it, and the current-lines read (the allergy
    // check's line set) holds only the real line.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_patient_safety($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "a set-aside version raises no safety signal");
    let terms: Vec<String> = c
        .query(
            "SELECT term FROM patient_medication_current WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(terms, vec!["metoprolol".to_string()]);
    // No §5.9 claim rides it (the builder carries none, and the floor refuses one: next test).
    let safety_null: bool = c
        .query_one(
            "SELECT safety IS NULL FROM event_log WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(safety_null);
}

#[tokio::test]
async fn charted_as_or_warning_override_is_refused_at_both_doors() {
    rig!(c, _sk, _kid);
    let (v, a) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    let base = json!({"version_id": v, "alternative_to": a, "info_source": "external-record",
                      "substance": {"term": "Lipitor"}});
    for (k, val) in [
        ("charted_as", json!("brand")),
        ("warning_override", json!({"reason": "x", "against": []})),
    ] {
        let mut p = base.clone();
        p[k] = val;
        for remote in [false, true] {
            let e = floor(&mut c, &floor_body(p.clone()), remote)
                .await
                .expect_err(&format!("{k} must be refused (remote={remote})"));
            assert!(e.contains(k), "the refusal names the key: {e}");
        }
    }
    // Control: the bare body passes both doors (structure only; the authoring-door chart check
    // needs a real event, so the remote door is the control here).
    floor(&mut c, &floor_body(base.clone()), true)
        .await
        .unwrap();
}

#[tokio::test]
async fn structural_shape_is_refused_at_both_doors() {
    rig!(c, _sk, _kid);
    let (v, a) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    let ok = json!({"version_id": v, "alternative_to": a, "info_source": "external-record",
                    "substance": {"term": "Lipitor"}});
    // (message fragment, payload key, bad value)
    let cases = [
        ("version_id", "version_id", json!("nope")),
        ("alternative_to", "alternative_to", json!(7)),
        ("substance.term", "substance", json!({"term": "  "})),
        ("info_source", "info_source", json!(3)),
        ("source_detail", "source_detail", json!({"a": 1})),
        ("why", "why", json!(true)),
    ];
    for (frag, key, bad) in cases {
        let mut p = ok.clone();
        p[key] = bad;
        for remote in [false, true] {
            let e = floor(&mut c, &floor_body(p.clone()), remote)
                .await
                .expect_err(&format!("{frag} must be refused (remote={remote})"));
            assert!(e.contains(frag), "wanted {frag:?} in {e}");
        }
    }
}

#[tokio::test]
async fn a_safety_claim_is_refused_at_both_doors() {
    rig!(c, _sk, _kid);
    let (v, a) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    let mut b = floor_body(json!({"version_id": v, "alternative_to": a,
                                  "info_source": "external-record", "substance": {"term": "x"}}));
    b["safety"] = json!({"rung": "existence"});
    // db/020 writes b->'safety' into event_log.safety for EVERY type and cairn_patient_safety
    // reports it, so a peer's claim would become a live safety signal: refused at both doors.
    for remote in [false, true] {
        let e = floor(&mut c, &b, remote)
            .await
            .expect_err("a claim is refused");
        assert!(e.contains("safety"), "remote={remote}: {e}");
    }
    // A JSON null reads as absent.
    b["safety"] = json!(null);
    floor(&mut c, &b, true).await.unwrap();
}

#[tokio::test]
async fn an_object_route_strength_or_components_is_refused_at_both_doors() {
    rig!(c, _sk, _kid);
    let (v, a) = (Uuid::now_v7().to_string(), Uuid::now_v7().to_string());
    let ok = json!({"version_id": v, "alternative_to": a, "info_source": "external-record",
                    "route": "oral", "substance": {"term": "Lipitor", "strength": "40 mg"}});
    floor(&mut c, &floor_body(ok.clone()), true).await.unwrap();
    for (frag, key, bad) in [
        ("route", "route", json!({"a": 1})),
        (
            "strength",
            "substance",
            json!({"term": "x", "strength": {"a": 1}}),
        ),
    ] {
        let mut p = ok.clone();
        p[key] = bad;
        for remote in [false, true] {
            let e = floor(&mut c, &floor_body(p.clone()), remote)
                .await
                .expect_err(&format!("{frag} must be refused (remote={remote})"));
            assert!(e.contains(frag), "wanted {frag:?} in {e}");
        }
    }
}

#[tokio::test]
async fn a_companion_on_another_chart_or_unknown_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, _med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let (_other, other_med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let e = add_version(&c, &sk, &kid, p, other_med)
        .await
        .expect_err("a companion on another chart is refused");
    assert!(e.contains("alternative_to"), "{e}");
    let e = add_version(&c, &sk, &kid, p, Uuid::now_v7())
        .await
        .expect_err("an unknown companion is refused");
    assert!(e.contains("alternative_to"), "{e}");
}

#[tokio::test]
async fn a_stopped_companion_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    cease(&mut c, &sk, &kid, p, med, None).await;
    let e = add_version(&c, &sk, &kid, p, med)
        .await
        .expect_err("a version beside a stopped line is refused");
    assert!(e.contains("current"), "{e}");
}

#[tokio::test]
async fn a_repeated_version_id_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let v = Uuid::now_v7();
    for (i, want_ok) in [true, false].into_iter().enumerate() {
        let body = version_body(&c, &kid, p, v, med, |_| {}).await;
        let r = seal_and_submit(&c, &sk, body).await.map_err(|e| db_msg(&e));
        assert_eq!(r.is_ok(), want_ok, "attempt {i}: {r:?}");
    }
}
