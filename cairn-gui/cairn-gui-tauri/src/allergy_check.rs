//! Allergy slice 3's window half: the drug list (drugref) where the allergy list and the
//! medication list meet.
//!
//! Three things, each a thin adapter over `cairn_node::allergy`:
//!
//! 1. **The pick** ([`drug_search`]): the drugs and drug groups matching what the clinician types
//!    in the allergy form, or `available: false` when this terminal has no drug list (fixture
//!    mode, no drugref installed, or a drug list that failed to answer) — the form then records
//!    the name as typed. A picked option comes back with `allergy_record` as a [`CodingArg`],
//!    whose code is re-spelled canonically before signing ([`canonical_code`]).
//! 2. **The group review status** ([`group_status`]): read in the allergy list's own snapshot
//!    (`allergies::read_list`), so the expanded list can say which coded groups were never
//!    clinically reviewed (D11). A failed read counts every group as not reviewed.
//! 3. **The medication check** ([`med_list_check`]): every current medication line read
//!    against the live drug allergies, handed to `chart_pane` for the rows. The rules are
//!    `cairn_node::allergy::check`'s; this file only gathers what they need.
//!
//! The rule all three share: **the drug list is optional, so it can never fail a pane.** A
//! drug list that is absent, unreadable or raises reads as "no drug list" (`DrugFacts::
//! Unavailable`, `available: false`), and the check then leaves lines "not checked" rather
//! than clean. Only a failed read of the ALLERGIES is worded on the medication list
//! ([`ALLERGIES_NOT_READ`]): with no allergies there is nothing to check against at all.
use crate::allergies::read_list;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::state::AppState;
use cairn_event::allergy::AllergenKind;
use cairn_medication_view::{ChartSet, MedicationRow, MedicationStatus, PatientMedicationList};
use cairn_node::allergy::check::{
    check_medications, codes_to_ask, LineCheck, MedLine, GROUP_SYSTEM, MOIETY_SYSTEM,
};
use cairn_node::allergy::drug_facts::{
    read_drug_facts, read_group_status, search_drugs, DrugFacts,
};
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::thread::resolve;
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// Above the medication list when the allergy list could not be read (Global Constraints).
pub const ALLERGIES_NOT_READ: &str =
    "Allergies could not be read: medications not checked against them";

/// The refusal of a picked drug whose code is not a uuid (a defect in the webview, or a drug
/// list answering in a shape this build does not know): the clinician can still type the name.
pub const BAD_CODE: &str = "That drug's code could not be read; type the name instead";

/// A drug-list coding as the webview receives it (a pick option) and sends it back with
/// `allergy_record`: `system` is `drugref-moiety` or `drugref-class`; `display` is the name the
/// drug list gave at the time of the pick (signed as it was, ADR-0059).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodingArg {
    pub system: String,
    pub code: String,
    pub display: String,
}

/// What the pick answers. `available: false` means there is no drug list to ask, and the form
/// shows its hint; an available list may still find nothing (`options` empty).
#[derive(Debug, serde::Serialize)]
pub struct DrugSearch {
    pub available: bool,
    pub options: Vec<CodingArg>,
}

/// The pick: drugs and drug groups matching `text` (the drug list's rules: literal, at least 3
/// characters, groups first).
#[tauri::command]
pub async fn drug_search(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<DrugSearch, ErrorView> {
    Ok(drug_search_impl(&state, &text).await)
}

/// [`drug_search`] against a plain state. Never fails: no database, a closed connection or a
/// drug list that raises all read as no drug list.
pub async fn drug_search_impl(state: &AppState, text: &str) -> DrugSearch {
    let found = match state.db.as_ref() {
        None => None,
        Some(db) => match db.lock().await {
            Ok(db) => search_drugs(&*db, text).await.unwrap_or_else(|e| {
                eprintln!("the drug list could not be searched: {e:#}");
                None
            }),
            Err(e) => {
                eprintln!("the drug list could not be searched: the connection was lost: {e}");
                None
            }
        },
    };
    DrugSearch {
        available: found.is_some(),
        options: found
            .unwrap_or_default()
            .into_iter()
            .map(|o| CodingArg {
                system: o.system,
                code: o.code,
                display: o.display,
            })
            .collect(),
    }
}

/// A pick sent on an entry that is not a drug (D1: only drug entries are coded): refused before
/// the key, so no passphrase is asked for an entry the floor would refuse.
pub const NOT_A_DRUG: &str =
    "Only a drug can be picked from the drug list — choose Drug, or type the name as it is.";

/// A pick whose system is neither of the drug list's own two (a defect in the webview, or a
/// reserved system such as `drugref-product`): refused before the key, as the floor would after it.
pub const NOT_FROM_THE_LIST: &str =
    "That pick is not a drug or drug group from the drug list; type the name instead";

/// What the clinician picked, as it will be signed: refused with [`NOT_A_DRUG`] on an entry that
/// is not a drug, with [`NOT_FROM_THE_LIST`] under any system but `drugref-moiety` or
/// `drugref-class`, else its code re-spelled ([`canonical_code`]). Said before the key. Pure.
pub(crate) fn coding_before_the_key(
    c: &CodingArg,
    kind: AllergenKind,
) -> Result<String, ErrorView> {
    if kind != AllergenKind::Drug {
        return Err(refused(NOT_A_DRUG));
    }
    if c.system != MOIETY_SYSTEM && c.system != GROUP_SYSTEM {
        return Err(refused(NOT_FROM_THE_LIST));
    }
    canonical_code(c)
}

/// A picked code, re-spelled as the canonical lowercase hyphenated uuid the floor admits (the
/// webview may send any spelling `Uuid` reads), or refused with [`BAD_CODE`]. Pure.
pub(crate) fn canonical_code(c: &CodingArg) -> Result<String, ErrorView> {
    c.code
        .trim()
        .parse::<Uuid>()
        .map(|u| u.to_string())
        .map_err(|_| refused(BAD_CODE))
}

/// The drug-group codes among `a`'s entries, sorted and deduplicated (what to ask the status of).
fn group_codes(a: &PatientAllergies) -> Vec<Uuid> {
    let mut codes: Vec<Uuid> = a
        .entries
        .iter()
        .filter_map(|e| e.coding.as_ref())
        .filter(|c| c.system == GROUP_SYSTEM)
        .filter_map(|c| c.code.parse().ok())
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

/// The review status of every drug group `a`'s entries are coded to, read inside the allergy
/// list's snapshot `tx`. Empty when there is nothing to ask, no drug list, or the read failed:
/// D11 counts each of those as not reviewed. The read runs under a savepoint, so a failure
/// leaves the snapshot usable and never fails the list.
pub(crate) async fn group_status(
    tx: &impl GenericClient,
    a: &PatientAllergies,
) -> HashMap<Uuid, String> {
    let groups = group_codes(a);
    if groups.is_empty() || tx.batch_execute("SAVEPOINT group_status").await.is_err() {
        return HashMap::new();
    }
    match read_group_status(tx, &groups).await {
        Ok(status) => status,
        Err(e) => {
            eprintln!("the drug groups' review status could not be read: {e:#}");
            let _ = tx.batch_execute("ROLLBACK TO SAVEPOINT group_status").await;
            HashMap::new()
        }
    }
}

/// The medication rows as the check reads them. A line names a moiety only when it is coded
/// under `drugref-moiety` with a code that reads as a uuid, and its threads agree on one coding:
/// a coding-conflicted group shows ONE of its codings (the display winner), so checking that one
/// alone could read a line clean whose other thread is the allergen. With no moiety a coded
/// allergy can only mark it by name, or leave it "not checked". It is current while active,
/// withheld included (D9). Pure.
pub fn med_lines(rows: &[MedicationRow]) -> Vec<MedLine<'_>> {
    rows.iter()
        .map(|r| MedLine {
            group_id: r.group_id,
            name: r.display_name(),
            term: &r.term,
            coding_display: r.coding_display.as_deref(),
            moiety: r
                .coding_code
                .as_deref()
                .filter(|_| r.coding_system.as_deref() == Some(MOIETY_SYSTEM) && !r.coding_conflict)
                .and_then(|c| c.parse().ok()),
            current: r.status == MedicationStatus::Active,
        })
        .collect()
}

/// Check `list` against allergy rows `a`, asking the drug list on `c` (`None`: there is no
/// connection to ask). A failed facts read is `DrugFacts::Unavailable` (T2R-b): the door can
/// raise for a drug list the door's owner can only partly read, and that must leave lines "not
/// checked", never fail the pane. Nothing is asked when one side has no codes: every pair would
/// be unverified either way. An allergy record this terminal cannot read (`a.unreadable`) leaves
/// every unmarked current line "not checked" on either path (#1059).
pub(crate) async fn check_list(
    c: Option<&impl GenericClient>,
    a: &PatientAllergies,
    list: &PatientMedicationList,
) -> HashMap<Uuid, LineCheck> {
    let lines = med_lines(&list.rows);
    let r = resolve(a);
    let (allergens, meds) = codes_to_ask(&r, &lines);
    let facts = match c {
        Some(c) if !allergens.is_empty() && !meds.is_empty() => {
            read_drug_facts(c, &allergens, &meds)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("the drug list's allergy answers could not be read: {e:#}");
                    DrugFacts::Unavailable
                })
        }
        _ => DrugFacts::Unavailable,
    };
    check_medications(&r, &lines, &facts, a.unreadable > 0)
}

/// The allergy rows the medication check may use: those of `read`, when it worked AND was read
/// over the same chart set as the medication list (`list_charts`). The two are read in separate
/// snapshots, so a link or unlink can land between them; rows over another set could miss a
/// chart whose lines are on screen and let them read clean. Either failure is
/// [`ALLERGIES_NOT_READ`], logged with its reason. Pure.
pub(crate) fn allergies_for_the_list(
    read: Result<(ChartSet, PatientAllergies, HashMap<Uuid, String>), ErrorView>,
    list_charts: &ChartSet,
) -> Result<PatientAllergies, String> {
    match read {
        Ok((set, allergies, _)) if &set == list_charts => Ok(allergies),
        Ok(_) => {
            eprintln!("the medication check was not run: the linked charts changed between reads");
            Err(ALLERGIES_NOT_READ.to_string())
        }
        Err(e) => {
            eprintln!(
                "the allergy list could not be read for the medication check: {}",
                e.text
            );
            Err(ALLERGIES_NOT_READ.to_string())
        }
    }
}

/// What `med_list` hands `chart_pane`: each line's check, or [`ALLERGIES_NOT_READ`] when the
/// allergy list could not be read over the list's chart set (the medication list still draws;
/// [`allergies_for_the_list`]). Fixture mode has no allergies, so nothing is said.
pub(crate) async fn med_list_check(
    state: &AppState,
    patient: Uuid,
    list: &PatientMedicationList,
) -> Result<HashMap<Uuid, LineCheck>, String> {
    let allergies = allergies_for_the_list(read_list(state, patient).await, &list.charts)?;
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
    Ok(check_list(db.as_deref(), &allergies, list).await)
}

#[cfg(test)]
#[path = "allergy_check_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "allergy_check_db_tests.rs"]
mod db_tests;
