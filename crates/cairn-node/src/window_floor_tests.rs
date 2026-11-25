use super::*;
use crate::db::{SchemaStamp, WindowFloor};

fn stamp(v: i32, build: &str, floor: Option<(i32, Option<&str>)>) -> SchemaStamp {
    SchemaStamp {
        version: v,
        loader_build: build.to_string(),
        schema_hash: None,
        window_floor: floor.map(|(f, r)| WindowFloor {
            floor: f,
            reason: r.map(str::to_string),
        }),
    }
}

const NODE: &str = "cairn-node 9.9.9";

#[test]
fn window_verdict_cases() {
    // No stamp, an older database, a foreign loader: refused, with today's wording.
    let e = match window_verdict(None, 5) {
        WindowVerdict::Refused(e) => e,
        v => panic!("{v:?}"),
    };
    assert!(
        e.contains("has not been prepared for this window") && e.contains("provision-window-role")
    );
    assert!(matches!(
        window_verdict(Some(&stamp(4, NODE, None)), 5),
        WindowVerdict::Refused(e) if e.contains("has not been prepared for this window")
    ));
    assert!(matches!(
        window_verdict(Some(&stamp(5, "cairn-sync 0.1.0", None)), 5),
        WindowVerdict::Refused(e) if e.contains("different build")
    ));
    // v == w: current, whatever the hash.
    assert_eq!(
        window_verdict(Some(&stamp(5, NODE, None)), 5),
        WindowVerdict::Current
    );
    // f <= w < v: still served, an update is waiting.
    assert_eq!(
        window_verdict(Some(&stamp(7, NODE, Some((5, None)))), 5),
        WindowVerdict::UpdateWaiting
    );
    assert_eq!(
        window_verdict(Some(&stamp(7, NODE, Some((5, None)))), 6),
        WindowVerdict::UpdateWaiting
    );
    // w < f: below the floor, carrying the stamped reason.
    assert_eq!(
        window_verdict(Some(&stamp(7, NODE, Some((6, Some("why"))))), 5),
        WindowVerdict::BelowFloor {
            reason: Some("why".into())
        }
    );
    // NULL floor means v: any older window is below it.
    assert_eq!(
        window_verdict(Some(&stamp(7, NODE, None)), 6),
        WindowVerdict::BelowFloor { reason: None }
    );
}

fn build(generation: i32, b: u8, release: u64) -> InstalledBuild {
    InstalledBuild {
        generation,
        hash: [b; 32],
        release,
    }
}

#[test]
fn install_fit_cases() {
    let s = stamp(7, NODE, Some((5, None)));
    let fit = |i: Option<InstalledBuild>, r| install_fit(i, r, Some(&s));
    let running = build(7, 1, 0);
    assert_eq!(fit(None, running), InstallFit::Same);
    assert_eq!(fit(Some(build(7, 1, 0)), running), InstallFit::Same);
    assert_eq!(fit(Some(build(7, 2, 0)), running), InstallFit::Fits);
    assert_eq!(fit(Some(build(8, 2, 0)), running), InstallFit::DoesNotFit);
    assert_eq!(fit(Some(build(4, 2, 0)), running), InstallFit::DoesNotFit);
    assert_eq!(fit(Some(build(6, 2, 0)), running), InstallFit::Fits);
}

/// Same schema, newer signed release: the restart is worth it (design D5).
#[test]
fn a_same_schema_higher_release_fits() {
    let s = stamp(7, NODE, Some((5, None)));
    assert_eq!(
        install_fit(Some(build(7, 1, 3)), build(7, 1, 2), Some(&s)),
        InstallFit::Fits
    );
}

/// An installed program that is not newer than the running one is not an update, whatever else
/// differs (an older program that cannot answer the release flag reads as release 0).
#[test]
fn a_lower_release_is_same() {
    let s = stamp(7, NODE, Some((5, None)));
    assert_eq!(
        install_fit(Some(build(8, 2, 1)), build(7, 1, 2), Some(&s)),
        InstallFit::Same
    );
}
