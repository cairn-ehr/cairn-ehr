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

/// Check a filing against the pair before anything is signed, and return the envelope chart.
/// `Subject` must name `low` or `high` (a wrong one would misfile the event in an unrelated
/// patient's stream, invisibly to the database floor); `RecordOf` is refused for a LINK and for
/// a chart that IS a subject (that is `Subject`, and saying otherwise hides which rule admitted
/// it). `Err(text)` names what is wrong.
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
///
/// `Ok(about)`: the chart to file the event under — `low` by the C1 convention when both are
/// held, else the one held chart (db/005 step 8b refuses a local event about a chart with
/// no history here). `Err(text)`: the refusal, naming the chart(s) at fault.
pub fn admit_judgement(
    verb: LinkVerb,
    (a, a_held): (Uuid, bool),
    (b, b_held): (Uuid, bool),
    shared_record: bool,
) -> Result<Uuid, String> {
    let (low, _) = canonical_pair(a, b);
    let rule = "a link needs both charts held on this node; an unlink needs the chart you \
                have open held here, and the other held too or already read as part of its \
                record";
    match (verb, a_held, b_held) {
        (_, true, true) => Ok(low),
        (_, false, false) => Err(format!(
            "neither chart {a} nor chart {b} is held on this node — {rule}"
        )),
        (LinkVerb::Unlink, true, false) if shared_record => Ok(a),
        (LinkVerb::Unlink, false, true) if shared_record => Ok(b),
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
            admit_judgement(LinkVerb::Link, (hi, true), (lo, true), false),
            Ok(lo)
        );
        // Even a chart already in the record: a link must not reach past this node.
        let refusal = admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true).unwrap_err();
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
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), true),
            Ok(hi)
        );
        assert_eq!(
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, true), true),
            Ok(hi)
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), false).unwrap_err();
        assert!(
            refusal.contains(&lo.to_string()),
            "names the stranger: {refusal}"
        );
        let refusal =
            admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true).unwrap_err();
        assert!(refusal.contains(&lo.to_string()) && refusal.contains(&hi.to_string()));
    }
}
