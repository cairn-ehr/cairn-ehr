//! §5.9 part B (ADR-0063) — EMISSION. The rung is chosen from the grade standing at
//! AUTHORING time; the precise class goes under the seal. This is the only coarsening that
//! binds a peer's raw-SQL client, because it decides what is put on the wire at all.
//!
//! # The two tiers, and why this file pins both at once
//!
//! A coded drug's interaction class is a drug-knowledge lookup, so a READER cannot
//! re-derive it without a drug database — and making the §5.9 safety floor depend on
//! holding one would defeat the floor (ADR-0059 decision 4 / #294). The authoring node, by
//! construction, had a coding authority in hand at that moment. So:
//!
//!   * the precise `{class, severity}` is looked up PRE-SEAL and travels INSIDE the sealed
//!     payload, where the seal is what protects it — never coarsened;
//!   * a RUNG, chosen from the grade standing on the chart right now, travels in the CLEAR
//!     on `event_log.safety`.
//!
//! Every test below asserts on BOTH tiers where both exist, because the whole design fails
//! if either half drifts: a coarsened sealed tier silently destroys the signal a
//! custody-holder is entitled to, and an un-coarsened clear tier publishes exactly the
//! disclosure the grade exists to prevent.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (`cs()` returns `None`), and cargo
//! then reports the suite as passing while running nothing — a green run that prints no
//! test names is a SKIP, not a pass.
mod common;
use cairn_event::sensitivity::SubjectKind;
use cairn_node::medication::{
    assert_medication, reconcile_medications, AssertMedicationInput, ReconcileInput,
    SubstanceCoding,
};
use common::{cs, medication_setup, submit_registration};
use uuid::Uuid;

// Moiety anchors shaped like drugref's: canonical lowercase UUIDs. db/041 registers
// `drugref-moiety` with code_shape 'uuid' and the strict door REFUSES a non-uuid code, so
// a readable stand-in like "moiety-1" never reaches the emission seam at all. Fixed, not
// random, because the tests seed the class map on exactly these values. Not cryptographic
// material, so house rule 6 does not apply (same reasoning as `medication_coding.rs`'s
// `MOIETY_ATORVASTATIN`).
const MOIETY_ANTI_D: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e11";
const MOIETY_TENOFOVIR: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e12";
const MOIETY_ANTI_D_SEQ: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e13";
const MOIETY_UNMAPPED: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e14";
const MOIETY_STATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e15";

/// Take ownership of this suite's slice of deployment state.
///
/// `safety_class_map` ships EMPTY and is deliberately NOT truncated by `medication_setup`
/// (it is deployment configuration, not clinical data), so rows seeded by one test persist
/// into every later test in this long-lived shared database. Each test therefore clears it
/// and seeds exactly what it needs, the same way `setup()` owns the projection tables.
///
/// The sweep is at SETUP, not teardown, for the reason `medication_coding.rs`'s `setup_node`
/// gives about `medication_coding_system`: a run killed mid-test never reaches a teardown,
/// and a leftover row in a shared database is then permanent and invisible. Sweeping on the
/// way IN is the only cleanup that survives being killed. The residue this leaves for OTHER
/// suites is bounded by construction — the moiety codes below are unique to this file, so a
/// row that outlives a killed run matches no other suite's coding.
async fn own_the_class_map(c: &tokio_postgres::Client) {
    c.execute("DELETE FROM safety_class_map", &[])
        .await
        .expect("clear the deployment class map");
}

/// Populate the deployment class map. The shipped table is EMPTY on purpose (Cairn ships
/// the lookup, never the drug knowledge), so a test that wants a class must supply one —
/// exactly as a deployment does.
async fn map_class(c: &tokio_postgres::Client, code: &str, class: &str, severity: &str) {
    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', $1, $2, $3) ON CONFLICT DO NOTHING",
        &[&code, &class, &severity],
    )
    .await
    .expect("seed the map");
}

/// The CLEAR signal stored on an event, or `None`.
///
/// Read through a `::text` cast and parsed back in Rust: this crate does not enable
/// tokio-postgres's `with-serde_json-1` feature, so a JSONB column has no
/// `FromSql<serde_json::Value>` impl at all (the `observed_evidence.rs` idiom, and no new
/// dependency features per the slice's constraints).
async fn stored_signal(c: &tokio_postgres::Client, event: Uuid) -> Option<serde_json::Value> {
    let raw: Option<String> = c
        .query_one(
            "SELECT safety::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    raw.map(|t| serde_json::from_str(&t).expect("the stored signal is json"))
}

/// The CLEAR shadow of a sealed payload — the tier a custody-holding node reads without
/// any drug database (#294). Same `::text` reason as `stored_signal`.
async fn clear_payload(c: &tokio_postgres::Client, event: Uuid) -> serde_json::Value {
    let raw: String = c
        .query_one(
            "SELECT body::text FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .expect("the sealed payload's clear shadow")
        .get(0);
    serde_json::from_str(&raw).expect("the clear shadow is json")
}

/// The event id of a thread's assertion (the thread's own assert event).
///
/// `assert_medication` returns the THREAD id, not the event id, so the event is recovered
/// through the projection row the assert wrote. `event_id` is read as `::text` and parsed:
/// `with-uuid-1` is not enabled either, so `Uuid` has no `FromSql` impl.
async fn assert_event_of(c: &tokio_postgres::Client, thread: Uuid) -> Uuid {
    let raw: String = c
        .query_one(
            "SELECT e.event_id::text FROM event_log e
             JOIN medication_statement m ON m.content_address = e.content_address
             WHERE m.medication_id = $1::text::uuid",
            &[&thread.to_string()],
        )
        .await
        .expect("the assert event")
        .get(0);
    raw.parse().expect("event_id is a uuid")
}

/// A coded medication input. Every field but the coding is the honest-unknown floor —
/// these tests are about the safety seam, not about dose or sig.
fn coded(term: &'static str, code: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: "drugref-moiety",
            code,
            display: term,
        }),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient",
        started: None,
        started_precision: None,
    }
}

/// Mint a chart and give it its registration act.
///
/// Since #345 db/005 step 8b refuses any chart whose first event is not that chart's
/// registration, and every verb in this suite goes through the STRICT door. Wall `0` keeps
/// the birth act below every event the suite then authors.
async fn fresh_chart(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let patient = Uuid::now_v7();
    submit_registration(c, sk, kid, patient, 0).await;
    patient
}

#[tokio::test]
async fn a_coded_assert_on_a_routine_chart_emits_the_precise_rung() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(&c, MOIETY_ANTI_D, "rh-sensitizing", "high").await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("anti-D immunoglobulin", MOIETY_ANTI_D),
        None,
        None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev)
        .await
        .expect("a coded medication emits a signal");
    assert_eq!(s["rung"], "precise", "no standing grade ⇒ full precision");
    assert_eq!(s["class"], "rh-sensitizing");
    assert_eq!(s["severity"], "high");

    // And the precise claim is ALSO under the seal — that is the tier a custody-holding
    // node reads without any drug database (#294).
    let clear = clear_payload(&c, ev).await;
    assert_eq!(clear["safety"]["class"], "rh-sensitizing");
    assert_eq!(clear["safety"]["severity"], "high");
    assert!(
        clear["safety"].get("rung").is_none(),
        "the sealed tier carries no rung — the rung is a disclosure decision, and the \
         sealed side discloses everything to whoever holds the key"
    );
}

#[tokio::test]
async fn a_graded_chart_coarsens_the_emitted_rung_but_never_the_sealed_claim() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(
        &c,
        MOIETY_TENOFOVIR,
        "antiretroviral-interaction",
        "critical",
    )
    .await;

    cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Patient,
        patient,
        "sensitive",
        Some("test fixture"),
    )
    .await
    .expect("grade the chart");

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("tenofovir", MOIETY_TENOFOVIR),
        None,
        None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev).await.expect("still a signal");
    assert_eq!(s["rung"], "kind", "'sensitive' coarsens to kind");
    assert!(
        s.get("class").is_none(),
        "the class must never be published in the clear on a graded chart — it IS the \
         disclosure the grade exists to prevent"
    );
    assert_eq!(
        s["severity"], "critical",
        "severity survives the middle rung"
    );

    let clear = clear_payload(&c, ev).await;
    assert_eq!(
        clear["safety"]["class"], "antiretroviral-interaction",
        "the sealed tier is never coarsened — the seal is what protects it"
    );
    assert_eq!(clear["safety"]["severity"], "critical");
}

#[tokio::test]
async fn a_sequestered_chart_emits_existence_only() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(&c, MOIETY_ANTI_D_SEQ, "rh-sensitizing", "high").await;

    cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Patient,
        patient,
        "sequestered",
        Some("protected witness"),
    )
    .await
    .expect("grade");

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("anti-D", MOIETY_ANTI_D_SEQ),
        None,
        None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev).await.expect("signal");
    assert_eq!(s["rung"], "existence");
    assert!(s.get("class").is_none());
    assert!(
        s.get("severity").is_none(),
        "severity is withheld at the coarsest rung — 'existence' means \"there is \
         something here and you are not cleared to see what\", and a severity beside it \
         would narrow exactly that (maintainer ruling, 2026-08-13)"
    );

    // Still the whole claim under the seal: sequestration withholds it from the WIRE, not
    // from the node that legitimately holds custody.
    let clear = clear_payload(&c, ev).await;
    assert_eq!(clear["safety"]["class"], "rh-sensitizing");
    assert_eq!(clear["safety"]["severity"], "high");
}

#[tokio::test]
async fn an_uncoded_medication_emits_no_signal_at_all() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &AssertMedicationInput {
            term: "little white pill",
            coding: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    assert!(
        stored_signal(&c, ev).await.is_none(),
        "an uncoded medication has no class on ANY node — that is principle 4 being \
         honest, not a degradation, and manufacturing an existence marker for it would \
         reproduce §5.12's alert fatigue on day one (ADR-0059 decision 4)"
    );
    assert!(
        clear_payload(&c, ev).await.get("safety").is_none(),
        "and nothing is written under the seal either — there was nothing to look up"
    );
}

#[tokio::test]
async fn a_coding_absent_from_the_map_emits_no_signal() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    // Deliberately do NOT seed the map: this deployment's coding authority has no opinion
    // about this substance. Absence of knowledge is not a graded secret.
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("atorvastatin", MOIETY_UNMAPPED),
        None,
        None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    assert!(
        stored_signal(&c, ev).await.is_none(),
        "a node with no row for this coding emits nothing rather than guessing"
    );
    assert!(clear_payload(&c, ev).await.get("safety").is_none());
}

#[tokio::test]
async fn the_reconciliation_path_emits_no_signal() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(&c, MOIETY_STATIN, "statin-interaction", "moderate").await;

    let a = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("atorvastatin", MOIETY_STATIN),
        None,
        None,
    )
    .await
    .expect("assert a");
    let b = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("Lipitor", MOIETY_STATIN),
        None,
        None,
    )
    .await
    .expect("assert b");

    // A reconciliation is a link between THREADS, not a drug claim: its body carries no
    // coding, so the emission seam looks up nothing and attaches nothing. The omission is
    // DELIBERATE, so it is pinned rather than left to be rediscovered as a bug — and this
    // is a real assertion, not a tautology: a seam that attached a rung to every event it
    // passed (rather than only to events carrying a precise claim) would fail here.
    let recon = reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        a,
        b,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: Some("brand vs generic"),
        },
        None,
        None,
    )
    .await
    .expect("reconcile");
    assert!(
        stored_signal(&c, recon).await.is_none(),
        "a reconciliation carries no drug claim, so it emits no safety signal"
    );

    // Both threads it links DO carry one, so the absence above is about this event's
    // content and not about the suite having failed to configure the map.
    assert!(
        stored_signal(&c, assert_event_of(&c, a).await)
            .await
            .is_some(),
        "the asserts either side of the reconciliation still emit"
    );
}
