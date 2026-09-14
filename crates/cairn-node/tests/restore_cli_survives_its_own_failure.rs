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
use cairn_medium::Plane;
use cairn_node::{db, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    a_recovery_code, an_op_passphrase, author_sealed_clinical_event, capture, cs, in_event_log,
    medium_with_export, provisioned_clinic, restore_cli, rewrite_medium, sealed_assert_body,
    twin_of, wipe_to_a_fresh_dr_machine, write_export_beside, ExportCustody,
};

/// How many `local_node` rows exist — zero means no identity has been minted on this database.
async fn identities(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM local_node", &[])
        .await
        .unwrap()
        .get(0)
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
    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, &code).unwrap();
    let key = dir.path().join("restored.key");

    // ATTEMPT 1 — the disk fills while the chart is being applied.
    crash_on_insert_of(&c, &chart.event_id).await;
    let first = restore_cli(&base, &key, &medium, Some(&code_file));
    remove_crash(&c).await;
    let stderr = String::from_utf8_lossy(&first.stderr);

    assert!(
        !first.status.success(),
        "a restore whose clinical apply failed must exit non-zero; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("LOCAL fault"),
        "anti-vacuity: it must have failed INSIDE the clinical apply, on the injected fault — not \
         earlier for some unrelated reason; stderr:\n{stderr}"
    );
    assert!(
        !in_event_log(&c, &chart.event_id).await,
        "anti-vacuity: the chart really was not applied"
    );
    // THE ASSERTION. No identity was minted, so this database is still a restore target.
    assert_eq!(
        identities(&c).await,
        0,
        "a failed clinical apply must leave NO identity behind — finalize_identity runs last, so \
         the whole restore stays inside the un-enrolled fence; stderr:\n{stderr}"
    );

    // The operator's retry, following the pre-flight's own instruction: attempt 1 installed the
    // dead node's custody key beside the new key, and `restore` refuses to run over an existing
    // unwrap key ("if it is the leftover of an earlier attempt at THIS restore, move it aside").
    let leftover = keystore::unwrap_key_path_for(&key);
    assert!(
        leftover.exists(),
        "anti-vacuity: attempt 1 got as far as installing custody before the clinical apply"
    );
    std::fs::rename(&leftover, dir.path().join("attempt-1.unwrap")).unwrap();

    // ATTEMPT 2 — the same medium, the same export, the SAME database.
    let second = restore_cli(&base, &key, &medium, Some(&code_file));
    let stdout = String::from_utf8_lossy(&second.stdout);
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        second.status.success(),
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
}

/// **Test 22.** Two nights: chart A, then chart B. B's segment is then rewritten (unsigned, as a
/// keyless capture writes one) so that B's wrapped DEK is damaged and a record signed by a
/// stranger's key rides beside it. A restores; B's registration restores; the two bad records are
/// refused for two DIFFERENT reasons.
///
/// Mutations this kills: the pen's `bail!` moved above the summary (the next-step lines vanish);
/// the per-reason loop deleted (the counts vanish).
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

    // A stranger: a key no actor registry on this medium ever enrolled.
    let (stranger, stranger_kid) = generate_key().unwrap();
    rewrite_medium(&medium, |segments| {
        let last = segments.last_mut().expect("two captures wrote segments");
        assert_eq!(
            last.plane,
            Plane::Clinical,
            "the second night's last segment is clinical"
        );
        // Unsigned, so editing its records leaves the chain intact rather than turning this into
        // an attestation failure the trust gate would stop before any record is offered.
        last.attestation = None;
        let chart = last
            .records
            .iter_mut()
            .find(|r| r.signed_bytes == b.signed_bytes)
            .expect("the second night captured chart B");
        let wrapped = chart.dek_wrapped.as_mut().expect("chart B carries custody");
        let tail = wrapped.len() - 1;
        wrapped[tail] ^= 0xFF; // the key no longer opens: a Rust-side refusal, no door text

        let event_id = Uuid::now_v7().to_string();
        let hlc = Hlc {
            wall: 3,
            counter: 0,
            node_origin: "stranger".into(),
        };
        let (body, _dek) = sealed_assert_body(&stranger_kid, b.patient, &event_id, "forged", hlc);
        let next_seq = last.records.iter().map(|r| r.source_seq).max().unwrap() + 1;
        last.records.push(cairn_medium::MediumRecord {
            signed_bytes: sign(&body, &stranger).unwrap().signed_bytes,
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: next_seq,
        });
    });
    write_export_beside(&c, &sk, &medium, &op, &code, ExportCustody::Carried).await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = dir.path().join("old-recovery-code");
    std::fs::write(&code_file, &code).unwrap();
    let out = restore_cli(
        &base,
        &dir.path().join("restored.key"),
        &medium,
        Some(&code_file),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a restore holding records in the pen is INCOMPLETE, and a script must see that; \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("2 clinical record(s) were refused"),
        "and the failure names how many; stderr:\n{stderr}"
    );
    assert_eq!(
        twin_of(&c, &a.event_id).await.as_deref(),
        Some(a.twin.as_str()),
        "anti-vacuity: everything else on the medium really did restore"
    );

    // COUNTED BY REASON, each with its own legible cause.
    for (reason, why) in [
        (
            "  refused — custody key would not open this record's DEK: 1",
            "a Rust-side unwrap failure never reaches the door, so its reason is the restore's own",
        ),
        (
            "  refused — refused by the apply door: 1",
            "a door refusal is grouped under one label, not listed per event",
        ),
    ] {
        assert!(stdout.contains(reason), "{why}; stdout:\n{stdout}");
    }

    // THE NEXT STEPS SURVIVE THE FAILURE.
    for next_step in [
        "new node ",
        "supersedes ",
        "re-peer with `cairn-node pair-offer`",
    ] {
        assert!(
            stdout.contains(next_step),
            "`{next_step}` is the operator's next step and must print BEFORE the non-zero exit, \
             never be lost to it; stdout:\n{stdout}"
        );
    }
}
