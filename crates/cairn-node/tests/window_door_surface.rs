//! #909 slice C1, Task 4 — the reference window's doors stay ADDITIVE within the floor.
//!
//! WHY. An older reference-UI window (generation G) keeps working against a database whose
//! window floor is <= G (`WINDOW_FLOOR_GENERATION`). That promise is only true if nothing the
//! window can reach disappears without the floor being raised. This guard makes that a CI
//! failure: `db/window_door_surface.txt` is a committed snapshot of everything the window's
//! database role (`cairn_window`) can reach, and a snapshot entry that vanishes from the live
//! catalogue fails the test unless
//!   (a) it is listed in `db/window_not_facing.txt` (`<entry>  # <reason>` — a door used by no
//!       window at or above the floor, so removing it cannot hurt a window still served), or
//!   (b) the floor ledger's LAST entry raised the floor (a deliberate, recorded break) AND the
//!       snapshot was recorded at an older generation than today's. The snapshot's first line,
//!       `# generation N`, says which. Only a REGENERATION applies (b): check mode fails a door
//!       (b) would excuse, saying to regenerate, and the regeneration drops it and writes the
//!       raising generation's header — after which an in-place edit (ADR-0080) at that generation
//!       that drops a door fails like any other.
//!
//! The snapshot has a `# generation N` header, then one sorted line per entry:
//!   `fn <regprocedure> -> <result type>` for each `public` function `cairn_window` may EXECUTE;
//!   `col <relation>.<column>` for each `public` column `cairn_window` may SELECT.
//! Extension-owned objects (a `pg_depend` row of `deptype = 'e'`, e.g. everything `cairn_pgx`
//! installs) are EXCLUDED: their surface moves with the extension's own version, not with
//! `db/*.sql`, and would make the snapshot differ between machines with different builds.
//!
//! Regenerate with `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test
//! window_door_surface`: it writes today's header, then (old snapshot + catalogue) minus the
//! entries the two exemptions allow to go — a vanished entry is never forgotten silently.
//!
//! The comparison is the pure `door_drift`, and check mode's decision the pure `check_vanished`,
//! both unit-tested below without a database.
use cairn_event::schema_generation::{SCHEMA_GENERATION, WINDOW_FLOOR_LEDGER};
use cairn_node::db;
use std::collections::BTreeSet;

const SNAPSHOT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../db/window_door_surface.txt"
);
const NOT_FACING: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../db/window_not_facing.txt"
);

/// What differs between the committed snapshot and the live catalogue.
#[derive(Debug, PartialEq)]
struct Drift {
    /// In the snapshot, gone from the catalogue, and not exempt — a break for old windows.
    vanished: Vec<String>,
    /// In the catalogue, absent from the snapshot — the snapshot is merely stale.
    unrecorded: Vec<String>,
}

/// Pure comparison. `not_facing` excuses individual vanished entries; a raise excuses nothing
/// here — only a regeneration applies it ([`check_vanished`]).
fn door_drift(
    snapshot: &BTreeSet<String>,
    catalogue: &BTreeSet<String>,
    not_facing: &BTreeSet<String>,
) -> Drift {
    let vanished = snapshot
        .difference(catalogue)
        .filter(|e| !not_facing.contains(*e))
        .cloned()
        .collect();
    Drift {
        vanished,
        unrecorded: catalogue.difference(snapshot).cloned().collect(),
    }
}

/// True when the ledger's last entry raised the floor over the one before it (or is the first).
fn last_entry_raised_floor() -> bool {
    let l = WINDOW_FLOOR_LEDGER;
    match l {
        [] => false,
        [_] => true,
        [.., prev, last] => last.floor > prev.floor,
    }
}

/// The snapshot's header line for `generation` (written on regeneration; `read_set` skips it).
fn snapshot_header(generation: i32) -> String {
    format!("# generation {generation}")
}

/// The generation the snapshot was recorded at, from its header; `None` if it has none.
fn snapshot_generation(text: &str) -> Option<i32> {
    text.lines()
        .find_map(|l| l.strip_prefix("# generation "))
        .and_then(|g| g.trim().parse().ok())
}

/// Does a raise excuse vanished doors? Only while the snapshot predates today's generation: the
/// raise is for the doors the generation before it had. A snapshot already regenerated at the
/// raising generation, or with no header, excuses nothing. **Pure.**
fn raise_excuses(snapshot_generation: Option<i32>, current: i32, last_raised: bool) -> bool {
    last_raised && snapshot_generation.is_some_and(|g| g < current)
}

/// What CHECK mode says about the vanished entries `not_facing` did not excuse.
#[derive(Debug, PartialEq)]
enum Check {
    Pass,
    /// A break for old windows: no raise excuses it.
    Vanished,
    /// A raise excuses it, but only a regeneration may apply that excuse: it drops the entries
    /// and bumps the header, so the raise cannot go on excusing doors for its generation's life.
    RegenerateAtTheRaise,
}

/// CHECK mode's decision. **Pure.** `raise_excuses` is [`raise_excuses`]'s answer.
fn check_vanished(vanished: &[String], raise_excuses: bool) -> Check {
    match (vanished.is_empty(), raise_excuses) {
        (true, _) => Check::Pass,
        (false, false) => Check::Vanished,
        (false, true) => Check::RegenerateAtTheRaise,
    }
}

/// Only `CAIRN_REGEN_DOOR_SURFACE=1` regenerates: `=0` must not rewrite the snapshot.
fn regen_requested(value: Option<&str>) -> bool {
    value == Some("1")
}

/// Parse a `<entry>  # <reason>` file into the set of entries (blank / `#` lines ignored).
fn read_set(path: &str, strip_reason: bool) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once("  # ") {
            Some((e, _)) if strip_reason => e.to_string(),
            _ => l.to_string(),
        })
        .collect()
}

#[tokio::test]
async fn the_windows_doors_never_vanish_within_the_floor() {
    let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
        eprintln!("skipped: CAIRN_TEST_PG unset");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    let mut catalogue = BTreeSet::new();
    for r in c
        .query(
            "SELECT 'fn ' || p.oid::regprocedure::text || ' -> ' || pg_get_function_result(p.oid)
               FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
              WHERE n.nspname = 'public'
                AND has_function_privilege('cairn_window', p.oid, 'EXECUTE')
                AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid
                                  AND d.classid = 'pg_proc'::regclass AND d.deptype = 'e')",
            &[],
        )
        .await
        .unwrap()
    {
        catalogue.insert(r.get::<_, String>(0));
    }
    for r in c
        .query(
            "SELECT 'col ' || cl.relname || '.' || a.attname
               FROM pg_attribute a JOIN pg_class cl ON cl.oid = a.attrelid
               JOIN pg_namespace n ON n.oid = cl.relnamespace
              WHERE n.nspname = 'public' AND cl.relkind IN ('r','v','m','p','f')
                AND a.attnum > 0 AND NOT a.attisdropped
                AND has_column_privilege('cairn_window', a.attrelid, a.attnum, 'SELECT')
                AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = cl.oid
                                  AND d.classid = 'pg_class'::regclass AND d.deptype = 'e')",
            &[],
        )
        .await
        .unwrap()
    {
        catalogue.insert(r.get::<_, String>(0));
    }

    let snapshot = read_set(SNAPSHOT, false);
    let not_facing = read_set(NOT_FACING, true);
    let recorded_at = snapshot_generation(&std::fs::read_to_string(SNAPSHOT).unwrap_or_default());
    let raised = raise_excuses(recorded_at, SCHEMA_GENERATION, last_entry_raised_floor());

    if regen_requested(std::env::var("CAIRN_REGEN_DOOR_SURFACE").ok().as_deref()) {
        // (old ∪ catalogue) minus what the exemptions let go; a non-exempt vanished entry stays.
        let mut next: BTreeSet<String> = snapshot.union(&catalogue).cloned().collect();
        for gone in snapshot.difference(&catalogue) {
            if raised || not_facing.contains(gone) {
                next.remove(gone);
            }
        }
        let mut lines = vec![snapshot_header(SCHEMA_GENERATION)];
        lines.extend(next);
        let mut text = lines.join("\n");
        text.push('\n');
        std::fs::write(SNAPSHOT, text).unwrap();
        return;
    }

    let d = door_drift(&snapshot, &catalogue, &not_facing);
    assert_ne!(
        check_vanished(&d.vanished, raised),
        Check::RegenerateAtTheRaise,
        "the floor was raised at generation {SCHEMA_GENERATION}, so these vanished doors are \
         excused — but only a regeneration applies that, bumping the snapshot's header to the \
         raise. Run `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test \
         window_door_surface` and commit db/window_door_surface.txt:\n{}",
        d.vanished.join("\n")
    );
    assert!(
        d.vanished.is_empty(),
        "doors the window could reach have VANISHED without the floor being raised (an older \
         window would break). Restore them, list them in db/window_not_facing.txt with a reason, \
         or raise the floor with a WINDOW_FLOOR_LEDGER entry at a new generation (a raise excuses \
         only a snapshot recorded before it, `# generation N`):\n{}",
        d.vanished.join("\n")
    );
    assert!(
        d.unrecorded.is_empty(),
        "new window-reachable doors are not in db/window_door_surface.txt; run \
         `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test window_door_surface`:\n{}",
        d.unrecorded.join("\n")
    );
}

#[cfg(test)]
mod pure {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_vanished_entry_fails_when_the_floor_was_not_raised() {
        let d = door_drift(&set(&["fn a", "fn b"]), &set(&["fn a"]), &set(&[]));
        assert_eq!(d.vanished, vec!["fn b".to_string()]);
    }

    #[test]
    fn a_vanished_entry_listed_as_not_facing_passes() {
        let d = door_drift(&set(&["fn a", "fn b"]), &set(&["fn a"]), &set(&["fn b"]));
        assert!(d.vanished.is_empty());
    }

    /// Re-review of I3a: CHECK mode never applies the raise's excuse itself. A vanished door
    /// under a raise fails with "regenerate at the raise", so the only way past it is a
    /// regeneration, which drops the door AND bumps the header — after which an in-place edit at
    /// the raising generation that drops another door is no longer excused.
    #[test]
    fn a_raise_is_applied_only_by_regenerating() {
        let gone = vec!["fn b".to_string()];
        assert_eq!(check_vanished(&[], false), Check::Pass);
        assert_eq!(check_vanished(&[], true), Check::Pass);
        assert_eq!(check_vanished(&gone, false), Check::Vanished);
        assert_eq!(check_vanished(&gone, true), Check::RegenerateAtTheRaise);
    }

    /// Final review I3: a raise excuses vanished doors only for a snapshot recorded BEFORE the
    /// current generation. Once regenerated at it, an in-place edit (ADR-0080) at that same
    /// generation that drops a door fails like any other.
    #[test]
    fn a_raise_excuses_only_a_snapshot_older_than_the_generation() {
        assert!(
            raise_excuses(Some(84), 85, true),
            "the snapshot predates the raise"
        );
        assert!(
            !raise_excuses(Some(85), 85, true),
            "regenerated at the raise: no excuse"
        );
        assert!(!raise_excuses(Some(84), 85, false), "no raise, no excuse");
        assert!(
            !raise_excuses(None, 85, true),
            "no header: every doubt toward failing"
        );
    }

    #[test]
    fn the_snapshot_header_names_its_generation() {
        assert_eq!(snapshot_generation("# generation 85\ncol a.b\n"), Some(85));
        assert_eq!(snapshot_generation("col a.b\n"), None);
        assert_eq!(snapshot_generation("# generation eighty\n"), None);
        // `read_set` skips the header, so it is never an entry.
        assert_eq!(snapshot_header(85), "# generation 85");
    }

    /// Final review M6: only `CAIRN_REGEN_DOOR_SURFACE=1` regenerates; `=0` or empty does not.
    #[test]
    fn only_1_regenerates() {
        assert!(regen_requested(Some("1")));
        for no in [None, Some("0"), Some(""), Some("true")] {
            assert!(!regen_requested(no), "{no:?}");
        }
    }

    #[test]
    fn a_new_catalogue_entry_is_unrecorded() {
        let d = door_drift(&set(&["fn a"]), &set(&["fn a", "fn c"]), &set(&[]));
        assert_eq!(d.unrecorded, vec!["fn c".to_string()]);
    }
}
