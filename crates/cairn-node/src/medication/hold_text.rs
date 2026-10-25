//! Human-readable lines for the withhold ("W") surface of the CLI (ADR-0081, #803). Pure
//! builders, kept out of `main.rs` so each output form is unit-tested and the two-`Uuid`
//! success lines cannot swap their ids unnoticed (the #828 lesson: `withheld_line` puts a
//! thread id and an event id on one line, and a swap would still compile).
use cairn_medication_view::{short_kid, MedicationHold};
use uuid::Uuid;

/// The line printed under a `medication-list` row for one hold: the paper chart's "W".
/// (The caller indents it under its row.) A pending hold (not started yet) says when it
/// starts, so it cannot be mistaken for a line that is withheld now.
///
/// It ends with the two ids `medication-release` needs — the withhold's event id and the
/// thread it is on — so the next shift can release a W it did not write without `--json`.
/// The thread matters on a reconciled line: the hold may sit on a sibling thread, not the
/// one the row is keyed by.
///
/// A release recorded for a later instant (#835) is said before the reason, which is free
/// text and must not seem to contain it: the hold still stands, but it ends earlier than its
/// printed `until`. A release at or before a pending hold's start cancels it instead, and
/// says so: `from <later> … effective <earlier>` would read as a W between the two.
pub fn hold_line(h: &MedicationHold) -> String {
    let when = if h.pending {
        format!("from {} until {}", h.starts_at, h.until)
    } else {
        format!("withheld until {}", h.until)
    };
    let release = match &h.release_pending {
        Some(at) if h.release_cancels_it() => {
            format!(" (cancelled by a release recorded for {at}, before it starts)")
        }
        Some(at) => format!(" (release recorded, effective {at})"),
        None => String::new(),
    };
    format!(
        "W {when}{release}: {} (by {}) [withhold {} on thread {}]",
        h.reason,
        short_kid(&h.withholder),
        h.withhold_event_id,
        h.medication_id
    )
}

/// The lines printed under a CEASED row: the reason the line shows, then every other stop's
/// reason (#857). When the shown stop gave none but another did, that is said first, so the
/// other reason is not read as the shown stop's. Empty when no stop gave a reason.
pub fn ceased_reason_lines(shown: Option<&str>, others: &[String]) -> Vec<String> {
    let first = match shown {
        Some(reason) => format!("ceased: {reason}"),
        None if !others.is_empty() => "ceased: no reason given".to_string(),
        None => return vec![],
    };
    std::iter::once(first)
        .chain(others.iter().map(|r| format!("another stop's reason: {r}")))
        .collect()
}

/// The lines printed under a CURRENT row for its stopped member threads' reasons (#861): a
/// sibling keeps the line current, so none of them may read "ceased". Empty when none gave one.
pub fn stopped_record_lines(reasons: &[String]) -> Vec<String> {
    reasons
        .iter()
        .enumerate()
        .map(|(i, r)| match i {
            0 => format!("a stopped record of this drug gave: {r}"),
            _ => format!("another stopped record gave: {r}"),
        })
        .collect()
}

/// `medication-withhold`'s success line.
pub fn withheld_line(event_id: Uuid, medication_id: Uuid) -> String {
    format!("withheld medication thread {medication_id}; event {event_id}")
}

/// `medication-release`'s success line; `n` is how many DISTINCT holds the release names
/// (the caller counts through `distinct_holds`, as the payload does).
pub fn released_line(event_id: Uuid, medication_id: Uuid, n: usize) -> String {
    format!("released {n} hold(s) on medication thread {medication_id}; event {event_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(until: &str, pending: bool) -> MedicationHold {
        MedicationHold {
            withhold_event_id: Uuid::from_u128(1),
            medication_id: Uuid::from_u128(2),
            reason: "BP 85/50".into(),
            starts_at: "2026-10-21T20:00:00Z".into(),
            until: until.into(),
            withholder: "1a2b3c4d5e6f".into(),
            pending,
            release_pending: None,
        }
    }

    /// The `[withhold <event> on thread <thread>]` suffix every hold line ends with.
    fn ids(h: &MedicationHold) -> String {
        format!(
            "[withhold {} on thread {}]",
            h.withhold_event_id, h.medication_id
        )
    }

    #[test]
    fn a_timed_hold_reads_withheld_until_the_instant() {
        let h = hold("2026-10-22T08:00:00Z", false);
        assert_eq!(
            hold_line(&h),
            format!(
                "W withheld until 2026-10-22T08:00:00Z: BP 85/50 (by 1a2b3c4d) {}",
                ids(&h)
            )
        );
    }

    #[test]
    fn an_open_hold_reads_until_reviewed() {
        let mut h = hold("reviewed", false);
        h.reason = "discuss with GP".into();
        assert_eq!(
            hold_line(&h),
            format!(
                "W withheld until reviewed: discuss with GP (by 1a2b3c4d) {}",
                ids(&h)
            )
        );
    }

    #[test]
    fn a_pending_hold_names_its_start() {
        let h = hold("reviewed", true);
        assert_eq!(
            hold_line(&h),
            format!(
                "W from 2026-10-21T20:00:00Z until reviewed: BP 85/50 (by 1a2b3c4d) {}",
                ids(&h)
            )
        );
    }

    /// #835: a release recorded for later is said, ahead of the free-text reason (so it can
    /// never read as part of it) and of the ids.
    #[test]
    fn a_pending_release_is_said_on_the_hold() {
        let mut h = hold("reviewed", false);
        h.release_pending = Some("2026-10-22T06:00:00Z".into());
        assert_eq!(
            hold_line(&h),
            format!(
                "W withheld until reviewed (release recorded, effective 2026-10-22T06:00:00Z): \
                 BP 85/50 (by 1a2b3c4d) {}",
                ids(&h)
            )
        );
    }

    /// A release before a pending hold's start cancels it: "release recorded, effective <t>"
    /// beside "from <later>" would read as a W from <later> to <t> (PR #856 review).
    #[test]
    fn a_release_before_the_start_says_the_hold_never_begins() {
        let mut h = hold("reviewed", true);
        h.release_pending = Some("2026-10-21T06:00:00Z".into());
        assert_eq!(
            hold_line(&h),
            format!(
                "W from 2026-10-21T20:00:00Z until reviewed (cancelled by a release recorded \
                 for 2026-10-21T06:00:00Z, before it starts): BP 85/50 (by 1a2b3c4d) {}",
                ids(&h)
            )
        );
    }

    #[test]
    fn a_hold_line_names_the_ids_a_release_needs_each_in_its_slot() {
        // Distinct ids, so a swap of the event id and the thread id cannot pass.
        let h = hold("reviewed", false);
        let line = hold_line(&h);
        assert!(
            line.ends_with(&format!(
                "[withhold {} on thread {}]",
                Uuid::from_u128(1),
                Uuid::from_u128(2)
            )),
            "got: {line}"
        );
    }

    #[test]
    fn a_ceased_row_shows_its_reason_and_every_other_stops() {
        let others = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ceased_reason_lines(Some("rash"), &[]), ["ceased: rash"]);
        assert_eq!(
            ceased_reason_lines(Some("patient request"), &others(&["angioedema"])),
            [
                "ceased: patient request",
                "another stop's reason: angioedema"
            ]
        );
        // #857's worst case: a later stop with no reason must not hide the adverse reaction.
        assert_eq!(
            ceased_reason_lines(None, &others(&["angioedema"])),
            [
                "ceased: no reason given",
                "another stop's reason: angioedema"
            ]
        );
        assert!(ceased_reason_lines(None, &[]).is_empty());
    }

    #[test]
    fn a_current_line_says_its_stopped_records_reasons_without_saying_ceased() {
        let others = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            stopped_record_lines(&others(&["angioedema", "rash"])),
            [
                "a stopped record of this drug gave: angioedema",
                "another stopped record gave: rash"
            ]
        );
        assert!(stopped_record_lines(&[]).is_empty());
    }

    #[test]
    fn withheld_line_puts_each_id_in_its_slot() {
        let (e, m) = (Uuid::from_u128(0xe), Uuid::from_u128(0xa));
        let s = withheld_line(e, m);
        assert_eq!(s, format!("withheld medication thread {m}; event {e}"));
        assert_ne!(s, withheld_line(m, e));
    }

    #[test]
    fn released_line_puts_each_id_in_its_slot() {
        let (e, m) = (Uuid::from_u128(0xe), Uuid::from_u128(0xa));
        let s = released_line(e, m, 2);
        assert_eq!(
            s,
            format!("released 2 hold(s) on medication thread {m}; event {e}")
        );
        assert_ne!(s, released_line(m, e, 2));
    }
}
