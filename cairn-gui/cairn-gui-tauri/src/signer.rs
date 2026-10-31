//! Who signs the next clinical act, and what the clinician is told when nobody can.
//!
//! # The flow, for a reader new to it
//!
//! Every writing command (sign-off, cease, withhold, release, link, unlink, confirm,
//! "Different people") carries an optional [`Credential`] from the webview's signer strip: the
//! clinician's staff ID and personal passphrase. The command asks
//! `AppState::signer_for_act` for a key, which asks [`decide`] first:
//!
//! - a passphrase was typed → [`HeldDecision::Release`]: the node's vault checks it INSIDE
//!   Postgres and hands back that clinician's key for this one act (db/061,
//!   `cairn_node::vault::release_for_act`);
//! - no passphrase, and this terminal holds a key that has not run out → [`HeldDecision::UseHeld`]
//!   when no staff ID or the holder's own was typed, [`HeldDecision::Forget`] when someone
//!   else's was (an empty passphrase never reaches the vault, where it would count as a guess);
//! - otherwise → [`HeldDecision::NeedCredential`].
//!
//! Whether a key is held at all is the terminal's `--key-hold-seconds` (design §5.4, docs/superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md): 0 on a
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

impl Credential {
    /// Both fields empty once spaces are trimmed: a webview that sent its empty strip. Treated
    /// as NO credential, so it never wipes a held key (`AppState::signer_for_act`).
    pub fn is_blank(&self) -> bool {
        self.staff_id.trim_matches(' ').is_empty() && !self.has_passphrase()
    }

    /// Was a passphrase typed (anything but spaces)? Without one, [`decide`] never asks the
    /// vault: an empty guess would be counted as a wrong one. The same rule every enrolment
    /// door applies to a new passphrase, so no member can be enrolled under one this refuses.
    pub fn has_passphrase(&self) -> bool {
        cairn_node::vault::passphrase_is_usable(&self.passphrase)
    }
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
    /// Someone other than the holder typed a staff ID with no passphrase: wipe the held key
    /// (they must not sign as the holder), then ask for their passphrase.
    Forget,
}

/// Pure: which key signs the next act.
///
/// - A typed passphrase ALWAYS means a release, even over a live held key — whoever typed it is
///   at the keyboard now (review focus 2).
/// - A staff ID with NO passphrase is never sent to the vault (it would count as a wrong
///   guess). On a held terminal the strip keeps the holder's ID, so that ID alone is the holder
///   carrying on: the live held key signs. Anyone else's ID alone means someone new is at the
///   keyboard: [`HeldDecision::Forget`]. The ID is matched trimmed of spaces and exactly, as the
///   held key stores it (`SessionKey::new`); case lives in SQL alone (R7).
/// - No credential, or a blank one (both fields empty): a held key serves only while the hold is
///   on (`key_hold > 0`) and has not run out.
pub fn decide(
    held: Option<&SessionKey>,
    credential: Option<&Credential>,
    key_hold: Duration,
    now: Now,
) -> HeldDecision {
    let live = held.filter(|key| !key_hold.is_zero() && !key.is_expired(now, key_hold));
    // A blank credential (both fields empty) is no credential here too, not only by the caller's
    // filter: it must never wipe a held key.
    let credential = credential.filter(|c| !c.is_blank());
    match (credential, live) {
        (Some(c), _) if c.has_passphrase() => HeldDecision::Release,
        (Some(c), Some(key)) if c.staff_id.trim_matches(' ') == key.staff_id => {
            HeldDecision::UseHeld
        }
        (Some(_), Some(_)) => HeldDecision::Forget,
        (Some(_), None) => HeldDecision::NeedCredential,
        (None, Some(_)) => HeldDecision::UseHeld,
        (None, None) => HeldDecision::NeedCredential,
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
    /// A superuser's key: it confers no clinical authorship (design §5.1), so the vault refused it.
    NotForClinicalActs,
    /// This member's key was revoked or replaced: it can sign nothing here.
    NotCurrent,
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
            KeyRefusal::NotCurrent => {
                "Your signing key is no longer enrolled here. Nothing was signed. Ask a \
                 superuser to enrol you again."
                    .into()
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
        Release::NotCurrent => Err(KeyRefusal::NotCurrent),
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
