//! #614 — a record this build cannot CLASSIFY is no longer silent.
//!
//! # What was silent
//!
//! `db/020_apply_remote_event.sql` admits an event whose `event_type` is absent from
//! `event_type_class` *uninterpreted*: its own words are "It yields NO projection rows and
//! confers NO power". It returns `Ok`. Custody is orthogonal to classification, so the restore's
//! custody check passed too, and `apply_clinical_plane` counted the record `applied` — which, in
//! the log, it genuinely is. The summary then said `N applied, 0 already present, 0 refused (of
//! N on the medium)` with every `Unrestored` field zero, and the run exited **0**.
//!
//! Per ADR-0012's additive schema evolution, a NEW CLINICAL EVENT TYPE is the case that actually
//! happens; a whole new PLANE — which ADR-0071 gives exit 3 — is rare. The realistic DR box, a
//! spare laptop one release behind the live node, hits this one and not that one.
//!
//! # ⚠️ The verdict deliberately does not move
//!
//! The record IS in this node's log, which is exactly what ADR-0071's exit-0 rule claims, and
//! `connect_and_load_schema` re-adjudicates deferred events — so an upgrade heals this with
//! nothing left on the medium and no second restore. The defect #614 names is the SILENCE.
//!
//! This is the decision `Unrestored`'s doc asks for ("the field set is closed at five … a sixth
//! cause gets a deliberate decision, not a silent widening"), and the answer is **no**:
//! `restore_exit_vocabulary.rs::the_cause_list_is_exactly_five` stays green, `is_complete()` is
//! untouched, and a medium of unclassifiable records still exits 0 — now saying so.

use cairn_node::restore::clinical::deferred_notice;

#[test]
fn a_clean_restore_says_nothing_about_deferral() {
    assert_eq!(
        deferred_notice(0),
        None,
        "a restore with nothing deferred must print no line at all — an operator who reads a \
         deferral note on every clean run stops reading it, and then misses the one that matters"
    );
}

#[test]
fn the_notice_names_the_count_the_remedy_and_the_command() {
    let n = deferred_notice(7).expect("a nonzero deferral must be reported");
    assert!(
        n.contains('7'),
        "the operator needs the NUMBER, not just the fact — it is what they check the chart \
         against: {n}"
    );
    assert!(
        n.contains("cairn-node deferred"),
        "the notice must name the subcommand that LISTS them, or it tells an operator that a \
         problem exists and not how to look at it: {n}"
    );
    assert!(
        n.contains("upgrade"),
        "the remedy is to upgrade this node, and it must be IN the text rather than inferrable \
         from it: {n}"
    );
    assert!(
        n.contains("no second restore"),
        "the notice must say the medium holds nothing back. Without it an operator reading a \
         deferral note will reasonably re-run the whole ceremony hunting for records that are \
         already here — the wrong act, on the day they can least afford it: {n}"
    );
}

// ---------------------------------------------------------------------------------------------
// The DB-gated arm: a medium from a newer Cairn, end to end.
// ---------------------------------------------------------------------------------------------

use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_medium::{MediumRecord, Plane};
use cairn_node::db;
use cairn_node::restore::clinical::apply_clinical_plane;
use cairn_node::restore::completeness::Unrestored;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    a_recovery_code, an_op_passphrase, chained_segment, cs, medium_with_export,
    old_recovery_code_file, provisioned_clinic, restore_cli, rewrite_medium,
    wipe_to_a_fresh_dr_machine, ExportCustody,
};

/// An `event_type` no build classifies. Named for what it is, so a reader of a failure knows
/// immediately that the type is *meant* to be unknown and has not merely been forgotten.
const UNKNOWN_TYPE: &str = "clinical.from.a.newer.cairn";

/// A fresh database with one enrolled actor — the restore-side precondition.
///
/// `TRUNCATE event_log … CASCADE` clears `event_deferred` through its FK, which matters more here
/// than anywhere else: `deferred_count` counts every row whose type is unclassified, so one left
/// by a predecessor in this shared database would be counted as this medium's. The de-classify is
/// what makes `UNKNOWN_TYPE` unclassified in the first place — it is also idempotent, so it
/// repairs the database after a predecessor that died mid-test, which is the #296 lesson and the
/// reason it is done HERE rather than in a cleanup a panic would skip.
///
/// Returns the enrolled signing key and its key-id — the record must be signed by an ENROLLED
/// key, because every clinical apply door resolves its author through `actor_current` and would
/// otherwise refuse the record as an unenrolled signer, penning it instead of deferring it.
async fn a_fresh_dr_machine_with_a_registry(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag, \
         t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = $1",
        &[&UNKNOWN_TYPE],
    )
    .await
    .unwrap();
    // Derived at runtime, never a byte literal (house rule 6a / #146).
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// One plaintext clinical record of `ty`, as it would sit on a medium written by a newer node.
///
/// Plaintext deliberately: a sealed body would drag the whole custody dance into a test whose
/// subject is CLASSIFICATION, and db/020 treats the two orthogonally — which is the very reason
/// the custody check above this one in `apply_clinical_plane` did not catch #614.
///
/// No authored twin: the mechanical skeleton carries it (ADR-0039), which is principle 11 doing
/// its job for a type this build has never heard of.
fn medium_record_of_type(sk: &SigningKey, kid: &str, ty: &str, source_seq: i64) -> MediumRecord {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: ty.into(),
        schema_version: "future/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "upgraded-peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"reason": "a field this build has no code for"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    MediumRecord {
        signed_bytes: sign(&body, sk).unwrap().signed_bytes,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq,
    }
}

/// A medium carrying a newer Cairn's event type is REPORTED, and leaves no `Unrestored` cause.
///
/// ⚠️ **What this does NOT pin, despite an earlier version of this comment saying it did.** It
/// claimed that a test checking only the line "would stay green if someone added a sixth
/// `Unrestored` cause" — and then asserted `Unrestored::default().is_complete()`, which never
/// touches `report` and is true of an all-zero struct no matter how many fields it has. That was a
/// tautology wearing a discrimination's clothes. **The property is pinned elsewhere and properly**:
/// `restore_exit_vocabulary.rs::the_cause_list_is_exactly_five` builds a FULL struct literal, so a
/// sixth field is an E0063 compile error there (mutation M8 confirms it).
///
/// What this test pins is the pair on THIS run's report: the record is admitted, counted deferred,
/// produces an operator line, and the `Unrestored` this run actually builds is complete.
#[tokio::test]
async fn an_unclassifiable_type_is_reported_and_still_exits_zero() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = a_fresh_dr_machine_with_a_registry(&c).await;

    let records = vec![medium_record_of_type(&sk, &kid, UNKNOWN_TYPE, 1)];
    let report = apply_clinical_plane(&c, &records, None)
        .await
        .expect("a type this build cannot classify is admitted, never a failure of the run");

    // The premise, asserted before anything else: the record must be ADMITTED, not refused.
    // Without this the test would pass just as happily over a door that penned the record — and
    // would then be proving the opposite of what its name claims.
    assert_eq!(
        report.penned(),
        0,
        "the record must be ADMITTED uninterpreted, not refused — if it is penned, this test \
         proves nothing about #614: {report:?}"
    );
    assert_eq!(
        report.applied, 1,
        "and counted as applied, which in the log it honestly is: {report:?}"
    );

    assert_eq!(
        report.deferred, 1,
        "the unclassifiable record must be COUNTED as deferred — this count is the whole of \
         #614's fix: {report:?}"
    );
    assert!(
        deferred_notice(report.deferred).is_some(),
        "and it must produce an operator line, or the count reaches nobody"
    );

    // The verdict does not move — asserted over the `Unrestored` THIS RUN produces, built from
    // this report the way `main.rs` builds it, rather than over `Unrestored::default()` (which
    // would be true of an all-zero struct regardless of what the run did).
    let unrestored = Unrestored {
        past_chain_break: 0,
        unknown_plane: 0,
        torn_tail: false,
        penned: report.penned(),
        no_registry: report.skipped_no_registry,
    };
    assert!(
        unrestored.is_complete(),
        "a deferred record is deliberately not a sixth Unrestored cause (ADR-0072): it is in the \
         log, and an upgrade heals it with no second restore. Got: {unrestored:?}"
    );
    assert_eq!(
        unrestored.notice(),
        None,
        "and a complete verdict prints no INCOMPLETE notice — the deferred line is the summary's, \
         not the verdict's"
    );
}

// ---------------------------------------------------------------------------------------------
// The CLI arm: the one line that makes any of this reach an operator.
// ---------------------------------------------------------------------------------------------

/// The deferred line actually reaches a scripted restore's **stdout**, and the run still exits 0.
///
/// ⚠️ **Why this exists even though the count and the notice are both already tested.** They are
/// tested *apart from the wiring*: `deferred_notice` is unit-tested pure, `report.deferred` is
/// asserted end to end against a real database, and the mutation that strips the notice's text is
/// caught by the pure test. **Nothing exercised the call site.** Deleting the whole
/// `if let Some(line) = deferred_notice(...) { println!("{line}") }` block from `main.rs` restores
/// #614 exactly — an operator reads `N applied` at exit 0 and hears nothing — and every other test
/// in this file stays green. A fix that reaches no operator is not a fix, so the last hop gets its
/// own spawned-binary test. (PR #618 review, finding I4.)
///
/// The unclassifiable record is injected by rewriting the medium rather than authored locally:
/// db/005 deliberately fails CLOSED on a type it cannot classify — *a node may CARRY a type it has
/// no code for, never AUTHOR one* — so a medium written by a newer Cairn is the only shape this
/// case has, and rewriting the file is how the sibling suites build one.
#[tokio::test]
async fn the_deferred_line_reaches_a_scripted_restores_stdout() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    // Belt and braces with the kit's own wipe: this count is scoped to unclassified types, and
    // UNKNOWN_TYPE must be unclassified for the run to mean anything (#296's lesson — repair at
    // the START, because a cleanup at the end does not survive a panic).
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = $1",
        &[&UNKNOWN_TYPE],
    )
    .await
    .expect("de-classifying the fixture type must succeed, or this test proves nothing");

    let op = an_op_passphrase(1);
    let code = a_recovery_code(1);
    let (sk, kid) = provisioned_clinic(&c).await;
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

    // A record of a type this build cannot classify, appended as its own correctly-chained
    // clinical segment — what a medium written by a newer Cairn looks like to this one.
    rewrite_medium(&medium, |segments| {
        let appended = chained_segment(
            segments,
            Plane::Clinical,
            vec![medium_record_of_type(&sk, &kid, UNKNOWN_TYPE, 9_000)],
        );
        segments.push(appended);
    });

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = old_recovery_code_file(dir.path(), &code);
    let new_key = dir.path().join("restored.key");
    let out = restore_cli(&base, &new_key, &medium, Some(&code_file));

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // Exit FIRST and explicitly: the whole decision is that this state is exit 0, so `success()`
    // here is an assertion about ADR-0072 decision 3 and not incidental.
    assert_eq!(
        out.status.code(),
        Some(0),
        "a medium whose only oddity is an unclassifiable TYPE leaves nothing behind, so the run \
         exits 0 (ADR-0072 decision 3); stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("cannot classify") && stdout.contains("cairn-node deferred"),
        "the deferred line must reach STDOUT — this is the hop that #614 was missing, and the \
         only thing that makes the count reach a human; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
