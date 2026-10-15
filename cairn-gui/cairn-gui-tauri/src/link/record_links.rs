//! The pane's "How these charts are linked" list (repair path R2b-2, R6): one line per standing
//! link, plus one line per bridged "different people" judgement — a pair a clinician separated
//! that other links still join (db/054 case (c)). A standing link db/054 doubts is marked on its
//! own line, saying what is held back and why. A link this node cannot verify (R7, #743 part 2)
//! carries a note that holds nothing back.
//!
//! Per PAIR, not per member chart (maintainer, 2026-09-30): each pair appears once, so each
//! control does too, and an unlink that leaves two charts joined through another link can point
//! at one list. ONE door per line — "Review this link…" / "Review this judgement…" — whose review
//! panel offers every verdict that applies (R6): the panel, not the list, decides which.

use crate::chart_set::MemberLine;
use crate::state::AppState;
use cairn_medication_view::ChartSet;
use cairn_node::patient::edges::{DoubtKind, LinkDoubt, RecordEdge, UnverifiedLink};
use serde::Serialize;
use uuid::Uuid;

/// The door on a standing link's line.
pub const REVIEW_LINK: &str = "Review this link…";
/// The door on a bridged separation's line.
pub const REVIEW_JUDGEMENT: &str = "Review this judgement…";

/// Appended to a standing link db/054 doubts (case (a)/(b)). Says what is held, and by what. It
/// says "while", never "until": with two doubted links, judging this one leaves the hold. And it
/// names what CAN be signed (a line recorded on this chart alone, hazard.rs's one exception)
/// rather than what cannot, which a line shared with another chart would make untrue.
const CLASH_NOTE: &str = " — in doubt: this node's identity check finds a clash between these two \
     charts; while this link is in doubt, only lines recorded on this chart alone can be signed off";

/// Appended to an un-attested link whose charts hold different identifiers this node cannot
/// verify (db/054's `cairn_chart_set_unverified_links`, R7, #743 part 2). `systems` names each
/// identifier system once. It says only what is known: db/016's `degrade_hold` must never demote
/// a link, so nothing is held back, and the note claims nothing about sign-off — it stays true
/// whether or not another link of the record is in doubt. Acting on it is the line's own door.
fn unverified_note(systems: &[&str]) -> String {
    format!(
        " — unverified here: the two charts hold different {} identifiers, and this node cannot \
         tell a real mismatch from a difference in formatting",
        systems.join(" and ")
    )
}

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

/// The three reads the list is built from, made in one snapshot ([`read_record_links`]): the
/// standing links, db/054's doubts about the record's pairs, and the links it cannot verify.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordLinks {
    /// The record's standing links (`record_edges`).
    pub edges: Vec<RecordEdge>,
    /// What db/054 doubts (`record_doubts`): clashing links and bridged separations.
    pub doubts: Vec<LinkDoubt>,
    /// Un-attested links over a pair this node cannot verify (`record_unverified_links`, R7):
    /// a note on the line, never a doubt.
    pub unverified: Vec<UnverifiedLink>,
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

/// The identifier systems db/054 cannot verify on this standing link, matched by the canonical
/// pair. A row naming a pair that is not a standing link is never matched, so it is dropped: it
/// cannot come from one snapshot, and no hold depends on it (unlike an orphan clash).
fn unverified_systems<'a>(edge: &RecordEdge, unverified: &'a [UnverifiedLink]) -> Vec<&'a str> {
    unverified
        .iter()
        .filter(|u| (u.low, u.high) == (edge.low, edge.high))
        .map(|u| u.system.as_str())
        .collect()
}

/// Does db/054 report a clash on a pair that is not among the standing links? Unreachable from
/// one consistent read; [`links_section`] refuses to draw the list when it happens.
fn has_orphan_clash(links: &RecordLinks) -> bool {
    links
        .doubts
        .iter()
        .filter(|d| d.kind == DoubtKind::Clash)
        .any(|d| {
            !links
                .edges
                .iter()
                .any(|e| (e.low, e.high) == (d.low, d.high))
        })
}

/// Read the record's standing links, db/054's doubts about them, and the links it cannot verify. Both empty for a single
/// chart (nothing joins it); an error, never an empty list, when a linked set cannot be read —
/// see [`links_section`]. Takes `state.db`'s lock, which means the caller must not already hold
/// it: `tokio::sync::Mutex` is not re-entrant.
///
/// All three reads run in ONE read-only REPEATABLE READ transaction, so they describe the same moment.
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
    let unverified = cairn_node::patient::edges::record_unverified_links(&snapshot, charts)
        .await
        .map_err(diagnose)?;
    // Read-only: dropping would roll back just as well; committing says the read is finished.
    snapshot
        .commit()
        .await
        .map_err(|e| format!("could not finish the read of the links: {e}"))?;
    Ok(RecordLinks {
        edges,
        doubts,
        unverified,
    })
}

/// The list, or — when it could not be read — no lines and a sentence saying so. An unread
/// list must never render as an empty one: that reads as "nothing joins these charts".
///
/// The lines: one per standing link, carrying [`CLASH_NOTE`] when db/054 doubts it, else the
/// unverified note when it cannot verify it (a clash line already says "in doubt"); then one per
/// bridged separation. A link nothing doubts or notes reads exactly as it did before R6.
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
                } else {
                    let systems = unverified_systems(e, &links.unverified);
                    if !systems.is_empty() {
                        line.text.push_str(&unverified_note(&systems));
                    }
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
#[path = "record_links_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "record_links_unverified_tests.rs"]
mod unverified_tests;
