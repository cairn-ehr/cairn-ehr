//! The allergy section as the webview reads it ([`AllergyPane`]): `cairn_node`'s pure
//! `build_view`, mapped onto plain strings in this terminal's zone, with the chart set the rows
//! were read over. Moved out of `allergies.rs` (allergy slice 2) to keep that file small; the
//! commands there and in `allergy_overlays.rs` answer with it.
use cairn_medication_view::ChartSet;
use cairn_node::allergy::read::PatientAllergies;
use cairn_node::allergy::view::{build_view, EntryLine, ListStatus, NoneLine};
use chrono::Offset;
use uuid::Uuid;

/// The allergy section as the webview reads it: `cairn_node`'s `AllergyView`, mapped onto plain
/// strings the way every other pane here is (the node's types carry no `Serialize`).
#[derive(Debug, serde::Serialize)]
pub struct AllergyPane {
    /// The one-line header strip.
    pub strip: String,
    /// `"entries"`, `"none-known"` or `"not-yet-asked"`, for the drug list and the other list.
    pub drug: &'static str,
    pub other: &'static str,
    pub drug_allergies: Vec<AllergyLine>,
    pub other_allergies: Vec<AllergyLine>,
    pub adverse_effects: Vec<AllergyLine>,
    /// "Previously recorded: …" lines: "none known" statements standing for no list (overridden,
    /// superseded, of an unknown scope, of a suspect (repudiated or doubtful) key, or on a chart
    /// whose link is in doubt).
    pub previously_none: Vec<NoneLineView>,
    /// The standing "none known" statement(s), each naming when and by whom it was signed.
    pub standing_none: Vec<NoneLineView>,
    /// De-labelled lines ("Historically reported: … Challenged: tolerated …"), never prominent;
    /// each names the de-label it rests on (`challenge`), which the window can withdraw.
    pub historical: Vec<AllergyLine>,
    /// Withdrawn entries, de-labels and "none known" statements: "Entered in error: …".
    pub entered_in_error: Vec<EnteredInErrorLine>,
    /// De-labels and withdrawals that count for nothing, and why: "Not counted: …".
    pub ignored_overlays: Vec<String>,
    /// Allergy records on this chart set this terminal cannot read (no custody, or shredded):
    /// the strip leads with them, and the window shows it prominent.
    pub unreadable: usize,
    /// The chart set these rows were read over: the webview sends it back with every write.
    pub charts: Vec<String>,
}

/// One line of an expanded list.
#[derive(Debug, serde::Serialize)]
pub struct AllergyLine {
    pub event_id: String,
    /// The line as the clinician reads it, doubt mark included.
    pub text: String,
    /// The red sticker: an allergy, not an adverse effect or a kind this node does not know.
    pub prominent: bool,
    pub doubted_chart: bool,
    /// Who signed it, when this node can name the key.
    pub signer: Option<String>,
    /// The local day it was recorded, `YYYY-MM-DD`.
    pub recorded: String,
    /// The chart the entry is filed under (a correction, de-label or withdrawal of it is too, D2).
    pub chart: String,
    /// This line's older versions, "Earlier: …", newest first.
    pub earlier: Vec<String>,
    /// On a historical line: the de-label it rests on (the window withdraws it). `None` otherwise.
    pub challenge: Option<String>,
    /// On a live line: its words as recorded, which the Correct form starts from. `None` on a
    /// historical line, which is never corrected (its de-label is withdrawn, or it is struck).
    pub words: Option<LineWords>,
}

/// A live line's words as recorded, in wire vocabulary (`cairn_event::allergy`), so the Correct
/// form can be filled exactly as the entry was signed.
#[derive(Debug, serde::Serialize)]
pub struct LineWords {
    pub allergen_kind: String,
    pub term: String,
    pub kind: String,
    pub category: String,
    pub reaction_text: Option<String>,
    pub onset_precision: String,
    pub onset_value: Option<String>,
    pub source: String,
}

/// One "none known" line: its words, and the statement it reads (the window withdraws it).
#[derive(Debug, serde::Serialize)]
pub struct NoneLineView {
    pub event_id: String,
    pub text: String,
    /// On a chart whose link to this one is in doubt: the window offers no act on it (review C1).
    pub doubted_chart: bool,
}

/// One "Entered in error: …" line, with the older versions a withdrawn entry had replaced.
#[derive(Debug, serde::Serialize)]
pub struct EnteredInErrorLine {
    pub text: String,
    pub earlier: Vec<String>,
}

/// `cairn_node`'s pure view over `a`, in this terminal's zone (the node never knows it), as the
/// pane, with the chart set it was read over.
pub(crate) fn pane(a: &PatientAllergies, set: &ChartSet) -> AllergyPane {
    let v = build_view(a, chrono::Local::now().offset().fix());
    let status = |s: &ListStatus| match s {
        ListStatus::Entries => "entries",
        ListStatus::NoneKnown { .. } => "none-known",
        ListStatus::NotYetAsked => "not-yet-asked",
    };
    let line = |l: EntryLine, words: Option<LineWords>| AllergyLine {
        event_id: l.event_id.to_string(),
        text: l.text,
        prominent: l.prominent,
        doubted_chart: l.doubted_chart,
        signer: l.signer,
        recorded: l.recorded.to_string(),
        chart: l.chart.to_string(),
        earlier: l.earlier,
        challenge: l.challenge.map(|c| c.to_string()),
        words,
    };
    // A live line carries its words for the Correct form; a historical one does not.
    let lines = |ls: Vec<EntryLine>| -> Vec<AllergyLine> {
        ls.into_iter()
            .map(|l| {
                let words = line_words(a, l.event_id);
                line(l, words)
            })
            .collect()
    };
    let nones = |ls: Vec<NoneLine>| -> Vec<NoneLineView> {
        ls.into_iter()
            .map(|n| NoneLineView {
                event_id: n.event_id.to_string(),
                text: n.text,
                doubted_chart: n.doubted_chart,
            })
            .collect()
    };
    AllergyPane {
        strip: v.strip,
        drug: status(&v.drug),
        other: status(&v.other),
        drug_allergies: lines(v.drug_allergies),
        other_allergies: lines(v.other_allergies),
        adverse_effects: lines(v.adverse_effects),
        previously_none: nones(v.previously_none),
        standing_none: nones(v.standing_none),
        unreadable: v.unreadable,
        historical: v.historical.into_iter().map(|l| line(l, None)).collect(),
        entered_in_error: v
            .entered_in_error
            .into_iter()
            .map(|e| EnteredInErrorLine {
                text: e.text,
                earlier: e.earlier,
            })
            .collect(),
        ignored_overlays: v.ignored_overlays,
        charts: set.members().iter().map(Uuid::to_string).collect(),
    }
}

/// The recorded words of entry `event_id`, looked up among `a`'s rows (`None` if it is not one).
/// Pure.
fn line_words(a: &PatientAllergies, event_id: Uuid) -> Option<LineWords> {
    let r = a.entries.iter().find(|r| r.event_id == event_id)?;
    Some(LineWords {
        allergen_kind: r.allergen_kind.clone(),
        term: r.term.clone(),
        kind: r.kind.clone(),
        category: r.category.clone(),
        reaction_text: r.reaction_text.clone(),
        onset_precision: r.onset_precision.clone(),
        onset_value: r.onset_value.clone(),
        source: r.source.clone(),
    })
}
