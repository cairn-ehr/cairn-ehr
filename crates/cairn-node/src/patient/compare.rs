//! The side-by-side comparison a human reads before linking two records (repair path R2b-1,
//! ADR-0076 decision 4; design "R2b — the window's gesture").
//!
//! Paper counterpart: the clerk fetches the other folder and lays the two FRONT SHEETS side by
//! side. This module reads what those front sheets carry, per member chart, and nothing is
//! merged: two duplicates differ by exactly the typo that made them, so a "winner" would erase
//! the evidence (the same rule as `person::ChartIdentity`).
//!
//! WHY EVERY NAME, NOT THE DISPLAY WINNER. `patient_name_current` picks one name per chart. A
//! maiden or preferred name is often precisely what tells a clerk two charts are one woman, so
//! the comparison reads every retained name with its `use`. Names a human REPUDIATED (§5.7) are
//! listed apart as `aliases` — "was recorded as" — never mixed in with the current ones.
//!
//! SHAPE: one flat `WHERE patient_id = ANY(...)` query per fact kind, joined in Rust — the
//! `person.rs` style, for the reason `chart_identities` gives (each projection has its own
//! absence convention, and an outer join per projection is harder to review). Every query goes
//! through [`rows`], which names the step on failure (#467: `db_errors_stay_legible.rs` guards
//! this file).
use crate::db_diagnosis::LocalDbFault;
use crate::patient::person::{read_held, read_trusts, trust_of};
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::collections::HashMap;
use tokio_postgres::{GenericClient, Row};
use uuid::Uuid;

/// One retained, non-repudiated name: the value exactly as authored, its `use` facet as the
/// author gave it (`None` when absent — not "legal"), and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameFact {
    pub value: String,
    pub use_: Option<String>,
    pub provenance: String,
}

/// A single-valued demographic field's standing value and where it came from — the
/// provenance is what tells a clerk "document-verified" from "patient-stated".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldFact {
    pub value: String,
    pub provenance: String,
    /// The `facets.precision` a dob assertion carries (principle 4): a year-precision
    /// "1950-01-01" must not read as a precise day, or it would look like a clash against
    /// another chart's genuine "1950-07-01". `None` when the field carries no precision
    /// facet at all — sex-at-birth's schema has none, so `None` there is not evidence of
    /// imprecision, only of the facet not applying.
    pub precision: Option<String>,
}

/// One identifier: its namespace, the value as entered, and its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierFact {
    pub system: String,
    pub value: String,
    pub provenance: String,
}

/// One current address (one per `use`, db/014's `patient_address_current`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressFact {
    pub use_: Option<String>,
    pub display: String,
    pub provenance: String,
}

/// Everything the comparison shows about ONE member chart. Every absence is `None` or empty —
/// never a placeholder: the window words it, and "not recorded" differs from "unknown — the
/// registration has not arrived" (`held`), a distinction only the reader can make.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChartFacts {
    pub patient_id: Uuid,
    /// A `patient_chart` row exists — the same meaning as `person::ChartIdentity::held`.
    pub held: bool,
    /// [`trust_of`]'s answer: a `chart_trust` state, `confirmed`, or `unknown` when not held.
    pub trust: String,
    /// Legal first, then newest first — the order a front sheet would list them.
    pub names: Vec<NameFact>,
    /// Repudiated names (`patient_alias_pool`), oldest first.
    pub aliases: Vec<String>,
    pub dob: Option<FieldFact>,
    pub sex_at_birth: Option<FieldFact>,
    pub identifiers: Vec<IdentifierFact>,
    pub addresses: Vec<AddressFact>,
}

// The name read repeats `patient_name_current`'s repudiation predicate (db/025) because it
// must return EVERY retained name, not that view's one winner. `a_repudiated_name_moves_to_
// the_aliases` pins that the two agree on what "repudiated" means.
const NAMES_SQL: &str =
    "SELECT n.patient_id::text AS patient_id, n.value, n.use_raw, n.provenance \
     FROM patient_name n \
     WHERE n.patient_id = ANY($1::text[]::uuid[]) \
       AND NOT EXISTS (SELECT 1 FROM name_repudiation r \
                        WHERE r.subject = n.patient_id AND r.value = n.value) \
     ORDER BY n.patient_id, (n.use_key = 'legal') DESC, n.last_hlc_wall DESC, \
              n.last_hlc_count DESC, n.value COLLATE \"C\"";
const ALIASES_SQL: &str = "SELECT patient_id::text AS patient_id, value FROM patient_alias_pool \
     WHERE patient_id = ANY($1::text[]::uuid[]) \
     ORDER BY patient_id, hlc_wall, hlc_counter, value COLLATE \"C\"";
const FIELDS_SQL: &str = "SELECT patient_id::text AS patient_id, field, value, provenance, \
     facets ->> 'precision' AS precision \
     FROM patient_demographic \
     WHERE field IN ('dob', 'sex-at-birth') AND patient_id = ANY($1::text[]::uuid[])";
const IDENTIFIERS_SQL: &str = "SELECT patient_id::text AS patient_id, system, value, provenance \
     FROM patient_identifier WHERE patient_id = ANY($1::text[]::uuid[]) \
     ORDER BY patient_id, system COLLATE \"C\", value COLLATE \"C\"";
const ADDRESSES_SQL: &str = "SELECT patient_id::text AS patient_id, use_raw, display, provenance \
     FROM patient_address_current WHERE patient_id = ANY($1::text[]::uuid[]) \
     ORDER BY patient_id, use_key COLLATE \"C\"";

/// Run one per-set query, naming the step if it fails (the ONE `LocalDbFault` site for
/// [`chart_facts`]'s five reads).
async fn rows<C: GenericClient + Sync>(
    client: &C,
    sql: &str,
    ids: &[String],
    step: &str,
) -> anyhow::Result<Vec<Row>> {
    Ok(client
        .query(sql, &[&ids])
        .await
        .map_err(|e| LocalDbFault::new(step, e))?)
}

/// The row's `patient_id` column as a `Uuid`.
fn patient_of(row: &Row) -> anyhow::Result<Uuid> {
    Ok(row.get::<_, String>("patient_id").parse()?)
}

/// Read [`ChartFacts`] for every member of `set`, in the set's own order.
///
/// Generic over `GenericClient` so a caller may read inside a transaction. Errors on the first
/// failed read — the window then shows the comparison as incomplete and offers no Link button.
pub async fn chart_facts<C: GenericClient + Sync>(
    client: &C,
    set: &ChartSet,
) -> anyhow::Result<Vec<ChartFacts>> {
    let ids: Vec<String> = set.members().iter().map(Uuid::to_string).collect();
    let held = read_held(client, &ids)
        .await
        .context("reading which of the charts this node holds")?;
    let trusts = read_trusts(client, &ids)
        .await
        .context("reading the charts' identity states")?;
    let mut by_id: HashMap<Uuid, ChartFacts> = set
        .members()
        .iter()
        .map(|id| {
            let is_held = held.contains(id);
            let facts = ChartFacts {
                patient_id: *id,
                held: is_held,
                trust: trust_of(is_held, trusts.get(id).map(String::as_str)),
                ..ChartFacts::default()
            };
            (*id, facts)
        })
        .collect();
    // `get_mut` can only miss if a query returned a chart it was not asked about; skipping it
    // is then correct (it is not a member), so no `expect`.
    for row in rows(client, NAMES_SQL, &ids, "reading the charts' names").await? {
        if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
            f.names.push(NameFact {
                value: row.get("value"),
                use_: row.get("use_raw"),
                provenance: row.get("provenance"),
            });
        }
    }
    for row in rows(
        client,
        ALIASES_SQL,
        &ids,
        "reading the charts' earlier names",
    )
    .await?
    {
        if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
            f.aliases.push(row.get("value"));
        }
    }
    for row in rows(
        client,
        FIELDS_SQL,
        &ids,
        "reading the charts' dates of birth and sex",
    )
    .await?
    {
        if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
            let fact = FieldFact {
                value: row.get("value"),
                provenance: row.get("provenance"),
                precision: row.get("precision"),
            };
            match row.get::<_, String>("field").as_str() {
                "dob" => f.dob = Some(fact),
                _ => f.sex_at_birth = Some(fact), // the query admits only the two fields
            }
        }
    }
    for row in rows(
        client,
        IDENTIFIERS_SQL,
        &ids,
        "reading the charts' identifiers",
    )
    .await?
    {
        if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
            f.identifiers.push(IdentifierFact {
                system: row.get("system"),
                value: row.get("value"),
                provenance: row.get("provenance"),
            });
        }
    }
    for row in rows(client, ADDRESSES_SQL, &ids, "reading the charts' addresses").await? {
        if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
            f.addresses.push(AddressFact {
                use_: row.get("use_raw"),
                display: row.get("display"),
                provenance: row.get("provenance"),
            });
        }
    }
    // Every member was inserted above, so `remove` always finds it.
    Ok(set
        .members()
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect())
}

/// One `cairn_match_veto` finding between a chart of the LEFT record and a chart of the RIGHT
/// one, tagged with that pair so the window can say which two charts disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VetoFinding {
    pub left: Uuid,
    pub right: Uuid,
    /// `identifier`, `dob` or `sex-at-birth` (db/016's closed vocabulary).
    pub kind: String,
    /// `hard_veto` or `degrade_hold`.
    pub severity: String,
    pub subject: String,
    /// db/016's own human-readable reason — shown verbatim, never reworded.
    pub detail: String,
}

/// Hard vetoes first, then a stable order (left, right, kind), so the panel always reads the
/// same way for the same records. Pure.
pub fn order_findings(mut findings: Vec<VetoFinding>) -> Vec<VetoFinding> {
    findings.sort_by(|x, y| {
        (x.severity != "hard_veto", x.left, x.right, &x.kind).cmp(&(
            y.severity != "hard_veto",
            y.left,
            y.right,
            &y.kind,
        ))
    });
    findings
}

// Every left × right pair, in one statement. `l <> r` is belt and braces: two records are two
// link components, so they share no chart — but a caller that passed overlapping sets must not
// be told a chart "clashes" with itself.
const VETO_SQL: &str = "SELECT l::text AS l, r::text AS r, v.veto_kind, v.severity, \
            coalesce(v.subject, '') AS subject, coalesce(v.detail, '') AS detail \
     FROM unnest($1::text[]::uuid[]) AS l \
     CROSS JOIN unnest($2::text[]::uuid[]) AS r \
     CROSS JOIN LATERAL cairn_match_veto(l, r) AS v \
     WHERE l <> r";

/// Every veto finding between a chart of `left` and a chart of `right`.
///
/// SET AGAINST SET, never chart against chart (design "R2b"): with A open and already linked to
/// C, linking B to A also joins B to C — so a B–C clash is exactly as relevant as an A–B one.
/// A hard veto forces a human decision and never an automatic refusal (§5.13): this only
/// REPORTS; the human may still link.
pub async fn cross_vetoes<C: GenericClient + Sync>(
    client: &C,
    left: &ChartSet,
    right: &ChartSet,
) -> anyhow::Result<Vec<VetoFinding>> {
    let l: Vec<String> = left.members().iter().map(Uuid::to_string).collect();
    let r: Vec<String> = right.members().iter().map(Uuid::to_string).collect();
    let found = client
        .query(VETO_SQL, &[&l, &r])
        .await
        .map_err(|e| LocalDbFault::new("checking the two records for veto findings", e))?;
    let findings = found
        .iter()
        .map(|row| {
            Ok(VetoFinding {
                left: row.get::<_, String>("l").parse()?,
                right: row.get::<_, String>("r").parse()?,
                kind: row.get("veto_kind"),
                severity: row.get("severity"),
                subject: row.get("subject"),
                detail: row.get("detail"),
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(order_findings(findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: &str, l: u128, r: u128, kind: &str) -> VetoFinding {
        VetoFinding {
            left: Uuid::from_u128(l),
            right: Uuid::from_u128(r),
            kind: kind.into(),
            severity: severity.into(),
            subject: kind.into(),
            detail: String::new(),
        }
    }

    /// A hard veto is the fact a clerk most needs, so it is read FIRST — before any
    /// degrade-hold — whatever order the database returned them in.
    #[test]
    fn hard_vetoes_come_first_then_a_stable_order() {
        let ordered = order_findings(vec![
            finding("degrade_hold", 1, 2, "identifier"),
            finding("hard_veto", 3, 4, "dob"),
            finding("hard_veto", 1, 2, "sex-at-birth"),
        ]);
        let got: Vec<(&str, u128)> = ordered
            .iter()
            .map(|f| (f.severity.as_str(), f.left.as_u128()))
            .collect();
        assert_eq!(
            got,
            vec![("hard_veto", 1), ("hard_veto", 3), ("degrade_hold", 1)]
        );
    }
}
