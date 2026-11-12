//! The same-drug de-label offer rule (charting slice 1b): which live allergy records a medication
//! line may offer to de-label. Pure.
//!
//! WHY code equality only (design section 3.1): the allergy WARNING may match on a name or a drug
//! group, because a false warning costs a glance. A de-label LIFTS an allergy, so an offer on a
//! guess could clear a true one. The offer therefore needs the line's moiety code and the
//! allergy's moiety code to be the same drug, nothing looser.
use super::check::MOIETY_SYSTEM;
use super::read::AllergyRow;
use super::thread::Resolved;
use uuid::Uuid;

/// The live drug-ALLERGY heads coded to exactly `moiety` ([`MOIETY_SYSTEM`], code equal as a
/// `Uuid`), excluding heads on `doubted` charts. Never a name, group or drug-list match: a de-label
/// offered on a guess lifts a true allergy. In `r.live` order.
///
/// `r.live` already leaves out de-labelled (historical) and withdrawn heads. Kind must be exactly
/// "allergy": an adverse effect is not an allergy, and an unknown kind from a newer peer is NOT
/// offered (the warning side treats it as an allergy; lifting it is the unsafe direction).
pub fn same_drug_heads<'a>(
    r: &Resolved<'a>,
    doubted: &[Uuid],
    moiety: Uuid,
) -> Vec<&'a AllergyRow> {
    r.live
        .iter()
        .map(|h| h.row)
        .filter(|row| row.allergen_kind == "drug" && row.kind == "allergy")
        .filter(|row| !doubted.contains(&row.chart))
        .filter(|row| {
            row.coding.as_ref().is_some_and(|c| {
                // A code that does not parse as a Uuid never matches.
                c.system == MOIETY_SYSTEM && c.code.parse::<Uuid>().is_ok_and(|u| u == moiety)
            })
        })
        .collect()
}

/// The offer's words, e.g. "De-label the Amoxicillin allergy: tolerates amoxicillin (recorded by
/// Dr A)"; the tail is left off when the signer is unknown.
pub fn offer_words(allergy_term: &str, drug: &str, signer: Option<&str>) -> String {
    let tail = signer.map_or_else(String::new, |s| format!(" (recorded by {s})"));
    format!("De-label the {allergy_term} allergy: tolerates {drug}{tail}")
}

#[cfg(test)]
#[path = "delabel_offer_tests.rs"]
mod tests;
