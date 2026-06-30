//! The clinical-form semantic model for Spike 0004 claim **A** (accessibility).
//!
//! Why this is in the pure core, not buried in the GUI: claim **A1** is "every
//! focusable control has a role + accessible label". That contract is *data*, so
//! we declare it here as a [`FormModel`], unit-test its completeness in CI, share
//! it with the iced widgets (one source of truth for labels), and let the GUI's
//! `--dump-a11y` print it as the **expected** accessibility tree. The operator
//! then confirms a live screen reader (Orca/NVDA) announces each entry — the dump
//! is the checklist, the screen-reader walk is the verdict.
//!
//! This deliberately does *not* read back iced/AccessKit internals (that API is
//! pre-1.0 and private); it states the contract the rendered form must honour, so
//! a divergence between "what we intended" and "what Orca says" is visible.
//!
//! Zero dependencies — including a tiny hand-rolled JSON writer — to keep the
//! core free of serde and friends.

/// Accessibility role of a control, mapped to the AccessKit/ARIA roles a screen
/// reader announces. Kept to the small set this form needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Heading,
    TextInput,
    Button,
    List,
    ListItem,
}

impl Role {
    /// The role name as a screen reader / AccessKit would label it.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Heading => "heading",
            Role::TextInput => "text_input",
            Role::Button => "button",
            Role::List => "list",
            Role::ListItem => "list_item",
        }
    }

    /// Is this role focusable? (Headings/lists are structural, not tab stops.)
    /// Claim A1 only requires labels on *focusable* controls; A2 requires every
    /// focusable control be keyboard-reachable.
    pub fn is_focusable(self) -> bool {
        matches!(self, Role::TextInput | Role::Button)
    }
}

/// One control in the form's accessibility contract.
#[derive(Debug, Clone)]
pub struct Field {
    /// Stable id (used to correlate the dump with the live tree).
    pub id: &'static str,
    /// Accessibility role.
    pub role: Role,
    /// The accessible label a screen reader must announce. Never empty.
    pub label: &'static str,
}

/// The whole form as an accessibility tree.
#[derive(Debug, Clone)]
pub struct FormModel {
    pub title: &'static str,
    pub fields: Vec<Field>,
}

/// Build the dense clinical form's a11y contract: a patient-identifier field,
/// the four multi-script name fields (shared with [`crate::corpus`] by script),
/// a two-row medication list with per-row add/remove, and a submit button.
///
/// This is intentionally dense and keyboard-driven — the point of the spike is a
/// *realistic* clinical surface, where paper-parity demands no mouse-hunting.
pub fn clinical_form() -> FormModel {
    let mut fields = vec![
        Field { id: "title", role: Role::Heading, label: "New patient — identity & medications" },
        Field { id: "identifier", role: Role::TextInput, label: "Patient identifier" },
        Field { id: "name_latn", role: Role::TextInput, label: "Name (Latin)" },
        Field { id: "name_arab", role: Role::TextInput, label: "Name (Arabic)" },
        Field { id: "name_deva", role: Role::TextInput, label: "Name (Devanagari)" },
        Field { id: "name_hani", role: Role::TextInput, label: "Name (Han / CJK)" },
        Field { id: "med_list", role: Role::List, label: "Medication list" },
    ];

    // Two medication rows, each a labelled item with add/remove buttons. Names
    // are explicit (not "button") so a screen-reader user knows *what* they act on.
    for (i, drug) in ["Amoxicillin 500 mg", "Metformin 1 g"].iter().enumerate() {
        let n = i + 1;
        fields.push(Field {
            id: leak_id("med_row", n),
            role: Role::ListItem,
            label: drug,
        });
        fields.push(Field {
            id: leak_id("med_remove", n),
            role: Role::Button,
            label: leak_label("Remove ", drug),
        });
    }
    fields.push(Field { id: "med_add", role: Role::Button, label: "Add medication" });
    fields.push(Field { id: "submit", role: Role::Button, label: "Save patient record" });

    FormModel { title: "Spike 0004 clinical form", fields }
}

// The form is static per process; leaking a handful of small id/label strings to
// get 'static is a deliberate, bounded trade for keeping `Field` borrow-free.
// (A real UI would carry owned Strings; the spike's form is fixed-size.)
fn leak_id(prefix: &str, n: usize) -> &'static str {
    Box::leak(format!("{prefix}_{n}").into_boxed_str())
}
fn leak_label(prefix: &str, drug: &str) -> &'static str {
    Box::leak(format!("{prefix}{drug}").into_boxed_str())
}

/// Serialise the form as the **expected** accessibility tree (minimal JSON,
/// no serde). The operator diffs this against what Accerciser/Orca reports.
pub fn to_json(model: &FormModel) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str(&format!("  \"title\": {},\n", json_str(model.title)));
    out.push_str("  \"fields\": [\n");
    for (i, f) in model.fields.iter().enumerate() {
        let comma = if i + 1 < model.fields.len() { "," } else { "" };
        out.push_str(&format!(
            "    {{ \"id\": {}, \"role\": {}, \"label\": {}, \"focusable\": {} }}{}\n",
            json_str(f.id),
            json_str(f.role.as_str()),
            json_str(f.label),
            f.role.is_focusable(),
            comma
        ));
    }
    out.push_str("  ]\n}\n");
    out
}

/// Minimal JSON string escaping (quotes + backslashes + the controls we might
/// hit). Enough for labels; not a general JSON library.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_focusable_control_has_a_nonempty_label() {
        // Claim A1 in CI-checkable form: no focusable control may be unlabelled.
        for f in clinical_form().fields {
            if f.role.is_focusable() {
                assert!(!f.label.is_empty(), "focusable {} has empty label", f.id);
            }
        }
    }

    #[test]
    fn every_field_has_a_nonempty_label_and_stable_id() {
        let mut seen = std::collections::HashSet::new();
        for f in clinical_form().fields {
            assert!(!f.label.is_empty(), "{} unlabelled", f.id);
            assert!(seen.insert(f.id), "duplicate id {}", f.id);
        }
    }

    #[test]
    fn includes_identifier_and_all_four_name_scripts() {
        let ids: Vec<&str> = clinical_form().fields.iter().map(|f| f.id).collect();
        for needed in ["identifier", "name_latn", "name_arab", "name_deva", "name_hani"] {
            assert!(ids.contains(&needed), "form must contain {needed}");
        }
    }

    #[test]
    fn has_at_least_the_expected_focusable_count() {
        // identifier + 4 names + 2 remove + add + submit = 9 focusable controls.
        let focusable = clinical_form()
            .fields
            .iter()
            .filter(|f| f.role.is_focusable())
            .count();
        assert_eq!(focusable, 9);
    }

    #[test]
    fn json_dump_mentions_every_label_and_is_balanced() {
        let model = clinical_form();
        let json = to_json(&model);
        for f in &model.fields {
            assert!(json.contains(f.label), "dump missing label {}", f.label);
        }
        // crude balance check on the hand-rolled writer
        assert_eq!(
            json.matches('{').count(),
            json.matches('}').count(),
            "unbalanced braces in dump"
        );
    }
}
