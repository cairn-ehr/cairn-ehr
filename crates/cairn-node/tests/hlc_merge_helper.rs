//! Issue #227 — the A3 HLC merge lives in ONE guarded helper, not one copy per door.
//!
//! ## What the A3 merge is
//!
//! Every door that ADMITS an event someone else authored must drag this node's Hybrid
//! Logical Clock forward past it, so the clock never falls behind anything in our own
//! log (the "A3" invariant, §3.6 / ADR-0003). The merge itself is three lines of
//! monotone arithmetic — take the greater wall; on a tie take the greater counter; on a
//! strictly greater wall adopt the incoming counter — and it was copied verbatim into
//! five places: three arms of `apply_remote_node_event` (db/007), `restore_node_event`
//! (db/009), and `apply_remote_event` (db/020).
//!
//! ## Why the copies were the defect
//!
//! A future edit that fixes one copy and misses another gives two admission doors
//! DIFFERENT clock semantics — silent divergence between the node plane and the
//! clinical plane, discoverable only as unexplained ordering weirdness months later.
//! That is the same class of drift the twin-registry row-count hit in PR #182.
//!
//! ## What this suite pins
//!
//! 1. **Source-level (no DB): exactly one migration may carry the merge.** This is the
//!    guard that actually prevents a SIXTH copy from being pasted in later — the reason
//!    the de-duplication holds over time rather than just today. Modelled on the #173
//!    `twin_dispatch_single_source` guard.
//! 2. **The helper is not a clock-ratchet door.** It writes `hlc_state`, so the
//!    unprivileged runtime role must not be able to call it — otherwise any runtime
//!    connection could ratchet the clock forward WITHOUT passing the drift ceiling that
//!    each door applies before merging (issues #102/#193). Two independent barriers are
//!    checked, because the helper is deliberately invoker-rights rather than SECURITY
//!    DEFINER: no EXECUTE grant, and no UPDATE on `hlc_state` even if one were granted
//!    by mistake.
//! 3. **NULL arguments fail closed.** Extracting a helper introduces a call signature,
//!    and a signature admits arguments the inline block could never see. Left
//!    unguarded, a NULL wall would be swallowed silently (`GREATEST` ignores NULLs and
//!    `NULL > x` is NULL, so the whole merge degrades to a no-op that LOOKS like it
//!    worked). Fail closed and say so instead.
//! 4. **The merge is monotone.** The property every caller depends on: an older event
//!    can never drag the clock BACKWARDS, no matter what a peer asserts.
//! 5. **Every door still CALLS the helper.** The mirror image of guard 1, and the one
//!    this suite originally missed: forbidding a new copy does nothing about a call
//!    site quietly disappearing.
//!
//! ## What the door suites do and do not cover
//!
//! `hlc_drift.rs`, `restore.rs`, `apply_remote_event.rs` and `cairn-sync`'s
//! `clinical_pull.rs` cover each door's ADMISSION behaviour — which event is accepted,
//! which is refused, and each door's drift ceiling — and they stay green across this
//! refactor, which is the regression proof for that behaviour.
//!
//! They do NOT cover the clock advance at every site. A positive "`hlc_state` moved
//! forward after admission" assertion exists for only two of the five call sites:
//! db/007's enroll arm (`hlc_drift.rs`) and db/020 (`apply_remote_event.rs`). The
//! supersede arm, the peer/revoke arm and `restore_node_event` have none — dropping
//! their merge leaves the entire tree green. That is precisely why guard 5 exists, and
//! why it is a source-level count rather than a behavioural assertion.
//!
//! DB-backed cases use real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard` (shared-DB pattern).
use cairn_node::db;
use std::fs;
use std::path::PathBuf;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Repo-root `db/` directory. `CARGO_MANIFEST_DIR` is `crates/cairn-node`; `db/` is two
/// levels up.
fn db_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../db")
        .canonicalize()
        .expect("db/ dir")
}

// ---------------------------------------------------------------------------
// 1. Source-level: one merge, one migration.
// ---------------------------------------------------------------------------

/// `GREATEST(hlc_wall,` is the fingerprint of the A3 merge and of nothing else in the
/// tree. The local clock TICK (`node_hlc_tick`, db/007) writes `hlc_state` too, but it
/// assigns already-computed locals rather than merging, so it does not match — the
/// needle is specific to "merge some OTHER clock into ours".
#[test]
fn the_a3_merge_is_written_in_exactly_one_migration() {
    let needle = "GREATEST(hlc_wall,";
    let mut carrying: Vec<String> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        if sql.contains(needle) {
            carrying.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    carrying.sort();
    assert_eq!(
        carrying,
        vec!["001_envelope.sql".to_string()],
        "the A3 HLC merge must live ONLY in cairn_node_hlc_merge (db/001, issue #227); \
         every door PERFORMs the helper instead of pasting the block. Found in: {carrying:?}"
    );
}

/// The helper must be declared once, and in `db/001` specifically — NOT in `db/007`
/// where `hlc_state` is idempotently restated.
///
/// This placement is load-bearing and non-obvious, so it gets its own guard: cairn-sync
/// loads a SUBSET of the migrations (db/001 + db/020 + others, but NOT db/007). PL/pgSQL
/// resolves a function call at first EXECUTION, so a helper declared in db/007 and called
/// from the db/020 clinical door would let cairn-sync's schema load cleanly and then fail
/// on its first admitted event — a first-write outage, exactly the late-binding trap
/// issue #198 was filed for.
#[test]
fn the_helper_is_declared_in_db001_so_the_sync_subset_can_reach_it() {
    let needle = "CREATE OR REPLACE FUNCTION cairn_node_hlc_merge(";
    let mut declaring: Vec<String> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        if sql.contains(needle) {
            declaring.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    declaring.sort();
    assert_eq!(
        declaring,
        vec!["001_envelope.sql".to_string()],
        "cairn_node_hlc_merge must be declared ONLY in db/001 — cairn-sync's SCHEMA subset \
         omits db/007, and PL/pgSQL late binding turns a misplaced declaration into a \
         first-write outage (#198). Found in: {declaring:?}"
    );
}

/// Every door that admits someone else's event must still CALL the merge — the mirror
/// image of the guard above.
///
/// The two guards above forbid a sixth COPY of the merge from re-growing. Neither
/// notices the opposite failure: a call site silently VANISHING. Both still pass with
/// every `PERFORM cairn_node_hlc_merge(...)` deleted, including all five at once. The
/// de-duplication itself made that failure easier to miss — removing the eight-line
/// block was conspicuous in review, removing one `PERFORM` line is not.
///
/// A source-level count is the right tool here because behavioural coverage does not
/// exist for three of the five sites (see the header): the supersede arm, the
/// peer/revoke arm and `restore_node_event` have no assertion anywhere in the tree that
/// the clock advanced, so their merge could be dropped with the whole suite green.
///
/// Concretely, what that would cost — take `restore_node_event`. A node restored from a
/// sneakernet medium whose events carry honest forward skew (walls inside the 24h
/// ceiling, so all admitted) would leave `hlc_state` at its fresh-database 0.
/// `node_hlc_tick` then returns `GREATEST(now_ms, 0)` = now, which is BELOW the restored
/// events' walls — so the first node event this node authors sorts causally BEFORE
/// history it already holds. An A3 violation inside an append-only log, surfacing only
/// as unexplained ordering weirdness much later: the exact defect class issue #227 was
/// filed to make impossible.
///
/// Shares the whitespace-sensitivity limitation of the #173 guard it is modelled on: a
/// call written `PERFORM  cairn_node_hlc_merge(` or `SELECT cairn_node_hlc_merge(` would
/// not be counted. That direction fails CLOSED (the test goes red and a human looks), so
/// it is the safe way for the needle to be wrong.
#[test]
fn every_door_still_calls_the_helper() {
    let needle = "PERFORM cairn_node_hlc_merge(";
    // (migration, how many of its arms merge the clock)
    let want: Vec<(String, usize)> = [
        ("007_node_federation.sql", 3), // apply_remote_node_event: enroll, supersede, peer/revoke
        ("009_node_supersede_and_restore.sql", 1), // restore_node_event
        ("020_apply_remote_event.sql", 1), // the clinical door, passing its clamped wall
    ]
    .iter()
    .map(|(f, n)| (f.to_string(), *n))
    .collect();

    let mut got: Vec<(String, usize)> = Vec::new();
    for entry in fs::read_dir(db_dir()).expect("read db/") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sql") {
            continue;
        }
        let sql = fs::read_to_string(&path).expect("read sql");
        let calls = sql.matches(needle).count();
        if calls > 0 {
            got.push((
                path.file_name().unwrap().to_string_lossy().into_owned(),
                calls,
            ));
        }
    }
    got.sort();
    assert_eq!(
        got, want,
        "every admission door must still PERFORM cairn_node_hlc_merge — a dropped call \
         leaves the clock behind events this node has already admitted (A3, issue #227)"
    );
}

// ---------------------------------------------------------------------------
// 2. The helper is not a clock-ratchet door.
// ---------------------------------------------------------------------------

/// The runtime role must reach the merge ONLY through an admission door, because each
/// door applies its drift ceiling BEFORE merging. A directly-callable merge would be a
/// bypass of that ceiling: a hostile or broken runtime connection could ratchet
/// `hlc_state` into the far future, and every event this node subsequently authors would
/// be refused by every peer's ceiling — the node wedges itself out of the federation.
///
/// Two independent barriers, both asserted, because defence in depth is the whole point
/// of choosing invoker rights over SECURITY DEFINER here:
///   * no EXECUTE for `cairn_node` (the grant floor), and
///   * no UPDATE on `hlc_state` for `cairn_node` — so even if a later edit mistakenly
///     GRANTed EXECUTE, the invoker-rights body would still fail. A SECURITY DEFINER
///     helper would have no such second barrier.
#[tokio::test]
async fn the_helper_is_not_callable_by_the_runtime_role() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let (can_execute, can_update): (bool, bool) = {
        let r = c
            .query_one(
                "SELECT has_function_privilege('cairn_node', \
                        'cairn_node_hlc_merge(bigint,integer)', 'EXECUTE'), \
                        has_table_privilege('cairn_node', 'hlc_state', 'UPDATE')",
                &[],
            )
            .await
            .unwrap();
        (r.get(0), r.get(1))
    };
    assert!(
        !can_execute,
        "cairn_node must NOT hold EXECUTE on cairn_node_hlc_merge — the doors call it as \
         the migration-defining owner, and a callable merge bypasses the drift ceiling"
    );
    assert!(
        !can_update,
        "cairn_node must NOT hold UPDATE on hlc_state — this is the second barrier that \
         makes an accidental EXECUTE grant harmless (invoker rights, not SECURITY DEFINER)"
    );
}

// ---------------------------------------------------------------------------
// 3. NULL arguments fail closed.
// ---------------------------------------------------------------------------

/// A NULL wall or counter is refused with a legible message rather than silently
/// no-op'ing.
///
/// Unreachable through any door today — `node_event.hlc_wall` and `event_log.hlc_wall`
/// are both NOT NULL and are inserted BEFORE the merge runs, so a body carrying no
/// `hlc` is rejected by the column constraint first. The guard exists because the
/// helper's signature is now a public-ish surface within the schema: the next caller to
/// be written does not inherit that accident of ordering, and a silent no-op merge is
/// precisely the kind of "looked like it worked" failure this project refuses.
#[tokio::test]
async fn the_helper_refuses_null_arguments() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    for (wall, counter, case) in [
        ("NULL::bigint", "0::integer", "a NULL wall"),
        ("0::bigint", "NULL::integer", "a NULL counter"),
    ] {
        let err = c
            .execute(
                &format!("SELECT cairn_node_hlc_merge({wall}, {counter})"),
                &[],
            )
            .await
            .expect_err(&format!("{case} must be refused, not silently ignored"));
        let msg = err
            .as_db_error()
            .map(|e| e.message().to_string())
            .unwrap_or_default();
        // Both halves matter: the message must name the helper (so a caller can find
        // it) AND the reason. Checking only the name would be satisfied by Postgres's
        // own "function cairn_node_hlc_merge(…) does not exist", i.e. it would pass
        // before the helper is written at all.
        assert!(
            msg.contains("cairn_node_hlc_merge") && msg.contains("must not be NULL"),
            "the refusal must name the helper and the reason; {case} gave: {msg}"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. The merge is monotone.
// ---------------------------------------------------------------------------

/// The one property every caller leans on: admitting an OLDER event never drags the
/// clock backwards, and a tie resolves by the greater counter.
///
/// The four cases together are the whole truth table of the merge:
///   * strictly older wall            → nothing moves (not even the counter)
///   * equal wall, lower counter      → nothing moves
///   * equal wall, higher counter     → counter advances, wall stays
///   * strictly newer wall            → wall advances and ADOPTS the incoming counter
///     (it does not keep the local one — the incoming counter is only meaningful
///     relative to its own wall)
#[tokio::test]
async fn the_merge_never_moves_the_clock_backwards() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // (incoming wall, incoming counter, expected wall, expected counter, why)
    let cases: &[(i64, i32, i64, i32, &str)] = &[
        (50, 9, 100, 5, "older wall: nothing moves"),
        (100, 3, 100, 5, "equal wall, lower counter: nothing moves"),
        (100, 7, 100, 7, "equal wall, higher counter: it rises"),
        (200, 2, 200, 2, "newer wall: adopts the incoming counter"),
    ];

    for (wall, counter, want_wall, want_counter, why) in cases {
        // Re-seed before each case so the cases are independent and order-insensitive.
        c.batch_execute("UPDATE hlc_state SET hlc_wall = 100, hlc_counter = 5 WHERE id")
            .await
            .unwrap();
        c.execute("SELECT cairn_node_hlc_merge($1, $2)", &[wall, counter])
            .await
            .unwrap();
        let (got_wall, got_counter): (i64, i32) = {
            let r = c
                .query_one("SELECT hlc_wall, hlc_counter FROM hlc_state WHERE id", &[])
                .await
                .unwrap();
            (r.get(0), r.get(1))
        };
        assert_eq!(
            (got_wall, got_counter),
            (*want_wall, *want_counter),
            "merging ({wall}, {counter}) into (100, 5): {why}"
        );
    }

    // Leave the shared singleton at its zero baseline rather than at this suite's last
    // case (the PR #285 convention `status.rs` follows). Harmless either way — the walls
    // above are 1970 timestamps, so `node_hlc_tick` self-corrects on the next tick — but
    // suites that share a database should not hand each other residue.
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0 WHERE id")
        .await
        .unwrap();
}
