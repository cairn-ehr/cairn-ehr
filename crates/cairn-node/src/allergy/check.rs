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
//!   `match`. One mark per allergen, worded with that allergen's most severe live account, so two
//!   differing accounts never under-warn. "One allergen" is every account linked to another by
//!   [`same_allergen_rows`], transitively (`allergen_groups`).
//! - **A note** ("Previous adverse effect: …") for an adverse effect that meets it the same way.
//!   Never a mark: an adverse effect is not an allergy. Every other entry kind, including a
//!   peer's newer word this build does not know, counts as an allergy (over-warning).
//! - **"Not checked"** (D8) when the line has no mark and some live ALLERGY could not be
//!   verified against it: either side uncoded, no drug list, an `unplaced` answer (any reason,
//!   ruling R1), or no answer at all — or the patient holds an allergy record this terminal
//!   cannot read at all (#1059: it has no head, but it may be the allergen). Only `no-match` is a verified negative (R2). Adverse
//!   effects never make a line "not checked" (R3).
//!
//! Every live head counts (D9): a doubted chart's and a suspect key's too. Ceased lines are never
//! checked. Every allergen kind except `other` takes part, a peer's newer kind (`biologic`) too:
//! the remote door admits any word, and dropping one would let its line read clean. An `other`
//! allergy (latex) never touches the medication list.
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
/// Appended to the expanded-list line of an entry coded to a group not clinically reviewed (D11).
pub const NOT_REVIEWED: &str = " (drug group not clinically reviewed)";

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
    /// Confirmed allergy hits, worded, in the order of each allergen's first hitting head (the
    /// reaction shown is that allergen's most severe live account). One line's check gives at
    /// most one per allergen; a combination's [`merge`] keeps one per allergy record, so one
    /// allergen met by two components through two records can mark twice (over-warning is the
    /// safe error; a merge never drops a mark).
    pub marks: Vec<String>,
    /// One allergy record per mark, in mark order: the live head whose hit produced it (its
    /// `event_id`). What a "charting despite a listed allergy" override names (design §3.5).
    pub against: Vec<Uuid>,
    /// Previous adverse effects that meet the line, worded.
    pub notes: Vec<String>,
    /// The small "not checked" tag.
    pub unverified: bool,
}

/// How one (allergy head, line) pair came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pair<'f> {
    /// The same drug: by name, by code, or a drug-list path whose kind is not `same-group`.
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

/// The live heads that take part (every allergen kind but `other`), most severe first (the expanded list's order: reaction rank, newest,
/// event id), so the first head of an allergen met below is its most severe account.
fn drug_heads<'a>(r: &Resolved<'a>) -> Vec<&'a AllergyRow> {
    let mut heads: Vec<&AllergyRow> = r
        .live
        .iter()
        .map(|h| h.row)
        .filter(|row| row.allergen_kind != "other")
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

/// Which heads are one allergen: each head's label is the smallest index of its connected
/// component under [`same_allergen_rows`]. That relation is not transitive ("Augmentin" uncoded
/// and "Augmentin" coded share a term; the coded one and "amoxicillin" share a code), so the
/// labels are merged until nothing changes. `heads` is most severe first, so a label is also the
/// index of its component's most severe account. Quadratic, over one patient's live heads.
fn allergen_groups(heads: &[&AllergyRow]) -> Vec<usize> {
    let mut label: Vec<usize> = (0..heads.len()).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..heads.len() {
            for j in 0..i {
                if label[i] != label[j] && same_allergen_rows(heads[i], heads[j]) {
                    let (keep, gone) = (label[i].min(label[j]), label[i].max(label[j]));
                    label
                        .iter_mut()
                        .filter(|l| **l == gone)
                        .for_each(|l| *l = keep);
                    changed = true;
                }
            }
        }
    }
    label
}

/// One set of words per allergen ([`allergen_groups`]), each beside the `event_id` of the head
/// whose hit produced it. `heads` is most severe first and `pairs` follows it, so an allergen's
/// first hit is its most severe hit (its term and coding pick the sentence); `worst` is the
/// allergen's most severe account, hit or not.
fn per_allergen(
    heads: &[&AllergyRow],
    pairs: &[Pair],
    words: impl Fn(Pair, &AllergyRow, &AllergyRow) -> String,
) -> Vec<(String, Uuid)> {
    let group = allergen_groups(heads);
    let mut said: HashSet<usize> = HashSet::new();
    let mut out = Vec::new();
    for (i, p) in pairs.iter().enumerate().filter(|(_, p)| p.is_hit()) {
        if said.insert(group[i]) {
            out.push((words(*p, heads[i], heads[group[i]]), heads[i].event_id));
        }
    }
    out
}

/// One current line against the sorted live drug heads. `unread_records`: the patient holds an
/// allergy record this terminal cannot read (#1059), which may be the allergen.
fn check_line(
    heads: &[&AllergyRow],
    line: &MedLine,
    facts: &DrugFacts,
    unread_records: bool,
) -> LineCheck {
    let (adverse, allergies): (Vec<&AllergyRow>, Vec<&AllergyRow>) = heads
        .iter()
        .copied()
        .partition(|h| h.kind == "adverse-effect");
    let allergy_pairs: Vec<Pair> = allergies.iter().map(|h| pair(h, line, facts)).collect();
    let adverse_pairs: Vec<Pair> = adverse.iter().map(|h| pair(h, line, facts)).collect();
    let (marks, against): (Vec<String>, Vec<Uuid>) =
        per_allergen(&allergies, &allergy_pairs, |p, hit, worst| {
            mark_words(p, hit, worst, line)
        })
        .into_iter()
        .unzip();
    let notes = per_allergen(&adverse, &adverse_pairs, |_, hit, worst| {
        let reaction = reaction_phrase(&worst.category, worst.reaction_text.as_deref());
        format!("Previous adverse effect: {} → {reaction}", hit.term)
    })
    .into_iter()
    .map(|(words, _)| words)
    .collect();
    let unverified =
        marks.is_empty() && (unread_records || allergy_pairs.contains(&Pair::Unverified));
    LineCheck {
        marks,
        against,
        notes,
        unverified,
    }
}

/// Two answers for ONE medication line, as one (charting slice 1a: a combination is checked
/// one component at a time, every part sharing the line's `group_id`). `a`'s words come first.
///
/// - A mark of `b` is kept unless its words are already there or its allergy record already has
///   a mark: one warning per allergy record, and `against` stays one record per mark.
/// - Notes are unioned by their words.
/// - "Not checked" if either part was, but only while there is no mark (ADR-0098's invariant:
///   the loud tag already sends the clinician to the allergy list).
pub fn merge(a: LineCheck, b: LineCheck) -> LineCheck {
    let mut out = a;
    for (mark, head) in b.marks.into_iter().zip(b.against) {
        if !out.marks.contains(&mark) && !out.against.contains(&head) {
            out.marks.push(mark);
            out.against.push(head);
        }
    }
    for note in b.notes {
        if !out.notes.contains(&note) {
            out.notes.push(note);
        }
    }
    out.unverified = out.marks.is_empty() && (out.unverified || b.unverified);
    out
}

/// Check every current line (D7, D8, D9). A line with nothing to say has no key. Lines that share
/// a `group_id` (a combination's components) answer as one line ([`merge`]).
///
/// `unread_records` is `PatientAllergies::unreadable > 0`: an allergy record this terminal holds
/// but cannot read has no head to pair with, yet it may be the allergen, so every current line
/// without a mark reads "not checked" (#1059) — the strip already says a record cannot be read.
///
/// A head of a kind this build does not know (neither `allergy` nor `adverse-effect`, a peer's
/// newer word) is treated as an allergy: it may mark, and it may leave a line "not checked".
/// Over-warning is the safe error.
pub fn check_medications(
    r: &Resolved,
    meds: &[MedLine],
    facts: &DrugFacts,
    unread_records: bool,
) -> HashMap<Uuid, LineCheck> {
    let heads = drug_heads(r);
    let mut out: HashMap<Uuid, LineCheck> = HashMap::new();
    // Lines sharing a group_id are the parts of one combination: merged, never overwritten, so
    // a later part can neither hide an earlier part's mark nor clear its "not checked".
    for m in meds.iter().filter(|m| m.current) {
        let part = check_line(&heads, m, facts, unread_records);
        let line = match out.remove(&m.group_id) {
            Some(earlier) => merge(earlier, part),
            None => part,
        };
        out.insert(m.group_id, line);
    }
    out.retain(|_, c| *c != LineCheck::default());
    out
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
        line.text.push_str(NOT_REVIEWED);
    }
    view
}

#[cfg(test)]
#[path = "check_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "check_unverified_tests.rs"]
mod unverified_tests;

#[cfg(test)]
#[path = "check_combination_tests.rs"]
mod combination_tests;
