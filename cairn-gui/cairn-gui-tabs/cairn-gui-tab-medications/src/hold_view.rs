//! How a hold on a drug line (the paper chart's "W", ADR-0081) and a cessation's reason read
//! on the med-list row.
//!
//! Pure functions of the hold and of `now`, kept out of `row_view.rs` so each wording is
//! unit-tested on its own. Instants arrive as RFC 3339 UTC text; the clinician reads the
//! WORKSTATION's local time ("W until 20:00"), because a nurse reads the clock on the wall,
//! not UTC. The CLI's equivalent (`cairn-node`'s `hold_text.rs`) prints the raw instants: it
//! is an operator tool, this is the bedside window.
use cairn_medication_view::{short_kid, MedicationHold};
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use serde::Serialize;
use std::fmt::Display;

/// One hold as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HoldView {
    /// What the Release gesture sends back.
    pub withhold_event_id: String,
    /// The button's text: "W until 20:00" | "W until reviewed" | "W from Thu 22 Oct 08:00".
    pub symbol: String,
    /// The revealed text, and the second half of the accessible label (`symbol — detail`).
    pub detail: String,
    /// Not started yet: the line is not withheld by this hold.
    pub pending: bool,
}

/// An RFC 3339 instant in `now`'s zone: "20:00" when it falls on `now`'s local date, else
/// "Thu 22 Oct 08:00". Seconds are shown only when the instant has them ("20:00:45"): a hold
/// written from the CLI or a peer may end mid-minute, and "until 20:00" would then read as over
/// while it is still in force. Text that does not parse is returned unchanged — shown as
/// recorded, never blank (principle 4).
pub fn local_instant<Tz: TimeZone>(utc: &str, now: &DateTime<Tz>) -> String
where
    Tz::Offset: Display,
{
    let Ok(t) = DateTime::parse_from_rfc3339(utc) else {
        return utc.to_string();
    };
    let t = t.with_timezone(&now.timezone());
    let clock = if t.second() == 0 { "%H:%M" } else { "%H:%M:%S" };
    let day = if t.date_naive() == now.date_naive() {
        ""
    } else if t.year() != now.year() {
        // A date in another year must not read as that month of this year.
        "%a %-d %b %Y "
    } else {
        "%a %-d %b "
    };
    t.format(&format!("{day}{clock}")).to_string()
}

/// The next instant after `now` at which one of `holds` starts or ends, as RFC 3339 UTC — the
/// moment the list on screen stops being true. A hold starts and ends with the clock, not with
/// an event, so nothing else re-reads the chart then: without this the window kept "withheld"
/// (or "W from 22:00" on a line reading "current") on screen long after it stopped being so,
/// and a same-day time, printed without its date, read as today's (#838 review). The window
/// re-reads at this instant. Every time printed without a date is such an instant (a pending
/// hold's start or end, a live hold's end, a recorded release taking effect (#835), all
/// later today), so it is never on screen past midnight.
/// An unparseable instant is no boundary (it is shown as recorded).
pub fn next_hold_change<'a, Tz: TimeZone>(
    holds: impl Iterator<Item = &'a MedicationHold>,
    now: &DateTime<Tz>,
) -> Option<String> {
    holds
        .flat_map(|h| {
            [h.starts_at.as_str(), h.until.as_str()]
                .into_iter()
                .chain(h.release_pending.as_deref())
        })
        .filter_map(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))
        .filter(|t| *t > now.with_timezone(&Utc))
        .min()
        .map(|t| t.to_rfc3339())
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
            format!("Withheld from {start} until {until} by {by}"),
        )
    } else {
        (
            format!("W until {until}"),
            format!("Withheld until {until} by {by}"),
        )
    };
    // #835: a release already recorded for a later instant, before the hold would end by
    // itself (db/059 7c), is said on the button itself and in the detail, ahead of the
    // free-text reason so it never reads as part of it. It ends the hold then; one at or
    // before a pending hold's start cancels it, and must say so — "W from 08:00, release at
    // 16:00" would read as a W from 08:00 to 16:00 (PR #856 review).
    let (symbol, detail) = match &h.release_pending {
        Some(at) if h.release_cancels_it() => {
            let at = local_instant(at, now);
            (
                format!("{symbol}, cancelled at {at}"),
                format!("{detail} (cancelled by a release recorded for {at}, before it starts)"),
            )
        }
        Some(at) => {
            let at = local_instant(at, now);
            (
                format!("{symbol}, release at {at}"),
                format!("{detail} (a release is recorded, taking effect at {at})"),
            )
        }
        None => (symbol, detail),
    };
    let detail = format!("{detail}: {}", h.reason);
    HoldView {
        withhold_event_id: h.withhold_event_id.to_string(),
        symbol,
        detail,
        pending: h.pending,
    }
}

/// What a ceased line reveals: why it was stopped — the reason the line shows, then every
/// other stop's reason (#857), one sentence each so a screen reader pauses between them. When
/// the shown stop gave none but another did, that is said first, so the other reason is not
/// read as the shown stop's. `None` when no stop gave a reason.
pub fn cessation_detail(shown: Option<&str>, others: &[String]) -> Option<String> {
    let first = match shown {
        Some(reason) => format!("Stopped: {reason}"),
        None if !others.is_empty() => "Stopped: no reason given".to_string(),
        None => return None,
    };
    let rest = others.iter().map(|r| format!("Another stop's reason: {r}"));
    Some(
        std::iter::once(first)
            .chain(rest)
            .collect::<Vec<_>>()
            .join(". "),
    )
}

/// What a CURRENT line reveals when a stopped member thread gave a reason (#861): a sibling
/// keeps the line current, so it is worded as a stopped record's, never "Stopped". One
/// sentence each, as `cessation_detail`. `None` when none gave one.
pub fn stopped_record_detail(reasons: &[String]) -> Option<String> {
    let (first, rest) = reasons.split_first()?;
    let rest = rest
        .iter()
        .map(|r| format!("Another stopped record gave: {r}"));
    Some(
        std::iter::once(format!("A stopped record of this drug gave: {first}"))
            .chain(rest)
            .collect::<Vec<_>>()
            .join(". "),
    )
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
            release_pending: None,
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

    /// #835: a release recorded for later is said on the button and in the detail, ahead of
    /// the free-text reason. 06:00 UTC is 16:00 at the test's UTC+10.
    #[test]
    fn a_pending_release_is_said_on_the_hold() {
        let mut h = hold("reviewed", false);
        h.release_pending = Some("2026-10-21T06:00:00Z".into());
        let v = hold_view(&h, &now());
        assert_eq!(v.symbol, "W until reviewed, release at 16:00");
        assert_eq!(
            v.detail,
            "Withheld until reviewed by 1a2b3c4d (a release is recorded, taking effect at \
             16:00): BP 85/50"
        );
        // A pending hold whose release falls after its start: it begins, then ends at the release.
        let mut p = hold("reviewed", true);
        p.release_pending = Some("2026-10-22T06:00:00Z".into());
        let v = hold_view(&p, &now());
        assert_eq!(
            v.symbol,
            "W from Thu 22 Oct 08:00, release at Thu 22 Oct 16:00"
        );
    }

    /// A release before a pending hold's start cancels it: "W from Thu 08:00, release at 16:00"
    /// reads as a W from 08:00 to 16:00, but this hold never begins (PR #856 review).
    #[test]
    fn a_release_before_the_start_says_the_hold_is_cancelled() {
        let mut p = hold("reviewed", true);
        p.release_pending = Some("2026-10-21T06:00:00Z".into());
        let v = hold_view(&p, &now());
        assert_eq!(v.symbol, "W from Thu 22 Oct 08:00, cancelled at 16:00");
        assert_eq!(
            v.detail,
            "Withheld from Thu 22 Oct 08:00 until reviewed by 1a2b3c4d (cancelled by a release \
             recorded for 16:00, before it starts): BP 85/50"
        );
    }

    /// The window must re-read when a recorded release takes effect, or the W stays on screen
    /// past it (#835): the release's instant is a hold change like a start or an end.
    #[test]
    fn a_pending_release_is_a_hold_change() {
        let mut h = hold("reviewed", false);
        h.starts_at = "2026-10-20T00:00:00Z".into(); // past
        h.release_pending = Some("2026-10-21T06:00:00Z".into());
        assert_eq!(
            next_hold_change([h].iter(), &now()).as_deref(),
            Some("2026-10-21T06:00:00+00:00")
        );
        // A pending hold's release, before its later start, is the one counted (the cancel).
        let mut p = hold("reviewed", true);
        p.release_pending = Some("2026-10-21T06:00:00Z".into());
        assert_eq!(
            next_hold_change([p].iter(), &now()).as_deref(),
            Some("2026-10-21T06:00:00+00:00")
        );
    }

    #[test]
    fn an_instant_with_seconds_keeps_them() {
        assert_eq!(local_instant("2026-10-21T10:00:45Z", &now()), "20:00:45");
    }

    /// The re-read instant: the earliest start or end still ahead — a pending hold's start
    /// before its own end, an ended hold's past end never — and none for open, unparseable or
    /// absent holds.
    #[test]
    fn the_next_hold_change_is_the_earliest_instant_still_ahead() {
        let mut pending = hold("2026-10-22T10:00:00Z", true); // starts 21 Oct 22:00 UTC
        pending.starts_at = "2026-10-21T12:00:00Z".into();
        let mut live = hold("2026-10-21T15:00:00Z", false);
        live.starts_at = "2026-10-20T00:00:00Z".into(); // past
        let holds = [pending, live];
        assert_eq!(
            next_hold_change(holds.iter(), &now()).as_deref(),
            Some("2026-10-21T12:00:00+00:00")
        );
        let mut open = hold("reviewed", false);
        open.starts_at = "garbage".into();
        assert_eq!(next_hold_change([open].iter(), &now()), None);
        assert_eq!(next_hold_change([].iter(), &now()), None);
    }

    #[test]
    fn an_unparseable_instant_is_shown_as_recorded() {
        let v = hold_view(&hold("garbage", false), &now());
        assert_eq!(v.symbol, "W until garbage");
    }

    #[test]
    fn a_ceased_line_reveals_its_reason_and_every_other_stops() {
        let others = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            cessation_detail(Some("rash"), &[]).as_deref(),
            Some("Stopped: rash")
        );
        assert_eq!(
            cessation_detail(Some("patient request"), &others(&["angioedema", "rash"])).as_deref(),
            Some(
                "Stopped: patient request. Another stop's reason: angioedema. \
                 Another stop's reason: rash"
            )
        );
        // #857's worst case: a later stop with no reason must not hide the adverse reaction.
        assert_eq!(
            cessation_detail(None, &others(&["angioedema"])).as_deref(),
            Some("Stopped: no reason given. Another stop's reason: angioedema")
        );
        assert_eq!(cessation_detail(None, &[]), None);
    }

    #[test]
    fn a_current_line_reveals_its_stopped_records_reasons_without_saying_stopped() {
        let others = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            stopped_record_detail(&others(&["angioedema", "rash"])).as_deref(),
            Some(
                "A stopped record of this drug gave: angioedema. Another stopped record gave: rash"
            )
        );
        assert_eq!(stopped_record_detail(&[]), None);
    }

    // ---- The row: a hold sits BESIDE the status (the durable ADR-0081 rule).

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
        r.other_cessation_reasons = vec!["angioedema".into()];
        let view = build_view(&chart(vec![r]));
        let r = &view.rows[0];
        assert_eq!(
            r.cessation_detail.as_deref(),
            Some("Stopped: rash. Another stop's reason: angioedema")
        );
        assert!(!r.can_withhold && !r.can_release);
    }

    #[test]
    fn a_current_line_carries_its_stopped_records_reasons_and_keeps_its_controls() {
        // #861: a sibling keeps the line current, but a stopped member gave a reason.
        let mut r = row(
            1,
            MedicationStatus::Active,
            vec![member(1, VouchState::Absent)],
        );
        r.stopped_member_reasons = vec!["angioedema".into()];
        let view = build_view(&chart(vec![r]));
        let r = &view.rows[0];
        assert_eq!(r.status_label, "current");
        assert_eq!(
            r.cessation_detail.as_deref(),
            Some("A stopped record of this drug gave: angioedema")
        );
        assert!(r.can_cease && r.can_withhold);
    }
}
