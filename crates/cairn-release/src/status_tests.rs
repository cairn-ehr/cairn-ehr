use crate::{InstallerState, InstallerStatus};

/// What the installer writes, the window reads back unchanged (`cairn-terminal-install` writes
/// through this type).
#[test]
fn installer_status_round_trips() {
    for status in [
        InstallerStatus {
            state: InstallerState::Idle,
            installed_release: 0,
            pinned_root_version: 1,
            sentence: None,
            refused_release: None,
        },
        InstallerStatus {
            state: InstallerState::Refused,
            installed_release: 7,
            pinned_root_version: 2,
            refused_release: Some(8),
            sentence: Some(
                "This update is not signed by enough of the Cairn project's release signers \
                 (1 of 2). Do not install it; report it to the Cairn project."
                    .into(),
            ),
        },
    ] {
        assert_eq!(InstallerStatus::from_json(&status.to_json()), Some(status));
    }
}

/// The plan's shape, field for field: a hand-written file in it is read.
#[test]
fn the_plans_shape_is_read() {
    let file = br#"{ "state": "installed", "installed_release": 12, "pinned_root_version": 3, "sentence": null }"#;
    let status = InstallerStatus::from_json(file).unwrap();
    assert_eq!(
        (
            status.state,
            status.installed_release,
            status.pinned_root_version
        ),
        (InstallerState::Installed, 12, 3)
    );
    assert_eq!(status.sentence, None);
    // A file written before `refused_release` existed still reads, as no such refusal.
    assert_eq!(status.refused_release, None);
}

/// A state this program does not know (a newer installer's) still reads, as `Unknown`: the two
/// programs are often of different releases, and a status is never thrown away for it.
#[test]
fn a_newer_installers_state_still_reads() {
    let file = br#"{ "state": "paused", "installed_release": 4, "pinned_root_version": 2, "sentence": null }"#;
    let status = InstallerStatus::from_json(file).expect("still a status");
    assert_eq!(
        (status.state, status.installed_release),
        (InstallerState::Unknown, 4)
    );
}

/// A damaged or half-written file is no status, never a guess (the window then fetches nothing).
#[test]
fn a_damaged_file_is_no_status() {
    for file in [
        &b""[..],
        b"{",
        b"{\"state\":\"idle\"}",
        b"not json",
        b"[1,2]",
    ] {
        assert_eq!(InstallerStatus::from_json(file), None, "{file:?}");
    }
}
