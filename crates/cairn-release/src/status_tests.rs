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
        },
        InstallerStatus {
            state: "refused".into(),
            installed_release: 7,
            pinned_root_version: 2,
            sentence: Some(
                "This update file is damaged or is not a Cairn update; fetch it again.".into(),
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
