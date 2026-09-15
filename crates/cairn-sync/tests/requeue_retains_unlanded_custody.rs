//! Issue #578 — `requeue` never counts a release it did not get.
//!
//! # The failure this file exists to catch
//!
//! A clinic's disk is gone. A restore brought the record back but could not complete its custody,
//! so every sealed event sits in `sync_quarantine` **with its wrapped DEK** — and the pen row is
//! now the only copy of that key in the world. The penned reason told the operator, in these words
//! (its wording before PR #582), what to do next:
//!
//! > The bytes AND the key are kept: fix the cause, then `cairn-sync requeue` to complete the
//! > restore without redoing it.
//!
//! The operator runs it. `do_requeue` unwraps the DEK, hands the plaintext to `apply_remote_event`,
//! and the door — finding no row in `node_unwrap_key`, because registering one is a *separate*
//! ceremony that sentence never named — takes its lenient arm: `RAISE WARNING`, admit the sealed
//! event **without custody**, return normally. Nothing in this tree polls the connection's message
//! stream, so that warning goes nowhere. `do_requeue` saw `Ok`, **deleted the pen row**, counted it
//! `released`, printed a success line and exited 0.
//!
//! The ciphertext was then permanently unopenable and the key gone. The operator did exactly what
//! the software told them to do.
//!
//! # The rule these tests pin
//!
//! > A pen row that carries a wrapped DEK is released only when custody for its event is SETTLED.
//!
//! Stated once, in `crates/cairn-sync/src/requeue.rs`, along with why it is uniform across every
//! way custody can fail to land, and enforced again in the database by `cairn_release_pen_row`.
//! These tests prove it holds through the **shipped binary**, and — the half that makes it a
//! recovery rather than a stall — that fixing the cause brings the record back, all the way to the
//! clinician's chart.
//!
//! # What "the record came back" means here, and why nothing weaker will do
//!
//! Two assertions, and the review of this file's first version is why there are two:
//!
//! * **The body opens**: `event_clear.twin` reads back the dead node's own text. Rows, counts and a
//!   green exit are all satisfiable by a door that admitted well-formed ciphertext nobody can read —
//!   which is the entire defect. (#568's standard, for the release path.)
//! * **The chart has it**: `medication_statement` holds the record. The first version stopped at the
//!   twin, and so passed while the recovery it described left the medication list EMPTY: a retained
//!   row's event was admitted without custody on the first run, the projection saw no clear payload
//!   and wrote nothing, and on the second run the key landed on an event already in the log — whose
//!   `AFTER INSERT` projection trigger never fires again. Since ADR-0070 (#584) the door projects a
//!   key that lands on an already-admitted event, so arm 1 asserts the chart straight after the
//!   release, with no heal step.
//!
//! # Exit status
//!
//! Every run asserts its status, not a success flag: 0 is complete, [`EXIT_INCOMPLETE`] is a run
//! that finished and left work (rows still held), 1 is a run that failed.
//! A cron wrapper that drops stderr and ignores JSON has only this.
//!
//! # Mutations run against these tests
//!
//! Each is listed with the arm that killed it. A test written against behaviour that already works
//! proves nothing until a deliberate break has been shown to fail it, and the break has to match
//! the shape of the claim (#568's own lesson, one file over).
//!
//! Since PR #582's review the database enforces the same rule (`cairn_release_pen_row`), so most of
//! these Rust-side breaks no longer LOSE the key — the floor keeps the row — and what fails is the
//! account: the count, or the cause the line names. That is the floor doing its job, and it is why
//! each entry below says which assertion kills it.
//!
//! 1. **Release on the door's `Ok`** (skip the verdict) → arm 1 phase one: the database keeps the
//!    row, and the line names the guard's refusal instead of the missing registration.
//! 2. **Key the custody check on the opened `dek` instead of the pen row's `dek_wrapped`** → arm 3:
//!    a damaged row has no `dek`, the Rust check never runs, the guard keeps the row, and the line
//!    names the guard rather than the damaged row. In `requeue_releases_custody.rs` it is arm 3 that
//!    fails; arms 2 and 5 still pass, because the guard keeps the row and the count is the same.
//! 3. **Drop the shred state from the settled list** → arm 5: the shredded record is held forever,
//!    with a copy of a key erasure destroyed on purpose. Also killed at the SQL layer
//!    (`db/tests/052_restore_doors_test.sql`).
//! 4. **Release acked rows** (the pre-#581 listing) → arm 4: a human's recorded exclusion is
//!    overridden.
//! 5. **Blame every withheld DEK on a missing registration** (the pre-review single cause) → arm 6
//!    names the ceremony on a node where a key IS registered.
//! 6. **Stop the door projecting a late key** (delete db/020's late-custody call) → arm 1 phase two,
//!    on the chart assertion. (Recorded in ADR-0070's plan, Task 7.)
//!
//! Arm 7 (a custody read that fails) is deliberately NOT listed: it pins an end-to-end property that
//! more than one layer enforces, so no single Rust-side break is observable there — see its doc.
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
//         operator's fix brings the record back to the chart
// ---------------------------------------------------------------------------

/// **The #578 chain, end to end, and then its recovery — to the medication list.**
///
/// Phase one is the defect's own scenario: a node with no `node_unwrap_key` row runs `requeue`
/// against a pen holding the last copy of a record's key. The pen row must survive, byte for byte,
/// and the operator must be told which key to register AND the one way of registering it that would
/// foreclose the real key forever.
///
/// Phase two: the operator registers the key and runs `requeue` again. The row releases WITH its
/// key, the body opens, AND the chart has the record — the door itself projects the key onto the
/// already-admitted event (ADR-0070, #584), so no heal step is owed. The run exits 0, with no heal
/// instruction printed.
///
/// A further run over the now-empty pen also exits 0.
#[tokio::test]
async fn an_unregistered_unwrap_key_keeps_the_pen_row_and_the_fix_reaches_the_chart() {
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

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(
        code, EXIT_INCOMPLETE,
        "a run that kept a row has left work, and a script must be able to see it\nstderr: {stderr}"
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
    assert!(
        event_survived(&c, &record).await,
        "retention is about the KEY: the event itself is admitted, sealed, so the chart can show a \
         record exists"
    );

    // The operator is told which cause applied and what to do about it, on ONE line naming this
    // record — see `stderr_line_with`'s doc for why a whole-stream `contains` would not do.
    let line = stderr_line_with(&stderr, "KEPT in the pen");
    assert!(
        line.contains("establish-unwrap-key"),
        "the no-key cause must name the ceremony that fixes it: {line}"
    );
    assert!(
        line.contains("derived from the signing key"),
        "…and WHICH key opened the DEK, since registering any other one is the mistake: {line}"
    );
    assert!(
        line.contains("Do NOT run"),
        "…and the one careless way of running it that forecloses the real key forever: {line}"
    );

    // --- Phase two: the operator registers the key, and runs it again. ---
    register_unwrap_key(&c, &sk).await;

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the fixed node must release the row: {m}");
    assert_eq!(
        m["released_with_custody"], 1,
        "and it must carry its key: {m}"
    );
    assert_eq!(m["custody_retained"], 0);
    assert_eq!(pen_rows(&c).await, 0, "the pen must now be empty");
    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "the sealed body must open back to the dead node's own text"
    );

    // THE REVIEW FINDING, answered at the door. The body opens AND the chart has it: the event was
    // admitted without custody on phase one, and the door projects a key that lands afterwards
    // (#584, ADR-0070). This used to be 0, and a separate `cairn-node reproject` heal step
    // followed.
    assert_eq!(
        medication_rows(&c).await,
        1,
        "the recovered record is on the chart as soon as its key lands"
    );
    assert!(
        m.get("reproject_owed").is_none(),
        "the heal signal is retired: the door projected the late key (ADR-0070): {m}"
    );
    assert_eq!(
        code, 0,
        "every row released with its key and its chart: a COMPLETE recovery\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("reproject"),
        "no heal instruction for a record that needs none\nstderr: {stderr}"
    );

    // An empty pen stays a complete run.
    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(code, 0, "an empty pen is a complete run\nstderr: {stderr}");
    assert_eq!(metrics(&stdout, &stderr)["examined"], 0);
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

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(
        code, 0,
        "nothing is left, so the run is complete\nstderr: {stderr}"
    );
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
// Arm 3 — a damaged pen row: kept, and not promised a rerun
// ---------------------------------------------------------------------------

/// **A wrong-length wrapped DEK is kept, blamed on the row, and not sent round again.**
///
/// The #581 half of the review: a truncated blob is a pen write defect, not somebody else's key, and
/// no rerun opens it — so the line must neither send the operator looking for another node's key nor
/// promise that `requeue` again will help.
///
/// Also mutation 2's target. A custody check keyed on the opened `dek` rather than on the pen row's
/// own `dek_wrapped` never runs here — a damaged row has no `dek`. Before the database guard that
/// deleted the row with its key; now the guard keeps it, and this arm fails on the cause its line
/// names.
#[tokio::test]
async fn a_damaged_pen_row_is_kept_and_blamed_on_the_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    let truncated = &record.dek_wrapped[..record.dek_wrapped.len() - 4];
    pen(&c, &record, Some(truncated)).await;

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(code, EXIT_INCOMPLETE, "stderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["custody_retained"], 1);
    assert_eq!(m["released"], 0);
    assert_eq!(
        pen_rows(&c).await,
        1,
        "the row must stay — whatever is wrong with it, it is the only copy there is"
    );

    let line = stderr_line_with(&stderr, "KEPT in the pen");
    assert!(
        line.contains("WRONG LENGTH"),
        "the row itself is named as the fault: {line}"
    );
    assert!(
        !line.contains("another node"),
        "a truncated row must not send the operator hunting for another node's key: {line}"
    );
    assert!(
        !line.contains("run `cairn-sync requeue` again"),
        "no rerun opens a wrong-length blob: {line}"
    );
}

// ---------------------------------------------------------------------------
// Arm 4 — an acked row is a recorded human decision (#581)
// ---------------------------------------------------------------------------

/// **A human's `acked` exclusion is honoured, announced, and reversible.**
///
/// `db/021` describes `acked` as the flag by which *"a human explicitly licenses the exclusion"* of
/// a record. `do_requeue` had no filter at all, so a requeue silently re-applied records a human had
/// decided to exclude. (`do_pull` does something different again — it re-offers acked bytes and
/// silences only their refusals; `requeue.rs` and pull's release site say why the two differ.)
///
/// The skip must not be silent either: the same run that skips a row tells the operator how to put
/// it back in play, typed out in full, and un-acking it does exactly that.
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

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(
        code, 0,
        "a human's decision is not work left undone\nstderr: {stderr}"
    );
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
    let unack = format!(
        "acked = FALSE WHERE content_digest = '\\x{}'",
        hex::encode(&record.digest)
    );
    assert!(
        line.contains(&unack),
        "a skipped row is only honest if the operator is told how to unskip it, in SQL that \
         matches the row: {line}"
    );

    // --- The way back in works, using exactly the SQL the line printed. ---
    c.batch_execute(&format!("UPDATE sync_quarantine SET {unack}"))
        .await
        .expect("the human changes their mind, with the statement they were given");

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(code, 0, "the un-acked run must complete\nstderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1);
    assert_eq!(m["skipped_acked"], 0);
    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "an un-acked row must come back with its body openable"
    );
    assert_eq!(
        medication_rows(&c).await,
        1,
        "…and on the chart: this event entered the log for the first time WITH its key, so its \
         projection fired and no heal is owed"
    );
}

// ---------------------------------------------------------------------------
// Arm 5 — a shredded record: settled, released, and counted as what it is
// ---------------------------------------------------------------------------

/// **A pen row over a SHREDDED event is released, and not counted as a recovery.**
///
/// ADR-0005's anti-resurrection rule: `db/020` refuses custody for a shredded target however often
/// it is re-delivered. A requeue that read that as "custody did not land" would hold the row
/// forever — keeping a copy of a key erasure destroyed on purpose. So the row goes; and because
/// three destroyed keys must not read to a monitor as three recovered charts, it is counted in
/// `released_shredded`, never `released_with_custody`.
#[tokio::test]
async fn a_shredded_record_releases_as_shredded_not_as_recovered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;
    // The shred, as the erasure plane logs it. Staged directly: this arm is about what `requeue`
    // does with a shredded record, not about the ceremony that shreds one (`seal_apply.rs`).
    c.execute(
        "INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis) \
         VALUES ($1::text::uuid, gen_random_uuid(), 'test: patient-requested erasure')",
        &[&record.event_id],
    )
    .await
    .expect("log the shred");

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(
        code, 0,
        "a shred is a settled state, not work left\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(
        m["released"], 1,
        "the row must go, and its copy of the key with it: {m}"
    );
    assert_eq!(m["released_shredded"], 1, "counted as shredded: {m}");
    assert_eq!(
        m["released_with_custody"], 0,
        "a destroyed key is not a recovered one: {m}"
    );
    assert_eq!(
        m["custody_retained"], 0,
        "and never held waiting forever: {m}"
    );
    assert_eq!(pen_rows(&c).await, 0);
    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "anti-resurrection: a shredded body must not open"
    );
}

// ---------------------------------------------------------------------------
// Arm 6 — a DEK that opens with this node's key but not the body
// ---------------------------------------------------------------------------

/// **The door's OTHER withholding arm is named as itself.**
///
/// `db/020` withholds custody for a DEK that opened on two arms: no unwrap key registered (arm 1),
/// and — here — a DEK that does not open THIS event's body, although it is wrapped to this node and
/// a key IS registered. The first version of this fix blamed both on the missing registration and
/// sent an operator who had already registered a key to register one again, run after run.
#[tokio::test]
async fn a_dek_that_does_not_open_the_body_is_not_blamed_on_registration() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;
    // A well-formed DEK wrapped to THIS node's own public half — it opens — that belongs to no
    // event. Derived at runtime (house rule 6a); `lineage`, not a cryptographic name (rule 6b).
    let lineage = 9u8;
    let wrong_dek = cairn_event::keys::Secret32::from_bytes(std::array::from_fn(|i| {
        (i as u8).wrapping_mul(lineage).wrapping_add(1)
    }));
    let wrapped_to_us = cairn_event::seal::wrap_dek_for(
        &wrong_dek,
        &cairn_event::seal::unwrap_public(&derived_unwrap_secret(&sk)),
    )
    .expect("wrap to this node");
    pen(&c, &record, Some(&wrapped_to_us)).await;

    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(code, EXIT_INCOMPLETE, "stderr: {stderr}");
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["custody_retained"], 1, "{m}");
    assert_eq!(pen_rows(&c).await, 1);

    let line = stderr_line_with(&stderr, "KEPT in the pen");
    assert!(
        line.contains("did not open this event's sealed body"),
        "the cause is the body, not the registration: {line}"
    );
    assert!(
        !line.contains("establish-unwrap-key"),
        "a key IS registered here; naming the ceremony again is the single-cause claim the review \
         found: {line}"
    );
}

// ---------------------------------------------------------------------------
// Arm 7 — a database fault while asking about custody stops the run, and keeps the key
// ---------------------------------------------------------------------------

/// **When the custody question itself cannot be answered, nothing is decided — least of all a
/// release.**
///
/// Every custody decision `requeue` makes rests on reading `cairn_custody_state`. If that read
/// fails — a lock storm, a revoked grant, a dropped connection — the only safe answer is no answer:
/// the run stops through the same partial-completion report as every other local fault (#471,
/// ADR-0060 decision 2), and the row keeps its key.
///
/// # What this arm does and does NOT prove
///
/// It pins the END-TO-END property: a fault on the custody path is a failed run (exit 1) that keeps
/// the key. It does NOT isolate `do_requeue`'s own handling of the error, and cannot: the same
/// function this arm breaks is also called by `cairn_release_pen_row` (via `cairn_custody_landed`),
/// so a Rust version that swallowed `do_requeue`'s own read and pressed on regardless would still
/// meet the same fault one statement later, inside the release door, and stop the same way. Two
/// layers ask the same question; this arm proves that together they never trade the key for a
/// transient fault.
///
/// # How the fault is forced
///
/// `cairn_custody_state` (db/052) is replaced, on the test's own connection, with a stand-in that
/// always `RAISE EXCEPTION`s `lock_not_available` (55P03) — same signature and return type as the
/// real one, so `CREATE OR REPLACE` is legal. Since ADR-0070 removed `do_requeue`'s pre-door read,
/// the apply door itself never calls `cairn_custody_state` (`grep -n cairn_custody_state
/// db/020_apply_remote_event.sql db/005_submit.sql` finds nothing), so the door admits the event
/// exactly as it would in any other run, and requeue's own POST-apply custody read is the first —
/// and only — statement that reaches the faulty function. (An earlier version of this arm locked
/// `event_dek` instead; once #584 removed the pre-door read, that lock was met by the door's step-9
/// custody write (`INSERT INTO event_dek`) before requeue's own read ever ran, so it stopped
/// testing what its name claimed — the controller's review, ADR-0070.) Restored — not merely rolled back — before
/// asserting: `db::connect_and_load_schema` replays every `db/*.sql`, db/052 included, so the real
/// `cairn_custody_state` is back in place before the next test in this process runs, the same
/// reason `a_requeue_interrupted_mid_loop_still_reports_what_it_released` gives for preferring
/// self-releasing state over anything a panic could leave behind.
#[tokio::test]
async fn a_custody_read_that_fails_stops_the_run_and_keeps_the_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();

    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    // The fault: a same-signature stand-in that always raises. Neither `apply_remote_event`
    // (db/020) nor `submit_event` (db/005) call `cairn_custody_state` — see this fn's doc —
    // so the door's admission below is unaffected, and requeue's own post-apply read is what fails.
    c.batch_execute(
        "CREATE OR REPLACE FUNCTION cairn_custody_state(p_content_address BYTEA)
         RETURNS TEXT LANGUAGE plpgsql SET search_path = public, pg_temp AS $$
         BEGIN
             RAISE EXCEPTION 'cairn_test fault: custody state unavailable'
                 USING ERRCODE = 'lock_not_available';
         END;
         $$;",
    )
    .await
    .expect("install the faulty stand-in");

    let (code, stdout, stderr) = run_requeue(&base, &key_path);

    // Restore BEFORE asserting: a panic below must not leave the fault installed for whatever
    // test in this process runs next. The fresh client this returns is also what the read at the
    // bottom of this test uses.
    let c = db::connect_and_load_schema(&base).await.unwrap();

    assert_eq!(
        code, 1,
        "an unanswerable custody question is a FAILED run, not an incomplete one\nstderr: {stderr}"
    );
    assert!(
        stderr.contains("INTERRUPTED") && stderr.contains("[55P03]"),
        "the operator is told the run stopped, and why: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(
        m["released"], 0,
        "nothing may be released on an unanswered question: {m}"
    );
    assert_eq!(
        m["custody_retained"], 0,
        "…nor counted as kept: the row's outcome is undecided, and the message says so: {m}"
    );

    let kept: Vec<u8> = c
        .query_one(
            "SELECT dek_wrapped FROM sync_quarantine WHERE content_digest = $1",
            &[&record.digest],
        )
        .await
        .expect("the row is still there")
        .get(0);
    assert_eq!(
        kept, record.dek_wrapped,
        "and it still holds the key, byte for byte"
    );
}
