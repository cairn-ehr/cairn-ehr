//! The pure accessibility contract shared by every tab and every piece of shell
//! chrome. It is *data*: declared here, unit-tested for completeness in CI, and
//! rendered by the iced layer. A divergence between "what we declared" and "what
//! a screen reader announces" is then visible (the shell's `--dump-a11y` prints
//! this tree; the operator confirms it against Orca/NVDA). Zero GUI dependency.

/// Accessibility role, mapped to what AccessKit / a screen reader announces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Heading,
    TextInput,
    Button,
    List,
    ListItem,
    /// A tab in a pane's tab strip (shell chrome).
    Tab,
    /// A workspace pane container (shell chrome).
    Pane,
    /// The draggable divider between panes (shell chrome).
    Divider,
}

impl Role {
    /// Focusable = a keyboard tab stop. Structural roles (headings, lists, panes)
    /// are announced but not stops. A1 requires labels on focusable controls;
    /// A2 requires each be keyboard-reachable.
    pub fn is_focusable(self) -> bool {
        matches!(self, Role::TextInput | Role::Button | Role::Tab | Role::Divider)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Heading => "heading",
            Role::TextInput => "text_input",
            Role::Button => "button",
            Role::List => "list",
            Role::ListItem => "list_item",
            Role::Tab => "tab",
            Role::Pane => "pane",
            Role::Divider => "divider",
        }
    }
}

/// One control in an accessibility contract.
#[derive(Debug, Clone)]
pub struct Field {
    pub id: String,
    pub role: Role,
    pub label: String,
}

/// A tab's (or shell-chrome region's) accessibility tree.
#[derive(Debug, Clone)]
pub struct SemanticNode {
    pub title: String,
    pub fields: Vec<Field>,
}

impl SemanticNode {
    /// CI-checkable form of accessibility claim A1: every focusable control has a
    /// non-empty label, and ids are unique. Returns Err(description) on the first
    /// violation so a failing test names the offender.
    pub fn assert_complete(&self) -> Result<(), String> {
        let mut seen = std::collections::HashSet::new();
        for f in &self.fields {
            if !seen.insert(&f.id) {
                return Err(format!("duplicate field id: {}", f.id));
            }
            if f.role.is_focusable() && f.label.trim().is_empty() {
                return Err(format!("focusable control {} has an empty label", f.id));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_focusable_field_must_have_a_nonempty_label() {
        let node = SemanticNode {
            title: "test".into(),
            fields: vec![Field { id: "a".into(), role: Role::Button, label: "".into() }],
        };
        assert!(node.assert_complete().is_err(), "empty label on a focusable control must fail");
    }

    #[test]
    fn complete_node_passes() {
        let node = SemanticNode {
            title: "test".into(),
            fields: vec![Field { id: "a".into(), role: Role::Button, label: "Save".into() }],
        };
        assert!(node.assert_complete().is_ok());
    }

    #[test]
    fn duplicate_ids_fail() {
        let node = SemanticNode {
            title: "t".into(),
            fields: vec![
                Field { id: "a".into(), role: Role::Button, label: "One".into() },
                Field { id: "a".into(), role: Role::Button, label: "Two".into() },
            ],
        };
        assert!(node.assert_complete().is_err(), "duplicate ids must fail");
    }

    #[test]
    fn structural_roles_are_not_focusable() {
        assert!(!Role::Heading.is_focusable());
        assert!(!Role::List.is_focusable());
        assert!(Role::TextInput.is_focusable());
        assert!(Role::Tab.is_focusable());
    }
}
