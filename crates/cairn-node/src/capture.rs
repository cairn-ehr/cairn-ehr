//! #500 slice 2c — read one page of either plane's event set, map a row to the record the
//! medium carries, and (Task 7) run the loop that decides which events reach the medium.
//!
//! The read half is deliberately thin: `read_clinical_page` and `read_node_page` are one
//! `SELECT` apiece, and `to_medium_record` is a pure, field-by-field copy. Two things a
//! reader cannot get from the code alone:
//!
//! 1. **`dek_wrapped` never carries a plaintext key.** It is copied VERBATIM — already
//!    wrapped to THIS node's own unwrap public key, which is the key a restored node
//!    inherits (ADR-0066). There is nothing to translate here and nothing to decrypt; the
//!    mapping is a byte-for-byte copy, never an unwrap-then-rewrap.
//! 2. **The crypto-shred filter is not this module's job.** `db/051`'s
//!    `cairn_clinical_page` already selects through `event_custody_surviving` internally,
//!    so a shredded body's `dek_wrapped` arrives here already `NULL` — this code never
//!    sees the shred log and never decides anything about it. Re-deriving that filter here
//!    would be a second, driftable spelling of a safety predicate that must have exactly
//!    one home; `shred_predicate_has_one_home` enforces that this file stays a CALLER,
//!    never a second definition.
//!
//! [`capture_plane`] is the loop on top of them, and the safety-critical part of this
//! module: it decides which events reach a backup medium and which are skipped. Read its
//! doc before changing a line of it — a defect there silently loses patient records.

use tokio_postgres::Client;

use cairn_event::SigningKey;
use cairn_medium::{
    build_segment_attestation, chain_report, chain_tail, parse_any, segment_commitment,
    verify_and_append_segment, watermark, MediumImage, MediumRecord, MediumV3, Plane, Segment,
};

/// One row exactly as db/051's `cairn_clinical_page` hands it over — or, for the
/// federation plane, as `read_node_page`'s NULL-padded mirror of the same shape.
///
/// A thin, honest mirror of the function's `RETURNS TABLE`, deliberately NOT
/// `MediumRecord` itself: keeping the DB shape and the medium shape as two distinct types
/// means each can evolve independently, and the mapping between them (`to_medium_record`)
/// stays one small, reviewable function instead of being smeared across every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalRow {
    /// This plane's local sequence number for the row — the paging cursor Task 7's loop
    /// advances `after_seq` to.
    pub seq: i64,
    /// The event's COSE_Sign1 bytes, verbatim.
    pub signed_bytes: Vec<u8>,
    /// The human attestation token, when the database holds one for this row. Always
    /// `None` on the federation plane (see `read_node_page`).
    pub attestation: Option<Vec<u8>>,
    /// The attester's public key, when an attestation accompanies this row.
    pub attester_key: Option<Vec<u8>>,
    /// This event's DEK, already wrapped to this node's own unwrap public key. `None`
    /// means no custody travels: the event is unsealed, this node holds no DEK for it, or
    /// — on the clinical plane only — the body has been shredded (db/051's filter, not
    /// this module's).
    pub dek_wrapped: Option<Vec<u8>>,
}

/// Read one page of the CLINICAL plane via db/051's `cairn_clinical_page(after_seq,
/// page_limit)`. `after_seq` is an EXCLUSIVE cursor (rows with `seq > after_seq`);
/// `page_limit` bounds how many rows come back. The shred filter has already run inside
/// the database function by the time this reads the result — see the module doc.
pub async fn read_clinical_page(
    db: &Client,
    after_seq: i64,
    page_limit: i64,
) -> anyhow::Result<Vec<ClinicalRow>> {
    use anyhow::Context;
    let rows = db
        .query(
            "SELECT seq, signed_bytes, attestation, attester_key, dek_wrapped \
             FROM cairn_clinical_page($1, $2)",
            &[&after_seq, &page_limit],
        )
        .await
        .context("reading one clinical page from cairn_clinical_page")?;
    Ok(rows
        .iter()
        .map(|r| ClinicalRow {
            seq: r.get("seq"),
            signed_bytes: r.get("signed_bytes"),
            attestation: r.get("attestation"),
            attester_key: r.get("attester_key"),
            dek_wrapped: r.get("dek_wrapped"),
        })
        .collect())
}

/// Read one page of the FEDERATION plane (`node_event`), in the same [`ClinicalRow`]
/// shape `read_clinical_page` returns, so Task 7's paging loop can walk either plane with
/// one piece of code rather than two.
///
/// The three trailing `NULL`s are HONEST, not lazy. The federation plane — enroll, peer,
/// revoke, supersede — carries no human attestation token and no sealed-body custody, and
/// never will: a node-federation event is never encrypted and never requires a human
/// attester, so there is no value these three columns could hold. `NULL` says exactly
/// that ("this concept does not apply here"), which is a different fact from "a value was
/// expected but did not arrive" — the distinction `MediumRecord`'s own doc insists on
/// (`None` vs. `Some(vec![])`) applies at the source, not only at the medium.
pub async fn read_node_page(
    db: &Client,
    after_seq: i64,
    page_limit: i64,
) -> anyhow::Result<Vec<ClinicalRow>> {
    use anyhow::Context;
    let rows = db
        .query(
            "SELECT seq, signed_bytes, NULL::bytea, NULL::bytea, NULL::bytea \
             FROM node_event WHERE seq > $1 ORDER BY seq LIMIT $2",
            &[&after_seq, &page_limit],
        )
        .await
        .context("reading one federation-plane page from node_event")?;
    Ok(rows
        .iter()
        .map(|r| ClinicalRow {
            seq: r.get(0),
            signed_bytes: r.get(1),
            attestation: r.get(2),
            attester_key: r.get(3),
            dek_wrapped: r.get(4),
        })
        .collect())
}

/// Map one row to the record the medium carries. PURE — the whole DB-shape-to-medium-shape
/// decision lives in this one function, so a reviewer can hold the entire mapping in their
/// head rather than trusting that several call sites agree.
///
/// `dek_wrapped` is copied VERBATIM (see the module doc: it is already wrapped to this
/// node's unwrap key, so there is nothing to translate and no plaintext key material
/// passes through this path). `attestation` and `attester_key` are copied the same way,
/// preserving `None` (no token travelled) as distinct from `Some(vec![])` (an empty token
/// travelled) — collapsing that distinction is exactly the mistake `MediumRecord`'s own
/// doc warns against, because the clinical apply door reacts to the two differently.
pub fn to_medium_record(row: &ClinicalRow) -> MediumRecord {
    MediumRecord {
        signed_bytes: row.signed_bytes.clone(),
        attestation: row.attestation.clone(),
        attester_key: row.attester_key.clone(),
        dek_wrapped: row.dek_wrapped.clone(),
        source_seq: row.seq,
    }
}

// ---------------------------------------------------------------------------
// Task 7 — the plane-generic capture loop.
// ---------------------------------------------------------------------------

/// What one plane's capture did.
///
/// `watermark` is the value a SUBSEQUENT capture will resume from, re-derived from the
/// bytes actually on the medium — never the cursor this loop happened to advance to. The
/// two differ exactly when something we appended does not read back verifiably, and in
/// that case the honest answer is the un-advanced one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneCapture {
    pub plane: Plane,
    /// How many event records were appended, across every segment this call wrote. `0`
    /// means the medium was NOT touched at all (not even by an empty segment).
    pub records_appended: usize,
    /// The highest `source_seq` this medium can now be trusted to hold for `plane`, or
    /// `None` when no verified segment of that plane exists. `None` is not zero: zero is
    /// a claim, absence is the honest answer (`cairn_medium::watermark`).
    pub watermark: Option<i64>,
}

/// Capture everything `plane` holds above the medium's watermark, as one or more appended
/// segments. **This is the safety-critical heart of #500 slice 2c.**
///
/// # The properties, in the order they matter
///
/// 1. **Resume from the WATERMARK, never from the file tail.** The cursor comes from
///    `cairn_medium::watermark`, which is derived from the last VERIFIED segment. An
///    unverifiable trailing segment — a torn append — therefore does NOT advance it, and
///    its records are re-captured. That is what makes an interrupted backup cost exactly
///    one increment rather than losing the events that were in flight.
/// 2. **An unchanged log appends NOTHING** — not an empty segment, not a byte. The loop
///    reads a page BEFORE it builds anything, and returns on an empty one. This is the
///    property CAIRNB3 exists for: a nightly backup of a quiet clinic must not grow the
///    medium, or a year of nightly backups is a year of re-recorded history.
/// 3. **No event is captured twice.** `after_seq` is a strict cursor (`seq > after_seq`)
///    advanced to the max seq of each appended page. Set-union makes a duplicate harmless
///    to correctness, which is exactly why nothing else in the system would ever notice
///    one — so the guarantee has to hold here.
/// 4. **A missing signing key never blocks a capture.** An unattended cron run has no
///    passphrase and therefore no key; `signer: None` writes the segment UNSIGNED and
///    flagged (it still NAMES itself via `self_node_id_hex`). Refusing would force a
///    second human act and break the slice's §1.2 paper-parity budget (M must stay 1).
/// 5. **Verify before the bytes can touch the medium.** Every append goes through
///    `verify_and_append_segment`, because a segment attestation is computed over the
///    content address of whatever bytes it is handed: feed the writer a corrupt read and
///    it signs a genuinely VALID attestation over corruption, which then reports itself
///    intact forever.
///
/// # What this function does NOT do: durability
///
/// It takes `medium: &mut Vec<u8>` and does no I/O whatsoever — `cairn-medium` is a pure
/// crate and this loop stays on the same side of that line. The caller owes the durability
/// half of `append_segment`'s contract: **write the buffer and `sync_all()` it BEFORE any
/// health record advances**, so a backup-health sidecar can never claim a medium the disk
/// does not hold. Today's caller (`backup::backup_to`) writes the whole image atomically
/// once, which is strictly stronger — the rename either lands whole or not at all — so no
/// torn tail can arise from our own writer. The torn-tail resumption in property 1 defends
/// against everything else: a different writer, a removable medium yanked mid-write, a
/// filesystem that reordered a non-atomic append.
///
/// # `page_events`
///
/// The caller passes `cairn_wire::DEFAULT_PAGE_EVENTS` (500). The capture and the slice-2b
/// PULL deliberately agree on that number because they are the same operation seen from two
/// sides: ADR-0026 decision 2 makes a backup medium *a cold peer*, so a page here is a page
/// there — same ≈2 MiB working set, same bound on how much work one interruption discards.
/// It is a parameter rather than a constant so a test can drive the multi-page path with a
/// tiny page, and so `cairn-node` need not depend on `cairn-wire` to name it.
///
/// # Refusals
///
/// Three things this loop cannot legitimately do, each refused BY NAME rather than
/// silently returning "nothing captured" — a successful-looking backup that gained nothing
/// is the composite untruth #500 is about:
///
///   - a CAIRNB1/CAIRNB2 medium, which has no segment chain to append to;
///   - a medium whose verified chain does not reach the end of the file (see the guard
///     below for why appending there would poison every future capture);
///   - `page_events < 1`, which can never make progress.
pub async fn capture_plane(
    db: &Client,
    medium: &mut Vec<u8>,
    plane: Plane,
    signer: Option<(&SigningKey, &str)>,
    self_id_hex: &str,
    page_events: i64,
) -> anyhow::Result<PlaneCapture> {
    // A page of zero (or a negative page) can never advance the cursor. Refused up front,
    // because the alternative is an unattended nightly backup spinning forever.
    if page_events < 1 {
        anyhow::bail!("page_events must be at least 1 to make progress, got {page_events}");
    }
    // Authoring a segment for a plane this build cannot name is a programming error, not a
    // runtime condition: `Plane::Unknown` only ever arrives by READING a newer Cairn's
    // medium. `build_segment_attestation` debug-asserts the same thing; refusing here
    // covers the UNSIGNED path too, and in release builds.
    if !plane.is_known() {
        anyhow::bail!(
            "refusing to capture into an unknown plane tag {} — this build cannot author one",
            plane.tag()
        );
    }

    // 1. Parse the existing image. A fresh medium is `serialize_v3(&[])`; anything else is
    //    whatever the last capture left behind, torn tail included.
    let m: MediumV3 = match parse_any(medium)? {
        MediumImage::V3(m) => m,
        MediumImage::Legacy(_) => anyhow::bail!(
            "this is a CAIRNB1/CAIRNB2 medium and a capture appends CAIRNB3 segments; \
             start a new medium (or restore this one and re-capture) rather than mixing \
             revisions in one file"
        ),
    };

    // A torn tail must be CUT BEFORE anything is appended (`MediumV3::complete_bytes`, I4):
    // otherwise the torn remnant becomes the next section's `[u32 length]` prefix and
    // parsing stops there forever, silently orphaning every later backup. The records that
    // were in the torn segment are not lost — the watermark below never counted them, so
    // the loop re-captures them.
    if m.truncated_tail {
        medium.truncate(m.complete_bytes);
    }

    // 2. One chain pass answers both questions a writer has: how far the medium can be
    //    trusted (the watermark → our cursor), and where the next segment goes.
    let report = chain_report(&m);
    let entry_watermark = watermark(&m, &report, plane);
    let tail = chain_tail(&m, &report);

    // GUARD — the chain must reach the END of the file before we append to it.
    //
    // `chain_tail` follows the last VERIFIED segment, which is what we want for a torn tail
    // (parse already dropped the incomplete section, so the two coincide). But a COMPLETE
    // segment that fails to verify mid-file leaves `next_index` pointing behind
    // `segments.len()`: the segment we appended would sit at file position N while
    // declaring index N-k, so every future read raises `IndexMismatch` on it, it never
    // becomes verified, and every subsequent capture re-captures the same records and
    // appends another mismatched segment. A damaged medium would grow without bound while
    // reporting a healthy-looking capture. Refuse instead, and name the faults so the
    // operator can act.
    if tail.next_index as usize != m.segments.len() {
        anyhow::bail!(
            "refusing to append to a medium whose verified chain stops at segment {} of {}: \
             a segment appended here would declare an index that does not match its file \
             position and could never verify. Faults: {:?}",
            tail.next_index,
            m.segments.len(),
            report.faults
        );
    }

    // The cursor. `unwrap_or(0)` is the same "full sweep" convention the slice-2b pull uses:
    // both `event_log.seq` and `node_event.seq` are `GENERATED ALWAYS AS IDENTITY`, so they
    // start at 1 and a strict `seq > 0` selects everything. `None` (nothing verified) and
    // "start from the beginning" are therefore the same instruction, which is why collapsing
    // them here is safe — see `watermark`'s doc for why the two must stay distinct in the
    // TYPE.
    let mut after = entry_watermark.unwrap_or(0);
    let mut index = tail.next_index;
    let mut prev = tail.prev_commitment;
    let mut records_appended = 0usize;

    loop {
        // 3. Read one page plus a LOOKAHEAD row. The extra row answers "is there another
        //    page?" without a second round trip, so a capture that exactly fills its last
        //    page does not pay an extra query to discover it is done.
        let rows = read_plane_page(db, plane, after, page_events.saturating_add(1)).await?;
        if rows.is_empty() {
            // PROPERTY 2. Nothing new: return without appending anything at all. This is the
            // ONLY place the loop can exit having written nothing, and it must stay that way.
            break;
        }
        let more_pages_follow = rows.len() as i64 > page_events;
        let page = &rows[..rows.len().min(page_events as usize)];

        // 4. Build the segment. `to_medium_record` is the single mapping; the attestation is
        //    minted only when a key is available (property 4).
        let records: Vec<MediumRecord> = page.iter().map(to_medium_record).collect();
        let attestation = signer.map(|(sk, key_id)| {
            build_segment_attestation(sk, key_id, self_id_hex, plane, index, &prev, &records)
        });
        let segment = Segment {
            plane,
            index,
            prev_commitment: prev.clone(),
            // Present even when unsigned: an unsigned segment still NAMES itself, which is
            // what closes the operator-typo footgun without a key. UNTRUSTED on read — see
            // `Segment::self_node_id_hex`.
            self_node_id_hex: self_id_hex.to_string(),
            attestation,
            records,
        };

        // 5. PROPERTY 5 — verify-before-write. `verify_and_append_segment` refuses any record
        //    whose signature this node cannot verify right now, BEFORE a byte reaches the
        //    buffer, and leaves the medium byte-identical when it refuses. Using the crate's
        //    own door rather than an open-coded check keeps one definition of the refusal.
        verify_and_append_segment(medium, &segment)?;
        records_appended += segment.records.len();

        // Advance the cursor to the highest seq we actually wrote. MAX rather than "the last
        // row", so a future change to the page query's ordering can only cost a duplicate
        // (harmless under set-union) and can never REWIND the cursor into an endless loop.
        let next_after = page
            .iter()
            .map(|r| r.seq)
            .max()
            .expect("a non-empty page has a maximum seq");
        // Belt and braces: both page readers select `seq > after_seq`, so this cannot fire
        // today. It is here because the failure it prevents — an unattended backup looping
        // forever, growing the medium with duplicate segments — is far worse than the
        // refusal.
        if next_after <= after {
            anyhow::bail!(
                "the {plane:?} page reader returned rows at or below the cursor ({next_after} \
                 <= {after}); refusing to loop"
            );
        }
        after = next_after;

        // Where the NEXT segment goes. `chain_tail` is the one derivation of this for a
        // medium read off disk (#522), and it is what seeded `index`/`prev` above; inside
        // the loop we advance from a segment we just wrote ourselves, using the same
        // `segment_commitment` `chain_tail` calls. Re-parsing the whole image per page to
        // ask `chain_tail` again would make one capture O(pages × medium size) — the exact
        // cost `append_segment` exists to avoid. A divergence between the two would surface
        // loudly as an `IndexMismatch` on the next read, never silently.
        prev = segment_commitment(&segment.records);
        index += 1;

        if !more_pages_follow {
            break;
        }
    }

    // 6. Report the plane's new watermark, RE-DERIVED from the bytes now on the medium
    //    rather than from `after`. `after` is what we asked the database for; the watermark
    //    is what a restore (or the next capture) will actually be able to trust, and the two
    //    differ precisely when an appended segment does not read back verifiably. Skipped
    //    when nothing was appended: the medium is unchanged, so the entry value is the same
    //    answer without a second O(medium) parse.
    let final_watermark = if records_appended == 0 {
        entry_watermark
    } else {
        match parse_any(medium)? {
            MediumImage::V3(after_image) => {
                let after_report = chain_report(&after_image);
                watermark(&after_image, &after_report, plane)
            }
            // Unreachable in practice — we appended to a CAIRNB3 image and appends never
            // rewrite the magic — but written as a refusal rather than an `expect` so a
            // future format change cannot turn this into a panic on an unattended backup.
            MediumImage::Legacy(_) => anyhow::bail!(
                "the medium stopped being CAIRNB3 during a capture; refusing to report a \
                 watermark for it"
            ),
        }
    };

    Ok(PlaneCapture {
        plane,
        records_appended,
        watermark: final_watermark,
    })
}

/// One page of whichever plane is being captured. The single place the plane tag is turned
/// into a table read, so [`capture_plane`] itself stays plane-generic — there is one loop,
/// not two that must be kept in step.
///
/// `Plane::Unknown` is unreachable: [`capture_plane`] refuses it before the loop starts. It
/// is answered with an error rather than a panic anyway, because this is a backup path and
/// an unattended node must degrade to a refusal, never a crash.
async fn read_plane_page(
    db: &Client,
    plane: Plane,
    after_seq: i64,
    page_limit: i64,
) -> anyhow::Result<Vec<ClinicalRow>> {
    match plane {
        Plane::Clinical => read_clinical_page(db, after_seq, page_limit).await,
        Plane::Node => read_node_page(db, after_seq, page_limit).await,
        Plane::Unknown(tag) => {
            anyhow::bail!("no page source exists for unknown plane tag {tag}")
        }
    }
}
