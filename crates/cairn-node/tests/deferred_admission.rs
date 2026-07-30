//! ADR-0056 decisions 1 + 4 (issues #265/#266): the clinical remote door admits an event
//! whose `event_type` it cannot classify — stored verbatim, no projection rows, no power —
//! and power is granted only after the classification-gated floor checks are re-run.
//!
//! Before this slice the door RAISED on an unclassifiable type, so the event was never
//! stored at all. A phone-tier node carrying a chart between two upgraded facilities (the
//! §6.1 sneakernet path, the case Cairn exists for) acquired NOTHING past the first
//! unknown-type event — not unrendered, absent. These tests pin the contract that replaces
//! it: custody is total, power is deferred.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21) so the t_effective ≤ t_recorded
/// ceiling compares against a sane "recorded" instant rather than 1970.
const WALL_2026: i64 = 1_782_000_000_000;

/// A type no migration classifies — a plausible FUTURE slice's event, which is exactly the
/// case ADR-0056 is about: an upgraded peer authors it, this node has no code for it.
const UNKNOWN_TYPE: &str = "clinical.medication.recall";

/// Truncate the clinical tables and enroll one agent signer + one human attester.
/// `TRUNCATE event_log ... CASCADE` clears `event_deferred` through its FK.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag, \
         t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    // Several tests below CLASSIFY `UNKNOWN_TYPE` to simulate a code-plane update landing.
    // That row would otherwise persist in this shared database and silently break every
    // test that needs the type to be unknown. Cleaning up at the END of those tests would
    // not survive a panic, so de-classify HERE: idempotent, and it repairs the database
    // after a predecessor that died mid-test (the issue #296 test-pollution lesson).
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = $1",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    // Keys are DERIVED at runtime, never byte literals (house rule 6 / issue #146).
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build a signed event of an arbitrary type, "arriving from a peer".
fn peer_event(kid: &str, patient: Uuid, ty: &str, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: ty.into(),
        schema_version: "future/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "upgraded-peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"reason": "batch recall"}),
        attachments: vec![],
        // No authored twin: the mechanical skeleton must carry it (ADR-0039), which is the
        // rendering half of "coarseness varies; existence never disappears".
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention, see identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A projection may only be registered for a CLASSIFIED type.
///
/// This guard is load-bearing, not hygiene. The deferred marker row is written AFTER the
/// `event_log` INSERT, but the AFTER-INSERT projection dispatcher fires DURING it. So a type
/// registered for projection without an `event_type_class` row would be projected at
/// admission — granting exactly the power the marker exists to withhold. Making that state
/// unreachable at migration time is one of two things bounding which apply fns run against a
/// deferred row: the other is `cairn_replay_eligible` (no reprojection path can reach one
/// later). It is NOT "no projection apply fn ever sees a deferred row" — db/043's gate 4
/// deliberately runs a promoted event's heal-safe apply fns while its marker is still
/// present, before deleting it (PR #302 finding F1). db/018 and db/034 stay safe under that
/// not by trusting `event_log.attester_key`, but by explicitly excluding
/// `event_attestation_unvouched` (db/001) — keyed on the token's verification, not on
/// deferral.
#[tokio::test]
async fn projection_registration_requires_a_classified_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    // `patient_chart_apply` exists with the right signature and `patient_chart` is a real
    // relation, so the two pre-existing registration guards pass — only the classification
    // one can fire, which is what makes this test discriminate.
    let err = c
        .execute(
            "INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables) \
             VALUES ('unclassified.for.test', 'patient_chart_apply', ARRAY['patient_chart'])",
            &[],
        )
        .await
        .expect_err("registering a projection for an unclassified type must fail closed");
    assert!(
        db_msg(&err).contains("not classified in event_type_class"),
        "got: {}",
        db_msg(&err)
    );
}

/// The marker table is the EXPLICIT deferred state ADR-0056's corollary demands ("never
/// inferred from a null classification lookup falling through the gates by three-valued
/// logic"). Pin its shape so a later edit cannot quietly drop the column that records WHY a
/// re-adjudication failed — the whole of decision 4's "flagged legibly".
#[tokio::test]
async fn event_deferred_table_has_the_designed_shape() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    let cols: Vec<String> = c
        .query(
            "SELECT column_name::text FROM information_schema.columns \
             WHERE table_name = 'event_deferred' ORDER BY column_name",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(
        cols,
        vec![
            "adjudication_error",
            "admitted_at",
            "event_id",
            "event_type",
            "last_attempt_at"
        ],
        "event_deferred shape drifted from the design"
    );
}

/// ADR-0056 decision 1: an unclassifiable type is ADMITTED — stored verbatim, no projection
/// rows, no power — and marked deferred. This is the §6.1 sneakernet case: a carrier node
/// must stop being a propagation barrier.
#[tokio::test]
async fn unknown_type_is_admitted_and_marked_deferred() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .expect("an unclassifiable type must be ADMITTED, not refused (ADR-0056 decision 1)");

    // Stored verbatim — the whole point. Before this slice the row did not exist at all.
    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 1, "the event must be in event_log verbatim");

    // Marked deferred — EXPLICITLY, not inferred from the absent classification row.
    let marked: i64 = c
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        marked, 1,
        "an admitted-uninterpreted event must carry a marker"
    );

    // Legible without its schema: the skeleton twin renders any type, registered or not.
    let twin: String = c
        .query_one(
            "SELECT plaintext_twin FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !twin.trim().is_empty(),
        "the skeleton twin must render an unregistered type (ADR-0039 honest degradation)"
    );
}

/// ADR-0056 decision 2: the STRICT door keeps failing closed. A node may CARRY a type it has
/// no code for; it may never AUTHOR one. This is the regression pin for the slice's whole
/// risk — over-relaxing the floor while relaxing the door.
#[tokio::test]
async fn strict_door_still_refuses_an_unclassifiable_type() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();

    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes.to_vec()])
        .await
        .expect_err("submit_event must still refuse a type this node cannot classify");
    assert!(
        db_msg(&err).contains("fail closed") || db_msg(&err).contains("unknown event_type"),
        "got: {}",
        db_msg(&err)
    );
    let marked: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(marked, 0, "a refused local author must leave no marker");
}

/// A deferred event is invisible to replay. This is the ADR-0057 seam doing the job it was
/// built for: even a hand-run mid-upgrade `cairn_reproject` cannot grant power to an event
/// whose classification-gated checks have never been run.
#[tokio::test]
async fn reproject_does_not_touch_a_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!eligible, "a deferred event must never be replay-eligible");
}

/// THE SECURITY TEST of this slice (design doc §4.2).
///
/// A deferred event's attestation token is CARRIED, NOT VOUCHED — the door stored it without
/// verifying it, because the gate that verifies it is deferred with the interpretation. It
/// must therefore not widen the ADR-0043 owner-gate, whose own contract is "wrong direction
/// is over-refusal, never over-permission".
///
/// Scenario: a hostile peer ships an unknown-type event carrying a token naming some other
/// key. Before the fix, `cairn_suppression_author_ok` unioned that unverified key into the
/// target's human-author set, so its holder could suppress the event on the strength of a
/// token nothing had checked. The gate must compute exactly as if no token had travelled.
#[tokio::test]
async fn a_carried_token_does_not_widen_the_owner_gate() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_agent, kid_agent, sk_human, kid_human) = setup(&c).await;
    let p = Uuid::now_v7();

    // Signed by the HUMAN, so the target's human-author set is non-empty via the signer arm
    // and the gate is genuinely restrictive — not the vacuous "no human authors ⇒ anyone may
    // suppress" branch, which would make this test pass for the wrong reason.
    let b = peer_event(&kid_human, p, UNKNOWN_TYPE, WALL_2026);
    let target_id = b.event_id.clone();
    let signed = sign(&b, &sk_human).unwrap();
    // A token from a DIFFERENT key rides along. Nothing verifies it on the deferred path —
    // that is precisely the hazard.
    let token = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_agent,
        "attested",
        &sk_agent,
    )
    .unwrap();
    let agent_key = hex::decode(&kid_agent).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &token, &agent_key],
    )
    .await
    .expect("a deferred event carrying a token is still admitted");

    // Precondition: the token really was stored (otherwise this test proves nothing).
    let stored: Option<Vec<u8>> = c
        .query_one(
            "SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&target_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored.as_deref(),
        Some(agent_key.as_slice()),
        "precondition: the carried token must be stored, else the hazard is not reproduced"
    );

    // The carried token must NOT put its key inside the target's author set.
    let widened: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &agent_key],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !widened,
        "a CARRIED (unverified) token on a deferred target must not widen the ADR-0043 \
         owner-gate — the gate must compute as if no token had travelled"
    );

    // Sanity: the target's genuine human signer is still an author of it, so the fix
    // narrowed only the unverified arm and did not break the gate outright.
    let genuine: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &hex::decode(&kid_human).unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        genuine,
        "the target's real human signer must still count as its author"
    );
}

/// Classification arrival PROMOTES a deferred event that passes the deferred gates: the
/// marker is deleted and the event becomes replay-eligible. An additive type that targets
/// nobody satisfies all three gates trivially, which is the common upgrade case.
#[tokio::test]
async fn classification_promotes_a_passing_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();

    // The code-plane update lands: a migration would classify the type exactly like this.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT promoted_type, promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "exactly one type should have been promoted");
    let ty: String = rows[0].get(0);
    let n: i64 = rows[0].get(1);
    assert_eq!(ty, UNKNOWN_TYPE);
    assert_eq!(n, 1);

    let still: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(still, 0, "a promoted event's marker must be DELETED");

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(eligible, "a promoted event must become replay-eligible");
}

/// ADR-0056 decision 4: an event that FAILS re-adjudication stays powerless and is flagged
/// legibly — never silently promoted. Here the type turns out to be SUPPRESSING and the
/// event carries no attestation, so the deferred attestation gate refuses it. This is the
/// case that makes "no unattested suppression holds at every instant" true rather than
/// violated-then-repaired.
#[tokio::test]
async fn failed_readjudication_stays_powerless_and_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    // Admitted with NO attestation token — legal for a deferred event, since the gate that
    // would demand one is deferred with the interpretation.
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();

    // The code plane classifies it SUPPRESSING, so the attestation gate now applies.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'suppressing', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT promoted_type, promoted_count FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "an un-attested suppress must NOT be promoted"
    );

    let r = c
        .query_one(
            "SELECT count(*)::bigint, max(adjudication_error) FROM event_deferred",
            &[],
        )
        .await
        .unwrap();
    let kept: i64 = r.get(0);
    let err: Option<String> = r.get(1);
    assert_eq!(kept, 1, "a failing event keeps its marker — powerless");
    let err = err.expect("the failure reason must be recorded");
    assert!(
        err.contains("attestation"),
        "the flag must be legible; got: {err}"
    );

    let eligible: bool = c
        .query_one(
            "SELECT cairn_replay_eligible(el) FROM event_log el WHERE el.event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!eligible, "a failed event must stay replay-ineligible");
}

/// The travelling-token trap, pinned end to end: a token that rode in with a deferred event
/// is stored, and is what lets re-adjudication promote it later. If the door ever dropped it
/// again, this test fails — and admit-and-defer would have quietly become a slower
/// fail-closed, with the event admitted but permanently powerless.
#[tokio::test]
async fn a_travelling_token_survives_defer_then_promote() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    // Signed by the HUMAN attester, so cairn_responsibility_bound is satisfied by the same
    // key that signs the token — the ordinary attested-write shape.
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.contributors = serde_json::json!([{
        "actor_id": kid_h, "role": "authored",
        "responsibility": {"held_by": kid_h}
    }]);
    let signed = sign(&b, &sk_h).unwrap();
    let token = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_h,
        "attested",
        &sk_h,
    )
    .unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &token, &hkey],
    )
    .await
    .unwrap();

    let stored: Option<Vec<u8>> = c
        .query_one(
            "SELECT attestation FROM event_log WHERE event_type = $1",
            &[&UNKNOWN_TYPE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        stored.is_some(),
        "the travelling token must be STORED on the deferred path, or re-adjudication has \
         nothing to verify and the event can never gain power"
    );

    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    let rows = c
        .query(
            "SELECT promoted_type FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "the carried token must SURVIVE defer→promote. Note this type is additive and bears \
         no responsibility, so no gate demands the token and nothing verifies it here — the \
         unvouched marker is what keeps it from counting as a vouch (PR #302 finding F2)",
    );
}

/// The loader runs the pass on EVERY connect and reprojects what it promotes.
///
/// Pinned with a type that HAS a registered projection, so "promoted" is observable as a
/// projection row rather than only as a deleted marker — and this is a STRONGER proof than
/// that alone sounds: `SCHEMA_GENERATION` never changes across this test, so the second
/// connect below stamps `recorded == embedded` and the loader's generation-gated heal
/// (`cairn_reproject`, guarded by `if recorded != Some(embedded)` in db.rs) never runs. The
/// only thing left that could have written the `patient_chart` row asserted below is gate 4's
/// own apply-fn run inside `cairn_readjudicate_deferred` — making this the one black-box test
/// that proves gate 4 itself projects, not merely that some later heal cleans up after it.
#[tokio::test]
async fn connect_promotes_and_reprojects_a_deferred_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();

    // Simulate "this node does not yet have the code for `patient.created`" by removing BOTH
    // things the migration that introduces a type provides: its classification AND its
    // projection registration. Removing only the class row would be an unfaithful
    // simulation — it produces a registered-but-unclassified state that no real node can
    // reach (db/005 registers the projection after classifying, and class rows are never
    // deleted by any migration), and in it the AFTER-INSERT dispatcher would still fire
    // because it reads cairn_projection_apply, not event_type_class.
    //
    // Both rows are restored by the next connect's migration replay (db/005:18 and
    // db/005:997), so this is self-healing even if the test dies partway.
    c.execute(
        "DELETE FROM cairn_projection_apply WHERE event_type = 'patient.created'",
        &[],
    )
    .await
    .unwrap();
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = 'patient.created'",
        &[],
    )
    .await
    .unwrap();
    let mut b = peer_event(&kid, p, "patient.created", WALL_2026);
    b.payload = serde_json::json!({"name": "Deferred Then Promoted"});
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();

    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 1, "precondition: the event is deferred");
    let projected: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(projected, 0, "a deferred event must project nothing");

    // A fresh connect replays every migration (restoring the class row) and must then
    // re-adjudicate and reproject.
    drop(c);
    let c2 = db::connect_and_load_schema(&base).await.unwrap();

    let deferred: i64 = c2
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "connect must promote the now-classified event");
    let name: Option<String> = c2
        .query_opt(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .map(|r| r.get(0));
    assert_eq!(
        name.as_deref(),
        Some("Deferred Then Promoted"),
        "connect must REPROJECT what it promoted, not merely clear the marker"
    );
}

/// The listing query behind `cairn-node deferred`. Pinned here so a schema change that
/// breaks the operator surface fails a test rather than only failing in the field —
/// decision 4's "flagged legibly" is only legible if something actually reads the flag.
#[tokio::test]
async fn deferred_listing_query_returns_the_operator_columns() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT event_id::text, event_type, admitted_at::text, \
                    COALESCE(adjudication_error, '(not yet re-adjudicated)') \
               FROM event_deferred ORDER BY admitted_at",
            &[],
        )
        .await
        .expect("the deferred listing query must run");
    assert_eq!(rows.len(), 1);
    let ty: String = rows[0].get(1);
    let reason: String = rows[0].get(3);
    assert_eq!(ty, UNKNOWN_TYPE);
    assert_eq!(
        reason, "(not yet re-adjudicated)",
        "a never-adjudicated row must read as such, not as a blank refusal"
    );
}

/// The marker's LIFETIME is the whole point (design §3). `event_deferred` answers "has this
/// been adjudicated?"; this second marker answers "is the stored attester_key vouched?" — and
/// those two facts stop agreeing the moment promotion deletes the first. Three states:
/// carried-and-unverified, verified-at-promotion (cleared), promoted-but-never-gated (kept).
#[tokio::test]
async fn an_unvouched_marker_tracks_whether_a_token_was_ever_verified() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    // A token that WOULD verify, on an event that bears responsibility — so gate 1 runs.
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.contributors = serde_json::json!([{
        "actor_id": kid_h, "role": "authored",
        "responsibility": {"held_by": kid_h}
    }]);
    let signed = sign(&b, &sk_h).unwrap();
    let token = cairn_event::sign_attestation(
        &cairn_event::event_address(&signed.signed_bytes),
        &kid_h,
        "attested",
        &sk_h,
    )
    .unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &token, &hkey],
    )
    .await
    .unwrap();

    // State 1: carried, not vouched. The door stored a token it could not verify.
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 1,
        "a deferred event carrying a token must be marked unvouched — nothing verified it"
    );

    // State 2: gate 1 runs (the type bears responsibility) and the token verifies.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 0,
        "gate 1 verified the token, so the unvouched marker must be CLEARED"
    );
}

/// The state that produced the F2 hole: an additive event bearing NO responsibility. No gate
/// ever demands its token, so promotion must leave the unvouched marker STANDING — the marker
/// outliving `event_deferred` is exactly what the fix depends on.
#[tokio::test]
async fn a_never_gated_token_stays_unvouched_after_promotion() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let b = peer_event(&kid_a, p, UNKNOWN_TYPE, WALL_2026);
    let signed = sign(&b, &sk_a).unwrap();
    // Keys are DERIVED, never literals (house rule 6): a blob that could never verify.
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(7)).collect();
    let akey = hex::decode(&kid_a).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &akey],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();

    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "precondition: an additive event promotes");
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        unvouched, 1,
        "no gate demanded this token, so nothing verified it — the marker must OUTLIVE \
         event_deferred, which is the whole reason it is a separate table"
    );
}

/// THE F2 REGRESSION PIN (PR #302 review). The sibling test
/// `a_carried_token_does_not_widen_the_owner_gate` covers the still-DEFERRED target. This
/// covers the target after PROMOTION — where the original fix stopped working, because it
/// keyed on the event_deferred marker that promotion deletes.
///
/// Scenario, measured before the fix: a hostile peer ships an unknown-type event signed by an
/// honest human, carrying a GARBAGE attestation blob naming Mallory. The node admits it
/// deferred and stores the blob unverified. The type is later classified ('additive', FALSE) —
/// no gate demands a token, so nothing ever checks it — and promotion deletes the marker. The
/// owner-gate then unioned Mallory's key into the target's human-author set, and she could
/// suppress another clinician's event on the strength of a blob nothing had ever looked at.
#[tokio::test]
async fn a_carried_token_never_widens_the_owner_gate_after_promotion() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    // A SECOND enrolled human — Mallory. The pinned determinants must differ from the
    // setup() human's, or enroll_actor refuses the pair as one actor (issue #152).
    let (_sk_m, kid_m) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"mallory\"}', $1)",
        &[&kid_m],
    )
    .await
    .unwrap();

    let p = Uuid::now_v7();
    // Signed by the HONEST human, so the target's author set is non-empty via the signer arm
    // and the gate is genuinely restrictive — not the vacuous "no human authors => anyone may
    // suppress" branch, which would make this test pass for the wrong reason.
    let b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    let target_id = b.event_id.clone();
    let signed = sign(&b, &sk_h).unwrap();
    // Derived at runtime, never a literal (house rule 6): a blob that could never verify.
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(7)).collect();
    let mkey = hex::decode(&kid_m).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &mkey],
    )
    .await
    .expect("a deferred event carrying a token is still admitted");

    // The code plane arrives. 'additive' + no responsibility contributor = NO gate demands a
    // token, so promotion never verifies this one.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    c.execute("SELECT 1 FROM cairn_readjudicate_deferred()", &[])
        .await
        .unwrap();

    // Preconditions — without these the test proves nothing.
    let deferred: i64 = c
        .query_one("SELECT count(*) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(deferred, 0, "precondition: the event was PROMOTED");
    let stored: Option<Vec<u8>> = c
        .query_one(
            "SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&target_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored.as_deref(),
        Some(mkey.as_slice()),
        "precondition: the unverified key is still on the row (event_log is append-only, \
         so it can never be scrubbed) — the hazard is not reproduced without it"
    );

    let widened: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &mkey],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !widened,
        "a token NO gate ever demanded must not widen the ADR-0043 owner-gate after \
         promotion — Mallory never signed, authored, or attested anything"
    );

    // Sanity: the fix narrowed only the unvouched arm; the real signer still owns the event.
    let genuine: bool = c
        .query_one(
            "SELECT cairn_suppression_author_ok($1::text::uuid, $2)",
            &[&target_id, &hex::decode(&kid_h).unwrap()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        genuine,
        "the target's real human signer must still count as its author"
    );
}

/// PR #302 review finding F2, third reader — a WHITE-BOX test, and deliberately so.
///
/// `medication_attestation_apply` projects `encode(e.attester_key,'hex')` as `attester_kid`:
/// the responsible human, the thing the whole ADR-0049 sign-off surface reads. Its header
/// asserts that column is a verified vouch. That is true today only because a
/// `-attestation.asserted` event always bears responsibility, so db/043's gate 1 always runs
/// for it — a property of the EVENT TYPE, not of the column. This forces the state that
/// property currently rules out, so the guard is pinned before some future type reaches it.
#[tokio::test]
async fn an_unvouched_token_never_becomes_an_attester_kid() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_a, _kid_a, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();

    // A well-formed attested event, admitted through the normal (classified) door so its
    // token is genuinely verified and it projects a row.
    let med_id = Uuid::now_v7();
    // medication_attestation is append-only / NOT truncated between serialized tests (no FK
    // to event_log for setup()'s CASCADE to reach) — at least eight test binaries in this
    // crate write it, and test_serial_guard only serializes access, never orders it. Scope
    // both counts to THIS probe's own med_id rather than counting the whole table, or this
    // precondition is a race against every other suite (issue #296 pattern: failure far from
    // cause, only on some runs).
    let rows_before: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        rows_before, 0,
        "precondition: no row yet for this probe's medication_id"
    );

    // Drive the ordinary attestation flow via the deferred path so we own the row: defer,
    // classify, promote. UNKNOWN_TYPE stands in for any future type that reads attester_key.
    // EVERY NOT NULL column medication_attestation demands (medication_id, patient_id,
    // attester_kid, reviewed_commitment, reviewed_count) must be satisfiable, or the pre-fix
    // run fails on a constraint instead of on the defect — and a test that fails for the
    // wrong reason proves nothing. reviewed_commitment is `decode(..., 'hex')`, so the
    // payload carries hex; derived at runtime, never a literal (house rule 6).
    let commitment: String = (0u8..32)
        .map(|i| format!("{:02x}", i.wrapping_mul(5)))
        .collect();
    let mut b = peer_event(&kid_h, p, UNKNOWN_TYPE, WALL_2026);
    b.payload = serde_json::json!({
        "medication_id": med_id.to_string(),
        "reviewed_commitment": commitment,
        "reviewed_count": 1
    });
    let signed = sign(&b, &sk_h).unwrap();
    let bogus: Vec<u8> = (0u8..64).map(|i| i.wrapping_mul(13)).collect();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes.to_vec(), &bogus, &hkey],
    )
    .await
    .unwrap();

    // The row now holds an attester_key nothing verified, and says so.
    let unvouched: i64 = c
        .query_one("SELECT count(*) FROM event_attestation_unvouched", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(unvouched, 1, "precondition: the token is marked unvouched");

    // Call the apply fn directly on that row — the white-box part. It must decline to
    // project rather than mint an attester_kid from an unverified key.
    c.execute(
        "SELECT medication_attestation_apply(el) FROM event_log el WHERE el.event_type = $1",
        &[&UNKNOWN_TYPE],
    )
    .await
    .expect("the apply fn must DEGRADE (no row), never raise — a raise wedges the event");

    let rows_after: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_attestation WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        rows_after, 0,
        "an unvouched token must never become an attester_kid — that column IS the \
         responsible human on the ADR-0049 sign-off surface"
    );
}

/// PR #302 review finding F1, first half. db/020 step 8 — `cairn_event_twin`'s dispatch to the
/// type's `check_fn` and `twin_required_msg` — is skipped for a deferred event for exactly the
/// same reason the other three gates are: the type has no registry row. db/043 re-ran three
/// gates and not this one, so it was WAIVED rather than deferred.
///
/// Pinned with `clinical.medication.asserted`, which hard-requires an authored twin and has a
/// real `check_fn`. The event below would be refused by BOTH doors if the type were known.
#[tokio::test]
async fn promotion_refuses_an_event_its_type_floor_rejects() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();

    // Simulate "no code for this type yet" — all three rows the migration provides. Restored
    // by the next connect's replay, so this is self-healing even if the test dies partway.
    for sql in [
        "DELETE FROM cairn_projection_apply WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'clinical.medication.asserted'",
        "DELETE FROM event_type_class WHERE event_type = 'clinical.medication.asserted'",
    ] {
        c.execute(sql, &[]).await.unwrap();
    }

    let mut b = peer_event(&kid, p, "clinical.medication.asserted", WALL_2026);
    b.schema_version = "clinical.medication.asserted/1".into();
    // STRUCTURALLY VALID (a real medication_id/substance.term/info_source), so the ONE
    // thing gate 0 catches below is the missing twin — not an incidental field-shape
    // refusal from cairn_check_medication_assertion's own checks, which run first inside
    // the same cairn_event_twin dispatch and would otherwise fire before ever reaching the
    // twin requirement, proving the wrong thing.
    b.payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "substance": {"term": "amoxicillin"},
        "info_source": "patient report"
    });
    b.plaintext_twin = None; // the type hard-REQUIRES an authored twin
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .expect("an unclassifiable type is admitted uninterpreted");

    // The code plane lands — restore only the classification, so promotion is attempted while
    // the projection registration is still absent. That isolates gate 0 from gate 4.
    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ('clinical.medication.asserted', 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[],
    )
    .await
    .unwrap();
    c.batch_execute(include_str!("../../../db/031_medication.sql"))
        .await
        .unwrap(); // restores cairn_event_twin_check for the type

    let rows = c
        .query(
            "SELECT promoted_type FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "an event its own type's structural floor rejects must NOT be promoted"
    );

    let err: Option<String> = c
        .query_one("SELECT max(adjudication_error) FROM event_deferred", &[])
        .await
        .unwrap()
        .get(0);
    let err = err.expect("the refusal must be recorded");
    assert!(
        err.contains("twin") || err.contains("§3.13"),
        "the flag must name a CLINICAL reason, not a constraint violation; got: {err}"
    );
}

/// PR #302 review finding F1, the part that BRICKS THE NODE.
///
/// Measured before this work began: promotion deleted the marker for an event whose apply fn
/// then raised, and because `event_log` is append-only nothing could undo it. Three consecutive
/// `connect_and_load_schema` calls failed with `post-upgrade heal replay: db error` and
/// `node_schema.version` never advanced past 42. `cairn-node deferred` could not diagnose it —
/// it calls `connect_and_load_schema` itself.
///
/// WHITE-BOX BY NECESSITY, and the necessity is itself the finding: with gate 0 in place there
/// is no longer any reachable event that passes gates 0-3 and then fails a heal-safe apply fn
/// (every registered `check_fn` covers its projection's payload-derived NOT NULL columns, and
/// the three types without a `check_fn` have no such columns). Gate 4 therefore guards the
/// stricter apply fn written years from now. To pin it at all, the failure must be constructed:
/// a test-only event type with a deliberately-raising apply fn.
///
/// The construction is safe to leak. `cairn_test_wedge_apply` can only ever be invoked for
/// events of type `readjudicate.wedge.probe`, and only this test creates one — so even if the
/// cleanup below never runs (a panic), `cairn_reproject` finds zero eligible rows of that type
/// and the function never executes.
///
/// The invariant this pins: A PROMOTED EVENT IS ONE THAT HAS ALREADY PROJECTED CLEANLY.
#[tokio::test]
async fn a_promotion_that_cannot_project_never_promotes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    const PROBE: &str = "readjudicate.wedge.probe";

    // Idempotent pre-clean, not post-clean: cleanup at the END does not survive a panic, and a
    // predecessor that died mid-test would otherwise poison this one (the issue #296 lesson,
    // applied forward). Registration first — the FK-less registry rows must go before the fn.
    let pre_clean = format!(
        "DELETE FROM cairn_projection_apply WHERE event_type = '{PROBE}'; \
         DELETE FROM event_type_class      WHERE event_type = '{PROBE}'; \
         DROP FUNCTION IF EXISTS cairn_test_wedge_apply(event_log);"
    );
    c.batch_execute(&pre_clean).await.unwrap();

    // An apply fn that always refuses. This is the "stricter apply fn written in 2027" that
    // gate 4 exists for, stood up deliberately because no real one exists yet.
    c.batch_execute(
        "CREATE FUNCTION cairn_test_wedge_apply(e event_log) RETURNS void \
         LANGUAGE plpgsql AS $fn$ BEGIN \
           RAISE EXCEPTION 'deliberate test failure: this apply fn always refuses'; \
         END $fn$;",
    )
    .await
    .unwrap();

    // Admit the event while the type is UNCLASSIFIED — the only way to become deferred.
    let mut b = peer_event(&kid, p, PROBE, WALL_2026);
    b.schema_version = "readjudicate.wedge.probe/1".into();
    let probe_id = b.event_id.clone();
    let signed = sign(&b, &sk).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&signed.signed_bytes.to_vec()],
    )
    .await
    .expect("an unclassifiable type is admitted uninterpreted");

    // The code plane lands: classification FIRST (db/005's registry trigger refuses a
    // projection for an unclassified type), then the projection registration.
    c.batch_execute(&format!(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
           VALUES ('{PROBE}', 'additive', FALSE) ON CONFLICT DO NOTHING; \
         INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables) \
           VALUES ('{PROBE}', 'cairn_test_wedge_apply', ARRAY['patient_chart']);"
    ))
    .await
    .unwrap();

    // A code-plane update bumps the generation too, so the loader takes the FULL-heal branch —
    // the realistic path, and the one that wedged permanently.
    c.execute("UPDATE node_schema SET version = version - 1", &[])
        .await
        .unwrap();
    drop(c);

    // THE ASSERTION THAT MATTERS: the loader survives, repeatedly.
    for attempt in 1..=3 {
        db::connect_and_load_schema(&base)
            .await
            .unwrap_or_else(|e| panic!("connect attempt {attempt} must succeed, got: {e}"));
    }

    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    let kept: i64 = c2
        .query_one(
            "SELECT count(*) FROM event_deferred WHERE event_id = $1::text::uuid",
            &[&probe_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        kept, 1,
        "an event that cannot project must KEEP its marker — powerless, retryable, and above \
         all unable to take the loader down with it"
    );
    let err: Option<String> = c2
        .query_one(
            "SELECT adjudication_error FROM event_deferred WHERE event_id = $1::text::uuid",
            &[&probe_id],
        )
        .await
        .unwrap()
        .get(0);
    let err = err.expect("the refusal must be recorded, not silent");
    assert!(
        err.contains("deliberate test failure"),
        "the apply fn's own refusal must be what gets flagged; got: {err}"
    );
    let embedded = db::embedded_schema_version();
    let recorded: i32 = c2
        .query_one("SELECT version FROM node_schema", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        recorded, embedded,
        "the generation must ADVANCE — a stuck stamp means every future connect retries the \
         same doomed heal forever, which is precisely how the node bricked"
    );

    c2.batch_execute(&pre_clean).await.unwrap();
    c2.batch_execute("TRUNCATE event_log CASCADE")
        .await
        .unwrap();
}
