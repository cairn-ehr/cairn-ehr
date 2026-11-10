//! Charting a medication from one typed line (charting slice 1a): the window's four commands.
//!
//! - [`med_line_read`]: the typed directions read into fields for the read-back row (pure; the
//!   rules are `cairn_node::medication::line`'s).
//! - [`med_drug_search`]: the medication pick, drugs only (no allergy groups), or `available:
//!   false` with no drug list. Never fails, as `allergy_check::drug_search` never does.
//! - [`med_line_check`]: the line being typed, checked against the live allergies, part by part
//!   (a combination is checked one component at a time), so the window can put the warning on
//!   the line before Sign.
//! - [`med_chart`]: Sign. Refusal order, every one before the key (a passphrase that writes
//!   nothing is a step paper never has): the chart on screen and writable, the press
//!   (`started-here` / `pre-existing`), each pick's code, then the allergy check RE-RUN on a
//!   fresh read over the set on screen (a set that changed since is refused as a re-read, R15)
//!   — the webview's earlier answer is never trusted — and, on a Start line with an
//!   allergy mark, the reason. Then one signed assertion carrying its charting fields
//!   (`chart_medication`), its timing sample, and the medication list read again.
//!
//! The reason is a window rule, never a floor rule (design §3.5, ADR-0021 soft policy): the
//! floor only checks the override's shape when it is there. The drug list is optional: with
//! none, or a check that fails, the line still charts, uncoded, and reads "not checked".
use crate::allergies::{failure_named, local_today, on_screen_and_writable, read_list};
use crate::allergy_check::{
    canonical_code, check_lines, drug_search_answer, part_line, CodingArg, DrugSearch,
    ALLERGIES_NOT_READ,
};
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::commands::{med_list_impl, record_timing};
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart_write::{charted, charted_input, substance, MedChartWrite};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write};
use cairn_event::medication::{AllergyOverride, ChartedAs, Charting, Component};
use cairn_medication_view::ChartSet;
use cairn_node::allergy::check::{LineCheck, MedLine, MOIETY_SYSTEM};
use cairn_node::allergy::drug_facts::search_medication_drugs;
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::medication::line::{read_directions, term_to_sign, Directions};
use cairn_node::medication::{chart_medication, AssertMedicationInput, AuthorParams};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// The refusal of Sign on a Start line with an allergy mark and no reason (Global Constraints).
pub const NEEDS_A_REASON: &str = "Charting despite a listed allergy needs a reason";

/// The refusal of a line (or a part of a combination) with no name.
pub const NAME_IT: &str = "Name the medication.";

/// The refusal of a pick that is not a drug (a drug group, or another system): the medication
/// pick lists drugs only, so only a defect in the webview sends one. Said before the key, as the
/// floor would refuse it after.
pub const NOT_A_LISTED_DRUG: &str =
    "That pick is not a drug from the drug list; type the name instead";

/// How a Sign whose commit got no ordinary answer names the act (#814).
const UNKNOWN_LEAD: &str = "Whether this medication was charted";

/// How many drugs the pick lists (as the allergy pick's `cairn_drug_search`).
const PICK_LIMIT: i32 = 12;

/// The group id the line being charted is checked under: it has no thread yet.
const THE_LINE: Uuid = Uuid::nil();

/// One part of the line as the webview sends it: the words typed for it, and the drug picked
/// for it (`med_drug_search`'s option, as sent back), if one was.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PartArg {
    pub term: String,
    pub coding: Option<CodingArg>,
}

/// The line, as Sign sends it. `parts` are the drug parts split at ` + ` (one for a single
/// drug); `typed_drug` is the drug as typed, `directions` everything after it. `charted_as` is
/// the button pressed (`started-here` | `pre-existing`); `reason` the reason field, if shown.
#[derive(Debug, serde::Deserialize)]
pub struct ChartArg {
    pub parts: Vec<PartArg>,
    pub typed_drug: String,
    pub directions: String,
    pub charted_as: String,
    pub reason: Option<String>,
}

/// The line's check as the webview draws it: the loud `Allergy` marks, the muted notes, the
/// small "not checked" tag, and whether a Start line needs a reason (it has a mark).
#[derive(Debug, serde::Serialize)]
pub struct LineCheckView {
    pub marks: Vec<String>,
    pub notes: Vec<String>,
    pub unverified: bool,
    pub reason_needed: bool,
}

/// Read typed directions into fields; pure, no state.
#[tauri::command]
pub async fn med_line_read(text: String) -> Directions {
    read_directions(&text)
}

/// The medication pick: drugs matching `text`.
#[tauri::command]
pub async fn med_drug_search(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<DrugSearch, ErrorView> {
    Ok(med_drug_search_impl(&state, &text).await)
}

/// Check the line being typed against the chart's allergies. `typed_drug` is the drug as typed
/// (as Sign sends it), so the line term here is the one Sign will check (ruling R16).
#[tauri::command]
pub async fn med_line_check(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    typed_drug: String,
    parts: Vec<PartArg>,
) -> Result<LineCheckView, ErrorView> {
    Ok(med_line_check_impl(&state, &patient_id, &charts, typed_drug, parts).await)
}

/// Sign the line on the chart on screen (refusal order: module doc).
#[tauri::command]
pub async fn med_chart(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    line: ChartArg,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    med_chart_impl(&state, &patient_id, charts, line, credential).await
}

/// [`med_drug_search`] against a plain state. Never fails: no database, a lost connection or a
/// drug list that raises all read as no drug list. On its own lock, never inside a snapshot
/// (ruling R13): the door swallows its failure, which would abort a shared transaction.
pub async fn med_drug_search_impl(state: &AppState, text: &str) -> DrugSearch {
    let found = match state.db.as_ref() {
        None => None,
        Some(db) => match db.lock().await {
            Ok(db) => search_medication_drugs(&*db, text, PICK_LIMIT)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("the drug list could not be searched for a medication: {e:#}");
                    None
                }),
            Err(e) => {
                eprintln!("the drug list could not be searched: the connection was lost: {e}");
                None
            }
        },
    };
    drug_search_answer(found)
}

/// [`med_line_check`]: never fails. A chart that is no longer on screen, or a part the webview
/// sent unreadably, reads "not checked"; the allergy list not read says so in a note too.
pub async fn med_line_check_impl(
    state: &AppState,
    patient_id: &str,
    charts: &[String],
    typed_drug: String,
    parts: Vec<PartArg>,
) -> LineCheckView {
    let not_checked = || {
        view(LineCheck {
            unverified: true,
            ..LineCheck::default()
        })
    };
    let Ok(patient) = state.displayed_patient(patient_id).await else {
        return not_checked();
    };
    // The term exactly as Sign builds it (ruling R16), so the two checks cannot disagree.
    let Ok((term, parts)) = parts_before_the_key(&parts, &typed_drug) else {
        return not_checked();
    };
    // A set changed since the screen was drawn is only "not checked" while typing; Sign refuses.
    let lines = the_line(&term, &parts);
    let check = fresh_check(state, patient, charts, &lines).await;
    view(check.unwrap_or_else(|_| not_read("the linked charts changed since the list was read")))
}

/// Sign the line (refusal order: module doc).
pub async fn med_chart_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    line: ChartArg,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let charted_as = press(&line.charted_as)?;
    let (term, parts) = parts_before_the_key(&line.parts, &line.typed_drug)?;
    // Re-run on a fresh read: the allergies may have changed since the webview last asked.
    let check = fresh_check(state, patient, &charts, &the_line(&term, &parts)).await?;
    let reason = override_before_the_key(charted_as, &check, line.reason.as_deref())?;
    let against: Vec<String> = check.against.iter().map(Uuid::to_string).collect();
    let against: Vec<&str> = against.iter().map(String::as_str).collect();
    let d = read_directions(&line.directions);
    let today = local_today().to_string();
    // One part is the line's own coding; two or more are its components, never both.
    let single = match parts.as_slice() {
        [one] => one.coding.as_ref().map(substance),
        _ => None,
    };
    let components: Vec<Component> = match parts.len() {
        0 | 1 => vec![],
        _ => parts
            .iter()
            .map(|p| Component {
                term: &p.term,
                coding: p.coding.as_ref().map(substance),
            })
            .collect(),
    };
    let input = charted_input(&term, single, &d, &line.directions, charted_as, &today);
    let charting = Charting {
        charted_as: Some(charted_as),
        route: d.route.as_deref(),
        strength: d.strength.as_deref(),
        components: &components,
        allergy_override: reason.map(|reason| AllergyOverride {
            reason,
            against: &against,
        }),
    };
    sign_then_reread(state, patient, charts, &input, &charting, credential).await
}

/// Ask for the key, sign the line on `patient` (the chart on screen), time it, and read the
/// medication list again. A failed sign is an `ErrorView` (`allergies::failure_named`: nothing
/// written, or NOT known); a sign that landed is an Ok whatever the re-read did ([`charted`]).
async fn sign_then_reread(
    state: &AppState,
    patient: Uuid,
    charts: Vec<String>,
    input: &AssertMedicationInput<'_>,
    charting: &Charting<'_>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let author = AuthorParams {
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
        let origin = state.node_origin.as_str();
        chart_medication(
            &mut db,
            node_sk,
            &node_kid,
            origin,
            patient,
            input,
            charting,
            Some(&author),
            None,
        )
        .await
        .map_err(|e| db.explain(e))
    };
    signed.map_err(|e| failure_named(&e, UNKNOWN_LEAD))?;
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    record_timing(state, "medication-chart", 1, elapsed_ms).await;
    // `charts` is what the window showed: only a change from elsewhere since then is announced.
    let reread = med_list_impl(state, &patient.to_string(), Some(charts)).await;
    Ok(charted(reread))
}

/// The press, parsed: `started-here` or `pre-existing`. The window has no button for any other
/// word (not even the wire's `unknown`), so one is a defect in the window, refused and named.
pub(crate) fn press(w: &str) -> Result<ChartedAs, ErrorView> {
    match ChartedAs::parse(w) {
        Some(c @ (ChartedAs::StartedHere | ChartedAs::PreExisting)) => Ok(c),
        _ => Err(refused(format!(
            "this window sent a word this node does not know (\"{w}\") — nothing was done"
        ))),
    }
}

/// The line's term and its parts as they will be signed, or the refusal, said before the key.
/// Each pick must be a drug ([`NOT_A_LISTED_DRUG`]) with a code that reads as a uuid (signed in
/// its canonical spelling, `canonical_code`). One part's term is `term_to_sign(typed_drug, its
/// pick)` ("amox" picked as amoxicillin signs "amoxicillin"); a combination's term is the drug
/// as typed ("amox + clav") and each part's is `term_to_sign` of its own words. No parts sent is
/// the drug as typed, uncoded. A blank name anywhere is [`NAME_IT`]. Pure.
pub(crate) fn parts_before_the_key(
    sent: &[PartArg],
    typed_drug: &str,
) -> Result<(String, Vec<PartArg>), ErrorView> {
    let sent = match sent {
        [] => vec![PartArg {
            term: typed_drug.to_string(),
            coding: None,
        }],
        parts => parts.to_vec(),
    };
    let single = sent.len() == 1;
    let mut parts = Vec::with_capacity(sent.len());
    for p in sent {
        let coding = p.coding.map(picked).transpose()?;
        let typed = if single { typed_drug } else { &p.term };
        let term = term_to_sign(typed, coding.as_ref().map(|c| c.display.as_str()));
        if term.is_empty() {
            return Err(refused(NAME_IT));
        }
        parts.push(PartArg { term, coding });
    }
    let term = match parts.as_slice() {
        [one] => one.term.clone(),
        _ => typed_drug.trim().to_string(),
    };
    if term.is_empty() {
        return Err(refused(NAME_IT));
    }
    Ok((term, parts))
}

/// A pick as it will be signed: refused unless it is a drug, its code re-spelled. Pure.
fn picked(c: CodingArg) -> Result<CodingArg, ErrorView> {
    if c.system != MOIETY_SYSTEM {
        return Err(refused(NOT_A_LISTED_DRUG));
    }
    let code = canonical_code(&c)?;
    Ok(CodingArg { code, ..c })
}

/// The reason to sign as the allergy override, or the refusal. Only a Start line with an
/// allergy mark needs one: an already-taking line records what the patient takes, a note or
/// "not checked" is no listed allergy. A blank trimmed reason is [`NEEDS_A_REASON`]. Pure.
pub(crate) fn override_before_the_key<'a>(
    charted_as: ChartedAs,
    check: &LineCheck,
    reason: Option<&'a str>,
) -> Result<Option<&'a str>, ErrorView> {
    if charted_as != ChartedAs::StartedHere || check.marks.is_empty() {
        return Ok(None);
    }
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(r) => Ok(Some(r)),
        None => Err(refused(NEEDS_A_REASON)),
    }
}

/// The line being charted as the check reads it: one line per part under [`THE_LINE`], each
/// carrying the line's own term (ruling R12). Pure.
pub(crate) fn the_line<'a>(term: &'a str, parts: &'a [PartArg]) -> Vec<MedLine<'a>> {
    parts
        .iter()
        .map(|p| {
            let coding = p
                .coding
                .as_ref()
                .map(|c| (c.system.as_str(), c.code.as_str(), c.display.as_str()));
            part_line(THE_LINE, term, &p.term, coding, true)
        })
        .collect()
}

/// `lines` checked against the allergies of `patient`, read now, over the chart set the window
/// showed (`charts`) — see [`fresh_allergies`] for the refusal and the "not checked" case.
async fn fresh_check(
    state: &AppState,
    patient: Uuid,
    charts: &[String],
    lines: &[MedLine<'_>],
) -> Result<LineCheck, ErrorView> {
    let Some(allergies) = fresh_allergies(read_list(state, patient).await, charts)? else {
        return Ok(not_read("the allergy list could not be read"));
    };
    let db = match state.db.as_ref() {
        Some(db) => match db.lock().await {
            Ok(db) => Some(db),
            Err(e) => {
                eprintln!("the drug list was not asked: the connection was lost: {e}");
                None
            }
        },
        None => None,
    };
    let mut checks = check_lines(db.as_deref(), &allergies, lines).await;
    Ok(checks.remove(&THE_LINE).unwrap_or_default())
}

/// The allergies a fresh check uses (ruling R15). Read over the set on screen: `Some`. Read
/// over ANOTHER set (a link or unlink landed since the screen was drawn): refused as
/// `ChartSetError::Changed` refuses everywhere, a re-read — a Start line must never sign
/// unchecked against allergies the clinician has not seen. Not read at all: `None` (logged),
/// and the line proceeds "not checked", since a failed check never blocks charting. Pure.
pub(crate) fn fresh_allergies(
    read: Result<(ChartSet, PatientAllergies, HashMap<Uuid, String>), ErrorView>,
    charts: &[String],
) -> Result<Option<PatientAllergies>, ErrorView> {
    match read {
        Ok((set, allergies, _)) => check_displayed_set(&set, charts)
            .map(|_| Some(allergies))
            .map_err(ChartSetError::into_view),
        Err(e) => {
            eprintln!(
                "the allergy list could not be read for the line: {}",
                e.text
            );
            Ok(None)
        }
    }
}

/// The check when the allergies could not be read for it (`why` goes to stderr).
fn not_read(why: &str) -> LineCheck {
    eprintln!("the line being charted was not checked against the allergies: {why}");
    LineCheck {
        notes: vec![ALLERGIES_NOT_READ.into()],
        unverified: true,
        ..LineCheck::default()
    }
}

/// The check as the webview draws it. Pure.
pub(crate) fn view(c: LineCheck) -> LineCheckView {
    LineCheckView {
        reason_needed: !c.marks.is_empty(),
        marks: c.marks,
        notes: c.notes,
        unverified: c.unverified,
    }
}

#[cfg(test)]
#[path = "med_chart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "med_chart_db_tests.rs"]
mod db_tests;
