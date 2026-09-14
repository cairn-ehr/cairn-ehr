//! #572 and #570 — the restore **command**, driven as a real process with **no pseudo-terminal**.
//!
//! # Why this file exists
//!
//! `restore`'s logic is well covered at the library level. Its COMMAND was not covered at all,
//! and the two facts have the same cause: until #572 the recovery-code prompt was read through
//! `rpassword`, which opens `/dev/tty` and fails on any non-tty. A CLI test could not drive it
//! without allocating a pseudo-terminal, so nobody did — `restore_torn_medium_cli.rs`, the only
//! CLI-level restore test that existed, asserts `status.success()` on a federation-only medium
//! and therefore never enters the clinical block at all.
//!
//! Every test here runs with a piped stdout and stderr. That is not incidental: **a piped run
//! is the thing under test.** If any of these ever needs a pty again, #572 has regressed.
//!
//! # Why the headline test reads a body in CLEAR
//!
//! Because it is the only assertion that can distinguish a correct restore from the double-wrap
//! `restore_reads_the_clinical_plane.rs`'s header describes: `apply_remote_event`'s `p_dek` is
//! fed straight into `cairn_wrap_dek(p_dek, v_pub)` — the door wraps what it is handed — so
//! piping an already-wrapped key through leaves every `event_dek` row present, well-formed,
//! exactly the right length, and unwrapping to noise. Counts agree. `verify-backup` is green.
//! The defect surfaces months later when a clinician opens a chart. **A test that counted rows
//! would have shipped it.**
//!
//! # How the fixture is built, and why the export is assembled through the LIBRARY
//!
//! The source node's medium and its `CAIRNL1` export sibling are built by calling the same
//! functions the `backup` command calls (`backup::backup_to`, `read_local_state`,
//! `build_export_container`), rather than by spawning `backup` itself. Two reasons: `backup`'s
//! own CLI arm is already covered by `cli_localstate.rs`, and assembling the export directly is
//! what lets the recovery code be **derived at runtime** (house rule 6a) instead of parsed back
//! out of a `init`/`seal-key` banner. The subject of this file is `restore`, and that is the
//! only command spawned.
//!
//! Those fixtures live in `tests/common/restore_kit.rs` since #593, which needed the same dead
//! clinic in four more suites.

use cairn_node::{backup, db, localstate};

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    a_recovery_code, an_op_passphrase, author_sealed_clinical_event, cairn_node, cs,
    medium_with_export, provisioned_clinic, wipe_to_a_fresh_dr_machine, Authored, ExportCustody,
};

// ---------------------------------------------------------------------------

/// **THE PROOF THAT #572 IS CLOSED.** A restore driven with no terminal at all brings the
/// clinical record back, and a sealed body OPENS.
///
/// Before this, piping a recovery code did not merely fail to be read: the read ERRORED, the
/// export never opened, and the restore finished having recovered ZERO PATIENTS while exiting
/// non-zero — #500's own signature, arriving inside the mechanism built to prevent it.
///
/// The final assertion is the point. Not "a row exists" — the body opens, to the exact twin
/// text the dead node held.
#[tokio::test]
async fn a_scripted_restore_brings_the_clinical_record_back() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let op = an_op_passphrase(1);
    let code = a_recovery_code(1);
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let Authored { event_id, twin, .. } = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = medium_with_export(
        &c,
        &sk,
        &kid,
        dir.path(),
        &op,
        &code,
        ExportCustody::Carried,
    )
    .await;

    wipe_to_a_fresh_dr_machine(&c).await;

    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, &code).unwrap();
    let new_key = dir.path().join("restored.key");

    // NO PSEUDO-TERMINAL. stdout and stderr are pipes, which is exactly the shape that used to
    // recover zero patients.
    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--old-recovery-code-file"])
        .arg(&code_file)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // The exit status is checked FIRST and is strictly broader than any summary line: `restore`
    // deliberately prints its whole report and only THEN fails, so a refused local-state bundle
    // — meaning the dead node's custody was NOT installed — arrives as a clean-looking clinical
    // line followed by a non-zero exit.
    assert!(
        out.status.success(),
        "a scripted restore must succeed; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("local-state restored"),
        "the export must have been unsealed with the supplied code; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );

    // THE ASSERTION THAT MATTERS. Not "a row exists" — the body OPENS.
    let restored_twin: String = c
        .query_one(
            "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect(
            "a restored node must be able to READ the chart, not merely hold ciphertext — a \
             double-wrapped DEK row is present, well-formed and exactly the right length",
        )
        .get(0);
    assert_eq!(
        restored_twin, twin,
        "the restored body must decrypt to what the dead node held"
    );

    // AND THE CLINICIAN CAN FIND IT. `event_clear` holding a readable body is not yet a chart:
    // a projection that silently no-opped under the remote-apply marker would leave every
    // assertion above green with the chart list EMPTY — the zero-patients outcome in its third
    // costume.
    let charted: i64 = c
        .query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .unwrap()
        .get(0);
    assert!(
        charted > 0,
        "a restored node must have PATIENTS, not merely rows; stdout:\n{stdout}"
    );
}

/// A wrong code in a file degrades exactly as a wrong TYPED code does, and says so honestly.
///
/// Two things are pinned here. First, the degradation: local-state is OPTIONAL and the events
/// are the load-bearing copy, so a bad code must not kill an otherwise complete restore — it
/// warns, skips, and the process exits non-zero AFTER the summary has printed.
///
/// Second, and this is the one a refactor would break: the message must say **one** attempt,
/// not three. `unsealing_failed_cause` takes the RESOLVED count, because re-reading a file
/// cannot change the answer — a line claiming three attempts were spent on a single file read
/// is a message that lies to an operator mid-disaster.
#[tokio::test]
async fn a_wrong_code_in_a_file_degrades_honestly_and_counts_one_attempt() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let op = an_op_passphrase(2);
    let code = a_recovery_code(2);
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = medium_with_export(
        &c,
        &sk,
        &kid,
        dir.path(),
        &op,
        &code,
        ExportCustody::Carried,
    )
    .await;

    wipe_to_a_fresh_dr_machine(&c).await;

    // A DIFFERENT lineage, so the code is well-formed but wrong — the operator-error case,
    // not a malformed-input case.
    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, a_recovery_code(9)).unwrap();
    let new_key = dir.path().join("restored.key");

    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--old-recovery-code-file"])
        .arg(&code_file)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a restore that inherited no custody must exit non-zero, so a script can see it; \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // The summary still printed. Losing it would cost the operator their next step at the one
    // moment they need it.
    assert!(
        stdout.contains("restored") || stdout.contains("clinical records"),
        "the summary must print BEFORE the failure; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // Principle 4: this node cannot tell a wrong code from a damaged export, and must not
    // pretend otherwise.
    assert!(
        stderr.contains("after 1 attempt.") && !stderr.contains("1 attempts"),
        "the spent-attempts line must report the RESOLVED count, correctly pluralized — a \
         file is read once, so claiming three lies to an operator mid-disaster and \
         \"1 attempts\" is the sentence they read at the worst moment of their year; \
         stderr:\n{stderr}"
    );
}

/// A supplied code with no export beside the medium is INERT, and the operator is told.
///
/// Nothing is wrong with such a restore, so it must not fail — a federation-only medium is a
/// legitimate thing to restore. But a drill script pointed at the wrong medium would otherwise
/// go green while exercising none of the custody path it exists to exercise, and a green run
/// that proves nothing is worse than a red one.
#[tokio::test]
async fn a_supplied_code_with_no_export_warns_that_it_is_inert() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;

    // A medium with NO export sibling: `backup_to` alone, and nothing beside it.
    let medium = dir.path().join("cairn.medium");
    backup::backup_to(
        &c,
        &medium,
        &dir.path().join("backup-status.json"),
        0,
        Some((&sk, kid.as_str())),
    )
    .await
    .unwrap();
    assert!(
        !localstate::localstate_path_for(&medium).exists(),
        "this fixture needs a medium with NO export sibling"
    );

    wipe_to_a_fresh_dr_machine(&c).await;

    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, a_recovery_code(3)).unwrap();
    let new_key = dir.path().join("restored.key");

    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--old-recovery-code-file"])
        .arg(&code_file)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "an inert code must not fail an otherwise fine restore; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("no local-state export sits beside"),
        "the operator must be told the code was inert, or a mis-pointed drill goes green \
         while exercising nothing; stderr:\n{stderr}"
    );
}

/// **WITHOUT the flag, a piped restore still cannot read the prompt — and this is what #572
/// was filed about.** Pinned because the issue asks for it in as many words: *"whatever is
/// decided, the current behaviour deserves a test that drives the real binary. There is none."*
///
/// It is also the anti-vacuity proof for every test above. If this one ever passes the way the
/// headline test does, then `--old-recovery-code-file` is not what made the difference and the
/// suite has stopped measuring the thing it claims to measure.
///
/// The degradation itself is correct and must not be "fixed": local-state is OPTIONAL, the
/// events are the load-bearing copy, and a prompt that cannot be read is not a reason to throw
/// away a restore that otherwise succeeded. What was wrong was that there was no way to take
/// the remedy it names.
#[tokio::test]
async fn without_the_flag_a_piped_restore_still_inherits_no_custody() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let op = an_op_passphrase(4);
    let code = a_recovery_code(4);
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let Authored { event_id, .. } = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = medium_with_export(
        &c,
        &sk,
        &kid,
        dir.path(),
        &op,
        &code,
        ExportCustody::Carried,
    )
    .await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let new_key = dir.path().join("restored.key");

    // The same medium, the same export, the same piped streams — and NO flag. The correct
    // code is not even offered, because there is no way to offer it.
    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a restore that inherited no custody must exit non-zero; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    // The custody never arrived, so the body cannot open. This is the contrast that makes the
    // headline test meaningful: same inputs, same pipes, one flag apart.
    let readable: i64 = c
        .query_one(
            "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        readable, 0,
        "without the recovery code the export never opened, so no sealed body can be read — \
         if this is ever non-zero, the headline test is not proving what it claims"
    );
}

/// **#570 item 1, the loudest bail: a restore that could not offer ONE record must exit
/// non-zero.** And #570 item 2 for the "NO actor registry" warning, whose reachability a
/// source-text grep cannot establish.
///
/// A medium carrying charts, with no export beside it, is the most incomplete of the three
/// outcomes: without the registry every record would be refused as authored by an unenrolled
/// signer, so the door is never offered any of them. It pens nothing, which is exactly why it
/// is checked separately from the pen — a `penned() > 0` test alone hands a monitoring script
/// exit 0 for a restore that recovered no charts at all.
///
/// The remedy in the message is the load-bearing part. `cairn-sync requeue` will NOT fix this,
/// because `finalize_identity` runs at the end of this restore and permanently closes the
/// registry door — printing the custody remedy here would be a false promise to someone
/// mid-disaster.
#[tokio::test]
async fn a_restore_that_offered_no_record_exits_non_zero_and_names_the_real_remedy() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let Authored { event_id, .. } = author_sealed_clinical_event(&c, &sk, &kid).await;

    // Charts on the medium, and NO export sibling — so no actor registry travels.
    let medium = dir.path().join("cairn.medium");
    backup::backup_to(
        &c,
        &medium,
        &dir.path().join("backup-status.json"),
        0,
        Some((&sk, kid.as_str())),
    )
    .await
    .unwrap();

    wipe_to_a_fresh_dr_machine(&c).await;
    let new_key = dir.path().join("restored.key");

    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a restore that offered NOT ONE record to the door must exit non-zero — a monitoring \
         script reading exit 0 files an incomplete restore as clean; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    // Reachability, which the text-grepping guard cannot establish: gate this block behind
    // `if false` and that guard stays green while this test goes red.
    assert!(
        stderr.contains("NO actor registry"),
        "the operator must be warned that no registry travelled; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("requeue` will NOT fix this") || stderr.contains("will NOT fix this"),
        "the warning must say requeue cannot fix this — finalize_identity closes the registry \
         door permanently, so the custody remedy would be a false promise; stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("NOT ONE record was offered"),
        "the summary must say the plane was never offered, not merely report zeroes — an \
         all-zero clinical line is indistinguishable from a medium that held nothing, and the \
         two have opposite remedies; stdout:\n{stdout}"
    );
    // The charts really are still unrecovered, so the non-zero exit is telling the truth.
    let readable: i64 = c
        .query_one(
            "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(readable, 0, "no chart can have been recovered here");
}

/// **#570 item 1, the pen bail — and the AEAD caveat's reachability (#570 item 2).**
///
/// An export written by a node whose `.unwrap` keystore file could not be loaded carries the
/// actor registry and the custody ROWS, but no key to open them. That is not a contrived
/// fixture: `seal_and_write_local_state_export` warns and writes exactly that, because the
/// export is optional and the medium is the load-bearing copy.
///
/// So the registry lands, the door accepts the signers, and every record carrying custody is
/// PENNED **with its key beside it** — recoverable later by `cairn-sync requeue`, which is the
/// remedy the message must name here and must NOT have named in the test above.
#[tokio::test]
async fn a_penned_clinical_restore_exits_non_zero_and_prints_the_aead_caveat() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let op = an_op_passphrase(5);
    let code = a_recovery_code(5);
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let Authored { event_id, .. } = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = medium_with_export(
        &c,
        &sk,
        &kid,
        dir.path(),
        &op,
        &code,
        ExportCustody::Missing,
    )
    .await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, &code).unwrap();
    let new_key = dir.path().join("restored.key");

    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&new_key)
        .args(["restore", "--from"])
        .arg(&medium)
        .args(["--old-recovery-code-file"])
        .arg(&code_file)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a restore holding records in the pen is INCOMPLETE and a script must see that; \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // THE AEAD CAVEAT. Reachable only when a registry actually restores, which is why it is
    // asserted here rather than guessed at from the source.
    assert!(
        stdout.contains("NOT by a per-row signature"),
        "the one part of a restore that is not verify-on-apply must be said out loud — a \
         limitation living only in a design doc is one nobody finds; stdout:\n{stdout}"
    );
    // The pen's remedy IS true here, unlike the registry case above: the bytes and the key are
    // both held, so a requeue genuinely completes the restore.
    assert!(
        stdout.contains("quarantine pen with their custody"),
        "the operator must be told the records are held WITH their key; stdout:\n{stdout}"
    );
    // The body is not readable yet, and that is correct — it is penned, not lost.
    let readable: i64 = c
        .query_one(
            "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        readable, 0,
        "a penned record is held, not admitted — if this is readable the pen did not engage"
    );
}
