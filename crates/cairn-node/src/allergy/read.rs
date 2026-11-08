//! Every allergy row of a patient's linked charts, for the strip view (allergy slice 1).
//!
//! Read ONLY from the `clinical_allergy` / `clinical_allergy_none` projections (as notes are), one
//! statement per table over the whole member set, in the caller's snapshot. Unlike notes and
//! medications, a doubted link does NOT withhold the other charts' rows: an allergy that might be a
//! stranger's is still shown (over-warning is the safe error), and `doubted_charts` names those
//! charts so the view can mark them. Vocabulary columns stay `String`: a peer's newer word must
//! still read.
use anyhow::Context;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One allergy entry as projected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllergyRow {
    pub event_id: Uuid,
    /// The chart it was written on (a linked chart's, when the read combines them).
    pub chart: Uuid,
    pub allergen_kind: String,
    pub term: String,
    pub kind: String,
    pub category: String,
    pub reaction_text: Option<String>,
    pub onset_precision: String,
    pub onset_value: Option<String>,
    pub source: String,
    /// The event's HLC wall as a time.
    pub recorded_at: DateTime<Utc>,
    pub hlc: (i64, i32),
    /// Who signed it (`cairn_note_signer_name`), when this node can name the key.
    pub signer: Option<String>,
}

/// One "none known" statement as projected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoneRow {
    pub event_id: Uuid,
    pub chart: Uuid,
    pub scope: String,
    pub recorded_at: DateTime<Utc>,
    pub hlc: (i64, i32),
    pub signer: Option<String>,
}

/// Everything the strip needs from the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientAllergies {
    pub entries: Vec<AllergyRow>,
    pub none: Vec<NoneRow>,
    /// The member charts other than the opened one, when the set holds a doubted link; else empty.
    pub doubted_charts: Vec<Uuid>,
}

/// `$1` = the member charts (text uuids). Shared WHERE/JOIN shape: projection joined to its event
/// for the signer key; the name door is asked once per row (small lists, see note::read for the
/// measured big-list case).
const ENTRIES_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.allergen_kind, a.term, a.kind, \
        a.reaction_category, a.reaction_text, a.onset_precision, a.onset_value, a.source, \
        a.hlc_wall, a.hlc_counter, cairn_note_signer_name(e.signer_key_id) \
     FROM clinical_allergy a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id::text = ANY($1) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";
const NONE_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.scope, a.hlc_wall, a.hlc_counter, \
        cairn_note_signer_name(e.signer_key_id) \
     FROM clinical_allergy_none a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id::text = ANY($1) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";

fn wall_time(ms: i64) -> anyhow::Result<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms).context("an allergy row's hlc_wall is out of range")
}

/// Every allergy row of every chart linked to `patient`, plus the doubted charts (see module doc).
pub async fn list_patient_allergies(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<PatientAllergies> {
    let set = crate::patient::person::person_charts(client, patient).await?;
    let members: Vec<String> = set.members().iter().map(|u| u.to_string()).collect();
    let doubted = crate::medication::read::set_has_doubted_link(client, &set).await?;
    let doubted_charts = if doubted {
        set.members()
            .iter()
            .copied()
            .filter(|c| *c != patient)
            .collect()
    } else {
        Vec::new()
    };

    let mut entries = Vec::new();
    for r in client.query(ENTRIES_SQL, &[&members]).await? {
        let hlc = (r.get::<_, i64>(10), r.get::<_, i32>(11));
        entries.push(AllergyRow {
            event_id: r.get::<_, String>(0).parse()?,
            chart: r.get::<_, String>(1).parse()?,
            allergen_kind: r.get(2),
            term: r.get(3),
            kind: r.get(4),
            category: r.get(5),
            reaction_text: r.get(6),
            onset_precision: r.get(7),
            onset_value: r.get(8),
            source: r.get(9),
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.get(12),
        });
    }
    let mut none = Vec::new();
    for r in client.query(NONE_SQL, &[&members]).await? {
        let hlc = (r.get::<_, i64>(3), r.get::<_, i32>(4));
        none.push(NoneRow {
            event_id: r.get::<_, String>(0).parse()?,
            chart: r.get::<_, String>(1).parse()?,
            scope: r.get(2),
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.get(5),
        });
    }
    Ok(PatientAllergies {
        entries,
        none,
        doubted_charts,
    })
}
