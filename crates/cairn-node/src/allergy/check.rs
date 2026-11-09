//! The medication check (allergy slice 3): every CURRENT medication line read against the
//! patient's LIVE drug allergies. Pure: no database, no clock. The caller reads the allergy rows,
//! folds them ([`resolve`](super::thread::resolve)), asks the drug read door about the codes
//! [`codes_to_ask`] names (`drug_facts::read_drug_facts`), and hands everything here.
//!
//! What a line can say (M6), and why each rule leans the way it does — a silent miss here is a
//! patient-safety defect, so over-warning is the safe error and an unverified line must never
//! read as checked:
//!
//! - **A mark** (the loud "Allergy" tag plus words) for each allergen that meets the line (D7):
//!   by name, by identical code (works with no drug list), or because the drug list answered
//!   `match`. One mark per allergen ([`same_allergen_rows`]), worded with that allergen's most
//!   severe live account, so two differing accounts never under-warn.
//! - **A note** ("Previous adverse effect: …") for an adverse effect that meets it the same way.
//!   Never a mark: an adverse effect is not an allergy.
//! - **"Not checked"** (D8) when the line has no mark and some live ALLERGY could not be
//!   verified against it: either side uncoded, no drug list, an `unplaced` answer (any reason,
//!   ruling R1), or no answer at all. Only `no-match` is a verified negative (R2). Adverse
//!   effects never make a line "not checked" (R3).
//!
//! Every live head counts (D9): a doubted chart's and a suspect key's too. Ceased lines are never
//! checked. Only `drug` entries take part: a latex allergy never touches the medication list.
//!
//! Also here: the D11 review status of a coded drug GROUP, said on the expanded allergy list only
//! ([`unreviewed_groups`], [`note_unreviewed_groups`]; M7 — the strip and the medication line stay
//! terse).
use super::drug_facts::{Answer, DrugFacts, MatchPath};
use super::read::{AllergyRow, PatientAllergies};
use super::thread::{same_allergen, same_allergen_rows, Resolved};
use super::view::{order_key, AllergyView};
use cairn_event::allergy::reaction_phrase;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The coding system of a single drug (a moiety) in the drug list.
pub const MOIETY_SYSTEM: &str = "drugref-moiety";
/// The coding system of a drug group (an allergy group) in the drug list (M2).
pub const GROUP_SYSTEM: &str = "drugref-class";

/// One medication line, as the check needs it (the caller builds it from a `MedicationRow`).
#[derive(Debug, Clone, Copy)]
pub struct MedLine<'a> {
    /// The line's key in the answer.
    pub group_id: Uuid,
    /// What the clinician reads the line as (`MedicationRow::display_name()`).
    pub name: &'a str,
    /// The name as typed.
    pub term: &'a str,
    /// The display of the drug it was coded to, when it was.
    pub coding_display: Option<&'a str>,
    /// The code, when the line is coded to a moiety ([`MOIETY_SYSTEM`]).
    pub moiety: Option<Uuid>,
    /// Active or withheld; a ceased line is never checked.
    pub current: bool,
}

/// What one line says. Invariant: `unverified` is false whenever `marks` is non-empty (the loud
/// tag already sends the clinician to the allergy list).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LineCheck {
    /// Confirmed allergy hits, worded, most severe first.
    pub marks: Vec<String>,
    /// Previous adverse effects that meet the line, worded.
    pub notes: Vec<String>,
    /// The small "not checked" tag.
    pub unverified: bool,
}

/// How one (allergy head, line) pair came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pair<'f> {
    /// The same drug: by name, by code, or a drug-list path that is not a named group.
    SameDrug,
    /// The drug list put both in one group (its name, when the list gave one).
    SameGroup(Option<&'f str>),
    /// The drug list checked the pair and they do not meet (a verified negative).
    Clear,
    /// Nothing verified the pair either way.
    Unverified,
}

impl Pair<'_> {
    fn is_hit(self) -> bool {
        matches!(self, Pair::SameDrug | Pair::SameGroup(_))
    }
}

/// The code a row is coded to under `system`, when it is and the code reads as a uuid.
fn code_under(row: &AllergyRow, system: &str) -> Option<Uuid> {
    let c = row.coding.as_ref().filter(|c| c.system == system)?;
    c.code.parse().ok()
}

/// The code to ask the drug list about: a moiety's or a group's.
fn drug_code(row: &AllergyRow) -> Option<Uuid> {
    code_under(row, MOIETY_SYSTEM).or_else(|| code_under(row, GROUP_SYSTEM))
}

/// The live DRUG heads, most severe first (the expanded list's order: reaction rank, newest,
/// event id), so the first head of an allergen met below is its most severe account.
fn drug_heads<'a>(r: &Resolved<'a>) -> Vec<&'a AllergyRow> {
    let mut heads: Vec<&AllergyRow> = r
        .live
        .iter()
        .map(|h| h.row)
        .filter(|row| row.allergen_kind == "drug")
        .collect();
    heads.sort_by_key(|row| order_key(row));
    heads
}

/// What to ask the drug read door: the live drug heads' codes and the current lines' moieties,
/// each sorted and deduplicated.
pub fn codes_to_ask(r: &Resolved, meds: &[MedLine]) -> (Vec<Uuid>, Vec<Uuid>) {
    let mut allergens: Vec<Uuid> = drug_heads(r).into_iter().filter_map(drug_code).collect();
    let mut moieties: Vec<Uuid> = meds
        .iter()
        .filter(|m| m.current)
        .filter_map(|m| m.moiety)
        .collect();
    for v in [&mut allergens, &mut moieties] {
        v.sort();
        v.dedup();
    }
    (allergens, moieties)
}

/// Ruling T4-a: any path that is not exactly `same-group` (`same-drug`, an empty word, a newer
/// word) is the stronger claim and reads as the same drug; otherwise the first named group.
fn match_pair(paths: &[MatchPath]) -> Pair<'_> {
    if paths.iter().any(|p| p.kind != "same-group") {
        return Pair::SameDrug;
    }
    Pair::SameGroup(paths.iter().find_map(|p| p.group_name.as_deref()))
}

/// D7 and D8 for one pair. A name or an identical code is a hit with no drug list at all; then
/// the drug list's answer, when both sides are coded and it gave one; anything else is
/// unverified.
fn pair<'f>(head: &AllergyRow, line: &MedLine, facts: &'f DrugFacts) -> Pair<'f> {
    let by_name = same_allergen(&head.term, line.term)
        || line
            .coding_display
            .is_some_and(|d| same_allergen(&head.term, d));
    let by_code = line.moiety.is_some() && code_under(head, MOIETY_SYSTEM) == line.moiety;
    if by_name || by_code {
        return Pair::SameDrug;
    }
    let answer = match (facts, drug_code(head), line.moiety) {
        (DrugFacts::Available(answers), Some(allergen), Some(med)) => answers.get(&(allergen, med)),
        _ => None,
    };
    match answer {
        Some(Answer::Match(paths)) => match_pair(paths),
        Some(Answer::NoMatch) => Pair::Clear,
        Some(Answer::Unplaced) | None => Pair::Unverified,
    }
}

/// A mark's words (Global Constraints, exact): `hit` is the account that met the line (its term
/// and coding pick the sentence), `worst` the allergen's most severe live account (its reaction).
fn mark_words(pair: Pair, hit: &AllergyRow, worst: &AllergyRow, line: &MedLine) -> String {
    let reaction = reaction_phrase(&worst.category, worst.reaction_text.as_deref());
    let (name, term) = (line.name, &hit.term);
    match pair {
        Pair::SameGroup(_) if code_under(hit, GROUP_SYSTEM).is_some() => {
            format!("{name} is a {term}; allergy recorded: {reaction}")
        }
        Pair::SameGroup(Some(group)) => {
            format!("{name} is in the same group as {term} ({group}); allergy recorded: {reaction}")
        }
        _ => format!("Allergy recorded: {term} → {reaction}"),
    }
}

/// One set of words per allergen. `heads` is most severe first and `pairs` follows it, so the
/// first hit of an allergen is its most severe hit; a later hit of the same allergen adds
/// nothing. `worst` is looked up among every head given, hit or not.
fn per_allergen(
    heads: &[&AllergyRow],
    pairs: &[Pair],
    words: impl Fn(Pair, &AllergyRow, &AllergyRow) -> String,
) -> Vec<String> {
    let mut said: Vec<&AllergyRow> = Vec::new();
    let mut out = Vec::new();
    for (hit, p) in heads.iter().zip(pairs).filter(|(_, p)| p.is_hit()) {
        if said.iter().any(|s| same_allergen_rows(s, hit)) {
            continue;
        }
        let worst = heads
            .iter()
            .find(|h| same_allergen_rows(h, hit))
            .unwrap_or(hit);
        said.push(hit);
        out.push(words(*p, hit, worst));
    }
    out
}

/// One current line against the sorted live drug heads.
fn check_line(heads: &[&AllergyRow], line: &MedLine, facts: &DrugFacts) -> LineCheck {
    let (adverse, allergies): (Vec<&AllergyRow>, Vec<&AllergyRow>) = heads
        .iter()
        .copied()
        .partition(|h| h.kind == "adverse-effect");
    let allergy_pairs: Vec<Pair> = allergies.iter().map(|h| pair(h, line, facts)).collect();
    let adverse_pairs: Vec<Pair> = adverse.iter().map(|h| pair(h, line, facts)).collect();
    let marks = per_allergen(&allergies, &allergy_pairs, |p, hit, worst| {
        mark_words(p, hit, worst, line)
    });
    let notes = per_allergen(&adverse, &adverse_pairs, |_, hit, worst| {
        let reaction = reaction_phrase(&worst.category, worst.reaction_text.as_deref());
        format!("Previous adverse effect: {} → {reaction}", hit.term)
    });
    let unverified = marks.is_empty() && allergy_pairs.contains(&Pair::Unverified);
    LineCheck {
        marks,
        notes,
        unverified,
    }
}

/// Check every current line (D7, D8, D9). A line with nothing to say has no key.
///
/// A head of a kind this build does not know (neither `allergy` nor `adverse-effect`, a peer's
/// newer word) is treated as an allergy: it may mark, and it may leave a line "not checked".
/// Over-warning is the safe error.
pub fn check_medications(
    r: &Resolved,
    meds: &[MedLine],
    facts: &DrugFacts,
) -> HashMap<Uuid, LineCheck> {
    let heads = drug_heads(r);
    meds.iter()
        .filter(|m| m.current)
        .map(|m| (m.group_id, check_line(&heads, m, facts)))
        .filter(|(_, c)| *c != LineCheck::default())
        .collect()
}

/// D11: the live heads' drug GROUP codes whose review status is not `reviewed`. A group the
/// status read did not return (unknown to the list, or the read failed and the map is empty)
/// and a status word this build does not know both read as not reviewed.
pub fn unreviewed_groups(r: &Resolved, status: &HashMap<Uuid, String>) -> HashSet<Uuid> {
    r.live
        .iter()
        .filter_map(|h| code_under(h.row, GROUP_SYSTEM))
        .filter(|g| status.get(g).map(String::as_str) != Some("reviewed"))
        .collect()
}

/// M7: append the not-reviewed suffix to the expanded-list line of every live entry coded to a
/// group in `unreviewed`. The strip was composed before this runs and is left as it is.
pub fn note_unreviewed_groups(
    mut view: AllergyView,
    a: &PatientAllergies,
    unreviewed: &HashSet<Uuid>,
) -> AllergyView {
    let flagged: HashSet<Uuid> = a
        .entries
        .iter()
        .filter(|e| code_under(e, GROUP_SYSTEM).is_some_and(|g| unreviewed.contains(&g)))
        .map(|e| e.event_id)
        .collect();
    let lines = view
        .drug_allergies
        .iter_mut()
        .chain(view.other_allergies.iter_mut())
        .chain(view.adverse_effects.iter_mut());
    for line in lines.filter(|l| flagged.contains(&l.event_id)) {
        // Built with `format!`: clinician words that name the drug list are prose, which
        // no_drugref_dependency.rs admits only inside a format-family macro's arguments.
        line.text = format!(
            "{} (group from drugref, not clinically reviewed)",
            line.text
        );
    }
    view
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;

/// The two plan-named tests whose NAMES say "drugref". They live in this file's
/// `#[cfg(test)]` tail, not in `check_tests.rs`: `no_drugref_dependency.rs` blanks a src file's
/// `#[cfg(test)] mod` tail but scans a `#[path]`-included test file whole, and a test's name is
/// not one of its exempt places.
#[cfg(test)]
mod no_drug_list_tests {
    use super::tests::{amox, marks, med, not_checked, only, run, AMOX, AMOX_SODIUM};
    use super::*;
    use crate::allergy::test_fixtures::{coded, entry, id};

    #[test]
    fn same_code_marks_the_line_without_drugref() {
        let a = only(vec![coded(
            entry(1, "drug", "Augmentin", "allergy", "anaphylaxis", 1),
            MOIETY_SYSTEM,
            AMOX,
        )]);
        let got = run(
            &a,
            &[med(900, "Amoxil", Some(AMOX))],
            &DrugFacts::Unavailable,
        );
        assert_eq!(
            got[&id(900)],
            marks(&["Allergy recorded: Augmentin → anaphylaxis"])
        );
    }

    /// Review Focus 5: a salt of the allergen is a different code; with no drug list nothing can
    /// say they are one drug, so the line is never silently clear.
    #[test]
    fn a_salt_without_drugref_is_not_checked() {
        let a = only(vec![amox("anaphylaxis")]);
        let line = [med(900, "amoxicillin sodium", Some(AMOX_SODIUM))];
        assert_eq!(
            run(&a, &line, &DrugFacts::Unavailable)[&id(900)],
            not_checked()
        );
    }
}
