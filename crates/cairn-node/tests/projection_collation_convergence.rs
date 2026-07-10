//! #69 — collation-independent projection winner tiebreaks. Each test constructs an exact
//! (rank, wall, counter) tie whose remaining TEXT tiebreak key holds a pair that orders
//! OPPOSITELY under "C" vs a locale (ICU) collation ('B' vs 'a'), then asserts the projection
//! picks the "C"-order winner in BOTH arrival orders — proving convergence is collation-
//! independent, not merely in-DB deterministic. Real Postgres, gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard.
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
    )
    .await
    .unwrap();
    (sk, kid)
}

/// True iff a locale (ICU) collation orders `hi > lo` — the guard that the chosen pair
/// really does flip vs "C" (where the projection must instead pick the byte-order winner).
async fn locale_flips(c: &Client, hi: &str, lo: &str) -> bool {
    c.query_one(
        "SELECT $1::text COLLATE \"unicode\" > $2::text COLLATE \"unicode\"",
        &[&hi, &lo],
    )
    .await
    .unwrap()
    .get(0)
}

/// Author + sign + submit one event with the wire HLC set verbatim from (wall, counter, origin).
/// submit_event stores the wire HLC as-is (db/005), so the projection sees exactly these values.
#[allow(clippy::too_many_arguments)]
async fn submit_generic(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    event_type: &str,
    wall: i64,
    counter: i32,
    origin: &str,
    payload: serde_json::Value,
    twin: &str,
) {
    // schema_version tracks event_type, not the other way round: identifier and field
    // events version independently, so the helper (shared by both) must derive it here
    // rather than hardcode one — else identifier events would carry the field schema's tag.
    let schema_version = if event_type == "demographic.identifier.asserted" {
        "demographic.identifier/1"
    } else {
        "demographic.field/1"
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: origin.into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin.into()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

/// A demographic.identifier.asserted payload (§4.4). One system, same normalized value on
/// both events so the (patient, system, match_key) PK collides — leaving the winner to the
/// (wall, counter, origin, value) tiebreak.
fn identifier_payload(system: &str, value: &str) -> serde_json::Value {
    json!({
        "system": system,
        "value": value,
        "use": "official",
        "provenance": "document-verified"
    })
}

/// #69: patient_identifier resolves an equal-(wall,counter) cross-origin tie by origin under
/// COLLATE "C". Origins 'B' vs 'a' flip between "C" and a locale collation; the retained
/// representative must be the byte-order winner ('a') regardless of apply order.
#[tokio::test]
async fn identifier_origin_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(
        locale_flips(&c, "B", "a").await,
        "pair must flip under a locale collation"
    );

    // Two arrival orders → both must land on origin 'a' (the C byte-order winner).
    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Same value ("ABC123") → same match_key → same PK; same (wall,counter); origin differs.
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.identifier.asserted",
            5,
            0,
            first,
            identifier_payload("ns:test", "ABC123"),
            "id ABC123",
        )
        .await;
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.identifier.asserted",
            5,
            0,
            second,
            identifier_payload("ns:test", "ABC123"),
            "id ABC123",
        )
        .await;

        let origin: String = c
            .query_one(
                "SELECT asserted_origin FROM patient_identifier WHERE patient_id = $1::text::uuid",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            origin, "a",
            "C byte-order winner regardless of arrival order {first}->{second}"
        );
    }
}
