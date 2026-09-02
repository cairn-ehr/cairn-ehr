//! What one page of a pull contributed, and what a whole cycle adds up to.
//!
//! WHY A MODULE: `do_pull` used to make ONE request and hold its per-cycle counters as a dozen
//! local `mut` bindings. Paging turns that into a loop, and the folding rules — which counters
//! sum, which flags are sticky, which value wins when two pages disagree — become decisions
//! worth testing on their own. They are pure, so they are tested with no peer and no database.
//! (New code lives here rather than in `main.rs`, which is 11.5k lines: #531.)

use crate::{merge_pen_refusal, PenRefusal};

/// What ONE page contributed.
///
/// `Default` is what makes the fold tests readable: a test that cares about one field says so
/// with `..PageTally::default()` instead of naming thirteen it does not care about.
///
/// `#[allow(dead_code)]`: nothing constructs this outside `#[cfg(test)]` yet — `do_pull` still
/// makes a single request and holds its counters as local `mut` bindings (Task 7 of this slice
/// folds pages through it and removes this attribute).
#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct PageTally {
    /// Events the peer shipped in this page (`resp.events.len()`).
    pub(crate) shipped: usize,
    /// Events the in-DB apply door admitted as NEW.
    pub(crate) applied: usize,
    /// Entries that could not be verified and were penned (bad signature, garbage, non-hex).
    pub(crate) skipped_unverifiable: usize,
    /// Verifiable events this node's floor deliberately refused and penned (ADR-0056 d5, #267).
    pub(crate) refused_verifiable: usize,
    /// Re-offered slots a human had already acked, skipped without pinning the floor.
    pub(crate) skipped_acked: usize,
    /// Decoded signed_bytes, summed — the payload half of `bytes_per_event`.
    pub(crate) event_bytes: usize,
    /// The response frame's length on the wire.
    pub(crate) wire_bytes: usize,
    /// Highest CONTIGUOUS handled seq after this page. Seeded from the cycle's current value,
    /// so folding takes this one rather than a max.
    pub(crate) max_seq: i64,
    /// The cursor halted in this page.
    pub(crate) frozen: bool,
    /// An apply failure in this page landed on THIS NODE'S database (PR #493).
    pub(crate) local_apply_fault: bool,
    /// The pen's own refusal (quota or insert failure), if it refused.
    pub(crate) pen_refused: Option<PenRefusal>,
    /// The seq of the FIRST unacked refused event in this page.
    pub(crate) pin: Option<i64>,
    /// The peer deliberately withheld custody for this page (ADR-0052, #231).
    pub(crate) custody_withheld: bool,
    /// Content addresses of every event the door ADMITTED, for the #465 ledger read.
    pub(crate) applied_addresses: Vec<Vec<u8>>,
}

/// What the whole cycle has contributed so far. Same fields, accumulated.
///
/// `#[allow(dead_code)]`: see [`PageTally`] — `do_pull` does not fold pages into this yet
/// (Task 7 of this slice wires it in and removes this attribute).
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct CycleTally {
    pub(crate) shipped: usize,
    pub(crate) applied: usize,
    pub(crate) skipped_unverifiable: usize,
    pub(crate) refused_verifiable: usize,
    pub(crate) skipped_acked: usize,
    pub(crate) event_bytes: usize,
    pub(crate) wire_bytes: usize,
    pub(crate) max_seq: i64,
    pub(crate) frozen: bool,
    pub(crate) local_apply_fault: bool,
    pub(crate) pen_refused: Option<PenRefusal>,
    pub(crate) pin: Option<i64>,
    pub(crate) custody_withheld: bool,
    pub(crate) applied_addresses: Vec<Vec<u8>>,
    /// Pages folded so far — reported as a metric, and the thing to look at when a cycle is
    /// slow for a reason no counter explains.
    pub(crate) pages: usize,
}

// `#[allow(dead_code)]`: `new`/`fold` have no caller outside `#[cfg(test)]` until Task 6 of
// this slice folds pages through `do_pull` (see the struct-level doc above).
#[allow(dead_code)]
impl CycleTally {
    /// `max_seq` starts at the COMMITTED cursor so re-offered low-seq events (below it, kept
    /// on the wire by the floor) never rewind the checkpoint.
    pub(crate) fn new(last_seq: i64) -> Self {
        Self {
            shipped: 0,
            applied: 0,
            skipped_unverifiable: 0,
            refused_verifiable: 0,
            skipped_acked: 0,
            event_bytes: 0,
            wire_bytes: 0,
            max_seq: last_seq,
            frozen: false,
            local_apply_fault: false,
            pen_refused: None,
            pin: None,
            custody_withheld: false,
            applied_addresses: Vec::new(),
            pages: 0,
        }
    }

    /// Fold one page in. See the module doc for why each rule is what it is.
    pub(crate) fn fold(&mut self, page: PageTally) {
        self.shipped += page.shipped;
        self.applied += page.applied;
        self.skipped_unverifiable += page.skipped_unverifiable;
        self.refused_verifiable += page.refused_verifiable;
        self.skipped_acked += page.skipped_acked;
        self.event_bytes += page.event_bytes;
        self.wire_bytes += page.wire_bytes;
        // TAKE, not max: a page's `max_seq` is seeded from this value and only ever advances
        // over its own contiguous handled prefix, so it is already the running answer.
        self.max_seq = page.max_seq;
        self.frozen |= page.frozen;
        self.local_apply_fault |= page.local_apply_fault;
        self.custody_withheld |= page.custody_withheld;
        self.applied_addresses.extend(page.applied_addresses);
        if let Some(next) = page.pen_refused {
            // `merge_pen_refusal` already encodes the cross-refusal rule for a CYCLE:
            // message first-wins (text and class must describe the same event), `local_fault`
            // OR-ed (it is a fact about this node's uptime, not about one event).
            self.pen_refused = Some(merge_pen_refusal(self.pen_refused.take(), next));
        }
        self.pin = match (self.pin, page.pin) {
            // MIN, not first-wins. Pages arrive in ascending seq so the two agree today, but
            // min is order-independent, and the floor's whole job is to be conservative.
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.pages += 1;
    }
}

/// The re-offer floor for this cycle. **Pure**, and computed over the CYCLE, never a page.
///
/// The three branches are unchanged from the single-shot version; what is new is the subject.
/// Per page, a clean page 2 would clear the pin a refusing page 1 set — and the cursor has
/// already advanced past that refused event, so it would never be re-offered again. Silent
/// exclusion is precisely what this floor exists to prevent.
pub(crate) fn quarantine_floor(
    skipped_unverifiable: usize,
    refused_verifiable: usize,
    pen_failed: bool,
    pin: Option<i64>,
    floor_at_start: Option<i64>,
) -> Option<i64> {
    if skipped_unverifiable == 0 && refused_verifiable == 0 && !pen_failed {
        None
    } else if !pen_failed {
        pin
    } else {
        match (floor_at_start, pin) {
            (Some(f), Some(p)) => Some(f.min(p)),
            (Some(f), None) => Some(f),
            (None, p) => p,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_cycle_clears_the_floor() {
        assert_eq!(quarantine_floor(0, 0, false, None, Some(5)), None);
    }

    #[test]
    fn unacked_refusals_with_a_healthy_pen_pin_at_the_first_refused_slot() {
        assert_eq!(quarantine_floor(1, 0, false, Some(7), Some(5)), Some(7));
        assert_eq!(quarantine_floor(0, 1, false, Some(7), None), Some(7));
    }

    #[test]
    fn a_pen_failure_keeps_the_most_conservative_of_the_old_floor_and_the_new_pin() {
        // A re-offered slot whose pen write FAILED produced no pin, so overwriting blindly
        // would clear a floor guarding a slot the cursor is already above — permanent
        // exclusion.
        assert_eq!(quarantine_floor(1, 0, true, Some(9), Some(5)), Some(5));
        assert_eq!(quarantine_floor(1, 0, true, None, Some(5)), Some(5));
        assert_eq!(quarantine_floor(0, 0, true, Some(9), None), Some(9));
    }

    /// THE defect paging could introduce. Page 1 refuses a slot and pins the floor; page 2 is
    /// clean. Computed PER PAGE, page 2 would CLEAR the pin page 1 set — and the cursor has
    /// already advanced past that refused event, so it would never be re-offered again.
    /// Computed over the CYCLE, the refusal is still counted and the floor still stands.
    #[test]
    fn a_clean_later_page_cannot_clear_a_pin_an_earlier_page_set() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            skipped_unverifiable: 1,
            pin: Some(7),
            ..PageTally::default()
        });
        cycle.fold(PageTally::default()); // a wholly clean page 2
        assert_eq!(
            quarantine_floor(
                cycle.skipped_unverifiable,
                cycle.refused_verifiable,
                cycle.pen_refused.is_some(),
                cycle.pin,
                None
            ),
            Some(7)
        );
    }

    #[test]
    fn the_earliest_pin_wins_whichever_page_carried_it() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(9),
            ..PageTally::default()
        });
        cycle.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(4),
            ..PageTally::default()
        });
        assert_eq!(
            cycle.pin,
            Some(4),
            "min, not first-wins: order-independent by construction"
        );
    }

    #[test]
    fn folding_sums_the_counters_and_makes_the_flags_sticky() {
        let mut cycle = CycleTally::new(10);
        cycle.fold(PageTally {
            applied: 3,
            shipped: 5,
            event_bytes: 100,
            max_seq: 14,
            custody_withheld: true,
            ..PageTally::default()
        });
        cycle.fold(PageTally {
            applied: 2,
            shipped: 5,
            event_bytes: 90,
            max_seq: 19,
            frozen: true,
            ..PageTally::default()
        });
        assert_eq!(
            (cycle.applied, cycle.shipped, cycle.event_bytes),
            (5, 10, 190)
        );
        assert_eq!(cycle.max_seq, 19);
        assert!(cycle.frozen && cycle.custody_withheld);
    }
}
