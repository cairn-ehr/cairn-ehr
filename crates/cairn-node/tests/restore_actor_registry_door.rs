//! `restore_actor_registry` — the privilege, and the Rust caller that drives it (#554 slice 2d).
//!
//! **The door's BEHAVIOUR is pinned at the SQL layer**, in
//! `db/tests/052_restore_doors_test.sql`: the two fences, the resume path, the
//! `(recorded_at, seq)` ordering property, the identity counter, and the `recorded_at`
//! refusal. That is where they belong — they are properties of the SQL, and a later slice
//! could legitimately replace this Rust caller without touching any of them.
//!
//! **Two things the mirror cannot prove, and they are here.**
//!
//! 1. **The privilege.** `restore_actor_registry` writes the trust anchor every clinical apply
//!    door gates on, and it deliberately bypasses `enroll_actor`'s collision guards, so it is
//!    the highest-value new privilege in this slice. It is granted to `cairn_node` and
//!    explicitly NOT to `cairn_agent`: an advisory actor that could "restore" a registry could
//!    re-authorise itself, and it would do so through a door that does not re-adjudicate.
//!    A `SET ROLE` needs a live connection, not a psql mirror. This is the #430/#431 shape —
//!    a decoy path around a floor that looks correct at its own site — applied to the most
//!    dangerous door the slice adds, so the grant is a TESTED property rather than a comment.
//!
//! 2. **That the Rust caller actually calls it.** `apply_local_state` carried the registry
//!    rows and installed nothing between slice 2c and this one, reporting a "carried" count
//!    that no operator could act on. A count that nobody applies is precisely the failure this
//!    slice corrects, so the wiring gets its own end-to-end assertion.
//!
//! ⚠️ `SET ROLE`, never `SET LOCAL ROLE` — outside a transaction block the latter is a
//! Postgres NO-OP that emits only a WARNING, so a refusal test written that way can pass
//! without ever having provoked one. `current_user` is asserted before any refusal is
//! trusted. Same reasoning, at length, in `custody_view_privileges.rs`'s header.

mod common;
use cairn_node::db;
use cairn_node::localstate::{
    apply_local_state, ActorRegistryRow, CustodyKeyDestination, LocalState,
};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A node-id-shaped actor id, DERIVED rather than written out (house rule 6): a 32-byte
/// literal in a file that also handles key material is exactly what CodeQL's hard-coded
/// cryptographic value query is for, and this value is a content address, not a key.
fn actor_id(lineage: u8) -> Vec<u8> {
    (0..32u8).map(|i| lineage.wrapping_add(i)).collect()
}

/// `cairn_agent` and `PUBLIC` cannot execute the registry restore door.
///
/// The refusal must be an ACL refusal, so the test proves the role switch actually happened
/// and proves the privileged role CAN call the door first — otherwise "it failed" would be
/// indistinguishable from "the function is missing" or "the fixture was malformed".
#[tokio::test]
async fn only_cairn_node_may_restore_the_actor_registry() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clear_restore_target(&c).await;

    // Anti-vacuity: the privileged role reaches the door and it works. An empty set is a
    // legal call — it inserts nothing and returns 0 — so this proves reachability without
    // writing a registry the refusal tests would then have to work around.
    let n: i32 = c
        .query_one("SELECT restore_actor_registry('[]'::jsonb)", &[])
        .await
        .expect("cairn_node may execute the door")
        .get(0);
    assert_eq!(n, 0, "an empty set inserts nothing and says so");

    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let who: String = c
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        who, "cairn_agent",
        "the role switch must actually have happened, or every refusal below is vacuous"
    );

    let err = c
        .query_one("SELECT restore_actor_registry('[]'::jsonb)", &[])
        .await
        .expect_err(
            "an advisory actor that could restore a registry could re-authorise itself, \
             through a door that deliberately does not re-adjudicate",
        );
    // `Display` on a tokio_postgres::Error is just "db error" — the server's message lives
    // on the `DbError`. `common::db_msg` is the shared accessor, so a refusal assertion here
    // reads the same text an operator would see.
    let msg = common::db_msg(&err);
    assert!(
        msg.contains("permission denied"),
        "the refusal must be the ACL's, not an incidental one: {msg}"
    );

    c.batch_execute("RESET ROLE").await.unwrap();
}

/// `apply_local_state` INSTALLS the carried registry, and reports what it installed.
///
/// Between slice 2c and this one it CARRIED the rows and installed nothing, reporting a
/// count no operator could act on — the exact shape of failure this slice corrects. So this
/// drives the production entry point rather than the SQL, which is the only way to prove the
/// wiring exists at all: every assertion below passes against a door that works perfectly and
/// a caller that never calls it, if the caller is left out of the test.
///
/// It also pins the two counts APART. A resumed restore carries N rows and installs fewer,
/// and either number reported alone tells a false story — "N carried" hides that a resume
/// happened, "0 restored" reads as data loss.
#[tokio::test]
async fn apply_local_state_installs_the_carried_registry_and_reports_what_landed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    clear_restore_target(&c).await;

    let dir = tempfile::tempdir().unwrap();
    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));

    // An export carrying a registry and no custody key. Custody is a different subsystem with
    // its own tests (`restore_inherits_custody.rs`); leaving it out keeps this test about the
    // registry, and exercises the `no_custody_key` arm — which must report the registry counts
    // exactly as the inherited arm does, or an operator whose export lost its key would also
    // silently lose the one signal saying their history is applicable.
    let rows: Vec<Vec<u8>> = [
        registry_row("aaaaaaaa-0000-7000-8000-00000000000a", &actor_id(1), 1),
        registry_row("aaaaaaaa-0000-7000-8000-00000000000b", &actor_id(2), 2),
    ]
    .iter()
    .map(cairn_node::localstate::actor_registry_row_to_cbor)
    .collect();
    let bundle = LocalState::from_custody_and_registry(Vec::new(), None, rows);

    let report = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Plaintext { path: &new_unwrap },
    )
    .await
    .expect("a registry-only export must apply");

    assert_eq!(report.actor_registry_carried(), 2);
    assert_eq!(
        report.actor_registry_restored(),
        2,
        "a clean install lands both rows, and the caller must SAY so — a carried count \
         nobody applies is the failure this slice corrects"
    );

    let present: i64 = c
        .query_one("SELECT count(*) FROM actor_event", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(present, 2, "the rows are actually in actor_event");

    // The RESUME path, through the production caller. The door is set-shaped and idempotent,
    // so a second run inserts nothing and reports zero — while still reporting that two rows
    // were carried. Design §3's "finalize_identity moves LAST" argument rests on exactly this
    // being true through the caller, not only through the SQL.
    let again = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Plaintext { path: &new_unwrap },
    )
    .await
    .expect("a resumed restore must complete, not refuse");
    assert_eq!(again.actor_registry_carried(), 2);
    assert_eq!(
        again.actor_registry_restored(),
        0,
        "a re-run reports what IT inserted; the two counts must not collapse into one"
    );

    clear_restore_target(&c).await;
}

/// One registry row in the shape the export carries. `kind`/`signing_key_id` are filled so the
/// optional-field encoding in `actor_registry_rows_to_json` is exercised rather than assumed.
fn registry_row(actor_event_id: &str, actor_id: &[u8], seq: i64) -> ActorRegistryRow {
    ActorRegistryRow {
        actor_event_id: actor_event_id.to_string(),
        actor_id: actor_id.to_vec(),
        op: "enroll".into(),
        kind: Some("human".into()),
        pinned: None,
        signing_key_id: Some(format!("beef{seq:02}")),
        superseded_by: None,
        seq,
        recorded_at: format!("2026-01-01 00:00:00.00000{seq}+00"),
    }
}

/// Put the database in the state a restore target is in: un-enrolled, with an empty registry.
///
/// `actor_event` is append-only (db/004 refuses DELETE by trigger), so clearing it means
/// disabling that trigger for the duration. That is a test-fixture act and never something a
/// node does — the door's own fence 2 is what protects a real registry, and it is pinned in
/// `db/tests/052_restore_doors_test.sql`.
async fn clear_restore_target(c: &tokio_postgres::Client) {
    c.batch_execute(
        "DELETE FROM local_node;
         ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update;
         DELETE FROM actor_event;
         ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update;",
    )
    .await
    .unwrap();
}
