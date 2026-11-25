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

/// Every long flag the window's `Cli` accepts, read from its source, plus the two print flags
/// it answers before clap. A flag the window does not know makes clap exit and the kiosk loop.
fn window_flags() -> Vec<String> {
    let src = repo_file("cairn-gui/cairn-gui-tauri/src/main.rs");
    let body = src.split("struct Cli {").nth(1).expect("struct Cli");
    let body = body.split("\n}").next().unwrap();
    let mut flags = vec![
        "--print-schema-generation".to_string(),
        "--print-release-version".to_string(),
    ];
    let (mut in_attr, mut attr) = (false, String::new());
    for line in body.lines().map(str::trim) {
        if line.starts_with("#[arg(") {
            in_attr = true;
            attr.clear();
        }
        if in_attr {
            attr.push_str(line);
            if line.ends_with(")]") {
                in_attr = false;
            }
        } else if let Some((name, _)) = line.split_once(':') {
            if attr.contains("long") && !name.contains(' ') {
                flags.push(format!("--{}", name.replace('_', "-")));
            }
            attr.clear();
        }
    }
    flags
}

fn long_flags_in(line: &str) -> Vec<String> {
    line.split_whitespace()
        .filter(|t| t.starts_with("--") && t.len() > 2)
        .map(|t| t.split('=').next().unwrap().to_string())
        .collect()
}

#[test]
fn every_flag_the_kiosk_passes_is_a_real_window_flag() {
    let known = window_flags();
    assert!(known.contains(&"--terminal-dir".to_string()), "{known:?}");
    let kiosk = repo_file("packaging/terminal/cairn-window-kiosk.service");
    let script = repo_file("packaging/terminal/install.sh");
    let mut used: Vec<String> = code_lines(&kiosk)
        .iter()
        .filter(|l| l.starts_with("ExecStart="))
        .flat_map(|l| long_flags_in(l))
        .collect();
    used.extend(
        code_lines(&script)
            .iter()
            .filter(|l| l.contains("WINDOW_ARGS="))
            .flat_map(|l| long_flags_in(l.split("WINDOW_ARGS=").nth(1).unwrap())),
    );
    assert!(!used.is_empty());
    for f in used {
        assert!(known.contains(&f), "{f} is not a flag of the window");
    }
}

/// systemd's EnvironmentFile parser strips backslashes and quotes from an UNQUOTED value, so a
/// password `ab\cd` would reach the window as `abcd`. The connection string is written
/// double-quoted with systemd's four escapes (`"`, `\`, backtick, `$`), and a multi-line one
/// is refused before the system is touched.
#[test]
fn install_sh_quotes_the_connection_string_for_systemd() {
    let s = repo_file("packaging/terminal/install.sh");
    let lines = code_lines(&s);
    assert!(lines
        .iter()
        .any(|l| l.contains(r#"CAIRN_CONN="%s""#) && l.contains("$conn_quoted")));
    assert!(lines.iter().any(|l| l.contains(r#"s/[\\"`$]/\\&/g"#)));
    assert!(lines
        .iter()
        .any(|l| l.contains("die") && l.contains("one line")));
}

#[test]
fn install_sh_refuses_a_remote_terminal_without_its_connection() {
    // The kiosk always passes --terminal-dir, which the window refuses for a local database, so
    // a first install without BOTH --conn and --node-fingerprint would loop on a blank screen.
    let s = repo_file("packaging/terminal/install.sh");
    let lines = code_lines(&s);
    assert!(lines
        .iter()
        .any(|l| l.contains("die") && l.contains("--conn") && l.contains("--node-fingerprint")));
    assert!(lines.iter().any(|l| l.contains("CAIRN_WINDOW_CONN")));
    assert!(lines.iter().any(|l| l.contains("--replace-window")));
    assert!(lines.iter().any(|l| l.contains("$# -ge 2")));
}

#[test]
fn a_crash_that_leaves_the_marker_cannot_kill_the_path_unit() {
    let p = repo_file("packaging/terminal/cairn-terminal-install.path");
    assert!(code_lines(&p)
        .iter()
        .any(|l| l.starts_with("TriggerLimitIntervalSec=")));
    assert!(code_lines(&p)
        .iter()
        .any(|l| l.starts_with("TriggerLimitBurst=")));
    let r = repo_file("packaging/terminal/README.md");
    for needle in ["reset-failed", "CAIRN_WINDOW_CONN", "chmod +x"] {
        assert!(r.contains(needle), "README lacks {needle}");
    }
}

/// Ruling R27: the window fetches nothing until `state/status.json` exists, and only the installer
/// writes it, which the path unit starts only once a release is staged. So `install.sh` must start
/// the installer once itself (nothing staged: it writes the idle status with the terminal's pin),
/// through systemd, so that it can never run beside a path-unit-triggered run, and only after the
/// path unit is enabled. It then refuses to finish unless the status says idle (R29, below).
#[test]
fn install_sh_starts_the_installer_once_so_the_window_has_a_status() {
    let s = repo_file("packaging/terminal/install.sh");
    let lines = code_lines(&s);
    let at = |wanted: &str| lines.iter().position(|l| l.starts_with(wanted));
    let enabled =
        at("systemctl enable --now cairn-terminal-install.path").expect("path unit enabled");
    let started =
        at("systemctl start cairn-terminal-install.service").expect("installer started once");
    assert!(
        started > enabled,
        "the installer is started after the path unit is enabled"
    );
    assert!(lines[started..]
        .iter()
        .any(|l| l.contains("/var/lib/cairn-terminal/state/status.json") && l.contains("die")));
}

/// Ruling R29 (rollback): the installer refuses any release not newer than
/// `state/installed_release`, which a fresh terminal lacks, so a compromised window could stage
/// an older genuine release and have root downgrade the program. `install.sh` therefore
/// records the hand-installed program's own release (asked of it as `cairn-window`, never as
/// root) when the file is absent or the window was replaced, before the installer first runs.
#[test]
fn install_sh_records_the_hand_installed_release_before_the_installer_runs() {
    let s = repo_file("packaging/terminal/install.sh");
    let lines = code_lines(&s);
    let at = |wanted: &str| lines.iter().position(|l| l.contains(wanted));
    let asked =
        at("runuser -u cairn-window -- /opt/cairn/bin/cairn-window --print-release-version")
            .expect("the program is asked its release as cairn-window");
    let recorded = lines
        .iter()
        .position(|l| {
            l.starts_with("install -o root -g root -m 0644")
                && l.ends_with("/var/lib/cairn-terminal/state/installed_release")
        })
        .expect("installed_release written root-owned");
    let started = at("systemctl start cairn-terminal-install.service").unwrap();
    assert!(asked < recorded && recorded < started);
    assert!(lines[..asked]
        .iter()
        .any(|l| l.contains("installed_release") && l.contains("$replace")));
}

/// Ruling R29 (trust file): the installer's idle run with a trust file it cannot read writes a
/// refused status, so `install.sh` must require the idle state, not merely a status file, and
/// name the trust file when it is not there.
#[test]
fn install_sh_requires_the_idle_status() {
    let s = repo_file("packaging/terminal/install.sh");
    let lines = code_lines(&s);
    let check = lines
        .iter()
        .position(|l| l.contains(r#""state": "idle""#) && l.contains("status.json"))
        .expect("the idle state is required");
    assert!(lines[check..]
        .iter()
        .take(3)
        .any(|l| l.contains("die") && l.contains("/etc/cairn/channel-root.cbor")));
}
