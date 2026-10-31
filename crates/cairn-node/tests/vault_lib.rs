//! DB-gated: `cairn_node::vault`, the Rust side of the clinicians' key vault (db/061).
//!
//! The SQL floor is pinned by `vault_floor.rs`; this file pins the LIBRARY over it: that a key
//! enrolled through `enrol_member` and released through `release_for_act` really signs an event
//! the strict floor accepts, that a superuser cannot be bootstrapped from a clinician role, and
//! that the library refuses a released seed that does not match the key id it was filed under.
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use cairn_node::vault::{
    bootstrap_superuser, enrol_member, release_for_act, NewMember, Purpose, Release,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Serialised, freshly-loaded connection with an empty vault + registry, plus this node's own
/// device actor and a registered unwrap key (what `medication_authorship.rs::setup` arranges).
async fn fresh() -> (Client, Client, cairn_event::SigningKey, String) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, event_log, actor_event, \
         patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (node_sk, node_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&node_kid],
    )
    .await
    .unwrap();
    let unwrap = cairn_event::seal::generate_unwrap_secret().unwrap();
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&unwrap)
            .as_bytes()
            .as_slice()],
    )
    .await
    .unwrap();
    (guard, c, node_sk, node_kid)
}

fn member<'a>(staff_id: &'a str, role: &'a str, passphrase: &'a str) -> NewMember<'a> {
    NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: Some("a hint"),
    }
}

const SU_PASS: &str = "boss passphrase";
const CLIN_PASS: &str = "nurse passphrase";

/// Bootstrap `boss`, release its key for an enrolment, return that key as the superuser signer.
async fn boss(c: &Client) -> cairn_event::SigningKey {
    bootstrap_superuser(c, &member("boss", "superuser", SU_PASS))
        .await
        .unwrap();
    match release_for_act(c, "boss", SU_PASS, Purpose::Enrol)
        .await
        .unwrap()
    {
        Release::Released { key, .. } => key,
        other => panic!("expected the superuser's key, got {other:?}"),
    }
}

#[tokio::test]
async fn a_vault_key_signs_an_event_the_floor_accepts() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, mut c, node_sk, node_kid) = fresh().await;
    let su = boss(&c).await;
    enrol_member(&c, &member("JSmith", "clinician", CLIN_PASS), "boss", &su)
        .await
        .unwrap();

    // Case/whitespace in the typed staff ID must still find the member.
    let (key, kid) = match release_for_act(&c, " jsmith ", CLIN_PASS, Purpose::Clinical)
        .await
        .unwrap()
    {
        Release::Released { key, kid } => (key, kid),
        other => panic!("expected release, got {other:?}"),
    };

    let patient = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, patient, 0).await;
    let input = AssertMedicationInput {
        term: "atorvastatin",
        coding: None,
        formulation: None,
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let author = AuthorParams {
        human_sk: &key,
        human_kid: &kid,
    };
    let med_id = assert_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        &input,
        Some(&author),
        None,
    )
    .await
    .expect("the floor accepts an event signed by a released vault key");

    // Same assertions as medication_authorship.rs: signed by the human, who is the 'authored' contributor.
    let row = c
        .query_one(
            "SELECT signer_key_id, contributors::text FROM event_log \
             WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("signer_key_id"), kid);
    let contributors: serde_json::Value =
        serde_json::from_str(&row.get::<_, String>("contributors")).unwrap();
    assert_eq!(contributors[0]["actor_id"], kid);
    assert_eq!(contributors[0]["role"], "authored");
}

#[tokio::test]
async fn an_outcome_per_answer_and_a_superuser_is_not_a_clinician() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c, _, _) = fresh().await;
    let su = boss(&c).await;
    enrol_member(&c, &member("jsmith", "clinician", CLIN_PASS), "boss", &su)
        .await
        .unwrap();
    assert!(matches!(
        release_for_act(&c, "jsmith", "not it", Purpose::Clinical)
            .await
            .unwrap(),
        Release::Wrong { retry_after: None }
    ));
    assert!(matches!(
        release_for_act(&c, "nobody", "x", Purpose::Clinical)
            .await
            .unwrap(),
        Release::Unknown
    ));
    assert!(matches!(
        release_for_act(&c, "boss", SU_PASS, Purpose::Clinical)
            .await
            .unwrap(),
        Release::RefusedPurpose
    ));
}

#[tokio::test]
async fn bootstrap_refuses_a_non_superuser_role() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c, _, _) = fresh().await;
    let err = bootstrap_superuser(&c, &member("x", "clinician", "pw"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("superuser"), "{err}");
}

#[tokio::test]
async fn debug_never_prints_the_key() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c, _, _) = fresh().await;
    let su = boss(&c).await;
    let shown = format!(
        "{:?}",
        Release::Released {
            key: su,
            kid: "kid".into()
        }
    );
    assert!(shown.contains("redacted"), "{shown}");
}

#[tokio::test]
async fn a_seed_that_does_not_match_its_key_id_is_never_handed_out() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c, _, _) = fresh().await;
    let su = boss(&c).await;
    enrol_member(&c, &member("jsmith", "clinician", CLIN_PASS), "boss", &su)
        .await
        .unwrap();
    // As owner, file jsmith's sealed key under somebody else's key id.
    let (_, other_kid) = generate_key().unwrap();
    c.batch_execute("ALTER TABLE vault_member DISABLE TRIGGER USER")
        .await
        .unwrap();
    let r = c
        .execute(
            "UPDATE vault_member SET signing_key_id = $1 WHERE staff_id = 'jsmith'",
            &[&other_kid],
        )
        .await;
    c.batch_execute("ALTER TABLE vault_member ENABLE TRIGGER USER")
        .await
        .unwrap();
    r.expect("the owner can corrupt a row for this test");
    let err = release_for_act(&c, "jsmith", CLIN_PASS, Purpose::Clinical)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("does not match"), "{err}");
}
