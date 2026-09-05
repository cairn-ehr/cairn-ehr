//! #500 slice 2c Task 7 — the plane-generic capture loop: which events reach a backup
//! medium, and which are skipped.
//!
//! SPLIT FROM `capture/mod.rs`, and not on a line count. The two halves have genuinely
//! different reasons to change — the same seam `cairn-medium` draws between `verify` ("do
//! these bytes verify") and `chain` ("what can this medium be trusted for"):
//!
//!   - `capture/mod.rs` tracks the DATABASE shape. `ClinicalRow` mirrors db/051's
//!     `RETURNS TABLE`, and `to_medium_record` is the one mapping onto the wire record. It
//!     changes when a column moves.
//!   - this file tracks the CAPTURE POLICY: resumption, paging, gap backfill, and what a
//!     capture refuses to do. It changes when the durability story changes.
//!
//! The whole of the policy's rationale is written out below rather than assumed, because
//! most of what this file prevents is invisible to a reader who has not thought about it —
//! an interleaved `IDENTITY` commit, a torn tail, an attestation signed over a corrupt read.
//! A defect here silently loses patient records.

use tokio_postgres::Client;

use cairn_event::SigningKey;
use cairn_medium::{
    build_segment_attestation, chain_report, chain_tail, parse_any, segment_commitment, seq_gaps,
    verify_and_append_segment, watermark, MediumImage, MediumRecord, MediumV3, Plane, Segment,
};

use super::{read_clinical_page, read_node_page, to_medium_record, ClinicalRow};

/// What one plane's capture did.
///
/// `watermark` is the value a SUBSEQUENT capture will resume from, re-derived from the
/// bytes actually on the medium — never the cursor this loop happened to advance to. The
/// two differ exactly when something we appended does not read back verifiably, and in
/// that case the honest answer is the un-advanced one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaneCapture {
    pub plane: Plane,
    /// How many event records were appended, across every segment this call wrote —
    /// backfilled records included. `0` means the medium was NOT touched at all (not even
    /// by an empty segment).
    pub records_appended: usize,
    /// The highest `source_seq` this medium can now be trusted to hold for `plane`, or
    /// `None` when no verified segment of that plane exists. `None` is not zero: zero is
    /// a claim, absence is the honest answer (`cairn_medium::watermark`).
    ///
    /// A high-water MARK is not a completeness claim — read it together with
    /// [`PlaneCapture::unfilled_gaps`], never alone.
    pub watermark: Option<i64>,
    /// Holes BELOW the watermark that this capture could not fill, as `(after, before)`
    /// exclusive pairs — `(3, 7)` meaning seqs 4, 5 and 6 are absent between the 3 and the
    /// 7 the medium holds. Normally empty.
    ///
    /// WHY THIS IS REPORTED AND NOT SWALLOWED. A gap is either transient (a seq committed
    /// after the capture read past it — see [`capture_plane`]'s backfill) or permanent (an
    /// identity value burned by a rolled-back transaction, which is never issued again).
    /// The backfill closes every transient one on the next run, so a gap that PERSISTS is a
    /// standing hole in the backup. Leaving that invisible while `watermark` says
    /// `Some(N)` would rebuild, one layer up, exactly the composite untruth #500 is about:
    /// every surface honest, the assembled picture false. The caller decides what to do
    /// with it (Cairn ships mechanism, not policy — principle 9); this type's job is to
    /// make it impossible to read the watermark without also being handed this.
    pub unfilled_gaps: Vec<(i64, i64)>,
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
/// 6. **A watermark is a high-water MARK, not a completeness claim** — so every capture
///    also backfills the holes below it. See the section on gaps.
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
/// # Gaps below the watermark — the policy this slice owes
///
/// `event_log.seq` and `node_event.seq` are `GENERATED ALWAYS AS IDENTITY`: the value is
/// handed out at INSERT, and **commits can land out of order**. Two overlapping
/// `submit_event` transactions take seqs 5 and 6; 6 commits first; a capture reading in
/// between sees 6 and not 5. Resuming from the watermark alone, seq 5 would be **skipped by
/// every future run for the life of the medium**, while the medium reported itself complete
/// through 6 — a clinical event silently absent from a backup that says it is whole, which
/// is #500 in miniature. `cairn_medium::watermark`'s own doc names this scenario and hands
/// the decision to "the slice that owns capture". This is that slice.
///
/// **The policy: fill from the medium's own reported holes, before extending the tail.**
/// `cairn_medium::seq_gaps` reports every hole in the verified prefix; each is re-queried
/// against the database and whatever the database still holds is appended. `cairn-sync`
/// solves the same hazard with a periodic full sweep from seq 0, which is not available
/// here — a nightly sweep would re-append the whole log and destroy property 2 — so the
/// medium telling us precisely what it is missing is the bounded equivalent.
///
/// **Transient vs. permanent gaps are deliberately NOT distinguished**, because the fill is
/// a QUERY, not a retry: we ask for the hole's contents and append whatever comes back. A
/// transient gap (the late commit) yields its rows and closes. A permanent gap — an
/// identity value burned by a rolled-back transaction, which PostgreSQL never re-issues —
/// yields an empty page, which appends nothing (property 2 is preserved) and leaves the
/// hole reported in [`PlaneCapture::unfilled_gaps`] rather than retried forever. There is
/// no retry counter to get wrong and no loop to run away: each gap costs one bounded query
/// per capture whether or not it can ever be filled, and termination comes from the cursor
/// strictly advancing or the page being empty. Telling the two apart would require asking
/// the database a question it cannot answer ("was this seq ever issued?"), and guessing
/// would risk abandoning a hole that a slow transaction was about to fill.
///
/// A backfilled segment carries `source_seq` values BELOW ones already on the medium. That
/// is fine and needs no reordering: the chain is by FILE order, the watermark is a `max`,
/// and restore is set-union.
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
    //
    // ⚠️ KNOWN LIMITATION — #523, and it belongs to `cairn-medium`, not here. A `?` on this
    // line refuses the whole capture, and one real-world artifact reaches it: the classic
    // ext4 delayed-allocation power-loss tail, where the interrupted section arrives as a
    // FULL-LENGTH run of NUL bytes rather than a short one. `take_section` then reads an
    // honest-looking length over a malformed body and returns `Damaged` — so the
    // truncate-to-`complete_bytes` recovery ten lines below, which handles every SHORT tear,
    // never runs. Until #523 lands ("a corrupt section length under the cap is
    // indistinguishable from a torn tail, and the two remedies are opposite"), expect such a
    // medium to be refused rather than repaired: the operator's remedy is to start a new
    // medium, and no data is lost, because a refused capture writes nothing and the previous
    // good medium is untouched. Fixing it HERE would mean this file guessing at a format
    // ambiguity, which is the one thing #523 says must not be done piecemeal.
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
             position and could never verify. This medium is damaged from segment {} onward \
             and cannot be repaired by appending — start a NEW medium for the next backup \
             and keep this one, whose segments up to {} are still verified and restorable. \
             Faults: {:?}",
            tail.next_index,
            m.segments.len(),
            tail.next_index,
            tail.next_index,
            report.faults
        );
    }

    // Where the next segment goes. Seeded ONCE from `chain_tail` (#522's single derivation)
    // and then advanced by the appends we make ourselves — see `ChainCursor`.
    let mut cursor = ChainCursor {
        index: tail.next_index,
        prev_commitment: tail.prev_commitment,
    };
    let mut records_appended = 0usize;

    // 3. BACKFILL FIRST, tail second. The holes the medium reports are the ones a previous
    //    capture read past while a lower seq was still uncommitted; see the "Gaps below the
    //    watermark" section above for why a capture cannot instead sweep from seq 0. Doing
    //    it before the tail keeps this pass reading the report we already computed, so the
    //    whole capture still parses the image exactly once on the way in.
    let entry_gaps = seq_gaps(&m, &report, plane);
    for &(gap_after, gap_before) in &entry_gaps {
        let mut at = gap_after;
        loop {
            // No lookahead here: a gap is bounded on BOTH sides, so the filter below is what
            // ends the walk, not a "was there another page" probe.
            let rows = read_plane_page(db, plane, at, page_events).await?;
            // Everything the database still holds strictly inside the hole. Filtered rather
            // than `take_while`d so the bound does not depend on the query's row order.
            let page: Vec<ClinicalRow> = rows.into_iter().filter(|r| r.seq < gap_before).collect();
            if page.is_empty() {
                // Either the hole is closed or the database can never supply it. Both end the
                // walk here, appending nothing — see the doc above for why this loop does not
                // try to tell them apart.
                break;
            }
            let highest = page
                .iter()
                .map(|r| r.seq)
                .max()
                .expect("a non-empty page has a maximum seq");
            records_appended +=
                append_as_segment(medium, plane, signer, self_id_hex, &mut cursor, &page)?;
            if highest <= at {
                anyhow::bail!(
                    "the {plane:?} page reader returned rows at or below the backfill cursor \
                     ({highest} <= {at}); refusing to loop"
                );
            }
            at = highest;
        }
    }

    // 4. Extend the tail. `unwrap_or(0)` is the same "full sweep" convention the slice-2b
    //    pull uses: both `event_log.seq` and `node_event.seq` are `GENERATED ALWAYS AS
    //    IDENTITY`, so they start at 1 and a strict `seq > 0` selects everything. `None`
    //    (nothing verified) and "start from the beginning" are therefore the same
    //    instruction, which is why collapsing them here is safe — see `watermark`'s doc for
    //    why the two must stay distinct in the TYPE.
    let mut after = entry_watermark.unwrap_or(0);

    loop {
        // Read one page plus a LOOKAHEAD row. The extra row answers "is there another page?"
        // without a second round trip, so a capture that exactly fills its last page does
        // not pay an extra query to discover it is done.
        let rows = read_plane_page(db, plane, after, page_events.saturating_add(1)).await?;
        if rows.is_empty() {
            // PROPERTY 2. Nothing new: return without appending anything at all. This and
            // the backfill's empty page are the only exits that write nothing, and they must
            // stay that way.
            break;
        }
        let more_pages_follow = rows.len() as i64 > page_events;
        let page = &rows[..rows.len().min(page_events as usize)];

        records_appended +=
            append_as_segment(medium, plane, signer, self_id_hex, &mut cursor, page)?;

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

        if !more_pages_follow {
            break;
        }
    }

    // 5. Report the plane's new watermark AND its remaining holes, RE-DERIVED from the bytes
    //    now on the medium rather than from `after`. `after` is what we asked the database
    //    for; these two are what a restore (or the next capture) will actually be able to
    //    trust, and they differ precisely when an appended segment does not read back
    //    verifiably. Skipped when nothing was appended: the medium is unchanged, so the entry
    //    values are the same answers without a second O(medium) parse.
    let (final_watermark, unfilled_gaps) = if records_appended == 0 {
        (entry_watermark, entry_gaps)
    } else {
        match parse_any(medium)? {
            MediumImage::V3(after_image) => {
                let after_report = chain_report(&after_image);
                (
                    watermark(&after_image, &after_report, plane),
                    seq_gaps(&after_image, &after_report, plane),
                )
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
        unfilled_gaps,
    })
}

/// Where the next appended segment goes, while a capture is mid-flight.
///
/// [`chain_tail`] is the ONE derivation of this for a medium read off disk (#522), and it is
/// what seeds this struct. Inside a capture we advance it ourselves from segments we just
/// wrote, using the same `segment_commitment` `chain_tail` calls: re-parsing the whole image
/// per page to ask `chain_tail` again would make one capture O(pages × medium size), the
/// exact cost `append_segment` exists to avoid. A divergence between the two derivations
/// would surface loudly as an `IndexMismatch` on the next read, never silently — and
/// `capture_plane`'s chain-reach guard refuses to append to a medium in that state at all.
struct ChainCursor {
    index: u32,
    prev_commitment: String,
}

/// Build one segment from a page of rows, verify it, append it, and advance `cursor`.
/// Returns how many records were appended.
///
/// Shared by the backfill pass and the tail pass so the two can never drift into two
/// different ideas of how a segment is built — the mistake #522 exists to prevent, one layer
/// down. Appends only; the caller owns the paging cursor and the loop's exit conditions.
fn append_as_segment(
    medium: &mut Vec<u8>,
    plane: Plane,
    signer: Option<(&SigningKey, &str)>,
    self_id_hex: &str,
    cursor: &mut ChainCursor,
    page: &[ClinicalRow],
) -> anyhow::Result<usize> {
    // `to_medium_record` is the single DB-shape-to-medium-shape mapping; the attestation is
    // minted only when a key is available (property 4).
    let records: Vec<MediumRecord> = page.iter().map(to_medium_record).collect();
    let attestation = signer.map(|(sk, key_id)| {
        build_segment_attestation(
            sk,
            key_id,
            self_id_hex,
            plane,
            cursor.index,
            &cursor.prev_commitment,
            &records,
        )
    });
    let segment = Segment {
        plane,
        index: cursor.index,
        prev_commitment: cursor.prev_commitment.clone(),
        // Present even when unsigned: an unsigned segment still NAMES itself, which is what
        // closes the operator-typo footgun without a key. UNTRUSTED on read — see
        // `Segment::self_node_id_hex`.
        self_node_id_hex: self_id_hex.to_string(),
        attestation,
        records,
    };

    // PROPERTY 5 — verify-before-write. `verify_and_append_segment` refuses any record whose
    // signature this node cannot verify right now, BEFORE a byte reaches the buffer, and
    // leaves the medium byte-identical when it refuses. Using the crate's own door rather
    // than an open-coded check keeps one definition of the refusal.
    verify_and_append_segment(medium, &segment)?;

    cursor.prev_commitment = segment_commitment(&segment.records);
    cursor.index += 1;
    Ok(segment.records.len())
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
