//! Task 9 of #500 slice 2c — `backup::backup_to`, the production site where a clinical
//! record finally reaches a backup medium.
//!
//! **Every assertion here is taken on the medium FILE `backup_to` actually writes**, never
//! on a fixture this file builds. That distinction is the whole reason this suite exists
//! beside `capture_loop.rs`: `capture_plane` is already pinned there, property by property,
//! against a buffer the test hands it. A pin whose fixture the test builds leaves the
//! PRODUCTION site — which medium bytes it starts from, which planes it captures, in which
//! order, and whether it writes the buffer at all — completely unguarded. #500 is exactly
//! that failure: every component honest, the composite a precise untruth.
//!
//! # What is pinned
//!
//! 1. **The clinical event, and its custody, are on the medium.** The inversion of #500's
//!    pin (`dr_clinical_guarantee_gap.rs`), taken independently here so that removing one
//!    guard cannot silently remove both.
//! 2. **An unattended run with NO signing key still captures the clinical plane** — §1.2
//!    paper-parity in test form. A cron backup has no passphrase and therefore no key; if
//!    the clinical capture ever came to depend on one, the operator's step count would go
//!    from 1 to 2 and that is an architecture defect (house rule 7), not a trade-off.
//! 3. **A second backup over an unchanged log leaves the medium byte-identical** — the
//!    property CAIRNB3 exists for, asserted through `backup_to` rather than through
//!    `capture_plane`, because it is `backup_to` that decides whether to LOAD the existing
//!    medium or start a new one. Loading it wrongly (a fresh buffer every night) would
//!    still produce a correct, verifiable medium and would silently re-record the clinic's
//!    entire history every night: green everywhere, ruinous in a year.
//! 4. **A legacy CAIRNB1/CAIRNB2 medium is succeeded by a NEW CAIRNB3 medium that holds at
//!    least as much** — the migration step, and the one where an operator could lose
//!    records if the first CAIRNB3 capture were anything less than a full sweep.
//! 5. **…and it is REFUSED when the successor would not hold at least as much** — the two
//!    arms of that same precondition, which nothing enforced until the final review of this
//!    slice (Critical 1). Succeeding a legacy medium is the only path here that DESTROYS an
//!    artifact, and an empty successor is perfectly *sound*, so no downstream check would
//!    have stopped it: a peer's medium on a shared volume, and the disk-died / re-`init` /
//!    `backup`-before-`restore` sequence, each end with the clinic's only copy overwritten
//!    and a success reported.
//!
//! COUNTS ARE DERIVED, NEVER HARDCODED (`capture_loop.rs`'s rule, for the same reason): one
//! `author_sealed_clinical_event` writes MORE than one `event_log` row — since #345 a
//! chart's first event must be its registration — and a future door change could move that
//! number again.
//!
//! DB-gated on `$CAIRN_TEST_PG`, the repo-wide pattern policed by `tests/db_gate_actually_ran.rs`.
//! Key material is derived at runtime by the production `seal_event_payload`/`generate_key`
//! paths, never a literal (house rule 6).

use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{event_address, generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_medium::{
    append_segment, build_self_attestation, parse_any, serialize_container,
    verify_self_attestation, MediumImage, MediumRecord, Plane, Segment, SelfMarker,
};
use cairn_node::{backup, db, identity};
use tokio_postgres::Client;
use uuid::Uuid;

// Shared scaffolding, for `submit_registration` (#345) and `medication_setup`, which owns
// the canonical truncation list this suite needs — see `clinic`.
mod common;

// ---------------------------------------------------------------------------
// Fixtures. Deliberately the same shape as `capture_loop.rs`'s rather than shared with it:
// integration-test binaries in this crate cannot `use` another test binary's helpers, and
// only `tests/common/mod.rs` is shared across them.
// ---------------------------------------------------------------------------

/// Everything one backup needs: a live database, this node's signing identity, and a
/// temporary directory standing in for the operator's backup volume.
///
/// `_guard` is held in a FIELD, not in a loose `let _ =` at each call site: the DB-gated
/// suites share one PostgreSQL database and each truncates on entry, so the advisory-lock
/// guard must stay alive for the whole test. One forgotten underscore would race them.
struct Clinic {
    _guard: Client,
    db: Client,
    sk: SigningKey,
    kid: String,
    dir: tempfile::TempDir,
}

impl Clinic {
    /// Where the medium goes. A single fixed name, because the point of several tests here
    /// is what happens on the SECOND backup to the same path.
    fn medium(&self) -> std::path::PathBuf {
        self.dir.path().join("cairn.medium")
    }

    fn health(&self) -> std::path::PathBuf {
        self.dir.path().join("backup-status.json")
    }
}

/// Bring the database to the state a real solo clinic node is in. `None` when
/// `$CAIRN_TEST_PG` is unset — the repo-wide self-skip.
async fn clinic() -> Option<Clinic> {
    let base = std::env::var("CAIRN_TEST_PG").ok()?;
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&c).await.unwrap();
    // Delegated rather than reimplemented: `medication_setup` owns the canonical truncation
    // list, including the medication PROJECTION tables that have no FK to `event_log` and so
    // survive a `TRUNCATE … CASCADE` from it.
    let (sk, kid, _sk_human, _kid_human) = common::medication_setup(&c).await;
    identity::provision(&c, &sk, &kid, "solo-clinic", "127.0.0.1:7931")
        .await
        .unwrap();
    Some(Clinic {
        _guard: guard,
        db: c,
        sk,
        kid,
        dir: tempfile::tempdir().unwrap(),
    })
}

/// Build a sealed `clinical.medication.asserted` body plus the DEK the strict door needs —
/// a real born-sealed body, not a hand-built row, so the custody this suite watches travel
/// onto the medium is the one the production door actually wrapped.
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
/// ANTI-VACUITY: reads the row back out of `event_log` before returning. `submit_event`'s
/// INSERT ends in `ON CONFLICT DO NOTHING`, so "no error" is an invariant of a distant door
/// rather than evidence visible here; without this read, every "the medium carries it"
/// assertion below could pass or fail for reasons that have nothing to do with the backup.
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
            "anti-vacuity: the event must genuinely BE in event_log, or its presence on \
             the medium proves nothing about the capture",
        )
        .get(0);
    assert_eq!(
        landed, signed.signed_bytes,
        "the log holds the exact bytes this test will look for"
    );
    signed.signed_bytes
}

// ---------------------------------------------------------------------------
// Medium-reading helpers. Small and shared so each test reads as a property.
// ---------------------------------------------------------------------------

/// Parse the medium file at `path`, insisting it is CAIRNB3. Every test in this file backs
/// up through `backup_to`, so a legacy image here is a broken writer, not a case to handle.
fn read_v3(path: &std::path::Path) -> MediumImage {
    let bytes = std::fs::read(path).expect("the backup must have written a medium");
    let image = parse_any(&bytes).expect("the medium `backup_to` wrote must parse");
    assert!(
        matches!(image, MediumImage::V3(_)),
        "a capture must leave a CAIRNB3 medium, never a legacy container: {path:?}"
    );
    image
}

/// This node's own genesis node-id (hex), read from `local_node` — the same answer
/// `backup_to` reads for the self-marker it writes onto every segment.
async fn self_node_id_hex(c: &Client) -> String {
    identity::load_local(c)
        .await
        .expect("the clinic fixture provisions a node")
        .node_id_hex
}

/// Every record the medium carries on `plane`, in file order.
fn records_on(image: &MediumImage, plane: Plane) -> Vec<&MediumRecord> {
    match image {
        MediumImage::V3(m) => m
            .segments
            .iter()
            .filter(|s| s.plane == plane)
            .flat_map(|s| s.records.iter())
            .collect(),
        MediumImage::Legacy(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// The properties.
// ---------------------------------------------------------------------------

/// **The payload of the whole slice.** ADR-0026 decision 1 promises that on total hardware
/// loss of a solo node *"the clinical event log survives"*; until this commit the medium
/// carried `node_event` and nothing else, so a restored clinic recovered who it had peered
/// with and zero patients.
///
/// Both halves are asserted, because either alone is worthless: the event's signed bytes
/// (the record) AND its wrapped DEK (the key that opens the record). ADR-0052 makes every
/// clinical body born-sealed, so a medium carrying the ciphertext without the custody would
/// restore noise while every surface reported success — the same composite untruth one
/// level down.
#[tokio::test]
async fn the_medium_carries_the_clinical_event_and_its_custody() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let signed = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    backup::backup_to(
        &cl.db,
        &cl.medium(),
        &cl.health(),
        1_700_000_000,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect("the backup ceremony succeeds");

    let image = read_v3(&cl.medium());
    let clinical = records_on(&image, Plane::Clinical);
    let found = clinical
        .iter()
        .find(|r| r.signed_bytes == signed)
        .expect("#500: the clinical event must be ON THE MEDIUM");
    assert!(
        found.dek_wrapped.is_some(),
        "and its custody must travel with it — a born-sealed body without its DEK restores \
         as noise (ADR-0052/ADR-0066)"
    );

    // ANTI-VACUITY on the other plane: the federation events must ALSO still be there. A
    // capture that wrote only the clinical plane would pass the assertion above and would
    // have destroyed the half of disaster recovery that already worked.
    let node = records_on(&image, Plane::Node);
    let node_rows: i64 = cl
        .db
        .query_one("SELECT count(*) FROM node_event", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        node.len() as i64,
        node_rows,
        "the federation plane must be captured in full, exactly as it was before this slice"
    );

    // Health names the scope rather than a bare total (BackupHealth v2, Task 10) — derived
    // from the medium on disk, so these numbers can never disagree with the assertions above.
    let health = backup::read_health(&cl.health()).expect("health is written after the medium");
    assert_eq!(health.clinical_events as usize, clinical.len());
    assert_eq!(health.node_events as usize, node.len());
    assert!(
        health.clinical_watermark.is_some(),
        "a medium carrying a verified clinical segment has a clinical watermark; `None` \
         would be the honest answer only if nothing clinical had been captured"
    );
}

/// **§1.2 paper-parity, in test form: `M` must stay 1.** An unattended nightly cron run has
/// no passphrase and therefore no signing key. If the clinical capture ever came to depend
/// on one, the operator would have to supply a secret to a job that previously needed none —
/// a second human act where paper has one, which house rule 7 says is an architecture
/// defect to be FILED, not accepted.
///
/// The medium is written UNSIGNED here (segments carry no attestation), which is a declared
/// limitation of the format — never a refusal (`cairn-medium` invariant 7).
#[tokio::test]
async fn an_unattended_capture_with_no_key_still_carries_the_clinical_plane() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let signed = author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // `None` — exactly what `main.rs`'s backup arm passes when no passphrase is available.
    let report = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 1_700_000_000, None)
        .await
        .expect("a missing signing key must NEVER block a backup");

    let image = read_v3(&cl.medium());
    assert!(
        records_on(&image, Plane::Clinical)
            .iter()
            .any(|r| r.signed_bytes == signed),
        "the clinical event must reach the medium with no key and no passphrase available"
    );
    assert_eq!(
        report.marker,
        backup::WrittenMarker::Unsigned,
        "an enrolled node with no key names itself in plaintext only — the medium travels \
         flagged, it is not refused"
    );
}

/// **The property CAIRNB3 exists for, asserted at the production site.** A nightly backup of
/// a quiet clinic must not grow the medium by a single byte, or a year of nightly backups is
/// a year of re-recorded history.
///
/// `capture_loop.rs` already pins this for `capture_plane` over a buffer the test builds.
/// What it CANNOT see is `backup_to`'s own decision: whether to load the medium already on
/// disk or to start a fresh one every night. A writer that started fresh each run would
/// produce a perfectly valid, fully verifying medium — and would re-record the clinic's
/// entire history nightly. Only an assertion over two consecutive real backups catches it.
#[tokio::test]
async fn a_second_backup_over_an_unchanged_log_leaves_the_medium_byte_identical() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let key = Some((&cl.sk, cl.kid.as_str()));
    let first = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 1_000, key)
        .await
        .unwrap();
    // ANTI-VACUITY: if the first run wrote nothing, "the second changed nothing" would be a
    // statement about an empty file rather than about an unchanged log.
    assert!(
        first.clinical_appended > 0 && first.node_appended > 0,
        "the first backup must genuinely capture both planes: {first:?}"
    );
    let after_first = std::fs::read(cl.medium()).unwrap();

    let second = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 2_000, key)
        .await
        .unwrap();

    assert_eq!(
        (second.node_appended, second.clinical_appended),
        (0, 0),
        "nothing new to capture, so nothing may be appended: {second:?}"
    );
    assert_eq!(
        std::fs::read(cl.medium()).unwrap(),
        after_first,
        "an unchanged log must leave the medium byte-identical — not even an empty segment"
    );
    assert_eq!(
        second.origin,
        backup::MediumOrigin::Continued,
        "the second run must APPEND to the medium already on disk, never start a new one"
    );
}

/// **The migration step, and the only one where records could go missing.** A clinic
/// upgrading into this commit has a CAIRNB1/CAIRNB2 medium sitting at `--to`. A CAIRNB3
/// segment cannot be appended to a legacy container (there is no chain to hang it from), so
/// the first backup after the upgrade necessarily starts a NEW medium at that path.
///
/// That replacement is only safe because the first capture of a fresh medium resumes from an
/// ABSENT watermark, which is the same instruction as "sweep from the beginning" — so the
/// new medium holds everything the legacy one did, plus the clinical plane it never could.
/// This test is what makes that a checked fact rather than a claim in a comment: it asserts
/// the successor is a strict superset of the predecessor's federation plane.
#[tokio::test]
async fn a_legacy_medium_is_succeeded_by_a_cairnb3_medium_holding_at_least_as_much() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // Stage the world as it is for an upgrading clinic: a real CAIRNB2 medium, written the
    // way the pre-2c writer wrote one — the federation plane and a self-marker, nothing else.
    //
    // The marker names THIS node, and that is not cosmetic (#500 slice 2c final review,
    // Critical 1): succeeding a legacy medium is now refused when the old medium belongs to
    // someone else, so a fixture naming an arbitrary id would be testing the refusal path
    // while claiming to test the upgrade. It also could not occur — a real marker is a
    // node's 32-byte content-address, not a short word.
    let legacy_events = backup::read_event_set(&cl.db).await.unwrap();
    assert!(
        !legacy_events.is_empty(),
        "anti-vacuity: the legacy medium must carry something for the successor to preserve"
    );
    let legacy = serialize_container(
        Some(&SelfMarker::Unsigned(self_node_id_hex(&cl.db).await)),
        &legacy_events,
    )
    .expect("the fixture fits the frame cap");
    std::fs::write(cl.medium(), &legacy).unwrap();

    let report = backup::backup_to(
        &cl.db,
        &cl.medium(),
        &cl.health(),
        1_700_000_000,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect("a legacy medium at the target path must not fail the backup");

    assert_eq!(
        report.origin,
        backup::MediumOrigin::SucceededLegacy,
        "an operator must be told a NEW medium was started — the file at this path is no \
         longer the one they backed up to yesterday: {report:?}"
    );

    let image = read_v3(&cl.medium());
    let node_bytes: Vec<Vec<u8>> = records_on(&image, Plane::Node)
        .iter()
        .map(|r| r.signed_bytes.clone())
        .collect();
    for event in &legacy_events {
        assert!(
            node_bytes.contains(event),
            "every event the legacy medium carried must be on its successor — replacing a \
             medium with one that holds less would lose records at the exact moment an \
             operator believes their backup improved"
        );
    }
    assert!(
        !records_on(&image, Plane::Clinical).is_empty(),
        "and the successor carries the clinical plane the legacy medium never could"
    );
}

/// **A torn tail is REPAIRED, and the operator is told** (#500 slice 2c review, Minor 5).
///
/// `capture_plane` must cut a torn medium back to its last complete section before appending
/// — otherwise the torn remnant becomes the next section's length prefix and every later
/// backup is silently orphaned. It does that whether or not it then appends anything, so a
/// nightly run over an unchanged log can print `+0 / +0 appended` beside a file that just
/// changed size. Unexplained, that reads as corruption.
///
/// The test drives the real `backup_to` over a genuinely torn medium and asserts three
/// things: the run succeeds, it SAYS it repaired, and nothing verified was lost — every
/// record the intact medium held is still there afterwards.
#[tokio::test]
async fn a_torn_medium_is_repaired_and_the_repair_is_reported() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let key = Some((&cl.sk, cl.kid.as_str()));
    let first = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 1_000, key)
        .await
        .unwrap();
    assert!(
        !first.repaired_torn_tail,
        "anti-vacuity: a medium this run created cannot already be torn: {first:?}"
    );
    let intact = std::fs::read(cl.medium()).unwrap();

    // Tear it the way an interrupted append actually tears: begin a REAL further section and
    // cut it off partway through, so the section's length prefix is honest and the bytes
    // behind it run out. (Appending zeroes instead would be the #523 artifact — a length
    // prefix that is itself corrupt — which `parse_any` reports as `Damaged`, not as a tear,
    // and whose remedy is the opposite one.)
    let mut torn = intact.clone();
    append_segment(
        &mut torn,
        &Segment {
            plane: Plane::Node,
            index: 99,
            prev_commitment: String::new(),
            self_node_id_hex: String::new(),
            attestation: None,
            records: vec![MediumRecord {
                signed_bytes: vec![7u8; 64],
                attestation: None,
                attester_key: None,
                dek_wrapped: None,
                source_seq: 99,
            }],
        },
    )
    .expect("the fixture segment fits the section cap");
    torn.truncate(intact.len() + 12); // partway into that section: a torn tail
    std::fs::write(cl.medium(), &torn).unwrap();
    match parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap() {
        MediumImage::V3(m) => assert!(
            m.truncated_tail,
            "anti-vacuity: the fixture must genuinely produce a torn medium"
        ),
        MediumImage::Legacy(_) => panic!("the fixture must stay CAIRNB3"),
    }

    let second = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 2_000, key)
        .await
        .expect(
            "a torn medium must be repaired, never refused — refusing would stop a \
                 clinic backing up over damage a single truncate fixes",
        );
    assert!(
        second.repaired_torn_tail,
        "the operator must be told the file changed size for a reason: {second:?}"
    );
    assert_eq!(
        std::fs::read(cl.medium()).unwrap(),
        intact,
        "the repair cuts back to exactly the last complete section — nothing verified is \
         lost, and nothing new was appended because the log did not change"
    );
}

// ---------------------------------------------------------------------------
// Succeeding a legacy medium DESTROYS it. Its safety argument has a precondition, and
// these two tests are that precondition made checkable (#500 slice 2c final review,
// Critical 1). Both drive the real `backup_to`; neither asserts on a fixture image.
// ---------------------------------------------------------------------------

/// A DIFFERENT node's genuine pre-2c backup medium — its own signed genesis plus the signed
/// self-attestation `backup` wrote beside it — and that node's id.
///
/// Built from the production primitives (`sign`, `event_address`, `build_self_attestation`)
/// rather than hand-assembled, so it is what a peer running the pre-2c writer would actually
/// have left on a shared backup volume: the case
/// [`backup::MediumOrigin::SucceededLegacy`]'s doc names. Nothing here touches this node's
/// database, and that is the whole point — the peer's events are precisely what a capture
/// HERE cannot sweep back onto a successor.
fn a_peers_legacy_medium() -> (Vec<u8>, String) {
    let (peer_sk, peer_kid) = generate_key().expect("entropy for the peer's signing key");
    let genesis = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "peer-clinic".into(),
        },
        t_effective: None,
        signer_key_id: peer_kid.clone(),
        contributors: serde_json::json!([{"actor_id": peer_kid, "role": "recorded"}]),
        payload: serde_json::json!({"display_name": "peer-clinic", "address": "127.0.0.1:7999"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let events = vec![
        sign(&genesis, &peer_sk)
            .expect("the peer signs its own genesis")
            .signed_bytes,
    ];
    // A node-id IS the content-address of its genesis, so this is the peer's real id — 32
    // bytes of hex, the shape a marker actually carries, never a short word.
    let peer_node_id = hex::encode(event_address(&events[0]));
    let attestation = build_self_attestation(&peer_sk, &peer_kid, &peer_node_id, &events);
    // ANTI-VACUITY: the marker must genuinely VERIFY against these events. An unreadable
    // marker degrades to "no claim", so without this the refusal under test could fire for
    // the count reason while the test claimed to be exercising the identity reason.
    assert_eq!(
        verify_self_attestation(&attestation, &events),
        Some(peer_node_id.clone()),
        "the fixture must be a real signed marker, not a blob that merely looks like one"
    );
    let bytes = serialize_container(Some(&SelfMarker::Signed(attestation)), &events)
        .expect("the fixture fits the frame cap");
    (bytes, peer_node_id)
}

/// **ARM 1 — a peer's medium on a shared backup volume is refused, never replaced.**
///
/// Two clinics rotating one USB stick, or one clinic whose stick still holds a medium from
/// before a restore gave the node a new identity. Succeeding a legacy medium REPLACES the
/// file, and the successor can only ever carry events that are in THIS database — so the
/// peer's record would be destroyed with no copy anywhere and no error at all: the successor
/// is internally sound, so every downstream check passes it.
#[tokio::test]
async fn a_legacy_medium_belonging_to_another_node_is_refused_not_replaced() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    let (peer_medium, peer_node_id) = a_peers_legacy_medium();
    std::fs::write(cl.medium(), &peer_medium).unwrap();
    assert_ne!(
        peer_node_id,
        self_node_id_hex(&cl.db).await,
        "anti-vacuity: the fixture must genuinely belong to a DIFFERENT node"
    );

    let err = backup::backup_to(
        &cl.db,
        &cl.medium(),
        &cl.health(),
        1_700_000_000,
        Some((&cl.sk, &cl.kid)),
    )
    .await
    .expect_err("replacing another node's only medium must be refused, not reported as a backup");

    let msg = format!("{err:#}");
    assert!(
        msg.contains(&peer_node_id),
        "the refusal must NAME whose medium this is — an operator holding two sticks has to \
         know which one they are looking at: {msg}"
    );
    assert!(
        msg.contains("--to at a NEW path"),
        "and it must name a remedy they can act on tonight, not merely say no: {msg}"
    );

    assert_eq!(
        std::fs::read(cl.medium()).unwrap(),
        peer_medium,
        "the peer's medium must be BYTE-IDENTICAL afterwards — a refusal that still wrote \
         would be the defect with an error message attached"
    );
    assert!(
        backup::read_health(&cl.health()).is_none(),
        "and health must not advance over a backup that did not happen"
    );
}

/// **ARM 2 — the successor may never hold LESS than the medium it replaces.**
///
/// The disaster the whole guard exists for, staged exactly as it happens: the clinic's disk
/// dies, the operator re-`init`s a node, and — *before* running `restore` — runs `backup --to`
/// at the USB stick holding their only medium. `local_node` is empty, so the medium claims no
/// node this run can be compared against and arm 1 cannot help; the capture reads zero
/// federation rows; and the staged image is genuinely SOUND (an intact chain over no node
/// segments, 0 of 0 signatures vacuously intact, no torn tail), so `refuse_unsound` passes it
/// happily. Only the count comparison stands between the operator and an empty file where
/// their record used to be.
///
/// The clinical rows are deliberately LEFT in the database, which makes the successor
/// non-empty and therefore not even `carries_nothing` — so the test cannot pass by accident
/// on the emptiness warning that fires after the write is already done.
#[tokio::test]
async fn a_legacy_medium_is_refused_when_the_successor_would_hold_less() {
    let Some(cl) = clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;

    // The stick: a marker-less legacy medium (a CAIRNB1-era backup, or one taken before
    // enrolment). No identity claim at all, so ONLY the count arm can refuse this — the test
    // isolates the arm it is named for.
    let legacy_events = backup::read_event_set(&cl.db).await.unwrap();
    assert!(
        !legacy_events.is_empty(),
        "anti-vacuity: there must be something on the medium for the successor to fall short of"
    );
    let legacy = serialize_container(None, &legacy_events).expect("the fixture fits the frame cap");
    std::fs::write(cl.medium(), &legacy).unwrap();

    // The dead disk, replaced: an initialised database that names no node. `local_node` is
    // exactly what `read_self_node_id` reads, and it answers `None` here WITHOUT an error —
    // which is what let this path reach the write in the first place.
    db::reset_node_federation_tables(&cl.db).await.unwrap();

    let err = backup::backup_to(&cl.db, &cl.medium(), &cl.health(), 1_700_000_000, None)
        .await
        .expect_err(
            "a backup that would replace the clinic's only medium with a smaller one must \
             refuse — this is the disk-died, re-init, backup-before-restore sequence",
        );

    let msg = format!("{err:#}");
    assert!(
        msg.contains(&format!("{} federation event(s)", legacy_events.len())),
        "the refusal must quote what the old medium holds, so the operator can see the \
         shortfall rather than take it on trust: {msg}"
    );
    assert!(
        msg.contains("--to at a NEW path"),
        "and name the remedy: {msg}"
    );

    assert_eq!(
        std::fs::read(cl.medium()).unwrap(),
        legacy,
        "the only copy of the clinic's record must be BYTE-IDENTICAL afterwards"
    );
    assert!(
        backup::read_health(&cl.health()).is_none(),
        "and health must not advance over a backup that did not happen"
    );
}
