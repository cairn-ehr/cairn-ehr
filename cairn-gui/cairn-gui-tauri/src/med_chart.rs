//! The one-line medication editor (charting slice 1a), which charts nothing itself: its three
//! commands, and the per-line rules the list's Sign (`med_list_sign.rs`) signs by.
//!
//! - [`med_line_read`]: the typed directions read into fields for the read-back row (pure; the
//!   rules are `cairn_node::medication::line`'s).
//! - [`med_drug_search`]: the medication pick, drugs only (no allergy groups), or `available:
//!   false` with no drug list. Never fails, as `allergy_check::drug_search` never does.
//! - [`med_line_check`]: the line being typed, checked against the live allergies, part by part
//!   (a combination is checked one component at a time), so the window can put the warning on
//!   the line before Sign.
//!
//! The window charts only through the list's ONE Sign (ADR-0100). Slice 1a's lone-line `med_chart`
//! command was deleted (#1139): nothing called it, and it signed every warning its fresh check
//! found as overridden, where the list's Sign vouches only for the warnings drawn (ruling R1). The
//! per-line rules the list's Sign shares stay here: [`press`], [`override_before_the_key`]
//! ([`NEEDS_A_REASON`], [`CHECK_DID_NOT_RUN`]), [`override_ids`] and [`UNKNOWN_LEAD`].
//!
//! Requiring an override is a window rule, never a floor rule (design §3.5, ADR-0021 soft
//! policy): the floor never requires one, and checks a present override's shape (its reason
//! non-blank, at the authoring door). The drug list is optional: with
//! none, or a check that fails, the line still charts, uncoded, and reads "not checked".
use crate::allergy_check::{drug_search_answer, CodingArg, DrugSearch};
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_line_sign::{fresh_check, not_read, parts_before_the_key, the_line};
use crate::state::AppState;
use cairn_event::medication::ChartedAs;
use cairn_node::allergy::check::LineCheck;
use cairn_node::allergy::drug_facts::search_medication_drugs;
use cairn_node::medication::line::{read_directions, Directions};
use uuid::Uuid;
// Names this file's tests read through `super::*` (from before the helpers moved out).
#[cfg(test)]
use crate::{
    allergy_check::check_lines, med_chart_write::charted_input, med_line_sign::fresh_allergies,
};
#[cfg(test)]
use cairn_node::allergy::check::MOIETY_SYSTEM;

/// The refusal of Sign on a Start line with a warning (an allergy mark, or an adverse effect at a
/// lowered threshold, ADR-0104) and no reason. General words: an adverse effect is no allergy.
pub const NEEDS_A_REASON: &str = "Charting despite a warning needs a reason";

/// The refusal of Sign when the window drew a warning (and so sent a reason) but Sign's own fresh
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
/// records what the patient takes, a quiet note or "not checked" is no warning. A blank trimmed
/// reason is [`NEEDS_A_REASON`]; a reason sent against a check that could not run is
/// [`CHECK_DID_NOT_RUN`]. Pure.
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
        // A reason the window sent for a warning this check could not confirm: never dropped.
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
