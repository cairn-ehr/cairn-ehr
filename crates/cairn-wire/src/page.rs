//! The paging contract: how big a page is, and how a puller knows it has reached the end.

/// Events per page by default (slice 2b, #101 item 1).
///
/// At roughly 4 KiB per event on the wire (≈1.5 KiB signed, hex-doubled, plus attestation and
/// wrapped DEK) that is about 2 MiB per page: 32× under [`crate::MAX_FRAME_BYTES`], and
/// comfortably inside the 30 s read timeout on the 700 ms-RTT double-Starlink link Spike 0001
/// measures against. A 20k-event sweep becomes 40 round trips, about 30 s of accumulated
/// latency — paid once, on a full sweep, in exchange for progress that survives an interruption.
pub const DEFAULT_PAGE_EVENTS: u32 = 500;

#[derive(Debug, PartialEq, Eq)]
pub enum PageDecision {
    /// Stop. Either the peer drained its log, or this cycle froze.
    Done,
    /// Ask for the next page from the advanced cursor.
    Continue,
    /// The peer answered with something no puller can act on. The string is the operator's
    /// diagnosis.
    Refuse(String),
}

/// Decide what to do after one page. **Pure**, so all four states are tested with no peer,
/// no socket and no database.
pub fn page_decision(complete: bool, page_len: usize, frozen: bool) -> PageDecision {
    // FROZEN FIRST, and the order is load-bearing. A freeze is this node's decision, not a
    // peer fault; letting the empty-page refusal below claim it would send an operator to
    // audit a healthy peer. It also cannot make progress: the checkpoint will not advance
    // past the freeze, so a next page would re-fetch what we have already declined to handle.
    if frozen {
        return PageDecision::Done;
    }
    if complete {
        return PageDecision::Done;
    }
    if page_len == 0 {
        return PageDecision::Refuse(
            "the peer returned an EMPTY page without declaring the stream complete. That is \
             neither an end nor a continuation: treating it as the end would checkpoint the \
             cursor as though the log were drained and silently strand every event above it, \
             and continuing would re-request the same cursor forever. The peer answered and \
             its wire format is the problem — check that it sets `complete` on every \
             response. If earlier pages of this same cycle DID deliver events with \
             `complete` unset, the peer most likely predates paging (slice 2b, #101 item 1) \
             and never sets the field at all — upgrade the peer binary."
                .to_string(),
        );
    }
    PageDecision::Continue
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_page_ends_the_loop() {
        assert_eq!(page_decision(true, 500, false), PageDecision::Done);
        assert_eq!(page_decision(true, 0, false), PageDecision::Done);
    }

    #[test]
    fn an_incomplete_non_empty_page_continues() {
        assert_eq!(page_decision(false, 500, false), PageDecision::Continue);
        assert_eq!(page_decision(false, 1, false), PageDecision::Continue);
    }

    #[test]
    fn an_empty_page_that_does_not_declare_completeness_is_refused() {
        // Neither an end nor a continuation. Treating it as the end risks a silent early
        // stop with the cursor checkpointed as if the log were drained; continuing spins
        // forever against the same cursor. The peer ANSWERED and the answer is unusable,
        // which is an integrity condition.
        match page_decision(false, 0, false) {
            PageDecision::Refuse(why) => {
                assert!(why.contains("complete"), "{why}");
                // …and it must name the CAUSE, not only the field. This is the line an
                // operator sees every cycle, forever, on a link that is otherwise healthy:
                // a pre-paging peer never sets `complete`, so its response decodes as
                // "there may be more", the puller asks once more, and gets exactly this
                // empty page. Naming the version boundary and the remedy is the house
                // standard `do_pull`'s pre-#196 transport message already sets. The
                // sentence is conditional in its own prose, so no state is needed to
                // decide whether to say it.
                assert!(
                    why.contains("predates paging") && why.contains("upgrade the peer"),
                    "the refusal must name the likeliest cause and its remedy: {why}"
                );
            }
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn a_frozen_cursor_ends_the_loop_whatever_the_page_said() {
        // Fetching another page after a freeze would pull events the puller has already
        // decided it will not handle, and the checkpoint cannot advance past the freeze.
        assert_eq!(page_decision(false, 500, true), PageDecision::Done);
        assert_eq!(page_decision(false, 0, true), PageDecision::Done);
    }
}
