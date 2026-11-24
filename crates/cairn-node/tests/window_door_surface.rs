//! #909 slice C1, Task 4 — the reference window's doors stay ADDITIVE within the floor.
//!
//! WHY. An older reference-UI window (generation G) keeps working against a database whose
//! window floor is <= G (`WINDOW_FLOOR_GENERATION`). That promise is only true if nothing the
//! window can reach disappears without the floor being raised. This guard makes that a CI
//! failure: `db/window_door_surface.txt` is a committed snapshot of everything the window's
//! database role (`cairn_window`) can reach, and a snapshot entry that vanishes from the live
//! catalogue fails the test unless
//!   (a) it is listed in `db/window_not_facing.txt` (`<entry>  # <reason>` — a door the window
//!       never faced, so removing it cannot hurt an old window), or
//!   (b) the floor ledger's LAST entry raised the floor (a deliberate, recorded break).
//!
//! The snapshot has one sorted line per entry:
//!   `fn <regprocedure> -> <result type>` for each `public` function `cairn_window` may EXECUTE;
//!   `col <relation>.<column>` for each `public` column `cairn_window` may SELECT.
//! Extension-owned objects (a `pg_depend` row of `deptype = 'e'`, e.g. everything `cairn_pgx`
//! installs) are EXCLUDED: their surface moves with the extension's own version, not with
//! `db/*.sql`, and would make the snapshot differ between machines with different builds.
//!
//! Regenerate with `CAIRN_REGEN_DOOR_SURFACE=1 cargo test -p cairn-node --test
//! window_door_surface`: it writes (old snapshot + catalogue) minus the entries the two
//! exemptions allow to go — a vanished entry is never forgotten silently.
//!
//! The comparison is the pure `door_drift`, unit-tested below without a database.
use cairn_event::schema_generation::WINDOW_FLOOR_LEDGER;
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

/// Pure comparison. `floor_raised` = the ledger's last entry raised the floor, which excuses
/// every vanished entry; `not_facing` excuses individual ones.
fn door_drift(
    snapshot: &BTreeSet<String>,
    catalogue: &BTreeSet<String>,
    not_facing: &BTreeSet<String>,
    floor_raised: bool,
) -> Drift {
    let vanished = if floor_raised {
        vec![]
    } else {
        snapshot
            .difference(catalogue)
            .filter(|e| !not_facing.contains(*e))
            .cloned()
            .collect()
    };
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
    let raised = last_entry_raised_floor();

    if std::env::var("CAIRN_REGEN_DOOR_SURFACE").is_ok() {
        // (old ∪ catalogue) minus what the exemptions let go; a non-exempt vanished entry stays.
        let mut next: BTreeSet<String> = snapshot.union(&catalogue).cloned().collect();
        for gone in snapshot.difference(&catalogue) {
            if raised || not_facing.contains(gone) {
                next.remove(gone);
            }
        }
        let mut text = next.into_iter().collect::<Vec<_>>().join("\n");
        text.push('\n');
        std::fs::write(SNAPSHOT, text).unwrap();
        return;
    }

    let d = door_drift(&snapshot, &catalogue, &not_facing, raised);
    assert!(
        d.vanished.is_empty(),
        "doors the window could reach have VANISHED without the floor being raised (an older \
         window would break). Restore them, list them in db/window_not_facing.txt with a reason, \
         or raise the floor with a WINDOW_FLOOR_LEDGER entry:\n{}",
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
        let d = door_drift(&set(&["fn a", "fn b"]), &set(&["fn a"]), &set(&[]), false);
        assert_eq!(d.vanished, vec!["fn b".to_string()]);
    }

    #[test]
    fn a_vanished_entry_listed_as_not_facing_passes() {
        let d = door_drift(
            &set(&["fn a", "fn b"]),
            &set(&["fn a"]),
            &set(&["fn b"]),
            false,
        );
        assert!(d.vanished.is_empty());
    }

    #[test]
    fn a_vanished_entry_passes_when_the_floor_was_raised() {
        let d = door_drift(&set(&["fn a", "fn b"]), &set(&["fn a"]), &set(&[]), true);
        assert!(d.vanished.is_empty());
    }

    #[test]
    fn a_new_catalogue_entry_is_unrecorded() {
        let d = door_drift(&set(&["fn a"]), &set(&["fn a", "fn c"]), &set(&[]), false);
        assert_eq!(d.unrecorded, vec!["fn c".to_string()]);
    }
}
