//! This terminal's stable id — what a note draft is "held by" (sign-in slice 3a, design §3.2).
//!
//! A draft is edited only at the terminal it was started on: nothing can tell reliably whether
//! someone is still typing into it on another machine, so two terminals editing one draft would
//! silently overwrite each other. The save door (db/065) enforces that by comparing the terminal
//! id each save names, which only works if a terminal names itself the SAME way after a restart —
//! otherwise every crash would leave its own drafts looking like another terminal's.
//!
//! So the id is minted once (a UUIDv7) and kept in a one-line file in the window's config
//! directory. It is not a secret and proves nothing about the machine (a raw-SQL client can name
//! any terminal; ADR-0087 records that residual) — it is a label, kept stable.
use std::path::Path;

/// The file, inside the window's config directory, that holds the id.
const FILE: &str = "terminal-id";

/// Read this terminal's id from `dir/terminal-id`, or mint a fresh UUIDv7, write it there
/// (creating `dir`), and return it.
///
/// A file holding blank or control text is REPLACED, never trusted: that text would be sent to
/// the save door as the terminal's name, and a blank or garbled one is no name at all. Surrounding
/// whitespace (a newline an editor added) is ignored. A file that cannot be read for any reason
/// other than its absence or non-UTF-8 content is an error — replacing an id that is merely
/// unreadable for a moment would orphan every draft held under it. A replacement is logged:
/// drafts held under the old id now read as another terminal's, and someone should know why.
///
/// The new id is written to a side file and renamed into place, so a crash mid-write never
/// leaves a blank `terminal-id` that the next launch would replace (orphaning this one too).
pub fn load_or_mint_terminal_id(dir: &Path) -> std::io::Result<String> {
    let path = dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) if is_usable(text.trim()) => return Ok(text.trim().to_string()),
        Ok(_) => eprintln!("{} holds no usable id; minting a new one", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
            eprintln!("{} is not text; minting a new one", path.display())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let id = uuid::Uuid::now_v7().to_string();
    std::fs::create_dir_all(dir)?;
    let side = dir.join(format!("{FILE}.new"));
    std::fs::write(&side, &id)?;
    std::fs::rename(&side, &path)?;
    Ok(id)
}

/// Whether `id` (already trimmed) can name a terminal: not empty, and no control characters.
fn is_usable(id: &str) -> bool {
    !id.is_empty() && !id.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory per test (a v7 uuid keeps runs and tests apart).
    fn scratch() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cairn-terminal-id-{}", uuid::Uuid::now_v7()))
    }

    #[test]
    fn mints_once_then_reads_the_same_id() {
        let dir = scratch().join("nested");
        let first = load_or_mint_terminal_id(&dir).unwrap();
        assert!(first.parse::<uuid::Uuid>().is_ok(), "a UUID: {first}");
        assert_eq!(load_or_mint_terminal_id(&dir).unwrap(), first);
        // An editor's trailing newline is not a different terminal.
        std::fs::write(dir.join(FILE), format!("{first}\n")).unwrap();
        assert_eq!(load_or_mint_terminal_id(&dir).unwrap(), first);
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_blank_file_is_replaced() {
        let dir = scratch();
        std::fs::create_dir_all(&dir).unwrap();
        for bad in ["", "  \n", "pc\u{7}1"] {
            std::fs::write(dir.join(FILE), bad).unwrap();
            let id = load_or_mint_terminal_id(&dir).unwrap();
            assert!(
                id.parse::<uuid::Uuid>().is_ok(),
                "{bad:?} was trusted: {id}"
            );
            assert_eq!(std::fs::read_to_string(dir.join(FILE)).unwrap(), id);
            assert!(
                !dir.join(format!("{FILE}.new")).exists(),
                "the side file is renamed away"
            );
        }
        // Not text at all: replaced too.
        std::fs::write(dir.join(FILE), [0xff, 0xfe, 0x00]).unwrap();
        let id = load_or_mint_terminal_id(&dir).unwrap();
        assert!(
            id.parse::<uuid::Uuid>().is_ok(),
            "non-UTF-8 was trusted: {id}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
