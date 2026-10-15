//! The "same person" half of the review panel's wording (R6, #716), as pure functions. The
//! same rule as `view.rs` and `unlink_view.rs`: on this panel the wording IS the safety content.
//!
//! The review panel (`unlink_view.rs`'s [`LinkReviewView`](super::unlink_view::LinkReviewView))
//! offers up to two verdicts on the pair a line of "How these charts are linked" showed. This
//! module decides the "same person" one — [`same_verdict`] says whether it is offered, under
//! which label, and what the clinician must know before clicking — and words what it did
//! ([`confirm_report`]). Both outcomes are an ordinary attested `identity.link.asserted`:
//! - on a doubted LINK it is "Same person — confirm": the link already stands, the clinician's
//!   judgement now vouches for it, so db/054 stops doubting it;
//! - on a SEPARATED pair it is "Same person — link them": it overlays an earlier clinician's
//!   "different people", and the panel says so before the click, not after.

use super::record_links::PairKind;
use super::unlink::ShownPair;
use super::view::{LinkReportView, LINK_OUTRANKED, LINK_STILL_JOINED};
use cairn_node::chart_link::LinkEffect;
use uuid::Uuid;

/// The verdict button on a doubted standing link.
pub const CONFIRM_SAME: &str = "Same person — confirm";
/// The verdict button on a bridged separation.
pub const LINK_THEM: &str = "Same person — link them";

/// The "same person" verdict the panel offers, if any. `label: None` means no button; `note`
/// is a sentence shown beside the verdicts whether or not a button is offered (it says either
/// why there is none, or what the button overrides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SameVerdict {
    /// The button's text, or `None` when the verdict is not offered.
    pub label: Option<String>,
    /// What the clinician must read before (or instead of) clicking.
    pub note: Option<String>,
}

/// Decide the "same person" verdict for the pair the panel shows.
///
/// `held` is each chart of the pair with whether this node holds it (`ChartFacts::held`). In
/// order:
/// 1. a chart not held here → no button, and a note naming it: the node refuses a judgement it
///    cannot make on both charts (`admit_judgement`), so the panel never offers one (Review
///    Focus 2). Unlink is unaffected — that is decided elsewhere.
/// 2. an attested link → nothing to confirm, nothing to say: a clinician already vouches for it.
/// 3. an un-attested link → "Same person — confirm".
/// 4. a separated pair → "Same person — link them", with a note that it overrides an earlier
///    clinician's judgement (both stay on record), and the alternative if they ARE different.
pub fn same_verdict(shown: &ShownPair, held: &[(Uuid, bool)]) -> SameVerdict {
    if let Some((chart, _)) = held.iter().find(|(_, is_held)| !is_held) {
        let what = match shown.kind {
            PairKind::Link => "this link can be confirmed",
            PairKind::Separated => "these charts can be linked",
        };
        return SameVerdict {
            label: None,
            note: Some(format!(
                "Chart {chart} is not held on this node, so {what} only on a node that holds both \
                 charts."
            )),
        };
    }
    match shown.kind {
        PairKind::Link if shown.attested => SameVerdict {
            label: None,
            note: None,
        },
        PairKind::Link => SameVerdict {
            label: Some(CONFIRM_SAME.into()),
            note: None,
        },
        PairKind::Separated => SameVerdict {
            label: Some(LINK_THEM.into()),
            note: Some(format!(
                "A clinician recorded on {} (UTC) that these two charts are different people. \
                 Linking them records your judgement over theirs; both judgements stay on record. \
                 If they are different people, unlink one of the links that still joins them \
                 instead.",
                shown.recorded_on
            )),
        },
    }
}

/// What the "same person" verdict did, as the outcome line says it. Never "combined" or
/// "linked N charts": a confirm on a standing link changes no membership, and a "link them" on
/// a bridged pair joins charts that were already one record through other links.
///
/// The non-effect arms are the link's own (`view.rs`): a confirm IS a link judgement.
pub fn confirm_report(effect: LinkEffect, kind: PairKind, low: Uuid, high: Uuid) -> LinkReportView {
    match (effect, kind) {
        (LinkEffect::TookEffect, PairKind::Link) => LinkReportView {
            sentence: format!(
                "Confirmed — charts {low} and {high} are recorded as the same person by your \
                 judgement; this link is no longer in doubt."
            ),
            reload: true,
        },
        (LinkEffect::TookEffect, PairKind::Separated) => LinkReportView {
            sentence: format!(
                "Linked — your judgement that charts {low} and {high} are the same person now \
                 stands over the earlier judgement that they were different people; both stay on \
                 record."
            ),
            reload: true,
        },
        // Recorded but not standing: the panel stays open on the disagreement (no reload).
        (LinkEffect::Outranked, _) => LinkReportView {
            sentence: LINK_OUTRANKED.into(),
            reload: false,
        },
        (LinkEffect::StillJoined, _) => LinkReportView {
            sentence: LINK_STILL_JOINED.into(),
            reload: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medication_view::ChartSet;

    fn shown(kind: PairKind, attested: bool) -> ShownPair {
        ShownPair {
            kind,
            attested,
            recorded_on: "2026-10-01".into(),
        }
    }
    const BOTH: [(Uuid, bool); 2] = [(Uuid::from_u128(1), true), (Uuid::from_u128(2), true)];

    #[test]
    fn an_unattested_link_held_both_sides_offers_confirm() {
        let v = same_verdict(&shown(PairKind::Link, false), &BOTH);
        assert_eq!(v.label.as_deref(), Some(CONFIRM_SAME));
        assert_eq!(v.note, None);
    }

    #[test]
    fn an_attested_link_offers_no_confirm_and_says_nothing() {
        let v = same_verdict(&shown(PairKind::Link, true), &BOTH);
        assert_eq!((v.label, v.note), (None, None));
    }

    #[test]
    fn a_separated_pair_offers_link_them_and_names_the_earlier_judgement() {
        let v = same_verdict(&shown(PairKind::Separated, true), &BOTH);
        assert_eq!(v.label.as_deref(), Some(LINK_THEM));
        let note = v.note.unwrap();
        assert!(note.contains("2026-10-01"), "{note}");
        assert!(
            note.contains("records your judgement over theirs"),
            "{note}"
        );
        assert!(note.contains("both judgements stay on record"), "{note}");
        assert!(
            note.contains("unlink one of the links that still joins them"),
            "{note}"
        );
    }

    /// Review Focus 2.
    #[test]
    fn a_chart_not_held_here_offers_no_confirm_and_names_it() {
        let held = [(Uuid::from_u128(1), true), (Uuid::from_u128(2), false)];
        let v = same_verdict(&shown(PairKind::Link, false), &held);
        assert_eq!(v.label, None);
        let note = v.note.unwrap();
        assert!(
            note.contains(&Uuid::from_u128(2).to_string())
                && note.contains("not held on this node"),
            "{note}"
        );
    }

    /// The not-held note names the verdict it withholds, per kind: a separated pair is not a
    /// link to confirm.
    #[test]
    fn a_not_held_note_names_the_withheld_verdict_per_kind() {
        let held = [(Uuid::from_u128(1), false), (Uuid::from_u128(2), true)];
        let link = same_verdict(&shown(PairKind::Link, false), &held);
        assert!(link.note.unwrap().contains("this link can be confirmed"));
        let sep = same_verdict(&shown(PairKind::Separated, true), &held);
        assert_eq!(sep.label, None);
        assert!(sep.note.unwrap().contains("these charts can be linked"));
    }

    #[test]
    fn a_confirm_report_never_says_it_combined_charts() {
        let (lo, hi) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let r = confirm_report(LinkEffect::TookEffect, PairKind::Link, lo, hi);
        assert!(
            r.sentence.starts_with("Confirmed —") && r.reload,
            "{}",
            r.sentence
        );
        let r = confirm_report(LinkEffect::TookEffect, PairKind::Separated, lo, hi);
        assert!(r.sentence.contains("now stands over the earlier judgement") && r.reload);
        let r = confirm_report(LinkEffect::Outranked, PairKind::Link, lo, hi);
        assert!(!r.reload && r.sentence.contains("NOT in effect"));
    }

    /// The confirm's Outranked and StillJoined lines are the link's own: a confirm IS a link
    /// judgement (an attested `identity.link.asserted`), so it lands exactly as one does.
    #[test]
    fn a_confirm_lands_like_a_link_when_it_does_not_take_effect() {
        let (lo, hi) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let r = confirm_report(LinkEffect::Outranked, PairKind::Separated, lo, hi);
        assert_eq!(r.sentence, crate::link::view::LINK_OUTRANKED);
        let set = ChartSet::single(lo);
        let link = crate::link::view::link_report(LinkEffect::StillJoined, &set, &set);
        let r = confirm_report(LinkEffect::StillJoined, PairKind::Link, lo, hi);
        assert_eq!(r.sentence, link.sentence);
        assert!(r.reload);
    }
}
