//! The pane's "How these charts are linked" list (repair path R2b-2, R6): one line per standing
//! link, plus one line per bridged "different people" judgement — a pair a clinician separated
//! that other links still join (db/054 case (c)). A standing link db/054 doubts is marked on its
//! own line, saying what is held back and why.
//!
//! Per PAIR, not per member chart (maintainer, 2026-09-30): each pair appears once, so each
//! control does too, and an unlink that leaves two charts joined through another link can point
//! at one list. ONE door per line — "Review this link…" / "Review this judgement…" — whose review
//! panel offers every verdict that applies (R6): the panel, not the list, decides which.

use crate::chart_set::MemberLine;
use crate::state::AppState;
use cairn_medication_view::ChartSet;
use cairn_node::patient::edges::{DoubtKind, LinkDoubt, RecordEdge};
use serde::Serialize;
use uuid::Uuid;

/// The door on a standing link's line.
pub const REVIEW_LINK: &str = "Review this link…";
/// The door on a bridged separation's line.
pub const REVIEW_JUDGEMENT: &str = "Review this judgement…";

/// Appended to a standing link db/054 doubts (case (a)/(b)). Says what is held, and by what. It
/// says "while", never "until": with two doubted links, judging this one leaves the hold.
const CLASH_NOTE: &str = " — in doubt: this node's identity check finds a clash between these two \
     charts; lines from other charts of this record cannot be signed off while this link is in \
     doubt";

/// What a line's pair IS, which decides the verdicts its review panel can offer. It travels to
/// the webview and comes back with "Review…", so the panel reviews the pair the line showed —
/// never a guess from the two ids alone (the same two charts can be a link one moment and a
/// separation the next).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PairKind {
    /// A standing link joining the two charts.
    Link,
    /// A standing "different people" judgement on two charts other links still join.
    Separated,
}

impl PairKind {
    /// The kind the webview sent back, in exactly the spelling serde sent it (`"link"` /
    /// `"separated"`). Anything else is `None`: the caller refuses it, never defaults it.
    pub fn parse(s: &str) -> Option<PairKind> {
        match s {
            "link" => Some(PairKind::Link),
            "separated" => Some(PairKind::Separated),
            _ => None,
        }
    }
}

/// The two reads the list is built from, made in one snapshot ([`read_record_links`]): the
/// standing links, and db/054's doubts about the record's pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordLinks {
    /// The record's standing links (`record_edges`).
    pub edges: Vec<RecordEdge>,
    /// What db/054 doubts (`record_doubts`): clashing links and bridged separations.
    pub doubts: Vec<LinkDoubt>,
}

/// One line as the pane lists it. `low`/`high`/`kind` travel back with the line's door.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordLinkView {
    /// The pair's lower chart id.
    pub low: String,
    /// The pair's higher chart id.
    pub high: String,
    /// The whole line as the clinician reads it.
    pub text: String,
    /// What the pair is: a standing link, or a bridged separation.
    pub kind: PairKind,
    /// Whether the line showed the pair's standing judgement as a clinician's. It travels back
    /// with the door, so the review panel can tell a line drawn before a colleague's confirm
    /// from the pair as it stands now (`unlink::line_still_shows`). Always true for a separation:
    /// db/054 reports only a clinician's "different people".
    pub attested: bool,
    /// The line's one door ([`REVIEW_LINK`] or [`REVIEW_JUDGEMENT`]).
    pub button: String,
}

/// A chart as a link line names it: its member line's name and its id, or the id alone when
/// the member lines could not be read (the ids still tie the line to the rows' source labels).
fn chart_label(chart: Uuid, members: &[MemberLine]) -> String {
    let id = chart.to_string();
    match members.iter().find(|m| m.patient_id == id) {
        Some(m) => format!("{} (chart {id})", m.name),
        None => format!("chart {id}"),
    }
}

/// One link's line. "Attested" is db/018's stored definition; its absence is worded as what
/// is known — no clinician's confirmation is on record HERE — never as "the matcher", which a
/// peer's human link with an attester this node has not enrolled would make untrue (principle 4).
pub fn record_link_line(edge: &RecordEdge, members: &[MemberLine]) -> RecordLinkView {
    let how = if edge.attested {
        "linked by a clinician's judgement"
    } else {
        "linked without a clinician's confirmation on record here"
    };
    RecordLinkView {
        low: edge.low.to_string(),
        high: edge.high.to_string(),
        text: format!(
            "{} and {} — {how}, recorded {} (UTC)",
            chart_label(edge.low, members),
            chart_label(edge.high, members),
            edge.recorded_on
        ),
        kind: PairKind::Link,
        attested: edge.attested,
        button: REVIEW_LINK.into(),
    }
}

/// A bridged "different people" (case (c)) — not a standing link, so its own line.
fn separated_line(d: &LinkDoubt, members: &[MemberLine]) -> RecordLinkView {
    RecordLinkView {
        low: d.low.to_string(),
        high: d.high.to_string(),
        text: format!(
            "{} and {} — a clinician recorded on {} (UTC) that these are different people, but \
             other links still join them",
            chart_label(d.low, members),
            chart_label(d.high, members),
            d.recorded_on
        ),
        kind: PairKind::Separated,
        attested: true,
        button: REVIEW_JUDGEMENT.into(),
    }
}

/// Is this standing link one db/054 finds a clash on? Matched by the canonical pair, which
/// both reads carry.
fn has_clash(edge: &RecordEdge, doubts: &[LinkDoubt]) -> bool {
    doubts
        .iter()
        .any(|d| d.kind == DoubtKind::Clash && (d.low, d.high) == (edge.low, edge.high))
}

/// Does db/054 report a clash on a pair that is not among the standing links? Unreachable from
/// one consistent read; [`links_section`] refuses to draw the list when it happens.
fn has_orphan_clash(links: &RecordLinks) -> bool {
    links
        .doubts
        .iter()
        .filter(|d| d.kind == DoubtKind::Clash)
        .any(|d| !links.edges.iter().any(|e| (e.low, e.high) == (d.low, d.high)))
}

/// Read the record's standing links and db/054's doubts about them. Both empty for a single
/// chart (nothing joins it); an error, never an empty list, when a linked set cannot be read —
/// see [`links_section`]. Takes `state.db`'s lock, which means the caller must not already hold
/// it: `tokio::sync::Mutex` is not re-entrant.
///
/// Both reads run in ONE read-only REPEATABLE READ transaction, so they describe the same moment.
/// The lock alone would not do that: it keeps out only this window's other commands, while the
/// sync door commits peer links through its own connection, and each autocommit statement takes
/// its own snapshot. Two snapshots could mark a clash on a link the edge read had not yet seen.
pub async fn read_record_links(state: &AppState, charts: &ChartSet) -> Result<RecordLinks, String> {
    if !charts.is_linked() {
        return Ok(RecordLinks::default());
    }
    let Some(db) = state.db.as_ref() else {
        return Err("there is no database to read the links from".into());
    };
    let mut db = db.lock().await;
    let snapshot = db
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await
        .map_err(|e| format!("could not open a read of the links: {e}"))?;
    let diagnose = |e: anyhow::Error| cairn_node::db_diagnosis::operator_chain(&e);
    let edges = cairn_node::patient::edges::record_edges(&snapshot, charts)
        .await
        .map_err(diagnose)?;
    let doubts = cairn_node::patient::edges::record_doubts(&snapshot, charts)
        .await
        .map_err(diagnose)?;
    // Read-only: dropping would roll back just as well; committing says the read is finished.
    snapshot
        .commit()
        .await
        .map_err(|e| format!("could not finish the read of the links: {e}"))?;
    Ok(RecordLinks { edges, doubts })
}

/// The list, or — when it could not be read — no lines and a sentence saying so. An unread
/// list must never render as an empty one: that reads as "nothing joins these charts".
///
/// The lines: one per standing link, carrying [`CLASH_NOTE`] when db/054 doubts it; then one per
/// bridged separation. A link nothing doubts reads exactly as it did before R6.
///
/// `linked` is whether the record's chart set has more than one chart. A linked record whose
/// read returned NO standing links is contradictory (something joins these charts), so it is
/// said, not shown — whatever doubts were read beside it, since a separation joins nothing; an
/// unlinked single chart legitimately has none and stays silent.
pub fn links_section(
    links: Result<RecordLinks, String>,
    members: &[MemberLine],
    linked: bool,
) -> (Vec<RecordLinkView>, Option<String>) {
    match links {
        Ok(links) if links.edges.is_empty() && linked => (
            vec![],
            Some(
                "No standing link joining these charts could be read here — reload the chart \
                 before undoing any."
                    .to_string(),
            ),
        ),
        // A clash is a property of a standing link (db/054), so one naming no standing link
        // means the two reads disagree. Drawn anyway, the doubt would vanish while the R1b hold
        // still points at a marked line nobody can find: said, never dropped.
        Ok(links) if has_orphan_clash(&links) => (
            vec![],
            Some(
                "Which of these links is in doubt could not be read consistently — reload the \
                 chart before judging any."
                    .to_string(),
            ),
        ),
        Ok(links) => {
            let link_lines = links.edges.iter().map(|e| {
                let mut line = record_link_line(e, members);
                if has_clash(e, &links.doubts) {
                    line.text.push_str(CLASH_NOTE);
                }
                line
            });
            let separated_lines = links
                .doubts
                .iter()
                .filter(|d| d.kind == DoubtKind::Separated)
                .map(|d| separated_line(d, members));
            (link_lines.chain(separated_lines).collect(), None)
        }
        Err(e) => (
            vec![],
            Some(format!(
                "The links joining these charts could not be read, so none can be judged from \
                 here — reload the chart; if this persists, the node needs its operator: {e}"
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(attested: bool) -> RecordEdge {
        RecordEdge {
            low: Uuid::from_u128(1),
            high: Uuid::from_u128(2),
            attested,
            recorded_on: "2026-09-28".into(),
        }
    }
    /// A db/054 doubt on the same pair `edge` builds (charts 1 and 2).
    fn doubt(kind: DoubtKind) -> LinkDoubt {
        LinkDoubt {
            low: Uuid::from_u128(1),
            high: Uuid::from_u128(2),
            kind,
            recorded_on: "2026-10-01".into(),
        }
    }
    fn member(n: u128, name: &str) -> MemberLine {
        MemberLine {
            patient_id: Uuid::from_u128(n).to_string(),
            name: name.into(),
            text: String::new(),
        }
    }

    #[test]
    fn a_link_line_names_both_charts_how_it_was_made_and_when() {
        let members = [member(1, "SMITH John"), member(2, "SMYTHE John")];
        let human = record_link_line(&edge(true), &members);
        assert!(human.text.contains("SMITH John") && human.text.contains("SMYTHE John"));
        assert!(human.text.contains("by a clinician's judgement"));
        assert!(human.text.contains("2026-09-28 (UTC)"));
        assert_eq!(human.low, Uuid::from_u128(1).to_string());
        let machine = record_link_line(&edge(false), &members);
        assert!(machine
            .text
            .contains("without a clinician's confirmation on record here"));
        assert!(
            !machine.text.contains("matcher"),
            "un-attested is not proof of the matcher"
        );
    }

    #[test]
    fn a_chart_missing_from_the_members_is_named_by_its_id_alone() {
        let line = record_link_line(&edge(true), &[]);
        assert!(line.text.contains(&Uuid::from_u128(1).to_string()));
    }

    /// Review Focus 5.
    #[test]
    fn an_unread_link_list_says_so_and_offers_no_unlink() {
        let (links, error) = links_section(Err("connection reset".into()), &[], true);
        assert!(links.is_empty());
        let error = error.expect("an unread list is said, never shown empty");
        assert!(error.contains("could not be read"));
        assert!(error.contains("reload"));
    }

    #[test]
    fn a_read_list_carries_no_error() {
        let (links, error) = links_section(
            Ok(RecordLinks {
                edges: vec![edge(true)],
                doubts: vec![],
            }),
            &[],
            true,
        );
        assert_eq!(links.len(), 1);
        assert!(error.is_none());
    }

    #[test]
    fn a_linked_record_with_no_edges_read_says_so() {
        let (links, error) = links_section(Ok(RecordLinks::default()), &[], true);
        assert!(links.is_empty());
        assert!(error.expect("never blank").contains("reload the chart"));
    }

    #[test]
    fn a_single_chart_with_no_edges_is_silent() {
        let (links, error) = links_section(Ok(RecordLinks::default()), &[], false);
        assert!(links.is_empty() && error.is_none());
    }

    #[test]
    fn a_clash_line_says_it_is_in_doubt_and_why() {
        let links = RecordLinks {
            edges: vec![edge(false)],
            doubts: vec![doubt(DoubtKind::Clash)],
        };
        let (lines, err) = links_section(Ok(links), &[], true);
        assert!(err.is_none());
        assert_eq!(lines.len(), 1);
        assert!(lines[0]
            .text
            .contains("in doubt: this node's identity check finds a clash"));
        assert_eq!(lines[0].button, REVIEW_LINK);
        assert_eq!(lines[0].kind, PairKind::Link);
    }

    /// With two doubted links, judging this one leaves the hold: the note may say only what is
    /// always true — lines from the record's other charts are held WHILE this link is in doubt —
    /// never that a judgement of this link lifts the hold ("until").
    #[test]
    fn a_clash_note_never_promises_that_judging_this_link_lifts_the_hold() {
        assert!(!CLASH_NOTE.contains("until"), "{CLASH_NOTE}");
        assert_eq!(
            CLASH_NOTE,
            " — in doubt: this node's identity check finds a clash between these two charts; \
             lines from other charts of this record cannot be signed off while this link is in \
             doubt"
        );
    }

    #[test]
    fn an_undoubted_link_line_is_byte_identical_to_before() {
        let links = RecordLinks {
            edges: vec![edge(true)],
            doubts: vec![],
        };
        let (lines, _) = links_section(Ok(links), &[], true);
        assert_eq!(lines[0].text, record_link_line(&edge(true), &[]).text);
        assert_eq!(lines[0].button, REVIEW_LINK);
    }

    #[test]
    fn a_separated_pair_gets_its_own_line_after_the_links() {
        let sep = LinkDoubt {
            low: Uuid::from_u128(1),
            high: Uuid::from_u128(3),
            kind: DoubtKind::Separated,
            recorded_on: "2026-10-01".into(),
        };
        let links = RecordLinks {
            edges: vec![edge(false)],
            doubts: vec![sep],
        };
        let (lines, _) = links_section(Ok(links), &[], true);
        assert_eq!(lines.len(), 2);
        let l = &lines[1];
        assert_eq!(l.kind, PairKind::Separated);
        assert_eq!(l.button, REVIEW_JUDGEMENT);
        assert!(l.text.contains(
            "a clinician recorded on 2026-10-01 (UTC) that these are different people, but \
             other links still join them"
        ));
    }

    /// A clash doubt naming a pair that is NOT a standing edge cannot come from one snapshot
    /// (db/054's clash arm is a property of a standing link), so the two reads disagree. Dropping
    /// the doubt would leave the R1b hold pointing at a line nobody can find; the list is said
    /// unread instead, never drawn without the mark (PR #777 review).
    #[test]
    fn a_clash_on_a_pair_with_no_line_is_said_never_dropped() {
        let links = RecordLinks {
            edges: vec![edge(true)],
            doubts: vec![LinkDoubt {
                high: Uuid::from_u128(3),
                ..doubt(DoubtKind::Clash)
            }],
        };
        let (lines, error) = links_section(Ok(links), &[], true);
        assert!(lines.is_empty(), "no line drawn without its mark");
        assert!(error.expect("never silent").contains("reload the chart"));
    }

    /// Each line carries the attestation it SHOWS, so the review panel can tell a line drawn
    /// before a colleague's confirm from the pair as it now stands (`unlink::line_still_shows`).
    /// A separation is always a clinician's judgement (db/054 reports only attested ones).
    #[test]
    fn a_line_carries_the_attestation_it_shows() {
        assert!(record_link_line(&edge(true), &[]).attested);
        assert!(!record_link_line(&edge(false), &[]).attested);
        assert!(separated_line(&doubt(DoubtKind::Separated), &[]).attested);
    }

    /// db/018 makes it unreachable, but a pair reported as BOTH kinds must still render: the
    /// link line carries the note, the separation gets its own line, nothing panics.
    #[test]
    fn a_pair_reported_as_both_kinds_renders_both_lines() {
        let links = RecordLinks {
            edges: vec![edge(false)],
            doubts: vec![doubt(DoubtKind::Clash), doubt(DoubtKind::Separated)],
        };
        let (lines, _) = links_section(Ok(links), &[], true);
        assert_eq!(lines.len(), 2);
        assert_eq!(
            (lines[0].kind, lines[1].kind),
            (PairKind::Link, PairKind::Separated)
        );
        assert!(lines[0].text.contains("in doubt"));
    }

    /// The webview sends the line's `kind` back as text; only the two serialized spellings parse.
    #[test]
    fn a_pair_kind_parses_only_its_own_spellings() {
        for k in [PairKind::Link, PairKind::Separated] {
            let sent = serde_json::to_value(k).unwrap();
            assert_eq!(PairKind::parse(sent.as_str().unwrap()), Some(k));
        }
        assert_eq!(PairKind::parse("Link"), None);
        assert_eq!(PairKind::parse(""), None);
    }

    /// A separation alone is not a standing link: a linked record whose edge read came back
    /// empty is still contradictory, whatever doubts were read beside it.
    #[test]
    fn a_linked_record_with_only_doubts_still_says_reload() {
        let links = RecordLinks {
            edges: vec![],
            doubts: vec![doubt(DoubtKind::Separated)],
        };
        let (lines, error) = links_section(Ok(links), &[], true);
        assert!(lines.is_empty());
        assert!(error.expect("never blank").contains("reload the chart"));
    }
}
