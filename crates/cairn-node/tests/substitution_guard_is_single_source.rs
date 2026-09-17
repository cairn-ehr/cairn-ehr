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

/// Every door this change guards still CALLS the helper.
///
/// ⚠️ **The test above cannot see this, and the difference is what #619 is made of.** It proves
/// nobody *duplicates* the refusal sentence; a door that simply never calls the helper contains no
/// sentence to find and is invisible to it. That is not hypothetical — `db/007`'s
/// `submit_node_event` and `apply_remote_node_event` write `node_event` through five
/// `ON CONFLICT DO NOTHING` sites with no guard at all, and the file passes the test above
/// cleanly. They are **deliberately not in this list** (#619 is a decision about refuse-vs-skip on
/// the pull path, not a patch); the list is the set this change is responsible for, so adding a
/// door here is how a future slice records that it took that responsibility on.
///
/// Behaviour tests already kill the deletion of each call (mutations M2/M3/M4). This exists so the
/// *inventory* is written down in one greppable place rather than inferred from three suites.
#[test]
fn every_door_this_change_guards_still_calls_the_helper() {
    const GUARDED_DOORS: [&str; 3] = [
        "005_submit.sql",
        "009_node_supersede_and_restore.sql",
        "020_apply_remote_event.sql",
    ];
    for door in GUARDED_DOORS {
        let text = fs::read_to_string(db_dir().join(door))
            .unwrap_or_else(|e| panic!("db/{door} must be readable: {e}"));
        assert!(
            text.contains("cairn_refuse_substitution"),
            "db/{door} no longer calls cairn_refuse_substitution. If a door genuinely stopped \
             needing the guard, say so here with the reason — do not just delete the call, and do \
             not re-inline the comparison (that is #608's shape returning)."
        );
    }
}
