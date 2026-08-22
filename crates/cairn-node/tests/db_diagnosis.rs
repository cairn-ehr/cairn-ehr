//! #467 — a database failure in `cairn-node` must name itself.
//!
//! The pure half of the renderer (`compose_db_diagnosis`) is unit-tested inside
//! `src/db_diagnosis.rs` with no database, because a `tokio_postgres::DbError` cannot be
//! constructed by hand. What CANNOT be tested there is the half that matters most: that
//! the three fields are actually pulled off a real server error, and that the schema
//! loader's own wrapping carries them through. Both need a live PostgreSQL, so they live
//! here.
//!
//! The acceptance criterion of #467 is a sentence about a MESSAGE:
//!
//! > A schema-load failure names the migration, the SQLSTATE **and** the server's message
//! > + DETAIL.
//!
//! …so the second test drives the loader's real per-migration door (`db::load_migration`)
//! with a body that is guaranteed to fail, and reads the text a human would have seen.

use cairn_node::db;
use cairn_node::db_diagnosis::legible_db_error;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The extraction arm: message, SQLSTATE and DETAIL all come off a real server error.
///
/// The mutation this pins is the one the whole issue is about — `e.to_string()` in place
/// of the helper — which renders every one of these as the literal `"db error"`. The
/// statement is a bare `RAISE … USING DETAIL`, which is the exact shape this project's
/// own in-DB doors take when they refuse something (issue #109 puts the reason in DETAIL).
#[tokio::test]
async fn a_server_error_carries_message_sqlstate_and_detail() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // No schema load and no TRUNCATE: this test touches nothing, so it needs neither the
    // serial guard nor a loaded database — a bare connection is the whole fixture.
    let c = db::connect(&base).await.unwrap();

    let e = c
        .batch_execute(
            "DO $$ BEGIN RAISE EXCEPTION 'submit_event: signer is not enrolled'
                        USING DETAIL = 'cairn_verify_error: unknown key'; END $$;",
        )
        .await
        .expect_err("a bare RAISE always fails");
    let text = legible_db_error(&e);

    assert_ne!(text, "db error", "the entire point of #467: {text}");
    assert!(
        text.contains("signer is not enrolled"),
        "the server's own message must survive: {text}"
    );
    assert!(
        text.contains("[P0001]"),
        "…and the SQLSTATE, which is the part an operator can search for: {text}"
    );
    assert!(
        text.contains("cairn_verify_error: unknown key"),
        "…and the DETAIL, which is where this project's doors put the reason: {text}"
    );
}

/// The acceptance criterion itself: a failed migration names the MIGRATION and the cause.
///
/// `load_migration` is the loader's real per-migration door — `connect_and_load_schema`
/// calls exactly this for each embedded body — so the composition under test is the one
/// that produced the undiagnosable CI line, not a re-statement of it. The SQL body is
/// synthetic on purpose: forcing a genuine failure inside a real migration would mean
/// planting a decoy object in a shared test database, which is both destructive and
/// coupled to whichever migration happened to trip over it.
#[tokio::test]
async fn a_failed_migration_names_the_migration_the_message_and_the_sqlstate() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = db::connect(&base).await.unwrap();

    let err = db::load_migration(&c, "031_medication", "SELECT no_such_function_here();")
        .await
        .expect_err("the body cannot succeed");
    let text = format!("{err}");

    assert!(
        text.contains("031_medication"),
        "the migration name is what turns a wall of SQL into a place to look: {text}"
    );
    assert!(
        text.contains("[42883]"),
        "undefined_function, the SQLSTATE that says which KIND of failure this was: {text}"
    );
    assert!(
        text.contains("no_such_function_here"),
        "and the server's message, which names the thing that was missing: {text}"
    );
    // The regression that filed the issue, stated as an assertion: the whole rendered
    // line used to be "loading 031_medication: db error", and a re-run was the only
    // diagnostic tool available.
    assert!(
        !text.ends_with("db error"),
        "this is the exact CI line #467 was filed for: {text}"
    );
}

/// The connect door: a failure there must name the server's reason, and must NEVER echo
/// the connection string back — it can carry a password.
#[tokio::test]
async fn a_failed_connect_names_the_reason_and_never_the_connection_string() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // A database name nothing will ever create, appended to the working base string so
    // host/port/user stay valid and the server itself answers with 3D000.
    let conn = format!("{base} dbname=cairn_no_such_database_467");
    let err = db::connect(&conn)
        .await
        .expect_err("that database does not exist");
    let text = format!("{err}");

    assert!(
        text.contains("[3D000]"),
        "invalid_catalog_name — the operator needs to know it is the DATABASE, not the \
         credentials or the network: {text}"
    );
    // The database NAME appearing is fine and useful — it is the server's own message.
    // What must never appear is the connection string we were handed, which in a real
    // deployment carries a password.
    assert!(
        !text.contains(&conn) && !text.contains("password="),
        "the connection string is never echoed back: {text}"
    );
}
