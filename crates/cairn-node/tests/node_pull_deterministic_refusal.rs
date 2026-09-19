//! #621 — the node pull no longer freezes on a failure that will never change.
//!
//! The three outcomes, end to end, through the real `pull_into` over the real mTLS self-pull
//! (`common/node_plane_kit.rs`): node A serves its own `node_event` log to itself, and a row
//! raw-inserted into that log carries whatever bytes the test wants a peer to have served.
//!
//! | what the door does | what the puller must do | why |
//! | --- | --- | --- |
//! | refuses a malformed field with P0001 (#621's db/007 half) | SKIP, advance | it is a verdict, and a later build may understand the event |
//! | fails deterministically with NO verdict (22/23/XX…) | PEN, advance | it will fail identically forever; freezing wedges the link |
//! | fails for a reason local to this node (40/42/53/57…) | FREEZE | the same bytes may well apply next cycle |
//!
//! The middle row is the fix. Before it, the second and third rows were one arm, so one poison
//! event held every later event on that link behind it — for ever, with nothing penned and so no
//! `ack` remedy.
//!
//! **Fault injection without residue:** the two no-verdict cases need a door failure that #621's
//! own db/007 work has made unreachable through any payload, so they are injected by a
//! `cairn_test_*` trigger on `node_event` that raises a chosen SQLSTATE for ONE marked event
//! (`scope_hint`), created and dropped inside the test. The alternative — asserting against a
//! malformed payload — would pin the CURRENT list of deterministic raises rather than the
//! puller's rule, and the whole point of the rule is the raise nobody has written yet.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_node::{db, sync};
use node_plane_kit::{
    cs, node_event_spelled, node_id_hex, pen_count, self_node, serve_raw, SelfNode,
};
use uuid::Uuid;

/// The marker that tells the injected trigger which event to fail. It rides `scope_hint`, a free
/// text column no door interprets, so the event is otherwise entirely ordinary.
const POISON: &str = "cairn-test-621-poison";

async fn full_pull(base: &str, n: &SelfNode) -> sync::PullStats {
    let cfg = sync::client_config(base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    sync::pull_once(n.addr, cfg, true).await.unwrap()
}

/// This node's committed cursor for the self-pull peer.
async fn cursor(n: &SelfNode) -> Option<i64> {
    n.a.query_opt(
        "SELECT last_seq FROM sync_cursor WHERE peer_addr = $1",
        &[&n.addr.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// Make `apply_remote_node_event` fail with `sqlstate` for the marked event only, by a trigger
/// named `cairn_test_*` and dropped again at the end of the test. Dropped FIRST as well as last:
/// a previous killed run must not be able to leave one behind and turn this run's result into a
/// lie (#583's lesson — `cairn_test` is never recreated between sweeps).
async fn inject_failure(n: &SelfNode, sqlstate: &str) {
    n.a.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_621_poison ON node_event;
         DROP FUNCTION IF EXISTS cairn_test_621_poison();",
    )
    .await
    .unwrap();
    n.a.batch_execute(&format!(
        "CREATE FUNCTION cairn_test_621_poison() RETURNS trigger
         LANGUAGE plpgsql SET search_path = public, pg_temp AS $fn$
         BEGIN
             RAISE EXCEPTION 'injected fault for the #621 pull test'
                 USING ERRCODE = '{sqlstate}';
         END;
         $fn$;
         CREATE TRIGGER cairn_test_621_poison BEFORE INSERT ON node_event
             FOR EACH ROW WHEN (NEW.scope_hint = '{POISON}')
             EXECUTE FUNCTION cairn_test_621_poison();"
    ))
    .await
    .unwrap();
}

async fn remove_injected_failure(n: &SelfNode) {
    n.a.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_621_poison ON node_event;
         DROP FUNCTION IF EXISTS cairn_test_621_poison();",
    )
    .await
    .unwrap();
}

/// A `peer.added` signed by A itself — so the admission gate's trust checks pass and the event
/// reaches the INSERT, where the injected trigger is waiting for its marker.
fn poisoned_event(n: &SelfNode, lineage: u8) -> Vec<u8> {
    node_event_spelled(
        &n.sk,
        "peer.added",
        &Uuid::now_v7().to_string(),
        2,
        serde_json::json!({
            "peer_node_id_hex": node_id_hex(lineage),
            "role": "peer",
            "scope_hint": POISON,
        }),
    )
}

/// THE ISSUE'S OWN SCENARIO. A verifiable event whose `event_id` is not a UUID used to raise
/// `22P02` from a cast before any trust check, and freeze this peer's cursor permanently. With
/// the door raising P0001 it is an ordinary verdict: skipped, the cursor advances, and a later
/// build that understands the event can still admit it on a full sweep.
#[tokio::test]
async fn a_non_uuid_event_id_is_skipped_and_the_cursor_advances() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:0").await;

    let malformed = node_event_spelled(
        &n.sk,
        "peer.added",
        "not-a-uuid",
        2,
        serde_json::json!({ "peer_node_id_hex": node_id_hex(1), "role": "peer" }),
    );
    let seq = serve_raw(&n.a, &malformed).await;

    let stats = full_pull(&base, &n).await;
    assert_eq!(
        stats.frozen, None,
        "a malformed event must not freeze the link: every LATER event on it would be held \
         behind this one, including a peer.revoked (#621)"
    );
    assert!(
        stats.rejected >= 1,
        "it must be counted as a refusal the puller skipped past: {stats:?}"
    );
    assert_eq!(
        pen_count(&n.a).await,
        0,
        "and NOT penned: a door verdict is the self-healing class — penning every malformed \
         event would ask a human to ack what an upgrade may simply admit"
    );
    assert!(
        cursor(&n).await.is_some_and(|c| c >= seq),
        "the cursor advanced past it"
    );
}

/// THE FIX. A failure with no verdict that will recur identically is penned — durably held, loud,
/// ack-able — and the cursor advances.
#[tokio::test]
async fn a_deterministic_failure_without_a_verdict_is_penned_not_frozen() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:0").await;
    // 23514 is what the node_event CHECKs raised before #621, and what a FUTURE constraint or a
    // widened vocabulary will raise again.
    inject_failure(&n, "23514").await;

    let poisoned = poisoned_event(&n, 2);
    let seq = serve_raw(&n.a, &poisoned).await;

    let stats = full_pull(&base, &n).await;
    assert_eq!(
        stats.frozen, None,
        "freezing under a failure that recurs identically is the #621 defect: the link never \
         moves again, and nothing is penned so there is nothing to ack"
    );
    assert_eq!(
        stats.quarantined, 1,
        "it must be PENNED: no verdict about these bytes exists, so the pen is the honest \
         record — and it auto-releases if a later build admits them. Got {stats:?}"
    );
    assert!(
        cursor(&n).await.is_some_and(|c| c >= seq),
        "the cursor advanced past the penned event, as it does for every other pen"
    );
    let reason: String =
        n.a.query_one(
            "SELECT reason FROM node_event_quarantine ORDER BY first_seen DESC LIMIT 1",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        reason.contains("23514"),
        "the reason must speak the DATABASE's vocabulary, SQLSTATE included — writing a \
         non-verdict in the door's voice is what #480 was filed about; got: {reason}"
    );
    remove_injected_failure(&n).await;
}

/// THE DIRECTION THAT MUST NOT MOVE. A failure local to this node — a deadlock, a missing grant,
/// a full disk — still freezes: the same bytes may well apply on the next cycle, and penning them
/// would record a refusal that never happened.
#[tokio::test]
async fn a_local_fault_still_freezes_the_cursor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:0").await;
    // 40001 serialization_failure: the textbook retry-and-it-works fault.
    inject_failure(&n, "40001").await;

    let poisoned = poisoned_event(&n, 3);
    let seq = serve_raw(&n.a, &poisoned).await;

    let stats = full_pull(&base, &n).await;
    assert_eq!(
        stats.frozen,
        Some(seq),
        "a transient local fault must still FREEZE below the event: {stats:?}"
    );
    assert_eq!(
        pen_count(&n.a).await,
        0,
        "and must not be penned — a pen row would claim the door refused these bytes when it \
         never reached a verdict about them"
    );
    assert!(
        cursor(&n).await.is_none_or(|c| c < seq),
        "the cursor must not advance past an event this node has not handled"
    );
    remove_injected_failure(&n).await;
}

/// THE NEW ARM'S OWN FREEZE PATH. Penning is how the cursor is allowed to advance past a
/// refusal — the bytes are durably held — so a pen that CANNOT be written must freeze instead.
/// Advancing past an unpenned refusal would lose it until the next full sweep, the #111 review's
/// A1, and this arm is new enough that the shared `pen_or_freeze` outcome could be dropped on the
/// floor here without any other suite noticing.
#[tokio::test]
async fn a_deterministic_refusal_whose_pen_cannot_be_written_freezes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:0").await;
    inject_failure(&n, "23514").await;
    // A second injected fault, on the pen itself: the door refuses deterministically AND the pen
    // write fails, which is the only combination that reaches this arm's Frozen outcome.
    n.a.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_621_pen_fails ON node_event_quarantine;
         DROP FUNCTION IF EXISTS cairn_test_621_pen_fails();
         CREATE FUNCTION cairn_test_621_pen_fails() RETURNS trigger
         LANGUAGE plpgsql SET search_path = public, pg_temp AS $fn$
         BEGIN
             RAISE EXCEPTION 'injected: the pen cannot be written';
         END;
         $fn$;
         CREATE TRIGGER cairn_test_621_pen_fails BEFORE INSERT ON node_event_quarantine
             FOR EACH ROW EXECUTE FUNCTION cairn_test_621_pen_fails();",
    )
    .await
    .unwrap();

    let poisoned = poisoned_event(&n, 5);
    let seq = serve_raw(&n.a, &poisoned).await;

    let stats = full_pull(&base, &n).await;
    assert_eq!(
        stats.frozen,
        Some(seq),
        "with nothing holding the refused bytes, the cursor must stop below them: {stats:?}"
    );
    assert_eq!(stats.quarantined, 0, "nothing was penned: {stats:?}");
    assert!(
        cursor(&n).await.is_none_or(|c| c < seq),
        "and the committed cursor did not move past the event"
    );

    n.a.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_621_pen_fails ON node_event_quarantine;
         DROP FUNCTION IF EXISTS cairn_test_621_pen_fails();",
    )
    .await
    .unwrap();
    remove_injected_failure(&n).await;
}

/// The anti-vacuity control for the two injected cases. With the SAME trigger installed, the
/// events that carry no marker — node A's own genesis and self-peering, which the self-pull
/// re-offers on every sweep — still apply. Without this, a trigger that raised for every row
/// would make both tests above pass for the wrong reason: the pen/freeze split would have been
/// measured against a node that could not admit anything at all.
///
/// It deliberately serves NOTHING extra. A row planted by `serve_raw` carries bytes that are
/// already in `node_event` under a DIFFERENT id (that is how the fixture makes a peer serve
/// something), so re-applying it conflicts on the `content_address` UNIQUE rather than on the
/// primary key — `ON CONFLICT (node_event_id)` does not cover that — and raises `23505`. The
/// puller now PENS that (correctly: it is deterministic), which is right but would make this
/// control assert a pen it is not about. The artifact cannot happen in production: bytes
/// determine the `event_id` inside them, so identical bytes always collide on the primary key
/// first and take the `DO NOTHING` path.
#[tokio::test]
async fn the_injected_fault_touches_only_the_marked_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:0").await;
    inject_failure(&n, "23514").await;

    let stats = full_pull(&base, &n).await;
    assert_eq!(
        stats.frozen, None,
        "an unmarked event is untouched: {stats:?}"
    );
    assert_eq!(
        stats.quarantined, 0,
        "nothing unmarked may be penned: {stats:?}"
    );
    assert_eq!(
        pen_count(&n.a).await,
        0,
        "and the pen is empty afterwards: {stats:?}"
    );
    assert!(
        stats.admitted >= 1,
        "A's own events were ADMITTED through the door with the trigger installed, which is \
         what makes the two tests above meaningful: {stats:?}"
    );
    remove_injected_failure(&n).await;
}
