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
    // It is what let Critical #1 through review: the three PRE-EXISTING raw-safety tests
    // (two above, one below — file order, not execution order) all drove the door
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

/// The prospective-grade lookup, replaced by one that STALLS rather than raising.
///
/// A stall is a materially different failure from `BREAK_GRADE_LOOKUP`'s raise, and the
/// difference is the whole point of the test below: under a `statement_timeout` a stall
/// surfaces as SQLSTATE `57014` `query_canceled`, and PostgreSQL's `WHEN OTHERS` matches
/// every error type EXCEPT `query_canceled` and `assert_failure`. So the blanket handler
/// that absorbs a raise does NOT absorb a timeout — the one failure mode ADR-0063
/// decision 8's originating incident actually named.
const STALL_GRADE_LOOKUP: &str = r#"
CREATE OR REPLACE FUNCTION cairn_prospective_sensitivity(p_patient uuid, p_thread uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE plpgsql STABLE AS $stall$
BEGIN
    PERFORM pg_sleep(3);
    RETURN;
END;
$stall$;
"#;

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

/// A STATEMENT TIMEOUT inside the advisory block must not cancel the clinical write either.
///
/// The sibling test above stages a *raise* and proves `EXCEPTION WHEN OTHERS` absorbs it.
/// This one stages a *stall* under a `statement_timeout`, which is a different SQLSTATE with
/// different handler semantics: PostgreSQL matches `OTHERS` against every error type EXCEPT
/// `query_canceled` (57014) and `assert_failure`, so a blanket `WHEN OTHERS` lets a timeout
/// propagate and abort `submit_event` — refusing the medication assert.
///
/// That is not a hypothetical (#410 review finding C2). It needs only two ordinary
/// conditions to co-occur: a deployment that sets `statement_timeout` (routine hardening)
/// and a populated `safety_class_map` (the whole point of ADR-0063 — it ships empty, so the
/// block is dormant until then). ADR-0063 decision 8's originating incident is *literally*
/// this: "a missing grant or a statement timeout aborted the MEDICATION ASSERTION over a
/// safety class no clinician caused". The block written to prevent that incident reproduced
/// it, for the one trigger its own comment named first.
///
/// The system may fail to RECORD an order; it may never CANCEL one.
#[tokio::test]
async fn a_stalled_grade_lookup_under_a_statement_timeout_still_admits_the_medication() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // Same licensing setup as the sibling outage test: a graded chart, so the block is
    // genuinely reached rather than skipped for an unrelated reason.
    common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    c.batch_execute(STALL_GRADE_LOOKUP)
        .await
        .expect("stage the advisory stall");
    // Short enough to fire well inside the staged 3s stall, long enough that the steps
    // BEFORE 7a (signature verify, ceremony, projections) comfortably complete first — so
    // the cancel lands where this test intends it to, inside the advisory block.
    c.batch_execute("SET statement_timeout = '400ms'")
        .await
        .expect("arm the statement timeout");

    let result = common::submit_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await;

    // Disarmed and restored BEFORE the assertions, so a failure still leaves the database
    // and the session usable for whatever runs next.
    c.batch_execute("RESET statement_timeout")
        .await
        .expect("disarm the statement timeout");
    c.batch_execute(DB049)
        .await
        .expect("restore db/049 after the staged stall");

    let ca = result.expect(
        "a safety-overclaim lookup that TIMED OUT must not cancel the clinical write — \
         WHEN OTHERS does not match query_canceled, so the handler must name it (ADR-0063 \
         decision 8)",
    );

    // Positive control, same reasoning as the sibling test: prove the event genuinely
    // landed carrying the rung this test believes it does, so the zero-flag assertion
    // below cannot pass because nothing was ever submitted.
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE content_address = $1",
            &[&ca],
        )
        .await
        .expect("the event must exist and carry a safety field")
        .get(0);
    assert_eq!(stored_rung, "precise");

    // And the ledger stays silent rather than guessing — a timed-out judgement is not a
    // finding.
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
        "a lookup that timed out must not have recorded a flag — the ledger must stay \
         silent on a cancelled judgement, never guess"
    );
}

/// The comparison is `<`, and an OVER-COARSENED emission is not an overclaim.
///
/// `cairn_safety_rung_rank` orders finer -> coarser as precise(0) < kind(10) < existence(20),
/// and the flag fires on `rank(emitted) < rank(licensed)` — emitted FINER than licensed.
/// Swapping that `<` for `<>` leaves the whole suite green (#410 review finding I2), because
/// every existing fixture emits either exactly the licensed rung or a finer one. Nothing
/// covered the third direction.
///
/// It matters because over-coarsening is the SAFE default everywhere else on the emission
/// path — db/049's own ELSE arms both round toward "disclose less" on an unrecognised value.
/// Under `<>` every one of those conservative emissions is recorded as an overclaim, and the
/// ledger fills with accusations against nodes behaving exactly as designed. ADR-0063
/// decision 8 names that outcome directly: a ledger whose rows are mostly false accusations
/// is worse than no ledger.
#[tokio::test]
async fn an_over_coarsened_rung_is_not_an_overclaim() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // No grade at all, so rank 0 licenses `precise` — the FINEST rung. Anything the event
    // emits can therefore only be equal or COARSER, which is the direction under test.
    let ca = common::submit_medication_with_raw_safety(
        &c,
        &sk,
        &kid,
        &sk_h,
        &kid_h,
        p,
        20,
        serde_json::json!({"rung":"existence"}),
    )
    .await
    .expect("an over-coarsened rung is ordinary traffic and must be admitted");

    // Positive control: the event landed carrying the coarse rung this test believes it
    // does, so `n == 0` below cannot pass because the fixture silently failed.
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE content_address = $1",
            &[&ca],
        )
        .await
        .expect("the event must exist and carry a safety field")
        .get(0);
    assert_eq!(stored_rung, "existence");

    // Premise, pinned rather than assumed: the two rungs really are DIFFERENT, so a `<>`
    // comparison would genuinely fire here. Without this the test would still pass if
    // `existence` and the licensed rung happened to coincide.
    let (r_emitted, r_licensed): (i32, i32) = {
        let row = c
            .query_one(
                "SELECT cairn_safety_rung_rank('existence'), cairn_safety_rung_rank('precise')",
                &[],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    assert!(
        r_emitted > r_licensed,
        "premise: the emitted rung must be strictly COARSER than the licensed one, or this \
         test does not exercise the `<` vs `<>` difference at all"
    );

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
        "emitting COARSER than licensed is conservative, not an overclaim — a `<>` \
         comparison would flag every safe over-coarsening as an accusation (#410 finding I2)"
    );
}

/// The overclaim ledger is written at the LOCAL door and NOWHERE ELSE — pinned, not assumed.
///
/// ADR-0064 decision 7 makes this asymmetry deliberate: an overclaim is a statement about
/// what THIS node's clinician-facing door let through, so recording a peer's event would
/// accuse every honest node that is simply older, or graded differently, or holds custody
/// this node does not. db/049 and the ADR both warn — in capitals — that a reviewer will
/// read local-only as an oversight and tidy it into symmetry.
///
/// Nothing anywhere paired `safety_overclaim_flag` with `apply_remote_event` (#410 review
/// finding I3), so that tidy-up would have passed the entire suite while the ledger began
/// filling with false accusations on ordinary sync traffic — ADR-0063 decision 8's own
/// stated failure mode ("a ledger whose rows are mostly false accusations is worse than no
/// ledger"), arriving through the one door the ADR left undefended.
///
/// The sibling `a_precise_rung_on_a_sequestered_chart_is_admitted_and_flagged` lands the
/// SAME overclaim through the LOCAL door and asserts a row IS written. Read the two
/// together: identical bytes, identical chart, different door, opposite expectation.
#[tokio::test]
async fn the_remote_door_admits_an_overclaim_and_records_no_flag() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // The same `sequestered` chart the local-door twin uses, so the emitted `precise` rung
    // is a genuine overclaim by this node's own reckoning — the flag's ABSENCE below is
    // therefore about the DOOR, not about the event being innocuous.
    common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    let ca = common::apply_remote_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await
    .expect("ADMITTED — apply never refuses on an advisory field (ADR-0060/ADR-0064)");

    // Positive control: the peer's event genuinely landed carrying the overclaiming rung.
    // Without this, `n == 0` would also hold if apply_remote_event had quietly refused it.
    let stored_rung: String = c
        .query_one(
            "SELECT safety ->> 'rung' FROM event_log WHERE content_address = $1",
            &[&ca],
        )
        .await
        .expect("the peer's event must exist and carry a safety field")
        .get(0);
    assert_eq!(stored_rung, "precise");

    // Premise, pinned rather than assumed: this node really would call it an overclaim.
    // If the chart's grade ever stopped licensing only `existence`, the assertion below
    // would pass for the wrong reason — because there was no overclaim to record at all.
    let licensed: String = c
        .query_one(
            "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank(g.grade))
               FROM cairn_prospective_sensitivity($1::text::uuid, NULL) g",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        licensed, "existence",
        "premise: the chart must license only `existence`, so `precise` IS an overclaim \
         this node would record had it come through the local door"
    );

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
        "LOCAL DOOR ONLY (ADR-0064 decision 7): a peer's event must never be flagged — \
         symmetry here turns the ledger into an accusation machine against honest nodes"
    );
}
