//! Batch charting slice 2b: read the settling of set-aside versions back (db/080).
//!
//! - [`read_settlements`]: every check and restore over a chart set, for the lines under a
//!   charted drug ("checked by …", "restored by …", "replaced: …").
//! - [`version_for_restore`]: one version as the restore press and Sign need it, including what
//!   currently stands for its companion, read through db/080's lineage function (ruling R4), so
//!   what Sign stops and what the floor demands at COMMIT come from ONE source.
//!
//! Like `set_aside_read::read_set_asides`, a row is returned only for a version on its OWN chart
//! whose companion is present on that same chart: an orphan, or a peer's row pointing at another
//! chart, is never shown. `version_id` is not unique at the remote door, so nothing keys on it
//! alone.
use anyhow::Context;
use cairn_event::medication::{drug_line_twin, Charting, MedicationAssertion};
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// How a version was settled.
#[derive(Debug, Clone, PartialEq)]
pub enum Settlement {
    /// "Checked: not current": `source` is the check's `source_detail`, else its word.
    Checked { source: String, why: Option<String> },
    /// Restored as the new thread `line`, replacing the companion spelled as `replaced_line`.
    Restored { line: Uuid, replaced_line: String },
}

/// One settlement of the version `version_id` on chart `patient_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementRow {
    pub version_id: Uuid,
    pub patient_id: Uuid,
    pub settlement: Settlement,
    /// The person who signed it (see `WHO`); `None` = not known ("signer not known").
    pub who: Option<String>,
    pub at: DateTime<Utc>,
}

/// Who signed event `e`, as a person's name. The key is the VOUCHED attester if there is one
/// (db/065's door, never an unverified stored key), else the envelope signer — which the door
/// verified, and which is the human under ADR-0053 authorship: the note read's `k`. Named only
/// when the registry enrolled that key for a HUMAN (db/067): a device or agent key is never read
/// as the person who checked or restored.
const WHO: &str = "CASE WHEN cairn_note_key_is_human(COALESCE(cairn_note_vouched_attester(e.event_id), e.signer_key_id)) \
     THEN cairn_note_signer_name(COALESCE(cairn_note_vouched_attester(e.event_id), e.signer_key_id)) END";

/// The version's own row must sit on its chart beside a companion present on that chart.
const SHOWN: &str = "EXISTS (SELECT 1 FROM medication_set_aside s \
       JOIN medication_statement m ON m.medication_id = s.alternative_to AND m.patient_id = s.patient_id \
      WHERE s.version_id = x.version_id AND s.patient_id = x.patient_id)";

/// A projected drug line spelled as its twin spells it (`drug_line_twin`), never a second copy
/// of its rules. Columns from `first`: term, dose_amount, dose_unit, formulation, sig, route,
/// strength, coding system/code/display, info_source.
fn drug_line(r: &tokio_postgres::Row, first: usize) -> anyhow::Result<String> {
    let text = |i: usize| r.try_get::<_, Option<String>>(first + i);
    let term: String = r.try_get(first)?;
    let info_source: String = r.try_get(first + 10)?;
    let (amt, unit, form, sig) = (text(1)?, text(2)?, text(3)?, text(4)?);
    let (route, strength) = (text(5)?, text(6)?);
    let (csys, ccode, cdisp) = (text(7)?, text(8)?, text(9)?);
    let coding = match (&csys, &ccode, &cdisp) {
        (Some(system), Some(code), Some(display)) => {
            Some(cairn_event::medication::SubstanceCoding {
                system,
                code,
                display,
            })
        }
        _ => None,
    };
    Ok(drug_line_twin(&MedicationAssertion {
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
    }))
}

/// Every check and restore over `charts`, oldest first (checks, then restores, merged by time).
/// Restores come from `medication_restore` (ruling R5); `replaced_line` is the version's
/// companion rebuilt from `medication_statement` + `medication_charting`, as `read_set_asides`
/// rebuilds a version. The companion's coding (`medication_coding`) is not joined here, so it is
/// spelled without one.
pub async fn read_settlements(
    client: &(impl GenericClient + Sync),
    charts: &[Uuid],
) -> anyhow::Result<Vec<SettlementRow>> {
    let charts: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    let checks = client
        .query(
            &format!(
                "SELECT x.version_id::text, x.patient_id::text, {WHO}, x.t_recorded, \
                        COALESCE(x.source_detail, x.info_source), x.why \
                   FROM medication_set_aside_checked x JOIN event_log e ON e.event_id = x.event_id \
                  WHERE x.patient_id::text = ANY($1) AND {SHOWN}"
            ),
            &[&charts],
        )
        .await
        .context("reading the set-aside versions' checks")?;
    let restores = client
        .query(
            &format!(
                "SELECT x.version_id::text, x.patient_id::text, {WHO}, \
                        to_timestamp(e.hlc_wall / 1000.0), x.medication_id::text, \
                        m.term, m.dose_amount, m.dose_unit, m.formulation, m.sig, ch.route, ch.strength, \
                        NULL::text, NULL::text, NULL::text, m.info_source \
                   FROM medication_restore x JOIN event_log e ON e.event_id = x.event_id \
                   CROSS JOIN LATERAL (SELECT s.alternative_to FROM medication_set_aside s \
                                        WHERE s.version_id = x.version_id AND s.patient_id = x.patient_id \
                                        ORDER BY s.t_recorded, s.event_id LIMIT 1) s \
                   JOIN medication_statement m \
                     ON m.medication_id = s.alternative_to AND m.patient_id = x.patient_id \
                   LEFT JOIN medication_charting ch ON ch.medication_id = m.medication_id \
                  WHERE x.patient_id::text = ANY($1)"
            ),
            &[&charts],
        )
        .await
        .context("reading the set-aside versions' restores")?;
    let head =
        |r: &tokio_postgres::Row| -> anyhow::Result<(Uuid, Uuid, Option<String>, DateTime<Utc>)> {
            Ok((
                r.try_get::<_, String>(0)?.parse()?,
                r.try_get::<_, String>(1)?.parse()?,
                r.try_get(2)?,
                r.try_get(3)?,
            ))
        };
    let mut out = Vec::with_capacity(checks.len() + restores.len());
    for r in &checks {
        let (version_id, patient_id, who, at) = head(r)?;
        let settlement = Settlement::Checked {
            source: r.try_get(4)?,
            why: r.try_get(5)?,
        };
        out.push(SettlementRow {
            version_id,
            patient_id,
            settlement,
            who,
            at,
        });
    }
    for r in &restores {
        let (version_id, patient_id, who, at) = head(r)?;
        let settlement = Settlement::Restored {
            line: r.try_get::<_, String>(4)?.parse()?,
            replaced_line: drug_line(r, 5)?,
        };
        out.push(SettlementRow {
            version_id,
            patient_id,
            settlement,
            who,
            at,
        });
    }
    out.sort_by_key(|s| s.at); // stable: equal times keep checks before restores
    Ok(out)
}

/// One version, as the restore press pre-fills its line and as Sign builds its stops.
#[derive(Debug, Clone, PartialEq)]
pub struct VersionForRestore {
    /// The companion: the thread the version was set aside beside.
    pub alternative_to: Uuid,
    /// The companion is in the record with no stop.
    pub companion_current: bool,
    /// The CURRENT lines of the companion's rooted restore lineage (db/080, rulings R4 + R10):
    /// what a restore must stop — any current line in the tree under the companion's original
    /// line, so the companion itself, a restore that replaced it, or a sibling restore of the
    /// original.
    pub standing: Vec<Uuid>,
    /// The members of `standing` that are themselves restore lines (a `medication_restore` row):
    /// a restore replacing one of these replaces a restore, not the original line.
    pub standing_restored: Vec<Uuid>,
    /// The companion's `charted_as` word; `None` = charted before slice 1a.
    pub companion_charted_as: Option<String>,
    /// Who reported the version: its `source_detail`, else its `info_source` word.
    pub source: String,
    pub term: String,
    /// `(system, code, display)`.
    pub coding: Option<(String, String, String)>,
    pub formulation: Option<String>,
    pub dose_amount: Option<String>,
    pub dose_unit: Option<String>,
    pub sig: Option<String>,
    pub route: Option<String>,
    pub strength: Option<String>,
    /// The version was restored already (a `medication_restore` row names it): the pane offers
    /// neither settle act on it, so a press from a stale pane is refused.
    pub restored: bool,
}

/// One version of `patient`, for the restore press and for Sign. `None`: not on this chart, its
/// companion is not on it, or it is gone. `standing` is read through
/// `cairn_medication_restore_lineage`, the function the floor's COMMIT check reads, once: its
/// current members, and those of them that are restore lines (`standing_restored`).
pub async fn version_for_restore(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    version: Uuid,
) -> anyhow::Result<Option<VersionForRestore>> {
    let row = client
        .query_opt(
            "SELECT a.alternative_to::text, \
                    NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = a.alternative_to), \
                    st.standing, \
                    ch.charted_as, COALESCE(a.source_detail, a.info_source), a.term, \
                    a.coding_system, a.coding_code, a.coding_display, a.formulation, a.dose_amount, \
                    a.dose_unit, a.sig, a.route, a.strength, st.restored, \
                    EXISTS (SELECT 1 FROM medication_restore r \
                             WHERE r.version_id = a.version_id AND r.patient_id = a.patient_id) \
               FROM medication_set_aside a \
               JOIN medication_statement m \
                 ON m.medication_id = a.alternative_to AND m.patient_id = a.patient_id \
               LEFT JOIN medication_charting ch ON ch.medication_id = a.alternative_to \
               CROSS JOIN LATERAL ( \
                    SELECT COALESCE(array_agg(l::text ORDER BY l), '{}') AS standing, \
                           COALESCE(array_agg(l::text ORDER BY l) FILTER (WHERE EXISTS \
                               (SELECT 1 FROM medication_restore r WHERE r.medication_id = l)), '{}') AS restored \
                      FROM cairn_medication_restore_lineage(a.alternative_to, a.patient_id) l \
                     WHERE EXISTS (SELECT 1 FROM medication_statement s WHERE s.medication_id = l) \
                       AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = l)) st \
              WHERE a.version_id = $2::text::uuid AND a.patient_id = $1::text::uuid \
              ORDER BY a.t_recorded, a.event_id LIMIT 1",
            &[&patient.to_string(), &version.to_string()],
        )
        .await
        .context("reading the set-aside version to restore")?;
    let Some(r) = row else { return Ok(None) };
    let text = |i: usize| r.try_get::<_, Option<String>>(i);
    let ids = |i: usize| -> anyhow::Result<Vec<Uuid>> {
        let v: Vec<String> = r.try_get(i)?;
        Ok(v.iter().map(|s| s.parse()).collect::<Result<_, _>>()?)
    };
    let coding = match (text(6)?, text(7)?, text(8)?) {
        (Some(s), Some(c), Some(d)) => Some((s, c, d)),
        _ => None,
    };
    Ok(Some(VersionForRestore {
        alternative_to: r.try_get::<_, String>(0)?.parse()?,
        companion_current: r.try_get(1)?,
        standing: ids(2)?,
        standing_restored: ids(15)?,
        companion_charted_as: text(3)?,
        source: r.try_get(4)?,
        term: r.try_get(5)?,
        coding,
        formulation: text(9)?,
        dose_amount: text(10)?,
        dose_unit: text(11)?,
        sig: text(12)?,
        route: text(13)?,
        strength: text(14)?,
        restored: r.try_get(16)?,
    }))
}
