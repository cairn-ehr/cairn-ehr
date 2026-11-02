//! "Forgotten passphrase": a superuser or an enroller gives a colleague a new key (sign-in
//! slice 2, Task 6).
//!
//! Same shape as [`crate::enrol`], and for the same reasons. One press does, in order:
//!
//! 1. [`check_rekey_form`] (pure, no database): the two new passphrases match and are usable.
//!    A credential with no passphrase is refused next (it would count as a wrong guess).
//! 2. Any held key is wiped (someone new is at the keyboard). The window asks whether the staff
//!    ID has a vault row (`vault::staff_id_taken`); with none — after a restore, design §5.8 —
//!    only a SUPERUSER may re-create it, and the door needs a display name and a contact, so
//!    [`missing_member_details`] refuses an enroller, or a form without them, BEFORE any key is
//!    released. Then the approver's key is released with `Purpose::Enrol`, used ONCE for the
//!    re-key, and dropped. Never `signer_for_act`: a held approver key would let the next person
//!    at the terminal re-key a colleague.
//! 3. `cairn_node::vault_admin::rekey_member` mints the new key, seals it under the new
//!    passphrase, and files it with the approver's attestation. The old passphrase then answers
//!    `Replaced` (for 14 days, or while the new key is unconfirmed or doubtful); the new key is unconfirmed until a superuser confirms it, unless a superuser
//!    approved it. [`rekey_success`] words the success line by who approved (the grant panel's
//!    read: `enrollers()` lists non-superusers only).
//!
//! A superuser is never re-keyed from the window (console only), and a re-key never changes a
//! member's role: the floor refuses both, and the sentence is the generic
//! [`RekeyRefusal::NotRecorded`], which names both checks. Approver refusals reuse
//! [`EnrolRefusal`](crate::enrol::EnrolRefusal), decided by the grant-panel read the window can make
//! (`enrol::release_enrol_key`), never by parsing a floor message.
//!
//! No secret in any output: sentences are built from enums alone; a database error's text goes
//! to the operator's stderr only.
use crate::enrol::{optional, release_enrol_key, standing, Standing};
use crate::signer::Credential;
use crate::state::AppState;
use cairn_node::vault_admin::Rekey;
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the re-key form sends (camelCase). Optional fields arrive as empty strings: with a vault
/// row they keep the stored value; with none (after a restore) the name and a contact are
/// required. Both passphrases are wiped on drop.
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
    /// The staff ID has no vault row (as after a restore), so the door re-creates it and needs a
    /// display name and a phone or an email. Decided before any key is released.
    NeedNameAndContact,
    /// The staff ID has no vault row and the approver is not a superuser: only a superuser may
    /// re-create one (db/063). Decided before any key is released.
    NeedSuperuserAfterRestore,
    /// No approver passphrase was typed. Decided before the vault is asked.
    NeedApprover,
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
            RekeyRefusal::NeedNameAndContact => "This staff ID has no vault entry here, as after \
                 a restore. Enter the member's display name and a phone or an email, then try \
                 again. Nobody was re-keyed."
                .into(),
            RekeyRefusal::NeedSuperuserAfterRestore => "This staff ID has no vault entry here, as \
                 after a restore. Only a superuser can re-key them. Ask a superuser. Nobody was \
                 re-keyed."
                .into(),
            RekeyRefusal::NeedApprover => "Enter your own staff ID and passphrase above, as a \
                 superuser or an enroller, to re-key a member. Nobody was re-keyed."
                .into(),
            RekeyRefusal::NotRecorded => "The member could not be re-keyed. Nothing was recorded. \
                 Check that the role is the one they are enrolled with: a re-key never changes a \
                 role. A superuser's key can only be replaced at the console. If it still fails, \
                 ask your IT support."
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

/// Pure: with no vault row for the staff ID (`has_member_row` false, as after a restore), the
/// door re-creates the row: only for a superuser (`approver` not listed in the grant panel's
/// read, which lists non-superusers only), and only with a display name and a phone or an email
/// (db/063 `cairn_vault_install_key`, the table's contact CHECK). The superuser rule is said
/// first: an enroller filling in the details would still be refused. With a row, blank fields
/// keep the stored values and nothing more is needed.
pub fn missing_member_details(
    has_member_row: bool,
    approver: Standing,
    form: &RekeyForm,
) -> Option<RekeyRefusal> {
    if has_member_row {
        return None;
    }
    if approver != Standing::NotListed {
        return Some(RekeyRefusal::NeedSuperuserAfterRestore);
    }
    let blank = |s: &str| s.trim().is_empty();
    let incomplete = blank(&form.display_name) || (blank(&form.phone) && blank(&form.email));
    incomplete.then_some(RekeyRefusal::NeedNameAndContact)
}

/// Pure: the success line, by who approved. `approver` is the approver's standing in the grant
/// panel's read (`None` when that read failed after the re-key had already landed): listed means
/// an enroller, so the key is unconfirmed until a superuser confirms it; not listed means a
/// superuser, whose approval confirmed it at once (db/063 `cairn_vault_rekey`).
pub fn rekey_success(staff_id: &str, approver: Option<Standing>) -> String {
    let base = format!("{staff_id} has a new key. Their old passphrase no longer works.");
    match approver {
        Some(Standing::NotListed) => base,
        Some(_) => format!("{base} A superuser must confirm the new key."),
        None => format!("{base} If an enroller approved it, a superuser must confirm the new key."),
    }
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
        return Err(RekeyRefusal::NeedApprover.sentence());
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
    // Asked BEFORE the approver's key is released, so an enroller facing a missing row, or a form
    // missing what a re-created row needs, is told plainly and costs the approver no release.
    // The floor stays the judge: a row deleted between this read and the write falls to the
    // generic `NotRecorded` sentence.
    let has_row = cairn_node::vault::staff_id_taken(&db, member.staff_id.trim())
        .await
        .map_err(|e| fault(db.explain(e)))?;
    let rows = cairn_node::vault_admin::enrollers(&db)
        .await
        .map_err(|e| fault(db.explain(e)))?;
    let approver = standing(&rows, &credential.staff_id);
    if let Some(r) = missing_member_details(has_row, approver, &member) {
        return Err(r.sentence());
    }
    // Released, used once below, dropped at the end of this function. Never stored.
    let key = release_enrol_key(
        &db,
        &credential,
        Some(&member.role),
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
    // The re-key has landed: a failed read here only words the line neutrally, never an error.
    let approver = match cairn_node::vault_admin::enrollers(&db).await {
        Ok(rows) => Some(standing(&rows, &credential.staff_id)),
        Err(e) => {
            eprintln!(
                "who approved the re-key could not be read: {}",
                cairn_node::db_diagnosis::operator_chain(&db.explain(e))
            );
            None
        }
    };
    Ok(rekey_success(r.staff_id, approver))
}

#[cfg(test)]
#[path = "rekey_tests.rs"]
mod tests;
