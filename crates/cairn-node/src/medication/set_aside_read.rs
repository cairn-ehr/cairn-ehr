//! Batch charting slice 2a: read the set-aside versions back, to show under their companion line.
use anyhow::Context;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One set-aside version, ready to display under the line `alternative_to`.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAsideRow {
    pub version_id: Uuid,
    pub alternative_to: Uuid,
    /// The drug as one line (term, dose, formulation, sig).
    pub drug_line: String,
    /// Who reported it: the `source_detail` if given, else the `info_source` word.
    pub source: String,
    pub why: Option<String>,
    pub t_recorded: DateTime<Utc>,
}

/// The set-asides over a chart set (one query). Two guards against showing another chart's
/// version: it is filtered on its OWN `patient_id` AND joined to a companion on that same chart
/// (a remote version flagged for another chart keeps its own `patient_id` but points at the
/// other chart's thread). An orphan, whose companion is absent here, is not returned.
/// `version_id` is not unique at the remote door, so nothing here keys on it.
pub async fn read_set_asides(
    client: &(impl GenericClient + Sync),
    charts: &[Uuid],
) -> anyhow::Result<Vec<SetAsideRow>> {
    let charts: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    let rows = client
        .query(
            "SELECT a.version_id::text, a.alternative_to::text, a.term, a.dose_amount, a.dose_unit, \
                    a.formulation, a.sig, COALESCE(a.source_detail, a.info_source), a.why, a.t_recorded \
               FROM medication_set_aside a \
               JOIN medication_statement m \
                 ON m.medication_id = a.alternative_to AND m.patient_id = a.patient_id \
              WHERE a.patient_id::text = ANY($1) \
              ORDER BY a.t_recorded, a.event_id",
            &[&charts],
        )
        .await
        .context("reading the set-aside versions")?;
    rows.iter()
        .map(|r| {
            let (term, amt, unit, form, sig): (
                String,
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = (
                r.try_get(2)?,
                r.try_get(3)?,
                r.try_get(4)?,
                r.try_get(5)?,
                r.try_get(6)?,
            );
            Ok(SetAsideRow {
                version_id: r.try_get::<_, String>(0)?.parse()?,
                alternative_to: r.try_get::<_, String>(1)?.parse()?,
                drug_line: drug_line(&term, amt, unit, form, sig),
                source: r.try_get(7)?,
                why: r.try_get(8)?,
                t_recorded: r.try_get(9)?,
            })
        })
        .collect()
}

/// `term amount unit formulation — sig`, each part only when present.
fn drug_line(
    term: &str,
    amt: Option<String>,
    unit: Option<String>,
    form: Option<String>,
    sig: Option<String>,
) -> String {
    let mut s = term.to_string();
    for p in [amt, unit, form].into_iter().flatten() {
        s.push(' ');
        s.push_str(&p);
    }
    if let Some(sig) = sig {
        s.push_str(" — ");
        s.push_str(&sig);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::drug_line;

    #[test]
    fn the_drug_line_names_only_what_is_known() {
        let s = |x: &str| Some(x.to_string());
        assert_eq!(
            drug_line("Lipitor", s("25"), s("mg"), None, s("BD")),
            "Lipitor 25 mg — BD"
        );
        assert_eq!(drug_line("Lipitor", None, None, None, None), "Lipitor");
    }
}
