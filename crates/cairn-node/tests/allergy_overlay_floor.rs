//! Allergy slice 2, Task 2 — the three allergy overlays end to end below the window: a
//! CORRECTION (an ordinary `clinical.allergy.asserted` whose `refs` names the line it replaces),
//! a DE-LABEL (`clinical.allergy-challenge.asserted`) and an ENTERED-IN-ERROR withdrawal
//! (`clinical.allergy-retraction.asserted`). Each is built by `cairn_event`, sealed, signed by a
//! HUMAN author (ADR-0053), judged by the strict door's floor (db/074 step 3, db/075), and
//! projected in the clear; a shred then scrubs the overlay rows (db/037).
//!
//! `db/tests/075_allergy_overlays_test.sql` pins the floor rule by rule; this file proves the
//! real pipeline, one test per section of that mirror. The remote-door sections call the floor
//! directly with `cairn.remote_apply = 'on'`, as db/020 sets it before the floor runs.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::allergy::{
    allergy_none_payload, allergy_payload, render_allergy_none_twin, render_allergy_twin,
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
    ALLERGY_EVENT_TYPE, ALLERGY_NONE_EVENT_TYPE, ALLERGY_NONE_SCHEMA_VERSION,
    ALLERGY_SCHEMA_VERSION,
};
use cairn_event::allergy_overlay::{
    allergy_challenge_payload, allergy_retraction_payload, render_allergy_challenge_twin,
    render_allergy_retraction_twin, AllergyChallenge, AllergyRetraction,
    ALLERGY_CHALLENGE_EVENT_TYPE, ALLERGY_CHALLENGE_SCHEMA_VERSION, ALLERGY_RETRACTION_EVENT_TYPE,
    ALLERGY_RETRACTION_SCHEMA_VERSION,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

/// The (event_type, schema_version) pairs, for brevity.
const ENTRY: (&str, &str) = (ALLERGY_EVENT_TYPE, ALLERGY_SCHEMA_VERSION);
const NONE: (&str, &str) = (ALLERGY_NONE_EVENT_TYPE, ALLERGY_NONE_SCHEMA_VERSION);
const CHALLENGE: (&str, &str) = (
    ALLERGY_CHALLENGE_EVENT_TYPE,
    ALLERGY_CHALLENGE_SCHEMA_VERSION,
);
const RETRACTION: (&str, &str) = (
    ALLERGY_RETRACTION_EVENT_TYPE,
    ALLERGY_RETRACTION_SCHEMA_VERSION,
);

/// A node with an enrolled human author, and one registered chart (#345: a chart's first event
/// is its registration). `seal_sign_submit` rewrites each body to the human's authorship.
struct Rig {
    node_sk: SigningKey,
    node_kid: String,
    human_sk: SigningKey,
    human_kid: String,
    patient: Uuid,
}

impl Rig {
    async fn new(c: &Client) -> Rig {
        let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(c).await;
        let patient = Uuid::now_v7();
        common::submit_registration(c, &node_sk, &node_kid, patient, 0).await;
        Rig {
            node_sk,
            node_kid,
            human_sk,
            human_kid,
            patient,
        }
    }

    /// Register a second chart on the same node (for the "another chart" refusals).
    async fn another_chart(&self, c: &Client) -> Uuid {
        let other = Uuid::now_v7();
        common::submit_registration(c, &self.node_sk, &self.node_kid, other, 1).await;
        other
    }

    /// Seal, sign and submit one body on `patient` through the strict door. The note_body
    /// skeleton: no `t_effective`, no safety signal (ADR-0096 decision 21).
    async fn submit(
        &self,
        c: &mut Client,
        patient: Uuid,
        (event_type, schema_version): (&str, &str),
        payload: Value,
        twin: String,
    ) -> anyhow::Result<Uuid> {
        let hlc = db::next_hlc(c, &self.node_kid).await.unwrap();
        let id = Uuid::now_v7();
        let body = EventBody {
            event_id: id.to_string(),
            patient_id: patient.to_string(),
            event_type: event_type.into(),
            schema_version: schema_version.into(),
            hlc,
            t_effective: None,
            signer_key_id: self.node_kid.clone(),
            contributors: json!([{"actor_id": self.node_kid, "role": "recorded"}]),
            payload,
            attachments: vec![],
            plaintext_twin: Some(twin),
            clock_grade: cairn_event::ClockGrade::SelfAsserted,
            safety: None,
        };
        let author = AuthorParams {
            human_sk: &self.human_sk,
            human_kid: &self.human_kid,
        };
        seal_sign_submit(c, &self.node_sk, body, Some(&author), None).await
    }

    /// An entry on `patient`, correcting `corrects` when given.
    async fn entry(
        &self,
        c: &mut Client,
        patient: Uuid,
        corrects: Option<&str>,
    ) -> anyhow::Result<Uuid> {
        let e = entry_of(corrects);
        self.submit(
            c,
            patient,
            ENTRY,
            allergy_payload(&e),
            render_allergy_twin(&e),
        )
        .await
    }

    /// A de-label of `target` on this chart, dated `date`.
    async fn challenge(&self, c: &mut Client, target: Uuid, date: Onset) -> anyhow::Result<Uuid> {
        let target = target.to_string();
        let ch = AllergyChallenge {
            target: &target,
            tolerated: "amoxicillin",
            date,
            text: Some("oral challenge"),
        };
        let (payload, twin) = (
            allergy_challenge_payload(&ch),
            render_allergy_challenge_twin(&ch),
        );
        self.submit(c, self.patient, CHALLENGE, payload, twin).await
    }

    /// A withdrawal of `target` on this chart.
    async fn retraction(&self, c: &mut Client, target: Uuid) -> anyhow::Result<Uuid> {
        let target = target.to_string();
        let r = AllergyRetraction {
            target: &target,
            text: Some("wrong patient"),
        };
        let (payload, twin) = (
            allergy_retraction_payload(&r),
            render_allergy_retraction_twin(&r),
        );
        self.submit(c, self.patient, RETRACTION, payload, twin)
            .await
    }

    /// A "none known drug allergies" on this chart.
    async fn none(&self, c: &mut Client) -> Uuid {
        let s = NoneScope::Drug;
        self.submit(
            c,
            self.patient,
            NONE,
            allergy_none_payload(s),
            render_allergy_none_twin(s),
        )
        .await
        .expect("a none statement is admitted")
    }
}

/// An amoxicillin entry, correcting `corrects` when given.
fn entry_of(corrects: Option<&str>) -> AllergyEntry<'_> {
    AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term: "amoxicillin",
        kind: EntryKind::Allergy,
        category: ReactionCategory::Rash,
        reaction_text: None,
        onset: Onset::Year("2019".into()),
        source: Source::PatientStated,
        corrects,
    }
}

/// The refusal text of a submit the floor must refuse, asserted to contain `frag`.
fn refused(r: anyhow::Result<Uuid>, frag: &str) {
    let e = format!("{:#}", r.expect_err("the floor must refuse this body"));
    assert!(
        e.contains(frag),
        "refused for the wrong reason: {e} (wanted {frag:?})"
    );
}

/// Rows in projection `table` for one event.
async fn rows(c: &Client, table: &str, id: Uuid) -> i64 {
    c.query_one(
        &format!("SELECT count(*) FROM {table} WHERE event_id = $1::text::uuid"),
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Call floor `check_fn` directly on a hand-built `body` at one door (`remote` = the sync-apply
/// door, as db/020 marks it) in a throwaway transaction; `Err` carries the refusal text.
async fn floor(c: &mut Client, check_fn: &str, body: Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            &format!("SELECT {check_fn}('', $1::text::jsonb)"),
            &[&body.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| {
        e.as_db_error()
            .map(|d| d.message().to_owned())
            .unwrap_or_default()
    })
}

/// A hand-built de-label body on `patient`, with `over` merged into its payload.
fn challenge_body(patient: Uuid, target: Uuid, over: Value) -> Value {
    let mut p = json!({
        "target": target.to_string(),
        "tolerated": {"term": "amoxicillin", "coding": null},
        "date": {"precision": "not-known", "value": null},
        "text": null,
    });
    p.as_object_mut()
        .unwrap()
        .extend(over.as_object().unwrap().clone());
    json!({"schema_version": ALLERGY_CHALLENGE_SCHEMA_VERSION, "patient_id": patient.to_string(),
           "plaintext_twin": "t", "payload": p})
}

/// Section 0: both new types are registered additive, aim at no other author, and their floors
/// and heal-safe projections are wired.
#[tokio::test]
async fn the_overlay_types_are_registered() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for (t, check_fn, apply_fn) in [
        (
            ALLERGY_CHALLENGE_EVENT_TYPE,
            "cairn_check_allergy_challenge",
            "clinical_allergy_challenge_apply",
        ),
        (
            ALLERGY_RETRACTION_EVENT_TYPE,
            "cairn_check_allergy_retraction",
            "clinical_allergy_retraction_apply",
        ),
    ] {
        let r = c
            .query_one(
                "SELECT (SELECT mode = 'additive' AND NOT targets_other_author \
                           FROM event_type_class WHERE event_type = $1), \
                        (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = $1), \
                        (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply \
                          WHERE event_type = $1 AND apply_fn = $2)",
                &[&t, &apply_fn],
            )
            .await
            .unwrap();
        assert_eq!(r.get::<_, Option<bool>>(0), Some(true), "{t}: additive");
        assert_eq!(r.get::<_, Option<String>>(1).as_deref(), Some(check_fn));
        assert_eq!(r.get::<_, Option<bool>>(2), Some(true), "{t}: heal-safe");
    }
}

/// Section 1: a correction of an entry, a de-label at every date precision, and a withdrawal of
/// an entry, a none statement and a de-label are admitted, and each projects as signed.
#[tokio::test]
async fn every_overlay_submits_and_projects() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let rig = Rig::new(&c).await;
    let p = rig.patient;

    let first = rig.entry(&mut c, p, None).await.expect("an entry");
    let fixed = rig
        .entry(&mut c, p, Some(&first.to_string()))
        .await
        .expect("a correction of an entry is admitted");
    let refs: String = c
        .query_one(
            "SELECT refs::text FROM clinical_allergy WHERE event_id = $1::text::uuid",
            &[&fixed.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let refs: Value = serde_json::from_str(&refs).unwrap();
    assert_eq!(
        refs,
        json!([{"rel": "correction-of", "event": first.to_string()}])
    );

    let mut delabels = Vec::new();
    for date in [
        Onset::Day("2022-03-14".into()),
        Onset::Month("2022-03".into()),
        Onset::Year("2022".into()),
        Onset::NotKnown,
    ] {
        let id = rig
            .challenge(&mut c, first, date)
            .await
            .expect("a de-label is admitted at every precision");
        delabels.push(id);
    }
    let r = c
        .query_one(
            "SELECT target::text, tolerated_term, tolerated_coding IS NULL, date_precision, \
                    date_value, text, patient_id::text \
               FROM clinical_allergy_challenge WHERE event_id = $1::text::uuid",
            &[&delabels[1].to_string()],
        )
        .await
        .unwrap();
    assert_eq!(r.get::<_, String>(0), first.to_string());
    assert_eq!(r.get::<_, String>(1), "amoxicillin");
    assert!(r.get::<_, bool>(2), "uncoded reads as SQL NULL");
    assert_eq!(r.get::<_, String>(3), "month");
    assert_eq!(r.get::<_, String>(4), "2022-03");
    assert_eq!(r.get::<_, String>(5), "oral challenge");
    assert_eq!(r.get::<_, String>(6), p.to_string());

    let none = rig.none(&mut c).await;
    for target in [first, none, delabels[0]] {
        let id = rig
            .retraction(&mut c, target)
            .await
            .expect("a withdrawal of an entry, a none statement or a de-label is admitted");
        let r = c
            .query_one(
                "SELECT target::text, text FROM clinical_allergy_retraction \
                  WHERE event_id = $1::text::uuid",
                &[&id.to_string()],
            )
            .await
            .unwrap();
        assert_eq!(r.get::<_, String>(0), target.to_string());
        assert_eq!(r.get::<_, String>(1), "wrong patient");
    }
}

/// Section 2: the authoring door refuses an overlay aimed at the wrong thing or the wrong chart,
/// a withdrawal of a withdrawal (D3), a future date, a /2 schema, a filled coding, and a refs
/// that is not exactly one `correction-of`.
#[tokio::test]
async fn the_authoring_door_refuses_wrong_targets_and_words() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let rig = Rig::new(&c).await;
    let p = rig.patient;
    let other = rig.another_chart(&c).await;
    let entry = rig.entry(&mut c, p, None).await.unwrap();
    let foreign = rig.entry(&mut c, other, None).await.unwrap();
    let none = rig.none(&mut c).await;
    let withdrawal = rig.retraction(&mut c, entry).await.unwrap();

    let on_this_chart = "is not an allergy entry on this chart";
    refused(
        rig.entry(&mut c, p, Some(&none.to_string())).await,
        on_this_chart,
    );
    refused(
        rig.entry(&mut c, p, Some(&foreign.to_string())).await,
        on_this_chart,
    );
    refused(
        rig.entry(&mut c, p, Some("")).await,
        "refs must be [] or one",
    );
    refused(
        rig.challenge(&mut c, none, Onset::NotKnown).await,
        on_this_chart,
    );
    refused(
        rig.challenge(&mut c, foreign, Onset::NotKnown).await,
        on_this_chart,
    );
    refused(
        rig.retraction(&mut c, withdrawal).await,
        "a withdrawal is never itself withdrawn",
    );
    refused(
        rig.retraction(&mut c, foreign).await,
        "is not an allergy entry, none statement or de-label on this chart",
    );
    refused(
        rig.challenge(&mut c, entry, Onset::Year("2999".into()))
            .await,
        "date.value \"2999\" is in the future",
    );

    // Hand-shaped bodies the builders cannot produce.
    let twin = || "De-labelled by challenge: allergy record".to_string();
    let base_payload = |over: Value| challenge_body(p, entry, over)["payload"].clone();
    refused(
        rig.submit(
            &mut c,
            p,
            (ALLERGY_CHALLENGE_EVENT_TYPE, "clinical.allergy-challenge/2"),
            base_payload(json!({})),
            twin(),
        )
        .await,
        "schema_version",
    );
    refused(
        rig.submit(
            &mut c,
            p,
            CHALLENGE,
            base_payload(json!({"tolerated": {"term": "amoxicillin", "coding": {
                "system": "drugref-moiety", "code": "01900000-0000-7000-8000-000000000001",
                "display": "amoxicillin"}}})),
            twin(),
        )
        .await,
        "coding must be null",
    );
    let mut two_refs = allergy_payload(&entry_of(None));
    two_refs["refs"] = json!([
        {"rel": "correction-of", "event": entry.to_string()},
        {"rel": "correction-of", "event": entry.to_string()},
    ]);
    refused(
        rig.submit(
            &mut c,
            p,
            ENTRY,
            two_refs,
            render_allergy_twin(&entry_of(None)),
        )
        .await,
        "refs must be [] or one {rel: correction-of, event}",
    );
}

/// Section 3: the structural rules refuse at BOTH doors — a non-uuid target, a blank tolerated
/// term, a date value present with `not-known`. One also goes through the real strict door.
#[tokio::test]
async fn structural_rules_refuse_at_both_doors() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let rig = Rig::new(&c).await;
    let (p, entry) = (rig.patient, Uuid::now_v7());

    let cases = [
        (
            "cairn_check_allergy_challenge",
            challenge_body(p, entry, json!({"target": "E1"})),
            "target must be a uuid string",
        ),
        (
            "cairn_check_allergy_challenge",
            challenge_body(
                p,
                entry,
                json!({"tolerated": {"term": " \t", "coding": null}}),
            ),
            "tolerated.term must be a non-blank string",
        ),
        (
            "cairn_check_allergy_challenge",
            challenge_body(
                p,
                entry,
                json!({"date": {"precision": "not-known", "value": "2022"}}),
            ),
            "date.value must be null exactly when",
        ),
        (
            "cairn_check_allergy_retraction",
            json!({"schema_version": ALLERGY_RETRACTION_SCHEMA_VERSION,
                   "patient_id": p.to_string(), "payload": {"target": "", "text": null}}),
            "target must be a uuid string",
        ),
    ];
    for (check_fn, body, frag) in cases {
        for remote in [false, true] {
            let e = floor(&mut c, check_fn, body.clone(), remote)
                .await
                .expect_err("a structural rule refuses at both doors");
            assert!(e.contains(frag), "remote={remote}: {e} (wanted {frag:?})");
        }
    }

    let payload = challenge_body(p, entry, json!({"target": "not-a-uuid"}))["payload"].clone();
    refused(
        rig.submit(&mut c, p, CHALLENGE, payload, "De-labelled".into())
            .await,
        "target must be a uuid string",
    );
}

/// Section 4: the remote door admits a newer peer's date word, and a peer's refs of another rel
/// (D6: the reader ignores a ref it cannot parse, so the entry reads as uncorrected).
#[tokio::test]
async fn the_remote_door_admits_newer_words_and_refs() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (p, target) = (Uuid::now_v7(), Uuid::now_v7());

    let season = challenge_body(
        p,
        target,
        json!({"date": {"precision": "season", "value": "2022-winter"}}),
    );
    let mut entry = allergy_payload(&entry_of(None));
    entry["refs"] = json!([{"rel": "supersedes", "event": target.to_string()}]);
    let entry = json!({"schema_version": ALLERGY_SCHEMA_VERSION, "patient_id": p.to_string(),
                       "payload": entry});
    for (check_fn, body, frag) in [
        ("cairn_check_allergy_challenge", season, "date.precision"),
        (
            "cairn_check_allergy_assertion",
            entry,
            "refs must be [] or one",
        ),
    ] {
        let e = floor(&mut c, check_fn, body.clone(), false)
            .await
            .expect_err("the authoring door refuses it");
        assert!(e.contains(frag), "{e} (wanted {frag:?})");
        floor(&mut c, check_fn, body, true)
            .await
            .expect("the remote door admits it");
    }
}

/// Section 5: a shred scrubs a de-label's row and a withdrawal's row (db/037).
#[tokio::test]
async fn a_shred_scrubs_both_overlay_projections() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let rig = Rig::new(&c).await;
    let entry = rig.entry(&mut c, rig.patient, None).await.unwrap();
    let delabel = rig.challenge(&mut c, entry, Onset::NotKnown).await.unwrap();
    let withdrawal = rig.retraction(&mut c, delabel).await.unwrap();
    assert_eq!(rows(&c, "clinical_allergy_challenge", delabel).await, 1);
    assert_eq!(rows(&c, "clinical_allergy_retraction", withdrawal).await, 1);

    for id in [delabel, withdrawal] {
        shred_event(
            &mut c,
            &rig.node_sk,
            &rig.node_kid,
            &rig.node_kid,
            id,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");
    }
    assert_eq!(
        rows(&c, "clinical_allergy_challenge", delabel).await,
        0,
        "the tolerated term is clinical plaintext: the shred scrubs its row"
    );
    assert_eq!(
        rows(&c, "clinical_allergy_retraction", withdrawal).await,
        0,
        "the withdrawal's reason is clinical plaintext too"
    );
}
