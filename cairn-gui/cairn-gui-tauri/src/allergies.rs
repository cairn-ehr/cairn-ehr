//! The allergy list's commands (allergy slice 1): read the strip and the expanded lists, record an
//! entry, record "no known (drug) allergies".
//!
//! Thin adapters over `cairn_node::allergy`, split as `commands.rs` is: each `#[tauri::command]`
//! forwards to a plain `*_impl(&AppState, …)`, so the rule a wrong-chart defect would break — act
//! only on the chart on SCREEN (`AppState::displayed_patient`), over the chart SET that list was
//! read across (`check_displayed_set`, ADR-0076 decision 3) — is tested against `AppState::mock`
//! with no Tauri runtime (`allergies_tests.rs`).
//!
//! Reading order for a junior: [`allergy_list_impl`] reads the rows and the chart set in one
//! snapshot and turns `cairn_node`'s pure `build_view` into an [`AllergyPane`] the webview can
//! read. The two writes run the same steps as `commands::cease_impl`, in the same order: the chart
//! on screen, the set, fixture mode, then every verdict on what was typed ([`entry_before_the_key`]
//! — a passphrase that writes nothing is a step paper never has), and only then the key; then one
//! signed event, its timing sample, and a fresh read for the pane ([`sign_then_reread`]), answered
//! as an [`AllergyWrite`]. A correction (an entry sent with `corrects`) and allergy slice 2's
//! de-label and withdrawal (`allergy_overlays.rs`) take one more step before the key: the line
//! they name must still be on a FRESH read of the list, and they are filed under its chart.
//!
//! No markup handling anywhere: the text the clinician typed is signed as typed (only surrounding
//! whitespace trimmed, no escaping) and the webview
//! draws it with `textContent` only.
use crate::allergy_overlays::{target_before_the_key, Names};
use crate::allergy_pane::{pane, AllergyPane};
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::commands::record_timing;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use crate::link::judgement_view::refused;
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{chart_read_failed, key_locked, node_cannot_write, node_failure};
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
};
use cairn_event::allergy_overlay::{AllergyChallenge, AllergyRetraction};
use cairn_medication_view::ChartSet;
use cairn_node::allergy::read::PatientAllergies;
use chrono::{Datelike, NaiveDate};
use std::time::Instant;
use uuid::Uuid;

/// One entry as the form sends it. Every word is a wire word (`cairn_event::allergy`'s
/// vocabularies); `kind` and `category` are `None` until the clinician presses one — the form
/// starts with neither pressed, so neither can be signed by default. `allergen_kind` and `source`
/// start pressed (Drug, Patient stated), but Correct on a newer peer's entry leaves a word this
/// window has no button for unpressed, so they can be `None` too, and are refused like the others.
#[derive(Debug, serde::Deserialize)]
pub struct EntryInput {
    pub allergen_kind: Option<String>,
    pub term: String,
    pub kind: Option<String>,
    pub category: Option<String>,
    pub reaction_text: Option<String>,
    /// As typed: `2019`, `03/2019`, `14/03/2019` (or ISO); empty or `None` = not known.
    pub onset: Option<String>,
    pub source: Option<String>,
    /// The line this entry corrects (its `event_id`, as the pane sent it); `None` for a new entry.
    pub corrects: Option<String>,
}

/// What a write answers once its event is signed: what happened, and the list read again when
/// that read worked. A write that signed nothing answers an `ErrorView` instead.
#[derive(Debug, serde::Serialize)]
pub struct AllergyWrite {
    pub outcome: String,
    pub pane: Option<AllergyPane>,
}

/// Read the allergy strip and lists of the chart on screen.
#[tauri::command]
pub async fn allergy_list(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<AllergyPane, ErrorView> {
    allergy_list_impl(&state, &patient_id).await
}

/// Record one allergy or adverse-effect entry; answers with the list read again.
#[tauri::command]
pub async fn allergy_record(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    entry: EntryInput,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    allergy_record_impl(&state, &patient_id, charts, entry, credential).await
}

/// Record "no known allergies" (`scope` `"all"`) or "no known drug allergies" (`"drug"`).
#[tauri::command]
pub async fn allergy_none(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    scope: String,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    allergy_none_impl(&state, &patient_id, charts, &scope, credential).await
}

/// The rows and the chart set of the chart on screen, in ONE snapshot (so the set sent back with
/// a write is the one these rows were read over), as the pane. Fixture mode has no allergies: it
/// reads as an empty list on the one chart, never a failure.
pub async fn allergy_list_impl(
    state: &AppState,
    patient_id: &str,
) -> Result<AllergyPane, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let (set, allergies) = read_list(state, patient).await?;
    Ok(pane(&allergies, &set))
}

/// The chart set `patient` belongs to and every member chart's allergy rows, read in ONE snapshot
/// (so the set and the rows agree). Fixture mode has no allergies: the chart alone, no rows.
pub(crate) async fn read_list(
    state: &AppState,
    patient: Uuid,
) -> Result<(ChartSet, PatientAllergies), ErrorView> {
    let Some(db) = state.db.as_ref() else {
        let empty = PatientAllergies {
            entries: vec![],
            none: vec![],
            challenges: vec![],
            retractions: vec![],
            doubted_charts: vec![],
            unreadable: 0,
        };
        return Ok((ChartSet::single(patient), empty));
    };
    let mut db = db.lock().await?;
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let set = cairn_node::patient::person::person_charts(tx, patient).await?;
        let allergies = cairn_node::allergy::read::list_patient_allergies(tx, patient).await?;
        Ok((set, allergies))
    })
    .await
    .map_err(|e| chart_read_failed(db.explain(e)))
}

/// Record one entry on the chart on screen (refusal order: module doc).
pub async fn allergy_record_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    entry: EntryInput,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let mut read = entry_before_the_key(&entry, local_today())?;
    // A correction names a live line of a fresh read and is filed under that line's chart (D2).
    let target = match &entry.corrects {
        None => None,
        Some(t) => Some(target_before_the_key(state, patient, &charts, t, Names::Entry).await?),
    };
    let id = target.map(|t| t.id.to_string());
    read.corrects = id.as_deref();
    let chart = target.map_or(patient, |t| t.chart);
    sign_then_reread(state, patient, chart, Act::Entry(read), credential).await
}

/// Record a "none known" statement on the chart on screen (refusal order: module doc).
pub async fn allergy_none_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    scope: &str,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let scope = word(NoneScope::parse, scope)?;
    sign_then_reread(state, patient, patient, Act::None(scope), credential).await
}

/// The first three refusals every write says, in order: not the chart on screen, not the chart
/// set the list was read over, fixture mode. Answers the chart.
pub(crate) async fn on_screen_and_writable(
    state: &AppState,
    patient_id: &str,
    charts: &[String],
) -> Result<Uuid, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let set = read_set(state, patient).await.map_err(chart_read_failed)?;
    check_displayed_set(&set, charts).map_err(ChartSetError::into_view)?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    Ok(patient)
}

/// The chart set `patient` belongs to now; fixture mode's is the chart alone.
async fn read_set(state: &AppState, patient: Uuid) -> anyhow::Result<ChartSet> {
    let Some(db) = state.db.as_ref() else {
        return Ok(ChartSet::single(patient));
    };
    let db = db.lock().await?;
    cairn_node::patient::person::person_charts(&*db, patient)
        .await
        .map_err(|e| db.explain(e))
}

/// The signed acts of the allergy list: an entry (new, or a correction when `corrects` is set),
/// a "none known" statement, a de-label and a withdrawal (`allergy_overlays.rs`).
pub(crate) enum Act<'a> {
    Entry(AllergyEntry<'a>),
    None(NoneScope),
    Challenge(AllergyChallenge<'a>),
    Retraction(AllergyRetraction<'a>),
}

impl Act<'_> {
    /// Its db/044 timing kind: a correction is timed as one, never as a new entry.
    fn timing_kind(&self) -> &'static str {
        match self {
            Act::Entry(e) if e.corrects.is_some() => "allergy-correct",
            Act::Entry(_) => "allergy-record",
            Act::None(_) => "allergy-none",
            Act::Challenge(_) => "allergy-challenge",
            Act::Retraction(_) => "allergy-retract",
        }
    }

    /// How a sign whose outcome is not known (#814) names the act.
    fn unknown_lead(&self) -> &'static str {
        match self {
            Act::Entry(e) if e.corrects.is_some() => "Whether this correction was recorded",
            Act::Entry(_) | Act::None(_) => "Whether this allergy was recorded",
            Act::Challenge(_) => "Whether this de-label was recorded",
            Act::Retraction(_) => "Whether this was marked entered in error",
        }
    }
}

/// Ask for the key, sign `act` filed under `chart` (the chart on screen, or for a correction,
/// de-label or withdrawal its target's own chart, D2), time it, and read the list of the chart
/// on screen (`patient`) again.
///
/// A failed sign is an `ErrorView` ([`sign_failure`]: nothing written, or NOT known). A sign
/// that landed is an Ok whatever the re-read did ([`written`]).
pub(crate) async fn sign_then_reread(
    state: &AppState,
    patient: Uuid,
    chart: Uuid,
    act: Act<'_>,
    credential: Option<Credential>,
) -> Result<AllergyWrite, ErrorView> {
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let author = cairn_node::medication::AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let started = Instant::now();
    let signed = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        use cairn_node::allergy::sign::*;
        let (sk, kid, origin) = (node_sk, node_kid.as_str(), state.node_origin.as_str());
        let result = match &act {
            Act::Entry(e) => sign_allergy(&mut db, sk, kid, origin, chart, e, &author).await,
            Act::None(s) => sign_allergy_none(&mut db, sk, kid, origin, chart, *s, &author).await,
            Act::Challenge(c) => {
                sign_allergy_challenge(&mut db, sk, kid, origin, chart, c, &author).await
            }
            Act::Retraction(r) => {
                sign_allergy_retraction(&mut db, sk, kid, origin, chart, r, &author).await
            }
        };
        result.map_err(|e| db.explain(e))
    };
    signed.map_err(|e| sign_failure(&e, &act))?;
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    record_timing(state, act.timing_kind(), 1, elapsed_ms).await;
    Ok(written(
        allergy_list_impl(state, &patient.to_string()).await,
    ))
}

/// Why a sign of `act` failed, as the clinician reads it. A commit that got no ordinary answer
/// (#814) MAY have landed: it says the outcome is NOT known, naming the act, and re-reads, never a
/// live retry that could record it twice (worded as `notes::note_sign_impl` words a note's). Any
/// other failure wrote nothing and is the node's own sentence. Pure.
pub(crate) fn sign_failure(e: &anyhow::Error, act: &Act) -> ErrorView {
    match cairn_node::db_diagnosis::commit_outcome_unknown_event(e) {
        Some(event) => ErrorView {
            text: format!(
                "{} is NOT known — no ordinary answer came back while saving it (event {event}: \
                 {}). Once the window can reach the node again, the chart shows whether it is \
                 there.",
                act.unknown_lead(),
                cairn_gui_live::cause_of(e)
            ),
            retry: Retry::AfterReread(Recorded::Unknown),
        },
        None => node_failure(e),
    }
}

/// What a sign that LANDED answers (ruling R11): always an Ok. With the list read again, the
/// pane; without it, a sentence saying it was signed and how to see it — never an error, which
/// the window would offer to retry and so record the allergy twice. Pure.
pub(crate) fn written(reread: Result<AllergyPane, ErrorView>) -> AllergyWrite {
    match reread {
        Ok(pane) => AllergyWrite {
            outcome: "Recorded.".into(),
            pane: Some(pane),
        },
        Err(e) => AllergyWrite {
            outcome: format!(
                "Signed — but the allergy list could not be read again ({}). \
                 Use “Read this chart again”.",
                e.text
            ),
            pane: None,
        },
    }
}

/// The form's verdicts, said before the key, in order: no substance, no allergen, no kind, no
/// category, an onset that cannot be read, is after today or is in year 0000, no source; then any
/// word the window should never send. The term is trimmed and
/// a blank free text is none. Pure.
pub(crate) fn entry_before_the_key(
    e: &EntryInput,
    today: NaiveDate,
) -> Result<AllergyEntry<'_>, ErrorView> {
    let term = e.term.trim();
    if term.is_empty() {
        return Err(refused("Name the substance."));
    }
    let allergen_kind = e
        .allergen_kind
        .as_deref()
        .ok_or_else(|| refused("Choose Drug or Other."))?;
    let kind = e
        .kind
        .as_deref()
        .ok_or_else(|| refused("Choose Allergy or Adverse effect."))?;
    let category = e
        .category
        .as_deref()
        .ok_or_else(|| refused("Choose what happened, or Not known."))?;
    let onset = parse_onset(e.onset.as_deref(), today).map_err(refused)?;
    let source = e
        .source
        .as_deref()
        .ok_or_else(|| refused("Choose where this came from."))?;
    Ok(AllergyEntry {
        allergen_kind: word(AllergenKind::parse, allergen_kind)?,
        term,
        kind: word(EntryKind::parse, kind)?,
        category: word(ReactionCategory::parse, category)?,
        reaction_text: e
            .reaction_text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
        onset,
        source: word(Source::parse, source)?,
        // Set by the caller once the line it names is found on a fresh read.
        corrects: None,
        coding: None,
    })
}

/// A wire word the webview sent, parsed. The window sends only known words, so one that does not
/// parse is a defect in the window: refused and named, never read as its nearest neighbour.
fn word<T>(parse: fn(&str) -> Option<T>, w: &str) -> Result<T, ErrorView> {
    parse(w).ok_or_else(|| {
        refused(format!(
            "this window sent a word this node does not know (\"{w}\") — nothing was done"
        ))
    })
}

/// Read the onset as typed — `2019`, `03/2019`, `14/03/2019`, or ISO `2019-03`, `2019-03-14`;
/// empty or `None` is not known — into its normalised ISO form. Refuses a date that does not
/// exist or one after `today`, naming what was typed. Pure (the caller passes today).
pub fn parse_onset(s: Option<&str>, today: NaiveDate) -> Result<Onset, String> {
    let typed = s.unwrap_or("").trim();
    if typed.is_empty() {
        return Ok(Onset::NotKnown);
    }
    let bad = || {
        format!(
            "\"{typed}\" is not a date this window can read — type 2019, 03/2019 or 14/03/2019, \
             or leave it empty if it is not known."
        )
    };
    // All digits, of an allowed length.
    let digits = |p: &str, lengths: std::ops::RangeInclusive<usize>| {
        (lengths.contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u32>().ok())
            .flatten()
            .ok_or_else(bad)
    };
    let slash: Vec<&str> = typed.split('/').collect();
    let dash: Vec<&str> = typed.split('-').collect();
    // (year, month, day) as typed, day-first with slashes, year-first with dashes.
    let (y, m, d) = match (slash.as_slice(), dash.as_slice()) {
        ([m, y], _) => (*y, Some(*m), None),
        ([d, m, y], _) => (*y, Some(*m), Some(*d)),
        ([_], [y, m]) => (*y, Some(*m), None),
        ([_], [y, m, d]) => (*y, Some(*m), Some(*d)),
        ([y], [_]) => (*y, None, None),
        _ => return Err(bad()),
    };
    let year = digits(y, 4..=4)?;
    // chrono reads year 0 as 1 BC; no clinician means that, and the floor refuses it too.
    if year == 0 {
        return Err(format!(
            "\"{typed}\" is not a date this window can read — there is no year 0000; type the \
             year it happened, or leave it empty if it is not known."
        ));
    }
    let month = m.map(|m| digits(m, 1..=2)).transpose()?;
    let day = d.map(|d| digits(d, 1..=2)).transpose()?;
    let date = NaiveDate::from_ymd_opt(year as i32, month.unwrap_or(1), day.unwrap_or(1))
        .ok_or_else(bad)?;
    // Later than today at the precision typed: "11/2026" in November 2026 is not the future.
    let future = match (month, day) {
        (None, _) => date.year() > today.year(),
        (Some(_), None) => (date.year(), date.month()) > (today.year(), today.month()),
        (Some(_), Some(_)) => date > today,
    };
    if future {
        return Err(format!(
            "\"{typed}\" is after today — when it happened cannot be later than today."
        ));
    }
    Ok(match (month, day) {
        (None, _) => Onset::Year(format!("{year:04}")),
        (Some(m), None) => Onset::Month(format!("{year:04}-{m:02}")),
        (Some(m), Some(d)) => Onset::Day(format!("{year:04}-{m:02}-{d:02}")),
    })
}

/// This terminal's calendar day: an onset "after today" is judged as the clinician here reads it.
pub(crate) fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
#[path = "allergies_tests.rs"]
mod tests;
