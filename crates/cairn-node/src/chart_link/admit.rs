//! Who may make a link/unlink judgement from this node, and which chart its event is FILED
//! under. **Pure** — no database — so every rule here is unit-tested on its own; `judge` in the
//! parent module reads the facts (is each chart held? does a record contain the pair?) and asks.
//!
//! "Filed under" is the event ENVELOPE's `patient_id`: the chart whose `event_log` stream the
//! event sits in. It is not what the event is ABOUT — db/018 reads the pair from the payload's
//! `subject_a`/`subject_b`, never from the envelope (audit, R2b-2 plan). db/005 step 8b refuses a
//! local event filed under a chart with no history here, so the filed-under chart must be HELD.
use super::{canonical_pair, LinkVerb};
use uuid::Uuid;

/// Which chart a judgement's event is filed under, and WHY that chart may carry it.
///
/// Typed rather than a bare `Uuid` so the one relaxation #699 (a) makes cannot leak: only an
/// UNLINK may be filed under a chart that is neither subject ([`filing_for`] refuses the rest).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FiledUnder {
    /// One of the two charts being judged (the C1 convention: `low`, or the held one).
    Subject(Uuid),
    /// The chart the clinician has OPEN, held here, whose record reads both subjects as part of
    /// it (#699 (a)): the far link B–C of an A–B–C record, when neither B nor C is held here.
    RecordOf(Uuid),
}

impl FiledUnder {
    /// The chart the envelope names, whichever reason admitted it.
    pub fn chart(self) -> Uuid {
        match self {
            FiledUnder::Subject(c) | FiledUnder::RecordOf(c) => c,
        }
    }
}

/// The chart the clinician has OPEN when they judge — the record the judgement is made from.
/// Only consulted for an UNLINK where neither subject is held here (#699 (a)): the far link
/// B–C of an A–B–C record, judged while reading A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenedChart {
    /// The open chart itself.
    pub chart: Uuid,
    /// A `patient_chart` row: db/005 step 8b will admit an event filed under it.
    pub held: bool,
    /// Its record (`person_charts`) reads BOTH subjects as part of it, here.
    pub holds_both: bool,
}

/// Check a filing against the pair before anything is signed, and return the envelope chart.
/// `Subject` must name `low` or `high` (a wrong one would misfile the event in an unrelated
/// patient's stream, invisibly to the database floor); `RecordOf` is refused for a LINK and for
/// a chart that IS a subject (that is `Subject`, and saying otherwise hides which rule admitted
/// it). `Err(text)` names what is wrong.
///
/// Being pure, it CANNOT check that a `RecordOf` chart's record really contains both subjects,
/// nor that the chart is held — that is the caller's job, done in `judge`, which re-reads the
/// record inside the judgement's own transaction before anything is signed.
pub fn filing_for(
    verb: LinkVerb,
    low: Uuid,
    high: Uuid,
    filed: FiledUnder,
) -> Result<Uuid, String> {
    match filed {
        FiledUnder::Subject(c) if c == low || c == high => Ok(c),
        FiledUnder::Subject(c) => Err(format!(
            "a judgement about ({low}, {high}) cannot be filed under chart {c} as one of its subjects"
        )),
        FiledUnder::RecordOf(_) if verb == LinkVerb::Link => Err(format!(
            "a link between {low} and {high} must be filed under one of them, never under a third chart"
        )),
        FiledUnder::RecordOf(c) if c == low || c == high => Err(format!(
            "chart {c} is a subject of the judgement, not a third chart"
        )),
        FiledUnder::RecordOf(c) => Ok(c),
    }
}

/// Whether a judgement may be made on this pair, given what this node holds — and if so,
/// which chart its event is filed under. **Pure**, so the rule is unit-testable apart from
/// the database.
///
/// - LINK needs BOTH charts held (a `patient_chart` row — see
///   `patient::person::ChartIdentity::held`). The floor admits a link naming a chart that
///   has not synced yet, correctly (offline-first); a human's deliberate act from this node
///   has no such excuse, and a typo would otherwise attach a stranger's future chart to
///   this person.
/// - UNLINK attaches nothing, so that risk does not apply. It needs one chart held (the
///   one the clinician has open) and the other either held too or already part of that
///   chart's record here (`shared_record`) — the member line R1 displays for a chart whose
///   registration has not reached this node. A never-linked stranger is still refused.
/// - UNLINK with NEITHER chart held (#699 (a)): admitted only when judged from an `opened`
///   chart that is held here and whose record reads both as part of it — the far link B–C of
///   an A–B–C record, judged while reading A. It is filed under that open chart
///   ([`FiledUnder::RecordOf`]). A held subject is still preferred whenever there is one, and
///   `opened` never admits a link.
///
/// `Ok(filed)`: the chart to file the event under, and why — `low` by the C1 convention when
/// both are held, else the one held chart (db/005 step 8b refuses a local event about a chart
/// with no history here), else the open chart. `Err(text)`: the refusal, naming the chart(s)
/// at fault.
pub fn admit_judgement(
    verb: LinkVerb,
    (a, a_held): (Uuid, bool),
    (b, b_held): (Uuid, bool),
    shared_record: bool,
    opened: Option<OpenedChart>,
) -> Result<FiledUnder, String> {
    let (low, _) = canonical_pair(a, b);
    let rule = "a link needs both charts held on this node; an unlink needs the chart you \
                have open held here, and the other held too or already read as part of its \
                record; or, for an unlink, the chart you have open held here with both in its \
                record";
    match (verb, a_held, b_held) {
        (_, true, true) => Ok(FiledUnder::Subject(low)),
        (LinkVerb::Unlink, false, false) => match opened {
            Some(o) if o.held && o.holds_both => Ok(FiledUnder::RecordOf(o.chart)),
            Some(o) => Err(format!(
                "neither chart {a} nor chart {b} is held on this node, and the chart you have \
                 open ({}) {} — {rule}",
                o.chart,
                if o.held {
                    "does not read both as part of its record"
                } else {
                    "is not held here either"
                }
            )),
            None => Err(format!(
                "neither chart {a} nor chart {b} is held on this node — {rule}"
            )),
        },
        (_, false, false) => Err(format!(
            "neither chart {a} nor chart {b} is held on this node — {rule}"
        )),
        (LinkVerb::Unlink, true, false) if shared_record => Ok(FiledUnder::Subject(a)),
        (LinkVerb::Unlink, false, true) if shared_record => Ok(FiledUnder::Subject(b)),
        // Exactly one chart unheld, and not admitted above. Say only what is true: for an
        // unlink that means it is also outside the other's record; for a link, whether it
        // is inside is beside the point.
        (_, a_held, _) => {
            let (unheld, other) = if a_held { (b, a) } else { (a, b) };
            let outside = match verb {
                LinkVerb::Unlink => format!(" and is not part of chart {other}'s record here"),
                LinkVerb::Link => String::new(),
            };
            Err(format!(
                "chart {unheld} is not held on this node{outside} — {rule}"
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Uuid, Uuid) {
        let lo = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let hi = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        (lo, hi)
    }

    #[test]
    fn a_subject_filing_must_name_one_of_the_pair() {
        let (lo, hi) = pair();
        let third = Uuid::from_u128(9);
        for verb in [LinkVerb::Link, LinkVerb::Unlink] {
            assert_eq!(filing_for(verb, lo, hi, FiledUnder::Subject(lo)), Ok(lo));
            assert_eq!(filing_for(verb, lo, hi, FiledUnder::Subject(hi)), Ok(hi));
            assert!(filing_for(verb, lo, hi, FiledUnder::Subject(third)).is_err());
        }
    }

    /// Review Focus 4: the #699 (a) relaxation must never reach a link.
    #[test]
    fn a_link_is_never_filed_under_a_third_chart() {
        let (lo, hi) = pair();
        let third = Uuid::from_u128(9);
        let refusal = filing_for(LinkVerb::Link, lo, hi, FiledUnder::RecordOf(third)).unwrap_err();
        assert!(refusal.contains("never under a third chart"), "{refusal}");
        assert_eq!(
            filing_for(LinkVerb::Unlink, lo, hi, FiledUnder::RecordOf(third)),
            Ok(third)
        );
    }

    #[test]
    fn a_subject_is_not_a_third_chart() {
        let (lo, hi) = pair();
        assert!(filing_for(LinkVerb::Unlink, lo, hi, FiledUnder::RecordOf(lo)).is_err());
    }

    #[test]
    fn a_link_needs_both_charts_held() {
        let (lo, hi) = pair();
        assert_eq!(
            admit_judgement(LinkVerb::Link, (hi, true), (lo, true), false, None),
            Ok(FiledUnder::Subject(lo))
        );
        // Even a chart already in the record: a link must not reach past this node.
        let refusal =
            admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true, None).unwrap_err();
        assert!(refusal.contains(&hi.to_string()), "{refusal}");
        assert!(
            !refusal.contains("not part of"),
            "true of this chart, so unsaid: {refusal}"
        );
    }

    #[test]
    fn an_unlink_may_name_a_displayed_member_not_held_here_but_not_a_stranger() {
        let (lo, hi) = pair();
        // Filed under whichever chart IS held, in either argument position.
        assert_eq!(
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), true, None),
            Ok(FiledUnder::Subject(hi))
        );
        assert_eq!(
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, true), true, None),
            Ok(FiledUnder::Subject(hi))
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), false, None).unwrap_err();
        assert!(
            refusal.contains(&lo.to_string()),
            "names the stranger: {refusal}"
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true, None).unwrap_err();
        assert!(refusal.contains(&lo.to_string()) && refusal.contains(&hi.to_string()));
    }

    /// #699 (a): the far link of A–B–C, neither B nor C held, judged from A.
    #[test]
    fn an_unlink_neither_held_is_filed_under_the_opened_record_that_holds_both() {
        let (lo, hi) = pair();
        let a = Uuid::from_u128(9);
        let opened = OpenedChart {
            chart: a,
            held: true,
            holds_both: true,
        };
        assert_eq!(
            admit_judgement(
                LinkVerb::Unlink,
                (lo, false),
                (hi, false),
                true,
                Some(opened)
            ),
            Ok(FiledUnder::RecordOf(a))
        );
        // Never for a link, whatever the opened record holds.
        assert!(
            admit_judgement(LinkVerb::Link, (lo, false), (hi, false), true, Some(opened)).is_err()
        );
    }

    #[test]
    fn the_opened_chart_must_be_held_and_its_record_must_hold_both() {
        let (lo, hi) = pair();
        let a = Uuid::from_u128(9);
        for (held, holds_both) in [(false, true), (true, false), (false, false)] {
            let opened = OpenedChart {
                chart: a,
                held,
                holds_both,
            };
            let refusal = admit_judgement(
                LinkVerb::Unlink,
                (lo, false),
                (hi, false),
                false,
                Some(opened),
            )
            .unwrap_err();
            assert!(refusal.contains(&a.to_string()), "names the opened chart");
        }
    }

    /// A held subject is still preferred: the third-chart arm is only for neither-held.
    #[test]
    fn a_held_subject_is_filed_under_itself_even_when_a_chart_is_open() {
        let (lo, hi) = pair();
        let opened = OpenedChart {
            chart: Uuid::from_u128(9),
            held: true,
            holds_both: true,
        };
        assert_eq!(
            admit_judgement(
                LinkVerb::Unlink,
                (lo, false),
                (hi, true),
                true,
                Some(opened)
            ),
            Ok(FiledUnder::Subject(hi))
        );
    }
}
