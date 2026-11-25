use crate::InstallerStatus;

/// What the installer writes, the window reads back unchanged (Task 9 writes through this type).
#[test]
fn installer_status_round_trips() {
    for status in [
        InstallerStatus {
            state: "idle".into(),
            installed_release: 0,
            pinned_root_version: 1,
            sentence: None,
            refused_release: None,
        },
        InstallerStatus {
            state: "refused".into(),
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
            status.state.as_str(),
            status.installed_release,
            status.pinned_root_version
        ),
        ("installed", 12, 3)
    );
    assert_eq!(status.sentence, None);
    // A file written before `refused_release` existed still reads, as no such refusal.
    assert_eq!(status.refused_release, None);
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
