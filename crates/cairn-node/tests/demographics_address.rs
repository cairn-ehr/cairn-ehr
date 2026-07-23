//! Integration coverage for the §4.3 ADDRESS field: the retained-set patient_address
//! projection (most-recent-assertion-per-member) + the patient_address_current per-use
//! display-winner VIEW (recency-first within each use). Real Postgres, gated on
//! `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`. Matching
//! (§5.2) is a separate subsystem, not exercised here.
use cairn_event::demographics::{
    address_assertion_body, render_address_twin, AddressAssertion, Geo, StructuredAddress,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_address CASCADE",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one demographic.field.asserted event with an explicit HLC.
#[allow(clippy::too_many_arguments)]
async fn submit(
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

/// The current display address for a (patient, use). NULL-safe.
async fn current_for_use(c: &Client, p: Uuid, use_key: &str) -> Option<String> {
    let p_str = p.to_string();
    c.query_opt(
        "SELECT display FROM patient_address_current WHERE patient_id::text=$1 AND use_key=$2",
        &[&p_str, &use_key],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// Count retained address members for a patient.
async fn addr_count(c: &Client, p: Uuid) -> i64 {
    let p_str = p.to_string();
    c.query_one(
        "SELECT count(*) FROM patient_address WHERE patient_id::text=$1",
        &[&p_str],
    )
    .await
    .unwrap()
    .get(0)
}

fn addr<'a>(display: &'a str, use_: Option<&'a str>, prov: &'a str) -> AddressAssertion<'a> {
    AddressAssertion {
        display,
        provenance: prov,
        use_,
        geo: None,
        structured: None,
    }
}

async fn submit_addr(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    wall: i64,
    a: &AddressAssertion<'_>,
) -> Result<u64, tokio_postgres::Error> {
    submit(
        c,
        sk,
        kid,
        p,
        wall,
        0,
        address_assertion_body(a),
        &render_address_twin(a),
    )
    .await
}

#[tokio::test]
async fn happy_path_display_only() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    let a = addr(
        "12 Main St, Springfield",
        Some("residential"),
        "patient-stated",
    );
    submit_addr(&c, &sk, &kid, p, 1, &a).await.unwrap();
    assert_eq!(addr_count(&c, p).await, 1);
    assert_eq!(
        current_for_use(&c, p, "residential").await.as_deref(),
        Some("12 Main St, Springfield")
    );
}

#[tokio::test]
async fn geo_and_structured_carried_into_retained_set() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    let a = AddressAssertion {
        display: "12 Main St, Springfield",
        provenance: "patient-stated",
        use_: Some("residential"),
        geo: Some(Geo {
            lat: -33.87,
            lon: 151.21,
            accuracy_m: 10.0,
            basis: "device_gps",
        }),
        structured: Some(StructuredAddress {
            profile: "au-address@b3-xyz",
            parts: json!({ "town": "Springfield", "country": "AU" }),
        }),
    };
    submit_addr(&c, &sk, &kid, p, 1, &a).await.unwrap();
    let p_str = p.to_string();
    let (geo, structured): (serde_json::Value, serde_json::Value) =
        {
            // Cast JSONB to TEXT so tokio-postgres (without with-serde_json-1 feature) can
            // receive them as Strings; parse into Value for the field assertions below.
            let row = c.query_one(
            "SELECT geo::text, structured::text FROM patient_address WHERE patient_id::text=$1",
            &[&p_str]).await.unwrap();
            let geo_s: String = row.get(0);
            let str_s: String = row.get(1);
            (
                serde_json::from_str(&geo_s).unwrap(),
                serde_json::from_str(&str_s).unwrap(),
            )
        };
    assert_eq!(geo["basis"], "device_gps");
    assert_eq!(geo["accuracy_m"], 10.0);
    assert_eq!(structured["profile"], "au-address@b3-xyz");
    assert_eq!(structured["parts"]["town"], "Springfield");
}

#[tokio::test]
async fn each_use_is_independently_current() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A residential AND a postal address are BOTH current — one winner per use.
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        1,
        &addr("12 Main St", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        2,
        &addr("PO Box 9", Some("postal"), "patient-stated"),
    )
    .await
    .unwrap();
    assert_eq!(
        current_for_use(&c, p, "residential").await.as_deref(),
        Some("12 Main St")
    );
    assert_eq!(
        current_for_use(&c, p, "postal").await.as_deref(),
        Some("PO Box 9")
    );
}

#[tokio::test]
async fn recency_beats_provenance_within_a_use() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Old, HIGHER-provenance address (document-verified, wall=1).
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        1,
        &addr("Old Rd 1", Some("residential"), "document-verified"),
    )
    .await
    .unwrap();
    // Newer, LOWER-provenance "I moved" (patient-stated, wall=2) — recency wins (volatile field).
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        2,
        &addr("New Rd 2", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();
    assert_eq!(
        current_for_use(&c, p, "residential").await.as_deref(),
        Some("New Rd 2"),
        "newer address wins display (recency beats provenance for a volatile field)"
    );
    assert_eq!(
        addr_count(&c, p).await,
        2,
        "the old address is retained, not overwritten"
    );
}

#[tokio::test]
async fn set_union_reassertion_is_idempotent_and_order_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Apply newer first, then older — winner must still be the newer (apply-order-independent),
    // and the same (use, display) re-asserted stays ONE member.
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        2,
        &addr("New Rd 2", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        1,
        &addr("Old Rd 1", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        3,
        &addr("New Rd 2", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();
    assert_eq!(
        addr_count(&c, p).await,
        2,
        "two distinct members; the re-assertion deduped"
    );
    assert_eq!(
        current_for_use(&c, p, "residential").await.as_deref(),
        Some("New Rd 2")
    );
}

#[tokio::test]
async fn floor_rejects_structured_without_profile() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // structured present without a profile violates the §4.3 structural invariant.
    let r = submit(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        json!({"field":"address","value":"X","provenance":"patient-stated",
               "facets":{"structured":{"parts":{"town":"Springfield"}}}}),
        "Address (patient-stated): X",
    )
    .await;
    assert!(
        r.is_err(),
        "structured-without-profile is rejected by the floor"
    );
    let p_str = p.to_string();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "rejected event is not appended");
    assert_eq!(
        addr_count(&c, p).await,
        0,
        "rejected event is not projected"
    );
}

#[tokio::test]
async fn floor_rejects_non_text_structured_part() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A non-text parts value (number) violates "parts values are opaque TEXT to the core".
    let r = submit(
        &c,
        &sk,
        &kid,
        p,
        1,
        0,
        json!({"field":"address","value":"X","provenance":"patient-stated",
               "facets":{"structured":{"profile":"au@b3","parts":{"postcode":2000}}}}),
        "Address (patient-stated): X",
    )
    .await;
    assert!(
        r.is_err(),
        "a non-text parts value is rejected by the floor"
    );
    assert_eq!(addr_count(&c, p).await, 0);
}

#[tokio::test]
async fn floor_rejects_malformed_geo() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Each malformed geo is its own isolated rejection: non-number lat, negative
    // accuracy_m, non-number accuracy_m (must yield the clean floor message, not a raw
    // ::numeric cast error — the floor checks typeof before casting), empty basis. None
    // may append or project.
    for facets in [
        json!({"geo":{"lat":"north","lon":151.2,"accuracy_m":10.0,"basis":"device_gps"}}),
        json!({"geo":{"lat":-33.8,"lon":151.2,"accuracy_m":-5.0,"basis":"device_gps"}}),
        json!({"geo":{"lat":-33.8,"lon":151.2,"accuracy_m":"north","basis":"device_gps"}}),
        json!({"geo":{"lat":-33.8,"lon":151.2,"accuracy_m":10.0,"basis":""}}),
    ] {
        let r = submit(
            &c,
            &sk,
            &kid,
            p,
            1,
            0,
            json!({"field":"address","value":"X","provenance":"declared","facets":facets}),
            "Address (declared): X",
        )
        .await;
        assert!(r.is_err(), "malformed geo must be rejected");
    }
    let p_str = p.to_string();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "no malformed-geo event appended");
    assert_eq!(addr_count(&c, p).await, 0);
}

#[tokio::test]
async fn unknown_field_and_legacy_unaffected() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A name event must NOT create a patient_address row; an address event must NOT
    // create a patient_name row. The projections are blind to each other's fields.
    submit(&c, &sk, &kid, p, 1, 0,
        json!({"field":"name","value":"Mary Jones","provenance":"patient-stated","facets":{"use":"legal"}}),
        "Name (legal): Mary Jones").await.unwrap();
    submit_addr(
        &c,
        &sk,
        &kid,
        p,
        2,
        &addr("12 Main St", Some("residential"), "patient-stated"),
    )
    .await
    .unwrap();

    let p_str = p.to_string();
    let names: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_name WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(names, 1, "name still projects only into patient_name");
    assert_eq!(
        addr_count(&c, p).await,
        1,
        "address projects only into patient_address"
    );
}
