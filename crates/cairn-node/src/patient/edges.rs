//! The links that join a record's charts (repair path R2b-2, ADR-0076 decision 4).
//!
//! Paper counterpart: the paper clips holding two folders together — one per pair the clerk
//! clipped. A combined record is joined by LINKS, not by members: in A–C–B the wrong clip may
//! be A–C or C–B, and only a human can say which (principle 2). So the window lists every
//! standing link, each with its own "Not the same person…", and this is the read behind it.
//!
//! Reads `patient_link` (db/018): one row per pair ever asserted, holding the STANDING
//! assertion. Only `state = 'link'` rows join anything; an `unlink` row is a pair a judgement
//! keeps apart. Both ends are required to be in the set — in a link component that is true of
//! every standing link touching it, and the double test keeps a stale set from listing a link
//! half outside it.
//!
//! `record_doubts` reads which of a record's pairs db/054 doubts — the marks beside the links, R6.
use crate::db_diagnosis::LocalDbFault;
use cairn_medication_view::ChartSet;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One standing link between two charts of a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordEdge {
    /// The pair, canonical (`low < high`) as db/018 stores it.
    pub low: Uuid,
    /// The other end of the pair (`high`).
    pub high: Uuid,
    /// Whether the standing assertion is a human's vouched judgement (db/018's ONE definition,
    /// stored as `patient_link.attested`) — never re-derived here.
    pub attested: bool,
    /// The day the standing assertion was recorded (its HLC wall clock, UTC, `YYYY-MM-DD`).
    pub recorded_on: String,
}

/// `hlc_wall` is epoch MILLISECONDS (the same `/ 1000.0` db/031 uses), hence the division.
const EDGES_SQL: &str = "SELECT low::text AS low, high::text AS high, attested, \
     to_char(to_timestamp(hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS recorded_on \
     FROM patient_link \
     WHERE state = 'link' AND low = ANY($1::text[]::uuid[]) AND high = ANY($1::text[]::uuid[]) \
     ORDER BY low, high";

/// Every standing link whose two charts are both in `charts`, ordered by pair.
///
/// A single-chart set yields nothing (a link needs two ends). A database fault is reported
/// as a [`LocalDbFault`] naming the step, so an operator can tell which read failed.
pub async fn record_edges<C: GenericClient + Sync>(
    client: &C,
    charts: &ChartSet,
) -> anyhow::Result<Vec<RecordEdge>> {
    let ids: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
    let rows = client
        .query(EDGES_SQL, &[&ids])
        .await
        .map_err(|e| LocalDbFault::new("reading the links that join this record's charts", e))?;
    rows.iter()
        .map(|r| {
            Ok(RecordEdge {
                low: r.get::<_, String>("low").parse()?,
                high: r.get::<_, String>("high").parse()?,
                attested: r.get("attested"),
                recorded_on: r.get("recorded_on"),
            })
        })
        .collect()
}

/// Why db/054 doubts a pair of a record's charts (`cairn_chart_set_doubts`'s `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoubtKind {
    /// A standing link joins the pair without a clinician's confirmation on record here, and
    /// the node's hard identity check finds a clash between them (db/054 cases (a) and (b)).
    /// A clinician may confirm it or unlink it.
    Clash,
    /// A clinician recorded that the pair are DIFFERENT people, and other links still join them
    /// (case (c), the A–C–X bridge). Not a standing link: it is the attested unlink row.
    Separated,
}

/// One doubted pair of a record's charts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkDoubt {
    /// The pair, canonical (`low < high`).
    pub low: Uuid,
    pub high: Uuid,
    pub kind: DoubtKind,
    /// The day the pair's standing assertion was recorded (UTC, `YYYY-MM-DD`).
    pub recorded_on: String,
}

const DOUBTS_SQL: &str = "SELECT low::text AS low, high::text AS high, kind, \
     to_char(to_timestamp(hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS recorded_on \
     FROM cairn_chart_set_doubts($1::text[]::uuid[]) ORDER BY low, high";

/// Every pair of `charts` db/054 doubts, ordered by pair. The ONE Rust read of the doubt rule:
/// it asks db/054 and never re-derives the flag, veto or attestation tests.
pub async fn record_doubts<C: GenericClient + Sync>(
    client: &C,
    charts: &ChartSet,
) -> anyhow::Result<Vec<LinkDoubt>> {
    let ids: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
    let rows = client
        .query(DOUBTS_SQL, &[&ids])
        .await
        .map_err(|e| LocalDbFault::new("reading which of this record's links are in doubt", e))?;
    rows.iter()
        .map(|r| {
            let kind = match r.get::<_, String>("kind").as_str() {
                "clash" => DoubtKind::Clash,
                "separated" => DoubtKind::Separated,
                other => anyhow::bail!("db/054 returned an unknown doubt kind {other:?}"),
            };
            Ok(LinkDoubt {
                low: r.get::<_, String>("low").parse()?,
                high: r.get::<_, String>("high").parse()?,
                kind,
                recorded_on: r.get("recorded_on"),
            })
        })
        .collect()
}
