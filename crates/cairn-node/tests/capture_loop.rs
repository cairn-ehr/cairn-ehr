//! Task 7 of #500 slice 2c — `capture::capture_plane`, the loop that decides which clinical
//! events reach a backup medium and which are skipped.
//!
//! This is the safety-critical heart of the slice, so every test below is written against a
//! PROPERTY rather than a line of the implementation. In the order the properties matter:
//!
//! 1. **Resume from the watermark, never from the file tail.** An unverifiable trailing
//!    segment (a torn append) must not advance the cursor, so its records are re-captured.
//!    That is what makes an interrupted backup cost exactly one increment.
//! 2. **An unchanged log appends nothing** — not an empty segment, not a byte. This is the
//!    property CAIRNB3 exists for: a nightly backup of a quiet clinic must not grow.
//! 3. **No event is captured twice.** Set-union makes a duplicate harmless to correctness,
//!    which is exactly why nothing else in the system would ever notice one — so it has to
//!    be pinned here.
//! 4. **A missing signing key never blocks a capture** (§1.2 paper-parity, in test form).
//! 5. **Verify before the bytes can touch the medium**, and refuse — by name — anything this
//!    loop cannot legitimately do, rather than silently doing nothing.
//!
//! COUNTS ARE DERIVED, NEVER HARDCODED. One `author_sealed_clinical_event` puts more than one
//! row on `event_log` (since #345 a chart's first event must be its registration, so the
//! helper authors a registration AND a medication assertion), and a future door change could
//! move that number again. Every assertion below therefore measures the log itself — a test
//! that hardcoded "1" would go green for the wrong reason the day the door changed.
//!
//! DB-gated on `$CAIRN_TEST_PG`, following the repo-wide pattern (`tests/db_gate_actually_ran.rs`
//! polices the skip). Key material is derived at runtime by the production
//! `seal_event_payload`/`generate_key` paths, never a literal (house rule 6).

use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{
    chain_report, parse_any, seq_gaps, serialize_container, serialize_v3, MediumImage,
    MediumRecord, MediumV3, Plane, Segment,
};
use cairn_node::capture;
use cairn_node::{db, identity};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding, for `submit_registration` (since #345 the first event on a chart must
// be its registration) and `medication_setup`, which owns the truncation list this suite
// needs — see `clinic`.
mod common;

// ---------------------------------------------------------------------------
// Fixtures. Copied from `clinical_capture_read.rs`/`dr_clinical_guarantee_gap.rs` rather
// than shared: integration-test binaries in this crate cannot `use` another test binary's
// private helpers, and only `tests/common/mod.rs` is shared. These exist to make ONE sealed
// event, which is a per-suite need rather than a cross-suite one.
// ---------------------------------------------------------------------------

/// Everything one test needs to drive a capture: a live database, the node's signing
/// identity, and the node's own genesis id.
///
/// Bundled into a struct rather than returned as a tuple because of `_guard`: the DB-gated
/// suites share one PostgreSQL database and each `TRUNCATE`s on entry, so the advisory-lock
/// guard must stay ALIVE for the whole test. Held in a field, it is dropped exactly when the
/// rest of the fixture is — a loose `let _ = …` binding at each call site would be one
/// forgotten underscore away from a racing suite.
struct Clinic {
    _guard: Client,
    db: Client,
    sk: SigningKey,
    kid: String,
    /// This node's own genesis node-id, hex — the value `local_node` holds and the value a
    /// segment attestation names. Read from the database rather than invented, because a
    /// segment whose attested id has no matching genesis on the medium is a `SelfIdUnbound`
    /// fault: a fixture that made one up would be testing a forged claim.
    id: String,
}

/// Bring the database to the state a real solo clinic node is in, and return the fixture.
/// `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip, policed by
/// `tests/db_gate_actually_ran.rs`.
async fn clinic() -> Option<Clinic> {
    let base = std::env::var("CAIRN_TEST_PG").ok()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
    // Delegated rather than reimplemented: `medication_setup` owns the canonical truncation
    // list, including the medication PROJECTION tables that have no FK to `event_log`.
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(&c).await;
    identity::provision(&c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();
    let id: String = c
        .query_one(
            "SELECT encode(node_id,'hex') AS id FROM local_node WHERE id",
            &[],
        )
        .await
        .expect("a provisioned node has a local_node row")
        .get("id");
    Some(Clinic {
        _guard: guard,
        db: c,
        sk,
        kid,
        id,
    })
}

/// Build a sealed `clinical.medication.asserted` body plus the DEK the strict door needs —
/// a real born-sealed body, not a hand-built row, so the custody these tests watch travel
/// onto the medium is produced by the production door.
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

/// Submit ONE real born-sealed clinical event on a fresh chart through the strict door, and
/// return its signed bytes — the exact bytes the medium must carry.
///
/// ANTI-VACUITY: reads the row back out of `event_log` before returning, so every "the
/// medium carries it" assertion below is evidence the event genuinely landed rather than
/// evidence that nothing was checked.
///
/// NOTE it writes TWO `event_log` rows: the chart's registration (#345) and the medication
/// assertion. Nothing in this file may assume that number — see the module doc.
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
            "anti-vacuity: the event must genuinely BE in event_log, or its absence from \
             the medium proves nothing",
        )
        .get(0);
    assert_eq!(
        landed, signed.signed_bytes,
        "the log holds the exact bytes this test will look for"
    );
    signed.signed_bytes
}

/// How many rows the clinical plane currently holds. The denominator every count assertion
/// in this file is measured against, so none of them hardcodes a door's arity.
async fn event_log_rows(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0)
}

/// Flip a byte in one already-landed event's `signed_bytes`, modelling a CORRUPT READ — a
/// torn page, a failing disk, a bug in the capture query. Returns the corrupted bytes.
///
/// WHY THIS REACHES PAST A TRIGGER. `event_log` is append-only, enforced in the database by
/// db/001's `event_log_no_update` (which is exactly right, and is why no legitimate door can
/// produce this state). The failure being modelled is not a bad WRITE — the floor already
/// prevents that — it is a bad READ on the way to the medium, and the only way to stage it
/// is to put the bad bytes where the reader will find them.
///
/// **LEAVES A TRANSACTION OPEN, and the caller must `ROLLBACK`.** That is the cleanup
/// mechanism, not an oversight. This database is shared and serialized across suites, so a
/// COMMITTED corrupt row would outlive this test and be visible to any later suite that reads
/// `event_log` without truncating first. Inside an uncommitted transaction it is visible to
/// this session alone, and it survives a panic: an unwound test drops the `Clinic` (and with
/// it the client), the connection closes, and the server aborts the transaction — so the
/// corruption and the trigger change are both undone whether the test passes, fails, or
/// panics. Cleanup that only runs on the happy path is not cleanup.
///
/// The `ENABLE TRIGGER` is still issued rather than left to the rollback, so that even an
/// unexpected commit could not leave the append-only floor down. `content_address` is
/// recomputed in the same statement because db/001's `event_content_addressed` CHECK binds
/// the two — updating one without the other would be refused by the constraint, not by the
/// code under test.
async fn corrupt_one_logged_event_in_an_open_transaction(c: &Client, signed: &[u8]) -> Vec<u8> {
    let mut corrupt = signed.to_vec();
    let mid = corrupt.len() / 2;
    corrupt[mid] ^= 0xff;

    c.batch_execute("BEGIN").await.unwrap();
    c.batch_execute("ALTER TABLE event_log DISABLE TRIGGER event_log_no_update")
        .await
        .unwrap();
    let updated = c
        .execute(
            "UPDATE event_log \
                SET signed_bytes = $2, \
                    content_address = '\\x1220'::bytea || digest($2::bytea, 'sha256') \
              WHERE signed_bytes = $1",
            &[&signed, &corrupt.as_slice()],
        )
        .await
        .unwrap();
    c.batch_execute("ALTER TABLE event_log ENABLE TRIGGER event_log_no_update")
        .await
        .unwrap();
    assert_eq!(
        updated, 1,
        "anti-vacuity: the fixture must genuinely corrupt the row it names"
    );
    corrupt
}

// ---------------------------------------------------------------------------
// Medium-reading helpers. Small and shared so each test reads as a property, not as a
// match arm.
// ---------------------------------------------------------------------------

/// The CAIRNB3 image, or a panic naming what was found instead. Every test in this file
/// captures onto a CAIRNB3 medium, so a `Legacy` here is a broken capture, not a case to
/// handle.
fn as_v3(image: &MediumImage) -> &MediumV3 {
    match image {
        MediumImage::V3(m) => m,
        MediumImage::Legacy(_) => {
            panic!("a capture must leave a CAIRNB3 medium, never a legacy container")
        }
    }
}

fn image_segments(image: &MediumImage) -> &[Segment] {
    &as_v3(image).segments
}

/// Every record the medium carries on the CLINICAL plane, in file order. Borrowed rather
/// than cloned: these are whole signed events, and the tests only ever compare bytes.
fn clinical_records(image: &MediumImage) -> Vec<&MediumRecord> {
    image_segments(image)
        .iter()
        .filter(|s| s.plane == Plane::Clinical)
        .flat_map(|s| s.records.iter())
        .collect()
}

// ---------------------------------------------------------------------------
// The properties.
// ---------------------------------------------------------------------------

/// **Property 2.** The property CAIRNB3 exists for: a nightly backup of a quiet clinic must
/// not grow the medium by a single byte. If this fails, the append-only design has bought
/// nothing and a year of nightly backups is a year of re-recorded history.
#[tokio::test]
async fn a_capture_over_an_unchanged_log_appends_nothing() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let first = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();
    // Anti-vacuity: if the first capture wrote nothing, "the second wrote nothing" is a
    // statement about an empty medium rather than about an unchanged log.
    assert!(
        first.records_appended > 0,
        "the first capture must genuinely write the authored events"
    );
    let after_first = medium.clone();

    let second = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(second.records_appended, 0, "nothing new to capture");
    assert_eq!(
        medium, after_first,
        "an unchanged log must append no segment at all — not even an empty one"
    );
    assert_eq!(
        second.watermark, first.watermark,
        "and the reported watermark must not drift when nothing was written"
    );
}

/// **Properties 1 and 3.** Resumption is by WATERMARK, so a second capture carries only what
/// is new — and carries it exactly once. A duplicate is not a correctness bug (set-union),
/// which is precisely why nothing else would catch it.
#[tokio::test]
async fn a_capture_resumes_from_the_watermark_and_never_re_appends() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    let bytes_a = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let first = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    let before_second = event_log_rows(&cl.db).await;
    let bytes_b = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let newly_authored = event_log_rows(&cl.db).await - before_second;
    assert!(
        newly_authored > 0,
        "anti-vacuity: the second authoring must genuinely add rows"
    );

    let second = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(
        second.records_appended as i64, newly_authored,
        "only what is new — the watermark, not the whole log, is the cursor"
    );

    let image = parse_any(&medium).unwrap();
    let all = clinical_records(&image);
    assert_eq!(
        all.iter().filter(|r| r.signed_bytes == bytes_a).count(),
        1,
        "the first event must appear EXACTLY once — set-union would hide a duplicate"
    );
    assert_eq!(
        all.iter().filter(|r| r.signed_bytes == bytes_b).count(),
        1,
        "and so must the second"
    );
    assert_eq!(
        all.len(),
        first.records_appended + second.records_appended,
        "every record on the medium was written by exactly one of the two captures"
    );
}

/// **Property 1.** A torn append costs exactly ONE increment, because the watermark follows
/// the last VERIFIED segment rather than the file tail. This is what makes an interrupted
/// backup safe to simply re-run.
#[tokio::test]
async fn a_torn_append_costs_exactly_one_increment() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    let bytes = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let first = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();
    assert!(
        first.records_appended > 0,
        "anti-vacuity: there must be a segment to tear"
    );

    // Simulate the interrupted write: cut the last segment in half.
    medium.truncate(medium.len() - 20);
    assert!(
        as_v3(&parse_any(&medium).unwrap()).truncated_tail,
        "anti-vacuity: the truncation must genuinely produce a torn tail, or this test \
         proves nothing about resumption"
    );

    let again = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(
        again.records_appended, first.records_appended,
        "the torn segment did not advance the watermark, so its records are re-captured"
    );
    let image = parse_any(&medium).unwrap();
    assert!(
        clinical_records(&image)
            .iter()
            .any(|r| r.signed_bytes == bytes),
        "and the record is readable again — a torn append loses at most one increment"
    );
    assert!(
        !as_v3(&image).truncated_tail,
        "the writer must truncate to `complete_bytes` BEFORE appending, or the torn remnant \
         becomes the next section's length prefix and parsing stops there forever (I4)"
    );
    let report = chain_report(as_v3(&image));
    assert!(
        report.chain_intact(),
        "a recovered medium must chain cleanly: {:?}",
        report.faults
    );
}

/// **Property 4 — PAPER-PARITY IN TEST FORM (§1.2, M must stay 1).** An unattended cron run
/// has no passphrase, so it has no signing key. The segment travels UNSIGNED and flagged; it
/// must never be refused, or the clinical capture would force a second human act.
#[tokio::test]
async fn a_capture_without_a_signing_key_still_captures() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    let bytes = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let done = capture::capture_plane(&cl.db, &mut medium, Plane::Clinical, None, &cl.id, 500)
        .await
        .expect("an unavailable signing key must never BLOCK a capture");

    assert!(done.records_appended > 0);
    let image = parse_any(&medium).unwrap();
    let seg = image_segments(&image).last().unwrap();
    assert!(seg.attestation.is_none(), "unsigned, and honestly so");
    assert_eq!(
        seg.self_node_id_hex, cl.id,
        "an unsigned segment still NAMES itself — that is what closes the operator-typo \
         footgun even without a key"
    );
    assert!(
        clinical_records(&image)
            .iter()
            .any(|r| r.signed_bytes == bytes),
        "the clinical record travels either way — the signature is provenance, not custody"
    );
    assert!(
        done.watermark.is_some(),
        "an unsigned segment still advances the watermark, so the next run resumes after it"
    );
}

/// **Property 3, at scale.** Paging is not cosmetic here: a capture larger than one page must
/// produce a chain whose indices are contiguous and whose predecessors link, or the medium is
/// unverifiable — and every record must still appear exactly once across the segment boundary.
#[tokio::test]
async fn a_multi_page_capture_produces_one_unbroken_chain() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    let mut authored = Vec::new();
    for _ in 0..5 {
        authored.push(author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await);
    }
    let total_rows = event_log_rows(&cl.db).await;

    let done = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        2,
    )
    .await
    .unwrap();

    assert_eq!(
        done.records_appended as i64, total_rows,
        "a multi-page capture must carry the WHOLE log, not just its first page"
    );

    let image = parse_any(&medium).unwrap();
    let report = chain_report(as_v3(&image));
    assert!(
        report.chain_intact(),
        "every appended segment must chain to its predecessor: {:?}",
        report.faults
    );
    let segments = image_segments(&image);
    assert!(
        segments.len() >= 3,
        "five events (with their registrations) at 2 per page is at least 3 segments, got {}",
        segments.len()
    );
    assert_eq!(
        report.signed_valid,
        segments.len(),
        "a signed capture must produce segments whose attestations all VERIFY"
    );

    let all = clinical_records(&image);
    for bytes in &authored {
        assert_eq!(
            all.iter().filter(|r| &r.signed_bytes == bytes).count(),
            1,
            "no record may straddle a page boundary and land twice"
        );
    }
}

/// **The shape Task 9 actually calls**: both planes onto ONE medium, node first. Pinned here
/// rather than left to that task because three things only fail once both planes are present,
/// and each of them fails SILENTLY in a single-plane test:
///
///   - CAIRNB3 has ONE global chain in file order across both planes, not one per plane, so a
///     clinical segment must chain off the node segment that precedes it;
///   - the watermark is PER PLANE, so a node capture must not move the clinical cursor (a
///     `watermark` that ignored its plane argument would skip clinical events forever, and
///     the medium would report itself complete);
///   - a signed segment's self-id must bind to a genesis ON THIS MEDIUM. Only once the node
///     plane is captured does the medium carry one, which is when `SelfIdUnbound` becomes
///     possible at all.
#[tokio::test]
async fn both_planes_share_one_chain_and_keep_separate_watermarks() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut medium = serialize_v3(&[]).unwrap();
    let clinical_bytes = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let signer = Some((&cl.sk, cl.kid.as_str()));

    let node = capture::capture_plane(&cl.db, &mut medium, Plane::Node, signer, &cl.id, 500)
        .await
        .unwrap();
    let clinical =
        capture::capture_plane(&cl.db, &mut medium, Plane::Clinical, signer, &cl.id, 500)
            .await
            .unwrap();

    assert!(
        node.records_appended > 0,
        "anti-vacuity: a provisioned node has federation events to capture"
    );
    assert!(clinical.records_appended > 0);

    let image = parse_any(&medium).unwrap();
    let report = chain_report(as_v3(&image));
    assert!(
        report.chain_intact(),
        "the clinical segment must chain off the node segment — one global chain, not two: \
         {:?}",
        report.faults
    );
    assert_eq!(
        report.signed_valid,
        image_segments(&image).len(),
        "every segment's attestation must verify, self-id bind included, now that the \
         medium carries the genesis to bind against"
    );
    assert!(clinical_records(&image)
        .iter()
        .any(|r| r.signed_bytes == clinical_bytes));

    // The watermarks are per plane, and a second pass over either one appends nothing.
    let node_again = capture::capture_plane(&cl.db, &mut medium, Plane::Node, signer, &cl.id, 500)
        .await
        .unwrap();
    let clinical_again =
        capture::capture_plane(&cl.db, &mut medium, Plane::Clinical, signer, &cl.id, 500)
            .await
            .unwrap();
    assert_eq!(
        node_again.records_appended, 0,
        "the clinical capture in between must not have disturbed the node plane's cursor"
    );
    assert_eq!(
        clinical_again.records_appended, 0,
        "nor the node capture the clinical plane's"
    );
    assert_eq!(node_again.watermark, node.watermark);
    assert_eq!(clinical_again.watermark, clinical.watermark);
}

/// **The gap hazard, and the reason this slice exists in miniature.**
///
/// `event_log.seq` is `GENERATED ALWAYS AS IDENTITY`: the value is issued at INSERT and
/// commits can land out of order, so a capture reading between two overlapping
/// `submit_event` transactions can see seq 6 and not seq 5. Resuming from the watermark
/// alone, **seq 5 would be skipped by every future run for the life of the medium** while
/// the medium reported itself complete through 6 — a clinical event silently absent from a
/// backup that says it is whole.
///
/// The state is BUILT directly rather than raced: a medium that holds the lowest and highest
/// seqs and nothing in between, over a database that holds them all. That is the same state
/// the interleaved commit produces, and it is deterministic.
#[tokio::test]
async fn a_capture_backfills_a_hole_below_its_own_watermark() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    for _ in 0..3 {
        author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    }
    let rows = capture::read_clinical_page(&cl.db, 0, 500).await.unwrap();
    assert!(
        rows.len() >= 3,
        "anti-vacuity: there must be rows to leave a hole between, got {}",
        rows.len()
    );
    let lowest = &rows[0];
    let highest = rows.last().unwrap();

    // A medium holding only the two ENDS of the run: exactly what a capture that read past
    // an uncommitted middle would have written. Unsigned, because the hazard has nothing to
    // do with signing and an unsigned segment still advances the watermark.
    let hole = Segment {
        plane: Plane::Clinical,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: cl.id.clone(),
        attestation: None,
        records: vec![
            capture::to_medium_record(lowest),
            capture::to_medium_record(highest),
        ],
    };
    let mut medium = serialize_v3(&[hole]).unwrap();

    // Anti-vacuity, both halves: the medium really is missing the middle, and it really does
    // claim a watermark above it.
    let missing: Vec<i64> = rows
        .iter()
        .map(|r| r.seq)
        .filter(|s| *s > lowest.seq && *s < highest.seq)
        .collect();
    assert!(
        !missing.is_empty(),
        "the fixture must genuinely leave a hole"
    );
    {
        let image = parse_any(&medium).unwrap();
        let held: Vec<i64> = clinical_records(&image)
            .iter()
            .map(|r| r.source_seq)
            .collect();
        assert_eq!(held, vec![lowest.seq, highest.seq]);
    }

    let done = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(
        done.records_appended,
        missing.len(),
        "every seq inside the hole must be captured — the watermark is a high-water MARK, \
         not a completeness claim"
    );
    assert!(
        done.unfilled_gaps.is_empty(),
        "and the medium must report no remaining hole: {:?}",
        done.unfilled_gaps
    );

    let image = parse_any(&medium).unwrap();
    let held: Vec<i64> = clinical_records(&image)
        .iter()
        .map(|r| r.source_seq)
        .collect();
    for seq in &rows {
        assert_eq!(
            held.iter().filter(|s| **s == seq.seq).count(),
            1,
            "seq {} must be on the medium exactly once",
            seq.seq
        );
    }
    let report = chain_report(as_v3(&image));
    assert!(
        report.chain_intact(),
        "a backfilled segment carries source_seqs BELOW ones already present, which the \
         chain (by FILE order) must not care about: {:?}",
        report.faults
    );
}

/// **The other half of the gap policy: a hole the database can never supply must cost
/// nothing.** A seq burned by a rolled-back transaction is never re-issued, so that hole is
/// permanent. The capture must not loop on it, must not append an empty segment chasing it
/// (property 2 survives), and must not swallow it either — an unfillable hole is a standing
/// defect in the backup, and the caller has to be told.
#[tokio::test]
async fn an_unfillable_hole_appends_nothing_and_is_reported_not_retried() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let rows = capture::read_clinical_page(&cl.db, 0, 500).await.unwrap();
    let highest = rows.last().unwrap();

    // A hole entirely ABOVE everything the database holds: seqs the log will never issue.
    // EVERY real row goes on the medium as well, so the floor probe has nothing to find and
    // this test measures the interior hole alone.
    let phantom_seq = highest.seq + 10;
    let mut phantom = capture::to_medium_record(highest);
    phantom.source_seq = phantom_seq;
    let mut records: Vec<MediumRecord> = rows.iter().map(capture::to_medium_record).collect();
    records.push(phantom);
    let seg = Segment {
        plane: Plane::Clinical,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: cl.id.clone(),
        attestation: None,
        records,
    };
    let mut medium = serialize_v3(&[seg]).unwrap();
    let before = medium.clone();

    let done = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(
        done.records_appended, 0,
        "the database cannot supply the hole, so nothing is appended"
    );
    assert_eq!(
        medium, before,
        "and property 2 holds even while a hole is outstanding — not one byte"
    );
    assert_eq!(
        done.unfilled_gaps,
        vec![(highest.seq, phantom_seq)],
        "but the hole is REPORTED, so a `watermark` of Some(N) can never be read as \
         completeness"
    );
    assert!(
        done.probed_empty.contains(&(highest.seq, phantom_seq)),
        "and it is reported as PROBED-and-empty, which is the stronger claim: an IDENTITY \
         value is never re-issued, so this range is permanently absent and #549's \
         persistence can stop re-asking. Got {:?}",
        done.probed_empty
    );

    // Running it again changes nothing: the fill is a query, not a retry.
    let again = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();
    assert_eq!(again.records_appended, 0);
    assert_eq!(medium, before);
    assert_eq!(again.unfilled_gaps, done.unfilled_gaps);
}

/// **The same hazard AT THE FLOOR, which the first fix did not reach.**
///
/// `seq_gaps` reports holes BETWEEN records the medium holds, and says so in its own doc
/// ("says nothing about seqs BELOW the medium's lowest"); `watermark` is a `max`. So on the
/// first capture of a fresh medium, if seq 1 is uncommitted while seq 2 is visible, the
/// medium's floor becomes 2, **no gap is ever enumerated, seq 1 is skipped forever, and it
/// does not even appear in `unfilled_gaps`** — the original failure mode, surviving at the
/// boundary.
///
/// The medium here holds every row EXCEPT the lowest, and the first assertion is the whole
/// point: `seq_gaps` genuinely reports nothing, so a capture that trusted it alone would
/// silently lose that event.
#[tokio::test]
async fn a_capture_backfills_the_hole_at_the_floor() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    for _ in 0..2 {
        author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    }
    let rows = capture::read_clinical_page(&cl.db, 0, 500).await.unwrap();
    assert!(
        rows.len() >= 3,
        "anti-vacuity: need a run to cut the floor off"
    );
    let lowest = &rows[0];

    let seg = Segment {
        plane: Plane::Clinical,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: cl.id.clone(),
        attestation: None,
        records: rows[1..].iter().map(capture::to_medium_record).collect(),
    };
    let mut medium = serialize_v3(&[seg]).unwrap();

    // ANTI-VACUITY, and the reason this test exists: the hole is invisible to `seq_gaps`.
    {
        let image = parse_any(&medium).unwrap();
        let v3 = as_v3(&image);
        assert!(
            seq_gaps(v3, &chain_report(v3), Plane::Clinical).is_empty(),
            "the hole is at the FLOOR, which seq_gaps by construction cannot see — that is \
             exactly why the floor needs its own probe"
        );
        assert!(
            !clinical_records(&image)
                .iter()
                .any(|r| r.source_seq == lowest.seq),
            "and the lowest row really is missing from the medium"
        );
    }

    let done = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(
        done.records_appended, 1,
        "the one row below the medium's floor must be captured"
    );
    let image = parse_any(&medium).unwrap();
    assert!(
        clinical_records(&image)
            .iter()
            .any(|r| r.signed_bytes == lowest.signed_bytes),
        "and it is the right row, byte for byte"
    );
    assert!(
        done.unfilled_gaps.is_empty(),
        "nothing is missing afterwards: {:?}",
        done.unfilled_gaps
    );
    let report = chain_report(as_v3(&image));
    assert!(report.chain_intact(), "{:?}", report.faults);
}

/// **The probe budget is bounded, so a capture cannot slow down with the medium's age.**
///
/// This is not a hypothetical cost. Both `event_log` INSERT doors end in `ON CONFLICT
/// (event_id) DO NOTHING` and PostgreSQL consumes the IDENTITY value BEFORE conflict
/// arbitration, so every duplicate that set-union sync re-delivers burns a seq and leaves a
/// permanent hole. On a federating node those accumulate for the life of the medium (#549).
/// Without a cap, every capture would re-probe all of them and each night's backup would be
/// slower than the last.
///
/// The budget bounds the WORK, never the honesty: every gap it could not reach is still
/// reported in `unfilled_gaps`.
#[tokio::test]
async fn the_gap_probe_budget_is_bounded_so_a_capture_cannot_slow_with_age() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let rows = capture::read_clinical_page(&cl.db, 0, 500).await.unwrap();
    let base = rows.last().unwrap().seq;

    // Every real row (so the floor probe finds nothing and this test measures the interior
    // budget alone), then more phantom records than the budget, each two seqs apart so each
    // adjacent pair leaves exactly one absent seq the database can never supply.
    let over_budget = capture::MAX_GAP_PROBES_PER_CAPTURE + 8;
    let mut records: Vec<MediumRecord> = rows.iter().map(capture::to_medium_record).collect();
    for i in 1..=over_budget {
        let mut phantom = capture::to_medium_record(rows.last().unwrap());
        phantom.source_seq = base + 2 * i as i64;
        records.push(phantom);
    }
    let seg = Segment {
        plane: Plane::Clinical,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: cl.id.clone(),
        attestation: None,
        records,
    };
    let mut medium = serialize_v3(&[seg]).unwrap();
    let before = medium.clone();

    let done = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .unwrap();

    assert_eq!(done.records_appended, 0, "none of the holes is fillable");
    assert_eq!(medium, before, "so not a byte is appended");

    // Counted over the PHANTOM region only, so the assertion does not depend on where this
    // database's IDENTITY sequence happens to start.
    let interior_probed = done
        .probed_empty
        .iter()
        .filter(|(after, _)| *after >= base)
        .count();
    assert_eq!(
        interior_probed,
        capture::MAX_GAP_PROBES_PER_CAPTURE,
        "exactly the budget is spent — never more, however old the medium gets"
    );
    assert!(
        done.unfilled_gaps.len() > capture::MAX_GAP_PROBES_PER_CAPTURE,
        "and every gap the budget could NOT reach is still reported, never hidden: {} \
         reported vs a budget of {}",
        done.unfilled_gaps.len(),
        capture::MAX_GAP_PROBES_PER_CAPTURE
    );
}

/// **Property 5 — verify BEFORE the bytes can touch the medium.** A segment attestation is
/// computed over the content address of whatever bytes it is handed, so a capture that
/// signed a corrupt read would produce a genuinely VALID attestation over corruption: the
/// chain pass would then report that segment fully intact and signed, forever, and only a
/// separate record-signature pass would ever notice. The capture must therefore refuse at
/// the door — and leave the previous medium untouched when it does.
#[tokio::test]
async fn a_corrupt_read_is_refused_before_it_can_be_signed_onto_the_medium() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let bytes = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    let corrupt = corrupt_one_logged_event_in_an_open_transaction(&cl.db, &bytes).await;

    let mut medium = serialize_v3(&[]).unwrap();
    let before = medium.clone();
    let err = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .expect_err("a record this node cannot verify must never be signed onto a medium");

    let text = format!("{err:#}");
    assert!(
        text.contains("signature verification"),
        "the refusal must say WHY, so an operator knows the medium is fine and the log is \
         not: {text}"
    );
    assert_eq!(
        medium, before,
        "a refused capture leaves the medium byte-identical — the previous good backup is \
         never damaged by a bad read"
    );
    assert!(
        !medium.windows(corrupt.len()).any(|w| w == corrupt),
        "and not one corrupt byte reached the medium"
    );

    // Undo the corruption for the suites that share this database. A panic above reaches the
    // same end by dropping the connection — see the fixture's doc.
    cl.db.batch_execute("ROLLBACK").await.unwrap();
    let still_corrupt: i64 = cl
        .db
        .query_one(
            "SELECT count(*) FROM event_log WHERE signed_bytes = $1",
            &[&corrupt.as_slice()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        still_corrupt, 0,
        "the corrupt row must not outlive this test — the next suite to read event_log \
         without truncating would see it"
    );
}

/// **Property 5, the other half.** What this loop cannot legitimately do, it REFUSES by name — it never
/// silently does nothing. Two such cases, checked together because they share one remedy
/// shape (tell the operator what is wrong and leave the medium untouched):
///
///   - a CAIRNB1/CAIRNB2 medium, which has no segments to append to at all. Silently
///     returning `records_appended: 0` here would report a successful backup of a medium
///     that gained nothing — exactly the composite untruth #500 is about;
///   - a `page_events` below 1, which cannot make progress and would otherwise spin an
///     unattended nightly backup forever.
#[tokio::test]
async fn a_capture_refuses_what_it_cannot_do_rather_than_silently_doing_nothing() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // A legacy container: a real one, built by the crate's own writer.
    let mut legacy = serialize_container(None, &[]).unwrap();
    let before = legacy.clone();
    let err = capture::capture_plane(
        &cl.db,
        &mut legacy,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        500,
    )
    .await
    .expect_err("a segment cannot be appended to a CAIRNB1/CAIRNB2 medium");
    let text = format!("{err:#}");
    assert!(
        text.contains("CAIRNB3"),
        "the refusal must name the revision a capture needs: {text}"
    );
    assert_eq!(
        legacy, before,
        "a refused capture leaves the medium byte-identical"
    );

    let mut medium = serialize_v3(&[]).unwrap();
    let err = capture::capture_plane(
        &cl.db,
        &mut medium,
        Plane::Clinical,
        Some((&cl.sk, &cl.kid)),
        &cl.id,
        0,
    )
    .await
    .expect_err("a page of zero events can never make progress");
    assert!(
        format!("{err:#}").contains("page_events"),
        "the refusal must name the parameter at fault: {err:#}"
    );
}
