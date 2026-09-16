//! DR slice 2d, design tests 19 and 22 (#593): **a restore that goes wrong still leaves the
//! operator somewhere to stand** — driven through the real binary.
//!
//! - **Test 19 — the ceremony's ORDER is load-bearing, shown by behaviour.** `finalize_identity`
//!   runs LAST, so a clinical apply that fails catastrophically leaves `local_node` EMPTY and the
//!   same database restores again to completion. Under the "minimal reordering" design §3 rejects
//!   (identity minted before the clinical apply), the same failure leaves a node already
//!   identity-minted and fenced, whose only way out is a fresh database. `restore_ceremony_order.rs`
//!   pins the order by SOURCE POSITION; nothing had shown it by what happens to a database.
//! - **Test 22 — the summary survives the failure.** A restore that pens records exits non-zero,
//!   but only after printing the `new node` / `supersedes` / `re-peer with …` lines, which are the
//!   operator's next step, and after counting each refusal BY REASON — including a Rust-side unwrap
//!   failure that never reached the door and so has no door text to quote.

use cairn_event::{generate_key, sign, Hlc};
use cairn_medium::{MediumRecord, Plane};
use cairn_node::{db, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    a_recovery_code, an_op_passphrase, author_sealed_clinical_event, capture,
    clinical_record_count, cs, in_event_log, keyless_record, medium_with_export,
    old_recovery_code_file, provisioned_clinic, restore_cli, rewrite_medium, sealed_assert_body,
    twin_of, wipe_to_a_fresh_dr_machine, write_export_beside, ExportCustody,
};

/// How many `local_node` rows exist — zero means no identity has been minted on this database.
async fn identities(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM local_node", &[])
        .await
        .unwrap()
        .get(0)
}

/// How many of `patient`'s events this node's `event_log` holds.
async fn events_for(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A validly signed, keyless clinical record for `patient`, signed by a key that no actor
/// registry on any medium ever enrolled — so the apply door refuses it, with door text.
fn a_record_signed_by_a_stranger(patient: Uuid, source_seq: i64) -> MediumRecord {
    let (stranger, stranger_kid) = generate_key().unwrap();
    let hlc = Hlc {
        wall: 3,
        counter: 0,
        node_origin: "stranger".into(),
    };
    let event_id = Uuid::now_v7().to_string();
    let (body, _dek) = sealed_assert_body(&stranger_kid, patient, &event_id, "forged", hlc);
    keyless_record(sign(&body, &stranger).unwrap().signed_bytes, source_seq)
}

/// Make inserting `event_id` into `event_log` fail as a FULL DISK would (SQLSTATE 53100).
///
/// Deliberately not a door verdict: a bare `RAISE EXCEPTION` (P0001) is how db/020 refuses a
/// record, and the restore pens those and carries on. Anything else is this node's own machine,
/// and the restore stops — which is the catastrophic failure test 19 needs. Scoped to ONE event id
/// so a trigger that somehow survived could never fire on anything else, and removed by
/// [`remove_crash`] before the test asserts anything (`attachment_reference_shape.rs`'s pattern).
async fn crash_on_insert_of(c: &Client, event_id: &str) {
    let event_id: Uuid = event_id.parse().unwrap();
    c.batch_execute(&format!(
        "CREATE OR REPLACE FUNCTION cairn_test_restore_crash() RETURNS trigger \
         LANGUAGE plpgsql AS $f$ BEGIN \
             IF NEW.event_id = '{event_id}'::uuid THEN \
                 RAISE EXCEPTION 'injected: the disk filled mid-restore' USING ERRCODE = '53100'; \
             END IF; RETURN NEW; END $f$;
         DROP TRIGGER IF EXISTS cairn_test_restore_crash_trg ON event_log;
         CREATE TRIGGER cairn_test_restore_crash_trg BEFORE INSERT ON event_log \
             FOR EACH ROW EXECUTE FUNCTION cairn_test_restore_crash();"
    ))
    .await
    .unwrap();
}

async fn remove_crash(c: &Client) {
    c.batch_execute(
        "DROP TRIGGER IF EXISTS cairn_test_restore_crash_trg ON event_log; \
         DROP FUNCTION IF EXISTS cairn_test_restore_crash();",
    )
    .await
    .unwrap();
}

/// **Test 19.** Mutation this kills: `finalize_identity` moved above the clinical apply — the
/// first, failed run then leaves an identity behind, and the database can no longer be restored
/// into.
///
/// Three runs, because that is what an operator meets today. Attempt 1 crashes mid-apply.
/// Attempt 2 is the retry the crash message invites, and the pre-flight refuses it: attempt 1
/// installed the dead node's custody key, and `restore` will not run over an existing one. The
/// operator moves that leftover aside, as the refusal says, and attempt 3 completes. (That the crash
/// message itself does not mention the leftover is #596.) Attempt 3 also exercises design test 9's
/// resume path, which the design asks this test to run "with 9": the registry attempt 1 installed
/// is found present and only the remainder is inserted.
#[tokio::test]
async fn a_restore_that_crashes_mid_apply_mints_no_identity_and_restores_again_in_place() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    remove_crash(&c).await; // reset-at-start: never trust a predecessor's cleanup (#583)
    let dir = tempfile::tempdir().unwrap();
    let (op, code) = (an_op_passphrase(19), a_recovery_code(19));

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart = author_sealed_clinical_event(&c, &sk, &kid).await;
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
    let code_file = old_recovery_code_file(dir.path(), &code);
    let key = dir.path().join("restored.key");

    // ATTEMPT 1 — the disk fills while the chart is being applied.
    crash_on_insert_of(&c, &chart.event_id).await;
    let first = restore_cli(&base, &key, &medium, Some(&code_file));
    remove_crash(&c).await;
    let stderr = String::from_utf8_lossy(&first.stderr);

    // **`Some(1)`, not merely non-zero (PR #612 review).** Until #594 those were the same claim;
    // since ADR-0071 gave the command a 3, `!success()` passes under either verdict and this
    // assertion stopped discriminating. A database fault is an ADR-named exit-**1** cause — the
    // ceremony was BLOCKED, nothing is recoverable by `requeue` — and until this line it was the
    // one such cause with no test pinning its number. A regression routing it through the verdict
    // instead of `?` would tell a cron wrapper "incomplete, run requeue" about a hard failure.
    assert_eq!(
        first.status.code(),
        Some(1),
        "a restore whose clinical apply hit a LOCAL database fault must exit 1 (FAILED), not 3 \
         (INCOMPLETE): nothing here is finishable by `cairn-sync requeue`; stderr:\n{stderr}"
    );
    // The door's error carries the server's message and SQLSTATE (`legible_db_error`), so the
    // failure is pinned to THIS trigger. "LOCAL fault" alone is printed by four different steps of
    // the clinical apply, including a registry read that fails before any record is offered.
    assert!(
        stderr.contains("applying a clinical record failed on THIS NODE's database")
            && stderr.contains("injected: the disk filled mid-restore [53100]"),
        "anti-vacuity: it must have failed on the injected fault, inside the clinical apply — not \
         earlier for some unrelated reason; stderr:\n{stderr}"
    );
    assert!(
        events_for(&c, chart.patient).await > 0,
        "anti-vacuity: the crash came MID-apply — the chart's registration, which sorts before the \
         chart, was already applied, so a retry has partial state to resume over"
    );
    assert!(
        !in_event_log(&c, &chart.event_id).await,
        "anti-vacuity: the chart itself really was not applied"
    );
    // THE ASSERTION. No identity was minted, so this database is still a restore target.
    assert_eq!(
        identities(&c).await,
        0,
        "a failed clinical apply must leave NO identity behind — finalize_identity runs last, so \
         the whole restore stays inside the un-enrolled fence; stderr:\n{stderr}"
    );

    // ATTEMPT 2 — the retry the crash message invites, over the key attempt 1 installed.
    let leftover = keystore::unwrap_key_path_for(&key);
    assert!(
        leftover.exists(),
        "anti-vacuity: attempt 1 got as far as installing custody before the clinical apply"
    );
    let refused = restore_cli(&base, &key, &medium, Some(&code_file));
    let stderr = String::from_utf8_lossy(&refused.stderr);
    // `Some(1)` for the same reason as attempt 1 (PR #612 review): a PRE-FLIGHT refusal is the
    // purest BLOCKED ceremony there is — it happens before a single byte is written, so there is
    // nothing partial to be incomplete ABOUT. Reporting it as 3 would invite a wrapper to run
    // `requeue` against a restore that never started.
    assert_eq!(
        refused.status.code(),
        Some(1),
        "the pre-flight refusal must exit 1 (FAILED), not 3 — nothing ran, so nothing is \
         partially recovered; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("leftover of an earlier attempt"),
        "and it must name the leftover case and its remedy; stderr:\n{stderr}"
    );
    assert_eq!(
        identities(&c).await,
        0,
        "and refuse before it touches anything — the database is still a restore target"
    );

    // ATTEMPT 3 — the operator moves the leftover aside, as the refusal says, and retries: the same
    // medium, the same export, the SAME database.
    std::fs::rename(&leftover, dir.path().join("attempt-1.unwrap")).unwrap();
    let third = restore_cli(&base, &key, &medium, Some(&code_file));
    let stdout = String::from_utf8_lossy(&third.stdout);
    let stderr = String::from_utf8_lossy(&third.stderr);
    assert!(
        third.status.success(),
        "the same database must restore again to completion; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert_eq!(
        twin_of(&c, &chart.event_id).await.as_deref(),
        Some(chart.twin.as_str()),
        "and the chart comes back readable"
    );
    assert_eq!(
        identities(&c).await,
        1,
        "with exactly one identity, minted once"
    );
    // It RESUMED rather than starting over: both the registry and the clinical apply found what
    // attempt 1 had already written.
    assert!(
        stdout.contains("(the rest were already present — a resumed restore)")
            && stdout.contains(" 1 already present,"),
        "the registry install and the clinical apply both resume over attempt 1's partial state, \
         and say so; stdout:\n{stdout}"
    );
}

/// **Test 22.** Two nights: chart A, then chart B. B's segment is then rewritten (unsigned, as a
/// keyless capture writes one) so that B's wrapped DEK is damaged and TWO records signed by a
/// stranger's key ride beside it. A restores; B's registration restores; the three bad records are
/// refused for two DIFFERENT reasons, counted 1 and 2 — unequal on purpose, so a count printed
/// against the wrong reason cannot pass.
///
/// Mutations this kills, each at the assertion that names it:
/// - the pen's `bail!` moved up to sit between the per-reason loop and the `new node` /
///   `supersedes` / `re-peer` lines — the next-step assertion fails. (Moved above the WHOLE
///   summary instead, it fails earlier — at the clinical summary line — and so proves nothing
///   about the next steps; that placement was tried first and is why this one is named.)
/// - the per-reason loop emptied — the reason lines vanish.
#[tokio::test]
async fn a_restore_that_pens_records_prints_its_next_steps_and_counts_each_reason() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (op, code) = (an_op_passphrase(22), a_recovery_code(22));

    let (sk, kid) = provisioned_clinic(&c).await;
    let a = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;
    let b = author_sealed_clinical_event(&c, &sk, &kid).await;
    capture(&c, &sk, &kid, dir.path()).await;

    let on_the_plane = rewrite_medium(&medium, |segments| {
        let last = segments.last_mut().expect("two captures wrote segments");
        assert_eq!(
            last.plane,
            Plane::Clinical,
            "the second night's last segment is clinical"
        );
        // Two separate reasons for dropping the signature. The capture's attestation signs these
        // exact records, so once they are edited it would fail verification (`AttestationInvalid`)
        // and this segment would be gated out before its records were offered. And unsigned is a
        // legitimate state: it is what a capture taken without the signing key writes. The chain
        // stays intact for a third reason — this is the LAST segment, so no later link commits to
        // the records being changed.
        last.attestation = None;
        let chart = last
            .records
            .iter_mut()
            .find(|r| r.signed_bytes == b.signed_bytes)
            .expect("the second night captured chart B");
        let wrapped = chart.dek_wrapped.as_mut().expect("chart B carries custody");
        let tail = wrapped.len() - 1;
        wrapped[tail] ^= 0xFF; // the key no longer opens: a Rust-side refusal, no door text

        let next_seq = last.records.iter().map(|r| r.source_seq).max().unwrap() + 1;
        for n in 0..2 {
            last.records
                .push(a_record_signed_by_a_stranger(b.patient, next_seq + n));
        }
        clinical_record_count(segments)
    });
    write_export_beside(&c, &sk, &medium, &op, &code, ExportCustody::Carried).await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = old_recovery_code_file(dir.path(), &code);
    let out = restore_cli(
        &base,
        &dir.path().join("restored.key"),
        &medium,
        Some(&code_file),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // #594/ADR-0071: 3 (INCOMPLETE), and the count now rides the VERDICT rather than an
    // `anyhow::bail!`. The old assertion matched `"Error: 3 clinical record(s) were refused"` —
    // the `Error:` prefix being `main`'s `Termination`, which is exactly the claim the restore's
    // own message then had to apologise for ("this exit code says the restore is INCOMPLETE, not
    // that it failed"). The apology is gone because the status now says it.
    assert_eq!(
        out.status.code(),
        Some(3),
        "a restore holding records in the pen is INCOMPLETE (3), not FAILED (1), and a script \
         must see that; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("restore: INCOMPLETE") && stderr.contains("3 clinical record(s) are HELD"),
        "and the verdict names how many, without calling the run an Error; stderr:\n{stderr}"
    );
    assert_eq!(
        twin_of(&c, &a.event_id).await.as_deref(),
        Some(a.twin.as_str()),
        "anti-vacuity: everything else on the medium really did restore"
    );
    // The whole clinical line, with numbers counted by hand from the medium: everything that was
    // not one of the three bad records applied — A's registration and chart, and B's registration.
    let applied = on_the_plane - 3;
    assert!(
        stdout.lines().any(|line| line
            == format!(
                "clinical records: {applied} applied, 0 already present, 3 refused (of \
                 {on_the_plane} on the medium)"
            )),
        "every good record on the medium restores beside the refused ones; stdout:\n{stdout}"
    );

    // COUNTED BY REASON, each with its own legible cause. Whole lines, so a count of 1 cannot
    // match a 10.
    for (reason, why) in [
        (
            "  refused — custody key would not open this record's DEK: 1",
            "a Rust-side unwrap failure never reaches the door, so its reason is the restore's own",
        ),
        (
            "  refused — refused by the apply door: 2",
            "door refusals are grouped under one label and counted, not listed per event",
        ),
    ] {
        assert!(
            stdout.lines().any(|line| line == reason),
            "{why}; stdout:\n{stdout}"
        );
    }

    // THE NEXT STEPS SURVIVE THE FAILURE.
    for next_step in [
        "new node ",
        "supersedes ",
        "re-peer with `cairn-node pair-offer`",
    ] {
        assert!(
            stdout.lines().any(|line| line.starts_with(next_step)),
            "`{next_step}` is the operator's next step and must print BEFORE the non-zero exit, \
             never be lost to it; stdout:\n{stdout}"
        );
    }
}
