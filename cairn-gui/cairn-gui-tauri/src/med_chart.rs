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
//!   (`started-here` / `pre-existing`), each part named and each pick's code, then the allergy check RE-RUN on a
//!   fresh read over the set on screen (a set that changed since is refused as a re-read, R15)
//!   — the webview's earlier answer is never trusted — and, on a Start line with an
//!   allergy mark, the reason (or, when the window sent one but that re-run could not check
//!   the line, [`CHECK_DID_NOT_RUN`]: a typed reason is never dropped). Then one signed assertion carrying its charting fields
//!   (`chart_medication`), its timing sample, and the medication list read again.
//!
//! Requiring an override is a window rule, never a floor rule (design §3.5, ADR-0021 soft
//! policy): the floor never requires one, and checks a present override's shape (its reason
//! non-blank, at the authoring door). The drug list is optional: with
//! none, or a check that fails, the line still charts, uncoded, and reads "not checked".
use crate::allergies::{failure_named, local_today, on_screen_and_writable};
use crate::allergy_check::{drug_search_answer, CodingArg, DrugSearch};
use crate::commands::{med_list_impl, record_timing};
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart_write::{charted, charted_input, MedChartWrite};
// The helpers this file shares with the list's Sign (`med_line_sign.rs`). Names its tests read
// through `super::*` from before the move are imported for them alone.
use crate::med_line_sign::{
    charting, components, fresh_check, not_read, override_refs, parts_before_the_key,
    single_coding, the_line,
};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write};
#[cfg(test)]
use crate::{allergy_check::check_lines, med_line_sign::fresh_allergies};
use cairn_event::medication::{ChartedAs, Charting};
use cairn_node::allergy::check::LineCheck;
#[cfg(test)]
use cairn_node::allergy::check::MOIETY_SYSTEM;
use cairn_node::allergy::drug_facts::search_medication_drugs;
use cairn_node::medication::line::{read_directions, Directions};
use cairn_node::medication::{chart_medication, AssertMedicationInput, AuthorParams};
use std::time::Instant;
use uuid::Uuid;

/// The refusal of Sign on a Start line with an allergy mark and no reason (Global Constraints).
pub const NEEDS_A_REASON: &str = "Charting despite a listed allergy needs a reason";

/// The refusal of Sign when the window drew a mark (and so sent a reason) but Sign's own fresh
/// check could not run: signing would drop the reason silently, so nothing is signed and the
/// window checks the line again (PR #1066 review finding).
pub const CHECK_DID_NOT_RUN: &str =
    "The allergy check could not run just now: nothing was signed. Look at the line again";

/// The refusal of a line (or a part of a combination) with no name.
pub const NAME_IT: &str = "Name the medication.";

/// The refusal of a pick that is not a drug (a drug group, or another system): the medication
/// pick lists drugs only, so only a defect in the webview sends one. Said before the key, as the
/// floor would refuse it after.
pub const NOT_A_LISTED_DRUG: &str =
    "That pick is not a drug from the drug list; type the name instead";

/// How a Sign whose commit got no ordinary answer names the act (#814).
pub(crate) const UNKNOWN_LEAD: &str = "Whether this medication was charted";

/// How many drugs the pick lists (as the allergy pick's `cairn_drug_search`).
const PICK_LIMIT: i32 = 12;

/// One part of the line as the webview sends it: the words typed for it, and the drug picked
/// for it (`med_drug_search`'s option, as sent back), if one was.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

/// The line's check as the webview draws it: the loud `Allergy` marks, the notes that warn at
/// the site's threshold (drawn as loud, under a `Warning` tag, ADR-0104), the muted notes, the
/// small "not checked" tag, and whether a Start line needs a reason (anything warns).
#[derive(Debug, serde::Serialize)]
pub struct LineCheckView {
    pub marks: Vec<String>,
    pub warning_notes: Vec<String>,
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
/// (as Sign sends it); the lines checked are built by the same `the_line` Sign uses, so the
/// on-screen check and the check at Sign read the same terms (rulings R16, R24).
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
    let lines = the_line(&typed_drug, &term, &parts);
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
    let check = fresh_check(
        state,
        patient,
        &charts,
        &the_line(&line.typed_drug, &term, &parts),
    )
    .await?;
    let reason = override_before_the_key(charted_as, &check, line.reason.as_deref())?;
    // What the fresh check found warning, each named by its allergy record's event id.
    let ids = override_ids(&check);
    let against = override_refs(&ids);
    let d = read_directions(&line.directions);
    let today = local_today().to_string();
    let components = components(&parts);
    let single = single_coding(&parts);
    let input = charted_input(&term, single, &d, &line.directions, charted_as, &today);
    let charting = charting(charted_as, &d, &components, reason, &against);
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

/// The reason to sign as the warning override (an allergy source today), or the refusal. Only a
/// Start line with a warning at the site's threshold needs one (an allergy mark; a previous
/// adverse effect too when the administrator lowered it, ADR-0104): an already-taking line
/// records what the patient takes, a quiet note or "not checked" is no warning. A blank trimmed reason is [`NEEDS_A_REASON`]; a reason
/// sent against a check that could not run is [`CHECK_DID_NOT_RUN`]. Pure.
pub(crate) fn override_before_the_key<'a>(
    charted_as: ChartedAs,
    check: &LineCheck,
    reason: Option<&'a str>,
) -> Result<Option<&'a str>, ErrorView> {
    if charted_as != ChartedAs::StartedHere {
        return Ok(None);
    }
    let reason = reason.map(str::trim).filter(|r| !r.is_empty());
    match (!check.warns(), reason) {
        // A reason the window sent for a mark this check could not confirm: never dropped.
        (true, Some(_)) if check.unverified => Err(refused(CHECK_DID_NOT_RUN)),
        (true, _) => Ok(None),
        (false, Some(r)) => Ok(Some(r)),
        (false, None) => Err(refused(NEEDS_A_REASON)),
    }
}

/// The check as the webview draws it. Pure.
pub(crate) fn view(c: LineCheck) -> LineCheckView {
    LineCheckView {
        reason_needed: c.warns(),
        warning_notes: c.warning_notes().to_vec(),
        notes: c.quiet_notes().to_vec(),
        marks: c.marks,
        unverified: c.unverified,
    }
}

/// The allergy records a signed override names (as text): every warning `c` found at its
/// threshold, marks first. Pure.
pub(crate) fn override_ids(c: &LineCheck) -> Vec<String> {
    c.warned_against().iter().map(Uuid::to_string).collect()
}

#[cfg(test)]
#[path = "med_chart_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "med_chart_db_tests.rs"]
mod db_tests;

#[cfg(test)]
#[path = "med_chart_threshold_tests.rs"]
mod threshold_tests;

#[cfg(test)]
#[path = "med_chart_threshold_db_tests.rs"]
mod threshold_db_tests;
