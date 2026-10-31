//! A superuser enrols a staff member from the window (sign-in slice 1, Task 7).
//!
//! # The flow, for a reader new to it
//!
//! Two people stand at the terminal. The superuser has typed their own staff ID and passphrase
//! into the signer strip; the new member types THEIR OWN new passphrase, twice, into the
//! "Enrol staff" form. One press does, in order:
//!
//! 1. [`check_form`] (pure, no database): the two passphrases match, are not empty, and the form
//!    carries an ID, a name, a role and at least one of phone/email (the vault's CHECK demands
//!    a contact). A typo here costs nothing and never touches the vault.
//! 2. The superuser's key is released with `Purpose::Enrol`, used ONCE to vouch for the
//!    enrolment, and dropped. It is never put in the held session, whatever
//!    `--key-hold-seconds` says: an enrol act is not a clinical act, and a held superuser key
//!    would let the next person at a hold terminal enrol staff. That is why this does NOT go
//!    through `AppState::signer_for_act`, which may hold keys.
//! 3. `cairn_node::vault::enrol_member` mints the new member's key, seals it under their
//!    passphrase, and files it with the superuser's attestation.
//!
//! # No secret in any output
//!
//! Every sentence comes from [`EnrolRefusal`] or [`KeyRefusal`], built from enums alone. A
//! database error's text is never passed on (it goes to the operator's stderr), and neither
//! passphrase is in any string, `Debug` output or log line this file produces.
use crate::signer::{from_release, Credential, KeyRefusal};
use crate::state::AppState;
use cairn_node::vault::{NewMember, Purpose, Release};
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the enrol form sends (camelCase, like [`Credential`]). Optional fields arrive as empty
/// strings. Both passphrases are wiped on drop.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemberForm {
    pub staff_id: String,
    pub display_name: String,
    /// `clinician`, `nurse` or `superuser` from the form's list. A soft list: the floor accepts
    /// any role string (#168), so this window only insists it is not blank.
    pub role: String,
    pub phone: String,
    pub email: String,
    pub passphrase: Zeroizing<String>,
    pub passphrase_again: Zeroizing<String>,
    pub hint: String,
}

/// Written by hand so a `{:?}` anywhere can never print a passphrase.
impl std::fmt::Debug for NewMemberForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewMemberForm")
            .field("staff_id", &self.staff_id)
            .field("display_name", &self.display_name)
            .field("role", &self.role)
            .field("passphrase", &"<redacted>")
            .field("passphrase_again", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Why nobody was enrolled. Carries no text from the database and no field of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrolRefusal {
    PassphrasesDiffer,
    PassphraseEmpty,
    NeedContact,
    MissingDetail,
    /// The credential's key may not enrol (a clinician's: the vault said `refused-purpose`).
    NotForEnrolment,
    /// The vault or the node failed after the checks passed; nothing was recorded.
    NotRecorded,
    Fixture,
    /// The staff ID is already enrolled (checked before the superuser's key is released).
    AlreadyEnrolled,
    /// No superuser credential was typed.
    NeedSuperuser,
}

impl EnrolRefusal {
    /// The sentence the superuser reads. Full stops, not dashes (screen readers).
    pub fn sentence(&self) -> String {
        match self {
            EnrolRefusal::PassphrasesDiffer => {
                "The two new passphrases do not match. Nobody was enrolled.".into()
            }
            EnrolRefusal::PassphraseEmpty => {
                "The new member needs a passphrase. Nobody was enrolled.".into()
            }
            EnrolRefusal::NeedContact => {
                "Enter a phone number or an email address for the new member. Nobody was enrolled."
                    .into()
            }
            EnrolRefusal::MissingDetail => {
                "Enter the new member's staff ID, name and role. Nobody was enrolled.".into()
            }
            EnrolRefusal::NotForEnrolment => {
                "Only a superuser can enrol staff. Enter a superuser's staff ID and passphrase above."
                    .into()
            }
            EnrolRefusal::NotRecorded => "The member could not be enrolled. Nothing was recorded. \
                 Check that the staff ID is not already in use, then try again."
                .into(),
            EnrolRefusal::AlreadyEnrolled => {
                "That staff ID is already in use. Nobody was enrolled.".into()
            }
            EnrolRefusal::NeedSuperuser => {
                "Enter a superuser's staff ID and passphrase to enrol staff.".into()
            }
            EnrolRefusal::Fixture => {
                "This window is showing sample data. Nobody can be enrolled here.".into()
            }
        }
    }
}

/// Pure: is this form worth sending to the vault? Runs BEFORE any database call.
pub fn check_form(form: &NewMemberForm) -> Result<(), EnrolRefusal> {
    if form.passphrase.is_empty() {
        return Err(EnrolRefusal::PassphraseEmpty);
    }
    if *form.passphrase != *form.passphrase_again {
        return Err(EnrolRefusal::PassphrasesDiffer);
    }
    let blank = |s: &str| s.trim().is_empty();
    if blank(&form.staff_id) || blank(&form.display_name) || blank(&form.role) {
        return Err(EnrolRefusal::MissingDetail);
    }
    if blank(&form.phone) && blank(&form.email) {
        return Err(EnrolRefusal::NeedContact);
    }
    Ok(())
}

/// `None` for a blank optional field, else the trimmed text.
fn optional(s: &str) -> Option<&str> {
    Some(s.trim()).filter(|t| !t.is_empty())
}

#[tauri::command]
pub async fn enrol_member(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    member: NewMemberForm,
) -> Result<String, String> {
    enrol_member_impl(&state, credential, member).await
}

/// The command's body, over plain `&AppState` so tests need no Tauri runtime.
pub async fn enrol_member_impl(
    state: &AppState,
    credential: Credential,
    member: NewMemberForm,
) -> Result<String, String> {
    check_form(&member).map_err(|r| r.sentence())?;
    if credential.is_blank() {
        return Err(EnrolRefusal::NeedSuperuser.sentence());
    }
    let Some(db) = state.db.as_ref() else {
        return Err(EnrolRefusal::Fixture.sentence());
    };
    let fault = |e: anyhow::Error| {
        eprintln!("an enrolment could not be completed: {e:#}");
        EnrolRefusal::NotRecorded.sentence()
    };
    // One hold of the node connection for release + enrol; every error goes through `explain`
    // so a cut-off says so (#893).
    let db = db.lock().await.map_err(|lost| fault(lost.into()))?;
    // Asked BEFORE the superuser's key is released (R11), so a duplicate is told plainly and costs
    // the superuser no release attempt. Two simultaneous enrols of one ID can both pass this and
    // still fall to the generic `NotRecorded` sentence below: the floor stays the judge.
    if cairn_node::vault::staff_id_taken(&db, member.staff_id.trim())
        .await
        .map_err(|e| fault(db.explain(e)))?
    {
        return Err(EnrolRefusal::AlreadyEnrolled.sentence());
    }
    // Released, used once below, dropped at the end of this function. Never stored.
    let released = cairn_node::vault::release_for_act(
        &db,
        &credential.staff_id,
        &credential.passphrase,
        Purpose::Enrol,
    )
    .await
    .map_err(|e| fault(db.explain(e)))?;
    let key = match released {
        Release::Released { key, .. } => key,
        Release::RefusedPurpose => return Err(EnrolRefusal::NotForEnrolment.sentence()),
        other => {
            return Err(from_release(other)
                .err()
                .unwrap_or(KeyRefusal::Fault)
                .sentence())
        }
    };
    let new = NewMember {
        staff_id: member.staff_id.trim(),
        display_name: member.display_name.trim(),
        role: member.role.trim(),
        phone: optional(&member.phone),
        email: optional(&member.email),
        passphrase: &member.passphrase,
        hint: optional(&member.hint),
    };
    cairn_node::vault::enrol_member(&db, &new, &credential.staff_id, &key)
        .await
        .map_err(|e| fault(db.explain(e)))?;
    Ok(format!(
        "{} (staff ID {}) is enrolled as {}.",
        new.display_name, new.staff_id, new.role
    ))
}

#[cfg(test)]
#[path = "enrol_tests.rs"]
mod tests;
