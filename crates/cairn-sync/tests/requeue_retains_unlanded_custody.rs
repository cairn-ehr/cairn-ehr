//! Issue #578 — `requeue` never counts a release it did not get.
//!
//! # The failure this file exists to catch
//!
//! A clinic's disk is gone. A restore brought the record back but could not complete its custody,
//! so every sealed event sits in `sync_quarantine` **with its wrapped DEK** — and the pen row is
//! now the only copy of that key in the world. The penned reason tells the operator, in these
//! words, what to do next:
//!
//! > The bytes AND the key are kept: fix the cause, then `cairn-sync requeue` to complete the
//! > restore without redoing it.
//!
//! The operator runs it. `do_requeue` unwraps the DEK, hands the plaintext to `apply_remote_event`,
//! and the door — finding no row in `node_unwrap_key`, because registering one is a *separate*
//! ceremony that sentence never named — takes its lenient arm: `RAISE WARNING`, admit the sealed
//! event **without custody**, return normally. Nothing in this tree polls the connection's message
//! stream, so that warning goes nowhere. `do_requeue` sees `Ok`, **deletes the pen row**, counts it
//! `released`, prints a success line and exits 0.
//!
//! The ciphertext is now permanently unopenable and the key is gone. The operator did exactly what
//! the software told them to do.
//!
//! # The rule these tests pin
//!
//! > A pen row that carries a wrapped DEK is released only when custody actually landed.
//!
//! Stated once, in `crates/cairn-sync/src/requeue.rs`, along with why it is uniform across all
//! three ways custody can fail to land. These tests prove it holds through the **shipped binary**,
//! and — the half that makes it a recovery rather than a stall — that fixing the cause and running
//! `requeue` again brings the body back.
//!
//! # What "the body opens" means here, and why nothing weaker will do
//!
//! Every arm that claims a record came back asserts that `event_clear.twin` reads back the dead
//! node's own text. Rows, counts and a green exit are all satisfiable by a door that admitted
//! well-formed ciphertext nobody can ever read — which is the entire defect. #568 established that
//! standard for the release path; #578 is what happens when the same question is not asked before
//! a DELETE.
//!
//! # Mutations run against these tests
//!
//! Each is listed with the arm that killed it. A test written against behaviour that already works
//! proves nothing until a deliberate break has been shown to fail it, and the break has to match
//! the shape of the claim (#568's own lesson, one file over).
//!
//! 1. **Delete the custody check entirely** (restore the pre-#578 `Ok(_)` arm) → arm 1 fails: the
//!    pen row is gone and its key with it.
//! 2. **Key the check on `dek.is_some()` instead of on the pen row's `dek_wrapped`** → arm 3 fails.
//!    That is the tempting narrow fix, and it misses the #580 case exactly: with no resolvable
//!    custody key `dek` is `None`, so the check never runs and the row holding the last copy is
//!    deleted anyway.
//! 3. **Drop the shred clause from `cairn_custody_landed`** → pinned one layer down, in
//!    `db/tests/052_restore_doors_test.sql`, where the door's behaviour belongs.
//! 4. **Release acked rows** (the pre-#581 listing) → arm 4 fails: a human's recorded exclusion is
//!    overridden.
//!
//! Skips unless `CAIRN_TEST_PG` is set. Serialized via cairn-node's `db::test_serial_guard` —
//! advisory locks are scoped PER DATABASE, not cluster-wide (#476) — because this file TRUNCATEs
//! tables every other DB-gated suite also uses.

#[path = "common/dead_node.rs"]
mod dead_node;
use dead_node::*;

use cairn_node::db;

// ---------------------------------------------------------------------------
// Arm 1 — the headline: an unregistered unwrap key keeps the row, and the
//         operator's fix brings the body back
// ---------------------------------------------------------------------------

/// **The #578 chain, end to end, and then its recovery.**
///
/// Phase one is the defect's own scenario: a node with no `node_unwrap_key` row runs `requeue`
/// against a pen holding the last copy of a record's key. The pen row must survive, byte for byte.
///
/// Phase two is what makes phase one worth having. The operator registers the unwrap key — the
/// ceremony the pen's remedy text never named — and runs `requeue` again. Now the row releases and
/// `event_clear.twin` reads back the dead node's text. Without phase two, "keeps the row" would be
/// indistinguishable from a command that had simply stopped working.
#[tokio::test]
async fn an_unregistered_unwrap_key_keeps_the_pen_row_and_a_second_run_opens_the_body() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    // THE RESTORED-NODE STATE. `dead_node_with_a_penned_record` registered this node's unwrap key
    // in order to author a sealed event at all; a node restored onto a fresh database has not. The
    // key FILE is untouched, so `resolve_at_startup` still yields a secret and the DEK still
    // unwraps — which is precisely why the defect was invisible: everything on the Rust side
    // succeeds, and only the door withholds.
    c.execute("DELETE FROM node_unwrap_key", &[])
        .await
        .expect("un-register this node's custody key");

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(
        ok,
        "requeue must not fail on a retained row\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);

    assert_eq!(m["custody_retained"], 1, "the row must be counted as kept");
    assert_eq!(m["released"], 0, "nothing may be reported released");
    assert_eq!(
        pen_rows(&c).await,
        1,
        "THE DEFECT: the pen row holding the last copy of this record's key was deleted"
    );

    // Byte-for-byte, not merely present: a row whose `dek_wrapped` had been cleared or rewritten
    // would satisfy a count and still have lost the key.
    let kept: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM sync_quarantine WHERE content_digest = $1",
            &[&record.digest],
        )
        .await
        .expect("read the retained row")
        .get(0);
    assert_eq!(kept, record.dek_wrapped, "the kept row must keep the key");

    // The operator is told which cause applied and what to do about it, on ONE line naming this
    // record — see `stderr_line_with`'s doc for why a whole-stream `contains` would not do.
    let line = stderr_line_with(&stderr, "KEPT in the pen");
    assert!(
        line.contains("establish-unwrap-key"),
        "the door-withheld cause must name the ceremony that fixes it: {line}"
    );

    // --- Phase two: the operator does what the line says, and runs it again. ---
    register_unwrap_key(&c, &sk).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(ok, "the second run must succeed\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the fixed node must release the row");
    assert_eq!(m["released_with_custody"], 1, "and it must carry its key");
    assert_eq!(m["custody_retained"], 0);
    assert_eq!(pen_rows(&c).await, 0, "the pen must now be empty");

    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "THE ASSERTION THAT MATTERS: the sealed body must open back to the dead node's own text"
    );
}

// ---------------------------------------------------------------------------
// Arm 2 — a row that never carried a key is unaffected
// ---------------------------------------------------------------------------

/// A keyless pen row still releases at once, and raises no custody alarm.
///
/// **This is the anti-vacuity arm, and it is not a formality.** Arm 1 would pass against a
/// `do_requeue` that had simply stopped releasing anything. It would also pass against one that
/// retained every row indiscriminately, which would strand every plaintext event a clinic has —
/// the modal pen row on the sync path, where there is no key to lose.
#[tokio::test]
async fn a_row_that_carried_no_key_releases_as_before() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, None).await;
    c.execute("DELETE FROM node_unwrap_key", &[])
        .await
        .expect("un-register this node's custody key");

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(ok, "requeue must succeed\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);

    assert_eq!(m["released"], 1, "a keyless row has no custody to lose");
    assert_eq!(m["custody_retained"], 0, "and must raise no custody alarm");
    assert_eq!(
        m["released_with_custody"], 0,
        "it carried no key, so it did not release with one"
    );
    assert_eq!(pen_rows(&c).await, 0);
    assert!(
        event_survived(&c, &record).await,
        "the event must reach the log, not merely leave the pen"
    );
}

// ---------------------------------------------------------------------------
// Arm 3 — the #580 case: no resolvable key at all
// ---------------------------------------------------------------------------

/// **When this node cannot resolve its custody key at all, the pen keeps BOTH halves.**
///
/// `cmd_requeue`'s warning has always promised exactly this — *"the pen holds both halves until you
/// do"* — and until #578 the very next statement emptied the pen (issue #580). The promise was true
/// for the duration of one function call.
///
/// This is also mutation 2's target. A fix keyed on `dek.is_some()` rather than on the pen row's
/// own `dek_wrapped` passes arm 1 and fails here: with no resolvable key there is no `dek`, so the
/// check never runs and the row is deleted with the key still in it.
#[tokio::test]
async fn an_unresolvable_custody_key_keeps_both_halves_in_the_pen() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (dir, _key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    // A DIFFERENT signing key, so `resolve_at_startup` derives an unwrap secret that diverges from
    // the one `node_unwrap_key` holds and REFUSES — the #495 shape, and the arm `cmd_requeue`
    // degrades on. The key file itself is valid: this models a lost custody key, not a broken one.
    let (other_sk, _kid) = cairn_event::generate_key().expect("generate a divergent signing key");
    let other_path = write_key_file(dir.path(), "other.key", &other_sk);

    let (ok, stdout, stderr) = run_requeue(&base, &other_path);
    assert!(
        ok,
        "requeue must not fail on a retained row\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);

    assert_eq!(m["custody_retained"], 1);
    assert_eq!(m["released"], 0);
    assert_eq!(
        pen_rows(&c).await,
        1,
        "THE #580 DEFECT: the warning promised the pen held both halves, then emptied it"
    );

    let line = stderr_line_with(&stderr, "KEPT in the pen");
    assert!(
        line.contains("could not be resolved"),
        "the operator must be told WHICH cause kept the row: {line}"
    );
}

// ---------------------------------------------------------------------------
// Arm 4 — an acked row is a recorded human decision (#581)
// ---------------------------------------------------------------------------

/// **A human's `acked` exclusion is honoured, announced, and reversible.**
///
/// `db/021` describes `acked` as the flag by which *"a human explicitly licenses the exclusion"* of
/// a record, and `do_pull` has always skipped such rows. `do_requeue` did not — it had no filter at
/// all — so a requeue silently re-applied records a human had decided would never enter the record.
/// A comment in `cairn-node`'s restore asserted the opposite behaviour (#581).
///
/// The skip must not be silent either: the same run that skips a row tells the operator how to put
/// it back in play, and un-acking it does exactly that.
#[tokio::test]
async fn an_acked_row_is_skipped_until_a_human_unacks_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;
    c.execute(
        "UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = $1",
        &[&record.digest],
    )
    .await
    .expect("a human licenses the exclusion");

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(ok, "requeue must succeed\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);

    assert_eq!(m["skipped_acked"], 1, "the human decision must be honoured");
    assert_eq!(m["released"], 0);
    assert_eq!(m["custody_retained"], 0, "a skip is not a custody failure");
    assert_eq!(pen_rows(&c).await, 1);
    assert!(
        !event_survived(&c, &record).await,
        "an acked row must not reach the log: that is what the human excluded"
    );

    let line = stderr_line_with(&stderr, "SKIPPED");
    assert!(
        line.contains("acked = FALSE"),
        "a skipped row is only honest if the operator is told how to unskip it: {line}"
    );

    // --- The way back in works. Without this, "skipped" could mean "stranded forever". ---
    c.execute(
        "UPDATE sync_quarantine SET acked = FALSE WHERE content_digest = $1",
        &[&record.digest],
    )
    .await
    .expect("the human changes their mind");

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(ok, "the un-acked run must succeed\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1);
    assert_eq!(m["skipped_acked"], 0);
    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "an un-acked row must come back with its body openable"
    );
}
