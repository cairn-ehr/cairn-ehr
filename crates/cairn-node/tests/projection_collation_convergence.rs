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

/// A demographic.field.asserted payload (§4.2). `facets` carries dob precision when needed.
fn field_payload(field: &str, value: &str, provenance: &str) -> serde_json::Value {
    let mut p = json!({"field": field, "value": value, "provenance": provenance});
    if field == "dob" {
        p["facets"] = json!({"precision": "day"});
    }
    p
}

/// #69: patient_demographic breaks an equal-(rank,wall,counter,origin) tie on `value` under
/// COLLATE "C", in BOTH winner-policy branches. Values 'B'/'a' flip between "C" and a locale
/// collation; the projected winner must be the byte-order winner ('a') regardless of arrival order.
#[tokio::test]
async fn demographic_value_tiebreak_is_collation_independent_both_branches() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    // ("dob", ...) exercises the provenance-first branch; ("gender-identity", ...) the recency-first.
    for field in ["dob", "gender-identity"] {
        for (first, second) in [("B", "a"), ("a", "B")] {
            let (sk, kid) = setup(&c).await;
            let p = Uuid::now_v7();
            // Same field+provenance (→ equal rank), same (wall,counter,origin); value differs.
            submit_generic(
                &c,
                &sk,
                &kid,
                p,
                "demographic.field.asserted",
                9,
                0,
                "n",
                field_payload(field, first, "patient-stated"),
                &format!("{field} {first}"),
            )
            .await;
            submit_generic(
                &c,
                &sk,
                &kid,
                p,
                "demographic.field.asserted",
                9,
                0,
                "n",
                field_payload(field, second, "patient-stated"),
                &format!("{field} {second}"),
            )
            .await;

            let value: String = c
                .query_one(
                    "SELECT value FROM patient_demographic \
                     WHERE patient_id = $1::text::uuid AND field = $2",
                    &[&p.to_string(), &field],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                value, "a",
                "{field}: C byte-order winner for {first}->{second}"
            );
        }
    }
}

/// A demographic.field.asserted name payload (§4.2). `use` selects the legal tier.
fn name_payload(value: &str, name_use: &str, provenance: &str) -> serde_json::Value {
    json!({"field": "name", "value": value, "provenance": provenance,
           "facets": {"use": name_use}})
}

/// #69: patient_name_current picks its DISPLAY name across equal-(rank,wall,counter,origin)
/// members by `value` under COLLATE "C". Values 'B'/'a' flip vs a locale collation; the
/// displayed name must be the byte-order winner ('a').
#[tokio::test]
async fn name_display_value_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Two legal names, equal (wall,counter,provenance,origin); values differ → VIEW tiebreak.
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            3,
            0,
            "n",
            name_payload(first, "legal", "patient-stated"),
            &format!("name {first}"),
        )
        .await;
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            3,
            0,
            "n",
            name_payload(second, "legal", "patient-stated"),
            &format!("name {second}"),
        )
        .await;

        let value: String = c
            .query_one(
                "SELECT value FROM patient_name_current WHERE patient_id = $1::text::uuid",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            value, "a",
            "displayed name is the C byte-order winner for {first}->{second}"
        );
    }
}

/// #69 review: `cairn_demographic_backfill()` (db/013) re-projects `demographic.field.asserted`
/// events straight from `event_log` — a SEPARATE code path from the `patient_demographic_apply()`
/// trigger exercised above, used for one-time catch-up when a node gains projection capability
/// for a field it previously only carried. It got the identical `COLLATE "C"` fix in its
/// `DISTINCT ON ... ORDER BY node_origin COLLATE "C" DESC, value COLLATE "C" DESC` and in both
/// branches of its `ON CONFLICT ... WHERE` CASE, but shipped without a test — this closes that
/// gap. We truncate `patient_demographic` after the trigger has already populated it (from the
/// same two submitted events) so that the read-back value can ONLY have come from the backfill
/// re-projection, never a trigger leftover, isolating the path under test.
#[tokio::test]
async fn backfill_value_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Same field+provenance (→ equal rank), same (wall,counter,origin); value differs.
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            9,
            0,
            "n",
            field_payload("dob", first, "patient-stated"),
            &format!("dob {first}"),
        )
        .await;
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            9,
            0,
            "n",
            field_payload("dob", second, "patient-stated"),
            &format!("dob {second}"),
        )
        .await;

        // Clear what the trigger just projected so only the backfill re-projection below
        // can repopulate the row — proving the assertion exercises cairn_demographic_backfill(),
        // not a trigger leftover.
        c.execute("TRUNCATE patient_demographic", &[])
            .await
            .unwrap();
        c.execute("SELECT cairn_demographic_backfill()", &[])
            .await
            .unwrap();

        let value: String = c
            .query_one(
                "SELECT value FROM patient_demographic \
                 WHERE patient_id = $1::text::uuid AND field = 'dob'",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            value, "a",
            "backfill: C byte-order winner for {first}->{second}"
        );
    }
}

/// #69 review: `patient_name_apply()`'s `ON CONFLICT ... WHERE` tiebreak on `asserted_origin`
/// has NO direct test — `name_display_value_tiebreak_is_collation_independent` above submits
/// two DIFFERENT `value`s, so they land as separate PK rows `(patient_id, use_key, value)` and
/// `ON CONFLICT` never fires; that test only exercises the `patient_name_current` VIEW's own
/// `ORDER BY ... COLLATE "C"`. This test isolates the TRIGGER's conflict path instead: two
/// events share the SAME (patient, use, value) — so the second submit collides on the retained
/// set's PK — with equal (wall, counter, provenance) so only `asserted_origin COLLATE "C"`
/// decides which row's assertion is retained. Reads `patient_name` (the retained-set TABLE)
/// directly, not the VIEW, so the VIEW's own tiebreak can't mask a regression in the trigger's.
#[tokio::test]
async fn name_trigger_origin_tiebreak_is_collation_independent() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert!(locale_flips(&c, "B", "a").await);

    for (first, second) in [("B", "a"), ("a", "B")] {
        let (sk, kid) = setup(&c).await;
        let p = Uuid::now_v7();
        // Same value+use+provenance+(wall,counter) → same retained-set PK, equal rank/HLC;
        // only origin differs → the second submit's ON CONFLICT WHERE decides the winner.
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            7,
            0,
            first,
            name_payload("Smith", "legal", "patient-stated"),
            &format!("name Smith ({first})"),
        )
        .await;
        submit_generic(
            &c,
            &sk,
            &kid,
            p,
            "demographic.field.asserted",
            7,
            0,
            second,
            name_payload("Smith", "legal", "patient-stated"),
            &format!("name Smith ({second})"),
        )
        .await;

        let origin: String = c
            .query_one(
                "SELECT asserted_origin FROM patient_name \
                 WHERE patient_id = $1::text::uuid AND value = 'Smith'",
                &[&p.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            origin, "a",
            "trigger ON CONFLICT WHERE: C byte-order winner for {first}->{second}"
        );
    }
}
