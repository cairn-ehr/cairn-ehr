//! Integration coverage for the §4.2 provenance-precedence fields (DOB +
//! sex-at-birth): the in-DB structural floor + the winner-by-(rank,HLC)
//! patient_demographic projection. Real Postgres, gated on `$CAIRN_TEST_PG`,
//! serialized cluster-wide via `db::test_serial_guard`. Matching (§5.2) is a
//! separate subsystem and is NOT exercised here.
use cairn_event::demographics::{
    dob_assertion_body, render_dob_twin, render_sex_at_birth_twin, sex_at_birth_assertion_body,
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

/// Author + sign + submit one demographic.field.asserted event. `payload` is the
/// already-built body (from a Task-1 builder or a raw json! for rejection tests);
/// `twin` is the authored §4.5 twin. Returns the raw submit result.
async fn submit_field(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    payload: serde_json::Value,
    twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

#[tokio::test]
async fn happy_path_projects_dob_and_sex_with_rank_and_facets() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified")),
    )
    .await
    .expect("valid dob accepted");
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        sex_at_birth_assertion_body("female", "clinician-observed"),
        Some(&render_sex_at_birth_twin("female", "clinician-observed")),
    )
    .await
    .expect("valid sex-at-birth accepted");

    let p_str = p.to_string();
    // DOB row: value, cached provenance_rank (document-verified -> 60), facets.precision.
    let row = c
        .query_one(
            "SELECT value, provenance_rank, facets->>'precision' \
         FROM patient_demographic WHERE patient_id::text=$1 AND field='dob'",
            &[&p_str],
        )
        .await
        .unwrap();
    let value: String = row.get(0);
    let rank: i32 = row.get(1);
    let precision: String = row.get(2);
    assert_eq!(value, "1980-07-15");
    assert_eq!(rank, 60);
    assert_eq!(precision, "day");
    // sex-at-birth row exists with the right value.
    let sex: String = c.query_one(
        "SELECT value FROM patient_demographic WHERE patient_id::text=$1 AND field='sex-at-birth'",
        &[&p_str]).await.unwrap().get(0);
    assert_eq!(sex, "female");
    // The AUTHORED dob twin was carried verbatim (cairn_event_twin demographic branch).
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1 AND hlc_wall=1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(twin, "Date of birth (document-verified): 1980-07-15 (day)");
}

/// Helper: read the current projected dob value for a patient.
async fn dob_value(c: &Client, p: Uuid) -> String {
    let p_str = p.to_string();
    c.query_one(
        "SELECT value FROM patient_demographic WHERE patient_id::text=$1 AND field='dob'",
        &[&p_str],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn provenance_beats_recency_and_verified_locks() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // 1) An early patient-stated dob (rank 50).
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        dob_assertion_body("1979", "year", None, "patient-stated"),
        Some(&render_dob_twin("1979", "year", "patient-stated")),
    )
    .await
    .unwrap();
    assert_eq!(dob_value(&c, p).await, "1979");

    // 2) A later document-verified dob (rank 60) — higher provenance wins.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified")),
    )
    .await
    .unwrap();
    assert_eq!(
        dob_value(&c, p).await,
        "1980-07-15",
        "higher provenance wins"
    );

    // 3) An EVEN LATER patient-stated dob (rank 50) — must NOT displace the verified
    //    value. "Verified value locks vs. lower provenance."
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        3,
        dob_assertion_body("1981", "year", None, "patient-stated"),
        Some(&render_dob_twin("1981", "year", "patient-stated")),
    )
    .await
    .unwrap();
    assert_eq!(
        dob_value(&c, p).await,
        "1980-07-15",
        "verified value locks vs lower provenance"
    );
}

#[tokio::test]
async fn recency_breaks_ties_among_equal_provenance() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Two document-verified dobs (equal rank) — the HLC-later one wins.
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        dob_assertion_body("1980-01-01", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-01-01", "day", "document-verified")),
    )
    .await
    .unwrap();
    // Confirm the FIRST assertion actually projected. Without this, a silent failure of
    // the first submission would leave a one-row table and let the final assertion pass
    // for the wrong reason — never exercising the equal-provenance conflict-resolution path.
    assert_eq!(
        dob_value(&c, p).await,
        "1980-01-01",
        "first equal-provenance assertion projected"
    );
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified")),
    )
    .await
    .unwrap();
    // hlc_wall=2 > hlc_wall=1 → "1980-07-15" wins among equal provenance.
    assert_eq!(
        dob_value(&c, p).await,
        "1980-07-15",
        "later HLC wins among equal provenance"
    );
}

/// Assert the floor REJECTS the payload (submit errors) AND nothing was written —
/// neither to event_log nor to the patient_demographic projection.
async fn assert_rejected_and_empty(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    payload: serde_json::Value,
    twin: Option<&str>,
    label: &str,
) {
    let r = submit_field(c, sk, kid, p, 1, payload, twin).await;
    assert!(r.is_err(), "{label}: must be rejected by the floor");
    let p_str = p.to_string();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "{label}: nothing appended to event_log");
    let m: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(m, 0, "{label}: nothing projected");
}

#[tokio::test]
async fn floor_rejects_each_invariant_violation() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    // `good` is a valid non-empty twin shared by the four PAYLOAD-violation cases below.
    // Its text is irrelevant there — it only has to be non-empty so the rejection is the
    // payload check and NOT the §4.5 twin check. The empty-twin case alone makes the twin
    // itself the subject (passes Some("")).
    let good = Some("Date of birth (document-verified): 1980 (year)");

    // NOTE: all five sub-cases share ONE test function. Each uses a fresh Uuid::now_v7() so
    // their DB state is independent, but Rust collapses them into a single test result — a
    // panic in any one aborts the rest, so a failure label tells you which case broke.
    // value empty
    assert_rejected_and_empty(
        &c,
        &sk,
        &kid,
        Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"","provenance":"document-verified",
                           "facets":{"precision":"year"}}),
        good,
        "value-empty",
    )
    .await;
    // provenance missing
    assert_rejected_and_empty(
        &c,
        &sk,
        &kid,
        Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"1980","facets":{"precision":"year"}}),
        good,
        "provenance-missing",
    )
    .await;
    // field missing
    assert_rejected_and_empty(
        &c,
        &sk,
        &kid,
        Uuid::now_v7(),
        serde_json::json!({"value":"1980","provenance":"document-verified",
                           "facets":{"precision":"year"}}),
        good,
        "field-missing",
    )
    .await;
    // dob missing precision — principle 4
    assert_rejected_and_empty(
        &c,
        &sk,
        &kid,
        Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"1980","provenance":"document-verified"}),
        good,
        "dob-missing-precision",
    )
    .await;
    // dob facets present but not an object — the precision check sees a non-object
    // facets (precision resolves to NULL), so the dob structural floor rejects it.
    // (The safe builder can't produce this; a raw/hostile client could.)
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"dob","value":"1980","provenance":"document-verified","facets":"oops"}),
        good, "dob-facets-not-object").await;
    // empty authored twin — §4.5
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"sex-at-birth","value":"female","provenance":"clinician-observed"}),
        Some(""), "empty-twin").await;
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

    // A well-formed assertion for a field this node has no projection policy for.
    // The floor ACCEPTS it (generic checks pass), it lands in event_log and is legible
    // via its twin — but it is NOT projected. This is the federation-forward contract:
    // an older node must store a newer node's field, just not project it (ADR-0012).
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        serde_json::json!({"field":"eye-color","value":"brown","provenance":"clinician-observed"}),
        Some("Eye color (clinician-observed): brown"),
    )
    .await
    .expect("well-formed unknown field accepted (carried, legible)");

    let p_str = p.to_string();
    let in_log: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        in_log, 1,
        "unknown field is stored in event_log (legible evidence)"
    );
    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_demographic WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        projected, 0,
        "unknown field has no projection policy — not projected"
    );
    // ...and it is genuinely LEGIBLE: the authored twin was carried verbatim. Storage alone
    // is only half the federation contract — "carried AND legible" needs the twin too.
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        twin, "Eye color (clinician-observed): brown",
        "unknown field is legible via its authored twin"
    );
}

#[tokio::test]
async fn regression_identifier_and_legacy_patient_created_still_work() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // Slice-1 identifier assertion still projects through the (now re-declared) twin hook.
    let id_body = serde_json::json!({
        "field":"identifier","value":"943 476 5919","system":"nhs-number",
        "provenance":"document-verified","normalized":"9434765919","profile":"nhs-number@b3-abc"});
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: id_body,
        attachments: vec![],
        plaintext_twin: Some("nhs-number, document-verified: 943 476 5919".into()),
    };
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("slice-1 identifier still accepted via re-declared twin hook");
    let p_str = p.to_string();
    let id_rows: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_identifier WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(id_rows, 1, "identifier still projects");
    // The re-declared cairn_event_twin hook still carries the AUTHORED identifier twin
    // verbatim (not just the projection — the legibility twin is the second half).
    let id_twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1",
            &[&p_str],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        id_twin, "nhs-number, document-verified: 943 476 5919",
        "identifier authored twin carried by re-declared hook"
    );

    // Legacy patient.created (no authored twin) still gets the derived skeleton twin.
    let p2 = Uuid::now_v7();
    let body2 = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p2.to_string(),
        event_type: "patient.created".into(),
        schema_version: "demo/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name":"A B","dob":"1980","sex":"x"}),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed2 = sign(&body2, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed2.signed_bytes])
        .await
        .expect("legacy event still accepted");
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE event_id::text=$1",
            &[&body2.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        twin.starts_with("[patient.created]"),
        "legacy still derives the skeleton twin"
    );
}

#[tokio::test]
async fn fact_proven_displaces_document_verified() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();

    // A document-verified dob (rank 60), then a later fact-proven dob (rank 70).
    // fact-proven is the new top tier, so it WINS — it can override what an official
    // document merely attests. This pins the mechanical winner; the design doc flags
    // the clinical contestability of fact-proven auto-override as a deferred,
    // sex-expansion-slice decision (legibility/event_log retains both regardless).
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        1,
        dob_assertion_body("1980-07-15", "day", Some("document"), "document-verified"),
        Some(&render_dob_twin("1980-07-15", "day", "document-verified")),
    )
    .await
    .unwrap();
    assert_eq!(dob_value(&c, p).await, "1980-07-15");
    submit_field(
        &c,
        &sk,
        &kid,
        p,
        2,
        dob_assertion_body("1979-03-02", "day", Some("genetic-test"), "fact-proven"),
        Some(&render_dob_twin("1979-03-02", "day", "fact-proven")),
    )
    .await
    .unwrap();
    assert_eq!(
        dob_value(&c, p).await,
        "1979-03-02",
        "fact-proven (70) displaces document-verified (60)"
    );
}
