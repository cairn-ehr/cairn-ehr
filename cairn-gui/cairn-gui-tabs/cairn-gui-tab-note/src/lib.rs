//! Progress-note tab. Slice 1 shows the note heading plus each cross-reference as
//! a focusable button whose id encodes the target id (`open:<id>`). Activating it
//! becomes Intent::OpenTab, which the shell routes to the OTHER pane (spec §5).
//! Rich editing (markdown-source + live preview) is a later slice — no WYSIWYG
//! widget exists in iced out of the box.
use cairn_gui_data::{ClinicalData, NoteRef};
use cairn_gui_tab::{Context, Field, Role, Semantic, SemanticNode, TabId};

#[derive(Default)]
pub struct NoteTab {
    pub refs: Vec<NoteRef>,
}

impl NoteTab {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&mut self, ctx: &Context, data: &dyn ClinicalData) {
        if let Some(p) = &ctx.patient {
            self.refs = data.note_refs(&p.uuid).unwrap_or_default();
        }
    }

    /// Parse the target tab/blob id out of a cross-reference field id.
    pub fn target_of(field_id: &str) -> Option<String> {
        field_id.strip_prefix("open:").map(|s| s.to_string())
    }
}

impl Semantic for NoteTab {
    fn tab_id(&self) -> TabId {
        TabId("note".into())
    }
    fn title(&self) -> String {
        "Current note".into()
    }
    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "note.heading".into(),
            role: Role::Heading,
            label: "Progress note".into(),
        }];
        for r in &self.refs {
            fields.push(Field {
                id: format!("open:{}", r.id),
                role: Role::Button,
                label: r.one_line.clone(),
            });
        }
        SemanticNode {
            title: "Current note".into(),
            fields,
        }
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
                display_name: "Amina".into(),
            }),
            user: UserRef {
                actor_id: "clin-1".into(),
                display_name: "Dr Vega".into(),
            },
            capabilities: Capabilities::clinician_all(),
        }
    }

    #[test]
    fn cross_reference_is_a_focusable_button_encoding_its_target() {
        let mut tab = NoteTab::new();
        tab.load(&ctx(), &MockData::with_fixtures());
        let node = tab.semantics(&ctx());
        node.assert_complete().unwrap();
        let btn = node
            .fields
            .iter()
            .find(|f| f.id.starts_with("open:"))
            .expect("a cross-ref button");
        assert_eq!(btn.role, cairn_gui_tab::Role::Button);
        assert!(
            btn.label.contains("X-ray"),
            "one-line summary is the button label"
        );
    }

    #[test]
    fn intent_target_can_be_parsed_from_field_id() {
        assert_eq!(
            NoteTab::target_of("open:xray-2026-07-01"),
            Some("xray-2026-07-01".to_string())
        );
        assert_eq!(NoteTab::target_of("note.body"), None);
    }
}
