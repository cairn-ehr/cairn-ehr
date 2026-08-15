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
const MOIETY_OVERLAY: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e16";
const MOIETY_OTHER_THREAD: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e1a";
const MOIETY_BLANK_CLASS: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e17";
const MOIETY_BLANK_SEVERITY: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e18";
const MOIETY_BLANK_BOTH: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e19";

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
async fn the_coding_overlay_emits_a_signal_and_a_thread_grade_coarsens_it() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(&c, MOIETY_OVERLAY, "rh-sensitizing", "high").await;

    // THIS TEST COVERS TWO GAPS THE REST OF THE SUITE LEAVES OPEN (2026-08-14 review).
    //
    // 1. THE OVERLAY HALF OF #294. `code_medication` — the pharmacist/professional-coder
    //    path that ADR-0059 names as the motivating case — is the OTHER emission seam, and
    //    nothing drove it. #294's obligation is that the class be CARRIED rather than
    //    re-derived; `safety_carried_class.rs` discharges that for `assert_medication`
    //    only, so the verb the obligation was actually written about was untested.
    //
    // 2. A THREAD-SCOPED GRADE REACHING EMISSION AT ALL. `assert_medication` mints a fresh
    //    `medication_id` per call, so no standing thread-scoped assertion can ever name it;
    //    `code_medication` is the only verb that writes a precise claim onto an EXISTING
    //    thread, and so the only end-to-end path where a thread grade can bite.
    //
    // WHAT THIS TEST PINS, AND WHERE ITS OTHER HALF LIVES. Verified by mutation: dropping
    // the overlay's safety claim fails it. It does NOT pin the thread PLUMBING —
    // `sealed_submit.rs`'s read of `payload.medication_id` into `prospective_rung` — because
    // a MATCHING thread and a NULL thread both coarsen, so hardcoding `None` leaves this arm
    // green. Its sibling
    // `a_grade_on_another_thread_of_the_same_chart_does_not_coarsen_this_one` is the one
    // that catches that, and it only became writable once #404 stopped the two thread arms
    // being exhaustive. The pair belongs together: this one proves a thread grade REACHES
    // emission, that one proves it reaches only the RIGHT thread.
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
    .expect("assert uncoded");

    // The control: the uncoded assert emits nothing, so any signal below is the OVERLAY's.
    let assert_ev = assert_event_of(&c, thread).await;
    assert!(
        stored_signal(&c, assert_ev).await.is_none(),
        "the uncoded assert must emit nothing — otherwise this test cannot tell the \
         overlay's signal from the assertion's"
    );

    // Grade the THREAD, not the chart. This is the granular subject kind ADR-0062 decision
    // 8 wants deployments to reach for, and the arm no end-to-end test had exercised.
    cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Thread,
        thread,
        "sensitive",
        Some("test fixture: thread-scoped grade"),
    )
    .await
    .expect("grade the thread");

    let coding_ev = cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        thread,
        &cairn_node::medication::CodeMedicationInput {
            coding: SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_OVERLAY,
                display: "anti-D immunoglobulin",
            },
        },
        None,
        None,
    )
    .await
    .expect("code the thread");

    let s = stored_signal(&c, coding_ev)
        .await
        .expect("a coding overlay emits a signal — this is the seam #294 is about");
    assert_eq!(
        s["rung"], "kind",
        "a thread-scoped grade must reach the overlay's rung — if this reads `precise`, \
         grading a thread is doing nothing at emission at all"
    );
    assert!(
        s.get("class").is_none(),
        "the class must not be published in the clear once the thread is graded"
    );
    assert_eq!(s["severity"], "high", "severity survives the middle rung");

    // …and the full precision is under the seal, uncoarsened — the tier a custody-holder
    // reads without any drug database (#294, now discharged for the overlay verb too).
    let clear = clear_payload(&c, coding_ev).await;
    assert_eq!(clear["safety"]["class"], "rh-sensitizing");
    assert_eq!(clear["safety"]["severity"], "high");
}

#[tokio::test]
async fn a_grade_on_another_thread_of_the_same_chart_does_not_coarsen_this_one() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    map_class(&c, MOIETY_OTHER_THREAD, "rh-sensitizing", "high").await;

    // THE #404 PIN, AND THE ONE THIS SUITE COULD NOT WRITE BEFORE.
    //
    // `cairn_prospective_sensitivity`'s thread arms used to be EXHAUSTIVE — the matched arm
    // fired on `subject_id = p_thread`, the catch-all on `p_thread IS NULL OR subject_id <>
    // p_thread` — so a thread-scoped grade coarsened UNCONDITIONALLY and `p_thread` was
    // inert. Two things followed, both bad:
    //
    //   * thread-scoping behaved exactly like chart-scoping at emission, which is what
    //     db/048 section 10b's type gate exists to prevent; and
    //   * emission disagreed with read ON THE SAME NODE — `cairn_effective_sensitivity`
    //     computes `routine` for this event, so the chart report would print
    //     "break glass" beside "grade routine, winning subject: none".
    //
    // db/049 now mirrors db/048's actual predicate: the catch-all fires only when the named
    // thread is DEMONSTRABLY on another chart. This suite is where the pin belongs because
    // `medication_setup` gives it custody, so `cairn_thread_patient` genuinely RESOLVES a
    // thread here — the reason safety_read.rs's version of this test could only reach the
    // unresolvable case.
    //
    // It is also the test that finally catches the thread PLUMBING: with the arms no longer
    // exhaustive, hardcoding `apply_safety_rung`'s thread lookup to `None` sends this down
    // the `p_thread IS NULL` bound and reads `kind` instead of `precise`.
    let graded_thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("the sensitive one", MOIETY_ANTI_D),
        None,
        None,
    )
    .await
    .expect("thread A");

    let other_thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &AssertMedicationInput {
            term: "an unrelated medication",
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
    .expect("thread B");

    // Grade thread A only. Thread B is a different medication on the same chart and is not
    // sensitive — the whole point of offering a thread-scoped subject kind.
    cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Thread,
        graded_thread,
        "sensitive",
        Some("test fixture: grade thread A only"),
    )
    .await
    .expect("grade thread A");

    let coding_ev = cairn_node::medication::code_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        other_thread,
        &cairn_node::medication::CodeMedicationInput {
            coding: SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_OTHER_THREAD,
                display: "an unrelated medication",
            },
        },
        None,
        None,
    )
    .await
    .expect("code thread B");

    let s = stored_signal(&c, coding_ev)
        .await
        .expect("thread B's coding still emits a signal");
    assert_eq!(
        s["rung"], "precise",
        "a grade on ANOTHER thread must not coarsen this one — that is the difference \
         between thread-scoping and chart-scoping, and #404 was it collapsing"
    );
    assert_eq!(s["class"], "rh-sensitizing");

    // The control that stops this passing for the wrong reason: emission must AGREE with
    // what every later read of this very event will compute. If these two ever diverge, the
    // node publishes one answer and displays another.
    let effective: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&coding_ev.to_string()],
        )
        .await
        .expect("effective grade")
        .get(0);
    assert_eq!(
        effective, "routine",
        "db/048 computes `routine` for an event on an ungraded thread, so emission must \
         too — the divergence #404 describes is exactly these two disagreeing"
    );
}

#[tokio::test]
async fn a_blank_class_map_row_emits_no_signal_and_still_records_the_medication() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;

    // THE POINT OF THIS TEST (2026-08-14 review finding). `safety_class_map`'s columns are
    // NOT NULL but NOT non-blank, so a deployment populating the map from a drugref export
    // can land a row with a blank class or a blank severity. Both shapes are exactly what
    // `cairn_check_safety_signal` REFUSES — at the STRICT door, the one every medication
    // write goes through. If `usable_precise_claim` did not intercept them first, one bad
    // configuration row would cancel every medication assert naming that drug, forever, on
    // every node. That is ADR-0060's sharpest case: "the system may fail to record an
    // order, but it may never cancel one."
    //
    // The existing coverage for this is unit-level only (`usable_precise_claim`'s own
    // tests, and two `apply_safety_rung` tests that return before any query). Neither
    // proves the STRICT DOOR admits the resulting event. This one drives the real door.
    for (code, class, severity) in [
        (MOIETY_BLANK_CLASS, "", "high"),
        (MOIETY_BLANK_SEVERITY, "rh-sensitizing", ""),
        (MOIETY_BLANK_BOTH, "   ", "   "),
    ] {
        map_class(&c, code, class, severity).await;

        let thread = assert_medication(
            &mut c,
            &sk,
            &kid,
            "n1",
            patient,
            &coded("a drug with a misconfigured map row", code),
            None,
            None,
        )
        .await
        .unwrap_or_else(|e| {
            panic!(
                "a blank class map row (class={class:?}, severity={severity:?}) must NOT \
                 cancel the clinical write — ADR-0060. Error was: {e:#}"
            )
        });

        // The clinical content landed…
        let ev = assert_event_of(&c, thread).await;
        // …and the advisory decoration withheld itself rather than minting a body the
        // door would refuse.
        assert!(
            stored_signal(&c, ev).await.is_none(),
            "a half-formed class must emit NO clear signal (class={class:?}, \
             severity={severity:?}) — a `precise` rung with a blank class is precisely \
             what the strict door refuses"
        );
    }
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

// ---------------------------------------------------------------------------
// ADR-0060's THIRD route: an advisory lookup that ERRORS.
//
// ADR-0063 decision 8 closes two routes by which an advisory field could cancel a
// clinical write — the apply door (decision 6) and a body the LOCAL door would refuse
// (`usable_precise_claim`). Propagating an ERROR out of either lookup with `?` was a
// third: the medication assertion would never be written at all, and the error would
// name a safety class no clinician caused. "The system may fail to record an order, but
// it may never cancel one" (ADR-0060) applies to the error path exactly as it applies to
// the other two, so both lookups now fall back in the WITHHOLDING direction.
// ---------------------------------------------------------------------------

const MOIETY_CLASS_OUTAGE: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e16";
const MOIETY_GRADE_OUTAGE: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e17";

/// db/049 exactly as this build embeds it — replayed to PUT BACK a function that the two
/// tests below deliberately replace with one that raises.
///
/// Restoring from the migration file itself (rather than from a hand-copied definition)
/// is what keeps the restore from drifting away from the thing it restores: if db/049's
/// function body changes, so does the text used to put it back.
const DB049: &str = include_str!("../../../db/049_safety_projection.sql");

/// Stage a TRANSIENT outage of one advisory lookup by replacing it, in this database,
/// with a same-signature function that raises.
///
/// # Why this staging, for a junior reader
///
/// The failure we must prove harmless is "the lookup returned `Err`" — a statement
/// timeout, a lock, a dropped plan, a missing grant on a future role split. None of those
/// can be provoked from Rust on demand, so the test provokes the one thing they all
/// reduce to: the statement raises.
///
/// The residue is SELF-HEALING even if a test is killed between the break and the
/// restore. `connect_and_load_schema` replays every `db/*.sql` on every connect, and
/// db/049's own `CREATE OR REPLACE` puts the real function back — so the next DB-gated
/// test in any suite repairs the database on its way in. The explicit restore below is
/// for the psql-driven SQL mirror (`db/tests/049_…`), which does not replay the schema.
async fn break_advisory_lookup(c: &tokio_postgres::Client, ddl: &str) {
    c.batch_execute(ddl)
        .await
        .expect("stage the advisory outage");
}

/// Put db/049 back exactly as the schema loader would.
async fn restore_advisory_lookups(c: &tokio_postgres::Client) {
    c.batch_execute(DB049)
        .await
        .expect("restore db/049 after the staged outage");
}

/// The class lookup, replaced by one that raises. Argument NAMES must match db/049's or
/// `CREATE OR REPLACE` refuses ("cannot change name of input parameter").
const BREAK_CLASS_LOOKUP: &str = r#"
CREATE OR REPLACE FUNCTION cairn_safety_class_candidate(p_coding jsonb)
RETURNS TABLE (class text, severity text)
LANGUAGE plpgsql STABLE AS $outage$
BEGIN
    RAISE EXCEPTION 'staged advisory outage: the class lookup cannot run';
END;
$outage$;
"#;

/// The prospective-grade lookup, replaced by one that raises.
const BREAK_GRADE_LOOKUP: &str = r#"
CREATE OR REPLACE FUNCTION cairn_prospective_sensitivity(p_patient uuid, p_thread uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE plpgsql STABLE AS $outage$
BEGIN
    RAISE EXCEPTION 'staged advisory outage: the prospective grade cannot be read';
END;
$outage$;
"#;

#[tokio::test]
async fn a_failing_class_lookup_still_records_the_medication() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;
    // A class IS configured for this drug — so a signal WOULD be emitted if the lookup
    // could run. The absence asserted below is therefore about the outage, not about the
    // suite having forgotten to seed the map.
    map_class(&c, MOIETY_CLASS_OUTAGE, "rh-sensitizing", "high").await;
    break_advisory_lookup(&c, BREAK_CLASS_LOOKUP).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("anti-D immunoglobulin", MOIETY_CLASS_OUTAGE),
        None,
        None,
    )
    .await;

    // Restored BEFORE the assertions, so a failing assertion still leaves the database
    // usable for the SQL mirror.
    restore_advisory_lookups(&c).await;
    let thread = thread.expect(
        "an advisory class lookup that cannot run must not cancel the clinical write — \
         the system may fail to record an order, but it may never cancel one (ADR-0060)",
    );

    let ev = assert_event_of(&c, thread).await;
    assert!(
        stored_signal(&c, ev).await.is_none(),
        "an error is not a class: with no class learned there is no signal, and inventing \
         one from a failed lookup would manufacture a warning from nothing"
    );
    assert!(
        clear_payload(&c, ev).await.get("safety").is_none(),
        "and nothing under the seal either — there was nothing to look up"
    );
}

#[tokio::test]
async fn a_failing_grade_lookup_falls_back_to_the_coarsest_rung() {
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
        MOIETY_GRADE_OUTAGE,
        "antiretroviral-interaction",
        "critical",
    )
    .await;
    // The CLASS lookup still works here; only the grade cannot be read. The chart carries
    // no standing grade at all, so a working lookup would license `precise` — which makes
    // the `existence` below a real assertion about the fallback rather than a coincidence.
    break_advisory_lookup(&c, BREAK_GRADE_LOOKUP).await;

    let thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &coded("tenofovir", MOIETY_GRADE_OUTAGE),
        None,
        None,
    )
    .await;

    restore_advisory_lookups(&c).await;
    let thread = thread
        .expect("a grade lookup that cannot run must not cancel the clinical write (ADR-0060)");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev)
        .await
        .expect("the class was learned, so a signal is still emitted");
    assert_eq!(
        s["rung"], "existence",
        "a grade this node could not read must disclose NOTHING: the fallback goes to the \
         coarsest rung, never to the finest one the chart happens to license"
    );
    assert!(
        s.get("class").is_none(),
        "the class must not ride out in the clear on a fallback rung"
    );
    assert!(s.get("severity").is_none());

    // The sealed tier is untouched by the outage: a custody-holder still gets the whole
    // claim, because the seal — not the grade — is what protects it (#294).
    let clear = clear_payload(&c, ev).await;
    assert_eq!(clear["safety"]["class"], "antiretroviral-interaction");
    assert_eq!(clear["safety"]["severity"], "critical");
}
