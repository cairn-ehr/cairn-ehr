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
//! as an [`AllergyWrite`].
//!
//! No markup handling anywhere: the text the clinician typed is signed as typed (only surrounding
//! whitespace trimmed, no escaping) and the webview
//! draws it with `textContent` only.
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
use cairn_medication_view::ChartSet;
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::view::{build_view, EntryLine, ListStatus};
use chrono::{Datelike, NaiveDate, Offset};
use std::time::Instant;
use uuid::Uuid;

/// One entry as the form sends it. Every word is a wire word (`cairn_event::allergy`'s
/// vocabularies); `kind` and `category` are `None` until the clinician presses one — the form
/// starts with neither pressed, so neither can be signed by default.
#[derive(Debug, serde::Deserialize)]
pub struct EntryInput {
    pub allergen_kind: String,
    pub term: String,
    pub kind: Option<String>,
    pub category: Option<String>,
    pub reaction_text: Option<String>,
    /// As typed: `2019`, `03/2019`, `14/03/2019` (or ISO); empty or `None` = not known.
    pub onset: Option<String>,
    pub source: String,
}

/// The allergy section as the webview reads it: `cairn_node`'s `AllergyView`, mapped onto plain
/// strings the way every other pane here is (the node's types carry no `Serialize`).
#[derive(Debug, serde::Serialize)]
pub struct AllergyPane {
    /// The one-line header strip.
    pub strip: String,
    /// `"entries"`, `"none-known"` or `"not-yet-asked"`, for the drug list and the other list.
    pub drug: &'static str,
    pub other: &'static str,
    pub drug_allergies: Vec<AllergyLine>,
    pub other_allergies: Vec<AllergyLine>,
    pub adverse_effects: Vec<AllergyLine>,
    /// "Previously recorded: …" lines: "none known" statements standing for no list (overridden,
    /// superseded, of an unknown scope, or on a chart whose link is in doubt).
    pub previously_none: Vec<String>,
    /// The standing "none known" statement(s), each naming when and by whom it was signed.
    pub standing_none: Vec<String>,
    /// Allergy records on this chart set this terminal cannot read (no custody, or shredded):
    /// the strip leads with them, and the window shows it prominent.
    pub unreadable: usize,
    /// The chart set these rows were read over: the webview sends it back with every write.
    pub charts: Vec<String>,
}

/// What a write answers once its event is signed: what happened, and the list read again when
/// that read worked. A write that signed nothing answers an `ErrorView` instead.
#[derive(Debug, serde::Serialize)]
pub struct AllergyWrite {
    pub outcome: String,
    pub pane: Option<AllergyPane>,
}

/// One line of an expanded list.
#[derive(Debug, serde::Serialize)]
pub struct AllergyLine {
    pub event_id: String,
    /// The line as the clinician reads it, doubt mark included.
    pub text: String,
    /// The red sticker: an allergy, not an adverse effect or a kind this node does not know.
    pub prominent: bool,
    pub doubted_chart: bool,
    /// Who signed it, when this node can name the key.
    pub signer: Option<String>,
    /// The local day it was recorded, `YYYY-MM-DD`.
    pub recorded: String,
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
    let Some(db) = state.db.as_ref() else {
        let empty = PatientAllergies {
            entries: vec![],
            none: vec![],
            doubted_charts: vec![],
            unreadable: 0,
        };
        return Ok(pane(&empty, &ChartSet::single(patient)));
    };
    let mut db = db.lock().await?;
    let (set, allergies) = cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let set = cairn_node::patient::person::person_charts(tx, patient).await?;
        let allergies = cairn_node::allergy::read::list_patient_allergies(tx, patient).await?;
        Ok((set, allergies))
    })
    .await
    .map_err(|e| chart_read_failed(db.explain(e)))?;
    Ok(pane(&allergies, &set))
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
    let entry = entry_before_the_key(&entry, local_today())?;
    sign_then_reread(state, patient, Act::Entry(entry), credential).await
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
    sign_then_reread(state, patient, Act::None(scope), credential).await
}

/// The first three refusals every write says, in order: not the chart on screen, not the chart
/// set the list was read over, fixture mode. Answers the chart.
async fn on_screen_and_writable(
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

/// The two signed acts this file writes.
enum Act<'a> {
    Entry(AllergyEntry<'a>),
    None(NoneScope),
}

/// Ask for the key, sign `act` on `patient`'s chart, time it, and read the list again.
///
/// A failed sign is an `ErrorView` ([`sign_failure`]: nothing written, or NOT known). A sign
/// that landed is an Ok whatever the re-read did ([`written`]).
async fn sign_then_reread(
    state: &AppState,
    patient: Uuid,
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
    let (signed, kind) = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        use cairn_node::allergy::sign::{sign_allergy, sign_allergy_none};
        let origin = &state.node_origin;
        let (result, kind) = match &act {
            Act::Entry(e) => (
                sign_allergy(&mut db, node_sk, &node_kid, origin, patient, e, &author).await,
                "allergy-record",
            ),
            Act::None(s) => (
                sign_allergy_none(&mut db, node_sk, &node_kid, origin, patient, *s, &author).await,
                "allergy-none",
            ),
        };
        (result.map_err(|e| db.explain(e)), kind)
    };
    signed.map_err(|e| sign_failure(&e))?;
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    record_timing(state, kind, 1, elapsed_ms).await;
    Ok(written(
        allergy_list_impl(state, &patient.to_string()).await,
    ))
}

/// Why a sign failed, as the clinician reads it. A commit that got no ordinary answer (#814) MAY
/// have landed: it says the outcome is NOT known and re-reads, never a live retry that could
/// record the allergy twice (worded as `notes::note_sign_impl` words a note's). Any other
/// failure wrote nothing and is the node's own sentence. Pure.
pub(crate) fn sign_failure(e: &anyhow::Error) -> ErrorView {
    match cairn_node::db_diagnosis::commit_outcome_unknown_event(e) {
        Some(event) => ErrorView {
            text: format!(
                "Whether this allergy was recorded is NOT known — no ordinary answer came back \
                 while saving it (event {event}: {}). Once the window can reach the node again, \
                 the chart shows whether it is there.",
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

/// The form's verdicts, said before the key, in order: no substance, no kind, no category, an
/// onset that cannot be read, is after today or is in year 0000; then any word the window should never send. The term is trimmed and
/// a blank free text is none. Pure.
pub(crate) fn entry_before_the_key(
    e: &EntryInput,
    today: NaiveDate,
) -> Result<AllergyEntry<'_>, ErrorView> {
    let term = e.term.trim();
    if term.is_empty() {
        return Err(refused("Name the substance."));
    }
    let kind = e
        .kind
        .as_deref()
        .ok_or_else(|| refused("Choose Allergy or Adverse effect."))?;
    let category = e
        .category
        .as_deref()
        .ok_or_else(|| refused("Choose what happened, or Not known."))?;
    let onset = parse_onset(e.onset.as_deref(), today).map_err(refused)?;
    Ok(AllergyEntry {
        allergen_kind: word(AllergenKind::parse, &e.allergen_kind)?,
        term,
        kind: word(EntryKind::parse, kind)?,
        category: word(ReactionCategory::parse, category)?,
        reaction_text: e
            .reaction_text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
        onset,
        source: word(Source::parse, &e.source)?,
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
fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

/// `cairn_node`'s pure view over `a`, in this terminal's zone (the node never knows it), as the
/// pane, with the chart set it was read over.
fn pane(a: &PatientAllergies, set: &ChartSet) -> AllergyPane {
    let v = build_view(a, chrono::Local::now().offset().fix());
    let status = |s: &ListStatus| match s {
        ListStatus::Entries => "entries",
        ListStatus::NoneKnown { .. } => "none-known",
        ListStatus::NotYetAsked => "not-yet-asked",
    };
    let lines = |ls: Vec<EntryLine>| -> Vec<AllergyLine> {
        ls.into_iter()
            .map(|l| AllergyLine {
                event_id: l.event_id.to_string(),
                text: l.text,
                prominent: l.prominent,
                doubted_chart: l.doubted_chart,
                signer: l.signer,
                recorded: l.recorded.to_string(),
            })
            .collect()
    };
    AllergyPane {
        strip: v.strip,
        drug: status(&v.drug),
        other: status(&v.other),
        drug_allergies: lines(v.drug_allergies),
        other_allergies: lines(v.other_allergies),
        adverse_effects: lines(v.adverse_effects),
        previously_none: v.previously_none,
        standing_none: v.standing_none,
        unreadable: v.unreadable,
        charts: set.members().iter().map(Uuid::to_string).collect(),
    }
}

#[cfg(test)]
#[path = "allergies_tests.rs"]
mod tests;
