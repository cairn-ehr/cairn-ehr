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

// Shared scaffolding, for `submit_registration`: since #345 the first event on a chart must
// be its registration, so every suite that mints a patient arranges one (#120/#327 — one copy).
mod common;

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
    // The coding-system REGISTRY is deliberately never TRUNCATEd (db/041's seeded
    // drugref-* rows must survive every test), so a test that registers a stand-in
    // system needs its row swept here rather than relying on its own cleanup: a run
    // killed mid-test would otherwise leave that system permanently registered in this
    // shared database, silently accepted at the strict door by every later run. Scoped
    // to non-drugref rows so the seed is untouched; harmless when nothing leaked.
    c.execute(
        "DELETE FROM medication_coding_system WHERE system NOT LIKE 'drugref-%'",
        &[],
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
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
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
    // #345: the chart is minted and registered here rather than inline below, so the
    // registration and the medication name the same patient.
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    // The principle-4 floor and the §1.2 M = N = 1 pin: coding is never required.
    assert_medication(&mut c, &sk, &kid, "test-node", patient, &input, None, None)
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

/// The dup-key (db/031 + db/033) and the anchor-conflict view (db/033) both flatten the
/// anchor to `<system>|<code>`, so `|` is a load-bearing SEPARATOR, not an ordinary
/// character. Without a constraint, registering a system named `a|b` would let its codes
/// collide with system `a`'s code `b|…` — two DIFFERENT substances silently sharing one
/// dup-key and reading as duplicates of each other.
///
/// Constraining the SYSTEM alone is sufficient: with systems guaranteed `|`-free, the
/// first `|` after the prefix is always the separator, so the flattened key parses
/// unambiguously no matter what the code contains (and codes are not free to be
/// reshaped — they arrive inside signed bodies).
#[tokio::test]
async fn a_coding_system_name_may_not_contain_the_key_separator() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for bad in ["pipe|system", "   "] {
        let e = c
            .execute(
                "INSERT INTO medication_coding_system (system, code_format, note) \
                   VALUES ($1, 'opaque', 'test-only row that must be refused')",
                &[&bad],
            )
            .await
            .expect_err(&format!("registering {bad:?} must be refused by the floor"));
        assert!(
            db_msg(&e).to_lowercase().contains("constraint")
                || db_msg(&e).contains("medication_coding_system"),
            "the refusal must come from the table's own shape constraint: {}",
            db_msg(&e)
        );
    }
    // The seeded systems obviously still satisfy it.
    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_coding_system", &[])
        .await
        .unwrap()
        .get(0);
    assert!(n >= 3, "the drugref-* seed rows survive the constraint");
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

/// Final-review finding 2: `code_format = 'opaque'` is admitted by the CHECK constraint
/// and is the ENTIRE mechanism behind ADR-0059 decision 7's claim — repeated in db/041's
/// comments, the ROADMAP, and the HANDOVER — that a deployment may plug a different
/// drug-identity authority as a REGISTRY ROW, never a patch to this file (principle 9).
/// Every other coding test in this file anchors on `drugref-moiety` ('uuid'); nothing
/// proved an 'opaque' system is actually accepted at the strict local door. This test
/// registers a stand-in national-formulary row with a non-uuid code and proves it is
/// accepted and projects exactly like a uuid-format coding does.
#[tokio::test]
async fn an_opaque_format_system_is_accepted_and_projects() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    const OPAQUE_SYSTEM: &str = "test-national-formulary";
    // The registry is never TRUNCATEd (the seeded drugref-* rows must survive every
    // test), so this test-only row is inserted explicitly. `setup_node` above has already
    // swept any non-drugref row a previously-killed run may have left behind, which is
    // what makes a re-run safe; the DELETE at the end of this test is the tidy path, not
    // the load-bearing one. ON CONFLICT DO UPDATE keeps the insert itself idempotent
    // (the same #214 convergence idiom db/041 uses for its own seed rows).
    c.execute(
        "INSERT INTO medication_coding_system AS r (system, code_format, note) \
           VALUES ($1, 'opaque', 'test-only row for the opaque-format floor test (finding 2)') \
           ON CONFLICT (system) DO UPDATE SET code_format = EXCLUDED.code_format, note = EXCLUDED.note",
        &[&OPAQUE_SYSTEM],
    )
    .await
    .unwrap();

    let input = AssertMedicationInput {
        term: "Diabex",
        coding: Some(SubstanceCoding {
            system: OPAQUE_SYSTEM,
            code: "A10BA02", // an ATC-shaped code — deliberately NOT a uuid
            display: "metformin",
        }),
        formulation: Some("tablet"),
        dose_amount: Some("500"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    };
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let med = assert_medication(&mut c, &sk, &kid, "test-node", patient, &input, None, None)
        .await
        .expect("a registered opaque-format system must be accepted at the strict local door");
    assert_eq!(
        coding_row(&c, med).await,
        Some((
            OPAQUE_SYSTEM.to_string(),
            "A10BA02".to_string(),
            "metformin".to_string()
        )),
        "an opaque-format coding must project exactly like a uuid-format one"
    );

    c.execute(
        "DELETE FROM medication_coding_system WHERE system = $1",
        &[&OPAQUE_SYSTEM],
    )
    .await
    .unwrap();
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
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
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
    // #345: minted and registered before the thread is asserted on it.
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let med = assert_medication(&mut c, &sk, &kid, "test-node", patient, &input, None, None)
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
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
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
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
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

// ---------------------------------------------------------------------------
// Task 5 (ADR-0059 decision 5): the (system, code) PAIR dup-key, prefer-coded
// group display, and the anchor-conflict advisory view.
// ---------------------------------------------------------------------------

async fn dup_keys(c: &Client, patient: Uuid) -> Vec<(String, i64)> {
    c.query(
        "SELECT dup_key, thread_count FROM patient_medication_reconciliation_flag \
           WHERE patient_id = $1::text::uuid ORDER BY dup_key",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

#[tokio::test]
async fn two_coded_threads_sharing_an_anchor_raise_one_flag() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    // The case ADR-0059 exists for: brand and generic, different words, same substance.
    for term in ["Lipitor", "atorvastatin"] {
        assert_medication(
            &mut c,
            &sk,
            &kid,
            "test-node",
            patient,
            &coded_input(term, MOIETY_ATORVASTATIN),
            None,
            None,
        )
        .await
        .unwrap();
    }
    let flags = dup_keys(&c, patient).await;
    assert_eq!(flags.len(), 1, "one duplicate group, got {flags:?}");
    assert_eq!(flags[0].1, 2);
    assert!(
        flags[0].0.starts_with("code:drugref-moiety|"),
        "the key is the (system, code) PAIR, never a bare code: {:?}",
        flags[0].0
    );
}

#[tokio::test]
async fn a_coded_and_an_uncoded_thread_still_key_apart() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("atorvastatin", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    let mut uncoded = coded_input("atorvastatin", MOIETY_ATORVASTATIN);
    uncoded.coding = None;
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &uncoded,
        None,
        None,
    )
    .await
    .unwrap();
    // ADR-0059 decision 5 is explicit: a coalesce picks PER ROW, so this case is NOT
    // closed by the key. It closes when the uncoded member gets coded, or later by the
    // drug-matcher. Claiming otherwise is the overstatement the ADR review caught.
    assert!(
        dup_keys(&c, patient).await.is_empty(),
        "coded and uncoded key apart — the honest, documented blind spot"
    );
}

#[tokio::test]
async fn a_reconciled_group_displays_its_coded_member() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let mut vague = coded_input("little white pill", MOIETY_ATORVASTATIN);
    vague.coding = None;
    let m_vague = assert_medication(&mut c, &sk, &kid, "test-node", patient, &vague, None, None)
        .await
        .unwrap();
    let m_coded = assert_medication(
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
    cairn_node::medication::reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        m_vague,
        m_coded,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let row = c
        .query_one(
            "SELECT term, coding_display FROM medication_group_display \
               WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap();
    // Pins the BREADTH the db/033 comment now states: the coded member wins the whole
    // row, not just the coding — `term` must read "Lipitor" (the coded member's), never
    // "little white pill" (the vague member's), even though both threads share one group.
    assert_eq!(
        row.get::<_, String>(0),
        "Lipitor",
        "the group's term comes from the coded member too, not just its coding_display"
    );
    assert_eq!(
        row.get::<_, String>(1),
        "atorvastatin",
        "the group takes its identity from the coded member, not from \"little white pill\""
    );
}

#[tokio::test]
async fn two_anchors_in_one_group_raise_a_conflict() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    let m1 = assert_medication(
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
    let m2 = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Diabex", MOIETY_METFORMIN),
        None,
        None,
    )
    .await
    .unwrap();
    cairn_node::medication::reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        m1,
        m2,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("reconciliation is a human judgment — never auto-refused over a coding");
    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_group_coding_conflict", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "two different anchors in one group is a possible-mis-reconciliation signal — \
         surfaced, never silently resolved"
    );
}

/// Issue #295: the anchor-conflict view's `count(DISTINCT …)` must stay `COLLATE "C"`-pinned.
///
/// THE HAZARD. `medication_group_coding_conflict` fires on
/// `HAVING count(DISTINCT <flattened anchor>) > 1`. Under a NON-DETERMINISTIC collation
/// (an ICU case-insensitive default, which a deployment is free to choose), two anchors
/// differing only in case compare EQUAL, so the count collapses to 1 and the conflict row
/// never appears — even though `anchors` would have listed both. That is the ADR-0045 class
/// of bug: the winner/flag depends on a node-LOCAL collation property, so honest nodes
/// disagree. Silently missing a mis-reconciliation signal is the bad direction.
///
/// This can really happen: the canonical-uuid pin only guards the STRICT door, and the
/// registry-derived tier is deliberately lenient on remote apply (ADR-0051/0056), so a peer
/// may legitimately hand us `0F8C4B1E-…` for a moiety we hold as `0f8c4b1e-…`.
///
/// TWO ASSERTIONS, because a behavioural test alone cannot discriminate on a cluster whose
/// default collation is deterministic (as the test rig's is):
///   1. the pin is DEMONSTRABLY load-bearing — a scratch non-deterministic ICU collation
///      collapses the unpinned form to 1 while the pinned form correctly returns 2;
///   2. the real view, over real events, flags case-differing anchors in one group.
#[tokio::test]
async fn the_anchor_conflict_count_is_collation_pinned() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;

    // 1. The pin is load-bearing. Build the comparison BOTH ways over the same two
    //    case-differing anchors under a non-deterministic collation. If a future Postgres
    //    ever made these agree, this fails and tells the reader the argument has changed.
    c.batch_execute(
        "CREATE COLLATION IF NOT EXISTS cairn_test_ci \
           (provider = icu, locale = 'und-u-ks-level2', deterministic = false)",
    )
    .await
    .expect("the test cluster is built --with-icu");
    let (unpinned, pinned): (i64, i64) = c
        .query_one(
            "WITH v(sys, code) AS (VALUES \
                 ('drugref-moiety'::text COLLATE cairn_test_ci, \
                  '0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01'::text COLLATE cairn_test_ci), \
                 ('drugref-moiety'::text COLLATE cairn_test_ci, \
                  '0F8C4B1E-1B7A-5C2D-9A3E-2B6F7C8D9E01'::text COLLATE cairn_test_ci)) \
             SELECT count(DISTINCT (sys || '|' || code)), \
                    count(DISTINCT ((sys || '|' || code) COLLATE \"C\")) FROM v",
            &[],
        )
        .await
        .map(|r| (r.get(0), r.get(1)))
        .unwrap();
    assert_eq!(
        unpinned, 1,
        "the hazard is real: unpinned, two case-differing anchors compare EQUAL and the \
         HAVING never fires"
    );
    assert_eq!(
        pinned, 2,
        "COLLATE \"C\" compares the identical-on-every-node UTF-8 bytes, so the two \
         anchors stay distinct"
    );

    // 2. The real view flags it. m1 is coded canonically through the strict door; m2 comes
    //    in through the LENIENT remote door in an uppercase spelling — the only way this
    //    state legitimately arises, and the reason it must be caught rather than assumed
    //    away.
    let patient = Uuid::now_v7();
    // #345: a chart must be registered before anything is recorded about it.
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    let m1 = assert_medication(
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
    let m2 = Uuid::now_v7();
    submit_raw_substance_for(
        &c,
        &sk,
        kid.as_str(),
        "apply_remote_event",
        m2,
        patient,
        serde_json::json!({
            "term": "atorvastatin",
            "coding": {
                "system": "drugref-moiety",
                "code": "0F8C4B1E-1B7A-5C2D-9A3E-2B6F7C8D9E01",
                "display": "atorvastatin"
            }
        }),
    )
    .await
    .expect("a peer's non-canonical uuid spelling is admitted at the remote door");
    cairn_node::medication::reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        m1,
        m2,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let (n, anchors): (i64, Vec<String>) = c
        .query_one(
            "SELECT anchor_count, anchors FROM medication_group_coding_conflict \
               WHERE group_id IN (SELECT group_id FROM medication_group_member \
                                   WHERE medication_id = $1::text::uuid)",
            &[&m1.to_string()],
        )
        .await
        .map(|r| (r.get(0), r.get(1)))
        .expect("two case-differing anchors in one group must raise the conflict row");
    assert_eq!(n, 2, "both spellings counted distinctly: {anchors:?}");
    assert_eq!(
        anchors.len(),
        2,
        "and both are listed for the human: {anchors:?}"
    );
}

// ---------------------------------------------------------------------------
// #192 thread-patient consistency, applied to the coding projection.
// ---------------------------------------------------------------------------

/// A medication_id belongs to ONE chart for life (#192). `medication_statement` enforces
/// that by RAISE-ing locally and converge-and-flagging on remote apply, after which its
/// `cairn_hlc_overlay_wins` guard decides whether the contradicting event actually wins
/// the row. `medication_coding`'s upsert has its OWN, INDEPENDENT overlay race — and on a
/// thread that has no coding row yet there is no conflict at all, so the INSERT lands
/// unconditionally.
///
/// That is the divergence this pins: a remote event with an EARLIER hlc re-asserting the
/// thread under a DIFFERENT patient LOSES the statement race (patient stays P1) but still
/// writes the coding row — which, taking `e.patient_id` verbatim, would record P2. The two
/// projections would then disagree about which chart the thread belongs to. Nothing reads
/// `medication_coding.patient_id` today, so this is latent rather than exploitable — but it
/// is a denormalised copy of a value #192 declares immortal, and slice 6b filtering codings
/// by patient would misfile on it. The fix sources it from the thread's STANDING patient
/// (cairn_medication_thread_patient) instead of from the event.
#[tokio::test]
async fn a_losing_cross_patient_reassert_cannot_misfile_the_coding_patient() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let p1 = Uuid::now_v7();
    let p2 = Uuid::now_v7();
    // #345: BOTH charts exist before anything is recorded about them — the cross-patient
    // misfile this test guards is only meaningful between two real charts.
    common::submit_registration(&c, &sk, &kid, p1, 0).await;
    common::submit_registration(&c, &sk, &kid, p2, 0).await;

    // Thread M, patient P1, UNCODED — so no coding row exists yet.
    let mut uncoded = coded_input("atorvastatin", MOIETY_ATORVASTATIN);
    uncoded.coding = None;
    let med = assert_medication(&mut c, &sk, &kid, "test-node", p1, &uncoded, None, None)
        .await
        .unwrap();

    // A remote re-assert of the SAME thread under P2, CODED, at an EARLIER hlc so it
    // loses medication_statement's overlay race. build_assert_body takes the hlc
    // verbatim, which is the only way to author a deliberately-stale event.
    let mut hlc = db::next_hlc(&c, "test-node").await.unwrap();
    hlc.wall -= 60_000; // a minute behind the standing row — comfortably losing
    let body = build_assert_body(
        Uuid::now_v7(),
        med,
        p2,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        kid.as_str(),
        hlc,
    );
    let signed = sign(&body, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.as_slice()],
    )
    .await
    .expect("the apply door converges-and-flags a cross-patient reassert, never refuses it");

    // The statement kept P1 (the stale event lost its overlay race) …
    let standing: String = c
        .query_one(
            "SELECT patient_id::text FROM medication_statement WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        standing,
        p1.to_string(),
        "precondition: the earlier-hlc event must LOSE the statement row, else this test \
         is not exercising the divergence it exists for"
    );

    // … so the coding row must agree, not carry the losing event's patient.
    let coding_patient: String = c
        .query_one(
            "SELECT patient_id::text FROM medication_coding WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        coding_patient, standing,
        "medication_coding.patient_id must follow the thread's STANDING chart (#192), not \
         the patient named by an event that lost the statement's overlay race"
    );
}
