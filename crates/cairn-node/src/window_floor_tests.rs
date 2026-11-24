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

#[test]
fn install_fit_cases() {
    let h = |b: u8| [b; 32];
    let s = stamp(7, NODE, Some((5, None)));
    assert_eq!(install_fit(None, (7, h(1)), Some(&s)), InstallFit::Same);
    assert_eq!(
        install_fit(Some((7, h(1))), (7, h(1)), Some(&s)),
        InstallFit::Same
    );
    assert_eq!(
        install_fit(Some((7, h(2))), (7, h(1)), Some(&s)),
        InstallFit::Fits
    );
    assert_eq!(
        install_fit(Some((8, h(2))), (7, h(1)), Some(&s)),
        InstallFit::DoesNotFit
    );
    assert_eq!(
        install_fit(Some((4, h(2))), (7, h(1)), Some(&s)),
        InstallFit::DoesNotFit
    );
    assert_eq!(
        install_fit(Some((6, h(2))), (7, h(1)), Some(&s)),
        InstallFit::Fits
    );
}
