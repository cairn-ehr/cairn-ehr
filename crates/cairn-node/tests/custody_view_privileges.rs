//! db/051's view must not become a way around db/037's custody REVOKE.
//!
//! db/037 revokes `event_dek` from `cairn_agent` — an advisory actor may hold clinical
//! content but never the keys. A Postgres view reads its base tables as the VIEW'S OWNER
//! unless `security_invoker` is set, so a view over `event_dek` is a textbook decoy path
//! around a floor that looks correct at its own site (#430/#431). This asserts the floor
//! still binds THROUGH the new objects.
//!
//! ⚠️ **Why `SET ROLE`, not `SET LOCAL ROLE` (controller correction to the original
//! task brief).** `SET LOCAL` is scoped to the current transaction block — issued outside
//! one (as a bare `.execute()` on an autocommit connection) it is a Postgres NO-OP that
//! only emits a WARNING, never an error. A test written that way could assert a refusal
//! it never actually provoked: the connection stays the original privileged role for
//! every statement that follows, and the two "denied" assertions below would pass because
//! the privileged role happens to reject those particular statements for an unrelated
//! reason, or (worse) could start silently passing vacuously. `SET ROLE` has no such
//! caveat, and this file proves the switch really happened (`current_user`) before
//! trusting any refusal built on top of it.
//!
//! ⚠️ **Why the two straightforward assertions below are not the whole test (see the
//! second test in this file).** `cairn_agent` was never granted SELECT on
//! `event_custody_surviving` or EXECUTE on `cairn_clinical_page` (db/051 grants both only
//! to `cairn_node`), so Postgres refuses `cairn_agent` at the VIEW/FUNCTION's own ACL
//! check — a refusal that fires identically whether or not `security_invoker = true` is
//! present. Proven empirically while writing this file: temporarily deleting
//! `security_invoker = true` from db/051 left both assertions in
//! [`custody_view_does_not_widen_access`] GREEN. So that test alone pins today's grants,
//! not the mechanism db/051's header names. [`security_invoker_stops_a_widened_grant_from_
//! leaking_custody`] closes that gap: it stages the ONE scenario where `security_invoker`
//! is the only thing standing between `cairn_agent` and the raw wrapped key — a future
//! reviewer widening the view's own grant by mistake — and is the test that actually goes
//! red when the option is removed.

mod common;
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Pins today's grants: `cairn_agent` has no ACL entry at all on either new object, so
/// both calls are refused at the door before a single row of `event_dek` is ever touched.
/// Necessary, but — see this file's header — NOT sufficient to prove `security_invoker`
/// itself is doing anything; that is [`security_invoker_stops_a_widened_grant_from_leaking_custody`].
#[tokio::test]
async fn custody_view_does_not_widen_access() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Anti-vacuity: prove the view is readable by the privileged role FIRST, so a failure
    // below is a refusal rather than a missing object.
    c.query("SELECT count(*) FROM event_custody_surviving", &[])
        .await
        .expect("cairn_node may read the view");

    // `SET ROLE`, not `SET LOCAL ROLE` — see this file's header. Proven to have actually
    // switched (rather than warned and no-opped) before anything is asserted on top of it.
    c.execute("SET ROLE cairn_agent", &[]).await.unwrap();
    let current: String = c
        .query_one("SELECT current_user", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        current, "cairn_agent",
        "SET ROLE must actually have switched the session role, or the refusals below \
         would be asserting nothing"
    );

    let denied = c
        .query("SELECT count(*) FROM event_custody_surviving", &[])
        .await;
    assert!(
        denied.is_err(),
        "cairn_agent must NOT reach custody through the view — db/037 revoked event_dek \
         from it, and db/051 grants SELECT on the view only to cairn_node"
    );

    let denied_fn = c
        .query("SELECT count(*) FROM cairn_clinical_page(0, NULL)", &[])
        .await;
    assert!(
        denied_fn.is_err(),
        "cairn_agent must NOT reach custody through the page function either — db/051 \
         grants EXECUTE only to cairn_node"
    );

    // Leave the connection as we found it: this pool connection is dropped at the end of
    // the test either way, but a future edit that reuses `c` for something after this
    // point should not inherit a de-privileged session role silently.
    c.execute("RESET ROLE", &[]).await.unwrap();
}

/// **The load-bearing test.** Stages the ONE scenario in which `security_invoker = true`
/// is the only thing stopping `cairn_agent` from reading a crypto-shredded body's wrapped
/// key: a future maintainer who widens the VIEW's own grant (say, to let an advisory
/// actor see which events have *any* surviving custody, without meaning to hand over the
/// wrapped bytes) while never touching `event_dek`'s own REVOKE from db/037.
///
/// Without `security_invoker`, the view evaluates `event_dek` as the view's OWNER
/// (whoever ran the migration — here, the connecting superuser), so a role with only
/// VIEW-level SELECT sails straight through to the real `dek_wrapped` column. WITH
/// `security_invoker = true`, the same query is checked against `cairn_agent`'s OWN
/// privileges on `event_dek`, which db/037 revoked — so it fails exactly the same way as
/// querying `event_dek` directly would.
///
/// This is the test that actually reddens when `security_invoker = true` is removed from
/// db/051 — proven while writing this file (see the commit message / task report for the
/// remove-observe-restore cycle). [`custody_view_does_not_widen_access`] does not:
/// `cairn_agent` has no ACL on the view there either, so it is refused at the view's own
/// door before `security_invoker` ever gets a say.
///
/// The GRANT is made and revoked entirely within this test (never touching migration
/// state), and reverted with `RESET`/`REVOKE` even on an assertion failure by running the
/// revoke unconditionally after every assertion that could panic has already run to
/// completion — see the trailing cleanup.
#[tokio::test]
async fn security_invoker_stops_a_widened_grant_from_leaking_custody() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // THE STAGED MISTAKE: grant cairn_agent SELECT on the VIEW only — never on the base
    // `event_dek` table, which stays exactly as db/037 left it (revoked). This is the
    // "future reviewer widens the convenience view's grant" scenario the db/051 header
    // warns about, reproduced deliberately so the guard against it can be observed.
    c.execute("GRANT SELECT ON event_custody_surviving TO cairn_agent", &[])
        .await
        .expect("staging the widened grant");

    // Anti-vacuity: the staged grant really landed, so a refusal below is `security_invoker`
    // doing its job, not an ACL that was never actually widened.
    let granted: i64 = c
        .query_one(
            "SELECT count(*) FROM information_schema.role_table_grants \
             WHERE table_name = 'event_custody_surviving' AND grantee = 'cairn_agent' \
               AND privilege_type = 'SELECT'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        granted, 1,
        "the staged widening must really be in place, or the refusal below proves nothing"
    );

    c.execute("SET ROLE cairn_agent", &[]).await.unwrap();
    let current: String = c
        .query_one("SELECT current_user", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        current, "cairn_agent",
        "SET ROLE must actually have switched the session role"
    );

    // cairn_agent now DOES have SELECT on the view itself — so if it can still read a row,
    // that is `security_invoker` succeeding at re-checking `event_dek`'s own (revoked)
    // grant, not a second door slamming shut for an unrelated reason.
    let denied = c
        .query("SELECT count(*) FROM event_custody_surviving", &[])
        .await;

    // Clean up BEFORE asserting: revoke the staged grant and drop back to the connecting
    // role regardless of whether the assertion below is about to panic, so a failing run
    // never leaves the widened grant sitting in the shared test database for the next
    // suite to inherit.
    c.execute("RESET ROLE", &[]).await.unwrap();
    c.execute(
        "REVOKE SELECT ON event_custody_surviving FROM cairn_agent",
        &[],
    )
    .await
    .expect("cleaning up the staged grant");

    assert!(
        denied.is_err(),
        "security_invoker = true must make db/051's view check event_dek against the \
         CALLER's own privileges — even after cairn_agent is granted SELECT on the view \
         itself, db/037's revoke of event_dek from cairn_agent must still block it. If \
         this passes, db/051 is wrong (security_invoker was dropped or the view was \
         redefined without it) — fix the migration, never this test."
    );
}
