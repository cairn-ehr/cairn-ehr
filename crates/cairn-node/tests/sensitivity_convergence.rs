//! Two nodes, opposite arrival orders, same effective grade — GIVEN EQUAL CUSTODY.
//!
//! Max-by-rank over a set of standing assertions (db/048's `cairn_effective_sensitivity`,
//! ADR-0062) is a join-semilattice: commutative, associative, idempotent. That algebraic
//! fact is exactly what makes the §5.9 sensitivity stream safe under set-union sync, where
//! there is no global ordering and two peers can receive the same events in any order —
//! so this test builds three sensitivity assertions once, applies them to node A in one
//! order and to node B in the REVERSE order through the remote door, and checks both nodes
//! land on the identical, and CORRECT, effective grade.
//!
//! # Why "given equal custody" is in the test name, not just this comment
//!
//! db/048 §11 states the sharp edge plainly: the effective grade is NON-MONOTONE IN
//! CUSTODY. A node that cannot resolve an event's medication thread (no custody of the
//! projection rows that would resolve it) deliberately computes a MORE conservative
//! (higher, never lower) grade via a bounding rule, rather than silently under-protecting.
//! Two perfectly honest nodes holding DIFFERENT custody of the same chart may therefore
//! legitimately compute DIFFERENT effective grades — that is not a bug, it is the safety
//! margin working as designed.
//!
//! This test's fixture gives both nodes the SAME custody: the same registration, the same
//! target event, and the same three signed assertion events, so a real mismatch here can
//! only mean the max-over-assertions projection itself is order-dependent. Stated loosely
//! (without the custody qualifier), this test would either fail spuriously the day someone
//! reuses the shape with unequal custody, or — far worse — a future maintainer chasing that
//! spurious failure "fixes" it by deleting the §10b conservative bound, reopening exactly
//! the disclosure the bound exists to close. The qualifier in the name is what stops that.
//!
//! Needs CAIRN_TEST_PG2; without it the test self-skips and cargo still reports "ok" (see
//! the guard clause below) — the dispatch that added this test insists the run output be
//! checked for "0 filtered out" for exactly that reason.
mod common;
use cairn_event::sensitivity::{SENSITIVITY_EVENT_TYPE, SENSITIVITY_SCHEMA_VERSION};
use cairn_event::{sign, ClockGrade, EventBody, Hlc, SigningKey};
use common::{cs, submit_registration, submit_signed_with_id, EventSpec};
use serde_json::json;
use tokio_postgres::Client;
use uuid::Uuid;

/// Truncate the clinical + sensitivity-overlay tables on ONE connection and enroll the
/// GIVEN key id as an agent actor there.
///
/// Deliberately NOT `common::setup`: that helper mints a FRESH, throwaway key on every
/// call, but this test needs the identical key id enrolled on BOTH databases. Why: the
/// three sensitivity events under test are signed ONCE and their exact bytes are applied
/// to both nodes, but `apply_remote_event` still refuses an event whose signer is not an
/// enrolled, non-revoked LOCAL actor (db/020) — enrollment itself is node-local state that
/// never travels with the event on the wire in this trimmed-down, no-federation fixture.
///
/// NOTE ON WHAT "EQUAL CUSTODY" MEANS HERE, because it is easy to weaken by accident.
/// Byte-identical event sets are NOT what makes custody equal — ADR-0062 §9 means DEK
/// read-custody of sealed clinical bodies, the thing that decides whether the medication
/// projections populate at all and therefore whether `cairn_event_thread` can resolve.
/// Two nodes holding identical bytes but different DEKs have UNEQUAL custody and may
/// legitimately compute different grades (the effective grade is non-monotone in custody —
/// gaining custody can LOWER it as the conservative bound collapses to the true value).
/// Custody is equal in this fixture because the graded target is a plaintext `note.added`
/// with no thread at all, so neither node needs custody of anything to resolve it.
async fn setup_with_shared_key(c: &Client, kid: &str) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.sensitivity_assertion') IS NOT NULL THEN TRUNCATE sensitivity_assertion; END IF; \
           IF to_regclass('public.sensitivity_withdrawal') IS NOT NULL THEN TRUNCATE sensitivity_withdrawal; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
}

/// Build one sensitivity assertion as a PEER event — mirrors `sensitivity_ceremony.rs`'s
/// `peer_chart_wide_raise`, generalised over subject kind/id/grade because this suite needs
/// THREE such events rather than one. `node_origin: "peer"` and the plain `"recorded"`
/// contributor role are what route it through `apply_remote_event` rather than
/// `submit_event`: the local door's db/048 ceremony (rationale for a chart-wide raise) is a
/// LOCAL-authoring rule only (ADR-0060) — irrelevant here since every event in this test
/// goes through the remote door on both nodes.
fn peer_assertion(
    kid: &str,
    patient: Uuid,
    subject_kind: &str,
    subject_id: Uuid,
    grade: &str,
    wall: i64,
) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: json!([{"actor_id": kid, "role": "recorded"}]),
        payload: json!({
            "subject_kind": subject_kind,
            "subject_id": subject_id.to_string(),
            "grade": grade,
            "source": "human",
        }),
        attachments: vec![],
        plaintext_twin: Some(format!("test fixture: {subject_kind}/{grade}")),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// The (grade, subject_kind) `cairn_effective_sensitivity` reports for `event` on `c`.
async fn effective(c: &Client, event: Uuid) -> (String, String) {
    c.query_one(
        "SELECT grade, subject_kind FROM cairn_effective_sensitivity($1::text::uuid)",
        &[&event.to_string()],
    )
    .await
    .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
    .unwrap()
}

/// Author the target event ("note.added") this test queries — identically on both nodes
/// (same signer, same event id, same fields), so `cairn_effective_sensitivity` is asked
/// about the SAME event on each side even though the two calls are independent.
async fn submit_target_note(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, event_id: Uuid) {
    submit_signed_with_id(
        c,
        sk,
        kid,
        event_id,
        EventSpec {
            patient,
            event_type: "note.added",
            schema_version: "note.added/1",
            payload: json!({ "text": "routine note" }),
            plaintext_twin: Some("routine note".into()),
            wall: 2,
        },
    )
    .await
    .expect("target note accepted");
}

#[tokio::test]
async fn effective_grade_converges_given_equal_custody_regardless_of_arrival_order() {
    let (Some(base_a), Some(base_b)) = (cs(), std::env::var("CAIRN_TEST_PG2").ok()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    // Taken BEFORE either connection is opened (the sync_watermark.rs two-database
    // fixture shape) — one advisory lock, keyed off node A's connection string, serializes
    // this suite against every other DB-gated test binary sharing this database.
    let _guard = cairn_node::db::test_serial_guard(&base_a).await.unwrap();
    let a = cairn_node::db::connect_and_load_schema(&base_a)
        .await
        .unwrap();
    let b = cairn_node::db::connect_and_load_schema(&base_b)
        .await
        .unwrap();

    // ONE signer, enrolled under the SAME key id on both nodes (see setup_with_shared_key's
    // doc for why this differs from the usual common::setup call).
    let (sk, kid) = cairn_event::generate_key().unwrap();
    setup_with_shared_key(&a, &kid).await;
    setup_with_shared_key(&b, &kid).await;

    // A chart's first event must be its registration (db/005 step 8b) — done on BOTH nodes
    // with the same patient id, because "equal custody" covers the whole chart, not just
    // the three assertions under test.
    let p = Uuid::now_v7();
    submit_registration(&a, &sk, &kid, p, 0).await;
    submit_registration(&b, &sk, &kid, p, 0).await;

    // The event the 'event'-scoped assertion below names, authored identically on both
    // nodes so both query the same target.
    let target = Uuid::now_v7();
    submit_target_note(&a, &sk, &kid, p, target).await;
    submit_target_note(&b, &sk, &kid, p, target).await;

    // Three assertions of DIFFERENT subject kinds and grades:
    //   - "event"   scoped to `target` directly     -> rank 20 (restricted)
    //   - "patient" scoped chart-wide                -> rank 30 (sequestered) <- the winner
    //   - "episode" an unrecognised future kind       -> rank 10 (sensitive)
    // db/048 §11 reads an unrecognised subject_kind conservatively as chart-wide (same
    // mechanism `sensitivity_ladder.rs`'s `an_unknown_subject_kind_is_read_as_chart_wide...`
    // pins), so all three apply to `target` and the maximum among them is unambiguous.
    //
    // The winning ("patient") assertion is built SECOND and applied in the MIDDLE of both
    // orders below — never first, never last, in EITHER arrival order. A "first wins" or
    // "last wins" bug in the projection would therefore fail this test instead of passing
    // it by accident (a test that can only pass is not a test).
    let event_scoped = peer_assertion(&kid, p, "event", target, "restricted", 10);
    let patient_scoped = peer_assertion(&kid, p, "patient", p, "sequestered", 11);
    let unknown_kind = peer_assertion(&kid, p, "episode", Uuid::now_v7(), "sensitive", 12);

    // Signed ONCE — the exact same bytes are what "equal custody" means here: both nodes
    // end up holding the identical event SET, only the arrival ORDER differs below.
    let signed_event = sign(&event_scoped, &sk).unwrap();
    let signed_patient = sign(&patient_scoped, &sk).unwrap();
    let signed_unknown = sign(&unknown_kind, &sk).unwrap();

    // Node A: forward order. Node B: the EXACT REVERSE. Applied through apply_remote_event
    // — the remote door, lenient by design (ADR-0060) — exactly as
    // sensitivity_ceremony.rs's peer-event tests do.
    for signed in [&signed_event, &signed_patient, &signed_unknown] {
        a.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
            .await
            .expect("node A admits the assertion");
    }
    for signed in [&signed_unknown, &signed_patient, &signed_event] {
        b.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
            .await
            .expect("node B admits the assertion");
    }

    let got_a = effective(&a, target).await;
    let got_b = effective(&b, target).await;
    let expected = ("sequestered".to_string(), "patient".to_string());

    assert_eq!(
        got_a, got_b,
        "opposite arrival order changed the effective grade — max-over-assertions is \
         supposed to be a join-semilattice, so this must never happen GIVEN EQUAL CUSTODY \
         (see the module doc for why the qualifier matters)"
    );
    // Agreement alone is not enough: two nodes agreeing on the WRONG grade must still fail.
    assert_eq!(
        got_a, expected,
        "both nodes converged, but not on the TRUE maximum among the three assertions"
    );
}
