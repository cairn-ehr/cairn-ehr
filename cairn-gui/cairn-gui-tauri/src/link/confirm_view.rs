//! The "same person" half of the review panel's wording (R6, #716), as pure functions. The
//! same rule as `view.rs` and `unlink_view.rs`: on this panel the wording IS the safety content.
//!
//! The review panel (`unlink_view.rs`'s [`LinkReviewView`](super::unlink_view::LinkReviewView))
//! offers up to two verdicts on the pair a line of "How these charts are linked" showed. This
//! module decides the "same person" one — [`same_verdict`] says whether it is offered, under
//! which label, and what the clinician must know before clicking — and words what it did
//! ([`confirm_report`]). Both outcomes are an ordinary attested `identity.link.asserted`:
//! - on an UN-ATTESTED link it is "Same person — confirm": the link already stands, the
//!   clinician's judgement now vouches for it, so db/054 stops doubting it (if it did);
//! - on a SEPARATED pair it is "Same person — link them": it overlays an earlier clinician's
//!   "different people", and the panel says so before the click, not after.

use super::judgement_view::{LinkReportView, LINK_OUTRANKED, LINK_STILL_JOINED};
use super::record_links::PairKind;
use super::unlink::ShownPair;
use cairn_node::chart_link::{LinkEffect, StandingLink};
use uuid::Uuid;

/// The verdict button on an un-attested standing link (doubted by db/054 or not).
pub const CONFIRM_SAME: &str = "Same person — confirm";
/// The verdict button on a bridged separation.
pub const LINK_THEM: &str = "Same person — link them";

/// The review panel's problem line when the pair's standing judgement, read under the panel's
/// lock, is no longer what the list line showed (see [`standing_agrees`]), or is gone. "Pair",
/// not "link": the line may have shown a clinician's "different people" judgement.
pub const PAIR_CHANGED_MID_READ: &str =
    "This pair changed while it was being read — close the comparison and review it again.";

/// Does the judgement standing on the pair (`standing_link`, read by the review panel) agree
/// with what the list line showed (`ShownPair`, read a moment earlier under another lock)?
///
/// Why it matters (safety): the panel sends the standing judgement's content address back with
/// either verdict, and the node accepts the verdict only if that address still stands. If a
/// colleague's attested unlink landed between the two reads while a third chart still joins the
/// record, the line says "doubted link" but the address is the UNLINK's — and the node would
/// accept a confirm "as shown" over a judgement the clinician never saw. So the panel keeps the
/// address only when the two reads agree:
/// - a `Link` line needs a standing `link` with the same attestation (a colleague's confirm
///   landing mid-read changes the attestation: the line is stale too);
/// - a `Separated` line needs a standing CLINICIAN's `unlink` (db/054 reports only those).
///
/// This is NOT a second rule for what is doubted (db/054's `cairn_chart_set_doubts` is the one
/// spelling of that): it only checks that two reads of the same pair agree, and a mismatch
/// withholds both verdicts — fail-safe, never a verdict offered on a guess.
pub fn standing_agrees(standing: &StandingLink, shown: &ShownPair) -> bool {
    match shown.kind {
        PairKind::Link => standing.state == "link" && standing.attested == shown.attested,
        PairKind::Separated => standing.state == "unlink" && standing.attested,
    }
}

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

/// Decide the "same person" verdict for the pair `low`–`high` the panel shows.
///
/// `held` is each chart the panel read facts for, with whether this node holds it
/// (`ChartFacts::held`). In order:
/// 1. an attested link → nothing to confirm, nothing to say: a clinician already vouches for it.
///    First, because it is true whether or not both charts are held here — a not-held note
///    ("can be confirmed only on a node that holds both") would contradict the line's own
///    "linked by a clinician's judgement".
/// 2. a chart not held here → no button, and a note naming it: the node refuses a judgement it
///    cannot make on both charts (`admit_judgement`), so the panel never offers one. Unlink is
///    unaffected — that is decided elsewhere.
/// 3. `low` or `high` missing from `held` → no button, no note: a judgement needs BOTH charts
///    of the pair known to be held. A short facts read already puts a problem on the panel;
///    this is the defence in depth that keeps the rule true here too.
/// 4. an un-attested link → "Same person — confirm".
/// 5. a separated pair → "Same person — link them", with a note that it overrides an earlier
///    clinician's judgement (both stay on record), and the alternative if they ARE different.
pub fn same_verdict(
    shown: &ShownPair,
    low: Uuid,
    high: Uuid,
    held: &[(Uuid, bool)],
) -> SameVerdict {
    let nothing = SameVerdict {
        label: None,
        note: None,
    };
    if shown.kind == PairKind::Link && shown.attested {
        return nothing;
    }
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
    if !(known_held(held, low) && known_held(held, high)) {
        return nothing;
    }
    match shown.kind {
        PairKind::Link => SameVerdict {
            label: Some(CONFIRM_SAME.into()),
            note: None,
        },
        PairKind::Separated => SameVerdict {
            label: Some(LINK_THEM.into()),
            note: Some(format!(
                "A clinician recorded on {} (UTC) that these two charts are different people. \
                 Linking them records your judgement over theirs; both judgements stay on record. \
                 If they are different people, open \"Review this link…\" on one of the links \
                 still joining them, under \"How these charts are linked\" (every link line \
                 carries it, marked in doubt or not), and unlink it instead.",
                shown.recorded_on
            )),
        },
    }
}

/// Is `chart` among the charts the panel read facts for, and held here?
fn known_held(held: &[(Uuid, bool)], chart: Uuid) -> bool {
    held.iter().any(|&(id, is_held)| id == chart && is_held)
}

/// What the "same person" verdict did, as the outcome line says it. Never "combined" or
/// "linked N charts": a confirm on a standing link changes no membership, and a "link them" on
/// a bridged pair joins charts that were already one record through other links.
///
/// The non-effect arms are the link's own (`judgement_view.rs`'s `LINK_OUTRANKED` /
/// `LINK_STILL_JOINED`): a confirm IS a link judgement.
pub fn confirm_report(effect: LinkEffect, kind: PairKind, low: Uuid, high: Uuid) -> LinkReportView {
    match (effect, kind) {
        (LinkEffect::TookEffect, PairKind::Link) => LinkReportView {
            // No "no longer in doubt": a confirm may be of a machine link nothing doubted.
            sentence: format!(
                "Confirmed — charts {low} and {high} are recorded as the same person by your \
                 judgement."
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
    use cairn_node::chart_link::StandingLink;

    fn shown(kind: PairKind, attested: bool) -> ShownPair {
        ShownPair {
            kind,
            attested,
            recorded_on: "2026-10-01".into(),
        }
    }
    const LOW: Uuid = Uuid::from_u128(1);
    const HIGH: Uuid = Uuid::from_u128(2);
    const BOTH: [(Uuid, bool); 2] = [(LOW, true), (HIGH, true)];

    fn standing(state: &str, attested: bool) -> StandingLink {
        StandingLink {
            // A content address, computed — not key material (house rule 6).
            content_address: vec![7; 34],
            state: state.into(),
            attested,
        }
    }

    /// Defence in depth: a verdict needs BOTH charts of the pair known to be held. An empty or
    /// one-sided `held` — a facts read that came back short — offers nothing.
    #[test]
    fn a_verdict_needs_both_charts_of_the_pair_known_held() {
        for kind in [PairKind::Link, PairKind::Separated] {
            let pair = shown(kind, kind == PairKind::Separated);
            assert_eq!(same_verdict(&pair, LOW, HIGH, &[]).label, None, "{kind:?}");
            assert_eq!(same_verdict(&pair, LOW, HIGH, &[(LOW, true)]).label, None);
            assert_eq!(same_verdict(&pair, LOW, HIGH, &[(HIGH, true)]).label, None);
        }
    }

    /// Facts for two OTHER charts, both held, are not facts about this pair.
    #[test]
    fn held_charts_that_are_not_the_pair_offer_no_verdict() {
        let others = [(Uuid::from_u128(5), true), (Uuid::from_u128(6), true)];
        let v = same_verdict(&shown(PairKind::Link, false), LOW, HIGH, &others);
        assert_eq!(v.label, None);
    }

    /// Which judgement stands must be the one the line showed.
    #[test]
    fn a_standing_judgement_agrees_only_with_the_pair_it_is() {
        let link = |attested| shown(PairKind::Link, attested);
        let sep = shown(PairKind::Separated, true);
        // A link line agrees with a standing link of the same attestation…
        assert!(standing_agrees(&standing("link", false), &link(false)));
        assert!(standing_agrees(&standing("link", true), &link(true)));
        // …and never with a standing unlink (a colleague's unlink landed mid-read).
        assert!(!standing_agrees(&standing("unlink", true), &link(false)));
        assert!(!standing_agrees(&standing("unlink", false), &link(false)));
        // A separation agrees only with a CLINICIAN's standing unlink…
        assert!(standing_agrees(&standing("unlink", true), &sep));
        assert!(!standing_agrees(&standing("unlink", false), &sep));
        // …and never with a standing link (someone relinked them mid-read).
        assert!(!standing_agrees(&standing("link", true), &sep));
        assert!(!standing_agrees(&standing("link", false), &sep));
    }

    /// The mismatch-attested case: a colleague confirmed the link between the list read and
    /// the panel read. The line said "doubted", the pair now says "vouched for".
    #[test]
    fn a_link_whose_attestation_changed_mid_read_does_not_agree() {
        assert!(!standing_agrees(
            &standing("link", true),
            &shown(PairKind::Link, false)
        ));
        assert!(!standing_agrees(
            &standing("link", false),
            &shown(PairKind::Link, true)
        ));
    }

    #[test]
    fn an_unattested_link_held_both_sides_offers_confirm() {
        let v = same_verdict(&shown(PairKind::Link, false), LOW, HIGH, &BOTH);
        assert_eq!(v.label.as_deref(), Some(CONFIRM_SAME));
        assert_eq!(v.note, None);
    }

    /// An attested link has nothing to confirm, and that is true whether or not both charts are
    /// held here: a note saying it "can be confirmed only on a node that holds both charts" would
    /// contradict the line's own "linked by a clinician's judgement".
    #[test]
    fn an_attested_link_with_a_chart_not_held_says_nothing() {
        let held = [(LOW, true), (HIGH, false)];
        let v = same_verdict(&shown(PairKind::Link, true), LOW, HIGH, &held);
        assert_eq!((v.label, v.note), (None, None));
    }

    #[test]
    fn an_attested_link_offers_no_confirm_and_says_nothing() {
        let v = same_verdict(&shown(PairKind::Link, true), LOW, HIGH, &BOTH);
        assert_eq!((v.label, v.note), (None, None));
    }

    #[test]
    fn a_separated_pair_offers_link_them_and_names_the_earlier_judgement() {
        let v = same_verdict(&shown(PairKind::Separated, true), LOW, HIGH, &BOTH);
        assert_eq!(v.label.as_deref(), Some(LINK_THEM));
        let note = v.note.unwrap();
        assert!(note.contains("2026-10-01"), "{note}");
        assert!(
            note.contains("records your judgement over theirs"),
            "{note}"
        );
        assert!(note.contains("both judgements stay on record"), "{note}");
        // #783 (the PR #792 review): the note names the unlink's door, as the hold's remedy does
        // — the links still joining the pair may carry no doubt mark.
        assert!(
            note.contains(
                "open \"Review this link…\" on one of the links still joining them, under \"How \
                 these charts are linked\" (every link line carries it, marked in doubt or not), \
                 and unlink it instead"
            ),
            "{note}"
        );
    }

    /// A doubted link with one chart not held here: no confirm, and the note says which chart.
    #[test]
    fn a_chart_not_held_here_offers_no_confirm_and_names_it() {
        let held = [(Uuid::from_u128(1), true), (Uuid::from_u128(2), false)];
        let v = same_verdict(&shown(PairKind::Link, false), LOW, HIGH, &held);
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
        let link = same_verdict(&shown(PairKind::Link, false), LOW, HIGH, &held);
        assert!(link.note.unwrap().contains("this link can be confirmed"));
        let sep = same_verdict(&shown(PairKind::Separated, true), LOW, HIGH, &held);
        assert_eq!(sep.label, None);
        assert!(sep.note.unwrap().contains("these charts can be linked"));
    }

    #[test]
    fn a_confirm_report_never_says_it_combined_charts() {
        let (lo, hi) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let r = confirm_report(LinkEffect::TookEffect, PairKind::Link, lo, hi);
        assert_eq!(
            r.sentence,
            format!(
                "Confirmed — charts {lo} and {hi} are recorded as the same person by your \
                 judgement."
            )
        );
        // A confirm may be of a machine link nothing doubted: never presuppose a doubt.
        assert!(!r.sentence.contains("doubt"), "{}", r.sentence);
        assert!(r.reload);
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
        assert_eq!(r.sentence, crate::link::judgement_view::LINK_OUTRANKED);
        let set = ChartSet::single(lo);
        let link = crate::link::judgement_view::link_report(LinkEffect::StillJoined, &set, &set);
        let r = confirm_report(LinkEffect::StillJoined, PairKind::Link, lo, hi);
        assert_eq!(r.sentence, link.sentence);
        assert!(r.reload);
    }
}
