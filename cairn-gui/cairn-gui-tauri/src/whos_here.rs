//! Who's here — the terminal's claim of who is at the keyboard, and the idle re-lock (sign-in
//! slice 3b-1, design §3.1 and §3.3).
//!
//! "Who's here" is a CLAIM, never authentication: naming someone signs nothing, releases no key
//! and is never needed to READ a chart. It only says whose words a new draft part will be; "Don't
//! know" ([`Claim::Unknown`]) is an allowed answer. What a pick DOES do is forget any held key
//! (`AppState::forget_held_key`): on a key-hold terminal, Dr B switched in must never sign with Dr
//! A's held key. The idle re-lock ([`relock`]) forgets the key and the claim.
//!
//! Everything here lives in window memory only — the claim, the recent people and the bookmarks
//! (the chart each person last opened here). A bookmark holds a chart header, i.e. a patient's
//! name, which must not sit on disk in the clear, so a window restart forgets them all.
//!
//! The split is `notes.rs`'s: each `#[tauri::command]` forwards to a plain `*_impl(&AppState, …)`
//! tested against `AppState::mock`, and every word the clinician reads comes from a pure function.
use crate::commands::{lock_state_impl, LockState};
use crate::funnel::view::{header_from_candidate, ChartHeaderView, ErrorView};
use crate::link::judgement_view::refused;
use crate::state::AppState;
use crate::write_failure::chart_read_failed;
use cairn_node::note::whos_here::StaffEntry;
use cairn_patient_search::Candidate;
use std::collections::HashMap;
use std::time::Duration;

/// The idle re-lock's default (`--idle-lock-minutes`), design §3.3. The same figure as
/// `state::IDLE_TIMEOUT` but a separate setting: that constant caps the key hold.
pub const DEFAULT_IDLE_LOCK: Duration = Duration::from_secs(15 * 60);

/// How many recent people the picker lists first.
const RECENT_CAP: usize = 8;

/// Who the terminal says is at the keyboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Nobody has been asked since launch or the last re-lock: starting a note asks first.
    NotAsked,
    /// "Don't know / just reading": a draft part written now has no named writer.
    Unknown,
    /// A named, enrolled member (`staff_id` as the directory normalised it).
    Named {
        staff_id: String,
        display_name: String,
    },
}

/// The terminal's who's-here memory: the claim, the recent people (newest first, at most
/// [`RECENT_CAP`]), each person's bookmark, and this terminal's idle re-lock interval (a launch
/// setting kept here so `AppState` gains one field, not two).
pub struct Here {
    /// Private: a claim changes only through [`Here::pick`] / [`Here::forget_claim`], so no crate
    /// code can name someone without `whos_here_pick_impl` first forgetting the held key.
    claim: Claim,
    recent: Vec<(String, String)>,
    bookmarks: HashMap<String, Candidate>,
    pub idle_lock: Duration,
}

impl Here {
    /// Nobody asked yet, nothing remembered.
    pub fn new(idle_lock: Duration) -> Self {
        Self {
            claim: Claim::NotAsked,
            recent: Vec::new(),
            bookmarks: HashMap::new(),
            idle_lock,
        }
    }

    /// Who the terminal says is here.
    pub fn claim(&self) -> &Claim {
        &self.claim
    }

    /// Take `claim` — called only by `whos_here_pick_impl`, which forgets any held key first, so
    /// one person's key can never sign under the next person's claim. A named person moves to the
    /// front of the recent list (once, capped).
    pub fn pick(&mut self, claim: Claim) {
        if let Claim::Named {
            staff_id,
            display_name,
        } = &claim
        {
            self.recent.retain(|(id, _)| id != staff_id);
            self.recent
                .insert(0, (staff_id.clone(), display_name.clone()));
            self.recent.truncate(RECENT_CAP);
        }
        self.claim = claim;
    }

    /// Back to "not asked" (the re-lock). The recent people and bookmarks stay.
    pub fn forget_claim(&mut self) {
        self.claim = Claim::NotAsked;
    }

    /// Bookmark `chart` as the named person's last chart. Nobody named, nothing remembered.
    pub fn remember_chart(&mut self, chart: &Candidate) {
        if let Claim::Named { staff_id, .. } = &self.claim {
            self.bookmarks.insert(staff_id.clone(), chart.clone());
        }
    }

    /// The chart to offer back to the person now named — offered, never auto-opened.
    pub fn offer(&self) -> Option<&Candidate> {
        match &self.claim {
            Claim::Named { staff_id, .. } => self.bookmarks.get(staff_id),
            _ => None,
        }
    }

    /// The recent people, newest first, as (staff id, display name).
    pub fn recent(&self) -> &[(String, String)] {
        &self.recent
    }
}

/// A writer's name as others read it: a claim, so marked as one in TEXT and in speech — never a
/// glyph or a colour alone.
#[derive(Debug, PartialEq, Eq)]
pub struct ClaimedName {
    pub shown: String,
    pub spoken: String,
}

/// `Some("Nurse A")` → "?Nurse A", heard "unverified Nurse A"; `None` → "writer not known". Pure.
pub fn claimed_name(name: Option<&str>) -> ClaimedName {
    match name {
        Some(n) => ClaimedName {
            shown: format!("?{n}"),
            spoken: format!("unverified {n}"),
        },
        None => ClaimedName {
            shown: "writer not known".into(),
            spoken: "writer not known".into(),
        },
    }
}

/// The claim as the webview reads it. `staff_id` is what the editor captures as its writer.
#[derive(Debug, serde::Serialize)]
pub struct ClaimView {
    /// "not-asked", "unknown" or "named".
    pub kind: &'static str,
    pub staff_id: Option<String>,
    pub display_name: Option<String>,
    /// The bar's words. No `?`: on this screen the claim is the user's own statement.
    pub words: String,
}

/// The claim's view. Pure.
pub fn claim_view(claim: &Claim) -> ClaimView {
    match claim {
        Claim::NotAsked => ClaimView {
            kind: "not-asked",
            staff_id: None,
            display_name: None,
            words: "Nobody has said who is at this terminal".into(),
        },
        Claim::Unknown => ClaimView {
            kind: "unknown",
            staff_id: None,
            display_name: None,
            words: "Writer not known".into(),
        },
        Claim::Named {
            staff_id,
            display_name,
        } => ClaimView {
            kind: "named",
            staff_id: Some(staff_id.clone()),
            display_name: Some(display_name.clone()),
            words: format!("Writing as {display_name}"),
        },
    }
}

/// One person the picker offers.
#[derive(Debug, serde::Serialize)]
pub struct StaffEntryView {
    pub staff_id: String,
    pub display_name: String,
    pub role: String,
    /// One of this terminal's recent people (listed first).
    pub recent: bool,
}

/// The directory with this terminal's recent people first, newest first, then everyone else in
/// the directory's order. A recent person no longer in the directory is not offered. Pure.
pub fn directory_view(
    entries: Vec<StaffEntry>,
    recent: &[(String, String)],
) -> Vec<StaffEntryView> {
    let rank = |e: &StaffEntry| recent.iter().position(|(id, _)| *id == e.staff_id);
    let mut ranked: Vec<(Option<usize>, StaffEntry)> =
        entries.into_iter().map(|e| (rank(&e), e)).collect();
    // Recent people by their place in `recent`, everyone else after them; the sort is stable.
    ranked.sort_by_key(|(r, _)| r.map_or(usize::MAX, |i| i));
    ranked
        .into_iter()
        .map(|(r, e)| StaffEntryView {
            staff_id: e.staff_id,
            display_name: e.display_name,
            role: e.role,
            recent: r.is_some(),
        })
        .collect()
}

/// What a pick answers: the claim now held, and the person's bookmarked chart, if any — already
/// openable, because the offer is a list on screen.
#[derive(Debug, serde::Serialize)]
pub struct PickView {
    pub claim: ClaimView,
    pub offer: Option<ChartHeaderView>,
}

/// clap's parser for `--idle-lock-minutes`: whole minutes, 1 to 240. Refused at launch with the
/// range named, never silently clamped.
pub fn parse_idle_lock_minutes(text: &str) -> Result<Duration, String> {
    let minutes: u64 = text
        .parse()
        .map_err(|_| format!("not a whole number of minutes: {text}"))?;
    if !(1..=240).contains(&minutes) {
        return Err("the idle re-lock is 1 to 240 minutes".into());
    }
    Ok(Duration::from_secs(minutes * 60))
}

/// Fixture mode's staff, so the operator accessibility pass can walk the picker without a vault.
const FIXTURE_STAFF: [(&str, &str, &str); 2] = [
    ("dr-b", "Dr B", "clinician"),
    ("nurse-a", "Nurse A", "nurse"),
];

/// The node's clinical members (fixture staff in fixture mode). A failed read is a worded error.
async fn read_directory(state: &AppState) -> Result<Vec<StaffEntry>, ErrorView> {
    let Some(db) = state.db.as_ref() else {
        return Ok(FIXTURE_STAFF
            .iter()
            .map(|(id, name, role)| StaffEntry {
                staff_id: (*id).into(),
                display_name: (*name).into(),
                role: (*role).into(),
            })
            .collect());
    };
    let db = db.lock().await?;
    cairn_node::note::whos_here::directory(&*db)
        .await
        .map_err(|e| chart_read_failed(db.explain(e)))
}

/// The picker's list: the directory, this terminal's recent people first.
pub async fn whos_here_directory_impl(state: &AppState) -> Result<Vec<StaffEntryView>, ErrorView> {
    let entries = read_directory(state).await?;
    let here = state.whos_here.lock().await;
    Ok(directory_view(entries, here.recent()))
}

/// Say who is at the terminal: `Some(staff_id)` from the directory, `None` for "Don't know".
///
/// The held key and the old claim are forgotten FIRST, whatever follows — someone new may be at
/// the keyboard, and a refused pick must not leave the last person's key or name in charge. A
/// staff id the directory does not hold is refused. The person's bookmark, if any, goes into
/// `AppState::shown`, so `open_chart` will open it (it is on screen as the offer).
pub async fn whos_here_pick_impl(
    state: &AppState,
    staff_id: Option<&str>,
) -> Result<PickView, ErrorView> {
    state.forget_held_key().await;
    state.whos_here.lock().await.forget_claim();
    let claim = match staff_id {
        None => Claim::Unknown,
        Some(id) => {
            let entry = read_directory(state)
                .await?
                .into_iter()
                .find(|e| e.staff_id == id)
                .ok_or_else(|| {
                    refused(format!(
                        "{id} is not an enrolled member of this node — pick a name from the \
                         list, or Don't know"
                    ))
                })?;
            Claim::Named {
                staff_id: entry.staff_id,
                display_name: entry.display_name,
            }
        }
    };
    let (claim, offer) = {
        let mut here = state.whos_here.lock().await;
        here.pick(claim);
        (claim_view(here.claim()), here.offer().cloned())
    };
    if let Some(c) = &offer {
        state.shown.lock().await.insert(c.patient_id, c.clone());
    }
    Ok(PickView {
        claim,
        offer: offer.as_ref().map(header_from_candidate),
    })
}

/// The idle re-lock: forget the held key and the claim, and report the lock as it then stands.
pub async fn relock_impl(state: &AppState) -> LockState {
    state.forget_held_key().await;
    state.whos_here.lock().await.forget_claim();
    lock_state_impl(state).await
}

/// The picker's list.
#[tauri::command]
pub async fn whos_here_directory(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<StaffEntryView>, ErrorView> {
    whos_here_directory_impl(&state).await
}

/// Say who is at the terminal (`staff_id` null = "Don't know").
#[tauri::command]
pub async fn whos_here_pick(
    state: tauri::State<'_, AppState>,
    staff_id: Option<String>,
) -> Result<PickView, ErrorView> {
    whos_here_pick_impl(&state, staff_id.as_deref()).await
}

/// The idle re-lock (`idle.js`).
#[tauri::command]
pub async fn relock(state: tauri::State<'_, AppState>) -> Result<LockState, String> {
    Ok(relock_impl(&state).await)
}

#[cfg(test)]
#[path = "whos_here_tests.rs"]
mod tests;
