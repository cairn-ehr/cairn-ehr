//! The copy every refusal shows is fixed by the slice design (Global Constraints): a
//! clinician or caretaker reads it, so it must be plain and must never leak crypto jargon.

use crate::{Refusal, Who};

/// Every variant once, paired with the verbatim sentence for `who`.
fn all(who: &str) -> Vec<(Refusal, String)> {
    vec![
        (
            Refusal::NoChannel,
            format!("No release channel is set up on this {who} yet, so it accepts no update. Whoever set it up must install the channel's first trust file."),
        ),
        (
            Refusal::Malformed("diagnostic only".into()),
            "This update file is damaged or is not a Cairn update; fetch it again.".into(),
        ),
        (
            Refusal::WrongChannel {
                pinned: "official".into(),
                offered: "community".into(),
            },
            format!("This update is from channel \"community\", but this {who} follows \"official\"."),
        ),
        (
            Refusal::ChainGap { pinned: 2, needs: 4 },
            format!("This {who} last accepted trust version 2; this update needs version 4. Take in the newer trust file, or ask the Cairn project."),
        ),
        (
            Refusal::RootSignaturesShort {
                version: 3,
                have: 1,
                need: 2,
            },
            "Trust version 3 does not carry enough valid signatures from the version before it (1 of 2). Do not install it; report it to the Cairn project.".into(),
        ),
        (
            Refusal::Fork { version: 5 },
            format!("Two different trust files both claim version 5. Updates are stopped on this {who} until the Cairn project resolves it; nothing that runs is affected."),
        ),
        (
            Refusal::ReleaseSignaturesShort { have: 0, need: 1 },
            "This update is not signed by enough of the Cairn project's release signers (0 of 1). Do not install it; report it to the Cairn project.".into(),
        ),
        (
            Refusal::NotNewer {
                installed: 7,
                offered: 6,
            },
            format!("This {who} already has release 7; the update offered is release 6, which is not newer."),
        ),
        (
            Refusal::ArtifactMismatch { index: 0 },
            "A file in this update is not the file its signed list names. Do not install it; fetch the update again.".into(),
        ),
        // The terminal installer's own three (Task 9): only a terminal says them, so they
        // name no `{who}`.
        (
            Refusal::NoProgramForThisTerminal {
                os: "linux".into(),
                arch: "aarch64".into(),
                platform: "debian-12".into(),
            },
            "This update has no program for this terminal (linux aarch64 debian-12).".into(),
        ),
        (
            Refusal::WontStart,
            "The new program does not start on this terminal; the one in place keeps running. Tell whoever looks after the node.".into(),
        ),
        (
            Refusal::TerminalFault,
            "This terminal could not install the update because of a fault on the terminal itself; the program in place keeps running. Tell whoever looks after this terminal.".into(),
        ),
        (
            Refusal::TooLarge,
            "The update file is larger than any Cairn program; it was not installed.".into(),
        ),
    ]
}

#[test]
fn every_refusal_speaks_plainly() {
    for (who, word) in [(Who::Node, "node"), (Who::Terminal, "terminal")] {
        for (refusal, want) in all(word) {
            let got = refusal.sentence(who);
            assert_eq!(got, want, "{refusal:?} as {word}");
            assert!(
                !got.to_lowercase().contains("signature verification failed"),
                "{got}"
            );
        }
    }
}

#[test]
fn an_offered_channel_name_cannot_steer_the_screen() {
    // A forged bundle chooses `offered`: a line break, a quote, a right-to-left override
    // and a very long tail must not reach the sentence (or the log line) as given.
    let offered = format!("evil\n\"x\u{202E}y{}", "z".repeat(200));
    let r = Refusal::WrongChannel {
        pinned: "official".into(),
        offered,
    };
    let shown = format!("evil\"xy{}…", "z".repeat(57));
    assert_eq!(shown.chars().count(), 65);
    assert_eq!(
        r.sentence(Who::Terminal),
        format!("This update is from channel \"{shown}\", but this terminal follows \"official\".")
    );
    assert_eq!(
        r.to_string(),
        format!("wrong channel: pinned official, offered {shown}")
    );
}

#[test]
fn sanitise_keeps_short_plain_names_and_drops_controls() {
    use crate::refusal::sanitise;
    assert_eq!(sanitise("official"), "official");
    assert_eq!(sanitise("a\u{2066}b\u{2069}c\u{202A}\td\u{7}"), "abcd");
    let exactly_64 = "q".repeat(64);
    assert_eq!(sanitise(&exactly_64), exactly_64);
}
