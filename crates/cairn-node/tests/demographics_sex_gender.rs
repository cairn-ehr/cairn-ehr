//! Integration coverage for the §4.2 administrative-sex + gender-identity fields:
//! the per-field winner-policy selector (db/013) over the slice-2 patient_demographic
//! projection. administrative-sex is provenance-first (like dob/sex-at-birth);
//! gender-identity is recency-first (the inverse ordering). Real Postgres, gated on
//! `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`. Matching
//! (§5.2) is a separate subsystem and is NOT exercised here.
use cairn_event::demographics::{
    administrative_sex_assertion_body, gender_identity_assertion_body,
    render_administrative_sex_twin, render_gender_identity_twin,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the clinical tables and enroll one agent signer. Returns (sk, kid).
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, patient_demographic CASCADE")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one demographic.field.asserted event at HLC (wall, counter).
/// `counter` is explicit so a test can pin two assertions to the same wall and exercise
/// the recency-first sub-tiebreaks. Returns the raw submit result.
#[allow(clippy::too_many_arguments)]
async fn submit_field(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    counter: i32,
    payload: serde_json::Value,
    twin: &str,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin.to_string()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

async fn winner(c: &Client, patient: &Uuid, field: &str) -> String {
    let p = patient.to_string();
    c.query_one(
        "SELECT value FROM patient_demographic WHERE patient_id::text=$1 AND field=$2",
        &[&p, &field],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn administrative_sex_provenance_locks_then_recency_among_equals() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // document-verified marker first.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        administrative_sex_assertion_body("M", "document-verified"),
        &render_administrative_sex_twin("M", "document-verified"),
    )
    .await
    .unwrap();
    // a LATER patient-stated claim must NOT displace it (provenance-first lock).
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        0,
        administrative_sex_assertion_body("F", "patient-stated"),
        &render_administrative_sex_twin("F", "patient-stated"),
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "administrative-sex").await,
        "M",
        "lower-provenance later claim must not displace a document-verified marker"
    );

    // a later EQUAL-provenance (document-verified) marker DOES win on recency.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        3,
        0,
        administrative_sex_assertion_body("F", "document-verified"),
        &render_administrative_sex_twin("F", "document-verified"),
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "administrative-sex").await,
        "F",
        "a newer equal-provenance marker wins (recency-among-equals)"
    );
}

#[tokio::test]
async fn gender_identity_recency_wins_regardless_of_provenance() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // a high-provenance value first.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        gender_identity_assertion_body("man", "document-verified"),
        &render_gender_identity_twin("man", "document-verified"),
    )
    .await
    .unwrap();
    // a LATER but LOWER-provenance assertion still wins — recency leads (inverse of dob).
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        0,
        gender_identity_assertion_body("non-binary", "clinician-observed"),
        &render_gender_identity_twin("non-binary", "clinician-observed"),
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "gender-identity").await,
        "non-binary",
        "newest gender-identity wins regardless of provenance"
    );
}

#[tokio::test]
async fn gender_identity_equal_hlc_breaks_on_provenance() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Same (wall, counter): the recency-first tuple falls through to provenance_rank.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        7,
        0,
        gender_identity_assertion_body("A", "patient-stated"),
        &render_gender_identity_twin("A", "patient-stated"),
    )
    .await
    .unwrap();
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        7,
        0,
        gender_identity_assertion_body("B", "document-verified"),
        &render_gender_identity_twin("B", "document-verified"),
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "gender-identity").await,
        "B",
        "equal HLC: higher provenance breaks the recency-first tie (convergence)"
    );
}

#[tokio::test]
async fn administrative_sex_converges_regardless_of_apply_order() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // The same three assertions as the forward lock test, but applied NEWEST-first
    // (set-union sync delivers events in arbitrary order). The provenance-first tuple is
    // a total order, so the converged winner must be identical to forward order: the
    // newest document-verified marker (wall=3, "F").
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        3,
        0,
        administrative_sex_assertion_body("F", "document-verified"),
        &render_administrative_sex_twin("F", "document-verified"),
    )
    .await
    .unwrap();
    // an OLDER, lower-provenance claim arriving later must not displace it.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        0,
        administrative_sex_assertion_body("F", "patient-stated"),
        &render_administrative_sex_twin("F", "patient-stated"),
    )
    .await
    .unwrap();
    // an OLDER document-verified marker arriving last must not displace it either.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        administrative_sex_assertion_body("M", "document-verified"),
        &render_administrative_sex_twin("M", "document-verified"),
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "administrative-sex").await,
        "F",
        "winner is apply-order-independent: newest document-verified marker wins"
    );
}

#[tokio::test]
async fn backfill_projects_carried_events_after_upgrade() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Two gender-identity assertions land in event_log; recency-first => "non-binary" wins.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        gender_identity_assertion_body("man", "document-verified"),
        &render_gender_identity_twin("man", "document-verified"),
    )
    .await
    .unwrap();
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        0,
        gender_identity_assertion_body("non-binary", "patient-stated"),
        &render_gender_identity_twin("non-binary", "patient-stated"),
    )
    .await
    .unwrap();

    // Simulate the pre-upgrade state: under db/011 these were carried in event_log but
    // NOT projected (the field had no policy). Wipe the projection row, leaving the
    // retained assertion set intact — the trigger will not re-fire for existing rows.
    let p_str = p.to_string();
    c.execute(
        "DELETE FROM patient_demographic WHERE patient_id::text=$1 AND field='gender-identity'",
        &[&p_str],
    )
    .await
    .unwrap();
    let before: i64 = c.query_one(
        "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1 AND field='gender-identity'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(
        before, 0,
        "precondition: carried-not-projected (no projection row)"
    );

    // The generic cairn_reproject (db/039, ADR-0057) re-folds the retained set into the
    // projection (idempotent) — it replaces the bespoke, db/013-only
    // cairn_demographic_backfill() this test used to call (#208): same heal semantics,
    // driven through the registered patient_demographic_apply fn instead of a
    // hand-duplicated re-projection query.
    c.execute(
        "SELECT count(*) FROM cairn_reproject('demographic.field', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "gender-identity").await,
        "non-binary",
        "reproject restores the policy-correct (recency-first) winner from event_log"
    );

    // Idempotent: a second run does not change or downgrade the healed winner.
    c.execute(
        "SELECT count(*) FROM cairn_reproject('demographic.field', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(
        winner(&c, &p, "gender-identity").await,
        "non-binary",
        "reproject is idempotent — re-running yields the same winner"
    );
}

#[tokio::test]
async fn unknown_field_is_carried_but_not_projected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A field this node has no policy for: passes the floor, lands in event_log, but
    // is NOT projected (the ADR-0012 federation-forward degrade is intact).
    let body = cairn_event::demographics::demographic_field_body(
        "gender-marker-v2",
        "x",
        None,
        "patient-stated",
    );
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        body,
        "Gender marker v2 (patient-stated): x",
    )
    .await
    .expect("unknown field passes the generic floor");

    let p_str = p.to_string();
    let in_log: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1 AND body->>'field'='gender-marker-v2'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(in_log, 1, "unknown field is carried in event_log");
    let projected: i64 = c.query_one(
        "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1 AND field='gender-marker-v2'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(projected, 0, "unknown field is not projected");
}
