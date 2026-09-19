//! #615/#608 — the substitution refusal is raised in exactly ONE place.
//!
//! No database: this reads `db/*.sql` as text, the same technique as
//! `twin_dispatch_single_source.rs` (ADR-0048) and `name_winner_order_drift.rs` (#159).
//!
//! # Why a source guard and not a behaviour test
//!
//! Replacing the two inline copies with a shared helper is a refactor: the doors refuse exactly
//! what they refused before, with exactly the text they used before. A behaviour test would be
//! green before the change and green after, proving nothing about what changed. What IS newly
//! true is the single-source property — and that is the thing that can silently regress, because
//! the natural way to give a fourth door this guard is to paste the four lines again.
//!
//! # The defect this prevents is not hypothetical
//!
//! It is the state the repo was in until #615. `db/005_submit.sql` and
//! `db/020_apply_remote_event.sql` each carried their own copy, and when `db/009` needed the
//! guard, the obvious move was a third copy. Both existing copies compared with `<>`, which
//! fails open on a NULL (#608) — so the third copy would have inherited the bug, and fixing the
//! bug afterwards would have meant knowing to look in three places.
//!
//! # What a door MAY still do
//!
//! Read its own stored content-address however suits it. db/005 and db/020 read under a
//! `ROW_COUNT` check because they are on the 100k-event clinical path; db/009 reads
//! unconditionally because a `ROW_COUNT` guard there would be disarmed by a later edit. That
//! freedom is deliberate. What no door may do is DECIDE the question itself.

use std::fs;
use std::path::{Path, PathBuf};

/// The refusal sentence, which must appear in [`HOME`] and nowhere else.
///
/// Deliberately the tail of the message rather than the whole of it: the door name is
/// interpolated (`'%: event_id % …'`), so matching the leading part would match nothing.
const SENTENCE: &str = "already exists with different content (substitution refused)";

/// The only migration allowed to contain [`SENTENCE`].
const HOME: &str = "053_substitution_guard.sql";

/// Repo-root `db/` directory. `CARGO_MANIFEST_DIR` is `crates/cairn-node`; `db/` is two levels up.
fn db_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../db")
}

#[test]
fn only_db_053_raises_the_substitution_refusal() {
    let mut offenders = Vec::new();
    let mut found_home = false;

    for entry in fs::read_dir(db_dir()).expect("db/ is readable") {
        let path = entry.expect("a readable dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".sql") {
            continue;
        }
        if !fs::read_to_string(&path)
            .expect("a readable migration")
            .contains(SENTENCE)
        {
            continue;
        }
        if name == HOME {
            found_home = true;
        } else {
            offenders.push(name.to_string());
        }
    }

    // The anti-vacuity control. Without it, renaming db/053 or rewording the message would leave
    // this test green over a tree where the sentence appears nowhere at all — it would be
    // asserting that no file contains a string no file contains.
    assert!(
        found_home,
        "db/{HOME} must contain the refusal sentence {SENTENCE:?}. If the file was renamed or \
         the wording changed, update the constants here — do not delete this guard."
    );
    assert!(
        offenders.is_empty(),
        "the substitution refusal must be raised ONLY by cairn_refuse_substitution in \
         db/{HOME}. A door may read its own stored content-address however it likes, but it must \
         not decide the question itself — an inline copy is how #608's `<>` fail-open came to \
         exist in two places at once, and how a third door came to have no guard at all. \
         Offenders: {offenders:?}"
    );
}

// The INVENTORY of guarded doors used to be a hand-written list here
// (`every_door_this_change_guards_still_calls_the_helper`), and it was wrong: it omitted db/007's two
// `node_event` writers, one of them the live federation admission gate (#619). A list says what its
// author believed. The inventory is now DERIVED from the catalogue — every function that writes an
// event log must call the helper — in `substitution_guard_covers_every_writer.rs`. This file keeps
// the other half: nobody DUPLICATES the refusal.
