//! #615 — one `node_event_id`, one body, through the RESTORE door.
//!
//! # The asymmetry that IS the issue
//!
//! The clinical-plane sibling of this question is `restore_one_event_id_one_body.rs` case 2,
//! which has passed since DR slice 2d: a different body under the same `event_id` is refused as
//! a substitution and penned with the door's reason. The node plane had no such test because it
//! had no such guard. `db/009_node_supersede_and_restore.sql`'s two
//! `INSERT … ON CONFLICT (node_event_id) DO NOTHING` sites carried no comparison at all, so a
//! second, DIFFERENT event under an id already present was discarded **in silence** and the
//! restore exited 0.
//!
//! # Why that is a security defect and not a tidiness one
//!
//! The node plane IS the trust set. The restore door is self-trusting by design — any
//! validly-signed `node.enrolled` is admitted without a trust check, because a fresh node has no
//! trust set to check against — and db/009's own drift-ceiling comment already argues that "the
//! medium can contain OTHER signers' events and is attacker-appendable" (it is why that ceiling
//! exists at all). So the attack is cheap: append an event carrying the `event_id` of the
//! clinic's `peer.revoked`, positioned earlier in file order. The genuine revocation is dropped.
//! The node comes back **trusting a peer the clinic had revoked**, and the summary says
//! `restored N event(s)`.
//!
//! The benign variants — a UUID generator bug, a hand-merged medium — produce identical silence.
//!
//! # Why the COUNT could never have caught it
//!
//! `apply_medium` returns `Ok(events.len())` — the slice length, and its own doc says so: "the
//! number of events PROCESSED … not the number newly inserted". So `restored {applied} event(s)`
//! counts what was OFFERED, and `events.len() == counts.node` by construction. Comparing them
//! would prove nothing. The guard is the load-bearing half.
//!
//! # What the refusal does to the ceremony
//!
//! It aborts the whole restore: `apply_medium` propagates every door error with `?`. That is not
//! a new posture — this door already aborts on an unknown node event type, an over-ceiling
//! event, an HLC wall past the drift ceiling, and an author key resolving to no restored enroll.
//! A medium carrying two rival events under one id is a compromised or corrupt medium, and
//! restoring a node whose peer list was decided by whichever copy the medium ordered first —
//! which whoever can append to the medium controls — is a worse outcome than refusing and
//! telling the operator to find another copy.

use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::{db, identity};

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Mint a signed `node.enrolled` (no DB). The medium's own genesis restores first, which is how
/// a later event's author key resolves — the restore door's non-enroll branch requires it.
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: hex::encode(sk.verifying_key().to_bytes()),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Mint a signed peer event under a CALLER-CHOSEN `event_id`, so two rivals can share one.
///
/// Everything else about the two rivals differs — `event_type` and `peer_node_id_hex` — which is
/// what makes their content-addresses differ and the pair a substitution rather than a repeat.
fn synth_peer_with_id(
    sk: &SigningKey,
    name: &str,
    event_id: uuid::Uuid,
    event_type: &str,
    peer_hex: &str,
) -> Vec<u8> {
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: event_type.into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 2,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: hex::encode(sk.verifying_key().to_bytes()),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "peer_node_id_hex": peer_hex, "role": "peer" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Mint a signed `node.superseded` under a caller-chosen `event_id`.
///
/// Separate from [`synth_peer_with_id`] because the supersede arm reads `superseded_node_id_hex`
/// where the peer arm reads `peer_node_id_hex` — the same branch, a different decode.
fn synth_supersede_with_id(
    sk: &SigningKey,
    name: &str,
    event_id: uuid::Uuid,
    superseded_hex: &str,
) -> Vec<u8> {
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.superseded".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 3,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: hex::encode(sk.verifying_key().to_bytes()),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "superseded_node_id_hex": superseded_hex }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// A 32-byte node id as lowercase hex.
///
/// Derived at runtime rather than written as a literal (house rule 6a), and the discriminator is
/// called `lineage` rather than `seed`/`salt`/`nonce` (house rule 6b): nothing here is
/// cryptographic — it is an opaque subject identifier the door only decodes.
fn node_id_hex(lineage: u8) -> String {
    hex::encode(
        (0..32u8)
            .map(|i| i.wrapping_mul(11).wrapping_add(lineage))
            .collect::<Vec<u8>>(),
    )
}

/// Restore a node event, returning the door's message if it refused.
async fn restore(c: &tokio_postgres::Client, signed: &[u8]) -> Result<(), String> {
    c.execute("SELECT restore_node_event($1)", &[&signed])
        .await
        .map(|_| ())
        .map_err(|e| {
            e.as_db_error()
                .map(|d| d.message().to_string())
                .unwrap_or_else(|| e.to_string())
        })
}

/// THE ATTACK: a rival node event under an id the log already holds is refused, not discarded.
#[tokio::test]
async fn a_rival_node_event_under_one_id_is_refused_not_discarded() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.expect(
        "the fixture reset must succeed — swallowing it with .ok() is the shape that \
                 produced the #296 pollution lessons: a leftover local_node fences the restore \
                 door closed and every assertion below then fails for the wrong reason",
    );

    let (sk, _kid) = cairn_event::generate_key().unwrap();
    restore(&c, &synth_enroll(&sk, "Restored"))
        .await
        .expect("the medium's own genesis restores first, so its key resolves");

    // The clinic's genuine revocation of a peer it no longer trusts, and the attacker's event
    // reusing that id with different content. On a real medium the rival is positioned EARLIER
    // in file order so it lands first and the revocation becomes the loser; here the order is
    // immaterial, because the property under test is that NEITHER may vanish in silence.
    let contested = uuid::Uuid::now_v7();
    let genuine = synth_peer_with_id(&sk, "Restored", contested, "peer.revoked", &node_id_hex(1));
    let rival = synth_peer_with_id(&sk, "Restored", contested, "peer.added", &node_id_hex(2));

    restore(&c, &rival)
        .await
        .expect("the first event under a fresh id applies normally");

    let msg = restore(&c, &genuine).await.expect_err(
        "a SECOND, different node event under one event_id must be refused. Discarding it \
         silently is #615: the clinic's peer.revoked is dropped and the node comes back \
         trusting a revoked peer, at exit 0",
    );
    assert!(
        msg.contains("restore_node_event")
            && msg.contains("already exists with different content (substitution refused)"),
        "the restore door's refusal must name ITSELF and the reason, so an operator reading a \
         failed restore knows which door refused and that it was a substitution rather than, \
         say, a signature failure; got: {msg}"
    );
}

/// The guard must refuse a RIVAL, never a REPEAT.
///
/// `apply_medium`'s doc promises that re-applying the same medium is a no-op, and that promise is
/// load-bearing on the resume path: a restore interrupted halfway is restarted over the same
/// file. A guard that refused an identical re-offer would turn a resumable ceremony into an
/// unresumable one — on the disaster path, where there may be no second copy of the medium.
#[tokio::test]
async fn re_restoring_the_identical_medium_is_still_a_silent_no_op() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.expect(
        "the fixture reset must succeed — swallowing it with .ok() is the shape that \
                 produced the #296 pollution lessons: a leftover local_node fences the restore \
                 door closed and every assertion below then fails for the wrong reason",
    );

    let (sk, _kid) = cairn_event::generate_key().unwrap();
    let genesis = synth_enroll(&sk, "Restored");
    let peer = synth_peer_with_id(
        &sk,
        "Restored",
        uuid::Uuid::now_v7(),
        "peer.added",
        &node_id_hex(3),
    );
    // `node.superseded` takes the SAME else-branch as a peer event but reads a DIFFERENT payload
    // field, and the guard below it is new — so the idempotence claim has to cover it too, not
    // just the arm that happened to be convenient to build.
    let supersede = synth_supersede_with_id(&sk, "Restored", uuid::Uuid::now_v7(), &node_id_hex(4));

    for pass in 1..=2 {
        for ev in [&genesis, &peer, &supersede] {
            restore(&c, ev).await.unwrap_or_else(|e| {
                panic!("pass {pass}: re-applying the SAME medium must stay a no-op, not raise: {e}")
            });
        }
    }
}
