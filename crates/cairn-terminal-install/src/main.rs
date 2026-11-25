//! `cairn-terminal-install` (#909 slice C2): the root-owned installer on a Cairn terminal.
//!
//! systemd's path unit starts it when the window (an unprivileged courier) has staged a
//! release and written `incoming/complete`. It verifies the staged bundle against THIS
//! terminal's own pin (never anything the node or the window says), test-runs the new program
//! unprivileged, swaps it in atomically keeping the previous one, records its state in
//! `status.json`, and clears the staging folder. A defect here would put an unsigned program in
//! front of the screen clinicians type their passphrase into, so the work is split small:
//! `decide.rs` (pure verdict), `files.rs` (hostile reads, own state), `swap.rs` (the swap),
//! `status.rs` (the status file), `run.rs` (one run, start to finish). This file only resolves
//! the command line and the machine.

#[cfg(not(unix))]
compile_error!(
    "cairn-terminal-install runs only on a Unix terminal: it swaps the program by rename and \
     test-runs it under another uid."
);

mod decide;
mod files;
#[cfg(test)]
mod fixtures;
mod run;
mod status;
mod swap;

use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

/// Install the window release the window staged, if this terminal's own pin verifies it.
#[derive(Parser)]
#[command(name = "cairn-terminal-install")]
struct Cli {
    /// The staging folder the window writes.
    #[arg(long, default_value = "/var/lib/cairn-terminal/incoming")]
    staging: PathBuf,
    /// This terminal's own state folder (roots, installed release, incident, status).
    #[arg(long, default_value = "/var/lib/cairn-terminal/state")]
    state: PathBuf,
    /// The installed window program.
    #[arg(long, default_value = "/opt/cairn/bin/cairn-window")]
    program: PathBuf,
    /// The unprivileged user the new program is test-run as (never root).
    #[arg(long, default_value = "cairn-window")]
    run_as: String,
    /// The channel's first trust file, read only until the terminal has accepted a release.
    #[arg(long, default_value = "/etc/cairn/channel-root.cbor")]
    genesis_root: PathBuf,
}

/// How long the new program may take to answer each print flag.
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn main() -> ExitCode {
    let cli = Cli::parse();
    // No /etc/os-release: no platform, so no program in any release fits ("unknown").
    let os_release = std::fs::read_to_string("/etc/os-release").ok();
    let this = cairn_release::staging::Target {
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        platform: os_release
            .as_deref()
            .and_then(cairn_release::staging::platform_of)
            .unwrap_or_else(|| "unknown".into()),
    };
    let cfg = run::Config {
        staging: cli.staging,
        state: cli.state,
        program: cli.program,
        run_as: Some(cli.run_as),
        genesis_root: cli.genesis_root,
        this,
        check_timeout: CHECK_TIMEOUT,
    };
    if run::install_staged(&cfg) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_defaults_are_the_terminal_layout() {
        let cli = Cli::try_parse_from(["cairn-terminal-install"]).unwrap();
        assert_eq!(
            cli.staging,
            PathBuf::from("/var/lib/cairn-terminal/incoming")
        );
        assert_eq!(cli.state, PathBuf::from("/var/lib/cairn-terminal/state"));
        assert_eq!(cli.program, PathBuf::from("/opt/cairn/bin/cairn-window"));
        assert_eq!(cli.run_as, "cairn-window");
        assert_eq!(
            cli.genesis_root,
            PathBuf::from("/etc/cairn/channel-root.cbor")
        );
    }
}
