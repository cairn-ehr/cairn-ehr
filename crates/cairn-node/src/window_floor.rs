//! May an older reference window keep working against a newer database? (#909 slice C1)
//!
//! The reference window is installed separately from the node. Without a rule, any schema
//! upgrade would lock out every window not yet updated. The schema stamp therefore carries a
//! *window floor* (db/085): the oldest window generation this schema still serves. At or above
//! the floor an older window keeps working and is merely told an update is waiting; below it,
//! it is told to update. This module is the pure decision; reading the stamp is `db.rs`'s job.
use crate::conn_posture::{different_build_message, not_prepared_message};
use crate::db::{SchemaStamp, LOADER_BUILD};

/// What a window of generation `w` should do about the database it found.
#[derive(Debug, Clone, PartialEq)]
pub enum WindowVerdict {
    /// Same generation: nothing to say.
    Current,
    /// Older than the database but at or above the floor: works, an update is waiting.
    UpdateWaiting,
    /// Older than the floor: must update. `reason` is what the floor's stamp says it would miss.
    BelowFloor { reason: Option<String> },
    /// Unusable: not prepared for a window, or stamped by another loader. Human-readable.
    Refused(String),
}

/// Only a stamp written by a cairn-node loader (any version) is trusted; cairn-sync stamps its
/// own name after replaying a subset, so it never counts.
fn stamped_by_node(s: &SchemaStamp) -> bool {
    let prefix = LOADER_BUILD.split(' ').next().unwrap_or("cairn-node");
    s.loader_build.starts_with(&format!("{prefix} "))
}

/// The verdict for a window of generation `window_generation` (design §4.2). **Pure.**
pub fn window_verdict(stamp: Option<&SchemaStamp>, window_generation: i32) -> WindowVerdict {
    let (who, verb, w) = ("window", "provision-window-role", window_generation);
    let Some(s) = stamp else {
        return WindowVerdict::Refused(not_prepared_message(None, w, who, verb));
    };
    if s.version < w {
        return WindowVerdict::Refused(not_prepared_message(Some(s.version), w, who, verb));
    }
    if !stamped_by_node(s) {
        return WindowVerdict::Refused(different_build_message(w, who, verb));
    }
    if s.version == w {
        return WindowVerdict::Current;
    }
    // v > w: a NULL floor means "no older window is served".
    match &s.window_floor {
        Some(f) if f.floor <= w => WindowVerdict::UpdateWaiting,
        f => WindowVerdict::BelowFloor {
            reason: f.as_ref().and_then(|f| f.reason.clone()),
        },
    }
}

/// Whether an installed window can stay in place when a newer one is offered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InstallFit {
    /// Nothing installed, or the identical build: nothing to replace.
    Same,
    /// The installed window still works against this database.
    Fits,
    /// The installed window would be refused or told to update.
    DoesNotFit,
}

/// Would the installed window `(generation, hash)` still be served by `stamp`? **Pure.**
/// "Not installed" reads as `Same`, the safe answer. It is [`window_verdict`] with the
/// installed generation as `w`.
pub fn install_fit(
    installed: Option<(i32, [u8; 32])>,
    running: (i32, [u8; 32]),
    stamp: Option<&SchemaStamp>,
) -> InstallFit {
    match installed {
        None => InstallFit::Same,
        Some(i) if i == running => InstallFit::Same,
        Some((gen, _)) => match window_verdict(stamp, gen) {
            WindowVerdict::Current | WindowVerdict::UpdateWaiting => InstallFit::Fits,
            _ => InstallFit::DoesNotFit,
        },
    }
}

#[cfg(test)]
#[path = "window_floor_tests.rs"]
mod tests;
