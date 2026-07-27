//! §3.3/ADR-0059 the substance.coding floor (db/041) — DB-gated on $CAIRN_TEST_PG,
//! serialized cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern,
//! like medication.rs). Two tiers: structural (RAISE at both doors, like
//! substance.term) and registry-derived (RAISE locally, silently admitted on remote
//! apply — a peer may run a newer or locally-extended registry, ADR-0056/ADR-0051).
use cairn_event::medication::SubstanceCoding;
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{assert_medication, build_assert_body, AssertMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement (see identity_dispute.rs /
/// identity_repudiate.rs) — `tokio_postgres::Error::to_string()` for a DB-originated
/// error just returns the generic "db error"; the real RAISE EXCEPTION text lives on
/// the wrapped DbError.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// ADR-0052: seal a CLEAR clinical EventBody like the node write path (payload + twin
/// under a fresh per-event DEK, outer stub twin), register the node's unwrap key, sign,
/// and submit through the 4-arg strict door. Returns the raw driver Result so refusal-
/// pinning tests keep using db_msg on the error. House rule 6: the DEK is generated
/// inside seal_event_payload, never a literal.
async fn seal_and_submit(
    c: &Client,
    sk: &SigningKey,
    mut body: EventBody,
) -> Result<u64, tokio_postgres::Error> {
    let twin = body
        .plaintext_twin
        .take()
        .expect("a clinical body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal the clear payload+twin");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk).expect("sign the sealed body");
    let secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await?;
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
}

/// Truncate the log + medication projections + the ADR-0052 custody plane and enroll a
/// fresh device actor. node_unwrap_key/event_dek/event_clear/erasure_shred_log have NO FK
/// to event_log, so the CASCADE does not reach them — a stale prior-test node key would
/// otherwise collide with this test's fresh key at cairn_register_unwrap_key.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// A moiety anchor shaped like drugref's: a UUIDv5. Fixed, not random — the tests
/// assert on it. Not cryptographic material, so house rule 6 does not apply.
const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

fn coded_input(term: &'static str, code: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: "drugref-moiety",
            code,
            display: "atorvastatin",
        }),
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    }
}

#[tokio::test]
async fn registry_and_check_fn_load() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // db/031's floor calls this fn by name and plpgsql resolves it only at execution,
    // so a missing db/041 would otherwise surface as a first-write surprise.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_coding_system WHERE system = 'drugref-moiety'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the drugref-moiety row must be seeded");
    let exists: Option<String> = c
        .query_one(
            "SELECT to_regprocedure('cairn_check_medication_coding(jsonb)')::text",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(exists.is_some(), "db/041 must declare the coding check fn");
}

#[tokio::test]
async fn a_complete_coding_is_accepted() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .expect("a well-formed drugref-moiety coding must pass the floor");
}

#[tokio::test]
async fn an_uncoded_assertion_still_passes() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let mut input = coded_input("little white pill", MOIETY_ATORVASTATIN);
    input.coding = None;
    // The principle-4 floor and the §1.2 M = N = 1 pin: coding is never required.
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        Uuid::now_v7(),
        &input,
        None,
        None,
    )
    .await
    .expect("uncoded must stay a first-class recordable value");
}

/// Submit a raw payload at a chosen door, bypassing the Rust builder — the only way to
/// present the malformed shapes a hostile or buggy peer could send.
async fn submit_raw_substance(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    substance: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let mut body = build_assert_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        kid,
        hlc,
    );
    body.payload["substance"] = substance;
    match door {
        "submit_event" => seal_and_submit(c, sk, body).await,
        _ => {
            let signed = cairn_event::sign(&body, sk).unwrap();
            c.execute(&format!("SELECT {door}($1)")[..], &[&signed.signed_bytes])
                .await
        }
    }
}

#[tokio::test]
async fn structural_gaps_are_refused_at_both_doors() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // display is structural, not optional: it is the honest-degradation label a
    // drugref-less reader depends on (ADR-0059 decision 4).
    for door in ["submit_event", "apply_remote_event"] {
        for bad in [
            serde_json::json!({"term": "Lipitor", "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN}}),
            serde_json::json!({"term": "Lipitor", "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN, "display": "  "}}),
            serde_json::json!({"term": "Lipitor", "coding": {"code": MOIETY_ATORVASTATIN, "display": "atorvastatin"}}),
            serde_json::json!({"term": "Lipitor", "coding": "drugref-moiety"}),
        ] {
            let e = submit_raw_substance(&c, &sk, kid.as_str(), door, bad.clone())
                .await
                .expect_err(&format!("{door} must refuse {bad}"));
            assert!(
                db_msg(&e).contains("substance.coding"),
                "the refusal must name the field: {}",
                db_msg(&e)
            );
        }
    }
}

#[tokio::test]
async fn an_unregistered_system_is_refused_locally_and_admitted_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let unknown = serde_json::json!({
        "term": "Lipitor",
        "coding": {"system": "national-formulary-xyz", "code": "A10BA02", "display": "metformin"}
    });
    // Strict submit: this door only authors codings it can vouch for (ADR-0051).
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", unknown.clone())
        .await
        .expect_err("an unregistered system must be refused at the local door");
    assert!(
        db_msg(&e).contains("national-formulary-xyz"),
        "{}",
        db_msg(&e)
    );
    // Lenient apply: a peer may run a newer or locally-extended registry. Admit it.
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", unknown)
        .await
        .expect("a peer's unregistered system must be admitted, never refused");
}

#[tokio::test]
async fn a_non_uuid_code_is_refused_locally_and_admitted_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "term": "Lipitor",
        "coding": {"system": "drugref-moiety", "code": "atorvastatin", "display": "atorvastatin"}
    });
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", bad.clone())
        .await
        .expect_err("a drugref-moiety code must be a uuid");
    assert!(db_msg(&e).contains("uuid"), "{}", db_msg(&e));
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", bad)
        .await
        .expect("the registry-derived tier is lenient at the apply door");
}

#[tokio::test]
async fn the_retired_inn_code_slot_is_refused_locally_and_ignored_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let retired = serde_json::json!({"term": "Lipitor", "inn_code": "INN:atorvastatin"});
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", retired.clone())
        .await
        .expect_err("the retired slot must fail loud at the source");
    assert!(db_msg(&e).contains("substance.coding"), "{}", db_msg(&e));
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", retired)
        .await
        .expect("a verifiable peer event is never refused over a retired slot");
}
