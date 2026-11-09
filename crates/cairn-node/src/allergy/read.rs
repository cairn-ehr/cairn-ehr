//! Every allergy row of a patient's linked charts, for the strip view (allergy slices 1 and 2).
//!
//! Read from the `clinical_allergy` / `clinical_allergy_none` projections and the two overlay
//! projections (`clinical_allergy_challenge`, `clinical_allergy_retraction`, db/075), as notes are:
//! one statement per table over the whole member set, in the caller's snapshot — plus one count of
//! the allergy events `event_log` holds that have NO projected row (`unreadable`), so a body this
//! node cannot read is said, never mistaken for "not yet asked". Unlike notes and medications, a
//! doubted link does NOT withhold the other charts' rows: an allergy that might be a stranger's is
//! still shown (over-warning is the safe error), and `doubted_charts` names those charts so the
//! view can mark them. Vocabulary columns stay `String`: a peer's newer word must still read.
//!
//! Every row also carries its signer key's standing ([`KeyState`], db/004's
//! `cairn_key_confirmation`), so the view can say when a key is unconfirmed, doubtful, or was
//! repudiated after it signed. This module only reports rows; which overlay takes which entry off the list is the view's job.
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
    /// The entry this one corrects (its `correction-of` ref, [`corrects_of`]); None when fresh.
    pub corrects: Option<Uuid>,
    /// The signer key's standing now.
    pub key: KeyState,
    /// The drug the entry was coded to (`substance.coding`), when it was.
    pub coding: Option<Coding>,
}

/// The drug code an allergy entry carries: exactly the wire's `{system, code, display}`. Vocabulary
/// stays `String`: a peer's newer system word must still read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coding {
    pub system: String,
    pub code: String,
    pub display: String,
}

/// Read a projected `coding` column. Pure. SQL NULL, JSON null, a non-object, or an object missing
/// a key (or holding a non-string) all read as `None`: an entry this build cannot place in a drug
/// list is still a readable allergy, so a malformed coding must never fail the list.
pub fn coding_of(coding: Option<&str>) -> Option<Coding> {
    let v: serde_json::Value = serde_json::from_str(coding?).ok()?;
    let field = |k: &str| v.get(k)?.as_str().map(str::to_owned);
    Some(Coding {
        system: field("system")?,
        code: field("code")?,
        display: field("display")?,
    })
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
    pub key: KeyState,
}

/// One de-label by challenge as projected: "the patient tolerated `tolerated` on this date".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeRow {
    pub event_id: Uuid,
    pub chart: Uuid,
    /// The allergy entry it de-labels.
    pub target: Uuid,
    pub tolerated: String,
    /// The date's precision word and (unless `not-known`) its ISO value, as an entry's onset.
    pub date_precision: String,
    pub date_value: Option<String>,
    pub text: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub hlc: (i64, i32),
    pub signer: Option<String>,
    pub key: KeyState,
}

/// One entered-in-error withdrawal as projected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetractionRow {
    pub event_id: Uuid,
    pub chart: Uuid,
    /// The entry, "none known" statement or de-label it withdraws.
    pub target: Uuid,
    pub text: Option<String>,
    pub recorded_at: DateTime<Utc>,
    pub hlc: (i64, i32),
    pub signer: Option<String>,
    pub key: KeyState,
}

/// What this node now says about the key that signed a row, folded from db/004's six
/// `cairn_key_confirmation` words into the four the allergy view acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    /// `confirmed`, `outside` or `unknown`: nothing says the signature is in doubt.
    Standing,
    /// `unconfirmed`: the key's enrolment has not (yet) been confirmed. Its acts count, marked.
    Unconfirmed,
    /// `doubtful`: unconfirmed, AND a key that approved its enrolment has since been repudiated.
    /// Suspect, as a repudiated key is ([`KeyState::is_suspect`]); its byline reads as an
    /// unconfirmed one's (the note view's word).
    Doubtful,
    /// `repudiated`: the key was struck out (e.g. stolen) after it may have signed.
    Repudiated,
}

impl KeyState {
    /// A suspect key's act may only ADD a warning, never take one away (ruling R-4b): a
    /// repudiated key, or a doubtful one (one hop from a repudiation). Review I1.
    pub fn is_suspect(self) -> bool {
        matches!(self, KeyState::Repudiated | KeyState::Doubtful)
    }
}

/// Map one confirmation word to a [`KeyState`]. Pure. `None` (the door does not know) reads as
/// standing, as `note::vouch_read::signer_of` reads it. A word outside db/004's list is an ERROR
/// that fails the read: reading a word this build does not know as standing would fail open.
pub fn key_state(word: Option<&str>) -> anyhow::Result<KeyState> {
    use crate::note::vouch_read::CONFIRMATION_WORDS;
    let Some(w) = word else {
        return Ok(KeyState::Standing);
    };
    anyhow::ensure!(
        CONFIRMATION_WORDS.contains(&w),
        "a key's confirmation is {w:?}, which this build does not know"
    );
    Ok(match w {
        "unconfirmed" => KeyState::Unconfirmed,
        "doubtful" => KeyState::Doubtful,
        "repudiated" => KeyState::Repudiated,
        _ => KeyState::Standing,
    })
}

/// The entry an allergy entry corrects: the `event` of the first element of `refs` whose `rel` is
/// `correction-of` and whose `event` parses as a uuid. Pure. Anything else in `refs` — another
/// rel, a malformed element, a non-array — is ignored, never an error (plan D6): a peer's newer ref
/// shape must not stop the list from reading.
pub fn corrects_of(refs: &serde_json::Value) -> Option<Uuid> {
    refs.as_array()?.iter().find_map(|r| {
        if r.get("rel")?.as_str()? != "correction-of" {
            return None;
        }
        r.get("event")?.as_str()?.parse().ok()
    })
}

/// Everything the strip needs from the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatientAllergies {
    pub entries: Vec<AllergyRow>,
    pub none: Vec<NoneRow>,
    /// The member charts other than the opened one, when the set holds a doubted link; else empty.
    pub doubted_charts: Vec<Uuid>,
    /// The de-labels and withdrawals on the member charts, as projected; the view applies them.
    pub challenges: Vec<ChallengeRow>,
    pub retractions: Vec<RetractionRow>,
    /// Allergy-stream events (all four types) on the member charts that have NO projected row: this
    /// node holds the event but cannot read its sealed body (no custody yet), or it was shredded.
    /// Either way the honest statement is "cannot be read" — never "not yet asked"
    /// (UNREADABLE_SQL).
    pub unreadable: usize,
}

/// `$1` = the member charts (text uuids, compared as uuid so the patient index is used). Shared
/// WHERE/JOIN shape: projection joined to its event for the signer key; the name door is asked once per row (small lists, see note::read for the
/// measured big-list case).
const ENTRIES_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.allergen_kind, a.term, a.kind, \
        a.reaction_category, a.reaction_text, a.onset_precision, a.onset_value, a.source, \
        a.hlc_wall, a.hlc_counter, cairn_note_signer_name(e.signer_key_id), a.refs::text, \
        cairn_key_confirmation(e.signer_key_id), a.coding::text \
     FROM clinical_allergy a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id = ANY($1::text[]::uuid[]) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";
const NONE_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.scope, a.hlc_wall, a.hlc_counter, \
        cairn_note_signer_name(e.signer_key_id), cairn_key_confirmation(e.signer_key_id) \
     FROM clinical_allergy_none a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id = ANY($1::text[]::uuid[]) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";
const CHALLENGES_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.target::text, a.tolerated_term, \
        a.date_precision, a.date_value, a.text, a.hlc_wall, a.hlc_counter, \
        cairn_note_signer_name(e.signer_key_id), cairn_key_confirmation(e.signer_key_id) \
     FROM clinical_allergy_challenge a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id = ANY($1::text[]::uuid[]) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";
const RETRACTIONS_SQL: &str =
    "SELECT a.event_id::text, a.patient_id::text, a.target::text, a.text, a.hlc_wall, \
        a.hlc_counter, cairn_note_signer_name(e.signer_key_id), \
        cairn_key_confirmation(e.signer_key_id) \
     FROM clinical_allergy_retraction a JOIN event_log e ON e.event_id = a.event_id \
     WHERE a.patient_id = ANY($1::text[]::uuid[]) ORDER BY a.hlc_wall, a.hlc_counter, a.event_id";

/// The events of all four allergy types on the member charts with no row in their projection. An
/// applier projects every body it can read (db/074 step 7, db/075 step 8), so a missing row is a
/// body this node cannot read: no custody, or shredded (the shred scrubs the row, db/037). Read off
/// `event_log`, which the window's role already reads for the signer key above; compared as uuid so
/// the lookup rides `event_log_patient_idx`.
const UNREADABLE_SQL: &str = "SELECT count(*) FROM event_log e \
     WHERE e.patient_id = ANY($1::text[]::uuid[]) AND ( \
        (e.event_type = 'clinical.allergy.asserted' \
         AND NOT EXISTS (SELECT 1 FROM clinical_allergy a WHERE a.event_id = e.event_id)) \
     OR (e.event_type = 'clinical.allergy-none.asserted' \
         AND NOT EXISTS (SELECT 1 FROM clinical_allergy_none a WHERE a.event_id = e.event_id)) \
     OR (e.event_type = 'clinical.allergy-challenge.asserted' \
         AND NOT EXISTS (SELECT 1 FROM clinical_allergy_challenge a WHERE a.event_id = e.event_id)) \
     OR (e.event_type = 'clinical.allergy-retraction.asserted' \
         AND NOT EXISTS (SELECT 1 FROM clinical_allergy_retraction a WHERE a.event_id = e.event_id)))";

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

    // `try_get`, not `get`: the projection's columns are nullable (a peer's newer version projects
    // under /1's keys), and a NULL must fail this read honestly ("Could not read the allergy
    // list") rather than panic the command.
    let mut entries = Vec::new();
    for r in client.query(ENTRIES_SQL, &[&members]).await? {
        let hlc = (r.try_get::<_, i64>(10)?, r.try_get::<_, i32>(11)?);
        entries.push(AllergyRow {
            event_id: r.try_get::<_, String>(0)?.parse()?,
            chart: r.try_get::<_, String>(1)?.parse()?,
            allergen_kind: r.try_get(2)?,
            term: r.try_get(3)?,
            kind: r.try_get(4)?,
            category: r.try_get(5)?,
            reaction_text: r.try_get(6)?,
            onset_precision: r.try_get(7)?,
            onset_value: r.try_get(8)?,
            source: r.try_get(9)?,
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.try_get(12)?,
            corrects: serde_json::from_str(&r.try_get::<_, String>(13)?)
                .ok()
                .and_then(|v| corrects_of(&v)),
            key: key_state(r.try_get(14)?)?,
            coding: coding_of(r.try_get::<_, Option<String>>(15)?.as_deref()),
        });
    }
    let mut none = Vec::new();
    for r in client.query(NONE_SQL, &[&members]).await? {
        let hlc = (r.try_get::<_, i64>(3)?, r.try_get::<_, i32>(4)?);
        none.push(NoneRow {
            event_id: r.try_get::<_, String>(0)?.parse()?,
            chart: r.try_get::<_, String>(1)?.parse()?,
            scope: r.try_get(2)?,
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.try_get(5)?,
            key: key_state(r.try_get(6)?)?,
        });
    }
    let mut challenges = Vec::new();
    for r in client.query(CHALLENGES_SQL, &[&members]).await? {
        let hlc = (r.try_get::<_, i64>(7)?, r.try_get::<_, i32>(8)?);
        challenges.push(ChallengeRow {
            event_id: r.try_get::<_, String>(0)?.parse()?,
            chart: r.try_get::<_, String>(1)?.parse()?,
            target: r.try_get::<_, String>(2)?.parse()?,
            tolerated: r.try_get(3)?,
            date_precision: r.try_get(4)?,
            date_value: r.try_get(5)?,
            text: r.try_get(6)?,
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.try_get(9)?,
            key: key_state(r.try_get(10)?)?,
        });
    }
    let mut retractions = Vec::new();
    for r in client.query(RETRACTIONS_SQL, &[&members]).await? {
        let hlc = (r.try_get::<_, i64>(4)?, r.try_get::<_, i32>(5)?);
        retractions.push(RetractionRow {
            event_id: r.try_get::<_, String>(0)?.parse()?,
            chart: r.try_get::<_, String>(1)?.parse()?,
            target: r.try_get::<_, String>(2)?.parse()?,
            text: r.try_get(3)?,
            recorded_at: wall_time(hlc.0)?,
            hlc,
            signer: r.try_get(6)?,
            key: key_state(r.try_get(7)?)?,
        });
    }
    let unreadable: i64 = client.query_one(UNREADABLE_SQL, &[&members]).await?.get(0);
    Ok(PatientAllergies {
        entries,
        none,
        challenges,
        retractions,
        doubted_charts,
        unreadable: usize::try_from(unreadable).context("a negative count of unreadable rows")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_coding_missing_a_key_reads_as_none_never_a_fault() {
        let ok = r#"{"system":"drugref-moiety","code":"c","display":"d"}"#;
        assert_eq!(
            coding_of(Some(ok)),
            Some(Coding {
                system: "drugref-moiety".into(),
                code: "c".into(),
                display: "d".into()
            })
        );
        for bad in [
            None,
            Some("null"),
            Some("[]"),
            Some("not json"),
            Some(r#"{"system":"s","code":"c"}"#),
            Some(r#"{"system":"s","code":"c","display":3}"#),
        ] {
            assert_eq!(coding_of(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn every_confirmation_word_maps_and_an_unknown_word_fails() {
        use crate::note::vouch_read::CONFIRMATION_WORDS;
        // A literal table, one row per db/004 word: a wrong mapping must be changed here too.
        let table = [
            ("unknown", KeyState::Standing),
            ("repudiated", KeyState::Repudiated),
            ("outside", KeyState::Standing),
            ("confirmed", KeyState::Standing),
            ("doubtful", KeyState::Doubtful),
            ("unconfirmed", KeyState::Unconfirmed),
        ];
        assert_eq!(
            table.map(|(w, _)| w),
            CONFIRMATION_WORDS,
            "every word, once"
        );
        for (w, want) in table {
            assert_eq!(key_state(Some(w)).unwrap(), want, "{w}");
        }
        assert_eq!(key_state(None).unwrap(), KeyState::Standing);
        let e = key_state(Some("revoked")).unwrap_err().to_string();
        assert!(e.contains("revoked"), "{e}");
    }

    #[test]
    fn corrects_of_takes_the_first_parseable_correction_ref() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        assert_eq!(corrects_of(&json!([])), None);
        assert_eq!(
            corrects_of(&json!([{"rel": "correction-of", "event": a.to_string()}])),
            Some(a)
        );
        // Other rels, a non-uuid event, a non-object and a missing event are all skipped.
        let mixed = json!([
            {"rel": "see-also", "event": b.to_string()},
            {"rel": "correction-of", "event": "not-a-uuid"},
            "junk",
            {"rel": "correction-of"},
            {"rel": "correction-of", "event": a.to_string()},
            {"rel": "correction-of", "event": b.to_string()},
        ]);
        assert_eq!(corrects_of(&mixed), Some(a));
        assert_eq!(corrects_of(&json!({"rel": "correction-of"})), None);
        assert_eq!(corrects_of(&json!(null)), None);
    }
}
