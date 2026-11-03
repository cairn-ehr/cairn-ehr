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
/// unreadable for a moment would orphan every draft held under it.
pub fn load_or_mint_terminal_id(dir: &Path) -> std::io::Result<String> {
    let path = dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) if is_usable(text.trim()) => return Ok(text.trim().to_string()),
        Ok(_) => {}
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
            ) => {}
        Err(e) => return Err(e),
    }
    let id = uuid::Uuid::now_v7().to_string();
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, &id)?;
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
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
