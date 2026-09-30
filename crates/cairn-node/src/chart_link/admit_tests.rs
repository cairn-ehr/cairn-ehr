//! Tests for `chart_link/admit.rs`, in a sibling file (house rule 4: files under 500 lines).
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
    for subject in [lo, hi] {
        assert!(filing_for(LinkVerb::Unlink, lo, hi, FiledUnder::RecordOf(subject)).is_err());
    }
}

/// What each refusal is a verdict ABOUT (PR #711 review): a chart not held here is the
/// node's state (sync may deliver it); an open chart whose record lacks the pair is the
/// picture judged from — stale or wrong — so it says to reload, never to wait.
#[test]
fn each_refusal_says_what_it_is_a_verdict_about() {
    let (lo, hi) = pair();
    let x = Uuid::from_u128(9);
    let open = |held, holds_both| {
        Some(OpenedChart {
            chart: x,
            held,
            holds_both,
        })
    };
    // Neither subject held: the open chart decides the filing.
    let r = admit_judgement(
        LinkVerb::Unlink,
        (lo, false),
        (hi, false),
        true,
        open(true, false),
    )
    .unwrap_err();
    assert_eq!(r.scope, RefusalScope::Input, "{}", r.text);
    assert!(r.text.contains("does not read both") && r.text.contains("reload the chart"));
    let r = admit_judgement(
        LinkVerb::Unlink,
        (lo, false),
        (hi, false),
        true,
        open(false, true),
    )
    .unwrap_err();
    assert_eq!(r.scope, RefusalScope::NodeState, "{}", r.text);
    assert!(r.text.contains("is not held here") && !r.text.contains("reload"));
    // A held subject decides the filing; the open chart is still checked (Ruling R5).
    let r = admit_judgement(
        LinkVerb::Unlink,
        (lo, true),
        (hi, true),
        false,
        open(true, false),
    )
    .unwrap_err();
    assert_eq!(r.scope, RefusalScope::Input, "{}", r.text);
    assert!(r.text.contains("does not read both") && r.text.contains("reload the chart"));
    let r = admit_judgement(
        LinkVerb::Unlink,
        (lo, true),
        (hi, true),
        false,
        open(false, false),
    )
    .unwrap_err();
    assert_eq!(r.scope, RefusalScope::NodeState, "{}", r.text);
    assert!(r.text.contains("is not held here"));
    // The subject rules are about what this node holds.
    for r in [
        admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true, None),
        admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true, None),
        admit_judgement(LinkVerb::Unlink, (lo, true), (hi, false), false, None),
    ] {
        assert_eq!(r.unwrap_err().scope, RefusalScope::NodeState);
    }
}

#[test]
fn a_link_needs_both_charts_held() {
    let (lo, hi) = pair();
    assert_eq!(
        admit_judgement(LinkVerb::Link, (hi, true), (lo, true), false, None),
        Ok(FiledUnder::Subject(lo))
    );
    // Even a chart already in the record: a link must not reach past this node.
    let refusal = admit_judgement(LinkVerb::Link, (lo, true), (hi, false), true, None)
        .unwrap_err()
        .text;
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
    let refusal = admit_judgement(LinkVerb::Unlink, (hi, true), (lo, false), false, None)
        .unwrap_err()
        .text;
    assert!(
        refusal.contains(&lo.to_string()),
        "names the stranger: {refusal}"
    );
    let refusal = admit_judgement(LinkVerb::Unlink, (lo, false), (hi, false), true, None)
        .unwrap_err()
        .text;
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
    assert!(admit_judgement(LinkVerb::Link, (lo, false), (hi, false), true, Some(opened)).is_err());
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
        .unwrap_err()
        .text;
        assert!(refusal.contains(&a.to_string()), "names the opened chart");
    }
}

/// Ruling R5: an open chart that is not a subject is checked even when a held subject
/// decides the filing — a mistyped `--from` is refused, never reported back as a record.
#[test]
fn an_unrelated_open_chart_is_refused_even_when_a_subject_is_held() {
    let (lo, hi) = pair();
    let x = Uuid::from_u128(9);
    for (held, holds_both) in [(true, false), (false, false), (false, true)] {
        let opened = OpenedChart {
            chart: x,
            held,
            holds_both,
        };
        let refusal = admit_judgement(
            LinkVerb::Unlink,
            (lo, false),
            (hi, true),
            true,
            Some(opened),
        )
        .unwrap_err()
        .text;
        assert!(refusal.contains(&x.to_string()), "names the opened chart");
        let refusal = admit_judgement(
            LinkVerb::Unlink,
            (lo, true),
            (hi, true),
            false,
            Some(opened),
        )
        .unwrap_err()
        .text;
        assert!(refusal.contains(&x.to_string()), "names the opened chart");
    }
}

/// An open chart that IS a subject needs no further check: the subject rules already
/// decided what may be judged from it.
#[test]
fn an_open_subject_needs_no_extra_check() {
    let (lo, hi) = pair();
    let opened = OpenedChart {
        chart: hi,
        held: true,
        holds_both: false,
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

#[test]
fn a_record_holds_both_only_when_it_contains_each() {
    let (lo, hi) = pair();
    let x = Uuid::from_u128(9);
    let both = ChartSet::new([x, lo, hi]).unwrap();
    assert!(record_holds_both(&both, lo, hi));
    assert!(record_holds_both(&both, hi, lo));
    let one = ChartSet::new([x, lo]).unwrap();
    assert!(!record_holds_both(&one, lo, hi));
    assert!(!record_holds_both(&one, hi, lo));
    assert!(!record_holds_both(&ChartSet::new([x]).unwrap(), lo, hi));
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
