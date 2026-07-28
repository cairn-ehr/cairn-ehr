//! ADR-0059 decision 3 / slice 6b — the coding-OVERLAY floor and projection (db/042).
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard
//! (shared-DB + TRUNCATE pattern, like medication_coding.rs, whose harness preamble this
//! mirrors).
//!
//! Slice 6a can only code a medication inline, at assertion time. These two verbs make
//! coding a separately-authored act: code a thread later, correct a wrong coding, or
//! STRIKE a coding back to honest not-yet-coded. The floor tests pin the shape rules
//! (exactly one of coding/strike; an unknown `corrects` target is LAWFUL — offline-first);
//! the projection tests pin the winner rule and the struck degradation.
use cairn_event::medication::{CodingClaim, SubstanceCoding};
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement —
/// `tokio_postgres::Error::to_string()` for a DB-originated error just returns the generic
/// "db error"; the real RAISE EXCEPTION text lives on the wrapped DbError.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// ADR-0052: seal a CLEAR clinical EventBody like the node write path (payload + twin
/// under a fresh per-event DEK, outer stub twin), register the node's unwrap key, sign,
/// and submit through the 4-arg strict door. Returns the raw driver Result so
/// refusal-pinning tests keep using db_msg on the error. House rule 6: the DEK is
/// generated inside seal_event_payload, never a literal.
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
           IF to_regclass('public.medication_dose_point') IS NOT NULL THEN TRUNCATE medication_dose_point; END IF; \
           IF to_regclass('public.medication_reconciliation_link') IS NOT NULL THEN TRUNCATE medication_reconciliation_link; END IF; \
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

/// A moiety anchor shaped like drugref's: a UUIDv5. Fixed, not random — the tests assert
/// on it. Not cryptographic material, so house rule 6 does not apply.
const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

/// Submit a raw overlay payload at a chosen door, bypassing the Rust builders — the only
/// way to present the malformed shapes a buggy or hostile peer could send.
async fn submit_raw_overlay(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some("coded as atorvastatin [drugref-moiety]".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    match door {
        "submit_event" => seal_and_submit(c, sk, body).await,
        _ => {
            let signed = sign(&body, sk).unwrap();
            c.execute(&format!("SELECT {door}($1)")[..], &[&signed.signed_bytes])
                .await
        }
    }
}

#[tokio::test]
async fn both_overlay_types_are_registered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for t in [
        "clinical.medication-coding.asserted",
        "clinical.medication-coding-correction.asserted",
    ] {
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM cairn_event_twin_check WHERE event_type = $1",
                &[&t],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 1, "{t} must be registered in the twin-check registry");
        let (mode, targets): (String, bool) = c
            .query_one(
                "SELECT mode, targets_other_author FROM event_type_class WHERE event_type = $1",
                &[&t],
            )
            .await
            .map(|r| (r.get(0), r.get(1)))
            .unwrap();
        // A correction ADDS a claim; the original stays in the log and the projection
        // picks a winner. targets_other_author = TRUE would route these through the
        // ADR-0043 owner gate and refuse a pharmacist correcting someone else's coding.
        assert_eq!(mode, "additive", "{t}");
        assert!(!targets, "{t} must not trip the suppression owner-gate");
    }
}

#[tokio::test]
async fn a_correction_claiming_both_replacement_and_strike_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string(),
        "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN, "display": "atorvastatin"},
        "strike": true
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("both is incoherent");
    assert!(db_msg(&e).contains("both"), "{}", db_msg(&e));
}

#[tokio::test]
async fn a_correction_claiming_neither_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string()
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("neither is incoherent");
    assert!(db_msg(&e).contains("strike"), "{}", db_msg(&e));
}

/// PR-review finding: `strike` must be a JSON BOOLEAN, pinned as such.
///
/// The floor used to read it as `(p ->> 'strike')::boolean`, which inherits Postgres's
/// permissive boolean input syntax. That made the wire shape of an IMMORTAL, signed field
/// depend on a cast's tolerance rather than on a stated rule: `1`, `"true"` and `"yes"` all
/// struck a coding, while `"banana"` failed with a raw
/// `invalid input syntax for type boolean` instead of one of this file's legible refusals.
/// Neither direction is acceptable for a field whose only two meanings are "retract this
/// drug identity" and "do not" — a peer whose serializer stringifies booleans would be
/// authoring strikes this node never agreed to accept, and once frozen into a signed body
/// that spelling is permanent (the db/041 canonical-uuid argument, applied to a boolean).
///
/// Structural, so refused at BOTH doors: this is a shape judgment, not a registry one.
#[tokio::test]
async fn a_non_boolean_strike_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Each of these is accepted or rejected ARBITRARILY by a bare ::boolean cast: the
    // first two parse to TRUE, the third parses to FALSE, the fourth raises a Postgres
    // cast error with no mention of the field. All four must instead meet one refusal that
    // names `strike`.
    for spelling in [
        serde_json::json!("true"),
        serde_json::json!(1),
        serde_json::json!(0),
        serde_json::json!("banana"),
    ] {
        for door in ["submit_event", "apply_remote_event"] {
            let bad = serde_json::json!({
                "medication_id": Uuid::now_v7().to_string(),
                "corrects": Uuid::now_v7().to_string(),
                "strike": spelling
            });
            let e = submit_raw_overlay(
                &c,
                &sk,
                &kid,
                door,
                "clinical.medication-coding-correction.asserted",
                "clinical.medication-coding-correction/1",
                bad,
            )
            .await
            .unwrap_err();
            let msg = db_msg(&e);
            assert!(
                msg.contains("strike must be a JSON boolean"),
                "a {spelling} strike at {door} must meet the floor's own refusal, got: {msg}"
            );
        }
    }
}

/// The other edge of the same pin: an explicit `"strike": false` is a WELL-FORMED claim of
/// "not a strike", so it must fall through to the neither-replacement-nor-strike refusal
/// rather than the shape one. Without this, tightening the type could have swallowed the
/// legitimate spelling a serializer that always emits the key produces.
#[tokio::test]
async fn an_explicit_false_strike_falls_through_to_the_neither_refusal() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string(),
        "strike": false
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("false + no coding is still neither");
    let msg = db_msg(&e);
    assert!(
        msg.contains("must carry a replacement coding or strike = true"),
        "an explicit false is well-formed and must reach the NEITHER refusal, got: {msg}"
    );
}

#[tokio::test]
async fn an_unknown_corrects_target_is_accepted() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Offline-first: the corrected event may replicate later, or never. Refusing an
    // unknown target would make a correction impossible on a node that has not yet
    // received the coding it fixes.
    let ok = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string(),
        "strike": true
    });
    submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        ok,
    )
    .await
    .expect("an unknown corrects target must be accepted");
}

#[tokio::test]
async fn a_non_uuid_corrects_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": "not-a-uuid",
        "strike": true
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("corrects must be a uuid");
    assert!(db_msg(&e).contains("corrects"), "{}", db_msg(&e));
}

#[tokio::test]
async fn a_coding_overlay_without_a_coding_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Unlike the assertion — where an absent coding is the honest not-yet-coded floor —
    // an overlay whose whole purpose is to code something has nothing to say without one.
    // Refused at BOTH doors: this is a structural incoherence, not a registry judgment.
    for door in ["submit_event", "apply_remote_event"] {
        let bad = serde_json::json!({ "medication_id": Uuid::now_v7().to_string() });
        let e = submit_raw_overlay(
            &c,
            &sk,
            &kid,
            door,
            "clinical.medication-coding.asserted",
            "clinical.medication-coding/1",
            bad,
        )
        .await
        .expect_err("a coding overlay carrying no coding must be refused");
        assert!(db_msg(&e).contains("required"), "{}", db_msg(&e));
    }
}

#[tokio::test]
async fn the_inherited_triple_checks_still_fire_on_an_overlay() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Structural tier: refused at BOTH doors, like substance.term.
    for door in ["submit_event", "apply_remote_event"] {
        let bad = serde_json::json!({
            "medication_id": Uuid::now_v7().to_string(),
            "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN}
        });
        let e = submit_raw_overlay(
            &c,
            &sk,
            &kid,
            door,
            "clinical.medication-coding.asserted",
            "clinical.medication-coding/1",
            bad,
        )
        .await
        .expect_err("a coding missing display must be refused at both doors");
        assert!(db_msg(&e).contains("display"), "{}", db_msg(&e));
    }
    // Registry-derived tier: strict-submit, lenient-apply.
    let unknown = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "coding": {"system": "national-formulary-xyz", "code": "A10BA02", "display": "metformin"}
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding.asserted",
        "clinical.medication-coding/1",
        unknown.clone(),
    )
    .await
    .expect_err("an unregistered system must be refused at the local door");
    assert!(
        db_msg(&e).contains("national-formulary-xyz"),
        "{}",
        db_msg(&e)
    );
    submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "apply_remote_event",
        "clinical.medication-coding.asserted",
        "clinical.medication-coding/1",
        unknown,
    )
    .await
    .expect("a peer's unregistered system must be admitted, never refused");
}

#[tokio::test]
async fn a_non_canonical_uuid_code_is_refused_at_the_strict_door() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Inherited from db/041: Postgres parses braced/uppercase spellings, but the
    // TEXT-compared dup-key would split the same moiety permanently once frozen into a
    // signed body. Registry-derived tier, so strict door only.
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "coding": {
            "system": "drugref-moiety",
            "code": MOIETY_ATORVASTATIN.to_uppercase(),
            "display": "atorvastatin"
        }
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding.asserted",
        "clinical.medication-coding/1",
        bad,
    )
    .await
    .expect_err("a non-canonical uuid spelling must be refused");
    assert!(db_msg(&e).contains("canonical"), "{}", db_msg(&e));
}

// ---------------------------------------------------------------------------
// Projection (Task 4): both apply fns write the EXISTING medication_coding table
// under the EXISTING overlay-winner rule, which is what keeps this slice additive.
// ---------------------------------------------------------------------------

fn coding() -> SubstanceCoding<'static> {
    SubstanceCoding {
        system: "drugref-moiety",
        code: MOIETY_ATORVASTATIN,
        display: "atorvastatin",
    }
}

/// The projected coding state of a thread: (system, display, struck), or None when no
/// coding row exists at all. The two are clinically different — "nobody coded it" vs
/// "a reviewer established the coding was wrong".
async fn coding_state(c: &Client, med: Uuid) -> Option<(Option<String>, Option<String>, bool)> {
    c.query_opt(
        "SELECT coding_system, coding_display, struck \
           FROM medication_coding WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
}

/// Assert a medication with no coding at all, and return its thread id.
async fn assert_uncoded(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid) -> Uuid {
    let input = cairn_node::medication::AssertMedicationInput {
        term: "little white pill",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    cairn_node::medication::assert_medication(c, sk, kid, "test-node", patient, &input, None, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_overlay_codes_a_previously_uncoded_thread() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    assert_eq!(coding_state(&c, med).await, None, "uncoded to begin with");

    cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((
            Some("drugref-moiety".to_string()),
            Some("atorvastatin".to_string()),
            false
        ))
    );
}

#[tokio::test]
async fn a_correction_replaces_the_claim() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coding_event = cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();

    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    cairn_node::medication::correct_medication_coding(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CorrectCodingInput {
            corrects: coding_event,
            claim: CodingClaim::Replace(SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_METFORMIN,
                display: "metformin",
            }),
            note: Some("misread the brand"),
        },
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((
            Some("drugref-moiety".to_string()),
            Some("metformin".to_string()),
            false
        ))
    );
}

#[tokio::test]
async fn a_strike_nulls_the_anchor_and_flags_the_row() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coding_event = cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();

    // The clinical case: established as NOT that substance, with no replacement known.
    cairn_node::medication::correct_medication_coding(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CorrectCodingInput {
            corrects: coding_event,
            claim: CodingClaim::Strike,
            note: Some("not atorvastatin; substance unidentified"),
        },
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((None, None, true)),
        "a strike NULLs the anchor and flags the row — the thread is honestly uncoded again"
    );
}

#[tokio::test]
async fn a_lower_hlc_overlay_arriving_later_does_not_win() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;

    // Build TWO coding bodies by hand so the HLCs are ours to order, then submit the
    // HIGHER one first: set-union sync delivers in arrival order, never HLC order, so the
    // winner must be decided by the HLC and not by who got there last.
    let low = db::next_hlc(&c, "test-node").await.unwrap();
    let high = db::next_hlc(&c, "test-node").await.unwrap();
    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    let winner = cairn_node::medication::build_coding_body(
        Uuid::now_v7(),
        med,
        patient,
        &cairn_node::medication::CodeMedicationInput {
            coding: SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_METFORMIN,
                display: "metformin",
            },
        },
        &kid,
        high,
    );
    let loser = cairn_node::medication::build_coding_body(
        Uuid::now_v7(),
        med,
        patient,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        &kid,
        low,
    );
    seal_and_submit(&c, &sk, winner).await.unwrap();
    seal_and_submit(&c, &sk, loser).await.unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((
            Some("drugref-moiety".to_string()),
            Some("metformin".to_string()),
            false
        )),
        "the higher-HLC coding must keep winning after a lower one arrives"
    );
}

#[tokio::test]
async fn an_overlay_for_an_absent_thread_still_lands() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Offline-first / arrival-order independence: the assertion may replicate later, or
    // never. The coding row's patient_id has no standing chart to read, so it falls back
    // to the coding event's own patient claim (medication_coding.patient_id is NOT NULL).
    let orphan = Uuid::now_v7();
    let patient = Uuid::now_v7();
    cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        orphan,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .expect("a coding for a not-yet-present thread must be accepted");
    // uuid columns are read as ::text and compared as strings — the project-wide idiom
    // (tokio-postgres has no FromSql for the uuid crate's type in this workspace).
    let filed: Option<String> = c
        .query_opt(
            "SELECT patient_id::text FROM medication_coding WHERE medication_id = $1::text::uuid",
            &[&orphan.to_string()],
        )
        .await
        .unwrap()
        .map(|r| r.get(0));
    assert_eq!(
        filed.as_deref(),
        Some(patient.to_string().as_str()),
        "an orphan coding files under its own patient claim"
    );
}

#[tokio::test]
async fn an_orphan_coding_claims_the_thread_for_its_chart() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // #192: a medication_id belongs to ONE chart for life. A coding that arrives before
    // its assert files under its own patient claim, so that claim must then be visible to
    // the shared guard — otherwise a later assert naming a DIFFERENT patient would be
    // admitted and the two projections would disagree about the thread's chart forever.
    let orphan = Uuid::now_v7();
    let coder_patient = Uuid::now_v7();
    cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        coder_patient,
        orphan,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();
    let standing: Option<String> = c
        .query_one(
            "SELECT cairn_medication_thread_patient($1::text::uuid)::text",
            &[&orphan.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        standing.as_deref(),
        Some(coder_patient.to_string().as_str()),
        "an orphan coding establishes the thread's standing chart"
    );
}

// ---------------------------------------------------------------------------
// Struck-aware downstream (Task 5). These are the tests that prove slice 6a's
// table-not-columns decision paid off: the dup-key and the anchor-conflict view degrade on
// their own through the existing coalesce, and only ONE downstream predicate needed an
// edit — the prefer-coded group display, which tested row EXISTENCE rather than anchor
// presence.
// ---------------------------------------------------------------------------

const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";

fn coded_input(
    term: &'static str,
    code: &'static str,
) -> cairn_node::medication::AssertMedicationInput<'static> {
    cairn_node::medication::AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: "drugref-moiety",
            code,
            display: "atorvastatin",
        }),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    }
}

/// Strike a thread's coding. `corrects` names an unknown event on purpose here: the floor
/// deliberately admits an unknown target (offline-first), and these tests are about the
/// projection, not about target resolution.
async fn strike_coding(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, med: Uuid) {
    cairn_node::medication::correct_medication_coding(
        c,
        sk,
        kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CorrectCodingInput {
            corrects: Uuid::now_v7(),
            claim: CodingClaim::Strike,
            note: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
}

/// The group display's (term, coding_display). BOTH matter: db/033's ORDER BY picks the
/// DISTINCT ON winner for the WHOLE ROW, so a struck member that is still preferred drags
/// its term along too — asserting only the coding display would pass on a NULL column while
/// the group still read under the retracted member's identity.
async fn group_display_row(c: &Client, patient: Uuid) -> (String, Option<String>) {
    let r = c
        .query_one(
            "SELECT term, coding_display FROM medication_group_display \
               WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn a_struck_coding_stops_winning_the_group_display() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // A vague thread and a coded thread, reconciled into one group. Slice 6a makes the
    // coded member the group's display; after a strike it must stop being preferred, or
    // the group reads under a coding somebody explicitly retracted.
    let vague = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coded = cairn_node::medication::assert_medication(
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
        vague,
        coded,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        group_display_row(&c, patient).await,
        ("Lipitor".to_string(), Some("atorvastatin".to_string())),
        "6a: the coded member wins the group display, term and all"
    );

    strike_coding(&mut c, &sk, &kid, patient, coded).await;
    assert_eq!(
        group_display_row(&c, patient).await,
        ("little white pill".to_string(), None),
        "a struck coding must stop being preferred: the group falls back to the pre-0059 \
         winner (the group_id member) rather than reading under a retracted coding"
    );
}

#[tokio::test]
async fn a_struck_thread_returns_to_the_term_dup_key() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Two threads with the SAME term, one coded: they key apart (6a's documented
    // coded<->uncoded blind spot). Striking the coding makes both key on the term, so the
    // duplicate flag appears — the degradation falling out of the existing coalesce with no
    // dup-key change at all ('code:' || NULL is NULL in SQL).
    let mut input = coded_input("atorvastatin", MOIETY_ATORVASTATIN);
    let coded = cairn_node::medication::assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &input,
        None,
        None,
    )
    .await
    .unwrap();
    input.coding = None;
    cairn_node::medication::assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &input,
        None,
        None,
    )
    .await
    .unwrap();
    let before: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        before, 0,
        "coded and uncoded key apart (6a's documented gap)"
    );

    strike_coding(&mut c, &sk, &kid, patient, coded).await;
    let after: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after, 1,
        "with the anchor struck, both threads key on the term again"
    );
}

#[tokio::test]
async fn a_struck_coding_clears_the_anchor_conflict() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Two DIFFERENTLY-coded threads reconciled into one group raise 6a's advisory
    // anchor-conflict row. Striking one anchor resolves the disagreement honestly — one
    // live anchor, one acknowledged unknown — and the count(DISTINCT ...) ignores the NULL
    // with no view change of its own.
    let a = cairn_node::medication::assert_medication(
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
    let b = cairn_node::medication::assert_medication(
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
        a,
        b,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    // The view is keyed on group_id (it carries no patient column), so reach it through
    // the membership table.
    let conflict_sql = "SELECT count(*) FROM medication_group_coding_conflict \
                          WHERE group_id IN (SELECT group_id FROM medication_group_member \
                                              WHERE medication_id = $1::text::uuid)";
    let conflicts: i64 = c
        .query_one(conflict_sql, &[&a.to_string()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(conflicts, 1, "6a: two anchors in one group conflict");

    strike_coding(&mut c, &sk, &kid, patient, b).await;
    let after: i64 = c
        .query_one(conflict_sql, &[&a.to_string()])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after, 0,
        "striking one of the two anchors clears the conflict — one live anchor is coherent"
    );
}

#[tokio::test]
async fn the_worklist_distinguishes_never_coded_from_struck() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let never = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let struck_thread = assert_uncoded(&mut c, &sk, &kid, patient).await;
    cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        struck_thread,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();
    strike_coding(&mut c, &sk, &kid, patient, struck_thread).await;

    let coded_thread = assert_uncoded(&mut c, &sk, &kid, patient).await;
    cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        coded_thread,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None,
        None,
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT medication_id::text, previously_struck FROM patient_medication_uncoded \
               WHERE patient_id = $1::text::uuid ORDER BY previously_struck",
            &[&patient.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "the coded thread must not appear in the worklist"
    );
    assert_eq!(rows[0].get::<_, String>(0), never.to_string());
    assert!(!rows[0].get::<_, bool>(1), "never coded");
    assert_eq!(rows[1].get::<_, String>(0), struck_thread.to_string());
    assert!(
        rows[1].get::<_, bool>(1),
        "a struck thread is genuinely uncoded and must stay in the queue, flagged"
    );
}

#[tokio::test]
async fn a_ceased_thread_leaves_the_worklist() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let listed: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_uncoded WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(listed, 1);

    // Coding a stopped medication is work nobody needs: the worklist is a queue of live
    // clinical identity questions, not an archive audit.
    cairn_node::medication::cease_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        med,
        &cairn_node::medication::CeaseMedicationInput {
            stopped: Some("2026-01"),
            stopped_precision: Some("month"),
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let after: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_uncoded WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(after, 0, "a ceased thread drops off the coder worklist");
}

// ---------------------------------------------------------------------------
// PR-review findings: the two hazards the nullable-anchor widening opened.
// Both are ADR-0045-class — a projection whose state depends on something other than
// the event SET, which is the one thing set-union sync cannot tolerate.
// ---------------------------------------------------------------------------

/// Submit a pre-built clear clinical body at a chosen HLC, sealing it like the write path.
/// Lets a test order events by HLC independently of the order they ARRIVE in — the whole
/// point of the two tests below, since sync delivers in arrival order and never in HLC
/// order.
async fn submit_at(c: &Client, sk: &SigningKey, body: EventBody) {
    seal_and_submit(c, sk, body).await.expect("submit");
}

/// FINDING 1 — `struck` must be a function of the event SET, not of arrival order.
///
/// `medication_coding` is written by THREE event types, and the two 6b overlays were not
/// the only writers of the row: db/031's INLINE coding upsert writes it too. That upsert
/// names the three anchor columns and (before the fix) not `struck`, so an inline coding
/// that WON the HLC race over an earlier-arriving strike left the row with a live anchor
/// and a stale `struck = TRUE`.
///
/// The failure is cross-node and entirely realistic. Node A asserts a medication with an
/// inline coding; node B, offline with a lagging clock, strikes it at a LOWER HLC. Both
/// nodes end up holding both events, and the assertion wins on both — but B applied the
/// strike first and A did not, so the two nodes' projections disagree about a column
/// `cairn_agent` can read. Two honest nodes, the same events, different answers.
///
/// This test replays the same two events in BOTH arrival orders and demands one answer.
#[tokio::test]
async fn struck_is_arrival_order_independent_against_an_inline_recoding() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // One helper per thread so the two orders are otherwise identical. The strike always
    // carries the LOWER HLC, so the inline coding always WINS — only arrival order differs.
    let build = |med: Uuid, low: cairn_event::Hlc, high: cairn_event::Hlc| {
        let strike = cairn_node::medication::build_coding_correction_body(
            Uuid::now_v7(),
            med,
            patient,
            &cairn_node::medication::CorrectCodingInput {
                corrects: Uuid::now_v7(),
                claim: CodingClaim::Strike,
                note: None,
            },
            &kid,
            low,
        );
        let asserted = cairn_node::medication::build_assert_body(
            Uuid::now_v7(),
            med,
            patient,
            &coded_input("Lipitor", MOIETY_ATORVASTATIN),
            &kid,
            high,
        );
        (strike, asserted)
    };

    // Thread 1: the strike arrives FIRST (node B's order).
    let med_strike_first = Uuid::now_v7();
    let low = db::next_hlc(&c, "test-node").await.unwrap();
    let high = db::next_hlc(&c, "test-node").await.unwrap();
    let (strike, asserted) = build(med_strike_first, low, high);
    submit_at(&c, &sk, strike).await;
    submit_at(&c, &sk, asserted).await;

    // Thread 2: the assertion arrives FIRST (node A's order). Same HLC relationship.
    let med_assert_first = Uuid::now_v7();
    let low = db::next_hlc(&c, "test-node").await.unwrap();
    let high = db::next_hlc(&c, "test-node").await.unwrap();
    let (strike, asserted) = build(med_assert_first, low, high);
    submit_at(&c, &sk, asserted).await;
    submit_at(&c, &sk, strike).await;

    let strike_first = coding_state(&c, med_strike_first).await;
    let assert_first = coding_state(&c, med_assert_first).await;
    assert_eq!(
        strike_first, assert_first,
        "the same two events in two arrival orders must project identically — \
         struck-first gave {strike_first:?}, assert-first gave {assert_first:?}"
    );
    assert_eq!(
        strike_first,
        Some((
            Some("drugref-moiety".to_string()),
            Some("atorvastatin".to_string()),
            false
        )),
        "the winning inline coding is live, so the row is NOT struck"
    );
}

/// The same invariant stated directly: nothing may ever leave a row claiming BOTH a live
/// anchor and a strike. `struck` means exactly "this thread has no drug identity", so it is
/// definitionally `coding_code IS NULL`; a row that says otherwise is incoherent whatever
/// produced it. Scanned over the whole table rather than one thread, so any future writer
/// that reintroduces the drift is caught by this test and not by a puzzled reader.
#[tokio::test]
async fn no_row_ever_claims_both_a_live_anchor_and_a_strike() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Exercise every writer of the table: inline (db/031), overlay (db/042 part 5),
    // correction-replacement and correction-strike (db/042 part 6) — plus the ordering
    // that used to drift.
    let med = Uuid::now_v7();
    let low = db::next_hlc(&c, "test-node").await.unwrap();
    let high = db::next_hlc(&c, "test-node").await.unwrap();
    submit_at(
        &c,
        &sk,
        cairn_node::medication::build_coding_correction_body(
            Uuid::now_v7(),
            med,
            patient,
            &cairn_node::medication::CorrectCodingInput {
                corrects: Uuid::now_v7(),
                claim: CodingClaim::Strike,
                note: None,
            },
            &kid,
            low,
        ),
    )
    .await;
    submit_at(
        &c,
        &sk,
        cairn_node::medication::build_assert_body(
            Uuid::now_v7(),
            med,
            patient,
            &coded_input("Lipitor", MOIETY_ATORVASTATIN),
            &kid,
            high,
        ),
    )
    .await;

    let incoherent: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_coding \
               WHERE struck IS DISTINCT FROM (coding_code IS NULL)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        incoherent, 0,
        "struck must mean exactly `coding_code IS NULL` on every row"
    );
}

/// FINDING 2 — the anchor-conflict view must not list a struck member as a phantom anchor.
///
/// Before 6b the anchor columns were NOT NULL, so the view's inner JOIN could not feed a
/// NULL into `array_agg`. Widening them broke that silently, because — unlike
/// `count(DISTINCT …)`, which skips NULLs — `array_agg` KEEPS them. A group of two live
/// anchors plus one struck member therefore reported `anchor_count = 2` beside an `anchors`
/// array of three elements, one of them NULL: a blank row in the human-readable listing the
/// view exists to produce, and a hard failure for any client reading the column as a
/// non-nullable `text[]` (this suite's own sibling test does exactly that).
///
/// The three-member shape is the one the existing struck tests skip: with only two members
/// the strike drops the count to 1 and the row vanishes before anyone can read `anchors`.
#[tokio::test]
async fn a_struck_member_is_not_listed_as_an_anchor_in_a_conflicted_group() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Three threads reconciled into ONE group: two distinct live anchors (the conflict
    // itself) plus a third whose coding is struck.
    let a = cairn_node::medication::assert_medication(
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
    let b = cairn_node::medication::assert_medication(
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
    let third = cairn_node::medication::assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("little white pill", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    for other in [b, third] {
        cairn_node::medication::reconcile_medications(
            &mut c,
            &sk,
            &kid,
            "test-node",
            patient,
            a,
            other,
            &cairn_node::medication::ReconcileInput {
                provenance: "clinician-judgment",
                reason: None,
            },
            None,
            None,
        )
        .await
        .unwrap();
    }
    strike_coding(&mut c, &sk, &kid, patient, third).await;

    // Reading `anchors` as Vec<String> is the assertion: a NULL element makes this row
    // undeliverable to an ordinary client, which is half the defect.
    let (n, anchors): (i64, Vec<String>) = c
        .query_one(
            "SELECT anchor_count, anchors FROM medication_group_coding_conflict \
               WHERE group_id IN (SELECT group_id FROM medication_group_member \
                                   WHERE medication_id = $1::text::uuid)",
            &[&a.to_string()],
        )
        .await
        .map(|r| (r.get(0), r.get(1)))
        .expect("two live anchors in one group still conflict");
    assert_eq!(n, 2, "the struck member contributes no anchor: {anchors:?}");
    assert_eq!(
        anchors.len(),
        2,
        "and it must not appear in the listing either — a struck member has no drug \
         identity to disagree with: {anchors:?}"
    );
}
