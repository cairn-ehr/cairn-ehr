//! The superuser's panel (sign-in slice 2, Task 8): grant or withdraw the enroller role, and judge
//! the keys that await a superuser (Confirm or Repudiate).
//!
//! # The flow, for a reader new to it
//!
//! Both lists are plain reads (`vault_admin::enrollers`, `vault_admin::unconfirmed`); the page
//! re-reads them after every write. A WRITE (`set_enroller`, `judge_key`) is a superuser act:
//!
//! 1. Any held key is wiped (someone new may be at the keyboard) and a credential with no
//!    passphrase is refused before the vault sees it (it would count as a wrong guess).
//! 2. [`superuser_refusal`] (pure over `enrollers()`, which lists NON-superusers only): a typed
//!    ID found there is not a superuser, so it is told so in one plain sentence BEFORE the vault
//!    is asked. The floor still refuses an enroller; this is only the courtesy copy, and no floor
//!    message is ever parsed.
//! 3. The superuser's key is released with `Purpose::Enrol` through the shared
//!    [`release_enrol_key`], used for ONE signed act and dropped. It never enters the held
//!    session (that is why this does not use `AppState::signer_for_act`).
//!
//! # No confirmation dialog for Repudiate
//!
//! Principle 3 (paper-parity): a "are you sure?" dialog fails paper-parity and is not a safety
//! mechanism. Repudiate is safe by other means: the act is signed by the superuser, it is audited,
//! and a wrong repudiation is repaired by a re-key (the member is given a fresh key).
//!
//! No secret is in any output: sentences are built from enums and the rows' own display text.
use crate::enrol::{release_enrol_key, standing, Standing};
use crate::signer::{local_at, Credential};
use crate::state::AppState;
use cairn_node::vault_admin::{EnrollerRow, Unconfirmed};
use serde::Serialize;

/// The sentence for "this credential belongs to someone who is not a superuser".
pub const ONLY_SUPERUSER: &str =
    "Only a superuser can do this. Enter a superuser's staff ID and passphrase above.";
const NOT_RECORDED: &str = "That could not be done. Nothing was recorded. Try again, and if it \
     fails again, ask your IT support.";
const OUTCOME_UNKNOWN: &str = "The node did not confirm the change, so it may have been made. \
     Look at the lists again before trying twice.";
const FIXTURE: &str = "This window is showing sample data. Nothing can be changed here.";
const NEED_SUPERUSER: &str = "Enter a superuser's staff ID and passphrase to do this.";

/// One key on the worklist, as the page shows it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct UnconfirmedView {
    pub kid: String,
    pub text: String,
    pub doubtful: bool,
}

/// One non-superuser member in the grant list, as the page shows it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct EnrollerView {
    pub staff_id: String,
    pub text: String,
    pub enroller: bool,
}

/// Pure: the sentence for one worklist row. `now` supplies the local time zone. An approver who
/// is no longer in the vault is said so, never left blank (principle 4).
pub fn unconfirmed_text<Tz: chrono::TimeZone>(u: &Unconfirmed, now: &chrono::DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let who = if u.display_name.is_empty() {
        u.staff_id.clone()
    } else {
        format!("{} ({})", u.display_name, u.staff_id)
    };
    let approver = if u.approver_staff_id.is_empty() {
        "someone no longer in the vault"
    } else {
        &u.approver_staff_id
    };
    let mut text = format!(
        "{who}: new key approved by {approver} on {}.",
        local_at(&u.approved_at, now)
    );
    if u.doubtful {
        text.push_str(" Doubtful: approved by a repudiated enroller.");
    }
    text
}

/// Pure: the sentence for one grant-list row.
pub fn enroller_text(r: &EnrollerRow) -> String {
    let can = if r.enroller {
        "can enrol staff"
    } else {
        "cannot enrol staff"
    };
    format!("{} ({}): {can}.", r.display_name, r.staff_id)
}

/// Pure: `Some(sentence)` when the typed ID is in `enrollers()` (which lists non-superusers only).
pub fn superuser_refusal(rows: &[EnrollerRow], staff_id: &str) -> Option<String> {
    (standing(rows, staff_id) != Standing::NotListed).then(|| ONLY_SUPERUSER.to_string())
}

fn fault(what: &str, e: anyhow::Error) -> String {
    eprintln!(
        "{what} could not be completed: {}",
        cairn_node::db_diagnosis::operator_chain(&e)
    );
    NOT_RECORDED.to_string()
}

/// Pure: a failed write is "nothing recorded" only on an ordinary ERROR answer; else unknown.
fn write_failure(e: &anyhow::Error) -> &'static str {
    if cairn_node::db_diagnosis::answered_with_error(e) {
        NOT_RECORDED
    } else {
        OUTCOME_UNKNOWN
    }
}

#[tauri::command]
pub async fn list_unconfirmed(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<UnconfirmedView>, String> {
    list_unconfirmed_impl(&state).await
}

pub async fn list_unconfirmed_impl(state: &AppState) -> Result<Vec<UnconfirmedView>, String> {
    let Some(db) = state.db.as_ref() else {
        return Err(FIXTURE.into());
    };
    let db = db
        .lock()
        .await
        .map_err(|l| fault("the worklist", l.into()))?;
    let rows = cairn_node::vault_admin::unconfirmed(&db)
        .await
        .map_err(|e| fault("the worklist", db.explain(e)))?;
    let now = chrono::Local::now();
    Ok(rows
        .iter()
        .map(|u| UnconfirmedView {
            kid: u.kid.clone(),
            text: unconfirmed_text(u, &now),
            doubtful: u.doubtful,
        })
        .collect())
}

#[tauri::command]
pub async fn list_enrollers(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<EnrollerView>, String> {
    list_enrollers_impl(&state).await
}

pub async fn list_enrollers_impl(state: &AppState) -> Result<Vec<EnrollerView>, String> {
    let Some(db) = state.db.as_ref() else {
        return Err(FIXTURE.into());
    };
    let db = db
        .lock()
        .await
        .map_err(|l| fault("the grant list", l.into()))?;
    let rows = cairn_node::vault_admin::enrollers(&db)
        .await
        .map_err(|e| fault("the grant list", db.explain(e)))?;
    Ok(rows
        .iter()
        .map(|r| EnrollerView {
            staff_id: r.staff_id.clone(),
            text: enroller_text(r),
            enroller: r.enroller,
        })
        .collect())
}

#[tauri::command]
pub async fn set_enroller(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    staff_id: String,
    grant: bool,
) -> Result<String, String> {
    set_enroller_impl(&state, credential, &staff_id, grant).await
}

/// Grant or withdraw the enroller role. Over plain `&AppState` so tests need no Tauri runtime.
pub async fn set_enroller_impl(
    state: &AppState,
    credential: Credential,
    staff_id: &str,
    grant: bool,
) -> Result<String, String> {
    let (db, key) = superuser_key(state, &credential).await?;
    cairn_node::vault_admin::set_enroller(&db, staff_id, grant, &credential.staff_id, &key)
        .await
        .map_err(|e| {
            let e = db.explain(e);
            eprintln!(
                "an enroller grant could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            write_failure(&e).to_string()
        })?;
    Ok(if grant {
        format!("{staff_id} can now enrol staff, once a superuser has confirmed their key.")
    } else {
        format!("{staff_id} can no longer enrol staff.")
    })
}

#[tauri::command]
pub async fn judge_key(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    kid: String,
    confirm: bool,
) -> Result<String, String> {
    judge_key_impl(&state, credential, &kid, confirm).await
}

/// Confirm (`true`) or repudiate (`false`) a key. NO confirmation dialog: see the module doc.
pub async fn judge_key_impl(
    state: &AppState,
    credential: Credential,
    kid: &str,
    confirm: bool,
) -> Result<String, String> {
    let (db, key) = superuser_key(state, &credential).await?;
    cairn_node::vault_admin::judge_key(&db, kid, confirm, &credential.staff_id, &key)
        .await
        .map_err(|e| {
            let e = db.explain(e);
            eprintln!(
                "a key judgement could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            write_failure(&e).to_string()
        })?;
    Ok(if confirm {
        "The key is confirmed.".into()
    } else {
        "The key is repudiated. Give the member a new key from the re-key form.".into()
    })
}

/// Steps 1 to 3 of the module doc: the held key wiped, the courtesy refusal, then the superuser's
/// key released for ONE use by the caller, who drops it. The connection guard comes back with it
/// so the write shares the same hold.
async fn superuser_key<'a>(
    state: &'a AppState,
    credential: &Credential,
) -> Result<(cairn_gui_live::ConnectionGuard<'a>, cairn_event::SigningKey), String> {
    if !credential.has_passphrase() {
        return Err(NEED_SUPERUSER.into());
    }
    state.forget_held_key().await;
    let Some(db) = state.db.as_ref() else {
        return Err(FIXTURE.into());
    };
    let db = db
        .lock()
        .await
        .map_err(|l| fault("a superuser act", l.into()))?;
    let rows = cairn_node::vault_admin::enrollers(&db)
        .await
        .map_err(|e| fault("a superuser act", db.explain(e)))?;
    if let Some(said) = superuser_refusal(&rows, &credential.staff_id) {
        return Err(said);
    }
    // An empty role: this is no enrolment, so the superuser/admin role rule does not apply.
    let key = release_enrol_key(&db, credential, "", NOT_RECORDED).await?;
    Ok((db, key))
}

#[cfg(test)]
#[path = "superuser_panel_tests.rs"]
mod tests;
