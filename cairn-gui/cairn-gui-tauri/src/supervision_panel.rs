//! The window's supervision panels (sign-in slice 3c-3a, Task 8): who supervises whom, and who
//! must have every signed note co-signed. Two panels over the same doors (db/072,
//! `cairn_node::supervision_admin`):
//!
//! * the **assigner's panel** lists every non-superuser member and offers all six acts (link or
//!   unlink a supervisor, require or lift co-signing, grant or withdraw the right to assign);
//! * **"My supervisees"** lists the members whose supervisors include the person named in who's
//!   here, and offers only "Lift co-signing" and "Step away" (unlink themself).
//!
//! # The flow, for a reader new to it
//!
//! This is the superuser panel's shape (`superuser_panel.rs`), copied on purpose:
//!
//! 1. Lists are plain reads with no credential, re-read after every write; a failed re-read says
//!    so in its own sentence (`READ_FAILED`), never "nothing was recorded".
//! 2. A write ([`supervision_act`]) takes the signer strip's credential. [`supervision_key`]
//!    refuses a blank passphrase before the vault sees it (it would count as a wrong guess),
//!    wipes any held key (someone new may be at the keyboard), and releases the presser's key for
//!    ONE act, dropped after. It never enters the held session.
//! 3. The DOOR decides who may do what (the permission matrix lives only in db/072). The window
//!    merely chooses which buttons to draw from the row's state, and says a refusal plainly
//!    ([`REFUSED`]); the door's own words are never parsed or shown, as in the superuser panel.
//!
//! The who's-here claim only chooses whose supervisees are LISTED. The key, not the claim,
//! decides whether an act is allowed (ADR-0092 decision 16).
//!
//! No dialog anywhere (principle 3): every act is signed, audited, and undone by its opposite act.
use crate::enrol::staff_id_key;
use crate::signer::{from_release_noting, Credential, KeyRefusal};
use crate::state::AppState;
use crate::superuser_panel::{fault, read_fault, write_failure_sentence, FIXTURE, NOT_RECORDED};
use crate::whos_here::Claim;
use cairn_node::supervision_admin::{act, staff, StaffSupervision, SupervisionOp};
use cairn_node::vault::{release_for_act, Purpose, Release};
use serde::Serialize;

/// The door refused on purpose: the presser may not have the right to this act, or the row was
/// out of date (lifted already, linked already). The page re-reads the list.
pub const REFUSED: &str = "The node refused this, so nothing was recorded. You may not have the \
     right to do it, or the list was out of date: it is read again below.";
pub const NEED_CREDENTIAL: &str = "Enter your own staff ID and passphrase to do this.";
pub const NEED_SUPERVISOR: &str = "Type the supervisor's staff ID first. Nothing was recorded.";
pub const NOBODY_NAMED: &str = "Nobody is named in who's here, so no supervisees can be listed. \
     Press \"Who's here?\" and pick yourself.";

/// One member as the panels show them. `text` is the sentence; the flags let the page decide
/// which buttons to draw (never whether the act is allowed: the door decides that).
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct StaffSupervisionView {
    pub staff_id: String,
    pub display_name: String,
    pub text: String,
    pub needs_cosigning: bool,
    /// A student needs co-signing by role: there is nothing to require or lift.
    pub student: bool,
    pub assigner: bool,
    /// Staff IDs of the member's current supervisors. In "My supervisees" narrowed to the one
    /// naming the person in who's here, as the row spells it (what "Step away" sends).
    pub supervisors: Vec<String>,
}

/// Pure: one member's row. A requirement with no supervisor says so ("no supervisor recorded"),
/// so the gap has an owner (design §3.4).
pub fn staff_view(r: &StaffSupervision) -> StaffSupervisionView {
    let student = r.role == "student";
    let mut parts = vec![match (student, r.needs_cosigning) {
        (true, _) => "needs co-signing (student)".to_string(),
        (false, true) => "needs co-signing".to_string(),
        (false, false) => "co-signing not required".to_string(),
    }];
    if r.supervisors.is_empty() {
        if r.needs_cosigning || student {
            parts.push("no supervisor recorded".into());
        }
    } else {
        parts.push(format!("supervised by {}", r.supervisors.join(", ")));
    }
    if r.assigner {
        parts.push("may assign supervision".into());
    }
    StaffSupervisionView {
        staff_id: r.staff_id.clone(),
        display_name: r.display_name.clone(),
        text: format!(
            "{} ({}): {}.",
            r.display_name,
            r.staff_id,
            parts.join(" · ")
        ),
        needs_cosigning: r.needs_cosigning || student,
        student,
        assigner: r.assigner,
        supervisors: r.supervisors.clone(),
    }
}

/// Pure: the rows `me` supervises, compared by the vault's own staff-ID key, each narrowed to
/// the row's own spelling of `me`.
pub fn supervisees(rows: &[StaffSupervision], me: &str) -> Vec<StaffSupervisionView> {
    let me = staff_id_key(me);
    rows.iter()
        .filter_map(|r| {
            let mine = r.supervisors.iter().find(|s| staff_id_key(s) == me)?;
            let mut v = staff_view(r);
            v.supervisors = vec![mine.clone()];
            Some(v)
        })
        .collect()
}

/// Pure: what a successful act recorded, in one sentence.
pub fn done_sentence(op: SupervisionOp, staff_id: &str, supervisor: Option<&str>) -> String {
    let sup = supervisor.unwrap_or_default();
    match op {
        SupervisionOp::LinkSupervisor => format!("{sup} now supervises {staff_id}."),
        SupervisionOp::UnlinkSupervisor => format!("{sup} no longer supervises {staff_id}."),
        SupervisionOp::RequireCosign => {
            format!("{staff_id} now needs every signed note co-signed.")
        }
        // Design §3.2: a part is judged as of its own commit, so earlier parts still need one.
        // Worded conditionally: there may be no such notes, or all may be co-signed already.
        SupervisionOp::LiftCosign => format!(
            "{staff_id} no longer needs notes co-signed. Notes signed while it was required \
             still need a co-signature if they have none."
        ),
        SupervisionOp::GrantAssigner => {
            format!("{staff_id} may now assign supervision while their own key is confirmed.")
        }
        SupervisionOp::WithdrawAssigner => format!("{staff_id} can no longer assign supervision."),
    }
}

/// A failed write: the door's deliberate refusal gets [`REFUSED`]; otherwise the superuser
/// panel's two sentences (an answer came back, so nothing was recorded; or it may have landed).
fn write_failure(e: &anyhow::Error) -> String {
    let deliberate =
        cairn_gui_live::error::refusal_is_deliberate(cairn_gui_live::error::sqlstate_of(e));
    if deliberate {
        return REFUSED.into();
    }
    write_failure_sentence(false, cairn_node::db_diagnosis::answered_with_error(e)).into()
}

/// The members with their supervision state, read with no credential.
async fn read_staff(state: &AppState) -> Result<Vec<StaffSupervision>, String> {
    let Some(db) = state.db.as_ref() else {
        return Err(FIXTURE.into());
    };
    let db = db
        .lock()
        .await
        .map_err(|l| read_fault("the supervision list", l.into()))?;
    staff(&db)
        .await
        .map_err(|e| read_fault("the supervision list", db.explain(e)))
}

#[tauri::command]
pub async fn list_supervision(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<StaffSupervisionView>, String> {
    list_supervision_impl(&state).await
}

pub async fn list_supervision_impl(state: &AppState) -> Result<Vec<StaffSupervisionView>, String> {
    Ok(read_staff(state).await?.iter().map(staff_view).collect())
}

#[tauri::command]
pub async fn list_my_supervisees(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<StaffSupervisionView>, String> {
    list_my_supervisees_impl(&state).await
}

/// The rows the person named in who's here supervises; with nobody named, [`NOBODY_NAMED`]
/// before the database is asked.
pub async fn list_my_supervisees_impl(
    state: &AppState,
) -> Result<Vec<StaffSupervisionView>, String> {
    let me = match state.whos_here.lock().await.claim() {
        Claim::Named { staff_id, .. } => staff_id.clone(),
        Claim::NotAsked | Claim::Unknown => return Err(NOBODY_NAMED.into()),
    };
    Ok(supervisees(&read_staff(state).await?, &me))
}

#[tauri::command]
pub async fn supervision_act(
    state: tauri::State<'_, AppState>,
    credential: Credential,
    op: String,
    staff_id: String,
    supervisor: Option<String>,
) -> Result<String, String> {
    supervision_act_impl(&state, credential, op, staff_id, supervisor).await
}

/// One supervision act, approved and signed by the credential's holder. Over plain `&AppState`
/// so tests need no Tauri runtime. The op word and the supervisor are checked before any key is
/// released; the door then decides whether this person may do it.
pub async fn supervision_act_impl(
    state: &AppState,
    credential: Credential,
    op: String,
    staff_id: String,
    supervisor: Option<String>,
) -> Result<String, String> {
    // An unknown word is the page's bug, not the person's: nothing was done.
    let op = SupervisionOp::from_cli(&op).ok_or_else(|| {
        eprintln!("supervision_act: unknown op word {op:?} from the page");
        NOT_RECORDED.to_string()
    })?;
    let supervisor = supervisor.filter(|s| !s.trim().is_empty());
    let supervisor = match (op.needs_supervisor(), supervisor) {
        (true, None) => return Err(NEED_SUPERVISOR.into()),
        (true, s) => s,
        (false, _) => None,
    };
    let (db, key) = supervision_key(state, &credential).await?;
    let sup = supervisor.as_deref();
    act(&db, op, &staff_id, sup, &credential.staff_id, &key)
        .await
        .map_err(|e| {
            let e = db.explain(e);
            eprintln!(
                "a supervision act could not be completed: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            );
            write_failure(&e)
        })?;
    Ok(done_sentence(op, &staff_id, sup))
}

/// Steps of the module doc: no passphrase refused, the held key wiped, the lock, then the
/// presser's key released for ONE use by the caller, who drops it. Clinical first (a clinician's
/// key serves only that); a superuser's or an admin's serves only `enrol`, and the vault refuses
/// the purpose BEFORE it counts a try, so asking again with enrol costs nothing. Every other answer
/// becomes a sentence through `from_release_noting`. It never calls `signer_for_act`.
async fn supervision_key<'a>(
    state: &'a AppState,
    credential: &Credential,
) -> Result<(cairn_gui_live::ConnectionGuard<'a>, cairn_event::SigningKey), String> {
    if !credential.has_passphrase() {
        return Err(NEED_CREDENTIAL.into());
    }
    state.forget_held_key().await;
    let Some(db) = state.db.as_ref() else {
        return Err(FIXTURE.into());
    };
    let db = db
        .lock()
        .await
        .map_err(|l| fault("a supervision act", l.into()))?;
    let release =
        |purpose| release_for_act(&db, &credential.staff_id, &credential.passphrase, purpose);
    let mut answer = release(Purpose::Clinical)
        .await
        .map_err(|e| fault("a supervision act", db.explain(e)))?;
    if matches!(answer, Release::RefusedPurpose) {
        answer = release(Purpose::Enrol)
            .await
            .map_err(|e| fault("a supervision act", db.explain(e)))?;
    }
    let key = from_release_noting(&db, &credential.staff_id, answer)
        .await
        .map_err(|r: KeyRefusal| r.sentence())?
        .0;
    Ok((db, key))
}

#[cfg(test)]
#[path = "supervision_panel_tests.rs"]
mod tests;
