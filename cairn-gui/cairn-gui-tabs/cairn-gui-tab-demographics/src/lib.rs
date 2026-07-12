//! Demographics tab — renders the fixture patient's identity + identifiers.
//! Contract-level (iced-free) here; the iced view() is added in the shell crate.
use cairn_gui_data::{ClinicalData, Demographics};
use cairn_gui_tab::{Context, Field, Role, Semantic, SemanticNode, TabId};

#[derive(Default)]
pub struct DemographicsTab {
    pub state: Option<Demographics>,
}

impl DemographicsTab {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lazy load: pull this patient's demographics from the port. Called eagerly
    /// when visible, or by the background scheduler when hidden.
    pub fn load(&mut self, ctx: &Context, data: &dyn ClinicalData) {
        if let Some(p) = &ctx.patient {
            self.state = data.demographics(&p.uuid).ok();
        }
    }
}

impl Semantic for DemographicsTab {
    fn tab_id(&self) -> TabId {
        TabId("demographics".into())
    }
    fn title(&self) -> String {
        "Demographics".into()
    }
    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "demographics.heading".into(),
            role: Role::Heading,
            label: "Patient demographics".into(),
        }];
        if let Some(d) = &self.state {
            fields.push(Field { id: "demographics.name".into(), role: Role::TextInput, label: format!("Name: {}", d.patient.display_name) });
            fields.push(Field { id: "demographics.sex".into(), role: Role::TextInput, label: format!("Sex: {}", d.sex) });
            fields.push(Field { id: "demographics.dob".into(), role: Role::TextInput, label: format!("Date of birth: {}", d.birth_date) });
            for (i, (system, value)) in d.identifiers.iter().enumerate() {
                fields.push(Field {
                    id: format!("demographics.id.{i}"),
                    role: Role::TextInput,
                    label: format!("{system}: {value}"),
                });
            }
        }
        SemanticNode { title: "Demographics".into(), fields }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_gui_data::MockData;
    use cairn_gui_tab::{Capabilities, UserRef};

    fn ctx() -> Context {
        Context {
            patient: Some(cairn_gui_tab::PatientRef {
                uuid: "00000000-0000-0000-0000-0000000000aa".into(),
                display_name: "Amina أمينة अमीना 阿明娜".into(),
            }),
            user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
            capabilities: Capabilities::clinician_all(),
        }
    }

    #[test]
    fn semantics_are_complete_after_load() {
        let mut tab = DemographicsTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        node.assert_complete().expect("every focusable control labelled");
        assert_eq!(tab.tab_id(), cairn_gui_tab::TabId("demographics".into()));
    }

    #[test]
    fn semantics_include_each_identifier() {
        let mut tab = DemographicsTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        let labels: Vec<String> = node.fields.iter().map(|f| f.label.clone()).collect();
        assert!(labels.iter().any(|l| l.contains("MRN")), "MRN identifier surfaced");
    }
}
