//! Why a release is refused, and the plain sentence a clinician-facing screen shows.
//! This file owns all the copy: the sentences are fixed by the slice design, are read by
//! clinicians and caretakers, and never name cryptography.

/// Who is speaking: the sentence says "node" or "terminal" so the reader knows which
/// machine is refusing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Node,
    Terminal,
}

impl Who {
    fn word(self) -> &'static str {
        match self {
            Who::Node => "node",
            Who::Terminal => "terminal",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// No trust root is pinned: before the genesis ceremony nothing is accepted.
    #[error("no release channel")]
    NoChannel,
    /// Bytes that do not decode as the expected document. The string is a diagnostic
    /// for logs only; it is never shown in the sentence.
    #[error("malformed release document: {0}")]
    Malformed(String),
    /// The update belongs to a different release channel than the one pinned here.
    /// `offered` comes from an unverified document, so it is only ever shown sanitised.
    #[error("wrong channel: pinned {}, offered {}", sanitise(.pinned), sanitise(.offered))]
    WrongChannel { pinned: String, offered: String },
    /// The offered root chain skips versions: `pinned` is the last version the caller holds,
    /// `needs` the lowest version offered above the point where the chain broke.
    #[error("root chain gap: pinned {pinned}, needs {needs}")]
    ChainGap { pinned: u64, needs: u64 },
    /// A trust root lacks enough valid signatures from its predecessor's root signers (or,
    /// for genesis, its own).
    #[error("root {version}: {have} of {need} root signatures")]
    RootSignaturesShort { version: u64, have: u32, need: u32 },
    /// Two different verifiable trust roots claim the same version (ADR-0055 decision 5).
    #[error("root fork at version {version}")]
    Fork { version: u64 },
    /// The manifest lacks enough valid signatures from the newest root's release signers.
    #[error("{have} of {need} release signatures")]
    ReleaseSignaturesShort { have: u32, need: u32 },
    /// The offered release is not newer than the one already installed.
    #[error("release {offered} is not newer than {installed}")]
    NotNewer { installed: u64, offered: u64 },
    /// Artifact `index` does not match its signed digest and length (or does not exist).
    #[error("artifact {index} does not match the manifest")]
    ArtifactMismatch { index: usize },
    /// Terminal installer only: the verified release lists no `window` program for this
    /// machine. The fields are this terminal's own (never from the update).
    #[error("no program for {os} {arch} {platform}")]
    NoProgramForThisTerminal {
        os: String,
        arch: String,
        platform: String,
    },
    /// Terminal installer only: the verified program, run unprivileged, did not print the
    /// generation line and release its signed manifest names.
    #[error("the new program does not start")]
    WontStart,
    /// Terminal installer only: the staged program is larger than `staging::MAX_ARTIFACT`.
    #[error("the staged program is too large")]
    TooLarge,
}

impl Refusal {
    /// The one-sentence, plain-language explanation for the person at the screen.
    pub fn sentence(&self, who: Who) -> String {
        let who = who.word();
        match self {
            Refusal::NoChannel => format!(
                "No release channel is set up on this {who} yet, so it accepts no update. Whoever set it up must install the channel's first trust file."
            ),
            Refusal::Malformed(_) => {
                "This update file is damaged or is not a Cairn update; fetch it again.".into()
            }
            Refusal::WrongChannel { pinned, offered } => format!(
                "This update is from channel \"{}\", but this {who} follows \"{}\".",
                sanitise(offered),
                sanitise(pinned)
            ),
            Refusal::ChainGap { pinned, needs } => format!(
                "This {who} last accepted trust version {pinned}; this update needs version {needs}. Take in the newer trust file, or ask the Cairn project."
            ),
            Refusal::RootSignaturesShort { version, have, need } => format!(
                "Trust version {version} does not carry enough valid signatures from the version before it ({have} of {need}). Do not install it; report it to the Cairn project."
            ),
            Refusal::Fork { version } => format!(
                "Two different trust files both claim version {version}. Updates are stopped on this {who} until the Cairn project resolves it; nothing that runs is affected."
            ),
            Refusal::ReleaseSignaturesShort { have, need } => format!(
                "This update is not signed by enough of the Cairn project's release signers ({have} of {need}). Do not install it; report it to the Cairn project."
            ),
            Refusal::NotNewer { installed, offered } => format!(
                "This {who} already has release {installed}; the update offered is release {offered}, which is not newer."
            ),
            Refusal::ArtifactMismatch { .. } => {
                "A file in this update is not the file its signed list names. Do not install it; fetch the update again.".into()
            }
            Refusal::NoProgramForThisTerminal { os, arch, platform } => {
                format!("This update has no program for this terminal ({os} {arch} {platform}).")
            }
            Refusal::WontStart => "The new program does not start on this terminal; the one in place keeps running. Tell whoever looks after the node.".into(),
            Refusal::TooLarge => {
                "The update file is larger than any Cairn program; it was not installed.".into()
            }
        }
    }
}

/// The longest name a sentence quotes; a forged name longer than this is cut.
const MAX_SHOWN: usize = 64;

/// Make a name taken from an UNVERIFIED document safe to quote on screen and in logs
/// (ruling R9): drop control characters (line breaks, tabs) and the bidirectional
/// overrides/isolates that can make text read backwards, and cut it to `MAX_SHOWN`
/// characters, marking the cut with "…". Pure, so it is tested on its own.
pub(crate) fn sanitise(s: &str) -> String {
    let is_bidi = |c: char| matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    let mut kept = s.chars().filter(|c| !c.is_control() && !is_bidi(*c));
    let mut out: String = kept.by_ref().take(MAX_SHOWN).collect();
    if kept.next().is_some() {
        out.push('…');
    }
    out
}
