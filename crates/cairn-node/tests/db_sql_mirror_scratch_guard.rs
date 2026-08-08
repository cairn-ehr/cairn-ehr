//! #169 — the SQL mirrors under `db/tests/` must refuse to run against a shared rig database.
//!
//! THE FAILURE THIS CLOSES. Every DB-gated Rust suite truncates the write tables at the *start*
//! of each test (`reset()` in `recall_epoch.rs` and its siblings) but not after the last one, so a
//! finished `cargo test` leaves `event_log`/`actor_event` rows behind in the shared `cairn_test`
//! database. That residue is inert for the Rust suites themselves — they self-serialize via
//! `db::test_serial_guard` and each truncates before it runs — but a `db/tests/*.sql` mirror
//! run BY HAND against that same database then collides with it, and the collision surfaces as a
//! confusing failure inside an unrelated floor guard (this is how a spurious #152-guard collision
//! appeared during the #166 work).
//!
//! THE FIX, AND WHY IT IS A GUARD RATHER THAN A `TRUNCATE`. The obvious repair — open every mirror
//! with an idempotent `TRUNCATE` — is wrong twice over. Eight of the mirrors have no transaction
//! wrapper, so theirs would *commit*; and, more seriously, `TRUNCATE` does not fire the row-level
//! append-only triggers, so seventeen tracked files each beginning with an unconditional
//! `TRUNCATE event_log, actor_event, patient_chart CASCADE` would be a loaded gun in the repo of an
//! append-only clinical record: one mistyped `-d` and the record is gone silently. Instead the
//! mirrors carry a shared preamble that REFUSES a `cairn_test*` target and names the sanctioned
//! runner — symmetric with the refusal `scripts/run-db-sql-tests.sh` already carries in the other
//! direction (it declines to DROP a `cairn_test*` database). The residue then never gets the chance
//! to bite, and the mistake fails loudly with an actionable message instead of quietly colliding.
//!
//! WHAT THIS FILE PINS. Two source-level guards (no database needed, so they run in every plain
//! `cargo test`): every mirror actually carries the preamble, and the preamble stays pure SQL. Plus
//! one DB-gated behavioural test that executes the real guard file and checks it fires. The
//! *passing* arm — the guard staying silent on a throwaway database — is exercised on every CI push
//! by `scripts/run-db-sql-tests.sh`, which runs all seventeen mirrors against `cairn_sqltest`.
use cairn_node::db;
use std::fs;
use std::path::PathBuf;

/// The shared preamble's filename. The leading underscore is load-bearing: the runner globs
/// `db/tests/[0-9]*.sql`, so an underscore-prefixed file is a preamble and never itself a test.
const GUARD_FILE: &str = "_scratch_database_guard.sql";

/// The exact line each mirror carries. `\ir` is psql's *include-relative* — resolved against the
/// including file's directory rather than the caller's CWD — so a mirror works whether it is run
/// from the repo root, from `db/`, or from anywhere else.
const INCLUDE_LINE: &str = r"\ir _scratch_database_guard.sql";

/// The database-name prefix the guard refuses: the shared rig databases (`cairn_test`,
/// `cairn_test2`, `cairn_test3`) that the Rust and matcher suites write residue into.
const SHARED_DB_PREFIX: &str = "cairn_test";

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
        .filter(|(_, sql)| !sql.contains(INCLUDE_LINE))
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "these db/tests mirrors do not include the scratch-database guard (#169): {missing:?}\n\
         Add `{INCLUDE_LINE}` near the top of each, before any statement that writes."
    );
}

/// A raised exception only *stops* a psql script when `ON_ERROR_STOP` is already on.
///
/// Without it psql reports the error and carries on with the next statement — so the guard would
/// print a warning and then let the mirror trample the shared database anyway, which is worse than
/// no guard at all because it looks like one. The sanctioned runner passes `-v ON_ERROR_STOP=1` on
/// the command line, but the case this whole mechanism exists for is precisely the *hand* run
/// (`psql -d cairn_test -f db/tests/004_actors_test.sql`), where the only setting that applies is
/// the one in the file. So each mirror must set it at or above its include line.
#[test]
fn on_error_stop_is_set_before_the_guard_runs() {
    const ON_ERROR_STOP: &str = r"\set ON_ERROR_STOP on";

    let mirrors = mirror_files();
    let unstopped: Vec<&str> = mirrors
        .iter()
        .filter(|(_, sql)| {
            let lines: Vec<&str> = sql.lines().map(str::trim).collect();
            match lines.iter().position(|line| *line == INCLUDE_LINE) {
                // A mirror missing the include is reported by the test above; not this one's job.
                None => false,
                Some(at) => !lines[..at].contains(&ON_ERROR_STOP),
            }
        })
        .map(|(name, _)| name.as_str())
        .collect();

    assert!(
        unstopped.is_empty(),
        "these db/tests mirrors include the guard without `{ON_ERROR_STOP}` above it, so a hand \
         run would print the refusal and then proceed anyway (#169): {unstopped:?}"
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
/// Which arm runs depends on where `CAIRN_TEST_PG` points, and BOTH are real assertions: against a
/// shared rig database (`cairn_test*`, what CI and the standard local rig use) the guard must raise
/// and name the runner; against anything else it must stay silent, because a guard that refused
/// every database would break the runner it exists to point people at.
#[tokio::test]
async fn the_guard_refuses_a_shared_rig_database() {
    let Some(conn) = std::env::var("CAIRN_TEST_PG").ok() else {
        return; // DB-gated, self-skipping — same convention as every other suite here.
    };
    // No schema load and no `test_serial_guard`: the guard file touches no table, so it neither
    // needs the migrations nor can it interfere with a concurrent suite.
    let client = db::connect(&conn).await.unwrap();
    let dbname: String = client
        .query_one("SELECT current_database()", &[])
        .await
        .unwrap()
        .get(0);

    let result = client.batch_execute(&guard_sql()).await;

    if dbname.starts_with(SHARED_DB_PREFIX) {
        let err = result.expect_err(
            "the guard must REFUSE a shared rig database — residue left by a finished cargo test \
             run lives here (#169)",
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
            message.contains(&dbname),
            "the refusal must name the database it refused; got: {message}"
        );
    } else {
        result.unwrap_or_else(|e| {
            panic!("the guard must stay SILENT on the throwaway database {dbname}: {e}")
        });
    }
}
