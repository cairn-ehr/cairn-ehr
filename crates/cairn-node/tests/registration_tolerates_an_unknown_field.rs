//! A registration carrying a payload field this build does not know is ADMITTED, at both doors,
//! and the chart it registers is an ordinary chart.
//!
//! # Why this is pinned
//!
//! ADR-0115 puts "this patient is synthetic" in a new field of the registration event, and its
//! "Not guaranteed" section says what a node on an OLDER build does with such a registration:
//! it ignores the field and shows the chart as a real one. That sentence was first written
//! from reading db/045, which is how ADR-0114 came to rest on a false premise (#1220). This
//! file is the same sentence, run: today's build IS the older build with respect to a field
//! it has never heard of.
//!
//! It also pins the additive-evolution rule (principle 11) for the registration floor in
//! general. `cairn_check_registration_assertion` checks the keys it knows and must not grow a
//! "no other keys" rule: that would turn every future additive field into a registration an
//! older node refuses, and since a chart's other events still arrive through the lenient door
//! (db/005 step 8b binds the strict door only), the chart would then exist on that node with
//! no registration at all.
//!
//! The field is named `some_field_from_a_later_build` on purpose. When ADR-0115 is built and
//! `synthetic` becomes a field this build DOES know, this test must go on meaning "unknown".
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name CASCADE",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    (sk, kid)
}

/// A standard registration of `patient`, signed, whose payload carries one extra field.
fn registration_with_an_unknown_field(sk: &SigningKey, kid: &str, patient: Uuid) -> Vec<u8> {
    let tokens = [patient.to_string()];
    let a = RegistrationAssertion {
        class: RegistrationClass::Standard,
        basis: None,
        search: Some(SearchAttestationInput {
            terms: SearchTerms {
                name_tokens: &tokens,
                birth_date: None,
                identifiers: &[],
            },
            displayed: &[],
            incomplete: false,
        }),
    };
    let mut payload = registration_assertion_body(&a);
    payload
        .as_object_mut()
        .expect("a registration payload is an object")
        .insert("some_field_from_a_later_build".into(), true.into());
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: REGISTRATION_EVENT_TYPE.into(),
        schema_version: REGISTRATION_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "another-node".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        // A later build may also word the twin differently. The floor requires a twin to be
        // present, never that it equal a derivation of its own, and this asserts that too.
        plaintext_twin: Some(format!(
            "{} [and something a later build says]",
            render_registration_twin(&a)
        )),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

async fn registration_rows(c: &Client, patient: Uuid) -> i64 {
    let p = patient.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_registration WHERE patient_id = $1::text::uuid",
        &[&p],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn the_local_door_admits_it_and_the_chart_is_registered() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let signed = registration_with_an_unknown_field(&sk, &kid, patient);
    db.execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect("the strict door admits a registration with a field it does not know");

    assert_eq!(registration_rows(&db, patient).await, 1);
}

/// The door a synced registration actually arrives through. A refusal here would be the
/// worse outcome: the registration penned, and the chart's other events admitted without it.
#[tokio::test]
async fn the_remote_door_admits_it_and_the_chart_is_registered() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&db).await;
    let patient = Uuid::now_v7();

    let signed = registration_with_an_unknown_field(&sk, &kid, patient);
    db.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("the remote door admits a registration with a field it does not know");

    assert_eq!(registration_rows(&db, patient).await, 1);
    // The unknown field is KEPT in the stored body, not stripped: when this node is upgraded
    // and reprojects, the fact is still there to be read (ADR-0057).
    let p = patient.to_string();
    let kept: bool = db
        .query_one(
            "SELECT COALESCE((body ->> 'some_field_from_a_later_build')::boolean, false) \
               FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        kept,
        "the stored body must carry the field for a later reprojection"
    );
}
