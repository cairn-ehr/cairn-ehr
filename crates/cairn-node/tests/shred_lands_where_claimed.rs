//! #431 — a crypto-shred destroys the REAL custody, even when the caller planted decoys.
//!
//! # Why this needs a behavioural test of its own
//!
//! `cairn_execute_shred` (db/037) is `SECURITY DEFINER` and pins `SET search_path = public,
//! pg_temp`; `search_path_pg_temp.rs` checks that spelling in the catalogue. #426 added behavioural
//! tests for the two write DOORS, whose failure is losing a write that was reported as saved. A
//! shredder fails differently, and worse: if its `DELETE`s resolve to tables in the CALLER's temp
//! schema, it deletes the caller's decoy rows, returns cleanly, and the audit trail records a
//! completed erasure while the real key and cleartext survive. Nobody looks again, and it is the
//! one erasure the data subject was told had happened (ADR-0005, §11.5).
//!
//! So this file demonstrates the property rather than the spelling: a client with no privilege
//! beyond `EXECUTE` on `submit_event` and the `TEMPORARY` every role holds plants `event_dek` and
//! `event_clear` decoys, shreds an event, and the real rows are the ones that are gone.
//!
//! # The positive control
//!
//! A decoy test that passes is only evidence if the decoy would have been HIT by a diverted shred.
//! The second test proves that, inside a transaction it rolls back: with the executor's
//! `search_path` deliberately flipped to put `pg_temp` first, the same shred reports success, the
//! decoy rows vanish and the real custody survives. That is the failure the first test rules out.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_event::{sign, Hlc};
use cairn_node::{db, shred::build_shred_body};
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// Decoys for the two custody tables, shaped like the real ones closely enough for the executor's
/// `DELETE … WHERE event_id = $1` to run against them, each seeded with a row for `target`. Created
/// by whoever is the current role, in that session's temp schema.
async fn plant_decoys(c: &Client, target: Uuid) {
    c.batch_execute(
        "CREATE TEMP TABLE event_dek   (event_id uuid PRIMARY KEY, dek_wrapped bytea); \
         CREATE TEMP TABLE event_clear (event_id uuid PRIMARY KEY, body jsonb, twin text);",
    )
    .await
    .expect("any role may create temp tables — that is what makes this reachable");
    c.execute(
        "INSERT INTO pg_temp.event_dek VALUES ($1::text::uuid, '\\x00'::bytea)",
        &[&target.to_string()],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO pg_temp.event_clear VALUES ($1::text::uuid, '{}'::jsonb, 'decoy')",
        &[&target.to_string()],
    )
    .await
    .unwrap();
}

/// `(public.event_dek, public.event_clear, pg_temp.event_dek, pg_temp.event_clear)` rows for
/// `target`. Schema-qualified on every side: an unqualified read from this session would itself be
/// answered by the decoy, which is the failure under test.
async fn where_custody_is(c: &Client, target: Uuid) -> (i64, i64, i64, i64) {
    let row = c
        .query_one(
            "SELECT (SELECT count(*) FROM public.event_dek    WHERE event_id = $1::text::uuid), \
                    (SELECT count(*) FROM public.event_clear  WHERE event_id = $1::text::uuid), \
                    (SELECT count(*) FROM pg_temp.event_dek   WHERE event_id = $1::text::uuid), \
                    (SELECT count(*) FROM pg_temp.event_clear WHERE event_id = $1::text::uuid)",
            &[&target.to_string()],
        )
        .await
        .unwrap();
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

/// A sealed medication assert that this node holds WITH custody — the thing a shred destroys.
async fn held_with_custody(c: &Client, keys: &Keys) -> SealedAssert {
    let e = sealed_assert(
        keys,
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        "amoxicillin",
        WALL,
    );
    apply_with_key(c, &e).await.expect("admitted with custody");
    e
}

/// Shred `e` through the STRICT door, as the runtime role a client holds.
async fn shred_as_client(c: &Client, keys: &Keys, e: &SealedAssert) {
    let body = build_shred_body(
        Uuid::now_v7(),
        e.patient,
        e.event_id,
        "patient request",
        &keys.kid_device,
        None,
        Hlc {
            wall: WALL + 5,
            counter: 0,
            node_origin: "peer".into(),
        },
    );
    let signed = sign(&body, &keys.sk_device).expect("sign the tombstone");
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the shred is admitted and reports success");
}

/// THE PROPERTY. Decoys planted by the caller, a shred through the door: the REAL custody and the
/// projection are gone, and the decoys were never touched.
#[tokio::test]
async fn a_shred_destroys_the_real_custody_not_a_callers_decoy() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = held_with_custody(&c, &keys).await;

    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    plant_decoys(&c, e.event_id).await;
    shred_as_client(&c, &keys, &e).await;
    c.batch_execute("RESET ROLE").await.unwrap();

    let custody = where_custody_is(&c, e.event_id).await;
    let projected = statement_rows(&c, e.medication_id).await;
    c.batch_execute("DROP TABLE pg_temp.event_dek, pg_temp.event_clear")
        .await
        .unwrap();

    assert_eq!(
        custody,
        (0, 0, 1, 1),
        "(real dek, real clear, decoy dek, decoy clear): the shred reported success, so the REAL \
         custody must be gone and the caller's decoys untouched — anything else is an erasure \
         recorded in the audit trail that never happened (#431)"
    );
    assert_eq!(projected, 0, "and the medication list row is scrubbed too");
}

/// THE POSITIVE CONTROL. With the executor's `search_path` flipped to put `pg_temp` FIRST — the
/// regression the test above guards against — the same shred reports success while deleting only
/// the decoys. Proves the decoys sit exactly where a diverted shred would land, so the test above
/// passing is evidence and not luck.
///
/// Everything runs inside one transaction that is ROLLED BACK, DDL included (Postgres DDL is
/// transactional): a panic drops the connection, which aborts the transaction the same way, so the
/// flipped function can never outlive this test.
#[tokio::test]
async fn a_diverted_shred_would_report_success_while_destroying_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;
    let e = held_with_custody(&c, &keys).await;

    c.batch_execute(
        "BEGIN; \
         ALTER FUNCTION cairn_execute_shred(uuid, uuid, text) SET search_path = pg_temp, public; \
         SET LOCAL ROLE cairn_agent;",
    )
    .await
    .unwrap();
    plant_decoys(&c, e.event_id).await;
    shred_as_client(&c, &keys, &e).await;
    // Back to the owner to read: cairn_agent has no SELECT on the real custody tables.
    c.batch_execute("RESET ROLE").await.unwrap();
    let custody = where_custody_is(&c, e.event_id).await;
    c.batch_execute("ROLLBACK").await.unwrap();

    assert_eq!(
        custody,
        (1, 1, 0, 0),
        "control: a shred whose search_path reaches pg_temp first must delete the DECOYS and \
         leave the real custody — if it does not, the decoys are not where a diversion would land \
         and the property test proves nothing"
    );
}
