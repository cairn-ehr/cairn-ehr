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
use cairn_node::patient::compare::{ChartFacts, NameFact, VetoFinding};
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
    /// The OTHER record's chart set as displayed — the webview sends it back with the link.
    pub other_charts: Vec<String>,
    /// The other record's CURRENT medications, one line each.
    pub other_medications: Vec<String>,
    /// Its list's own warnings (withheld / missing), or the empty-list sentence.
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
    pub reload: bool,
}

/// A refusal the clinician cannot retry as-is.
pub fn refused(text: impl Into<String>) -> ErrorView {
    ErrorView {
        text: text.into(),
        retry: Retry::Never,
    }
}

/// One veto finding as a plain fact: which kind of disagreement, db/016's own words, and the
/// two charts. No judgement words ("likely", "probably different") — a hard veto forces a
/// human decision, it does not make it (§5.13). An unknown severity is shown verbatim.
pub fn finding_line(f: &VetoFinding) -> String {
    let label = match f.severity.as_str() {
        "hard_veto" => "Verified facts differ",
        "degrade_hold" => "Facts differ, not verified",
        other => other,
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

fn name_text(n: &NameFact) -> String {
    let use_ = n.use_.as_deref().unwrap_or("use not recorded");
    format!("{} ({use_}, {})", n.value, n.provenance)
}

/// The column heading: the first current name (or its absence) and the chart id, whole — it is
/// what ties the column to each medication row's source label.
fn heading(f: &ChartFacts) -> String {
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
fn fact_rows(charts: &[ChartFacts]) -> Vec<FactRowView> {
    type Cell = fn(&ChartFacts) -> String;
    let kinds: [(&str, Cell); 7] = [
        ("Names", |f| {
            joined(f, f.names.iter().map(name_text).collect())
        }),
        // An empty alias pool is a fact ("none"), not an unknown.
        ("Earlier recorded names", |f| {
            if f.aliases.is_empty() {
                "none".into()
            } else {
                f.aliases.join("; ")
            }
        }),
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
            joined(
                f,
                f.addresses
                    .iter()
                    .map(|a| {
                        format!(
                            "{} ({})",
                            a.display,
                            a.use_.as_deref().unwrap_or(&a.provenance)
                        )
                    })
                    .collect(),
            )
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
/// them (the list's own withheld/missing warnings, or the named absence of any drug).
pub fn medication_lines(list: &MedListView) -> (Vec<String>, Vec<String>) {
    let lines: Vec<String> = list
        .rows
        .iter()
        .filter(|r| r.status_label == "current")
        .map(|r| format!("{} — {} — {}", r.primary, r.dose, r.sig))
        .collect();
    let mut notes: Vec<String> = [&list.withheld_message, &list.missing_message]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
    if lines.is_empty() {
        notes.push("No current medications recorded on the other record.".into());
    }
    (lines, notes)
}

/// Assemble the panel from whatever was read. Pure: the availability rule (show what was read,
/// name what was not, offer no link unless everything was read) is tested with no database.
pub fn comparison_view(parts: ComparisonParts, other_charts: &ChartSet) -> ComparisonView {
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
            (vec![], vec![])
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
        other_charts: other_charts.members().iter().map(Uuid::to_string).collect(),
        other_medications,
        other_medication_notes,
        can_link: problems.is_empty(),
        problems,
    }
}

/// What the link did, as the outcome line says it. Never "linked" for a link that did not
/// take effect (R2a: recorded is not took effect).
pub fn link_report(effect: LinkEffect, charts: &ChartSet) -> LinkReportView {
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
                       the person who made the other one; pressing Link again changes nothing."
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
    match cairn_gui_live::error::data_error_from(e) {
        DataError::Refused(t) => ErrorView {
            text: format!("The link was refused: {t}"),
            retry: Retry::Never,
        },
        DataError::NotProvisioned(t) => ErrorView {
            text: format!("This node cannot record the link until an operator acts: {t}"),
            retry: Retry::AfterOperator,
        },
        DataError::Unavailable(t) => ErrorView {
            text: format!("The link was not confirmed: {t}"),
            retry: Retry::Now,
        },
        DataError::NotFound => refused("The link was not recorded."),
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

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_node::patient::compare::{FieldFact, NameFact};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn held(n: u128) -> ChartFacts {
        ChartFacts {
            patient_id: id(n),
            held: true,
            trust: "confirmed".into(),
            names: vec![NameFact {
                value: "N".into(),
                use_: Some("legal".into()),
                provenance: "patient-stated".into(),
            }],
            // `precision: Some("day")` renders identically to the pre-controller-ruling
            // fixture: day precision is the case that must NOT grow the "(N precision, …)"
            // suffix (only a coarser facet does — see `a_coarse_dob_names_its_precision`).
            dob: Some(FieldFact {
                value: "1950-07-01".into(),
                provenance: "document-verified".into(),
                precision: Some("day".into()),
            }),
            ..ChartFacts::default()
        }
    }

    fn meds() -> MedListView {
        cairn_gui_tab_medications::view::build_view(&cairn_medication_view::fixtures::sample_chart())
    }

    fn parts() -> ComparisonParts {
        ComparisonParts {
            left: Ok(vec![held(1)]),
            right: Ok(vec![held(2)]),
            findings: Ok(vec![]),
            other_medications: Ok(meds()),
        }
    }

    #[test]
    fn a_full_comparison_is_linkable_and_names_the_other_record() {
        let v = comparison_view(parts(), &ChartSet::single(id(2)));
        assert!(v.can_link && v.problems.is_empty());
        assert_eq!(v.left_count, 1);
        assert_eq!(v.columns.len(), 2);
        assert_eq!(
            v.other_charts,
            vec![id(2).to_string()],
            "sent back with the link"
        );
        assert!(
            v.rows.iter().all(|r| r.cells.len() == 2),
            "every row has a cell per chart"
        );
    }

    /// Review Focus 4: a comparison read only in part shows what it has, names what it lacks,
    /// and offers NO link — a judgement needs the whole picture.
    #[test]
    fn a_partial_comparison_names_what_is_missing_and_cannot_link() {
        let mut p = parts();
        p.other_medications = Err("connection reset".into());
        let v = comparison_view(p, &ChartSet::single(id(2)));
        assert!(!v.can_link);
        assert_eq!(v.problems.len(), 1);
        assert!(
            v.problems[0].contains("medications"),
            "names the part that is missing"
        );
        assert_eq!(
            v.columns.len(),
            2,
            "the facts that WERE read are still shown"
        );
    }

    #[test]
    fn unreadable_facts_leave_no_columns_and_cannot_link() {
        let mut p = parts();
        p.right = Err("boom".into());
        let v = comparison_view(p, &ChartSet::single(id(2)));
        assert!(!v.can_link);
        assert_eq!(v.left_count, 1);
        assert_eq!(
            v.columns.len(),
            1,
            "no invented column for a record that was not read"
        );
    }

    /// Principle 4: absence is worded — and differently for a chart this node does not hold.
    #[test]
    fn an_absent_fact_says_not_recorded_or_unknown() {
        let mut unheld = ChartFacts {
            patient_id: id(2),
            held: false,
            trust: "unknown".into(),
            ..ChartFacts::default()
        };
        unheld.names.clear();
        let mut p = parts();
        p.right = Ok(vec![unheld]);
        let v = comparison_view(p, &ChartSet::single(id(2)));
        let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
        assert_eq!(dob.cells[0], "1950-07-01 (document-verified)");
        assert_eq!(dob.cells[1], "unknown — registration not yet received here");
        let mut p = parts();
        p.right = Ok(vec![ChartFacts {
            patient_id: id(2),
            held: true,
            trust: "confirmed".into(),
            ..ChartFacts::default()
        }]);
        let v = comparison_view(p, &ChartSet::single(id(2)));
        let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
        assert_eq!(dob.cells[1], "not recorded");
    }

    /// Controller ruling (principle 4): a coarse date must never read as a precise day. Day
    /// precision (and no precision facet at all, e.g. sex-at-birth) render as before; anything
    /// coarser says so, so "1950" is never mistaken for a verified "1950-01-01".
    #[test]
    fn a_coarse_dob_names_its_precision() {
        let coarse = ChartFacts {
            patient_id: id(2),
            held: true,
            trust: "confirmed".into(),
            dob: Some(FieldFact {
                value: "1950".into(),
                provenance: "patient-stated".into(),
                precision: Some("year".into()),
            }),
            ..ChartFacts::default()
        };
        let mut p = parts();
        p.right = Ok(vec![coarse]);
        let v = comparison_view(p, &ChartSet::single(id(2)));
        let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
        assert_eq!(dob.cells[1], "1950 (year precision, patient-stated)");
    }

    #[test]
    fn a_finding_is_a_plain_fact_naming_both_charts() {
        let f = VetoFinding {
            left: id(1),
            right: id(2),
            kind: "dob".into(),
            severity: "hard_veto".into(),
            subject: "dob".into(),
            detail: "verified dob clash (precision day): 'x' vs 'y'".into(),
        };
        let line = finding_line(&f);
        assert!(line.starts_with("Verified facts differ"), "{line}");
        assert!(line.contains(&id(1).to_string()) && line.contains(&id(2).to_string()));
        assert!(
            line.contains("verified dob clash"),
            "db/016's own words, verbatim"
        );
        let hold = finding_line(&VetoFinding {
            severity: "degrade_hold".into(),
            ..f
        });
        assert!(hold.starts_with("Facts differ, not verified"), "{hold}");
    }

    /// Never "no conflicts": an empty finding list renders nothing at all.
    #[test]
    fn no_findings_means_no_lines_and_no_clearance_sentence() {
        let v = comparison_view(parts(), &ChartSet::single(id(2)));
        assert!(v.findings.is_empty());
    }

    #[test]
    fn only_current_medications_are_listed_and_warnings_carry_over() {
        let (lines, _notes) = medication_lines(&meds());
        let current = meds()
            .rows
            .iter()
            .filter(|r| r.status_label == "current")
            .count();
        assert_eq!(
            lines.len(),
            current,
            "ceased drugs are not 'active medications'"
        );
    }

    #[test]
    fn an_empty_list_says_so() {
        let empty = cairn_gui_tab_medications::view::build_view(
            &cairn_medication_view::PatientMedicationList::empty(ChartSet::single(id(2))),
        );
        let (lines, notes) = medication_lines(&empty);
        assert!(lines.is_empty());
        assert!(
            notes.iter().any(|n| n.contains("No current medications")),
            "absence is named"
        );
    }

    #[test]
    fn a_link_that_took_effect_reloads_and_one_outranked_does_not() {
        let set = ChartSet::new([id(1), id(2)]).unwrap();
        let took = link_report(LinkEffect::TookEffect, &set);
        assert!(
            took.reload && took.sentence.starts_with("Linked"),
            "{}",
            took.sentence
        );
        assert!(took.sentence.contains("2 charts"));
        let lost = link_report(LinkEffect::Outranked, &ChartSet::single(id(1)));
        assert!(!lost.reload, "a disagreement is shown, never reloaded away");
        assert!(lost.sentence.contains("NOT in effect"), "{}", lost.sentence);
    }

    /// Review Focus 5 (outage half): an outage is "not confirmed", never "nothing changed" — a
    /// connection lost mid-commit leaves the outcome unknown.
    #[test]
    fn an_outage_is_worded_not_confirmed_and_retryable() {
        let outage = anyhow::anyhow!("connection reset");
        assert_eq!(link_error_view(&outage).retry, Retry::Now);
        let text = link_error_view(&outage).text;
        assert!(text.contains("not confirmed"), "{text}");
    }
}
