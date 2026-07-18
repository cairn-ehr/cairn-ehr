//! The repo's schema generation — the single number both loaders report (issue #188).
//!
//! # Why this lives in `cairn-event` and not beside a loader
//!
//! Two binaries replay `db/*.sql` into a database, and they do **not** replay the same
//! set:
//!
//!   * `cairn-node` (`db::connect_and_load_schema`) embeds the FULL migration list;
//!   * `cairn-sync` (`init` → `load_schema`) embeds a deliberate SUBSET — 14 of the 38
//!     files today, skipping `007`, `009`, `010`–`019`, `022`–`025`, `028`, `030`–`035`.
//!     That subset is a load-bearing guarantee in its own right (issue #198: it must
//!     satisfy both write doors standing alone), so it must stay free to lag.
//!
//! The #188 downgrade guard compares "the generation this binary carries" against the
//! generation recorded in the database. If each loader derived that number from *its own
//! list*, the two binaries would disagree the moment a migration landed in one list only
//! — which is the normal case, since most new migrations are clinical or node-only and
//! never enter cairn-sync's subset. Concretely: a node-only `db/039_*` makes `cairn-node`
//! report 39 and stamp every database it touches at 39, while `cairn-sync` still reports
//! 38 — so `cairn-sync init` refuses every database in the fleet and the federation
//! daemon stops. A downgrade guard that bricks the sync daemon is worse than no guard.
//!
//! So the generation is a property of **the repo build**, not of a loader's subset: one
//! constant, used by both doors. Each loader still replays only its own list; they just
//! agree on which generation of the schema that build came from.
//!
//! # Why a constant, and what keeps it honest
//!
//! It is hand-maintained by exactly one line — and `schema_generation.rs`'s companion
//! guard (`crates/cairn-event/tests/schema_generation.rs`) reads `db/` at test time and
//! fails if this constant is not the newest migration's numeric prefix. Forgetting to
//! bump it when adding `db/039_*` is therefore a CI failure, not a silent drift. That is
//! the same register-and-guard discipline the twin-check registry uses (ADR-0048): a
//! hand-written value is safe when a test derives the truth independently.
//!
//! This is a node-LOCAL operational number. It never appears in a signed body and never
//! travels the wire core (principle 12) — it lives here only because `cairn-event` is the
//! crate both loaders already depend on.

/// The numeric prefix of the newest migration in `db/` (`db/038_node_schema.sql` → 38).
///
/// Bump this in the same commit that adds a `db/*.sql` file; the guard test enforces it.
pub const SCHEMA_GENERATION: i32 = 38;

/// Advisory-lock key (ASCII `"CARNLOAD"`) serializing a loader's whole
/// check→replay→stamp sequence against every other loader on the same database.
///
/// Without it the #188 downgrade guard is check-then-act: an old and a new binary
/// connecting together can interleave so the old one reads a stale generation,
/// passes the check, and still `CREATE OR REPLACE`s the newer safety floor away
/// while (or after) the new one loads. Both loaders take this SESSION-level lock
/// (blocking — a concurrent loader waits its turn, then re-reads the now-current
/// record) before the guard check and release it after the stamp; an error path
/// releases it when the session closes. Distinct from the test-only `"CARN"`
/// serialization key, which belongs to the test harness, not the product path.
pub const SCHEMA_LOAD_LOCK: i64 = 0x4341_524E_4C4F_4144;

/// The numeric prefix of a migration name — `"038_node_schema"` → `Some(38)`.
///
/// Pure and total: anything that does not begin with a run of digits followed by `_`
/// yields `None` rather than panicking, so callers scanning a directory can skip
/// unrelated files instead of aborting the process.
pub fn migration_prefix(name: &str) -> Option<i32> {
    let (prefix, _rest) = name.split_once('_')?;
    prefix.parse().ok()
}

/// The newest (highest) migration prefix in a list of migration names.
///
/// `None` for an empty list or one containing no recognizable prefixes. Used by the
/// loader-completeness guards, never by the downgrade guard itself — see the module
/// docs for why a loader must not derive its generation from its own list.
pub fn newest_migration_prefix<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<i32> {
    names.into_iter().filter_map(migration_prefix).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_prefix_reads_the_leading_number() {
        assert_eq!(migration_prefix("038_node_schema"), Some(38));
        assert_eq!(migration_prefix("001_envelope"), Some(1));
    }

    #[test]
    fn migration_prefix_is_total_on_unrecognized_names() {
        // A directory scan must be able to skip these, not panic.
        assert_eq!(migration_prefix("README"), None);
        assert_eq!(migration_prefix("notes_about_007"), None);
        assert_eq!(migration_prefix(""), None);
    }

    #[test]
    fn newest_migration_prefix_takes_the_maximum_not_the_last() {
        // Deliberately unsorted: the newest generation is the highest prefix, which is
        // NOT the same as "whatever entry happens to sit last in the list".
        let names = ["020_apply_remote_event", "037_born_sealed", "001_envelope"];
        assert_eq!(newest_migration_prefix(names), Some(37));
        assert_eq!(newest_migration_prefix([]), None);
    }
}
