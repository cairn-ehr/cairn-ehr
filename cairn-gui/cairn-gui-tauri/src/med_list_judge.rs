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
use crate::med_line_sign::{fresh_check, line_under, not_read, parts_before_the_key, the_line};
use crate::med_list::{LineArg, WarningRef};
use crate::med_list_delabel::{offers_for, DelabelOffer};
use crate::med_list_restore::{current_besides_stops, read_versions, replaces_a_restore, Versions};
use crate::med_list_versions::{charted_companions, charted_words};
use crate::state::AppState;
use crate::write_failure::chart_read_failed;
use cairn_event::medication::ALLERGY_SOURCE;
use cairn_medication_view::{ChartSet, MedicationRow};
use cairn_node::allergy::check::LineCheck;
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::view::{build_view, ListStatus};
use cairn_node::medication::draft::{self, DraftLine};
use cairn_node::medication::draft_tick::{default_tick, hand_tick_holds, TickFacts};
use cairn_node::medication::line::{read_directions, Directions};
use chrono::Offset;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// A stored line whose parts no longer read as parts (a row older than this build's shape): it is
/// never signed as something else, and the clinician types it again.
pub const LINE_UNREADABLE: &str = "This line could not be read back; type it again";

/// What a duplicate line says on the list (spec §3.4; review I2): the line it duplicates, as the
/// clinician reads it. Pure.
pub fn already_on_the_list(line: &str) -> String {
    format!("already on the list: {line}")
}

/// What one snapshot gives the list: the chart set, its allergies, the chart's current
/// single-moiety lines as `(code, the line as the list reads it)` (the duplicate guard's "already
/// on the chart"), the writer's lines, the OPENED chart's current lines a version may be set
/// aside beside as `(medication_id, moiety code)` (`med_list_versions::charted_companions`), and
/// the writer's "From:" as `(preset, detail)`; and the opened chart's lines as the "Version of
/// which line?" pick names them (`med_list_versions::charted_words`). Slice 2b: the versions the
/// restore lines name, on the opened chart (`med_list_restore::read_versions`), and each restore
/// line's own `current`, without the lines it will stop
/// (`med_list_restore::current_besides_stops`).
pub(crate) struct Fresh {
    pub set: ChartSet,
    pub allergies: PatientAllergies,
    pub current: Vec<(String, String)>,
    pub lines: Vec<DraftLine>,
    pub charted: Vec<(Uuid, String)>,
    pub from: Option<(Option<String>, Option<String>)>,
    pub charted_words: Vec<(Uuid, String)>,
    pub versions: Versions,
    pub beside: BTreeMap<Uuid, Vec<(String, String)>>,
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
            charted: vec![],
            from: None,
            charted_words: vec![],
            versions: Versions::new(),
            beside: BTreeMap::new(),
        });
    };
    let mut db = db.lock().await?;
    cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let list = cairn_node::medication::read::list_patient_medications(tx, patient).await?;
        let allergies = cairn_node::allergy::read::list_patient_allergies(tx, patient).await?;
        let lines = draft::read_draft_lines(tx, patient, writer).await?;
        let from = draft::read_draft_from(tx, patient, writer).await?;
        let versions = read_versions(tx, patient, &lines).await?;
        Ok(Fresh {
            current: current_lines(&list.rows),
            charted: charted_companions(&list.rows, patient),
            charted_words: charted_words(&list.rows, patient),
            beside: current_besides_stops(&list.rows, &lines, &versions),
            versions,
            from,
            set: list.charts,
            allergies,
            lines,
        })
    })
    .await
    .map_err(|e| chart_read_failed(db.explain(e)))
}

/// The terminal holding the list when it is not this one (`terminal`; `None` while the id is not
/// yet read: then no line is this terminal's). Pure.
pub(crate) fn elsewhere(judged: &[Judged], terminal: Option<&str>) -> Option<String> {
    judged
        .iter()
        .map(|j| &j.line.terminal_id)
        .find(|t| Some(t.as_str()) != terminal)
        .cloned()
}

/// The chart's CURRENT single-drug lines as `(moiety code, canonically spelled; the line's name
/// as the list shows it)`: what a draft line duplicates. A coding-conflicted group names no
/// moiety (`med_lines`' rule). Pure.
pub(crate) fn current_lines(rows: &[MedicationRow]) -> Vec<(String, String)> {
    crate::allergy_check::med_lines(rows)
        .into_iter()
        .filter(|l| l.current)
        .filter_map(|l| l.moiety.map(|m| (m.to_string(), l.name.to_string())))
        .collect()
}

/// The line that draft line `i` duplicates, as the clinician reads it: another line of the list
/// on the same moiety (its text as typed), else a current chart line on it (its name), else
/// `None`. `codes[i]` is `Some` only for a signable single moiety-coded line, so uncoded lines and
/// combinations never duplicate (Task 2's contract). Pure.
pub(crate) fn duplicate_of(
    i: usize,
    codes: &[Option<&str>],
    texts: &[&str],
    current: &[(String, String)],
) -> Option<String> {
    let code = codes[i]?;
    let draft = (0..codes.len()).find(|&j| j != i && codes[j] == Some(code));
    draft.map(|j| texts[j].to_string()).or_else(|| {
        current
            .iter()
            .find(|(c, _)| c == code)
            .map(|(_, name)| name.clone())
    })
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
    /// The line it duplicates, as the clinician reads it ([`duplicate_of`]).
    pub duplicate_of: Option<String>,
    pub default_ticked: bool,
    /// The same-drug de-label offers ([`offers_for`]); empty unless the allergies were read over
    /// the set on screen and the line is signable.
    pub offers: Vec<DelabelOffer>,
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
    let duplicates: Vec<(bool, Option<String>)> = {
        let codes: Vec<Option<&str>> = decoded
            .iter()
            // A set-aside version is signed beside its companion, so it never counts as a duplicate.
            .map(|(l, _, s)| l.set_aside_of.is_none().then(|| moiety_of(s)).flatten())
            .collect();
        let texts: Vec<&str> = decoded
            .iter()
            .map(|(l, _, _)| l.line_text.as_str())
            .collect();
        // A line is a duplicate exactly when it duplicates something (`duplicate_of`); a restore
        // line is judged against the chart without the lines it will stop (Review Focus 2).
        (0..codes.len())
            .map(|i| {
                let line = decoded[i].0.line_id;
                let current = fresh.beside.get(&line).unwrap_or(&fresh.current);
                let of = duplicate_of(i, &codes, &texts, current);
                (of.is_some(), of)
            })
            .collect()
    };
    let resolved = cairn_node::allergy::thread::resolve(&fresh.allergies);
    decoded
        .into_iter()
        .zip(duplicates)
        .map(|((line, parts, signable), (duplicate, duplicate_of))| {
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
                set_aside: line.set_aside_of.is_some(),
            })
            // Replacing a restore that was just charted must be a deliberate act (ruling R9).
            && !replaces_a_restore(&line, &fresh.versions);
            let offers = match (&signable, use_allergies) {
                (Ok((_, p)), true) => offers_for(&resolved, &fresh.allergies.doubted_charts, p),
                _ => vec![],
            };
            Judged {
                line,
                parts,
                signable,
                read,
                check,
                duplicate,
                duplicate_of,
                default_ticked,
                offers,
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

/// Whether a line's tick holds on this read: a default tick by its own derivation unless the
/// clinician vetoed it (`held`, review C1), a hand tick while every warning found now is one it
/// was given against (`hand_tick_holds`). The view shows
/// a tick only while it holds (review M1), and the Sign refuses the whole Sign when a ticked line's
/// does not (`med_list_sign.rs`, step 4). The reason rules are per-line, at Sign (ruling R3). Pure.
pub(crate) fn tick_holds(j: &Judged) -> bool {
    match j.line.hand_ticked {
        true => hand_tick_holds(&j.check, &j.line.ticked_against),
        false => j.default_ticked && !j.line.held,
    }
}

/// What a hand tick vouches for (ruling R1): the warnings BOTH found now and drawn on the line
/// when it was ticked (`seen`). One found but never drawn was never seen, so it is left out, and
/// the tick fails at Sign; one drawn but gone now needs no vouching. Pure.
pub(crate) fn vouched(now: &[Uuid], seen: &[WarningRef]) -> Vec<(String, Uuid)> {
    now.iter()
        .filter(|id| {
            seen.iter()
                .any(|w| w.source == ALLERGY_SOURCE && w.reference.parse() == Ok(**id))
        })
        .map(|id| (ALLERGY_SOURCE.to_string(), *id))
        .collect()
}

/// The warnings on `line` now: a fresh check over the set on screen. Any failure is none, which
/// can only make the tick fail at Sign (it needs every warning seen).
pub(crate) async fn warnings_now(
    state: &AppState,
    patient: Uuid,
    charts: &[String],
    line: &LineArg,
) -> Vec<Uuid> {
    let Ok((term, parts)) = parts_before_the_key(&line.parts, &line.typed_drug) else {
        return vec![];
    };
    let lines = the_line(&line.typed_drug, &term, &parts);
    fresh_check(state, patient, charts, &lines)
        .await
        .map(|check| check.against)
        .unwrap_or_default()
}

/// Clear the hand tick of each line in `lines` that has one, through the save door (the same
/// content with `hand_ticked = false`): a tick that no longer holds must not survive to the next
/// press. A default tick is derived, never stored, so there is nothing to clear. A failure is
/// logged: the next Sign re-derives the tick and refuses it again.
pub(crate) async fn untick(
    state: &AppState,
    patient: Uuid,
    writer: Option<&str>,
    lines: &[&Judged],
) {
    let Some(db) = state.db.as_ref() else { return };
    let db = match db.lock().await {
        Ok(db) => db,
        Err(e) => return eprintln!("the changed lines' ticks were not cleared: {e}"),
    };
    for j in lines.iter().filter(|j| j.line.hand_ticked) {
        let mut line = j.line.clone();
        line.hand_ticked = false;
        line.ticked_against.clear();
        match draft::save_draft_line(&*db, patient, writer, &line).await {
            Ok(a) if a == "saved" => {}
            Ok(a) => eprintln!("a changed line's tick was not cleared: the list answered {a}"),
            Err(e) => eprintln!(
                "a changed line's tick was not cleared: {}",
                cairn_gui_live::cause_of(&db.explain(e))
            ),
        }
    }
}
