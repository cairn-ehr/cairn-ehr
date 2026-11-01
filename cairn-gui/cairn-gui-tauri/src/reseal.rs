//! "Change my passphrase": a member changes their OWN passphrase from the window (sign-in
//! slice 2, Task 7).
//!
//! One press does, in order:
//!
//! 1. [`check_reseal_form`] (pure, no database): the new passphrase is usable, differs from the
//!    current one, and matches its repeat. A typo costs nothing and never touches the vault.
//! 2. Any held key is wiped (someone new is at the keyboard). The key is NOT put in the held
//!    session: this is not a clinical act.
//! 3. `cairn_node::vault_admin::change_passphrase` checks the current passphrase through the
//!    counted release door (so guessing it here is throttled like a sign-in), re-seals the same
//!    key under the new one, and drops it. A refused release is told with the signer's own
//!    sentences, so a replaced passphrase says so here too.
//!
//! No secret in any output: sentences are built from enums alone; a database error's text goes
//! to the operator's stderr only.
use crate::signer::{from_release_noting, KeyRefusal};
use crate::state::AppState;
use cairn_node::vault_admin::Reseal;
use serde::Deserialize;
use zeroize::Zeroizing;

/// What the form sends (camelCase). All three passphrases are wiped on drop.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResealForm {
    pub staff_id: String,
    pub current: Zeroizing<String>,
    pub new: Zeroizing<String>,
    pub new_again: Zeroizing<String>,
    pub hint: String,
}

/// Written by hand so a `{:?}` anywhere can never print a passphrase.
impl std::fmt::Debug for ResealForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResealForm")
            .field("staff_id", &self.staff_id)
            .field("current", &"<redacted>")
            .field("new", &"<redacted>")
            .field("new_again", &"<redacted>")
            .finish_non_exhaustive()
    }
}

/// Why the passphrase was not changed. Carries no database text and no form value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResealRefusal {
    MissingDetail,
    NewEmpty,
    NewSameAsCurrent,
    NewsDiffer,
    /// The node did not change it (answered with an error).
    NotChanged,
    /// No ordinary answer arrived: the passphrase may have been changed.
    OutcomeUnknown,
    Fixture,
}

impl ResealRefusal {
    /// The sentence the member reads. Full stops, not dashes (screen readers).
    pub fn sentence(&self) -> String {
        match self {
            ResealRefusal::MissingDetail => {
                "Enter your staff ID and your current passphrase. Nothing was changed.".into()
            }
            ResealRefusal::NewEmpty => {
                "Your new passphrase must not be empty. Nothing was changed.".into()
            }
            ResealRefusal::NewSameAsCurrent => {
                "Your new passphrase is the same as your current one. Nothing was changed.".into()
            }
            ResealRefusal::NewsDiffer => {
                "The two new passphrases do not match. Nothing was changed.".into()
            }
            ResealRefusal::NotChanged => "Your passphrase could not be changed. Nothing was \
                 recorded. Try again, and if it fails again, ask your IT support."
                .into(),
            ResealRefusal::OutcomeUnknown => "The node did not confirm the change, so your \
                 passphrase may have changed. Try signing in with the new one before trying again."
                .into(),
            ResealRefusal::Fixture => {
                "This window is showing sample data. No passphrase can be changed here.".into()
            }
        }
    }
}

/// Pure: is this form worth sending to the vault? Runs BEFORE any database call.
pub fn check_reseal_form(form: &ResealForm) -> Result<(), ResealRefusal> {
    if form.staff_id.trim().is_empty() || form.current.trim_matches(' ').is_empty() {
        return Err(ResealRefusal::MissingDetail);
    }
    if !cairn_node::vault::passphrase_is_usable(&form.new) {
        return Err(ResealRefusal::NewEmpty);
    }
    if *form.new == *form.current {
        return Err(ResealRefusal::NewSameAsCurrent);
    }
    if *form.new != *form.new_again {
        return Err(ResealRefusal::NewsDiffer);
    }
    Ok(())
}

#[tauri::command]
pub async fn change_passphrase(
    state: tauri::State<'_, AppState>,
    form: ResealForm,
) -> Result<String, String> {
    change_passphrase_impl(&state, form).await
}

/// The command's body, over plain `&AppState` so tests need no Tauri runtime.
pub async fn change_passphrase_impl(state: &AppState, form: ResealForm) -> Result<String, String> {
    check_reseal_form(&form).map_err(|r| r.sentence())?;
    state.forget_held_key().await;
    let Some(db) = state.db.as_ref() else {
        return Err(ResealRefusal::Fixture.sentence());
    };
    let fault = |e: anyhow::Error| {
        eprintln!(
            "a passphrase change could not be completed: {}",
            cairn_node::db_diagnosis::operator_chain(&e)
        );
        ResealRefusal::NotChanged.sentence()
    };
    let db = db.lock().await.map_err(|lost| fault(lost.into()))?;
    let hint = crate::enrol::optional(&form.hint);
    let staff_id = form.staff_id.trim();
    match cairn_node::vault_admin::change_passphrase(&db, staff_id, &form.current, &form.new, hint)
        .await
    {
        Ok(Reseal::Done) => Ok("Your passphrase is changed.".into()),
        Ok(Reseal::Refused(release)) => Err(from_release_noting(&db, staff_id, release)
            .await
            .err()
            .unwrap_or(KeyRefusal::Fault)
            .sentence()),
        Err(e) => {
            let e = db.explain(e);
            let refusal = if cairn_node::db_diagnosis::answered_with_error(&e) {
                ResealRefusal::NotChanged
            } else {
                ResealRefusal::OutcomeUnknown
            };
            eprintln!(
                "a passphrase change could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            Err(refusal.sentence())
        }
    }
}

#[cfg(test)]
#[path = "reseal_tests.rs"]
mod tests;
