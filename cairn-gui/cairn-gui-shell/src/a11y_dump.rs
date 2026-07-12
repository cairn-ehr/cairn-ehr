//! The accessibility tree for the whole shell — chrome (panes, tab strips, the
//! resizable divider) plus each tab's fields. `--dump-a11y` prints this. This is
//! the widened Spike 0004 A-claim: shell-level a11y, not just a single form.
//!
//! The TAB BODY nodes (note, demographics) are read directly off each tab's
//! `semantics()`, the same call `app.rs` renders from — so they cannot drift
//! from what's on screen. The CHROME node (`shell_chrome`) is different: it is
//! the INTENDED/TARGET tree for panes, tab strips and the divider, hand-written
//! to match what `app.rs` renders today. The operator diffs the whole dump
//! against what Orca/NVDA actually announces to find where iced's current
//! rendering falls short of the target — a gap there is expected spike
//! feedback, not a bug in this file.
use cairn_gui_data::MockData;
use cairn_gui_tab::{Capabilities, Context, Field, PatientRef, Role, SemanticNode, Semantic, UserRef};
use cairn_gui_tab_demographics::DemographicsTab;
use cairn_gui_tab_note::NoteTab;

pub fn sample_ctx() -> Context {
    Context {
        patient: Some(PatientRef {
            uuid: "00000000-0000-0000-0000-0000000000aa".into(),
            display_name: "Amina أمينة अमीना 阿明娜".into(),
        }),
        user: UserRef { actor_id: "clin-1".into(), display_name: "Dr Vega".into() },
        capabilities: Capabilities::clinician_all(),
    }
}

fn shell_chrome(patient_name: &str) -> SemanticNode {
    SemanticNode {
        title: "Shell chrome".into(),
        fields: vec![
            // Matches app.rs's `text(format!("Patient: {name}"))` identity card.
            Field { id: "chrome.identity".into(), role: Role::Heading, label: format!("Patient: {}", patient_name) },
            // Panes and the divider: iced's `pane_grid` does not currently expose
            // these as accessible labels, so today's operator run will record them
            // as gaps against this target tree — that is intended spike feedback
            // (the whole point of the diff), not a claim that they are announced
            // by the shell as it stands.
            Field { id: "chrome.pane.left".into(), role: Role::Pane, label: "Left pane".into() },
            Field { id: "chrome.pane.right".into(), role: Role::Pane, label: "Right pane".into() },
            // Labels match `title()` as rendered by app.rs's tab-strip buttons.
            Field { id: "chrome.tab.left.note".into(), role: Role::Tab, label: "Current note".into() },
            Field { id: "chrome.tab.right.demographics".into(), role: Role::Tab, label: "Demographics".into() },
            Field { id: "chrome.divider".into(), role: Role::Divider, label: "Resize panes".into() },
        ],
    }
}

pub fn expected_tree(ctx: &Context) -> Vec<SemanticNode> {
    let data = MockData::with_fixtures();
    let mut note = NoteTab::new();
    note.load(ctx, &data);
    let mut demo = DemographicsTab::new();
    demo.load(ctx, &data);
    let patient_name = ctx.patient.as_ref().map(|p| p.display_name.as_str()).unwrap_or("");
    vec![shell_chrome(patient_name), note.semantics(ctx), demo.semantics(ctx)]
}

pub fn print_expected_tree() {
    for node in expected_tree(&sample_ctx()) {
        println!("# {}", node.title);
        for f in node.fields {
            println!("  [{}] {} — {}", f.role.as_str(), f.id, f.label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_tree_covers_shell_chrome_and_is_complete() {
        let nodes = expected_tree(&sample_ctx());
        // Must include a shell-chrome node with pane/tab/divider controls...
        let chrome = nodes.iter().find(|n| n.title == "Shell chrome").expect("chrome node present");
        assert!(chrome.fields.iter().any(|f| f.role == Role::Divider),
            "divider must be in the a11y tree (keyboard-resizable)");
        assert!(chrome.fields.iter().any(|f| f.role == Role::Tab),
            "tab strip stops must be in the a11y tree");
        // ...and every node must pass the completeness contract.
        for n in &nodes {
            n.assert_complete().unwrap_or_else(|e| panic!("incomplete node {}: {e}", n.title));
        }
    }
}
