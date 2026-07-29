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
/// unreachable at migration time is also one of the two legs (the other is
/// `cairn_replay_eligible`) holding up the guarantee that no projection apply fn ever sees a
/// deferred row, which is what lets db/018 and db/034 keep trusting `event_log.attester_key`.
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
    assert!(rows.is_empty(), "an un-attested suppress must NOT be promoted");

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
        "the carried token must now VERIFY and promote the event"
    );
}

/// The loader runs the pass on EVERY connect and reprojects what it promotes.
///
/// Pinned with a type that HAS a registered projection, so "promoted" is observable as a
/// projection row rather than only as a deleted marker — the marker alone would pass even if
/// the reprojection half were missing, which is the half that makes the event actually
/// visible in a chart.
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
