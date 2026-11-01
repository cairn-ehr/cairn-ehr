//! A superuser, or an enroller, enrols a staff member from the window (sign-in slice 1, Task 7).
//!
//! # The flow, for a reader new to it
//!
//! Two people stand at the terminal. The superuser has typed their own staff ID and passphrase
//! into the signer strip; the new member types THEIR OWN new passphrase, twice, into the
//! "Enrol staff" form. One press does, in order:
//!
//! 1. [`check_form`] (pure, no database): the two passphrases match, are usable (not empty, not
//!    only spaces: `cairn_node::vault::passphrase_is_usable`, the rule sign-in uses), and the form
//!    carries an ID, a name, a role and at least one of phone/email (the vault's CHECK demands
//!    a contact). A typo here costs nothing and never touches the vault.
//! 2. Any key this terminal holds is wiped: a typed passphrase means someone new is at the
//!    keyboard, as on a clinical act. Then the superuser's key is released with `Purpose::Enrol`, used ONCE to vouch for the
//!    enrolment, and dropped. It is never put in the held session, whatever
//!    `--key-hold-seconds` says: an enrol act is not a clinical act, and a held superuser key
//!    would let the next person at a hold terminal enrol staff. That is why this does NOT go
//!    through `AppState::signer_for_act`, which may hold keys.
//! 3. `cairn_node::vault::enrol_member` mints the new member's key, seals it under their
//!    passphrase, and files it with the superuser's attestation.
//!
//! # No secret in any output
//!
//! Every refusal sentence comes from [`EnrolRefusal`] or [`KeyRefusal`], built from enums alone
//! (the success line names the new member, which the superuser just typed). A
//! database error's text is never passed on (it goes to the operator's stderr), and neither
//! passphrase is in any string, `Debug` output or log line this file produces.
use crate::signer::{from_release_noting, Credential, KeyRefusal};
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
    /// The vault or the node refused or failed after the checks passed, with an answer: nothing
    /// was recorded.
    NotRecorded,
    /// A step of the enrolment got no ordinary database answer (a lost connection, a cut-off, or
    /// a failure with no database error at all): it may have been recorded. Errs towards
    /// "unknown": a failure BEFORE the write (the address query, minting the key) says so too,
    /// which claims less than the truth, never more. Said so, never "nothing was recorded" — a retry would then be told the ID
    /// is taken, and the natural next step, a new staff ID, is a second actor for one person.
    OutcomeUnknown,
    Fixture,
    /// The staff ID is already enrolled (checked before the superuser's key is released).
    AlreadyEnrolled,
    /// No superuser credential was typed.
    NeedSuperuser,
    /// An enroller tried to enrol (or re-key) a superuser or an admin: only a superuser may.
    ApproverMayNotEnrolThisRole,
    /// The approver holds the enroller grant but their own key still awaits a superuser's
    /// confirmation, so the floor will not take their word yet.
    ApproverNotConfirmed,
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
                "Only a superuser or an enroller can enrol staff. Enter their staff ID and passphrase above."
                    .into()
            }
            EnrolRefusal::ApproverMayNotEnrolThisRole => {
                "Only a superuser can enrol a superuser or an admin. Ask a superuser to do it."
                    .into()
            }
            EnrolRefusal::ApproverNotConfirmed => {
                "Your own key is waiting for a superuser to confirm it, so you cannot enrol staff yet. Ask a superuser."
                    .into()
            }
            EnrolRefusal::NotRecorded => {
                "The member could not be enrolled. Nothing was recorded. Try again, and if it \
                 fails again, ask your IT support."
                    .into()
            }
            EnrolRefusal::OutcomeUnknown => {
                "The node did not confirm the enrolment, so the member may have been enrolled. \
                 Check the staff list before enrolling them again, and never under a new staff ID."
                    .into()
            }
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
    if !cairn_node::vault::passphrase_is_usable(&form.passphrase) {
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

/// Pure: what a failed ENROLMENT write means. Only an ordinary ERROR answer proves the statement
/// rolled back; anything else may have landed (`cairn_node::db_diagnosis::answered_with_error`,
/// the rule every clinical write uses).
pub fn enrol_failure(e: &anyhow::Error) -> EnrolRefusal {
    if cairn_node::db_diagnosis::answered_with_error(e) {
        EnrolRefusal::NotRecorded
    } else {
        EnrolRefusal::OutcomeUnknown
    }
}

/// Pure: roles only a superuser may vouch for (the floor's rule, db/063; this is the courtesy copy).
pub fn role_needs_superuser(role: &str) -> bool {
    matches!(role.trim(), "superuser" | "admin")
}

/// What the grant panel's read (`vault_admin::enrollers`, which lists NON-superusers only) says
/// about the approver. Pure over the rows, so the decisions below are testable without a database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Not in the list: a superuser (or unknown, which the vault would not have released for).
    NotListed,
    /// A non-superuser holding the enroller grant now.
    Granted,
    /// A non-superuser without the grant (a clinician, say).
    NotGranted,
}

pub fn standing(rows: &[cairn_node::vault_admin::EnrollerRow], approver: &str) -> Standing {
    match rows
        .iter()
        .find(|r| r.staff_id.eq_ignore_ascii_case(approver.trim()))
    {
        None => Standing::NotListed,
        Some(r) if r.enroller => Standing::Granted,
        Some(_) => Standing::NotGranted,
    }
}

/// Pure: AFTER the vault released the approver's enrol key (so they are a superuser or a confirmed
/// enroller), may they vouch for this role? A superuser always may (never refused here, even with
/// a stray grant: the floor lets a superuser approve first); an enroller may not for superuser/admin.
pub fn precheck_decision(standing: Standing, role: &str) -> Option<EnrolRefusal> {
    (standing != Standing::NotListed && role_needs_superuser(role))
        .then_some(EnrolRefusal::ApproverMayNotEnrolThisRole)
}

/// Pure: the vault refused the enrol purpose. A member holding the grant is waiting on a
/// superuser's confirmation of their key; anyone else gets the plain refusal.
pub fn refused_purpose_decision(standing: Standing) -> EnrolRefusal {
    if standing == Standing::Granted {
        EnrolRefusal::ApproverNotConfirmed
    } else {
        EnrolRefusal::NotForEnrolment
    }
}

/// Release the approver's key with `Purpose::Enrol` for ONE use by the caller (enrol or re-key);
/// the caller drops it. Shared so the two commands cannot drift. `not_recorded` is the caller's
/// own sentence for a database fault; every `Err` is a finished sentence. The courtesy refusal
/// for superuser/admin targets runs after the release (see [`precheck_decision`]); the floor
/// stays the authority and a floor message is never parsed.
pub async fn release_enrol_key(
    db: &cairn_gui_live::ConnectionGuard<'_>,
    credential: &Credential,
    role: &str,
    not_recorded: &str,
) -> Result<cairn_event::SigningKey, String> {
    let fault = |e: anyhow::Error| {
        let e = db.explain(e);
        eprintln!(
            "an enrol-purpose release could not be completed: {}",
            cairn_node::db_diagnosis::operator_chain(&e)
        );
        not_recorded.to_string()
    };
    let released = cairn_node::vault::release_for_act(
        db,
        &credential.staff_id,
        &credential.passphrase,
        Purpose::Enrol,
    )
    .await
    .map_err(fault)?;
    let key = match released {
        Release::Released { key, .. } => key,
        Release::RefusedPurpose => {
            let rows = cairn_node::vault_admin::enrollers(db)
                .await
                .map_err(fault)?;
            return Err(refused_purpose_decision(standing(&rows, &credential.staff_id)).sentence());
        }
        Release::OverPowered { why } => {
            // Operator diagnostics, as the clinical signer gives; `why` is a fixed sentence.
            eprintln!("the window's connection is over-powered: {why}");
            return Err(KeyRefusal::OverPowered.sentence());
        }
        other => {
            return Err(from_release_noting(db, &credential.staff_id, other)
                .await
                .err()
                .unwrap_or(KeyRefusal::Fault)
                .sentence())
        }
    };
    if role_needs_superuser(role) {
        let rows = cairn_node::vault_admin::enrollers(db)
            .await
            .map_err(fault)?;
        if let Some(r) = precheck_decision(standing(&rows, &credential.staff_id), role) {
            return Err(r.sentence());
        }
    }
    Ok(key)
}

/// `None` for a blank optional field, else the trimmed text.
pub fn optional(s: &str) -> Option<&str> {
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
    // No passphrase, no superuser: a staff ID alone (the strip keeps it on a held terminal) must
    // never reach the vault, where it would count as the superuser's wrong guess.
    if !credential.has_passphrase() {
        return Err(EnrolRefusal::NeedSuperuser.sentence());
    }
    // Whoever typed that passphrase is at the keyboard now: the last holder's key must not sign
    // the next act that carries an empty strip (ADR-0083 decision 8, as on a clinical act).
    state.forget_held_key().await;
    let Some(db) = state.db.as_ref() else {
        return Err(EnrolRefusal::Fixture.sentence());
    };
    let fault = |e: anyhow::Error| {
        eprintln!(
            "an enrolment could not be completed: {}",
            cairn_node::db_diagnosis::operator_chain(&e)
        );
        EnrolRefusal::NotRecorded.sentence()
    };
    // One hold of the node connection for release + enrol; every error goes through `explain`
    // so the operator's stderr says when the window cut it off (#893). The screen gets the
    // sentence alone: before the enrolment write, a failure wrote nothing.
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
    let key = release_enrol_key(
        &db,
        &credential,
        &member.role,
        &EnrolRefusal::NotRecorded.sentence(),
    )
    .await?;
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
        .map_err(|e| {
            let e = db.explain(e);
            eprintln!(
                "an enrolment could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            enrol_failure(&e).sentence()
        })?;
    Ok(format!(
        "{} (staff ID {}) is enrolled as {}.",
        new.display_name, new.staff_id, new.role
    ))
}

#[cfg(test)]
#[path = "enrol_tests.rs"]
mod tests;
