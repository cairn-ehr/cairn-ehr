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
