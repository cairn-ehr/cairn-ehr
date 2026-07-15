//! Issue #199 (review finding B4) — `clinical.medication.*` through the db/020
//! remote-apply door. Before this file, no medication event had ever been driven
//! through `apply_remote_event`: the assert/cessation/dose/reconciliation floor and
//! projections were exercised at the LOCAL door only, and the slice-4 attestation
//! token had never been proven to round-trip the sync seam (the token must travel
//! with the event and the db/005-mirror gate must re-run at apply — exactly where
//! PR #183's M1 class of bug lives). Also closes the #176 deferral: the db/033
//! oversize-group REMOTE branch (clamp-and-flag, never veto) finally has a harness.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`
//! (shared-DB + TRUNCATE pattern, like apply_remote_event.rs).
use cairn_event::{event_address, generate_key, sign, sign_attestation, Hlc, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    build_assert_body, build_attestation_body, build_cease_body, build_dose_change_body,
    build_reconcile_body, AssertMedicationInput, CeaseMedicationInput, ChangeDoseInput,
    ReconcileInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21), mirroring apply_remote_event.rs.
const WALL_2026: i64 = 1_782_000_000_000;

/// The Postgres RAISE text (Display on a DB error is just "db error" — see
/// apply_remote_event.rs / identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log + every medication projection, reset the HLC, and enroll one
/// device signer (the peer's authoring key) + one human attester.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, medication_statement, \
         medication_cessation, medication_dose_event, medication_dose_correction, \
         medication_reconciliation, medication_group_member, medication_projection_flag, \
         medication_attestation, medication_patient_conflict_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"peer-ward-terminal\"}', $1)",
        &[&kid_d],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_d, kid_d, sk_h, kid_h)
}

/// An HLC "as minted on the peer" — the events in this file arrive by sync, so their
/// clocks are the remote node's, not ours.
fn peer_hlc(wall: i64) -> Hlc {
    Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    }
}

/// Apply one signed event through the 1-arg remote door (no attestation travelled).
async fn apply(c: &Client, signed: &[u8]) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&signed]).await
}

fn sample_assert_input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "metformin",
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("500"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2023"),
        started_precision: Some("year"),
    }
}

/// Sign + apply a peer-authored medication ASSERT for `patient`, returning the
/// minted thread id.
async fn apply_assert(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64) -> Uuid {
    let med = Uuid::now_v7();
    let body = build_assert_body(
        Uuid::now_v7(),
        med,
        patient,
        &sample_assert_input(),
        kid,
        peer_hlc(wall),
    );
    apply(c, &sign(&body, sk).unwrap().signed_bytes)
        .await
        .expect("peer medication assert applies");
    med
}

/// Sign + apply a peer-authored reconciliation edge between two threads.
async fn apply_reconcile(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    a: Uuid,
    b: Uuid,
    wall: i64,
) -> Result<u64, tokio_postgres::Error> {
    let body = build_reconcile_body(
        Uuid::now_v7(),
        a,
        b,
        patient,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        kid,
        peer_hlc(wall),
    );
    apply(c, &sign(&body, sk).unwrap().signed_bytes).await
}

/// The current-medication terms for a patient (through the group-collapsed view).
async fn current_terms(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT term FROM patient_medication_current WHERE patient_id = $1::text::uuid ORDER BY term",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

// ---------------------------------------------------------------------------
// #176 — the db/033 oversize-group REMOTE branch: clamp-and-flag, never veto.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oversize_group_clamps_and_flags_at_apply_never_vetoes() {
    // A validly-signed reconciliation that grows a connected component past the
    // node-local cap arrives BY SYNC. Peers already accepted it, so a local veto
    // would fork the event set (ADR-0045's convergence discipline): the event must
    // be ADMITTED, the group recompute skipped, and the pathology surfaced in
    // medication_projection_flag — mirroring identity's oversize clamp (A5b).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    c.batch_execute("SET cairn.max_medication_group_size = 3")
        .await
        .unwrap();
    let patient = Uuid::now_v7();

    // Four threads; three edges chain them into one component of 4 > cap 3.
    let a = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;
    let b = apply_assert(&c, &sk, &kid, patient, WALL_2026 + 1).await;
    let d3 = apply_assert(&c, &sk, &kid, patient, WALL_2026 + 2).await;
    let d4 = apply_assert(&c, &sk, &kid, patient, WALL_2026 + 3).await;
    apply_reconcile(&c, &sk, &kid, patient, a, b, WALL_2026 + 10)
        .await
        .expect("size-2 group applies"); // {a,b}
    apply_reconcile(&c, &sk, &kid, patient, b, d3, WALL_2026 + 11)
        .await
        .expect("at-cap group applies"); // {a,b,d3} == cap
    apply_reconcile(&c, &sk, &kid, patient, d3, d4, WALL_2026 + 12)
        .await
        .expect("over-cap reconciliation is ADMITTED at apply (clamp-and-flag, never veto)");

    // The edge landed in the event log AND the standing edge overlay (set-union held).
    let edges: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_reconciliation WHERE state = 'reconciled'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(edges, 3, "all three reconciled edges are standing");

    // The recompute was SKIPPED, not truncated: d4 never joined the group.
    let d4_group: String = c
        .query_one(
            "SELECT coalesce((SELECT group_id FROM medication_group_member \
              WHERE medication_id = $1::text::uuid), $1::text::uuid)::text",
            &[&d4.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        d4_group,
        d4.to_string(),
        "over-cap recompute skipped — d4 stays its own group"
    );

    // The pathology is LOUD: a flag row with the observed size and the cap.
    let (observed, cap): (i32, i32) = {
        let row = c
            .query_one(
                "SELECT observed_size, cap FROM medication_projection_flag \
                 ORDER BY flag_id DESC LIMIT 1",
                &[],
            )
            .await
            .expect("clamp left a medication_projection_flag row");
        (row.get(0), row.get(1))
    };
    assert_eq!(
        (observed, cap),
        (4, 3),
        "flag names the observed size and cap"
    );
}

// ---------------------------------------------------------------------------
// The content verbs through the remote door: project + set-union idempotence.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn medication_assert_applies_projects_and_reapply_is_a_noop() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let patient = Uuid::now_v7();

    let body = build_assert_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        patient,
        &sample_assert_input(),
        &kid,
        peer_hlc(WALL_2026),
    );
    let signed = sign(&body, &sk).unwrap();
    apply(&c, &signed.signed_bytes)
        .await
        .expect("peer medication assert applies");
    assert_eq!(
        current_terms(&c, patient).await,
        vec!["metformin".to_string()]
    );

    // Idempotent re-apply (set-union): a second delivery is a silent no-op, and the
    // projection does not double up.
    apply(&c, &signed.signed_bytes)
        .await
        .expect("re-apply of the same bytes is a silent no-op");
    let rows: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 1, "one statement row after a duplicate delivery");
}

#[tokio::test]
async fn cessation_arriving_before_its_assert_converges_either_order() {
    // Offline-first arrival-order independence at the SYNC door: the cessation of a
    // thread can arrive before the assert that minted it. Both must apply, and once
    // both are here the thread reads as PAST, exactly as if they arrived in order.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let patient = Uuid::now_v7();
    let med = Uuid::now_v7();

    // Cessation first — an orphan on arrival.
    let cease = build_cease_body(
        Uuid::now_v7(),
        med,
        patient,
        &CeaseMedicationInput {
            stopped: Some("2026-05"),
            stopped_precision: Some("month"),
            reason: Some("completed course"),
        },
        &kid,
        peer_hlc(WALL_2026 + 100),
    );
    apply(&c, &sign(&cease, &sk).unwrap().signed_bytes)
        .await
        .expect("orphan cessation applies (arrival order is not ours to choose)");

    // The assert lands later.
    let assert_body = build_assert_body(
        Uuid::now_v7(),
        med,
        patient,
        &sample_assert_input(),
        &kid,
        peer_hlc(WALL_2026),
    );
    apply(&c, &sign(&assert_body, &sk).unwrap().signed_bytes)
        .await
        .expect("the late assert applies");

    assert_eq!(
        current_terms(&c, patient).await,
        Vec::<String>::new(),
        "a ceased thread is not current, whatever the arrival order"
    );
    let past: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_past WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        past, 1,
        "the thread reads as past once both events are here"
    );
}

#[tokio::test]
async fn dose_change_applies_and_drives_the_current_dose() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let patient = Uuid::now_v7();
    let med = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;

    let change = build_dose_change_body(
        Uuid::now_v7(),
        med,
        patient,
        &ChangeDoseInput {
            dose_amount: Some("1000"),
            dose_unit: Some("mg"),
            effective: Some("2026-06"),
            effective_precision: Some("month"),
            info_source: "clinician",
            reason: Some("HbA1c above target"),
        },
        &kid,
        peer_hlc(WALL_2026 + 50),
    );
    apply(&c, &sign(&change, &sk).unwrap().signed_bytes)
        .await
        .expect("peer dose change applies");

    // The bitemporal dose timeline drives the current view: the synced change wins.
    let dose: Option<String> = c
        .query_one(
            "SELECT dose_amount FROM patient_medication_current \
             WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        dose.as_deref(),
        Some("1000"),
        "the synced dose change drives the displayed current dose"
    );
}

// ---------------------------------------------------------------------------
// The slice-4 attestation token round trip at the sync seam (the B4 headline):
// responsibility travels WITH the event and the db/005-mirror gate re-runs at apply.
// ---------------------------------------------------------------------------

/// Build a signed medication-attestation event vouching for `med`'s CURRENT local
/// content-set (commitment read from the same fn the floor uses, so a fresh vouch
/// reads non-stale). `claimed_kid` is who the body's responsibility contributor
/// names; `signer_kid`/`signer` actually sign the envelope. The two coincide in the
/// honest flow — the #195 test drives them apart (a validly-signed body claiming a
/// DIFFERENT human than the one whose token travels).
async fn signed_attestation(
    c: &Client,
    med: Uuid,
    patient: Uuid,
    claimed_kid: &str,
    signer_kid: &str,
    signer: &SigningKey,
    wall: i64,
) -> Vec<u8> {
    let (commitment, count): (String, i64) = {
        let row = c
            .query_one(
                "SELECT encode(cairn_medication_thread_commitment($1::text::uuid), 'hex'), \
                 (SELECT count(*) FROM event_log \
                   WHERE (body ->> 'medication_id')::uuid = $1::text::uuid \
                     AND event_type <> 'clinical.medication-attestation.asserted')",
                &[&med.to_string()],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    let mut body = build_attestation_body(
        Uuid::now_v7(),
        med,
        patient,
        &commitment,
        count as u32,
        Some("current-visit-review"),
        None,
        claimed_kid,
        peer_hlc(wall),
    );
    // The envelope's signer is a separate claim from the body's responsibility
    // contributor (principle 10: authorship ≠ accountability): pin the signer here so
    // the signature verifies for `signer` even when the body claims someone else.
    body.signer_key_id = signer_kid.to_string();
    sign(&body, signer).unwrap().signed_bytes
}

#[tokio::test]
async fn unattested_medication_attestation_is_refused_at_apply() {
    // The event type carries a responsibility contributor, so the db/020 gate must
    // demand the token — an attestation whose token did NOT travel is refused, exactly
    // as submit_event would refuse it locally. ONE floor, two doors.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();
    let med = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;

    let signed = signed_attestation(&c, med, patient, &kid_h, &kid_h, &sk_h, WALL_2026 + 60).await;
    let err = apply(&c, &signed).await.expect_err("no token travelled");
    assert!(
        db_msg(&err).contains("attestation"),
        "refusal names the missing attestation: {}",
        db_msg(&err)
    );
    let rows: i64 = c
        .query_one("SELECT count(*) FROM medication_attestation", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 0, "no vouch row landed");
}

#[tokio::test]
async fn attestation_token_round_trips_and_projects_the_verified_attester() {
    // The happy path the review called out as never exercised: a valid human token
    // bound to the event's content-address travels with it, the apply door verifies
    // it, stores it for re-serving, and the db/034 projection records the VERIFIED
    // attester (attester_key), never merely the signer.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();
    let med = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;

    let signed = signed_attestation(&c, med, patient, &kid_h, &kid_h, &sk_h, WALL_2026 + 60).await;
    let ca = event_address(&signed);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed, &token, &hkey],
    )
    .await
    .expect("attested medication vouch applies at the sync door");

    // Projection: the standing vouch names the verified human and reads NON-stale
    // (the commitment it reviewed is the thread's current content set).
    let (attester, stale): (String, bool) = {
        let row = c
            .query_one(
                "SELECT attester_kid, stale FROM medication_thread_attestation \
                 WHERE medication_id = $1::text::uuid",
                &[&med.to_string()],
            )
            .await
            .expect("the vouch projects");
        (row.get(0), row.get(1))
    };
    assert_eq!(attester, kid_h, "projection keys on the VERIFIED attester");
    assert!(!stale, "a vouch for the current content set is not stale");

    // The proof is STORED so this node can re-serve it to its own peers (the token
    // must keep travelling with the event).
    let (att, akey): (Option<Vec<u8>>, Option<Vec<u8>>) = {
        let row = c
            .query_one(
                "SELECT attestation, attester_key FROM event_log \
                 WHERE event_type = 'clinical.medication-attestation.asserted'",
                &[],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert_eq!(att.as_deref(), Some(&token[..]), "token stored verbatim");
    assert_eq!(akey.as_deref(), Some(&hkey[..]), "attester key stored");
}

#[tokio::test]
async fn attestation_token_from_a_non_human_is_refused_at_apply() {
    // A token minted by an enrolled DEVICE key must not confer responsibility: the
    // apply door re-checks kind='human' exactly like db/005 (forged human authorship).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let patient = Uuid::now_v7();
    let med = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;

    // The device key signs the event, mints the token, AND is claimed as attester.
    let signed = signed_attestation(&c, med, patient, &kid, &kid, &sk, WALL_2026 + 60).await;
    let ca = event_address(&signed);
    let token = sign_attestation(&ca, &kid, "attested", &sk).unwrap();
    let dkey = hex::decode(&kid).unwrap();
    let err = c
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&signed, &token, &dkey],
        )
        .await
        .expect_err("a device key cannot vouch");
    assert!(
        db_msg(&err).contains("human"),
        "refusal names the non-human attester: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn responsibility_claim_for_another_actor_is_refused_at_apply() {
    // #195 at the medication surface: the body's responsibility contributor must name
    // the human whose token was verified. A vouch body claiming human B, carried by a
    // valid token from human A, is an unverified responsibility claim — refused.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    // A second enrolled human — the one the body will (falsely) claim. The pinned set
    // needs a person-distinguishing determinant or ADR-0044 refuses the enroll as an
    // actor_id collision with the first clinician.
    let (_sk_h2, kid_h2) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"person\":\"second-clinician\"}', $1)",
        &[&kid_h2],
    )
    .await
    .unwrap();
    let patient = Uuid::now_v7();
    let med = apply_assert(&c, &sk, &kid, patient, WALL_2026).await;

    // Body claims kid_h2 as the responsible human, but kid_h signs it and mints the token.
    let signed = signed_attestation(&c, med, patient, &kid_h2, &kid_h, &sk_h, WALL_2026 + 60).await;
    let ca = event_address(&signed);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    let err = c
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&signed, &token, &hkey],
        )
        .await
        .expect_err("responsibility must bind to the verified attester");
    assert!(
        db_msg(&err).contains("attester"),
        "refusal names the unbound claim: {}",
        db_msg(&err)
    );
}
