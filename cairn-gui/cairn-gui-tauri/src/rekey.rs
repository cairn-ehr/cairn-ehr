//! "Forgotten passphrase": a superuser or an enroller gives a colleague a new key (sign-in
//! slice 2, Task 6).
//!
//! Same shape as [`crate::enrol`], and for the same reasons. One press does, in order:
//!
//! 1. [`check_rekey_form`] (pure, no database): the two new passphrases match and are usable.
//! 2. Any held key is wiped (someone new is at the keyboard). The approver's key is released
//!    with `Purpose::Enrol`, used ONCE for the re-key, and dropped. Never `signer_for_act`: a
//!    held approver key would let the next person at the terminal re-key a colleague.
//! 3. `cairn_node::vault_admin::rekey_member` mints the new key, seals it under the new
//!    passphrase, and files it with the approver's attestation. The old passphrase then answers
//!    `Replaced`; the new key is unconfirmed until a superuser confirms it.
//!
//! A superuser is never re-keyed from the window (console only): the floor refuses, and the
//! sentence is the generic [`RekeyRefusal::NotRecorded`]. Approver refusals reuse
//! [`EnrolRefusal`](crate::enrol::EnrolRefusal), decided by the grant-panel read the window can make
//! (`enrol::release_enrol_key`), never by parsing a floor message.
//!
//! No secret in any output: sentences are built from enums alone; a database error's text goes
//! to the operator's stderr only.
use crate::enrol::{optional, release_enrol_key};
use crate::signer::Credential;
use crate::state::AppState;
use cairn_node::vault_admin::Rekey;
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the re-key form sends (camelCase). Optional fields arrive as empty strings, which keep
/// the stored value. Both passphrases are wiped on drop.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RekeyForm {
    pub staff_id: String,
    pub role: String,
    pub display_name: String,
    pub phone: String,
    pub email: String,
    pub passphrase: Zeroizing<String>,
    pub passphrase_again: Zeroizing<String>,
    pub hint: String,
}

/// Written by hand so a `{:?}` anywhere can never print a passphrase.
impl std::fmt::Debug for RekeyForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RekeyForm")
            .field("staff_id", &self.staff_id)
            .field("role", &self.role)
            .field("passphrase", &"<redacted>")
            .field("passphrase_again", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Why nobody was re-keyed. Carries no database text and no form value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RekeyRefusal {
    PassphrasesDiffer,
    PassphraseEmpty,
    MissingDetail,
    /// The floor or node refused with an answer: nothing was recorded.
    NotRecorded,
    /// No ordinary answer arrived: the key may have been replaced.
    OutcomeUnknown,
    Fixture,
}

impl RekeyRefusal {
    /// The sentence the approver reads. Full stops, not dashes (screen readers).
    pub fn sentence(&self) -> String {
        match self {
            RekeyRefusal::PassphrasesDiffer => {
                "The two new passphrases do not match. Nobody was re-keyed.".into()
            }
            RekeyRefusal::PassphraseEmpty => {
                "The member needs a new passphrase. Nobody was re-keyed.".into()
            }
            RekeyRefusal::MissingDetail => {
                "Enter the member's staff ID and role. Nobody was re-keyed.".into()
            }
            RekeyRefusal::NotRecorded => "The member could not be re-keyed. Nothing was recorded. \
                 A superuser's key can only be replaced at the console. Otherwise try again, and \
                 if it fails again, ask your IT support."
                .into(),
            RekeyRefusal::OutcomeUnknown => {
                "The node did not confirm the re-key, so the key may have been replaced. Ask the \
                 member to try signing in with the new passphrase before re-keying again."
                    .into()
            }
            RekeyRefusal::Fixture => {
                "This window is showing sample data. Nobody can be re-keyed here.".into()
            }
        }
    }
}

/// Pure: is this form worth sending to the vault? Runs BEFORE any database call.
pub fn check_rekey_form(form: &RekeyForm) -> Result<(), RekeyRefusal> {
    if !cairn_node::vault::passphrase_is_usable(&form.passphrase) {
        return Err(RekeyRefusal::PassphraseEmpty);
    }
    if *form.passphrase != *form.passphrase_again {
        return Err(RekeyRefusal::PassphrasesDiffer);
    }
    if form.staff_id.trim().is_empty() || form.role.trim().is_empty() {
        return Err(RekeyRefusal::MissingDetail);
    }
    Ok(())
}

#[tauri::command]
pub async fn rekey_member(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    member: RekeyForm,
) -> Result<String, String> {
    rekey_member_impl(&state, credential, member).await
}

/// The command's body, over plain `&AppState` so tests need no Tauri runtime.
pub async fn rekey_member_impl(
    state: &AppState,
    credential: Credential,
    member: RekeyForm,
) -> Result<String, String> {
    check_rekey_form(&member).map_err(|r| r.sentence())?;
    // A staff ID alone must never reach the vault, where it would count as a wrong guess.
    if !credential.has_passphrase() {
        return Err(crate::enrol::EnrolRefusal::NeedSuperuser.sentence());
    }
    state.forget_held_key().await;
    let Some(db) = state.db.as_ref() else {
        return Err(RekeyRefusal::Fixture.sentence());
    };
    let fault = |e: anyhow::Error| {
        eprintln!(
            "a re-key could not be completed: {}",
            cairn_node::db_diagnosis::operator_chain(&e)
        );
        RekeyRefusal::NotRecorded.sentence()
    };
    let db = db.lock().await.map_err(|lost| fault(lost.into()))?;
    // Released, used once below, dropped at the end of this function. Never stored.
    let key = release_enrol_key(
        &db,
        &credential,
        &member.role,
        &RekeyRefusal::NotRecorded.sentence(),
    )
    .await?;
    let r = Rekey {
        staff_id: member.staff_id.trim(),
        role: member.role.trim(),
        display_name: optional(&member.display_name),
        phone: optional(&member.phone),
        email: optional(&member.email),
        passphrase: &member.passphrase,
        hint: optional(&member.hint),
    };
    cairn_node::vault_admin::rekey_member(&db, &r, &credential.staff_id, &key)
        .await
        .map_err(|e| {
            let e = db.explain(e);
            eprintln!(
                "a re-key could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            if cairn_node::db_diagnosis::answered_with_error(&e) {
                RekeyRefusal::NotRecorded
            } else {
                RekeyRefusal::OutcomeUnknown
            }
            .sentence()
        })?;
    Ok(format!(
        "{} has a new key. Their old passphrase no longer works; a superuser must confirm the new key.",
        r.staff_id
    ))
}

#[cfg(test)]
#[path = "rekey_tests.rs"]
mod tests;
