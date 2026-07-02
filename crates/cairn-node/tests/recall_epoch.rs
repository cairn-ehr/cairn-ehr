//! Issue #99 (comprehensive review A10): the contamination-cascade recall key.
//!
//! `events_by_actor_epoch` used to resolve (key, epoch) against `actor_current`,
//! so the moment a supersede/re-enroll bumped an agent's skill_epoch, a recall of
//! the OLD epoch silently returned nothing — a production ADR-0011/0029/0030
//! contamination cascade would under-select. These tests pin the fixed semantics:
//!
//!   * resolution is against the FULL registry history (`actor_event`), so a
//!     superseded epoch's events stay selectable forever;
//!   * each admitted event carries a node-local `actor_id` attribution stamp
//!     (unique resolution at the door) so selection is EXACT per epoch;
//!   * when attribution is ambiguous (one key concurrently registered to several
//!     actors) the stamp is NULL — honestly unknown, principle 4 — and the recall
//!     query includes such events for EVERY epoch the key registered: a recall
//!     must over-select, never silently miss;
//!   * `recall_event` refuses a target that is not in the log (a fat-fingered
//!     UUID must fail loud, not "succeed" recalling nothing).
//!
//! Real Postgres, gated on $CAIRN_TEST_PG, serialized cluster-wide via
//! db::test_serial_guard (shared-DB + TRUNCATE pattern, identical to attestation.rs).
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the write tables (CASCADE covers recall_overlay via its FK).
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

/// Enroll `kid` as an agent pinned to `epoch`; returns the minted actor_id.
async fn enroll_epoch(c: &Client, kid: &str, epoch: &str) -> Vec<u8> {
    let pinned = format!(
        "{{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"{epoch}\"}}"
    );
    c.query_one(
        &format!("SELECT enroll_actor('agent', '{pinned}', $1)"),
        &[&kid],
    )
    .await
    .unwrap()
    .get(0)
}

/// Revoke an actor_id (owner ceremony — the registry has no runtime revoke door yet).
async fn revoke(c: &Client, actor_id: &[u8]) {
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&actor_id],
    )
    .await
    .unwrap();
}

/// A minimal additive note authored by `kid` (no attestation machinery involved).
fn note(patient: Uuid, kid: &str, wall: i64) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "advisory/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "agent".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "triaged"}]),
        payload: serde_json::json!({ "text": "seen, stable" }),
        attachments: vec![],
        plaintext_twin: None,
    }
}

async fn submit(c: &Client, b: &EventBody, sk: &SigningKey) {
    let signed = sign(b, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

/// (event_id, attribution) rows for one (key, epoch) recall query.
async fn epoch_events(c: &Client, kid: &str, epoch: &str) -> Vec<(String, String)> {
    c.query(
        "SELECT event_id::text, attribution FROM events_by_actor_epoch($1, $2) ORDER BY event_id",
        &[&kid, &epoch],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

/// The regression at the heart of issue #99: after an epoch bump (revoke + re-enroll
/// of the SAME key under a new skill_epoch), a recall of the OLD epoch must still
/// find its events — exactly those, and none of the new epoch's.
#[tokio::test]
async fn superseded_epoch_events_remain_selectable() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (sk, kid) = generate_key().unwrap();
    let patient = Uuid::now_v7();

    // Epoch A: enroll, author one event while A is the key's unique identity.
    let actor_a = enroll_epoch(&c, &kid, "epoch-a").await;
    let e1 = note(patient, &kid, 1);
    submit(&c, &e1, &sk).await;

    // Epoch bump: revoke A, re-enroll the same key under epoch B, author another.
    revoke(&c, &actor_a).await;
    let _actor_b = enroll_epoch(&c, &kid, "epoch-b").await;
    let e2 = note(patient, &kid, 2);
    submit(&c, &e2, &sk).await;

    // The old epoch's recall set: exactly e1, exactly attributed.
    let a = epoch_events(&c, &kid, "epoch-a").await;
    assert_eq!(
        a,
        vec![(e1.event_id.clone(), "pinned".to_string())],
        "a superseded epoch must keep selecting exactly its own events (issue #99)"
    );

    // The new epoch's recall set: exactly e2.
    let b = epoch_events(&c, &kid, "epoch-b").await;
    assert_eq!(b, vec![(e2.event_id.clone(), "pinned".to_string())]);

    // A (key, epoch) pair never registered selects nothing.
    let z = epoch_events(&c, &kid, "epoch-z").await;
    assert!(z.is_empty(), "an unregistered epoch must select nothing, got {z:?}");
}

/// Ambiguous attribution (one key concurrently current for two actors) degrades
/// honestly: the stamp is NULL and the event is over-selected into BOTH epochs'
/// recall sets, flagged 'unattributed' — never silently missing from either.
#[tokio::test]
async fn ambiguous_attribution_over_selects_never_buries() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (sk, kid) = generate_key().unwrap();
    let patient = Uuid::now_v7();

    // Both epochs current for the same key (no revoke): resolution is ambiguous.
    enroll_epoch(&c, &kid, "epoch-a").await;
    enroll_epoch(&c, &kid, "epoch-b").await;
    let e1 = note(patient, &kid, 1);
    submit(&c, &e1, &sk).await;

    let expected = vec![(e1.event_id.clone(), "unattributed".to_string())];
    assert_eq!(epoch_events(&c, &kid, "epoch-a").await, expected,
        "ambiguous events must appear in every registered epoch's recall set");
    assert_eq!(epoch_events(&c, &kid, "epoch-b").await, expected);
}

/// The sync apply door stamps attribution identically to the authoring door
/// (one floor, two doors — issue #91 discipline applied to the recall key).
#[tokio::test]
async fn apply_remote_event_stamps_attribution_too() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (sk, kid) = generate_key().unwrap();
    let patient = Uuid::now_v7();

    let actor_a = enroll_epoch(&c, &kid, "epoch-a").await;
    let e1 = note(patient, &kid, 1);
    let signed = sign(&e1, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    // Bump the epoch, then recall the old one: the replicated event is found.
    revoke(&c, &actor_a).await;
    enroll_epoch(&c, &kid, "epoch-b").await;
    assert_eq!(
        epoch_events(&c, &kid, "epoch-a").await,
        vec![(e1.event_id.clone(), "pinned".to_string())],
        "the apply door must stamp the same attribution the authoring door does"
    );
}

/// Registry lag at the replication door: an event authored under the OLD epoch but
/// arriving AFTER the local registry bumped the key must NOT be stamped with the
/// merely-current (new) actor — that would misattribute it and a recall of the old
/// epoch would silently miss it (the #99 failure reborn). The apply door resolves
/// the stamp against the key's ENTIRE local history: once the key has meant more
/// than one actor here, the stamp is NULL and the event lands in BOTH epochs'
/// recall sets, flagged 'unattributed'.
#[tokio::test]
async fn late_arriving_remote_event_is_never_misattributed() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let (sk, kid) = generate_key().unwrap();
    let patient = Uuid::now_v7();

    // The event is signed while (conceptually, on its origin node) epoch A ruled…
    let e1 = note(patient, &kid, 1);
    let signed = sign(&e1, &sk).unwrap();

    // …but the LOCAL registry has already bumped the key to epoch B when it arrives.
    let actor_a = enroll_epoch(&c, &kid, "epoch-a").await;
    revoke(&c, &actor_a).await;
    enroll_epoch(&c, &kid, "epoch-b").await;
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    // Both epochs must see it, honestly unattributed — a recall of epoch A (where it
    // truly belongs) must not miss it, and epoch B over-selects rather than claims.
    let expected = vec![(e1.event_id.clone(), "unattributed".to_string())];
    assert_eq!(epoch_events(&c, &kid, "epoch-a").await, expected,
        "a late-arriving old-epoch event must never vanish from the old epoch's recall set");
    assert_eq!(epoch_events(&c, &kid, "epoch-b").await, expected);
}

/// A recall naming an event that is not in the log must fail loud (FK), not
/// "succeed" while recalling nothing.
#[tokio::test]
async fn recall_of_unknown_target_fails_loud() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;

    let bogus = Uuid::now_v7().to_string();
    let r = c
        .execute("SELECT recall_event($1::text::uuid, 'fat-fingered target')", &[&bogus])
        .await;
    assert!(r.is_err(), "recalling a nonexistent event must be refused");

    // And a recall of a REAL event still works (the FK does not over-block).
    let (sk, kid) = generate_key().unwrap();
    enroll_epoch(&c, &kid, "epoch-a").await;
    let e1 = note(Uuid::now_v7(), &kid, 1);
    submit(&c, &e1, &sk).await;
    c.execute("SELECT recall_event($1::text::uuid, 'skill-epoch contamination')", &[&e1.event_id])
        .await
        .unwrap();
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM recall_overlay WHERE target_event_id = $1::text::uuid",
            &[&e1.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the legitimate recall lands in the overlay");
}
