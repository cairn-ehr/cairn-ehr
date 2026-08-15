//! #405 part 2 — a rung the chart's grade does not license is recorded, never refused.
//!
//! ADR-0060 forbids an advisory field cancelling a medication assert, and the door cannot
//! rewrite event_log.safety without making the column disagree with signed_bytes. So the
//! door records instead: the bypass becomes auditable at zero clinical cost.
mod common;
use common::{cs, medication_setup};
use uuid::Uuid;

#[tokio::test]
async fn a_precise_rung_on_a_sequestered_chart_is_admitted_and_flagged() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    // A hostile client bypassing apply_safety_rung: it signs a body whose clear safety
    // field claims `precise` on a chart this node grades `sequestered` (licensed:
    // existence). Spike-0002's C1-C5 threat model, treated here as live.
    let ca = common::submit_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await
    .expect("ADMITTED — an advisory field may never cancel a clinical write (ADR-0060)");

    let (emitted, licensed): (String, String) = {
        let r = c
            .query_one(
                "SELECT emitted_rung, licensed_rung FROM safety_overclaim_flag
                  WHERE content_address = $1",
                &[&ca],
            )
            .await
            .expect("the overclaim must be recorded");
        (r.get(0), r.get(1))
    };
    assert_eq!(
        (emitted.as_str(), licensed.as_str()),
        ("precise", "existence")
    );

    // The read model still coarsens — the flag bounds the SILENCE, not the effect.
    let rung: String = c
        .query_one(
            "SELECT rung FROM cairn_event_safety(
                (SELECT event_id FROM event_log WHERE content_address = $1))",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rung, "existence");
}

#[tokio::test]
async fn a_licensed_rung_is_not_flagged() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // No grade at all: `precise` is exactly what rank 0 licenses.
    let ca = common::submit_medication_with_raw_safety(
        &c,
        &sk,
        &kid,
        &sk_h,
        &kid_h,
        p,
        20,
        serde_json::json!({"rung":"precise","class":"rh-sensitizing","severity":"high"}),
    )
    .await
    .unwrap();

    // Positive control: the event landed and carries the rung this test believes it
    // does. Without this, `n == 0` below would also hold if the fixture failed silently
    // (the medication event never landing, or the safety field never being written) —
    // exactly the "assertion with no failure mode" defect this slice keeps re-producing.
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE content_address = $1",
            &[&ca],
        )
        .await
        .expect("the event must exist and carry a safety field")
        .get(0);
    assert_eq!(stored_rung, "precise");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM safety_overclaim_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the ordinary path must produce no noise");
}

// ---------------------------------------------------------------------------
// ADR-0063 decision 8, stated categorically: the overclaim block must never be able to
// fail a clinical write. The ADR records a real incident that this repeats otherwise — an
// earlier safety lookup propagated its error with `?`, so a missing grant or a statement
// timeout aborted the MEDICATION ASSERTION over a safety class no clinician caused.
//
// The failure this test injects: `cairn_prospective_sensitivity` — the one lookup inside
// db/005's new block that reads the sensitivity stream (via `cairn_sensitivity_standing`)
// and so is the plausible site of a missing grant / lock / statement timeout in
// production — is replaced, IN THIS DATABASE, with a same-signature function that
// unconditionally raises.
// Modelled on safety_emission.rs's BREAK_GRADE_LOOKUP staging, which proves the identical
// property for `apply_safety_rung`'s own call to the same function.
// ---------------------------------------------------------------------------

/// db/049 exactly as this build embeds it — replayed to PUT BACK the function the test
/// below deliberately replaces with one that raises. Restoring from the migration file
/// itself (rather than a hand-copied definition) keeps the restore from drifting away from
/// the thing it restores.
const DB049: &str = include_str!("../../../db/049_safety_projection.sql");

/// The prospective-grade lookup, replaced by one that raises. Argument NAMES must match
/// db/049's or `CREATE OR REPLACE` refuses ("cannot change name of input parameter").
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
async fn a_failing_grade_lookup_still_admits_the_medication_and_records_no_flag() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // Sequestered — a working lookup WOULD flag this (proven by
    // `a_precise_rung_on_a_sequestered_chart_is_admitted_and_flagged` above), so a missing
    // flag below is evidence of the outage, not a coincidence of an unlicensing chart.
    common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    c.batch_execute(BREAK_GRADE_LOOKUP)
        .await
        .expect("stage the advisory outage");

    let result = common::submit_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await;

    // Restored BEFORE the assertions, so a failing assertion still leaves the database
    // usable for whatever runs next.
    c.batch_execute(DB049)
        .await
        .expect("restore db/049 after the staged outage");

    let ca = result.expect(
        "a safety-overclaim lookup that cannot run must not cancel the clinical write — \
         the system may fail to record an order, but it may never cancel one (ADR-0063 \
         decision 8)",
    );

    // Positive control: the medication event genuinely landed and carries the rung this
    // test thinks it does — without this, a query returning zero rows below could mean
    // either "the check swallowed the outage" (what we want) or "nothing was ever
    // submitted" (a silently broken fixture).
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE content_address = $1",
            &[&ca],
        )
        .await
        .expect("the event must exist and carry a safety field")
        .get(0);
    assert_eq!(stored_rung, "precise");

    // And the outage genuinely prevented the flag from being recorded — the exception
    // handler discarded the whole nested block, including the INSERT, not just the raise.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM safety_overclaim_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 0,
        "a lookup that could not run must not have recorded a flag either — the whole \
         block is swallowed, not just the part that failed"
    );
}
