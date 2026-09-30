//! Every sentence the "Same person as…" panel shows, as pure functions (R2b-1).
//!
//! The webview renders and decides nothing (see `src-ui/main.js`'s header): on this panel the
//! wording IS the safety content — the panel's safety is what it SHOWS, never an "are you
//! sure?" (principle 3) — so every sentence is built and tested here.
use crate::funnel::view::{ErrorView, Retry};
use cairn_gui_data::port::DataError;
use cairn_gui_tab_medications::view::MedListView;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::LinkEffect;
use cairn_node::patient::compare::{AddressFact, ChartFacts, NameFact, VetoFinding};
use serde::Serialize;
use uuid::Uuid;

/// Refused because the picked chart is already one of this record's charts.
pub const ALREADY_IN_RECORD: &str =
    "that chart is already part of this record — there is nothing to link";
/// Refused because the picked chart was never in a list on screen.
pub const NOT_ON_SCREEN: &str = "that chart was not in a list on screen — search again";
/// Refused because the OTHER record's charts changed between Compare and Link (decision 3,
/// widened to the right-hand side: a peer's link landing mid-review must not clip a chart into
/// this record sight unseen).
pub const OTHER_CHANGED: &str =
    "the other record changed while you were comparing — nothing was done; compare again";
/// Refused because THIS record's charts changed between Compare and Link (final review I1).
/// The Link sends back the left-hand set the comparison was built over, so a chart that joined
/// this record after Compare (a peer's link, re-read by a sign-off's refresh) is caught here
/// rather than signed over sight unseen. The remedy is reload THEN compare: a Compare sends the
/// list on screen (`renderedCharts`), which the same change may have left stale — "compare
/// again" alone would be refused a second time with the list's own "reload the chart" (PR #711
/// review). Shared by the unlink panel, whose Unlink sends back its comparison's set the same way.
pub const THIS_CHANGED: &str =
    "this record changed while you were comparing — nothing was done; reload the chart and \
     compare again";

/// One chart's column heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnView {
    pub patient_id: String,
    pub heading: String,
}

/// One fact kind across every chart, a cell per column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FactRowView {
    pub label: String,
    pub cells: Vec<String>,
}

/// The medication section's own note when the other record's list could not be read.
const MEDICATIONS_UNREAD: &str =
    "The other record's medications could not be read — none are shown.";

/// What `compare_records` hands the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ComparisonView {
    /// Veto findings, worded; hard ones first. Empty → render nothing (never "no conflicts").
    pub findings: Vec<String>,
    /// Left record's charts first, then the right record's.
    pub columns: Vec<ColumnView>,
    /// How many of `columns` belong to "This record".
    pub left_count: usize,
    pub rows: Vec<FactRowView>,
    /// THIS record's chart set the comparison was built over — the webview sends it back with
    /// the link, so the judgement names the set the clinician compared, not whatever the
    /// window happens to show by the time Link is pressed (final review I1).
    pub left_charts: Vec<String>,
    /// The OTHER record's chart set as displayed — the webview sends it back with the link.
    pub other_charts: Vec<String>,
    /// The other record's CURRENT medications, one line each.
    pub other_medications: Vec<String>,
    /// Its list's own warnings (withheld / missing), the empty-list sentence, or — when the
    /// list could not be read — a sentence saying so.
    pub other_medication_notes: Vec<String>,
    /// What could NOT be read. Non-empty → `can_link` is false.
    pub problems: Vec<String>,
    pub can_link: bool,
}

/// The four reads a comparison is built from, each of which may have failed on its own.
pub struct ComparisonParts {
    pub left: Result<Vec<ChartFacts>, String>,
    pub right: Result<Vec<ChartFacts>, String>,
    pub findings: Result<Vec<VetoFinding>, String>,
    pub other_medications: Result<MedListView, String>,
}

/// What `link_records` hands back: the sentence for the outcome line, and whether the chart
/// should be re-read (it should whenever the record may have changed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkReportView {
    pub sentence: String,
    /// Re-read the chart. `false` means the judgement did NOT change the record — only
    /// [`LinkEffect::Outranked`] — and the webview relies on exactly that: `link.js` and
    /// `unlink.js` keep the panel open and show the sentence there once, instead of closing it and
    /// re-reading. A new `false` case must be worded for that panel.
    pub reload: bool,
}

/// A refusal the clinician cannot retry as-is.
pub fn refused(text: impl Into<String>) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::Never,
    }
}

/// Link pressed while the signing key is locked. NOT a verdict: nothing about the comparison
/// was wrong, and once the clinician unlocks the key the identical Link succeeds — so it keeps
/// the button (`Retry::Now`). As `Retry::Never` the webview took the comparison away and forced
/// a second Compare, an act the paper workflow does not have (PR #707 review).
pub fn key_locked() -> ErrorView {
    key_locked_for("Link — same person")
}

/// [`key_locked`] for any judgement button: the message must name the button the clinician
/// actually pressed — on a safety panel, naming the opposite act ("press Link" after an
/// Unlink click) is a wrong instruction. Still `Retry::Now`.
pub fn key_locked_for(button: &str) -> ErrorView {
    ErrorView {
        text: format!("your signing key is locked — unlock it, then press \"{button}\" again"),
        retry: Retry::Now,
    }
}

/// One veto finding as a plain fact: which kind of disagreement, db/016's own words, and the
/// two charts. No judgement words ("likely", "probably different") — a hard veto forces a
/// human decision, it does not make it (§5.13). An unknown severity is shown verbatim.
///
/// The label depends on the KIND as well as the severity, because db/016's severity means
/// different things per kind (PR #707 review, principle 4). A dob / sex-at-birth hard veto
/// (`cairn_field_clash`) fires only when BOTH winners are verified (provenance rank ≥ 60), so
/// "verified" is true there. An identifier's severity (`cairn_identifier_veto`) says only
/// whether both values passed a format profile — two patient-stated numbers can be a hard veto
/// — so an identifier line never claims verification.
pub fn finding_line(f: &VetoFinding) -> String {
    let label = match (f.kind.as_str(), f.severity.as_str()) {
        ("identifier", "hard_veto") => "Identifiers differ (both in a checked format)",
        ("identifier", "degrade_hold") => "Identifiers differ (format not checked)",
        (_, "hard_veto") => "Verified facts differ",
        (_, "degrade_hold") => "Facts differ, not verified",
        (_, other) => other,
    };
    format!(
        "{label} — {} — between chart {} and chart {}",
        f.detail, f.left, f.right
    )
}

/// The word for an absent fact: "not recorded" on a chart held here; "unknown" on one whose
/// registration has not arrived (nothing says it was never recorded). Principle 4.
fn absent(f: &ChartFacts) -> String {
    if f.held {
        "not recorded".into()
    } else {
        "unknown — registration not yet received here".into()
    }
}

/// Join a list of values, or the absence word when there are none.
fn joined(f: &ChartFacts, values: Vec<String>) -> String {
    if values.is_empty() {
        absent(f)
    } else {
        values.join("; ")
    }
}

/// One name's cell text: the value, its `use` facet (or its named absence), and provenance.
fn name_text(n: &NameFact) -> String {
    let use_ = n.use_.as_deref().unwrap_or("use not recorded");
    format!("{} ({use_}, {})", n.value, n.provenance)
}

/// One address's cell text: `"{display} ({use}, {provenance})"` when a `use` facet was
/// recorded, else `"{display} ({provenance})"` — every other fact row carries provenance, so
/// an address is never shown with only its `use` and none.
fn address_text(a: &AddressFact) -> String {
    match a.use_.as_deref() {
        Some(use_) => format!("{} ({use_}, {})", a.display, a.provenance),
        None => format!("{} ({})", a.display, a.provenance),
    }
}

/// The "Names struck as false" cell: `patient_alias_pool`, which holds the names REPUDIATED as
/// known-false on this chart (db/025) — not earlier or former names, which is why the row is
/// labelled for what it is (final review M5).
///
/// Struck names that ARE here are always listed, held or not: a peer's repudiation can arrive
/// before the registration (the sync door), and hiding it behind "unknown" would hide a fact
/// this node has (PR #707 review). Only the ABSENCE depends on `held`: on a chart held here an
/// empty pool IS a fact ("none"); on one not held, nothing says it has no struck names — only
/// that its registration has not arrived — so the cell says [`absent`]'s "unknown" (principle 4).
fn alias_cell(f: &ChartFacts) -> String {
    if !f.aliases.is_empty() {
        f.aliases.join("; ")
    } else if f.held {
        "none".into()
    } else {
        absent(f)
    }
}

/// The column heading: the first current name (or its absence) and the chart id, whole — it is
/// what ties the column to each medication row's source label.
pub(crate) fn heading(f: &ChartFacts) -> String {
    let name = f
        .names
        .first()
        .map(|n| n.value.clone())
        .unwrap_or_else(|| absent(f));
    format!("{name} · chart {}", f.patient_id)
}

/// The "Date of birth" cell: `"{value} ({provenance})"`, unless the assertion's precision is
/// coarser than a day, in which case the cell says so — `"{value} ({precision} precision,
/// {provenance})"`. Controller ruling (principle 4): a year- or month-precision date must
/// never read as a precise day, or a genuinely different day on the other chart would look
/// like agreement (or a coarse date would look like a clash it is not). `None` precision (no
/// facet recorded) and `Some("day")` are both the precise case and render unchanged.
fn dob_cell(f: &ChartFacts) -> String {
    f.dob
        .as_ref()
        .map(|d| match d.precision.as_deref() {
            None | Some("day") => format!("{} ({})", d.value, d.provenance),
            Some(precision) => format!("{} ({precision} precision, {})", d.value, d.provenance),
        })
        .unwrap_or_else(|| absent(f))
}

/// The "Sex at birth" cell: `"{value} ({provenance})"`. Its schema carries no precision facet
/// (see [`cairn_node::patient::compare::FieldFact::precision`]'s doc), so there is nothing to
/// word beyond the plain value.
fn sex_cell(f: &ChartFacts) -> String {
    f.sex_at_birth
        .as_ref()
        .map(|d| format!("{} ({})", d.value, d.provenance))
        .unwrap_or_else(|| absent(f))
}

/// The fact rows, in the order a front sheet is read. Each closure renders one chart's cell.
pub(crate) fn fact_rows(charts: &[ChartFacts]) -> Vec<FactRowView> {
    type Cell = fn(&ChartFacts) -> String;
    let kinds: [(&str, Cell); 7] = [
        ("Names", |f| {
            joined(f, f.names.iter().map(name_text).collect())
        }),
        ("Names struck as false", alias_cell),
        ("Date of birth", dob_cell),
        ("Sex at birth", sex_cell),
        ("Identifiers", |f| {
            joined(
                f,
                f.identifiers
                    .iter()
                    .map(|i| format!("{}: {} ({})", i.system, i.value, i.provenance))
                    .collect(),
            )
        }),
        ("Addresses", |f| {
            joined(f, f.addresses.iter().map(address_text).collect())
        }),
        ("Identity", |f| f.trust.clone()),
    ];
    kinds
        .iter()
        .map(|(label, cell)| FactRowView {
            label: (*label).into(),
            cells: charts.iter().map(cell).collect(),
        })
        .collect()
}

/// The other record's CURRENT medications as one line each, plus the notes to show before
/// them (the list's own withheld/missing warnings, prefixed so they read as about the OTHER
/// record — they are worded for the med-list tab, which speaks of "this chart" — or the named
/// absence of any drug).
///
/// The absence sentence is added ONLY when there are no current lines AND no withheld/missing
/// note either: a list the system itself calls incomplete (a withheld or missing note present)
/// must never ALSO claim "no current medications" — that is an absence claim over a list this
/// same function knows is not the whole picture (controller ruling, principle 4).
pub fn medication_lines(list: &MedListView) -> (Vec<String>, Vec<String>) {
    let lines: Vec<String> = list
        .rows
        .iter()
        .filter(|r| r.current)
        .map(|r| format!("{} — {} — {}", r.primary, r.dose, r.sig))
        .collect();
    let mut notes: Vec<String> = [&list.withheld_message, &list.missing_message]
        .into_iter()
        .flatten()
        .map(|n| format!("On the other record: {n}"))
        .collect();
    if lines.is_empty() && notes.is_empty() {
        notes.push("No current medications recorded on the other record.".into());
    }
    (lines, notes)
}

/// Assemble the panel from whatever was read. Pure: the availability rule (show what was read,
/// name what was not, offer no link unless everything was read) is tested with no database.
///
/// `left_charts` / `other_charts` are the two sets the comparison was read over; they are
/// carried in the view so the Link can name exactly them.
pub fn comparison_view(
    parts: ComparisonParts,
    left_charts: &ChartSet,
    other_charts: &ChartSet,
) -> ComparisonView {
    let mut problems = vec![];
    let mut take = |r: Result<Vec<ChartFacts>, String>, what: &str| match r {
        Ok(v) => v,
        Err(e) => {
            problems.push(format!("{what} could not be read: {e}"));
            vec![]
        }
    };
    let left = take(parts.left, "This record's identity facts");
    let right = take(parts.right, "The other record's identity facts");
    let findings = match parts.findings {
        Ok(f) => f.iter().map(finding_line).collect(),
        Err(e) => {
            problems.push(format!(
                "The check for disagreeing facts could not be run: {e}"
            ));
            vec![]
        }
    };
    let (other_medications, other_medication_notes) = match parts.other_medications {
        Ok(list) => medication_lines(&list),
        Err(e) => {
            problems.push(format!(
                "The other record's medications could not be read: {e}"
            ));
            // Said in the section too, not only in `problems`: its heading over two empty
            // lists would otherwise read as "no medications" (PR #707 review).
            (vec![], vec![MEDICATIONS_UNREAD.to_string()])
        }
    };
    let charts: Vec<ChartFacts> = left.iter().chain(right.iter()).cloned().collect();
    ComparisonView {
        findings,
        columns: charts
            .iter()
            .map(|f| ColumnView {
                patient_id: f.patient_id.to_string(),
                heading: heading(f),
            })
            .collect(),
        left_count: left.len(),
        rows: fact_rows(&charts),
        left_charts: ids(left_charts),
        other_charts: ids(other_charts),
        other_medications,
        other_medication_notes,
        can_link: problems.is_empty(),
        problems,
    }
}

/// A chart set as the id strings the webview sends back.
fn ids(set: &ChartSet) -> Vec<String> {
    set.members().iter().map(Uuid::to_string).collect()
}

/// The charts `now` combines that were in neither compared set — `compared` is the union of
/// the two sets the panel showed. Empty in the ordinary case.
fn uncompared(now: &ChartSet, compared: &ChartSet) -> Vec<String> {
    now.members()
        .iter()
        .filter(|c| !compared.contains(c))
        .map(Uuid::to_string)
        .collect()
}

/// What the link did, as the outcome line says it. Never "linked" for a link that did not
/// take effect (R2a: recorded is not took effect).
///
/// `charts` is what the record reads as now (the node's read inside the judgement's own
/// transaction); `compared` is both sets the panel showed. If the record now also includes a
/// chart the comparison never showed — a third chart already linked to one side by the time the
/// judgement landed — the sentence names it, so it is reviewed rather than assumed (final review
/// O1): the outcome tells the truth about what the link joined.
pub fn link_report(effect: LinkEffect, charts: &ChartSet, compared: &ChartSet) -> LinkReportView {
    let mut view = effect_report(effect, charts);
    let extra = uncompared(charts, compared);
    if !extra.is_empty() {
        view.sentence = format!(
            "{} The record now also includes chart(s) {} that were not in the comparison — \
             review them.",
            view.sentence,
            extra.join(", ")
        );
    }
    view
}

/// The outcome sentence for each [`LinkEffect`], before [`link_report`] adds anything about
/// charts the comparison did not show.
fn effect_report(effect: LinkEffect, charts: &ChartSet) -> LinkReportView {
    match effect {
        LinkEffect::TookEffect => LinkReportView {
            sentence: format!(
                "Linked — this record now combines {} charts.",
                charts.members().len()
            ),
            reload: true,
        },
        LinkEffect::Outranked => LinkReportView {
            sentence: "Recorded, but NOT in effect: a later judgement on this pair says these \
                       are different people. The two judgements disagree — settle it with \
                       the person who made the other one. Linking again would normally record a newer \
                       judgement that overrules theirs — it would not settle the disagreement."
                .into(),
            reload: false,
        },
        // R2a never returns this for a link; worded honestly in case it ever does.
        LinkEffect::StillJoined => LinkReportView {
            sentence: "Recorded, but the record did not change the way a link should — the \
                       chart is being re-read so you can see what it now combines."
                .into(),
            reload: true,
        },
    }
}

/// A failed link, worded by the SAME classification the funnel uses (`data_error_from`: a
/// `P0001` floor refusal or a marked `cairn-node` verdict is a verdict; anything else an
/// outage) and the funnel's `Retry` vocabulary (#702). An outage is "not confirmed", never
/// "nothing changed": a connection lost during the commit leaves the outcome unknown, and the
/// node's own message (carried in `t`) says to check before retrying. A second identical link
/// is harmless (the same standing state), so `Retry::Now` is safe.
pub fn link_error_view(e: &anyhow::Error) -> ErrorView {
    link_error_from(cairn_gui_live::error::data_error_from(e))
}

/// The wording half of [`link_error_view`], split out so every arm is unit-tested with no
/// database: the classification (`data_error_from`) is `cairn-gui-live`'s and tested there,
/// and the verdicts `cairn-node` raises are `pub(crate)` to it, so this crate cannot build them.
///
/// `NotProvisioned` says "yet", not "until an operator acts": one of its causes (a chart this
/// node does not hold yet) resolves by sync, with no operator involved (final review M4).
pub fn link_error_from(error: DataError) -> ErrorView {
    judgement_error_from("link", error)
}

/// A failed judgement worded for its act (`"link"` / `"unlink"`), by the classification the
/// funnel uses — see [`link_error_from`], which is this with `"link"`.
pub fn judgement_error_from(act: &str, error: DataError) -> ErrorView {
    match error {
        DataError::Refused(t) => ErrorView {
            text: format!("The {act} was refused: {t}"),
            retry: Retry::Never,
        },
        DataError::NotProvisioned(t) => ErrorView {
            text: format!("This node cannot record the {act} yet: {t}"),
            retry: Retry::AfterOperator,
        },
        DataError::Unavailable(t) => ErrorView {
            text: format!("The {act} was not confirmed: {t}"),
            retry: Retry::Now,
        },
        DataError::NotFound => refused(format!("The {act} was not recorded.")),
    }
}

/// A fixture chart's facts for `--mock`, where there is no database: the name the list showed,
/// nothing else. Enough to walk and time the panel; fixture mode refuses the link itself.
pub fn fixture_facts(patient: Uuid, name: &str, trust: &str) -> ChartFacts {
    ChartFacts {
        patient_id: patient,
        held: true,
        trust: trust.into(),
        names: vec![NameFact {
            value: name.into(),
            use_: None,
            provenance: "fixture".into(),
        }],
        ..ChartFacts::default()
    }
}

// Kept in a sibling file (fix round 1, task 4 review): the test module reproduces the
// brief's tests verbatim plus this round's fixes, and together they no longer fit under the
// house 500-line guideline alongside the implementation above.
#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
