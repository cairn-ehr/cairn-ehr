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
/// present the malformed shapes a hostile or buggy peer could send. Re-asserts the
/// CHOSEN (medication_id, patient) pair — db::next_hlc is monotonic, so a second call
/// against the same thread always lands at a strictly LATER hlc than the first,
/// exercising the overlay-winner path rather than minting a fresh thread.
async fn submit_raw_substance_for(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    med: Uuid,
    patient: Uuid,
    substance: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let mut body = build_assert_body(
        Uuid::now_v7(),
        med,
        patient,
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

/// The common case: a fresh thread each call, med/patient minted here. The floor/refusal
/// tests below only care about the substance SHAPE, not thread continuity.
async fn submit_raw_substance(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    substance: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    submit_raw_substance_for(c, sk, kid, door, Uuid::now_v7(), Uuid::now_v7(), substance).await
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
    // Pin the specific message, not just any mention of "uuid" — the canonical-form
    // refusal below also mentions "uuid", and a weaker pin would pass against either.
    assert!(
        db_msg(&e).contains("requires a uuid code"),
        "{}",
        db_msg(&e)
    );
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", bad)
        .await
        .expect("the registry-derived tier is lenient at the apply door");
}

#[tokio::test]
async fn a_non_canonical_uuid_spelling_is_refused_locally() {
    // uuid_in accepts braces, uppercase, and a missing-hyphens spelling — all three
    // PARSE, but none is the canonical form the dup-key (Task 5) will compare as TEXT.
    // Two events naming the SAME moiety in different spellings must not key apart, and
    // the spelling can never be fixed after the fact (it is inside a signed body), so
    // the strict door must catch this, not just "is it a uuid at all".
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    for spelling in [
        "{0F8C4B1E-1B7A-5C2D-9A3E-2B6F7C8D9E01}", // braced + uppercase
        "0F8C4B1E-1B7A-5C2D-9A3E-2B6F7C8D9E01",   // uppercase only
        "0f8c4b1e1b7a5c2d9a3e2b6f7c8d9e01",       // hyphens stripped
    ] {
        let bad = serde_json::json!({
            "term": "Lipitor",
            "coding": {"system": "drugref-moiety", "code": spelling, "display": "atorvastatin"}
        });
        let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", bad)
            .await
            .expect_err(&format!(
                "non-canonical spelling {spelling} must be refused"
            ));
        assert!(
            db_msg(&e).contains("canonical"),
            "the refusal must name the canonical-form requirement: {}",
            db_msg(&e)
        );
    }
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
    // Pin the actual offending key, not merely a substring ("substance.coding" happens
    // to also appear in this message's suggested replacement, so it would pass even if
    // the retired-slot check silently stopped firing and a DIFFERENT check's message
    // coincidentally matched).
    assert!(db_msg(&e).contains("inn_code"), "{}", db_msg(&e));
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", retired)
        .await
        .expect("a verifiable peer event is never refused over a retired slot");
}

#[tokio::test]
async fn an_explicit_null_coding_is_admitted_at_the_apply_door() {
    // jsonb_typeof(c) = 'null' for an explicit JSON `"coding": null`, distinct from the
    // key being absent altogether (c IS NULL) but meaning the SAME thing: nobody has
    // coded this yet. A peer whose serializer emits explicit nulls for absent optionals
    // must not have an otherwise-verifiable event refused over that encoding choice —
    // exactly the ADR-0056 watermark-freeze failure mode this file exists to prevent.
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let explicit_null = serde_json::json!({"term": "Lipitor", "coding": null});
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", explicit_null)
        .await
        .expect("an explicit null coding must be treated as absent, never refused");
}

async fn coding_row(c: &Client, med: Uuid) -> Option<(String, String, String)> {
    c.query_opt(
        "SELECT coding_system, coding_code, coding_display \
           FROM medication_coding WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
}

#[tokio::test]
async fn a_coded_assertion_projects_its_coding() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_medication(
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
    .unwrap();
    assert_eq!(
        coding_row(&c, med).await,
        Some((
            "drugref-moiety".to_string(),
            MOIETY_ATORVASTATIN.to_string(),
            "atorvastatin".to_string()
        ))
    );
    // The honest-degradation read: the med list itself carries the label, so a node
    // with no drug database still shows the preferred name (ADR-0059 decision 4).
    let row = c
        .query_one(
            "SELECT term, coding_display FROM patient_medication_current \
               WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "Lipitor");
    assert_eq!(row.get::<_, String>(1), "atorvastatin");
}

#[tokio::test]
async fn an_uncoded_assertion_projects_no_coding_row() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let mut input = coded_input("little white pill", MOIETY_ATORVASTATIN);
    input.coding = None;
    let med = assert_medication(
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
    .unwrap();
    assert_eq!(
        coding_row(&c, med).await,
        None,
        "no coding claimed, no coding row — and nothing to clear later"
    );
}

#[tokio::test]
async fn the_deprecated_inn_code_column_survives_unread() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // ADR-0059 decision 2: deprecated IN PLACE, never dropped — a DROP is the
    // non-additive move principle 11 forbids (the db/036 sync_state.hlc_wall treatment).
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM information_schema.columns \
              WHERE table_name = 'medication_statement' AND column_name = 'inn_code'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the deprecated column stays");
}

// ---------------------------------------------------------------------------
// Retraction safety: the whole reason `medication_coding` is a separate table with a
// CONDITIONAL write is that a later uncoded re-assert must never silently clear an
// existing coding (retracting one is slice 6b's own authored correction event). The
// code shape (an `IF coding present THEN insert/overlay`, no `ELSE clear`) makes this
// structurally impossible today, but nothing pinned it — a future edit to the guard
// (e.g. "simplify" it into an unconditional upsert) would pass CI silently. These two
// tests pin it for both shapes principle 4 treats as the SAME honest-unknown claim:
// the key absent entirely, and an explicit JSON `"coding": null`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_uncoded_reassert_does_not_clear_an_existing_coding() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_medication(
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
    .unwrap();
    let original = coding_row(&c, med).await;
    assert!(original.is_some(), "the first assert must have coded it");

    // A LATER re-assert of the SAME thread (db::next_hlc is monotonic — the raw-submit
    // helper always advances it), with the coding key ABSENT this time (the
    // explicit-JSON-null shape is covered separately below). medication_id is the
    // thread's immortal key, so this is a re-assertion, not a new thread.
    submit_raw_substance_for(
        &c,
        &sk,
        kid.as_str(),
        "submit_event",
        med,
        patient,
        serde_json::json!({"term": "Lipitor"}),
    )
    .await
    .expect("an uncoded re-assert of an already-coded thread must still be accepted");

    assert_eq!(
        coding_row(&c, med).await,
        original,
        "an uncoded re-assert (coding key ABSENT) must never clear an existing coding"
    );
}

#[tokio::test]
async fn an_explicit_null_reassert_does_not_clear_an_existing_coding() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_medication(
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
    .unwrap();
    let original = coding_row(&c, med).await;
    assert!(original.is_some(), "the first assert must have coded it");

    // Same path, the OTHER honest-unknown shape: an explicit JSON `"coding": null`
    // (only reachable via a raw payload — the Rust builder always omits the key
    // instead, see `assertion_body_omits_absent_coding_...` in cairn-event). Submitted
    // at the lenient apply door, mirroring `an_explicit_null_coding_is_admitted_at_the_apply_door`.
    submit_raw_substance_for(
        &c,
        &sk,
        kid.as_str(),
        "apply_remote_event",
        med,
        patient,
        serde_json::json!({"term": "Lipitor", "coding": null}),
    )
    .await
    .expect("an explicit null coding on a re-assert must still be admitted");

    assert_eq!(
        coding_row(&c, med).await,
        original,
        "an explicit-null re-assert must never clear an existing coding either"
    );
}

// ---------------------------------------------------------------------------
// Rebuild-scope metadata must be exhaustive (db/031's own stated requirement): pin
// that medication_coding actually appears in the registered inventory for the assert
// verb, not just that the functional tests happen to exercise it.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_assert_verb_registers_medication_coding_in_its_rebuild_scope() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let tables: Vec<String> = c
        .query_one(
            "SELECT projection_tables FROM cairn_projection_apply \
              WHERE event_type = 'clinical.medication.asserted' \
                AND apply_fn = 'medication_statement_apply'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        tables.iter().any(|t| t == "medication_coding"),
        "medication_coding must be in the assert verb's rebuild-scope inventory, got {tables:?}"
    );
}
