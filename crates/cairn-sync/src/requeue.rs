//! The pure half of `cairn-sync requeue` — outcome accounting and the operator's words.
//!
//! **Why this module exists at all.** `do_requeue` used to make every decision inline, and one of
//! them was wrong in a way no test could see: it deleted a pen row whenever the apply door returned
//! `Ok`, without ever asking whether the custody it was carrying had actually landed. On a restored
//! solo node that pen row is the last copy of the event's DEK, so the command the pen's own remedy
//! text points an operator at destroyed the key it was holding, printed nothing a monitor could
//! read, and exited 0 (issue #578).
//!
//! Fixing that meant giving the loop a fourth and fifth outcome, three new operator messages and a
//! rule about which. All of it is decidable from values alone, so all of it lives here as pure
//! functions with unit tests that need no database — which matters, because the behaviour they
//! serve is otherwise reachable only through a DB-gated suite that drives the shipped binary.
//!
//! **THE ONE RULE, stated once, here.**
//!
//! > A pen row that carries a wrapped DEK is released only when custody actually landed.
//!
//! It is deliberately uniform across all three ways custody can fail to land ([`CustodyGap`]),
//! because the three are not distinguishable *as recoverability* at the moment of the decision.
//! Most tempting to carve out is [`WrappedDekFault::DidNotOpen`] — "this key is not ours, so its
//! row is worthless". That claim cannot be made from here: *did not open with the key we have right
//! now* is not *not ours*. The operator may be holding the right `<key>.unwrap` on a USB stick they
//! have not plugged in, which is the #495 shape this whole disaster-recovery path exists to
//! survive. Deleting the row bets an unrecoverable key on an inference the code cannot support.
//!
//! **How a row that truly is unopenable ever leaves the pen**, since the rule alone would hold it
//! forever: `db/021`'s `acked` flag, which it describes as *"a recorded human decision, never an
//! automatic one"*. `do_requeue` honours it (issue #581) exactly as `do_pull` always has. A human
//! licenses the exclusion; the code never guesses it.

use cairn_event::seal::WRAPPED_DEK_LEN;

/// Why a wrapped DEK sitting in the pen would not open.
///
/// The two arms call for **different operator actions**, which is the whole reason the distinction
/// is kept (issue #581): one says the stored bytes are damaged, the other says the key is wrong.
/// Before this existed, both were reported as the second — a precise untruth where an imprecise
/// near-truth was available for free (principle 4 inverted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappedDekFault {
    /// The stored blob is not the right **size**, so it never reached decryption at all.
    ///
    /// `cairn_event::seal::unwrap_dek` length-checks before it does anything else. A
    /// `sync_quarantine.dek_wrapped` of any other length was written short or has been damaged on
    /// disk — a `db/052` write defect or storage rot, not a foreign medium. Telling an operator to
    /// go and find the right key would send them after a key that would not have helped.
    Damaged,
    /// Correctly sized, and it did not open with the key this node is holding.
    ///
    /// Either the medium came from another node, or this node has not been given its own custody
    /// key yet. **The code does not claim to know which**, and the message says so.
    DidNotOpen,
}

/// Classify an unwrap failure **structurally**, never by matching the error's text.
///
/// `WRAPPED_DEK_LEN` is public, and the length check inside `unwrap_dek` is the first thing it
/// does, so the caller can ask the same question the same way. That keeps this a pure function of
/// the bytes and leaves no string to drift when `cairn-event`'s wording changes.
///
/// Called only when an unwrap has already failed: a blob of the right length that *did* open is
/// never classified.
pub fn classify_wrapped_dek(wrapped: &[u8]) -> WrappedDekFault {
    if wrapped.len() == WRAPPED_DEK_LEN {
        WrappedDekFault::DidNotOpen
    } else {
        WrappedDekFault::Damaged
    }
}

/// Why the custody a pen row was carrying did not reach `event_dek`.
///
/// Three causes, three remedies. The loop can always tell them apart because they are decided at
/// three different points: before the unwrap, at the unwrap, and after the apply door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyGap {
    /// This node could not resolve its own custody key at all, so nothing was even attempted.
    ///
    /// `cmd_requeue` has already printed the reason; this is the per-row consequence.
    KeyUnresolved,
    /// A key was available and the row's wrapped DEK still would not open.
    DekFault(WrappedDekFault),
    /// The DEK opened, the plaintext went to the apply door, and the door admitted the event
    /// **without** custody anyway.
    ///
    /// This is the #578 chain proper. `db/020` step 9 reads `node_unwrap_key` to re-wrap the DEK
    /// for this node; with no row registered it takes a lenient arm that `RAISE WARNING`s and
    /// skips custody entirely — no `event_dek`, no `event_clear`, no twin — and returns normally.
    /// Nothing in this tree polls the connection's message stream, so that warning goes nowhere.
    DoorWithheld,
}

/// One line telling the operator a row was **kept** rather than released, and what to do.
///
/// Every arm ends in the same promise, and it is now a true one: the pen holds both halves, so the
/// remedy is to fix the cause and run `requeue` again. Before #578 this sentence appeared in
/// `cmd_requeue`'s warning and was falsified by the very next statement.
pub fn custody_retained_message(digest_prefix: &str, gap: CustodyGap) -> String {
    let cause = match gap {
        CustodyGap::KeyUnresolved => {
            "this node's custody key could not be resolved, so the penned key was never opened"
                .to_string()
        }
        CustodyGap::DekFault(WrappedDekFault::Damaged) => {
            // Deliberately does NOT mention other nodes, even to rule them out. This arm exists
            // because the old message sent operators hunting for the right key when the bytes
            // themselves were short; a sentence that says "not another node's key" still puts
            // that idea in front of someone reading at 3am. Say what IS wrong, and stop.
            "the penned wrapped DEK is the WRONG LENGTH — the pen row itself is damaged \
             (a truncated write or storage rot), so no key could have opened it"
                .to_string()
        }
        CustodyGap::DekFault(WrappedDekFault::DidNotOpen) => {
            "the penned wrapped DEK did not open with the custody key this node is holding \
             (either the medium came from another node, or this node's own key has not been \
             established yet)"
                .to_string()
        }
        CustodyGap::DoorWithheld => {
            "the apply door admitted the event WITHOUT custody — this node has no registered \
             unwrap key (`cairn-node establish-unwrap-key`), so there was nothing to re-wrap \
             the DEK to"
                .to_string()
        }
    };
    format!(
        "requeue: {digest_prefix} KEPT in the pen — {cause}. The event is in the log but its \
         body cannot be opened, and the pen row is the only copy of its key, so the row is NOT \
         released: fix the cause and run `cairn-sync requeue` again. To accept the loss instead, \
         ack the row (UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = …)."
    )
}

/// One line for a row skipped because a human already licensed its exclusion (issue #581).
///
/// It must not be silent in either direction. An operator who acked rows during a flood, fixed the
/// cause and then ran `requeue` needs to be told why those rows did not come back, and needs the
/// way to put them back in play.
pub fn acked_skip_message(digest_prefix: &str) -> String {
    format!(
        "requeue: {digest_prefix} SKIPPED — a human acked this row, which records a decision that \
         it will never enter the record (db/021). To put it back in play, clear the flag \
         (UPDATE sync_quarantine SET acked = FALSE WHERE content_digest = …) and run requeue again."
    )
}

/// Every outcome one `requeue` run reached, counted.
///
/// **`released_with_custody` is a SUBSET of `released`, not a sixth outcome** — it answers "of the
/// rows that left the pen, how many carried a key that landed", which is the question #579 says a
/// monitor could not ask. Everything else partitions: see [`RequeueCounts::accounted_for`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RequeueCounts {
    /// Rows that left the pen: the event is in the log and the row is gone.
    pub released: usize,
    /// Of those, the ones that carried a wrapped DEK **and** whose custody landed.
    pub released_with_custody: usize,
    /// Rows kept because they carried a key that did not land (the rule at the top of this module).
    pub custody_retained: usize,
    /// Rows a human had already acked, left untouched.
    pub skipped_acked: usize,
    /// Rows the apply door still refuses: not admitted, annotated, kept.
    pub still_quarantined: usize,
    /// Rows that left the pen between the listing and their turn (a concurrent pull, a second
    /// requeue, an operator DELETE).
    pub vanished: usize,
}

impl RequeueCounts {
    /// The rows this run reached a verdict on. On a COMPLETE run this equals `examined`.
    ///
    /// `released_with_custody` is deliberately absent: adding a subset to its own superset would
    /// double-count, which is exactly the "second spelling of a count" defect the slice-2d
    /// measurement rig deleted a field to avoid.
    pub fn accounted_for(&self) -> usize {
        self.released
            + self.custody_retained
            + self.skipped_acked
            + self.still_quarantined
            + self.vanished
    }

    /// The `--metrics` object, built in one place so a successful run and an interrupted one can
    /// never disagree about what a requeue reports (ADR-0060 decision 2).
    ///
    /// `references_unlearnable` is passed in rather than counted here because it is `null` — never
    /// `0` — when the #465 report did not run, and only the caller knows that. The custody counts
    /// take no such treatment: under this module's rule the code always looks, so a `0` in
    /// `released_with_custody` truthfully means "no released row carried custody".
    pub fn metrics(
        &self,
        examined: usize,
        references_unlearnable: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "op": "requeue",
            // examined == accounted_for() on a COMPLETE run. An interrupted one is short by the
            // rows it never reached PLUS ONE — the row it stopped on, which was reached and is
            // deliberately counted nowhere because the failure is what left its outcome
            // undecided. The interruption message states both in words; reconciling the numbers
            // alone would mislead by exactly that one.
            "examined": examined,
            "released": self.released,
            "released_with_custody": self.released_with_custody,
            "custody_retained": self.custody_retained,
            "skipped_acked": self.skipped_acked,
            "still_quarantined": self.still_quarantined,
            "vanished": self.vanished,
            "references_unlearnable": references_unlearnable
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blob of the exact wrapped-DEK length that will not open is a KEY problem.
    #[test]
    fn a_correctly_sized_blob_that_will_not_open_is_a_key_problem() {
        let wrapped = vec![0u8; WRAPPED_DEK_LEN];
        assert_eq!(classify_wrapped_dek(&wrapped), WrappedDekFault::DidNotOpen);
    }

    /// Anything else is a DAMAGED row, whichever side of the length it falls.
    #[test]
    fn any_other_length_is_a_damaged_row() {
        for len in [0, 1, WRAPPED_DEK_LEN - 1, WRAPPED_DEK_LEN + 1, 4096] {
            assert_eq!(
                classify_wrapped_dek(&vec![0u8; len]),
                WrappedDekFault::Damaged,
                "length {len} must classify as a damaged pen row"
            );
        }
    }

    /// The classifier agrees with the function whose failure it is explaining.
    ///
    /// ANTI-VACUITY, and the reason this test is worth its DB-free cost: the classifier's whole
    /// claim is that `unwrap_dek` rejects a wrong-length blob *before* it attempts decryption.
    /// That is a property of `cairn-event`, asserted here rather than assumed, so a change to
    /// `WRAPPED_DEK_LEN` or to that early return fails this test instead of silently turning
    /// "damaged" into "wrong key" in an operator's log.
    #[test]
    fn the_classifier_agrees_with_the_unwrap_it_explains() {
        // House rule 6: derived at runtime, never a literal, and not named for a cryptographic
        // role it does not play — this is a stand-in blob, not key material.
        let secret = cairn_event::seal::Secret32::from_bytes(std::array::from_fn(|i| i as u8));
        let truncated: Vec<u8> = (0..WRAPPED_DEK_LEN - 8).map(|i| i as u8).collect();
        let right_size: Vec<u8> = (0..WRAPPED_DEK_LEN).map(|i| i as u8).collect();

        let truncated_err = cairn_event::seal::unwrap_dek(&truncated, &secret)
            .expect_err("a short blob cannot open");
        assert!(
            truncated_err.to_string().contains("malformed"),
            "cairn-event no longer length-checks first; this classifier's premise is gone: \
             {truncated_err}"
        );
        assert_eq!(classify_wrapped_dek(&truncated), WrappedDekFault::Damaged);

        cairn_event::seal::unwrap_dek(&right_size, &secret)
            .expect_err("arbitrary bytes of the right length must not open");
        assert_eq!(
            classify_wrapped_dek(&right_size),
            WrappedDekFault::DidNotOpen
        );
    }

    /// Each retention cause produces its OWN remedy, and every one keeps the row.
    ///
    /// The distinctness assertion is the point: three arms that rendered the same sentence would
    /// pass a "does it mention the digest" test while telling every operator the same wrong thing.
    #[test]
    fn each_retention_cause_names_its_own_remedy() {
        let gaps = [
            CustodyGap::KeyUnresolved,
            CustodyGap::DekFault(WrappedDekFault::Damaged),
            CustodyGap::DekFault(WrappedDekFault::DidNotOpen),
            CustodyGap::DoorWithheld,
        ];
        let messages: Vec<String> = gaps
            .iter()
            .map(|g| custody_retained_message("abc123", *g))
            .collect();

        for (gap, m) in gaps.iter().zip(&messages) {
            assert!(m.contains("abc123"), "{gap:?} must name the row");
            assert!(
                m.contains("KEPT in the pen"),
                "{gap:?} must say it kept the row"
            );
            assert!(m.contains("requeue` again"), "{gap:?} must name the remedy");
        }
        for (i, a) in messages.iter().enumerate() {
            for b in messages.iter().skip(i + 1) {
                assert_ne!(a, b, "two retention causes rendered the same sentence");
            }
        }
    }

    /// A damaged row must NOT be reported as somebody else's key — the #581 finding, pinned.
    #[test]
    fn a_damaged_row_is_not_reported_as_a_foreign_key() {
        let damaged =
            custody_retained_message("abc123", CustodyGap::DekFault(WrappedDekFault::Damaged));
        assert!(damaged.contains("WRONG LENGTH"));
        assert!(
            !damaged.contains("another node"),
            "a truncated pen row must not send the operator looking for another node's key"
        );
    }

    /// The acked skip tells the operator both what happened and the way back in.
    #[test]
    fn the_acked_skip_names_the_way_back_in() {
        let m = acked_skip_message("abc123");
        assert!(m.contains("abc123") && m.contains("SKIPPED"));
        assert!(
            m.contains("acked = FALSE"),
            "a skipped row is only honest if the operator is told how to unskip it"
        );
    }

    /// The five partitioning outcomes add up, and the subset does not join them.
    #[test]
    fn the_outcomes_partition_and_the_subset_stays_out() {
        let c = RequeueCounts {
            released: 4,
            released_with_custody: 3,
            custody_retained: 2,
            skipped_acked: 1,
            still_quarantined: 5,
            vanished: 6,
        };
        assert_eq!(c.accounted_for(), 4 + 2 + 1 + 5 + 6);
    }

    /// Every field reaches the metrics object, because a count a monitor cannot see is a count
    /// that does not exist (#579's whole complaint).
    #[test]
    fn every_count_reaches_the_metrics_object() {
        let c = RequeueCounts {
            released: 4,
            released_with_custody: 3,
            custody_retained: 2,
            skipped_acked: 1,
            still_quarantined: 5,
            vanished: 6,
        };
        let m = c.metrics(18, serde_json::Value::Null);
        assert_eq!(m["op"], "requeue");
        assert_eq!(m["examined"], 18);
        assert_eq!(m["released"], 4);
        assert_eq!(m["released_with_custody"], 3);
        assert_eq!(m["custody_retained"], 2);
        assert_eq!(m["skipped_acked"], 1);
        assert_eq!(m["still_quarantined"], 5);
        assert_eq!(m["vanished"], 6);
        assert!(m["references_unlearnable"].is_null());
    }

    /// A run that recovered everything and a run that recovered nothing must not serialize
    /// identically — the exact failure #579 describes.
    #[test]
    fn a_recovered_run_and_a_lost_run_do_not_look_alike() {
        let recovered = RequeueCounts {
            released: 3,
            released_with_custody: 3,
            ..Default::default()
        };
        let lost = RequeueCounts {
            custody_retained: 3,
            ..Default::default()
        };
        assert_ne!(
            recovered.metrics(3, serde_json::Value::Null).to_string(),
            lost.metrics(3, serde_json::Value::Null).to_string()
        );
    }
}
