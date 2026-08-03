//! The medications tab: the med-list view model plus its accessibility contract.
//!
//! The contract is DATA, declared here and asserted in CI. The webview renders from it,
//! so — unlike the iced shell, where the declaration only hoped to match what a screen
//! reader announced — what is declared here is what the markup is written to produce.
//! Verifying that the browser really announces it is still an operator act with a live
//! screen reader; automating the DOM assertions is issue #332.
pub mod view;

pub use view::{build_view, MedListRowView, MedListView};

use cairn_gui_tab::context::Context;
use cairn_gui_tab::semantics::{Field, Role, SemanticNode};
use cairn_gui_tab::{Semantic, TabId};

pub struct MedicationsTab {
    view: MedListView,
}

impl MedicationsTab {
    pub fn new(view: MedListView) -> Self {
        Self { view }
    }

    /// The view model this tab renders — the Tauri command hands the same value to the
    /// webview, so the announced contract and the drawn table describe one state.
    pub fn view(&self) -> &MedListView {
        &self.view
    }
}

impl Semantic for MedicationsTab {
    fn tab_id(&self) -> TabId {
        TabId("medications".into())
    }

    fn title(&self) -> String {
        "Medications".into()
    }

    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "medications-heading".into(),
            role: Role::Heading,
            label: "Medications".into(),
        }];

        // The chart-level warnings come FIRST, before the rows.
        //
        // Order is the whole point (ADR-0060 decision 2). A screen-reader user moves
        // through this tree linearly; a warning that the chart is missing a drug is
        // useless after the reader has already been told what the chart contains, because
        // by then they have formed the belief the warning exists to prevent. On paper the
        // equivalent is a note at the top of the chart, not a footnote.
        for (id, message) in [
            ("chart-incomplete", self.view.missing_message.as_ref()),
            ("chart-withheld", self.view.withheld_message.as_ref()),
        ] {
            if let Some(message) = message {
                fields.push(Field {
                    id: id.into(),
                    role: Role::Heading,
                    label: message.clone(),
                });
            }
        }

        for row in &self.view.rows {
            // The list item's label reads the way a screen reader user needs it: drug,
            // dose, status and WHOSE signature, in one utterance. Splitting these across
            // silent cells would make the signature state announceable only by hunting.
            let mut label = format!(
                "{}, {}, {}, {}",
                row.primary, row.dose, row.status_label, row.vouch_label
            );
            if row.will_be_signed {
                label.push_str(", will be signed");
            }
            // Row flags ride the SAME utterance rather than a separate node: a hazard
            // announced as its own item can be skipped past, and the one it warns about
            // ("the dose shown may not be this patient's") is a claim about THIS line.
            for flag in &row.flags {
                label.push_str(", ");
                label.push_str(flag);
            }
            fields.push(Field {
                id: format!("row-{}", row.group_id),
                role: Role::ListItem,
                label,
            });
            if row.can_cease {
                // "Stop" alone is ambiguous when buttons are read out of table context, so
                // the accessible name names the drug.
                fields.push(Field {
                    id: format!("cease-{}", row.group_id),
                    role: Role::Button,
                    label: format!("Stop {}", row.primary),
                });
            }
        }

        fields.push(Field {
            id: "sign-off".into(),
            role: Role::Button,
            label: if self.view.sign_off_enabled {
                format!(
                    "Sign off {} unsigned medication(s)",
                    self.view.sign_off_count
                )
            } else {
                // Never an empty label, even when the control is unavailable: an unlabelled
                // focusable control is exactly what `assert_complete` refuses.
                "Nothing to sign off".into()
            },
        });

        SemanticNode {
            title: self.title(),
            fields,
        }
    }
}

#[cfg(test)]
mod semantic_tests {
    use super::*;
    use cairn_gui_tab::context::{Capabilities, Context, PatientRef, UserRef};
    use cairn_gui_tab::{Role, Semantic};

    fn ctx() -> Context {
        Context {
            patient: Some(PatientRef {
                uuid: "00000000-0000-0000-0000-000000000001".into(),
                display_name: "Test Patient".into(),
            }),
            user: UserRef {
                actor_id: "kid".into(),
                display_name: "Dr A".into(),
            },
            capabilities: Capabilities::clinician_all(),
        }
    }

    fn fixture_tab() -> MedicationsTab {
        MedicationsTab::new(crate::view::build_view(
            &cairn_medication_view::fixtures::sample_chart(),
        ))
    }

    /// The accessibility bar that cost iced the reference UI: every focusable control
    /// carries a non-empty label and ids are unique.
    #[test]
    fn the_rendered_contract_is_accessibility_complete() {
        fixture_tab()
            .semantics(&ctx())
            .assert_complete()
            .expect("a11y contract must be complete");
    }

    #[test]
    fn every_current_row_contributes_a_labelled_cease_control() {
        let node = fixture_tab().semantics(&ctx());
        let cease_buttons = node
            .fields
            .iter()
            .filter(|f| f.id.starts_with("cease-"))
            .count();
        assert!(
            cease_buttons > 0,
            "each current drug needs its own cease control"
        );
        assert!(
            node.fields
                .iter()
                .filter(|f| f.id.starts_with("cease-"))
                .all(|f| f.label.contains("Stop ")),
            "a bare 'Stop' is ambiguous when read out of table context"
        );
    }

    /// ADR-0060 decision 2 at the accessibility layer. A warning rendered only as a
    /// coloured banner is invisible to a screen-reader user, so the incomplete-chart and
    /// withheld-line reports must exist as announced fields — otherwise the one clinician
    /// most dependent on the announcement is the one who never hears the chart is missing a
    /// drug.
    #[test]
    fn the_chart_warnings_are_announced_not_merely_displayed() {
        let node = fixture_tab().semantics(&ctx());
        let labels: Vec<&str> = node.fields.iter().map(|f| f.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("INCOMPLETE")),
            "the invisible-group warning must be in the a11y tree: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("will NOT be signed")),
            "the withheld-line warning must be in the a11y tree: {labels:?}"
        );
    }

    /// A healthy chart must not announce warnings it does not have — the same
    /// "quiet unless there is something to say" rule the view model follows.
    #[test]
    fn a_healthy_chart_announces_no_warnings() {
        let node = MedicationsTab::new(crate::view::build_view(
            &cairn_medication_view::PatientMedicationList::empty(),
        ))
        .semantics(&ctx());
        assert!(node
            .fields
            .iter()
            .all(|f| !f.label.contains("INCOMPLETE") && !f.label.contains("will NOT be signed")));
    }

    /// The sign-off control is always present and always labelled, even when it is
    /// unavailable: an unlabelled focusable control is exactly what `assert_complete`
    /// refuses, and a button that vanishes is a control the clinician cannot find again.
    #[test]
    fn the_sign_off_control_is_labelled_even_when_there_is_nothing_to_sign() {
        let node = MedicationsTab::new(crate::view::build_view(
            &cairn_medication_view::PatientMedicationList::empty(),
        ))
        .semantics(&ctx());
        let button = node
            .fields
            .iter()
            .find(|f| f.id == "sign-off")
            .expect("the gesture control must always exist");
        assert_eq!(button.role, Role::Button);
        assert!(!button.label.trim().is_empty());
    }
}
