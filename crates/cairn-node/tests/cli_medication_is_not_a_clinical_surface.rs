//! #1136 — **the CLI's medication commands say they are not a clinical surface.**
//!
//! `cairn-node medication-assert` charts a drug with no allergy check, no warning and no override,
//! and `medication-list` draws no allergy marks. Requiring an override is a window rule (ADR-0099,
//! ADR-0021 soft policy), so that is by design only while the CLI is never used to chart for a
//! patient, and the person who could misuse it reads `--help`, not the ADRs. So the help says it.
//!
//! Spawns the binary because clap's help is assembled at runtime from the doc comments: asserting
//! against the source text would pass while the `--help` a human reads stayed silent (as
//! `restore_exit_vocabulary.rs` does). No database is needed.

/// `cairn-node <command> --help`, stdout and stderr together (a clap version that moves long help
/// cannot make an assertion pass vacuously by finding nothing to search).
fn help_of(command: &str) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_cairn-node"))
        .args([command, "--help"])
        .output()
        .expect("cairn-node --help");
    assert!(out.status.success(), "`{command} --help` exits 0");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn medication_assert_and_list_say_they_check_no_allergies() {
    for command in ["medication-assert", "medication-list"] {
        // Lower-cased: the words open a paragraph, so they start with a capital.
        let help = help_of(command).to_lowercase();
        assert!(
            help.contains("--json") || help.contains("--term"),
            "positive control: the real long help for `{command}`. Got:\n{help}"
        );
        for needle in ["not a clinical surface", "no allergy check"] {
            assert!(
                help.contains(needle),
                "`{command} --help` must say \"{needle}\" (#1136). Got:\n{help}"
            );
        }
    }
}
