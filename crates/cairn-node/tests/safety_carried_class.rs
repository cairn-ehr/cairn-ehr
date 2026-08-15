//! #294's obligation, discharged: a node with NO local drug knowledge still reports the
//! precise class, proving it was CARRIED rather than re-derived.
//!
//! This is the test crates/cairn-node/tests/medication_coding.rs owed since slice 6a and
//! could not write, because there was no safety projection to fire.
//!
//! Every test here self-skips without `$CAIRN_TEST_PG` (`cs()` returns `None`), and cargo
//! then reports the suite as passing while running nothing — a green run that prints no
//! test names is a SKIP, not a pass.
mod common;
use cairn_event::sensitivity::SubjectKind;
use cairn_node::medication::{assert_medication, AssertMedicationInput, SubstanceCoding};
use common::{cs, medication_setup, submit_registration};
use uuid::Uuid;

// Moiety anchors shaped like drugref's: canonical lowercase UUIDs. db/041 registers
// `drugref-moiety` with code_shape 'uuid' and the strict door REFUSES a non-uuid code
// (ruling T3-A of the task-6 brief), so a readable stand-in like "m-294" never reaches the
// emission seam at all. Fixed, not random, because the tests seed the class map on exactly
// these values; distinct from `safety_emission.rs`'s constants so a row left behind by a
// killed run in this file can never be mistaken for that suite's. Not cryptographic
// material, so house rule 6 does not apply.
const MOIETY_ANTI_D_294: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e21";
const MOIETY_TENOFOVIR_SEQ: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e22";

/// Take ownership of this suite's slice of deployment state.
///
/// `safety_class_map` ships EMPTY and is deliberately NOT truncated by `medication_setup`
/// (ruling P1 of the task-6 brief: it is deployment configuration, not clinical data), so a
/// row seeded by one test persists into every later test sharing this long-lived database.
/// Each test clears it before seeding its own row — the same shape `safety_emission.rs`'s
/// `own_the_class_map` uses.
async fn own_the_class_map(c: &tokio_postgres::Client) {
    c.execute("DELETE FROM safety_class_map", &[])
        .await
        .expect("clear the deployment class map");
}

/// Mint a fresh patient and immediately register the chart.
///
/// Ruling P2 of the task-6 brief: since #345, `db/005` step 8b refuses any chart whose
/// FIRST event is not that chart's registration. Both tests below author through
/// `assert_medication` (the strict door), so a bare `Uuid::now_v7()` with no registration
/// would have its very first write refused — the same shape `safety_emission.rs`'s
/// `fresh_chart` uses.
async fn fresh_chart(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let patient = Uuid::now_v7();
    submit_registration(c, sk, kid, patient, 0).await;
    patient
}

#[tokio::test]
async fn a_node_with_an_empty_class_map_still_reports_the_carried_class() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;

    // The AUTHORING node has a coding authority: one row in the map.
    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', $1, 'rh-sensitizing', 'high')",
        &[&MOIETY_ANTI_D_294],
    )
    .await
    .expect("seed");

    let _thread = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &AssertMedicationInput {
            term: "anti-D immunoglobulin",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_ANTI_D_294,
                display: "anti-D immunoglobulin",
            }),
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

    // Now become a node with NO drug knowledge at all. The map is where every scrap of
    // local class knowledge lives, so emptying it — AFTER authoring — is exactly "this
    // node holds no drugref". Nothing else in this fixture can supply the class: the
    // lookup is deleted, and re-deriving it from `MOIETY_ANTI_D_294` alone is impossible
    // without a drug database.
    c.execute("DELETE FROM safety_class_map", &[])
        .await
        .expect("drop all local drug knowledge");

    let lines = cairn_node::safety::chart_safety(&c, patient)
        .await
        .expect("the chart report");
    assert_eq!(lines.len(), 1, "the signal is still there");
    assert_eq!(
        lines[0].class.as_deref(),
        Some("rh-sensitizing"),
        "a drugref-less node honours the §5.9 floor for a CODED medication because the \
         class was captured pre-seal on the coding node and CARRIED — never re-derived \
         (ADR-0059 decision 4 / #294)"
    );
    assert_eq!(lines[0].severity.as_deref(), Some("high"));
}

#[tokio::test]
async fn the_report_names_nothing_beyond_what_the_rung_licenses() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    own_the_class_map(&c).await;
    let patient = fresh_chart(&c, &sk, &kid).await;

    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', $1, 'antiretroviral-interaction', 'critical')",
        &[&MOIETY_TENOFOVIR_SEQ],
    )
    .await
    .expect("seed");
    cairn_node::sensitivity::assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Patient,
        patient,
        "sequestered",
        Some("test"),
    )
    .await
    .expect("grade");

    let _ = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &AssertMedicationInput {
            term: "tenofovir",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_TENOFOVIR_SEQ,
                display: "tenofovir",
            }),
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

    let lines = cairn_node::safety::chart_safety(&c, patient)
        .await
        .expect("report");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].rung, "existence");
    assert!(
        lines[0].class.is_none(),
        "the class must not reach the report"
    );
    assert!(lines[0].severity.is_none());
    // ADR-0062 decision 8 control 3: never just the grade.
    assert_eq!(lines[0].grade, "sequestered");
    assert_eq!(lines[0].subject_kind, "patient");
}
