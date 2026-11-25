//! Why a release is refused, and the plain sentence a clinician-facing screen shows.
//! Task 3 adds the signature/chain variants; this file owns all the copy.

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
}

impl Refusal {
    /// The one-sentence, plain-language explanation for the person at the screen.
    pub fn sentence(&self, who: Who) -> String {
        match self {
            Refusal::NoChannel => format!(
                "No release channel is set up on this {} yet, so it accepts no update. Whoever set it up must install the channel's first trust file.",
                who.word()
            ),
            Refusal::Malformed(_) => {
                "This update file is damaged or is not a Cairn update; fetch it again.".into()
            }
        }
    }
}
