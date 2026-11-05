//! #948 — a peer's addendum that reaches this node BEFORE the note it adds to.
//!
//! WHY THIS FILE EXISTS. An addendum is a `clinical.note.asserted` whose `refs` hold
//! `{rel: "addendum-to", event: <the note>}`. Sync is set-union, not causally ordered, so a peer
//! can deliver the addendum A ahead of its note N. Issue #948 asked what should happen then; the
//! maintainer ruled (2026-11-05) that it stays as it is: the remote door (`apply_remote_event`,
//! db/020) refuses A through db/064 rule 6's bare `RAISE` ("… is not held on this node (a dangling
//! reference)"), whose SQLSTATE is `P0001`. That one SQLSTATE is what `cairn-sync`'s
//! `is_deliberate_refusal` reads as "a verdict about these bytes": the puller pens A, re-offers it
//! every cycle, and A is admitted once N has landed — delayed, never lost.
//!
//! WHAT IT PROVES (the door-side half of that ruling), through the real remote door:
//!   1. A applied first is refused with SQLSTATE exactly `P0001` and the dangling-reference
//!      message — not some earlier refusal (unknown signer, chart precedence) that would happen
//!      to share the code;
//!   2. once N is applied, A's SAME signed bytes are admitted, and both project into
//!      `clinical_note`, A still naming N.
//!
//! WHAT IT DOES NOT PROVE: the puller half — that a `P0001` refusal is penned, re-offered and
//! released. That lives in `crates/cairn-sync/tests/clinical_pull.rs::
//! refused_apply_pens_the_bytes_and_recovers_without_loss` (there the refusal is an un-enrolled
//! signer, the same `P0001` class this test pins). Nor does it prove anything of a SEALED
//! addendum. One that arrives WITHOUT its DEK (a node that is not a custody holder, or a
//! byte-lazy pull) is admitted uninterpreted by db/020 step 7: the floor cannot see its `refs`,
//! so rule 6 never runs and it is never penned. It is not projected until its body can be read.
//! The sealed cases, with and without the DEK, are untested: issue #995 (ADR-0091).
//!
//! UNSEALED BODIES, deliberately. A real note is born sealed (ADR-0052), but only the STRICT door
//! enforces that; the remote door's lenient arm admits an unsealed `clinical.*` body and runs the
//! full floor on it (db/020 step 7), which is what reaches rule 6. A sealed A would need its DEK
//! sidecar for the floor to see its `refs` at all, and the sealing path (`prepare_sealed`) is
//! crate-private; the unsealed body exercises the same rule with no custody plumbing.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_event::note::{EscapedNarrative, NoteAssertion, StatedTime};
use cairn_event::{sign, Hlc};
use cairn_node::db;
use cairn_node::note::sign::note_body;
use common::{db_msg, enroll_human_with_role, note_setup};
use tokio_postgres::error::SqlState;
use tokio_postgres::Client;
use uuid::Uuid;

/// Apply signed bytes through the 1-argument remote door (no attestation, no DEK), as the
/// puller does for an unsealed event.
async fn apply(c: &Client, signed: &[u8]) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&signed]).await
}

/// Sign a peer's note on `chart`, optionally `addendum-to` `target`. Returns the note's event id
/// and its signed bytes (kept so the SAME bytes can be offered twice, as the puller re-offers).
fn peer_note(
    sk: &cairn_event::SigningKey,
    kid: &str,
    chart: Uuid,
    wall: i64,
    target: Option<Uuid>,
) -> (Uuid, Vec<u8>) {
    let target = target.map(|t| t.to_string());
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed("chest clear"),
        stated: StatedTime::NotStated,
        recording_started: "2026-11-01T01:00:00Z",
        recording_ended: "2026-11-01T01:05:00Z",
        addendum_to: target.as_deref(),
    };
    let hlc = Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    };
    let id = Uuid::now_v7();
    let body = note_body(id, chart, kid, hlc, &a);
    (id, sign(&body, sk).unwrap().signed_bytes)
}

#[tokio::test]
async fn an_addendum_ahead_of_its_note_is_penned_then_admitted() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // A registered chart, as a real one would be. (db/005 step 8b's precedence rule runs at the
    // strict door only, so the remote door would not insist; the message assertion below is
    // what proves the refusal is rule 6's and no other.)
    let (_, _, _, _, chart) = note_setup(&c).await;
    // The peer's clinician, enrolled here by the owner ceremony (actors do not travel, #205).
    let (peer_sk, peer_kid) = enroll_human_with_role(&c, "peer-clinician").await;
    let wall = chrono::Utc::now().timestamp_millis();
    let (n_id, n_bytes) = peer_note(&peer_sk, &peer_kid, chart, wall, None);
    let (a_id, a_bytes) = peer_note(&peer_sk, &peer_kid, chart, wall + 1, Some(n_id));

    // 1. A arrives first: refused as a deliberate verdict, for the dangling reference itself.
    let e = apply(&c, &a_bytes)
        .await
        .expect_err("an addendum ahead of its note is refused");
    assert_eq!(
        e.code(),
        Some(&SqlState::RAISE_EXCEPTION),
        "P0001 is the class cairn-sync pens and re-offers: {}",
        db_msg(&e)
    );
    assert!(
        db_msg(&e).contains("is not held on this node"),
        "the FIRST refusal is the dangling reference, not another rule: {}",
        db_msg(&e)
    );

    // 2. N lands, then the puller re-offers A's SAME bytes: both are admitted and projected.
    apply(&c, &n_bytes).await.expect("the note applies");
    apply(&c, &a_bytes)
        .await
        .unwrap_or_else(|e| panic!("the re-offered addendum applies: {}", db_msg(&e)));
    let rows = c
        .query(
            "SELECT event_id::text, refs::text FROM clinical_note WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap();
    let refs_of = |id: Uuid| {
        rows.iter()
            .find(|r| r.get::<_, String>(0) == id.to_string())
            .map(|r| serde_json::from_str::<serde_json::Value>(r.get(1)).unwrap())
    };
    assert_eq!(refs_of(n_id), Some(serde_json::json!([])), "N is projected");
    assert_eq!(
        refs_of(a_id),
        Some(serde_json::json!([{"rel": "addendum-to", "event": n_id.to_string()}])),
        "A is projected, still naming N"
    );
}
