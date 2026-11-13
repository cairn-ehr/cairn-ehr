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
use cairn_event::medication::{
    render_set_aside_twin, set_aside_body, MedicationAssertion, MedicationSetAside, SourceNote,
    SET_ASIDE_EVENT_TYPE, SET_ASIDE_SCHEMA,
};
use cairn_event::{sign, ClockGrade, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::build_assert_body;
use cairn_node::shred::shred_event;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// The drug the source reported (the "old" Lipitor 40 mg of a GP letter).
fn drug() -> MedicationAssertion<'static> {
    MedicationAssertion {
        medication_id: "",
        term: "Lipitor",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "external-record",
        started: None,
        started_precision: None,
        safety: None,
        charting: None,
        source_note: Some(SourceNote {
            source_detail: Some("GP letter, 2026-03"),
            why: Some("packet says 25 mg"),
        }),
    }
}

/// A signed-ready set-aside body on `patient`, beside thread `alt`.
async fn version_body(
    c: &Client,
    kid: &str,
    patient: Uuid,
    version: Uuid,
    alt: Uuid,
    tweak: impl FnOnce(&mut Value),
) -> EventBody {
    let d = drug();
    let (v, a) = (version.to_string(), alt.to_string());
    let s = MedicationSetAside {
        version_id: &v,
        alternative_to: &a,
        drug: &d,
    };
    let mut payload = set_aside_body(&s);
    tweak(&mut payload);
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SET_ASIDE_EVENT_TYPE.into(),
        schema_version: SET_ASIDE_SCHEMA.into(),
        hlc: db::next_hlc(c, "test-node").await.unwrap(),
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(render_set_aside_twin(&s)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Submit a version through the strict (authoring) door; returns its event id.
async fn add_version(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    alt: Uuid,
) -> Result<Uuid, String> {
    let body = version_body(c, kid, patient, Uuid::now_v7(), alt, |_| {}).await;
    let id: Uuid = body.event_id.parse().unwrap();
    seal_and_submit(c, sk, body)
        .await
        .map(|_| id)
        .map_err(|e| db_msg(&e))
}

/// Call the set-aside floor directly on `body` at one door, in a throwaway transaction.
async fn floor(c: &mut Client, body: &Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            "SELECT cairn_check_medication_set_aside($1, $2::text::jsonb)",
            &[&SET_ASIDE_EVENT_TYPE, &body.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

/// A bare floor body (payload + schema + patient) for the direct calls.
fn floor_body(payload: Value) -> Value {
    json!({"schema_version": SET_ASIDE_SCHEMA, "patient_id": Uuid::now_v7().to_string(),
           "payload": payload})
}

async fn count(c: &Client, sql: &str, id: Uuid) -> i64 {
    c.query_one(sql, &[&id.to_string()]).await.unwrap().get(0)
}

macro_rules! rig {
    ($c:ident, $sk:ident, $kid:ident) => {
        let Some(base) = cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _guard = db::test_serial_guard(&base).await.unwrap();
        #[allow(unused_mut)]
        let mut $c = db::connect_and_load_schema(&base).await.unwrap();
        let ($sk, $kid) = setup_node(&$c).await;
    };
}

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

#[tokio::test]
async fn the_remote_door_admits_an_orphan_and_no_reader_shows_it() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let orphan_of = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, p, Uuid::now_v7(), orphan_of, |_| {}).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body)
        .await
        .expect("the sync door admits a version whose companion has not arrived");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside WHERE alternative_to = $1::text::uuid",
            orphan_of
        )
        .await,
        1,
        "held"
    );
    // Readers join on alternative_to: no thread exists, so nothing shows it.
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside s \
               JOIN medication_statement m ON m.medication_id = s.alternative_to \
              WHERE s.alternative_to = $1::text::uuid",
            orphan_of
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
        0
    );
}

#[tokio::test]
async fn a_remote_companion_on_another_chart_is_flagged_not_refused() {
    rig!(c, sk, kid);
    let (_p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let elsewhere = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, elsewhere, Uuid::now_v7(), med, |_| {}).await;
    body.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &body).await.expect("admitted");
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
            med
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_shred_removes_the_row() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med).await.unwrap();
    let n = "SELECT count(*) FROM medication_set_aside WHERE event_id = $1::text::uuid";
    assert_eq!(count(&c, n, ev).await, 1);
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
        count(&c, n, ev).await,
        0,
        "the reported drug is clinical text: the shred takes it"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_it() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let ev = add_version(&c, &sk, &kid, p, med).await.unwrap();
    c.batch_execute("TRUNCATE medication_set_aside")
        .await
        .unwrap();
    let n = "SELECT count(*) FROM medication_set_aside WHERE event_id = $1::text::uuid";
    assert_eq!(count(&c, n, ev).await, 0);
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication-set-aside', true, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(count(&c, n, ev).await, 1);
}

#[tokio::test]
async fn a_companion_in_the_same_transaction_satisfies_the_floor() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 0).await;
    let med = Uuid::now_v7();
    let companion = build_assert_body(
        Uuid::now_v7(),
        med,
        p,
        &sample_input(),
        &kid,
        db::next_hlc(&c, "test-node").await.unwrap(),
        None,
    );
    let version = version_body(&c, &kid, p, Uuid::now_v7(), med, |_| {}).await;
    let tx = c.transaction().await.unwrap();
    for mut body in [companion, version] {
        // Seal as seal_and_submit does, but submit on the transaction.
        let twin = body.plaintext_twin.take().unwrap();
        let (container, dek) =
            cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id).unwrap();
        body.payload = container;
        body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
        let signed = sign(&body, &sk).unwrap();
        tx.execute(
            "SELECT submit_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {}", body.event_type, db_msg(&e)));
    }
    tx.commit().await.unwrap();
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM medication_set_aside WHERE alternative_to = $1::text::uuid",
            med
        )
        .await,
        1
    );
}

#[tokio::test]
async fn a_peers_set_aside_with_a_safety_claim_is_not_admitted_and_raises_no_signal() {
    rig!(c, sk, kid);
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut body = version_body(&c, &kid, p, Uuid::now_v7(), med, |_| {}).await;
    body.hlc = peer_hlc(wall);
    body.safety = Some(json!({"rung": "existence"}));
    let _ = apply_remote(&c, &sk, &body).await;
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_patient_safety($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "no safety signal from a set-aside version");
}
