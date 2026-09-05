//! Task 6 of #500 slice 2c — `capture::read_clinical_page`, `capture::read_node_page`, and
//! `capture::to_medium_record`.
//!
//! This suite proves the two things a backup medium's clinical plane needs before Task 7's
//! paging loop can be built on top of it: a page genuinely carries an authored event's
//! attestation and custody (not just its bytes), and the page function's own paging
//! contract — `page_limit` and the exclusive `after_seq` cursor — is honoured by the Rust
//! side exactly as `db/tests/051_clinical_capture_source_test.sql` proves it at the SQL
//! layer. A third test covers the federation plane's `read_node_page`, whose three `NULL`
//! columns are the point: they must decode as `None`, not as some other falsy value.
//!
//! Fixtures are copied from `dr_clinical_guarantee_gap.rs` rather than shared, because
//! integration-test binaries in this crate cannot `use` another test binary's private
//! helpers — only `tests/common/mod.rs` is shared, and these two fixtures do not belong
//! there (they exist to make ONE sealed event, which is this file's whole job, not a
//! cross-suite need).
//!
//! DB-gated on `$CAIRN_TEST_PG`, following the repo-wide pattern (`tests/db_gate_actually_ran.rs`
//! polices the skip). Key material (the DEK, the signing key) is derived at runtime by the
//! production `seal_event_payload`/`generate_key` paths, never a literal (house rule 6).

use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_node::capture;
use cairn_node::{db, identity};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding, for `submit_registration` (since #345 the first event on a chart must
// be its registration) and `medication_setup`, which owns the truncation list this suite
// needs — see `provisioned_clinic`.
mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Bring the database to the state a real solo clinic node is in: a provisioned node
/// identity (so `node_event` is non-empty) and an enrolled actor. Copied from
/// `dr_clinical_guarantee_gap.rs::provisioned_clinic` — see that file's doc for why the
/// truncation is delegated to `common::medication_setup` rather than reimplemented here.
async fn provisioned_clinic(c: &Client) -> (SigningKey, String) {
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(c).await;
    identity::provision(c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();
    (sk, kid)
}

/// Build a sealed `clinical.medication.asserted` body plus the DEK the strict door needs.
/// Copied from `dr_clinical_guarantee_gap.rs::sealed_assert_body` — a real born-sealed
/// body, not a hand-built row, so the `event_dek` custody this suite reads is produced by
/// the production door.
fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Secret32) {
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    (body, dek)
}

/// Submit ONE real born-sealed clinical event on a fresh chart through the strict door.
/// Returns its signed bytes — the exact bytes a clinical page must carry.
///
/// ANTI-VACUITY, copied from `dr_clinical_guarantee_gap.rs::author_sealed_clinical_event`:
/// reads the row back out of `event_log` before returning, so a passing "the page carries
/// it" assertion below is evidence the event genuinely landed, not evidence that nothing
/// was checked.
async fn author_sealed_clinical_event(c: &Client, sk: &SigningKey, kid: &str) -> Vec<u8> {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;

    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let (body, dek) = sealed_assert_body(kid, patient, hlc);
    let event_id = body.event_id.clone();
    let signed = sign(&body, sk).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
    .expect("a sealed body with its DEK is admitted");

    let landed: Vec<u8> = c
        .query_one(
            "SELECT signed_bytes FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect(
            "anti-vacuity: the event must genuinely BE in event_log, or its absence \
             from the page proves nothing",
        )
        .get(0);
    assert_eq!(
        landed, signed.signed_bytes,
        "the log holds the exact bytes this test will look for"
    );

    signed.signed_bytes
}

/// A clinical page carries the authored event's exact bytes, and its custody: a
/// born-sealed body must have a wrapped DEK on the row, and `to_medium_record` must copy
/// both across verbatim (never re-serializing the bytes, never re-wrapping the key).
#[tokio::test]
async fn a_clinical_page_carries_the_event_its_token_and_its_custody() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    let signed = author_sealed_clinical_event(&c, &sk, &kid).await;

    let page = capture::read_clinical_page(&c, 0, 500).await.unwrap();

    let row = page
        .iter()
        .find(|r| r.signed_bytes == signed)
        .expect("the sealed clinical event must be on the page");
    assert!(
        row.dek_wrapped.is_some(),
        "a born-sealed body must carry its wrapped DEK"
    );
    assert!(row.seq > 0);

    let record = capture::to_medium_record(row);
    assert_eq!(
        record.signed_bytes, signed,
        "signed bytes travel VERBATIM, never re-serialized"
    );
    assert_eq!(record.source_seq, row.seq);
    assert_eq!(
        record.dek_wrapped, row.dek_wrapped,
        "custody is copied wrapped, never re-wrapped"
    );
}

/// The Rust reader honours the same two paging rules `cairn_clinical_page` itself proves
/// at the SQL layer (`db/tests/051_clinical_capture_source_test.sql`, assertions 3 and 4):
/// `page_limit` bounds the page exactly, and `after_seq` is an EXCLUSIVE cursor — a row
/// already read is never handed back on the next page.
#[tokio::test]
async fn a_page_respects_its_limit_and_its_exclusive_cursor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = provisioned_clinic(&c).await;
    // Two authored events (each preceded by its own registration, #345) put several rows
    // on the log, so a page_limit of 1 is a genuine truncation rather than a no-op.
    let _first = author_sealed_clinical_event(&c, &sk, &kid).await;
    let _second = author_sealed_clinical_event(&c, &sk, &kid).await;

    let total: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert!(
        total >= 4,
        "anti-vacuity: two authored events plus their two registrations must be on the \
         log, or the assertions below prove nothing"
    );

    let first_page = capture::read_clinical_page(&c, 0, 1).await.unwrap();
    assert_eq!(
        first_page.len(),
        1,
        "page_limit must be honoured exactly, not merely as an upper bound"
    );
    let cursor = first_page[0].seq;

    let rest = capture::read_clinical_page(&c, cursor, 500).await.unwrap();
    assert!(
        rest.iter().all(|r| r.seq > cursor),
        "after_seq is EXCLUSIVE: no returned row may repeat the cursor position"
    );
    assert_eq!(
        rest.len() as i64,
        total - 1,
        "the second page must hold every row except the one already read"
    );
}

/// The federation plane carries no human attestation and no clinical custody — and never
/// will (`node.enrolled` etc. are never encrypted and never require a human attester). A
/// node-plane row's three optional columns must decode as `None`, not as some other
/// falsy stand-in, because `None` and `Some(vec![])` are different fail-closed facts on
/// the apply side (`cairn-medium`'s `MediumRecord` doc).
#[tokio::test]
async fn a_node_page_carries_no_attestation_and_no_custody() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk, _kid) = provisioned_clinic(&c).await;

    let page = capture::read_node_page(&c, 0, 500).await.unwrap();
    assert!(
        !page.is_empty(),
        "the node was provisioned, so node_event must carry at least the enrollment event \
         — an empty page would make the assertions below vacuous"
    );

    for row in &page {
        assert_eq!(
            row.attestation, None,
            "the federation plane carries no attestation"
        );
        assert_eq!(
            row.attester_key, None,
            "the federation plane carries no attester key"
        );
        assert_eq!(
            row.dek_wrapped, None,
            "the federation plane carries no custody"
        );
        let record = capture::to_medium_record(row);
        assert_eq!(record.source_seq, row.seq);
        assert_eq!(record.signed_bytes, row.signed_bytes);
    }
}
