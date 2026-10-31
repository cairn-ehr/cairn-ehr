//! Who signs the next clinical act, and what the clinician is told when nobody can.
//!
//! # The flow, for a reader new to it
//!
//! Every writing command (sign-off, cease, withhold, release, link, unlink, confirm,
//! "Different people") carries an optional [`Credential`] from the webview's signer strip: the
//! clinician's staff ID and personal passphrase. The command asks
//! `AppState::signer_for_act` for a key, which asks [`decide`] first:
//!
//! - a credential was typed → [`HeldDecision::Release`]: the node's vault checks the passphrase
//!   INSIDE Postgres and hands back that clinician's key for this one act (db/061,
//!   `cairn_node::vault::release_for_act`);
//! - no credential, and this terminal holds a key that has not run out → [`HeldDecision::UseHeld`];
//! - otherwise → [`HeldDecision::NeedCredential`].
//!
//! Whether a key is held at all is the terminal's `--key-hold-seconds` (spec §5.4): 0 on a
//! shared terminal (the default — the next person can never write as the last one), up to the
//! 15-minute `IDLE_TIMEOUT` on a personal workstation, where #288's whole-list sign-off stays
//! one gesture with one passphrase.
//!
//! # Why every sentence here is built from the enum alone
//!
//! A refusal is said aloud by VoiceOver and shown on a shared screen. [`KeyRefusal`] carries
//! no text from the database and no field of the credential, so no sentence can echo a
//! passphrase however a future error is worded. A fault's detail goes to the operator's
//! stderr, never to the screen.
use crate::state::{Now, SessionKey, IDLE_TIMEOUT};
use cairn_event::SigningKey;
use cairn_node::vault::Release;
use serde::Deserialize;
use std::time::Duration;
use zeroize::Zeroizing;

/// The staff ID and passphrase the clinician typed for ONE act. The webview sends
/// `{staffId, passphrase}` (R3). The passphrase is wiped when this is dropped.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    pub staff_id: String,
    pub passphrase: Zeroizing<String>,
}

/// Written by hand so a `{:?}` anywhere (a panic, a log line) can never print the passphrase.
impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credential")
            .field("staff_id", &self.staff_id)
            .field("passphrase", &"<redacted>")
            .finish()
    }
}

/// What [`decide`] says to do for the next act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldDecision {
    /// Sign with the key this terminal already holds.
    UseHeld,
    /// Ask the vault for the typed credential's key.
    Release,
    /// Nothing can sign: ask the clinician for their staff ID and passphrase.
    NeedCredential,
}

/// Pure: which key signs the next act. A credential ALWAYS means a release, even over a live
/// held key — whoever typed it is at the keyboard now (review focus 2). Without one, a held
/// key serves only while the hold is on (`key_hold > 0`) and has not run out.
pub fn decide(
    held: Option<&SessionKey>,
    credential: Option<&Credential>,
    key_hold: Duration,
    now: Now,
) -> HeldDecision {
    if credential.is_some() {
        return HeldDecision::Release;
    }
    match held {
        Some(key) if !key_hold.is_zero() && !key.is_expired(now, key_hold) => HeldDecision::UseHeld,
        _ => HeldDecision::NeedCredential,
    }
}

/// Why no key signs this act. Carries no database text and no credential (module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRefusal {
    /// No credential was typed and no key is held.
    NeedCredential,
    /// The passphrase did not match; `retry_after` is the back-off before the next try, if any.
    Wrong { retry_after: Option<Duration> },
    /// Too many wrong tries: the vault will not check another for `retry_after`.
    Waiting { retry_after: Duration },
    /// No one with that staff ID is enrolled on this node.
    Unknown,
    /// A superuser's key: it confers no clinical authorship (spec §5.1), so the vault refused it.
    NotForClinicalActs,
    /// The node could not be asked, or answered something we will not sign with. The detail is
    /// on the operator's stderr.
    Fault,
}

impl KeyRefusal {
    /// The sentence the clinician reads (and hears). Plain words; full stops, not dashes, so a
    /// screen reader pauses where a reader would.
    pub fn sentence(&self) -> String {
        match self {
            KeyRefusal::NeedCredential => "Enter your staff ID and passphrase to sign.".into(),
            KeyRefusal::Wrong { retry_after: None } => "That passphrase is not right.".into(),
            KeyRefusal::Wrong {
                retry_after: Some(wait),
            } => format!(
                "That passphrase is not right. Wait {} s, then try again.",
                seconds(*wait)
            ),
            KeyRefusal::Waiting { retry_after } => format!(
                "Too many tries. Wait {} s, then try again.",
                seconds(*retry_after)
            ),
            KeyRefusal::Unknown => "No one with that staff ID is enrolled here.".into(),
            KeyRefusal::NotForClinicalActs => {
                "A superuser cannot sign a clinical act. Enter your own staff ID.".into()
            }
            KeyRefusal::Fault => {
                "Your passphrase could not be checked. Nothing was signed. Try again, and if it \
                 fails again, ask your IT support."
                    .into()
            }
        }
    }
}

/// Whole seconds, rounded UP: a wait of 1.2 s said as "1 s" invites a press that is refused again.
fn seconds(d: Duration) -> u64 {
    d.as_millis().div_ceil(1000).max(1) as u64
}

/// Pure: a vault answer as the key that signs, or the refusal it means.
pub(crate) fn from_release(release: Release) -> Result<(SigningKey, String), KeyRefusal> {
    match release {
        Release::Released { key, kid } => Ok((key, kid)),
        Release::Wrong { retry_after } => Err(KeyRefusal::Wrong { retry_after }),
        Release::Waiting { retry_after } => Err(KeyRefusal::Waiting { retry_after }),
        Release::Unknown => Err(KeyRefusal::Unknown),
        Release::RefusedPurpose => Err(KeyRefusal::NotForClinicalActs),
    }
}

/// clap's parser for `--key-hold-seconds`: whole seconds, at most `IDLE_TIMEOUT`. Refused at
/// launch with the cap named, never silently clamped.
pub fn parse_key_hold(text: &str) -> Result<Duration, String> {
    let secs: u64 = text
        .parse()
        .map_err(|_| format!("not a whole number of seconds: {text}"))?;
    let cap = IDLE_TIMEOUT.as_secs();
    if secs > cap {
        return Err(format!(
            "the key hold is at most {cap} seconds (the idle timeout)"
        ));
    }
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
#[path = "signer_tests.rs"]
mod tests;
