//! SQL function-body text helpers for the catalogue guards — shared so two guards cannot drift.
//!
//! `late_custody_guards.rs` (#584) and `substitution_guard_covers_every_writer.rs` (#619) both read
//! function bodies out of `pg_proc.prosrc` and ask what the CODE does while ignoring what the
//! comments say. That needs one comment stripper. A stripper written twice is the drift #608 was
//! made of — one invariant spelled in two places, wrong in both at once — so it was moved here,
//! unchanged, the moment a second guard needed it.
//!
//! Include with `#[path = "common/sql_text.rs"] mod sql_text;`.
//!
//! Literal-blind, like the original: a `--` inside a string literal hides the rest of its line, and a
//! `/*` inside one hides everything up to the next `*/`. No call site either guard looks for follows
//! a comment marker inside a literal; keep it that way.
#![allow(dead_code)] // each including suite uses a different subset

/// The body with its SQL comments removed: `--` line comments and `/* ... */` block comments.
/// **Pure, and literal-blind.**
///
/// One left-to-right pass, because the two comment kinds hide each other and the order matters:
/// a `/*` inside a line comment opens nothing (our own SQL writes `-- ... db/*.sql ...` in
/// function bodies), and a `--` inside a block comment is just comment text. Stripping block
/// comments first and line comments second would get the first case wrong and swallow real code.
///
/// Block comments NEST in Postgres (`/* a /* b */ still comment */`), so a depth counter tracks
/// them; a comment is replaced by one space because it separates tokens exactly as a space does.
/// An unterminated block comment swallows the rest of the body — Postgres would refuse such a
/// body anyway.
///
/// Literal-blind: there is no awareness of string or dollar-quoted literals, so a `--` inside a
/// literal hides the rest of that line and a `/*` inside one hides everything up to its `*/`.
pub fn without_sql_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    // > 0 while inside a block comment; counts how many `/*` are still open.
    let mut block_depth = 0usize;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("/*") {
            // Checked first so a `/*` opens (or nests) a block comment wherever it starts —
            // unless an earlier `--` on this line already skipped it (the branch below).
            block_depth += 1;
            rest = after;
        } else if block_depth > 0 {
            if let Some(after) = rest.strip_prefix("*/") {
                block_depth -= 1;
                if block_depth == 0 {
                    out.push(' ');
                }
                rest = after;
            } else {
                rest = without_first_char(rest);
            }
        } else if let Some(after) = rest.strip_prefix("--") {
            // Skip to the end of the line, keeping the newline itself.
            rest = after.find('\n').map_or("", |at| &after[at..]);
        } else {
            let mut chars = rest.chars();
            if let Some(ch) = chars.next() {
                out.push(ch);
            }
            rest = chars.as_str();
        }
    }
    out
}

/// `text` without its first character (UTF-8 aware, so a multi-byte character such as an em dash
/// in a comment is never split). **Pure.**
fn without_first_char(text: &str) -> &str {
    let mut chars = text.chars();
    chars.next();
    chars.as_str()
}

/// Uppercase with every whitespace run collapsed to one space, so `insert  into\n event_clear`
/// matches. **Pure.**
pub fn normalised(body: &str) -> String {
    without_sql_comments(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}
