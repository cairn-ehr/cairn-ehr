//! #619 / ADR-0073 — when the puller cannot ASK whether a refusal is a substitution, it FREEZES.
//!
//! # What is being pinned
//!
//! `pull_into`'s arm for a verifiable event refused with `P0001` asks `node_event` one question
//! before it skips: does this node already hold the event's `event_id` under a different content
//! address? (`sync/substitution.rs::held_content_address`.) A "yes" pens the event as a
//! substitution; a "no" skips it as routine scoping. If the READ ITSELF fails, the loop cannot tell
//! which of the two it is looking at — and the rule for a refusal it cannot classify is the one the
//! pen-write failure already follows: FREEZE the cursor below the event, never advance past it.
//! Skipping instead would file a possible substitution under "self-healing" and move the cursor
//! over it, which is exactly the silence #619 exists to end.
//!
//! # The seam: a role that can do everything `pull_into` does except read `node_event`
//!
//! The lookup has to fail while everything else in the pull still works. A role is how: the pull
//! runs on a connection that has `SET ROLE` to a NOLOGIN role granted exactly what `pull_into`
//! needs, derived from the statements it runs —
//!
//! * `SELECT` on `sync_cursor` (the committed cursor it reads first);
//! * `SELECT`, `INSERT`, `UPDATE`, `DELETE` on `node_event_quarantine` (the re-offer floor, the
//!   pen and its dedupe bump, the auto-release on apply, the `pending` count);
//! * `EXECUTE` on `apply_remote_node_event` and `checkpoint_sync_cursor` (the admission gate and
//!   the advance-only cursor door)
//!
//! — and NOT `SELECT` on `node_event`, the one table only the substitution question reads. Both
//! doors are `SECURITY DEFINER`, so they still read and write `node_event` as their owner: every
//! event before the rival is admitted as usual, the rival is refused with `P0001` as usual, and
//! only the puller's own lookup is refused (`42501`).
//!
//! This is the test behind mutation M10 in `scripts/mutations/2026-09-19-619.sh` (the freeze turned
//! into a skip). It is a separate file only to keep `node_substitution_is_penned.rs` short.
//!
//! # The second seam: a role that can ask, but cannot pen
//!
//! Once the question is answered "substitution", the loop pens the rival — and if the PEN WRITE
//! fails (or the pen is at quota), the same rule holds: freeze, never advance past an event nothing
//! holds (`pen_or_freeze`'s `Frozen`). The second test grants the lookup back (`SELECT` on
//! `node_event`) and withholds exactly one thing instead: `INSERT` on `node_event_quarantine`, the
//! pen's new-row write (PR #623 review, test gap 3; mutation M13). The dedupe `UPDATE` before it is
//! still granted, so the failure lands on the insert itself. The unverifiable arm (#111) reaches the
//! same `pen_or_freeze`, so this one test covers the freeze both arms share.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_node::{db, sync, transport};
use node_plane_kit::{
    call, cs, node_id_hex, peer_event, pen_count, self_node, serve_raw, SelfNode,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The restricted roles. Named `cairn_test_*` so a leftover from a crashed run is recognisable as
/// test debris in the shared cluster; roles are cluster-wide, not per database.
///
/// [`BLIND_PULLER`] cannot read `node_event` (the lookup fails); [`PENLESS_PULLER`] can, but cannot
/// insert a pen row (the pen write fails).
const BLIND_PULLER: &str = "cairn_test_blind_puller";
const PENLESS_PULLER: &str = "cairn_test_penless_puller";

/// Everything `pull_into` needs EXCEPT reading `node_event` and inserting a pen row — the grants
/// both roles share, listed in this file's header. Each role adds back what its test does not
/// withhold.
const COMMON_GRANTS: &str = "GRANT SELECT ON sync_cursor TO {role};
     GRANT SELECT, UPDATE, DELETE ON node_event_quarantine TO {role};
     GRANT EXECUTE ON FUNCTION apply_remote_node_event(bytea) TO {role};
     GRANT EXECUTE ON FUNCTION checkpoint_sync_cursor(text, bigint) TO {role};";

/// Drop `role` if it exists. Run at the START of each test as well as the end (#583's rule: never
/// trust a predecessor's cleanup — a run that panicked mid-way left the role behind). `DROP OWNED`
/// comes first because a role that still holds grants cannot be dropped; it revokes every privilege
/// granted to the role in this database.
async fn drop_role(owner: &Client, role: &str) {
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN
               IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{role}') THEN
                 EXECUTE 'DROP OWNED BY {role}';
                 EXECUTE 'DROP ROLE {role}';
               END IF;
             END $$;"
        ))
        .await
        .expect("dropping the test role must succeed, or the next run inherits it");
}

/// Create `role` with [`COMMON_GRANTS`] plus `extra` (more `GRANT … TO {role};` statements).
async fn create_role(owner: &Client, role: &str, extra: &str) {
    let grants = format!("{COMMON_GRANTS}\n{extra}").replace("{role}", role);
    owner
        .batch_execute(&format!("CREATE ROLE {role} NOLOGIN;\n{grants}"))
        .await
        .expect("the owner may create and grant the test role");
}

/// The fixture both tests share: A holds `peer.added` under `contested` through its own door, and
/// serves a rival `peer.revoked` under the same id as the LAST row. The same fixture as
/// `a_rival_under_a_held_id_is_penned_not_skipped` in `node_substitution_is_penned.rs`. Returns
/// (held seq, rival seq).
async fn hold_then_serve_a_rival(n: &SelfNode) -> (i64, i64) {
    let contested = Uuid::now_v7();
    let subject = node_id_hex(1);
    let held = peer_event(&n.sk, "peer.added", contested, &subject);
    let rival = peer_event(&n.sk, "peer.revoked", contested, &subject);
    call(&n.a, "submit_node_event", &held)
        .await
        .expect("A holds the genuine event");
    let rival_seq = serve_raw(&n.a, &rival).await;
    let held_seq: i64 =
        n.a.query_one(
            "SELECT seq FROM node_event WHERE node_event_id = $1::text::uuid",
            &[&contested.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    (held_seq, rival_seq)
}

/// A connection to the test database that has `SET ROLE` to `role`, with the switch proven.
async fn connect_as(base: &str, role: &str) -> Client {
    let c = db::connect(base).await.unwrap();
    c.batch_execute(&format!("SET ROLE {role}")).await.unwrap();
    let current: String = c
        .query_one("SELECT current_user", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(current, role, "SET ROLE must actually have switched");
    c
}

/// Where this node's committed cursor for the self-pull peer stands, read as the owner.
async fn cursor(n: &SelfNode) -> Option<i64> {
    n.a.query_opt(
        "SELECT last_seq FROM sync_cursor WHERE peer_addr = $1",
        &[&n.addr.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

#[tokio::test]
async fn a_substitution_check_that_cannot_read_the_log_freezes_rather_than_skips() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7954").await;
    drop_role(&n.a, BLIND_PULLER).await;
    let (held_seq, rival_seq) = hold_then_serve_a_rival(&n).await;

    // The trust store is read as the OWNER, before the role switch: it is TLS setup, not part of
    // the pull under test.
    let tls =
        transport::client_config(&n.sk, sync::trust_store_from_db(&n.a).await.unwrap()).unwrap();

    create_role(
        &n.a,
        BLIND_PULLER,
        "GRANT INSERT ON node_event_quarantine TO {role};",
    )
    .await;
    let puller = connect_as(&base, BLIND_PULLER).await;
    // Anti-vacuity: the seam is actually closed — the one read the substitution question makes
    // is refused.
    let denied = puller
        .query("SELECT content_address FROM node_event LIMIT 1", &[])
        .await
        .expect_err("the restricted role must NOT be able to read node_event");
    assert_eq!(
        denied.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "the lookup must fail on a missing grant (42501): {denied:?}"
    );

    let s = sync::pull_into(n.addr, tls, &puller, true)
        .await
        .expect("the cycle itself succeeds — a freeze is reported in PullStats, not as an error");
    puller.batch_execute("RESET ROLE").await.unwrap();

    assert_eq!(
        s.frozen,
        Some(rival_seq),
        "the cycle FROZE at the rival: the loop could not tell a substitution from scoping, so \
         it must not advance past it"
    );
    assert_eq!(
        s.rejected, 0,
        "a refusal the loop could not classify is not filed under self-healing"
    );
    assert_eq!(
        s.quarantined, 0,
        "nothing was penned this cycle — the lookup failed before the pen was reached"
    );
    assert_eq!(pen_count(&n.a).await, 0, "and nothing is in the pen");
    assert_eq!(
        cursor(&n).await,
        Some(held_seq),
        "the cursor holds the handled prefix — up to the genuine event just below the rival — and \
         did not advance past the rival (seq {rival_seq})"
    );

    drop(puller);
    drop_role(&n.a, BLIND_PULLER).await;
    n.serve.abort();
}

/// The loop KNOWS it holds a substitution but cannot pen it: it freezes, never advances past an
/// event nothing holds. Turning that freeze into a skip (mutation M13) would lose the rival — not
/// delay it — with the INTEGRITY line silent, since `pending` counts pen rows and there is none.
#[tokio::test]
async fn a_substitution_that_cannot_be_penned_freezes_rather_than_skips() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7958").await;
    drop_role(&n.a, PENLESS_PULLER).await;
    let (held_seq, rival_seq) = hold_then_serve_a_rival(&n).await;

    let tls =
        transport::client_config(&n.sk, sync::trust_store_from_db(&n.a).await.unwrap()).unwrap();

    create_role(
        &n.a,
        PENLESS_PULLER,
        "GRANT SELECT ON node_event TO {role};",
    )
    .await;
    let puller = connect_as(&base, PENLESS_PULLER).await;
    // Anti-vacuity: the lookup is OPEN (so the loop does find the substitution) …
    puller
        .query("SELECT content_address FROM node_event LIMIT 1", &[])
        .await
        .expect("this role must be able to ask the substitution question");
    // … and the pen's insert is CLOSED. Probed inside a rolled-back transaction, so even a role
    // that could insert would leave nothing behind.
    puller.batch_execute("BEGIN").await.unwrap();
    let denied = puller
        .execute(
            "INSERT INTO node_event_quarantine
                 (content_digest, signed_bytes, peer, refused_seq, reason)
             VALUES ('\\x00', '\\x00', 'probe', 0, 'probe')",
            &[],
        )
        .await
        .expect_err("the restricted role must NOT be able to insert a pen row");
    puller.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(
        denied.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "the pen write must fail on a missing grant (42501): {denied:?}"
    );

    let s = sync::pull_into(n.addr, tls, &puller, true)
        .await
        .expect("the cycle itself succeeds — a freeze is reported in PullStats, not as an error");
    puller.batch_execute("RESET ROLE").await.unwrap();

    assert_eq!(
        s.frozen,
        Some(rival_seq),
        "the cycle FROZE at the rival it could not pen: {s:?}"
    );
    assert_eq!(
        s.rejected, 0,
        "a substitution is never filed under self-healing"
    );
    assert_eq!(
        s.quarantined, 0,
        "nothing was penned — the write was refused"
    );
    assert_eq!(pen_count(&n.a).await, 0, "and nothing is in the pen");
    assert_eq!(
        cursor(&n).await,
        Some(held_seq),
        "the cursor stopped below the rival (seq {rival_seq}), so it is re-offered next cycle"
    );

    drop(puller);
    drop_role(&n.a, PENLESS_PULLER).await;
    n.serve.abort();
}
