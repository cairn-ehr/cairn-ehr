//! Batch charting slice 2a in the window: "From:", a line's own source and why, and signing a
//! "set aside" version with its companion (design §3.1, §3.5).
//!
//! - **"From:"** ([`med_list_from_save`]) is set once above the list: a preset LABEL (`GP letter`,
//!   …) and a free-text detail, stored per (chart, writer) by db/078 so it survives a crash. Each
//!   NEW line saved after it (no stored row yet; ruling R12), carrying no source of its own, is
//!   stamped with it ([`stamp`]): the preset's category goes to `info_source`, the free text (or,
//!   when blank, the preset's label) to `source_detail`. The category is never guessed from the
//!   text. A line typed before "From:" keeps what it stored, none included. A line may name its
//!   own source instead, and keeps it. Unset, nothing is stamped and the press's default stands
//!   (`med_chart_write::charted_input`: Start is the clinician's, Already taking the patient's).
//! - **A line's source and why** are signed into its event ([`list_input`]): the two keys of
//!   cairn-event's `SourceNote`, never one merged text.
//! - **A version** is a line marked `set_aside_of` a companion of the same coded drug: another
//!   list line (`line:<id>`) or a current line of the OPENED chart (`med:<id>`, never a linked
//!   chart's: spec §2). At Sign ([`plan_versions`]) each version's companion is re-derived from
//!   the stored mark on the fresh read (`version_holds`, ruling R9): the webview's word is never
//!   asked. A version beside a TICKED list line signs in that line's transaction, after it; beside
//!   a charted line it signs alone ([`sign_alone`]); any other stays in the list, its line saying
//!   why ([`VERSION_KEPT`]). So no version is ever in the record beside nothing (anti-goal).
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::press;
use crate::med_chart_write::charted_input;
use crate::med_line_sign::single_coding;
use crate::med_list::{med_list_read_impl, Judged, ListLineView, MedListView, FIXTURE};
use crate::med_list_sign::outcome;
use crate::state::AppState;
use crate::write_failure::{node_cannot_write, node_failure};
use cairn_event::medication::{
    ChartedAs, Charting, MedicationAssertion, SourceNote, SubstanceCoding,
};
use cairn_medication_view::{MedicationRow, MedicationStatus};
use cairn_node::allergy::check::MOIETY_SYSTEM;
use cairn_node::medication::draft::{self, DraftLine};
use cairn_node::medication::draft_versions::{
    companion_candidates, info_source_of, version_holds, Companion,
};
use cairn_node::medication::draft_versions_sign::{set_aside_draft_line, VersionToSign};
use cairn_node::medication::{AssertMedicationInput, AuthorParams};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Why a version stays in the list when its companion line was not charted (Global Constraints).
pub const VERSION_KEPT: &str = "its line was not charted";

/// The writer's "From:" as the pane draws it: the preset label and the free text.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FromView {
    pub preset: Option<String>,
    pub detail: Option<String>,
}

#[tauri::command]
pub async fn med_list_from_save(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    preset: Option<String>,
    detail: Option<String>,
    writer: Option<String>,
) -> Result<MedListView, ErrorView> {
    med_list_from_save_impl(
        &state,
        &patient_id,
        &charts,
        preset,
        detail,
        writer.as_deref(),
    )
    .await
}

/// Save "From:" under `writer`, the writer the list was READ under (`MedListView::writer`, ruling
/// R2: a pick landing after a who's-here switch still names the person whose list it was), then
/// read the list again over `charts`, the set on screen (as `med_list_read`). db/078 holds the
/// preset to its six labels (ruling R6).
pub async fn med_list_from_save_impl(
    state: &AppState,
    patient_id: &str,
    charts: &[String],
    preset: Option<String>,
    detail: Option<String>,
    writer: Option<&str>,
) -> Result<MedListView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let (preset, detail) = (given(preset), given(detail));
    {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        draft::save_draft_from(&*db, patient, writer, preset.as_deref(), detail.as_deref())
            .await
            .map_err(|e| node_failure(&db.explain(e)))?;
    }
    med_list_read_impl(state, patient_id, Some(charts)).await
}

/// A typed value, or `None` when blank: the floor refuses a blank source or why. Pure.
pub(crate) fn given(s: Option<String>) -> Option<String> {
    s.filter(|s| !s.trim().is_empty())
}

/// What "From:" stamps on a line saved with no source of its own: `(info_source, source_detail)`,
/// the detail being the free text or, when blank, the preset's label. `None` when no preset is
/// chosen (or one not in the list): then the press's default stands. Pure.
pub(crate) fn stamp(from: &FromView) -> Option<(&'static str, String)> {
    let preset = from.preset.as_deref()?;
    let category = info_source_of(Some(preset))?;
    let detail = from
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .unwrap_or(preset);
    Some((category, detail.to_string()))
}

/// Stamp "From:" ([`stamp`]) on `row` when it is a NEW line (no stored row yet under `writer` on
/// `patient`) carrying no source of its own; read under the same writer the save is filed under.
/// A line typed before "From:" keeps what it stored, none included (ruling R12): an edit to it
/// never signs a provenance nobody gave, nor loses its hand tick to one.
pub(crate) async fn stamp_new_line(
    db: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
    row: &mut DraftLine,
) -> anyhow::Result<()> {
    if row.info_source.is_some() || row.source_detail.is_some() {
        return Ok(());
    }
    let stored = draft::read_draft_lines(db, patient, writer).await?;
    if stored.iter().any(|l| l.line_id == row.line_id) {
        return Ok(());
    }
    let from = draft::read_draft_from(db, patient, writer).await?;
    if let Some((category, detail)) = from
        .map(|(preset, detail)| FromView { preset, detail })
        .as_ref()
        .and_then(stamp)
    {
        row.info_source = Some(category.into());
        row.source_detail = Some(detail);
    }
    Ok(())
}

/// The OPENED chart's current lines a version may be set aside beside, as `(medication_id,
/// moiety code canonically spelled)`: active, single-coded under [`MOIETY_SYSTEM`], no coding
/// conflict and no wrong-chart hazard, through a thread on `opened` (never a linked chart's,
/// spec §2). Pure.
pub(crate) fn charted_companions(rows: &[MedicationRow], opened: Uuid) -> Vec<(Uuid, String)> {
    rows.iter()
        .filter(|r| r.status == MedicationStatus::Active && !r.coding_conflict)
        .filter(|r| !r.is_wrong_chart_hazard())
        .filter(|r| r.coding_system.as_deref() == Some(MOIETY_SYSTEM))
        .filter_map(|r| {
            let code: Uuid = r.coding_code.as_deref()?.parse().ok()?;
            let thread = r.members.iter().find(|m| m.patient_id == opened)?;
            Some((thread.medication_id, code.to_string()))
        })
        .collect()
}

/// Each of the OPENED chart's lines as the "Version of which line?" pick names it:
/// `(medication_id, "{drug} {directions}")`, on the same thread [`charted_companions`] offers.
/// Pure.
pub(crate) fn charted_words(rows: &[MedicationRow], opened: Uuid) -> Vec<(Uuid, String)> {
    rows.iter()
        .filter_map(|r| {
            let thread = r.members.iter().find(|m| m.patient_id == opened)?;
            let words = format!("{} {}", r.display_name(), r.sig.as_deref().unwrap_or(""));
            Some((thread.medication_id, words.trim().to_string()))
        })
        .collect()
}

/// The words the pick shows for each of `marks` (as [`companions_of`] gives them): a list line
/// by its words as typed (`lines`), a charted line by its [`charted_words`]. A mark naming
/// neither is shown as itself, never as nothing. Pure.
pub(crate) fn companion_words(
    marks: &[String],
    lines: &[DraftLine],
    charted: &[(Uuid, String)],
) -> Vec<String> {
    let words = |mark: &String| -> Option<String> {
        let (kind, id) = mark.split_once(':')?;
        let id: Uuid = id.parse().ok()?;
        match kind {
            "line" => lines
                .iter()
                .find(|l| l.line_id == id)
                .map(|l| format!("on this list: {}", l.line_text)),
            _ => charted
                .iter()
                .find(|(m, _)| *m == id)
                .map(|(_, w)| format!("charted: {w}")),
        }
    };
    marks
        .iter()
        .map(|m| words(m).unwrap_or_else(|| m.clone()))
        .collect()
}

/// `charted` as `version_holds` and `companion_candidates` take it. Pure.
fn as_current(charted: &[(Uuid, String)]) -> Vec<(Uuid, &str)> {
    charted.iter().map(|(id, c)| (*id, c.as_str())).collect()
}

/// The lines `line` could be set aside beside, as the marks the door stores (`line:<id>`,
/// `med:<id>`), so the webview can send one back as `set_aside_of`. Empty: no "Set aside". Pure.
pub(crate) fn companions_of(
    line: &DraftLine,
    lines: &[DraftLine],
    charted: &[(Uuid, String)],
) -> Vec<String> {
    companion_candidates(line, lines, &as_current(charted))
        .into_iter()
        .map(|c| match c {
            Companion::Draft(id) => format!("line:{id}"),
            Companion::Charted(id) => format!("med:{id}"),
        })
        .collect()
}

/// Sort the set-aside lines of `lines` for Sign, each by its STORED mark on this read: those
/// whose companion is in `ticked` (the lines about to be signed) go with it (`companion →
/// versions`, list order); those beside a charted current line sign alone (`(version,
/// medication_id)`); the rest are kept with why (companion not ticked, or the mark no longer
/// holds: gone, or another drug now). Pure.
#[allow(clippy::type_complexity)]
pub(crate) fn group_versions(
    lines: &[DraftLine],
    ticked: &BTreeSet<Uuid>,
    current: &[(Uuid, &str)],
) -> (
    BTreeMap<Uuid, Vec<Uuid>>,
    Vec<(Uuid, Uuid)>,
    Vec<(Uuid, &'static str)>,
) {
    let mut with = BTreeMap::<Uuid, Vec<Uuid>>::new();
    let (mut alone, mut kept) = (vec![], vec![]);
    for v in lines.iter().filter(|l| l.set_aside_of.is_some()) {
        match version_holds(v, lines, current) {
            Ok(Companion::Draft(c)) if ticked.contains(&c) => {
                with.entry(c).or_default().push(v.line_id)
            }
            Ok(Companion::Draft(_)) => kept.push((v.line_id, VERSION_KEPT)),
            Ok(Companion::Charted(m)) => alone.push((v.line_id, m)),
            Err(why) => kept.push((v.line_id, why)),
        }
    }
    (with, alone, kept)
}

/// The outcome's head and its kept versions: slice 1's "Charted {k} of {n}", then "; {v}
/// version(s) set aside" when any were, then one line per version kept, `(its line as typed,
/// why)`. Pure.
pub(crate) fn outcome_words(
    charted: usize,
    of: usize,
    versions: usize,
    kept: &[(String, &str)],
) -> String {
    let mut head = outcome(charted, of, &[]);
    if versions > 0 {
        head.push_str(&format!("; {versions} version(s) set aside"));
    }
    std::iter::once(head)
        .chain(kept.iter().map(|(line, why)| kept_line(line, why)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One version kept, as the outcome names it. Pure.
fn kept_line(line: &str, why: &str) -> String {
    format!("{line} — version kept: {why}")
}

/// The versions of one Sign, judged on its fresh read: `with` each ticked companion, `alone`
/// beside a charted line, and `kept` as `(line id, line as typed, why)`.
#[derive(Default)]
pub(crate) struct VersionPlan<'a> {
    pub with: BTreeMap<Uuid, Vec<&'a Judged>>,
    pub alone: Vec<(&'a Judged, Uuid)>,
    pub kept: Vec<(String, String, String)>,
}

impl VersionPlan<'_> {
    /// Keep `v` in the list, saying why.
    pub fn keep(&mut self, v: &Judged, why: &str) {
        let l = &v.line;
        self.kept
            .push((l.line_id.to_string(), l.line_text.clone(), why.to_string()));
    }

    /// The kept versions as `not_charted` names them: `(line id, "version kept: why")`.
    pub fn not_charted(&self) -> impl Iterator<Item = (String, String)> + '_ {
        self.kept
            .iter()
            .map(|(id, _, why)| (id.clone(), format!("version kept: {why}")))
    }

    /// The outcome's lines for the kept versions ([`outcome_words`]'s spelling).
    pub fn kept_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.kept.iter().map(|(_, line, why)| kept_line(line, why))
    }
}

/// Plan the versions of a Sign over `judged` (the fresh read): only this `terminal`'s lines,
/// `to_sign` the lines about to be signed, `charted` the opened chart's current lines. A version
/// that cannot be signed for its own line's reason is kept with it.
pub(crate) fn plan_versions<'a>(
    judged: &'a [Judged],
    to_sign: &BTreeSet<Uuid>,
    charted: &[(Uuid, String)],
    terminal: &str,
) -> VersionPlan<'a> {
    let here: Vec<&'a Judged> = judged
        .iter()
        .filter(|j| j.line.terminal_id == terminal)
        .collect();
    let lines: Vec<DraftLine> = here.iter().map(|j| j.line.clone()).collect();
    // Every id group_versions answers is one of `lines`, so it is one of `here`.
    let by_id = |id: Uuid| -> &'a Judged {
        here.iter()
            .copied()
            .find(|j| j.line.line_id == id)
            .expect("a line of this read")
    };
    let (with, alone, kept) = group_versions(&lines, to_sign, &as_current(charted));
    let mut plan = VersionPlan::default();
    for (v, why) in kept {
        plan.keep(by_id(v), why);
    }
    for (companion, versions) in with {
        for v in versions.into_iter().map(by_id) {
            match &v.signable {
                Ok(_) => plan.with.entry(companion).or_default().push(v),
                Err(e) => plan.keep(v, &e.text),
            }
        }
    }
    for (v, med) in alone {
        let v = by_id(v);
        match &v.signable {
            Ok(_) => plan.alone.push((v, med)),
            Err(e) => plan.keep(v, &e.text),
        }
    }
    plan
}

/// How many versions a Sign of the lines drawn ticked NOW (`lines`, as the pane draws them)
/// would record: those [`plan_versions`] signs with a ticked line or alone beside a charted one
/// (ruling R16) — the grouping Sign itself uses, so the count near Sign never names a version
/// Sign would keep. None while this terminal's id is not read (no line is this terminal's).
/// Pure.
pub(crate) fn versions_recorded(
    judged: &[Judged],
    lines: &[ListLineView],
    charted: &[(Uuid, String)],
    terminal: Option<&str>,
) -> usize {
    let Some(terminal) = terminal else { return 0 };
    let ticked: BTreeSet<Uuid> = lines
        .iter()
        .filter(|l| l.ticked)
        .filter_map(|l| l.line_id.parse().ok())
        .collect();
    let plan = plan_versions(judged, &ticked, charted, terminal);
    plan.with.values().map(Vec::len).sum::<usize>() + plan.alone.len()
}

/// The source note a line signs: its source detail and why, each only when not blank; `None`
/// when it has neither, so its event is byte-identical to slice 1's. Pure.
fn source_note(l: &DraftLine) -> Option<SourceNote<'_>> {
    fn non_blank(s: &Option<String>) -> Option<&str> {
        s.as_deref().filter(|s| !s.trim().is_empty())
    }
    let n = SourceNote {
        source_detail: non_blank(&l.source_detail),
        why: non_blank(&l.why),
    };
    (n.source_detail.is_some() || n.why.is_some()).then_some(n)
}

/// The assertion a list line signs: `charted_input`'s, with the line's own source (stamped by
/// "From:" or chosen for it) over the press's default, and its source detail and why. Pure.
pub(crate) fn list_input<'a>(
    j: &'a Judged,
    term: &'a str,
    coding: Option<SubstanceCoding<'a>>,
    charted_as: ChartedAs,
    today: &'a str,
) -> AssertMedicationInput<'a> {
    let mut input = charted_input(term, coding, &j.read, &j.line.directions, charted_as, today);
    if let Some(source) = j.line.info_source.as_deref() {
        input.info_source = source;
    }
    input.source_note = source_note(&j.line);
    input
}

/// What a version signs, rebuilt from its STORED row: the drug fields exactly as a charted line
/// spells them ([`list_input`]), with its route and strength, but no start, no press, no
/// override and no safety claim (a version is never current). Pure.
pub(crate) fn version_to_sign(v: &Judged) -> Result<VersionToSign<'_>, ErrorView> {
    let (term, parts) = v.signable.as_ref().map_err(Clone::clone)?;
    let i = list_input(
        v,
        term,
        single_coding(parts),
        press(&v.line.charted_as)?,
        "",
    );
    Ok(VersionToSign {
        line: v.line.line_id,
        saved_at: &v.line.saved_at,
        drug: MedicationAssertion {
            medication_id: "",
            term: i.term,
            coding: i.coding,
            formulation: i.formulation,
            dose_amount: i.dose_amount,
            dose_unit: i.dose_unit,
            sig: i.sig,
            info_source: i.info_source,
            started: None,
            started_precision: None,
            safety: None,
            charting: Some(Charting {
                route: v.read.route.as_deref(),
                strength: v.read.strength.as_deref(),
                ..Charting::default()
            }),
            source_note: i.source_note,
        },
    })
}

/// Sign one version beside the charted line `med`, alone: its event and its spend in one
/// transaction (`set_aside_draft_line`). The floor refuses a line stopped since; it rolls back.
/// `list` is the list's identity: `(chart, terminal, writer)`.
pub(crate) async fn sign_alone(
    state: &AppState,
    node_sk: &cairn_event::SigningKey,
    (patient, terminal, writer): (Uuid, &str, Option<&str>),
    v: &Judged,
    med: Uuid,
    author: &AuthorParams<'_>,
) -> anyhow::Result<()> {
    let version = version_to_sign(v).map_err(|e| anyhow::anyhow!("{}", e.text))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no database connection"))?
        .lock()
        .await?;
    set_aside_draft_line(
        &mut db,
        node_sk,
        &node_kid,
        &state.node_origin,
        patient,
        terminal,
        writer,
        &version,
        med,
        author,
    )
    .await
    .map(|_| ())
    .map_err(|e| db.explain(e))
}

#[cfg(test)]
#[path = "med_list_versions_tests.rs"]
mod tests;
