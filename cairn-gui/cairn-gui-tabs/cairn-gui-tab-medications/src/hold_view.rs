//! How a hold on a drug line (the paper chart's "W", ADR-0081) and a cessation's reason read
//! on the med-list row.
//!
//! Pure functions of the hold and of `now`, kept out of `row_view.rs` so each wording is
//! unit-tested on its own. Instants arrive as RFC 3339 UTC text; the clinician reads the
//! WORKSTATION's local time ("W until 20:00"), because a nurse reads the clock on the wall,
//! not UTC. The CLI's equivalent (`cairn-node`'s `hold_text.rs`) prints the raw instants: it
//! is an operator tool, this is the bedside window.
use cairn_medication_view::{short_kid, MedicationHold};
use chrono::{DateTime, Datelike, TimeZone};
use serde::Serialize;
use std::fmt::Display;

/// One hold as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HoldView {
    /// What the Release gesture sends back.
    pub withhold_event_id: String,
    /// The button's text: "W until 20:00" | "W until reviewed" | "W from Thu 22 Oct 08:00".
    pub symbol: String,
    /// The revealed text AND the accessible label.
    pub detail: String,
    /// Not started yet: the line is not withheld by this hold.
    pub pending: bool,
}

/// An RFC 3339 instant in `now`'s zone: "20:00" when it falls on `now`'s local date, else
/// "Thu 22 Oct 08:00". Text that does not parse is returned unchanged — shown as recorded,
/// never blank (principle 4).
pub fn local_instant<Tz: TimeZone>(utc: &str, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    let Ok(t) = DateTime::parse_from_rfc3339(utc) else {
        return utc.to_string();
    };
    let t = t.with_timezone(&now.timezone());
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if t.year() != now.year() {
        // A far date must not read as the coming month of this year.
        t.format("%a %-d %b %Y %H:%M").to_string()
    } else {
        t.format("%a %-d %b %H:%M").to_string()
    }
}

/// The hold's end as words: "reviewed" is the wire's spelling of an open hold.
fn until_text<Tz: TimeZone>(until: &str, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    if until == "reviewed" {
        "reviewed".to_string()
    } else {
        local_instant(until, now)
    }
}

/// Build the view of one hold, worded against `now`.
pub fn hold_view<Tz: TimeZone>(h: &MedicationHold, now: &DateTime<Tz>) -> HoldView
where
    Tz::Offset: Display,
{
    let until = until_text(&h.until, now);
    let by = short_kid(&h.withholder);
    let (symbol, detail) = if h.pending {
        let start = local_instant(&h.starts_at, now);
        (
            format!("W from {start}"),
            format!("Withheld from {start} until {until} by {by}: {}", h.reason),
        )
    } else {
        (
            format!("W until {until}"),
            format!("Withheld until {until} by {by}: {}", h.reason),
        )
    };
    HoldView {
        withhold_event_id: h.withhold_event_id.to_string(),
        symbol,
        detail,
        pending: h.pending,
    }
}

/// What a ceased line reveals: why it was stopped.
pub fn cessation_detail(reason: &str) -> String {
    format!("Stopped: {reason}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_rows::{chart, member, row};
    use crate::view::build_view;
    use cairn_medication_view::{MedicationStatus, VouchState};
    use chrono::FixedOffset;
    use uuid::Uuid;

    /// 2026-10-21 09:00 at UTC+10.
    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-21T09:00:00+10:00").unwrap()
    }

    fn hold(until: &str, pending: bool) -> MedicationHold {
        MedicationHold {
            withhold_event_id: Uuid::from_u128(7),
            medication_id: Uuid::from_u128(1),
            reason: "BP 85/50".into(),
            starts_at: "2026-10-21T22:00:00Z".into(),
            until: until.into(),
            withholder: "1a2b3c4d5e6f".into(),
            pending,
        }
    }

    #[test]
    fn a_hold_ending_today_shows_the_local_clock_time() {
        let v = hold_view(&hold("2026-10-21T10:00:00Z", false), &now());
        assert_eq!(v.symbol, "W until 20:00");
    }

    #[test]
    fn a_hold_ending_another_day_names_the_day() {
        let v = hold_view(&hold("2026-10-21T22:00:00Z", false), &now());
        assert_eq!(v.symbol, "W until Thu 22 Oct 08:00");
    }

    /// "Thu 1 Jan 08:00" would be read as the coming January; a far date names its year.
    #[test]
    fn an_instant_in_another_year_names_its_year() {
        assert_eq!(
            local_instant("2098-12-31T22:00:00Z", &now()),
            "Thu 1 Jan 2099 08:00"
        );
    }

    #[test]
    fn an_open_hold_reads_until_reviewed() {
        let v = hold_view(&hold("reviewed", false), &now());
        assert_eq!(v.symbol, "W until reviewed");
        assert_eq!(v.detail, "Withheld until reviewed by 1a2b3c4d: BP 85/50");
        assert_eq!(v.withhold_event_id, Uuid::from_u128(7).to_string());
    }

    #[test]
    fn a_pending_hold_names_its_start() {
        let v = hold_view(&hold("reviewed", true), &now());
        assert!(v.pending);
        assert!(v.symbol.starts_with("W from "), "{}", v.symbol);
        assert!(v.detail.starts_with("Withheld from "), "{}", v.detail);
    }

    #[test]
    fn an_unparseable_instant_is_shown_as_recorded() {
        let v = hold_view(&hold("garbage", false), &now());
        assert_eq!(v.symbol, "W until garbage");
    }

    #[test]
    fn a_ceased_line_reveals_its_reason() {
        assert_eq!(cessation_detail("rash"), "Stopped: rash");
    }

    // ---- The row: a hold sits BESIDE the status (the durable ADR-0081 rule).
    // (These live here, not in row_view.rs, to keep that file's size down.)

    fn active_row_with(holds: Vec<MedicationHold>) -> cairn_medication_view::MedicationRow {
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.holds = holds;
        r
    }

    #[test]
    fn a_withheld_line_stays_current_signable_and_ceasable() {
        let view = build_view(&chart(vec![active_row_with(vec![hold("reviewed", false)])]));
        let r = &view.rows[0];
        assert_eq!(r.status_label, "withheld");
        assert!(r.current && r.can_cease && r.can_withhold && r.can_release);
        assert!(
            r.will_be_signed,
            "a hold must not drop the line from sign-off"
        );
    }

    #[test]
    fn a_pending_hold_leaves_the_line_reading_current() {
        let view = build_view(&chart(vec![active_row_with(vec![hold("reviewed", true)])]));
        assert_eq!(view.rows[0].status_label, "current");
        assert!(
            view.rows[0].can_release,
            "a release also cancels a pending hold"
        );
    }

    #[test]
    fn a_ceased_line_carries_its_reason_and_no_controls() {
        let mut r = row(
            1,
            MedicationStatus::Ceased,
            vec![member(1, VouchState::Absent)],
        );
        r.cessation_reason = Some("rash".into());
        let view = build_view(&chart(vec![r]));
        let r = &view.rows[0];
        assert_eq!(r.cessation_detail.as_deref(), Some("Stopped: rash"));
        assert!(!r.can_withhold && !r.can_release);
    }
}
