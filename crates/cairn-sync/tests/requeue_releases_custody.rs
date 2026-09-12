//! Issue #568 — `cairn-sync requeue` releases a penned sealed event WITH its custody.
//!
//! # The failure this file exists to catch
//!
//! A solo clinic runs an unattended nightly backup. No passphrase is on the machine, so when the
//! restore comes it cannot open the node's custody export, and every sealed clinical record is
//! **penned** in `sync_quarantine` with its wrapped DEK rather than admitted without its key. That
//! is the deliberate behaviour of DR slice 2d (#554, ADR-0067), and every penned reason the restore
//! prints tells the operator the same remedy: recover the export, then run `cairn-sync requeue`.
//!
//! Now the disk is gone and the pen is the only copy of the record in the world. If `requeue` hands
//! the WRAPPED DEK where the door expects the plaintext, or resolves the wrong key file, or reads
//! the wrong column, then the events release **without custody at exit 0 while the pen row is
//! deleted**. That is permanent, silent loss of every sealed chart, arriving inside the mechanism
//! built to prevent exactly that — #500's own shape, one layer down.
//!
//! # Why this had no test before
//!
//! `do_requeue` gained its custody arm in slice 2d, and every call site in the suite passes `None`
//! for the unwrap secret; only production ever passed a real key. `cmd_requeue`'s `--key` /
//! `--unwrap-key` plumbing — *"where a wrong default would live"* — had no test at all.
//!
//! This file closes the `--key` half. `--unwrap-key` is still uncovered, deliberately — see
//! *What this file does NOT cover* below, so nobody reads the paragraph above as a list of what
//! now exists.
//!
//! # What these tests assert, and why it is the twin and not a row count
//!
//! The load-bearing assertion is that **a sealed body OPENS**: `event_clear.twin` reads back the
//! exact text the dead node held. A row count is a trace of the property; the twin IS the property,
//! and it stays right against a future door that writes custody before proving it can be used.
//!
//! ⚠️ #568's own wording — *"counting an `event_dek` row is not enough; that is exactly the
//! assertion that would pass under a double-wrap"* — is not true of THIS door, and a reader should
//! not go looking for the case it describes. `db/020` unseals with `p_dek` FIRST; a double-wrapped
//! value fails that unseal, `v_inner` is left NULL, and the whole custody block is skipped, so a
//! double-wrap leaves no `event_dek` row AND no `event_clear` row. The instruction is still the
//! right one, for the better reason above.
//!
//! # The four arms, and why all four are here
//!
//! `do_requeue`'s custody decision produces three OUTCOMES from four INPUT PAIRS, and the
//! difference matters: the match at `main.rs`'s `let dek = match (&unwrap_secret, &penned_dek)`
//! has a `_` arm that absorbs three distinct pairs into one silent `None`. Counting outcomes and
//! calling the job done is how the modal pair goes untested, so the arms here are indexed by
//! INPUT:
//!
//! 1. `(Some, Some)` → opens. **Custody resolves and the DEK opens** — the body comes back.
//! 2. `(None, Some)` → `_`. **No custody resolves at all** — the event is applied and the pen row
//!    is KEPT, with its key.
//! 3. `(Some, Some)` → fails. **This key does not open THAT DEK** — the event is applied, the row
//!    is KEPT, and the reason is named.
//! 4. `(Some, None)` → `_`. **A keyless pen row while custody resolves** — the ordinary, non-DR
//!    shape (`dek_wrapped` is nullable and every plaintext pull row has it NULL). Releases, and
//!    must raise NO custody alarm.
//!
//! ⚠️ **ARMS 2, 3 AND 5 WERE INVERTED BY #578, AND THEY SAY SO IN PLACE.** All three used to assert
//! that the row was RELEASED when custody could not be recovered, which is how a requeue came to
//! destroy the last copy of a clinical DEK at exit 0. The rule now is that a pen row carrying a
//! wrapped DEK is released only when custody actually landed (`crates/cairn-sync/src/requeue.rs`),
//! and the retention path has its own suite in `requeue_retains_unlanded_custody.rs`. Read the
//! inverted assertions there and here as one change, not two behaviours.
//!
//! The remaining pair, `(None, None)`, is the trivial composition of 2 and 4 and is left untested
//! deliberately: it reaches the same `_` arm with neither input present and has no behaviour of
//! its own.
//!
//! Arm 2 is the anti-vacuity twin of arm 1: without it, a suite that never opened anything at all
//! would still pass arm 1's shape. Arm 4 is the FALSE-POSITIVE twin of arm 3 — arm 3 proves the
//! alarm fires when custody is genuinely lost, arm 4 that it stays silent when nothing is wrong.
//!
//! # What this file does NOT cover
//!
//! Stated here rather than left for a reader to discover, because a coverage boundary belongs with
//! the durable artifact and not with the disposable plan:
//!
//! * **It cannot isolate a fault inside `do_requeue` from one inside the resolution.** The route is
//!   the composed CLI command, so a red run says the composition is wrong, never which half.
//! * **`--unwrap-key` and the whole `FileOutcome::Loaded` path are untested.** `run_requeue` passes
//!   `--key` alone and never writes a `<key>.unwrap` sibling, so every arm resolves through the
//!   pre-ADR-0066 DERIVED fallback. The provisioned-file path — including a `--unwrap-key` pointed
//!   at the wrong directory, which is #515's shape — would stay green under a mutation. Closing
//!   that is #517's business, not this file's.
//! * **Every arm pens exactly ONE record.** `do_requeue` computes `dek` inside its loop, so
//!   degradation is per-row today; nothing here would notice it being hoisted above the loop.
//!
//! # Mutation results — why this file is trusted
//!
//! A test written against behaviour that already works proves nothing until a deliberate break has
//! been shown to make it fail. Every mutation below was applied to `main.rs` and RUN; every one is
//! killed, and mutation 4 is the reason this section exists at all.
//!
//! | # | mutation in `main.rs` | 1 opens | 2 unresolvable | 3 foreign | 4 keyless | 5 minting |
//! |---|-----------------------|---------|----------------|-----------|-----------|-----------|
//! | 1 | the wrapped DEK passed through unwrapped, i.e. NOT unwrapped at all (the double-wrap) | **FAIL** | ok | **FAIL** | ok | ok |
//! | 2 | `row.get(3)` → `row.get(2)`, the `attester_key` column | **FAIL** | ok | **FAIL** | ok | ok |
//! | 3 | `do_requeue(&mut client, None)` — the pre-slice-2d behaviour | **FAIL** | ok | **FAIL** | ok | ok |
//! | 4 | `load_existing_key` → the minting `load_or_create_key` | ok | ok | ok | ok | **FAIL** |
//! | 5 | custody resolution REFUSES instead of degrading best-effort | ok | **FAIL** | ok | ok | **FAIL** |
//! | 6 | the `_` arm folded in: `(Some(secret), maybe)` → `unwrap_dek(maybe.as_deref().unwrap_or(&[]), secret)` | ok | ok | ok | **FAIL** | ok |
//! | 7 | `apply_signed` never called — the door's OK assumed, the pen row still deleted | **FAIL** | **FAIL** | **FAIL** | **FAIL** | **FAIL** |
//!
//! **Where mutations 1–3 surface is one assertion PER ARM, and not the same one.** In arm 1 it is
//! the twin: `event_clear.twin` is `None` where the dead node's text belongs. In arm 3 the twin is
//! legitimately `None` either way — that arm EXPECTS a sealed body — so the killer there is the
//! per-record stderr report, which all three mutations remove by routing around the unwrap. Reading
//! this as "one assertion" would send a maintainer debugging a red arm 3 to the wrong line.
//!
//! **TWO mutations survived a first draft, and both write-ups are the point of this section.**
//!
//! **Mutation 4** named a key path inside a subdirectory that did not exist, so the mint failed on
//! the missing parent rather than being refused, and test 5 was green for a reason unrelated to its
//! subject. The correction is written into that test: an operator in the wrong directory is in a
//! directory that EXISTS, and that is the case to model.
//!
//! **Mutation 7** is the one the first review caught, and it took two attempts to write. Arms 2, 3
//! and 4 originally proved the event had survived using only `released`, a counter inside the
//! function under test, so a release path that deleted the pen row without applying the bytes would
//! report `released: 1` over a record that no longer existed — and only arm 1, via the twin, would
//! notice. `event_survived` is the fix: every arm now asks the LOG, not the tool.
//!
//! ⚠️ Its first draft SURVIVED. It merely moved the `DELETE FROM sync_quarantine` ahead of
//! `apply_signed` and swallowed the apply's error. That is a real defect in production — an apply
//! that fails now loses the pen row — but it is invisible to THIS suite, because every arm feeds
//! the door bytes it accepts, so the apply succeeds and the reordering changes nothing observable.
//! Modelling "the pen row is deleted and the event never reaches the log" requires skipping the
//! door entirely, which is what row 7 now does. The lesson is the file's own, one turn deeper: a
//! mutation that cannot fail is not evidence, and the shape of the break has to match the shape of
//! the claim. **The ordering defect it failed to model is therefore still uncovered here** —
//! `do_requeue`'s own `interrupted(...)` path is what guards it, and issue #471's tests are its
//! home, not this file's.
//!
//! Skips unless `CAIRN_TEST_PG` is set. Serialized via cairn-node's `db::test_serial_guard` —
//! advisory locks are scoped PER DATABASE, not cluster-wide (#476) — because this file TRUNCATEs
//! tables every other DB-gated suite also uses.

// The dead-node fixture, shared with `requeue_retains_unlanded_custody.rs` (#578) rather than
// copied into it — see that module's own header for why four hundred lines of provisioning is not
// something to have twice.
#[path = "common/dead_node.rs"]
mod dead_node;
use dead_node::*;

use cairn_node::db;

// ---------------------------------------------------------------------------
// Arm 1 — the headline: custody survives the pen, and the body opens
// ---------------------------------------------------------------------------

/// **The remedy every restore-penned reason advertises, proved end to end.**
///
/// Design test 4 of slice 2d's plan — *"custody survives the pen, via requeue"* — of which only the
/// first half was ever built: `restore_reads_the_clinical_plane.rs` proves the pen HOLDS the key;
/// until now nothing proved `requeue` RELEASES it correctly.
///
/// Watched failing before it was trusted: with `do_requeue`'s unwrap mutated to pass the wrapped
/// bytes straight through (the double-wrap), this fails on the twin assertion — `event_clear` has
/// no row at all, because `db/020` could not unseal the body and skipped the custody block.
#[tokio::test]
async fn a_penned_sealed_record_releases_with_its_custody_and_the_body_opens() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(
        ok,
        "requeue must succeed\nstdout: {stdout}\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["released"], 1, "the penned record must be released: {m}");
    assert_eq!(m["still_quarantined"], 0, "nothing should stay held: {m}");

    // THE ASSERTION. Not "a row exists" — the clinician's chart is readable again.
    assert_eq!(
        twin_after_release(&c, &record).await.as_deref(),
        Some(record.twin.as_str()),
        "THE BODY MUST OPEN. A release that admits ciphertext whose key is gone is permanent, \
         silent loss of the chart at exit 0 — and on a restored solo node the pen was the last \
         copy in the world.\nstderr: {stderr}"
    );

    // And the custody the node now holds is re-wrapped for itself, so a later crypto-shred can
    // still reach this body. Stated after the twin because it is the trace, not the property.
    let stored: Vec<u8> = c
        .query_one(
            "SELECT d.dek_wrapped FROM event_dek d \
               JOIN event_log e ON e.event_id = d.event_id \
              WHERE e.content_address = $1",
            &[&record.digest],
        )
        .await
        .expect("the released event carries custody")
        .get(0);
    cairn_event::seal::unwrap_dek(&stored, &derived_unwrap_secret(&sk))
        .expect("the stored DEK must open with this node's own custody key");

    assert_eq!(pen_rows(&c).await, 0, "a released row leaves the pen");
}

// ---------------------------------------------------------------------------
// Arm 2 — no custody resolves at all: the event still comes back, sealed
// ---------------------------------------------------------------------------

/// **The anti-vacuity twin of arm 1, and `cmd_requeue`'s best-effort arm.**
///
/// `requeue` run under a DIFFERENT node's key: the derived secret does not match what
/// `node_unwrap_key` registered, so `resolve_at_startup` refuses and `cmd_requeue` degrades to
/// `None` rather than aborting. That degradation is deliberate and unlike `cmd_pull` (#554 review
/// finding 3): `requeue` is the recovery command a restore's own output points operators at, and a
/// recovery command that aborts before releasing anything is worse than one that releases without
/// custody.
///
/// ⚠️ **INVERTED BY #578, NOT DELETED.** This test used to assert `released: 1` and an empty pen:
/// the event came back sealed and the pen row — the only copy of its key — was deleted on the way.
/// That is issue #580's defect, and `cmd_requeue`'s own warning had been promising the opposite
/// ("the pen holds both halves until you do") one statement before it happened. The old assertions
/// are kept here in words because the reasoning they encoded is still half right: a recovery
/// command that ABORTS is worse than one that degrades. What changed is where the degradation
/// lands. It now costs a retained row and a second run, not a key nobody can ever get back.
///
/// Two things must both hold, and they pull in opposite directions: the EVENT must still be
/// recovered, and the KEY must not be spent to do it.
#[tokio::test]
async fn without_resolvable_custody_the_record_is_kept_rather_than_released() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (dir, _key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    // A stranger's key file: a real, well-formed signing key that is simply not this node's.
    let (stranger_sk, _kid) = cairn_event::generate_key().unwrap();
    let stranger_path = write_key_file(dir.path(), "stranger.key", &stranger_sk);

    let (ok, stdout, stderr) = run_requeue(&base, &stranger_path);
    assert!(
        ok,
        "a recovery command must still recover the EVENT when it cannot recover the KEY\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(
        m["custody_retained"], 1,
        "the key must be kept, not spent: {m}"
    );
    assert_eq!(
        m["released"], 0,
        "and nothing may be reported released: {m}"
    );

    // POSITIVE evidence, not the binary's own counter. `custody_retained` is incremented inside
    // the function under test; only the log says the record itself is actually back. THE EVENT
    // STILL COMES BACK — the apply door ran and admitted it. Only the pen row stays, so a later
    // run can finish what this one could not.
    assert!(
        event_survived(&c, &record).await,
        "the EVENT must be in the log. Retaining the pen row is about the KEY; a retention that \
         also withheld the record would be a different and worse behaviour.\nstderr: {stderr}"
    );
    assert_eq!(pen_rows(&c).await, 1, "the row holding the key must stay");

    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "with no custody the body must stay SEALED — if this reads back, arm 1 is passing for \
         some reason other than the custody arm and the whole file is vacuous"
    );
    // The RESOLUTION failed, which is a different arm from a DEK that would not open (arm 3).
    // `"WITHOUT custody"` alone cannot tell them apart — it appears in both messages — so this
    // asserts the fragment unique to `cmd_requeue`'s resolution failure.
    assert!(
        stderr.contains("custody key could not be resolved"),
        "the operator must be told they did not get custody, and WHY, or they will read a clean \
         release as a complete one: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Arm 3 — custody resolves, but not for THAT key
// ---------------------------------------------------------------------------

/// **A foreign medium: the pen holds a DEK wrapped for somebody else's node.**
///
/// The only arm that reaches `do_requeue`'s unwrap-failure branch. Arms 1 and 2 pass a good secret
/// and no secret respectively; here the secret is this node's own and correct, and it simply cannot
/// open this key.
///
/// ⚠️ **INVERTED BY #578, NOT DELETED.** This used to assert the row was released, on the reasoning
/// that a key belonging to another node is worth nothing here. That reasoning does not survive
/// contact with the situation: *"did not open with the key we have right now"* is not *"not ours"*,
/// and the operator may be holding the right `<key>.unwrap` on a USB stick they have not plugged
/// in — the #495 shape this whole path exists to survive. So the row is kept, and `db/021`'s
/// `acked` flag is how a human, not the code, finally decides the key is unrecoverable.
#[tokio::test]
async fn a_penned_dek_from_another_node_is_kept_until_a_human_decides_otherwise() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_dir, key_path, sk, record) = dead_node_with_a_penned_record(&mut c).await;

    // Re-wrap this record's real DEK for a stranger's custody key: byte-for-byte what a medium
    // written by another node carries. Every key here is generated at runtime (house rule 6).
    let dek = cairn_event::seal::unwrap_dek(&record.dek_wrapped, &derived_unwrap_secret(&sk))
        .expect("the fixture's own DEK opens with the node that sealed it");
    let stranger_secret =
        cairn_event::seal::generate_unwrap_secret().expect("mint a stranger's custody key");
    let foreign =
        cairn_event::seal::wrap_dek_for(&dek, &cairn_event::seal::unwrap_public(&stranger_secret))
            .expect("re-wrap for a stranger");
    pen(&c, &record, Some(&foreign)).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(
        ok,
        "a key this node cannot open is not a reason to lose the record\n\
         stdout: {stdout}\nstderr: {stderr}"
    );
    let m = metrics(&stdout, &stderr);
    assert_eq!(m["custody_retained"], 1, "the key must be kept: {m}");
    assert_eq!(m["released"], 0, "and nothing reported released: {m}");

    assert!(
        event_survived(&c, &record).await,
        "losing a record because its key belongs to another node would be the worst of both — \
         the event must be in the log.\nstderr: {stderr}"
    );
    assert_eq!(pen_rows(&c).await, 1, "the row holding the key must stay");

    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "a DEK that does not open must not somehow produce a clear view"
    );
    // ONE line must carry both the phrase and the record, for the reason in `stderr_line_with`:
    // the success line names this same record on this same run, so two separate `contains` checks
    // over the whole stream would still pass with the digest stripped out of THIS message.
    let failure_line = stderr_line_with(&stderr, "did not open with the custody key");
    assert!(
        failure_line.contains(&hex::encode(&record.digest)[..8]),
        "the unwrap failure must name WHICH record ON ITS OWN LINE, or an operator reading a \
         multi-record run cannot act on it. Line was: {failure_line}"
    );
}

// ---------------------------------------------------------------------------
// Arm 4 — a KEYLESS pen row while custody resolves: the quiet, modal case
// ---------------------------------------------------------------------------

/// **The input pair `do_requeue`'s `_` arm absorbs in silence, and the one that is NOT a disaster.**
///
/// `sync_quarantine.dek_wrapped` is nullable (`db/052`), and every pen row an ordinary `pull`
/// creates for a PLAINTEXT event has it NULL. So `(Some(secret), None)` — a working custody key
/// over a keyless row — is not an exotic combination; outside disaster recovery it is the modal
/// one, and until this test nothing reached it: arms 1 and 3 pen a DEK, arms 2 and 5 resolve no
/// secret.
///
/// What must hold is mostly NEGATIVE, and that is the point. The event releases, no clear view
/// appears (there is no key and the event is sealed), and — the assertion with teeth — **neither
/// custody warning is printed**, because nothing went wrong. An operator requeueing an ordinary
/// pen must not be told their custody failed.
///
/// Watched failing before it was trusted (mutation 6): collapsing the match to
/// `(Some(secret), dek) => unwrap_dek(dek.as_deref().unwrap_or(&[]), secret)` makes every keyless
/// row report a custody failure, and this test fails on the "no false alarm" assertion. The
/// `.expect()` variant of the same slip panics mid-loop and discards the partial-completion report
/// #471 exists to preserve; that fails here too, on `assert!(ok)`.
#[tokio::test]
async fn a_keyless_pen_row_releases_quietly_and_raises_no_custody_alarm() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (_dir, key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    // The node's own key resolves; the PEN simply carries no DEK.
    pen(&c, &record, None).await;

    let (ok, stdout, stderr) = run_requeue(&base, &key_path);
    assert!(
        ok,
        "an ordinary keyless pen row is not a failure\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(metrics(&stdout, &stderr)["released"], 1);
    assert!(
        event_survived(&c, &record).await,
        "the event must be recovered\nstderr: {stderr}"
    );
    assert_eq!(pen_rows(&c).await, 0, "a released row leaves the pen");
    assert_eq!(
        twin_after_release(&c, &record).await,
        None,
        "a sealed event whose pen carried no key cannot gain a clear view from nowhere"
    );

    // THE ASSERTION. A keyless row is not a custody failure, and reporting one here would train
    // operators to ignore the message on the run where it is real.
    assert!(
        !stderr.contains("did not open with this node's custody key"),
        "FALSE ALARM: a pen row that never carried a DEK was reported as one that failed to \
         open. An operator who sees this on every ordinary requeue stops reading it — and the \
         run where custody genuinely was lost is the one it then hides.\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("custody key could not be resolved"),
        "the key resolved fine; only the pen row was keyless: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// The plumbing guard
// ---------------------------------------------------------------------------

/// **`requeue` must never MINT a signing key, however wrong the `--key` path is.**
///
/// The comment INSIDE `cmd_requeue` — it has no `///`, so this does not appear in `cargo doc`;
/// look just above its `let custody = match load_existing_key(...)` — names this as the reason it
/// calls `load_existing_key` rather than the
/// `load_or_create_key` the pull path uses: an operator running `requeue` from the wrong directory
/// would otherwise create a stray signing key and then resolve custody against it — which is not
/// merely useless but actively misleading, since the resulting node has a key file that belongs to
/// nothing. Nothing checked it.
///
/// The event must STILL come back, and its key must still be kept: the same reasoning as arm 2,
/// which #578 inverted in the same way. What this arm is ABOUT is unchanged — no key file may be
/// minted, whatever else happens.
#[tokio::test]
async fn requeue_refuses_a_missing_key_file_rather_than_minting_one() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (dir, _key_path, _sk, record) = dead_node_with_a_penned_record(&mut c).await;
    pen(&c, &record, Some(&record.dek_wrapped)).await;

    // ⚠️ THE PATH MUST BE IN AN EXISTING DIRECTORY, and this is the whole difficulty of the test.
    // The first draft named a file inside a subdirectory that did not exist either — and MUTATION
    // TESTING CAUGHT IT: swapping `load_existing_key` for the minting `load_or_create_key` left
    // this test GREEN, because the mint failed on the missing parent directory rather than being
    // refused, so the assertion below held for a reason that had nothing to do with the guard.
    // An operator in the wrong directory is in a directory that EXISTS; that is the case to model.
    let absent = dir.path().join("wrong-directory-node.key");
    assert!(!absent.exists(), "the fixture path must start absent");
    let absent_path = absent.to_str().unwrap().to_string();

    let (ok, stdout, stderr) = run_requeue(&base, &absent_path);
    assert!(
        ok,
        "the recovery command still recovers the event\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert_eq!(metrics(&stdout, &stderr)["custody_retained"], 1);
    assert!(
        event_survived(&c, &record).await,
        "the recovery command still recovers the EVENT, whatever happened to the key\nstderr: {stderr}"
    );
    assert_eq!(pen_rows(&c).await, 1, "the row holding the key must stay");
    assert!(
        !absent.exists(),
        "requeue MINTED a signing key at {} — an operator in the wrong directory now has a key \
         file that belongs to nothing, and custody resolved against it",
        absent.display()
    );
    assert!(
        stderr.contains("custody key could not be resolved"),
        "and it must say custody was not obtained, and why: {stderr}"
    );
}
