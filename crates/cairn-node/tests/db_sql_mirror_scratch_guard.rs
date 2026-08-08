//! #169 — a `db/tests/*.sql` mirror runs only against a database marked disposable.
//!
//! THE FAILURE THIS CLOSES. Every DB-gated Rust suite truncates the write tables at the *start* of
//! each test (`reset()` in `recall_epoch.rs` and its siblings) but not after the last one, so a
//! finished `cargo test` leaves `event_log`/`actor_event` rows behind in the shared `cairn_test`
//! database. That residue is inert for the Rust suites themselves — they self-serialize via
//! `db::test_serial_guard` and each truncates before it runs — but a mirror run BY HAND against that
//! same database then collides with it, and the collision surfaces as a confusing failure inside an
//! unrelated floor guard (this is how a spurious #152-guard collision appeared during the #166 work).
//!
//! WHY THE GUARD IS AN ALLOW-LIST. Refusing a list of known-bad database *names* would close that
//! nuisance and leave the far worse case open: eight mirrors have no transaction wrapper and
//! therefore COMMIT, and `017` even drops constraints and replays a migration, so a mistyped `-d`
//! aimed at a real node mutilates a clinical record. The dangerous target is precisely the one
//! nobody thought to name, so the polarity is inverted — a mirror refuses everywhere unless the
//! database carries an explicit marker table saying it is a throwaway. Both sanctioned runners
//! (`scripts/run-db-sql-tests.sh`, `db/bench/run_b5.sh`) stamp that marker on the disposable
//! database they use; every other database in the cluster is refused by default.
//!
//! Issue #169's own suggestion — open each mirror with an idempotent `TRUNCATE` — was declined for
//! the same reason: `TRUNCATE` does not fire the row-level append-only triggers, so it would put the
//! destructive case *inside* seventeen tracked files rather than guarding against it.
//!
//! WHAT THIS FILE PINS. Three source-level guards (no database needed, so they run in every plain
//! `cargo test`): every mirror carries the preamble, as a real directive rather than prose; nothing
//! that writes precedes it; and the preamble stays pure SQL. Plus one DB-gated test that executes
//! the REAL guard file and exercises BOTH arms — refusal without the marker, silence with it.
use cairn_node::db;
use std::fs;
use std::path::PathBuf;

/// The shared preamble's filename. The leading underscore is load-bearing: the runner globs
/// `db/tests/[0-9]*.sql`, so an underscore-prefixed file is a preamble and never itself a test.
const GUARD_FILE: &str = "_scratch_database_guard.sql";

/// The exact line each mirror carries. `\ir` is psql's *include-relative* — resolved against the
/// including file's directory rather than the caller's CWD — so the include itself survives being
/// run from anywhere (a mirror may still carry its own CWD-relative `\i` lines, as `017` does).
const INCLUDE_LINE: &str = r"\ir _scratch_database_guard.sql";

/// psql's "stop at the first error". A raise only *stops* a script when this is already on.
const ON_ERROR_STOP: &str = r"\set ON_ERROR_STOP on";

/// The marker table whose presence makes a database eligible for the mirrors. Created by the
/// sanctioned runners; deliberately absent everywhere else, `cairn_test` included.
const MARKER_TABLE: &str = "cairn_scratch_database";

/// Repo-root `db/tests/` directory. `CARGO_MANIFEST_DIR` is `crates/cairn-node`; `db/` is two
/// levels up. Same idiom as `twin_dispatch_single_source.rs`'s `db_dir()`.
fn mirrors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db/tests")
        .canonicalize()
        .expect("db/tests/ dir")
}

/// Every file the runner treats as a test, i.e. `db/tests/[0-9]*.sql`, as (filename, contents).
///
/// Mirrors the runner's glob exactly, so this guard binds precisely the set of files that actually
/// execute — nothing more (the preamble itself is skipped by the leading-digit rule) and nothing
/// less (a mirror added tomorrow is picked up with no edit here).
fn mirror_files() -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = fs::read_dir(mirrors_dir())
        .expect("read db/tests/")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("sql"))
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with(|c: char| c.is_ascii_digit())
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&path).expect("read mirror"))
        })
        .collect();
    found.sort();
    found
}

/// The real guard file's contents — the same bytes psql includes and the DB-gated test executes.
fn guard_sql() -> String {
    fs::read_to_string(mirrors_dir().join(GUARD_FILE))
        .expect("read db/tests/_scratch_database_guard.sql")
}

/// Where the include sits in a mirror, as a line index.
///
/// Matched as a whole trimmed LINE, never as a substring of the file: a substring test would be
/// satisfied by the directive appearing inside a comment — and the guard file's own "HOW TO USE IT"
/// block shows exactly that line in exactly that position, so a contributor copying the wrong half
/// of it is an ordinary mistake, not an adversarial one.
fn include_line_index(sql: &str) -> Option<usize> {
    sql.lines().position(|line| line.trim() == INCLUDE_LINE)
}

/// Anti-drift: a mirror added later without the preamble fails `cargo test`, so the convention
/// cannot decay back into "documented somewhere" the way the pre-#212 one did.
#[test]
fn every_sql_mirror_includes_the_scratch_database_guard() {
    let mirrors = mirror_files();

    // Anti-vacuity: an empty or mis-globbed directory would make the loop below pass while
    // checking nothing at all. Seventeen mirrors exist at write time; assert we saw a plausible
    // number rather than a hard count, so adding a mirror does not need an edit here.
    assert!(
        mirrors.len() >= 10,
        "expected the db/tests/ mirrors to be discovered, found {} — has the directory moved?",
        mirrors.len()
    );

    let missing: Vec<&str> = mirrors
        .iter()
        .filter(|(_, sql)| include_line_index(sql).is_none())
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "these db/tests mirrors do not include the scratch-database guard (#169): {missing:?}\n\
         Add `{INCLUDE_LINE}` as its own line near the top of each, under `{ON_ERROR_STOP}`."
    );
}

/// The preamble must come before anything it is meant to prevent, and must be able to stop the run.
///
/// Two ways a present-but-useless preamble would otherwise sneak through, both of them plausible
/// drift rather than sabotage:
///
///   * appended at the BOTTOM of a mirror by someone who did not read the file top to bottom — the
///     fixtures have already run by then, so the guard reports on a database it failed to protect;
///   * included without `ON_ERROR_STOP`, in which case psql prints the refusal and carries on with
///     the next statement. The sanctioned runners pass `-v ON_ERROR_STOP=1`, but the case this
///     mechanism exists for is the *hand* run, where the only setting that applies is the file's own.
#[test]
fn the_guard_runs_before_anything_else_and_can_stop_the_script() {
    let mirrors = mirror_files();
    let mut problems: Vec<String> = Vec::new();

    for (name, sql) in &mirrors {
        let Some(include_at) = include_line_index(sql) else {
            continue; // Reported by every_sql_mirror_includes_the_scratch_database_guard.
        };
        let lines: Vec<&str> = sql.lines().map(str::trim).collect();

        if !lines[..include_at].contains(&ON_ERROR_STOP) {
            problems.push(format!(
                "{name}: `{ON_ERROR_STOP}` must appear above the include, or a hand run prints the \
                 refusal and then proceeds anyway"
            ));
        }

        // Everything above the include must be inert: blank lines, `--` comments, and the psql
        // `\set`/`\ir` directives that configure the run. Anything else is a statement that would
        // reach the database before the guard could refuse it.
        if let Some(offending) = lines[..include_at].iter().find(|line| {
            !line.is_empty()
                && !line.starts_with("--")
                && !line.starts_with(r"\set")
                && !line.starts_with(r"\ir")
        }) {
            problems.push(format!(
                "{name}: `{offending}` executes before the guard include — move the include above it"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "db/tests mirrors with an ineffective scratch-database guard (#169):\n  {}",
        problems.join("\n  ")
    );
}

/// The preamble must stay executable by BOTH psql (via `\ir`) and a plain SQL client.
///
/// A psql backslash meta-command is a client-side directive, not SQL — it would make the file
/// unusable through `tokio_postgres`, and the DB-gated test below deliberately runs the REAL file
/// rather than a copy of its text, so that the thing pinned is the thing shipped.
#[test]
fn the_guard_file_is_pure_sql() {
    let sql = guard_sql();
    let offending: Vec<&str> = sql
        .lines()
        .filter(|line| line.trim_start().starts_with('\\'))
        .collect();
    assert!(
        offending.is_empty(),
        "db/tests/{GUARD_FILE} must contain no psql meta-commands (it is executed as plain SQL by \
         the DB-gated test); found: {offending:?}"
    );
}

/// The guard actually fires — behaviour, not merely the presence of some text.
///
/// BOTH arms are exercised here against the real guard file, and neither depends on where
/// `CAIRN_TEST_PG` happens to point: the marker is what decides, so the test creates it inside a
/// transaction it rolls back. `CAIRN_TEST_PG` names a shared rig database, which must never carry
/// the marker — that it does not is asserted up front, so a misconfigured rig fails legibly rather
/// than quietly turning the refusal arm into a no-op.
#[tokio::test]
async fn the_guard_admits_only_a_marked_scratch_database() {
    let Some(conn) = std::env::var("CAIRN_TEST_PG").ok() else {
        return; // DB-gated, self-skipping — same convention as every other suite here.
    };
    // No schema load and no `test_serial_guard`: the guard file reads one catalog entry and writes
    // nothing, so it neither needs the migrations nor can interfere with a concurrent suite. The
    // marker below lives and dies inside a rolled-back transaction, invisible to anyone else.
    let client = db::connect(&conn).await.unwrap();
    let guard = guard_sql();

    let marker_present: bool = client
        .query_one(
            "SELECT to_regclass('public.' || $1) IS NOT NULL",
            &[&MARKER_TABLE],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !marker_present,
        "CAIRN_TEST_PG points at a database carrying the {MARKER_TABLE} marker: it is a SHARED rig \
         database that mirrors must refuse (#169), so the marker does not belong there"
    );

    // Arm 1 — unmarked: refuse, and say something a human can act on.
    let err = client.batch_execute(&guard).await.expect_err(
        "the guard must REFUSE an unmarked database — residue from finished cargo test runs lives \
         here, and an unmarked database might equally be a real node (#169)",
    );
    let message = err
        .as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| err.to_string());
    assert!(
        message.contains("run-db-sql-tests.sh"),
        "the refusal must name the sanctioned runner so the message is actionable; got: {message}"
    );
    assert!(
        message.contains(MARKER_TABLE),
        "the refusal must name the marker it looked for; got: {message}"
    );

    // Arm 2 — marked: stay silent, or the sanctioned runners could never run a mirror at all.
    client.batch_execute("BEGIN").await.unwrap();
    client
        .batch_execute(&format!("CREATE TABLE {MARKER_TABLE} ()"))
        .await
        .unwrap();
    let admitted = client.batch_execute(&guard).await;
    client.batch_execute("ROLLBACK").await.unwrap();
    admitted.unwrap_or_else(|e| panic!("the guard must ADMIT a marked scratch database: {e}"));
}
