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
    /// The note this one continues (its `addendum-to` ref): a later writer's part of one draft.
    pub addendum_to: Option<Uuid>,
    /// The display name of the note's `drafted` contributor — who typed a part the signer
    /// attested — when this node can name that key (`cairn_note_signer_name`); `None` otherwise.
    pub drafted_by: Option<String>,
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
/// The attester comes through db/065's `cairn_note_vouched_attester` door, which answers only
/// for an attester `cairn_attestation_vouched` clears: db/020 can store a peer's attester key
/// UNVERIFIED, and minting a `VerifiedKid` from such a row would re-open #412 (see
/// `VerifiedKid::from_event_log_column`). A door, because the window role may not call the
/// predicate itself (ruling R11). `signer_key_id` needs no such check — no row exists unless
/// the door verified the signature against it.
const NOTES_SQL: &str = "\
    SELECT n.event_id::text, n.patient_id::text, n.narrative, n.t_effective, n.effective_basis, \
           n.recording_started, e.signer_key_id, e.contributors::text, \
           cairn_note_vouched_attester(e.event_id), \
           cairn_note_signer_name(e.signer_key_id), \
           (SELECT r ->> 'event' FROM jsonb_array_elements(n.refs) r \
             WHERE r ->> 'rel' = 'addendum-to' LIMIT 1), \
           cairn_note_signer_name((SELECT c ->> 'actor_id' FROM jsonb_array_elements(e.contributors) c \
                                    WHERE c ->> 'role' = 'drafted' LIMIT 1)) \
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
        // always, the attester only through the vouch door in NOTES_SQL.
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
            addendum_to: r
                .get::<_, Option<String>>(10)
                .map(|s| s.parse())
                .transpose()?,
            drafted_by: r.get(11),
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

#[cfg(test)]
mod tests {
    use super::NOTES_SQL;

    /// #412 guard (PR #931 review): the note list reads the attester ONLY through the vouch
    /// door. Reading `event_log.attester_key` directly would mint a `VerifiedKid` from a peer's
    /// unverified token and grade a note "signed by" someone nobody vouched for. (A DB test
    /// cannot show this on the window's own notes: their human author is the signer, verified
    /// by the signature itself, so they carry no separate attester.)
    #[test]
    fn the_attester_is_read_only_through_the_vouch_door() {
        assert!(NOTES_SQL.contains("cairn_note_vouched_attester(e.event_id)"));
        assert!(
            !NOTES_SQL.contains("attester_key"),
            "the raw attester column must never reach the classifier: {NOTES_SQL}"
        );
    }
}
