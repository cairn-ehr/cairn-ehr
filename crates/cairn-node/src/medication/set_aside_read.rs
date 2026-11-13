//! Batch charting slice 2a: read the set-aside versions back, to show under their companion line.
use anyhow::Context;
use cairn_event::medication::{drug_line_twin, Charting, MedicationAssertion, SubstanceCoding};
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One set-aside version, ready to display under the line `alternative_to`.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAsideRow {
    pub version_id: Uuid,
    pub alternative_to: Uuid,
    /// The chart the version is on (its own `patient_id`): settlements join on it (slice 2b).
    pub patient_id: Uuid,
    /// The drug as `drug_line_twin` spells it (term, dose, formulation, strength, route, sig,
    /// `(info_source)`, coding display). It carries the info_source WORD in brackets, as the
    /// set-aside's own twin does; `source` below is the finer detail, shown beside it.
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
                    a.formulation, a.sig, a.route, a.strength, \
                    a.coding_system, a.coding_code, a.coding_display, a.info_source, \
                    COALESCE(a.source_detail, a.info_source), a.why, a.t_recorded, a.patient_id::text \
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
            let text = |i: usize| r.try_get::<_, Option<String>>(i);
            let (term, info_source) = (r.try_get::<_, String>(2)?, r.try_get::<_, String>(12)?);
            let (amt, unit, form, sig) = (text(3)?, text(4)?, text(5)?, text(6)?);
            let (route, strength) = (text(7)?, text(8)?);
            let (csys, ccode, cdisp) = (text(9)?, text(10)?, text(11)?);
            let coding = match (&csys, &ccode, &cdisp) {
                (Some(system), Some(code), Some(display)) => Some(SubstanceCoding {
                    system,
                    code,
                    display,
                }),
                _ => None,
            };
            // Rebuilt from the projected columns so the line is spelled by the ONE function the
            // set-aside's own twin uses (`drug_line_twin`), never a second copy of its rules.
            let drug = MedicationAssertion {
                medication_id: "",
                term: &term,
                coding,
                formulation: form.as_deref(),
                dose_amount: amt.as_deref(),
                dose_unit: unit.as_deref(),
                sig: sig.as_deref(),
                info_source: &info_source,
                started: None,
                started_precision: None,
                safety: None,
                charting: Some(Charting {
                    route: route.as_deref(),
                    strength: strength.as_deref(),
                    ..Charting::default()
                }),
                source_note: None,
            };
            Ok(SetAsideRow {
                version_id: r.try_get::<_, String>(0)?.parse()?,
                alternative_to: r.try_get::<_, String>(1)?.parse()?,
                drug_line: drug_line_twin(&drug),
                source: r.try_get(13)?,
                why: r.try_get(14)?,
                t_recorded: r.try_get(15)?,
                patient_id: r.try_get::<_, String>(16)?.parse()?,
            })
        })
        .collect()
}
