//! Source checks on the terminal's packaging (#909 slice C2, Task 10).
//!
//! The units, the install script and the release workflow cannot run in CI (they need a Pi,
//! systemd and a steward's hands), so these tests read their text and pin the properties a
//! reviewer must never see change silently: who runs what, which folder is writable by whom,
//! and that the build workflow never touches a signing key.

use std::path::PathBuf;

/// Read a file relative to the repository root (two levels above this crate).
fn repo_file(rel: &str) -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The non-comment lines of a unit/script/workflow, so a comment can not satisfy a check.
fn code_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn has_line(text: &str, wanted: &str) -> bool {
    code_lines(text).iter().any(|l| l == wanted)
}

#[test]
fn the_path_unit_watches_the_completion_marker() {
    let p = repo_file("packaging/terminal/cairn-terminal-install.path");
    assert!(has_line(
        &p,
        "PathExists=/var/lib/cairn-terminal/incoming/complete"
    ));
}

#[test]
fn the_installer_service_runs_as_root_and_reaps_its_children() {
    let s = repo_file("packaging/terminal/cairn-terminal-install.service");
    assert!(has_line(&s, "User=root"));
    assert!(has_line(&s, "KillMode=control-group"));
    assert!(has_line(&s, "Type=oneshot"));
}

#[test]
fn the_kiosk_never_runs_as_root() {
    let k = repo_file("packaging/terminal/cairn-window-kiosk.service");
    assert!(has_line(&k, "User=cairn-window"));
    assert!(!code_lines(&k)
        .iter()
        .any(|l| l == "User=root" || l == "User=0"));
    assert!(code_lines(&k).iter().any(|l| l.starts_with("ExecStart=")
        && l.contains("cage")
        && l.contains("/opt/cairn/bin/cairn-window")
        && l.contains("--release-dir /var/lib/cairn-terminal")));
}

#[test]
fn install_sh_sets_the_modes_the_installer_relies_on() {
    let s = repo_file("packaging/terminal/install.sh");
    assert!(has_line(&s, "set -eu"));
    assert!(code_lines(&s).iter().any(
        |l| l == "install -d -o root -g cairn-window -m 2770 /var/lib/cairn-terminal/incoming"
    ));
    assert!(code_lines(&s)
        .iter()
        .any(|l| l == "install -d -o root -g root -m 0755 /var/lib/cairn-terminal/state"));
}

#[test]
fn the_release_workflow_holds_no_secret_and_pins_every_action() {
    let w = repo_file(".github/workflows/release-build.yml");
    assert!(!w.contains("secrets."), "the build must never see a key");
    let uses: Vec<String> = code_lines(&w)
        .into_iter()
        .filter_map(|l| {
            let l = l.trim_start_matches("- ").to_string();
            l.strip_prefix("uses:").map(|v| v.trim().to_string())
        })
        .collect();
    assert!(!uses.is_empty());
    for u in uses {
        let sha = u.split('@').nth(1).unwrap_or("").split_whitespace().next();
        let sha = sha.unwrap_or("");
        assert!(
            sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
            "`uses: {u}` is not pinned to a full commit SHA"
        );
    }
}
