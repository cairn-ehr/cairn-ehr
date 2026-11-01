//! Test-only helpers for guards that read this crate's own source.
//!
//! The window has no DB-backed tests (#738), so a property that lives in the ORDER or the CHOICE
//! of node calls inside a command — invisible to every pure test — is pinned by reading the
//! command's source instead. Each such guard says in its doc comment why no behavioural test can
//! reach the property; this module only finds the text.
use std::path::Path;

/// The body of Rust function `name` in `src`, to the first line that is a lone `}` (rustfmt
/// closes every top-level fn that way).
pub fn rust_fn<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("{name} is declared"));
    let len = src[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} closes"));
    &src[start..start + len]
}

/// The body of top-level JS function `name` in `src`: from its declaration to the first line
/// that is a lone `}` (the house formatting closes every top-level function that way). The
/// webview's half of a guard: its scripts have no test rig of their own (no npm, by design), so
/// a property that lives in the ORDER of a function's steps is pinned by reading it.
pub fn js_fn<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("{name} is declared"));
    let len = src[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} closes"));
    &src[start..start + len]
}

/// The text of top-level JS `const name = …;` in `src` — every string piece of a sentence the
/// window words itself, concatenated, so a guard can read the sentence a clinician sees.
pub fn js_const(src: &str, name: &str) -> String {
    let start = src
        .find(&format!("const {name} ="))
        .unwrap_or_else(|| panic!("{name} is declared"));
    let declaration = &src[start..start + src[start..].find(";\n").expect("the const closes")];
    // The odd-numbered pieces between double quotes are the string literals.
    declaration.split('"').skip(1).step_by(2).collect()
}

/// Where `needle` first appears in `body`, or a panic naming what was looked for.
pub fn position(body: &str, needle: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is called in:\n{body}"))
}

/// Every non-test `.rs` file under `dir`, as (path, source). Files named `*tests.rs` are skipped.
pub fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("the source dir reads") {
        let path = entry.expect("a dir entry").path();
        let name = path.to_string_lossy().into_owned();
        if path.is_dir() {
            sources(&path, out);
        } else if name.ends_with(".rs") && !name.ends_with("tests.rs") {
            out.push((
                name,
                std::fs::read_to_string(&path).expect("the file reads"),
            ));
        }
    }
}
