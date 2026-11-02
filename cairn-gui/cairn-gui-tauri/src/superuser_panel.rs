//! The superuser's panel (sign-in slice 2, Task 8): grant or withdraw the enroller role, and judge
//! the keys that await a superuser (Confirm or Repudiate). Each grant-list row also offers
//! Repudiate on that member's CURRENT key: a confirmed key (an impostor found later) and an
//! enroller's own key (which must be confirmed to approve anyone) are never on the worklist, and
//! repudiating an enroller's key is what turns their unconfirmed approvals doubtful (§5.5).
//!
//! # The flow, for a reader new to it
//!
//! Both lists are plain reads (`vault_admin::enrollers`, `vault_admin::unconfirmed`), made with no
//! credential; the page re-reads them after every write, and a failed re-read says so in its own
//! sentence ([`READ_FAILED`]), never "nothing was recorded". A WRITE (`set_enroller`, `judge_key`)
//! is a superuser act:
//!
//! 1. A credential with no passphrase is refused before the vault sees it (it would count as a
//!    wrong guess); then any held key is wiped (someone new may be at the keyboard).
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
use serde::{Deserialize, Serialize};

/// The sentence for "this credential belongs to someone who is not a superuser".
pub const ONLY_SUPERUSER: &str =
    "Only a superuser can do this. Enter a superuser's staff ID and passphrase above.";
const NOT_RECORDED: &str = "That could not be done. Nothing was recorded. Try again, and if it \
     fails again, ask your IT support.";
const OUTCOME_UNKNOWN: &str = "The node did not confirm the change, so it may have been made. \
     Look at the lists again before trying twice.";
/// The floor refused on purpose (a `RAISE`): the row was out of date (a re-key since the lists
/// were read, a grant already given) or the act no longer applies. The page re-reads the lists.
const REFUSED: &str = "The node refused this, so nothing was recorded. The lists may have been \
     out of date: they are read again below. Check the row before trying again.";
/// A list read failed. It runs AFTER every write, so it must never say "nothing was recorded":
/// the write before it may have landed.
const READ_FAILED: &str = "The lists could not be read again. Close and reopen this panel to see \
     where things stand.";
const FIXTURE: &str = "This window is showing sample data. Nothing can be changed here.";
const NEED_SUPERUSER: &str = "Enter a superuser's staff ID and passphrase to do this.";

/// What a superuser decides about one key. Named on the wire ("confirm" / "repudiate"), never a
/// bare boolean: repudiation is final and asks no question (principle 3), so the request itself
/// must say which act it is, and serde refuses anything else before a key is released.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Judgement {
    Confirm,
    Repudiate,
}

/// One key on the worklist, as the page shows it.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct UnconfirmedView {
    pub kid: String,
    pub text: String,
    pub doubtful: bool,
}

/// One non-superuser member in the grant list, as the page shows it. `kid` is the member's
/// CURRENT key, offered to Repudiate (a confirmed key is never on the worklist, so this is the
/// only way to reach it); `None` when there is nothing to repudiate.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct EnrollerView {
    pub staff_id: String,
    pub text: String,
    pub enroller: bool,
    pub kid: Option<String>,
}

/// Pure: the sentence for one worklist row. `now` supplies the local time zone. The approver is
/// named by their vault staff ID, or by their registry handle when their member row is gone (as
/// after a restore, `cairn_vault_key_holder`); only when no registry row carries the approving
/// key is it "someone no longer in the vault", never left blank (principle 4). Under a site expiry the
/// row also says when the key stops (or stopped) signing.
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
    // §5.5: the superuser is the one who can stop it, so the date is said here too.
    if let Some(at) = &u.stops_at {
        let past = chrono::DateTime::parse_from_rfc3339(at).is_ok_and(|t| t <= *now);
        text.push_str(&if past {
            format!(
                " Stopped signing on {}. Confirm it to let it sign again.",
                local_at(at, now)
            )
        } else {
            format!(" Stops signing on {}.", local_at(at, now))
        });
    }
    text
}

/// Pure: the sentence for one grant-list row. It says whether the member HAS the role, never
/// that they "can enrol": the floor also wants their own key confirmed and current
/// (`cairn_vault_enroller_problem`, db/061), which a grant alone does not make true.
pub fn enroller_text(r: &EnrollerRow) -> String {
    let has = if r.enroller { "has" } else { "does not have" };
    format!(
        "{} ({}): {has} the enroller role.",
        r.display_name, r.staff_id
    )
}

/// Pure: one grant-list row as the page shows it. A key already repudiated (final) is said so
/// and offered no more; the remedy is a re-key, which gives the member a fresh key to judge.
pub fn enroller_view(r: &EnrollerRow) -> EnrollerView {
    let mut text = enroller_text(r);
    if r.key_repudiated {
        text.push_str(" Their key is repudiated: give them a new key from the re-key form.");
    }
    EnrollerView {
        staff_id: r.staff_id.clone(),
        text,
        enroller: r.enroller,
        kid: r.kid.clone().filter(|_| !r.key_repudiated),
    }
}

/// Pure: `Some(sentence)` when the typed ID is in `enrollers()` (which lists non-superusers only).
pub fn superuser_refusal(rows: &[EnrollerRow], staff_id: &str) -> Option<String> {
    (standing(rows, staff_id) != Standing::NotListed).then(|| ONLY_SUPERUSER.to_string())
}

/// A failure BEFORE any write (the connection, the courtesy read): nothing was recorded.
fn fault(what: &str, e: anyhow::Error) -> String {
    eprintln!(
        "{what} could not be completed: {}",
        cairn_node::db_diagnosis::operator_chain(&e)
    );
    NOT_RECORDED.to_string()
}

/// A failed list read. Its own sentence, because it follows every write (see [`READ_FAILED`]).
fn read_fault(what: &str, e: anyhow::Error) -> String {
    eprintln!(
        "{what} could not be read: {}",
        cairn_node::db_diagnosis::operator_chain(&e)
    );
    READ_FAILED.to_string()
}

/// Pure: the sentence for a failed write. `deliberate`: the floor refused on purpose (SQLSTATE
/// P0001); `answered`: an ordinary ERROR answer came back, so the statement rolled back. Without
/// an answer the write may have landed.
pub fn write_failure_sentence(deliberate: bool, answered: bool) -> &'static str {
    match (deliberate, answered) {
        (true, _) => REFUSED,
        (false, true) => NOT_RECORDED,
        (false, false) => OUTCOME_UNKNOWN,
    }
}

fn write_failure(e: &anyhow::Error) -> &'static str {
    write_failure_sentence(
        cairn_gui_live::error::refusal_is_deliberate(cairn_gui_live::error::sqlstate_of(e)),
        cairn_node::db_diagnosis::answered_with_error(e),
    )
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
        .map_err(|l| read_fault("the worklist", l.into()))?;
    let rows = cairn_node::vault_admin::unconfirmed(&db)
        .await
        .map_err(|e| read_fault("the worklist", db.explain(e)))?;
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
        .map_err(|l| read_fault("the grant list", l.into()))?;
    let rows = cairn_node::vault_admin::enrollers(&db)
        .await
        .map_err(|e| read_fault("the grant list", db.explain(e)))?;
    Ok(rows.iter().map(enroller_view).collect())
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
        format!("{staff_id} now has the enroller role. They can enrol staff while their own key is confirmed.")
    } else {
        format!("{staff_id} can no longer enrol staff.")
    })
}

#[tauri::command]
pub async fn judge_key(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    kid: String,
    judgement: Judgement,
) -> Result<String, String> {
    judge_key_impl(&state, credential, &kid, judgement).await
}

/// Confirm or repudiate a key. NO confirmation dialog: see the module doc. The floor judges only
/// the person's CURRENT key, so a row read before a re-key is refused ([`REFUSED`]), never
/// applied to the old key (db/063 `cairn_vault_judge_key`).
pub async fn judge_key_impl(
    state: &AppState,
    credential: Credential,
    kid: &str,
    judgement: Judgement,
) -> Result<String, String> {
    let confirm = judgement == Judgement::Confirm;
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
    // No role: this enrols nobody, so the superuser/admin role rule does not apply.
    let key = release_enrol_key(&db, credential, None, NOT_RECORDED).await?;
    Ok((db, key))
}

#[cfg(test)]
#[path = "superuser_panel_tests.rs"]
mod tests;
