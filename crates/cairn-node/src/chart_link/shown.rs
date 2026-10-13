//! #708 — a human judgement made from a COMPARISON is signed only over the records the clinician
//! was shown: [`RecordsShown`], the re-check `assert_link_in_tx` runs under the link lock
//! (CARNLK) before anything is signed, and the pure comparison it rests on ([`stale_record`]).
//!
//! Lives apart from `chart_link.rs` (house rule 4: files under 500 lines); re-exported by the
//! parent, so `chart_link::RecordsShown` keeps its path.

use crate::db_diagnosis::{deliberate_refusal, LocalDbFault};
use anyhow::Context;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// The two records a human judgement was made over, as the clinician was SHOWN them (#708):
/// `this` is the record on screen, `other` the record compared with it. Each subject of the
/// judgement must be in one of them.
#[derive(Debug, Clone, Copy)]
pub struct RecordsShown<'a> {
    pub this: &'a ChartSet,
    pub other: &'a ChartSet,
}

/// The one fact a judgement made from a COMPARISON rests on — each subject's record still reads
/// as the clinician was shown it — checked where it cannot go stale before the commit (#708).
///
/// The window checks both displayed sets against fresh reads before it calls in, but a peer's
/// link or unlink applied through the sync door between that check and this submit would then be
/// signed over. Two shapes, both silent before this check:
///
/// - **"Same person" over {A} and {B}** while a peer links A–X: the event joins {A, X} with {B},
///   a record nobody compared.
/// - **"Different people" over {A} and {B}** while a colleague's ATTESTED link of A–B lands: our
///   unlink is stamped above it (db/020 merges the peer's clock) and overrules a human judgement
///   the clinician never saw, reported as a plain `TookEffect`.
///
/// So, exactly as `refuse_unless_record_holds_both` in the parent: CARNLK FIRST (db/018's global link lock,
/// held by every link/unlink apply — the only writer of `person_member` — until it commits), then
/// the records. The caller has already locked the proposal row, so the order stays
/// row-then-CARNLK, and `submit_event`'s own acquisition later is a no-op re-acquire.
pub(super) async fn refuse_unless_records_read_as_shown(
    tx: &tokio_postgres::Transaction<'_>,
    shown: RecordsShown<'_>,
    low: Uuid,
    high: Uuid,
) -> anyhow::Result<()> {
    tx.execute("SELECT pg_advisory_xact_lock(x'4341524E4C4B'::bigint)", &[])
        .await
        .map_err(|e| {
            LocalDbFault::new(
                "taking the link lock (CARNLK) before re-reading the compared records",
                e,
            )
        })?;
    for subject in [low, high] {
        let now = crate::patient::person::person_charts(tx, subject)
            .await
            .context("re-reading a compared record under the link lock (CARNLK)")?;
        if let Some(stale) = stale_record(shown, subject, &now) {
            // Stale INPUT, as in `refuse_unless_record_holds_both`: the same call refuses the
            // same way until the caller re-reads, so it is never worded as an outage to retry.
            return Err(deliberate_refusal(stale));
        }
    }
    Ok(())
}

/// Why `subject`'s record (`now`, read under CARNLK) no longer matches what the clinician was
/// shown, or `None` if it does. **Pure**, so the comparison is unit-tested apart from the
/// database. A subject in NEITHER shown record is a caller bug — the judgement was not made over
/// it — and is refused rather than waved through.
pub fn stale_record(shown: RecordsShown<'_>, subject: Uuid, now: &ChartSet) -> Option<String> {
    let was = [shown.this, shown.other]
        .into_iter()
        .find(|record| record.contains(&subject));
    match was {
        Some(was) if was == now => None,
        Some(was) => Some(format!(
            "the records changed while you were comparing them: chart {subject}'s record was \
             {} chart(s) when you compared it and is {} now (another link or unlink was \
             recorded meanwhile) — nothing was done; compare again",
            was.members().len(),
            now.members().len()
        )),
        None => Some(format!(
            "chart {subject} is in neither of the records you compared — nothing was done; \
             compare again"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compared_record_is_stale_exactly_when_it_reads_differently_now() {
        let (a, b, x) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
        let (this, other) = (ChartSet::single(a), ChartSet::single(b));
        let shown = RecordsShown {
            this: &this,
            other: &other,
        };
        // As shown, from either record.
        assert_eq!(stale_record(shown, a, &ChartSet::single(a)), None);
        assert_eq!(stale_record(shown, b, &ChartSet::single(b)), None);
        // Grown by a peer link, or joined to the other record by a colleague's link.
        let grown = ChartSet::new([a, x]).unwrap();
        assert!(stale_record(shown, a, &grown)
            .unwrap()
            .contains("compare again"));
        let joined = ChartSet::new([a, b]).unwrap();
        assert!(stale_record(shown, b, &joined).is_some());
        // Shrunk: a member was unlinked away.
        let wide = ChartSet::new([a, x]).unwrap();
        let shown_wide = RecordsShown {
            this: &wide,
            other: &other,
        };
        assert!(stale_record(shown_wide, a, &ChartSet::single(a)).is_some());
        // A subject the judgement was not made over.
        assert!(stale_record(shown, x, &ChartSet::single(x))
            .unwrap()
            .contains("neither"));
    }
}
