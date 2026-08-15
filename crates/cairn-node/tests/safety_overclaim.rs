//! #405 part 2 — a rung the chart's grade does not license is recorded, never refused.
//!
//! ADR-0060 forbids an advisory field cancelling a medication assert, and the door cannot
//! rewrite event_log.safety without making the column disagree with signed_bytes. So the
//! door records instead: the bypass becomes auditable at zero clinical cost.
mod common;
use cairn_event::sensitivity::SubjectKind;
use cairn_node::medication::{assert_medication, AssertMedicationInput, SubstanceCoding};
use cairn_node::sensitivity::assert_sensitivity;
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

// A moiety code fixed and uuid-shaped for the same reason safety_emission.rs's constants
// are: db/041 registers `drugref-moiety` with code_shape 'uuid', and the strict door
// refuses a non-uuid code. Not one of safety_emission.rs's own MOIETY_* constants — a
// distinct value so a row this test seeds can never collide with one another suite left
// behind in the shared `safety_class_map` table (deliberately not truncated between
// suites; see safety_emission.rs's `own_the_class_map` doc for why).
const MOIETY_UNGRADED_THREAD: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e30";

#[tokio::test]
async fn a_thread_scoped_grade_elsewhere_on_the_chart_does_not_false_flag_this_threads_precise_emission(
) {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, _sk_h, _kid_h) = medication_setup(&c).await;

    let patient = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, patient, 0).await;
    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', $1, 'rh-sensitizing', 'high') ON CONFLICT DO NOTHING",
        &[&MOIETY_UNGRADED_THREAD],
    )
    .await
    .expect("seed the deployment class map");

    // THIS IS THE REAL EMISSION PATH (assert_medication -> seal_sign_submit ->
    // apply_safety_rung), not the raw-safety bypass every other test in this file uses.
    // It is what let Critical #1 through review: all three prior tests drove the door
    // with a hand-built body, so nothing here ever called cairn_prospective_sensitivity
    // with the thread the daemon itself would have resolved.

    // Thread A: an unrelated medication on the same chart, later graded `sensitive` —
    // thread-scoped, the granularity ADR-0062 decision 8 tells deployments to reach for.
    let thread_a = assert_medication(
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
    .expect("assert thread A");
    assert_sensitivity(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        SubjectKind::Thread,
        thread_a,
        "sensitive",
        Some("test fixture: grade thread A only"),
    )
    .await
    .expect("grade thread A");

    // Thread B: UNGRADED, coded at assert time — #404's own guarantee is that this emits
    // `precise` (nothing on thread B licenses anything coarser). If the door's overclaim
    // check reads the wrong thread — or no thread at all — it will find thread A's
    // `sensitive` grade instead, license only `kind`, and flag this correct `precise`
    // emission as an overclaim it never made.
    let thread_b = assert_medication(
        &mut c,
        &sk,
        &kid,
        "n1",
        patient,
        &AssertMedicationInput {
            term: "the sensitive one",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_UNGRADED_THREAD,
                display: "the sensitive one",
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
    .expect("assert thread B, coded");

    // assert_medication returns the THREAD id it mints, not the event id — recover the
    // event through the projection row (safety_emission.rs's own `assert_event_of` idiom).
    let assert_ev: Uuid = {
        let r = c
            .query_one(
                "SELECT e.event_id::text FROM event_log e
                 JOIN medication_statement m ON m.content_address = e.content_address
                 WHERE m.medication_id = $1::text::uuid",
                &[&thread_b.to_string()],
            )
            .await
            .expect("thread B's assert event");
        r.get::<_, String>(0).parse().expect("event_id is a uuid")
    };

    // Positive control: emission genuinely produced `precise` — #404's own guarantee. If
    // this is not `precise`, the ledger assertion below proves nothing about the
    // overclaim check; it would just mean the fixture broke somewhere upstream.
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE event_id = $1::text::uuid",
            &[&assert_ev.to_string()],
        )
        .await
        .expect("the event must exist and carry a safety field")
        .get(0);
    assert_eq!(
        stored_rung, "precise",
        "thread B is ungraded, so a working daemon emits precise (#404) — if this is not \
         precise, the ledger assertion below cannot mean anything"
    );

    // THE POINT OF THIS TEST (2026-08-15 review, Important #2 — the test that would have
    // caught Critical #1). An earlier version of the door-side check passed
    // p_thread = NULL unconditionally, which made cairn_prospective_sensitivity coarsen
    // using thread A's `sensitive` grade — ANY thread-scoped grade anywhere on the chart,
    // not only a grade on the thread this event is actually on — computing a licensed
    // rung of `kind` for an event that has nothing to do with thread A, and flagging the
    // daemon's own correct, #404-guaranteed `precise` emission as an overclaim it never
    // made. The door must resolve the SAME thread emission did (payload.medication_id,
    // read from b_clear after the unseal) to avoid false-flagging ordinary,
    // correctly-licensed traffic.
    let ca = common::content_address_of(&c, assert_ev).await;
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
        "an ordinary, correctly-licensed emission on an UNGRADED thread must not be \
         flagged just because SOME OTHER thread on the same chart carries a grade"
    );
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

    // No flag was recorded either. NOTE what this does and does not prove (2026-08-15
    // review, Minor #4): BREAK_GRADE_LOOKUP raises unconditionally as its very FIRST
    // statement, before `cairn_prospective_sensitivity` would ever reach a RETURN — so
    // this assertion cannot distinguish "the nested block ran partway and was rolled
    // back" from "the block never got past the grade lookup at all". What it DOES prove,
    // and the only thing that matters here: after an outage, the ledger stays SILENT
    // rather than lying — no flag row exists for an event this check never finished
    // judging.
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
        "a lookup that could not run must not have recorded a flag either — the ledger \
         must stay silent on an outage, never guess"
    );
}
