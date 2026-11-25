//! The staging folder's contract between the window (courier) and the terminal installer.
use crate::staging::*;

/// The names are a contract ACROSS releases: an older window stages for a newer installer
/// and the reverse, so they never change.
#[test]
fn the_staging_names_never_change() {
    assert_eq!(
        (META, ARTIFACT, COMPLETE),
        ("meta.cbor", "artifact.bin", "complete")
    );
    assert_eq!(MAX_ARTIFACT, 64 * 1024 * 1024);
}

const OS_RELEASE: &str = "PRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nNAME=\"Debian GNU/Linux\"\nVERSION_ID=\"12\"\nID=debian\n";

/// `/etc/os-release`'s two keys, quoted or not; anything without both is no platform.
#[test]
fn the_platform_is_id_and_version_id() {
    assert_eq!(platform_of(OS_RELEASE).as_deref(), Some("debian-12"));
    assert_eq!(
        platform_of("ID='raspbian'\nVERSION_ID=11").as_deref(),
        Some("raspbian-11")
    );
    assert_eq!(
        platform_of("VERSION_ID=\"12\"\nID=debian").as_deref(),
        Some("debian-12")
    );
    // A `#` comment and an `ID_LIKE` line are not `ID`.
    assert_eq!(
        platform_of("# ID=fake\nID_LIKE=debian\nID=ubuntu\nVERSION_ID=\"24.04\"").as_deref(),
        Some("ubuntu-24.04")
    );
    for text in ["", "ID=debian", "VERSION_ID=12", "ID=\nVERSION_ID=12"] {
        assert_eq!(platform_of(text), None, "{text:?}");
    }
    let target = this_target(Some(OS_RELEASE)).unwrap();
    assert_eq!(
        (target.os.as_str(), target.arch.as_str()),
        (std::env::consts::OS, std::env::consts::ARCH)
    );
}
