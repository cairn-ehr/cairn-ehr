//! Test-only helpers for guards that read this crate's own source.
//!
//! The window has no DB-backed tests (#738), so a property that lives in the ORDER or the CHOICE
//! of node calls inside a command — invisible to every pure test — is pinned by reading the
//! command's source instead. Each such guard says in its doc comment why no behavioural test can
//! reach the property; this module only finds the text.

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

/// Where `needle` first appears in `body`, or a panic naming what was looked for.
pub fn position(body: &str, needle: &str) -> usize {
    body.find(needle)
        .unwrap_or_else(|| panic!("`{needle}` is called in:\n{body}"))
}
