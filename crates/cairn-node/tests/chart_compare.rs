//! Repair path R2b-1 — the side-by-side comparison's node read (`patient::compare`).
//!
//! Two paper front sheets laid next to each other: every member chart of each record, with
//! EVERY name it carries (not just the display winner), its earlier recorded names apart,
//! DOB and sex-at-birth with provenance, identifiers and addresses. Real Postgres, gated on
//! `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
mod common;
use cairn_event::demographics::{
    address_assertion_body, dob_assertion_body, identifier_assertion_body, name_assertion_body,
    render_address_twin, render_dob_twin, render_identifier_twin, render_name_twin,
    render_sex_at_birth_twin, sex_at_birth_assertion_body, AddressAssertion, IdentifierAssertion,
};
use cairn_event::identity::{
    render_repudiate_twin, repudiation_assertion_body, RepudiationAssertion,
};
use cairn_event::{ClockGrade, EventBody, Hlc};
use cairn_medication_view::ChartSet;
use cairn_node::db;
use cairn_node::patient::compare::chart_facts;
use common::{
    cs, enroll_human, medication_setup as setup, submit_attested, submit_link_event,
    submit_registration, submit_signed, EventSpec,
};
use uuid::Uuid;

/// One demographic-field event (name / dob / sex / address) at `wall`.
async fn field(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    p: Uuid,
    wall: i64,
    payload: serde_json::Value,
    twin: String,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload,
            plaintext_twin: Some(twin),
            wall,
        },
    )
    .await
    .expect("demographic field accepted");
}

async fn fresh(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 1).await;
    p
}

#[tokio::test]
async fn every_fact_of_a_chart_is_read_with_its_provenance() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = fresh(&c, &sk, &kid).await;
    field(
        &c,
        &sk,
        &kid,
        p,
        2,
        name_assertion_body("Mary SMITH", Some("legal"), "patient-stated"),
        render_name_twin("Mary SMITH", Some("legal"), "patient-stated"),
    )
    .await;
    field(
        &c,
        &sk,
        &kid,
        p,
        3,
        name_assertion_body("Mary JONES", Some("maiden"), "patient-stated"),
        render_name_twin("Mary JONES", Some("maiden"), "patient-stated"),
    )
    .await;
    field(
        &c,
        &sk,
        &kid,
        p,
        4,
        dob_assertion_body("1950-07-01", "day", Some("document"), "document-verified"),
        render_dob_twin("1950-07-01", "day", "document-verified"),
    )
    .await;
    field(
        &c,
        &sk,
        &kid,
        p,
        5,
        sex_at_birth_assertion_body("female", "patient-stated"),
        render_sex_at_birth_twin("female", "patient-stated"),
    )
    .await;
    let addr = AddressAssertion {
        display: "1 Main St, Bamaga",
        provenance: "patient-stated",
        use_: Some("residential"),
        geo: None,
        structured: None,
    };
    field(
        &c,
        &sk,
        &kid,
        p,
        6,
        address_assertion_body(&addr),
        render_address_twin(&addr),
    )
    .await;
    let id = IdentifierAssertion {
        value: "1234 56789 0",
        system: "au-medicare",
        provenance: "document-verified",
        normalized: None,
        profile: None,
        use_: None,
    };
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "demographic.identifier.asserted",
            schema_version: "demographic.identifier/1",
            payload: identifier_assertion_body(&id),
            plaintext_twin: Some(render_identifier_twin(&id)),
            wall: 7,
        },
    )
    .await
    .expect("identifier accepted");

    let facts = chart_facts(&c, &ChartSet::single(p)).await.unwrap();
    assert_eq!(facts.len(), 1);
    let f = &facts[0];
    assert_eq!(f.patient_id, p);
    assert!(f.held, "a registered chart is held here");
    assert_eq!(f.trust, "confirmed");
    // EVERY retained name, legal first — a maiden name is often precisely the clue.
    let names: Vec<(&str, Option<&str>)> = f
        .names
        .iter()
        .map(|n| (n.value.as_str(), n.use_.as_deref()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("Mary SMITH", Some("legal")),
            ("Mary JONES", Some("maiden"))
        ]
    );
    assert!(f.aliases.is_empty());
    let dob = f.dob.as_ref().expect("a dob was asserted");
    assert_eq!(
        (dob.value.as_str(), dob.provenance.as_str()),
        ("1950-07-01", "document-verified")
    );
    assert_eq!(
        f.sex_at_birth.as_ref().map(|s| s.value.as_str()),
        Some("female")
    );
    assert_eq!(f.identifiers.len(), 1);
    assert_eq!(f.identifiers[0].system, "au-medicare");
    assert_eq!(f.addresses.len(), 1);
    assert_eq!(f.addresses[0].use_.as_deref(), Some("residential"));
}

#[tokio::test]
async fn a_repudiated_name_moves_to_the_aliases() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let p = fresh(&c, &sk, &kid).await;
    field(
        &c,
        &sk,
        &kid,
        p,
        2,
        name_assertion_body("John DOE", Some("legal"), "patient-stated"),
        render_name_twin("John DOE", Some("legal"), "patient-stated"),
    )
    .await;
    field(
        &c,
        &sk,
        &kid,
        p,
        3,
        name_assertion_body("Jack DOE", Some("legal"), "patient-stated"),
        render_name_twin("Jack DOE", Some("legal"), "patient-stated"),
    )
    .await;
    let s = p.to_string();
    let r = RepudiationAssertion {
        subject: &s,
        value: "John DOE",
        reason: "confessed fabricated persona",
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: s.clone(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall: 10,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([
            {"actor_id": kid_h, "role": "attested", "responsibility": {"held_by": kid_h}}
        ]),
        payload: repudiation_assertion_body(&r),
        attachments: vec![],
        plaintext_twin: Some(render_repudiate_twin(&r)),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    submit_attested(&c, &sk, body, &sk_h, &kid_h)
        .await
        .expect("repudiation accepted");

    let f = &chart_facts(&c, &ChartSet::single(p)).await.unwrap()[0];
    assert!(
        f.names.iter().all(|n| n.value != "John DOE"),
        "a repudiated name is not a current name"
    );
    assert_eq!(
        f.aliases,
        vec!["John DOE".to_string()],
        "it is kept, apart, as an earlier name"
    );
}

#[tokio::test]
async fn a_linked_set_reads_every_member_and_an_unheld_one_says_so() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let unheld = Uuid::now_v7(); // named by a link; its registration never arrived here
    submit_link_event(&c, &sk, &kid, a, unheld, 10, true).await;
    let set = ChartSet::new([a, unheld]).unwrap();

    let facts = chart_facts(&c, &set).await.unwrap();
    assert_eq!(
        facts.iter().map(|f| f.patient_id).collect::<Vec<_>>(),
        set.members().to_vec(),
        "one entry per member, in the set's own order"
    );
    let u = facts.iter().find(|f| f.patient_id == unheld).unwrap();
    assert!(!u.held);
    assert_eq!(
        u.trust, "unknown",
        "no row about an unheld chart is not evidence of 'confirmed'"
    );
    assert!(
        u.names.is_empty() && u.dob.is_none(),
        "absent, never invented"
    );
}
