//! Issue #102 — the clock-drift admission ceiling on the two remote-apply doors.
//!
//! Both doors merge the local HLC forward past every accepted event with
//! `hlc_wall = GREATEST(local, remote)` (the A3 invariant). Without a ceiling, ONE
//! verified event from a trusted-but-broken or hostile peer carrying an absurd future
//! wall permanently ratchets this node's clock forward (GREATEST is monotone; real time
//! never catches up) — and on the node plane it also poisons `trust_peer`'s
//! `ORDER BY hlc_wall DESC`. `cairn_max_hlc_drift_ms()` (db/001, 24h) bounds it.
//!
//! The two doors use DIFFERENT mechanisms, forced by their pull loops:
//!   * NODE plane (`apply_remote_node_event`, db/007) REJECTS a too-future event — the
//!     value must never enter `node_event` (trust_peer orders on it); the node pull loop
//!     treats the bare RAISE (P0001) as a self-healing deny-all (skip-and-advance,
//!     re-offered on the next full sweep), so nothing wedges.
//!   * CLINICAL plane (`apply_remote_event`, db/020) CLAMPS the local-clock merge and
//!     admits the event unchanged — cairn-sync FREEZES its watermark on any refusal of a
//!     verifiable event, so rejecting here would let one insane event wedge clinical
//!     replication (availability over consistency). The event's original wall is preserved
//!     verbatim in `event_log` (principle 1); only the clock side-effect is bounded.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` (shared-DB + TRUNCATE pattern).
use cairn_event::{event_address, generate_key, sign, EventBody, Hlc, PairingBundle};
use cairn_node::{db, identity, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The ceiling the DB enforces (db/001). Kept in sync by `config_fn_reports_24h`.
const DRIFT_MS: i64 = 86_400_000; // 24h

/// This machine's wall clock in ms — the same instant `clock_timestamp()` reads in-DB
/// (both run on the test host), so drift math lines up within a small margin.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

// ---------------------------------------------------------------------------
// The shared config function.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn config_fn_reports_24h() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let ms: i64 = c
        .query_one("SELECT cairn_max_hlc_drift_ms()", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        ms, DRIFT_MS,
        "the DB ceiling must match the constant this suite reasons with"
    );
}

// ---------------------------------------------------------------------------
// Node plane (db/007): REJECT a too-future event; never ratchet the clock.
// ---------------------------------------------------------------------------

/// Build B's genesis (`node.enrolled`) with a chosen HLC wall, signed by B's own key.
/// Returns (signed_bytes, b_node_id_hex, pubkey_hex).
fn b_genesis(wall: i64) -> (Vec<u8>, String, String) {
    let (sk_b, kid_b) = generate_key().unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({"display_name": "B", "address": "127.0.0.1:7901"}),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed = sign(&body, &sk_b).unwrap().signed_bytes;
    let node_id = hex::encode(event_address(&signed));
    (signed, node_id, kid_b)
}

/// Provision A and record an active peer(B) with B's real node_id + pubkey, so B's genesis
/// would pass the admission trust check — isolating the drift guard as the only variable.
async fn provision_a_trusting_b(
    c: &Client,
    b_node_id: &str,
    b_pubkey: &str,
) -> (cairn_event::SigningKey, String) {
    c.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();
    // Reset the SHARED hlc_state singleton so the GREATEST merge these tests assert on is
    // deterministic. `provision` below ticks it back up to `now` (safely below the
    // within-ceiling future wall we admit), but a prior test can leave it ARBITRARILY high
    // — the clinical clamp test deliberately parks it at now+ceiling — and GREATEST would
    // then keep that stale high value instead of merging our admitted event forward. (This
    // non-hermeticity passes single-threaded locally but fails under the parallel workspace
    // run's ordering — caught by the CI gate, exactly what it is for.)
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .ok();
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(c, &sk_a, &kid_a, "A", "127.0.0.1:7900")
        .await
        .unwrap();
    let bundle = PairingBundle {
        node_id_hex: b_node_id.into(),
        pubkey_hex: b_pubkey.into(),
        address: "127.0.0.1:7901".into(),
        fingerprint: cairn_event::short_fingerprint(b_pubkey).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.into(),
        },
    };
    identity::author_peer(c, &sk_a, &kid_a, "A", &bundle, Some("peer"))
        .await
        .unwrap();
    (sk_a, kid_a)
}

async fn hlc_state(c: &Client) -> (i64, i32) {
    let r = c
        .query_one("SELECT hlc_wall, hlc_counter FROM hlc_state WHERE id", &[])
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

/// A verified genesis from a TRUSTED peer whose wall is absurdly far in our future is
/// refused (drift, not trust — B is trusted), and the local clock is NOT ratcheted.
#[tokio::test]
async fn node_plane_rejects_insane_future_and_does_not_ratchet_the_clock() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // ~10 years ahead — far beyond the 24h ceiling.
    let insane = now_ms() + 10 * 365 * 24 * 3_600_000;
    let (signed, b_node_id, b_pubkey) = b_genesis(insane);
    provision_a_trusting_b(&c, &b_node_id, &b_pubkey).await;

    let before = hlc_state(&c).await;
    let err = c
        .execute("SELECT apply_remote_node_event($1)", &[&signed])
        .await
        .unwrap_err();
    let m = db_msg(&err);
    assert!(
        m.contains("clock-drift ceiling"),
        "must be refused by the drift guard (B is trusted), got: {m}"
    );

    // The rejection RAISEd before the merge, and the whole statement rolled back — the
    // clock is exactly where it was, and the poison value never entered node_event.
    assert_eq!(
        hlc_state(&c).await,
        before,
        "a drift-refused event must not advance hlc_state"
    );
    let admitted: i64 = c
        .query_one("SELECT count(*) FROM node_event WHERE op = 'enroll' AND subject_node_id = decode($1,'hex')", &[&b_node_id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(admitted, 0, "the poison genesis must never be stored");
}

/// A genesis whose wall is modestly ahead (within the ceiling) is admitted and DOES
/// advance the local clock — the guard must not over-reject honest clock skew.
#[tokio::test]
async fn node_plane_admits_within_ceiling_and_merges_forward() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // 1h ahead — well inside the 24h ceiling.
    let sane_future = now_ms() + 3_600_000;
    let (signed, b_node_id, b_pubkey) = b_genesis(sane_future);
    provision_a_trusting_b(&c, &b_node_id, &b_pubkey).await;

    c.execute("SELECT apply_remote_node_event($1)", &[&signed])
        .await
        .expect("a within-ceiling future event must be admitted");

    let stored: i64 = c
        .query_one("SELECT hlc_wall FROM node_event WHERE op='enroll' AND subject_node_id = decode($1,'hex')", &[&b_node_id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored, sane_future,
        "the admitted genesis keeps its asserted wall"
    );
    // The clock merged forward to the admitted (future-but-sane) wall.
    assert_eq!(
        hlc_state(&c).await.0,
        sane_future,
        "the local clock must merge forward to an admitted event"
    );
}

// ---------------------------------------------------------------------------
// Clinical plane (db/020): CLAMP the clock; admit the event unchanged.
// ---------------------------------------------------------------------------

/// Truncate the clinical tables, reset the clock to 0, and enroll one agent signer.
async fn clinical_setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

fn note(kid: &str, patient: Uuid, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "arrived by sync"}),
        attachments: vec![],
        plaintext_twin: Some("Progress note: arrived by sync".into()),
    }
}

/// An insane-future CLINICAL event is ADMITTED (availability — no wedge), its original wall
/// preserved verbatim in event_log (principle 1), but the local clock is CLAMPED to
/// now + ceiling rather than ratcheted to the absurd value.
#[tokio::test]
async fn clinical_plane_admits_but_clamps_the_clock() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = clinical_setup(&c).await;

    let patient = Uuid::now_v7();
    let insane = now_ms() + 10 * 365 * 24 * 3_600_000; // ~10 years ahead
    let e = note(&kid, patient, insane);
    let signed = sign(&e, &sk).unwrap().signed_bytes;

    // Admitted, not refused (the clinical door never wedges sync on a future date).
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("a future-dated clinical event must be admitted, never wedge the puller");

    // The event's own asserted wall is preserved verbatim (never rewrite the claim).
    let stored: i64 = c
        .query_one(
            "SELECT hlc_wall FROM event_log WHERE event_id = $1::text::uuid",
            &[&e.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        stored, insane,
        "event_log must preserve the event's original asserted wall"
    );

    // But the local clock was CLAMPED to ~now + ceiling, not ratcheted to the absurd value.
    let (clock, _) = hlc_state(&c).await;
    assert!(
        clock < insane,
        "the clock must NOT ratchet to the insane wall (got {clock})"
    );
    let ceiling = now_ms() + DRIFT_MS;
    assert!(
        clock <= ceiling + 60_000 && clock >= ceiling - 60_000,
        "the clock must land at ~now+ceiling ({ceiling}±60s), got {clock}"
    );
}

// ---------------------------------------------------------------------------
// Local door (db/005 submit_event): REJECT a too-future event (issue #187).
// ---------------------------------------------------------------------------
//
// The local door is where a hostile-but-enrolled writer (the Spike-0002 / ADR-0030
// threat actor) authors NEW events. Unlike the clinical REMOTE door — which must
// clamp-and-admit because refusing a verifiable event wedges the puller's watermark —
// nothing has accepted a locally-submitted event yet, so rejection here cannot fork
// the fleet or wedge anything (the same argument db/007 uses for the node plane).
// Without this guard, one event with hlc_wall ≈ 2^62 wins every
// `ORDER BY hlc_wall DESC` overlay on every node forever (finding A1, issue #187).

/// An insane-future wall at the LOCAL door is refused with a legible drift message,
/// and the poison event never enters event_log.
#[tokio::test]
async fn local_door_rejects_insane_future_hlc_wall() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = clinical_setup(&c).await;

    let patient = Uuid::now_v7();
    let insane = now_ms() + 10 * 365 * 24 * 3_600_000; // ~10 years ahead
    let e = note(&kid, patient, insane);
    let signed = sign(&e, &sk).unwrap().signed_bytes;

    let err = c
        .execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect_err("an insane-future wall must be refused at the local door");
    let m = db_msg(&err);
    assert!(
        m.contains("clock-drift ceiling"),
        "the rejection must name the drift ceiling legibly, got: {m}"
    );

    let admitted: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&e.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(admitted, 0, "the poison event must never be stored");
}

/// A modestly-future wall (honest clock skew, within the ceiling) is still admitted at
/// the local door — the guard must not over-reject.
#[tokio::test]
async fn local_door_admits_within_ceiling_future_wall() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = clinical_setup(&c).await;

    let patient = Uuid::now_v7();
    let sane_future = now_ms() + 3_600_000; // 1h ahead, inside the 24h ceiling
    let e = note(&kid, patient, sane_future);
    let signed = sign(&e, &sk).unwrap().signed_bytes;
    c.execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect("a within-ceiling future wall must be admitted (honest skew)");

    let stored: i64 = c
        .query_one(
            "SELECT hlc_wall FROM event_log WHERE event_id = $1::text::uuid",
            &[&e.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, sane_future, "the admitted event keeps its asserted wall");
}

/// A within-ceiling clinical event merges the clock forward UNCLAMPED — the clamp must not
/// touch honest, modestly-future events.
#[tokio::test]
async fn clinical_plane_within_ceiling_is_not_clamped() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = clinical_setup(&c).await;

    let patient = Uuid::now_v7();
    let sane_future = now_ms() + 3_600_000; // 1h ahead, inside the ceiling
    let e = note(&kid, patient, sane_future);
    let signed = sign(&e, &sk).unwrap().signed_bytes;
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .unwrap();

    let (clock, _) = hlc_state(&c).await;
    assert_eq!(
        clock, sane_future,
        "an in-ceiling event must merge the clock forward unclamped"
    );
}
