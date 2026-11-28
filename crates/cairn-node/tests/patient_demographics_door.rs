//! #1211 — the door for the demographics a registration does not record.
//!
//! `patient::demographic::assert_demographics` writes a stated sex, a gender identity, a sex
//! at birth, an address, or a further name with its use, on an EXISTING chart, through the
//! real `submit_event` door. Before it, those fields existed in the floor (db/011, db/013,
//! db/014) and no command could reach them.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::patient::demographic::{self, DemographicFact};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding, for `submit_registration`: since #345 the first event on a chart must
// be its registration (#120/#327 — one copy).
mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (cairn_event::SigningKey, String, Uuid) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_address CASCADE",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    let patient = Uuid::now_v7();
    common::submit_registration(c, &sk, &kid, patient, 0).await;
    (sk, kid, patient)
}

async fn field_of(c: &Client, patient: Uuid, field: &str) -> Option<(String, String)> {
    let p = patient.to_string();
    c.query_opt(
        "SELECT value, provenance FROM patient_demographic \
         WHERE patient_id = $1::text::uuid AND field = $2",
        &[&p, &field],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

#[tokio::test]
async fn a_stated_sex_and_an_address_land_on_the_chart_in_one_act() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, patient) = setup(&db).await;

    let facts = [
        DemographicFact::AdministrativeSex {
            value: "female".into(),
        },
        DemographicFact::Address {
            display: "197 Kurrajong Place, SOLDIERS HILL VIC 3350".into(),
            use_: Some("residential".into()),
        },
    ];
    let ids = demographic::assert_demographics(
        &mut db,
        &sk,
        &kid,
        "test-node",
        patient,
        &facts,
        "patient-stated",
    )
    .await
    .unwrap();
    assert_eq!(ids.len(), 2, "one event per fact, in the order given");

    assert_eq!(
        field_of(&db, patient, "administrative-sex").await,
        Some(("female".into(), "patient-stated".into()))
    );
    let p = patient.to_string();
    let row = db
        .query_one(
            "SELECT display, use_raw, provenance FROM patient_address \
             WHERE patient_id = $1::text::uuid",
            &[&p],
        )
        .await
        .unwrap();
    assert_eq!(
        row.get::<_, String>(0),
        "197 Kurrajong Place, SOLDIERS HILL VIC 3350"
    );
    assert_eq!(
        row.get::<_, Option<String>>(1).as_deref(),
        Some("residential")
    );
    assert_eq!(row.get::<_, String>(2), "patient-stated");
}

/// The gap this door closes: before it, the only way to record a sex was
/// `assert-observed-evidence`, which can only say "a clinician looked" (rank 30). What the
/// patient states (rank 50) must win over what a clinician guessed, in either order.
#[tokio::test]
async fn what_the_patient_states_outranks_what_a_clinician_observed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, patient) = setup(&db).await;

    let stated = [DemographicFact::AdministrativeSex { value: "X".into() }];
    let observed = [DemographicFact::AdministrativeSex { value: "M".into() }];
    demographic::assert_demographics(
        &mut db,
        &sk,
        &kid,
        "test-node",
        patient,
        &stated,
        "patient-stated",
    )
    .await
    .unwrap();
    demographic::assert_demographics(
        &mut db,
        &sk,
        &kid,
        "test-node",
        patient,
        &observed,
        "clinician-observed",
    )
    .await
    .unwrap();

    assert_eq!(
        field_of(&db, patient, "administrative-sex").await,
        Some(("X".into(), "patient-stated".into())),
        "a later, weaker claim must not displace the patient's own statement"
    );
}

#[tokio::test]
async fn gender_identity_sex_at_birth_and_a_named_use_are_all_recordable() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, patient) = setup(&db).await;

    let facts = [
        DemographicFact::GenderIdentity {
            value: "non-binary".into(),
        },
        DemographicFact::SexAtBirth {
            value: "unknown".into(),
        },
        DemographicFact::Name {
            value: "Biwei Chipp".into(),
            use_: Some("legal".into()),
        },
    ];
    demographic::assert_demographics(
        &mut db,
        &sk,
        &kid,
        "test-node",
        patient,
        &facts,
        "patient-stated",
    )
    .await
    .unwrap();

    assert_eq!(
        field_of(&db, patient, "gender-identity").await.map(|r| r.0),
        Some("non-binary".into())
    );
    // "unknown" is a recordable value, distinct from never having been asked (principle 4).
    assert_eq!(
        field_of(&db, patient, "sex-at-birth").await.map(|r| r.0),
        Some("unknown".into())
    );
    let p = patient.to_string();
    let name: String = db
        .query_one(
            "SELECT value FROM patient_name WHERE patient_id = $1::text::uuid \
             AND use_raw = 'legal'",
            &[&p],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(name, "Biwei Chipp");
}

/// A chart that was never registered cannot be given demographics: the floor's precedence
/// rule (db/005 step 8b, #345) refuses it, and nothing is left behind.
#[tokio::test]
async fn an_unregistered_chart_is_refused_and_nothing_is_written() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut db = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _registered) = setup(&db).await;
    let stranger = Uuid::now_v7();

    let facts = [
        DemographicFact::AdministrativeSex { value: "F".into() },
        DemographicFact::Address {
            display: "1 Nowhere Street".into(),
            use_: None,
        },
    ];
    let outcome = demographic::assert_demographics(
        &mut db,
        &sk,
        &kid,
        "test-node",
        stranger,
        &facts,
        "patient-stated",
    )
    .await;
    assert!(
        outcome.is_err(),
        "the floor must refuse an unregistered chart"
    );
    let s = stranger.to_string();
    let n: i64 = db
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&s],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the facts commit together or not at all");
}
