//! Applying a backup medium's CLINICAL plane — the half of disaster recovery a solo clinic's
//! survival actually depends on (#554 slice 2d).
//!
//! # What this is
//!
//! Slice 2c made `cairn-node backup` write a CAIRNB3 medium carrying every `event_log` row
//! with its wrapped DEK beside it. Nothing read one back: `restore` went through
//! [`crate::backup::node_plane_events`], which returns the federation plane alone on purpose,
//! so a solo clinic backed up nightly, passed `verify-backup`, lost its disk, and restored a
//! node that knew who it had peered with and **zero patients**. This module is the reader.
//!
//! It sits beside [`super::apply_medium`] (the federation plane's applier) rather than inside
//! it, because the two planes differ in three ways that are all safety-relevant: the clinical
//! plane carries custody, it goes through a different door, and a per-event refusal must not
//! abort the run.
//!
//! # Three decisions that are easy to undo by accident
//!
//! **1. The DEK reaching the door is PLAINTEXT, not the carried wrapped one.**
//! `apply_remote_event`'s `p_dek` parameter is fed straight into `cairn_wrap_dek(p_dek,
//! v_pub)` — **the door wraps what it is handed**. Both carriers (a `MediumRecord`'s
//! `dek_wrapped` and the export's `EpisodeDek`) hold keys that are *already* wrapped to this
//! node's unwrap public key, so piping either through would **double-wrap every key in the
//! clinic's record**. Every test would still pass: the `event_dek` rows exist, the counts
//! agree, `verify-backup` is green — and the defect surfaces months later when a clinician
//! opens a chart on a node that can no longer be re-restored. The door also *needs* the
//! plaintext: `cairn_unseal_body(container, dek, event_id)` takes the DEK itself, and without
//! a clear view there is no twin, no projection, no chart. So the unwrap happens **here**, in
//! Rust, and the door re-wraps to the registered public half. This is not a new idiom —
//! `cairn-sync`'s `do_pull` does exactly the same on every pull, unwrapping at the call site
//! and handing `apply_signed` the plaintext.
//!
//! **2. A per-event refusal skips, counts and PENS — it never aborts.** In the one command
//! that exists for the disaster where re-running the backup is impossible, converting a
//! partial loss into a total one is the wrong trade. Same ruling 2c made for a torn medium.
//! No confirmation dialog: principle 3 rejects those as a safety mechanism.
//!
//! **3. The no-export test is CUSTODY, never SEALEDNESS**, and they are different questions.
//! A body shredded *before* its first capture is sealed and arrives with `dek_wrapped = None`
//! — its ciphertext travels, its key was destroyed (ADR-0005: a shred destroys the key, never
//! the event) — and it must restore custody-less, exactly as it stands on the dead node.
//! Keying off sealedness would pen it forever over a key that does not exist and is not
//! supposed to.
//!
//! # Why this does NOT reuse db/020's lenient missing-key arm
//!
//! That arm downgrades a missing unwrap key to a `WARNING` and admits the event without
//! custody. Correct for a **puller**, which will see the DEK again on a later cycle. For a
//! **restore** it would admit ciphertext into a node that `finalize_identity` then fences,
//! with no second delivery ever. Penning instead keeps both the bytes and the key, so
//! recovering the export later and running `cairn-sync requeue` completes the restore without
//! redoing it.

use std::collections::BTreeMap;

use cairn_event::seal::Secret32;
use cairn_medium::MediumRecord;
use tokio_postgres::Client;

/// The `peer` value a restore-penned row carries.
///
/// An explicit sentinel, not an empty string: `sync_quarantine.peer` is `NOT NULL` and the
/// per-peer quota probes filter on it, so a restore-penned row must be identifiable as one
/// rather than blend into an unnamed link. An operator running `cairn-sync quarantine` after a
/// disaster needs to see at a glance which rows came from their medium.
pub const RESTORE_PEER_SENTINEL: &str = "(restore)";

/// The ordinary per-peer pen quota, mirrored from `cairn-sync` so a restore can SAY when it
/// has exceeded what a sync link would have been allowed — see [`ClinicalRestoreReport::
/// exceeds_ordinary_quota`]. A restore passes NULL to the door (unbounded) and reports
/// instead of enforcing; these are the numbers it reports against.
///
/// ⚠️ A second home for two constants whose first home is `cairn-sync`'s `main.rs`, which is
/// a **binary-only crate this one cannot depend on**. That is the same wall that put the pen
/// itself in the database (db/052). They are used here for a MESSAGE, never for a decision, so
/// a drift costs an inaccurate sentence rather than a wrong verdict — but if `cairn-sync` ever
/// gains a `lib.rs`, these should follow the pen and stop being copied.
pub const ORDINARY_QUOTA_ROWS: usize = 10_000;
pub const ORDINARY_QUOTA_BYTES: usize = 64 * 1024 * 1024;

/// What a clinical restore did, in the shape the operator summary needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClinicalRestoreReport {
    /// Records offered to the apply door that it admitted.
    pub applied: usize,
    /// Records the door accepted as a set-union no-op because this node already held them.
    ///
    /// Counted separately from `applied` because on a RESUMED restore this is most of the
    /// medium, and folding the two together would make a resume indistinguishable from a
    /// first run that silently did nothing.
    pub already_present: usize,
    /// Records refused and penned, by the reason they were refused.
    ///
    /// A map rather than a count, because #536's lesson is that a refusal nobody can name is
    /// a refusal nobody can fix — and the restore path is where an operator has the least
    /// context and the most at stake.
    pub refusals: BTreeMap<String, usize>,
    /// Total `signed_bytes` penned, for the disk-cost line in the summary.
    pub penned_bytes: usize,
}

impl ClinicalRestoreReport {
    /// How many records were refused and penned.
    pub fn penned(&self) -> usize {
        self.refusals.values().sum()
    }

    /// Whether this restore's pen exceeded what an ordinary sync peer would have been
    /// allowed. **A report, never a gate** — the quota does not apply to a restore (db/052),
    /// and this exists so "unbounded" does not silently mean "unreported". A bound the
    /// operator can see beats a bound that drops the record.
    pub fn exceeds_ordinary_quota(&self) -> bool {
        self.penned() > ORDINARY_QUOTA_ROWS || self.penned_bytes > ORDINARY_QUOTA_BYTES
    }
}

/// Why one record could not be applied, in the vocabulary the pen stores and the summary
/// counts. **Pure.**
///
/// Restore-side refusals carry their OWN text, because since the unwrap moved into Rust
/// (decision 1 above) an unwrap failure never reaches the door and so has no door text to
/// quote. Door refusals carry the door's, prefixed. Both are prefixed identically so a row in
/// `sync_quarantine` says where it came from — an operator reading the pen weeks later cannot
/// otherwise tell a restore refusal from a peer's.
pub fn pen_reason(cause: &RefusalCause) -> String {
    match cause {
        RefusalCause::NoCustodyKey => format!(
            "{PREFIX}: this record carries a wrapped DEK but no custody key was installed, so \
             the key could not be opened and would have been lost. The bytes AND the key are \
             held here: recover the local-state export, then `cairn-sync requeue` to complete \
             the restore without redoing it."
        ),
        RefusalCause::DekWillNotOpen => format!(
            "{PREFIX}: this record's wrapped DEK did not open with the installed custody key. \
             The key held here is the one from the medium; if the wrong export was restored, \
             recover the right one and `cairn-sync requeue`."
        ),
        RefusalCause::Door(text) => format!("{PREFIX}: the apply door refused it — {text}"),
    }
}

/// The prefix every restore-penned reason carries, so the pen says where a row came from.
const PREFIX: &str = "restore";

/// Why one record was refused. Separated from its text so the two cannot drift and so the
/// summary can group without string-matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefusalCause {
    /// The record carries custody and this node installed no key to open it (design §6's
    /// no-export path). **Keyed on the record's CUSTODY, never on whether the event is
    /// sealed** — see the module header.
    NoCustodyKey,
    /// A key was installed and the record's DEK still did not open under it.
    DekWillNotOpen,
    /// `apply_remote_event` refused, carrying its own legible text.
    Door(String),
}

/// A short, groupable label for a refusal — what the summary counts by.
///
/// The full [`pen_reason`] text is what the PEN stores (an operator inspecting one row wants
/// the remedy); this is what the SUMMARY groups by (an operator reading a restore wants to
/// know *which kinds* of thing failed, and how many of each, without three thousand lines).
/// Door refusals collapse to one label deliberately: their texts carry event ids and would
/// otherwise make every refusal its own group, which is a list, not a summary.
pub fn refusal_label(cause: &RefusalCause) -> &'static str {
    match cause {
        RefusalCause::NoCustodyKey => "carries custody, no key installed to open it",
        RefusalCause::DekWillNotOpen => "custody key would not open this record's DEK",
        RefusalCause::Door(_) => "refused by the apply door",
    }
}

/// Apply a medium's clinical records, in the order [`crate::backup::clinical_plane_records`]
/// returns them (ascending `source_seq` — see that function for why the sort is load-bearing).
///
/// **MUST run while the database is still un-enrolled and AFTER custody and the actor
/// registry are installed.** Both halves of that are design §3's reordered ceremony:
/// `apply_remote_event` wraps each DEK to the registered public half (so custody first), every
/// apply door resolves its author through `actor_current` (so the registry first), and
/// `finalize_identity` runs last so a failure here leaves a database that is still restorable
/// from the same medium.
///
/// `unwrap_secret` is `None` when no usable export was applied — no passphrase (every
/// unattended cron run), a corrupt `.lsk`, or an export carrying rows but no key. In that case
/// a record with no custody restores normally and a record that CARRIES custody is refused
/// here, with that custody preserved in the pen.
pub async fn apply_clinical_plane(
    db: &Client,
    records: &[MediumRecord],
    unwrap_secret: Option<&Secret32>,
) -> anyhow::Result<ClinicalRestoreReport> {
    let mut report = ClinicalRestoreReport::default();

    for record in records {
        // Step 1 — resolve custody, in Rust, before the door is ever called.
        let dek = match (&record.dek_wrapped, unwrap_secret) {
            // No custody on this record. Restores normally with a NULL `p_dek`. This is the
            // arm a body shredded BEFORE its first capture takes: sealed, keyless, and
            // legitimately so. Keying this decision on sealedness instead would pen it
            // forever over a key that was destroyed on purpose.
            (None, _) => None,
            (Some(_), None) => {
                pen(db, record, RefusalCause::NoCustodyKey, &mut report).await?;
                continue;
            }
            (Some(wrapped), Some(secret)) => match cairn_event::seal::unwrap_dek(wrapped, secret) {
                Ok(plain) => Some(plain),
                Err(_) => {
                    // The error is deliberately not quoted: it is a decryption failure whose
                    // text says nothing an operator can act on, and the remedy — recover the
                    // matching export — is the same whichever way it failed.
                    pen(db, record, RefusalCause::DekWillNotOpen, &mut report).await?;
                    continue;
                }
            },
        };

        // Step 2 — the newness probe, for the counts only. The door is idempotent (a re-apply
        // of identical bytes is a set-union no-op), so this never gates admission — it exists
        // so a resumed restore can be told apart from one that did nothing.
        let content_address = cairn_event::event_address(&record.signed_bytes);
        let existed: bool = db
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM event_log WHERE content_address = $1)",
                &[&content_address],
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "reading this node's own event_log failed during a restore ({}). This is \
                     a LOCAL fault, not a bad medium: the restore stops rather than penning \
                     every remaining record under a diagnosis that would be wrong.",
                    crate::db_diagnosis::legible_db_error(&e)
                )
            })?
            .get(0);

        // Step 3 — the one door. The DEK is the PLAINTEXT (module header, decision 1).
        match db
            .execute(
                "SELECT apply_remote_event($1, $2, $3, $4)",
                &[
                    &record.signed_bytes,
                    &record.attestation,
                    &record.attester_key,
                    &dek.as_ref().map(|d| d.as_bytes().to_vec()),
                ],
            )
            .await
        {
            Ok(_) => {
                if existed {
                    report.already_present += 1;
                } else {
                    report.applied += 1;
                }
            }
            Err(e) => {
                let text = crate::db_diagnosis::legible_db_error(&e);
                pen(db, record, RefusalCause::Door(text), &mut report).await?;
            }
        }
    }

    Ok(report)
}

/// Pen one refused record, preserving its custody, and record it in the report.
///
/// **The quota is passed as NULL (unbounded), and that is a deliberate carve-out** — db/052's
/// header carries the full argument. In short: the quota bounds a hostile peer and a restore
/// has none; the bytes it would refuse are bytes the node is about to lose permanently; and
/// its own promise ("the watermark freezes instead — delayed, never lost") needs a cursor and
/// a re-serving peer, neither of which a restore has.
///
/// **No `quarantine_floor_seq` is pinned.** The floor exists so a peer keeps re-offering a
/// refused slot; no peer re-offers a medium. A floor pinned by a restore would make the
/// restored node's first real pull re-fetch from a position no peer will ever resolve, wedging
/// federation over an event that has nothing to do with any peer. `cairn-sync requeue` is the
/// release mechanism for a restore-penned row, and it reads the pen directly.
async fn pen(
    db: &Client,
    record: &MediumRecord,
    cause: RefusalCause,
    report: &mut ClinicalRestoreReport,
) -> anyhow::Result<()> {
    let digest = cairn_event::event_address(&record.signed_bytes);
    let reason = pen_reason(&cause);
    db.execute(
        "SELECT cairn_quarantine_event($1, $2, $3, $4, $5, $6, $7, $8, NULL, NULL)",
        &[
            &digest,
            &record.signed_bytes,
            &record.attestation,
            &record.attester_key,
            &RESTORE_PEER_SENTINEL,
            &record.source_seq,
            &reason,
            &record.dek_wrapped,
        ],
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "a refused clinical record could not be penned ({}). The restore stops here \
             rather than continuing: penning is what preserves the bytes AND the key for a \
             later `cairn-sync requeue`, so a restore that could not pen would be silently \
             discarding the clinic's record one event at a time.",
            crate::db_diagnosis::legible_db_error(&e)
        )
    })?;

    *report
        .refusals
        .entry(refusal_label(&cause).into())
        .or_default() += 1;
    report.penned_bytes += record.signed_bytes.len();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two restore-side causes carry their OWN text, because since the unwrap moved into
    /// Rust neither ever reaches the door — so neither has a door text to quote. Both name
    /// the remedy, and every reason names the restore as its origin so a pen row read weeks
    /// later is not mistaken for a peer's refusal.
    #[test]
    fn every_pen_reason_names_its_origin_and_a_remedy() {
        for cause in [
            RefusalCause::NoCustodyKey,
            RefusalCause::DekWillNotOpen,
            RefusalCause::Door("apply_remote_event: overlay targets unknown event".into()),
        ] {
            let text = pen_reason(&cause);
            assert!(
                text.starts_with("restore:"),
                "a pen row must say where it came from: {text}"
            );
            assert!(
                text.contains("requeue") || text.contains("apply door"),
                "a refusal an operator cannot act on is #536's shape: {text}"
            );
        }
        assert!(
            pen_reason(&RefusalCause::Door("targets unknown event".into()))
                .contains("targets unknown event"),
            "a door refusal must carry the DOOR's text — that is the only place the actual \
             cause exists"
        );
    }

    /// Door refusals group under ONE label. Their texts carry event ids, so grouping by text
    /// would make every refusal its own group — a list, not a summary, at the moment an
    /// operator is least able to read one.
    #[test]
    fn door_refusals_collapse_to_one_summary_label() {
        assert_eq!(
            refusal_label(&RefusalCause::Door("event 1 is bad".into())),
            refusal_label(&RefusalCause::Door("event 2 is bad".into()))
        );
        assert_ne!(
            refusal_label(&RefusalCause::NoCustodyKey),
            refusal_label(&RefusalCause::DekWillNotOpen),
            "the two custody failures have DIFFERENT remedies and must not be folded together"
        );
    }

    /// `exceeds_ordinary_quota` reports, and it reports on EITHER bound.
    ///
    /// The row cap alone would miss a restore that penned a few hundred very large events;
    /// the byte cap alone would miss #512's 100 000-event scale of small ones. The quota it
    /// compares against is not enforced here — it is what a sync peer would have been allowed
    /// — so this is the sentence that keeps "unbounded" from meaning "unreported".
    #[test]
    fn the_quota_notice_fires_on_either_bound() {
        let mut r = ClinicalRestoreReport::default();
        assert!(
            !r.exceeds_ordinary_quota(),
            "an empty pen is under any bound"
        );

        r.refusals.insert("x".into(), ORDINARY_QUOTA_ROWS + 1);
        assert!(r.exceeds_ordinary_quota(), "the row bound must fire");

        let mut r = ClinicalRestoreReport::default();
        r.refusals.insert("x".into(), 1);
        r.penned_bytes = ORDINARY_QUOTA_BYTES + 1;
        assert!(
            r.exceeds_ordinary_quota(),
            "the byte bound must fire independently — a few hundred large events reach it \
             long before the row cap"
        );
    }

    /// `penned` counts across every reason, not just the first.
    #[test]
    fn penned_sums_every_refusal_group() {
        let mut r = ClinicalRestoreReport::default();
        r.refusals.insert("a".into(), 2);
        r.refusals.insert("b".into(), 3);
        assert_eq!(r.penned(), 5);
    }
}
