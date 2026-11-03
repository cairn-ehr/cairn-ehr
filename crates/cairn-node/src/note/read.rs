//! A chart's notes, as the window lists them.
//!
//! Linked charts read as one, as the medication list does (ADR-0076): the notes of every chart
//! `person_charts` says is this person. One exception, the same as the medication list's
//! (#697 (b)): while the set holds a link this node DOUBTS, a note on a chart other than the
//! opened one may be a stranger's, so it is counted (`withheld_on_linked`) and left out.
//!
//! Notes are read ONLY from the `clinical_note` projection, never from the event log's clear
//! view (ruling R8): the projection is what keeps a newer, unchecked note profile carried by a
//! peer off the screen. `event_log` is joined only for what grades authorship.
use cairn_event::contributor::{classify_authorship_confidence, VerifiedKid};
use cairn_event::note::unescape_note_markup;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use super::draft::{open_drafts, NoteDraft};
use super::view::{note_grade, NoteGrade};

/// One signed note, ready for the window to word (`view::time_words`, `view::grade_words`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRow {
    pub event_id: Uuid,
    /// The chart the note was written on (a linked chart's, when the read combines them).
    pub chart: Uuid,
    /// The text as the writer typed it (the signed narrative, unescaped).
    pub text: String,
    pub t_effective: DateTime<Utc>,
    /// `"stated"` or `"recording-started"`.
    pub effective_basis: String,
    pub recording_started: DateTime<Utc>,
    pub grade: NoteGrade,
}

/// What the note pane shows for one opened chart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientNoteList {
    /// Newest `t_effective` first.
    pub notes: Vec<NoteRow>,
    /// Notes on other charts of a set holding a doubted link: counted, not shown.
    pub withheld_on_linked: usize,
    /// The opened chart's open drafts.
    pub drafts: Vec<NoteDraft>,
}

/// The notes of every chart in `charts`, with what grades their authorship.
///
/// The attester is read only when `cairn_attestation_vouched` clears it: db/020 can store a
/// peer's attester key UNVERIFIED, and minting a `VerifiedKid` from such a row would re-open
/// #412 (see `VerifiedKid::from_event_log_column`). `signer_key_id` needs no such check — no
/// row exists unless the door verified the signature against it.
const NOTES_SQL: &str = "\
    SELECT n.event_id::text, n.patient_id::text, n.narrative, n.t_effective, n.effective_basis, \
           n.recording_started, e.signer_key_id, e.contributors::text, \
           CASE WHEN e.attester_key IS NOT NULL AND cairn_attestation_vouched(e.event_id) \
                THEN encode(e.attester_key, 'hex') END, \
           cairn_note_signer_name(e.signer_key_id) \
      FROM clinical_note n JOIN event_log e ON e.event_id = n.event_id \
     WHERE n.patient_id = ANY($1::text[]::uuid[])";

/// Read the notes of the person `patient` is a chart of, plus `patient`'s open drafts.
pub async fn list_patient_notes(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<PatientNoteList> {
    let charts = crate::patient::person::person_charts(client, patient).await?;
    let doubted = crate::medication::read::set_has_doubted_link(client, &charts).await?;
    let members: Vec<String> = charts.members().iter().map(Uuid::to_string).collect();
    let mut notes = Vec::new();
    let mut withheld_on_linked = 0;
    for r in client.query(NOTES_SQL, &[&members]).await? {
        let chart: Uuid = r.get::<_, String>(1).parse()?;
        if doubted && chart != patient {
            withheld_on_linked += 1;
            continue;
        }
        let signer: String = r.get(6);
        let contributors: serde_json::Value = serde_json::from_str(&r.get::<_, String>(7))?;
        let attester: Option<String> = r.get(8);
        // Both kids come from event_log columns whose existence is a verification: the signer
        // always, the attester only through the vouch check in NOTES_SQL.
        let confidence = classify_authorship_confidence(
            &contributors,
            VerifiedKid::from_event_log_column(&signer),
            attester.as_deref().map(VerifiedKid::from_event_log_column),
        );
        notes.push(NoteRow {
            event_id: r.get::<_, String>(0).parse()?,
            chart,
            text: unescape_note_markup(&r.get::<_, String>(2)),
            t_effective: r.get(3),
            effective_basis: r.get(4),
            recording_started: r.get(5),
            grade: note_grade(confidence, r.get(9)),
        });
    }
    // Ordered here rather than by SQL ORDER BY (ADR-0045); the id breaks a tie stably.
    notes.sort_by_key(|n| std::cmp::Reverse((n.t_effective, n.event_id)));
    Ok(PatientNoteList {
        notes,
        withheld_on_linked,
        drafts: open_drafts(client, patient).await?,
    })
}
