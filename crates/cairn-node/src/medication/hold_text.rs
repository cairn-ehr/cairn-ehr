//! Human-readable lines for the withhold ("W") surface of the CLI (ADR-0081, #803). Pure
//! builders, kept out of `main.rs` so each output form is unit-tested and the two-`Uuid`
//! success lines cannot swap their ids unnoticed (the #828 lesson: `withheld_line` puts a
//! thread id and an event id on one line, and a swap would still compile).
use cairn_medication_view::{short_kid, MedicationHold};
use uuid::Uuid;

/// The line printed under a `medication-list` row for one hold: the paper chart's "W".
/// (The caller indents it under its row.) A pending hold (not started yet) says when it starts, so it cannot be mistaken for a
/// line that is withheld now.
pub fn hold_line(h: &MedicationHold) -> String {
    let until = h.until.as_deref().unwrap_or("reviewed");
    let when = if h.pending {
        format!("from {} until {until}", h.starts_at)
    } else {
        format!("withheld until {until}")
    };
    format!("W {when}: {} (by {})", h.reason, short_kid(&h.withholder))
}

/// The line printed under a CEASED row when the cessation gave a reason.
pub fn ceased_reason_line(reason: &str) -> String {
    format!("ceased: {reason}")
}

/// `medication-withhold`'s success line.
pub fn withheld_line(event_id: Uuid, medication_id: Uuid) -> String {
    format!("withheld medication thread {medication_id}; event {event_id}")
}

/// `medication-release`'s success line; `n` is how many holds the release names.
pub fn released_line(event_id: Uuid, medication_id: Uuid, n: usize) -> String {
    format!("released {n} hold(s) on medication thread {medication_id}; event {event_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hold(until: Option<&str>, pending: bool) -> MedicationHold {
        MedicationHold {
            withhold_event_id: Uuid::from_u128(1),
            medication_id: Uuid::from_u128(2),
            reason: "BP 85/50".into(),
            starts_at: "2026-10-21T20:00Z".into(),
            until: until.map(Into::into),
            withholder: "1a2b3c4d5e6f".into(),
            pending,
        }
    }

    #[test]
    fn a_timed_hold_reads_withheld_until_the_instant() {
        assert_eq!(
            hold_line(&hold(Some("2026-10-22T08:00Z"), false)),
            "W withheld until 2026-10-22T08:00Z: BP 85/50 (by 1a2b3c4d)"
        );
    }

    #[test]
    fn an_open_hold_reads_until_reviewed() {
        let mut h = hold(None, false);
        h.reason = "discuss with GP".into();
        assert_eq!(
            hold_line(&h),
            "W withheld until reviewed: discuss with GP (by 1a2b3c4d)"
        );
    }

    #[test]
    fn a_pending_hold_names_its_start() {
        assert_eq!(
            hold_line(&hold(None, true)),
            "W from 2026-10-21T20:00Z until reviewed: BP 85/50 (by 1a2b3c4d)"
        );
    }

    #[test]
    fn a_ceased_row_shows_its_reason() {
        assert_eq!(ceased_reason_line("rash"), "ceased: rash");
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
