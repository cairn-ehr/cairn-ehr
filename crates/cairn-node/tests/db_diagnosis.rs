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
//!
//! The last two need no database at all and are therefore NOT gated: the arm that renders
//! a failure with no `DbError` to read (a refused socket) needs only a port nothing
//! listens on, and it is the arm the PR #472 review found had regressed — the DB-gated
//! tests below reach `connect` only through `3D000`, which is a server ANSWERING.

use cairn_node::db;
use cairn_node::db_diagnosis::legible_db_error;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A password-shaped canary, DERIVED at runtime — house rule 6.
///
/// Every test below asserts this string is ABSENT from some rendering, so it has to look
/// like a real secret. Writing it as a literal beside a `password=` key is exactly the
/// shape CodeQL's hard-coded-credential queries flag, which blocks the scan until a human
/// dismisses it (issue #146). Deriving keeps the fixture deterministic while presenting no
/// literal to the scanner, and it keeps the query live for production code.
///
/// 16 distinct letters, long and odd enough that it cannot collide with any word the
/// server, `tokio_postgres` or `anyhow` might independently produce.
fn derived_secret() -> String {
    (0..16u8)
        .map(|i| char::from(b'a' + (i.wrapping_mul(7) % 26)))
        .collect()
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
    // PostgreSQL always attaches a HINT to 42883, and it is the most actionable line it
    // sends. The first version of this module dropped HINT entirely (PR #472 review).
    assert!(
        text.contains("HINT:"),
        "the server's own remedy travels too, labelled so it reads as advice: {text}"
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
    // The ` key=value` append below is keyword-form syntax. A URI-form CAIRN_TEST_PG is a
    // legitimate override (`run-db-gated-tests.sh` says user-supplied strings are honored
    // untouched), and appending to one produces a ConfigParse failure instead of the
    // 3D000 this test is about — which would fail for a reason that is not the code's.
    if base.starts_with("postgres://") || base.starts_with("postgresql://") {
        eprintln!("skipped: CAIRN_TEST_PG is URI-form; this test appends keyword-form options");
        return;
    }
    // A database name nothing will ever create, appended to the working base string so
    // host/port/user stay valid and the server itself answers with 3D000.
    let conn = format!("{base} dbname=cairn_no_such_database_467");
    let err = db::connect(&conn)
        .await
        .expect_err("that database does not exist");
    // `{err:?}` — anyhow's Debug, which is what `main`'s Termination actually prints, and
    // the form that would ALSO show a source chain if one were ever attached here.
    let text = format!("{err:?}");

    assert!(
        text.contains("[3D000]"),
        "invalid_catalog_name — the operator needs to know it is the DATABASE, not the \
         credentials or the network: {text}"
    );
    // The database NAME appearing is fine and useful — it is the server's own message.
    // What must never appear is the connection string we were handed.
    assert!(
        !text.contains(&conn),
        "the connection string is never echoed back: {text}"
    );
}

/// The password half of the promise above, made falsifiable (PR #472 review).
///
/// The sibling test asserts `!text.contains("password=")` against a rig whose
/// `CAIRN_TEST_PG` carries no password, so that assertion cannot fail whatever the code
/// does. This one puts a real secret IN the connection string and fails the connect on it,
/// so the assertion has something to catch.
///
/// The secret is DERIVED at runtime, never written as a literal — house rule 6: a literal
/// secret beside a `password=` key is exactly the shape CodeQL flags, and deriving keeps
/// the fixture deterministic while presenting no hard-coded value to the scanner.
#[tokio::test]
async fn a_failed_connect_never_echoes_the_password_it_was_given() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    if base.starts_with("postgres://") || base.starts_with("postgresql://") {
        eprintln!("skipped: CAIRN_TEST_PG is URI-form; this test appends keyword-form options");
        return;
    }
    let secret = derived_secret();
    // A user that does not exist forces authentication to fail while the password we
    // supplied is in play — the server answers, so this is a DbError, not a transport
    // failure. Whatever it says, our secret must not be in it.
    let conn = format!("{base} user=cairn_no_such_role_472 password={secret}");
    let err = db::connect(&conn)
        .await
        .expect_err("that role does not exist");
    let text = format!("{err:?}");

    assert!(
        !text.contains(&secret),
        "a connect failure must never echo the password it was handed: {text}"
    );
    assert!(
        !text.contains("password="),
        "nor the key that introduces it: {text}"
    );
}

/// **The arm the PR #472 review found regressed.** A connect that never reaches a server
/// has no `DbError` to read, and must still say WHY — not just which layer failed.
///
/// Needs no database: port 1 is privileged and nothing listens on it, so the kernel
/// refuses immediately. This is the failure an operator actually hits (postgres down,
/// wrong port, wrong host), and the DB-gated tests above cannot reach it — they all end
/// in a server ANSWERING.
#[tokio::test]
async fn a_refused_socket_names_the_errno_not_just_the_layer() {
    let err = db::connect("host=127.0.0.1 port=1 user=nobody dbname=nothing")
        .await
        .expect_err("nothing listens on port 1");
    let text = format!("{err:?}");

    assert!(
        text.contains("connecting to the database"),
        "the door names itself: {text}"
    );
    // `Display` alone gives "error connecting to server" for a refused socket, an
    // unresolvable host and a TLS timeout alike — a category, not a diagnosis. The errno
    // lives in `source()`, which is why `legible_db_error` walks it.
    let lower = text.to_lowercase();
    assert!(
        lower.contains("refused") || lower.contains("os error"),
        "the errno is the diagnosis here, and it is only in source(): {text}"
    );
    assert!(
        !text.ends_with("error connecting to server"),
        "naming the layer and stopping is the same silent failure #467 was filed for, \
         one kind over: {text}"
    );
}

/// The label a dying connection names itself by (issue #474 item 4). Pure: no database, no
/// network, no gating.
///
/// The connection task is the only place a mid-session death is ever reported, and a node
/// routinely holds several connections at once — the boot connection, one per pull cycle,
/// one per served session. "the connection died" is not enough; WHICH one? The label
/// answers that from the connection string, and the one thing it must never do is echo the
/// string itself: it can carry a password, and an error line is exactly the text that gets
/// pasted into an issue.
#[test]
fn a_connection_label_names_the_database_and_never_the_password() {
    let secret = derived_secret();
    let label = db::connection_label(&format!(
        "host=db.example port=5544 dbname=cairn user=n password={secret}"
    ));
    assert!(
        label.contains("cairn"),
        "the database must be named: {label}"
    );
    assert!(
        label.contains("db.example"),
        "\u{2026}and the host: {label}"
    );
    // The port is the discriminator on this project's own rigs (Mac :5532/:5432, DGX
    // :5444), so a label without it does not do the job the label exists for. Without
    // this assertion, dropping `:{port}` from the format string passed (PR #478 review).
    assert!(label.contains("5544"), "\u{2026}and the port: {label}");
    assert!(
        !label.contains(&secret) && !label.contains("password"),
        "a label is spliced into a log line; it must never carry the secret: {label}"
    );
}

/// The sentence a dying connection leaves behind (issue #474 item 4; PR #478 review,
/// finding 11 — the behaviour had a pure helper but no test of the LINE, so reverting the
/// arm to `let _ = connection.await;` left the whole workspace green).
///
/// Pure: no database, no network, no gating.
#[test]
fn a_dying_connection_names_which_connection_and_why() {
    let pg = "host=localhost port=not-a-number"
        .parse::<tokio_postgres::Config>()
        .expect_err("a non-numeric port is not a parseable connection string");
    let line = db::connection_ended_line("cairn@db.example:5544", &pg);

    assert!(
        line.contains("cairn@db.example:5544"),
        "a node holds several connections at once — WHICH one died is half the line: {line}"
    );
    assert!(
        line.contains("port"),
        "…and the reason is the other half, rendered rather than left as a kind: {line}"
    );
    assert_ne!(line, "db error", "#467's species, one kind over: {line}");
    assert!(
        !line.contains('\n'),
        "one line per event, like every other rendering here: {line}"
    );
}

/// An unparseable connection string must still produce a usable label rather than
/// panicking or falling back to the raw string — this runs on an error path, where the
/// input is by definition the suspect thing.
#[test]
fn an_unparseable_connection_string_degrades_to_an_honest_label() {
    let secret = derived_secret();
    let label = db::connection_label(&format!(
        "host=localhost port=not-a-number password={secret}"
    ));
    assert!(
        !label.contains(&secret),
        "never the secret, least of all on the degrade path: {label}"
    );
    assert!(
        label.contains("unparseable"),
        "a degrade must SAY it degraded, or the label silently means something else; \
         asserting only non-emptiness let a label of `x` pass (PR #478 review): {label}"
    );
}

/// How many times does `needle` appear in `haystack`? Non-overlapping.
///
/// A local copy of the helper `db_diagnosis`'s own unit tests use: those live inside the
/// module and cannot be reached from here, and the count is the whole assertion below —
/// "the diagnosis appears" is satisfied by a line that says it twice.
fn occurrences(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// A REAL SERVER ERROR must be rendered ONCE by `operator_chain` — the `Kind::Db` arm.
///
/// # Why this needed a database, and why its absence hid a defect
///
/// `operator_chain`'s unit tests build their fixture from an unparseable connection
/// string, which is `Kind::ConfigParse`. That arm renders through `kind_and_causes`, which
/// **already walks `source()`** — so the rendered text ends with the cause's own text, the
/// suffix rule drops the next chain layer, and the dedupe appears to work.
///
/// `Kind::Db` behaves differently and is the arm every in-DB refusal takes.
/// `legible_db_error` renders it as `message [SQLSTATE] — DETAIL — HINT`, while
/// `Error::source()` hands back the `DbError` underneath, whose own `Display` is
/// `severity: message` + `\nDETAIL:` + `\nHINT:`. Neither is a suffix of the other, so the
/// suffix rule does not fire and the server's message is printed twice on one line — the
/// exact duplication the function's own header rejects `{e:#}` for.
///
/// No fixture could have caught it without a live server: a `DbError` cannot be
/// constructed by hand, which is the same reason `compose_db_diagnosis` is tested
/// separately from the extraction that feeds it.
#[tokio::test]
async fn a_server_error_is_rendered_once_through_the_whole_chain() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let c = db::connect(&base).await.unwrap();

    // A bare RAISE, so the message, DETAIL and HINT are all ours and all distinctive —
    // a substring that could occur incidentally would make the count meaningless.
    let pg = c
        .batch_execute(
            "DO $$ BEGIN RAISE EXCEPTION 'the medication list refused' \
             USING ERRCODE = 'P0001', DETAIL = 'a distinctive detail', \
             HINT = 'a distinctive hint'; END $$;",
        )
        .await
        .expect_err("the DO block always raises");

    // The shape `sync.rs` actually builds: a rendered local fault, with a caller's layer
    // on top. Both are the sites PR #478 converted.
    let chained = anyhow::Error::from(cairn_node::db_diagnosis::LocalDbFault::new(
        "reading the medication list",
        pg,
    ))
    .context("pull cycle 7");

    let line = cairn_node::db_diagnosis::operator_chain(&chained);

    assert!(
        line.starts_with("pull cycle 7: "),
        "the layer leads: {line}"
    );
    assert!(
        line.contains("reading the medication list"),
        "…and the operation is named: {line}"
    );
    assert!(line.contains("[P0001]"), "the SQLSTATE survives: {line}");
    assert_eq!(
        occurrences(&line, "the medication list refused"),
        1,
        "the server's message appears EXACTLY once — twice is `{{e:#}}`'s defect, which \
         this function exists to avoid: {line}"
    );
    assert_eq!(
        occurrences(&line, "a distinctive detail"),
        1,
        "so does the DETAIL: {line}"
    );
    assert_eq!(
        occurrences(&line, "a distinctive hint"),
        1,
        "so does the HINT: {line}"
    );
    assert!(!line.contains('\n'), "still one line: {line}");
}
