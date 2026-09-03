//! What one page of a pull contributed, and what a whole cycle adds up to.
//!
//! WHY A MODULE: `do_pull` used to make ONE request and hold its per-cycle counters as a dozen
//! local `mut` bindings. Paging turns that into a loop, and the folding rules — which counters
//! sum, which flags are sticky, which value wins when two pages disagree — become decisions
//! worth testing on their own. They are pure, so they are tested with no peer and no database.
//! (New code lives here rather than in `main.rs`, whose size is #531's whole subject. No
//! line count: the figure this line used to carry was already wrong by the end of the slice
//! that wrote it, and the issue number says the same thing without going stale.)

use crate::{merge_pen_refusal, PenRefusal};

/// Events one cycle will pull before YIELDING to the next (final review, Critical 2).
///
/// # Why a budget exists at all, when the loop's own comment argued none was needed
///
/// `do_pull`'s loop used to carry an "emergent bound" argument: a garbage flood is penned,
/// the per-peer quarantine quota is finite, the pen eventually REFUSES, which freezes the
/// cursor, which ends the cycle. That argument is **wrong**, and it is wrong for the two
/// cheapest streams a peer can serve:
///
/// * events this node ALREADY HOLDS — `apply_signed` returns `Ok(false)`, nothing is penned,
///   no quota moves, nothing freezes, and `max_seq` still advances because "handled" includes
///   an idempotent no-op;
/// * bytes ALREADY PENNED — `quarantine_event` dedupes *before* the quota-checked INSERT and
///   returns, so the pen never grows and never refuses.
///
/// A peer re-serving a handful of genuine events at strictly ascending FABRICATED seqs, never
/// setting `complete`, therefore satisfies `validate_page` and the anti-loop invariant on
/// every page for ever. `do_pull` never returns, and `cmd_run`'s cycle loop is blocked with
/// it: no further pulls, no fingerprint, **no periodic full sweep** — and the sweep is the
/// consolation `validate_page`'s own comment offers against a peer lying high about its seqs.
///
/// # Why a YIELD and not a refusal
///
/// Exceeding the budget is not an accusation. A legitimate first sweep of a very large log
/// genuinely needs many pages, and every page has already committed its cursor and floor, so
/// stopping costs nothing but the round trips already spent: the next cycle resumes exactly
/// where this one stopped. Refusing would turn an honest catch-up into a failing link.
///
/// It is emphatically **not a silent `break`**, which is the outcome `page_decision`'s own
/// refusal text warns about: the cycle prints an operator line and publishes
/// `budget_exhausted` in its metrics, and it never claims `complete`, so the cursor is
/// checkpointed at real progress rather than as though the log were drained.
///
/// # The value
///
/// One million events. Before paging, ONE cycle was implicitly capped by the 64 MiB frame at
/// roughly twenty thousand events — the whole log had to fit in a single response — so this
/// budget is fifty times more generous than the bound paging removed, and no deployment this
/// project has (or plans) reaches it in one cycle.
pub(crate) const MAX_EVENTS_PER_CYCLE: usize = 1_000_000;

/// How many pages [`MAX_EVENTS_PER_CYCLE`] buys at `page_limit`. **Pure**, so the rule is
/// tested with no peer and no database.
///
/// Never zero, and never divides by zero: `main` already refuses `--page 0` (see
/// `parse_page_limit`), but a cycle that fetched no pages at all would checkpoint nothing and
/// spin on the same cursor for ever — the exact shape the budget exists to prevent — so the
/// floor of one is enforced here too rather than assumed from a caller.
pub(crate) fn page_budget(page_limit: u32) -> usize {
    (MAX_EVENTS_PER_CYCLE / (page_limit.max(1) as usize)).max(1)
}

/// What ONE page contributed.
///
/// # No `Default`, and that is deliberate (final review)
///
/// `Default` would give `max_seq: 0`, which VIOLATES this struct's own documented rule: the
/// field is seeded from the cycle's running value, and `fold` TAKES it rather than maxing it,
/// so folding a defaulted page would rewind the cycle cursor to zero. Production never did
/// (`apply_page` always seeds it, and `commit_cursor`'s `GREATEST` guards the database
/// besides) but `metrics["cursor_seq"]` would have published a false zero, and the illegal
/// state was not merely representable — it was the default.
///
/// `PageTally::seeded` (test-only, so NOT an intra-doc link — `cargo doc` does not compile
/// `#[cfg(test)]` items and a link to one fails the doc build under `-D warnings`) keeps what
/// `Default` was FOR: a test that cares about one field says so with `..PageTally::seeded(0)`
/// instead of naming thirteen it does not, while making the one field that must not be
/// guessed impossible to skip.
#[derive(Debug)]
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

impl PageTally {
    /// An empty page whose running cursor starts at `max_seq_in` — the cycle's current value,
    /// which is the committed cursor for page 1 and the previous page's answer thereafter.
    /// Every other field starts at "this page did nothing", which is true of a page that has
    /// not been applied yet.
    ///
    /// TEST-ONLY, deliberately. Production builds the full struct literal in `apply_page`,
    /// where the compiler already forces every field including `max_seq`; the hazard removing
    /// `Default` closed lived entirely in the TESTS, where `..PageTally::default()` silently
    /// supplied `max_seq: 0` and would have rewound a folded cycle's cursor. Gating it here
    /// keeps that fix without adding an unused production constructor.
    #[cfg(test)]
    pub(crate) fn seeded(max_seq_in: i64) -> Self {
        Self {
            shipped: 0,
            applied: 0,
            skipped_unverifiable: 0,
            refused_verifiable: 0,
            skipped_acked: 0,
            event_bytes: 0,
            wire_bytes: 0,
            max_seq: max_seq_in,
            frozen: false,
            local_apply_fault: false,
            pen_refused: None,
            pin: None,
            custody_withheld: false,
            applied_addresses: Vec::new(),
        }
    }
}

/// What the whole cycle has contributed so far. Same fields, accumulated.
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
        // DESTRUCTURED EXHAUSTIVELY, with no `..`, and that is the point (final review).
        // Reading `page.x` field by field compiled untouched when a field was added to both
        // structs — `apply_page`'s literal and `CycleTally::new` both force an update, but
        // this function, the only place a field's FOLD RULE can be written, did not. A
        // forgotten counter reports a permanent zero; a forgotten sticky safety flag (a
        // sibling of `frozen`, `pin` or `local_apply_fault`) reports a cycle as clean when it
        // was not. `error[E0027]: pattern does not mention field` is the guard, and it costs
        // one line. Do not "tidy" a `..` back in.
        let PageTally {
            shipped,
            applied,
            skipped_unverifiable,
            refused_verifiable,
            skipped_acked,
            event_bytes,
            wire_bytes,
            max_seq,
            frozen,
            local_apply_fault,
            pen_refused,
            pin,
            custody_withheld,
            applied_addresses,
        } = page;
        self.shipped += shipped;
        self.applied += applied;
        self.skipped_unverifiable += skipped_unverifiable;
        self.refused_verifiable += refused_verifiable;
        self.skipped_acked += skipped_acked;
        self.event_bytes += event_bytes;
        self.wire_bytes += wire_bytes;
        // TAKE, not max: a page's `max_seq` is seeded from this value and only ever advances
        // over its own contiguous handled prefix, so it is already the running answer.
        self.max_seq = max_seq;
        self.frozen |= frozen;
        self.local_apply_fault |= local_apply_fault;
        self.custody_withheld |= custody_withheld;
        self.applied_addresses.extend(applied_addresses);
        if let Some(next) = pen_refused {
            // `merge_pen_refusal` already encodes the cross-refusal rule for a CYCLE:
            // message first-wins (text and class must describe the same event), `local_fault`
            // OR-ed (it is a fact about this node's uptime, not about one event).
            self.pen_refused = Some(merge_pen_refusal(self.pen_refused.take(), next));
        }
        self.pin = match (self.pin, pin) {
            // MIN, not first-wins. Pages arrive in ascending seq so the two agree today, but
            // min is order-independent, and the floor's whole job is to be conservative.
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.pages += 1;
    }
}

impl CycleTally {
    /// The re-offer floor for this cycle. **Pure**, and computed over the CYCLE, never a page.
    ///
    /// The three branches are unchanged from the single-shot version; what is new is the
    /// subject. Per page, a clean page 2 would clear the pin a refusing page 1 set — and the
    /// cursor has already advanced past that refused event, so it would never be re-offered
    /// again. Silent exclusion is precisely what this floor exists to prevent.
    ///
    /// # Why this is a METHOD and not a free function taking five scalars
    ///
    /// It used to be one, and `PageTally` carries the same four fields at the same types — so
    /// `quarantine_floor(page.skipped_unverifiable, page.refused_verifiable,
    /// page.pen_refused.is_some(), page.pin, floor_seq)` compiled, and it is verbatim "THE
    /// defect paging could introduce" from this function's own test name. The doc said "over
    /// the CYCLE, never a page" in three files; the signature said nothing. It also put `pin`
    /// and `floor_at_start` — two adjacent `Option<i64>` — next to each other, where a
    /// transposition compiles and silently returns the old floor instead of the new pin.
    ///
    /// Taking `&self` makes both a compile error. The RULE stays pure and separately testable
    /// in [`quarantine_floor_rule`] (final review).
    pub(crate) fn quarantine_floor(&self, floor_at_start: Option<i64>) -> Option<i64> {
        quarantine_floor_rule(
            self.skipped_unverifiable,
            self.refused_verifiable,
            self.pen_refused.is_some(),
            self.pin,
            floor_at_start,
        )
    }
}

/// The floor rule itself, over bare values. **Pure**, private, and unit-tested directly so the
/// three branches can be pinned without building a cycle for each — but not reachable from
/// outside this module, so nothing can hand it a page's counters. See
/// [`CycleTally::quarantine_floor`].
fn quarantine_floor_rule(
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

/// What may actually be COMMITTED as the floor after one page. **Pure.**
///
/// THE OTHER HALF OF THE FLOOR RULE, and the one the first draft of paging missed.
/// [`CycleTally::quarantine_floor`] makes the PIN cumulative, so a clean page 2 cannot clear a
/// pin page 1
/// set. This is its mirror image: the clean branch returns `None`, and `None` is not silence,
/// it is a positive claim — *nothing is being withheld any more* — and paging commits it after
/// EVERY page, including page 1 of a cycle that has not yet reached the slot the floor guards.
///
/// On an INCREMENTAL cycle the claim USUALLY holds without this guard doing any work, because
/// `do_pull` fetches from `floor_seq.saturating_sub(1).min(last_seq)`: ordinarily
/// `floor_seq <= last_seq` (the floor was pinned at a slot the cursor has since advanced past),
/// so the fetch point is `floor_seq - 1` and the guarded slot is the first row of page 1. But
/// that expression is a MIN, not an unconditional `floor_seq - 1` — when `last_seq` is the
/// smaller of the two, the fetch starts there instead, and the guarded slot can sit several
/// pages in, exactly as on a full sweep below. This function does not special-case either
/// cycle kind; it is what makes BOTH safe, by asking whether `floor_at_start` is at or below
/// what THIS page actually reached, never by assuming the fetch point put it in page 1. On a
/// FULL SWEEP the gap is the common case. A sweep fetches from seq 0 and ignores the floor
/// entirely, so a floor at seq 900 sits several pages in — and the first cycle after every
/// daemon start is a full sweep. The failure needs no hostile peer:
///
/// * `sync_state`: `last_seq = 1000`, `quarantine_floor_seq = 900` (an unverifiable event
///   penned at seq 900, kept on the wire so a repaired version is admitted automatically);
/// * a full sweep at the default page size. Page 1 is seqs 1..500, every one an idempotent
///   no-op — a clean page — so the floor is computed `None` and written NULL;
/// * the cycle ends before page 2. A dropped link on a 700 ms double-Starlink hop, or, with
///   nothing wrong anywhere, ONE transient apply failure in page 1: that freezes, and
///   `page_decision` ends the loop;
/// * the next cycle is incremental, reads a NULL floor, and fetches from `last_seq = 1000`.
///   **Seq 900 is never re-offered again.** `skipped_unverifiable` is 0, so the cycle is not
///   even loud — the pull goes quiet while the pen row stands.
///
/// This is a REGRESSION the loop introduced: before paging the floor was written once, after
/// the whole suffix had been offered, so the clean branch could only fire on a cycle that had
/// actually seen the slot. So the rule is that **the tally may only speak for seqs the cycle
/// has actually been offered.** A clear is withheld unless one of two things licenses it:
///
/// * `complete` — the peer says nothing exists above this page, so a clean cycle really has
///   seen everything the floor could guard. This is the pre-paging behaviour, unchanged; or
/// * the cycle has already been offered past the guarded slot (`floor_at_start <= reached`),
///   which is the same evidence a single-shot cycle had.
///
/// A PIN needs no such guard. It comes from a refusal in a page this cycle handled, so it is
/// at or below `reached` by construction, and where it is *below* an existing floor it only
/// widens what gets re-offered next cycle — conservative in the safe direction.
///
/// `reached` is the highest seq this cycle has been offered so far — the last seq of this
/// page, or the cursor the page was fetched from when it carried no seqs.
pub(crate) fn committable_floor(
    computed: Option<i64>,
    complete: bool,
    reached: i64,
    floor_at_start: Option<i64>,
) -> Option<i64> {
    match computed {
        Some(pin) => Some(pin),
        None if complete => None,
        None => floor_at_start.filter(|guarded| *guarded > reached),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_wire::DEFAULT_PAGE_EVENTS;

    /// The budget is an EVENT count, so the page count it buys has to move inversely with the
    /// page size — a bigger page must not buy a bigger cycle.
    #[test]
    fn the_page_budget_is_an_event_budget_divided_by_the_page_size() {
        assert_eq!(page_budget(1), MAX_EVENTS_PER_CYCLE);
        assert_eq!(page_budget(1000), MAX_EVENTS_PER_CYCLE / 1000);
        assert_eq!(
            page_budget(DEFAULT_PAGE_EVENTS),
            MAX_EVENTS_PER_CYCLE / DEFAULT_PAGE_EVENTS as usize,
            "the default page size must buy the whole event budget, not a rounded-down slice"
        );
    }

    /// Never zero, from either direction. A cycle that fetched no pages would checkpoint
    /// nothing and re-ask the same cursor for ever — the very shape the budget prevents — and
    /// a `page_limit` of 0 must not divide by zero even though `parse_page_limit` refuses it
    /// one layer up.
    #[test]
    fn the_page_budget_is_never_zero_and_never_divides_by_zero() {
        assert_eq!(page_budget(0), MAX_EVENTS_PER_CYCLE, "0 is clamped to 1");
        assert_eq!(
            page_budget(u32::MAX),
            1,
            "a page larger than the budget still gets one"
        );
        assert_eq!(
            page_budget(MAX_EVENTS_PER_CYCLE as u32),
            1,
            "the exact boundary is one page, not zero"
        );
    }

    /// A standing bounds guard on the constant itself (same class as
    /// `frame_cap_holds_a_realistic_event_batch`): the budget must stay far above the ~20k
    /// events ONE cycle could carry before paging, or it would be a regression rather than a
    /// backstop — and finite, or it would not bound the livelock at all.
    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn the_cycle_budget_is_far_above_the_bound_paging_removed() {
        assert!(
            MAX_EVENTS_PER_CYCLE >= 500_000,
            "must dwarf the ~20k events one 64 MiB frame held before paging"
        );
        assert!(
            MAX_EVENTS_PER_CYCLE <= 50_000_000,
            "must still bound a peer serving fabricated ascending seqs for ever"
        );
    }

    #[test]
    fn a_clean_cycle_clears_the_floor() {
        assert_eq!(quarantine_floor_rule(0, 0, false, None, Some(5)), None);
    }

    /// Fix round 1, finding 6: the test above passes `pin: None`, so flipping the first
    /// branch's `!pen_failed` to `pen_failed` would fall through to the `pin` branch, which
    /// ALSO yields `None` there — the mutant survives. A stale `pin: Some(9)` (left over from
    /// a PRIOR cycle's refusal, now clean) makes the two branches disagree: the clean-cycle
    /// branch must still clear it even though a pin value is sitting right there.
    #[test]
    fn a_clean_cycle_clears_the_floor_even_over_a_stale_pin() {
        assert_eq!(quarantine_floor_rule(0, 0, false, Some(9), Some(5)), None);
    }

    #[test]
    fn unacked_refusals_with_a_healthy_pen_pin_at_the_first_refused_slot() {
        assert_eq!(
            quarantine_floor_rule(1, 0, false, Some(7), Some(5)),
            Some(7)
        );
        assert_eq!(quarantine_floor_rule(0, 1, false, Some(7), None), Some(7));
    }

    #[test]
    fn a_pen_failure_keeps_the_most_conservative_of_the_old_floor_and_the_new_pin() {
        // A re-offered slot whose pen write FAILED produced no pin, so overwriting blindly
        // would clear a floor guarding a slot the cursor is already above — permanent
        // exclusion.
        assert_eq!(quarantine_floor_rule(1, 0, true, Some(9), Some(5)), Some(5));
        assert_eq!(quarantine_floor_rule(1, 0, true, None, Some(5)), Some(5));
        assert_eq!(quarantine_floor_rule(0, 0, true, Some(9), None), Some(9));
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
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally::seeded(0)); // a wholly clean page 2
        assert_eq!(
            cycle.quarantine_floor(None),
            Some(7),
            "over the CYCLE, so page 1's refusal still counts and the floor still stands"
        );
    }

    #[test]
    fn the_earliest_pin_wins_whichever_page_carried_it() {
        // Descending (9 then 4): min and "last wins" happen to agree — both give 4 — so this
        // pair alone does not rule out last-wins. ("First wins" would wrongly give 9, so this
        // pair DOES rule out first-wins on its own; the ascending case below is what closes
        // the last-wins gap.)
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(9),
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(4),
            ..PageTally::seeded(0)
        });
        assert_eq!(
            cycle.pin,
            Some(4),
            "min, not first-wins: order-independent by construction"
        );

        // Fix round 1, finding 5: ascending (4 then 9) is the pair's other half. Here min and
        // "first wins" also agree (both give 4), but "last wins" would wrongly give 9. Only
        // the TWO cases TOGETHER pin down MIN specifically against both first-wins and
        // last-wins mutants.
        let mut ascending = CycleTally::new(0);
        ascending.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(4),
            ..PageTally::seeded(0)
        });
        ascending.fold(PageTally {
            refused_verifiable: 1,
            pin: Some(9),
            ..PageTally::seeded(0)
        });
        assert_eq!(
            ascending.pin,
            Some(4),
            "min, not last-wins: order-independent by construction"
        );
    }

    #[test]
    fn folding_sums_the_counters_and_makes_the_flags_sticky() {
        let mut cycle = CycleTally::new(10);
        cycle.fold(PageTally {
            applied: 3,
            shipped: 5,
            event_bytes: 100,
            // wire_bytes/skipped_acked given DIFFERENT values on each page (fix round 1,
            // finding 7): a `+=` -> `=` regression on either would make the total equal the
            // LAST page's value alone (30 / 2) rather than the sum (70 / 3), so distinct
            // per-page values are what makes the mutant visible.
            wire_bytes: 40,
            skipped_acked: 1,
            max_seq: 14,
            custody_withheld: true,
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally {
            applied: 2,
            shipped: 5,
            event_bytes: 90,
            wire_bytes: 30,
            skipped_acked: 2,
            max_seq: 19,
            frozen: true,
            ..PageTally::seeded(0)
        });
        assert_eq!(
            (
                cycle.applied,
                cycle.shipped,
                cycle.event_bytes,
                cycle.wire_bytes,
                cycle.skipped_acked,
            ),
            (5, 10, 190, 70, 3)
        );
        assert_eq!(cycle.max_seq, 19);
        assert!(cycle.frozen && cycle.custody_withheld);
    }

    /// Fix round 1, finding 4: the test above folds `max_seq` ASCENDING (14 then 19), which
    /// cannot distinguish TAKE (`self.max_seq = page.max_seq`) from MAX
    /// (`self.max_seq = self.max_seq.max(page.max_seq)`) — both give 19. Folding a page with a
    /// LOWER `max_seq` than the one before it is the only way to tell them apart: TAKE gives
    /// the lower, later value; MAX would wrongly keep the higher, earlier one. Do not "tidy"
    /// this back to an ascending pair — that silently deletes the coverage.
    #[test]
    fn max_seq_takes_the_latest_page_even_when_it_is_lower() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            max_seq: 19,
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally {
            max_seq: 14,
            ..PageTally::seeded(0)
        });
        assert_eq!(
            cycle.max_seq, 14,
            "TAKE, not MAX: a page's max_seq is already the running answer over its own \
             contiguous handled prefix, seeded from the value before it"
        );
    }

    /// Fix round 1, finding 2: `fold`'s `pen_refused` arm is the only part of the function that
    /// is not a plain sum/OR/take — it delegates to `merge_pen_refusal` across the module
    /// boundary, and nothing exercised that delegation. `expected` is computed by calling the
    /// REAL `merge_pen_refusal` directly, so this test fails if `fold` ever stops calling it
    /// (a naive overwrite, or a naive first-wins-only merge, both diverge from `expected`).
    /// The two pages deliberately DISAGREE on `local_fault` (false, then true) — an OR that
    /// isn't exercised by a disagreeing pair proves nothing.
    #[test]
    fn fold_delegates_the_pen_refused_merge_to_merge_pen_refusal() {
        let first = PenRefusal {
            message: "quota exceeded".to_string(),
            local_fault: false,
        };
        let second = PenRefusal {
            message: "disk full".to_string(),
            local_fault: true,
        };
        let expected = merge_pen_refusal(Some(first.clone()), second.clone());

        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            pen_refused: Some(first),
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally {
            pen_refused: Some(second),
            ..PageTally::seeded(0)
        });

        let merged = cycle
            .pen_refused
            .expect("a pen refusal folded in must survive the fold");
        assert_eq!(merged.message, expected.message);
        assert_eq!(merged.local_fault, expected.local_fault);
        assert!(
            merged.local_fault,
            "local_fault is OR-ed across pages (a fact about this node's uptime), not \
             first-wins — the first page alone was healthy, but the cycle as a whole was not"
        );
    }

    // -----------------------------------------------------------------------------------
    // `committable_floor` — the mid-cycle clear guard (final review, Critical 1).
    // -----------------------------------------------------------------------------------

    /// THE REGRESSION. A clean page 1 of a FULL SWEEP must not clear a floor the cycle has
    /// not yet reached: the numbers here are the scenario from the function's own doc, with
    /// the floor at 900 and page 1 ending at 500.
    #[test]
    fn a_clean_page_cannot_clear_a_floor_the_cycle_has_not_reached_yet() {
        assert_eq!(
            committable_floor(None, false, 500, Some(900)),
            Some(900),
            "the cycle has been offered 1..500 and says nothing about seq 900 — clearing \
             would claim it, and one freeze or dropped link later that slot is unreachable"
        );
    }

    /// …but a clean cycle that HAS reached past the guarded slot clears it, exactly as
    /// before. Without this the floor would only ever clear on a `complete` page, and a
    /// resolved refusal would keep re-shipping from a low seq for no reason.
    #[test]
    fn a_clean_page_that_has_passed_the_guarded_slot_still_clears_it() {
        assert_eq!(committable_floor(None, false, 500, Some(300)), None);
        // THE BOUNDARY, and it is `>` not `>=` for a reason: seq 500 was IN this page (the
        // page's last seq IS `reached`), so it has been offered and found clean.
        assert_eq!(committable_floor(None, false, 500, Some(500)), None);
        // One above the boundary is the first slot the page did not reach.
        assert_eq!(committable_floor(None, false, 500, Some(501)), Some(501));
    }

    /// `complete` is the peer's statement that nothing exists above this page, so a clean
    /// cycle really has seen everything the floor could guard. This is the pre-paging rule,
    /// and it must survive: a floor set on a seq the peer no longer serves would otherwise
    /// never clear.
    #[test]
    fn a_complete_page_clears_the_floor_even_above_what_it_reached() {
        assert_eq!(committable_floor(None, true, 500, Some(900)), None);
        // An empty complete page (`reached` falls back to the cursor) is the same statement.
        assert_eq!(committable_floor(None, true, 0, Some(900)), None);
    }

    /// A PIN is committed unguarded, and both directions matter. It is at or below `reached`
    /// by construction (it came from a page this cycle handled), and where it sits BELOW an
    /// existing floor it lowers it — which only widens next cycle's re-offer window. The
    /// dangerous direction is clearing, never pinning.
    #[test]
    fn a_pin_is_committed_whatever_the_cycle_reached() {
        assert_eq!(committable_floor(Some(7), false, 500, Some(900)), Some(7));
        assert_eq!(committable_floor(Some(7), true, 500, None), Some(7));
    }

    /// No floor to begin with and nothing refused: there is nothing to withhold, on any page.
    /// (Without the `filter`'s `Option` handling this would be the arm that panicked.)
    #[test]
    fn a_clean_page_with_no_floor_at_all_commits_none() {
        assert_eq!(committable_floor(None, false, 500, None), None);
        assert_eq!(committable_floor(None, true, 500, None), None);
    }

    /// Fix round 1, finding 3: nothing populated `applied_addresses` before, so deleting the
    /// `.extend(...)` call outright still passed every test. Two pages, two different address
    /// sets, asserted concatenated IN ORDER.
    #[test]
    fn fold_extends_applied_addresses_in_order() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally {
            applied_addresses: vec![vec![1, 2, 3]],
            ..PageTally::seeded(0)
        });
        cycle.fold(PageTally {
            applied_addresses: vec![vec![4, 5], vec![6]],
            ..PageTally::seeded(0)
        });
        assert_eq!(
            cycle.applied_addresses,
            vec![vec![1, 2, 3], vec![4, 5], vec![6]]
        );
    }
}
