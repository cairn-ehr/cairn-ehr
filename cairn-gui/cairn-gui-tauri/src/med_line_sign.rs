//! What one medication line signs, shared by the one-line Sign (`med_chart.rs`) and the list's
//! Sign (`med_list_sign.rs`), so the two can never disagree about a line.
//!
//! - [`parts_before_the_key`]: the term and parts a line signs, or the per-line refusal (a blank
//!   name, a pick that is not a drug), said before any key is asked for.
//! - [`the_line`] / [`line_under`]: the line as the allergy check reads it.
//! - [`fresh_check`] / [`fresh_allergies`]: the check re-run on a fresh read over the chart set on
//!   screen (the webview's earlier answer is never trusted).
//! - [`override_refs`], [`single_coding`], [`components`], [`charting`]: the pure assembly of the
//!   signed event's charting fields from a checked line.
//!
//! Moved here from `med_chart.rs` unchanged in behaviour when the list's Sign became the second
//! caller (batch charting slice 1); `med_chart.rs` re-imports them, so its tests read them as
//! before.
use crate::allergies::read_list;
use crate::allergy_check::{canonical_code, check_lines, part_line, CodingArg, ALLERGIES_NOT_READ};
use crate::chart_set::{check_displayed_set, ChartSetError};
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::{PartArg, NAME_IT, NOT_A_LISTED_DRUG};
use crate::med_chart_write::substance;
use crate::state::AppState;
use cairn_event::medication::{
    ChartedAs, Charting, Component, OverrideRef, SubstanceCoding, WarningOverride, ALLERGY_SOURCE,
};
use cairn_medication_view::ChartSet;
use cairn_node::allergy::check::{LineCheck, MedLine, MOIETY_SYSTEM};
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::medication::line::{term_to_sign, Directions};
use std::collections::HashMap;
use uuid::Uuid;

/// The group id the line being charted is checked under: it has no thread yet.
pub(crate) const THE_LINE: Uuid = Uuid::nil();

/// The line's term and its parts as they will be signed, or the refusal, said before the key.
/// Each pick must be a drug ([`NOT_A_LISTED_DRUG`]) with a code that reads as a uuid (signed in
/// its canonical spelling, `canonical_code`). One part's term is `term_to_sign(typed_drug, its
/// pick)` ("amox" picked as amoxicillin signs "amoxicillin"); a combination's term is its parts'
/// terms joined " + " (R22), each part's being `term_to_sign` of its own words ("amox + clav"
/// with both picked signs "amoxicillin + clavulanic acid"; an unpicked part keeps its words). No parts sent is
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
    // A combination signs its parts' signed terms ("amoxicillin + clavulanic acid", R22).
    let term = match parts.as_slice() {
        [one] => one.term.clone(),
        _ => parts
            .iter()
            .map(|p| p.term.as_str())
            .collect::<Vec<_>>()
            .join(" + "),
    };
    if term.is_empty() {
        return Err(refused(NAME_IT));
    }
    Ok((term, parts))
}

/// A pick as it will be signed: refused unless it is a drug, its code re-spelled. Pure.
fn picked(c: CodingArg) -> Result<CodingArg, ErrorView> {
    // A blank display fails db/041's coding check at both doors — after the key, so refused here.
    if c.system != MOIETY_SYSTEM || c.display.trim().is_empty() {
        return Err(refused(NOT_A_LISTED_DRUG));
    }
    let code = canonical_code(&c)?;
    Ok(CodingArg { code, ..c })
}

/// The line being charted as the check reads it: one line per part under [`THE_LINE`], each
/// carrying a line term (ruling R12). One part's line term is its signed `term` (R16). A
/// combination is checked under BOTH its signed term ("amoxicillin + clavulanic acid", R22 —
/// what the list re-checks the row under once drawn) and the drug AS TYPED ("Augmentin", "amox +
/// clav"), so a by-name allergy on either marks the line here as it would on the list (R24).
/// Every line shares the group, so `merge` folds them into one answer; over-warning is the safe
/// error. Both `med_line_check` and Sign call this, so the two checks agree (R16). Pure.
pub(crate) fn the_line<'a>(
    typed_drug: &'a str,
    term: &'a str,
    parts: &'a [PartArg],
) -> Vec<MedLine<'a>> {
    line_under(THE_LINE, typed_drug, term, parts)
}

/// [`the_line`] under group `group`: the list checks all its lines in one pass, each under its
/// own line id, so one line's answer never folds into another's. Pure.
pub(crate) fn line_under<'a>(
    group: Uuid,
    typed_drug: &'a str,
    term: &'a str,
    parts: &'a [PartArg],
) -> Vec<MedLine<'a>> {
    let typed = typed_drug.trim();
    let mut terms = vec![term];
    if parts.len() > 1 && typed != term {
        terms.push(typed);
    }
    terms
        .into_iter()
        .flat_map(|line_term| {
            parts.iter().map(move |p| {
                let coding = p
                    .coding
                    .as_ref()
                    .map(|c| (c.system.as_str(), c.code.as_str(), c.display.as_str()));
                part_line(group, line_term, &p.term, coding, true)
            })
        })
        .collect()
}

/// `lines` checked against the allergies of `patient`, read now, over the chart set the window
/// showed (`charts`) — see [`fresh_allergies`] for the refusal and the "not checked" case.
pub(crate) async fn fresh_check(
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
pub(crate) fn not_read(why: &str) -> LineCheck {
    eprintln!("the line being charted was not checked against the allergies: {why}");
    LineCheck {
        notes: vec![ALLERGIES_NOT_READ.into()],
        unverified: true,
        ..LineCheck::default()
    }
}

/// The fresh check's warnings as the override names them (its marks, and its adverse effects at
/// a lowered threshold), each by its allergy record's event id (`ids`: those ids as text, owned
/// by the caller). Pure.
pub(crate) fn override_refs(ids: &[String]) -> Vec<OverrideRef<'_>> {
    ids.iter()
        .map(|id| OverrideRef {
            source: ALLERGY_SOURCE,
            reference: id,
        })
        .collect()
}

/// One part is the line's own coding; two or more are its components, never both. Pure.
pub(crate) fn single_coding(parts: &[PartArg]) -> Option<SubstanceCoding<'_>> {
    match parts {
        [one] => one.coding.as_ref().map(substance),
        _ => None,
    }
}

/// A combination's components (empty for one part, see [`single_coding`]). Pure.
pub(crate) fn components(parts: &[PartArg]) -> Vec<Component<'_>> {
    match parts.len() {
        0 | 1 => vec![],
        _ => parts
            .iter()
            .map(|p| Component {
                term: &p.term,
                coding: p.coding.as_ref().map(substance),
            })
            .collect(),
    }
}

/// The charting fields the line signs: the press, the route and strength read from the
/// directions, the components, the warning override when a reason was required, and `stat` when
/// the line is a stat ORDER (`med_stat::is_stat_order`) — set here, so both Sign paths sign it.
/// Pure.
pub(crate) fn charting<'a>(
    charted_as: ChartedAs,
    d: &'a Directions,
    components: &'a [Component<'a>],
    reason: Option<&'a str>,
    against: &'a [OverrideRef<'a>],
) -> Charting<'a> {
    Charting {
        charted_as: Some(charted_as),
        route: d.route.as_deref(),
        strength: d.strength.as_deref(),
        components,
        warning_override: reason.map(|reason| WarningOverride { reason, against }),
        restores: None,
        stat: crate::med_stat::is_stat_order(charted_as, d),
    }
}
