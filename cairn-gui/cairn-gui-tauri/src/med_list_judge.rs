//! The medication list's judging (batch charting slice 1), shared by the list's read
//! (`med_list.rs`) and its Sign (`med_list_sign.rs`) so the screen and the Sign judge a line by the
//! same rules: one snapshot read of the lines and what they are judged against ([`fresh_read`]),
//! then each line's per-line refusal, check, duplicate and default tick ([`judge`]). The tick rules
//! themselves are `cairn_node::medication::draft_tick`'s (pure); this file only gathers their facts.
use crate::allergies::read_list;
use crate::allergy_check::check_lines;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::PartArg;
use crate::med_line_sign::{line_under, not_read, parts_before_the_key};
use crate::state::AppState;
use crate::write_failure::chart_read_failed;
use cairn_medication_view::{ChartSet, MedicationRow};
use cairn_node::allergy::check::LineCheck;
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::view::{build_view, ListStatus};
use cairn_node::medication::draft::{self, DraftLine};
use cairn_node::medication::draft_tick::{
    default_tick, duplicate_codes, hand_tick_holds, TickFacts,
};
use cairn_node::medication::line::{read_directions, Directions};
use chrono::Offset;
use std::collections::HashMap;
use uuid::Uuid;

/// A stored line whose parts no longer read as parts (a row older than this build's shape): it is
/// never signed as something else, and the clinician types it again.
pub const LINE_UNREADABLE: &str = "This line could not be read back; type it again";

/// What one snapshot gives the list: the chart set, its allergies, the chart's current
/// single-moiety codes (the duplicate guard's "already on the chart"), and the writer's lines.
pub(crate) struct Fresh {
    pub set: ChartSet,
    pub allergies: PatientAllergies,
    pub current: Vec<String>,
    pub lines: Vec<DraftLine>,
}

/// [`Fresh`], read in ONE snapshot, so a line is never judged against allergies or a chart from
/// another moment than the lines themselves. Fixture mode has no list: no lines.
pub(crate) async fn fresh_read(
    state: &AppState,
    patient: Uuid,
    writer: Option<&str>,
) -> Result<Fresh, ErrorView> {
    let Some(db) = state.db.as_ref() else {
        let (set, allergies, _) = read_list(state, patient).await?;
        return Ok(Fresh {
            set,
            allergies,
            current: vec![],
            lines: vec![],
        });
    };
    let mut db = db.lock().await?;
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let list = cairn_node::medication::read::list_patient_medications(tx, patient).await?;
        let allergies = cairn_node::allergy::read::list_patient_allergies(tx, patient).await?;
        let lines = draft::read_draft_lines(tx, patient, writer).await?;
        Ok(Fresh {
            current: current_codes(&list.rows),
            set: list.charts,
            allergies,
            lines,
        })
    })
    .await
    .map_err(|e| chart_read_failed(db.explain(e)))
}

/// The moiety codes of the chart's CURRENT single-drug lines, canonically spelled: what a draft
/// line duplicates. A coding-conflicted group names no moiety (`med_lines`' rule). Pure.
pub(crate) fn current_codes(rows: &[MedicationRow]) -> Vec<String> {
    crate::allergy_check::med_lines(rows)
        .into_iter()
        .filter(|l| l.current)
        .filter_map(|l| l.moiety.map(|m| m.to_string()))
        .collect()
}

/// A stored line, its parts as stored, and what they sign (or the per-line refusal).
type Decoded = (
    DraftLine,
    Vec<PartArg>,
    Result<(String, Vec<PartArg>), ErrorView>,
);

/// One draft line, re-derived on a fresh read: what it would sign (or its per-line refusal), its
/// directions read, its check, and its ticks.
#[derive(Debug)]
pub(crate) struct Judged {
    pub line: DraftLine,
    pub parts: Vec<PartArg>,
    pub signable: Result<(String, Vec<PartArg>), ErrorView>,
    pub read: Directions,
    pub check: LineCheck,
    pub duplicate: bool,
    pub default_ticked: bool,
}

/// Judge every line of `fresh`. `use_allergies`: the allergies were read over the set on screen
/// (else every line is "not checked"). `named_writer`: the list has a writer (else no line is
/// default-ticked).
pub(crate) async fn judge(
    state: &AppState,
    fresh: Fresh,
    use_allergies: bool,
    named_writer: bool,
) -> Vec<Judged> {
    let decoded: Vec<_> = fresh.lines.into_iter().map(decode).collect();
    let mut checks = match use_allergies {
        true => check_all(state, &fresh.allergies, &decoded).await,
        false => Default::default(),
    };
    let status = match use_allergies {
        true => build_view(&fresh.allergies, chrono::Local::now().offset().fix()).drug,
        false => ListStatus::NotYetAsked,
    };
    let duplicates: Vec<bool> = {
        let codes: Vec<Option<&str>> = decoded.iter().map(|(_, _, s)| moiety_of(s)).collect();
        let current: Vec<&str> = fresh.current.iter().map(String::as_str).collect();
        let dups = duplicate_codes(&codes, &current);
        codes
            .iter()
            .map(|c| c.is_some_and(|c| dups.contains(c)))
            .collect()
    };
    decoded
        .into_iter()
        .zip(duplicates)
        .map(|((line, parts, signable), duplicate)| {
            let check = match (use_allergies, signable.is_ok()) {
                // The check leaves a clean line out of its answer (`check_medications`).
                (true, true) => checks.remove(&line.line_id).unwrap_or_default(),
                // A line it was never given (it cannot be read as a drug) is never clean.
                (true, false) => not_checked(),
                (false, _) => not_read("the linked charts changed since the list was read"),
            };
            let read = read_directions(&line.directions);
            let all_parts_coded = signable
                .as_ref()
                .is_ok_and(|(_, p)| !p.is_empty() && p.iter().all(|p| p.coding.is_some()));
            let default_ticked = default_tick(&TickFacts {
                finished: line.finished,
                all_parts_coded,
                directions: &read,
                check: &check,
                drug_allergies: &status,
                duplicate,
                named_writer,
            });
            Judged {
                line,
                parts,
                signable,
                read,
                check,
                duplicate,
                default_ticked,
            }
        })
        .collect()
}

/// A stored line's parts, and what they sign ([`parts_before_the_key`]). Unreadable parts are
/// [`LINE_UNREADABLE`], never a guess. Pure.
fn decode(line: DraftLine) -> Decoded {
    match serde_json::from_value::<Vec<PartArg>>(line.parts.clone()) {
        Ok(parts) => {
            let signable = parts_before_the_key(&parts, &line.typed_drug);
            (line, parts, signable)
        }
        Err(_) => (line, vec![], Err(refused(LINE_UNREADABLE))),
    }
}

/// The moiety a signable single-drug line is coded to (`None`: uncoded, a combination, or not
/// signable). `parts_before_the_key` admits only `drugref-moiety` picks, canonically spelled, so
/// this is the duplicate guard's `Some(code)` (Task 2's contract). Pure.
fn moiety_of(signable: &Result<(String, Vec<PartArg>), ErrorView>) -> Option<&str> {
    match signable {
        Ok((_, parts)) => match parts.as_slice() {
            [one] => one.coding.as_ref().map(|c| c.code.as_str()),
            _ => None,
        },
        Err(_) => None,
    }
}

/// Every signable line checked in one pass, each under its own line id, on the drug list when it
/// answers (never inside the snapshot: the drug-list door may swallow a failure, which would abort
/// a shared transaction, ruling R13).
async fn check_all(
    state: &AppState,
    allergies: &PatientAllergies,
    decoded: &[Decoded],
) -> HashMap<Uuid, LineCheck> {
    let lines: Vec<_> = decoded
        .iter()
        .filter_map(|(l, _, s)| s.as_ref().ok().map(|(term, parts)| (l, term, parts)))
        .flat_map(|(l, term, parts)| line_under(l.line_id, &l.typed_drug, term, parts))
        .collect();
    let db = match state.db.as_ref() {
        Some(db) => match db.lock().await {
            Ok(db) => Some(db),
            Err(e) => {
                eprintln!("the drug list was not asked for the list: the connection was lost: {e}");
                None
            }
        },
        None => None,
    };
    check_lines(db.as_deref(), allergies, &lines).await
}

/// "Not checked", for a line the check was never given (it cannot be read as a drug). Pure.
fn not_checked() -> LineCheck {
    LineCheck {
        unverified: true,
        ..LineCheck::default()
    }
}

/// Whether a line's tick holds on this read: a default tick by its own derivation, a hand tick
/// while every warning found now is one it was given against (`hand_tick_holds`). The view shows
/// a tick only while it holds (review M1), and the Sign refuses the whole Sign when a ticked line's
/// does not (`med_list_sign.rs`, step 4). The reason rules are per-line, at Sign (ruling R3). Pure.
pub(crate) fn tick_holds(j: &Judged) -> bool {
    match j.line.hand_ticked {
        true => hand_tick_holds(&j.check, &j.line.ticked_against),
        false => j.default_ticked,
    }
}
